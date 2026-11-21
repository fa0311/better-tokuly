//! The numbers of a subject (`subject_stats`, `reaction_emoji_counts`,
//! `share_counts`), and what follows from a change to them.
//!
//! Counters move in the transaction that changes the rows they count, so
//! they cannot drift. After the commit two things happen, both best effort:
//! the owning domain hears about it (`SubjectResolver::counter_changed`, for
//! its own denormalized copies), and everyone watching the subject gets the
//! new totals pushed, at most about once a second per subject.
use std::{collections::HashMap, time::Duration};

use sqlx::PgConnection;
use tokuly_core::prelude::*;
use tokuly_proto::{
    engagement::{CountersChanged, EmojiCount, EngagementEvent, Reaction, ReactionSummary},
    realtime::server_frame::Body,
};

use crate::{Engagement, subjects::SubjectKey};

/// Emoji listed in a `ReactionSummary`.
pub(crate) const SUMMARY_EMOJI: i64 = 20;

/// The quiet time between two pushes of a subject's counters.
const PUSH_INTERVAL: Duration = Duration::from_secs(1);

/// One of the counters of a subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Counter {
    Like,
    Dislike,
    Emoji,
    Comment,
    Bookmark,
    Share,
}

impl Counter {
    /// The name the owning domain hears in `counter_changed`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Like => "like_count",
            Self::Dislike => "dislike_count",
            Self::Emoji => "reaction_count",
            Self::Comment => "comment_count",
            Self::Bookmark => "bookmark_count",
            Self::Share => "share_count",
        }
    }

    fn insert_sql(self) -> &'static str {
        macro_rules! upsert {
            ($column:literal) => {
                concat!(
                    "INSERT INTO subject_stats (subject_type, subject_id, owner_id, ",
                    $column,
                    ") VALUES ($1, $2, $3, GREATEST($4, 0)) ON CONFLICT (subject_type, subject_id) DO UPDATE SET ",
                    $column,
                    " = GREATEST(subject_stats.",
                    $column,
                    " + $4, 0)"
                )
            };
        }
        match self {
            Self::Like => upsert!("like_count"),
            Self::Dislike => upsert!("dislike_count"),
            Self::Emoji => upsert!("emoji_count"),
            Self::Comment => upsert!("comment_count"),
            Self::Bookmark => upsert!("bookmark_count"),
            Self::Share => upsert!("share_count"),
        }
    }

    fn update_sql(self) -> &'static str {
        macro_rules! update {
            ($column:literal) => {
                concat!(
                    "UPDATE subject_stats SET ",
                    $column,
                    " = GREATEST(",
                    $column,
                    " + $3, 0) WHERE subject_type = $1 AND subject_id = $2"
                )
            };
        }
        match self {
            Self::Like => update!("like_count"),
            Self::Dislike => update!("dislike_count"),
            Self::Emoji => update!("emoji_count"),
            Self::Comment => update!("comment_count"),
            Self::Bookmark => update!("bookmark_count"),
            Self::Share => update!("share_count"),
        }
    }
}

/// Moves a counter of a subject. `owner_id` is the subject's owner, written
/// when this is the first number the subject gets; without one (a subject
/// that no longer resolves) only an existing row is changed.
pub(crate) async fn add(
    conn: &mut PgConnection,
    key: SubjectKey,
    owner_id: Option<Uuid>,
    counter: Counter,
    delta: i64,
) -> AppResult<()> {
    if delta == 0 {
        return Ok(());
    }
    match owner_id {
        Some(owner_id) => {
            sqlx::query(counter.insert_sql())
                .bind(key.kind)
                .bind(key.id)
                .bind(owner_id)
                .bind(delta)
                .execute(conn)
                .await?
        }
        None => sqlx::query(counter.update_sql()).bind(key.kind).bind(key.id).bind(delta).execute(conn).await?,
    };
    Ok(())
}

/// Moves the count of one emoji on a subject. Rows that reach zero go away.
pub(crate) async fn add_emoji(conn: &mut PgConnection, key: SubjectKey, emoji: &str, delta: i64) -> AppResult<()> {
    if delta > 0 {
        sqlx::query(
            "INSERT INTO reaction_emoji_counts (subject_type, subject_id, emoji, count) VALUES ($1, $2, $3, $4)
             ON CONFLICT (subject_type, subject_id, emoji) DO UPDATE SET count = reaction_emoji_counts.count + $4",
        )
        .bind(key.kind)
        .bind(key.id)
        .bind(emoji)
        .bind(delta)
        .execute(conn)
        .await?;
    } else if delta < 0 {
        // Two statements: one statement cannot change and delete the same row.
        sqlx::query(
            "UPDATE reaction_emoji_counts SET count = count + $4 WHERE subject_type = $1 AND subject_id = $2 AND emoji = $3",
        )
        .bind(key.kind)
        .bind(key.id)
        .bind(emoji)
        .bind(delta)
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "DELETE FROM reaction_emoji_counts WHERE subject_type = $1 AND subject_id = $2 AND emoji = $3 AND count <= 0",
        )
        .bind(key.kind)
        .bind(key.id)
        .bind(emoji)
        .execute(conn)
        .await?;
    }
    Ok(())
}

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub(crate) struct StatsRow {
    pub subject_type: i32,
    pub subject_id: Uuid,
    pub like_count: i64,
    pub dislike_count: i64,
    pub emoji_count: i64,
    pub comment_count: i64,
    pub bookmark_count: i64,
    pub share_count: i64,
}

#[derive(sqlx::FromRow)]
struct EmojiRow {
    subject_type: i32,
    subject_id: Uuid,
    emoji: String,
    count: i64,
}

/// The numbers of many subjects: counters, and the most used emoji of each.
#[derive(Debug, Default)]
pub(crate) struct Stats {
    rows: HashMap<SubjectKey, StatsRow>,
    emoji: HashMap<SubjectKey, Vec<EmojiCount>>,
}

impl Stats {
    /// The counters of a subject; all zero when it has none yet.
    pub(crate) fn of(&self, key: SubjectKey) -> StatsRow {
        self.rows.get(&key).cloned().unwrap_or_default()
    }

    /// The reaction summary of a subject for a viewer whose own reaction is
    /// `viewer_reaction`.
    pub(crate) fn reactions(&self, key: SubjectKey, viewer_reaction: Option<Reaction>) -> ReactionSummary {
        let row = self.of(key);
        ReactionSummary {
            like_count: row.like_count,
            dislike_count: row.dislike_count,
            emoji_counts: self.emoji.get(&key).cloned().unwrap_or_default(),
            emoji_total: row.emoji_count,
            viewer_reaction: viewer_reaction.into(),
            ..Default::default()
        }
    }
}

fn split(keys: &[SubjectKey]) -> (Vec<i32>, Vec<Uuid>) {
    keys.iter().map(|key| (key.kind, key.id)).unzip()
}

/// Loads the numbers of `keys` in two queries.
pub(crate) async fn load(core: &Core, keys: &[SubjectKey]) -> AppResult<Stats> {
    if keys.is_empty() {
        return Ok(Stats::default());
    }
    let (kinds, ids) = split(keys);
    let rows: Vec<StatsRow> = sqlx::query_as(
        "SELECT subject_type, subject_id, like_count, dislike_count, emoji_count, comment_count, bookmark_count, share_count
         FROM subject_stats
         WHERE (subject_type, subject_id) IN (SELECT * FROM unnest($1::integer[], $2::uuid[]))",
    )
    .bind(&kinds)
    .bind(&ids)
    .fetch_all(&core.db)
    .await?;
    let emoji: Vec<EmojiRow> = sqlx::query_as(
        "SELECT subject_type, subject_id, emoji, count FROM (
             SELECT subject_type, subject_id, emoji, count,
                    row_number() OVER (PARTITION BY subject_type, subject_id ORDER BY count DESC, emoji) AS rank
             FROM reaction_emoji_counts
             WHERE (subject_type, subject_id) IN (SELECT * FROM unnest($1::integer[], $2::uuid[]))
         ) ranked
         WHERE rank <= $3
         ORDER BY subject_type, subject_id, rank",
    )
    .bind(&kinds)
    .bind(&ids)
    .bind(SUMMARY_EMOJI)
    .fetch_all(&core.db)
    .await?;
    let mut stats = Stats::default();
    for row in rows {
        stats.rows.insert(SubjectKey { kind: row.subject_type, id: row.subject_id }, row);
    }
    for row in emoji {
        stats.emoji.entry(SubjectKey { kind: row.subject_type, id: row.subject_id }).or_default().push(EmojiCount {
            emoji: row.emoji,
            count: row.count,
            ..Default::default()
        });
    }
    Ok(stats)
}

fn lead_key(key: SubjectKey) -> String {
    format!("engagement:push:lead:{}:{}", key.kind, key.id)
}

fn trail_key(key: SubjectKey) -> String {
    format!("engagement:push:trail:{}:{}", key.kind, key.id)
}

impl Engagement {
    /// Tells the owning domain that a counter of its subject moved. Never
    /// fails the caller: the change itself is already committed.
    pub(crate) async fn counter_changed(&self, key: SubjectKey, counter: Counter, delta: i64) {
        if delta == 0 {
            return;
        }
        // `Subjects::counter_changed` keeps its registry lock across the
        // resolver's await, which makes its future `!Send` — and with it
        // every RPC handler that awaited it. Driving it on a blocking thread
        // keeps that out of the handlers' futures.
        let core = self.core.clone();
        let runtime = tokio::runtime::Handle::current();
        let called = tokio::task::spawn_blocking(move || {
            runtime.block_on(async move {
                core.subjects.counter_changed(key.subject_type(), key.id, counter.name(), delta).await
            })
        })
        .await;
        match called {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(subject_type = key.kind, subject_id = %key.id, counter = counter.name(), %error, "counter callback failed")
            }
            Err(error) => {
                tracing::warn!(subject_type = key.kind, subject_id = %key.id, counter = counter.name(), %error, "counter callback panicked")
            }
        }
    }

    /// Pushes the subject's totals to its topic — now if nothing was pushed
    /// during the last second, otherwise once that second is over, so a
    /// burst of likes costs one push and the last state always arrives.
    pub(crate) async fn counters_dirty(&self, key: SubjectKey) {
        let cache = &self.core.cache;
        match cache.set_nx(&lead_key(key), "1", PUSH_INTERVAL).await {
            Ok(true) => self.push_counters(key).await,
            Ok(false) => {
                // One trailing push per window. Its task forgets the marker
                // before it reads the totals, so a change committed after
                // that read finds the marker gone and schedules the next.
                if matches!(cache.set_nx(&trail_key(key), "1", PUSH_INTERVAL * 3).await, Ok(true)) {
                    let engagement = self.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(PUSH_INTERVAL).await;
                        let _ = engagement.core.cache.del(&trail_key(key)).await;
                        engagement.push_counters(key).await;
                    });
                }
            }
            Err(error) => tracing::warn!(%error, "counter push throttle unavailable"),
        }
    }

    async fn push_counters(&self, key: SubjectKey) {
        let stats = match load(&self.core, &[key]).await {
            Ok(stats) => stats,
            Err(error) => {
                tracing::warn!(%error, "counter push skipped");
                return;
            }
        };
        let row = stats.of(key);
        let event = EngagementEvent {
            subject: key.to_ref().into(),
            event: Some(
                CountersChanged {
                    reactions: stats.reactions(key, None).into(),
                    comment_count: row.comment_count,
                    bookmark_count: row.bookmark_count,
                    share_count: row.share_count,
                    ..Default::default()
                }
                .into(),
            ),
            ..Default::default()
        };
        self.core.rt.notify_topic(&key.topic(), Body::from(event)).await;
    }
}

#[cfg(test)]
mod tests {
    use tokuly_proto::common::SubjectType;

    use super::*;

    #[test]
    fn every_counter_has_its_own_column_and_name() {
        let all =
            [Counter::Like, Counter::Dislike, Counter::Emoji, Counter::Comment, Counter::Bookmark, Counter::Share];
        let names: std::collections::HashSet<&str> = all.iter().map(|c| c.name()).collect();
        let inserts: std::collections::HashSet<&str> = all.iter().map(|c| c.insert_sql()).collect();
        let updates: std::collections::HashSet<&str> = all.iter().map(|c| c.update_sql()).collect();
        assert_eq!((names.len(), inserts.len(), updates.len()), (6, 6, 6));
        assert!(
            Counter::Comment.insert_sql().contains("comment_count = GREATEST(subject_stats.comment_count + $4, 0)")
        );
        assert!(
            Counter::Share
                .update_sql()
                .starts_with("UPDATE subject_stats SET share_count = GREATEST(share_count + $3, 0)")
        );
    }

    #[test]
    fn a_subject_without_numbers_reads_as_zero() {
        let key = SubjectKey::new(SubjectType::SUBJECT_TYPE_NOTE, Uuid::now_v7());
        let stats = Stats::default();
        assert_eq!(stats.of(key).comment_count, 0);
        let summary = stats.reactions(key, None);
        assert_eq!((summary.like_count, summary.emoji_total, summary.emoji_counts.len()), (0, 0, 0));
        assert!(summary.viewer_reaction.as_option().is_none());
    }
}
