//! Shorts-specific settings.
//!
//! The server flattens every domain's settings into one command line, where
//! both the flag and clap's internal argument ID must be unique, so every
//! argument carries a `shorts` prefix in both.
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct ShortsConfig {
    /// Longest short, in seconds. An upload that turns out longer fails.
    #[arg(
        id = "shorts_max_duration_seconds",
        long = "shorts-max-duration-seconds",
        env = "TOKULY_SHORTS_MAX_DURATION_SECONDS",
        default_value_t = 180.0
    )]
    pub max_duration_seconds: f64,

    /// Widest video accepted, as width divided by height. Portrait and
    /// square videos are below 1; the default admits landscape up to 16:9,
    /// which players centre-crop.
    #[arg(
        id = "shorts_max_aspect_ratio",
        long = "shorts-max-aspect-ratio",
        env = "TOKULY_SHORTS_MAX_ASPECT_RATIO",
        default_value_t = 1.78
    )]
    pub max_aspect_ratio: f64,

    /// Milliseconds of playback after which a stay on a short counts as a
    /// view (the whole short when it is shorter than this).
    #[arg(
        id = "shorts_min_view_ms",
        long = "shorts-min-view-ms",
        env = "TOKULY_SHORTS_MIN_VIEW_MS",
        default_value_t = 1000
    )]
    pub min_view_ms: i64,

    /// Seconds during which further views of a short by the same viewer
    /// (account, or network address when anonymous) are not counted again.
    #[arg(
        id = "shorts_view_dedupe_seconds",
        long = "shorts-view-dedupe-seconds",
        env = "TOKULY_SHORTS_VIEW_DEDUPE_SECONDS",
        default_value_t = 21_600
    )]
    pub view_dedupe_seconds: u64,

    /// Shorts remembered as seen per viewer; older ones may come round again.
    #[arg(id = "shorts_seen_cap", long = "shorts-seen-cap", env = "TOKULY_SHORTS_SEEN_CAP", default_value_t = 2000)]
    pub seen_cap: i64,

    /// Days without any playback after which a viewer's seen list is dropped.
    #[arg(
        id = "shorts_seen_ttl_days",
        long = "shorts-seen-ttl-days",
        env = "TOKULY_SHORTS_SEEN_TTL_DAYS",
        default_value_t = 30
    )]
    pub seen_ttl_days: u64,

    /// Seconds between flushes of the buffered playback counters to PostgreSQL.
    #[arg(
        id = "shorts_flush_interval_seconds",
        long = "shorts-flush-interval-seconds",
        env = "TOKULY_SHORTS_FLUSH_INTERVAL_SECONDS",
        default_value_t = 5
    )]
    pub flush_interval_seconds: u64,

    /// Seconds between runs of the ranking worker (scores, trending hashtags).
    #[arg(
        id = "shorts_ranking_interval_seconds",
        long = "shorts-ranking-interval-seconds",
        env = "TOKULY_SHORTS_RANKING_INTERVAL_SECONDS",
        default_value_t = 60
    )]
    pub ranking_interval_seconds: u64,

    /// Hours after which an engagement signal has lost half its weight in
    /// the ranking; a short's freshness halves every three times this.
    #[arg(
        id = "shorts_ranking_half_life_hours",
        long = "shorts-ranking-half-life-hours",
        env = "TOKULY_SHORTS_RANKING_HALF_LIFE_HOURS",
        default_value_t = 24.0
    )]
    pub ranking_half_life_hours: f64,

    /// Shorts ranked and kept per for-you batch.
    #[arg(
        id = "shorts_feed_batch_size",
        long = "shorts-feed-batch-size",
        env = "TOKULY_SHORTS_FEED_BATCH_SIZE",
        default_value_t = 200
    )]
    pub feed_batch_size: usize,

    /// Seconds a for-you batch stays readable through its cursor.
    #[arg(
        id = "shorts_feed_batch_ttl_seconds",
        long = "shorts-feed-batch-ttl-seconds",
        env = "TOKULY_SHORTS_FEED_BATCH_TTL_SECONDS",
        default_value_t = 1800
    )]
    pub feed_batch_ttl_seconds: u64,

    /// Subscribers notified per `shorts.announce` job when a channel posts.
    #[arg(
        id = "shorts_announce_batch_size",
        long = "shorts-announce-batch-size",
        env = "TOKULY_SHORTS_ANNOUNCE_BATCH_SIZE",
        default_value_t = 500
    )]
    pub announce_batch_size: i64,

    /// Shorts of one channel whose publication is announced to its
    /// subscribers per 24 hours. Further shorts are published all the same;
    /// subscribers find them in their following feed.
    #[arg(
        id = "shorts_announcements_per_day",
        long = "shorts-announcements-per-day",
        env = "TOKULY_SHORTS_ANNOUNCEMENTS_PER_DAY",
        default_value_t = 3
    )]
    pub announcements_per_day: i64,

    /// Days a short that had been reported stays reviewable by moderators
    /// after its owner deleted it, counted from the deletion.
    #[arg(
        id = "shorts_reported_retention_days",
        long = "shorts-reported-retention-days",
        env = "TOKULY_SHORTS_REPORTED_RETENTION_DAYS",
        default_value_t = 30
    )]
    pub reported_retention_days: i32,

    /// Hours an account must have existed before what it does (views,
    /// watch-through, likes, comments, saves, shares) moves a short's
    /// ranking. Younger accounts, like visitors, only add to the view count.
    #[arg(
        id = "shorts_signal_min_account_age_hours",
        long = "shorts-signal-min-account-age-hours",
        env = "TOKULY_SHORTS_SIGNAL_MIN_ACCOUNT_AGE_HOURS",
        default_value_t = 24
    )]
    pub signal_min_account_age_hours: i32,

    /// Shorts one account may keep saved.
    #[arg(id = "shorts_max_saves", long = "shorts-max-saves", env = "TOKULY_SHORTS_MAX_SAVES", default_value_t = 5000)]
    pub max_saves: i64,

    /// The ffmpeg binary used to cut cover frames.
    #[arg(
        id = "shorts_ffmpeg_path",
        long = "shorts-ffmpeg-path",
        env = "TOKULY_SHORTS_FFMPEG_PATH",
        default_value = "ffmpeg"
    )]
    pub ffmpeg_path: String,
}

impl Default for ShortsConfig {
    fn default() -> Self {
        Self {
            max_duration_seconds: 180.0,
            max_aspect_ratio: 1.78,
            min_view_ms: 1000,
            view_dedupe_seconds: 21_600,
            seen_cap: 2000,
            seen_ttl_days: 30,
            flush_interval_seconds: 5,
            ranking_interval_seconds: 60,
            ranking_half_life_hours: 24.0,
            feed_batch_size: 200,
            feed_batch_ttl_seconds: 1800,
            announce_batch_size: 500,
            announcements_per_day: 3,
            reported_retention_days: 30,
            signal_min_account_age_hours: 24,
            max_saves: 5000,
            ffmpeg_path: "ffmpeg".to_owned(),
        }
    }
}

impl ShortsConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.max_duration_seconds.is_finite() && self.max_duration_seconds > 0.0,
            "TOKULY_SHORTS_MAX_DURATION_SECONDS must be positive"
        );
        anyhow::ensure!(
            self.max_aspect_ratio.is_finite() && self.max_aspect_ratio >= 1.0,
            "TOKULY_SHORTS_MAX_ASPECT_RATIO must be at least 1 (square)"
        );
        anyhow::ensure!(self.min_view_ms >= 0, "TOKULY_SHORTS_MIN_VIEW_MS must not be negative");
        anyhow::ensure!(self.seen_cap >= 1, "TOKULY_SHORTS_SEEN_CAP must be at least 1");
        anyhow::ensure!(
            self.ranking_half_life_hours.is_finite() && self.ranking_half_life_hours > 0.0,
            "TOKULY_SHORTS_RANKING_HALF_LIFE_HOURS must be positive"
        );
        anyhow::ensure!(self.feed_batch_size >= 30, "TOKULY_SHORTS_FEED_BATCH_SIZE must be at least 30");
        anyhow::ensure!(self.feed_batch_ttl_seconds >= 60, "TOKULY_SHORTS_FEED_BATCH_TTL_SECONDS must be at least 60");
        anyhow::ensure!(self.announce_batch_size >= 1, "TOKULY_SHORTS_ANNOUNCE_BATCH_SIZE must be at least 1");
        anyhow::ensure!(self.announcements_per_day >= 0, "TOKULY_SHORTS_ANNOUNCEMENTS_PER_DAY must not be negative");
        anyhow::ensure!(
            self.reported_retention_days >= 0,
            "TOKULY_SHORTS_REPORTED_RETENTION_DAYS must not be negative"
        );
        anyhow::ensure!(self.max_saves >= 1, "TOKULY_SHORTS_MAX_SAVES must be at least 1");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_nonsense_is_refused() {
        ShortsConfig::default().validate().unwrap();
        assert!(ShortsConfig { max_duration_seconds: 0.0, ..Default::default() }.validate().is_err());
        assert!(ShortsConfig { max_aspect_ratio: 0.5, ..Default::default() }.validate().is_err());
        assert!(ShortsConfig { ranking_half_life_hours: f64::NAN, ..Default::default() }.validate().is_err());
        assert!(ShortsConfig { feed_batch_size: 3, ..Default::default() }.validate().is_err());
    }
}
