//! Stamp-specific settings.
//!
//! The server flattens every domain's settings into one command line, where
//! both the flag and clap's internal argument ID (by default the field name)
//! must be unique. Every argument therefore carries a `stamp` prefix in both.
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct StampConfig {
    /// Share of every paid pack sale kept by the platform, in percent (0-100).
    #[arg(
        id = "stamp_fee_percent",
        long = "stamp-fee-percent",
        env = "TOKULY_STAMP_FEE_PERCENT",
        default_value_t = 30
    )]
    pub fee_percent: u32,

    /// Packs one creator may have, counting every status.
    #[arg(id = "stamp_max_packs", long = "stamp-max-packs", env = "TOKULY_STAMP_MAX_PACKS", default_value_t = 100)]
    pub max_packs: i64,

    /// Personal (quick) stamps one user may keep.
    #[arg(
        id = "stamp_max_personal_stamps",
        long = "stamp-max-personal",
        env = "TOKULY_STAMP_MAX_PERSONAL",
        default_value_t = 300
    )]
    pub max_personal_stamps: i64,

    /// Seconds between refreshes of the popularity ranking and send counters.
    #[arg(
        id = "stamp_ranking_interval_seconds",
        long = "stamp-ranking-interval-seconds",
        env = "TOKULY_STAMP_RANKING_INTERVAL_SECONDS",
        default_value_t = 300
    )]
    pub ranking_interval_seconds: u64,

    /// Days a row of the usage log is kept. Daily statistics are kept forever.
    #[arg(
        id = "stamp_usage_retention_days",
        long = "stamp-usage-retention-days",
        env = "TOKULY_STAMP_USAGE_RETENTION_DAYS",
        default_value_t = 180
    )]
    pub usage_retention_days: i64,

    /// Create the built-in official packs when the workers start.
    #[arg(id = "stamp_seed_official", long = "stamp-seed-official", env = "TOKULY_STAMP_SEED_OFFICIAL", default_value_t = true, action = clap::ArgAction::Set)]
    pub seed_official: bool,
}

impl Default for StampConfig {
    fn default() -> Self {
        Self {
            fee_percent: 30,
            max_packs: 100,
            max_personal_stamps: 300,
            ranking_interval_seconds: 300,
            usage_retention_days: 180,
            seed_official: true,
        }
    }
}

impl StampConfig {
    /// The settings the staff console (「サーバー設定」) may change, by
    /// `tokuly.toml` key. Everything else is set in tokuly.toml or the
    /// environment only: addresses, credentials, egress guards, paths,
    /// money and privacy floors never belong here (backend/README.md,
    /// "Declaring a setting").
    pub const CONSOLE_EDITABLE: &'static [&'static str] = &[
        "max_packs",
        "max_personal",
        "ranking_interval_seconds",
        "usage_retention_days",
        "seed_official",
    ];
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        stamp: StampConfig,
    }

    /// Another domain's settings that happen to use the same field names.
    #[derive(Args)]
    struct Lookalike {
        #[arg(long = "other-fee-percent", default_value_t = 1)]
        fee_percent: u32,
        #[arg(long = "other-max-packs", default_value_t = 1)]
        max_packs: i64,
        #[arg(long = "other-max-personal", default_value_t = 1)]
        max_personal_stamps: i64,
        #[arg(long = "other-ranking-interval-seconds", default_value_t = 1)]
        ranking_interval_seconds: u64,
        #[arg(long = "other-usage-retention-days", default_value_t = 1)]
        usage_retention_days: i64,
        #[arg(long = "other-seed-official", default_value_t = false, action = clap::ArgAction::Set)]
        seed_official: bool,
    }

    #[derive(Parser)]
    struct Server {
        #[command(flatten)]
        stamp: StampConfig,
        #[command(flatten)]
        other: Lookalike,
    }

    #[test]
    fn arguments_do_not_collide_with_same_named_fields_of_other_domains() {
        use clap::CommandFactory as _;
        Server::command().debug_assert();
        let parsed = Server::parse_from(["tokuly", "--stamp-fee-percent", "7", "--other-max-packs", "9"]);
        assert_eq!((parsed.stamp.fee_percent, parsed.other.fee_percent), (7, 1));
        assert_eq!((parsed.stamp.max_packs, parsed.other.max_packs), (100, 9));
        assert_eq!((parsed.stamp.ranking_interval_seconds, parsed.other.ranking_interval_seconds), (300, 1));
        assert_eq!((parsed.stamp.usage_retention_days, parsed.other.usage_retention_days), (180, 1));
        assert_eq!((parsed.stamp.max_personal_stamps, parsed.other.max_personal_stamps), (300, 1));
        assert_eq!((parsed.stamp.seed_official, parsed.other.seed_official), (true, false));
    }

    #[test]
    fn command_line_defaults_match_default() {
        let parsed = Cli::parse_from(["tokuly"]).stamp;
        let default = StampConfig::default();
        assert_eq!(
            (
                parsed.fee_percent,
                parsed.max_packs,
                parsed.max_personal_stamps,
                parsed.ranking_interval_seconds,
                parsed.usage_retention_days,
                parsed.seed_official
            ),
            (
                default.fee_percent,
                default.max_packs,
                default.max_personal_stamps,
                default.ranking_interval_seconds,
                default.usage_retention_days,
                default.seed_official
            )
        );
    }

    #[test]
    fn flags_are_prefixed_and_booleans_take_a_value() {
        let parsed = Cli::parse_from(["tokuly", "--stamp-fee-percent", "15", "--stamp-seed-official", "false"]).stamp;
        assert_eq!((parsed.fee_percent, parsed.seed_official), (15, false));
    }
}
