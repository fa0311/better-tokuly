//! End-to-end tests of the upload lifecycle against the real object store:
//! bytes go to presigned URLs with plain HTTP, exactly as clients send them.
mod support;

use std::time::Duration;

use support::*;
use tokuly_core::events::Event;
use tokuly_media::{MediaConfig, assets, events::AssetEvent};
use tokuly_proto::media::*;

const MB: usize = 1024 * 1024;

/// Small multipart parts so a resumable upload does not need a huge file.
fn multipart_config() -> MediaConfig {
    MediaConfig { multipart_threshold_mb: 1, part_size_mb: 5, ..Default::default() }
}

/// Deterministic bytes that are not any recognisable file format.
fn noise(len: usize) -> Vec<u8> {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8 | 0x80 // high bit set: binary, whatever the sniffer makes of it
        })
        .collect()
}

/// The janitor sweeps the whole (shared) database, so tests that count what
/// a sweep did must not sweep at the same time.
static SWEEPING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn reason(result: Result<impl std::fmt::Debug, tokuly_testkit::RpcError>) -> String {
    result.expect_err("expected an error").reason
}

#[tokio::test]
async fn create_upload_validates_what_is_declared() {
    let h = start().await;
    let alice = h.app.user("alice").await;
    let valid = CreateUploadRequest {
        purpose: AssetPurpose::ASSET_PURPOSE_AVATAR.into(),
        file_name: "me.png".into(),
        content_type: "image/png".into(),
        size_bytes: 1000,
        ..Default::default()
    };
    let cases: Vec<(&str, Box<dyn Fn(&mut CreateUploadRequest)>)> = vec![
        ("PURPOSE_INVALID", Box::new(|r| r.purpose = AssetPurpose::ASSET_PURPOSE_UNSPECIFIED.into())),
        ("PURPOSE_NOT_ALLOWED", Box::new(|r| {
            r.purpose = AssetPurpose::ASSET_PURPOSE_LIVE_ARCHIVE.into();
            r.content_type = "video/mp4".into();
        })),
        ("UNSUPPORTED_CONTENT_TYPE", Box::new(|r| r.content_type = "video/mp4".into())),
        ("UNSUPPORTED_CONTENT_TYPE", Box::new(|r| r.content_type = "image/svg+xml".into())),
        ("FILE_EMPTY", Box::new(|r| r.size_bytes = 0)),
        ("FILE_TOO_LARGE", Box::new(|r| r.size_bytes = 10 * 1024 * 1024 + 1)),
        ("SHA256_INVALID", Box::new(|r| r.sha256 = "abc".into())),
    ];
    for (expected, mutate) in cases {
        let mut request = valid.clone();
        mutate(&mut request);
        let error = h.rpc::<_, CreateUploadResponse>("CreateUpload", Some(&alice), &request).await.unwrap_err();
        assert_eq!(error.reason, expected, "{error}");
    }

    let anonymous = h.rpc::<_, CreateUploadResponse>("CreateUpload", None, &valid).await.unwrap_err();
    assert_eq!((anonymous.status, anonymous.reason.as_str()), (401, "SIGN_IN_REQUIRED"));

    // Limits follow the purpose: the same size is fine for a long-form video.
    let video = CreateUploadRequest {
        purpose: AssetPurpose::ASSET_PURPOSE_VIDEO.into(),
        file_name: "film.mp4".into(),
        content_type: "video/mp4".into(),
        size_bytes: 3 * 1024 * 1024 * 1024,
        ..Default::default()
    };
    let upload: CreateUploadResponse = h.rpc("CreateUpload", Some(&alice), &video).await.unwrap();
    assert!(upload.put_url.is_empty(), "large files are multipart");
    assert_eq!((upload.part_size_bytes, upload.part_count), (16 * 1024 * 1024, 192));
    assert_eq!(upload.asset_id, upload.upload_id);
    assert!(upload.expires_at.is_set());

    // An upload that was only announced is visible to its owner, as UPLOADING.
    let asset = h.get_asset(&alice, &upload.asset_id).await.unwrap();
    assert_eq!(asset.status, AssetStatus::ASSET_STATUS_UPLOADING);
    let _: AbortUploadResponse = h.rpc("AbortUpload", Some(&alice), &AbortUploadRequest { upload_id: upload.upload_id.clone(), ..Default::default() }).await.unwrap();
}

#[tokio::test]
async fn image_upload_becomes_webp_variants_without_metadata() {
    let h = start().await;
    let alice = h.app.user("alice").await;
    let mut owner_frames = h.app.listen_user(alice.id).await;

    // A landscape JPEG that its EXIF orientation turns into a portrait.
    let jpeg = jpeg_with_exif(1200, 800, 6);
    assert!(contains(&jpeg, b"Exif") && contains(&jpeg, GPS_MARKER), "the fixture carries EXIF with a position");

    let uploaded = h.upload(&alice, AssetPurpose::ASSET_PURPOSE_CHAT_IMAGE, "IMG_0001.JPG", "image/jpeg", jpeg).await;
    assert_eq!(uploaded.status, AssetStatus::ASSET_STATUS_PROCESSING);
    assert_eq!((uploaded.kind, uploaded.content_type.as_str()), (AssetKind::ASSET_KIND_IMAGE.into(), "image/jpeg"));
    let mut topic_frames = h.app.listen_topic(&format!("asset:{}", uploaded.id)).await;

    h.media.process_now(asset_id(&uploaded)).await.unwrap();
    let asset = h.get_asset(&alice, &uploaded.id).await.unwrap();
    assert_eq!(asset.status, AssetStatus::ASSET_STATUS_READY);
    assert_eq!(asset.progress, 100);

    let image = asset.image.as_option().expect("image");
    assert_eq!((image.width, image.height), (800, 1200), "EXIF orientation is applied to the pixels");
    assert_eq!(image.asset_id, asset.id);
    assert_eq!(image.url, asset.url);
    assert!(image.dominant_color.starts_with('#') && image.dominant_color.len() == 7, "{}", image.dominant_color);
    assert!(!image.thumbhash.is_empty());
    assert_eq!(image.variants.iter().map(|v| v.size).collect::<Vec<_>>(), vec![1200, 1024, 512, 256, 96], "nothing above the source size");
    assert_eq!(image.variants[0].url, image.url);

    for variant in &image.variants {
        let bytes = h.media_object(&variant.url).await.expect("variant stored");
        let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::WebP).expect("valid WebP");
        assert_eq!(decoded.height().max(decoded.width()) as i32, variant.size);
        assert!(decoded.height() > decoded.width(), "portrait at every size");
        // No metadata chunk and no trace of the position survives.
        assert!(!contains(&bytes, b"EXIF") && !contains(&bytes, b"Exif") && !contains(&bytes, b"XMP "), "metadata chunk in {}", variant.url);
        assert!(!contains(&bytes, GPS_MARKER));
        // Content-addressed: the name is derived from the bytes.
        assert!(variant.url.contains(&sha256_hex(&bytes)[..16]), "{}", variant.url);
    }
    let served = h.media_response(&image.url).await;
    assert_eq!(served.status(), 200, "renditions are publicly readable");
    assert_eq!(served.headers()["cache-control"], "public, max-age=31536000, immutable");
    assert_eq!(served.headers()["content-type"], "image/webp");

    // One ready event, and realtime pushes to the owner and the asset topic.
    let events = h.app.events_by("media.asset.", alice.id).await;
    let ready: Vec<&Event> = events.iter().filter(|e| e.data["asset_id"] == asset.id.as_str()).collect();
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].topic, "media.asset.ready");
    let payload: AssetEvent = ready[0].data().unwrap();
    assert_eq!((payload.owner_id, payload.purpose, payload.kind), (alice.id, AssetPurpose::ASSET_PURPOSE_CHAT_IMAGE as i32, AssetKind::ASSET_KIND_IMAGE as i32));

    let last_status = |events| pushed_assets(events).into_iter().rfind(|pushed| pushed.id == asset.id);
    let to_owner = last_status(owner_frames.events(Duration::from_millis(300)).await).expect("pushed to the owner");
    assert_eq!((to_owner.status, to_owner.file_name.as_str()), (AssetStatus::ASSET_STATUS_READY.into(), "IMG_0001.JPG"));
    let to_topic = last_status(topic_frames.events(Duration::from_millis(300)).await).expect("pushed to the asset topic");
    assert_eq!(to_topic.status, AssetStatus::ASSET_STATUS_READY);
    assert_eq!(to_topic.file_name, "", "topic subscribers get the public view");
}

#[tokio::test]
async fn transparency_small_images_and_checksums() {
    let h = start().await;
    let alice = h.app.user("alice").await;

    // A small PNG with transparency: never upscaled, alpha kept.
    let asset = h.upload_and_process(&alice, AssetPurpose::ASSET_PURPOSE_STAMP, "sticker.png", "image/png", png_with_alpha(180, 120)).await;
    let image = asset.image.as_option().unwrap();
    assert_eq!((image.width, image.height), (180, 120));
    assert_eq!(image.variants.iter().map(|v| v.size).collect::<Vec<_>>(), vec![180, 96]);
    let top = image::load_from_memory(&h.media_object(&image.url).await.unwrap()).unwrap().into_rgba8();
    assert_eq!(top.get_pixel(170, 60)[3], 0, "transparent stays transparent");
    assert_eq!(top.get_pixel(10, 60)[3], 255);

    // A correct checksum passes; the extension-less name and an empty
    // content type fall back to the extension.
    let bytes = png(64, 64);
    let request = CreateUploadRequest {
        purpose: AssetPurpose::ASSET_PURPOSE_AVATAR.into(),
        file_name: "avatar.png".into(),
        size_bytes: bytes.len() as i64,
        sha256: sha256_hex(&bytes).to_uppercase(),
        ..Default::default()
    };
    let upload: CreateUploadResponse = h.rpc("CreateUpload", Some(&alice), &request).await.unwrap();
    assert_eq!(upload.put_headers["Content-Type"], "application/octet-stream");
    assert!(h.put(&upload, bytes).await.status().is_success());
    let asset = h.complete(&alice, &upload.upload_id).await.unwrap();
    assert_eq!(asset.content_type, "image/png", "the stored type is the detected one");

    // A wrong checksum fails the upload and discards the bytes.
    let bytes = png(64, 64);
    let request = CreateUploadRequest {
        purpose: AssetPurpose::ASSET_PURPOSE_AVATAR.into(),
        file_name: "avatar.png".into(),
        content_type: "image/png".into(),
        size_bytes: bytes.len() as i64,
        sha256: sha256_hex(b"something else"),
        ..Default::default()
    };
    let upload: CreateUploadResponse = h.rpc("CreateUpload", Some(&alice), &request).await.unwrap();
    assert!(h.put(&upload, bytes).await.status().is_success());
    assert_eq!(reason(h.complete(&alice, &upload.upload_id).await), "CHECKSUM_MISMATCH");
    let failed = h.get_asset(&alice, &upload.asset_id).await.unwrap();
    assert_eq!((failed.status, failed.error.as_str()), (AssetStatus::ASSET_STATUS_FAILED.into(), "CHECKSUM_MISMATCH"));
    assert!(!h.original_exists(upload.asset_id.parse().unwrap()).await);
}

#[tokio::test]
async fn animated_gif_keeps_its_animation_where_the_purpose_allows() {
    let h = start().await;
    let alice = h.app.user("alice").await;

    let asset = h.upload_and_process(&alice, AssetPurpose::ASSET_PURPOSE_STAMP, "wave.gif", "image/gif", gif(160, 120, 8)).await;
    assert_eq!((asset.status, asset.kind), (AssetStatus::ASSET_STATUS_READY.into(), AssetKind::ASSET_KIND_ANIMATION.into()));
    let animation = h.media_object(&asset.url).await.expect("animated WebP stored");
    let decoder = image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(&animation)).unwrap();
    assert!(decoder.has_animation());
    assert_eq!(image::AnimationDecoder::into_frames(decoder).collect_frames().unwrap().len(), 8);
    // The still poster is a regular image with variants.
    let poster = asset.image.as_option().expect("poster");
    assert_eq!((poster.width, poster.height), (160, 120));
    assert_ne!(poster.url, asset.url);
    let still = image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(h.media_object(&poster.url).await.unwrap())).unwrap();
    assert!(!still.has_animation());
    // Domains that only want a picture accept it (they get the poster).
    let json = assets::require_image_json(&h.app.core, asset_id(&asset), alice.id, &[AssetPurpose::ASSET_PURPOSE_STAMP]).await.unwrap();
    assert_eq!(json["url"], poster.url.as_str());

    // A banner cannot move: the same file is flattened to its first frame.
    let banner = h.upload_and_process(&alice, AssetPurpose::ASSET_PURPOSE_BANNER, "wave.gif", "image/gif", gif(160, 120, 8)).await;
    assert_eq!(banner.kind, AssetKind::ASSET_KIND_IMAGE);
    assert_eq!(banner.url, banner.image.as_option().unwrap().url);
}

#[tokio::test]
async fn complete_upload_trusts_bytes_not_declarations() {
    let h = start().await;
    let alice = h.app.user("alice").await;

    // Nothing uploaded yet.
    let upload = h.create_upload(&alice, AssetPurpose::ASSET_PURPOSE_AVATAR, "a.png", "image/png", &[0u8; 2000]).await;
    let error = h.complete(&alice, &upload.upload_id).await.unwrap_err();
    assert_eq!((error.code.as_str(), error.reason.as_str()), ("failed_precondition", "UPLOAD_INCOMPLETE"));

    // The URL only accepts exactly the announced number of bytes.
    let too_long = h.put(&upload, vec![0u8; 5000]).await;
    assert!(!too_long.status().is_success(), "a longer body must be refused, got {}", too_long.status());

    // Claimed to be a PNG, is a script.
    let script = b"<html><script>alert(document.cookie)</script></html>".repeat(40);
    let upload = h.create_upload(&alice, AssetPurpose::ASSET_PURPOSE_AVATAR, "a.png", "image/png", &script).await;
    assert!(h.put(&upload, script).await.status().is_success());
    let error = h.complete(&alice, &upload.upload_id).await.unwrap_err();
    assert_eq!((error.code.as_str(), error.reason.as_str()), ("invalid_argument", "UNSUPPORTED_CONTENT"));
    let failed = h.get_asset(&alice, &upload.asset_id).await.unwrap();
    assert_eq!((failed.status, failed.error.as_str()), (AssetStatus::ASSET_STATUS_FAILED.into(), "UNSUPPORTED_CONTENT"));
    assert!(!h.original_exists(upload.asset_id.parse().unwrap()).await, "rejected bytes are discarded");
    let events = h.app.events_by("media.asset.failed", alice.id).await;
    assert!(events.iter().any(|e| e.data["asset_id"] == upload.asset_id.as_str() && e.data["error"] == "UNSUPPORTED_CONTENT"));
    // Completing again reports the same asset, now failed.
    assert_eq!(h.complete(&alice, &upload.upload_id).await.unwrap().status, AssetStatus::ASSET_STATUS_FAILED);

    // A real image passes sniffing but can still turn out to be broken.
    let mut broken = png(300, 300);
    let cut = broken.len() / 2;
    broken.truncate(cut);
    let asset = h.upload_and_process(&alice, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE, "broken.png", "image/png", broken).await;
    assert_eq!((asset.status, asset.error.as_str()), (AssetStatus::ASSET_STATUS_FAILED.into(), "CORRUPT_MEDIA"));

    // Other people's uploads do not exist as far as a caller can tell.
    let bob = h.app.user("bob").await;
    let upload = h.create_upload(&alice, AssetPurpose::ASSET_PURPOSE_AVATAR, "a.png", "image/png", &png(32, 32)).await;
    assert_eq!(reason(h.complete(&bob, &upload.upload_id).await), "UPLOAD_NOT_FOUND");
    assert_eq!(reason(h.complete(&alice, "not-an-id").await), "INVALID_ID");
}

#[tokio::test]
async fn completing_is_idempotent() {
    let h = start().await;
    let alice = h.app.user("alice").await;
    let upload = h.create_upload(&alice, AssetPurpose::ASSET_PURPOSE_AVATAR, "a.png", "image/png", &png(100, 100)).await;
    assert!(h.put(&upload, png(100, 100)).await.status().is_success());

    let first = h.complete(&alice, &upload.upload_id).await.unwrap();
    let second = h.complete(&alice, &upload.upload_id).await.unwrap();
    assert_eq!((first.status, second.status), (AssetStatus::ASSET_STATUS_PROCESSING.into(), AssetStatus::ASSET_STATUS_PROCESSING.into()));

    h.media.process_now(asset_id(&first)).await.unwrap();
    let third = h.complete(&alice, &upload.upload_id).await.unwrap();
    assert_eq!(third.status, AssetStatus::ASSET_STATUS_READY);
    // A job delivered twice finds nothing left to do.
    h.media.process_now(asset_id(&first)).await.unwrap();
    let events = h.app.events_by("media.asset.ready", alice.id).await;
    assert_eq!(events.iter().filter(|e| e.data["asset_id"] == first.id.as_str()).count(), 1);
}

#[tokio::test]
async fn multipart_upload_resumes_where_it_stopped() {
    let h = start_with(multipart_config()).await;
    let alice = h.app.user("alice").await;
    let bytes = noise(11 * MB + 12_345);

    let request = CreateUploadRequest {
        purpose: AssetPurpose::ASSET_PURPOSE_CHAT_FILE.into(),
        file_name: "backup.bin".into(),
        content_type: "application/octet-stream".into(),
        size_bytes: bytes.len() as i64,
        sha256: sha256_hex(&bytes),
        ..Default::default()
    };
    let upload: CreateUploadResponse = h.rpc("CreateUpload", Some(&alice), &request).await.unwrap();
    assert!(upload.put_url.is_empty() && upload.put_headers.is_empty());
    assert_eq!((upload.part_size_bytes, upload.part_count), (5 * MB as i64, 3));
    let part = |number: usize| bytes[(number - 1) * 5 * MB..(number * 5 * MB).min(bytes.len())].to_vec();
    let parts = |numbers: Vec<i32>| GetUploadPartsRequest { upload_id: upload.upload_id.clone(), part_numbers: numbers, ..Default::default() };

    // Validation of part requests.
    for (numbers, expected) in [(vec![0], "PART_NUMBER_INVALID"), (vec![4], "PART_NUMBER_INVALID"), ((1..=101).collect(), "TOO_MANY_PARTS")] {
        assert_eq!(reason(h.rpc::<_, GetUploadPartsResponse>("GetUploadParts", Some(&alice), &parts(numbers)).await), expected);
    }
    let bob = h.app.user("bob").await;
    assert_eq!(reason(h.rpc::<_, GetUploadPartsResponse>("GetUploadParts", Some(&bob), &parts(vec![1])).await), "UPLOAD_NOT_FOUND");

    // First session: part 1 goes up, then the connection "drops".
    let signed: GetUploadPartsResponse = h.rpc("GetUploadParts", Some(&alice), &parts(vec![1, 2, 3])).await.unwrap();
    assert_eq!(signed.parts.iter().map(|p| p.part_number).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert!(signed.completed_part_numbers.is_empty());
    let put_part = |url: String, body: Vec<u8>| {
        let http = h.http.clone();
        async move { http.put(url).body(body).send().await.unwrap() }
    };
    let first = put_part(signed.parts[0].put_url.clone(), part(1)).await;
    assert!(first.status().is_success(), "{}", first.status());
    let first_etag = first.headers().get("etag").expect("the store returns an ETag").to_str().unwrap().to_owned();

    let incomplete = h.complete(&alice, &upload.upload_id).await.unwrap_err();
    assert_eq!((incomplete.reason.as_str(), incomplete.message.contains("part 2")), ("UPLOAD_INCOMPLETE", true), "{incomplete}");

    // Second session: ask what is there, upload only what is missing.
    let resumed: GetUploadPartsResponse = h.rpc("GetUploadParts", Some(&alice), &parts(vec![])).await.unwrap();
    assert_eq!(resumed.completed_part_numbers, vec![1]);
    assert!(resumed.parts.is_empty());
    let signed: GetUploadPartsResponse = h.rpc("GetUploadParts", Some(&alice), &parts(vec![3, 2])).await.unwrap();
    // A part URL takes exactly its part: the wrong length is refused.
    let wrong = put_part(signed.parts[0].put_url.clone(), part(3)).await;
    assert!(!wrong.status().is_success(), "part 2's URL accepted part 3's bytes ({})", wrong.status());
    for (index, number) in [(0usize, 2usize), (1, 3)] {
        assert_eq!(signed.parts[index].part_number, number as i32);
        assert!(put_part(signed.parts[index].put_url.clone(), part(number)).await.status().is_success());
    }
    let all: GetUploadPartsResponse = h.rpc("GetUploadParts", Some(&alice), &parts(vec![])).await.unwrap();
    assert_eq!(all.completed_part_numbers, vec![1, 2, 3]);

    // An ETag that does not match what is stored is refused…
    let mismatch = CompleteUploadRequest {
        upload_id: upload.upload_id.clone(),
        parts: vec![CompletedPart { part_number: 1, etag: "\"0123456789abcdef0123456789abcdef\"".into(), ..Default::default() }],
        ..Default::default()
    };
    assert_eq!(reason(h.rpc::<_, CompleteUploadResponse>("CompleteUpload", Some(&alice), &mismatch).await), "PART_ETAG_MISMATCH");
    // …the right one (or none at all, after a resume) completes the upload.
    let complete = CompleteUploadRequest {
        upload_id: upload.upload_id.clone(),
        parts: vec![CompletedPart { part_number: 1, etag: first_etag, ..Default::default() }],
        ..Default::default()
    };
    let response: CompleteUploadResponse = h.rpc("CompleteUpload", Some(&alice), &complete).await.unwrap();
    let asset = response.asset.into_option().unwrap();
    assert_eq!((asset.status, asset.kind), (AssetStatus::ASSET_STATUS_READY.into(), AssetKind::ASSET_KIND_FILE.into()), "files need no processing");
    assert_eq!(asset.size_bytes, bytes.len() as i64);

    // The assembled object is byte-identical (the checksum was verified too).
    let downloaded = h.http.get(&asset.url).send().await.unwrap();
    assert_eq!(downloaded.status(), 200);
    assert_eq!(sha256_hex(&downloaded.bytes().await.unwrap()), sha256_hex(&bytes));
    // Parts cannot be requested for a finished upload.
    assert_eq!(reason(h.rpc::<_, GetUploadPartsResponse>("GetUploadParts", Some(&alice), &parts(vec![1])).await), "UPLOAD_NOT_ACTIVE");
}

#[tokio::test]
async fn files_stay_private() {
    let h = start().await;
    let alice = h.app.user("alice").await;
    let bob = h.app.user("bob").await;

    let html = b"<html><body>quarterly numbers</body></html>".to_vec();
    let asset = h.upload(&alice, AssetPurpose::ASSET_PURPOSE_CHAT_FILE, "../secret/報告書 Q3.html", "text/html", html.clone()).await;
    assert_eq!((asset.status, asset.kind), (AssetStatus::ASSET_STATUS_READY.into(), AssetKind::ASSET_KIND_FILE.into()));
    assert_eq!(asset.file_name, "報告書 Q3.html", "directories are stripped from the name");

    // The owner gets a presigned URL that downloads, never renders.
    assert!(asset.url.contains("X-Amz-Signature"), "{}", asset.url);
    let download = h.http.get(&asset.url).send().await.unwrap();
    assert_eq!(download.status(), 200);
    let disposition = download.headers()["content-disposition"].to_str().unwrap().to_owned();
    assert!(disposition.starts_with("attachment;") && disposition.contains("filename*=UTF-8''%E5%A0%B1%E5%91%8A%E6%9B%B8%20Q3.html"), "{disposition}");
    assert_eq!(download.bytes().await.unwrap(), html);
    // Without the signature the object is not readable.
    let bare = asset.url.split('?').next().unwrap();
    assert_eq!(h.http.get(bare).send().await.unwrap().status(), 403);

    // Someone else sees that the asset exists, but not whose it is, its name or its URL.
    let seen_by_bob = h.get_asset(&bob, &asset.id).await.unwrap();
    assert_eq!((seen_by_bob.owner_id.as_str(), seen_by_bob.url.as_str(), seen_by_bob.file_name.as_str()), ("", "", ""));
    assert_eq!(h.get_asset(&alice, &asset.id).await.unwrap().owner_id, alice.id.to_string());
    assert_eq!(seen_by_bob.size_bytes, html.len() as i64);

    // Unfinished assets are invisible to others.
    let pending = h.upload(&alice, AssetPurpose::ASSET_PURPOSE_AVATAR, "a.png", "image/png", png(40, 40)).await;
    assert_eq!(reason(h.get_asset(&bob, &pending.id).await), "ASSET_NOT_FOUND");
    assert_eq!(h.get_asset(&alice, &pending.id).await.unwrap().status, AssetStatus::ASSET_STATUS_PROCESSING);
    let anonymous = h.rpc::<_, GetAssetResponse>("GetAsset", None, &GetAssetRequest { asset_id: asset.id.clone(), ..Default::default() }).await.unwrap_err();
    assert_eq!(anonymous.status, 401);

    // Batch lookups keep the request order and skip what the caller may not see.
    let unknown = uuid::Uuid::now_v7().to_string();
    let batch = BatchGetAssetsRequest { asset_ids: vec![pending.id.clone(), unknown, asset.id.clone()], ..Default::default() };
    let for_alice: BatchGetAssetsResponse = h.rpc("BatchGetAssets", Some(&alice), &batch).await.unwrap();
    assert_eq!(for_alice.assets.iter().map(|a| a.id.clone()).collect::<Vec<_>>(), vec![pending.id.clone(), asset.id.clone()]);
    let for_bob: BatchGetAssetsResponse = h.rpc("BatchGetAssets", Some(&bob), &batch).await.unwrap();
    assert_eq!(for_bob.assets.iter().map(|a| a.id.clone()).collect::<Vec<_>>(), vec![asset.id.clone()]);
    let too_many = BatchGetAssetsRequest { asset_ids: vec![asset.id.clone(); 101], ..Default::default() };
    assert_eq!(reason(h.rpc::<_, BatchGetAssetsResponse>("BatchGetAssets", Some(&alice), &too_many).await), "TOO_MANY_ASSETS");

    // Stored attachments get fresh URLs on demand.
    let row = assets::require_ready(&h.app.core, asset_id(&asset), alice.id, &[AssetPurpose::ASSET_PURPOSE_CHAT_FILE]).await.unwrap();
    let mut attachments = vec![row.to_attachment()];
    attachments[0].url = "https://expired.example/old".into();
    assets::refresh_attachments(&h.app.core, &mut attachments).await.unwrap();
    assert!(attachments[0].url.contains("X-Amz-Signature"));
    assert_eq!(assets::read_original(&h.app.core, &row, 1024).await.unwrap(), html);
    assert_eq!(assets::read_original(&h.app.core, &row, 10).await.unwrap_err().reason, "FILE_TOO_LARGE");
}

#[tokio::test]
async fn subtitles_and_lottie_stamps() {
    let h = start().await;
    let alice = h.app.user("alice").await;

    let vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:02.000\nこんにちは!\n".as_bytes().to_vec();
    let subtitle = h.upload(&alice, AssetPurpose::ASSET_PURPOSE_SUBTITLE, "ja.vtt", "", vtt).await;
    assert_eq!((subtitle.status, subtitle.content_type.as_str()), (AssetStatus::ASSET_STATUS_READY.into(), "text/vtt"));
    let srt = b"1\r\n00:00:00,000 --> 00:00:02,000\r\nhello\r\n".to_vec();
    let subtitle = h.upload(&alice, AssetPurpose::ASSET_PURPOSE_SUBTITLE, "en.srt", "application/x-subrip", srt).await;
    assert_eq!(subtitle.content_type, "application/x-subrip");
    // Not a subtitle, whatever the name says.
    let upload = h.create_upload(&alice, AssetPurpose::ASSET_PURPOSE_SUBTITLE, "notes.vtt", "text/vtt", b"just some text").await;
    assert!(h.put(&upload, b"just some text".to_vec()).await.status().is_success());
    assert_eq!(reason(h.complete(&alice, &upload.upload_id).await), "UNSUPPORTED_CONTENT");

    // A dotLottie stamp (a ZIP with a manifest) is published as it is.
    let mut lottie = b"PK\x03\x04\x14\0\0\0\0\0".to_vec();
    lottie.extend_from_slice(b"\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x0d\0\0\0manifest.json{\"animations\":[]}");
    let stamp = h.upload(&alice, AssetPurpose::ASSET_PURPOSE_STAMP, "wink.lottie", "", lottie.clone()).await;
    assert_eq!((stamp.status, stamp.kind), (AssetStatus::ASSET_STATUS_READY.into(), AssetKind::ASSET_KIND_FILE.into()));
    assert_eq!(stamp.content_type, "application/zip+dotlottie");
    assert_eq!(h.media_object(&stamp.url).await.expect("published"), lottie);
    assert!(stamp.url.ends_with(".lottie"));
    // Any other ZIP is not a stamp.
    let zip = b"PK\x03\x04\x14\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x09\0\0\0hello.txthello".to_vec();
    let upload = h.create_upload(&alice, AssetPurpose::ASSET_PURPOSE_STAMP, "x.zip", "application/zip", &zip).await;
    assert!(h.put(&upload, zip).await.status().is_success());
    assert_eq!(reason(h.complete(&alice, &upload.upload_id).await), "UNSUPPORTED_CONTENT");
    let rejected = h.get_asset(&alice, &upload.asset_id).await.unwrap();
    assert_eq!((rejected.status, rejected.error.as_str()), (AssetStatus::ASSET_STATUS_FAILED.into(), "UNSUPPORTED_CONTENT"));
}

#[tokio::test]
async fn abort_and_delete_remove_everything() {
    let h = start_with(multipart_config()).await;
    let alice = h.app.user("alice").await;
    let bob = h.app.user("bob").await;
    let abort = |upload_id: &str| AbortUploadRequest { upload_id: upload_id.to_owned(), ..Default::default() };
    let delete = |asset_id: &str| DeleteAssetRequest { asset_id: asset_id.to_owned(), ..Default::default() };

    // Abort: only the owner, only while unfinished, and repeatable.
    let bytes = noise(6 * MB);
    let upload = h.create_upload(&alice, AssetPurpose::ASSET_PURPOSE_CHAT_FILE, "big.bin", "application/octet-stream", &bytes).await;
    assert_eq!(upload.part_count, 2);
    assert_eq!(reason(h.rpc::<_, AbortUploadResponse>("AbortUpload", Some(&bob), &abort(&upload.upload_id)).await), "UPLOAD_NOT_FOUND");
    let _: AbortUploadResponse = h.rpc("AbortUpload", Some(&alice), &abort(&upload.upload_id)).await.unwrap();
    let _: AbortUploadResponse = h.rpc("AbortUpload", Some(&alice), &abort(&upload.upload_id)).await.unwrap();
    assert_eq!(reason(h.get_asset(&alice, &upload.asset_id).await), "ASSET_NOT_FOUND");
    assert_eq!(reason(h.complete(&alice, &upload.upload_id).await), "UPLOAD_NOT_FOUND");
    assert!(h.app.events_by("media.asset.deleted", alice.id).await.iter().all(|e| e.data["asset_id"] != upload.asset_id.as_str()), "an aborted upload was never an asset");

    // Delete: removes the original and every rendition.
    let asset = h.upload_and_process(&alice, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE, "photo.png", "image/png", png(600, 400)).await;
    let variant_urls: Vec<String> = asset.image.as_option().unwrap().variants.iter().map(|v| v.url.clone()).collect();
    assert!(h.original_exists(asset_id(&asset)).await);
    assert_eq!(reason(h.rpc::<_, AbortUploadResponse>("AbortUpload", Some(&alice), &abort(&asset.id)).await), "UPLOAD_NOT_ACTIVE");
    assert_eq!(reason(h.rpc::<_, DeleteAssetResponse>("DeleteAsset", Some(&bob), &delete(&asset.id)).await), "ASSET_NOT_FOUND");
    assert!(h.media_object(&variant_urls[0]).await.is_some(), "still there after a stranger's attempt");

    let _: DeleteAssetResponse = h.rpc("DeleteAsset", Some(&alice), &delete(&asset.id)).await.unwrap();
    assert_eq!(reason(h.get_asset(&alice, &asset.id).await), "ASSET_NOT_FOUND");
    assert_eq!(reason(h.rpc::<_, DeleteAssetResponse>("DeleteAsset", Some(&alice), &delete(&asset.id)).await), "ASSET_NOT_FOUND");
    for url in &variant_urls {
        assert!(h.media_object(url).await.is_none(), "{url} should be gone");
    }
    assert!(!h.original_exists(asset_id(&asset)).await);
    let purged: bool = sqlx::query_scalar("SELECT purged_at IS NOT NULL FROM media_assets WHERE id = $1").bind(asset_id(&asset)).fetch_one(&h.app.core.db).await.unwrap();
    assert!(purged, "the row stays as a tombstone, marked purged");
    let deleted = h.app.events_by("media.asset.deleted", alice.id).await;
    assert_eq!(deleted.iter().filter(|e| e.data["asset_id"] == asset.id.as_str()).count(), 1);
    // A deleted asset cannot be attached any more.
    let gone = assets::require_ready(&h.app.core, asset_id(&asset), alice.id, &[]).await.unwrap_err();
    assert_eq!(gone.reason, "ASSET_NOT_FOUND");
}

#[tokio::test]
async fn watch_asset_streams_until_ready() {
    let h = start().await;
    let alice = h.app.user("alice").await;
    let bob = h.app.user("bob").await;
    let asset = h.upload(&alice, AssetPurpose::ASSET_PURPOSE_CHAT_IMAGE, "p.png", "image/png", png(900, 600)).await;
    let request = WatchAssetRequest { asset_id: asset.id.clone(), ..Default::default() };

    let stranger = h.app.stream::<_, WatchAssetResponse>("tokuly.media.v1.MediaService/WatchAsset", Some(&bob), &request, 1, Duration::from_secs(2)).await;
    assert_eq!(stranger.unwrap_err().reason, "ASSET_NOT_FOUND");

    let watching = h.app.stream::<_, WatchAssetResponse>("tokuly.media.v1.MediaService/WatchAsset", Some(&alice), &request, 50, Duration::from_secs(20));
    let processing = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        h.media.process_now(asset_id(&asset)).await.unwrap();
    };
    let (messages, ()) = tokio::join!(watching, processing);
    let states: Vec<(AssetStatus, i32)> =
        messages.unwrap().into_iter().map(|m| m.asset.into_option().unwrap()).map(|a| (a.status.as_known().unwrap(), a.progress)).collect();
    assert_eq!(states.first(), Some(&(AssetStatus::ASSET_STATUS_PROCESSING, 0)), "the current state comes first: {states:?}");
    assert_eq!(states.last(), Some(&(AssetStatus::ASSET_STATUS_READY, 100)), "the stream ends with the final state: {states:?}");
    assert!(states.windows(2).all(|pair| pair[0].1 <= pair[1].1), "progress never goes backwards: {states:?}");

    // Watching something already finished yields exactly that state.
    let done = h.app.stream::<_, WatchAssetResponse>("tokuly.media.v1.MediaService/WatchAsset", Some(&alice), &request, 5, Duration::from_secs(5)).await.unwrap();
    assert_eq!(done.len(), 1);
}

#[tokio::test]
async fn attaching_assets_from_other_domains() {
    let h = start().await;
    let core = &h.app.core;
    let alice = h.app.user("alice").await;
    let bob = h.app.user("bob").await;
    let avatar = [AssetPurpose::ASSET_PURPOSE_AVATAR];

    let pending = h.upload(&alice, AssetPurpose::ASSET_PURPOSE_AVATAR, "a.png", "image/png", png(128, 128)).await;
    let id = asset_id(&pending);
    assert_eq!(assets::require_ready(core, id, alice.id, &avatar).await.unwrap_err().reason, "ASSET_NOT_READY");
    assert!(assets::require(core, id, alice.id, &avatar, false).await.is_ok(), "not-yet-ready assets can be referenced when allowed");

    h.media.process_now(id).await.unwrap();
    let row = assets::require_ready(core, id, alice.id, &avatar).await.unwrap();
    assert!(row.is_ready() && row.image().is_some());
    assert_eq!(assets::require_ready(core, id, bob.id, &avatar).await.unwrap_err().reason, "ASSET_NOT_FOUND", "someone else's asset");
    assert_eq!(assets::require_ready(core, id, alice.id, &[AssetPurpose::ASSET_PURPOSE_BANNER]).await.unwrap_err().reason, "ASSET_WRONG_PURPOSE");
    let json = assets::require_image_json(core, id, alice.id, &avatar).await.unwrap();
    assert_eq!(assets::image_from_json(Some(&json)).unwrap().width, 128);

    let file = h.upload(&alice, AssetPurpose::ASSET_PURPOSE_CHAT_FILE, "a.txt", "text/plain", b"hello".to_vec()).await;
    assert_eq!(assets::require_image_json(core, asset_id(&file), alice.id, &[]).await.unwrap_err().reason, "ASSET_NOT_IMAGE");
    let rows = assets::find_many(core, &[asset_id(&file), uuid::Uuid::now_v7(), id]).await.unwrap();
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![asset_id(&file), id]);

    // Fixtures for other domains' tests look like the real thing.
    let fixture = tokuly_media::testing::ready_video(core, alice.id, AssetPurpose::ASSET_PURPOSE_VIDEO).await.unwrap();
    let attachment = assets::require_ready(core, fixture.id, alice.id, &[AssetPurpose::ASSET_PURPOSE_VIDEO]).await.unwrap().to_attachment();
    assert!(attachment.video.as_option().is_some_and(|v| v.hls_url.ends_with("master.m3u8") && v.poster.is_set()));
    let fixture = tokuly_media::testing::ready_image(core, alice.id, AssetPurpose::ASSET_PURPOSE_AVATAR).await.unwrap();
    assert!(assets::require_image_json(core, fixture.id, alice.id, &avatar).await.is_ok());
}

#[tokio::test]
async fn server_side_assets_and_account_deletion() {
    let h = start().await;
    let core = &h.app.core;
    let alice = h.app.user("alice").await;
    let dir = fixture_dir();
    let path = dir.path().join("generated.png");
    tokio::fs::write(&path, png(320, 200)).await.unwrap();

    let row = tokuly_media::create_asset_from_file(core, alice.id, AssetPurpose::ASSET_PURPOSE_THUMBNAIL, &path, "thumb.png").await.unwrap();
    assert_eq!(row.status, AssetStatus::ASSET_STATUS_PROCESSING as i32);
    h.media.process_now(row.id).await.unwrap();
    let asset = h.get_asset(&alice, &row.id.to_string()).await.unwrap();
    assert_eq!(asset.status, AssetStatus::ASSET_STATUS_READY);
    assert_eq!(asset.image.as_option().unwrap().width, 320);
    assert!(path.exists(), "the caller's file is left alone");
    // Content still has to suit the purpose.
    let wrong = tokuly_media::create_asset_from_file(core, alice.id, AssetPurpose::ASSET_PURPOSE_CHAT_AUDIO, &path, "x.png").await.unwrap_err();
    assert_eq!(wrong.reason, "UNSUPPORTED_CONTENT");

    // The export lists what the user uploaded.
    let exporter = core.exporters.all().into_iter().find(|e| e.name() == "media").expect("media exporter registered");
    let export = exporter.export(alice.id).await.unwrap();
    assert!(export["assets"].as_array().unwrap().iter().any(|a| a["id"] == asset.id.as_str() && a["file_name"] == "thumb.png"));

    // Account deletion takes every asset and its objects along.
    let other = h.upload_and_process(&alice, AssetPurpose::ASSET_PURPOSE_AVATAR, "a.png", "image/png", png(64, 64)).await;
    let bob = h.app.user("bob").await;
    let bobs = h.upload_and_process(&bob, AssetPurpose::ASSET_PURPOSE_AVATAR, "b.png", "image/png", png(64, 64)).await;
    let event = Event {
        id: uuid::Uuid::now_v7(),
        topic: "identity.user.deleted".into(),
        actor_id: Some(alice.id),
        occurred_at: chrono::Utc::now(),
        data: serde_json::json!({ "user_id": alice.id }),
    };
    h.media.on_user_deleted(event.clone()).await.unwrap();
    h.media.on_user_deleted(event).await.unwrap(); // at-least-once delivery
    for gone in [&asset, &other] {
        assert!(assets::find(core, asset_id(gone)).await.unwrap().is_none());
        assert!(h.media_object(&gone.url).await.is_none());
        assert!(!h.original_exists(asset_id(gone)).await);
    }
    assert!(h.media_object(&bobs.url).await.is_some(), "other accounts are untouched");
}

#[tokio::test]
async fn large_server_side_files_are_stored_in_parts() {
    let h = start().await;
    let alice = h.app.user("alice").await;
    let dir = fixture_dir();
    // Above the single-request size the ingest uses: two parts.
    let bytes = noise(70 * MB);
    let path = dir.path().join("recording.bin");
    tokio::fs::write(&path, &bytes).await.unwrap();

    let row = tokuly_media::create_asset_from_file(&h.app.core, alice.id, AssetPurpose::ASSET_PURPOSE_CHAT_FILE, &path, "recording.bin").await.unwrap();
    assert_eq!(row.size_bytes, bytes.len() as i64);
    h.media.process_now(row.id).await.unwrap();
    let asset = h.get_asset(&alice, &row.id.to_string()).await.unwrap();
    assert_eq!(asset.status, AssetStatus::ASSET_STATUS_READY);
    let stored = h.http.get(&asset.url).send().await.unwrap().bytes().await.unwrap();
    assert_eq!(stored.len(), bytes.len());
    assert_eq!(sha256_hex(&stored), sha256_hex(&bytes), "the parts were assembled in order");
}

#[tokio::test]
async fn janitor_expires_uploads_and_revives_stalled_jobs() {
    let _sweeping = SWEEPING.lock().await;
    let h = start_with(multipart_config()).await;
    let core = &h.app.core;
    let alice = h.app.user("alice").await;

    // An upload abandoned two days ago, with one part already stored.
    let bytes = noise(6 * MB);
    let stale = h.create_upload(&alice, AssetPurpose::ASSET_PURPOSE_CHAT_FILE, "old.bin", "application/octet-stream", &bytes).await;
    let signed: GetUploadPartsResponse =
        h.rpc("GetUploadParts", Some(&alice), &GetUploadPartsRequest { upload_id: stale.upload_id.clone(), part_numbers: vec![1], ..Default::default() }).await.unwrap();
    assert!(h.http.put(&signed.parts[0].put_url).body(bytes[..5 * MB].to_vec()).send().await.unwrap().status().is_success());
    sqlx::query("UPDATE media_assets SET created_at = now() - interval '2 days' WHERE id = $1").bind(stale.asset_id.parse::<uuid::Uuid>().unwrap()).execute(&core.db).await.unwrap();
    let fresh = h.create_upload(&alice, AssetPurpose::ASSET_PURPOSE_AVATAR, "new.png", "image/png", &png(32, 32)).await;

    // An asset whose worker died mid-job (its last heartbeat is long ago).
    let stalled = h.upload(&alice, AssetPurpose::ASSET_PURPOSE_AVATAR, "a.png", "image/png", png(48, 48)).await;
    sqlx::query("UPDATE media_assets SET attempts = 1, heartbeat_at = now() - interval '10 minutes' WHERE id = $1").bind(asset_id(&stalled)).execute(&core.db).await.unwrap();
    // …and one that is being worked on right now.
    let busy = h.upload(&alice, AssetPurpose::ASSET_PURPOSE_AVATAR, "b.png", "image/png", png(48, 48)).await;
    sqlx::query("UPDATE media_assets SET attempts = 1, heartbeat_at = now() WHERE id = $1").bind(asset_id(&busy)).execute(&core.db).await.unwrap();

    let sweep = h.media.sweep().await.unwrap();
    assert!(sweep.expired_uploads >= 1 && sweep.requeued_assets >= 1, "{sweep:?}");
    assert_eq!(reason(h.get_asset(&alice, &stale.asset_id).await), "ASSET_NOT_FOUND");
    assert_eq!(reason(h.rpc::<_, GetUploadPartsResponse>("GetUploadParts", Some(&alice), &GetUploadPartsRequest { upload_id: stale.upload_id.clone(), ..Default::default() }).await), "UPLOAD_NOT_FOUND");
    assert_eq!(h.get_asset(&alice, &fresh.asset_id).await.unwrap().status, AssetStatus::ASSET_STATUS_UPLOADING, "recent uploads are left alone");

    // A duplicate job leaves the busy asset to its worker; the stalled one is taken over.
    h.media.process_now(asset_id(&busy)).await.unwrap();
    assert_eq!(h.get_asset(&alice, &busy.id).await.unwrap().status, AssetStatus::ASSET_STATUS_PROCESSING);
    h.media.process_now(asset_id(&stalled)).await.unwrap();
    assert_eq!(h.get_asset(&alice, &stalled.id).await.unwrap().status, AssetStatus::ASSET_STATUS_READY);

    // An asset that keeps killing its workers is given up on.
    let poison = h.upload(&alice, AssetPurpose::ASSET_PURPOSE_AVATAR, "c.png", "image/png", png(48, 48)).await;
    sqlx::query("UPDATE media_assets SET attempts = 3, heartbeat_at = now() - interval '10 minutes' WHERE id = $1").bind(asset_id(&poison)).execute(&core.db).await.unwrap();
    h.media.process_now(asset_id(&poison)).await.unwrap();
    let failed = h.get_asset(&alice, &poison.id).await.unwrap();
    assert_eq!((failed.status, failed.error.as_str()), (AssetStatus::ASSET_STATUS_FAILED.into(), "PROCESSING_FAILED"));
}

#[tokio::test]
async fn janitor_cleans_up_after_late_uploads_and_old_failures() {
    let _sweeping = SWEEPING.lock().await;
    let h = start().await;
    let core = &h.app.core;
    let alice = h.app.user("alice").await;

    // An upload is aborted, but its presigned URL is still valid: the client
    // (or anyone holding the URL) stores the bytes after the purge.
    let bytes = png(50, 50);
    let upload = h.create_upload(&alice, AssetPurpose::ASSET_PURPOSE_AVATAR, "late.png", "image/png", &bytes).await;
    let id: uuid::Uuid = upload.asset_id.parse().unwrap();
    let _: AbortUploadResponse = h.rpc("AbortUpload", Some(&alice), &AbortUploadRequest { upload_id: upload.upload_id.clone(), ..Default::default() }).await.unwrap();
    assert!(h.put(&upload, bytes).await.status().is_success(), "the URL outlives the upload");
    assert!(h.original_exists(id).await);

    // Nothing happens while the URL may still be used…
    h.media.sweep().await.unwrap();
    assert!(h.original_exists(id).await);
    // …and once it has expired the asset is purged a second time.
    sqlx::query("UPDATE media_assets SET created_at = created_at - interval '2 hours', repurge_after = repurge_after - interval '2 hours' WHERE id = $1")
        .bind(id)
        .execute(&core.db)
        .await
        .unwrap();
    h.media.sweep().await.unwrap();
    assert!(!h.original_exists(id).await);
    let pending: bool = sqlx::query_scalar("SELECT repurge_after IS NOT NULL FROM media_assets WHERE id = $1").bind(id).fetch_one(&core.db).await.unwrap();
    assert!(!pending, "purged for good");

    // Failed assets linger for a week, then go.
    let failed = h.upload_and_process(&alice, AssetPurpose::ASSET_PURPOSE_AVATAR, "bad.png", "image/png", png(40, 40)[..60].to_vec()).await;
    assert_eq!(failed.status, AssetStatus::ASSET_STATUS_FAILED);
    h.media.sweep().await.unwrap();
    assert!(h.get_asset(&alice, &failed.id).await.is_ok(), "recent failures stay visible");
    // `updated_at` is maintained by a trigger; age the row behind its back.
    let mut tx = core.db.begin().await.unwrap();
    sqlx::query("SET LOCAL session_replication_role = replica").execute(&mut *tx).await.unwrap();
    sqlx::query("UPDATE media_assets SET updated_at = now() - interval '8 days' WHERE id = $1").bind(asset_id(&failed)).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let sweep = h.media.sweep().await.unwrap();
    assert!(sweep.expired_failures >= 1, "{sweep:?}");
    assert_eq!(reason(h.get_asset(&alice, &failed.id).await), "ASSET_NOT_FOUND");
}

fn in_use(result: Result<impl std::fmt::Debug, tokuly_testkit::RpcError>) -> bool {
    let error = result.expect_err("expected an error");
    (error.code.as_str(), error.reason.as_str()) == ("failed_precondition", "ASSET_IN_USE")
}

async fn holder_count(h: &Harness, asset_id: uuid::Uuid) -> (i32, i64) {
    sqlx::query_as("SELECT holder_count, (SELECT count(*) FROM media_asset_holders WHERE asset_id = $1) FROM media_assets WHERE id = $1")
        .bind(asset_id)
        .fetch_one(&h.app.core.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn held_assets_cannot_be_deleted_by_their_owner() {
    let h = start().await;
    let core = &h.app.core;
    let alice = h.app.user("alice").await;
    let delete = |asset_id: &str| DeleteAssetRequest { asset_id: asset_id.to_owned(), ..Default::default() };

    let asset = h.upload_and_process(&alice, AssetPurpose::ASSET_PURPOSE_AVATAR, "me.png", "image/png", png(96, 96)).await;
    let id = asset_id(&asset);
    assert!(!assets::is_attached(core, id).await.unwrap());

    // Attaching is idempotent per holder.
    let avatar = format!("identity:avatar:{}", alice.id);
    assets::attach(&core.db, id, &avatar).await.unwrap();
    assets::attach(&core.db, id, &avatar).await.unwrap();
    assert_eq!(holder_count(&h, id).await, (1, 1));
    assert!(assets::is_attached(core, id).await.unwrap());
    assert!(assets::find(core, id).await.unwrap().unwrap().is_attached());

    // Neither the RPC nor a delete on the owner's behalf removes it.
    assert!(in_use(h.rpc::<_, DeleteAssetResponse>("DeleteAsset", Some(&alice), &delete(&asset.id)).await));
    let refused = assets::delete(core, id, Some(alice.id)).await.unwrap_err();
    assert_eq!(refused.reason, "ASSET_IN_USE");
    assert!(h.media_object(&asset.url).await.is_some(), "nothing was removed");
    assert!(h.app.events_by("media.asset.deleted", alice.id).await.iter().all(|e| e.data["asset_id"] != asset.id.as_str()));
    // Someone else still learns nothing about it.
    let bob = h.app.user("bob").await;
    assert_eq!(reason(h.rpc::<_, DeleteAssetResponse>("DeleteAsset", Some(&bob), &delete(&asset.id)).await), "ASSET_NOT_FOUND");

    // A second holder (the photo was also sent in a chat); each must let go.
    assets::attach(&core.db, id, "chat:message:42").await.unwrap();
    assert_eq!(holder_count(&h, id).await, (2, 2));
    assets::detach(&core.db, id, &avatar).await.unwrap();
    assets::detach(&core.db, id, &avatar).await.unwrap(); // detaching twice is harmless
    assets::detach(&core.db, id, "never:attached").await.unwrap();
    assert_eq!(holder_count(&h, id).await, (1, 1));
    assert!(in_use(h.rpc::<_, DeleteAssetResponse>("DeleteAsset", Some(&alice), &delete(&asset.id)).await));

    assets::detach(&core.db, id, "chat:message:42").await.unwrap();
    assert_eq!(holder_count(&h, id).await, (0, 0));
    assert!(!assets::is_attached(core, id).await.unwrap());
    let detached: bool = sqlx::query_scalar("SELECT detached_at IS NOT NULL FROM media_assets WHERE id = $1").bind(id).fetch_one(&core.db).await.unwrap();
    assert!(detached, "the moment the last holder let go is recorded");
    let _: DeleteAssetResponse = h.rpc("DeleteAsset", Some(&alice), &delete(&asset.id)).await.unwrap();
    assert!(h.media_object(&asset.url).await.is_none());

    // Nothing can be attached to an asset that is gone (or never existed).
    assert_eq!(assets::attach(&core.db, id, "chat:message:43").await.unwrap_err().reason, "ASSET_NOT_FOUND");
    assert_eq!(assets::attach(&core.db, uuid::Uuid::now_v7(), "chat:message:43").await.unwrap_err().reason, "ASSET_NOT_FOUND");
    assert_eq!(assets::attach(&core.db, id, "").await.unwrap_err().reason, "HOLDER_INVALID");
    assert!(!assets::is_attached(core, id).await.unwrap());

    // An upload something already refers to cannot be aborted either.
    let upload = h.create_upload(&alice, AssetPurpose::ASSET_PURPOSE_VIDEO, "draft.mp4", "video/mp4", &[0u8; 64]).await;
    let upload_id: uuid::Uuid = upload.asset_id.parse().unwrap();
    assets::attach(&core.db, upload_id, "video:draft:1").await.unwrap();
    let abort = AbortUploadRequest { upload_id: upload.upload_id.clone(), ..Default::default() };
    assert!(in_use(h.rpc::<_, AbortUploadResponse>("AbortUpload", Some(&alice), &abort).await));
    assert_eq!(assets::detach_holder(&core.db, "video:draft:1").await.unwrap(), 1);
    let _: AbortUploadResponse = h.rpc("AbortUpload", Some(&alice), &abort).await.unwrap();
}

#[tokio::test]
async fn holds_join_the_callers_transaction_and_holders_release_in_bulk() {
    let h = start().await;
    let core = &h.app.core;
    let alice = h.app.user("alice").await;
    let mut ids = Vec::new();
    for _ in 0..3 {
        ids.push(tokuly_media::testing::ready_image(core, alice.id, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await.unwrap().id);
    }
    let note = format!("notes:note:{}", uuid::Uuid::now_v7());

    // Rolled back with the transaction that would have stored the reference…
    let mut tx = core.db.begin().await.unwrap();
    for id in &ids {
        assets::attach(&mut *tx, *id, &note).await.unwrap();
    }
    tx.rollback().await.unwrap();
    assert_eq!(holder_count(&h, ids[0]).await, (0, 0));

    // …or committed with it.
    let mut tx = core.db.begin().await.unwrap();
    for id in &ids {
        assets::attach(&mut *tx, *id, &note).await.unwrap();
    }
    assets::attach(&mut *tx, ids[0], "notes:note:quoting").await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(holder_count(&h, ids[0]).await, (2, 2));
    assert_eq!(holder_count(&h, ids[1]).await, (1, 1));

    // Deleting the note releases everything it held, in the deleting transaction.
    let mut tx = core.db.begin().await.unwrap();
    assert_eq!(assets::detach_holder(&mut *tx, &note).await.unwrap(), 3);
    tx.commit().await.unwrap();
    assert_eq!(assets::detach_holder(&core.db, &note).await.unwrap(), 0);
    assert_eq!(holder_count(&h, ids[0]).await, (1, 1), "still quoted by another note");
    assert_eq!(holder_count(&h, ids[1]).await, (0, 0));
    assert_eq!(holder_count(&h, ids[2]).await, (0, 0));

    // A domain acting on its own authority (a takedown) deletes even what is
    // held; the holders are told through the event.
    assert!(assets::delete(core, ids[0], None).await.unwrap());
    assert!(assets::find(core, ids[0]).await.unwrap().is_none());
    assert_eq!(holder_count(&h, ids[0]).await.1, 0, "its holds are gone with it");
    assert_eq!(h.app.events_by("media.asset.deleted", alice.id).await.iter().filter(|e| e.data["asset_id"] == ids[0].to_string().as_str()).count(), 1);
    assert!(!assets::delete(core, ids[0], None).await.unwrap(), "nothing left to delete");
}

#[tokio::test]
async fn attaching_and_deleting_at_once_never_loses_a_held_asset() {
    let h = start().await;
    let core = &h.app.core;
    let alice = h.app.user("alice").await;
    // Race an attach against the owner's delete on many assets. Whatever the
    // interleaving, exactly one of them wins.
    let mut races = Vec::new();
    for round in 0..40 {
        let id = tokuly_media::testing::ready_image(core, alice.id, AssetPurpose::ASSET_PURPOSE_CHAT_IMAGE).await.unwrap().id;
        let (core_a, core_b) = (core.clone(), core.clone());
        let holder = format!("chat:message:{round}");
        let attach = tokio::spawn(async move { assets::attach(&core_a.db, id, &holder).await.map_err(|e| e.reason.to_string()) });
        let delete = tokio::spawn(async move { assets::delete(&core_b, id, Some(alice.id)).await.map_err(|e| e.reason.to_string()) });
        races.push((id, attach, delete));
    }
    let (mut attach_won, mut delete_won) = (0, 0);
    for (id, attach, delete) in races {
        let (attached, deleted) = (attach.await.unwrap(), delete.await.unwrap());
        let alive = assets::find(core, id).await.unwrap();
        match (&attached, &deleted) {
            (Ok(()), Err(reason)) if reason == "ASSET_IN_USE" => {
                attach_won += 1;
                assert!(alive.is_some_and(|row| row.is_attached()), "an attached asset survived");
                assert_eq!(holder_count(&h, id).await, (1, 1));
            }
            (Err(reason), Ok(true)) if reason == "ASSET_NOT_FOUND" => {
                delete_won += 1;
                assert!(alive.is_none());
                assert_eq!(holder_count(&h, id).await, (0, 0), "a refused attach leaves no trace");
            }
            other => panic!("attach and delete both went through, or both failed: {other:?}"),
        }
    }
    assert_eq!(attach_won + delete_won, 40);
}

#[tokio::test]
async fn unused_assets_are_collected_and_held_ones_are_not() {
    let _sweeping = SWEEPING.lock().await;
    let h = start().await;
    let core = &h.app.core;
    let alice = h.app.user("alice").await;
    let age_ready = |id: uuid::Uuid| sqlx::query("UPDATE media_assets SET ready_at = now() - interval '3 days' WHERE id = $1").bind(id).execute(&core.db);

    // Three uploads finished three days ago: one never used, one in use,
    // one that was in use until just now.
    let unused = h.upload_and_process(&alice, AssetPurpose::ASSET_PURPOSE_CHAT_IMAGE, "unused.png", "image/png", png(80, 80)).await;
    let held = h.upload_and_process(&alice, AssetPurpose::ASSET_PURPOSE_CHAT_IMAGE, "held.png", "image/png", png(80, 80)).await;
    let released = h.upload_and_process(&alice, AssetPurpose::ASSET_PURPOSE_CHAT_IMAGE, "released.png", "image/png", png(80, 80)).await;
    let recent = h.upload_and_process(&alice, AssetPurpose::ASSET_PURPOSE_CHAT_IMAGE, "recent.png", "image/png", png(80, 80)).await;
    assets::attach(&core.db, asset_id(&held), "chat:message:1").await.unwrap();
    assets::attach(&core.db, asset_id(&released), "chat:message:2").await.unwrap();
    for asset in [&unused, &held, &released] {
        age_ready(asset_id(asset)).await.unwrap();
    }
    assets::detach_holder(&core.db, "chat:message:2").await.unwrap();

    let sweep = h.media.sweep().await.unwrap();
    assert_eq!(sweep.unattached_assets, 1, "{sweep:?}");
    assert_eq!(reason(h.get_asset(&alice, &unused.id).await), "ASSET_NOT_FOUND");
    assert!(h.media_object(&unused.url).await.is_none(), "its objects are removed");
    assert!(!h.original_exists(asset_id(&unused)).await);
    assert_eq!(h.app.events_by("media.asset.deleted", alice.id).await.iter().filter(|e| e.data["asset_id"] == unused.id.as_str()).count(), 1);
    for kept in [&held, &released, &recent] {
        assert!(h.get_asset(&alice, &kept.id).await.is_ok(), "{} must survive", kept.file_name);
        assert!(h.media_object(&kept.url).await.is_some());
    }

    // The clock of a released asset starts when its last holder let go.
    sqlx::query("UPDATE media_assets SET detached_at = now() - interval '49 hours' WHERE id = $1").bind(asset_id(&released)).execute(&core.db).await.unwrap();
    let sweep = h.media.sweep().await.unwrap();
    assert_eq!(sweep.unattached_assets, 1, "{sweep:?}");
    assert_eq!(reason(h.get_asset(&alice, &released.id).await), "ASSET_NOT_FOUND");
    assert!(h.get_asset(&alice, &held.id).await.is_ok(), "held assets are never collected, however old");

    // Re-attaching before the deadline keeps an asset for good.
    assets::detach_holder(&core.db, "chat:message:1").await.unwrap();
    assets::attach(&core.db, asset_id(&held), "chat:message:3").await.unwrap();
    sqlx::query("UPDATE media_assets SET detached_at = now() - interval '49 hours' WHERE id = $1 AND detached_at IS NOT NULL").bind(asset_id(&held)).execute(&core.db).await.unwrap();
    assert_eq!(h.media.sweep().await.unwrap().unattached_assets, 0);
    assert!(h.get_asset(&alice, &held.id).await.is_ok());

    // With the lifetime set to 0 nothing unused is ever collected.
    let keep_all = start_with(MediaConfig { unattached_ttl_hours: 0, ..Default::default() }).await;
    age_ready(asset_id(&recent)).await.unwrap();
    assert_eq!(keep_all.media.sweep().await.unwrap().unattached_assets, 0);
    assert!(h.get_asset(&alice, &recent.id).await.is_ok());
    assert_eq!(h.media.sweep().await.unwrap().unattached_assets, 1);
}

#[tokio::test]
async fn account_deletion_spares_assets_that_are_still_held() {
    let _sweeping = SWEEPING.lock().await;
    let h = start().await;
    let core = &h.app.core;
    let creator = h.app.user("creator").await;

    // The creator's stamp art is part of a pack that outlives the account;
    // their avatar is not held by anything that survives.
    let art = h.upload_and_process(&creator, AssetPurpose::ASSET_PURPOSE_STAMP, "art.png", "image/png", png_with_alpha(120, 120)).await;
    let avatar = h.upload_and_process(&creator, AssetPurpose::ASSET_PURPOSE_AVATAR, "me.png", "image/png", png(64, 64)).await;
    let pending = h.create_upload(&creator, AssetPurpose::ASSET_PURPOSE_AVATAR, "later.png", "image/png", &png(16, 16)).await;
    assets::attach(&core.db, asset_id(&art), "stamp:123").await.unwrap();

    let event = Event {
        id: uuid::Uuid::now_v7(),
        topic: "identity.user.deleted".into(),
        actor_id: Some(creator.id),
        occurred_at: chrono::Utc::now(),
        data: serde_json::json!({ "user_id": creator.id }),
    };
    h.media.on_user_deleted(event).await.unwrap();

    assert!(assets::find(core, asset_id(&avatar)).await.unwrap().is_none());
    assert!(h.media_object(&avatar.url).await.is_none());
    assert!(assets::find(core, pending.asset_id.parse().unwrap()).await.unwrap().is_none());
    let kept = assets::find(core, asset_id(&art)).await.unwrap().expect("held art survives its creator");
    assert!(kept.is_attached());
    assert!(h.media_object(&art.url).await.is_some());

    // When the pack finally lets go, the art is collected like any unused asset.
    assert_eq!(assets::detach_holder(&core.db, "stamp:123").await.unwrap(), 1);
    assert_eq!(h.media.sweep().await.unwrap().unattached_assets, 0, "not before the unattached lifetime has passed");
    sqlx::query("UPDATE media_assets SET detached_at = now() - interval '3 days', ready_at = now() - interval '9 days' WHERE id = $1").bind(asset_id(&art)).execute(&core.db).await.unwrap();
    assert_eq!(h.media.sweep().await.unwrap().unattached_assets, 1);
    assert!(h.media_object(&art.url).await.is_none());
}

#[tokio::test]
async fn open_uploads_are_capped_per_user() {
    let h = start().await;
    let alice = h.app.user("alice").await;
    let bytes = [0u8; 16];
    for _ in 0..100 {
        h.create_upload(&alice, AssetPurpose::ASSET_PURPOSE_CHAT_FILE, "f.bin", "application/octet-stream", &bytes).await;
    }
    let request = CreateUploadRequest {
        purpose: AssetPurpose::ASSET_PURPOSE_CHAT_FILE.into(),
        file_name: "f.bin".into(),
        content_type: "application/octet-stream".into(),
        size_bytes: 16,
        ..Default::default()
    };
    let error = h.rpc::<_, CreateUploadResponse>("CreateUpload", Some(&alice), &request).await.unwrap_err();
    assert_eq!((error.code.as_str(), error.reason.as_str()), ("resource_exhausted", "TOO_MANY_UPLOADS"));
}

#[tokio::test]
async fn a_completed_upload_cannot_be_replaced_through_its_upload_url() {
    let _sweeping = SWEEPING.lock().await;
    let h = start().await;
    let alice = h.app.user("alice").await;

    // A chat file is READY as soon as it completes, and its recipients
    // download whatever the original holds.
    let signed = b"%PDF-1.7 I owe you nothing. -- signed, alice".to_vec();
    let upload = h.create_upload(&alice, AssetPurpose::ASSET_PURPOSE_CHAT_FILE, "contract.pdf", "application/pdf", &signed).await;
    assert!(h.put(&upload, signed.clone()).await.status().is_success());
    let asset = h.complete(&alice, &upload.upload_id).await.unwrap();
    assert_eq!(asset.status, AssetStatus::ASSET_STATUS_READY);

    // The upload URL is still within its lifetime. A second PUT of the same
    // size (the signature only fixes the length) must not change the file.
    let forged: Vec<u8> = signed.iter().map(u8::to_ascii_uppercase).collect();
    let _ = h.put(&upload, forged.clone()).await;
    let url = h.get_asset(&alice, &asset.id).await.unwrap().url;
    assert_eq!(h.http.get(&url).send().await.unwrap().bytes().await.unwrap(), signed, "the file is what was completed");

    // Two completions at once each check a copy of their own; the winner's is kept.
    let note = b"see you at the station at 7".to_vec();
    let upload = h.create_upload(&alice, AssetPurpose::ASSET_PURPOSE_CHAT_FILE, "note.txt", "text/plain", &note).await;
    assert!(h.put(&upload, note.clone()).await.status().is_success());
    let (first, second) = tokio::join!(h.complete(&alice, &upload.upload_id), h.complete(&alice, &upload.upload_id));
    assert!(first.is_ok() && second.is_ok(), "{first:?} {second:?}");
    h.media.process_now(upload.asset_id.parse().unwrap()).await.unwrap();
    let url = h.get_asset(&alice, &upload.asset_id).await.unwrap().url;
    assert_eq!(h.http.get(&url).send().await.unwrap().bytes().await.unwrap(), note);

    // The same between completion and processing: the worker processes what
    // was checked, not what was put there afterwards.
    let picture = png(64, 64);
    let upload = h.create_upload(&alice, AssetPurpose::ASSET_PURPOSE_AVATAR, "me.png", "image/png", &picture).await;
    assert!(h.put(&upload, picture.clone()).await.status().is_success());
    let pending = h.complete(&alice, &upload.upload_id).await.unwrap();
    assert_eq!(pending.status, AssetStatus::ASSET_STATUS_PROCESSING);
    let _ = h.put(&upload, noise(picture.len())).await;
    h.media.process_now(asset_id(&pending)).await.unwrap();
    assert_eq!(h.get_asset(&alice, &pending.id).await.unwrap().status, AssetStatus::ASSET_STATUS_READY);

    // Whatever a late PUT leaves behind is removed once the URL has expired,
    // and the asset itself keeps its file.
    let id = asset_id(&asset);
    let late_key = format!("originals/{id}/source.pdf");
    let core = &h.app.core;
    h.media.sweep().await.unwrap();
    sqlx::query("UPDATE media_assets SET created_at = created_at - interval '2 hours', repurge_after = repurge_after - interval '2 hours' WHERE id = $1")
        .bind(id)
        .execute(&core.db)
        .await
        .unwrap();
    h.media.sweep().await.unwrap();
    assert!(core.storage.head(tokuly_core::storage::Bucket::Private, &late_key).await.unwrap().is_none(), "the late PUT is gone");
    assert!(h.original_exists(id).await, "the completed file stays");
    let url = h.get_asset(&alice, &asset.id).await.unwrap().url;
    assert_eq!(h.http.get(&url).send().await.unwrap().bytes().await.unwrap(), signed);
}

#[tokio::test]
async fn private_downloads_are_never_cached_by_shared_caches() {
    let h = start().await;
    let alice = h.app.user("alice").await;
    let bytes = b"bank statement".to_vec();
    let upload = h.create_upload(&alice, AssetPurpose::ASSET_PURPOSE_CHAT_FILE, "statement.txt", "text/plain", &bytes).await;
    // The uploader may send headers the signature does not cover.
    let mut request = h.http.put(&upload.put_url).body(bytes.clone()).header("Cache-Control", "public, max-age=31536000");
    for (name, value) in &upload.put_headers {
        request = request.header(name, value);
    }
    assert!(request.send().await.unwrap().status().is_success());
    let asset = h.complete(&alice, &upload.upload_id).await.unwrap();

    let download = h.http.get(&asset.url).send().await.unwrap();
    assert_eq!(download.status(), 200);
    let cache_control = download.headers().get("cache-control").map(|v| v.to_str().unwrap().to_owned()).unwrap_or_default();
    assert!(cache_control.contains("private") && cache_control.contains("no-store") && !cache_control.contains("public"), "{cache_control:?}");
}
