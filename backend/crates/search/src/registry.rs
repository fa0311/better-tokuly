//! The document registry (`search_documents`): what the indexes should hold.
//!
//! Every change carries a version, the ID of the event that caused it. IDs
//! are UUIDv7, so comparing them compares time; a change is applied only if
//! its version is newer than the stored one. That single rule makes the
//! consumer idempotent (a redelivery has the stored version) and keeps the
//! order per document (a delete leaves a tombstone, so an older upsert
//! arriving later is recognized and dropped).
//!
//! A row is pending when `generation > synced_generation`; the indexer
//! ([`crate::indexer`]) writes pending rows to Meilisearch. Storing a
//! document identical to what is already there does not make it pending.
use sqlx::PgExecutor;
use tokuly_core::{outbound::SearchChange, prelude::*};

use crate::{Search, document::IndexDoc, kinds};

/// Applies only when the incoming version is newer. A changed document
/// becomes pending; an identical one just takes the newer version.
const STORE_SQL: &str = "
    INSERT INTO search_documents (kind, doc_id, version, owner_id, doc)
    SELECT $1, d.doc_id, $2, d.owner_id, d.doc
    FROM unnest($3::uuid[], $4::uuid[], $5::jsonb[]) AS d(doc_id, owner_id, doc)
    ON CONFLICT (kind, doc_id) DO UPDATE SET
        version = EXCLUDED.version,
        owner_id = EXCLUDED.owner_id,
        doc = EXCLUDED.doc,
        generation = search_documents.generation + (search_documents.doc IS DISTINCT FROM EXCLUDED.doc)::int,
        queued_at = CASE WHEN search_documents.doc IS DISTINCT FROM EXCLUDED.doc THEN now() ELSE search_documents.queued_at END,
        updated_at = CASE WHEN search_documents.doc IS DISTINCT FROM EXCLUDED.doc THEN now() ELSE search_documents.updated_at END
    WHERE search_documents.version < EXCLUDED.version";

/// A removal of a document the registry never held is stored as a tombstone
/// that needs no write to the engine (`synced_generation = generation`).
const REMOVE_SQL: &str = "
    INSERT INTO search_documents (kind, doc_id, version, generation, synced_generation)
    VALUES ($1, $2, $3, 1, 1)
    ON CONFLICT (kind, doc_id) DO UPDATE SET
        version = EXCLUDED.version,
        owner_id = NULL,
        doc = NULL,
        generation = search_documents.generation + (search_documents.doc IS NOT NULL)::int,
        queued_at = CASE WHEN search_documents.doc IS NOT NULL THEN now() ELSE search_documents.queued_at END,
        updated_at = CASE WHEN search_documents.doc IS NOT NULL THEN now() ELSE search_documents.updated_at END
    WHERE search_documents.version < EXCLUDED.version";

/// Whether a change was applied or was older than what the registry holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    Stored,
    /// The registry already reflects this change or a newer one.
    Stale,
    /// The change was malformed (unknown kind, mismatched IDs) and dropped.
    Rejected,
}

impl Search {
    /// Applies one `search.changed` payload. `version` is the ID of the
    /// event that carried it.
    pub async fn apply_change(&self, version: Uuid, change: SearchChange) -> AppResult<Applied> {
        let Some(kind) = kinds::by_name(&change.kind) else {
            tracing::warn!(kind = %change.kind, id = %change.id, "search change for an unknown kind dropped");
            return Ok(Applied::Rejected);
        };
        let stored = match change.doc {
            Some(doc) if doc.kind != change.kind || doc.id != change.id => {
                tracing::warn!(kind = %change.kind, id = %change.id, "search change whose document does not match it dropped");
                return Ok(Applied::Rejected);
            }
            Some(doc) if doc.public => self.store_document(version, IndexDoc::from_search_doc(doc)).await?,
            _ => self.remove_document(version, kind.name, change.id).await?,
        };
        Ok(if stored { Applied::Stored } else { Applied::Stale })
    }

    /// Stores a document unless the registry holds a newer version of it.
    /// Returns whether it was stored.
    pub async fn store_document(&self, version: Uuid, doc: IndexDoc) -> AppResult<bool> {
        let kind = doc.kind.clone();
        Ok(self.store_documents(version, &kind, std::slice::from_ref(&doc)).await? > 0)
    }

    /// Stores documents of one kind under one version; returns how many were
    /// newer than what the registry held.
    pub(crate) async fn store_documents(&self, version: Uuid, kind: &str, docs: &[IndexDoc]) -> AppResult<u64> {
        if docs.is_empty() {
            return Ok(0);
        }
        debug_assert!(docs.iter().all(|doc| doc.kind == kind));
        let ids: Vec<Uuid> = docs.iter().map(|doc| doc.id).collect();
        let owners: Vec<Option<Uuid>> = docs.iter().map(|doc| doc.owner_id).collect();
        let bodies: Vec<serde_json::Value> = docs
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<_, _>>()
            .map_err(|e| AppError::internal(anyhow::anyhow!("search document encode: {e}")))?;
        let done = sqlx::query(STORE_SQL)
            .bind(kind)
            .bind(version)
            .bind(&ids)
            .bind(&owners)
            .bind(&bodies)
            .execute(&self.core.db)
            .await?;
        if done.rows_affected() > 0 {
            self.indexer_wake.notify_one();
        }
        Ok(done.rows_affected())
    }

    /// Removes a document unless the registry holds a newer version of it.
    /// Returns whether the removal was recorded.
    pub async fn remove_document(&self, version: Uuid, kind: &str, id: Uuid) -> AppResult<bool> {
        let done = sqlx::query(REMOVE_SQL).bind(kind).bind(id).bind(version).execute(&self.core.db).await?;
        if done.rows_affected() > 0 {
            self.indexer_wake.notify_one();
        }
        Ok(done.rows_affected() > 0)
    }

    /// Removes those of `ids` the registry holds a live, older document for.
    /// Unlike [`Self::remove_document`] it leaves no tombstone for unknown
    /// IDs: it is for documents derived from current state (users), where an
    /// older event cannot bring stale content back.
    pub(crate) async fn remove_existing(&self, version: Uuid, kind: &str, ids: &[Uuid]) -> AppResult<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let done = sqlx::query(
            "UPDATE search_documents
             SET version = $2, owner_id = NULL, doc = NULL, generation = generation + 1, queued_at = now(), updated_at = now()
             WHERE kind = $1 AND doc_id = ANY($3) AND doc IS NOT NULL AND version < $2",
        )
        .bind(kind)
        .bind(version)
        .bind(ids)
        .execute(&self.core.db)
        .await?;
        if done.rows_affected() > 0 {
            self.indexer_wake.notify_one();
        }
        Ok(done.rows_affected())
    }

    /// Has every live document of an owner written to the index again. The
    /// indexer leaves out documents of suspended owners, so this is what
    /// hides them on a ban and brings them back when it is lifted.
    pub(crate) async fn requeue_owner(&self, owner_id: Uuid) -> AppResult<u64> {
        let done = sqlx::query(
            "UPDATE search_documents SET generation = generation + 1, queued_at = now()
             WHERE owner_id = $1 AND doc IS NOT NULL",
        )
        .bind(owner_id)
        .execute(&self.core.db)
        .await?;
        if done.rows_affected() > 0 {
            self.indexer_wake.notify_one();
        }
        Ok(done.rows_affected())
    }

    /// Removes every document of an owner (account deletion). Their versions
    /// move up to `version`, so nothing published before the deletion can
    /// return.
    pub(crate) async fn remove_owner_documents<'e, E: PgExecutor<'e>>(executor: E, version: Uuid, owner_id: Uuid) -> AppResult<u64> {
        let done = sqlx::query(
            "UPDATE search_documents
             SET version = GREATEST(version, $2), owner_id = NULL, doc = NULL,
                 generation = generation + 1, queued_at = now(), updated_at = now()
             WHERE owner_id = $1 AND doc IS NOT NULL",
        )
        .bind(owner_id)
        .bind(version)
        .execute(executor)
        .await?;
        Ok(done.rows_affected())
    }

    /// Has every live document of a kind written to its index again.
    pub(crate) async fn requeue_kind<'e, E: PgExecutor<'e>>(executor: E, kind: &str) -> AppResult<u64> {
        let done = sqlx::query(
            "UPDATE search_documents SET generation = generation + 1, queued_at = now()
             WHERE kind = $1 AND doc IS NOT NULL",
        )
        .bind(kind)
        .execute(executor)
        .await?;
        Ok(done.rows_affected())
    }

    /// Has one row written to the index again whatever its state, so a
    /// removal reaches the engine even if the registry never held the
    /// document. Returns whether the row held a live document before.
    pub(crate) async fn force_remove(&self, version: Uuid, kind: &str, id: Uuid) -> AppResult<bool> {
        let mut tx = self.core.db.begin().await?;
        let was_live: Option<bool> =
            sqlx::query_scalar("SELECT doc IS NOT NULL FROM search_documents WHERE kind = $1 AND doc_id = $2 FOR UPDATE")
                .bind(kind)
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        sqlx::query(REMOVE_SQL).bind(kind).bind(id).bind(version).execute(&mut *tx).await?;
        sqlx::query(
            "UPDATE search_documents SET generation = generation + 1, queued_at = now()
             WHERE kind = $1 AND doc_id = $2 AND doc IS NULL AND synced_generation >= generation",
        )
        .bind(kind)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.indexer_wake.notify_one();
        Ok(was_live.unwrap_or(false))
    }

    /// Changes not yet written to the engine.
    pub async fn pending_count(&self) -> AppResult<i64> {
        Ok(sqlx::query_scalar("SELECT count(*) FROM search_documents WHERE synced_generation < generation")
            .fetch_one(&self.core.db)
            .await?)
    }
}
