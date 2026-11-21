//! The indexer: copies pending registry rows into Meilisearch.
//!
//! One pass ([`Search::flush_once`]):
//!
//! 1. take the indexer lock (a PostgreSQL advisory lock held for the pass),
//!    so a single process writes to the engine at any time;
//! 2. read a batch of pending rows, oldest first, with their owner's state;
//! 3. per kind, send one "add or replace" task for the live documents and
//!    one "delete" task for the rest (removed documents and documents whose
//!    owner is suspended or deleted);
//! 4. wait for the engine to finish those tasks (task API);
//! 5. mark the rows as written — at the generation that was read, so a row
//!    that changed in the meantime stays pending.
//!
//! The lock is what keeps the order per document: the registry always holds
//! the newest state, and with one writer the engine receives states in the
//! order they were read. Rows are not locked, so the consumer is never
//! blocked by a pass. A failed pass changes nothing and is simply repeated;
//! writing a document twice is harmless.
use std::{collections::BTreeMap, time::Duration};

use sqlx::{Postgres, Transaction};
use tokuly_core::prelude::*;

use crate::{
    Search,
    document::IndexDoc,
    indexes::{TaskEnd, engine, engine_error, wait_for_task},
    kinds::{self, Kind},
};

/// How long a pass waits for the engine to work through its tasks. A task
/// that is still queued after this is counted as written: the engine's queue
/// is durable, and sending the same batch again every pass would only make a
/// backlog worse.
const WRITE_TASK_TIMEOUT: Duration = Duration::from_secs(120);

/// What one or several passes wrote.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FlushReport {
    /// Documents added to or replaced in an index.
    pub upserted: usize,
    /// Documents taken out of an index.
    pub removed: usize,
}

impl FlushReport {
    pub fn total(&self) -> usize {
        self.upserted + self.removed
    }
}

#[derive(sqlx::FromRow)]
struct PendingRow {
    kind: String,
    doc_id: Uuid,
    generation: i64,
    doc: Option<serde_json::Value>,
    owner_inactive: bool,
}

#[derive(Default)]
struct KindBatch {
    upserts: Vec<serde_json::Value>,
    removals: Vec<String>,
}

impl Search {
    fn indexer_lock_key(&self) -> String {
        format!("search:indexer:{}", self.cfg.index_prefix)
    }

    /// Takes the indexer lock for the transaction. With `wait` it queues
    /// behind the current holder; without, `false` means someone else is
    /// indexing right now.
    async fn lock_indexer(&self, tx: &mut Transaction<'_, Postgres>, wait: bool) -> AppResult<bool> {
        if wait {
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))").bind(self.indexer_lock_key()).execute(&mut **tx).await?;
            return Ok(true);
        }
        Ok(sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(self.indexer_lock_key())
            .fetch_one(&mut **tx)
            .await?)
    }

    /// One indexer pass over at most `TOKULY_SEARCH_INDEX_BATCH` pending
    /// rows. `None` when another process holds the indexer lock.
    pub async fn flush_once(&self) -> AppResult<Option<FlushReport>> {
        self.flush_pass(false).await
    }

    /// Writes everything pending and returns when the engine has applied it
    /// (waiting for another process's pass if one is running). Search results
    /// reflect every change stored before the call.
    pub async fn flush(&self) -> AppResult<FlushReport> {
        let mut total = FlushReport::default();
        loop {
            let pass = self.flush_pass(true).await?.unwrap_or_default();
            if pass.total() == 0 {
                return Ok(total);
            }
            total.upserted += pass.upserted;
            total.removed += pass.removed;
        }
    }

    async fn flush_pass(&self, wait_for_lock: bool) -> AppResult<Option<FlushReport>> {
        let mut tx = self.core.db.begin().await?;
        if !self.lock_indexer(&mut tx, wait_for_lock).await? {
            return Ok(None);
        }
        // Documents of suspended and deleted owners are held in the registry
        // but must not be searchable. An owner unknown to identity (the nil
        // UUID of official content) counts as active.
        let rows: Vec<PendingRow> = sqlx::query_as(
            "SELECT d.kind, d.doc_id, d.generation, d.doc,
                    COALESCE(u.deleted_at IS NOT NULL OR u.banned_at IS NOT NULL, false) AS owner_inactive
             FROM search_documents d
             LEFT JOIN users u ON u.id = d.owner_id
             WHERE d.synced_generation < d.generation
             ORDER BY d.queued_at
             LIMIT $1",
        )
        .bind(self.cfg.index_batch as i64)
        .fetch_all(&mut *tx)
        .await?;
        if rows.is_empty() {
            return Ok(Some(FlushReport::default()));
        }

        let mut batches: BTreeMap<&'static str, (&'static Kind, KindBatch)> = BTreeMap::new();
        for row in &rows {
            // The registry only stores known kinds; a row of a kind that was
            // since retired has no index and is simply marked as written.
            let Some(kind) = kinds::by_name(&row.kind) else { continue };
            let batch = &mut batches.entry(kind.name).or_insert_with(|| (kind, KindBatch::default())).1;
            let live = match (&row.doc, row.owner_inactive) {
                (Some(doc), false) => serde_json::from_value::<IndexDoc>(doc.clone()).ok(),
                _ => None,
            };
            match live {
                Some(doc) => batch.upserts.push(doc.to_index_json(kind)),
                None => batch.removals.push(row.doc_id.to_string()),
            }
        }

        // Queue everything first, then wait: the engine works on different
        // indexes concurrently.
        let mut report = FlushReport::default();
        let mut tasks = Vec::new();
        for (kind, batch) in batches.values() {
            let index = self.index(kind);
            if !batch.upserts.is_empty() {
                tasks.push(engine("writing documents", index.add_or_replace(&batch.upserts, Some("id"))).await?);
                report.upserted += batch.upserts.len();
            }
            if !batch.removals.is_empty() {
                tasks.push(engine("deleting documents", index.delete_documents(&batch.removals)).await?);
                report.removed += batch.removals.len();
            }
        }
        for task in &tasks {
            match wait_for_task(self, task, WRITE_TASK_TIMEOUT).await? {
                TaskEnd::Succeeded => {}
                TaskEnd::Failed(error) => return Err(engine_error("an indexing task failed", error.into())),
                TaskEnd::Unfinished => {
                    tracing::warn!(task = task.task_uid, "search engine is behind; an indexing task is still queued");
                }
            }
        }

        let kinds: Vec<&str> = rows.iter().map(|row| row.kind.as_str()).collect();
        let ids: Vec<Uuid> = rows.iter().map(|row| row.doc_id).collect();
        let generations: Vec<i64> = rows.iter().map(|row| row.generation).collect();
        sqlx::query(
            "UPDATE search_documents d SET synced_generation = v.generation
             FROM unnest($1::text[], $2::uuid[], $3::bigint[]) AS v(kind, doc_id, generation)
             WHERE d.kind = v.kind AND d.doc_id = v.doc_id AND d.synced_generation < v.generation",
        )
        .bind(&kinds)
        .bind(&ids)
        .bind(&generations)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(report))
    }

    /// The indexer loop of a worker process: passes back to back while there
    /// is work, otherwise a look every `TOKULY_SEARCH_INDEX_INTERVAL_MS` or
    /// as soon as this process stores a change.
    pub(crate) async fn run_indexer(&self) {
        let idle = Duration::from_millis(self.cfg.index_interval_ms);
        let mut failures: u32 = 0;
        tracing::info!("search indexer started");
        loop {
            let pause = match self.flush_once().await {
                Ok(Some(report)) if report.total() >= self.cfg.index_batch as usize => {
                    failures = 0;
                    Duration::ZERO
                }
                Ok(Some(report)) => {
                    failures = 0;
                    if report.total() > 0 {
                        tracing::debug!(upserted = report.upserted, removed = report.removed, "search index updated");
                    }
                    idle
                }
                Ok(None) => idle,
                Err(error) => {
                    failures = failures.saturating_add(1);
                    tracing::error!(%error, failures, "search indexing pass failed; will retry");
                    Duration::from_secs((1u64 << failures.min(5)).min(30))
                }
            };
            tokio::select! {
                _ = self.core.shutdown.cancelled() => break,
                _ = tokio::time::sleep(pause) => {}
                _ = self.indexer_wake.notified(), if failures == 0 && !pause.is_zero() => {}
            }
        }
        tracing::info!("search indexer stopped");
    }
}
