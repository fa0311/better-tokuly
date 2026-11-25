//! Every state change of an asset, in one place.
//!
//! ```text
//! UPLOADING ──complete──▶ PROCESSING ──finish──▶ READY
//!     │                       │
//!     │ abort / expire        └──fail──▶ FAILED
//!     ▼
//!  (deleted)            any state ──delete──▶ (deleted) ──purge──▶ objects gone
//! ```
//!
//! Deleting is where usage tracking bites: an asset with holders (see
//! `assets`) is not deleted on behalf of its owner, nor collected by the
//! janitor.
//!
//! Each transition is a single guarded `UPDATE`, so concurrent callers (a
//! retried RPC, a redelivered job, the janitor) cannot apply it twice: the
//! one whose update matches a row proceeds, the others see `None`.
//! Transitions that other domains care about emit their event in the same
//! transaction and push the new state to connected clients afterwards.
use serde::{Deserialize, Serialize};
use tokuly_core::{events, prelude::*, storage::Bucket};
use tokuly_proto::{
    common::Image,
    media::{AssetKind, AssetStatus, VideoInfo},
};

use crate::{
    assets::{ASSET_COLUMNS, AssetRow},
    events::{ASSET_DELETED, ASSET_FAILED, ASSET_READY, AssetEvent, push_update},
    store,
};

/// Light work: images, animations, voice messages, file checks.
pub const JOB_PROCESS: &str = "media.process";
/// Video transcodes; a separate queue so they get their own concurrency limit.
pub const JOB_TRANSCODE: &str = "media.transcode";

/// Processing attempts before an asset is given up as FAILED.
pub const MAX_ATTEMPTS: i32 = 3;

/// Lifetime of presigned upload URLs. Parts can be re-signed at any time.
pub const UPLOAD_URL_SECONDS: u64 = 3600;

/// A running job refreshes its asset's `heartbeat_at` this often.
pub const HEARTBEAT_SECONDS: u64 = 20;
/// A heartbeat older than this means the worker is gone and another may
/// take over. Must stay below [`JOB_LEASE_SECONDS`]: when a worker dies its
/// job is redelivered after the lease, and by then the claim has to be free.
pub const CLAIM_SECONDS: i64 = 90;
pub const JOB_LEASE_SECONDS: u64 = 120;
/// A PROCESSING asset nobody has worked on for this long lost its job and
/// is queued again by the janitor (or by a repeated `CompleteUpload`).
pub const STALLED_AFTER_SECONDS: i64 = 180;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessJob {
    pub asset_id: Uuid,
}

/// What processing produced; written to the row when the asset becomes READY.
#[derive(Debug, Default)]
pub struct Processed {
    /// Set when processing refined the kind (an image that turned out animated).
    pub kind: Option<AssetKind>,
    /// Set when processing refined the content type (a ZIP that is a dotLottie).
    pub content_type: Option<String>,
    /// Main rendition; empty for private files.
    pub url: String,
    pub image: Option<Image>,
    pub video: Option<VideoInfo>,
    pub waveform: Option<Vec<u8>>,
}

fn to_json<T: Serialize>(value: &Option<T>) -> AppResult<Option<serde_json::Value>> {
    value
        .as_ref()
        .map(|v| serde_json::to_value(v).map_err(|e| AppError::internal(anyhow::anyhow!("asset result encode: {e}"))))
        .transpose()
}

fn returning(update: &str) -> sqlx::AssertSqlSafe<String> {
    sqlx::AssertSqlSafe(format!("{update} RETURNING {ASSET_COLUMNS}"))
}

/// Queues processing of an asset on the queue matching its kind. The dedupe
/// ID makes repeated calls for the same attempt harmless.
pub(crate) async fn enqueue(core: &Core, row: &AssetRow) -> AppResult<()> {
    let queue = if row.has_kind(AssetKind::ASSET_KIND_VIDEO) { JOB_TRANSCODE } else { JOB_PROCESS };
    core.jobs.enqueue_unique(queue, &format!("{}:{}", row.id, row.attempts), &ProcessJob { asset_id: row.id }).await
}

/// UPLOADING → PROCESSING once the bytes are stored and identified.
///
/// `stored_key` is set when the bytes were moved out of reach of the upload
/// URL ([`store::stored_key`]); the asset then reads from there, and what a
/// late PUT may still write under the upload key is removed once the URL
/// has expired ([`due_for_upload_cleanup`]).
pub(crate) async fn mark_uploaded(
    core: &Core,
    asset_id: Uuid,
    kind: AssetKind,
    content_type: &str,
    stored_key: Option<&str>,
) -> AppResult<Option<AssetRow>> {
    let row: Option<AssetRow> = sqlx::query_as(returning(
        "UPDATE media_assets SET status = $2, kind = $3, content_type = $4, multipart_id = NULL, progress = 0, heartbeat_at = NULL,
                original_key = COALESCE($6, original_key),
                repurge_after = CASE WHEN $6 IS NOT NULL THEN GREATEST(now(), created_at + make_interval(secs => $7)) END
         WHERE id = $1 AND status = $5 AND deleted_at IS NULL",
    ))
    .bind(asset_id)
    .bind(AssetStatus::ASSET_STATUS_PROCESSING as i32)
    .bind(kind as i32)
    .bind(content_type)
    .bind(AssetStatus::ASSET_STATUS_UPLOADING as i32)
    .bind(stored_key)
    .bind(UPLOAD_URL_SECONDS as f64 + 60.0)
    .fetch_optional(&core.db)
    .await?;
    Ok(row)
}

/// Claims an asset for processing and counts the attempt. `None` when there
/// is nothing to do: the asset is finished, failed or deleted, or another
/// worker is on it right now (a duplicate job).
pub(crate) async fn begin_attempt(core: &Core, asset_id: Uuid) -> AppResult<Option<AssetRow>> {
    let row: Option<AssetRow> = sqlx::query_as(returning(
        "UPDATE media_assets SET attempts = attempts + 1, heartbeat_at = now()
         WHERE id = $1 AND status = $2 AND deleted_at IS NULL
           AND (heartbeat_at IS NULL OR heartbeat_at < now() - make_interval(secs => $3))",
    ))
    .bind(asset_id)
    .bind(AssetStatus::ASSET_STATUS_PROCESSING as i32)
    .bind(CLAIM_SECONDS as f64)
    .fetch_optional(&core.db)
    .await?;
    Ok(row)
}

/// Claims the video that is fairest to transcode next and counts the
/// attempt; `None` when no waiting video may start now.
///
/// A `media.transcode` job is a turn on the transcoder, not a ticket for
/// one asset: whichever worker takes a job transcodes the video whose
/// owner has waited longest for their share. Videos are taken round-robin
/// across owners (each owner's oldest waiting video first, an owner with
/// videos already running further back, and among equals the owner served
/// longest ago), and an owner with
/// `per_account` transcodes running gets no more until one ends. So one
/// account's backlog never holds up another account's first video.
///
/// Claims are serialized on one advisory lock, so two workers cannot both
/// start an owner's third video; every read happens on the lock's
/// transaction.
pub(crate) async fn claim_next_transcode(core: &Core, per_account: i64) -> AppResult<Option<AssetRow>> {
    let mut tx = core.db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('media:transcode-claim', 0))").execute(&mut *tx).await?;
    let next: Option<Uuid> = sqlx::query_scalar(
        "WITH processing AS (
             SELECT id, owner_id, created_at, heartbeat_at IS NOT NULL AND heartbeat_at >= now() - make_interval(secs => $3) AS running
             FROM media_assets
             WHERE status = $1 AND kind = $2 AND deleted_at IS NULL
         ), running AS (
             SELECT owner_id, count(*) AS n FROM processing WHERE running GROUP BY owner_id
         ), waiting AS (
             SELECT id, owner_id, created_at, row_number() OVER (PARTITION BY owner_id ORDER BY created_at, id) AS turn
             FROM processing WHERE NOT running
         )
         SELECT w.id FROM waiting w
         LEFT JOIN running r ON r.owner_id = w.owner_id
         LEFT JOIN media_transcode_turns t ON t.owner_id = w.owner_id
         WHERE COALESCE(r.n, 0) < $4
         ORDER BY COALESCE(r.n, 0) + w.turn, t.last_turn_at ASC NULLS FIRST, w.created_at, w.id
         LIMIT 1",
    )
    .bind(AssetStatus::ASSET_STATUS_PROCESSING as i32)
    .bind(AssetKind::ASSET_KIND_VIDEO as i32)
    .bind(CLAIM_SECONDS as f64)
    .bind(per_account.max(1))
    .fetch_optional(&mut *tx)
    .await?;
    let Some(asset_id) = next else {
        tx.commit().await?;
        return Ok(None);
    };
    let row: Option<AssetRow> = sqlx::query_as(returning(
        "UPDATE media_assets SET attempts = attempts + 1, heartbeat_at = now()
         WHERE id = $1 AND status = $2 AND deleted_at IS NULL
           AND (heartbeat_at IS NULL OR heartbeat_at < now() - make_interval(secs => $3))",
    ))
    .bind(asset_id)
    .bind(AssetStatus::ASSET_STATUS_PROCESSING as i32)
    .bind(CLAIM_SECONDS as f64)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(row) = &row {
        sqlx::query(
            "INSERT INTO media_transcode_turns (owner_id, last_turn_at) VALUES ($1, clock_timestamp())
             ON CONFLICT (owner_id) DO UPDATE SET last_turn_at = EXCLUDED.last_turn_at",
        )
        .bind(row.owner_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(row)
}

/// Whether some video is waiting for a transcoder (nobody is on it).
pub(crate) async fn transcodes_waiting(core: &Core) -> AppResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM media_assets
                        WHERE status = $1 AND kind = $2 AND deleted_at IS NULL
                          AND (heartbeat_at IS NULL OR heartbeat_at < now() - make_interval(secs => $3)))",
    )
    .bind(AssetStatus::ASSET_STATUS_PROCESSING as i32)
    .bind(AssetKind::ASSET_KIND_VIDEO as i32)
    .bind(CLAIM_SECONDS as f64)
    .fetch_one(&core.db)
    .await?)
}

/// Queues another turn on the transcoder (see [`claim_next_transcode`]).
/// `after` names the turn that asks for it, so a redelivered job does not
/// queue two.
pub(crate) async fn enqueue_transcode_turn(core: &Core, after: &AssetRow) -> AppResult<()> {
    core.jobs.enqueue_unique(JOB_TRANSCODE, &format!("turn:{}:{}", after.id, after.attempts), &ProcessJob { asset_id: after.id }).await
}

/// Books `seconds` of transcoding on the owner's daily budget for `asset`,
/// or returns `false` when the owner's last 24 hours plus this video would
/// exceed `budget_seconds`. An asset is booked once, whatever the number of
/// attempts; what was booked stays booked when the asset fails or is
/// deleted later (the work was done or reserved). Serialized per owner, so
/// two of their videos probed at the same moment cannot both squeeze in.
pub(crate) async fn book_transcode(core: &Core, asset: &AssetRow, seconds: f64, budget_seconds: f64) -> AppResult<bool> {
    let mut tx = core.db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('media:transcode-budget:' || $1::text, 0))")
        .bind(asset.owner_id)
        .execute(&mut *tx)
        .await?;
    let used: f64 = sqlx::query_scalar(
        "SELECT COALESCE(sum(seconds), 0)::float8 FROM media_transcode_usage
         WHERE owner_id = $1 AND created_at > now() - interval '24 hours' AND asset_id <> $2",
    )
    .bind(asset.owner_id)
    .bind(asset.id)
    .fetch_one(&mut *tx)
    .await?;
    if used + seconds > budget_seconds {
        tx.commit().await?;
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO media_transcode_usage (asset_id, owner_id, seconds) VALUES ($1, $2, $3)
         ON CONFLICT (asset_id) DO NOTHING",
    )
    .bind(asset.id)
    .bind(asset.owner_id)
    .bind(seconds)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// Renews the claim of a running job.
pub(crate) async fn heartbeat(core: &Core, asset_id: Uuid) -> AppResult<()> {
    sqlx::query("UPDATE media_assets SET heartbeat_at = now() WHERE id = $1 AND status = $2 AND deleted_at IS NULL")
        .bind(asset_id)
        .bind(AssetStatus::ASSET_STATUS_PROCESSING as i32)
        .execute(&core.db)
        .await?;
    Ok(())
}

/// Gives an asset back after a failed attempt so the retry can claim it.
pub(crate) async fn release(core: &Core, asset_id: Uuid) -> AppResult<()> {
    sqlx::query("UPDATE media_assets SET heartbeat_at = NULL WHERE id = $1 AND status = $2")
        .bind(asset_id)
        .bind(AssetStatus::ASSET_STATUS_PROCESSING as i32)
        .execute(&core.db)
        .await?;
    Ok(())
}

/// PROCESSING assets without a live worker, oldest first.
pub(crate) async fn stalled(core: &Core, limit: i64) -> AppResult<Vec<AssetRow>> {
    let rows = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ASSET_COLUMNS} FROM media_assets
         WHERE status = $1 AND deleted_at IS NULL AND COALESCE(heartbeat_at, updated_at) < now() - make_interval(secs => $2)
         ORDER BY updated_at LIMIT $3"
    )))
    .bind(AssetStatus::ASSET_STATUS_PROCESSING as i32)
    .bind(STALLED_AFTER_SECONDS as f64)
    .bind(limit)
    .fetch_all(&core.db)
    .await?;
    Ok(rows)
}

/// Whether a PROCESSING asset has been without a worker for too long.
pub(crate) async fn is_stalled(core: &Core, asset_id: Uuid) -> AppResult<bool> {
    let stalled: Option<bool> = sqlx::query_scalar(
        "SELECT COALESCE(heartbeat_at, updated_at) < now() - make_interval(secs => $2) FROM media_assets WHERE id = $1",
    )
    .bind(asset_id)
    .bind(STALLED_AFTER_SECONDS as f64)
    .fetch_optional(&core.db)
    .await?;
    Ok(stalled.unwrap_or(false))
}

/// Records progress (0-99) and pushes it to clients. Returns `false` when
/// the asset is no longer being processed (deleted, or finished by someone
/// else): the worker should stop.
pub(crate) async fn report_progress(core: &Core, asset_id: Uuid, percent: i32) -> AppResult<bool> {
    let row: Option<AssetRow> = sqlx::query_as(returning(
        "UPDATE media_assets SET progress = GREATEST(progress, $2), heartbeat_at = now()
         WHERE id = $1 AND status = $3 AND deleted_at IS NULL",
    ))
    .bind(asset_id)
    .bind(percent.clamp(0, 99))
    .bind(AssetStatus::ASSET_STATUS_PROCESSING as i32)
    .fetch_optional(&core.db)
    .await?;
    match row {
        Some(row) => {
            push_update(core, &row).await;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// PROCESSING → READY. `None` when the asset was deleted (or finished by a
/// duplicate job) in the meantime.
pub(crate) async fn finish(core: &Core, asset_id: Uuid, processed: Processed) -> AppResult<Option<AssetRow>> {
    let mut tx = core.db.begin().await?;
    let row: Option<AssetRow> = sqlx::query_as(returning(
        "UPDATE media_assets SET status = $2, kind = COALESCE($3, kind), content_type = COALESCE($9, content_type),
                url = $4, image = $5, video = $6, waveform = $7, progress = 100, error = '', ready_at = now()
         WHERE id = $1 AND status = $8 AND deleted_at IS NULL",
    ))
    .bind(asset_id)
    .bind(AssetStatus::ASSET_STATUS_READY as i32)
    .bind(processed.kind.map(|k| k as i32))
    .bind(&processed.url)
    .bind(to_json(&processed.image)?)
    .bind(to_json(&processed.video)?)
    .bind(&processed.waveform)
    .bind(AssetStatus::ASSET_STATUS_PROCESSING as i32)
    .bind(&processed.content_type)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(mut row) = row else { return Ok(None) };
    events::emit(&mut *tx, ASSET_READY, Some(row.owner_id), &AssetEvent::of(&row)).await?;
    tx.commit().await?;
    row.sign_private_url(core).await?;
    push_update(core, &row).await;
    Ok(Some(row))
}

/// UPLOADING or PROCESSING → FAILED with a machine-readable reason. The
/// original is discarded: a failed upload cannot be retried in place.
pub(crate) async fn fail(core: &Core, asset_id: Uuid, reason: &str) -> AppResult<Option<AssetRow>> {
    let mut tx = core.db.begin().await?;
    let row: Option<AssetRow> = sqlx::query_as(returning(
        "UPDATE media_assets SET status = $2, error = $3
         WHERE id = $1 AND status IN ($4, $5) AND deleted_at IS NULL",
    ))
    .bind(asset_id)
    .bind(AssetStatus::ASSET_STATUS_FAILED as i32)
    .bind(reason)
    .bind(AssetStatus::ASSET_STATUS_UPLOADING as i32)
    .bind(AssetStatus::ASSET_STATUS_PROCESSING as i32)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else { return Ok(None) };
    events::emit(&mut *tx, ASSET_FAILED, Some(row.owner_id), &AssetEvent::of(&row)).await?;
    tx.commit().await?;
    push_update(core, &row).await;
    if let Err(e) = remove_objects(core, &row).await {
        // The janitor does not revisit failed assets; the owner deleting the
        // asset (or their account) retries the removal.
        tracing::warn!(asset_id = %row.id, error = %e, "could not remove the objects of a failed asset");
    }
    Ok(Some(row))
}

/// Deletes an asset: it disappears for every reader at once, and its
/// objects are removed (right away when possible; the janitor retries
/// otherwise). Returns whether an asset was deleted; `false` means there
/// was none.
///
/// With an `owner_id` this is a delete on behalf of that user: it only
/// applies to their own asset and is refused with `ASSET_IN_USE` while the
/// asset has holders. Without one, the caller is a domain acting on its own
/// authority (a takedown): the asset goes whatever holds it, and the
/// holders learn of it through `media.asset.deleted`.
pub async fn delete(core: &Core, asset_id: Uuid, owner_id: Option<Uuid>) -> AppResult<bool> {
    let held = if owner_id.is_some() { Held::Refuse } else { Held::Delete };
    match remove(core, asset_id, owner_id, held).await? {
        Removal::Done => Ok(true),
        Removal::NotFound => Ok(false),
        Removal::InUse => Err(AppError::precondition("ASSET_IN_USE", "the upload is in use and cannot be deleted")),
    }
}

/// Deletes an asset unless something holds it (the janitor's collection of
/// unused assets). Returns whether it was deleted.
pub(crate) async fn delete_if_unheld(core: &Core, asset_id: Uuid) -> AppResult<bool> {
    Ok(matches!(remove(core, asset_id, None, Held::Refuse).await?, Removal::Done))
}

/// What to do when the asset to delete has holders.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Held {
    Refuse,
    Delete,
}

enum Removal {
    Done,
    NotFound,
    InUse,
}

async fn remove(core: &Core, asset_id: Uuid, owner_id: Option<Uuid>, held: Held) -> AppResult<Removal> {
    let mut tx = core.db.begin().await?;
    // The holder check is part of the statement that deletes, on the row it
    // deletes: an attach racing with it either gets in first (and the delete
    // is refused) or waits for this row and finds the asset gone. See `holders`.
    let row: Option<AssetRow> = sqlx::query_as(returning(
        "UPDATE media_assets SET deleted_at = now()
         WHERE id = $1 AND deleted_at IS NULL AND ($2::uuid IS NULL OR owner_id = $2) AND ($3 OR holder_count = 0)",
    ))
    .bind(asset_id)
    .bind(owner_id)
    .bind(held == Held::Delete)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM media_assets WHERE id = $1 AND deleted_at IS NULL AND ($2::uuid IS NULL OR owner_id = $2))",
        )
        .bind(asset_id)
        .bind(owner_id)
        .fetch_one(&mut *tx)
        .await?;
        return Ok(if exists { Removal::InUse } else { Removal::NotFound });
    };
    if row.is_attached() {
        sqlx::query("DELETE FROM media_asset_holders WHERE asset_id = $1").bind(row.id).execute(&mut *tx).await?;
    }
    // An upload that never completed and that nothing held was never
    // visible to other domains.
    if !row.has_status(AssetStatus::ASSET_STATUS_UPLOADING) || row.is_attached() {
        events::emit(&mut *tx, ASSET_DELETED, Some(row.owner_id), &AssetEvent::of(&row)).await?;
    }
    tx.commit().await?;
    if let Err(e) = purge(core, &row).await {
        tracing::warn!(asset_id = %row.id, error = %e, "asset purge deferred to the janitor");
    }
    Ok(Removal::Done)
}

/// Deletes every asset of a user that nothing holds (account deletion).
/// Held assets stay until their holders let go; the janitor collects them
/// then. The janitor also removes the objects of what is deleted here.
pub(crate) async fn delete_unheld_of(core: &Core, owner_id: Uuid) -> AppResult<u64> {
    let done = sqlx::query("UPDATE media_assets SET deleted_at = now() WHERE owner_id = $1 AND deleted_at IS NULL AND holder_count = 0")
        .bind(owner_id)
        .execute(&core.db)
        .await?;
    Ok(done.rows_affected())
}

/// READY assets that nothing has held for `ttl_seconds`: never attached
/// since they became ready, or released by their last holder that long ago.
pub(crate) async fn unattached(core: &Core, ttl_seconds: f64, limit: i64) -> AppResult<Vec<AssetRow>> {
    let rows = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ASSET_COLUMNS} FROM media_assets
         WHERE status = $1 AND deleted_at IS NULL AND holder_count = 0
           AND GREATEST(ready_at, detached_at) < now() - make_interval(secs => $2)
         ORDER BY ready_at LIMIT $3"
    )))
    .bind(AssetStatus::ASSET_STATUS_READY as i32)
    .bind(ttl_seconds)
    .bind(limit)
    .fetch_all(&core.db)
    .await?;
    Ok(rows)
}

/// Removes everything a deleted asset stored and records that it is gone.
///
/// A single-PUT upload URL stays valid for [`UPLOAD_URL_SECONDS`] after the
/// asset was created, whatever happens to the asset. If the purge runs
/// within that time, a late PUT could put the original back, so the asset
/// is scheduled for one more purge once the URL has expired.
pub(crate) async fn purge(core: &Core, row: &AssetRow) -> AppResult<()> {
    remove_objects(core, row).await?;
    sqlx::query(
        "UPDATE media_assets SET purged_at = now(), multipart_id = NULL, url = '', image = NULL, video = NULL, waveform = NULL,
                repurge_after = CASE WHEN now() < created_at + make_interval(secs => $2) THEN created_at + make_interval(secs => $2) END
         WHERE id = $1 AND deleted_at IS NOT NULL",
    )
    .bind(row.id)
    .bind(UPLOAD_URL_SECONDS as f64 + 60.0)
    .execute(&core.db)
    .await?;
    Ok(())
}

/// Purged assets whose second purge is due.
pub(crate) async fn due_for_repurge(core: &Core, limit: i64) -> AppResult<Vec<AssetRow>> {
    let rows = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ASSET_COLUMNS} FROM media_assets WHERE repurge_after < now() AND deleted_at IS NOT NULL ORDER BY repurge_after LIMIT $1"
    )))
    .bind(limit)
    .fetch_all(&core.db)
    .await?;
    Ok(rows)
}

/// Live assets completed while their upload URL was still valid, now that
/// it has expired: whatever a late PUT stored under the upload key can go.
pub(crate) async fn due_for_upload_cleanup(core: &Core, limit: i64) -> AppResult<Vec<AssetRow>> {
    let rows = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ASSET_COLUMNS} FROM media_assets WHERE repurge_after < now() AND deleted_at IS NULL ORDER BY repurge_after LIMIT $1"
    )))
    .bind(limit)
    .fetch_all(&core.db)
    .await?;
    Ok(rows)
}

/// Removes the object under a live asset's upload key (never its stored
/// original) and records that nothing is left to clean up.
pub(crate) async fn remove_upload_object(core: &Core, row: &AssetRow) -> AppResult<()> {
    let upload_key = store::original_key(row.id, &crate::recipe::extension_of(&row.file_name));
    if upload_key != row.original_key {
        core.storage.delete(Bucket::Private, &upload_key).await?;
    }
    sqlx::query("UPDATE media_assets SET repurge_after = NULL WHERE id = $1 AND deleted_at IS NULL")
        .bind(row.id)
        .execute(&core.db)
        .await?;
    Ok(())
}

/// FAILED assets that nothing holds and nobody deleted for
/// `older_than_seconds`. They can never be used, so they are only clutter.
pub(crate) async fn old_failures(core: &Core, older_than_seconds: i64, limit: i64) -> AppResult<Vec<AssetRow>> {
    let rows = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ASSET_COLUMNS} FROM media_assets
         WHERE status = $1 AND deleted_at IS NULL AND holder_count = 0 AND updated_at < now() - make_interval(secs => $2)
         ORDER BY updated_at LIMIT $3"
    )))
    .bind(AssetStatus::ASSET_STATUS_FAILED as i32)
    .bind(older_than_seconds as f64)
    .bind(limit)
    .fetch_all(&core.db)
    .await?;
    Ok(rows)
}

/// Deletes the original (aborting an unfinished multipart upload) and every
/// rendition of an asset. Safe to repeat.
pub(crate) async fn remove_objects(core: &Core, row: &AssetRow) -> AppResult<()> {
    if let Some(multipart_id) = &row.multipart_id {
        // Fails when the upload was already completed or aborted; either way
        // nothing is left to abort.
        if let Err(e) = core.storage.abort_multipart(Bucket::Private, &row.original_key, multipart_id).await {
            tracing::debug!(asset_id = %row.id, error = %e, "multipart abort skipped");
        }
    }
    // What the public can fetch goes first: it stops being served the
    // moment it is gone from the bucket.
    remove_renditions(core, row.id).await?;
    core.storage.delete_prefix(Bucket::Private, &store::original_prefix(row.id)).await?;
    Ok(())
}

/// Deletes every rendition of an asset from the media bucket.
pub(crate) async fn remove_renditions(core: &Core, asset_id: Uuid) -> AppResult<()> {
    for prefix in store::public_prefixes(asset_id) {
        core.storage.delete_prefix(Bucket::Media, &prefix).await?;
    }
    Ok(())
}
