//! Settings of the moments domain.
use clap::Args;

/// Every argument has an explicit `moments_…` ID: the server flattens all
/// domains' settings into one command line, where IDs must be unique.
#[derive(Debug, Clone, Args)]
pub struct MomentsConfig {
    /// Seconds after the prompt during which a moment counts as on time.
    #[arg(
        id = "moments_window_seconds",
        long = "moments-window-seconds",
        env = "TOKULY_MOMENTS_WINDOW_SECONDS",
        default_value_t = 120
    )]
    pub window_seconds: i64,

    /// Seconds a moment taken in the window may take to upload and still
    /// count as on time.
    #[arg(
        id = "moments_upload_grace_seconds",
        long = "moments-upload-grace-seconds",
        env = "TOKULY_MOMENTS_UPLOAD_GRACE_SECONDS",
        default_value_t = 60
    )]
    pub upload_grace_seconds: i64,

    /// Local hour (0-23) from which the daily prompt may go out.
    #[arg(
        id = "moments_prompt_from_hour",
        long = "moments-prompt-from-hour",
        env = "TOKULY_MOMENTS_PROMPT_FROM_HOUR",
        default_value_t = 9
    )]
    pub prompt_from_hour: u32,

    /// Local hour (1-24) before which the daily prompt has gone out.
    #[arg(
        id = "moments_prompt_until_hour",
        long = "moments-prompt-until-hour",
        env = "TOKULY_MOMENTS_PROMPT_UNTIL_HOUR",
        default_value_t = 22
    )]
    pub prompt_until_hour: u32,

    /// Seconds between passes of the worker that draws the days ahead and
    /// queues the prompts that are due.
    #[arg(
        id = "moments_worker_interval_seconds",
        long = "moments-worker-interval-seconds",
        env = "TOKULY_MOMENTS_WORKER_INTERVAL_SECONDS",
        default_value_t = 20
    )]
    pub worker_interval_seconds: u64,

    /// Days a deleted moment, and a replaced or taken back reaction, stay
    /// on record for reports, counted from the deletion.
    #[arg(
        id = "moments_evidence_days",
        long = "moments-evidence-days",
        env = "TOKULY_MOMENTS_EVIDENCE_DAYS",
        default_value_t = 30
    )]
    pub evidence_days: i64,
}

impl Default for MomentsConfig {
    fn default() -> Self {
        Self {
            window_seconds: 120,
            upload_grace_seconds: 60,
            prompt_from_hour: 9,
            prompt_until_hour: 22,
            worker_interval_seconds: 20,
            evidence_days: 30,
        }
    }
}

impl MomentsConfig {
    /// Rejects settings the domain cannot work with.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (30..=3600).contains(&self.window_seconds),
            "TOKULY_MOMENTS_WINDOW_SECONDS must be between 30 and 3600"
        );
        anyhow::ensure!(
            (0..=600).contains(&self.upload_grace_seconds),
            "TOKULY_MOMENTS_UPLOAD_GRACE_SECONDS must be between 0 and 600"
        );
        anyhow::ensure!(
            self.prompt_from_hour + crate::rules::WIDEST_BAND_HOURS < self.prompt_until_hour
                && self.prompt_until_hour <= 24,
            "TOKULY_MOMENTS_PROMPT_UNTIL_HOUR (at most 24) must be more than {} hours after \
             TOKULY_MOMENTS_PROMPT_FROM_HOUR: a prompt region spans that many hours of offsets",
            crate::rules::WIDEST_BAND_HOURS
        );
        anyhow::ensure!(self.worker_interval_seconds > 0, "TOKULY_MOMENTS_WORKER_INTERVAL_SECONDS must be positive");
        anyhow::ensure!(
            (1..=365).contains(&self.evidence_days),
            "TOKULY_MOMENTS_EVIDENCE_DAYS must be between 1 and 365"
        );
        Ok(())
    }

    /// The settings the staff console (「サーバー設定」) may change, by
    /// `tokuly.toml` key (backend/README.md, "Declaring a setting").
    pub const CONSOLE_EDITABLE: &'static [&'static str] = &[
        "window_seconds",
        "upload_grace_seconds",
        "prompt_from_hour",
        "prompt_until_hour",
        "worker_interval_seconds",
        "evidence_days",
    ];
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        moments: MomentsConfig,
    }

    #[test]
    fn the_command_line_and_default_agree() {
        let parsed = Cli::parse_from(["tokuly"]).moments;
        let default = MomentsConfig::default();
        assert_eq!(format!("{parsed:?}"), format!("{default:?}"));
        assert!(default.validate().is_ok());
        let set = Cli::parse_from(["tokuly", "--moments-window-seconds", "300"]).moments;
        assert_eq!(set.window_seconds, 300);
    }

    #[test]
    fn an_empty_prompt_window_is_refused() {
        let cfg = MomentsConfig { prompt_from_hour: 12, prompt_until_hour: 12, ..Default::default() };
        assert!(cfg.validate().is_err());
        let cfg = MomentsConfig { prompt_until_hour: 25, ..Default::default() };
        assert!(cfg.validate().is_err());
        // Narrower than a band: nowhere in it would the window be shared.
        let cfg = MomentsConfig { prompt_from_hour: 12, prompt_until_hour: 16, ..Default::default() };
        assert!(cfg.validate().is_err());
        let cfg = MomentsConfig { prompt_from_hour: 12, prompt_until_hour: 17, ..Default::default() };
        assert!(cfg.validate().is_ok());
    }
}
