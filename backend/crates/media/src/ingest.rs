//! Assets created by the server itself rather than uploaded by a client
//! (e.g. the recording of a live stream that becomes its archive).
use std::path::Path;

use tokio::io::AsyncReadExt as _;
use tokuly_core::prelude::*;
use tokuly_proto::media::{AssetPurpose, AssetStatus};

use crate::{assets::AssetRow, lifecycle, recipe, store};

/// Stores a local file as a new asset of `owner_id` and queues its
/// processing; the asset becomes READY (or FAILED) like any upload, with
/// the same events. The file is copied: the caller keeps (and cleans up)
/// `path`.
///
/// The content is detected from the file's bytes and must suit `purpose`
/// (`UNSUPPORTED_CONTENT` otherwise). Upload size limits do not apply.
pub async fn create_asset_from_file(core: &Core, owner_id: Uuid, purpose: AssetPurpose, path: &Path, file_name: &str) -> AppResult<AssetRow> {
    let unreadable = |e: std::io::Error| AppError::internal(anyhow::anyhow!("cannot read {}: {e}", path.display()));
    let size_bytes = tokio::fs::metadata(path).await.map_err(unreadable)?.len() as i64;
    ensure!(size_bytes > 0, invalid, "FILE_EMPTY", "the file is empty");

    let mut prefix = Vec::with_capacity(recipe::SNIFF_BYTES);
    let file = tokio::fs::File::open(path).await.map_err(unreadable)?;
    file.take(recipe::SNIFF_BYTES as u64).read_to_end(&mut prefix).await.map_err(unreadable)?;
    let sniffed = recipe::sniff(&prefix);
    // Recordings arrive as MPEG-TS, which has no magic number to sniff; a
    // server-side caller asking for a video purpose is taken at its word
    // and ffprobe has the final say.
    let class = if sniffed.class == recipe::ContentClass::Other && recipe::resolve(purpose, recipe::ContentClass::AvContainer).is_some() {
        recipe::ContentClass::AvContainer
    } else {
        sniffed.class
    };
    let Some(kind) = recipe::resolve(purpose, class) else {
        bail!(invalid, "UNSUPPORTED_CONTENT", "{} cannot be stored for this purpose", sniffed.content_type);
    };

    let asset_id = new_id();
    let file_name = recipe::sanitize_file_name(file_name);
    let key = store::original_key(asset_id, &recipe::extension_of(&file_name));
    // The row exists (as an upload in flight) before the bytes do, so a
    // crash in between leaves something the janitor will clean up.
    sqlx::query(
        "INSERT INTO media_assets (id, owner_id, purpose, kind, status, content_type, size_bytes, file_name, original_key)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(asset_id)
    .bind(owner_id)
    .bind(purpose as i32)
    .bind(kind as i32)
    .bind(AssetStatus::ASSET_STATUS_UPLOADING as i32)
    .bind(&sniffed.content_type)
    .bind(size_bytes)
    .bind(&file_name)
    .bind(&key)
    .execute(&core.db)
    .await?;

    if let Err(error) = store::OriginalStore::new(&core.cfg).put_file(core, &key, path, &sniffed.content_type).await {
        lifecycle::delete(core, asset_id, None).await?;
        return Err(error);
    }
    let row = lifecycle::mark_uploaded(core, asset_id, kind, &sniffed.content_type).await?.or_not_found("ASSET_NOT_FOUND", "asset not found")?;
    lifecycle::enqueue(core, &row).await?;
    Ok(row)
}
