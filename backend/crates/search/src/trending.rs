//! Trending queries, popular completions and the blocklist.
//!
//! Counting happens in Valkey, per hour: for every query one HyperLogLog of
//! the signed-in users who searched it, and one sorted set per hour scoring
//! each query by how many distinct users that is. Nothing else is kept about
//! who searched what, and the keys expire after the trending window.
//!
//! [`Search::refresh_trending`] (a worker timer) folds the hours of the
//! window into `search_popular_queries`: recent hours weigh more, and a query
//! only qualifies when enough *different* users searched it within the
//! window — one person repeating a query, however often, adds one user. The
//! table serves the trending list and, by prefix, the popular completions of
//! the typeahead, so a query only one person ever typed is never shown to
//! anyone else.
//!
//! Moderators keep a blocklist; a query containing a blocked term never
//! trends and is never offered as a completion.
use std::{
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use tokuly_core::prelude::*;

use crate::{Search, text::prefix_upper_bound};

/// Longest query that can trend, in characters.
const MAX_TREND_CHARS: usize = 40;
/// Queries looked at per refresh, by weighted searcher count.
const CANDIDATES: isize = 300;
/// How long a process trusts its copy of the blocklist.
const BLOCKLIST_TTL: Duration = Duration::from_secs(15);

/// Adds a searcher to a query's HyperLogLog and, when they are new to it this
/// hour, moves the query up in the hour's sorted set.
/// KEYS: the HyperLogLog, the sorted set. ARGV: user, query, TTL in seconds.
const RECORD_SCRIPT: &str = r"
if redis.call('PFADD', KEYS[1], ARGV[1]) == 1 then
    redis.call('ZINCRBY', KEYS[2], 1, ARGV[2])
end
redis.call('EXPIRE', KEYS[1], ARGV[3])
redis.call('EXPIRE', KEYS[2], ARGV[3])
return 1";

/// A query that qualified in the latest refresh.
#[derive(Debug, Clone, PartialEq)]
pub struct PopularQuery {
    pub query: String,
    /// Distinct signed-in users who searched it within the window.
    pub searchers: u32,
    /// Recency-weighted searcher count.
    pub weight: f64,
    /// Position in the trending list (1 = top); `None` below the cut.
    pub rank: Option<i32>,
}

/// A process's copy of the blocklist.
#[derive(Default)]
pub struct BlocklistCache {
    loaded: RwLock<Option<(Instant, Arc<Vec<String>>)>>,
}

/// Whether a query (by its key) contains a blocked term.
pub fn is_blocked(terms: &[String], query_key: &str) -> bool {
    terms.iter().any(|term| query_key.contains(term.as_str()))
}

/// Whether a query can trend at all: short enough to be a query rather than
/// a pasted text, and more than a lone ASCII character or punctuation.
pub fn can_trend(query_key: &str) -> bool {
    let chars = query_key.chars().count();
    let meaningful = query_key.chars().filter(|c| c.is_alphanumeric()).count();
    chars <= MAX_TREND_CHARS && meaningful >= 1 && (meaningful >= 2 || !query_key.is_ascii())
}

/// Weight of searches `age_hours` old: halves every `half_life_hours`.
pub fn hour_weight(age_hours: u32, half_life_hours: u32) -> f64 {
    0.5f64.powf(age_hours as f64 / half_life_hours.max(1) as f64)
}

impl Search {
    fn trend_set_key(&self, hour: i64) -> String {
        // The braces make every trending key of a deployment hash to the
        // same cluster slot, which the multi-key commands below need.
        format!("search:{{{}}}:trend:z:{hour}", self.cfg.index_prefix)
    }

    fn trend_users_key(&self, hour: i64, query_key: &str) -> String {
        format!("search:{{{}}}:trend:u:{hour}:{query_key}", self.cfg.index_prefix)
    }

    /// The blocklist, at most a few seconds old.
    pub(crate) async fn blocked_terms(&self) -> AppResult<Arc<Vec<String>>> {
        let fresh = {
            let loaded = self.blocklist.loaded.read().expect("blocklist lock");
            loaded.as_ref().filter(|(loaded_at, _)| loaded_at.elapsed() < BLOCKLIST_TTL).map(|(_, terms)| terms.clone())
        };
        if let Some(terms) = fresh {
            return Ok(terms);
        }
        let terms: Vec<String> = sqlx::query_scalar("SELECT term FROM search_blocked_terms").fetch_all(&self.core.db).await?;
        let terms = Arc::new(terms);
        *self.blocklist.loaded.write().expect("blocklist lock") = Some((Instant::now(), terms.clone()));
        Ok(terms)
    }

    /// Forgets this process's copy of the blocklist (after changing it).
    pub(crate) fn invalidate_blocklist(&self) {
        *self.blocklist.loaded.write().expect("blocklist lock") = None;
    }

    /// Counts a signed-in user's search towards trending. Never fails the
    /// search it belongs to.
    pub(crate) async fn record_trend(&self, user_id: Uuid, query_key: &str) {
        if !can_trend(query_key) {
            return;
        }
        match self.blocked_terms().await {
            Ok(terms) if is_blocked(&terms, query_key) => return,
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(%error, "blocklist unavailable; search not counted towards trending");
                return;
            }
        }
        let hour = now().timestamp().div_euclid(3600);
        // Kept one hour longer than the window, so the oldest hour of a
        // refresh is still complete.
        let ttl = (self.cfg.trending_window_hours as i64 + 1) * 3600;
        let counted: Result<i64, redis::RedisError> = redis::Script::new(RECORD_SCRIPT)
            .key(self.trend_users_key(hour, query_key))
            .key(self.trend_set_key(hour))
            .arg(user_id.simple().to_string())
            .arg(query_key)
            .arg(ttl)
            .invoke_async(&mut self.core.cache.conn())
            .await;
        if let Err(error) = counted {
            tracing::warn!(%error, "search not counted towards trending");
        }
    }

    /// Recomputes the trending list and the popular queries from the hourly
    /// counters. Refreshes are serialized, so the list a refresh leaves
    /// behind includes every search counted before the call. Returns the
    /// queries that qualified, most searched first.
    pub async fn refresh_trending(&self) -> AppResult<Vec<PopularQuery>> {
        let mut tx = self.core.db.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("search:trending:{}", self.cfg.index_prefix))
            .execute(&mut *tx)
            .await?;

        let hour = now().timestamp().div_euclid(3600);
        let window = self.cfg.trending_window_hours;
        let hours: Vec<i64> = (0..window as i64).map(|age| hour - age).collect();
        let set_keys: Vec<String> = hours.iter().map(|h| self.trend_set_key(*h)).collect();
        let weights: Vec<f64> = (0..window).map(|age| hour_weight(age, self.cfg.trending_half_life_hours)).collect();
        let scratch = format!("search:{{{}}}:trend:tmp:{}", self.cfg.index_prefix, Uuid::now_v7().simple());

        let mut conn = self.core.cache.conn();
        let (candidates,): (Vec<(String, f64)>,) = redis::pipe()
            .atomic()
            .cmd("ZUNIONSTORE")
            .arg(&scratch)
            .arg(set_keys.len())
            .arg(&set_keys)
            .arg("WEIGHTS")
            .arg(&weights)
            .ignore()
            .cmd("ZREVRANGE")
            .arg(&scratch)
            .arg(0)
            .arg(CANDIDATES - 1)
            .arg("WITHSCORES")
            .cmd("DEL")
            .arg(&scratch)
            .ignore()
            .query_async(&mut conn)
            .await?;

        // Distinct users over the whole window: the union of the hourly
        // HyperLogLogs, so someone searching every hour still counts once.
        let searchers: Vec<u64> = if candidates.is_empty() {
            Vec::new()
        } else {
            let mut pipe = redis::pipe();
            for (query, _) in &candidates {
                let keys: Vec<String> = hours.iter().map(|h| self.trend_users_key(*h, query)).collect();
                pipe.cmd("PFCOUNT").arg(keys);
            }
            pipe.query_async(&mut conn).await?
        };

        let blocked = self.blocked_terms().await?;
        let mut qualified: Vec<PopularQuery> = candidates
            .into_iter()
            .zip(searchers)
            .filter(|((query, _), searchers)| *searchers >= self.cfg.trending_min_users as u64 && !is_blocked(&blocked, query))
            .map(|((query, weight), searchers)| PopularQuery { query, searchers: searchers.min(u32::MAX as u64) as u32, weight, rank: None })
            .collect();
        for (position, entry) in qualified.iter_mut().take(self.cfg.trending_size as usize).enumerate() {
            entry.rank = Some(position as i32 + 1);
        }

        sqlx::query("UPDATE search_popular_queries SET trending_rank = NULL WHERE trending_rank IS NOT NULL").execute(&mut *tx).await?;
        if !qualified.is_empty() {
            let queries: Vec<&str> = qualified.iter().map(|q| q.query.as_str()).collect();
            let counts: Vec<i32> = qualified.iter().map(|q| q.searchers.min(i32::MAX as u32) as i32).collect();
            let weights: Vec<f64> = qualified.iter().map(|q| q.weight).collect();
            let ranks: Vec<Option<i32>> = qualified.iter().map(|q| q.rank).collect();
            sqlx::query(
                "INSERT INTO search_popular_queries (query, searchers, weight, trending_rank, last_seen_at)
                 SELECT t.query, t.searchers, t.weight, t.trending_rank, now()
                 FROM unnest($1::text[], $2::int[], $3::float8[], $4::int[]) AS t(query, searchers, weight, trending_rank)
                 ON CONFLICT (query) DO UPDATE SET
                     searchers = EXCLUDED.searchers, weight = EXCLUDED.weight,
                     trending_rank = EXCLUDED.trending_rank, last_seen_at = EXCLUDED.last_seen_at",
            )
            .bind(&queries)
            .bind(&counts)
            .bind(&weights)
            .bind(&ranks)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "INSERT INTO search_state (key, updated_at) VALUES ('trending', now())
             ON CONFLICT (key) DO UPDATE SET updated_at = now()",
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(qualified)
    }

    /// The current trending list (at most `limit` queries, best first) and
    /// when it was computed.
    pub async fn trending(&self, limit: i64) -> AppResult<(Vec<String>, Option<Time>)> {
        let queries: Vec<String> = sqlx::query_scalar(
            "SELECT query::text FROM search_popular_queries WHERE trending_rank IS NOT NULL ORDER BY trending_rank LIMIT $1",
        )
        .bind(self.cfg.trending_size as i64)
        .fetch_all(&self.core.db)
        .await?;
        let updated_at: Option<Time> =
            sqlx::query_scalar("SELECT updated_at FROM search_state WHERE key = 'trending'").fetch_optional(&self.core.db).await?;
        // The list may predate the newest blocked term.
        let blocked = self.blocked_terms().await?;
        let queries = queries.into_iter().filter(|query| !is_blocked(&blocked, query)).take(limit.max(0) as usize).collect();
        Ok((queries, updated_at))
    }

    /// Popular queries starting with `prefix_key`, most searched first.
    pub(crate) async fn popular_completions(&self, prefix_key: &str, limit: i64) -> AppResult<Vec<String>> {
        if prefix_key.is_empty() {
            return Ok(Vec::new());
        }
        // A byte-order range scan on the primary key ("C" collation).
        let queries: Vec<String> = sqlx::query_scalar(
            "SELECT query::text FROM search_popular_queries
             WHERE query >= $1 AND query < $2 AND query <> $1
             ORDER BY weight DESC, query
             LIMIT $3",
        )
        .bind(prefix_key)
        .bind(prefix_upper_bound(prefix_key))
        .bind(limit + 5)
        .fetch_all(&self.core.db)
        .await?;
        let blocked = self.blocked_terms().await?;
        Ok(queries.into_iter().filter(|query| !is_blocked(&blocked, query)).take(limit.max(0) as usize).collect())
    }

    /// Removes popular queries that have not qualified for `days` days.
    pub(crate) async fn sweep_popular_queries(&self, days: i32) -> AppResult<u64> {
        let done = sqlx::query("DELETE FROM search_popular_queries WHERE last_seen_at < now() - make_interval(days => $1)")
            .bind(days)
            .execute(&self.core.db)
            .await?;
        Ok(done.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blocked_term_blocks_every_query_containing_it() {
        let terms = vec!["spam".to_owned(), "わるい言葉".to_owned()];
        assert!(is_blocked(&terms, "spam"));
        assert!(is_blocked(&terms, "free spam coins"));
        assert!(is_blocked(&terms, "とてもわるい言葉です"));
        assert!(!is_blocked(&terms, "span"));
        assert!(!is_blocked(&[], "spam"));
    }

    #[test]
    fn only_real_queries_can_trend() {
        for ok in ["渋谷", "猫", "tokyo tower", "#渋谷", "v2", "ab"] {
            assert!(can_trend(ok), "{ok} should be able to trend");
        }
        for not in ["", "a", "#", "!!!", "7", &"あ".repeat(MAX_TREND_CHARS + 1)] {
            assert!(!can_trend(not), "{not:?} should not trend");
        }
        assert!(can_trend(&"あ".repeat(MAX_TREND_CHARS)));
    }

    #[test]
    fn older_hours_weigh_less() {
        assert_eq!(hour_weight(0, 6), 1.0);
        assert!((hour_weight(6, 6) - 0.5).abs() < 1e-12);
        assert!((hour_weight(12, 6) - 0.25).abs() < 1e-12);
        assert!(hour_weight(3, 6) > hour_weight(4, 6));
        // A zero half-life is refused by the configuration; never divide by it.
        assert!(hour_weight(1, 0).is_finite());
    }
}
