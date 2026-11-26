//! The pictures and clips the bots post, and how they are uploaded.
//!
//! Nothing here shows a person or a place: avatars and story pictures are
//! painted from a number (soft blobs of colour), and the shorts are three
//! abstract clips made with ffmpeg's test sources (`assets/README.md`).
use std::time::Duration;

use tokuly_proto::media::{
    AssetPurpose, AssetStatus, CompleteUploadRequest, CompleteUploadResponse, CreateUploadRequest,
    CreateUploadResponse, GetAssetRequest, GetAssetResponse,
};

use crate::{
    api::{Api, ApiError, ApiResult},
    rng::Rng,
};

/// The clips shorts are made of.
pub const CLIPS: [&[u8]; crate::posts::CLIP_COUNT] = [
    include_bytes!("../assets/clip-1.mp4"),
    include_bytes!("../assets/clip-2.mp4"),
    include_bytes!("../assets/clip-3.mp4"),
];

/// A PNG of `width` × `height` painted from `colours`: a gradient with a
/// few soft blobs.
pub fn paint(colours: u64, width: u32, height: u32) -> Vec<u8> {
    let mut rng = Rng::new(colours);
    let hue = rng.between(0.0, 360.0);
    let (top, bottom) = (hsl(hue, 0.75, 0.72), hsl(hue + rng.between(40.0, 110.0), 0.7, 0.55));
    let blobs: Vec<(f64, f64, f64, [f64; 3])> = (0..5)
        .map(|_| (rng.unit(), rng.unit(), rng.between(0.12, 0.35), hsl(hue + rng.between(-60.0, 160.0), 0.8, 0.8)))
        .collect();
    let mut pixels = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        let down = f64::from(y) / f64::from(height.max(2) - 1);
        for x in 0..width {
            let across = f64::from(x) / f64::from(width.max(2) - 1);
            let mut colour = [0, 1, 2].map(|channel| top[channel] + (bottom[channel] - top[channel]) * down);
            for (bx, by, radius, tint) in &blobs {
                // Distances in units of the width, so blobs are round.
                let distance = (across - bx).hypot((down - by) * f64::from(height) / f64::from(width));
                let weight = (1.0 - distance / radius).clamp(0.0, 1.0).powi(2) * 0.6;
                colour = [0, 1, 2].map(|channel| colour[channel] + (tint[channel] - colour[channel]) * weight);
            }
            pixels.extend(colour.map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8));
        }
    }
    png(width, height, &pixels)
}

/// Hue (degrees), saturation and lightness as red, green and blue in `0..=1`.
fn hsl(hue: f64, saturation: f64, lightness: f64) -> [f64; 3] {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = hue.rem_euclid(360.0) / 60.0;
    let second = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
    let (r, g, b) = match sector as u32 {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };
    let lift = lightness - chroma / 2.0;
    [r + lift, g + lift, b + lift]
}

/// Encodes 8-bit RGB pixels as a PNG. The image data is stored without
/// compression: the pictures are small and the server makes WebP of them.
fn png(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let row = width as usize * 3;
    let mut raw = Vec::with_capacity((row + 1) * height as usize);
    for line in rgb.chunks(row) {
        raw.push(0); // filter: none
        raw.extend_from_slice(line);
    }
    // A zlib stream of stored blocks (at most 65535 bytes each).
    let mut data = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65_535).peekable();
    while let Some(block) = blocks.next() {
        data.push(u8::from(blocks.peek().is_none()));
        data.extend((block.len() as u16).to_le_bytes());
        data.extend((!(block.len() as u16)).to_le_bytes());
        data.extend_from_slice(block);
    }
    let (a, b) = raw
        .iter()
        .fold((1u32, 0u32), |(a, b), byte| ((a + u32::from(*byte)) % 65_521, (a + u32::from(*byte) + b) % 65_521));
    data.extend((b << 16 | a).to_be_bytes());

    let mut header = Vec::new();
    header.extend(width.to_be_bytes());
    header.extend(height.to_be_bytes());
    header.extend([8, 2, 0, 0, 0]); // 8 bits per channel, RGB
    let mut file = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, body) in [(b"IHDR", header.as_slice()), (b"IDAT", data.as_slice()), (b"IEND", &[][..])] {
        file.extend((body.len() as u32).to_be_bytes());
        file.extend(kind);
        file.extend(body);
        file.extend(crc32(kind, body).to_be_bytes());
    }
    file
}

fn crc32(kind: &[u8], body: &[u8]) -> u32 {
    !kind.iter().chain(body).fold(!0u32, |crc, byte| {
        (0..8).fold(crc ^ u32::from(*byte), |crc, _| if crc & 1 == 1 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 })
    })
}

/// A file to upload.
pub struct Upload<'a> {
    pub purpose: AssetPurpose,
    pub file_name: &'a str,
    pub content_type: &'a str,
    pub bytes: Vec<u8>,
}

/// Uploads a file as the bot with `token` and waits until the server has
/// processed it. Returns the asset ID.
pub async fn upload(api: &Api, token: &str, file: Upload<'_>) -> ApiResult<String> {
    let token = Some(token);
    let request = CreateUploadRequest {
        purpose: file.purpose.into(),
        file_name: file.file_name.to_owned(),
        content_type: file.content_type.to_owned(),
        size_bytes: file.bytes.len() as i64,
        ..Default::default()
    };
    let created: CreateUploadResponse = api.call(token, "tokuly.media.v1.MediaService/CreateUpload", &request).await?;
    api.put(&created.put_url, &created.put_headers, file.bytes).await?;
    let complete = CompleteUploadRequest { upload_id: created.upload_id.clone(), ..Default::default() };
    let completed: CompleteUploadResponse =
        api.call(token, "tokuly.media.v1.MediaService/CompleteUpload", &complete).await?;
    let mut status = completed.asset.as_option().map(|asset| asset.status);
    // A picture takes a moment, a clip up to a minute or two.
    for _ in 0..90 {
        match status.as_ref().and_then(|status| status.as_known()) {
            Some(AssetStatus::ASSET_STATUS_READY) => return Ok(created.asset_id),
            Some(AssetStatus::ASSET_STATUS_FAILED) => break,
            _ => tokio::time::sleep(Duration::from_secs(2)).await,
        }
        let request = GetAssetRequest { asset_id: created.asset_id.clone(), ..Default::default() };
        let asset: GetAssetResponse = api.call(token, "tokuly.media.v1.MediaService/GetAsset", &request).await?;
        status = asset.asset.as_option().map(|asset| asset.status);
    }
    Err(ApiError {
        code: "aborted".into(),
        reason: "ASSET_NOT_READY".into(),
        message: format!("{} was not processed", file.file_name),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_a_png_of_the_asked_size_and_the_same_for_the_same_number() {
        let picture = paint(7, 90, 160);
        assert_eq!(&picture[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!((&picture[16..20], &picture[20..24]), (&90u32.to_be_bytes()[..], &160u32.to_be_bytes()[..]));
        assert_eq!(picture, paint(7, 90, 160));
        assert_ne!(picture, paint(8, 90, 160));
        // Header, one data chunk, end: stored rows plus the wrapping.
        assert!(picture.len() > 90 * 160 * 3 && picture.len() < 90 * 160 * 3 + 400);
    }

    #[test]
    fn checksums_match_the_png_specification() {
        // The CRC of an empty IEND chunk is a constant of the format.
        assert_eq!(crc32(b"IEND", &[]), 0xae42_6082);
    }

    #[test]
    fn colours_convert() {
        assert_eq!(hsl(0.0, 1.0, 0.5), [1.0, 0.0, 0.0]);
        assert_eq!(hsl(120.0, 1.0, 0.5), [0.0, 1.0, 0.0]);
        assert_eq!(hsl(240.0, 1.0, 0.5), [0.0, 0.0, 1.0]);
        assert_eq!(hsl(480.0, 1.0, 0.5), hsl(120.0, 1.0, 0.5));
    }

    #[test]
    fn the_clips_are_mp4_files() {
        for clip in CLIPS {
            assert_eq!(&clip[4..8], b"ftyp");
            assert!(clip.len() < 400_000, "a clip of {} bytes is too heavy for the repository", clip.len());
        }
    }
}
