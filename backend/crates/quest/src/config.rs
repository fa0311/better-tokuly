//! Quest-specific settings.
//!
//! What things are worth (the rule table, the mission pool, the badge
//! catalogue) is data in the crate; the settings here are the knobs an
//! operator may want to turn. Flags and argument IDs carry a `quest` prefix
//! because the server flattens every domain's settings into one command line.
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct QuestConfig {
    /// How long earnings are collected in Valkey before they are written to
    /// PostgreSQL and pushed to the player, in milliseconds. A player gets
    /// at most one "XP gained" push per this interval.
    #[arg(
        id = "quest_flush_delay_ms",
        long = "quest-flush-delay-ms",
        env = "TOKULY_QUEST_FLUSH_DELAY_MS",
        default_value_t = 3000
    )]
    pub flush_delay_ms: u64,

    /// Coins granted on every level-up.
    #[arg(
        id = "quest_level_up_coins",
        long = "quest-level-up-coins",
        env = "TOKULY_QUEST_LEVEL_UP_COINS",
        default_value_t = 20
    )]
    pub level_up_coins: i64,

    /// Coins granted on every tenth level instead.
    #[arg(
        id = "quest_milestone_level_coins",
        long = "quest-milestone-level-coins",
        env = "TOKULY_QUEST_MILESTONE_LEVEL_COINS",
        default_value_t = 100
    )]
    pub milestone_level_coins: i64,

    /// Price of restoring a lost streak, in coins (one restore per player
    /// and calendar month).
    #[arg(
        id = "quest_streak_restore_coins",
        long = "quest-streak-restore-coins",
        env = "TOKULY_QUEST_STREAK_RESTORE_COINS",
        default_value_t = 100
    )]
    pub streak_restore_coins: i64,

    /// Minutes after a player's midnight before a running streak is
    /// declared lost, so events that arrive late still count.
    #[arg(
        id = "quest_streak_grace_minutes",
        long = "quest-streak-grace-minutes",
        env = "TOKULY_QUEST_STREAK_GRACE_MINUTES",
        default_value_t = 10
    )]
    pub streak_grace_minutes: i64,

    /// Shortest streak a player is reminded about before it expires.
    #[arg(
        id = "quest_streak_reminder_min_days",
        long = "quest-streak-reminder-min-days",
        env = "TOKULY_QUEST_STREAK_REMINDER_MIN_DAYS",
        default_value_t = 3
    )]
    pub streak_reminder_min_days: i32,

    /// A rule counts as "live" when it fired for anyone within this many
    /// days. Missions and leaderboards are only offered for live rules, so
    /// features that do not exist (yet) on this server never show up.
    #[arg(
        id = "quest_live_rule_days",
        long = "quest-live-rule-days",
        env = "TOKULY_QUEST_LIVE_RULE_DAYS",
        default_value_t = 14
    )]
    pub live_rule_days: i64,

    /// Seconds between two recomputations of badge rarity.
    #[arg(
        id = "quest_rarity_interval_seconds",
        long = "quest-rarity-interval-seconds",
        env = "TOKULY_QUEST_RARITY_INTERVAL_SECONDS",
        default_value_t = 3600
    )]
    pub rarity_interval_seconds: u64,

    /// Namespace of the quest keys in Valkey. Deployments that share one
    /// Valkey but not one database must not share the queue of earnings
    /// waiting to be written; left empty, the namespace is derived from the
    /// database URL and the NATS prefix, which is right unless several
    /// databases hold the same players.
    #[arg(id = "quest_keyspace", long = "quest-keyspace", env = "TOKULY_QUEST_KEYSPACE", default_value = "")]
    pub keyspace: String,
}

impl Default for QuestConfig {
    fn default() -> Self {
        Self {
            flush_delay_ms: 3000,
            level_up_coins: 20,
            milestone_level_coins: 100,
            streak_restore_coins: 100,
            streak_grace_minutes: 10,
            streak_reminder_min_days: 3,
            live_rule_days: 14,
            rarity_interval_seconds: 3600,
            keyspace: String::new(),
        }
    }
}

impl QuestConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (100..=60_000).contains(&self.flush_delay_ms),
            "TOKULY_QUEST_FLUSH_DELAY_MS must be between 100 and 60000"
        );
        anyhow::ensure!(
            self.level_up_coins >= 0 && self.milestone_level_coins >= 0,
            "TOKULY_QUEST_LEVEL_UP_COINS and TOKULY_QUEST_MILESTONE_LEVEL_COINS must not be negative"
        );
        anyhow::ensure!(self.streak_restore_coins > 0, "TOKULY_QUEST_STREAK_RESTORE_COINS must be positive");
        anyhow::ensure!(
            (0..=120).contains(&self.streak_grace_minutes),
            "TOKULY_QUEST_STREAK_GRACE_MINUTES must be between 0 and 120"
        );
        anyhow::ensure!(self.streak_reminder_min_days >= 1, "TOKULY_QUEST_STREAK_REMINDER_MIN_DAYS must be at least 1");
        anyhow::ensure!(self.live_rule_days >= 1, "TOKULY_QUEST_LIVE_RULE_DAYS must be at least 1");
        anyhow::ensure!(self.rarity_interval_seconds >= 1, "TOKULY_QUEST_RARITY_INTERVAL_SECONDS must be positive");
        anyhow::ensure!(
            self.keyspace.len() <= 32
                && self.keyspace.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
            "TOKULY_QUEST_KEYSPACE may only contain letters, digits, '-' and '_' (at most 32)"
        );
        Ok(())
    }
}

impl QuestConfig {
    /// The settings the staff console (「サーバー設定」) may change, by
    /// `tokuly.toml` key. Everything else is set in tokuly.toml or the
    /// environment only: addresses, credentials, egress guards, paths,
    /// money and privacy floors never belong here (backend/README.md,
    /// "Declaring a setting").
    pub const CONSOLE_EDITABLE: &'static [&'static str] = &[
        "flush_delay_ms",
        "streak_grace_minutes",
        "streak_reminder_min_days",
        "live_rule_days",
        "rarity_interval_seconds",
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_nonsense_is_refused() {
        assert!(QuestConfig::default().validate().is_ok());
        assert!(QuestConfig { flush_delay_ms: 0, ..Default::default() }.validate().is_err());
        assert!(QuestConfig { streak_restore_coins: 0, ..Default::default() }.validate().is_err());
        assert!(QuestConfig { keyspace: "a b".into(), ..Default::default() }.validate().is_err());
        assert!(QuestConfig { keyspace: "blue-1".into(), ..Default::default() }.validate().is_ok());
    }

    #[test]
    fn the_command_line_defaults_match_default() {
        #[derive(clap::Parser)]
        struct Cli {
            #[command(flatten)]
            quest: QuestConfig,
        }
        use clap::Parser as _;
        let parsed = Cli::parse_from(["test"]).quest;
        let default = QuestConfig::default();
        assert_eq!(format!("{parsed:?}"), format!("{default:?}"));
    }
}
