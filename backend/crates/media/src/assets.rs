//! Assets as other domains use them. This is the whole contract between the
//! media domain and everything that attaches media; other domains never
//! touch the `media_*` tables themselves.
//!
//! # Using an upload
//!
//! A client uploads a file, gets an asset ID, and passes it to some other
//! RPC ("set my avatar", "send this message"). The receiving domain then:
//!
//! 1. **Resolves** the ID with [`require_ready`] (or [`require_image_json`],
//!    or [`require`] when a still-processing asset is acceptable). That
//!    checks that the asset exists, belongs to the caller, was uploaded for
//!    an acceptable purpose and finished processing. It returns the
//!    [`AssetRow`], from which [`AssetRow::image`], [`AssetRow::to_attachment`]
//!    and friends produce what is stored and sent to clients.
//! 2. **Attaches** it, in the same transaction that stores the reference:
//!    `assets::attach(&mut *tx, asset_id, "chat:message:<id>")`.
//! 3. **Detaches** it when the reference goes away: [`detach`] when one
//!    reference is replaced or removed (a new avatar replaces the old one),
//!    [`detach_holder`] when the holding thing itself is deleted.
//!
//! # Holders: what keeps an asset alive
//!
//! A *holder* is a string naming the thing that uses an asset. The using
//! domain chooses it; the convention is `<domain>:<entity>:<id>`, e.g.
//! `identity:avatar:<user_id>`, `stamp:<stamp_id>`, `chat:message:<id>`. An
//! asset may have any number of holders (a forwarded photo), and a holder may
//! hold any number of assets (a note with four images).
//!
//! The rules that follow from it:
//!
//! - **Held assets cannot be deleted by their owner.** `DeleteAsset` and
//!   [`delete`] with an owner fail with `ASSET_IN_USE` while a holder
//!   exists. An avatar in use does not vanish because its uploader pruned
//!   their uploads.
//! - **Unheld assets are temporary.** A READY asset that has had no holder
//!   for `TOKULY_MEDIA_UNATTACHED_TTL_HOURS` (48 h by default) is deleted by
//!   the janitor, counted from when it became ready or from when its last
//!   holder detached. An upload that is never used cleans itself up; so does
//!   an asset whose message was deleted. **A domain that stores an asset's
//!   URLs without attaching it will find them dead two days later.**
//! - **Account deletion spares what is still held.** When a user is deleted
//!   their unheld assets go at once; held ones (art in a stamp pack that
//!   outlives its creator) stay until their holders detach.
//! - **A domain may delete regardless**: [`delete`] without an owner removes
//!   the asset whatever holds it (moderation takedowns). Holders learn of it
//!   through the `media.asset.deleted` event.
//!
//! [`attach`], [`detach`] and [`detach_holder`] take any `sqlx` executor, so
//! they join the caller's transaction: a reference and its hold are stored
//! or rolled back together. All three are idempotent.
//!
//! # Private files
//!
//! Private files (chat files, subtitles) have no permanent URL: every row
//! returned by this module carries a freshly presigned download URL valid for
//! [`PRIVATE_URL_TTL`]. A domain that stores an [`Attachment`] must therefore
//! renew file URLs when it serves it later: [`refresh_attachments`].
use std::{collections::HashMap, time::Duration};

use bytes::Bytes;
use chrono::{DateTime, Utc};
use tokuly_core::{prelude::*, storage::Bucket};
use tokuly_proto::{
    common::Image,
    media::{Asset, AssetKind, AssetPurpose, AssetStatus, Attachment, VideoInfo},
};

pub use crate::holders::{attach, detach, detach_holder, is_attached};
pub use crate::lifecycle::delete;
use crate::store;

/// How long the presigned download URL of a private file stays valid.
pub const PRIVATE_URL_TTL: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AssetRow {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub purpose: i32,
    pub kind: i32,
    pub status: i32,
    pub content_type: String,
    pub size_bytes: i64,
    pub file_name: String,
    pub sha256: String,
    pub original_key: String,
    pub multipart_id: Option<String>,
    pub part_size: i64,
    pub part_count: i32,
    pub progress: i32,
    pub error: String,
    /// Times a worker started processing this asset.
    pub attempts: i32,
    /// Main rendition. For private files this is filled in on read with a
    /// presigned URL (the column itself stays empty).
    pub url: String,
    pub image: Option<serde_json::Value>,
    pub video: Option<serde_json::Value>,
    pub waveform: Option<Vec<u8>>,
    /// How many holders use the asset (see the module documentation).
    pub holder_count: i32,
    pub attrs: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub ready_at: Option<DateTime<Utc>>,
}

pub const ASSET_COLUMNS: &str = "id, owner_id, purpose, kind, status, content_type, size_bytes, file_name, sha256, \
     original_key, multipart_id, part_size, part_count, progress, error, attempts, url, image, video, waveform, holder_count, attrs, created_at, ready_at";

impl AssetRow {
    pub fn is_ready(&self) -> bool {
        self.status == AssetStatus::ASSET_STATUS_READY as i32
    }

    /// Whether anything holds the asset (as of when the row was read).
    pub fn is_attached(&self) -> bool {
        self.holder_count > 0
    }

    pub fn has_status(&self, status: AssetStatus) -> bool {
        self.status == status as i32
    }

    pub fn has_kind(&self, kind: AssetKind) -> bool {
        self.kind == kind as i32
    }

    /// The purpose as an enum (`UNSPECIFIED` for values this build does not know).
    pub fn asset_purpose(&self) -> AssetPurpose {
        buffa::EnumValue::<AssetPurpose>::from(self.purpose).as_known().unwrap_or(AssetPurpose::ASSET_PURPOSE_UNSPECIFIED)
    }

    /// The kind as an enum (`UNSPECIFIED` for values this build does not know).
    pub fn asset_kind(&self) -> AssetKind {
        buffa::EnumValue::<AssetKind>::from(self.kind).as_known().unwrap_or(AssetKind::ASSET_KIND_UNSPECIFIED)
    }

    /// Whether the asset is a file served from the private bucket (through
    /// presigned URLs) rather than from the public media bucket.
    pub fn is_private_file(&self) -> bool {
        self.has_kind(AssetKind::ASSET_KIND_FILE) && !crate::recipe::publishes_files(self.asset_purpose())
    }

    pub fn image(&self) -> Option<Image> {
        self.image.clone().and_then(|v| serde_json::from_value(v).ok())
    }

    pub fn video(&self) -> Option<VideoInfo> {
        self.video.clone().and_then(|v| serde_json::from_value(v).ok())
    }

    /// The asset as its owner sees it.
    pub fn to_proto(&self) -> Asset {
        Asset {
            id: self.id.to_string(),
            owner_id: self.owner_id.to_string(),
            purpose: self.purpose.into(),
            kind: self.kind.into(),
            status: self.status.into(),
            content_type: self.content_type.clone(),
            size_bytes: self.size_bytes,
            file_name: self.file_name.clone(),
            created_at: ts(self.created_at),
            progress: self.progress,
            error: self.error.clone(),
            url: self.url.clone(),
            image: self.image().into(),
            video: self.video().into(),
            waveform: self.waveform.clone().unwrap_or_default(),
            ..Default::default()
        }
    }

    /// The asset as anyone else sees it: no owner, no file name, and no URL
    /// when the file is private. Who uploaded a file is for the content
    /// around it to say (the sender of a message, the author of a note);
    /// where that content is anonymous, the file must not give its author
    /// away.
    pub fn to_public_proto(&self) -> Asset {
        let mut asset = self.to_proto();
        asset.owner_id.clear();
        asset.file_name.clear();
        if self.is_private_file() {
            asset.url.clear();
        }
        asset
    }

    /// The asset as an attachment embedded in messages, notes and stories.
    pub fn to_attachment(&self) -> Attachment {
        Attachment {
            asset_id: self.id.to_string(),
            kind: self.kind.into(),
            url: self.url.clone(),
            content_type: self.content_type.clone(),
            size_bytes: self.size_bytes,
            file_name: self.file_name.clone(),
            image: self.image().into(),
            video: self.video().into(),
            waveform: self.waveform.clone().unwrap_or_default(),
            status: self.status.into(),
            ..Default::default()
        }
    }

    /// Gives a ready private file its temporary download URL.
    pub(crate) async fn sign_private_url(&mut self, core: &Core) -> AppResult<()> {
        if self.is_ready() && self.is_private_file() {
            self.url = store::presigner(core).presign_download(&self.original_key, &self.file_name, &self.content_type, PRIVATE_URL_TTL).await?;
        }
        Ok(())
    }
}

/// The row as stored, without a presigned URL. `None` for deleted assets.
pub(crate) async fn load(core: &Core, asset_id: Uuid) -> AppResult<Option<AssetRow>> {
    let row = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ASSET_COLUMNS} FROM media_assets WHERE id = $1 AND deleted_at IS NULL"
    )))
    .bind(asset_id)
    .fetch_optional(&core.db)
    .await?;
    Ok(row)
}

pub async fn find(core: &Core, asset_id: Uuid) -> AppResult<Option<AssetRow>> {
    let Some(mut row) = load(core, asset_id).await? else { return Ok(None) };
    row.sign_private_url(core).await?;
    Ok(Some(row))
}

pub async fn find_many(core: &Core, asset_ids: &[Uuid]) -> AppResult<Vec<AssetRow>> {
    if asset_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut rows: Vec<AssetRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ASSET_COLUMNS} FROM media_assets WHERE id = ANY($1) AND deleted_at IS NULL"
    )))
    .bind(asset_ids)
    .fetch_all(&core.db)
    .await?;
    // Preserve the caller's order.
    rows.sort_by_key(|r| asset_ids.iter().position(|id| *id == r.id).unwrap_or(usize::MAX));
    for row in &mut rows {
        row.sign_private_url(core).await?;
    }
    Ok(rows)
}

/// Resolves an asset a client wants to attach to something.
///
/// Fails unless the asset exists, belongs to `owner_id`, was uploaded for one
/// of `purposes`, and (when `must_be_ready`) finished processing.
pub async fn require(core: &Core, asset_id: Uuid, owner_id: Uuid, purposes: &[AssetPurpose], must_be_ready: bool) -> AppResult<AssetRow> {
    let row = find(core, asset_id).await?.or_not_found("ASSET_NOT_FOUND", "asset not found")?;
    ensure!(row.owner_id == owner_id, not_found, "ASSET_NOT_FOUND", "asset not found");
    ensure!(
        purposes.is_empty() || purposes.iter().any(|p| *p as i32 == row.purpose),
        invalid,
        "ASSET_WRONG_PURPOSE",
        "this upload cannot be used here"
    );
    if row.has_status(AssetStatus::ASSET_STATUS_FAILED) {
        bail!(precondition, "ASSET_FAILED", "the upload could not be processed");
    }
    ensure!(!must_be_ready || row.is_ready(), precondition, "ASSET_NOT_READY", "the upload is still being processed");
    Ok(row)
}

/// Shorthand for [`require`] with `must_be_ready = true`.
pub async fn require_ready(core: &Core, asset_id: Uuid, owner_id: Uuid, purposes: &[AssetPurpose]) -> AppResult<AssetRow> {
    require(core, asset_id, owner_id, purposes, true).await
}

/// Resolves an image asset and returns its `Image` as stored JSON, ready to
/// be copied into another table's `jsonb` column. An animation qualifies
/// too: its `Image` is the still poster.
pub async fn require_image_json(core: &Core, asset_id: Uuid, owner_id: Uuid, purposes: &[AssetPurpose]) -> AppResult<serde_json::Value> {
    let row = require_ready(core, asset_id, owner_id, purposes).await?;
    ensure!(
        row.has_kind(AssetKind::ASSET_KIND_IMAGE) || row.has_kind(AssetKind::ASSET_KIND_ANIMATION),
        invalid,
        "ASSET_NOT_IMAGE",
        "an image is required"
    );
    row.image.or_not_found("ASSET_NOT_READY", "the image has not been processed")
}

/// Parses an `Image` stored as proto JSON in a `jsonb` column.
pub fn image_from_json(value: Option<&serde_json::Value>) -> Option<Image> {
    value.and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// Renews the download URLs of private files in stored attachments. Call it
/// on attachments read back from your own tables before returning them;
/// attachments of other kinds (and of deleted assets) are left untouched.
pub async fn refresh_attachments(core: &Core, attachments: &mut [Attachment]) -> AppResult<()> {
    let file = AssetKind::ASSET_KIND_FILE;
    let ids: Vec<Uuid> = attachments.iter().filter(|a| a.kind == file).filter_map(|a| Uuid::parse_str(&a.asset_id).ok()).collect();
    if ids.is_empty() {
        return Ok(());
    }
    let fresh: HashMap<String, String> =
        find_many(core, &ids).await?.into_iter().filter(|row| row.is_private_file()).map(|row| (row.id.to_string(), row.url)).collect();
    for attachment in attachments.iter_mut().filter(|a| a.kind == file) {
        if let Some(url) = fresh.get(&attachment.asset_id) {
            attachment.url.clone_from(url);
        }
    }
    Ok(())
}

/// The uploaded bytes of an asset, for domains that parse small uploads
/// themselves (e.g. subtitle files). Refuses anything larger than `max_bytes`.
pub async fn read_original(core: &Core, row: &AssetRow, max_bytes: i64) -> AppResult<Bytes> {
    ensure!(row.size_bytes <= max_bytes, invalid, "FILE_TOO_LARGE", "the file is too large to read");
    core.storage.get(Bucket::Private, &row.original_key).await
}
