//! Video-specific settings.
//!
//! The server flattens every domain's settings into one command line, where
//! both the flag and clap's internal argument ID must be unique, so every
//! argument carries a `video` prefix in both.
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct VideoConfig {
    /// Channels one account may own.
    #[arg(
        id = "video_max_channels",
        long = "video-max-channels",
        env = "TOKULY_VIDEO_MAX_CHANNELS",
        default_value_t = 5
    )]
    pub max_channels: i64,

    /// Seconds of watching after which a playback session counts as a view
    /// (half the duration for videos shorter than twice this).
    #[arg(
        id = "video_min_view_seconds",
        long = "video-min-view-seconds",
        env = "TOKULY_VIDEO_MIN_VIEW_SECONDS",
        default_value_t = 10.0
    )]
    pub min_view_seconds: f64,

    /// Seconds during which further views of a video by the same viewer
    /// (account, or network address when anonymous) are not counted again.
    #[arg(
        id = "video_view_dedupe_seconds",
        long = "video-view-dedupe-seconds",
        env = "TOKULY_VIDEO_VIEW_DEDUPE_SECONDS",
        default_value_t = 21_600
    )]
    pub view_dedupe_seconds: u64,

    /// How often clients are asked to report playback progress, in seconds.
    #[arg(
        id = "video_report_interval_seconds",
        long = "video-report-interval-seconds",
        env = "TOKULY_VIDEO_REPORT_INTERVAL_SECONDS",
        default_value_t = 10
    )]
    pub report_interval_seconds: u32,

    /// Seconds between runs of the publishing worker (scheduled videos and
    /// uploads whose processing result was missed).
    #[arg(
        id = "video_publish_interval_seconds",
        long = "video-publish-interval-seconds",
        env = "TOKULY_VIDEO_PUBLISH_INTERVAL_SECONDS",
        default_value_t = 15
    )]
    pub publish_interval_seconds: u64,

    /// Seconds between refreshes of the trending scores.
    #[arg(
        id = "video_trending_interval_seconds",
        long = "video-trending-interval-seconds",
        env = "TOKULY_VIDEO_TRENDING_INTERVAL_SECONDS",
        default_value_t = 300
    )]
    pub trending_interval_seconds: u64,

    /// Hours after which a view or like has lost half its weight in the
    /// trending score.
    #[arg(
        id = "video_trending_half_life_hours",
        long = "video-trending-half-life-hours",
        env = "TOKULY_VIDEO_TRENDING_HALF_LIFE_HOURS",
        default_value_t = 24.0
    )]
    pub trending_half_life_hours: f64,

    /// Custom playlists one account may have.
    #[arg(
        id = "video_max_playlists",
        long = "video-max-playlists",
        env = "TOKULY_VIDEO_MAX_PLAYLISTS",
        default_value_t = 200
    )]
    pub max_playlists: i64,

    /// Videos one playlist may hold.
    #[arg(
        id = "video_max_playlist_items",
        long = "video-max-playlist-items",
        env = "TOKULY_VIDEO_MAX_PLAYLIST_ITEMS",
        default_value_t = 5000
    )]
    pub max_playlist_items: i64,

    /// Subscribers notified per `video.announce` job when a channel publishes.
    #[arg(
        id = "video_announce_batch_size",
        long = "video-announce-batch-size",
        env = "TOKULY_VIDEO_ANNOUNCE_BATCH_SIZE",
        default_value_t = 500
    )]
    pub announce_batch_size: i64,

    /// Members a watch party takes, guests included.
    #[arg(
        id = "video_party_max_members",
        long = "video-party-max-members",
        env = "TOKULY_VIDEO_PARTY_MAX_MEMBERS",
        default_value_t = 100
    )]
    pub party_max_members: i64,

    /// Open watch parties one account may host at a time.
    #[arg(
        id = "video_party_max_hosted",
        long = "video-party-max-hosted",
        env = "TOKULY_VIDEO_PARTY_MAX_HOSTED",
        default_value_t = 3
    )]
    pub party_max_hosted: i64,

    /// Hours after which a watch party ends on its own.
    #[arg(
        id = "video_party_max_hours",
        long = "video-party-max-hours",
        env = "TOKULY_VIDEO_PARTY_MAX_HOURS",
        default_value_t = 12
    )]
    pub party_max_hours: i64,

    /// Seconds without a heartbeat after which a watch party member is dropped.
    #[arg(
        id = "video_party_member_timeout_seconds",
        long = "video-party-member-timeout-seconds",
        env = "TOKULY_VIDEO_PARTY_MEMBER_TIMEOUT_SECONDS",
        default_value_t = 60
    )]
    pub party_member_timeout_seconds: u64,

    /// Distinct viewers a video or channel needs within a report's range
    /// before analytics break its audience down (retention curve, traffic
    /// sources): below that, the breakdown describes what one or two known
    /// people did.
    #[arg(
        id = "video_analytics_min_audience",
        long = "video-analytics-min-audience",
        env = "TOKULY_VIDEO_ANALYTICS_MIN_AUDIENCE",
        default_value_t = 5
    )]
    pub analytics_min_audience: i64,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            max_channels: 5,
            min_view_seconds: 10.0,
            view_dedupe_seconds: 21_600,
            report_interval_seconds: 10,
            publish_interval_seconds: 15,
            trending_interval_seconds: 300,
            trending_half_life_hours: 24.0,
            max_playlists: 200,
            max_playlist_items: 5000,
            announce_batch_size: 500,
            party_max_members: 100,
            party_max_hosted: 3,
            party_max_hours: 12,
            party_member_timeout_seconds: 60,
            analytics_min_audience: 5,
        }
    }
}

impl VideoConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.max_channels > 0, "TOKULY_VIDEO_MAX_CHANNELS must be positive");
        anyhow::ensure!(
            self.min_view_seconds.is_finite() && self.min_view_seconds >= 0.0,
            "TOKULY_VIDEO_MIN_VIEW_SECONDS must be a non-negative number"
        );
        anyhow::ensure!(self.report_interval_seconds > 0, "TOKULY_VIDEO_REPORT_INTERVAL_SECONDS must be positive");
        anyhow::ensure!(
            self.trending_half_life_hours.is_finite() && self.trending_half_life_hours > 0.0,
            "TOKULY_VIDEO_TRENDING_HALF_LIFE_HOURS must be positive"
        );
        anyhow::ensure!(self.max_playlists > 0 && self.max_playlist_items > 0, "the playlist limits must be positive");
        anyhow::ensure!(self.announce_batch_size > 0, "TOKULY_VIDEO_ANNOUNCE_BATCH_SIZE must be positive");
        anyhow::ensure!(
            self.party_max_members >= 2 && self.party_max_hosted > 0 && self.party_max_hours > 0,
            "the watch party limits must be positive (and a party needs room for two)"
        );
        anyhow::ensure!(
            self.party_member_timeout_seconds >= 10,
            "TOKULY_VIDEO_PARTY_MEMBER_TIMEOUT_SECONDS must be at least 10"
        );
        anyhow::ensure!(self.analytics_min_audience >= 1, "TOKULY_VIDEO_ANALYTICS_MIN_AUDIENCE must be at least 1");
        Ok(())
    }
}

impl VideoConfig {
    /// The settings the staff console (「サーバー設定」) may change, by
    /// `tokuly.toml` key. Everything else is set in tokuly.toml or the
    /// environment only: addresses, credentials, egress guards, paths,
    /// money and privacy floors never belong here (backend/README.md,
    /// "Declaring a setting").
    pub const CONSOLE_EDITABLE: &'static [&'static str] = &[
        "max_channels",
        "min_view_seconds",
        "view_dedupe_seconds",
        "report_interval_seconds",
        "publish_interval_seconds",
        "trending_interval_seconds",
        "trending_half_life_hours",
        "max_playlists",
        "max_playlist_items",
        "announce_batch_size",
        "party_max_members",
        "party_max_hosted",
        "party_max_hours",
        "party_member_timeout_seconds",
    ];
}
