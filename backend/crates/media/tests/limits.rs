//! How long a recording may be, per purpose, and the daily transcoding
//! budget. Durations are checked on the probed file before anything is
//! transcoded, so the long fixtures are tiny and quick to make.
mod support;

use support::*;
use tokuly_media::MediaConfig;
use tokuly_proto::media::*;

async fn processed(h: &Harness, user: &tokuly_testkit::TestUser, purpose: AssetPurpose, path: &std::path::Path) -> Asset {
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let uploaded = h.upload(user, purpose, &name, "video/mp4", read(path).await).await;
    h.media.process_now(asset_id(&uploaded)).await.unwrap();
    h.get_asset(user, &uploaded.id).await.unwrap()
}

fn tiny(seconds: u32) -> VideoSpec<'static> {
    VideoSpec { size: "64x64", seconds, rate: 1, audio: false }
}

fn failed_as(asset: &Asset, reason: &str) -> bool {
    asset.status == AssetStatus::ASSET_STATUS_FAILED && asset.error == reason
}

#[tokio::test]
async fn each_purpose_refuses_recordings_longer_than_it_takes() {
    let h = start().await;
    let alice = h.app.user("alice").await;
    let dir = fixture_dir();

    // The limit is announced when the upload starts, and kept on the asset.
    let story = video(dir.path(), "story.mp4", tiny(62)).await;
    let upload = h.create_upload(&alice, AssetPurpose::ASSET_PURPOSE_STORY, "story.mp4", "video/mp4", &read(&story).await).await;
    assert_eq!(upload.max_duration_seconds, 60);
    let photo = h.create_upload(&alice, AssetPurpose::ASSET_PURPOSE_STORY, "p.png", "image/png", &png(8, 8)).await;
    assert_eq!(photo.max_duration_seconds, 0, "a photo has no duration");

    for (purpose, seconds, limit) in [
        (AssetPurpose::ASSET_PURPOSE_STORY, 62, 60),
        (AssetPurpose::ASSET_PURPOSE_AVATAR_VIDEO, 17, 15),
        (AssetPurpose::ASSET_PURPOSE_SHORT, 182, 180),
        (AssetPurpose::ASSET_PURPOSE_CHAT_VIDEO, 602, 600),
        (AssetPurpose::ASSET_PURPOSE_NOTE_VIDEO, 602, 600),
    ] {
        let path = video(dir.path(), &format!("{}-{seconds}.mp4", purpose as i32), tiny(seconds)).await;
        let asset = processed(&h, &alice, purpose, &path).await;
        assert!(failed_as(&asset, "MEDIA_TOO_LONG"), "{purpose:?} {seconds} s: {:?} {}", asset.status, asset.error);
        assert_eq!(asset.max_duration_seconds, limit, "{purpose:?}");
        assert!(!h.original_exists(asset_id(&asset)).await, "nothing of a refused upload is kept");
    }

    // Right at the limit is fine.
    let avatar = video(dir.path(), "avatar.mp4", tiny(15)).await;
    let asset = processed(&h, &alice, AssetPurpose::ASSET_PURPOSE_AVATAR_VIDEO, &avatar).await;
    assert_eq!(asset.status, AssetStatus::ASSET_STATUS_READY, "{}", asset.error);

    // Voice messages: five minutes.
    let long_voice = tone(dir.path(), "long.webm", 302).await;
    let uploaded = h.upload(&alice, AssetPurpose::ASSET_PURPOSE_CHAT_AUDIO, "long.webm", "audio/webm", read(&long_voice).await).await;
    h.media.process_now(asset_id(&uploaded)).await.unwrap();
    let asset = h.get_asset(&alice, &uploaded.id).await.unwrap();
    assert!(failed_as(&asset, "MEDIA_TOO_LONG"), "{:?} {}", asset.status, asset.error);
    assert_eq!(asset.max_duration_seconds, 300);
}

#[tokio::test]
async fn long_form_videos_follow_the_configured_hours() {
    let h = start_with(MediaConfig { max_video_hours: 1, ..MediaConfig::default() }).await;
    let alice = h.app.user("alice").await;
    let dir = fixture_dir();
    let long = video(dir.path(), "long.mp4", tiny(3602)).await;
    let asset = processed(&h, &alice, AssetPurpose::ASSET_PURPOSE_VIDEO, &long).await;
    assert!(failed_as(&asset, "MEDIA_TOO_LONG"), "{:?} {}", asset.status, asset.error);
    assert_eq!(asset.max_duration_seconds, 3600);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_daily_transcoding_budget_is_per_account_and_cannot_be_raced() {
    // One minute a day: one 40-second video fits, two do not.
    let h = start_with(MediaConfig { daily_transcode_minutes: 1, ..MediaConfig::default() }).await;
    let (alice, bob) = (h.app.user("alice").await, h.app.user("bob").await);
    let dir = fixture_dir();
    let clip = read(&video(dir.path(), "clip.mp4", VideoSpec { size: "64x64", seconds: 40, rate: 2, audio: false }).await).await;
    let mut ids = Vec::new();
    for _ in 0..2 {
        ids.push(h.upload(&alice, AssetPurpose::ASSET_PURPOSE_VIDEO, "clip.mp4", "video/mp4", clip.clone()).await.id);
    }
    // Both probed at the same moment: only one is booked.
    let (first, second) = tokio::join!(h.media.process_now(ids[0].parse().unwrap()), h.media.process_now(ids[1].parse().unwrap()));
    first.unwrap();
    second.unwrap();
    let mut outcomes = Vec::new();
    for id in &ids {
        let asset = h.get_asset(&alice, id).await.unwrap();
        outcomes.push((asset.status.to_i32(), asset.error));
    }
    outcomes.sort();
    assert_eq!(
        outcomes,
        vec![
            (AssetStatus::ASSET_STATUS_READY as i32, String::new()),
            (AssetStatus::ASSET_STATUS_FAILED as i32, "DAILY_TRANSCODE_LIMIT_EXCEEDED".to_owned()),
        ]
    );

    // Somebody else's budget is their own.
    let theirs = h.upload(&bob, AssetPurpose::ASSET_PURPOSE_VIDEO, "clip.mp4", "video/mp4", clip.clone()).await;
    h.media.process_now(asset_id(&theirs)).await.unwrap();
    let asset = h.get_asset(&bob, &theirs.id).await.unwrap();
    assert_eq!(asset.status, AssetStatus::ASSET_STATUS_READY, "{}", asset.error);
}
