//! Shorts through the real media pipeline: fixtures generated with ffmpeg
//! are uploaded the way a client does it and transcoded by the media
//! domain; the short follows through the `media.asset.*` events, and its
//! cover frame is cut out of the upload and processed as an image.
mod common;

use std::path::{Path, PathBuf};

use common::*;
use tokuly_core::storage::Bucket;
use tokuly_media::assets;
use tokuly_proto::{
    common::Visibility,
    media::{
        Asset, AssetPurpose, AssetStatus, CompleteUploadRequest, CompleteUploadResponse, CreateUploadRequest,
        CreateUploadResponse,
    },
    shorts::*,
};
use tokuly_shorts::events::CoverJob;
use tokuly_testkit::TestUser;

/// A short H.264 + AAC test pattern of the given frame size.
async fn fixture(dir: &Path, name: &str, size: &str, seconds: u32) -> PathBuf {
    let path = dir.join(name);
    let source = format!("testsrc2=size={size}:rate=15:duration={seconds}");
    let tone = format!("sine=frequency=440:duration={seconds}");
    let output = tokio::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", &source, "-f", "lavfi", "-i", &tone])
        .args(["-c:a", "aac", "-shortest", "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
        .arg(&path)
        .output()
        .await
        .expect("ffmpeg must be on PATH to run the pipeline tests");
    assert!(output.status.success(), "fixture ffmpeg failed: {}", String::from_utf8_lossy(&output.stderr));
    path
}

/// CreateUpload + PUT + CompleteUpload, like a browser.
async fn upload(ctx: &Ctx, user: &TestUser, file_name: &str, bytes: Vec<u8>) -> Asset {
    let request = CreateUploadRequest {
        purpose: AssetPurpose::ASSET_PURPOSE_SHORT.into(),
        file_name: file_name.to_owned(),
        content_type: "video/mp4".into(),
        size_bytes: bytes.len() as i64,
        ..Default::default()
    };
    let created: CreateUploadResponse =
        ctx.app.call("tokuly.media.v1.MediaService/CreateUpload", Some(user), &request).await.expect("CreateUpload");
    assert!(!created.put_url.is_empty(), "small fixtures are single-request uploads");
    let mut put = reqwest::Client::new().put(&created.put_url).body(bytes);
    for (name, value) in &created.put_headers {
        put = put.header(name, value);
    }
    let stored = put.send().await.expect("PUT to object store");
    assert!(stored.status().is_success(), "upload PUT failed: {}", stored.status());
    let request = CompleteUploadRequest { upload_id: created.upload_id.clone(), ..Default::default() };
    let completed: CompleteUploadResponse = ctx
        .app
        .call("tokuly.media.v1.MediaService/CompleteUpload", Some(user), &request)
        .await
        .expect("CompleteUpload");
    completed.asset.as_option().cloned().expect("the uploaded asset")
}

/// Whether the object behind a media URL exists.
async fn stored(ctx: &Ctx, url: &str) -> bool {
    let key = url.strip_prefix(&ctx.app.core.storage.media_url("")).expect("a media URL").to_owned();
    ctx.app.core.storage.head(Bucket::Media, &key).await.unwrap().is_some()
}

#[tokio::test]
async fn an_upload_becomes_a_short_with_the_cover_frame_its_creator_chose() {
    let ctx = start().await;
    ctx.app.core.storage.ensure_buckets().await.unwrap();
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let dir = tempfile::tempdir().unwrap();
    let source = fixture(dir.path(), "vertical.mp4", "180x320", 3).await;

    let asset = upload(&ctx, &alice, "はじめてのショート.mp4", tokio::fs::read(&source).await.unwrap()).await;
    assert_eq!(asset.status, AssetStatus::ASSET_STATUS_PROCESSING);

    // The short is created (and captioned) while the upload is still being processed.
    let tag = unique("初投稿");
    let request = CreateShortRequest {
        channel_id: channel.summary.id.clone(),
        asset_id: asset.id.clone(),
        caption: format!("はじめてのショート! #{tag}"),
        visibility: Visibility::VISIBILITY_PUBLIC.into(),
        cover: buffa::MessageField::some(CoverInput {
            cover: Some(cover_input::Cover::OffsetMs(1500)),
            ..Default::default()
        }),
        ..Default::default()
    };
    let draft = ctx.create(&alice, &request).await.unwrap();
    assert_eq!(draft.status, ShortStatus::SHORT_STATUS_PROCESSING);
    assert!(draft.playback.as_option().is_none() && draft.cover.as_option().is_none());
    assert_eq!(ctx.get(Some(&bob), &draft.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");

    ctx.media.process_now(id(&asset.id)).await.unwrap();
    // The outbox relay would now deliver `media.asset.ready` to the shorts domain.
    assert_eq!(ctx.deliver("media.asset.ready", alice.id).await, 1);

    let live = ctx.get(Some(&bob), &draft.id).await.expect("the short went live");
    assert_eq!(live.status, ShortStatus::SHORT_STATUS_PUBLISHED);
    assert!(live.height > live.width, "{}x{}", live.width, live.height);
    assert!((live.duration_seconds - 3.0).abs() < 0.3, "{}", live.duration_seconds);
    let playback = live.playback.as_option().expect("streams");
    assert!(playback.hls_url.ends_with("master.m3u8") && !playback.renditions.is_empty() && playback.has_audio);
    // What the player will fetch really exists.
    assert!(stored(&ctx, &playback.hls_url).await);
    assert!(live.sound.as_option().is_some(), "a video with audio has an original sound");
    assert_eq!(live.hashtags, [tag.clone()]);
    // Until the chosen frame is cut, the poster frame stands in.
    assert_eq!(live.cover.as_option().expect("a cover").asset_id, asset.id);
    let studio = ctx.get(Some(&alice), &draft.id).await.unwrap().studio.as_option().cloned().unwrap();
    assert!(studio.cover_pending && studio.cover_offset_ms == 1500);

    // The cover job: the frame at 1.5 s becomes an image asset of the owner.
    let job = CoverJob { short_id: id(&draft.id), offset_ms: 1500 };
    ctx.shorts.cut_cover(job.clone()).await.unwrap();
    let frame = ctx.row(&draft.id).await.cover_pending_asset_id.expect("a frame waiting to be processed");
    let frame_asset = assets::find(&ctx.app.core, frame).await.unwrap().expect("the frame asset");
    assert_eq!((frame_asset.owner_id, frame_asset.purpose), (alice.id, AssetPurpose::ASSET_PURPOSE_THUMBNAIL as i32));
    assert!(frame_asset.is_attached(), "held by the short from the start, so the janitor leaves it alone");
    ctx.media.process_now(frame).await.unwrap();
    ctx.deliver("media.asset.ready", alice.id).await;

    let covered = ctx.get(Some(&alice), &draft.id).await.unwrap();
    let cover = covered.cover.as_option().expect("the chosen frame");
    assert_eq!(
        (cover.asset_id.as_str(), covered.cover_kind),
        (frame.to_string().as_str(), CoverKind::COVER_KIND_FRAME.into())
    );
    assert!(cover.height > cover.width && !cover.thumbhash.is_empty(), "a portrait image with a placeholder");
    assert!(stored(&ctx, &cover.url).await);
    assert!(!covered.studio.as_option().unwrap().cover_pending);
    // The same job again (a retry) changes nothing.
    ctx.shorts.cut_cover(job).await.unwrap();
    assert_eq!(ctx.row(&draft.id).await.cover_asset_id, Some(frame));
    assert!(ctx.row(&draft.id).await.cover_pending_asset_id.is_none());

    // Another offset: the old frame is let go and deleted, the new one takes its place.
    let move_cover = UpdateShortRequest {
        short_id: draft.id.clone(),
        cover: buffa::MessageField::some(CoverInput {
            cover: Some(cover_input::Cover::OffsetMs(999_000)),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(ctx.update(&alice, &move_cover).await.unwrap_err().reason, "COVER_OFFSET_INVALID");
    let move_cover = UpdateShortRequest {
        cover: buffa::MessageField::some(CoverInput {
            cover: Some(cover_input::Cover::OffsetMs(100_000)),
            ..Default::default()
        }),
        ..move_cover
    };
    ctx.update(&alice, &move_cover).await.unwrap();
    assert!(assets::find(&ctx.app.core, frame).await.unwrap().is_none());
    // Past the end of a three second video: the last picture.
    ctx.shorts.cut_cover(CoverJob { short_id: id(&draft.id), offset_ms: 100_000 }).await.unwrap();
    let last_frame = ctx.row(&draft.id).await.cover_pending_asset_id.expect("the last frame");
    ctx.media.process_now(last_frame).await.unwrap();
    ctx.deliver("media.asset.ready", alice.id).await;
    assert_eq!(
        ctx.get(Some(&bob), &draft.id).await.unwrap().cover.as_option().unwrap().asset_id,
        last_frame.to_string()
    );

    // Deleting the short removes its upload and its frame.
    ctx.delete(&alice, &draft.id).await;
    assert!(assets::find(&ctx.app.core, id(&asset.id)).await.unwrap().is_none());
    assert!(assets::find(&ctx.app.core, last_frame).await.unwrap().is_none());
}

#[tokio::test]
async fn a_landscape_upload_is_cropped_and_a_panorama_is_refused() {
    let ctx = start().await;
    ctx.app.core.storage.ensure_buckets().await.unwrap();
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let dir = tempfile::tempdir().unwrap();
    let landscape = fixture(dir.path(), "landscape.mp4", "320x180", 2).await;
    let panorama = fixture(dir.path(), "panorama.mp4", "640x180", 2).await;

    let mut drafts = Vec::new();
    for (path, caption) in [(&landscape, "横長"), (&panorama, "パノラマ")] {
        let asset = upload(&ctx, &alice, "clip.mp4", tokio::fs::read(path).await.unwrap()).await;
        let draft = ctx.short_with(&alice, &channel, caption, |request| request.asset_id = asset.id.clone()).await;
        assert_eq!(draft.status, ShortStatus::SHORT_STATUS_PROCESSING);
        ctx.media.process_now(id(&asset.id)).await.unwrap();
        drafts.push(draft);
    }
    assert_eq!(ctx.deliver("media.asset.ready", alice.id).await, 2);

    // 16:9 is accepted: players centre-crop it.
    let cropped = ctx.get(Some(&bob), &drafts[0].id).await.expect("a 16:9 short is published");
    assert!(cropped.width > cropped.height && cropped.status == ShortStatus::SHORT_STATUS_PUBLISHED);
    // Wider than that cannot fill a phone held upright.
    assert_eq!(ctx.get(Some(&bob), &drafts[1].id).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    let refused = ctx.get(Some(&alice), &drafts[1].id).await.unwrap();
    assert_eq!(refused.status, ShortStatus::SHORT_STATUS_FAILED);
    assert_eq!(refused.studio.as_option().unwrap().processing_error, "SHORT_NOT_VERTICAL");
    let told = ctx.notifications(alice.id).await;
    assert_eq!(told.len(), 1);
    assert_eq!(
        (told[0].0.as_str(), told[0].1["data"]["error"].as_str()),
        ("shorts.processing_failed", Some("SHORT_NOT_VERTICAL"))
    );
}
