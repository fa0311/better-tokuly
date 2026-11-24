//! Notes-specific settings.
//!
//! The server flattens every domain's settings into one command line, where
//! both the flag and clap's argument ID must be unique, so every argument
//! carries a `notes` prefix in both.
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct NotesConfig {
    /// Hours a story stays visible to friends before it moves to the archive.
    #[arg(
        id = "notes_story_ttl_hours",
        long = "notes-story-ttl-hours",
        env = "TOKULY_NOTES_STORY_TTL_HOURS",
        default_value_t = 24
    )]
    pub story_ttl_hours: i64,

    /// Notes (replies and quotes included) one user may post per minute.
    #[arg(
        id = "notes_posts_per_minute",
        long = "notes-posts-per-minute",
        env = "TOKULY_NOTES_POSTS_PER_MINUTE",
        default_value_t = 30
    )]
    pub posts_per_minute: u32,

    /// Stories one user may post per hour.
    #[arg(
        id = "notes_stories_per_hour",
        long = "notes-stories-per-hour",
        env = "TOKULY_NOTES_STORIES_PER_HOUR",
        default_value_t = 30
    )]
    pub stories_per_hour: u32,

    /// Hours within which repeated views of a note by the same viewer count once.
    #[arg(
        id = "notes_view_window_hours",
        long = "notes-view-window-hours",
        env = "TOKULY_NOTES_VIEW_WINDOW_HOURS",
        default_value_t = 6
    )]
    pub view_window_hours: u64,

    /// Seconds between recomputations of the trending hashtags.
    #[arg(
        id = "notes_trend_interval_seconds",
        long = "notes-trend-interval-seconds",
        env = "TOKULY_NOTES_TREND_INTERVAL_SECONDS",
        default_value_t = 300
    )]
    pub trend_interval_seconds: u64,

    /// Hours of public notes the trending ranking looks at.
    #[arg(
        id = "notes_trend_window_hours",
        long = "notes-trend-window-hours",
        env = "TOKULY_NOTES_TREND_WINDOW_HOURS",
        default_value_t = 24
    )]
    pub trend_window_hours: i64,

    /// Different people who must have used a hashtag within the window for it to trend.
    #[arg(
        id = "notes_trend_min_authors",
        long = "notes-trend-min-authors",
        env = "TOKULY_NOTES_TREND_MIN_AUTHORS",
        default_value_t = 2
    )]
    pub trend_min_authors: i64,

    /// Hours an account must exist before its posts and reactions count towards trends.
    #[arg(
        id = "notes_trend_min_account_age_hours",
        long = "notes-trend-min-account-age-hours",
        env = "TOKULY_NOTES_TREND_MIN_ACCOUNT_AGE_HOURS",
        default_value_t = 72
    )]
    pub trend_min_account_age_hours: i64,

    /// Accounts younger than this many days that joined on the same day count as one person in trends.
    #[arg(
        id = "notes_trend_new_account_days",
        long = "notes-trend-new-account-days",
        env = "TOKULY_NOTES_TREND_NEW_ACCOUNT_DAYS",
        default_value_t = 30
    )]
    pub trend_new_account_days: i32,

    /// Seconds between passes of the poll closing worker and the scheduler of notes.
    #[arg(
        id = "notes_schedule_interval_seconds",
        long = "notes-schedule-interval-seconds",
        env = "TOKULY_NOTES_SCHEDULE_INTERVAL_SECONDS",
        default_value_t = 30
    )]
    pub schedule_interval_seconds: u64,

    /// Seconds between passes of the story expiry worker.
    #[arg(
        id = "notes_story_expiry_interval_seconds",
        long = "notes-story-expiry-interval-seconds",
        env = "TOKULY_NOTES_STORY_EXPIRY_INTERVAL_SECONDS",
        default_value_t = 60
    )]
    pub story_expiry_interval_seconds: u64,
}

impl Default for NotesConfig {
    fn default() -> Self {
        Self {
            story_ttl_hours: 24,
            posts_per_minute: 30,
            stories_per_hour: 30,
            view_window_hours: 6,
            trend_interval_seconds: 300,
            trend_window_hours: 24,
            trend_min_authors: 2,
            trend_min_account_age_hours: 72,
            trend_new_account_days: 30,
            schedule_interval_seconds: 30,
            story_expiry_interval_seconds: 60,
        }
    }
}

impl NotesConfig {
    /// Refuses settings the domain cannot run with.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.story_ttl_hours > 0, "TOKULY_NOTES_STORY_TTL_HOURS must be positive");
        anyhow::ensure!(self.posts_per_minute > 0, "TOKULY_NOTES_POSTS_PER_MINUTE must be positive");
        anyhow::ensure!(self.stories_per_hour > 0, "TOKULY_NOTES_STORIES_PER_HOUR must be positive");
        anyhow::ensure!(self.view_window_hours > 0, "TOKULY_NOTES_VIEW_WINDOW_HOURS must be positive");
        anyhow::ensure!(self.trend_interval_seconds > 0, "TOKULY_NOTES_TREND_INTERVAL_SECONDS must be positive");
        anyhow::ensure!(self.trend_window_hours > 0, "TOKULY_NOTES_TREND_WINDOW_HOURS must be positive");
        anyhow::ensure!(self.trend_min_authors > 0, "TOKULY_NOTES_TREND_MIN_AUTHORS must be positive");
        anyhow::ensure!(
            self.trend_min_account_age_hours >= 0,
            "TOKULY_NOTES_TREND_MIN_ACCOUNT_AGE_HOURS must not be negative"
        );
        anyhow::ensure!(self.trend_new_account_days >= 0, "TOKULY_NOTES_TREND_NEW_ACCOUNT_DAYS must not be negative");
        anyhow::ensure!(self.schedule_interval_seconds > 0, "TOKULY_NOTES_SCHEDULE_INTERVAL_SECONDS must be positive");
        anyhow::ensure!(
            self.story_expiry_interval_seconds > 0,
            "TOKULY_NOTES_STORY_EXPIRY_INTERVAL_SECONDS must be positive"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        notes: NotesConfig,
    }

    #[test]
    fn command_line_defaults_match_default_impl() {
        let parsed = Cli::parse_from(["test"]).notes;
        let default = NotesConfig::default();
        assert_eq!(format!("{parsed:?}"), format!("{default:?}"));
        assert!(default.validate().is_ok());
    }

    #[test]
    fn zero_settings_are_refused() {
        assert!(NotesConfig { story_ttl_hours: 0, ..Default::default() }.validate().is_err());
        assert!(NotesConfig { trend_min_authors: 0, ..Default::default() }.validate().is_err());
    }
}
