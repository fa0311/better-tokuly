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

/// Writes a copy of `path` whose MP4 header claims `seconds`, and checks the
/// forgery took (ffprobe believes it).
async fn forged(path: &std::path::Path, seconds: u32) -> std::path::PathBuf {
    let out = path.with_file_name(format!("forged-{}", path.file_name().unwrap().to_string_lossy()));
    tokio::fs::write(&out, forge_mp4_duration(read(path).await, seconds)).await.unwrap();
    let claimed = probed_seconds(&out).await;
    assert!((claimed - f64::from(seconds)).abs() < 1.0, "the forged header claims {claimed} s");
    out
}

#[tokio::test]
async fn a_header_that_lies_about_the_length_does_not_get_a_recording_through() {
    let h = start().await;
    let alice = h.app.user("alice").await;
    let dir = fixture_dir();

    // 120 s of video that says it lasts 10 s: still too long for a story.
    let long = video(dir.path(), "long.mp4", tiny(120)).await;
    let lying = forged(&long, 10).await;
    let asset = processed(&h, &alice, AssetPurpose::ASSET_PURPOSE_STORY, &lying).await;
    assert!(failed_as(&asset, "MEDIA_TOO_LONG"), "{:?} {}", asset.status, asset.error);

    // The same for a voice message: 6 minutes claiming 10 s.
    let voice = tone(dir.path(), "voice.m4a", 360).await;
    let lying = forged(&voice, 10).await;
    let uploaded = h.upload(&alice, AssetPurpose::ASSET_PURPOSE_CHAT_AUDIO, "voice.m4a", "audio/mp4", read(&lying).await).await;
    h.media.process_now(asset_id(&uploaded)).await.unwrap();
    let asset = h.get_asset(&alice, &uploaded.id).await.unwrap();
    assert!(failed_as(&asset, "MEDIA_TOO_LONG"), "{:?} {}", asset.status, asset.error);
}

#[tokio::test]
async fn the_budget_is_charged_with_the_real_length() {
    // One minute a day; the file says 10 s but plays for 90.
    let h = start_with(MediaConfig { daily_transcode_minutes: 1, ..MediaConfig::default() }).await;
    let alice = h.app.user("alice").await;
    let dir = fixture_dir();
    let long = video(dir.path(), "long.mp4", tiny(90)).await;
    let lying = forged(&long, 10).await;
    let asset = processed(&h, &alice, AssetPurpose::ASSET_PURPOSE_VIDEO, &lying).await;
    assert!(failed_as(&asset, "DAILY_TRANSCODE_LIMIT_EXCEEDED"), "{:?} {}", asset.status, asset.error);
}

#[tokio::test]
async fn an_edit_list_that_repeats_a_short_file_for_hours_is_refused() {
    let h = start_with(MediaConfig { daily_transcode_minutes: 60, ..MediaConfig::default() }).await;
    let alice = h.app.user("alice").await;
    let dir = fixture_dir();
    // 2 s of video, an edit list playing it 3600 times: two hours, while
    // the header and the packets say 2 s.
    let short = video(dir.path(), "short.mp4", VideoSpec { size: "64x64", seconds: 2, rate: 5, audio: false }).await;
    let amplified = dir.path().join("amplified.mp4");
    tokio::fs::write(&amplified, repeat_edit_list(&read(&short).await, 3600)).await.unwrap();
    let claimed = probed_seconds(&amplified).await;
    assert!(claimed < 3.0, "the header still says {claimed} s");

    // A story: refused, not cut down to a minute.
    let asset = processed(&h, &alice, AssetPurpose::ASSET_PURPOSE_STORY, &amplified).await;
    assert!(failed_as(&asset, "MEDIA_TOO_LONG"), "{:?} {}", asset.status, asset.error);
    // A long-form video: within 3 hours, but two hours do not fit an hour's budget.
    let asset = processed(&h, &alice, AssetPurpose::ASSET_PURPOSE_VIDEO, &amplified).await;
    assert!(failed_as(&asset, "DAILY_TRANSCODE_LIMIT_EXCEEDED"), "{:?} {}", asset.status, asset.error);
}
