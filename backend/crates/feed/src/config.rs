//! Feed-specific settings.
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct FeedConfig {
    /// Days an activity entry is kept before the retention sweep deletes it.
    /// 0 keeps entries forever.
    #[arg(
        id = "feed_activity_retention_days",
        long = "feed-activity-retention-days",
        env = "TOKULY_FEED_ACTIVITY_RETENTION_DAYS",
        default_value_t = 400
    )]
    pub activity_retention_days: u32,

    /// How far back map stories reach, in hours.
    #[arg(
        id = "feed_map_story_hours",
        long = "feed-map-story-hours",
        env = "TOKULY_FEED_MAP_STORY_HOURS",
        default_value_t = 48
    )]
    pub map_story_hours: u32,

    /// Entries of the same kind by the same person that are no further apart
    /// than this many minutes become one card in the friends' and home feeds.
    #[arg(
        id = "feed_burst_minutes",
        long = "feed-burst-minutes",
        env = "TOKULY_FEED_BURST_MINUTES",
        default_value_t = 60
    )]
    pub burst_minutes: u32,

    /// Seconds the recap of a period that is still running stays cached.
    /// Finished periods are cached for six hours.
    #[arg(
        id = "feed_recap_cache_seconds",
        long = "feed-recap-cache-seconds",
        env = "TOKULY_FEED_RECAP_CACHE_SECONDS",
        default_value_t = 300
    )]
    pub recap_cache_seconds: u64,

    /// Entries a user needs in a month to be told that their recap is ready.
    #[arg(
        id = "feed_recap_min_activities",
        long = "feed-recap-min-activities",
        env = "TOKULY_FEED_RECAP_MIN_ACTIVITIES",
        default_value_t = 5
    )]
    pub recap_min_activities: u32,

    /// RSS/Atom feed URLs, comma separated, that exist as news sources from
    /// the first start (admins manage the rest through the API).
    #[arg(id = "news_feeds", long = "news-feeds", env = "TOKULY_NEWS_FEEDS", default_value = "")]
    pub news_feeds: String,

    /// Seconds between two fetches of the same news source.
    #[arg(
        id = "news_fetch_interval_seconds",
        long = "news-fetch-interval-seconds",
        env = "TOKULY_NEWS_FETCH_INTERVAL_SECONDS",
        default_value_t = 900
    )]
    pub news_fetch_interval_seconds: u64,

    /// Seconds one fetch may take, redirects included.
    #[arg(
        id = "news_fetch_timeout_seconds",
        long = "news-fetch-timeout-seconds",
        env = "TOKULY_NEWS_FETCH_TIMEOUT_SECONDS",
        default_value_t = 15
    )]
    pub news_fetch_timeout_seconds: u64,

    /// Largest feed document accepted, in bytes (after decompression).
    #[arg(id = "news_max_feed_bytes", long = "news-max-feed-bytes", env = "TOKULY_NEWS_MAX_FEED_BYTES", default_value_t = 5 * 1024 * 1024)]
    pub news_max_feed_bytes: usize,

    /// Development and tests only: let the news fetcher reach loopback
    /// addresses and non-default ports (a local feed server). Refused in
    /// production.
    #[arg(id = "news_allow_loopback", long = "news-allow-loopback", env = "TOKULY_NEWS_ALLOW_LOOPBACK", default_value_t = false, action = clap::ArgAction::Set)]
    pub news_allow_loopback: bool,

    /// Days a news article is kept after it was published (and left its feed).
    #[arg(
        id = "news_retention_days",
        long = "news-retention-days",
        env = "TOKULY_NEWS_RETENTION_DAYS",
        default_value_t = 30
    )]
    pub news_retention_days: u32,

    /// Share of the recommended news order filled with random recent
    /// articles, in percent (0-100).
    #[arg(
        id = "news_random_share_percent",
        long = "news-random-share-percent",
        env = "TOKULY_NEWS_RANDOM_SHARE_PERCENT",
        default_value_t = 20
    )]
    pub news_random_share_percent: u32,

    /// Days after which a reader's interest in a tag has faded to half.
    #[arg(
        id = "news_affinity_half_life_days",
        long = "news-affinity-half-life-days",
        env = "TOKULY_NEWS_AFFINITY_HALF_LIFE_DAYS",
        default_value_t = 30
    )]
    pub news_affinity_half_life_days: u32,

    /// Keyword dictionary for tagging articles: `tag=keyword|keyword;tag=...`.
    /// An article whose title or summary contains a keyword gets the tag.
    /// Empty uses the built-in dictionary; `-` disables keyword tagging.
    #[arg(id = "news_tag_keywords", long = "news-tag-keywords", env = "TOKULY_NEWS_TAG_KEYWORDS", default_value = "")]
    pub news_tag_keywords: String,
}

impl Default for FeedConfig {
    fn default() -> Self {
        Self {
            activity_retention_days: 400,
            map_story_hours: 48,
            burst_minutes: 60,
            recap_cache_seconds: 300,
            recap_min_activities: 5,
            news_feeds: String::new(),
            news_fetch_interval_seconds: 900,
            news_fetch_timeout_seconds: 15,
            news_max_feed_bytes: 5 * 1024 * 1024,
            news_allow_loopback: false,
            news_retention_days: 30,
            news_random_share_percent: 20,
            news_affinity_half_life_days: 30,
            news_tag_keywords: String::new(),
        }
    }
}

impl FeedConfig {
    /// Refuses settings that cannot work, and those that are only safe on a
    /// developer's machine when running in production.
    pub fn validate(&self, production: bool) -> anyhow::Result<()> {
        anyhow::ensure!(
            !(production && self.news_allow_loopback),
            "TOKULY_NEWS_ALLOW_LOOPBACK is for development and tests; it must be off in production"
        );
        anyhow::ensure!(self.map_story_hours > 0, "TOKULY_FEED_MAP_STORY_HOURS must be positive");
        anyhow::ensure!(
            self.news_fetch_interval_seconds >= 60,
            "TOKULY_NEWS_FETCH_INTERVAL_SECONDS must be at least 60"
        );
        anyhow::ensure!(self.news_fetch_timeout_seconds > 0, "TOKULY_NEWS_FETCH_TIMEOUT_SECONDS must be positive");
        anyhow::ensure!(self.news_max_feed_bytes >= 1024, "TOKULY_NEWS_MAX_FEED_BYTES must be at least 1024");
        anyhow::ensure!(self.news_retention_days > 0, "TOKULY_NEWS_RETENTION_DAYS must be positive");
        anyhow::ensure!(
            self.news_random_share_percent <= 100,
            "TOKULY_NEWS_RANDOM_SHARE_PERCENT must be between 0 and 100"
        );
        anyhow::ensure!(self.news_affinity_half_life_days > 0, "TOKULY_NEWS_AFFINITY_HALF_LIFE_DAYS must be positive");
        Ok(())
    }

    /// The feed URLs of `TOKULY_NEWS_FEEDS`, trimmed, without empty entries.
    pub fn seed_feed_urls(&self) -> Vec<&str> {
        self.news_feeds.split(',').map(str::trim).filter(|url| !url.is_empty()).collect()
    }

    /// The interest half-life in seconds.
    pub fn affinity_half_life_seconds(&self) -> f64 {
        f64::from(self.news_affinity_half_life_days) * 86_400.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_everywhere() {
        assert!(FeedConfig::default().validate(true).is_ok());
        assert!(FeedConfig::default().validate(false).is_ok());
    }

    #[test]
    fn loopback_fetching_is_refused_in_production() {
        let cfg = FeedConfig { news_allow_loopback: true, ..Default::default() };
        assert!(cfg.validate(false).is_ok());
        assert!(cfg.validate(true).is_err());
    }

    #[test]
    fn nonsense_values_are_refused() {
        assert!(FeedConfig { news_random_share_percent: 101, ..Default::default() }.validate(false).is_err());
        assert!(FeedConfig { news_fetch_interval_seconds: 5, ..Default::default() }.validate(false).is_err());
        assert!(FeedConfig { map_story_hours: 0, ..Default::default() }.validate(false).is_err());
        assert!(FeedConfig { news_affinity_half_life_days: 0, ..Default::default() }.validate(false).is_err());
    }

    #[test]
    fn seed_urls_are_split_and_trimmed() {
        let cfg = FeedConfig {
            news_feeds: " https://a.example/rss , ,https://b.example/atom.xml,".into(),
            ..Default::default()
        };
        assert_eq!(cfg.seed_feed_urls(), vec!["https://a.example/rss", "https://b.example/atom.xml"]);
        assert!(FeedConfig::default().seed_feed_urls().is_empty());
    }
}
