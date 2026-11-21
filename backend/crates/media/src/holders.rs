//! Usage tracking: who holds an asset. The model and its rules are
//! described at the top of [`crate::assets`], which re-exports these
//! functions; this file is about keeping the bookkeeping exact.
//!
//! Two things record a hold: a row in `media_asset_holders`, and the
//! `holder_count` on the asset itself. The count exists so that "does
//! anything hold this asset?" is a property of the asset's own row. A delete
//! is `UPDATE … WHERE holder_count = 0`, and PostgreSQL re-checks such a
//! condition against the latest version of a row it had to wait for —
//! something it does not do for a subquery on another table.
//!
//! Every function here first locks the asset row and only then touches the
//! holder rows. With that order, attaching and deleting the same asset at
//! the same moment has exactly two outcomes: the attach wins and the delete
//! is refused, or the delete wins and the attach reports that the asset is
//! gone. It also means concurrent attaches and detaches of one asset queue
//! up instead of deadlocking.
//!
//! Each function is a single statement, because the caller hands in an
//! executor (often its own open transaction) that can run only one.
use sqlx::PgExecutor;
use tokuly_core::prelude::*;

/// Longest holder reference accepted.
const MAX_HOLDER_LENGTH: usize = 200;

fn check_holder(holder: &str) -> AppResult<()> {
    ensure!(
        !holder.is_empty() && holder.len() <= MAX_HOLDER_LENGTH,
        invalid,
        "HOLDER_INVALID",
        "a holder is a short reference like `chat:message:<id>`"
    );
    Ok(())
}

/// Records that `holder` uses the asset. Attaching the same holder again
/// changes nothing. Fails with `ASSET_NOT_FOUND` when the asset does not
/// exist or was deleted.
pub async fn attach<'e, E: PgExecutor<'e>>(executor: E, asset_id: Uuid, holder: &str) -> AppResult<()> {
    check_holder(holder)?;
    // `held` yields a row only for a hold that is new, so the count moves
    // exactly once per holder.
    let found: bool = sqlx::query_scalar(
        "WITH asset AS (
             SELECT id FROM media_assets WHERE id = $1 AND deleted_at IS NULL FOR UPDATE
         ), held AS (
             INSERT INTO media_asset_holders (asset_id, holder)
             SELECT id, $2 FROM asset
             ON CONFLICT (asset_id, holder) DO NOTHING
             RETURNING asset_id
         ), counted AS (
             UPDATE media_assets SET holder_count = holder_count + 1, detached_at = NULL
             WHERE id IN (SELECT asset_id FROM held)
         )
         SELECT EXISTS (SELECT 1 FROM asset)",
    )
    .bind(asset_id)
    .bind(holder)
    .fetch_one(executor)
    .await?;
    ensure!(found, not_found, "ASSET_NOT_FOUND", "asset not found");
    Ok(())
}

/// Records that `holder` no longer uses the asset. Does nothing when it did
/// not. When this was the last holder the asset becomes collectable (after
/// the unattached lifetime).
pub async fn detach<'e, E: PgExecutor<'e>>(executor: E, asset_id: Uuid, holder: &str) -> AppResult<()> {
    sqlx::query(
        "WITH asset AS (
             SELECT id FROM media_assets WHERE id = $1 FOR UPDATE
         ), released AS (
             DELETE FROM media_asset_holders WHERE asset_id IN (SELECT id FROM asset) AND holder = $2
             RETURNING asset_id
         )
         UPDATE media_assets
         SET holder_count = GREATEST(holder_count - 1, 0),
             detached_at = CASE WHEN holder_count <= 1 THEN now() ELSE detached_at END
         WHERE id IN (SELECT asset_id FROM released)",
    )
    .bind(asset_id)
    .bind(holder)
    .execute(executor)
    .await?;
    Ok(())
}

/// Releases every asset `holder` uses, for when the holding thing itself is
/// deleted. Returns how many assets it held.
pub async fn detach_holder<'e, E: PgExecutor<'e>>(executor: E, holder: &str) -> AppResult<u64> {
    // Assets are locked in ID order, so two bulk releases that overlap
    // cannot wait on each other in a circle.
    let released: i64 = sqlx::query_scalar(
        "WITH asset AS (
             SELECT id FROM media_assets
             WHERE id IN (SELECT asset_id FROM media_asset_holders WHERE holder = $1)
             ORDER BY id FOR UPDATE
         ), released AS (
             DELETE FROM media_asset_holders WHERE holder = $1 AND asset_id IN (SELECT id FROM asset)
             RETURNING asset_id
         ), counted AS (
             UPDATE media_assets
             SET holder_count = GREATEST(holder_count - 1, 0),
                 detached_at = CASE WHEN holder_count <= 1 THEN now() ELSE detached_at END
             WHERE id IN (SELECT asset_id FROM released)
         )
         SELECT count(*) FROM released",
    )
    .bind(holder)
    .fetch_one(executor)
    .await?;
    Ok(released as u64)
}

/// Whether anything holds the asset right now. `false` for assets that do
/// not exist.
pub async fn is_attached(core: &Core, asset_id: Uuid) -> AppResult<bool> {
    let attached: Option<bool> = sqlx::query_scalar("SELECT holder_count > 0 FROM media_assets WHERE id = $1 AND deleted_at IS NULL")
        .bind(asset_id)
        .fetch_optional(&core.db)
        .await?;
    Ok(attached.unwrap_or(false))
}
