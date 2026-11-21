//! The real asynchronous path: `CompleteUpload` queues a job on JetStream
//! and the workers started by `spawn_workers` pick it up. (The other test
//! files run jobs in place so that they do not depend on worker timing;
//! this file is on its own so its workers only ever see its own jobs.)
mod support;

use std::time::Duration;

use support::*;
use tokuly_proto::media::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queued_jobs_are_processed_by_the_workers() {
    let h = start().await;
    h.media.spawn_workers();
    let alice = h.app.user("alice").await;
    let dir = fixture_dir();

    // One job for each queue: an image (media.process) and a video (media.transcode).
    let image = h.upload(&alice, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE, "p.png", "image/png", png(640, 480)).await;
    let clip = video(dir.path(), "clip.mp4", VideoSpec { size: "320x180", seconds: 1, rate: 10, audio: true }).await;
    let clip = h.upload(&alice, AssetPurpose::ASSET_PURPOSE_CHAT_VIDEO, "clip.mp4", "video/mp4", read(&clip).await).await;
    assert_eq!((image.status, clip.status), (AssetStatus::ASSET_STATUS_PROCESSING.into(), AssetStatus::ASSET_STATUS_PROCESSING.into()));

    for asset in [&image, &clip] {
        let request = WatchAssetRequest { asset_id: asset.id.clone(), ..Default::default() };
        let updates = h.app.stream::<_, WatchAssetResponse>("tokuly.media.v1.MediaService/WatchAsset", Some(&alice), &request, 100, Duration::from_secs(60)).await.unwrap();
        let last = updates.last().expect("at least the current state").asset.as_option().unwrap();
        assert_eq!(last.status, AssetStatus::ASSET_STATUS_READY, "{} ended as {:?} ({})", asset.file_name, last.status, last.error);
    }
    let ready = h.get_asset(&alice, &clip.id).await.unwrap();
    assert!(ready.video.as_option().is_some_and(|v| !v.hls_url.is_empty() && !v.mp4_url.is_empty()));
}
