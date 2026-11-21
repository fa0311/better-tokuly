//! Fixtures for the integration tests of domains that attach media.
//!
//! These insert finished assets straight into the database, without storing
//! or processing any bytes, so a chat or video test does not have to run the
//! upload pipeline to get something it can attach. The URLs they carry are
//! well-formed but point at nothing.
use tokuly_core::prelude::*;
use tokuly_proto::{
    common::{Image, ImageVariant},
    media::{AssetKind, AssetPurpose, AssetStatus, Rendition, VideoInfo},
};

use crate::{
    assets::{self, AssetRow},
    store,
};

fn to_json<T: serde::Serialize>(value: &Option<T>) -> AppResult<Option<serde_json::Value>> {
    value.as_ref().map(serde_json::to_value).transpose().map_err(|e| AppError::internal(anyhow::anyhow!("fixture encode: {e}")))
}

fn sample_image(core: &Core, asset_id: Uuid) -> Image {
    let variant = |size: u32| ImageVariant { size: size as i32, url: core.storage.media_url(&store::image_key(asset_id, size, "0000000000000000")), ..Default::default() };
    let variants = vec![variant(1024), variant(512), variant(256), variant(96)];
    Image {
        asset_id: asset_id.to_string(),
        url: variants[0].url.clone(),
        width: 1024,
        height: 768,
        thumbhash: "3OcRJYB4d3h/iIeHeEh3eIhw+j3A".to_owned(),
        dominant_color: "#4a90d9".to_owned(),
        variants,
        ..Default::default()
    }
}

fn sample_video(core: &Core, asset_id: Uuid, poster: Image) -> VideoInfo {
    let prefix = store::video_prefix(asset_id);
    let url = |path: &str| core.storage.media_url(&format!("{prefix}{path}"));
    VideoInfo {
        duration_seconds: 12.0,
        width: 1280,
        height: 720,
        frame_rate: 30.0,
        hls_url: url("hls/master.m3u8"),
        mp4_url: url("clip.mp4"),
        poster: Some(poster).into(),
        storyboard_url: url("storyboard/storyboard.vtt"),
        renditions: vec![Rendition { name: "720p".into(), width: 1280, height: 720, bitrate_kbps: 2500, codec: "h264".into(), url: url("hls/720p/index.m3u8"), ..Default::default() }],
        has_audio: true,
        preview_url: url("preview.webp"),
        loudness_lufs: -16.0,
        ..Default::default()
    }
}

/// Inserts a READY asset of `kind` owned by `owner_id`, shaped like the
/// result of real processing: images and animations have an `Image`, videos
/// a `VideoInfo` with poster, audio a waveform, files nothing.
pub async fn ready_asset(core: &Core, owner_id: Uuid, purpose: AssetPurpose, kind: AssetKind) -> AppResult<AssetRow> {
    let asset_id = new_id();
    let image = sample_image(core, asset_id);
    let (content_type, file_name, url, image, video, waveform) = match kind {
        AssetKind::ASSET_KIND_IMAGE => ("image/jpeg", "photo.jpg", image.url.clone(), Some(image), None, None),
        AssetKind::ASSET_KIND_ANIMATION => {
            let url = core.storage.media_url(&store::animation_key(asset_id, "0000000000000000"));
            ("image/gif", "animation.gif", url, Some(image), None, None)
        }
        AssetKind::ASSET_KIND_VIDEO => {
            let video = sample_video(core, asset_id, image.clone());
            ("video/mp4", "video.mp4", video.mp4_url.clone(), Some(image), Some(video), None)
        }
        AssetKind::ASSET_KIND_AUDIO => {
            let url = core.storage.media_url(&format!("{}voice.m4a", store::audio_prefix(asset_id)));
            let video = VideoInfo { duration_seconds: 4.0, has_audio: true, mp4_url: url.clone(), loudness_lufs: -20.0, ..Default::default() };
            let waveform: Vec<u8> = (0..64u32).map(|i| (i * 4) as u8).collect();
            ("audio/m4a", "voice.m4a", url, None, Some(video), Some(waveform))
        }
        _ => ("application/pdf", "document.pdf", String::new(), None, None, None),
    };
    sqlx::query(
        "INSERT INTO media_assets
             (id, owner_id, purpose, kind, status, content_type, size_bytes, file_name, original_key, progress, url, image, video, waveform, ready_at)
         VALUES ($1, $2, $3, $4, $5, $6, 123456, $7, $8, 100, $9, $10, $11, $12, now())",
    )
    .bind(asset_id)
    .bind(owner_id)
    .bind(purpose as i32)
    .bind(kind as i32)
    .bind(AssetStatus::ASSET_STATUS_READY as i32)
    .bind(content_type)
    .bind(file_name)
    .bind(store::original_key(asset_id, ""))
    .bind(url)
    .bind(to_json(&image)?)
    .bind(to_json(&video)?)
    .bind(waveform)
    .execute(&core.db)
    .await?;
    assets::find(core, asset_id).await?.or_not_found("ASSET_NOT_FOUND", "asset not found")
}

/// Shorthand for a READY still image.
pub async fn ready_image(core: &Core, owner_id: Uuid, purpose: AssetPurpose) -> AppResult<AssetRow> {
    ready_asset(core, owner_id, purpose, AssetKind::ASSET_KIND_IMAGE).await
}

/// Shorthand for a READY video.
pub async fn ready_video(core: &Core, owner_id: Uuid, purpose: AssetPurpose) -> AppResult<AssetRow> {
    ready_asset(core, owner_id, purpose, AssetKind::ASSET_KIND_VIDEO).await
}
