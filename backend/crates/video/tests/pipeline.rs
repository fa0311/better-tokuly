//! Videos through the real media pipeline: a fixture generated with ffmpeg
//! is uploaded the way a client does it, transcoded by the media domain, and
//! the video follows through the `media.asset.*` events.
mod common;

use std::path::{Path, PathBuf};

use common::*;
use tokuly_core::storage::Bucket;
use tokuly_proto::{
    common::Visibility,
    media::{Asset, AssetPurpose, AssetStatus, CompleteUploadRequest, CompleteUploadResponse, CreateUploadRequest, CreateUploadResponse},
    video::*,
};
use tokuly_testkit::TestUser;
use tokuly_video::videos::{self, NewVideo};

/// A short H.264 + AAC test pattern.
async fn fixture(dir: &Path, seconds: u32) -> PathBuf {
    let path = dir.join("fixture.mp4");
    let source = format!("testsrc2=size=320x180:rate=15:duration={seconds}");
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
        purpose: AssetPurpose::ASSET_PURPOSE_VIDEO.into(),
        file_name: file_name.to_owned(),
        content_type: "video/mp4".into(),
        size_bytes: bytes.len() as i64,
        ..Default::default()
    };
    let created: CreateUploadResponse = ctx.app.call("tokuly.media.v1.MediaService/CreateUpload", Some(user), &request).await.expect("CreateUpload");
    assert!(!created.put_url.is_empty(), "small fixtures are single-request uploads");
    let mut put = reqwest::Client::new().put(&created.put_url).body(bytes);
    for (name, value) in &created.put_headers {
        put = put.header(name, value);
    }
    let stored = put.send().await.expect("PUT to object store");
    assert!(stored.status().is_success(), "upload PUT failed: {}", stored.status());
    let request = CompleteUploadRequest { upload_id: created.upload_id.clone(), ..Default::default() };
    let completed: CompleteUploadResponse = ctx.app.call("tokuly.media.v1.MediaService/CompleteUpload", Some(user), &request).await.expect("CompleteUpload");
    completed.asset.as_option().cloned().expect("the uploaded asset")
}

/// Notifications requested for `user_id`, as `(kind, data)`.
async fn notifications(ctx: &Ctx, user_id: uuid::Uuid) -> Vec<(String, serde_json::Value)> {
    ctx.app
        .events("notify.requested")
        .await
        .into_iter()
        .filter(|event| event.data["user_id"] == user_id.to_string())
        .map(|event| (event.data["kind"].as_str().unwrap_or_default().to_owned(), event.data["data"].clone()))
        .collect()
}

#[tokio::test]
async fn an_upload_becomes_a_published_video_once_it_is_processed() {
    let ctx = start().await;
    ctx.app.core.storage.ensure_buckets().await.unwrap();
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let dir = tempfile::tempdir().unwrap();
    let source = fixture(dir.path(), 2).await;

    let asset = upload(&ctx, &alice, "はじめての動画.mp4", tokio::fs::read(&source).await.unwrap()).await;
    assert_eq!(asset.status, AssetStatus::ASSET_STATUS_PROCESSING);

    // The video can be created (and set up) while the upload is still being processed.
    let request = CreateVideoRequest { channel_id: channel.summary.id.clone(), asset_id: asset.id.clone(), visibility: Visibility::VISIBILITY_PUBLIC.into(), ..Default::default() };
    let draft = ctx.create_video(&alice, &request).await.unwrap();
    let video_id = card(&draft).id.clone();
    assert_eq!(card(&draft).title, "はじめての動画", "the title defaults to the file name");
    assert_eq!(card(&draft).processing, ProcessingState::PROCESSING_STATE_PROCESSING);
    assert_eq!(draft.playback_block, PlaybackBlock::PLAYBACK_BLOCK_NOT_READY);
    assert!(draft.playback.as_option().is_none() && card(&draft).thumbnail.as_option().is_none());
    let play = StartPlaybackRequest { video_id: video_id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(PLAYBACK, "StartPlayback", Some(&alice), &play).await, "PLAYBACK_BLOCKED");

    // "Publish" now means: as soon as it is ready.
    let waiting = ctx.publish(&alice, &video_id).await;
    assert_eq!(card(&waiting).status, VideoStatus::VIDEO_STATUS_SCHEDULED);
    assert_eq!(ctx.get_video(Some(&bob), &video_id).await.unwrap_err().reason, "VIDEO_NOT_FOUND");

    let mut frames = ctx.app.listen_user(alice.id).await;
    ctx.media.process_now(id(&asset.id)).await.unwrap();
    // The outbox relay would now deliver `media.asset.ready` to the video domain.
    assert_eq!(ctx.deliver("media.asset.ready", alice.id).await, 1);

    let live = ctx.get_video(Some(&bob), &video_id).await.expect("the video went live");
    let summary = card(&live);
    assert_eq!((summary.status, summary.processing), (VideoStatus::VIDEO_STATUS_PUBLISHED.into(), ProcessingState::PROCESSING_STATE_READY.into()));
    assert!((summary.duration_seconds - 2.0).abs() < 0.3, "{}", summary.duration_seconds);
    assert_eq!(summary.thumbnail.as_option().expect("the poster frame").asset_id, asset.id);
    assert!(summary.published_at.as_option().is_some() && !summary.preview_url.is_empty());
    let playback = live.playback.as_option().expect("streams");
    assert!(playback.hls_url.ends_with("master.m3u8") && !playback.renditions.is_empty() && playback.has_audio);
    // What the player will fetch really exists.
    let key = playback.hls_url.strip_prefix(&ctx.app.core.storage.media_url("")).unwrap().to_owned();
    assert!(ctx.app.core.storage.head(Bucket::Media, &key).await.unwrap().is_some());

    // The owner is told, in the inbox and on their devices; the publication is announced.
    let told = notifications(&ctx, alice.id).await;
    assert_eq!(told.len(), 1);
    assert_eq!((told[0].0.as_str(), told[0].1["published"].as_bool()), ("video.processing_done", Some(true)));
    let updates: Vec<VideoSummary> = pushes(frames.drain(QUIET).await).into_iter().filter_map(|push| if let Push::VideoUpdated(v) = push { Some(*v) } else { None }).collect();
    assert_eq!(updates.len(), 1);
    assert_eq!((updates[0].id.as_str(), updates[0].status), (video_id.as_str(), VideoStatus::VIDEO_STATUS_PUBLISHED.into()));
    assert_eq!(ctx.app.events_by("video.video.published", alice.id).await.len(), 1);

    // A redelivered event changes nothing.
    ctx.deliver("media.asset.ready", alice.id).await;
    assert_eq!(notifications(&ctx, alice.id).await.len(), 1);
    assert_eq!(ctx.app.events_by("video.video.published", alice.id).await.len(), 1);

    // And it plays.
    let started: StartPlaybackResponse = ctx.call(PLAYBACK, "StartPlayback", Some(&bob), &play).await.unwrap();
    assert!(!started.session_id.is_empty() && started.min_view_seconds <= 1.0);
}

#[tokio::test]
async fn a_draft_stays_a_draft_when_its_upload_is_processed() {
    let ctx = start().await;
    ctx.app.core.storage.ensure_buckets().await.unwrap();
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "ch").await;
    let dir = tempfile::tempdir().unwrap();
    let asset = upload(&ctx, &alice, "draft.mp4", tokio::fs::read(fixture(dir.path(), 1).await).await.unwrap()).await;
    // Chapters are accepted while the length is unknown.
    let chapters = vec![
        Chapter { start_seconds: 0.0, title: "OP".into(), ..Default::default() },
        Chapter { start_seconds: 600.0, title: "far beyond the end".into(), ..Default::default() },
    ];
    let request = CreateVideoRequest { channel_id: channel.summary.id.clone(), asset_id: asset.id.clone(), chapters, ..Default::default() };
    let draft = ctx.create_video(&alice, &request).await.unwrap();
    assert_eq!(draft.chapters.len(), 2);

    ctx.media.process_now(id(&asset.id)).await.unwrap();
    // Nobody delivers the event this time: the worker that catches missed
    // results only looks at videos that have been waiting for a while.
    assert_eq!(videos::reconcile_processing(&ctx.app.core).await.unwrap(), 0);
    sqlx::query("UPDATE videos SET created_at = now() - interval '5 minutes' WHERE id = $1").bind(id(&card(&draft).id)).execute(&ctx.app.core.db).await.unwrap();
    assert!(videos::reconcile_processing(&ctx.app.core).await.unwrap() >= 1);

    let ready = ctx.get_video(Some(&alice), &card(&draft).id).await.unwrap();
    assert_eq!((card(&ready).status, card(&ready).processing), (VideoStatus::VIDEO_STATUS_DRAFT.into(), ProcessingState::PROCESSING_STATE_READY.into()));
    assert!(ready.playback.as_option().is_some());
    assert!(ready.chapters.is_empty(), "chapters past the end are dropped once the length is known");
    let told = notifications(&ctx, alice.id).await;
    assert_eq!((told[0].0.as_str(), told[0].1["published"].as_bool()), ("video.processing_done", Some(false)));
}

#[tokio::test]
async fn a_failed_upload_fails_the_video_and_cancels_its_publication() {
    let ctx = start().await;
    ctx.app.core.storage.ensure_buckets().await.unwrap();
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "ch").await;
    // Looks like an MP4 from the first bytes, but there is no video in it.
    let mut bytes = b"\x00\x00\x00\x18ftypisom\x00\x00\x02\x00isomiso2mp41".to_vec();
    bytes.extend(std::iter::repeat_n(0x5a, 4096));
    let asset = upload(&ctx, &alice, "broken.mp4", bytes).await;
    assert_eq!(asset.status, AssetStatus::ASSET_STATUS_PROCESSING);

    let request = CreateVideoRequest { channel_id: channel.summary.id.clone(), asset_id: asset.id.clone(), title: "壊れてる".into(), ..Default::default() };
    let draft = ctx.create_video(&alice, &request).await.unwrap();
    let video_id = card(&draft).id.clone();
    ctx.publish(&alice, &video_id).await;

    let mut frames = ctx.app.listen_user(alice.id).await;
    ctx.media.process_now(id(&asset.id)).await.unwrap();
    assert_eq!(ctx.deliver("media.asset.failed", alice.id).await, 1);

    let failed = ctx.get_video(Some(&alice), &video_id).await.unwrap();
    assert_eq!(card(&failed).processing, ProcessingState::PROCESSING_STATE_FAILED);
    assert_eq!(card(&failed).status, VideoStatus::VIDEO_STATUS_DRAFT, "the pending publication is cancelled");
    assert_eq!(failed.playback_block, PlaybackBlock::PLAYBACK_BLOCK_NOT_READY);
    assert!(!failed.studio.as_option().unwrap().processing_error.is_empty());
    let told = notifications(&ctx, alice.id).await;
    assert_eq!(told.iter().map(|(kind, _)| kind.as_str()).collect::<Vec<_>>(), vec!["video.processing_failed"]);
    assert!(pushes(frames.drain(QUIET).await).iter().any(|push| matches!(push, Push::VideoUpdated(v) if v.processing == ProcessingState::PROCESSING_STATE_FAILED)));

    // It cannot be published, and an upload that already failed cannot become a video.
    let publish = PublishVideoRequest { video_id: video_id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(VIDEOS, "PublishVideo", Some(&alice), &publish).await, "VIDEO_PROCESSING_FAILED");
    let _: DeleteVideoResponse = ctx.call(VIDEOS, "DeleteVideo", Some(&alice), &DeleteVideoRequest { video_id, ..Default::default() }).await.unwrap();
}

#[tokio::test]
async fn a_live_archive_becomes_a_video_through_the_rust_api() {
    let ctx = start().await;
    ctx.app.core.storage.ensure_buckets().await.unwrap();
    let core = &ctx.app.core;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "配信ch").await;
    let dir = tempfile::tempdir().unwrap();
    let recording = fixture(dir.path(), 1).await;

    // What the live domain does when a stream ends.
    let archive = tokuly_media::create_asset_from_file(core, alice.id, AssetPurpose::ASSET_PURPOSE_LIVE_ARCHIVE, &recording, "配信アーカイブ.mp4").await.unwrap();
    let stream_id = uuid::Uuid::now_v7();
    let new = NewVideo {
        description: "ライブ配信のアーカイブだよ!".into(),
        visibility: Visibility::VISIBILITY_PUBLIC,
        origin: VideoOrigin::VIDEO_ORIGIN_LIVE_ARCHIVE,
        origin_id: Some(stream_id),
        publish: true,
        notify_subscribers: false,
        ..NewVideo::new(id(&channel.summary.id), archive.id, "深夜の雑談配信")
    };
    let video_id = videos::create_from_asset(core, new.clone()).await.unwrap();
    let row = videos::find_by_id(core, video_id).await.unwrap().unwrap();
    assert_eq!((row.status, row.processing, row.origin_id), (videos::SCHEDULED, videos::PROCESSING, Some(stream_id)));

    // The same archive cannot back two videos; the asset must belong to the channel's owner.
    assert_eq!(videos::create_from_asset(core, new.clone()).await.unwrap_err().reason, "ASSET_ALREADY_USED");
    let bob = ctx.app.user("bob").await;
    let foreign = NewVideo::new(id(&ctx.channel(&bob, "bob").await.summary.id), archive.id, "not mine");
    assert_eq!(videos::create_from_asset(core, foreign).await.unwrap_err().reason, "ASSET_NOT_FOUND");
    let unknown = NewVideo::new(uuid::Uuid::now_v7(), archive.id, "no channel");
    assert_eq!(videos::create_from_asset(core, unknown).await.unwrap_err().reason, "CHANNEL_NOT_FOUND");

    ctx.media.process_now(archive.id).await.unwrap();
    ctx.deliver("media.asset.ready", alice.id).await;
    let live = ctx.get_video(None, &video_id.to_string()).await.expect("the archive is public");
    assert_eq!((card(&live).origin, card(&live).status), (VideoOrigin::VIDEO_ORIGIN_LIVE_ARCHIVE.into(), VideoStatus::VIDEO_STATUS_PUBLISHED.into()));
    assert_eq!(ctx.get_video(Some(&alice), &video_id.to_string()).await.unwrap().studio.as_option().unwrap().origin_id, stream_id.to_string());

    // Archives have their own row on the channel page and can be listed on their own.
    let home: GetChannelPageResponse = ctx.call(CHANNELS, "GetChannelPage", None, &GetChannelPageRequest { channel_id: channel.summary.id.clone(), ..Default::default() }).await.unwrap();
    let archives = home.sections.iter().find(|section| section.kind == ChannelSectionKind::CHANNEL_SECTION_KIND_LIVE_ARCHIVES).expect("an archives section");
    assert_eq!(titles(&archives.videos), vec!["深夜の雑談配信"]);
    let upload = ctx.public_video(&alice, &channel, "ふつうの動画").await;
    let only = |origin: VideoOrigin| ListChannelVideosRequest { channel_id: channel.summary.id.clone(), origin: origin.into(), ..Default::default() };
    let listed: ListChannelVideosResponse = ctx.call(CHANNELS, "ListChannelVideos", None, &only(VideoOrigin::VIDEO_ORIGIN_LIVE_ARCHIVE)).await.unwrap();
    assert_eq!(titles(&listed.videos), vec!["深夜の雑談配信"]);
    let listed: ListChannelVideosResponse = ctx.call(CHANNELS, "ListChannelVideos", None, &only(VideoOrigin::VIDEO_ORIGIN_UPLOAD)).await.unwrap();
    assert_eq!(titles(&listed.videos), vec![card(&upload).title.as_str()]);

    // `find` resolves both ID forms for other domains.
    let by_short = videos::find(core, &tokuly_core::ids::Ref::parse(&card(&live).short_id).unwrap()).await.unwrap().unwrap();
    assert_eq!(by_short.id, video_id);
}
