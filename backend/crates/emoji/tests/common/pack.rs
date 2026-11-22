//! Emoji packs for the tests: built by hand (a ZIP writer and JSON as a
//! Misskey server writes it) with real images inside, and the calls that
//! start an import and read it back.
use std::io::{Cursor, Write as _};

use image::{Rgba, RgbaImage};
use tokuly_core::storage::Bucket;
use tokuly_media::testing::ready_asset;
use tokuly_proto::{
    emoji::*,
    media::{AssetKind, AssetPurpose},
};
use tokuly_testkit::TestUser;
use uuid::Uuid;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use super::{Ctx, page};

/// A PNG with transparency, like an emoji.
pub fn png(size: u32) -> Vec<u8> {
    let image =
        RgbaImage::from_fn(
            size,
            size,
            |x, y| if (x + y) % 2 == 0 { Rgba([230, 60, 40, 255]) } else { Rgba([0, 0, 0, 0]) },
        );
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image).write_to(&mut out, image::ImageFormat::Png).expect("png");
    out.into_inner()
}

/// An animated GIF of two frames.
pub fn gif(size: u32) -> Vec<u8> {
    use image::{
        Delay, Frame,
        codecs::gif::{GifEncoder, Repeat},
    };
    let mut out = Vec::new();
    {
        let mut encoder = GifEncoder::new(&mut out);
        encoder.set_repeat(Repeat::Infinite).expect("repeat");
        for color in [[255, 0, 0, 255], [0, 0, 255, 255]] {
            let frame = Frame::from_parts(
                RgbaImage::from_pixel(size, size, Rgba(color)),
                0,
                0,
                Delay::from_numer_denom_ms(100, 1),
            );
            encoder.encode_frame(frame).expect("frame");
        }
    }
    out
}

/// A JPEG as a phone writes it: with an EXIF block that says where the
/// photo was taken (35°41'22"N).
pub fn photo_with_gps() -> Vec<u8> {
    let mut jpeg = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(48, 48, image::Rgb([200, 80, 40])))
        .write_to(&mut jpeg, image::ImageFormat::Jpeg)
        .expect("jpeg");
    let jpeg = jpeg.into_inner();
    // A little-endian TIFF: IFD0 points to a GPS IFD with a latitude.
    let mut tiff: Vec<u8> = vec![b'I', b'I', 0x2A, 0, 8, 0, 0, 0];
    tiff.extend([1, 0]);
    tiff.extend([0x25, 0x88, 4, 0, 1, 0, 0, 0, 26, 0, 0, 0]);
    tiff.extend([0, 0, 0, 0]);
    tiff.extend([2, 0]);
    tiff.extend([1, 0, 2, 0, 2, 0, 0, 0, b'N', 0, 0, 0]);
    tiff.extend([2, 0, 5, 0, 3, 0, 0, 0, 56, 0, 0, 0]);
    tiff.extend([0, 0, 0, 0]);
    for (numerator, denominator) in [(35u32, 1u32), (41, 1), (2222, 100)] {
        tiff.extend(numerator.to_le_bytes());
        tiff.extend(denominator.to_le_bytes());
    }
    let mut exif = b"Exif\0\0".to_vec();
    exif.extend(tiff);
    // The APP1 segment goes right after the start-of-image marker.
    let mut out = jpeg[..2].to_vec();
    out.extend([0xFF, 0xE1]);
    out.extend(((exif.len() + 2) as u16).to_be_bytes());
    out.extend(exif);
    out.extend(&jpeg[2..]);
    out
}

/// Uploads `bytes` as an emoji picture of `owner` and lets the media domain
/// process it for real. Returns the upload's ID.
pub async fn processed_picture(ctx: &Ctx, owner: &TestUser, bytes: &[u8], file_name: &str) -> String {
    let dir = tempfile::tempdir().expect("scratch directory");
    let path = dir.path().join(file_name);
    std::fs::write(&path, bytes).expect("write picture");
    let upload = tokuly_media::create_asset_from_file(
        &ctx.app.core,
        owner.id,
        AssetPurpose::ASSET_PURPOSE_STAMP,
        &path,
        file_name,
    )
    .await
    .expect("store picture");
    ctx.media.process_now(upload.id).await.expect("media processing");
    upload.id.to_string()
}

/// A ZIP archive of the given files.
pub fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, bytes) in files {
        zip.start_file(*name, options).expect("zip entry");
        zip.write_all(bytes).expect("zip write");
    }
    zip.finish().expect("zip finish").into_inner()
}

/// One record of a `meta.json`, as Misskey writes it.
pub fn record(name: &str, file_name: &str, emoji: serde_json::Value) -> serde_json::Value {
    let mut full = serde_json::json!({
        "id": "9abcdefghi", "updatedAt": "2026-01-01T00:00:00.000Z", "name": name, "host": null, "category": null,
        "originalUrl": format!("https://misskey.example/files/{file_name}"),
        "publicUrl": format!("https://misskey.example/files/{file_name}"),
        "uri": null, "type": "image/png", "aliases": [], "license": null, "localOnly": false, "isSensitive": false,
        "roleIdsThatCanBeUsedThisEmojiAsReaction": [],
    });
    for (key, value) in emoji.as_object().expect("emoji fields").clone() {
        full[key] = value;
    }
    serde_json::json!({ "fileName": file_name, "downloaded": true, "emoji": full })
}

pub fn meta(records: Vec<serde_json::Value>) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "metaVersion": 2,
        "host": "misskey.example",
        "exportedAt": "Sat Nov 21 2026 10:00:00 GMT+0900 (Japan Standard Time)",
        "emojis": records,
    }))
    .unwrap()
}

/// Stores `bytes` as a finished upload of `owner` (a chat file), the way a
/// moderator uploads a pack, and returns its ID.
pub async fn upload_archive(ctx: &Ctx, owner: &TestUser, bytes: Vec<u8>) -> String {
    let core = &ctx.app.core;
    let upload =
        ready_asset(core, owner.id, AssetPurpose::ASSET_PURPOSE_CHAT_FILE, AssetKind::ASSET_KIND_FILE).await.unwrap();
    core.storage.put(Bucket::Private, &upload.original_key, bytes, "application/zip").await.unwrap();
    upload.id.to_string()
}

pub async fn start_import(
    ctx: &Ctx,
    moderator: &TestUser,
    archive: &str,
    policy: ImportConflictPolicy,
    category: &str,
) -> Import {
    let request = StartImportRequest {
        asset_id: archive.to_owned(),
        on_conflict: policy.into(),
        category: category.to_owned(),
        ..Default::default()
    };
    let response: StartImportResponse = ctx.call("Admin", "StartImport", Some(moderator), &request).await.unwrap();
    response.import.into_option().unwrap()
}

pub async fn get_import(ctx: &Ctx, moderator: &TestUser, import_id: &str) -> GetImportResponse {
    let request = GetImportRequest { import_id: import_id.to_owned(), page: page(500, ""), ..Default::default() };
    ctx.call("Admin", "GetImport", Some(moderator), &request).await.unwrap()
}

pub async fn run_import(ctx: &Ctx, import_id: &str) {
    ctx.emoji.run_import(import_id.parse().unwrap()).await.expect("the import job");
}

/// The uploads made from an import's pictures so far.
pub async fn picture_uploads(ctx: &Ctx, import_id: &str) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT asset_id FROM emoji_import_entries WHERE import_id = $1::uuid AND asset_id IS NOT NULL ORDER BY position")
        .bind(import_id)
        .fetch_all(&ctx.app.core.db)
        .await
        .unwrap()
}

/// What the media worker does with an import's pictures: processes each.
pub async fn process_pictures(ctx: &Ctx, import_id: &str) -> Vec<Uuid> {
    let uploads = picture_uploads(ctx, import_id).await;
    for upload in &uploads {
        ctx.media.process_now(*upload).await.expect("media processing");
    }
    uploads
}

/// Delivers the events the media domain emitted for these uploads.
pub async fn deliver_media_events(ctx: &Ctx, uploads: &[Uuid]) {
    for event in ctx.app.events("media.asset.").await {
        let about = event.data["asset_id"].as_str().and_then(|id| id.parse::<Uuid>().ok());
        if about.is_some_and(|id| uploads.contains(&id)) {
            ctx.emoji.handle_event(event).await.expect("media event");
        }
    }
}

pub fn entry<'a>(import: &'a GetImportResponse, source_name: &str) -> &'a ImportEntry {
    import
        .entries
        .iter()
        .find(|entry| entry.source_name == source_name)
        .unwrap_or_else(|| panic!("no entry {source_name}"))
}
