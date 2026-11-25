//! Plans-specific settings.
use clap::Args;

/// Every argument has an explicit `plans_…` ID: the server flattens all
/// domains' settings into one command line, where IDs must be unique.
#[derive(Debug, Clone, Args)]
pub struct PlansConfig {
    /// Most people in one plan or one date poll, the host included. Bounds
    /// every per-participant fan-out (notifications, realtime pushes).
    #[arg(
        id = "plans_max_participants",
        long = "plans-max-participants",
        env = "TOKULY_PLANS_MAX_PARTICIPANTS",
        default_value_t = 100
    )]
    pub max_participants: i64,

    /// Hours a "free now" status lasts.
    #[arg(
        id = "plans_free_now_hours",
        long = "plans-free-now-hours",
        env = "TOKULY_PLANS_FREE_NOW_HOURS",
        default_value_t = 3
    )]
    pub free_now_hours: i64,

    /// Seconds between passes of the worker that sends reminders, closes
    /// polls and announces milestones.
    #[arg(
        id = "plans_worker_interval_seconds",
        long = "plans-worker-interval-seconds",
        env = "TOKULY_PLANS_WORKER_INTERVAL_SECONDS",
        default_value_t = 30
    )]
    pub worker_interval_seconds: u64,

    /// Local hour (0-23) at which people are reminded, the day before, of a
    /// plan that has a day but no time.
    #[arg(
        id = "plans_day_before_hour",
        long = "plans-day-before-hour",
        env = "TOKULY_PLANS_DAY_BEFORE_HOUR",
        default_value_t = 18
    )]
    pub day_before_hour: u32,

    /// Local hour (0-23) at which "today is the day" notices go out:
    /// anniversary milestones and countdowns that reached zero.
    #[arg(
        id = "plans_morning_hour",
        long = "plans-morning-hour",
        env = "TOKULY_PLANS_MORNING_HOUR",
        default_value_t = 8
    )]
    pub morning_hour: u32,
}

impl Default for PlansConfig {
    fn default() -> Self {
        Self {
            max_participants: 100,
            free_now_hours: 3,
            worker_interval_seconds: 30,
            day_before_hour: 18,
            morning_hour: 8,
        }
    }
}

impl PlansConfig {
    /// Rejects settings the domain cannot work with.
    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (2..=1000).contains(&self.max_participants),
            "TOKULY_PLANS_MAX_PARTICIPANTS must be between 2 and 1000"
        );
        anyhow::ensure!(
            (1..=24).contains(&self.free_now_hours),
            "TOKULY_PLANS_FREE_NOW_HOURS must be between 1 and 24"
        );
        anyhow::ensure!(self.worker_interval_seconds > 0, "TOKULY_PLANS_WORKER_INTERVAL_SECONDS must be positive");
        anyhow::ensure!(self.day_before_hour < 24, "TOKULY_PLANS_DAY_BEFORE_HOUR must be an hour of the day (0-23)");
        anyhow::ensure!(self.morning_hour < 24, "TOKULY_PLANS_MORNING_HOUR must be an hour of the day (0-23)");
        Ok(())
    }
}

impl PlansConfig {
    /// The settings the staff console (「サーバー設定」) may change, by
    /// `tokuly.toml` key. Everything else is set in tokuly.toml or the
    /// environment only: addresses, credentials, egress guards, paths,
    /// money and privacy floors never belong here (backend/README.md,
    /// "Declaring a setting").
    pub const CONSOLE_EDITABLE: &'static [&'static str] = &[
        "max_participants",
        "free_now_hours",
        "worker_interval_seconds",
        "day_before_hour",
        "morning_hour",
    ];
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        plans: PlansConfig,
    }

    #[test]
    fn the_command_line_and_default_agree() {
        let parsed = Cli::parse_from(["tokuly"]).plans;
        let default = PlansConfig::default();
        assert_eq!(
            (
                parsed.max_participants,
                parsed.free_now_hours,
                parsed.worker_interval_seconds,
                parsed.day_before_hour,
                parsed.morning_hour
            ),
            (
                default.max_participants,
                default.free_now_hours,
                default.worker_interval_seconds,
                default.day_before_hour,
                default.morning_hour
            )
        );
        assert!(default.validate().is_ok());
        let set = Cli::parse_from(["tokuly", "--plans-max-participants", "20", "--plans-morning-hour", "7"]).plans;
        assert_eq!((set.max_participants, set.morning_hour), (20, 7));
    }

    #[test]
    fn settings_out_of_range_are_rejected() {
        let broken = [
            PlansConfig { max_participants: 1, ..PlansConfig::default() },
            PlansConfig { free_now_hours: 0, ..PlansConfig::default() },
            PlansConfig { worker_interval_seconds: 0, ..PlansConfig::default() },
            PlansConfig { day_before_hour: 24, ..PlansConfig::default() },
            PlansConfig { morning_hour: 99, ..PlansConfig::default() },
        ];
        for cfg in broken {
            assert!(cfg.validate().is_err(), "{cfg:?}");
        }
    }
}
