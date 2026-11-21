//! What each [`AssetPurpose`] accepts, and what uploaded bytes really are.
//!
//! The client declares a content type when it starts an upload; that only
//! picks the size limit and lets obviously wrong uploads fail early. Once the
//! bytes are stored their type is detected from the content ([`sniff`]) and
//! matched against the purpose again ([`resolve`]). The declared type is
//! never trusted for anything that matters.
use tokuly_proto::media::{AssetKind, AssetPurpose};

use crate::config::{MediaConfig, megabytes};

/// Bytes read from the start of an upload to detect its type.
pub const SNIFF_BYTES: usize = 64 * 1024;

/// Largest dotLottie stamp and subtitle file.
const MAX_LOTTIE_BYTES: i64 = megabytes(2);
const MAX_SUBTITLE_BYTES: i64 = megabytes(2);

pub const LOTTIE_CONTENT_TYPE: &str = "application/zip+dotlottie";
const OCTET_STREAM: &str = "application/octet-stream";

/// Coarse class of uploaded content, as far as the leading bytes tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentClass {
    /// JPEG, PNG (incl. APNG), WebP or GIF: what the image pipeline decodes.
    Image,
    /// A container that may hold video and/or audio (MP4, MOV, WebM, MKV...).
    AvContainer,
    /// An audio-only format (MP3, M4A, Ogg, FLAC, WAV...).
    Audio,
    /// WebVTT or SubRip text.
    Subtitle,
    /// A ZIP archive (dotLottie stamps are ZIP files).
    Zip,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sniffed {
    pub class: ContentClass,
    pub content_type: String,
}

impl Sniffed {
    fn new(class: ContentClass, content_type: &str) -> Self {
        Self { class, content_type: content_type.to_owned() }
    }
}

/// The purposes a client may upload for, and the largest file of each.
/// `None` for purposes that only the server creates (or that do not exist).
pub fn max_bytes(cfg: &MediaConfig, purpose: AssetPurpose, declared: ContentClass) -> Option<i64> {
    use AssetPurpose::*;
    let mb = match purpose {
        ASSET_PURPOSE_AVATAR
        | ASSET_PURPOSE_BANNER
        | ASSET_PURPOSE_THUMBNAIL
        | ASSET_PURPOSE_CHANNEL_ICON
        | ASSET_PURPOSE_CHANNEL_BANNER => cfg.max_avatar_mb,
        ASSET_PURPOSE_STAMP => {
            return Some(if declared == ContentClass::Zip { MAX_LOTTIE_BYTES } else { megabytes(cfg.max_avatar_mb) });
        }
        ASSET_PURPOSE_CHAT_IMAGE | ASSET_PURPOSE_NOTE_IMAGE | ASSET_PURPOSE_PLACE_PHOTO | ASSET_PURPOSE_DAY_MEMO => {
            cfg.max_image_mb
        }
        ASSET_PURPOSE_STORY => {
            if declared == ContentClass::Image {
                cfg.max_image_mb
            } else {
                cfg.max_clip_mb
            }
        }
        ASSET_PURPOSE_AVATAR_VIDEO | ASSET_PURPOSE_CHAT_VIDEO | ASSET_PURPOSE_NOTE_VIDEO => cfg.max_clip_mb,
        ASSET_PURPOSE_VIDEO => cfg.max_video_mb,
        ASSET_PURPOSE_SHORT => cfg.max_short_mb,
        ASSET_PURPOSE_CHAT_AUDIO => cfg.max_audio_mb,
        ASSET_PURPOSE_CHAT_FILE => cfg.max_file_mb,
        ASSET_PURPOSE_SUBTITLE => return Some(MAX_SUBTITLE_BYTES),
        // Live archives are produced by the server (`create_asset_from_file`).
        ASSET_PURPOSE_LIVE_ARCHIVE | ASSET_PURPOSE_UNSPECIFIED => return None,
    };
    Some(megabytes(mb))
}

/// The kind an upload of `class` becomes for `purpose`, or `None` when the
/// purpose does not take such content.
pub fn resolve(purpose: AssetPurpose, class: ContentClass) -> Option<AssetKind> {
    use AssetKind::*;
    use AssetPurpose::*;
    use ContentClass::*;
    match (purpose, class) {
        (
            ASSET_PURPOSE_AVATAR
            | ASSET_PURPOSE_BANNER
            | ASSET_PURPOSE_THUMBNAIL
            | ASSET_PURPOSE_CHANNEL_ICON
            | ASSET_PURPOSE_CHANNEL_BANNER
            | ASSET_PURPOSE_CHAT_IMAGE
            | ASSET_PURPOSE_NOTE_IMAGE
            | ASSET_PURPOSE_PLACE_PHOTO
            | ASSET_PURPOSE_DAY_MEMO
            | ASSET_PURPOSE_STAMP
            | ASSET_PURPOSE_STORY,
            Image,
        ) => Some(ASSET_KIND_IMAGE),
        (ASSET_PURPOSE_STAMP, Zip) => Some(ASSET_KIND_FILE),
        (
            ASSET_PURPOSE_AVATAR_VIDEO
            | ASSET_PURPOSE_CHAT_VIDEO
            | ASSET_PURPOSE_NOTE_VIDEO
            | ASSET_PURPOSE_STORY
            | ASSET_PURPOSE_VIDEO
            | ASSET_PURPOSE_SHORT
            | ASSET_PURPOSE_LIVE_ARCHIVE,
            AvContainer,
        ) => Some(ASSET_KIND_VIDEO),
        // Browsers record voice messages into WebM/MP4 containers.
        (ASSET_PURPOSE_CHAT_AUDIO, Audio | AvContainer) => Some(ASSET_KIND_AUDIO),
        (ASSET_PURPOSE_CHAT_FILE, _) => Some(ASSET_KIND_FILE),
        (ASSET_PURPOSE_SUBTITLE, Subtitle) => Some(ASSET_KIND_FILE),
        _ => None,
    }
}

/// Whether an animated upload keeps its animation for this purpose (it is
/// flattened to its first frame everywhere else).
pub fn keeps_animation(purpose: AssetPurpose) -> bool {
    use AssetPurpose::*;
    matches!(
        purpose,
        ASSET_PURPOSE_AVATAR | ASSET_PURPOSE_STAMP | ASSET_PURPOSE_CHAT_IMAGE | ASSET_PURPOSE_NOTE_IMAGE | ASSET_PURPOSE_STORY
    )
}

/// Whether files of this purpose are published in the media bucket instead
/// of staying private (dotLottie stamps are shown to everyone).
pub fn publishes_files(purpose: AssetPurpose) -> bool {
    purpose == AssetPurpose::ASSET_PURPOSE_STAMP
}

/// Tallest HLS rung produced for a purpose (short side in pixels).
pub fn max_rung(purpose: AssetPurpose) -> u32 {
    use AssetPurpose::*;
    match purpose {
        ASSET_PURPOSE_AVATAR_VIDEO => 480,
        ASSET_PURPOSE_CHAT_VIDEO | ASSET_PURPOSE_NOTE_VIDEO | ASSET_PURPOSE_STORY => 720,
        _ => 1080,
    }
}

/// Class implied by what the client says it is about to upload. Used before
/// any bytes exist; an empty or generic content type falls back to the file
/// extension.
pub fn classify_declared(content_type: &str, file_name: &str) -> Sniffed {
    let essence = content_type.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    let extension = extension_of(file_name);
    let class = match essence.as_str() {
        "image/jpeg" | "image/jpg" | "image/png" | "image/apng" | "image/webp" | "image/gif" => ContentClass::Image,
        "text/vtt" | "application/x-subrip" | "text/srt" => ContentClass::Subtitle,
        "application/zip" | "application/x-zip-compressed" | LOTTIE_CONTENT_TYPE => ContentClass::Zip,
        t if t.starts_with("video/") => ContentClass::AvContainer,
        t if t.starts_with("audio/") => ContentClass::Audio,
        "" | OCTET_STREAM | "text/plain" => match extension.as_str() {
            "jpg" | "jpeg" | "png" | "apng" | "webp" | "gif" => ContentClass::Image,
            "mp4" | "m4v" | "mov" | "webm" | "mkv" | "avi" | "3gp" => ContentClass::AvContainer,
            "mp3" | "m4a" | "aac" | "ogg" | "oga" | "opus" | "wav" | "flac" | "amr" => ContentClass::Audio,
            "vtt" | "srt" => ContentClass::Subtitle,
            "lottie" | "zip" => ContentClass::Zip,
            _ => ContentClass::Other,
        },
        _ => ContentClass::Other,
    };
    let content_type = if essence.is_empty() { OCTET_STREAM.to_owned() } else { essence };
    Sniffed { class, content_type }
}

/// Detects what `prefix` (the first [`SNIFF_BYTES`] of an upload) is.
pub fn sniff(prefix: &[u8]) -> Sniffed {
    if let Some(kind) = infer::get(prefix) {
        let mime = kind.mime_type();
        let class = match kind.matcher_type() {
            infer::MatcherType::Image if matches!(mime, "image/jpeg" | "image/png" | "image/webp" | "image/gif") => {
                ContentClass::Image
            }
            infer::MatcherType::Video => ContentClass::AvContainer,
            infer::MatcherType::Audio if mime != "audio/midi" => ContentClass::Audio,
            _ if mime == "application/zip" => ContentClass::Zip,
            _ => ContentClass::Other,
        };
        return Sniffed::new(class, mime);
    }
    if let Some(subtitle) = sniff_subtitle(prefix) {
        return Sniffed::new(ContentClass::Subtitle, subtitle);
    }
    if looks_like_text(prefix) {
        return Sniffed::new(ContentClass::Other, "text/plain");
    }
    Sniffed::new(ContentClass::Other, OCTET_STREAM)
}

fn looks_like_text(prefix: &[u8]) -> bool {
    !prefix.is_empty() && !prefix.contains(&0)
}

/// WebVTT starts with its magic word; SubRip has no magic, so the first cue
/// must look like one ("1", then a line with a timing arrow).
fn sniff_subtitle(prefix: &[u8]) -> Option<&'static str> {
    if !looks_like_text(prefix) {
        return None;
    }
    let text = String::from_utf8_lossy(prefix);
    let text = text.trim_start_matches('\u{feff}');
    if text.starts_with("WEBVTT") {
        return Some("text/vtt");
    }
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let counter = lines.next()?;
    let timing = lines.next()?;
    let is_cue = counter.bytes().all(|b| b.is_ascii_digit()) && timing.contains("-->") && timing.starts_with(|c: char| c.is_ascii_digit());
    is_cue.then_some("application/x-subrip")
}

/// A dotLottie file is a ZIP archive with a `manifest.json` entry. The whole
/// file is checked: entry names of a ZIP are stored uncompressed.
pub fn is_dot_lottie(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") && bytes.windows(13).any(|w| w == b"manifest.json")
}

/// Lowercase extension of a file name, limited to short alphanumeric ones
/// (it becomes part of an object key).
pub fn extension_of(file_name: &str) -> String {
    file_name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .filter(|ext| (1..=8).contains(&ext.len()) && ext.bytes().all(|b| b.is_ascii_alphanumeric()))
        .unwrap_or_default()
}

/// A display-safe version of a client-supplied file name: no directories, no
/// control characters, bounded length, never empty.
pub fn sanitize_file_name(file_name: &str) -> String {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or_default();
    let clean: String = base.chars().filter(|c| !c.is_control()).collect();
    let clean = clean.trim().trim_matches('.').trim();
    let mut out: String = clean.chars().take(200).collect();
    if out.is_empty() {
        out.push_str("upload");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
    const MP4: &[u8] = b"\0\0\0\x18ftypmp42\0\0\0\0mp42isom";

    #[test]
    fn sniffing_ignores_what_the_client_claims() {
        assert_eq!(sniff(PNG).class, ContentClass::Image);
        assert_eq!(sniff(MP4).class, ContentClass::AvContainer);
        assert_eq!(sniff(b"ID3\x03\0\0\0\0\0\0").class, ContentClass::Audio);
        assert_eq!(sniff(b"MZ\x90\0\x03\0\0\0").class, ContentClass::Other);
        assert_eq!(sniff(b"<html><script>alert(1)</script>").class, ContentClass::Other);
        assert_eq!(sniff(b"plain words").content_type, "text/plain");
        assert_eq!(sniff(b"\0\x01\x02\x03").content_type, "application/octet-stream");
        assert_eq!(sniff(b"").class, ContentClass::Other);
    }

    #[test]
    fn subtitles_are_recognised_by_their_first_cue() {
        assert_eq!(sniff(b"WEBVTT\n\n00:00.000 --> 00:01.000\nhi").content_type, "text/vtt");
        assert_eq!(sniff("\u{feff}1\r\n00:00:01,000 --> 00:00:02,000\r\nこんにちは".as_bytes()).content_type, "application/x-subrip");
        assert_eq!(sniff(b"just some notes --> not a cue").class, ContentClass::Other);
    }

    #[test]
    fn purposes_only_take_their_own_kind_of_content() {
        use AssetPurpose::*;
        assert_eq!(resolve(ASSET_PURPOSE_AVATAR, ContentClass::Image), Some(AssetKind::ASSET_KIND_IMAGE));
        assert_eq!(resolve(ASSET_PURPOSE_AVATAR, ContentClass::AvContainer), None);
        assert_eq!(resolve(ASSET_PURPOSE_VIDEO, ContentClass::Image), None);
        assert_eq!(resolve(ASSET_PURPOSE_STORY, ContentClass::AvContainer), Some(AssetKind::ASSET_KIND_VIDEO));
        assert_eq!(resolve(ASSET_PURPOSE_CHAT_AUDIO, ContentClass::AvContainer), Some(AssetKind::ASSET_KIND_AUDIO));
        assert_eq!(resolve(ASSET_PURPOSE_CHAT_FILE, ContentClass::Other), Some(AssetKind::ASSET_KIND_FILE));
        assert_eq!(resolve(ASSET_PURPOSE_SUBTITLE, ContentClass::Other), None);
        assert_eq!(resolve(ASSET_PURPOSE_STAMP, ContentClass::Zip), Some(AssetKind::ASSET_KIND_FILE));
        assert_eq!(resolve(ASSET_PURPOSE_UNSPECIFIED, ContentClass::Image), None);
    }

    #[test]
    fn limits_depend_on_purpose_and_declared_content() {
        let cfg = MediaConfig::default();
        use AssetPurpose::*;
        assert_eq!(max_bytes(&cfg, ASSET_PURPOSE_AVATAR, ContentClass::Image), Some(megabytes(10)));
        assert_eq!(max_bytes(&cfg, ASSET_PURPOSE_CHAT_VIDEO, ContentClass::AvContainer), Some(megabytes(200)));
        assert_eq!(max_bytes(&cfg, ASSET_PURPOSE_VIDEO, ContentClass::AvContainer), Some(megabytes(20 * 1024)));
        assert_eq!(max_bytes(&cfg, ASSET_PURPOSE_SHORT, ContentClass::AvContainer), Some(megabytes(1024)));
        assert_eq!(max_bytes(&cfg, ASSET_PURPOSE_STORY, ContentClass::Image), Some(megabytes(25)));
        assert_eq!(max_bytes(&cfg, ASSET_PURPOSE_STORY, ContentClass::AvContainer), Some(megabytes(200)));
        assert_eq!(max_bytes(&cfg, ASSET_PURPOSE_LIVE_ARCHIVE, ContentClass::AvContainer), None);
    }

    #[test]
    fn declared_types_fall_back_to_the_extension() {
        assert_eq!(classify_declared("image/jpeg", "a.bin").class, ContentClass::Image);
        assert_eq!(classify_declared("video/mp4; codecs=avc1", "a").class, ContentClass::AvContainer);
        assert_eq!(classify_declared("", "clip.MOV").class, ContentClass::AvContainer);
        assert_eq!(classify_declared("", "sticker.lottie").class, ContentClass::Zip);
        assert_eq!(classify_declared("application/pdf", "a.png").class, ContentClass::Other);
        assert_eq!(classify_declared("", "noext").content_type, "application/octet-stream");
    }

    #[test]
    fn file_names_are_made_safe() {
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name("C:\\Users\\me\\写真.jpg"), "写真.jpg");
        assert_eq!(sanitize_file_name("  \u{7}\n "), "upload");
        assert_eq!(sanitize_file_name(&"x".repeat(500)).chars().count(), 200);
        assert_eq!(extension_of("a.tar.GZ"), "gz");
        assert_eq!(extension_of("weird.ex/../t"), "");
        assert_eq!(extension_of("noext"), "");
    }

    #[test]
    fn dot_lottie_needs_a_manifest() {
        assert!(is_dot_lottie(b"PK\x03\x04....manifest.json...."));
        assert!(!is_dot_lottie(b"PK\x03\x04....other.json...."));
        assert!(!is_dot_lottie(b"manifest.json"));
    }
}
