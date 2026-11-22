//! Live-specific settings.
//!
//! The server flattens every domain's settings into one command line, where
//! both the flag and clap's internal argument ID must be unique, so every
//! argument carries a `live` (or `mediamtx`) prefix in both.
//!
//! The media server (MediaMTX) counts as configured when
//! `TOKULY_MEDIAMTX_API_URL` is set. Without it streams can be created and
//! managed, but nothing can go on air. The public URLs default to the ports
//! of `infra/mediamtx/mediamtx.yml` on localhost.
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct LiveConfig {
    /// Control API of the media server as the worker reaches it, e.g.
    /// `http://mediamtx:9997`. Empty: live streaming is not available.
    #[arg(id = "mediamtx_api_url", long = "mediamtx-api-url", env = "TOKULY_MEDIAMTX_API_URL", default_value = "")]
    pub mediamtx_api_url: String,

    /// Secret the media server presents with every question to
    /// `/hooks/live/auth` (in the `Authorization` header). Required when
    /// the media server is configured.
    #[arg(
        id = "live_hook_secret",
        long = "live-hook-secret",
        env = "TOKULY_LIVE_HOOK_SECRET",
        default_value = "",
        hide_env_values = true
    )]
    pub hook_secret: String,

    /// Password the worker presents to the media server's control API
    /// (which has the hook confirm it). Not the hook secret: one must not
    /// open the other. Required when the media server is configured.
    #[arg(
        id = "mediamtx_api_secret",
        long = "mediamtx-api-secret",
        env = "TOKULY_MEDIAMTX_API_SECRET",
        default_value = "",
        hide_env_values = true
    )]
    pub mediamtx_api_secret: String,

    /// Where encoders publish over RTMP, as they reach it.
    #[arg(
        id = "live_rtmp_url",
        long = "live-rtmp-url",
        env = "TOKULY_LIVE_RTMP_URL",
        default_value = "rtmp://localhost:1935"
    )]
    pub rtmp_url: String,

    /// Where encoders publish over SRT.
    #[arg(
        id = "live_srt_url",
        long = "live-srt-url",
        env = "TOKULY_LIVE_SRT_URL",
        default_value = "srt://localhost:8890"
    )]
    pub srt_url: String,

    /// Where browsers publish over WebRTC (WHIP).
    #[arg(
        id = "live_whip_url",
        long = "live-whip-url",
        env = "TOKULY_LIVE_WHIP_URL",
        default_value = "http://localhost:8889"
    )]
    pub whip_url: String,

    /// Where players read over WebRTC (WHEP).
    #[arg(
        id = "live_whep_url",
        long = "live-whep-url",
        env = "TOKULY_LIVE_WHEP_URL",
        default_value = "http://localhost:8889"
    )]
    pub whep_url: String,

    /// Where players read Low-Latency HLS.
    #[arg(
        id = "live_hls_url",
        long = "live-hls-url",
        env = "TOKULY_LIVE_HLS_URL",
        default_value = "http://localhost:8888"
    )]
    pub hls_url: String,

    /// The media server's playback server (rewinding a stream on air).
    #[arg(
        id = "live_playback_url",
        long = "live-playback-url",
        env = "TOKULY_LIVE_PLAYBACK_URL",
        default_value = "http://localhost:9996"
    )]
    pub playback_url: String,

    /// Directory the media server records into, as this process sees it.
    /// The media server's `recordPath` must be `<this>/%path/%s-%f`.
    #[arg(
        id = "live_recordings_dir",
        long = "live-recordings-dir",
        env = "TOKULY_LIVE_RECORDINGS_DIR",
        default_value = "/recordings"
    )]
    pub recordings_dir: String,

    /// Seconds between two looks at the media server (who is on air, who watches).
    #[arg(
        id = "live_reconcile_interval_seconds",
        long = "live-reconcile-interval-seconds",
        env = "TOKULY_LIVE_RECONCILE_INTERVAL_SECONDS",
        default_value_t = 3
    )]
    pub reconcile_interval_seconds: u64,

    /// Seconds an encoder may stay away before its stream ends.
    #[arg(
        id = "live_reconnect_grace_seconds",
        long = "live-reconnect-grace-seconds",
        env = "TOKULY_LIVE_RECONNECT_GRACE_SECONDS",
        default_value_t = 60
    )]
    pub reconnect_grace_seconds: i64,

    /// Longest a stream may stay on air, in hours.
    #[arg(id = "live_max_hours", long = "live-max-hours", env = "TOKULY_LIVE_MAX_HOURS", default_value_t = 24)]
    pub max_hours: i64,

    /// How far back viewers may rewind a stream with DVR, in seconds.
    #[arg(
        id = "live_dvr_window_seconds",
        long = "live-dvr-window-seconds",
        env = "TOKULY_LIVE_DVR_WINDOW_SECONDS",
        default_value_t = 3600
    )]
    pub dvr_window_seconds: i64,

    /// Hours after which a recording nobody claimed is deleted.
    #[arg(
        id = "live_recordings_max_age_hours",
        long = "live-recordings-max-age-hours",
        env = "TOKULY_LIVE_RECORDINGS_MAX_AGE_HOURS",
        default_value_t = 48
    )]
    pub recordings_max_age_hours: i64,

    /// Days a deleted stream and its chat are kept before they are erased,
    /// counted from the deletion: what was said can still be reported.
    #[arg(
        id = "live_deleted_retention_days",
        long = "live-deleted-retention-days",
        env = "TOKULY_LIVE_DELETED_RETENTION_DAYS",
        default_value_t = 30
    )]
    pub deleted_retention_days: i32,

    /// Streams a channel may have that are not over yet.
    #[arg(
        id = "live_max_open_streams",
        long = "live-max-open-streams",
        env = "TOKULY_LIVE_MAX_OPEN_STREAMS",
        default_value_t = 20
    )]
    pub max_open_streams: i64,

    /// How often players report that they are watching, in seconds.
    #[arg(
        id = "live_report_interval_seconds",
        long = "live-report-interval-seconds",
        env = "TOKULY_LIVE_REPORT_INTERVAL_SECONDS",
        default_value_t = 15
    )]
    pub report_interval_seconds: u32,

    /// Recipients per `live.announce` job.
    #[arg(
        id = "live_announce_batch_size",
        long = "live-announce-batch-size",
        env = "TOKULY_LIVE_ANNOUNCE_BATCH_SIZE",
        default_value_t = 500
    )]
    pub announce_batch_size: i64,

    /// `ffmpeg` and `ffprobe` used for archives and clips.
    #[arg(
        id = "live_ffmpeg_path",
        long = "live-ffmpeg-path",
        env = "TOKULY_LIVE_FFMPEG_PATH",
        default_value = "ffmpeg"
    )]
    pub ffmpeg_path: String,
    #[arg(
        id = "live_ffprobe_path",
        long = "live-ffprobe-path",
        env = "TOKULY_LIVE_FFPROBE_PATH",
        default_value = "ffprobe"
    )]
    pub ffprobe_path: String,
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self {
            mediamtx_api_url: String::new(),
            hook_secret: String::new(),
            mediamtx_api_secret: String::new(),
            rtmp_url: "rtmp://localhost:1935".to_owned(),
            srt_url: "srt://localhost:8890".to_owned(),
            whip_url: "http://localhost:8889".to_owned(),
            whep_url: "http://localhost:8889".to_owned(),
            hls_url: "http://localhost:8888".to_owned(),
            playback_url: "http://localhost:9996".to_owned(),
            recordings_dir: "/recordings".to_owned(),
            reconcile_interval_seconds: 3,
            reconnect_grace_seconds: 60,
            max_hours: 24,
            dvr_window_seconds: 3600,
            recordings_max_age_hours: 48,
            deleted_retention_days: 30,
            max_open_streams: 20,
            report_interval_seconds: 15,
            announce_batch_size: 500,
            ffmpeg_path: "ffmpeg".to_owned(),
            ffprobe_path: "ffprobe".to_owned(),
        }
    }
}

impl LiveConfig {
    /// Whether a media server is configured at all.
    pub fn media_server_configured(&self) -> bool {
        !self.mediamtx_api_url.trim().is_empty()
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.media_server_configured() {
            anyhow::ensure!(
                self.hook_secret.len() >= 16,
                "TOKULY_LIVE_HOOK_SECRET must be set (at least 16 characters) when TOKULY_MEDIAMTX_API_URL is"
            );
            anyhow::ensure!(
                self.mediamtx_api_secret.len() >= 16,
                "TOKULY_MEDIAMTX_API_SECRET must be set (at least 16 characters) when TOKULY_MEDIAMTX_API_URL is"
            );
            anyhow::ensure!(
                self.mediamtx_api_secret != self.hook_secret,
                "TOKULY_MEDIAMTX_API_SECRET and TOKULY_LIVE_HOOK_SECRET must be two different secrets"
            );
        }
        for (name, url, schemes) in [
            ("TOKULY_LIVE_RTMP_URL", &self.rtmp_url, &["rtmp://", "rtmps://"][..]),
            ("TOKULY_LIVE_SRT_URL", &self.srt_url, &["srt://"][..]),
            ("TOKULY_LIVE_WHIP_URL", &self.whip_url, &["http://", "https://"][..]),
            ("TOKULY_LIVE_WHEP_URL", &self.whep_url, &["http://", "https://"][..]),
            ("TOKULY_LIVE_HLS_URL", &self.hls_url, &["http://", "https://"][..]),
            ("TOKULY_LIVE_PLAYBACK_URL", &self.playback_url, &["http://", "https://"][..]),
        ] {
            anyhow::ensure!(
                schemes.iter().any(|scheme| url.starts_with(scheme)),
                "{name} must start with {}",
                schemes.join(" or ")
            );
        }
        anyhow::ensure!(
            self.reconcile_interval_seconds >= 1,
            "TOKULY_LIVE_RECONCILE_INTERVAL_SECONDS must be at least 1"
        );
        anyhow::ensure!(self.reconnect_grace_seconds >= 0, "TOKULY_LIVE_RECONNECT_GRACE_SECONDS cannot be negative");
        anyhow::ensure!(self.max_hours >= 1, "TOKULY_LIVE_MAX_HOURS must be at least 1");
        anyhow::ensure!(self.dvr_window_seconds >= 60, "TOKULY_LIVE_DVR_WINDOW_SECONDS must be at least 60");
        anyhow::ensure!(self.recordings_max_age_hours >= 1, "TOKULY_LIVE_RECORDINGS_MAX_AGE_HOURS must be at least 1");
        anyhow::ensure!(self.deleted_retention_days >= 1, "TOKULY_LIVE_DELETED_RETENTION_DAYS must be at least 1");
        anyhow::ensure!(self.max_open_streams >= 1, "TOKULY_LIVE_MAX_OPEN_STREAMS must be at least 1");
        anyhow::ensure!(self.report_interval_seconds >= 5, "TOKULY_LIVE_REPORT_INTERVAL_SECONDS must be at least 5");
        anyhow::ensure!(self.announce_batch_size >= 1, "TOKULY_LIVE_ANNOUNCE_BATCH_SIZE must be at least 1");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_leave_the_media_server_unconfigured() {
        let cfg = LiveConfig::default();
        cfg.validate().unwrap();
        assert!(!cfg.media_server_configured());
    }

    #[test]
    fn a_media_server_needs_a_real_hook_secret() {
        let mut cfg = LiveConfig { mediamtx_api_url: "http://mediamtx:9997".into(), ..Default::default() };
        assert!(cfg.validate().is_err());
        cfg.hook_secret = "0123456789abcdef".into();
        assert!(cfg.validate().is_err(), "the control API needs its own secret");
        cfg.mediamtx_api_secret = "0123456789abcdef".into();
        assert!(cfg.validate().is_err(), "and not the same one");
        cfg.mediamtx_api_secret = "fedcba9876543210".into();
        cfg.validate().unwrap();
        cfg.hls_url = "localhost:8888".into();
        assert!(cfg.validate().is_err());
    }
}
