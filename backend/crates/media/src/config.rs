//! Media-specific settings.
use std::{path::PathBuf, time::Duration};

use clap::Args;

const MB: i64 = 1024 * 1024;

/// S3 refuses multipart parts smaller than this (except the last one).
pub const MIN_PART_BYTES: i64 = 5 * MB;
/// S3 refuses uploads with more parts than this.
pub const MAX_PARTS: i64 = 10_000;

#[derive(Debug, Clone, Args)]
pub struct MediaConfig {
    // ── upload limits (megabytes) ──
    /// Largest profile-style image: avatars, banners, channel art, thumbnails, stamps.
    #[arg(long, env = "TOKULY_MEDIA_MAX_AVATAR_MB", default_value_t = 10)]
    pub max_avatar_mb: i64,
    /// Largest photo in chats, notes, stories, places and day memos.
    #[arg(long, env = "TOKULY_MEDIA_MAX_IMAGE_MB", default_value_t = 25)]
    pub max_image_mb: i64,
    /// Largest short clip: chat and note videos, stories, avatar videos.
    #[arg(long, env = "TOKULY_MEDIA_MAX_CLIP_MB", default_value_t = 200)]
    pub max_clip_mb: i64,
    /// Largest long-form video upload.
    #[arg(long, env = "TOKULY_MEDIA_MAX_VIDEO_MB", default_value_t = 20 * 1024)]
    pub max_video_mb: i64,
    /// Largest short (vertical video) upload.
    #[arg(long, env = "TOKULY_MEDIA_MAX_SHORT_MB", default_value_t = 1024)]
    pub max_short_mb: i64,
    /// Largest voice message.
    #[arg(long, env = "TOKULY_MEDIA_MAX_AUDIO_MB", default_value_t = 25)]
    pub max_audio_mb: i64,
    /// Largest file attachment in chats.
    #[arg(long, env = "TOKULY_MEDIA_MAX_FILE_MB", default_value_t = 100)]
    pub max_file_mb: i64,

    // ── uploads ──
    /// Files up to this size are uploaded with one PUT; larger ones in parts.
    #[arg(long, env = "TOKULY_MEDIA_MULTIPART_THRESHOLD_MB", default_value_t = 16)]
    pub multipart_threshold_mb: i64,
    /// Preferred part size of multipart uploads (grows for very large files
    /// so the part count stays within the store's limit).
    #[arg(long, env = "TOKULY_MEDIA_PART_SIZE_MB", default_value_t = 16)]
    pub part_size_mb: i64,
    /// Hours an unfinished upload is kept before the janitor discards it.
    #[arg(long, env = "TOKULY_MEDIA_UPLOAD_TTL_HOURS", default_value_t = 24)]
    pub upload_ttl_hours: i64,
    /// Hours a finished upload may stay unused (no holder) before the janitor
    /// deletes it. 0 keeps unused uploads forever.
    #[arg(long, env = "TOKULY_MEDIA_UNATTACHED_TTL_HOURS", default_value_t = 48)]
    pub unattached_ttl_hours: i64,

    // ── processing ──
    /// Images, animations and voice messages processed in parallel per worker process.
    #[arg(long, env = "TOKULY_MEDIA_IMAGE_CONCURRENCY", default_value_t = 4)]
    pub image_concurrency: usize,
    /// Videos transcoded in parallel per worker process.
    #[arg(long, env = "TOKULY_MEDIA_VIDEO_CONCURRENCY", default_value_t = 1)]
    pub video_concurrency: usize,
    /// Threads each ffmpeg process may use; 0 lets ffmpeg decide (all cores).
    #[arg(long, env = "TOKULY_MEDIA_FFMPEG_THREADS", default_value_t = 0)]
    pub ffmpeg_threads: u32,
    /// x264 speed/quality preset for the HLS ladder.
    #[arg(long, env = "TOKULY_MEDIA_X264_PRESET", default_value = "veryfast")]
    pub x264_preset: String,
    /// Adds an AV1 rendition of the top rung (smaller files, slower encode).
    #[arg(long, env = "TOKULY_MEDIA_AV1", default_value_t = false, action = clap::ArgAction::Set)]
    pub av1: bool,
    /// Longest video accepted, in hours.
    #[arg(long, env = "TOKULY_MEDIA_MAX_VIDEO_HOURS", default_value_t = 12)]
    pub max_video_hours: i64,
    #[arg(long, env = "TOKULY_MEDIA_FFMPEG_PATH", default_value = "ffmpeg")]
    pub ffmpeg_path: PathBuf,
    #[arg(long, env = "TOKULY_MEDIA_FFPROBE_PATH", default_value = "ffprobe")]
    pub ffprobe_path: PathBuf,
    /// Scratch directory for transcodes; needs room for the largest upload
    /// plus its renditions. Defaults to the system temp directory.
    #[arg(long, env = "TOKULY_MEDIA_WORK_DIR")]
    pub work_dir: Option<PathBuf>,

    /// Lets the link preview fetcher reach loopback addresses on any port.
    /// Exists for integration tests only, so it is not a flag or variable.
    #[arg(skip)]
    pub link_preview_allow_loopback: bool,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            max_avatar_mb: 10,
            max_image_mb: 25,
            max_clip_mb: 200,
            max_video_mb: 20 * 1024,
            max_short_mb: 1024,
            max_audio_mb: 25,
            max_file_mb: 100,
            multipart_threshold_mb: 16,
            part_size_mb: 16,
            upload_ttl_hours: 24,
            unattached_ttl_hours: 48,
            image_concurrency: 4,
            video_concurrency: 1,
            ffmpeg_threads: 0,
            x264_preset: "veryfast".to_owned(),
            av1: false,
            max_video_hours: 12,
            ffmpeg_path: PathBuf::from("ffmpeg"),
            ffprobe_path: PathBuf::from("ffprobe"),
            work_dir: None,
            link_preview_allow_loopback: false,
        }
    }
}

impl MediaConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.part_size_mb * MB >= MIN_PART_BYTES,
            "TOKULY_MEDIA_PART_SIZE_MB must be at least 5 (the object store rejects smaller parts)"
        );
        anyhow::ensure!(self.multipart_threshold_mb >= 1, "TOKULY_MEDIA_MULTIPART_THRESHOLD_MB must be at least 1");
        anyhow::ensure!(self.upload_ttl_hours >= 1, "TOKULY_MEDIA_UPLOAD_TTL_HOURS must be at least 1");
        anyhow::ensure!(self.unattached_ttl_hours >= 0, "TOKULY_MEDIA_UNATTACHED_TTL_HOURS cannot be negative (0 disables collection)");
        Ok(())
    }

    pub fn multipart_threshold_bytes(&self) -> i64 {
        self.multipart_threshold_mb * MB
    }

    pub fn upload_ttl(&self) -> Duration {
        Duration::from_secs(self.upload_ttl_hours.max(1) as u64 * 3600)
    }

    /// How long a READY asset may go without a holder; `None` when unused
    /// uploads are never collected.
    pub fn unattached_ttl(&self) -> Option<Duration> {
        (self.unattached_ttl_hours > 0).then(|| Duration::from_secs(self.unattached_ttl_hours as u64 * 3600))
    }

    pub fn max_video_seconds(&self) -> f64 {
        self.max_video_hours.max(1) as f64 * 3600.0
    }

    /// Part size and part count for a multipart upload of `size_bytes`.
    pub fn part_layout(&self, size_bytes: i64) -> (i64, i32) {
        let preferred = (self.part_size_mb * MB).max(MIN_PART_BYTES);
        // Very large files need bigger parts to stay under the part limit.
        let needed = div_ceil(size_bytes, MAX_PARTS);
        let part_size = preferred.max(div_ceil(needed, MB) * MB);
        (part_size, div_ceil(size_bytes, part_size) as i32)
    }
}

/// `a / b` rounded up, for positive numbers.
fn div_ceil(a: i64, b: i64) -> i64 {
    (a + b - 1) / b
}

pub const fn megabytes(mb: i64) -> i64 {
    mb * MB
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_layout_covers_the_file_within_store_limits() {
        let cfg = MediaConfig::default();
        assert_eq!(cfg.part_layout(megabytes(40)), (megabytes(16), 3));
        assert_eq!(cfg.part_layout(megabytes(16) + 1), (megabytes(16), 2));
        // 400 GB would need 25 600 parts of 16 MB: the part size grows instead.
        let (part_size, parts) = cfg.part_layout(megabytes(400 * 1024));
        assert!(i64::from(parts) <= MAX_PARTS && part_size * i64::from(parts) >= megabytes(400 * 1024));
        assert_eq!(part_size % MB, 0);
    }

    #[test]
    fn defaults_match_the_command_line_defaults() {
        #[derive(clap::Parser)]
        struct Cli {
            #[command(flatten)]
            media: MediaConfig,
        }
        let parsed = <Cli as clap::Parser>::parse_from(["test"]).media;
        assert_eq!(format!("{parsed:?}"), format!("{:?}", MediaConfig::default()));
        parsed.validate().unwrap();
    }
}
