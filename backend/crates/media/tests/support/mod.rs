//! Shared helpers for the media integration tests: an app with the media
//! domain registered, uploads through presigned URLs the way a client does
//! them, and fixtures generated on the fly.
#![allow(dead_code)]

use std::{io::Cursor, path::{Path, PathBuf}};

use image::{ImageEncoder as _, Rgb, RgbImage, Rgba, RgbaImage};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use tokuly_core::storage::Bucket;
use tokuly_media::{Media, MediaConfig};
use tokuly_proto::media::*;
use tokuly_testkit::{RpcError, TestApp, TestUser};
use uuid::Uuid;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0400"];

pub struct Harness {
    pub app: TestApp,
    pub media: Media,
    pub http: reqwest::Client,
}

pub async fn start() -> Harness {
    start_with(MediaConfig::default()).await
}

pub async fn start_with(cfg: MediaConfig) -> Harness {
    // Show the cause of internal errors (logged, never sent to clients) in
    // the output of a failing test.
    let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::WARN).with_test_writer().try_init();
    let mut created = None;
    let app = TestApp::start(MIGRATIONS, |core, _identity, router| {
        let media = Media::new(core.clone(), cfg).expect("media domain");
        let router = media.register(router);
        created = Some(media);
        router
    })
    .await;
    app.core.storage.ensure_buckets().await.expect("object store buckets");
    Harness { app, media: created.expect("register ran"), http: reqwest::Client::new() }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn asset_id(asset: &Asset) -> Uuid {
    asset.id.parse().expect("asset id")
}

impl Harness {
    pub async fn rpc<Req: Serialize, Resp: DeserializeOwned>(&self, method: &str, user: Option<&TestUser>, request: &Req) -> Result<Resp, RpcError> {
        self.app.call(&format!("tokuly.media.v1.MediaService/{method}"), user, request).await
    }

    pub async fn create_upload(&self, user: &TestUser, purpose: AssetPurpose, file_name: &str, content_type: &str, bytes: &[u8]) -> CreateUploadResponse {
        let request = CreateUploadRequest {
            purpose: purpose.into(),
            file_name: file_name.to_owned(),
            content_type: content_type.to_owned(),
            size_bytes: bytes.len() as i64,
            ..Default::default()
        };
        self.rpc("CreateUpload", Some(user), &request).await.expect("CreateUpload")
    }

    /// PUTs the bytes to a single-request upload URL, with the headers the
    /// server asked for, exactly like a browser would.
    pub async fn put(&self, upload: &CreateUploadResponse, bytes: Vec<u8>) -> reqwest::Response {
        assert!(!upload.put_url.is_empty(), "not a single-PUT upload");
        let mut request = self.http.put(&upload.put_url).body(bytes);
        for (name, value) in &upload.put_headers {
            request = request.header(name, value);
        }
        request.send().await.expect("PUT to object store")
    }

    pub async fn complete(&self, user: &TestUser, upload_id: &str) -> Result<Asset, RpcError> {
        let request = CompleteUploadRequest { upload_id: upload_id.to_owned(), ..Default::default() };
        let response: CompleteUploadResponse = self.rpc("CompleteUpload", Some(user), &request).await?;
        Ok(response.asset.into_option().expect("asset in response"))
    }

    /// CreateUpload + PUT (in one request or in parts, as the server
    /// decides) + CompleteUpload.
    pub async fn upload(&self, user: &TestUser, purpose: AssetPurpose, file_name: &str, content_type: &str, bytes: Vec<u8>) -> Asset {
        let upload = self.create_upload(user, purpose, file_name, content_type, &bytes).await;
        if upload.put_url.is_empty() {
            self.put_parts(user, &upload, &bytes).await;
        } else {
            let stored = self.put(&upload, bytes).await;
            assert!(stored.status().is_success(), "upload PUT failed: {}", stored.status());
        }
        self.complete(user, &upload.upload_id).await.expect("CompleteUpload")
    }

    /// Uploads every part of a multipart upload.
    pub async fn put_parts(&self, user: &TestUser, upload: &CreateUploadResponse, bytes: &[u8]) {
        let request = GetUploadPartsRequest { upload_id: upload.upload_id.clone(), part_numbers: (1..=upload.part_count).collect(), ..Default::default() };
        let signed: GetUploadPartsResponse = self.rpc("GetUploadParts", Some(user), &request).await.expect("GetUploadParts");
        let part_size = upload.part_size_bytes as usize;
        for part in &signed.parts {
            let start = (part.part_number as usize - 1) * part_size;
            let body = bytes[start..(start + part_size).min(bytes.len())].to_vec();
            let stored = self.http.put(&part.put_url).body(body).send().await.expect("PUT part");
            assert!(stored.status().is_success(), "part {} failed: {}", part.part_number, stored.status());
        }
    }

    /// Uploads and runs the processing job in place; returns the final asset.
    pub async fn upload_and_process(&self, user: &TestUser, purpose: AssetPurpose, file_name: &str, content_type: &str, bytes: Vec<u8>) -> Asset {
        let asset = self.upload(user, purpose, file_name, content_type, bytes).await;
        self.media.process_now(asset_id(&asset)).await.expect("processing");
        self.get_asset(user, &asset.id).await.expect("GetAsset")
    }

    pub async fn get_asset(&self, user: &TestUser, id: &str) -> Result<Asset, RpcError> {
        let response: GetAssetResponse = self.rpc("GetAsset", Some(user), &GetAssetRequest { asset_id: id.to_owned(), ..Default::default() }).await?;
        Ok(response.asset.into_option().expect("asset in response"))
    }

    /// Object key behind a public media URL.
    pub fn media_key(&self, url: &str) -> String {
        let base = self.app.core.storage.media_url("");
        url.strip_prefix(&base).unwrap_or_else(|| panic!("{url} is not a media URL")).to_owned()
    }

    /// The bytes behind a public media URL, or `None` when the object is gone.
    pub async fn media_object(&self, url: &str) -> Option<Vec<u8>> {
        let key = self.media_key(url);
        match self.app.core.storage.head(Bucket::Media, &key).await.expect("head") {
            Some(_) => Some(self.app.core.storage.get(Bucket::Media, &key).await.expect("get").to_vec()),
            None => None,
        }
    }

    /// Fetches a public media URL anonymously, straight from the object
    /// store (what the edge would proxy), to inspect response headers.
    pub async fn media_response(&self, url: &str) -> reqwest::Response {
        let cfg = &self.app.core.cfg;
        let direct = format!("{}/{}/{}", cfg.s3_endpoint.trim_end_matches('/'), cfg.s3_media_bucket, self.media_key(url));
        self.http.get(direct).send().await.expect("GET from object store")
    }

    pub async fn original_exists(&self, asset_id: Uuid) -> bool {
        let key: String = sqlx::query_scalar("SELECT original_key FROM media_assets WHERE id = $1").bind(asset_id).fetch_one(&self.app.core.db).await.expect("asset row");
        self.app.core.storage.head(Bucket::Private, &key).await.expect("head").is_some()
    }
}

/// The assets pushed as `MediaEvent.asset_updated`, in order.
pub fn pushed_assets(events: Vec<MediaEvent>) -> Vec<Asset> {
    events.into_iter().filter_map(|event| event.event.map(|media_event::Event::AssetUpdated(asset)| *asset)).collect()
}

// ── image fixtures ──

pub fn gradient(width: u32, height: u32) -> RgbImage {
    RgbImage::from_fn(width, height, |x, y| Rgb([(x * 255 / width.max(1)) as u8, (y * 255 / height.max(1)) as u8, 120]))
}

pub fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(gradient(width, height)).write_to(&mut out, image::ImageFormat::Png).expect("png");
    out.into_inner()
}

/// A PNG whose right half is fully transparent.
pub fn png_with_alpha(width: u32, height: u32) -> Vec<u8> {
    let image = RgbaImage::from_fn(width, height, |x, _| if x < width / 2 { Rgba([230, 60, 40, 255]) } else { Rgba([0, 0, 0, 0]) });
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image).write_to(&mut out, image::ImageFormat::Png).expect("png");
    out.into_inner()
}

/// The marker planted in fixtures' metadata; it must never reach a rendition.
pub const GPS_MARKER: &[u8] = b"TOKULY-SECRET-PLACE";

/// A minimal EXIF block (TIFF, little endian) with an orientation, a GPS
/// position in Tokyo and an image description carrying [`GPS_MARKER`].
pub fn exif(orientation: u16) -> Vec<u8> {
    fn entry(out: &mut Vec<u8>, tag: u16, kind: u16, count: u32, value: [u8; 4]) {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&value);
    }
    let rationals = |values: [(u32, u32); 3]| values.iter().flat_map(|(n, d)| [n.to_le_bytes(), d.to_le_bytes()].concat()).collect::<Vec<u8>>();
    let description = [GPS_MARKER, b"\0"].concat();

    // Layout: header (8) | IFD0 (2 + 3*12 + 4 = 42) | GPS IFD (2 + 4*12 + 4 = 54) | description | latitude | longitude
    let gps_ifd = 8 + 42u32;
    let description_at = gps_ifd + 54;
    let latitude_at = description_at + description.len() as u32;
    let longitude_at = latitude_at + 24;

    let mut out = Vec::new();
    out.extend_from_slice(b"II\x2a\x00");
    out.extend_from_slice(&8u32.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes());
    entry(&mut out, 0x010e, 2, description.len() as u32, description_at.to_le_bytes()); // ImageDescription
    entry(&mut out, 0x0112, 3, 1, [orientation as u8, (orientation >> 8) as u8, 0, 0]); // Orientation
    entry(&mut out, 0x8825, 4, 1, gps_ifd.to_le_bytes()); // GPS IFD pointer
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    entry(&mut out, 0x0001, 2, 2, *b"N\0\0\0"); // GPSLatitudeRef
    entry(&mut out, 0x0002, 5, 3, latitude_at.to_le_bytes()); // GPSLatitude
    entry(&mut out, 0x0003, 2, 2, *b"E\0\0\0"); // GPSLongitudeRef
    entry(&mut out, 0x0004, 5, 3, longitude_at.to_le_bytes()); // GPSLongitude
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&description);
    out.extend_from_slice(&rationals([(35, 1), (41, 1), (2221, 100)]));
    out.extend_from_slice(&rationals([(139, 1), (41, 1), (3012, 100)]));
    out
}

/// A JPEG as a phone camera writes it: EXIF with orientation and GPS position.
pub fn jpeg_with_exif(width: u32, height: u32, orientation: u16) -> Vec<u8> {
    let image = gradient(width, height);
    let mut out = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90);
    encoder.set_exif_metadata(exif(orientation)).expect("exif");
    encoder.write_image(image.as_raw(), width, height, image::ExtendedColorType::Rgb8).expect("jpeg");
    out
}

/// An animated GIF: a red square moving over a blue background.
pub fn gif(width: u32, height: u32, frames: u32) -> Vec<u8> {
    use image::{Delay, Frame, codecs::gif::{GifEncoder, Repeat}};
    let mut out = Vec::new();
    {
        let mut encoder = GifEncoder::new_with_speed(&mut out, 30);
        encoder.set_repeat(Repeat::Infinite).expect("repeat");
        for index in 0..frames {
            let buffer = RgbaImage::from_fn(width, height, |x, y| {
                let left = index * 6 % width;
                if (left..left + 12).contains(&x) && y < 12 { Rgba([250, 40, 60, 255]) } else { Rgba([20, 90, 200, 255]) }
            });
            encoder.encode_frame(Frame::from_parts(buffer, 0, 0, Delay::from_numer_denom_ms(100, 1))).expect("frame");
        }
    }
    out
}

/// Whether `needle` occurs anywhere in `haystack`.
pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

// ── audio / video fixtures (generated with ffmpeg's synthetic sources) ──

pub fn fixture_dir() -> tempfile::TempDir {
    tempfile::Builder::new().prefix("tokuly-media-test-").tempdir().expect("temp dir")
}

/// Runs the `ffmpeg` CLI to build a fixture.
pub async fn ffmpeg(args: &[&str]) {
    let output = tokio::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .await
        .expect("ffmpeg must be on PATH to run the media tests");
    assert!(output.status.success(), "fixture ffmpeg failed: {}", String::from_utf8_lossy(&output.stderr));
}

pub struct VideoSpec<'a> {
    pub size: &'a str,
    pub seconds: u32,
    pub rate: u32,
    pub audio: bool,
}

impl Default for VideoSpec<'_> {
    fn default() -> Self {
        Self { size: "640x360", seconds: 3, rate: 30, audio: true }
    }
}

/// An H.264 (+AAC) MP4 of a test pattern, tagged with a title and a GPS
/// position that must not survive transcoding.
pub async fn video(dir: &Path, name: &str, spec: VideoSpec<'_>) -> PathBuf {
    let path = dir.join(name);
    let source = format!("testsrc2=size={}:rate={}:duration={}", spec.size, spec.rate, spec.seconds);
    let tone = format!("sine=frequency=440:duration={}", spec.seconds);
    let marker = String::from_utf8_lossy(GPS_MARKER).into_owned();
    let mut args = vec!["-f", "lavfi", "-i", &source];
    if spec.audio {
        args.extend(["-f", "lavfi", "-i", &tone, "-c:a", "aac", "-shortest"]);
    }
    let title = format!("title={marker}");
    let location = format!("location={marker}+35.6895+139.6917/");
    args.extend(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-metadata", &title, "-metadata", &location]);
    let out = path.to_string_lossy().into_owned();
    args.push(&out);
    ffmpeg(&args).await;
    path
}

/// Copies a video, flagging it as rotated by `degrees` (how phones store
/// portrait recordings: landscape pixels plus a rotation).
pub async fn rotated(source: &Path, degrees: i32) -> PathBuf {
    let path = source.with_file_name(format!("rotated-{degrees}.mp4"));
    let rotation = degrees.to_string();
    ffmpeg(&["-display_rotation", &rotation, "-i", &source.to_string_lossy(), "-c", "copy", &path.to_string_lossy()]).await;
    path
}

/// A two-second tone, encoded the way a browser records a voice message
/// (Opus in WebM) unless the file name says otherwise.
pub async fn tone(dir: &Path, name: &str, seconds: u32) -> PathBuf {
    let path = dir.join(name);
    let source = format!("sine=frequency=440:duration={seconds}");
    ffmpeg(&["-f", "lavfi", "-i", &source, &path.to_string_lossy()]).await;
    path
}

pub async fn read(path: &Path) -> Vec<u8> {
    tokio::fs::read(path).await.expect("read fixture")
}

/// Rewrites the durations an MP4 declares (`mvhd`, `tkhd`, `mdhd`, `elst`)
/// to `seconds`, leaving the samples alone: a file whose header says it is
/// short while it plays for as long as it did.
pub fn forge_mp4_duration(mut bytes: Vec<u8>, seconds: u32) -> Vec<u8> {
    fn read(bytes: &[u8], at: usize, wide: bool) -> u64 {
        if wide { u64::from_be_bytes(bytes[at..at + 8].try_into().unwrap()) } else { u64::from(u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap())) }
    }
    fn write(bytes: &mut [u8], at: usize, wide: bool, value: u64) {
        if wide { bytes[at..at + 8].copy_from_slice(&value.to_be_bytes()) } else { bytes[at..at + 4].copy_from_slice(&(value as u32).to_be_bytes()) }
    }
    let find = |bytes: &[u8], tag: &[u8]| -> Vec<usize> { bytes.windows(4).enumerate().filter(|(_, w)| *w == tag).map(|(i, _)| i).collect() };
    // Movie timescale first: tkhd and elst count in it.
    let mvhd = find(&bytes, b"mvhd")[0];
    let wide = bytes[mvhd + 4] == 1;
    let (scale_at, duration_at) = if wide { (mvhd + 24, mvhd + 28) } else { (mvhd + 16, mvhd + 20) };
    let movie_scale = read(&bytes, scale_at, false);
    write(&mut bytes, duration_at, wide, movie_scale * u64::from(seconds));
    for tkhd in find(&bytes, b"tkhd") {
        let wide = bytes[tkhd + 4] == 1;
        write(&mut bytes, if wide { tkhd + 32 } else { tkhd + 24 }, wide, movie_scale * u64::from(seconds));
    }
    for mdhd in find(&bytes, b"mdhd") {
        let wide = bytes[mdhd + 4] == 1;
        let (scale_at, duration_at) = if wide { (mdhd + 24, mdhd + 28) } else { (mdhd + 16, mdhd + 20) };
        let scale = read(&bytes, scale_at, false);
        write(&mut bytes, duration_at, wide, scale * u64::from(seconds));
    }
    for elst in find(&bytes, b"elst") {
        let wide = bytes[elst + 4] == 1;
        write(&mut bytes, elst + 12, wide, movie_scale * u64::from(seconds));
    }
    bytes
}

/// What ffprobe says a file lasts (the container's claim).
pub async fn probed_seconds(path: &Path) -> f64 {
    let output = tokio::process::Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
        .arg(path)
        .output()
        .await
        .expect("ffprobe must be on PATH to run the media tests");
    String::from_utf8_lossy(&output.stdout).trim().parse().unwrap_or(0.0)
}

/// Gives the first track of an MP4 (whose `moov` comes last, as ffmpeg
/// writes it) an edit list of `entries` copies of its whole span, leaving
/// every header duration alone: a 2-second file that plays for
/// `entries` × 2 seconds.
pub fn repeat_edit_list(bytes: &[u8], entries: u32) -> Vec<u8> {
    fn boxes(buf: &[u8], start: usize, end: usize) -> Vec<(usize, usize, [u8; 4])> {
        let mut out = Vec::new();
        let mut at = start;
        while at + 8 <= end {
            let size = u32::from_be_bytes(buf[at..at + 4].try_into().unwrap()) as usize;
            out.push((at, size, buf[at + 4..at + 8].try_into().unwrap()));
            at += size;
        }
        out
    }
    let top = boxes(bytes, 0, bytes.len());
    let &(moov_at, moov_size, _) = top.iter().find(|b| &b.2 == b"moov").expect("a moov box");
    assert_eq!(moov_at + moov_size, bytes.len(), "moov must come last");
    let moov = &bytes[moov_at..];
    let children = boxes(moov, 8, moov_size);
    let &(mvhd, ..) = children.iter().find(|b| &b.2 == b"mvhd").unwrap();
    // Version 0 mvhd: the duration is in movie timescale units.
    let movie_duration = u32::from_be_bytes(moov[mvhd + 24..mvhd + 28].try_into().unwrap());
    let mut new_moov = moov[..8].to_vec();
    let mut edited = false;
    for &(at, size, kind) in &children {
        if &kind != b"trak" || edited {
            new_moov.extend_from_slice(&moov[at..at + size]);
            continue;
        }
        edited = true;
        let trak = &moov[at..at + size];
        let mut new_trak = trak[..8].to_vec();
        for &(child, child_size, child_kind) in &boxes(trak, 8, size) {
            if &child_kind == b"edts" {
                continue;
            }
            new_trak.extend_from_slice(&trak[child..child + child_size]);
            if &child_kind == b"tkhd" {
                let mut elst = Vec::new();
                elst.extend_from_slice(&(16 + 12 * entries).to_be_bytes());
                elst.extend_from_slice(b"elst\0\0\0\0");
                elst.extend_from_slice(&entries.to_be_bytes());
                for _ in 0..entries {
                    elst.extend_from_slice(&movie_duration.to_be_bytes());
                    elst.extend_from_slice(&0i32.to_be_bytes());
                    elst.extend_from_slice(&0x0001_0000u32.to_be_bytes());
                }
                new_trak.extend_from_slice(&((8 + elst.len()) as u32).to_be_bytes());
                new_trak.extend_from_slice(b"edts");
                new_trak.extend_from_slice(&elst);
            }
        }
        let trak_size = new_trak.len() as u32;
        new_trak[..4].copy_from_slice(&trak_size.to_be_bytes());
        new_moov.extend_from_slice(&new_trak);
    }
    let moov_size = new_moov.len() as u32;
    new_moov[..4].copy_from_slice(&moov_size.to_be_bytes());
    [&bytes[..moov_at], &new_moov[..]].concat()
}
