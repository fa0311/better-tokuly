//! Animated images (GIF, APNG, animated WebP) → animated WebP.
//!
//! Frames are decoded one at a time, scaled down as they arrive and
//! re-encoded with libwebp, which drops all metadata like the still path
//! does. The encoder needs every frame in memory at once, so the animation
//! is bounded three ways: a longest edge of [`MAX_EDGE`], at most
//! [`MAX_FRAMES`] frames, and a total pixel budget across all frames
//! ([`PIXEL_BUDGET`]). An animation over budget is halved in size (down to
//! [`MIN_EDGE`]) before it is refused.
use std::io::Cursor;

use image::{
    AnimationDecoder as _, DynamicImage, ImageDecoder as _, ImageFormat, RgbaImage,
    codecs::{gif::GifDecoder, png::PngDecoder, webp::WebPDecoder},
    imageops::FilterType,
};

use super::{
    ProcessError, ProcessResult,
    image::{check_pixel_count, decode_error, decode_limits, scaled_dimensions},
};

/// Longest edge of the animated rendition (stamps, avatars, chat GIFs).
pub const MAX_EDGE: u32 = 512;
const MIN_EDGE: u32 = 128;
pub const MAX_FRAMES: usize = 600;
/// Pixels held in memory across all frames (×4 bytes, twice: ours and the
/// encoder's copy — about 256 MB at the limit).
const PIXEL_BUDGET: u64 = 32_000_000;
const WEBP_QUALITY: f32 = 75.0;
/// Browsers play frames with a near-zero delay at 10 fps; so do we.
const MIN_DELAY_MS: u32 = 20;
const DEFAULT_DELAY_MS: u32 = 100;

#[derive(Debug)]
pub struct RenderedAnimation {
    pub webp: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub frame_count: usize,
    /// The first frame at full resolution: the still poster.
    pub poster: DynamicImage,
}

/// Whether the bytes hold an image format that can carry more than one frame
/// and this file does.
pub fn is_animated(bytes: &[u8]) -> bool {
    match image::guess_format(bytes) {
        Ok(ImageFormat::Gif) => {
            GifDecoder::new(Cursor::new(bytes)).map(|decoder| decoder.into_frames().take(2).filter(Result::is_ok).count() == 2).unwrap_or(false)
        }
        Ok(ImageFormat::Png) => PngDecoder::new(Cursor::new(bytes)).and_then(|decoder| decoder.is_apng()).unwrap_or(false),
        Ok(ImageFormat::WebP) => WebPDecoder::new(Cursor::new(bytes)).map(|decoder| decoder.has_animation()).unwrap_or(false),
        _ => false,
    }
}

fn frames(bytes: &[u8]) -> ProcessResult<image::Frames<'_>> {
    let cursor = Cursor::new(bytes);
    let frames = match image::guess_format(bytes).map_err(decode_error)? {
        ImageFormat::Gif => {
            let mut decoder = GifDecoder::new(cursor).map_err(decode_error)?;
            decoder.set_limits(decode_limits()).map_err(decode_error)?;
            decoder.into_frames()
        }
        ImageFormat::Png => PngDecoder::with_limits(cursor, decode_limits()).and_then(|decoder| decoder.apng()).map_err(decode_error)?.into_frames(),
        ImageFormat::WebP => {
            let mut decoder = WebPDecoder::new(cursor).map_err(decode_error)?;
            decoder.set_limits(decode_limits()).map_err(decode_error)?;
            decoder.into_frames()
        }
        other => return Err(ProcessError::rejected("UNSUPPORTED_CONTENT", format!("{other:?} cannot be animated"))),
    };
    Ok(frames)
}

fn delay_ms(frame: &image::Frame) -> u32 {
    let (numerator, denominator) = frame.delay().numer_denom_ms();
    let delay = numerator / denominator.max(1);
    if delay < MIN_DELAY_MS { DEFAULT_DELAY_MS } else { delay.min(60_000) }
}

fn fit(frame: &RgbaImage, edge: u32) -> RgbaImage {
    let (width, height) = scaled_dimensions(frame.width(), frame.height(), edge);
    if (width, height) == frame.dimensions() { frame.clone() } else { image::imageops::resize(frame, width, height, FilterType::Triangle) }
}

/// Decodes and re-encodes an animation. `Ok(None)` when the file turns out
/// to hold a single frame (the caller then treats it as a still).
///
/// `max_edge` is the longest edge its purpose keeps (at most [`MAX_EDGE`]).
pub fn render(bytes: &[u8], max_edge: u32) -> ProcessResult<Option<RenderedAnimation>> {
    let mut edge = max_edge.clamp(MIN_EDGE, MAX_EDGE);
    let mut scaled: Vec<(RgbaImage, u32)> = Vec::new();
    let mut poster: Option<RgbaImage> = None;
    let mut pixels: u64 = 0;

    for frame in frames(bytes)? {
        let frame = frame.map_err(decode_error)?;
        let delay = delay_ms(&frame);
        let buffer = frame.into_buffer();
        if poster.is_none() {
            check_pixel_count(buffer.width(), buffer.height())?;
        }
        if scaled.len() == MAX_FRAMES {
            return Err(ProcessError::rejected("ANIMATION_TOO_LARGE", format!("more than {MAX_FRAMES} frames")));
        }
        let small = fit(&buffer, edge);
        pixels += u64::from(small.width()) * u64::from(small.height());
        scaled.push((small, delay));
        poster.get_or_insert(buffer);

        // Over budget: continue at half the size rather than give up.
        while pixels > PIXEL_BUDGET {
            edge /= 2;
            if edge < MIN_EDGE {
                return Err(ProcessError::rejected("ANIMATION_TOO_LARGE", "the animation is too long to keep in memory"));
            }
            for (image, _) in &mut scaled {
                *image = fit(image, edge);
            }
            pixels = scaled.iter().map(|(image, _)| u64::from(image.width()) * u64::from(image.height())).sum();
        }
    }

    let Some(poster) = poster else {
        return Err(ProcessError::rejected("CORRUPT_MEDIA", "the animation has no frames"));
    };
    if scaled.len() < 2 {
        return Ok(None);
    }
    let (width, height) = scaled[0].0.dimensions();
    let webp = encode(&scaled, width, height)?;
    Ok(Some(RenderedAnimation { webp, width, height, frame_count: scaled.len(), poster: DynamicImage::ImageRgba8(poster) }))
}

fn encode(frames: &[(RgbaImage, u32)], width: u32, height: u32) -> ProcessResult<Vec<u8>> {
    let mut config = webp::WebPConfig::new().map_err(|()| ProcessError::transient("webp config"))?;
    config.lossless = 0;
    config.alpha_compression = 1;
    config.quality = WEBP_QUALITY;
    let mut encoder = webp::AnimEncoder::new(width, height, &config);
    encoder.set_loop_count(0); // loop forever
    let mut timestamp_ms: i32 = 0;
    for (image, delay) in frames {
        // Every frame starts when the previous one ends. The encoder gives
        // the last frame the average duration of the others.
        encoder.add_frame(webp::AnimFrame::from_rgba(image.as_raw(), width, height, timestamp_ms));
        timestamp_ms = timestamp_ms.saturating_add(*delay as i32);
    }
    let encoded = encoder.try_encode().map_err(|e| ProcessError::transient(format!("animated webp encode: {e:?}")))?;
    Ok(encoded.to_vec())
}

#[cfg(test)]
mod tests {
    use image::{Delay, Frame, Rgba, codecs::gif::GifEncoder};

    use super::*;

    /// A GIF whose frames slide a coloured square to the right.
    pub(crate) fn test_gif(width: u32, height: u32, frame_count: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = GifEncoder::new_with_speed(&mut out, 30);
            encoder.set_repeat(image::codecs::gif::Repeat::Infinite).unwrap();
            for index in 0..frame_count {
                let buffer = RgbaImage::from_fn(width, height, |x, y| {
                    let left = index * 4 % width;
                    if (left..left + 16).contains(&x) && y < 16 { Rgba([250, 40, 60, 255]) } else { Rgba([20, 90, 200, 255]) }
                });
                encoder.encode_frame(Frame::from_parts(buffer, 0, 0, Delay::from_numer_denom_ms(80, 1))).unwrap();
            }
        }
        out
    }

    #[test]
    fn animated_gif_becomes_animated_webp() {
        let gif = test_gif(120, 80, 6);
        assert!(is_animated(&gif));
        let rendered = render(&gif, MAX_EDGE).unwrap().expect("animated");
        assert_eq!((rendered.width, rendered.height, rendered.frame_count), (120, 80, 6));
        assert_eq!((rendered.poster.width(), rendered.poster.height()), (120, 80));

        let decoder = WebPDecoder::new(Cursor::new(&rendered.webp)).unwrap();
        assert!(decoder.has_animation());
        let decoded = decoder.into_frames().collect_frames().unwrap();
        assert_eq!(decoded.len(), 6);
        assert_eq!(decoded[0].delay().numer_denom_ms(), (80, 1));
    }

    /// An APNG with the given solid-colour frames.
    fn test_apng(width: u32, height: u32, colours: &[[u8; 4]]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_animated(colours.len() as u32, 0).unwrap();
            encoder.set_frame_delay(3, 20).unwrap(); // 150 ms
            let mut writer = encoder.write_header().unwrap();
            for colour in colours {
                let frame: Vec<u8> = colour.iter().copied().cycle().take((width * height * 4) as usize).collect();
                writer.write_image_data(&frame).unwrap();
            }
            writer.finish().unwrap();
        }
        out
    }

    #[test]
    fn apng_and_animated_webp_inputs_are_animations_too() {
        let apng = test_apng(48, 32, &[[255, 0, 0, 255], [0, 255, 0, 128], [0, 0, 255, 255]]);
        assert!(is_animated(&apng));
        let rendered = render(&apng, MAX_EDGE).unwrap().expect("animated");
        assert_eq!((rendered.width, rendered.height, rendered.frame_count), (48, 32, 3));
        let frames = WebPDecoder::new(Cursor::new(&rendered.webp)).unwrap().into_frames().collect_frames().unwrap();
        assert_eq!(frames[0].delay().numer_denom_ms(), (150, 1));
        assert!(frames[0].buffer().get_pixel(10, 10)[0] > 200, "first frame is red");

        // Our own output is an animated WebP: feed it back in.
        assert!(is_animated(&rendered.webp));
        let again = render(&rendered.webp, MAX_EDGE).unwrap().expect("animated");
        assert_eq!((again.width, again.height, again.frame_count), (48, 32, 3));

        // A PNG with a single animation frame is just a picture.
        assert!(render(&test_apng(16, 16, &[[9, 9, 9, 255]]), MAX_EDGE).unwrap().is_none());
    }

    #[test]
    fn large_animations_are_scaled_down() {
        let gif = test_gif(1024, 640, 3);
        let rendered = render(&gif, MAX_EDGE).unwrap().expect("animated");
        assert_eq!((rendered.width, rendered.height), (512, 320));
        assert_eq!(rendered.poster.width(), 1024, "the poster keeps the original resolution");
    }

    #[test]
    fn single_frames_and_stills_are_not_animations() {
        let gif = test_gif(32, 32, 1);
        assert!(!is_animated(&gif));
        assert!(render(&gif, MAX_EDGE).unwrap().is_none());

        let mut png = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 8, Rgba([1, 2, 3, 255]))).write_to(&mut png, ImageFormat::Png).unwrap();
        assert!(!is_animated(png.get_ref()));
        assert!(!is_animated(b"not an image"));
    }
}
