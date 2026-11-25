//! Emoji-specific settings.
//!
//! The server flattens every domain's settings into one command line, where
//! both the flag and clap's internal argument ID (by default the field name)
//! must be unique. Every argument therefore carries an `emoji` prefix in both.
use clap::Args;

/// Deck handed to users who never saved one.
pub const DEFAULT_DECK: &str = "👍,❤️,😆,🤔,😮,🎉,💢,😥,😇,🍮";

#[derive(Debug, Clone, Args)]
pub struct EmojiConfig {
    /// Submissions one user may have waiting for review at the same time.
    #[arg(
        id = "emoji_max_pending",
        long = "emoji-max-pending",
        env = "TOKULY_EMOJI_MAX_PENDING",
        default_value_t = 10
    )]
    pub max_pending: i64,

    /// Largest upload accepted as an emoji picture, in kilobytes.
    #[arg(
        id = "emoji_max_image_kb",
        long = "emoji-max-image-kb",
        env = "TOKULY_EMOJI_MAX_IMAGE_KB",
        default_value_t = 1024
    )]
    pub max_image_kb: i64,

    /// Names users may not submit (moderators may add them), comma separated,
    /// on top of the built-in list.
    #[arg(
        id = "emoji_reserved_names",
        long = "emoji-reserved-names",
        env = "TOKULY_EMOJI_RESERVED_NAMES",
        default_value = "",
        value_delimiter = ','
    )]
    pub reserved_names: Vec<String>,

    /// The deck of users who never saved one: `:name:` or a Unicode emoji
    /// each, comma separated.
    #[arg(
        id = "emoji_default_deck",
        long = "emoji-default-deck",
        env = "TOKULY_EMOJI_DEFAULT_DECK",
        default_value = DEFAULT_DECK,
        value_delimiter = ','
    )]
    pub default_deck: Vec<String>,

    /// Seconds between runs of the usage worker, which moves the use
    /// counters into the database and recomputes "popular".
    #[arg(
        id = "emoji_usage_flush_seconds",
        long = "emoji-usage-flush-seconds",
        env = "TOKULY_EMOJI_USAGE_FLUSH_SECONDS",
        default_value_t = 60
    )]
    pub usage_flush_seconds: u64,

    /// Days of use that count towards "popular" (older days weigh less).
    #[arg(
        id = "emoji_popular_days",
        long = "emoji-popular-days",
        env = "TOKULY_EMOJI_POPULAR_DAYS",
        default_value_t = 7
    )]
    pub popular_days: i32,

    /// Days the catalogue remembers that an emoji was removed. A client
    /// that was away for longer downloads the whole catalogue again.
    #[arg(
        id = "emoji_removal_retention_days",
        long = "emoji-removal-retention-days",
        env = "TOKULY_EMOJI_REMOVAL_RETENTION_DAYS",
        default_value_t = 90
    )]
    pub removal_retention_days: i64,

    /// Largest emoji pack accepted for import, in megabytes.
    #[arg(
        id = "emoji_import_max_mb",
        long = "emoji-import-max-mb",
        env = "TOKULY_EMOJI_IMPORT_MAX_MB",
        default_value_t = 100
    )]
    pub import_max_mb: i64,

    /// Most entries an imported pack may have.
    #[arg(
        id = "emoji_import_max_entries",
        long = "emoji-import-max-entries",
        env = "TOKULY_EMOJI_IMPORT_MAX_ENTRIES",
        default_value_t = 5000
    )]
    pub import_max_entries: usize,

    /// Hours an export file can be downloaded before it is deleted.
    #[arg(
        id = "emoji_export_ttl_hours",
        long = "emoji-export-ttl-hours",
        env = "TOKULY_EMOJI_EXPORT_TTL_HOURS",
        default_value_t = 168
    )]
    pub export_ttl_hours: i64,
}

impl Default for EmojiConfig {
    fn default() -> Self {
        Self {
            max_pending: 10,
            max_image_kb: 1024,
            reserved_names: Vec::new(),
            default_deck: DEFAULT_DECK.split(',').map(str::to_owned).collect(),
            usage_flush_seconds: 60,
            popular_days: 7,
            removal_retention_days: 90,
            import_max_mb: 100,
            import_max_entries: 5000,
            export_ttl_hours: 168,
        }
    }
}

impl EmojiConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.max_pending > 0, "TOKULY_EMOJI_MAX_PENDING must be positive");
        anyhow::ensure!(self.max_image_kb > 0, "TOKULY_EMOJI_MAX_IMAGE_KB must be positive");
        anyhow::ensure!(self.popular_days > 0, "TOKULY_EMOJI_POPULAR_DAYS must be positive");
        anyhow::ensure!(self.removal_retention_days > 0, "TOKULY_EMOJI_REMOVAL_RETENTION_DAYS must be positive");
        anyhow::ensure!(
            self.import_max_mb > 0 && self.import_max_entries > 0,
            "the emoji import limits must be positive"
        );
        anyhow::ensure!(self.export_ttl_hours > 0, "TOKULY_EMOJI_EXPORT_TTL_HOURS must be positive");
        Ok(())
    }

    /// Largest emoji picture in bytes.
    pub fn max_image_bytes(&self) -> i64 {
        self.max_image_kb * 1024
    }

    /// Largest import archive in bytes.
    pub fn import_max_bytes(&self) -> i64 {
        self.import_max_mb * 1024 * 1024
    }
}

impl EmojiConfig {
    /// Settings whose empty value means something (none, off): an empty
    /// environment variable is that value, not "unset".
    pub const EMPTY_IS_A_VALUE: &'static [&'static str] = &["default_deck"];

    /// The settings the staff console (「サーバー設定」) may change, by
    /// `tokuly.toml` key. Everything else is set in tokuly.toml or the
    /// environment only: addresses, credentials, egress guards, paths,
    /// money and privacy floors never belong here (backend/README.md,
    /// "Declaring a setting").
    pub const CONSOLE_EDITABLE: &'static [&'static str] = &[
        "max_pending",
        "max_image_kb",
        "reserved_names",
        "default_deck",
        "usage_flush_seconds",
        "popular_days",
        "removal_retention_days",
        "import_max_mb",
        "import_max_entries",
        "export_ttl_hours",
    ];
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        emoji: EmojiConfig,
    }

    /// Another domain's settings that happen to use the same field names.
    #[derive(Args)]
    struct Lookalike {
        #[arg(long = "other-max-pending", default_value_t = 1)]
        max_pending: i64,
        #[arg(long = "other-popular-days", default_value_t = 1)]
        popular_days: i32,
        #[arg(long = "other-reserved-names", default_value = "", value_delimiter = ',')]
        reserved_names: Vec<String>,
    }

    #[derive(Parser)]
    struct Server {
        #[command(flatten)]
        emoji: EmojiConfig,
        #[command(flatten)]
        other: Lookalike,
    }

    #[test]
    fn arguments_do_not_collide_with_same_named_fields_of_other_domains() {
        use clap::CommandFactory as _;
        Server::command().debug_assert();
        let parsed = Server::parse_from(["tokuly", "--emoji-max-pending", "3", "--other-popular-days", "9"]);
        assert_eq!((parsed.emoji.max_pending, parsed.other.max_pending), (3, 1));
        assert_eq!((parsed.emoji.popular_days, parsed.other.popular_days), (7, 9));
        assert!(parsed.other.reserved_names.iter().all(String::is_empty));
    }

    #[test]
    fn command_line_defaults_match_default() {
        let parsed = Cli::parse_from(["tokuly"]).emoji;
        let default = EmojiConfig::default();
        assert_eq!(parsed.max_pending, default.max_pending);
        assert_eq!(parsed.max_image_kb, default.max_image_kb);
        assert_eq!(parsed.default_deck, default.default_deck);
        assert_eq!(parsed.usage_flush_seconds, default.usage_flush_seconds);
        assert_eq!(parsed.popular_days, default.popular_days);
        assert_eq!(parsed.removal_retention_days, default.removal_retention_days);
        assert_eq!(parsed.import_max_mb, default.import_max_mb);
        assert_eq!(parsed.import_max_entries, default.import_max_entries);
        assert_eq!(parsed.export_ttl_hours, default.export_ttl_hours);
        // An empty variable means "no extra names", not one empty name.
        assert!(parsed.reserved_names.iter().all(String::is_empty));
        default.validate().unwrap();
    }

    #[test]
    fn lists_are_comma_separated() {
        let parsed = Cli::parse_from([
            "tokuly",
            "--emoji-reserved-names",
            "tokuly_chan,mascot",
            "--emoji-default-deck",
            ":tokuly:,👍",
        ])
        .emoji;
        assert_eq!(parsed.reserved_names, ["tokuly_chan", "mascot"]);
        assert_eq!(parsed.default_deck, [":tokuly:", "👍"]);
    }

    #[test]
    fn nonsense_limits_are_refused() {
        assert!(EmojiConfig { max_pending: 0, ..Default::default() }.validate().is_err());
        assert!(EmojiConfig { import_max_entries: 0, ..Default::default() }.validate().is_err());
    }
}
