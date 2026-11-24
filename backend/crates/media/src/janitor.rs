//! Housekeeping: everything that has to happen eventually even when the
//! request or job that should have done it did not finish.
//!
//! - unfinished uploads older than the upload lifetime are aborted;
//! - failed assets that nothing holds are removed after a week;
//! - finished assets that nothing has held for the unattached lifetime are
//!   deleted (uploads that were never used, media of deleted content);
//! - deleted assets whose objects could not be removed at once are purged,
//!   and those purged while an upload URL was still valid are purged again;
//! - what a late PUT stored under the upload key of a completed upload is
//!   removed once the URL has expired;
//! - assets stuck in PROCESSING without a live worker are queued again;
//! - stale link preview cache rows are dropped, and cached preview pictures
//!   older than a day are removed with their objects;
//! - when an account is deleted, its assets that nothing holds go at once;
//!   the rest follow when their holders let go.
use serde::Deserialize;
use tokuly_core::{events::Event, prelude::*};

use crate::{
    Media,
    assets::{ASSET_COLUMNS, AssetRow},
    lifecycle, link_preview,
};

/// Rows handled per sweep step; a sweep repeats a step until it runs dry.
const BATCH: i64 = 200;

/// FAILED assets are kept this long (so the owner can see what went wrong).
const FAILED_RETENTION_SECONDS: i64 = 7 * 24 * 3600;

/// What one janitor sweep did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Sweep {
    pub expired_uploads: u64,
    pub expired_failures: u64,
    /// READY assets deleted because nothing held them for too long.
    pub unattached_assets: u64,
    pub purged_assets: u64,
    pub requeued_assets: u64,
    /// Cached link preview pictures that expired.
    pub expired_link_images: u64,
}

#[derive(Deserialize)]
struct UserDeleted {
    user_id: Uuid,
}

impl Media {
    /// Runs every housekeeping step once.
    pub async fn sweep(&self) -> AppResult<Sweep> {
        let core = &self.core;
        let mut sweep = Sweep::default();

        loop {
            let stale = self.stale_uploads(BATCH).await?;
            for upload in &stale {
                if lifecycle::delete(core, upload.id, None).await? {
                    sweep.expired_uploads += 1;
                }
            }
            if (stale.len() as i64) < BATCH {
                break;
            }
        }

        for failed in lifecycle::old_failures(core, FAILED_RETENTION_SECONDS, BATCH).await? {
            if lifecycle::delete_if_unheld(core, failed.id).await? {
                sweep.expired_failures += 1;
            }
        }

        if let Some(ttl) = self.cfg.unattached_ttl() {
            loop {
                let unused = lifecycle::unattached(core, ttl.as_secs_f64(), BATCH).await?;
                for asset in &unused {
                    // Something may have attached it since the query ran.
                    if lifecycle::delete_if_unheld(core, asset.id).await? {
                        sweep.unattached_assets += 1;
                    }
                }
                if (unused.len() as i64) < BATCH {
                    break;
                }
            }
        }

        sweep.purged_assets = self.purge_deleted(None).await?;
        for asset in lifecycle::due_for_repurge(core, BATCH).await? {
            match lifecycle::purge(core, &asset).await {
                Ok(()) => sweep.purged_assets += 1,
                Err(e) => tracing::warn!(asset_id = %asset.id, error = %e, "asset re-purge failed; will retry"),
            }
        }

        for asset in lifecycle::due_for_upload_cleanup(core, BATCH).await? {
            if let Err(e) = lifecycle::remove_upload_object(core, &asset).await {
                tracing::warn!(asset_id = %asset.id, error = %e, "late upload cleanup failed; will retry");
            }
        }

        for asset in lifecycle::stalled(core, BATCH).await? {
            lifecycle::enqueue(core, &asset).await?;
            sweep.requeued_assets += 1;
        }

        sqlx::query("DELETE FROM link_previews WHERE fetched_at < now() - make_interval(days => $1)")
            .bind(link_preview::CACHE_RETENTION_DAYS)
            .execute(&core.db)
            .await?;
        loop {
            match link_preview::image_proxy::expire(core, BATCH).await {
                Ok(expired) => {
                    sweep.expired_link_images += expired;
                    if (expired as i64) < BATCH {
                        break;
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "expiring link preview pictures failed; will retry");
                    break;
                }
            }
        }
        Ok(sweep)
    }

    /// Removes the objects of deleted assets (of one owner, or of everyone).
    /// An asset whose objects cannot be removed right now is left for the
    /// next sweep.
    async fn purge_deleted(&self, owner_id: Option<Uuid>) -> AppResult<u64> {
        let core = &self.core;
        let mut purged = 0;
        // Keyset over IDs, so one asset that keeps failing cannot stall the rest.
        let mut after = Uuid::nil();
        loop {
            let rows: Vec<AssetRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {ASSET_COLUMNS} FROM media_assets
                 WHERE deleted_at IS NOT NULL AND purged_at IS NULL AND id > $1 AND ($2::uuid IS NULL OR owner_id = $2)
                 ORDER BY id LIMIT $3"
            )))
            .bind(after)
            .bind(owner_id)
            .bind(BATCH)
            .fetch_all(&core.db)
            .await?;
            let Some(last) = rows.last() else { break };
            after = last.id;
            for row in &rows {
                match lifecycle::purge(core, row).await {
                    Ok(()) => purged += 1,
                    Err(e) => tracing::warn!(asset_id = %row.id, error = %e, "asset purge failed; will retry"),
                }
            }
        }
        Ok(purged)
    }

    /// `identity.user.deleted`: the account is gone, and so is its media —
    /// except what other things still hold (art in a stamp pack that
    /// outlives its creator). Those assets are collected like any other once
    /// their holders detach.
    pub async fn on_user_deleted(&self, event: Event) -> AppResult<()> {
        let UserDeleted { user_id } = event.data()?;
        let deleted = lifecycle::delete_unheld_of(&self.core, user_id).await?;
        let purged = self.purge_deleted(Some(user_id)).await?;
        if deleted > 0 {
            tracing::info!(%user_id, deleted, purged, "removed the media of a deleted account");
        }
        Ok(())
    }
}
