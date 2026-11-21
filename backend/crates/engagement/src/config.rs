//! Engagement-specific settings.
//!
//! The server flattens every domain's settings into one command line, so
//! each argument carries an `engagement` prefix in its ID and its flag.
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct EngagementConfig {
    /// Comments (and replies) one user may write per minute.
    #[arg(
        id = "engagement_comments_per_minute",
        long = "engagement-comments-per-minute",
        env = "TOKULY_ENGAGEMENT_COMMENTS_PER_MINUTE",
        default_value_t = 10
    )]
    pub comments_per_minute: u32,

    /// Comments (and replies) one user may write per hour.
    #[arg(
        id = "engagement_comments_per_hour",
        long = "engagement-comments-per-hour",
        env = "TOKULY_ENGAGEMENT_COMMENTS_PER_HOUR",
        default_value_t = 120
    )]
    pub comments_per_hour: u32,

    /// Reports one user may file per hour.
    #[arg(
        id = "engagement_reports_per_hour",
        long = "engagement-reports-per-hour",
        env = "TOKULY_ENGAGEMENT_REPORTS_PER_HOUR",
        default_value_t = 20
    )]
    pub reports_per_hour: u32,

    /// Days after which an unanswered report no longer stops its reporter
    /// from reporting the same subject again.
    #[arg(
        id = "engagement_report_repeat_days",
        long = "engagement-report-repeat-days",
        env = "TOKULY_ENGAGEMENT_REPORT_REPEAT_DAYS",
        default_value_t = 7
    )]
    pub report_repeat_days: i32,

    /// Bookmark collections one user may have, counting the default one.
    #[arg(
        id = "engagement_max_collections",
        long = "engagement-max-collections",
        env = "TOKULY_ENGAGEMENT_MAX_COLLECTIONS",
        default_value_t = 100
    )]
    pub max_collections: i64,

    /// Subjects one bookmark collection may hold.
    #[arg(
        id = "engagement_max_bookmarks",
        long = "engagement-max-bookmarks",
        env = "TOKULY_ENGAGEMENT_MAX_BOOKMARKS",
        default_value_t = 5000
    )]
    pub max_bookmarks: i32,
}

impl Default for EngagementConfig {
    fn default() -> Self {
        Self {
            comments_per_minute: 10,
            comments_per_hour: 120,
            reports_per_hour: 20,
            report_repeat_days: 7,
            max_collections: 100,
            max_bookmarks: 5000,
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        engagement: EngagementConfig,
    }

    #[test]
    fn command_line_defaults_match_default() {
        use clap::CommandFactory as _;
        Cli::command().debug_assert();
        let parsed = Cli::parse_from(["tokuly"]).engagement;
        let default = EngagementConfig::default();
        assert_eq!(
            (
                parsed.comments_per_minute,
                parsed.comments_per_hour,
                parsed.reports_per_hour,
                parsed.report_repeat_days,
                parsed.max_collections,
                parsed.max_bookmarks
            ),
            (
                default.comments_per_minute,
                default.comments_per_hour,
                default.reports_per_hour,
                default.report_repeat_days,
                default.max_collections,
                default.max_bookmarks
            )
        );
    }

    #[test]
    fn flags_are_prefixed() {
        let parsed =
            Cli::parse_from(["tokuly", "--engagement-comments-per-minute", "3", "--engagement-max-bookmarks", "9"])
                .engagement;
        assert_eq!((parsed.comments_per_minute, parsed.max_bookmarks), (3, 9));
    }
}
