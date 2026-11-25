//! Community-specific settings.
//!
//! The server flattens every domain's settings into one command line, so
//! each argument carries a `community` prefix in its ID and its flag.
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct CommunityConfig {
    /// Communities one account may own (the creation limit).
    #[arg(
        id = "community_max_owned",
        long = "community-max-owned",
        env = "TOKULY_COMMUNITY_MAX_OWNED",
        default_value_t = 5
    )]
    pub max_owned: u32,

    /// Communities one account may be a member of.
    #[arg(
        id = "community_max_joined",
        long = "community-max-joined",
        env = "TOKULY_COMMUNITY_MAX_JOINED",
        default_value_t = 200
    )]
    pub max_joined: u32,

    /// Members one community may have (the member cap).
    #[arg(
        id = "community_max_members",
        long = "community-max-members",
        env = "TOKULY_COMMUNITY_MAX_MEMBERS",
        default_value_t = 5000
    )]
    pub max_members: u32,

    /// Channels one community may have.
    #[arg(
        id = "community_max_channels",
        long = "community-max-channels",
        env = "TOKULY_COMMUNITY_MAX_CHANNELS",
        default_value_t = 50
    )]
    pub max_channels: u32,

    /// Reports by different members that hide a post until the staff
    /// reviewed it. A community may choose its own number.
    #[arg(
        id = "community_auto_hide_reports",
        long = "community-auto-hide-reports",
        env = "TOKULY_COMMUNITY_AUTO_HIDE_REPORTS",
        default_value_t = 3
    )]
    pub auto_hide_reports: u32,

    /// Threads and replies one member may write per minute.
    #[arg(
        id = "community_posts_per_minute",
        long = "community-posts-per-minute",
        env = "TOKULY_COMMUNITY_POSTS_PER_MINUTE",
        default_value_t = 10
    )]
    pub posts_per_minute: u32,

    /// Threads and replies one member may write per hour.
    #[arg(
        id = "community_posts_per_hour",
        long = "community-posts-per-hour",
        env = "TOKULY_COMMUNITY_POSTS_PER_HOUR",
        default_value_t = 120
    )]
    pub posts_per_hour: u32,

    /// Members who could have written a post before an anonymous channel
    /// takes posts: "anonymous" among two people is not anonymous.
    #[arg(
        id = "community_anonymous_min_members",
        long = "community-anonymous-min-members",
        env = "TOKULY_COMMUNITY_ANONYMOUS_MIN_MEMBERS",
        default_value_t = 5
    )]
    pub anonymous_min_members: u32,

    /// Days a removed thread, reply or event keeps its content and uploads,
    /// counted from its removal, so that it can still be reported and
    /// acted on. After that it is purged.
    #[arg(
        id = "community_removed_retention_days",
        long = "community-removed-retention-days",
        env = "TOKULY_COMMUNITY_REMOVED_RETENTION_DAYS",
        default_value_t = 30
    )]
    pub removed_retention_days: u32,

    /// Seconds between two passes of the worker that recomputes the "hot"
    /// order of threads.
    #[arg(
        id = "community_hot_interval_seconds",
        long = "community-hot-interval-seconds",
        env = "TOKULY_COMMUNITY_HOT_INTERVAL_SECONDS",
        default_value_t = 300
    )]
    pub hot_interval_seconds: u64,

    /// Seconds between two passes of the worker that recomputes which
    /// communities are trending.
    #[arg(
        id = "community_trend_interval_seconds",
        long = "community-trend-interval-seconds",
        env = "TOKULY_COMMUNITY_TREND_INTERVAL_SECONDS",
        default_value_t = 900
    )]
    pub trend_interval_seconds: u64,

    /// Seconds an event check-in code is shown before it changes.
    #[arg(
        id = "community_check_in_code_seconds",
        long = "community-check-in-code-seconds",
        env = "TOKULY_COMMUNITY_CHECK_IN_CODE_SECONDS",
        default_value_t = 60
    )]
    pub check_in_code_seconds: u64,
}

impl Default for CommunityConfig {
    fn default() -> Self {
        Self {
            max_owned: 5,
            max_joined: 200,
            max_members: 5000,
            max_channels: 50,
            auto_hide_reports: 3,
            posts_per_minute: 10,
            posts_per_hour: 120,
            anonymous_min_members: 5,
            removed_retention_days: 30,
            hot_interval_seconds: 300,
            trend_interval_seconds: 900,
            check_in_code_seconds: 60,
        }
    }
}

impl CommunityConfig {
    /// Rejects settings that would switch a feature off by accident.
    pub fn validate(&self) -> anyhow::Result<()> {
        for (name, value) in [
            ("TOKULY_COMMUNITY_MAX_OWNED", self.max_owned as u64),
            ("TOKULY_COMMUNITY_MAX_JOINED", self.max_joined as u64),
            ("TOKULY_COMMUNITY_MAX_MEMBERS", self.max_members as u64),
            ("TOKULY_COMMUNITY_MAX_CHANNELS", self.max_channels as u64),
            ("TOKULY_COMMUNITY_POSTS_PER_MINUTE", self.posts_per_minute as u64),
            ("TOKULY_COMMUNITY_POSTS_PER_HOUR", self.posts_per_hour as u64),
            ("TOKULY_COMMUNITY_REMOVED_RETENTION_DAYS", self.removed_retention_days as u64),
            ("TOKULY_COMMUNITY_HOT_INTERVAL_SECONDS", self.hot_interval_seconds),
            ("TOKULY_COMMUNITY_TREND_INTERVAL_SECONDS", self.trend_interval_seconds),
        ] {
            anyhow::ensure!(value > 0, "{name} must be positive");
        }
        anyhow::ensure!(
            (2..=50).contains(&self.auto_hide_reports),
            "TOKULY_COMMUNITY_AUTO_HIDE_REPORTS must be between 2 and 50, got {}",
            self.auto_hide_reports
        );
        anyhow::ensure!(
            self.anonymous_min_members >= 2,
            "TOKULY_COMMUNITY_ANONYMOUS_MIN_MEMBERS must be at least 2, got {}",
            self.anonymous_min_members
        );
        anyhow::ensure!(
            (15..=600).contains(&self.check_in_code_seconds),
            "TOKULY_COMMUNITY_CHECK_IN_CODE_SECONDS must be between 15 and 600, got {}",
            self.check_in_code_seconds
        );
        Ok(())
    }
}

impl CommunityConfig {
    /// The settings the staff console (「サーバー設定」) may change, by
    /// `tokuly.toml` key. Everything else is set in tokuly.toml or the
    /// environment only: addresses, credentials, egress guards, paths,
    /// money and privacy floors never belong here (backend/README.md,
    /// "Declaring a setting").
    pub const CONSOLE_EDITABLE: &'static [&'static str] = &[
        "max_owned",
        "max_joined",
        "max_members",
        "max_channels",
        "auto_hide_reports",
        "posts_per_minute",
        "posts_per_hour",
        "removed_retention_days",
        "hot_interval_seconds",
        "trend_interval_seconds",
        "check_in_code_seconds",
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_nonsense_is_rejected() {
        assert!(CommunityConfig::default().validate().is_ok());
        assert!(CommunityConfig { max_members: 0, ..Default::default() }.validate().is_err());
        assert!(CommunityConfig { auto_hide_reports: 1, ..Default::default() }.validate().is_err());
        assert!(CommunityConfig { check_in_code_seconds: 5, ..Default::default() }.validate().is_err());
        assert!(CommunityConfig { anonymous_min_members: 1, ..Default::default() }.validate().is_err());
        assert!(CommunityConfig { removed_retention_days: 0, ..Default::default() }.validate().is_err());
    }
}
