//! Settings of the developer platform.
use std::time::Duration;

use clap::Args;
use tokuly_core::api_tokens::RateTier;

#[derive(Debug, Clone, PartialEq, Args)]
pub struct DevConfig {
    /// Personal access tokens one account may have at a time.
    #[arg(id = "dev_max_tokens", long = "dev-max-tokens", env = "TOKULY_DEV_MAX_TOKENS", default_value_t = 50)]
    pub max_tokens: i64,

    /// OAuth applications one account may register.
    #[arg(id = "dev_max_apps", long = "dev-max-apps", env = "TOKULY_DEV_MAX_APPS", default_value_t = 25)]
    pub max_apps: i64,

    /// Bots one person may own.
    #[arg(id = "dev_max_bots", long = "dev-max-bots", env = "TOKULY_DEV_MAX_BOTS", default_value_t = 5)]
    pub max_bots: i64,

    /// Tokens one bot may have at a time.
    #[arg(
        id = "dev_max_bot_tokens",
        long = "dev-max-bot-tokens",
        env = "TOKULY_DEV_MAX_BOT_TOKENS",
        default_value_t = 10
    )]
    pub max_bot_tokens: i64,

    /// Webhooks one account (a person or a bot) may have.
    #[arg(id = "dev_max_webhooks", long = "dev-max-webhooks", env = "TOKULY_DEV_MAX_WEBHOOKS", default_value_t = 20)]
    pub max_webhooks: i64,

    /// Requests per minute of a token in the `standard` tier (0 = unlimited).
    #[arg(
        id = "dev_rate_standard",
        long = "dev-rate-standard",
        env = "TOKULY_DEV_RATE_STANDARD",
        default_value_t = 600
    )]
    pub rate_standard: u32,

    /// Requests per minute in the `elevated` tier, which an operator gives
    /// to an application or token that needs more (0 = unlimited).
    #[arg(
        id = "dev_rate_elevated",
        long = "dev-rate-elevated",
        env = "TOKULY_DEV_RATE_ELEVATED",
        default_value_t = 3000
    )]
    pub rate_elevated: u32,

    /// Lifetime of an OAuth access token, in seconds.
    #[arg(
        id = "dev_access_token_seconds",
        long = "dev-access-token-seconds",
        env = "TOKULY_DEV_ACCESS_TOKEN_SECONDS",
        default_value_t = 3600
    )]
    pub access_token_seconds: i64,

    /// Days an OAuth refresh token stays usable (renewed on every use).
    #[arg(
        id = "dev_refresh_token_days",
        long = "dev-refresh-token-days",
        env = "TOKULY_DEV_REFRESH_TOKEN_DAYS",
        default_value_t = 90
    )]
    pub refresh_token_days: i64,

    /// Seconds an authorization code is valid.
    #[arg(
        id = "dev_auth_code_seconds",
        long = "dev-auth-code-seconds",
        env = "TOKULY_DEV_AUTH_CODE_SECONDS",
        default_value_t = 60
    )]
    pub auth_code_seconds: u64,

    /// Seconds a device has to get its code approved (RFC 8628).
    #[arg(
        id = "dev_device_code_seconds",
        long = "dev-device-code-seconds",
        env = "TOKULY_DEV_DEVICE_CODE_SECONDS",
        default_value_t = 900
    )]
    pub device_code_seconds: u64,

    /// Seconds a device must wait between polls of the token endpoint.
    #[arg(
        id = "dev_device_poll_seconds",
        long = "dev-device-poll-seconds",
        env = "TOKULY_DEV_DEVICE_POLL_SECONDS",
        default_value_t = 5
    )]
    pub device_poll_seconds: u64,

    /// Hours a rotated client secret or webhook secret keeps working.
    #[arg(
        id = "dev_secret_overlap_hours",
        long = "dev-secret-overlap-hours",
        env = "TOKULY_DEV_SECRET_OVERLAP_HOURS",
        default_value_t = 24
    )]
    pub secret_overlap_hours: i64,

    /// Seconds a webhook endpoint has to answer.
    #[arg(
        id = "dev_webhook_timeout_seconds",
        long = "dev-webhook-timeout-seconds",
        env = "TOKULY_DEV_WEBHOOK_TIMEOUT_SECONDS",
        default_value_t = 10
    )]
    pub webhook_timeout_seconds: u64,

    /// Attempts per delivery before giving up.
    #[arg(
        id = "dev_webhook_max_attempts",
        long = "dev-webhook-max-attempts",
        env = "TOKULY_DEV_WEBHOOK_MAX_ATTEMPTS",
        default_value_t = 8
    )]
    pub webhook_max_attempts: i32,

    /// Pause before the first retry, in seconds; each further one waits four
    /// times longer, up to a day.
    #[arg(
        id = "dev_webhook_retry_seconds",
        long = "dev-webhook-retry-seconds",
        env = "TOKULY_DEV_WEBHOOK_RETRY_SECONDS",
        default_value_t = 60
    )]
    pub webhook_retry_seconds: i64,

    /// A webhook is disabled once this many attempts in a row failed...
    #[arg(
        id = "dev_webhook_disable_after_failures",
        long = "dev-webhook-disable-after-failures",
        env = "TOKULY_DEV_WEBHOOK_DISABLE_AFTER_FAILURES",
        default_value_t = 20
    )]
    pub webhook_disable_after_failures: i32,

    /// ...and it has been failing for at least this many hours.
    #[arg(
        id = "dev_webhook_disable_after_hours",
        long = "dev-webhook-disable-after-hours",
        env = "TOKULY_DEV_WEBHOOK_DISABLE_AFTER_HOURS",
        default_value_t = 48
    )]
    pub webhook_disable_after_hours: i64,

    /// Deliveries per minute that one person's webhooks (their bots'
    /// included) may send to one host; events beyond that are dropped.
    /// Bounds what a webhook can be made to throw at somebody else's
    /// server. 0 = unlimited.
    #[arg(
        id = "dev_webhook_rate_per_minute",
        long = "dev-webhook-rate-per-minute",
        env = "TOKULY_DEV_WEBHOOK_RATE_PER_MINUTE",
        default_value_t = 120
    )]
    pub webhook_rate_per_minute: u32,

    /// Days the record of who owned a deleted bot is kept, counted from
    /// the deletion.
    #[arg(
        id = "dev_bot_record_days",
        long = "dev-bot-record-days",
        env = "TOKULY_DEV_BOT_RECORD_DAYS",
        default_value_t = 180
    )]
    pub bot_record_days: i64,

    /// Days the delivery log is kept.
    #[arg(
        id = "dev_webhook_log_days",
        long = "dev-webhook-log-days",
        env = "TOKULY_DEV_WEBHOOK_LOG_DAYS",
        default_value_t = 30
    )]
    pub webhook_log_days: i64,

    /// Development and tests: let webhooks use `http://` and reach private,
    /// loopback and link-local addresses. Refused in production.
    #[arg(id = "dev_webhook_allow_private", long = "dev-webhook-allow-private", env = "TOKULY_DEV_WEBHOOK_ALLOW_PRIVATE", default_value_t = false, action = clap::ArgAction::Set)]
    pub webhook_allow_private: bool,

    /// E-mail addresses of the accounts the demo simulator (`tokuly demo`)
    /// signs up to own its bots, comma-separated. Leave it unset unless the
    /// simulator runs against this server. Bots owned by these accounts may
    /// be given tokens that write to every domain except the wallet, so
    /// they can move on the map, chat and post like people, as long as the
    /// account's password is TOKULY_DEV_DEMO_PASSWORD; every other bot
    /// stays limited to its own profile, uploads and inbox.
    #[arg(
        id = "dev_demo_operators",
        long = "dev-demo-operators",
        env = "TOKULY_DEV_DEMO_OPERATORS",
        value_delimiter = ',',
        default_value = ""
    )]
    pub demo_operators: Vec<String>,

    /// Password of the demo's operator accounts (at least 16 characters),
    /// shared with the simulator. Signing up needs no verified address, so
    /// an address alone proves nothing: only an account that also knows
    /// this password gets its bots the demo's write scopes.
    #[arg(
        id = "dev_demo_password",
        long = "dev-demo-password",
        env = "TOKULY_DEV_DEMO_PASSWORD",
        default_value = "",
        hide_env_values = true
    )]
    pub demo_password: String,
}

impl Default for DevConfig {
    fn default() -> Self {
        Self {
            max_tokens: 50,
            max_apps: 25,
            max_bots: 5,
            max_bot_tokens: 10,
            max_webhooks: 20,
            rate_standard: 600,
            rate_elevated: 3000,
            access_token_seconds: 3600,
            refresh_token_days: 90,
            auth_code_seconds: 60,
            device_code_seconds: 900,
            device_poll_seconds: 5,
            secret_overlap_hours: 24,
            webhook_timeout_seconds: 10,
            webhook_max_attempts: 8,
            webhook_retry_seconds: 60,
            webhook_disable_after_failures: 20,
            webhook_disable_after_hours: 48,
            webhook_rate_per_minute: 120,
            bot_record_days: 180,
            webhook_log_days: 30,
            webhook_allow_private: false,
            demo_operators: Vec::new(),
            demo_password: String::new(),
        }
    }
}

impl DevConfig {
    pub fn validate(&self, production: bool) -> anyhow::Result<()> {
        anyhow::ensure!(
            !(production && self.webhook_allow_private),
            "TOKULY_DEV_WEBHOOK_ALLOW_PRIVATE must not be set in production"
        );
        anyhow::ensure!(
            self.demo_operators().next().is_none() || self.demo_password.chars().count() >= 16,
            "TOKULY_DEV_DEMO_OPERATORS needs TOKULY_DEV_DEMO_PASSWORD (at least 16 characters; tools/demo-env.sh makes one)"
        );
        anyhow::ensure!(self.access_token_seconds >= 60, "TOKULY_DEV_ACCESS_TOKEN_SECONDS must be at least 60");
        anyhow::ensure!(self.refresh_token_days >= 1, "TOKULY_DEV_REFRESH_TOKEN_DAYS must be at least 1");
        anyhow::ensure!(self.auth_code_seconds >= 10, "TOKULY_DEV_AUTH_CODE_SECONDS must be at least 10");
        anyhow::ensure!(self.webhook_max_attempts >= 1, "TOKULY_DEV_WEBHOOK_MAX_ATTEMPTS must be at least 1");
        anyhow::ensure!(self.webhook_timeout_seconds >= 1, "TOKULY_DEV_WEBHOOK_TIMEOUT_SECONDS must be at least 1");
        Ok(())
    }

    /// The limits of a rate tier by name; unknown names get the standard one.
    pub fn rate_tier(&self, name: &str) -> RateTier {
        match name {
            "elevated" => RateTier { name: "elevated".into(), per_minute: self.rate_elevated },
            _ => RateTier { name: "standard".into(), per_minute: self.rate_standard },
        }
    }

    /// The demo operators' addresses, without blanks.
    fn demo_operators(&self) -> impl Iterator<Item = &str> {
        self.demo_operators.iter().map(|operator| operator.trim()).filter(|operator| !operator.is_empty())
    }

    /// Whether `email` is the address of a demo operator: an account whose
    /// bots take part in the demo (see `demo_operators`). Only the address:
    /// a new bot token also needs the password (`Dev::create_token`).
    pub(crate) fn is_demo_operator(&self, email: Option<&str>) -> bool {
        let Some(email) = email.map(str::trim).filter(|email| !email.is_empty()) else { return false };
        self.demo_password.chars().count() >= 16
            && self.demo_operators().any(|operator| operator.eq_ignore_ascii_case(email))
    }

    pub(crate) fn secret_overlap(&self) -> chrono::Duration {
        chrono::Duration::hours(self.secret_overlap_hours)
    }

    pub(crate) fn webhook_timeout(&self) -> Duration {
        Duration::from_secs(self.webhook_timeout_seconds)
    }

    /// How long to wait after the `attempt`-th failed attempt (1-based).
    pub(crate) fn retry_delay(&self, attempt: i32) -> chrono::Duration {
        let factor = 4i64.saturating_pow((attempt - 1).clamp(0, 12) as u32);
        chrono::Duration::seconds(self.webhook_retry_seconds.max(1).saturating_mul(factor).min(24 * 3600))
    }
}

impl DevConfig {
    /// The settings the staff console (「サーバー設定」) may change, by
    /// `tokuly.toml` key. Everything else is set in tokuly.toml or the
    /// environment only: addresses, credentials, egress guards, paths,
    /// money and privacy floors never belong here (backend/README.md,
    /// "Declaring a setting").
    pub const CONSOLE_EDITABLE: &'static [&'static str] = &[
        "max_tokens",
        "max_apps",
        "max_bots",
        "max_bot_tokens",
        "max_webhooks",
        "rate_standard",
        "rate_elevated",
        "access_token_seconds",
        "refresh_token_days",
        "auth_code_seconds",
        "device_code_seconds",
        "device_poll_seconds",
        "secret_overlap_hours",
        "webhook_timeout_seconds",
        "webhook_max_attempts",
        "webhook_retry_seconds",
        "webhook_disable_after_failures",
        "webhook_disable_after_hours",
        "webhook_rate_per_minute",
        "bot_record_days",
        "webhook_log_days",
    ];
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct CommandLine {
        #[command(flatten)]
        dev: DevConfig,
    }

    #[test]
    fn demo_operators_are_named_by_address_and_nobody_is_one_by_default() {
        let parse = |flags: &[&str]| CommandLine::parse_from([&["tokuly"], flags].concat()).dev;
        for cfg in [
            DevConfig::default(),
            parse(&[]),
            parse(&["--dev-demo-operators", ""]),
            parse(&["--dev-demo-operators", " , "]),
        ] {
            for address in [None, Some(""), Some(" "), Some("demo@example.com")] {
                assert!(!cfg.is_demo_operator(address), "{address:?}");
            }
        }
        let operators = ["--dev-demo-operators", "Demo-1@Example.com, demo-2@example.com"];
        let named = parse(&[&operators[..], &["--dev-demo-password", "0123456789abcdef"]].concat());
        assert!(
            named.is_demo_operator(Some("demo-1@example.com")) && named.is_demo_operator(Some("DEMO-2@example.com"))
        );
        assert!(
            !named.is_demo_operator(Some("demo-3@example.com"))
                && !named.is_demo_operator(Some("example.com"))
                && !named.is_demo_operator(None)
        );
        named.validate(true).unwrap();
        // Addresses without a (long enough) password name nobody, and the server refuses to start.
        for password in [&[][..], &["--dev-demo-password", "short"]] {
            let unproven = parse(&[&operators[..], password].concat());
            assert!(!unproven.is_demo_operator(Some("demo-1@example.com")));
            assert!(unproven.validate(false).unwrap_err().to_string().contains("TOKULY_DEV_DEMO_PASSWORD"));
        }
    }

    #[test]
    fn the_command_line_defaults_are_the_default_config() {
        let defaults = CommandLine::parse_from(["tokuly"]).dev;
        // An empty list setting parses as one empty entry: nobody.
        assert!(defaults.demo_operators.iter().all(String::is_empty));
        assert_eq!(DevConfig { demo_operators: Vec::new(), ..defaults }, DevConfig::default());
        // Every flag is prefixed, so it cannot clash with another domain's.
        let parsed =
            CommandLine::parse_from(["tokuly", "--dev-max-bots", "9", "--dev-webhook-allow-private", "true"]).dev;
        assert_eq!((parsed.max_bots, parsed.webhook_allow_private), (9, true));
    }

    #[test]
    fn retries_back_off_fourfold_up_to_a_day() {
        let cfg = DevConfig::default();
        let seconds: Vec<i64> = (1..=8).map(|attempt| cfg.retry_delay(attempt).num_seconds()).collect();
        assert_eq!(seconds, [60, 240, 960, 3840, 15360, 61440, 86400, 86400]);
    }

    #[test]
    fn tiers_fall_back_to_standard() {
        let cfg = DevConfig::default();
        assert_eq!(cfg.rate_tier("elevated").per_minute, 3000);
        assert_eq!(cfg.rate_tier("standard").per_minute, 600);
        assert_eq!(cfg.rate_tier("nonsense").name, "standard");
    }

    #[test]
    fn private_webhook_targets_are_refused_in_production() {
        let cfg = DevConfig { webhook_allow_private: true, ..Default::default() };
        assert!(cfg.validate(false).is_ok());
        assert!(cfg.validate(true).is_err());
        assert!(DevConfig::default().validate(true).is_ok());
    }
}
