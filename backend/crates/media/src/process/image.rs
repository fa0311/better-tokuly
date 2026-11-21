//! Still images: decode, orient, strip, resize, encode.
//!
//! An upload is decoded to pixels and re-encoded as WebP, so nothing of the
//! original file survives besides the picture itself: EXIF (including GPS
//! position), XMP, IPTC and embedded thumbnails are all gone. Two pieces of
//! metadata change how the picture looks, so they are applied to the pixels
//! before they go: the EXIF orientation, and the ICC colour profile (phones
//! shoot Display P3; without conversion to sRGB the colours would be dull).
//!
//! Everything here is CPU-bound and synchronous; callers run it on the
//! blocking pool.
use std::io::Cursor;

use base64::Engine as _;
use image::{DynamicImage, ImageDecoder as _, ImageError, ImageReader, Limits, RgbaImage, imageops::FilterType};

use super::{ProcessError, ProcessResult};

/// Longest edge of the variants produced, largest first. An image smaller
/// than a size simply does not get that variant: nothing is ever upscaled.
pub const VARIANT_SIZES: [u32; 5] = [2048, 1024, 512, 256, 96];

/// Decoding limits: protection against decompression bombs.
pub const MAX_DIMENSION: u32 = 16_384;
pub const MAX_PIXELS: u64 = 64_000_000;
const MAX_DECODE_BYTES: u64 = 1024 * 1024 * 1024;

const WEBP_QUALITY: f32 = 82.0;
/// ThumbHash input must fit in 100×100.
const THUMB_EDGE: u32 = 100;

/// One encoded WebP rendition.
#[derive(Debug)]
pub struct Variant {
    /// Longest edge in pixels.
    pub size: u32,
    pub width: u32,
    pub height: u32,
    pub webp: Vec<u8>,
}

/// Everything derived from one still image.
#[derive(Debug)]
pub struct Rendered {
    /// Largest first; never empty.
    pub variants: Vec<Variant>,
    /// ThumbHash, base64.
    pub thumbhash: String,
    /// `#rrggbb`.
    pub dominant_color: String,
}

pub(crate) fn decode_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    limits
}

pub(crate) fn decode_error(error: ImageError) -> ProcessError {
    match error {
        ImageError::Limits(e) => ProcessError::rejected("IMAGE_TOO_LARGE", e.to_string()),
        ImageError::Unsupported(e) => ProcessError::rejected("UNSUPPORTED_CONTENT", e.to_string()),
        ImageError::IoError(e) if e.kind() != std::io::ErrorKind::UnexpectedEof => ProcessError::transient(e),
        other => ProcessError::rejected("CORRUPT_MEDIA", other.to_string()),
    }
}

pub(crate) fn check_pixel_count(width: u32, height: u32) -> ProcessResult<()> {
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(ProcessError::rejected("IMAGE_TOO_LARGE", format!("{width}x{height} exceeds {MAX_PIXELS} pixels")));
    }
    Ok(())
}

/// Decodes a JPEG, PNG, WebP or GIF (first frame) with its EXIF orientation applied.
pub fn decode(bytes: &[u8]) -> ProcessResult<DynamicImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(ProcessError::transient)?;
    reader.limits(decode_limits());
    let mut decoder = reader.into_decoder().map_err(decode_error)?;
    let (width, height) = decoder.dimensions();
    check_pixel_count(width, height)?;
    // Broken metadata is not a reason to refuse the picture.
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let profile = decoder.icc_profile().ok().flatten();
    let mut image = DynamicImage::from_decoder(decoder).map_err(decode_error)?;
    if let Some(profile) = profile {
        convert_to_srgb(&mut image, &profile);
    }
    image.apply_orientation(orientation);
    Ok(image)
}

/// Converts 8-bit RGB(A) pixels from the colour space described by an
/// embedded ICC profile to sRGB, in place. Anything unusual (a profile that
/// does not parse or is not RGB, deeper pixel formats) is left as it is:
/// slightly off colours are better than no picture.
fn convert_to_srgb(image: &mut DynamicImage, icc_profile: &[u8]) {
    use moxcms::{ColorProfile, DataColorSpace, Layout, TransformOptions};
    let Ok(source) = ColorProfile::new_from_slice(icc_profile) else { return };
    if source.color_space != DataColorSpace::Rgb {
        return;
    }
    let (layout, pixels): (Layout, &mut [u8]) = match image {
        DynamicImage::ImageRgb8(rgb) => (Layout::Rgb, &mut **rgb),
        DynamicImage::ImageRgba8(rgba) => (Layout::Rgba, &mut **rgba),
        _ => return,
    };
    let Ok(transform) = source.create_transform_8bit(layout, &ColorProfile::new_srgb(), layout, TransformOptions::default()) else { return };
    let mut converted = vec![0u8; pixels.len()];
    if transform.transform(pixels, &mut converted).is_ok() {
        pixels.copy_from_slice(&converted);
    }
}

/// Longest edges to render for an image whose longest edge is `longest`:
/// the image itself capped at the largest size, then every standard size
/// that is meaningfully (at least 10%) smaller.
pub fn variant_sizes(longest: u32) -> Vec<u32> {
    let top = longest.clamp(1, VARIANT_SIZES[0]);
    let mut sizes = vec![top];
    sizes.extend(VARIANT_SIZES.iter().copied().filter(|size| u64::from(*size) * 10 <= u64::from(top) * 9));
    sizes
}

/// Dimensions of `width`×`height` scaled so the longest edge is `longest`.
pub fn scaled_dimensions(width: u32, height: u32, longest: u32) -> (u32, u32) {
    let current = width.max(height).max(1);
    if longest >= current {
        return (width.max(1), height.max(1));
    }
    let scale = |edge: u32| ((u64::from(edge) * u64::from(longest) + u64::from(current) / 2) / u64::from(current)).max(1) as u32;
    (scale(width), scale(height))
}

/// Resizing straight alpha bleeds the colour of fully transparent pixels
/// into the edges of cut-outs (dark halos around stamps). Resampling
/// premultiplied colour avoids that.
fn premultiply(image: &mut RgbaImage) {
    for pixel in image.pixels_mut() {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel.0[..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
}

fn unpremultiply(image: &mut RgbaImage) {
    for pixel in image.pixels_mut() {
        let alpha = u16::from(pixel[3]);
        if alpha == 0 {
            pixel.0 = [0, 0, 0, 0];
            continue;
        }
        for channel in &mut pixel.0[..3] {
            *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
        }
    }
}

/// High-quality downscale to exactly `width`×`height`.
pub(crate) fn resize(image: &DynamicImage, width: u32, height: u32) -> DynamicImage {
    if (image.width(), image.height()) == (width, height) {
        return image.clone();
    }
    match image {
        DynamicImage::ImageRgba8(rgba) => {
            let mut source = rgba.clone();
            premultiply(&mut source);
            let mut resized = image::imageops::resize(&source, width, height, FilterType::Lanczos3);
            unpremultiply(&mut resized);
            DynamicImage::ImageRgba8(resized)
        }
        other => other.resize_exact(width, height, FilterType::Lanczos3),
    }
}

/// Lossy WebP of an RGB8 or RGBA8 image.
pub(crate) fn encode_webp(image: &DynamicImage, quality: f32) -> ProcessResult<Vec<u8>> {
    let encoder = match image {
        DynamicImage::ImageRgb8(rgb) => webp::Encoder::from_rgb(rgb.as_raw(), rgb.width(), rgb.height()),
        DynamicImage::ImageRgba8(rgba) => webp::Encoder::from_rgba(rgba.as_raw(), rgba.width(), rgba.height()),
        _ => return Err(ProcessError::transient("webp encoder needs 8-bit RGB(A)")),
    };
    let encoded = encoder.encode_simple(false, quality).map_err(|e| ProcessError::transient(format!("webp encode: {e:?}")))?;
    Ok(encoded.to_vec())
}

/// Reduces any decoded image to the two layouts the encoder takes, dropping
/// an alpha channel that carries no transparency.
pub(crate) fn normalize(image: DynamicImage) -> DynamicImage {
    if !image.color().has_alpha() {
        return DynamicImage::ImageRgb8(image.into_rgb8());
    }
    let rgba = image.into_rgba8();
    if rgba.pixels().all(|pixel| pixel[3] == 255) {
        DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(rgba).into_rgb8())
    } else {
        DynamicImage::ImageRgba8(rgba)
    }
}

/// Renders the variants, ThumbHash and dominant colour of a decoded image.
pub fn render(image: DynamicImage) -> ProcessResult<Rendered> {
    let image = normalize(image);
    let (width, height) = (image.width(), image.height());
    let sizes = variant_sizes(width.max(height));

    // Resample the original once, to the largest variant; the smaller ones
    // are derived from that (much cheaper, visually equivalent).
    let (top_width, top_height) = scaled_dimensions(width, height, sizes[0]);
    let top = resize(&image, top_width, top_height);
    drop(image);

    let mut variants = Vec::with_capacity(sizes.len());
    for size in sizes {
        let (w, h) = scaled_dimensions(top_width, top_height, size);
        let scaled = resize(&top, w, h);
        variants.push(Variant { size: w.max(h), width: w, height: h, webp: encode_webp(&scaled, WEBP_QUALITY)? });
    }

    let thumb = top.thumbnail(THUMB_EDGE, THUMB_EDGE).into_rgba8();
    Ok(Rendered { variants, thumbhash: thumbhash_of(&thumb), dominant_color: dominant_color(&thumb) })
}

/// ThumbHash (base64) of an image no larger than 100×100.
pub fn thumbhash_of(thumb: &RgbaImage) -> String {
    let hash = thumbhash::rgba_to_thumb_hash(thumb.width() as usize, thumb.height() as usize, thumb.as_raw());
    base64::engine::general_purpose::STANDARD.encode(hash)
}

/// The colour that best represents the image, as `#rrggbb`.
///
/// Pixels are grouped into coarse colour buckets (3 bits per channel); the
/// heaviest bucket wins and its average colour is returned. Saturated pixels
/// weigh up to twice as much as grey ones, so a vivid subject can beat a
/// slightly larger dull background — the colour is used to tint cards.
pub fn dominant_color(thumb: &RgbaImage) -> String {
    const BUCKETS: usize = 8 * 8 * 8;
    let mut weight = vec![0f32; BUCKETS];
    let mut sums = vec![[0u32; 3]; BUCKETS];
    let mut counts = vec![0u32; BUCKETS];
    for pixel in thumb.pixels() {
        let [r, g, b, a] = pixel.0;
        if a < 128 {
            continue; // mostly transparent: not part of the picture
        }
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let saturation = if max == 0 { 0.0 } else { f32::from(max - min) / f32::from(max) };
        let bucket = (usize::from(r >> 5) << 6) | (usize::from(g >> 5) << 3) | usize::from(b >> 5);
        weight[bucket] += 1.0 + saturation;
        counts[bucket] += 1;
        for (sum, channel) in sums[bucket].iter_mut().zip([r, g, b]) {
            *sum += u32::from(channel);
        }
    }
    let Some(best) = (0..BUCKETS).filter(|b| counts[*b] > 0).max_by(|a, b| weight[*a].total_cmp(&weight[*b])) else {
        return "#000000".to_owned(); // fully transparent image
    };
    let [r, g, b] = sums[best].map(|sum| sum / counts[best]);
    format!("#{r:02x}{g:02x}{b:02x}")
}

#[cfg(test)]
mod tests {
    use image::{Rgb, RgbImage, Rgba};

    use super::*;

    fn encode_png(image: &DynamicImage) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        image.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn variants_never_upscale() {
        assert_eq!(variant_sizes(4000), vec![2048, 1024, 512, 256, 96]);
        assert_eq!(variant_sizes(2048), vec![2048, 1024, 512, 256, 96]);
        assert_eq!(variant_sizes(800), vec![800, 512, 256, 96]);
        assert_eq!(variant_sizes(530), vec![530, 256, 96], "512 is too close to 530 to be worth a variant");
        assert_eq!(variant_sizes(96), vec![96]);
        assert_eq!(variant_sizes(40), vec![40]);
    }

    #[test]
    fn scaling_keeps_the_aspect_ratio() {
        assert_eq!(scaled_dimensions(4000, 3000, 2048), (2048, 1536));
        assert_eq!(scaled_dimensions(3000, 4000, 1024), (768, 1024));
        assert_eq!(scaled_dimensions(800, 600, 2048), (800, 600), "never upscaled");
        assert_eq!(scaled_dimensions(5000, 10, 96), (96, 1), "an edge never collapses to zero");
    }

    #[test]
    fn render_produces_decodable_webp_variants() {
        let source = DynamicImage::ImageRgb8(RgbImage::from_fn(1200, 800, |x, y| Rgb([(x % 256) as u8, (y % 256) as u8, 90])));
        let rendered = render(decode(&encode_png(&source)).unwrap()).unwrap();
        let sizes: Vec<(u32, u32, u32)> = rendered.variants.iter().map(|v| (v.size, v.width, v.height)).collect();
        assert_eq!(sizes, vec![(1200, 1200, 800), (1024, 1024, 683), (512, 512, 341), (256, 256, 171), (96, 96, 64)]);
        for variant in &rendered.variants {
            let decoded = image::load_from_memory_with_format(&variant.webp, image::ImageFormat::WebP).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (variant.width, variant.height));
            assert!(!decoded.color().has_alpha(), "opaque sources stay opaque");
        }
        assert!(!rendered.thumbhash.is_empty());
        assert!(base64::engine::general_purpose::STANDARD.decode(&rendered.thumbhash).is_ok());
    }

    #[test]
    fn transparency_survives_without_dark_halos() {
        // A white disc on a fully transparent *black* background: naive
        // resizing would darken the disc's edge.
        let source = RgbaImage::from_fn(400, 400, |x, y| {
            let (dx, dy) = (x as i32 - 200, y as i32 - 200);
            if dx * dx + dy * dy < 150 * 150 { Rgba([255, 255, 255, 255]) } else { Rgba([0, 0, 0, 0]) }
        });
        let resized = resize(&DynamicImage::ImageRgba8(source), 100, 100).into_rgba8();
        assert_eq!(resized.get_pixel(0, 0)[3], 0);
        for pixel in resized.pixels().filter(|p| p[3] > 32) {
            assert!(pixel[0] > 235, "edge pixel darkened to {:?}", pixel);
        }

        let source = RgbaImage::from_fn(300, 300, |x, _| if x < 150 { Rgba([200, 30, 30, 255]) } else { Rgba([0, 0, 0, 0]) });
        let rendered = render(DynamicImage::ImageRgba8(source)).unwrap();
        let decoded = image::load_from_memory_with_format(&rendered.variants[0].webp, image::ImageFormat::WebP).unwrap().into_rgba8();
        assert_eq!(decoded.get_pixel(290, 150)[3], 0);
        assert_eq!(decoded.get_pixel(10, 150)[3], 255);
    }

    #[test]
    fn dominant_color_prefers_the_majority_and_the_vivid() {
        let mostly_red = RgbaImage::from_fn(50, 50, |x, _| if x < 35 { Rgba([220, 20, 30, 255]) } else { Rgba([20, 30, 220, 255]) });
        assert_eq!(dominant_color(&mostly_red), "#dc141e");
        let grey_with_orange = RgbaImage::from_fn(100, 10, |x, _| if x < 55 { Rgba([128, 128, 128, 255]) } else { Rgba([250, 140, 10, 255]) });
        assert_eq!(dominant_color(&grey_with_orange), "#fa8c0a");
        let transparent = RgbaImage::from_pixel(10, 10, Rgba([255, 0, 0, 0]));
        assert_eq!(dominant_color(&transparent), "#000000");
    }

    fn png_with_profile(pixel: [u8; 3], profile: &moxcms::ColorProfile) -> Vec<u8> {
        use image::ImageEncoder as _;
        let pixels = RgbImage::from_pixel(16, 16, Rgb(pixel));
        let mut out = Vec::new();
        let mut encoder = image::codecs::png::PngEncoder::new(&mut out);
        encoder.set_icc_profile(profile.encode().unwrap()).unwrap();
        encoder.write_image(pixels.as_raw(), 16, 16, image::ExtendedColorType::Rgb8).unwrap();
        out
    }

    #[test]
    fn wide_gamut_images_are_converted_to_srgb() {
        // The same numbers mean a more saturated colour in Display P3, so
        // the sRGB equivalent pushes the channels further apart.
        let p3 = decode(&png_with_profile([200, 100, 50], &moxcms::ColorProfile::new_display_p3())).unwrap().into_rgb8();
        let [r, g, b] = p3.get_pixel(8, 8).0;
        assert!(r > 205 && g < 100 && b < 45, "P3 (200,100,50) became ({r},{g},{b})");

        // An sRGB profile changes nothing (beyond rounding).
        let srgb = decode(&png_with_profile([200, 100, 50], &moxcms::ColorProfile::new_srgb())).unwrap().into_rgb8();
        let pixel = srgb.get_pixel(8, 8).0;
        assert!(pixel.iter().zip([200u8, 100, 50]).all(|(a, b)| a.abs_diff(b) <= 1), "{pixel:?}");

        // A broken profile is ignored rather than failing the upload.
        let mut image = DynamicImage::ImageRgb8(RgbImage::from_pixel(2, 2, Rgb([1, 2, 3])));
        convert_to_srgb(&mut image, b"not an icc profile");
        assert_eq!(image.into_rgb8().get_pixel(0, 0).0, [1, 2, 3]);
    }

    #[test]
    fn garbage_and_bombs_are_rejected_not_crashed_on() {
        let reason = |result: ProcessResult<DynamicImage>| match result {
            Err(ProcessError::Rejected { reason, .. }) => reason,
            other => panic!("expected a rejection, got {other:?}"),
        };
        assert_eq!(reason(decode(b"\x89PNG\r\n\x1a\nnot really a png")), "CORRUPT_MEDIA");
        assert_eq!(reason(decode(b"plain text")), "UNSUPPORTED_CONTENT");
        assert!(check_pixel_count(10_000, 10_000).is_err());
        assert!(check_pixel_count(8_000, 8_000).is_ok());
    }
}
