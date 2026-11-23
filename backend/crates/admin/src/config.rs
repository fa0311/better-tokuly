//! Admin-specific settings.
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct AdminConfig {
    /// E-mail address of the account that runs this deployment. The account
    /// holding it is given the `admin` role once that address is verified
    /// (at startup, or the moment it is), and only once per deployment and
    /// address: whoever holds the address later is not promoted. A handle is
    /// not accepted (whoever signed up first would hold it). Empty: nobody is
    /// promoted automatically.
    #[arg(long, env = "TOKULY_ADMIN_USER", default_value = "")]
    pub bootstrap_admin: String,

    /// IANA time zone whose midnight starts "today" in the server stats
    /// (new accounts today, daily active users).
    #[arg(long, env = "TOKULY_ADMIN_STATS_TIME_ZONE", default_value = "Asia/Tokyo")]
    pub stats_time_zone: String,

    /// An announcement broadcast notifies accounts with a signed-in device
    /// that was active within this many days.
    #[arg(long, env = "TOKULY_ANNOUNCEMENT_NOTIFY_ACTIVE_DAYS", default_value_t = 30)]
    pub announcement_notify_active_days: i32,

    /// Users notified per transaction during an announcement broadcast.
    #[arg(long, env = "TOKULY_ANNOUNCEMENT_NOTIFY_CHUNK", default_value_t = 500)]
    pub announcement_notify_chunk: i64,

    /// Suspensions, bans, warnings and takedowns one moderator may issue per
    /// hour. Admins are not limited. Guards against a compromised or runaway
    /// moderator account.
    #[arg(long, env = "TOKULY_MODERATOR_SANCTIONS_PER_HOUR", default_value_t = 120)]
    pub moderator_sanctions_per_hour: u32,
}

impl Default for AdminConfig {
    fn default() -> Self {
        Self {
            bootstrap_admin: String::new(),
            stats_time_zone: "Asia/Tokyo".to_owned(),
            announcement_notify_active_days: 30,
            announcement_notify_chunk: 500,
            moderator_sanctions_per_hour: 120,
        }
    }
}

impl AdminConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.stats_time_zone.parse::<chrono_tz::Tz>().is_ok(),
            "TOKULY_ADMIN_STATS_TIME_ZONE is not an IANA time zone: {}",
            self.stats_time_zone
        );
        anyhow::ensure!(self.announcement_notify_active_days >= 1, "TOKULY_ANNOUNCEMENT_NOTIFY_ACTIVE_DAYS must be at least 1");
        anyhow::ensure!((1..=5000).contains(&self.announcement_notify_chunk), "TOKULY_ANNOUNCEMENT_NOTIFY_CHUNK must be between 1 and 5000");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_bad_values_are_rejected() {
        assert!(AdminConfig::default().validate().is_ok());
        let bad_zone = AdminConfig { stats_time_zone: "Mars/Olympus".into(), ..Default::default() };
        assert!(bad_zone.validate().is_err());
        let bad_chunk = AdminConfig { announcement_notify_chunk: 0, ..Default::default() };
        assert!(bad_chunk.validate().is_err());
    }
}
