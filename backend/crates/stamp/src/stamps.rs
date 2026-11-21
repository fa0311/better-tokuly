//! Stamps: making them from uploads, loading them for a viewer, ordering
//! and deleting them.
//!
//! A stamp serves the processed files of a media asset. To keep those files
//! alive it *holds* the asset in the media domain under the name
//! `stamp:<stamp_id>` ([`holder`]): [`insert`] attaches the hold in the
//! transaction that stores the stamp, [`delete`] and [`delete_of_pack`]
//! release it in the transaction that removes it. Every stamp is created
//! and deleted through these functions, so holds and stamps cannot drift
//! apart. While a stamp exists its uploader cannot delete the upload, and
//! the upload outlives the uploader's account.
use std::collections::HashSet;

use sqlx::{Postgres, Transaction};
use tokuly_core::prelude::*;
use tokuly_media::assets::{self, AssetRow};
use tokuly_proto::{
    common::{Image, StampFormat},
    media::{AssetKind, AssetPurpose},
};

use crate::{
    model::{StampView, stamp_view_select},
    validate::StampText,
};

pub const FORMAT_STATIC: i32 = StampFormat::STAMP_FORMAT_STATIC as i32;
pub const FORMAT_ANIMATED: i32 = StampFormat::STAMP_FORMAT_ANIMATED as i32;
pub const FORMAT_LOTTIE: i32 = StampFormat::STAMP_FORMAT_LOTTIE as i32;
const _: () = assert!(FORMAT_STATIC == 1, "packs::refresh_derived treats format 1 as the only still format");

pub fn not_found() -> AppError {
    AppError::not_found("STAMP_NOT_FOUND", "stamp not found")
}

/// The picture of a stamp: everything that is fixed when it is created.
#[derive(Debug, Clone)]
pub struct StampSource {
    pub format: i32,
    /// The upload it comes from; `None` for built-in art.
    pub asset_id: Option<Uuid>,
    /// `tokuly.common.v1.Image` as proto JSON.
    pub image: serde_json::Value,
    pub image_url: String,
    pub animation_url: String,
    pub width: i32,
    pub height: i32,
}

/// What a stamp needs to know about a processed upload.
struct Upload<'a> {
    id: Uuid,
    kind: AssetKind,
    content_type: &'a str,
    file_name: &'a str,
    /// The main rendition: the animation file for animations and Lottie.
    url: &'a str,
    /// The processed still image (the poster, for animations).
    image: Option<Image>,
}

impl StampSource {
    /// Builds the picture from a processed upload.
    pub fn from_asset(asset: &AssetRow) -> AppResult<Self> {
        Self::from_upload(Upload {
            id: asset.id,
            kind: buffa::EnumValue::<AssetKind>::from(asset.kind)
                .as_known()
                .unwrap_or(AssetKind::ASSET_KIND_UNSPECIFIED),
            content_type: &asset.content_type,
            file_name: &asset.file_name,
            url: &asset.url,
            image: asset.image(),
        })
    }

    /// Still images are served from the processed image; animations keep
    /// their animation file and use the processed image as static preview.
    fn from_upload(upload: Upload<'_>) -> AppResult<Self> {
        let format = format_of(&upload)?;
        let preview = upload.image.filter(|image| !image.url.is_empty());
        let (image, animation_url) = if format == FORMAT_STATIC {
            (preview.ok_or_else(|| invalid_asset("the image has not been processed"))?, String::new())
        } else {
            ensure!(!upload.url.is_empty(), invalid, "STAMP_ASSET_INVALID", "the animation has not been processed");
            // Without a still preview an animated image can stand in for itself; a Lottie file cannot.
            let stand_in = if format == FORMAT_ANIMATED { upload.url.to_owned() } else { String::new() };
            let image = preview.unwrap_or_else(|| Image {
                asset_id: upload.id.to_string(),
                url: stand_in,
                ..Default::default()
            });
            (image, upload.url.to_owned())
        };
        Ok(Self {
            format,
            asset_id: Some(upload.id),
            width: image.width,
            height: image.height,
            image_url: image.url.clone(),
            animation_url,
            image: serde_json::to_value(&image)
                .map_err(|e| AppError::internal(anyhow::anyhow!("image encode: {e}")))?,
        })
    }
}

fn invalid_asset(message: &'static str) -> AppError {
    AppError::invalid("STAMP_ASSET_INVALID", message)
}

/// Which stamp format an upload is. The media domain turns still images
/// into `IMAGE` assets, animated GIF/APNG/WebP into `ANIMATION` assets and
/// publishes dotLottie archives as `FILE` assets.
fn format_of(upload: &Upload<'_>) -> AppResult<i32> {
    match upload.kind {
        AssetKind::ASSET_KIND_IMAGE => Ok(FORMAT_STATIC),
        AssetKind::ASSET_KIND_ANIMATION => Ok(FORMAT_ANIMATED),
        AssetKind::ASSET_KIND_FILE if is_lottie(upload.content_type, upload.file_name) => Ok(FORMAT_LOTTIE),
        _ => Err(invalid_asset("a stamp is an image, an animated image or a dotLottie file")),
    }
}

/// dotLottie has no registered media type; the media domain labels it
/// `application/zip+dotlottie`, and the file extension is `.lottie`.
fn is_lottie(content_type: &str, file_name: &str) -> bool {
    content_type.to_ascii_lowercase().contains("lottie") || file_name.to_ascii_lowercase().ends_with(".lottie")
}

/// Resolves an upload the caller wants to turn into a stamp: it must be
/// theirs, uploaded for stamps, and processed.
pub async fn source_from_upload(core: &Core, asset_id: &str, user_id: Uuid) -> AppResult<StampSource> {
    let asset = assets::require_ready(core, parse_id(asset_id)?, user_id, &[AssetPurpose::ASSET_PURPOSE_STAMP]).await?;
    StampSource::from_asset(&asset)
}

/// Where a new stamp goes.
#[derive(Debug, Clone, Copy)]
pub enum StampHome {
    /// Into a pack; `approved` says whether it needs no review.
    Pack { pack_id: Uuid, approved: bool },
    /// Into a user's personal stamps.
    Personal { owner_id: Uuid },
}

/// The name under which a stamp holds its upload in the media domain.
pub fn holder(stamp_id: Uuid) -> String {
    format!("stamp:{stamp_id}")
}

/// Inserts a stamp at `position`, takes its hold on the upload, and returns
/// its ID. Fails with `ASSET_NOT_FOUND` when the upload was deleted since it
/// was resolved.
pub async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    home: StampHome,
    source: &StampSource,
    text: &StampText,
    position: i32,
) -> AppResult<Uuid> {
    let (pack_id, owner_id, approved) = match home {
        StampHome::Pack { pack_id, approved } => (Some(pack_id), None, approved),
        StampHome::Personal { owner_id } => (None, Some(owner_id), false),
    };
    let id = sqlx::query_scalar(
        "INSERT INTO stamps (pack_id, owner_id, name, emoji, keywords, format, asset_id, image, image_url, animation_url,
                             width, height, position, approved_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, CASE WHEN $14 THEN now() END)
         RETURNING id",
    )
    .bind(pack_id)
    .bind(owner_id)
    .bind(&text.name)
    .bind(&text.emoji)
    .bind(&text.keywords)
    .bind(source.format)
    .bind(source.asset_id)
    .bind(&source.image)
    .bind(&source.image_url)
    .bind(&source.animation_url)
    .bind(source.width)
    .bind(source.height)
    .bind(position)
    .bind(approved)
    .fetch_one(&mut **tx)
    .await?;
    if let Some(asset_id) = source.asset_id {
        assets::attach(&mut **tx, asset_id, &holder(id)).await?;
    }
    Ok(id)
}

/// A stamp that was just deleted.
#[derive(Debug, sqlx::FromRow)]
pub struct DeletedStamp {
    pub id: Uuid,
    /// Set for personal stamps.
    pub owner_id: Option<Uuid>,
    pub asset_id: Option<Uuid>,
}

/// Lets go of the uploads deleted stamps held. An upload nothing else holds
/// is then collected by the media domain.
async fn release(tx: &mut Transaction<'_, Postgres>, mut deleted: Vec<DeletedStamp>) -> AppResult<Vec<DeletedStamp>> {
    // Uploads are released in a fixed order, so two deletions that share
    // uploads cannot wait on each other.
    deleted.sort_by_key(|stamp| stamp.asset_id);
    for stamp in &deleted {
        if let Some(asset_id) = stamp.asset_id {
            assets::detach(&mut **tx, asset_id, &holder(stamp.id)).await?;
        }
    }
    Ok(deleted)
}

/// Deletes stamps and releases their uploads. Returns what was deleted
/// (IDs that name no stamp are skipped).
pub async fn delete(tx: &mut Transaction<'_, Postgres>, stamp_ids: &[Uuid]) -> AppResult<Vec<DeletedStamp>> {
    let deleted: Vec<DeletedStamp> =
        sqlx::query_as("DELETE FROM stamps WHERE id = ANY($1) RETURNING id, owner_id, asset_id")
            .bind(stamp_ids)
            .fetch_all(&mut **tx)
            .await?;
    release(tx, deleted).await
}

/// Deletes every stamp of a pack and releases their uploads. Call it before
/// deleting the pack itself: the database would remove the stamps too, but
/// not let go of their uploads.
pub async fn delete_of_pack(tx: &mut Transaction<'_, Postgres>, pack_id: Uuid) -> AppResult<Vec<DeletedStamp>> {
    let deleted: Vec<DeletedStamp> =
        sqlx::query_as("DELETE FROM stamps WHERE pack_id = $1 RETURNING id, owner_id, asset_id")
            .bind(pack_id)
            .fetch_all(&mut **tx)
            .await?;
    release(tx, deleted).await
}

/// One stamp as `viewer_id` relates to it.
pub async fn view(core: &Core, viewer_id: Option<Uuid>, stamp_id: Uuid) -> AppResult<Option<StampView>> {
    Ok(sqlx::query_as(concat!(stamp_view_select!(), "WHERE s.id = $2"))
        .bind(viewer_id)
        .bind(stamp_id)
        .fetch_optional(&core.db)
        .await?)
}

/// Every stamp of a pack, in the creator's order.
pub async fn views_of_pack(core: &Core, viewer_id: Option<Uuid>, pack_id: Uuid) -> AppResult<Vec<StampView>> {
    Ok(sqlx::query_as(concat!(stamp_view_select!(), "WHERE s.pack_id = $2 ORDER BY s.position, s.id"))
        .bind(viewer_id)
        .bind(pack_id)
        .fetch_all(&core.db)
        .await?)
}

/// Changes the text of a stamp; `None` leaves a field as it is.
pub async fn update_text<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    stamp_id: Uuid,
    name: Option<String>,
    emoji: Option<String>,
    keywords: Option<Vec<String>>,
) -> AppResult<()> {
    sqlx::query("UPDATE stamps SET name = COALESCE($2, name), emoji = COALESCE($3, emoji), keywords = COALESCE($4, keywords) WHERE id = $1")
        .bind(stamp_id)
        .bind(name)
        .bind(emoji)
        .bind(keywords)
        .execute(executor)
        .await?;
    Ok(())
}

/// Checks that `requested` lists exactly the IDs in `current`, each once.
pub fn ensure_same_set(current: &[Uuid], requested: &[Uuid]) -> AppResult<()> {
    let wanted: HashSet<Uuid> = requested.iter().copied().collect();
    let same = wanted.len() == requested.len()
        && requested.len() == current.len()
        && current.iter().all(|id| wanted.contains(id));
    ensure!(same, invalid, "ORDER_MISMATCH", "the new order must list every item exactly once");
    Ok(())
}

/// Numbers the stamps 0, 1, 2… in the order given.
pub async fn apply_order(tx: &mut Transaction<'_, Postgres>, ordered_ids: &[Uuid]) -> AppResult<()> {
    sqlx::query(
        "UPDATE stamps s SET position = (o.ordinal - 1)::int
         FROM unnest($1::uuid[]) WITH ORDINALITY AS o(id, ordinal)
         WHERE s.id = o.id",
    )
    .bind(ordered_ids)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upload<'a>(
        kind: AssetKind,
        content_type: &'a str,
        file_name: &'a str,
        image: Option<Image>,
        url: &'a str,
    ) -> Upload<'a> {
        Upload { id: Uuid::now_v7(), kind, content_type, file_name, url, image }
    }

    fn preview() -> Option<Image> {
        Some(Image {
            asset_id: "a".into(),
            url: "https://media.test/s/512.webp".into(),
            width: 512,
            height: 480,
            ..Default::default()
        })
    }

    #[test]
    fn still_images_become_static_stamps() {
        let source = StampSource::from_upload(upload(
            AssetKind::ASSET_KIND_IMAGE,
            "image/png",
            "cat.png",
            preview(),
            "https://media.test/s/512.webp",
        ))
        .unwrap();
        assert_eq!(source.format, FORMAT_STATIC);
        assert_eq!(source.image_url, "https://media.test/s/512.webp");
        assert_eq!(source.animation_url, "");
        assert_eq!((source.width, source.height), (512, 480));
        assert_eq!(source.image["url"], "https://media.test/s/512.webp", "the image is stored as proto JSON");
    }

    #[test]
    fn animations_keep_their_file_and_use_the_poster_as_still() {
        let source = StampSource::from_upload(upload(
            AssetKind::ASSET_KIND_ANIMATION,
            "image/webp",
            "dance.webp",
            preview(),
            "https://media.test/s/anim.webp",
        ))
        .unwrap();
        assert_eq!(source.format, FORMAT_ANIMATED);
        assert_eq!(source.image_url, "https://media.test/s/512.webp");
        assert_eq!(source.animation_url, "https://media.test/s/anim.webp");

        // Without a poster the animation stands in for itself.
        let source = StampSource::from_upload(upload(
            AssetKind::ASSET_KIND_ANIMATION,
            "image/gif",
            "dance.gif",
            None,
            "https://media.test/s/anim.webp",
        ))
        .unwrap();
        assert_eq!(source.image_url, "https://media.test/s/anim.webp");
    }

    #[test]
    fn dot_lottie_files_become_lottie_stamps() {
        for (content_type, file_name) in [("application/zip+dotlottie", "wave"), ("application/zip", "WAVE.LOTTIE")] {
            let source = StampSource::from_upload(upload(
                AssetKind::ASSET_KIND_FILE,
                content_type,
                file_name,
                None,
                "https://media.test/s/wave.lottie",
            ))
            .unwrap();
            assert_eq!(source.format, FORMAT_LOTTIE, "{content_type} {file_name}");
            assert_eq!(source.image_url, "", "a Lottie file is not an image");
            assert_eq!(source.animation_url, "https://media.test/s/wave.lottie");
        }
    }

    #[test]
    fn other_uploads_are_not_stamps() {
        let cases = [
            upload(AssetKind::ASSET_KIND_VIDEO, "video/mp4", "clip.mp4", None, "https://media.test/v.mp4"),
            upload(AssetKind::ASSET_KIND_FILE, "application/zip", "archive.zip", None, "https://media.test/a.zip"),
            // An image whose processed rendition is missing, and an animation without a file.
            upload(AssetKind::ASSET_KIND_IMAGE, "image/png", "cat.png", None, ""),
            upload(AssetKind::ASSET_KIND_ANIMATION, "image/gif", "dance.gif", preview(), ""),
        ];
        for case in cases {
            let label = format!("{:?} {}", case.kind, case.file_name);
            assert_eq!(StampSource::from_upload(case).unwrap_err().reason, "STAMP_ASSET_INVALID", "{label}");
        }
    }

    #[test]
    fn a_new_order_must_be_a_permutation() {
        let (a, b, c) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        assert!(ensure_same_set(&[a, b, c], &[c, a, b]).is_ok());
        assert!(ensure_same_set(&[], &[]).is_ok());
        for bad in [vec![a, b], vec![a, b, b], vec![a, b, c, Uuid::now_v7()], vec![a, a, b, c]] {
            assert_eq!(ensure_same_set(&[a, b, c], &bad).unwrap_err().reason, "ORDER_MISMATCH");
        }
    }
}
