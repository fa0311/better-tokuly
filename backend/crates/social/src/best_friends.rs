//! Best friends: the worker that turns the interaction log into scores,
//! titles and ranks, and the list built from them.
//!
//! [`crate::graph::record_interaction`] marks a pair dirty. Each run of the
//! worker claims the dirty pairs (plus pairs whose score is due for decay),
//! recomputes them from their day buckets with the pure functions in
//! [`crate::ranking`], and then re-ranks the friends of every user involved.
//! Several worker processes may run at once: pairs are claimed with
//! `SKIP LOCKED` and no lock is held while computing.
use std::collections::{HashMap, HashSet};

use chrono::{NaiveDate, Utc};
use tokuly_core::prelude::*;
use tokuly_proto::social::Friend;

use crate::{
    Social,
    friends::{Detail, FriendRow, friend_columns},
    graph::{self, Pair},
    ranking::{self, Bucket, DayBucket},
};

/// Pairs rescored per batch.
const BATCH: i64 = 500;
/// Upper bound on batches per run, so one run cannot monopolize the worker.
const MAX_BATCHES_PER_RUN: usize = 40;
/// Longest best-friend list returned (ranked friends plus starred ones).
const MAX_LISTED: i64 = 100;

impl Social {
    /// Rescores every pair that is due and re-ranks the users involved.
    /// Returns how many pairs were processed. Run by the ranking worker;
    /// public so tests and operators can trigger a pass.
    pub async fn rescore_pending(&self) -> AppResult<usize> {
        let today = Utc::now().date_naive();
        let mut total = 0;
        for _ in 0..MAX_BATCHES_PER_RUN {
            let processed = self.rescore_batch(today).await?;
            total += processed;
            if (processed as i64) < BATCH {
                break;
            }
        }
        Ok(total)
    }

    async fn rescore_batch(&self, today: NaiveDate) -> AppResult<usize> {
        let db = &self.core.db;
        // Claim first, compute afterwards: an interaction recorded while we
        // compute sets `dirty` again and the pair is simply rescored next time.
        let claimed: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "UPDATE friend_pairs p SET dirty = false, scored_at = now()
             FROM (
                 SELECT user_lo, user_hi FROM friend_pairs
                 WHERE dirty OR (score > 0 AND scored_at < now() - interval '6 hours')
                 ORDER BY scored_at NULLS FIRST
                 LIMIT $1
                 FOR UPDATE SKIP LOCKED
             ) due
             WHERE p.user_lo = due.user_lo AND p.user_hi = due.user_hi
             RETURNING p.user_lo, p.user_hi",
        )
        .bind(BATCH)
        .fetch_all(db)
        .await?;
        if claimed.is_empty() {
            return Ok(0);
        }
        let pairs: Vec<Pair> = claimed.iter().map(|(lo, hi)| Pair { lo: *lo, hi: *hi }).collect();
        let (los, his): (Vec<Uuid>, Vec<Uuid>) = claimed.into_iter().unzip();
        let window_start = ranking::window_start(today);

        let day_rows: Vec<(Uuid, Uuid, NaiveDate, i16, i64, i64)> = sqlx::query_as(
            "SELECT d.user_lo, d.user_hi, d.day, d.kind, d.lo_weight, d.hi_weight
             FROM friend_interaction_days d
             JOIN unnest($1::uuid[], $2::uuid[]) AS due(lo, hi) ON d.user_lo = due.lo AND d.user_hi = due.hi
             WHERE d.day >= $3",
        )
        .bind(&los)
        .bind(&his)
        .bind(window_start)
        .fetch_all(db)
        .await?;
        let mut days: HashMap<Pair, Vec<DayBucket>> = HashMap::new();
        for (lo, hi, day, kind, lo_weight, hi_weight) in day_rows {
            let Some(bucket) = Bucket::from_code(kind) else { continue };
            days.entry(Pair { lo, hi }).or_default().push(DayBucket { age_days: ranking::age_days(day, today), bucket, lo: lo_weight, hi: hi_weight });
        }

        let users: Vec<Uuid> = pairs.iter().flat_map(|p| [p.lo, p.hi]).collect::<HashSet<_>>().into_iter().collect();
        let closest = self.most_time_together(&users, window_start).await?;

        let (mut scores, mut interactions, mut titles) = (Vec::new(), Vec::new(), Vec::new());
        for pair in &pairs {
            let window = ranking::summarize(days.get(pair).map(Vec::as_slice).unwrap_or_default());
            let mutual_top = closest.get(&pair.lo) == Some(&pair.hi) && closest.get(&pair.hi) == Some(&pair.lo);
            scores.push(window.score);
            interactions.push(window.messages_and_stamps.min(i32::MAX as i64) as i32);
            // Arrays of arrays cannot be bound; titles never contain commas.
            titles.push(ranking::window_titles(&window, mutual_top).join(","));
        }
        sqlx::query(
            "UPDATE friend_pairs p SET score = v.score, interaction_score = v.interactions, titles = string_to_array(v.titles, ',')
             FROM unnest($1::uuid[], $2::uuid[], $3::float8[], $4::int4[], $5::text[]) AS v(lo, hi, score, interactions, titles)
             WHERE p.user_lo = v.lo AND p.user_hi = v.hi",
        )
        .bind(&los)
        .bind(&his)
        .bind(&scores)
        .bind(&interactions)
        .bind(&titles)
        .execute(db)
        .await?;

        self.recompute_ranks(&users).await?;
        Ok(pairs.len())
    }

    /// For each of `users`, the current friend they spent the most time with
    /// since `window_start` (users with no time together are absent).
    async fn most_time_together(&self, users: &[Uuid], window_start: NaiveDate) -> AppResult<HashMap<Uuid, Uuid>> {
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT DISTINCT ON (who) who, partner FROM (
                 SELECT d.user_lo AS who, d.user_hi AS partner, sum(d.lo_weight + d.hi_weight) AS minutes
                 FROM friend_interaction_days d
                 WHERE d.kind = $3 AND d.day >= $2 AND d.user_lo = ANY($1)
                   AND EXISTS (SELECT 1 FROM friendships f WHERE f.user_id = d.user_lo AND f.friend_id = d.user_hi)
                 GROUP BY d.user_lo, d.user_hi
                 UNION ALL
                 SELECT d.user_hi, d.user_lo, sum(d.lo_weight + d.hi_weight)
                 FROM friend_interaction_days d
                 WHERE d.kind = $3 AND d.day >= $2 AND d.user_hi = ANY($1)
                   AND EXISTS (SELECT 1 FROM friendships f WHERE f.user_id = d.user_lo AND f.friend_id = d.user_hi)
                 GROUP BY d.user_hi, d.user_lo
             ) together
             ORDER BY who, minutes DESC, partner",
        )
        .bind(users)
        .bind(window_start)
        .bind(Bucket::TimeTogether.code())
        .fetch_all(&self.core.db)
        .await?;
        Ok(rows.into_iter().collect())
    }

    /// Re-ranks the friends of `user_ids` by pair score: the top
    /// `best_friend_count` friends above the minimum score get ranks 1..N,
    /// everyone else rank 0. Only rows whose rank changes are written.
    pub(crate) async fn recompute_ranks(&self, user_ids: &[Uuid]) -> AppResult<()> {
        if user_ids.is_empty() {
            return Ok(());
        }
        sqlx::query(
            "WITH ranked AS (
                 SELECT f.user_id, f.friend_id,
                        row_number() OVER (PARTITION BY f.user_id ORDER BY p.score DESC, f.created_at, f.friend_id) AS position
                 FROM friendships f
                 JOIN friend_pairs p ON p.user_lo = LEAST(f.user_id, f.friend_id) AND p.user_hi = GREATEST(f.user_id, f.friend_id)
                 WHERE f.user_id = ANY($1) AND p.score >= $2
             ),
             target AS (
                 SELECT f.user_id, f.friend_id, COALESCE(r.position, 0)::int AS rank
                 FROM friendships f
                 LEFT JOIN ranked r ON r.user_id = f.user_id AND r.friend_id = f.friend_id AND r.position <= $3
                 WHERE f.user_id = ANY($1)
             )
             UPDATE friendships f SET best_friend_rank = target.rank
             FROM target
             WHERE f.user_id = target.user_id AND f.friend_id = target.friend_id AND f.best_friend_rank <> target.rank",
        )
        .bind(user_ids)
        .bind(ranking::MIN_BEST_FRIEND_SCORE)
        .bind(self.cfg.best_friend_count)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    /// `owner`'s best friends as `viewer` sees them: the automatically ranked
    /// ones in rank order, then the starred ones. Someone else's list is only
    /// shown to their friends.
    pub(crate) async fn list_best_friends(&self, viewer: Uuid, owner: Uuid) -> AppResult<Vec<Friend>> {
        let own = viewer == owner;
        if !own {
            self.require_friend(viewer, owner).await?;
        }
        let hidden = if own { Vec::new() } else { graph::blocked_ids_either(&self.core, viewer).await? };
        let rows: Vec<FriendRow> = sqlx::query_as(concat!(
            "SELECT ",
            friend_columns!(),
            " FROM friendships f JOIN users u ON u.id = f.friend_id
              WHERE f.user_id = $1 AND u.deleted_at IS NULL AND (f.best_friend_rank > 0 OR f.favorite)
                AND NOT (f.friend_id = ANY($2))
              ORDER BY (f.best_friend_rank = 0), f.best_friend_rank, f.created_at, f.friend_id
              LIMIT $3"
        ))
        .bind(owner)
        .bind(&hidden)
        .bind(MAX_LISTED)
        .fetch_all(&self.core.db)
        .await?;
        self.friends_to_proto(rows, if own { Detail::Own } else { Detail::Ranking }).await
    }
}
