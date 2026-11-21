//! Server stats for the operations dashboard: accounts, content volume, the
//! moderation queue, event/job backlogs and dependency health.
use std::{
    collections::HashMap,
    future::Future,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use futures::StreamExt as _;
use tokuly_core::{
    bus::{ConnectionState, STREAM_ANALYTICS, STREAM_EVENTS, STREAM_JOBS, STREAM_LOCATIONS},
    prelude::*,
};
use tokuly_proto::admin::{ConsumerStats, ContentCount, DependencyStatus, GetServerStatsResponse, ModerationStats, QueueStats, StreamStats, UserStats};

use crate::Admin;

/// How long a dependency probe or the JetStream listing may take before it
/// is reported as failed; the dashboard must stay responsive during outages.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Tables larger than this are reported from the planner's row estimate
/// instead of an exact (and slow) `count(*)`.
const EXACT_COUNT_LIMIT: i64 = 1_000_000;

/// Content worth a number on the dashboard, in display order: the key
/// reported to clients and the table it lives in. Other domains own these
/// tables, and not every deployment (or test database) has all of them, so
/// each is counted only if it exists. Where a domain might reasonably have
/// prefixed its table, both spellings are listed and the first that exists
/// is used.
const CONTENT_TABLES: &[(&str, &[&str])] = &[
    ("friendships", &["friendships"]),
    ("conversations", &["conversations", "chat_conversations"]),
    ("chat_messages", &["chat_messages", "messages"]),
    ("calls", &["calls", "chat_calls"]),
    ("notes", &["notes"]),
    ("stories", &["stories", "note_stories"]),
    ("channels", &["channels", "video_channels"]),
    ("videos", &["videos"]),
    ("playlists", &["playlists", "video_playlists"]),
    ("watch_parties", &["watch_parties", "video_watch_parties"]),
    ("live_streams", &["live_streams", "streams"]),
    ("live_chat_messages", &["live_chat_messages", "live_messages"]),
    ("live_gifts", &["live_gifts", "gifts"]),
    ("clips", &["clips", "live_clips"]),
    ("shorts", &["shorts"]),
    ("comments", &["comments", "engagement_comments"]),
    ("reactions", &["reactions", "engagement_reactions"]),
    ("reports", &["reports", "engagement_reports"]),
    ("stamp_packs", &["stamp_packs"]),
    ("stamps", &["stamps"]),
    ("media_assets", &["media_assets"]),
    ("places", &["places", "geo_places"]),
    ("visits", &["visits", "geo_visits"]),
    ("notifications", &["notifications"]),
    ("activities", &["activities", "feed_activities", "activity_log"]),
    ("news_articles", &["news_articles", "feed_news_articles"]),
    ("wallet_entries", &["wallet_entries"]),
];

const STREAMS: &[&str] = &[STREAM_EVENTS, STREAM_JOBS, STREAM_ANALYTICS, STREAM_LOCATIONS];

/// Runs one health probe with a timeout and reports how it went.
async fn probe<F>(name: &str, check: F) -> DependencyStatus
where
    F: Future<Output = Result<(), String>>,
{
    let started = Instant::now();
    let (reachable, detail) = match tokio::time::timeout(PROBE_TIMEOUT, check).await {
        Ok(Ok(())) => (true, String::new()),
        Ok(Err(error)) => (false, error),
        Err(_) => (false, format!("no answer within {} s", PROBE_TIMEOUT.as_secs())),
    };
    DependencyStatus { name: name.to_owned(), reachable, latency_ms: started.elapsed().as_millis() as i64, detail, ..Default::default() }
}

impl Admin {
    /// A snapshot of the deployment for `SystemService.GetServerStats`.
    pub(crate) async fn server_stats(&self) -> AppResult<GetServerStatsResponse> {
        let (users, content, moderation, (outbox_pending, outbox_oldest_age_seconds)) =
            tokio::try_join!(self.user_stats(), self.content_counts(), self.moderation_stats(), self.outbox_backlog())?;
        let (streams, dependencies) = tokio::join!(self.stream_stats(), self.dependency_statuses());
        Ok(GetServerStatsResponse {
            users: users.into(),
            content,
            moderation: moderation.into(),
            queues: QueueStats { outbox_pending, outbox_oldest_age_seconds, streams, ..Default::default() }.into(),
            dependencies,
            generated_at: ts(Utc::now()),
            ..Default::default()
        })
    }

    async fn user_stats(&self) -> AppResult<UserStats> {
        let zone = &self.cfg.stats_time_zone;
        let (total, new_today, new_7d, suspended, deletion_scheduled, verified): (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT count(*),
                    count(*) FILTER (WHERE created_at >= date_trunc('day', now(), $1)),
                    count(*) FILTER (WHERE created_at >= now() - interval '7 days'),
                    count(*) FILTER (WHERE banned_at IS NOT NULL),
                    count(*) FILTER (WHERE deletion_scheduled_at IS NOT NULL),
                    count(*) FILTER (WHERE verified)
             FROM users WHERE deleted_at IS NULL",
        )
        .bind(zone)
        .fetch_one(&self.core.db)
        .await?;
        let (active_sessions, active_today, active_7d, active_30d): (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT count(*) FILTER (WHERE revoked_at IS NULL AND expires_at > now()),
                    count(DISTINCT user_id) FILTER (WHERE last_active_at >= date_trunc('day', now(), $1)),
                    count(DISTINCT user_id) FILTER (WHERE last_active_at >= now() - interval '7 days'),
                    count(DISTINCT user_id) FILTER (WHERE last_active_at >= now() - interval '30 days')
             FROM sessions",
        )
        .bind(zone)
        .fetch_one(&self.core.db)
        .await?;
        Ok(UserStats {
            total,
            new_today,
            new_7d,
            active_sessions,
            active_today,
            active_7d,
            active_30d,
            suspended,
            deletion_scheduled,
            verified,
            ..Default::default()
        })
    }

    /// Row counts of the content tables that exist in this database.
    async fn content_counts(&self) -> AppResult<Vec<ContentCount>> {
        let candidates: Vec<&str> = CONTENT_TABLES.iter().flat_map(|(_, tables)| tables.iter().copied()).collect();
        // `to_regclass` yields NULL for a missing table, which the join drops.
        // `reltuples` is the planner's row estimate (-1 before the first ANALYZE).
        let existing: HashMap<String, i64> = sqlx::query_as::<_, (String, i64)>(
            "SELECT t.name, greatest(c.reltuples, 0)::bigint
             FROM unnest($1::text[]) AS t(name)
             JOIN pg_class c ON c.oid = to_regclass('public.' || t.name)",
        )
        .bind(&candidates)
        .fetch_all(&self.core.db)
        .await?
        .into_iter()
        .collect();

        let mut counts = Vec::new();
        for (key, tables) in CONTENT_TABLES {
            // `table` comes from our own list, never from database output, so
            // it is safe to interpolate below.
            let Some((table, estimate)) = tables.iter().find_map(|table| existing.get(*table).map(|estimate| (*table, *estimate))) else {
                continue;
            };
            let (count, approximate) = if estimate > EXACT_COUNT_LIMIT {
                (estimate, true)
            } else {
                let exact: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM public.{table}"))).fetch_one(&self.core.db).await?;
                (exact, false)
            };
            counts.push(ContentCount { name: (*key).to_owned(), count, approximate, ..Default::default() });
        }
        Ok(counts)
    }

    async fn moderation_stats(&self) -> AppResult<ModerationStats> {
        let (open_cases, unassigned_cases, resolved_7d, oldest_open): (i64, i64, i64, Option<DateTime<Utc>>) = sqlx::query_as(
            "SELECT count(*) FILTER (WHERE status = 'open'),
                    count(*) FILTER (WHERE status = 'open' AND assignee_id IS NULL),
                    count(*) FILTER (WHERE status = 'resolved' AND resolved_at >= now() - interval '7 days'),
                    min(created_at) FILTER (WHERE status = 'open')
             FROM moderation_cases",
        )
        .fetch_one(&self.core.db)
        .await?;
        let reports_24h: i64 =
            sqlx::query_scalar("SELECT count(*) FROM moderation_reports WHERE created_at >= now() - interval '24 hours'").fetch_one(&self.core.db).await?;
        Ok(ModerationStats { open_cases, unassigned_cases, reports_24h, resolved_7d, oldest_open_case_at: opt_ts(oldest_open), ..Default::default() })
    }

    /// Events waiting in the outbox and the age of the oldest, in seconds.
    async fn outbox_backlog(&self) -> AppResult<(i64, i64)> {
        Ok(sqlx::query_as("SELECT count(*), COALESCE(extract(epoch FROM now() - min(occurred_at)), 0)::bigint FROM outbox").fetch_one(&self.core.db).await?)
    }

    /// JetStream streams with their consumers' lag. Streams that cannot be
    /// read (NATS down) are left out; the `nats` dependency entry says why.
    async fn stream_stats(&self) -> Vec<StreamStats> {
        let listing = async {
            let mut streams = Vec::with_capacity(STREAMS.len());
            for name in STREAMS {
                match self.one_stream(name).await {
                    Ok(stats) => streams.push(stats),
                    Err(error) => {
                        tracing::debug!(stream = name, %error, "stream stats unavailable")
                    }
                }
            }
            streams
        };
        tokio::time::timeout(PROBE_TIMEOUT, listing).await.unwrap_or_default()
    }

    async fn one_stream(&self, name: &str) -> Result<StreamStats, String> {
        let stream = self.core.bus.js.get_stream(self.core.bus.stream_name(name)).await.map_err(|e| e.to_string())?;
        let state = &stream.cached_info().state;
        let mut consumers = Vec::new();
        let mut listing = std::pin::pin!(stream.consumers());
        while let Some(consumer) = listing.next().await {
            let info = consumer.map_err(|e| e.to_string())?;
            consumers.push(ConsumerStats {
                name: info.name,
                pending: info.num_pending as i64,
                ack_pending: info.num_ack_pending as i64,
                redelivered: info.num_redelivered as i64,
                ..Default::default()
            });
        }
        consumers.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(StreamStats { name: name.to_owned(), messages: state.messages as i64, bytes: state.bytes as i64, consumers, ..Default::default() })
    }

    /// Probes every backing service concurrently.
    async fn dependency_statuses(&self) -> Vec<DependencyStatus> {
        let core = &self.core;
        let postgres = probe("postgres", async { sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&core.db).await.map(drop).map_err(|e| e.to_string()) });
        let valkey = probe("valkey", async { core.cache.ping().await.map_err(|e| e.to_string()) });
        let nats = probe("nats", async {
            match core.bus.client.connection_state() {
                ConnectionState::Connected => core.bus.client.flush().await.map_err(|e| e.to_string()),
                state => Err(format!("connection is {state}")),
            }
        });
        let clickhouse = probe("clickhouse", async { core.analytics.ch.query("SELECT 1").execute().await.map_err(|e| e.to_string()) });
        let meilisearch = probe("meilisearch", async {
            match core.search.health().await {
                Ok(health) if health.status == "available" => Ok(()),
                Ok(health) => Err(format!("status is {}", health.status)),
                Err(e) => Err(e.to_string()),
            }
        });
        let (postgres, valkey, nats, clickhouse, meilisearch) = tokio::join!(postgres, valkey, nats, clickhouse, meilisearch);
        vec![postgres, valkey, nats, clickhouse, meilisearch]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn probe_reports_success_failure_and_latency() {
        let up = probe("up", async { Ok(()) }).await;
        assert!(up.reachable && up.detail.is_empty() && up.latency_ms >= 0);
        let down = probe("down", async { Err("connection refused".to_owned()) }).await;
        assert!(!down.reachable);
        assert_eq!((down.name.as_str(), down.detail.as_str()), ("down", "connection refused"));
    }

    #[tokio::test(start_paused = true)]
    async fn probe_gives_up_after_the_timeout() {
        let stuck = probe("stuck", std::future::pending()).await;
        assert!(!stuck.reachable);
        assert!(stuck.detail.contains("no answer"), "{}", stuck.detail);
    }

    #[test]
    fn content_tables_are_plain_identifiers_with_unique_keys() {
        // Table names are interpolated into SQL, so nothing but identifiers may ever be listed.
        let mut keys = Vec::new();
        for (key, tables) in CONTENT_TABLES {
            assert!(!tables.is_empty(), "{key} names no table");
            for table in *tables {
                assert!(table.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'), "{table}");
            }
            assert!(!keys.contains(key), "{key} is listed twice");
            keys.push(key);
        }
    }
}
