//! Search-specific settings.
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct SearchConfig {
    /// Prefix of every Meilisearch index name (`<prefix>_<kind>`) and of the
    /// search keys in Valkey, so several deployments (and test runs) can
    /// share one Meilisearch and one Valkey. Letters, digits, `-` and `_`.
    #[arg(id = "search_index_prefix", long = "search-index-prefix", env = "TOKULY_SEARCH_INDEX_PREFIX", default_value = "tokuly")]
    pub index_prefix: String,

    /// Documents written to Meilisearch per indexer pass.
    #[arg(id = "search_index_batch", long = "search-index-batch", env = "TOKULY_SEARCH_INDEX_BATCH", default_value_t = 1000)]
    pub index_batch: u32,

    /// How long the indexer waits for more changes before it looks again.
    #[arg(id = "search_index_interval_ms", long = "search-index-interval-ms", env = "TOKULY_SEARCH_INDEX_INTERVAL_MS", default_value_t = 500)]
    pub index_interval_ms: u64,

    /// Searches one signed-in user may run per minute.
    #[arg(id = "search_rate_per_minute", long = "search-rate-per-minute", env = "TOKULY_SEARCH_RATE_PER_MINUTE", default_value_t = 120)]
    pub rate_per_minute: u32,

    /// Searches one client address may run per minute without signing in.
    #[arg(id = "search_anonymous_rate_per_minute", long = "search-anonymous-rate-per-minute", env = "TOKULY_SEARCH_ANONYMOUS_RATE_PER_MINUTE", default_value_t = 40)]
    pub anonymous_rate_per_minute: u32,

    /// Typeahead requests per minute: signed-in users get this many, anonymous
    /// client addresses half.
    #[arg(id = "search_suggest_rate_per_minute", long = "search-suggest-rate-per-minute", env = "TOKULY_SEARCH_SUGGEST_RATE_PER_MINUTE", default_value_t = 300)]
    pub suggest_rate_per_minute: u32,

    /// Distinct signed-in users who must have searched a query within the
    /// trending window before it may trend or be offered as a completion.
    #[arg(id = "search_trending_min_users", long = "search-trending-min-users", env = "TOKULY_SEARCH_TRENDING_MIN_USERS", default_value_t = 5)]
    pub trending_min_users: u32,

    /// Hours of searches the trending list looks back on (1-168).
    #[arg(id = "search_trending_window_hours", long = "search-trending-window-hours", env = "TOKULY_SEARCH_TRENDING_WINDOW_HOURS", default_value_t = 24)]
    pub trending_window_hours: u32,

    /// Hours after which a search counts half as much towards trending.
    #[arg(id = "search_trending_half_life_hours", long = "search-trending-half-life-hours", env = "TOKULY_SEARCH_TRENDING_HALF_LIFE_HOURS", default_value_t = 6)]
    pub trending_half_life_hours: u32,

    /// Seconds between two computations of the trending list.
    #[arg(id = "search_trending_refresh_seconds", long = "search-trending-refresh-seconds", env = "TOKULY_SEARCH_TRENDING_REFRESH_SECONDS", default_value_t = 60)]
    pub trending_refresh_seconds: u64,

    /// Queries kept in the trending list.
    #[arg(id = "search_trending_size", long = "search-trending-size", env = "TOKULY_SEARCH_TRENDING_SIZE", default_value_t = 20)]
    pub trending_size: u32,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            index_prefix: "tokuly".to_owned(),
            index_batch: 1000,
            index_interval_ms: 500,
            rate_per_minute: 120,
            anonymous_rate_per_minute: 40,
            suggest_rate_per_minute: 300,
            trending_min_users: 5,
            trending_window_hours: 24,
            trending_half_life_hours: 6,
            trending_refresh_seconds: 60,
            trending_size: 20,
        }
    }
}

impl SearchConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        let prefix_ok = !self.index_prefix.is_empty()
            && self.index_prefix.len() <= 64
            && self.index_prefix.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        anyhow::ensure!(prefix_ok, "TOKULY_SEARCH_INDEX_PREFIX must be 1-64 characters of a-z, A-Z, 0-9, - and _");
        anyhow::ensure!((1..=10_000).contains(&self.index_batch), "TOKULY_SEARCH_INDEX_BATCH must be 1-10000");
        anyhow::ensure!(self.index_interval_ms >= 10, "TOKULY_SEARCH_INDEX_INTERVAL_MS must be at least 10");
        anyhow::ensure!(self.trending_min_users >= 1, "TOKULY_SEARCH_TRENDING_MIN_USERS must be at least 1");
        anyhow::ensure!((1..=168).contains(&self.trending_window_hours), "TOKULY_SEARCH_TRENDING_WINDOW_HOURS must be 1-168");
        anyhow::ensure!(self.trending_half_life_hours >= 1, "TOKULY_SEARCH_TRENDING_HALF_LIFE_HOURS must be at least 1");
        anyhow::ensure!(self.trending_refresh_seconds >= 1, "TOKULY_SEARCH_TRENDING_REFRESH_SECONDS must be at least 1");
        anyhow::ensure!((1..=100).contains(&self.trending_size), "TOKULY_SEARCH_TRENDING_SIZE must be 1-100");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        SearchConfig::default().validate().unwrap();
    }

    #[test]
    fn prefixes_that_cannot_name_an_index_are_refused() {
        for bad in ["", "has space", "dots.are.out", "日本語"] {
            let cfg = SearchConfig { index_prefix: bad.to_owned(), ..Default::default() };
            assert!(cfg.validate().is_err(), "{bad:?} should be refused");
        }
        let cfg = SearchConfig { index_prefix: "tokuly-test_1".to_owned(), ..Default::default() };
        cfg.validate().unwrap();
    }

    #[test]
    fn nonsensical_trending_settings_are_refused() {
        assert!(SearchConfig { trending_min_users: 0, ..Default::default() }.validate().is_err());
        assert!(SearchConfig { trending_window_hours: 0, ..Default::default() }.validate().is_err());
        assert!(SearchConfig { trending_window_hours: 500, ..Default::default() }.validate().is_err());
        assert!(SearchConfig { trending_size: 0, ..Default::default() }.validate().is_err());
    }
}
