//! The video and audio recipes, end to end: fixtures are generated with
//! ffmpeg's synthetic sources, uploaded through presigned URLs, transcoded
//! by the real pipeline and inspected in the object store.
mod support;

use std::time::Duration;

use support::*;
use tokuly_media::MediaConfig;
use tokuly_proto::media::*;

/// Fetches a text object (playlist, VTT) that must exist.
async fn text(h: &Harness, url: &str) -> String {
    String::from_utf8(h.media_object(url).await.unwrap_or_else(|| panic!("{url} is missing"))).expect("utf-8")
}

/// URL of a file next to `base` (how players resolve playlist entries).
fn sibling(base: &str, relative: &str) -> String {
    format!("{}/{relative}", base.rsplit_once('/').expect("url has a path").0)
}

/// Everything a media playlist references, checked to exist:
/// `(init segment bytes, segment count)`.
async fn check_media_playlist(h: &Harness, playlist_url: &str) -> (Vec<u8>, usize) {
    let playlist = text(h, playlist_url).await;
    assert!(playlist.contains("#EXT-X-PLAYLIST-TYPE:VOD") && playlist.trim_end().ends_with("#EXT-X-ENDLIST"), "{playlist}");
    let init_name = playlist.lines().find_map(|l| l.strip_prefix("#EXT-X-MAP:URI=\"")).expect("fMP4 playlists have an init segment").trim_end_matches('"');
    let init = h.media_object(&sibling(playlist_url, init_name)).await.expect("init segment stored");
    assert!(contains(&init, b"ftyp") && contains(&init, b"moov"), "the init segment is an fMP4 header");
    let segments: Vec<&str> = playlist.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()).collect();
    assert!(!segments.is_empty());
    for segment in &segments {
        assert!(segment.ends_with(".m4s"), "{segment}");
        let bytes = h.media_object(&sibling(playlist_url, segment)).await.unwrap_or_else(|| panic!("segment {segment} is missing"));
        assert!(contains(&bytes, b"moof") && contains(&bytes, b"mdat"), "{segment} is a CMAF fragment");
        assert!(!contains(&bytes, GPS_MARKER));
    }
    (init, segments.len())
}

/// The `(uri, attributes)` of every variant in a master playlist.
fn variants(master: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = master.lines().collect();
    lines
        .windows(2)
        .filter_map(|pair| pair[0].strip_prefix("#EXT-X-STREAM-INF:").map(|attributes| (pair[1].to_owned(), attributes.to_owned())))
        .collect()
}

async fn upload_video(h: &Harness, user: &tokuly_testkit::TestUser, purpose: AssetPurpose, path: &std::path::Path) -> Asset {
    h.upload(user, purpose, "clip.mp4", "video/mp4", read(path).await).await
}

#[tokio::test]
async fn video_becomes_hls_with_poster_storyboard_preview_and_mp4() {
    let h = start().await;
    let alice = h.app.user("alice").await;
    let dir = fixture_dir();
    let source = video(dir.path(), "landscape.mp4", VideoSpec::default()).await;
    assert!(contains(&read(&source).await, GPS_MARKER), "the fixture carries a title and position");

    let mut frames = h.app.listen_user(alice.id).await;
    let uploaded = upload_video(&h, &alice, AssetPurpose::ASSET_PURPOSE_VIDEO, &source).await;
    assert_eq!((uploaded.status, uploaded.kind), (AssetStatus::ASSET_STATUS_PROCESSING.into(), AssetKind::ASSET_KIND_VIDEO.into()));
    h.media.process_now(asset_id(&uploaded)).await.unwrap();
    let asset = h.get_asset(&alice, &uploaded.id).await.unwrap();
    assert_eq!(asset.status, AssetStatus::ASSET_STATUS_READY, "{}", asset.error);

    let info = asset.video.as_option().expect("video info");
    assert!((info.duration_seconds - 3.0).abs() < 0.2, "{}", info.duration_seconds);
    assert_eq!((info.width, info.height, info.frame_rate), (640, 360, 30.0));
    assert!(info.has_audio);
    assert!(info.loudness_lufs < -5.0 && info.loudness_lufs > -60.0, "{}", info.loudness_lufs);
    let renditions: Vec<(&str, &str, i32, i32)> = info.renditions.iter().map(|r| (r.name.as_str(), r.codec.as_str(), r.width, r.height)).collect();
    assert_eq!(renditions, vec![("360p", "h264", 640, 360), ("240p", "h264", 426, 240), ("audio", "aac", 0, 0)], "never upscaled");
    assert!(info.renditions.iter().all(|r| r.bitrate_kbps > 0));

    // ── HLS: master → media playlists → init + segments ──
    let master = text(&h, &info.hls_url).await;
    assert!(master.starts_with("#EXTM3U"));
    assert!(master.contains("#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\"") && master.contains("URI=\"audio/index.m3u8\""), "{master}");
    let listed = variants(&master);
    assert_eq!(listed.iter().map(|(uri, _)| uri.as_str()).collect::<Vec<_>>(), vec!["360p/index.m3u8", "240p/index.m3u8"]);
    for (uri, attributes) in &listed {
        assert!(attributes.contains("CODECS=\"avc1.64") && attributes.contains(",mp4a.40.2\"") && attributes.contains("AUDIO=\"audio\""), "{attributes}");
        assert!(attributes.contains("FRAME-RATE=30.000") && attributes.contains("BANDWIDTH="), "{attributes}");
        let (_, segments) = check_media_playlist(&h, &sibling(&info.hls_url, uri)).await;
        assert_eq!(segments, 1, "a 3 s clip fits one 4 s segment");
    }
    assert!(listed[0].1.contains("RESOLUTION=640x360") && listed[1].1.contains("RESOLUTION=426x240"));
    check_media_playlist(&h, &sibling(&info.hls_url, "audio/index.m3u8")).await;
    // Each rendition is addressable on its own too.
    assert_eq!(info.renditions[0].url, sibling(&info.hls_url, "360p/index.m3u8"));
    let served = h.media_response(&info.hls_url).await;
    assert_eq!(served.headers()["content-type"], "application/vnd.apple.mpegurl");
    assert_eq!(served.headers()["cache-control"], "public, max-age=31536000, immutable");

    // ── poster: an image like any other ──
    let poster = info.poster.as_option().expect("poster");
    assert_eq!((poster.width, poster.height), (640, 360));
    assert_eq!(asset.image.as_option().unwrap().url, poster.url);
    assert_eq!(poster.variants.iter().map(|v| v.size).collect::<Vec<_>>(), vec![640, 512, 256, 96]);
    assert!(image::load_from_memory(&h.media_object(&poster.url).await.unwrap()).is_ok());

    // ── storyboard: WebVTT pointing into sprite sheets ──
    let vtt = text(&h, &info.storyboard_url).await;
    assert!(vtt.starts_with("WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nsprite_001.webp#xywh=0,0,160,90\n"), "{vtt}");
    assert!((3..=4).contains(&vtt.matches("-->").count()), "one cue per second: {vtt}");
    let sheet = image::load_from_memory(&h.media_object(&sibling(&info.storyboard_url, "sprite_001.webp")).await.expect("sprite sheet")).unwrap();
    assert_eq!((sheet.width(), sheet.height()), (1600, 900), "a 10×10 grid of 160×90 frames");

    // ── animated preview ──
    let preview = h.media_object(&info.preview_url).await.expect("preview");
    let decoder = image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(&preview)).unwrap();
    assert!(decoder.has_animation());
    assert_eq!(image::ImageDecoder::dimensions(&decoder), (320, 180));

    // ── progressive MP4 for short clips, playable before it is fully downloaded ──
    assert_eq!(asset.url, info.mp4_url, "short clips are served as MP4 first");
    let mp4 = h.media_object(&info.mp4_url).await.expect("mp4");
    let position = |needle: &[u8]| mp4.windows(needle.len()).position(|w| w == needle).unwrap_or_else(|| panic!("no {needle:?} box"));
    assert!(position(b"moov") < position(b"mdat"), "fast start: the index precedes the media data");

    // ── nothing of the source's metadata is published ──
    for object in [&mp4, &preview, &h.media_object(&info.hls_url).await.unwrap()] {
        assert!(!contains(object, GPS_MARKER));
    }

    // ── progress was pushed along the way, ending with READY ──
    let pushed: Vec<Asset> = pushed_assets(frames.events(Duration::from_millis(300)).await).into_iter().filter(|pushed| pushed.id == asset.id).collect();
    assert!(pushed.len() >= 2, "expected progress pushes, got {}", pushed.len());
    assert!(pushed.windows(2).all(|pair| pair[0].progress <= pair[1].progress));
    assert!(pushed.iter().any(|a| a.status == AssetStatus::ASSET_STATUS_PROCESSING && a.progress > 0));
    assert_eq!(pushed.last().unwrap().status, AssetStatus::ASSET_STATUS_READY);
    assert_eq!(h.app.events_by("media.asset.ready", alice.id).await.iter().filter(|e| e.data["asset_id"] == asset.id.as_str()).count(), 1);

    // Deleting the asset takes every rendition with it.
    let _: DeleteAssetResponse = h.rpc("DeleteAsset", Some(&alice), &DeleteAssetRequest { asset_id: asset.id.clone(), ..Default::default() }).await.unwrap();
    for url in [&info.hls_url, &info.mp4_url, &info.preview_url, &info.storyboard_url, &poster.url, &sibling(&info.hls_url, "240p/index.m3u8")] {
        assert!(h.media_object(url).await.is_none(), "{url} should be gone");
    }
}

#[tokio::test]
async fn portrait_video_stays_portrait() {
    let h = start().await;
    let alice = h.app.user("alice").await;
    let dir = fixture_dir();
    // Stored as 640×360 pixels plus a 90° rotation, like a phone recording;
    // no audio track.
    let landscape = video(dir.path(), "silent.mp4", VideoSpec { audio: false, ..Default::default() }).await;
    let source = rotated(&landscape, 90).await;

    let uploaded = upload_video(&h, &alice, AssetPurpose::ASSET_PURPOSE_SHORT, &source).await;
    h.media.process_now(asset_id(&uploaded)).await.unwrap();
    let asset = h.get_asset(&alice, &uploaded.id).await.unwrap();
    assert_eq!(asset.status, AssetStatus::ASSET_STATUS_READY, "{}", asset.error);

    let info = asset.video.as_option().unwrap();
    assert_eq!((info.width, info.height), (360, 640), "upright");
    assert!(!info.has_audio);
    assert_eq!(info.loudness_lufs, 0.0);
    let sizes: Vec<(&str, i32, i32)> = info.renditions.iter().map(|r| (r.name.as_str(), r.width, r.height)).collect();
    assert_eq!(sizes, vec![("360p", 360, 640), ("240p", 240, 426)], "rungs are named after the short side");

    let master = text(&h, &info.hls_url).await;
    assert!(!master.contains("EXT-X-MEDIA") && !master.contains("AUDIO="), "{master}");
    assert!(master.contains("RESOLUTION=360x640") && master.contains("RESOLUTION=240x426"));
    for (uri, attributes) in variants(&master) {
        assert!(attributes.contains("CODECS=\"avc1.64") && !attributes.contains("mp4a"), "{attributes}");
        check_media_playlist(&h, &sibling(&info.hls_url, &uri)).await;
    }
    let poster = info.poster.as_option().unwrap();
    assert_eq!((poster.width, poster.height), (360, 640));
    let vtt = text(&h, &info.storyboard_url).await;
    assert!(vtt.contains("#xywh=0,0,90,160"), "portrait thumbnails: {vtt}");
    assert!(h.media_object(&info.mp4_url).await.is_some());
}

#[tokio::test]
async fn purposes_cap_the_ladder_and_long_videos_skip_the_mp4() {
    let h = start().await;
    let alice = h.app.user("alice").await;
    let dir = fixture_dir();
    // 61 s at a tiny size and frame rate: just over the MP4 limit, quick to encode.
    let long = video(dir.path(), "long.mp4", VideoSpec { size: "320x180", seconds: 61, rate: 5, audio: true }).await;
    let uploaded = upload_video(&h, &alice, AssetPurpose::ASSET_PURPOSE_VIDEO, &long).await;
    h.media.process_now(asset_id(&uploaded)).await.unwrap();
    let asset = h.get_asset(&alice, &uploaded.id).await.unwrap();
    assert_eq!(asset.status, AssetStatus::ASSET_STATUS_READY, "{}", asset.error);
    let info = asset.video.as_option().unwrap();
    assert_eq!(info.mp4_url, "", "only clips of at most 60 s get an MP4");
    assert_eq!(asset.url, info.hls_url);
    assert_eq!(info.renditions.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["180p", "audio"]);
    let (_, segments) = check_media_playlist(&h, &info.renditions[0].url).await;
    assert_eq!(segments, 16, "61 s in 4 s segments");
    // Audio and video are cut at the same boundaries.
    let (_, audio_segments) = check_media_playlist(&h, &info.renditions[1].url).await;
    assert_eq!(audio_segments, 16);
    let vtt = text(&h, &info.storyboard_url).await;
    assert!((61..=62).contains(&vtt.matches("-->").count()), "one cue per second");

    // An avatar video tops out at 480p even when the source is larger.
    let hd = video(dir.path(), "hd.mp4", VideoSpec { size: "1280x720", seconds: 1, rate: 10, audio: false }).await;
    let uploaded = upload_video(&h, &alice, AssetPurpose::ASSET_PURPOSE_AVATAR_VIDEO, &hd).await;
    h.media.process_now(asset_id(&uploaded)).await.unwrap();
    let asset = h.get_asset(&alice, &uploaded.id).await.unwrap();
    let info = asset.video.as_option().unwrap();
    assert_eq!(info.renditions.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["480p", "360p", "240p"]);
    assert_eq!((info.width, info.height), (854, 480));
    // Playback starts on the best rung that suits a phone.
    assert_eq!(variants(&text(&h, &info.hls_url).await)[0].0, "480p/index.m3u8");
}

#[tokio::test]
async fn av1_rendition_is_added_when_enabled() {
    let encoders = tokio::process::Command::new("ffmpeg").args(["-hide_banner", "-encoders"]).output().await.expect("ffmpeg");
    if !String::from_utf8_lossy(&encoders.stdout).contains("libsvtav1") {
        eprintln!("skipped: this ffmpeg build has no libsvtav1");
        return;
    }
    let h = start_with(MediaConfig { av1: true, ..Default::default() }).await;
    let alice = h.app.user("alice").await;
    let dir = fixture_dir();
    let source = video(dir.path(), "small.mp4", VideoSpec { size: "320x240", seconds: 2, rate: 15, audio: true }).await;
    let uploaded = upload_video(&h, &alice, AssetPurpose::ASSET_PURPOSE_VIDEO, &source).await;
    h.media.process_now(asset_id(&uploaded)).await.unwrap();
    let asset = h.get_asset(&alice, &uploaded.id).await.unwrap();
    assert_eq!(asset.status, AssetStatus::ASSET_STATUS_READY, "{}", asset.error);

    let info = asset.video.as_option().unwrap();
    let renditions: Vec<(&str, &str)> = info.renditions.iter().map(|r| (r.name.as_str(), r.codec.as_str())).collect();
    assert_eq!(renditions, vec![("240p", "h264"), ("240p-av1", "av1"), ("audio", "aac")]);
    let master = text(&h, &info.hls_url).await;
    let listed = variants(&master);
    assert_eq!(listed.iter().map(|(uri, _)| uri.as_str()).collect::<Vec<_>>(), vec!["240p/index.m3u8", "240p-av1/index.m3u8"]);
    assert!(listed[1].1.contains("CODECS=\"av01.0.") && listed[1].1.contains("AUDIO=\"audio\""), "{}", listed[1].1);
    let (init, _) = check_media_playlist(&h, &sibling(&info.hls_url, "240p-av1/index.m3u8")).await;
    assert!(contains(&init, b"av01"), "the rendition really is AV1");
}

#[tokio::test]
async fn deleting_an_asset_stops_its_transcode() {
    // One encoder thread: the full transcode would take far longer than the test allows.
    let h = start_with(MediaConfig { ffmpeg_threads: 1, ..Default::default() }).await;
    let alice = h.app.user("alice").await;
    let dir = fixture_dir();
    // Long enough that the transcode is still running when the delete arrives.
    let source = video(dir.path(), "long.mp4", VideoSpec { size: "1280x720", seconds: 40, rate: 30, audio: true }).await;
    let uploaded = upload_video(&h, &alice, AssetPurpose::ASSET_PURPOSE_VIDEO, &source).await;
    let id = asset_id(&uploaded);

    let started = std::time::Instant::now();
    let job = h.media.process_now(id);
    let delete = async {
        tokio::time::sleep(Duration::from_millis(700)).await;
        let _: DeleteAssetResponse = h.rpc("DeleteAsset", Some(&alice), &DeleteAssetRequest { asset_id: uploaded.id.clone(), ..Default::default() }).await.unwrap();
    };
    let (finished, ()) = tokio::join!(job, delete);
    finished.expect("a cancelled job is not an error");
    assert!(started.elapsed() < Duration::from_secs(15), "the job stopped early instead of transcoding 40 s of 720p ({:?})", started.elapsed());

    // Nothing is left behind: no original, no rendition, no ready event.
    assert!(!h.original_exists(id).await);
    let master = h.app.core.storage.media_url(&format!("video/{id}/hls/master.m3u8"));
    assert!(h.media_object(&master).await.is_none());
    assert!(h.app.events_by("media.asset.ready", alice.id).await.iter().all(|e| e.data["asset_id"] != uploaded.id.as_str()));
    assert_eq!(h.get_asset(&alice, &uploaded.id).await.unwrap_err().reason, "ASSET_NOT_FOUND");
}

#[tokio::test]
async fn hdr_video_is_tone_mapped_to_sdr() {
    let encoders = tokio::process::Command::new("ffmpeg").args(["-hide_banner", "-encoders"]).output().await.expect("ffmpeg");
    let filters = tokio::process::Command::new("ffmpeg").args(["-hide_banner", "-filters"]).output().await.expect("ffmpeg");
    if !String::from_utf8_lossy(&encoders.stdout).contains("libx265") || !String::from_utf8_lossy(&filters.stdout).contains(" zscale ") {
        eprintln!("skipped: this ffmpeg build lacks libx265 (fixture) or zscale (tone mapping)");
        return;
    }
    let h = start().await;
    let alice = h.app.user("alice").await;
    let dir = fixture_dir();
    // 10-bit HLG in BT.2020, as phones record HDR.
    let path = dir.path().join("hlg.mp4");
    ffmpeg(&[
        "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=15:duration=2", "-c:v", "libx265", "-preset", "ultrafast", "-pix_fmt", "yuv420p10le",
        "-x265-params", "log-level=error:colorprim=bt2020:transfer=arib-std-b67:colormatrix=bt2020nc", &path.to_string_lossy(),
    ])
    .await;

    let uploaded = upload_video(&h, &alice, AssetPurpose::ASSET_PURPOSE_VIDEO, &path).await;
    h.media.process_now(asset_id(&uploaded)).await.unwrap();
    let asset = h.get_asset(&alice, &uploaded.id).await.unwrap();
    assert_eq!(asset.status, AssetStatus::ASSET_STATUS_READY, "{}", asset.error);
    let info = asset.video.as_option().unwrap();
    assert_eq!(info.renditions[0].codec, "h264");
    // The rendition is plain 8-bit BT.709: its header carries no HDR transfer.
    let (init, _) = check_media_playlist(&h, &info.renditions[0].url).await;
    assert!(contains(&init, b"avc1"));
    // The poster is cut from it and shows the test pattern's colours.
    let poster = image::load_from_memory(&h.media_object(&info.poster.as_option().unwrap().url).await.unwrap()).unwrap().into_rgb8();
    let brightest = poster.pixels().map(|p| p[0].max(p[1]).max(p[2])).max().unwrap();
    let darkest = poster.pixels().map(|p| p[0].min(p[1]).min(p[2])).min().unwrap();
    assert!(brightest > 180 && darkest < 60, "tone mapping kept the contrast ({darkest}..{brightest})");
}

#[tokio::test]
async fn non_video_and_corrupt_uploads_fail_cleanly() {
    let h = start().await;
    let alice = h.app.user("alice").await;
    let dir = fixture_dir();

    // An MP4 container with nothing but audio in it.
    let audio_only = tone(dir.path(), "audio-only.mp4", 1).await;
    let uploaded = upload_video(&h, &alice, AssetPurpose::ASSET_PURPOSE_VIDEO, &audio_only).await;
    h.media.process_now(asset_id(&uploaded)).await.unwrap();
    let asset = h.get_asset(&alice, &uploaded.id).await.unwrap();
    assert_eq!((asset.status, asset.error.as_str()), (AssetStatus::ASSET_STATUS_FAILED.into(), "NOT_A_VIDEO"));
    assert!(!h.original_exists(asset_id(&asset)).await, "the rejected original is discarded");
    assert!(h.app.events_by("media.asset.failed", alice.id).await.iter().any(|e| e.data["asset_id"] == asset.id.as_str() && e.data["error"] == "NOT_A_VIDEO"));

    // A real MP4 header followed by garbage.
    let good = read(&video(dir.path(), "good.mp4", VideoSpec { seconds: 1, ..Default::default() }).await).await;
    let mut corrupt = good[..32].to_vec();
    corrupt.extend(std::iter::repeat_n(0x5a, 20_000));
    let uploaded = h.upload(&alice, AssetPurpose::ASSET_PURPOSE_CHAT_VIDEO, "broken.mp4", "video/mp4", corrupt).await;
    h.media.process_now(asset_id(&uploaded)).await.unwrap();
    let asset = h.get_asset(&alice, &uploaded.id).await.unwrap();
    assert_eq!((asset.status, asset.error.as_str()), (AssetStatus::ASSET_STATUS_FAILED.into(), "CORRUPT_MEDIA"));

    // An image is refused for a video purpose before any processing.
    let upload = h.create_upload(&alice, AssetPurpose::ASSET_PURPOSE_VIDEO, "fake.mp4", "video/mp4", &png(64, 64)).await;
    assert!(h.put(&upload, png(64, 64)).await.status().is_success());
    assert_eq!(h.complete(&alice, &upload.upload_id).await.unwrap_err().reason, "UNSUPPORTED_CONTENT");
}

#[tokio::test]
async fn voice_message_gets_two_renditions_and_a_waveform() {
    let h = start().await;
    let alice = h.app.user("alice").await;
    let dir = fixture_dir();
    // Opus in WebM, as browsers record; such files do not state their duration.
    let recording = tone(dir.path(), "voice.webm", 2).await;

    let uploaded = h.upload(&alice, AssetPurpose::ASSET_PURPOSE_CHAT_AUDIO, "voice.webm", "audio/webm;codecs=opus", read(&recording).await).await;
    assert_eq!((uploaded.status, uploaded.kind), (AssetStatus::ASSET_STATUS_PROCESSING.into(), AssetKind::ASSET_KIND_AUDIO.into()));
    h.media.process_now(asset_id(&uploaded)).await.unwrap();
    let asset = h.get_asset(&alice, &uploaded.id).await.unwrap();
    assert_eq!(asset.status, AssetStatus::ASSET_STATUS_READY, "{}", asset.error);

    assert_eq!(asset.waveform.len(), 64);
    assert!(asset.waveform.iter().all(|bar| *bar > 200), "a steady tone is loud throughout: {:?}", asset.waveform);
    let info = asset.video.as_option().unwrap();
    assert!((info.duration_seconds - 2.0).abs() < 0.1, "{}", info.duration_seconds);
    assert!(info.has_audio && info.loudness_lufs < 0.0);
    assert_eq!(info.renditions.iter().map(|r| r.codec.as_str()).collect::<Vec<_>>(), vec!["aac", "opus"]);
    assert_eq!(asset.url, info.renditions[0].url);

    let m4a = h.media_object(&asset.url).await.expect("AAC rendition");
    assert!(contains(&m4a, b"ftyp") && contains(&m4a, b"mp4a"));
    let webm = h.media_object(&info.renditions[1].url).await.expect("Opus rendition");
    assert!(webm.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) && contains(&webm, b"OpusHead"));
    assert_eq!(h.media_response(&asset.url).await.headers()["content-type"], "audio/mp4");
    // The attachment other domains embed carries the waveform.
    let row = tokuly_media::assets::require_ready(&h.app.core, asset_id(&asset), alice.id, &[AssetPurpose::ASSET_PURPOSE_CHAT_AUDIO]).await.unwrap();
    assert_eq!(row.to_attachment().waveform.len(), 64);

    // A video without sound is not a voice message.
    let silent = video(dir.path(), "silent.mp4", VideoSpec { seconds: 1, audio: false, ..Default::default() }).await;
    let uploaded = h.upload(&alice, AssetPurpose::ASSET_PURPOSE_CHAT_AUDIO, "silent.mp4", "audio/mp4", read(&silent).await).await;
    h.media.process_now(asset_id(&uploaded)).await.unwrap();
    let asset = h.get_asset(&alice, &uploaded.id).await.unwrap();
    assert_eq!((asset.status, asset.error.as_str()), (AssetStatus::ASSET_STATUS_FAILED.into(), "NO_AUDIO"));
}

#[tokio::test]
async fn live_archive_is_ingested_from_a_server_side_file() {
    let h = start().await;
    let alice = h.app.user("alice").await;
    let dir = fixture_dir();
    // Recordings arrive as MPEG-TS, which cannot be recognised by magic bytes.
    let mp4 = video(dir.path(), "recording.mp4", VideoSpec { seconds: 2, ..Default::default() }).await;
    let ts = dir.path().join("recording.ts");
    ffmpeg(&["-i", &mp4.to_string_lossy(), "-c", "copy", "-f", "mpegts", &ts.to_string_lossy()]).await;

    let row = tokuly_media::create_asset_from_file(&h.app.core, alice.id, AssetPurpose::ASSET_PURPOSE_LIVE_ARCHIVE, &ts, "配信アーカイブ.ts").await.unwrap();
    assert_eq!((row.kind, row.status), (AssetKind::ASSET_KIND_VIDEO as i32, AssetStatus::ASSET_STATUS_PROCESSING as i32));
    h.media.process_now(row.id).await.unwrap();
    let asset = h.get_asset(&alice, &row.id.to_string()).await.unwrap();
    assert_eq!(asset.status, AssetStatus::ASSET_STATUS_READY, "{}", asset.error);
    assert_eq!(asset.purpose, AssetPurpose::ASSET_PURPOSE_LIVE_ARCHIVE);
    let info = asset.video.as_option().unwrap();
    assert!(info.has_audio && (info.duration_seconds - 2.0).abs() < 0.3, "{}", info.duration_seconds);
    check_media_playlist(&h, &info.renditions[0].url).await;
}
