//! Cover frames: the picture of a short at an offset its creator chose.
//!
//! The media pipeline picks a poster frame by itself. A creator who wants
//! another moment names an offset (`CoverInput.offset_ms`); this module
//! cuts that frame out of the uploaded video with ffmpeg and hands it to
//! the media domain as an image asset (`ASSET_PURPOSE_THUMBNAIL`), so it
//! gets the same variants, placeholder and URLs as any image:
//!
//! ```text
//! cover asked for ──(short is PUBLISHED)──▶ job shorts.cover ──ffmpeg──▶ image asset (processing)
//!                                             │ cover_pending_asset_id        │ media.asset.ready
//!                                             ▼                               ▼
//!                               the poster frame stands in        cover = the frame, cover_asset_id
//! ```
//!
//! A frame that cannot be cut (the offset holds no picture, the file is
//! unreadable) falls back to the poster frame rather than failing forever.
use std::{path::Path, process::Stdio, time::Duration};

use tokuly_core::{prelude::*, storage::Bucket};
use tokuly_media::assets;
use tokuly_proto::media::{AssetPurpose, AssetStatus};

use crate::{
    Shorts,
    events::CoverJob,
    listing,
    shorts::{self, COVER_FRAME, holder},
};

/// Longest a single ffmpeg run may take.
const FFMPEG_TIMEOUT: Duration = Duration::from_secs(120);

impl Shorts {
    /// Handles one `shorts.cover` job: cuts the frame and stores it as an
    /// image asset the short waits for. A job that no longer matches what
    /// the short asks for (the cover was changed again, the short is gone)
    /// does nothing.
    pub async fn cut_cover(&self, job: CoverJob) -> AppResult<()> {
        let core = &self.core;
        let Some(short) = shorts::find_by_id(core, job.short_id).await? else { return Ok(()) };
        let wanted = short.is_published()
            && short.cover_kind == COVER_FRAME
            && short.cover_offset_ms == Some(job.offset_ms)
            && short.cover_asset_id.is_none()
            && short.cover_pending_asset_id.is_none();
        if !wanted {
            return Ok(());
        }
        let Some(video) = assets::find(core, short.asset_id).await? else { return Ok(()) };

        let dir = tempfile::tempdir().map_err(|e| AppError::internal(anyhow::anyhow!("cover scratch space: {e}")))?;
        let source = dir.path().join("source");
        core.storage.download(Bucket::Private, &video.original_key, &source).await?;
        let frame = dir.path().join("cover.png");
        // A little before the end at most: the very last instant has no frame.
        let offset = (f64::from(job.offset_ms) / 1000.0).clamp(0.0, (short.duration_seconds - 0.1).max(0.0));
        let cut = match self.grab_frame(&source, &format!("{offset:.3}"), false, &frame).await {
            Ok(()) => Ok(()),
            // An offset past the last picture means the last picture.
            Err(_) => self.grab_frame(&source, "-0.5", true, &frame).await,
        };
        if let Err(reason) = cut {
            tracing::warn!(short_id = %short.id, offset_ms = job.offset_ms, %reason, "no cover frame could be cut; using the poster");
            self.fall_back_to_poster(short.id, job.offset_ms).await?;
            return Ok(());
        }

        let image = tokuly_media::create_asset_from_file(
            core,
            short.owner_id,
            AssetPurpose::ASSET_PURPOSE_THUMBNAIL,
            &frame,
            "cover.png",
        )
        .await?;
        let mut tx = core.db.begin().await?;
        let claimed: Option<Uuid> = sqlx::query_scalar(
            "UPDATE shorts SET cover_pending_asset_id = $2
             WHERE id = $1 AND cover_kind = 2 AND cover_offset_ms = $3 AND cover_asset_id IS NULL
               AND cover_pending_asset_id IS NULL
             RETURNING id",
        )
        .bind(short.id)
        .bind(image.id)
        .bind(job.offset_ms)
        .fetch_optional(&mut *tx)
        .await?;
        if claimed.is_none() {
            // The cover changed while the frame was being cut.
            drop(tx);
            assets::delete(core, image.id, None).await?;
            return Ok(());
        }
        assets::attach(&mut *tx, image.id, &holder(short.id)).await?;
        tx.commit().await?;
        // Its `ready` event may have been consumed before the short pointed at it.
        self.sync_cover(image.id).await?;
        Ok(())
    }

    /// Runs ffmpeg to write the frame at `position` of `source` to `out`.
    /// `from_end` counts the position back from the end (a negative number
    /// of seconds). The error is a short description for the log.
    async fn grab_frame(&self, source: &Path, position: &str, from_end: bool, out: &Path) -> Result<(), String> {
        let _ = tokio::fs::remove_file(out).await;
        let mut command = tokio::process::Command::new(&self.cfg.ffmpeg_path);
        command
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
            // The input is an upload: it must not be able to make ffmpeg
            // open anything but itself (playlists and the like can name
            // other files and network addresses).
            .args(["-protocol_whitelist", "file"])
            .arg(if from_end { "-sseof" } else { "-ss" })
            .arg(position)
            .arg("-i")
            .arg(source)
            // Full HD at most; the image pipeline makes the smaller variants.
            .args(["-frames:v", "1", "-vf", "scale='min(1080,iw)':-2", "-f", "image2", "-c:v", "png"])
            .arg(out)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let output = tokio::time::timeout(FFMPEG_TIMEOUT, command.output())
            .await
            .map_err(|_| "ffmpeg timed out".to_owned())?
            .map_err(|e| format!("ffmpeg could not be started: {e}"))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().chars().take(300).collect());
        }
        match tokio::fs::metadata(out).await {
            Ok(meta) if meta.len() > 0 => Ok(()),
            _ => Err("ffmpeg wrote no frame".to_owned()),
        }
    }

    /// Gives up on a frame cover: the short shows its poster frame again.
    async fn fall_back_to_poster(&self, short_id: Uuid, offset_ms: i32) -> AppResult<()> {
        sqlx::query(
            "UPDATE shorts SET cover_kind = 1, cover_offset_ms = NULL, cover = playback -> 'poster'
             WHERE id = $1 AND cover_kind = 2 AND cover_offset_ms = $2 AND cover_asset_id IS NULL
               AND cover_pending_asset_id IS NULL",
        )
        .bind(short_id)
        .bind(offset_ms)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    /// Brings the short waiting for cover frame `asset_id` in step with that
    /// asset: READY shows the frame; FAILED or gone falls back to the poster
    /// frame. Idempotent; called for `media.asset.*` events and by the
    /// reconciling worker. Returns whether a short changed.
    pub async fn sync_cover(&self, asset_id: Uuid) -> AppResult<bool> {
        let core = &self.core;
        let asset = assets::find(core, asset_id).await?;
        let frame = match &asset {
            Some(asset) if asset.is_ready() => asset.image.clone(),
            Some(asset) if !asset.has_status(AssetStatus::ASSET_STATUS_FAILED) => return Ok(false), // still processing
            _ => None,
        };
        // Everything the change needs from elsewhere is read before the transaction.
        let waiting: Option<(Uuid, Uuid, Option<Uuid>)> =
            sqlx::query_as("SELECT channel_id, owner_id, circle_id FROM shorts WHERE cover_pending_asset_id = $1")
                .bind(asset_id)
                .fetch_optional(&core.db)
                .await?;
        let Some((channel_id, owner_id, circle_id)) = waiting else { return Ok(false) };
        let context = listing::Context::of(core, channel_id, owner_id, circle_id).await?;
        let mut tx = core.db.begin().await?;
        let short_id: Option<Uuid> = match &frame {
            Some(image) => {
                sqlx::query_scalar(
                    "UPDATE shorts SET cover = $2, cover_asset_id = $1, cover_pending_asset_id = NULL
                     WHERE cover_pending_asset_id = $1 RETURNING id",
                )
                .bind(asset_id)
                .bind(image)
                .fetch_optional(&mut *tx)
                .await?
            }
            None => {
                sqlx::query_scalar(
                    "UPDATE shorts SET cover_pending_asset_id = NULL, cover_kind = 1, cover_offset_ms = NULL,
                            cover = playback -> 'poster'
                     WHERE cover_pending_asset_id = $1 RETURNING id",
                )
                .bind(asset_id)
                .fetch_optional(&mut *tx)
                .await?
            }
        };
        let Some(short_id) = short_id else { return Ok(false) };
        if frame.is_none() {
            assets::detach(&mut *tx, asset_id, &holder(short_id)).await?;
        }
        // The search document carries the cover.
        listing::after_change_in(&mut tx, short_id, false, &context).await?;
        tx.commit().await?;
        if frame.is_none() && asset.is_some() {
            // A frame that failed to process is of no use to anyone.
            if let Err(error) = assets::delete(core, asset_id, None).await {
                tracing::warn!(%asset_id, %error, "could not delete a failed cover frame");
            }
        }
        Ok(true)
    }
}
