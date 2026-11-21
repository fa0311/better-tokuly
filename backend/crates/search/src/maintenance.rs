//! Background work of the search domain: the indexer, the event consumer,
//! jobs, the trending timer, the retention sweep, account deletion and data
//! export.
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokuly_core::{
    events::{self, ConsumeOptions},
    export::DataExporter,
    jobs::JobOptions,
    prelude::*,
};

use crate::{
    Search,
    consumer::{CONSUMER_NAME, TOPICS},
    users::{REINDEX_USERS_JOB, ReindexUsersJob},
};

/// Job that applies the stored synonym groups to every index.
pub const APPLY_SYNONYMS_JOB: &str = "search.synonyms.apply";

const SWEEP_INTERVAL: Duration = Duration::from_secs(3600);
/// Rows deleted per statement, so a backlog never holds one long lock.
const SWEEP_BATCH: i64 = 5000;
/// Tombstones are kept a day longer than the event stream keeps events
/// (7 days): after that no older upsert can arrive any more.
const TOMBSTONE_DAYS: i32 = 8;
/// A popular query that stopped qualifying is offered as a completion for
/// this long.
const POPULAR_QUERY_DAYS: i32 = 30;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApplySynonymsJob {}

/// What one retention sweep removed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SweepReport {
    pub tombstones: u64,
    pub popular_queries: u64,
}

impl Search {
    /// Starts the search workers (worker processes only):
    /// index setup followed by the indexer, the event consumer, the
    /// `search.reindex.users` and `search.synonyms.apply` job runners, the
    /// trending timer and the hourly retention sweep.
    pub fn spawn_workers(&self) {
        // The indexer starts once the indexes exist with their settings.
        let search = self.clone();
        tokio::spawn(async move {
            let mut attempt: u32 = 0;
            loop {
                match search.ensure_indexes().await {
                    Ok(()) => break,
                    Err(error) => {
                        attempt = attempt.saturating_add(1);
                        tracing::error!(%error, attempt, "search index setup failed; will retry");
                        let pause = Duration::from_secs((1u64 << attempt.min(5)).min(30));
                        tokio::select! {
                            _ = search.core.shutdown.cancelled() => return,
                            _ = tokio::time::sleep(pause) => {}
                        }
                    }
                }
            }
            search.run_indexer().await;
        });

        let search = self.clone();
        tokio::spawn(async move {
            let handler = search.clone();
            events::consume(
                search.core.bus.clone(),
                CONSUMER_NAME,
                TOPICS,
                ConsumeOptions::default(),
                search.core.shutdown.clone(),
                move |event| {
                    let search = handler.clone();
                    async move { search.handle_event(event).await }
                },
            )
            .await;
        });

        let search = self.clone();
        tokio::spawn(async move {
            let handler = search.clone();
            search
                .core
                .jobs
                .run(
                    REINDEX_USERS_JOB,
                    JobOptions { concurrency: 1, lease: Duration::from_secs(120), max_deliver: 3 },
                    search.core.shutdown.clone(),
                    move |_: ReindexUsersJob| {
                        let search = handler.clone();
                        async move {
                            let report = search.reindex_users().await?;
                            tracing::info!(?report, "users reindexed");
                            Ok(())
                        }
                    },
                )
                .await;
        });

        let search = self.clone();
        tokio::spawn(async move {
            let handler = search.clone();
            search
                .core
                .jobs
                .run(
                    APPLY_SYNONYMS_JOB,
                    JobOptions { concurrency: 1, lease: Duration::from_secs(120), max_deliver: 5 },
                    search.core.shutdown.clone(),
                    move |_: ApplySynonymsJob| {
                        let search = handler.clone();
                        // Index setup compares the stored synonyms with what
                        // each index has and changes only what differs.
                        async move { search.ensure_indexes().await }
                    },
                )
                .await;
        });

        let search = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(search.cfg.trending_refresh_seconds));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = search.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        if let Err(error) = search.refresh_trending().await {
                            tracing::error!(%error, "trending refresh failed");
                        }
                    }
                }
            }
        });

        let search = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(SWEEP_INTERVAL);
            loop {
                tokio::select! {
                    _ = search.core.shutdown.cancelled() => break,
                    _ = tick.tick() => match search.run_retention().await {
                        Ok(report) if report == SweepReport::default() => {}
                        Ok(report) => tracing::info!(?report, "search retention sweep"),
                        Err(error) => tracing::error!(%error, "search retention sweep failed"),
                    },
                }
            }
        });
    }

    /// Removes what is past its keep-by date: tombstones no older event can
    /// reach any more, and popular queries nobody has searched in a month.
    pub async fn run_retention(&self) -> AppResult<SweepReport> {
        let mut report = SweepReport::default();
        loop {
            let done = sqlx::query(
                "DELETE FROM search_documents WHERE (kind, doc_id) IN (
                     SELECT kind, doc_id FROM search_documents
                     WHERE doc IS NULL AND synced_generation >= generation
                       AND updated_at < now() - make_interval(days => $1)
                     LIMIT $2
                 )",
            )
            .bind(TOMBSTONE_DAYS)
            .bind(SWEEP_BATCH)
            .execute(&self.core.db)
            .await?;
            report.tombstones += done.rows_affected();
            if done.rows_affected() < SWEEP_BATCH as u64 {
                break;
            }
        }
        report.popular_queries = self.sweep_popular_queries(POPULAR_QUERY_DAYS).await?;
        Ok(report)
    }

    /// Handles `identity.user.deleted`: everything the user owns leaves the
    /// indexes for good (their own document included), and their search
    /// history and settings are deleted. `version` is the event's ID.
    pub async fn purge_user(&self, version: Uuid, user_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        Search::remove_owner_documents(&mut *tx, version, user_id).await?;
        sqlx::query("DELETE FROM search_recent WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM search_user_settings WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
        // The word lists stay; who entered them is forgotten.
        sqlx::query("UPDATE search_blocked_terms SET added_by = NULL WHERE added_by = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE search_synonym_groups SET updated_by = NULL WHERE updated_by = $1").bind(user_id).execute(&mut *tx).await?;
        tx.commit().await?;
        self.indexer_wake.notify_one();
        Ok(())
    }
}

/// The search section of a personal data export.
pub struct SearchExporter {
    pub core: Core,
}

#[async_trait]
impl DataExporter for SearchExporter {
    fn name(&self) -> &'static str {
        "search"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        let recent: Vec<(String, String, Time)> =
            sqlx::query_as("SELECT query, kind, searched_at FROM search_recent WHERE user_id = $1 ORDER BY searched_at DESC, id DESC")
                .bind(user_id)
                .fetch_all(&self.core.db)
                .await?;
        let paused: Option<bool> = sqlx::query_scalar("SELECT history_paused FROM search_user_settings WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&self.core.db)
            .await?;
        Ok(serde_json::json!({
            "history_paused": paused.unwrap_or(false),
            "recent_searches": recent
                .into_iter()
                .map(|(query, kind, searched_at)| serde_json::json!({
                    "query": query,
                    "kind": if kind.is_empty() { "all" } else { kind.as_str() },
                    "searched_at": searched_at.to_rfc3339(),
                }))
                .collect::<Vec<_>>(),
        }))
    }
}
