//! Wallet-specific settings.
//!
//! Flags and argument IDs carry a `wallet` prefix because the server
//! flattens every domain's settings into one command line.
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct WalletConfig {
    /// Enables the built-in "development" payment provider, which treats
    /// every purchase as paid on the spot (free coins). Unset means: on,
    /// unless `TOKULY_ENV=production`.
    #[arg(id = "wallet_dev_provider", long = "wallet-dev-provider", env = "TOKULY_WALLET_DEV_PROVIDER", action = clap::ArgAction::Set)]
    pub dev_provider: Option<bool>,

    /// Share of every transfer between users (gifts, stamp pack sales) the
    /// platform keeps, in percent (0-100).
    #[arg(id = "wallet_platform_fee_percent", long = "wallet-platform-fee-percent", env = "TOKULY_WALLET_PLATFORM_FEE_PERCENT", default_value_t = 30)]
    pub platform_fee_percent: u32,

    /// Daily login bonus ladder: coins for streak day 1, 2, ... separated by
    /// commas. After the last step the ladder starts over. Empty disables the
    /// bonus.
    #[arg(id = "wallet_daily_bonus_coins", long = "wallet-daily-bonus-coins", env = "TOKULY_WALLET_DAILY_BONUS_COINS", default_value = "10,10,15,15,20,20,50")]
    pub daily_bonus_coins: String,

    /// Smallest number of earned coins a payout request may ask for.
    #[arg(id = "wallet_payout_min_coins", long = "wallet-payout-min-coins", env = "TOKULY_WALLET_PAYOUT_MIN_COINS", default_value_t = 1000)]
    pub payout_min_coins: i64,

    /// Currency payouts are made in (ISO 4217).
    #[arg(id = "wallet_payout_currency", long = "wallet-payout-currency", env = "TOKULY_WALLET_PAYOUT_CURRENCY", default_value = "JPY")]
    pub payout_currency: String,

    /// Money paid per 100 earned coins, in the minor unit of the payout
    /// currency (100 = ¥100 for JPY).
    #[arg(id = "wallet_payout_minor_per_100_coins", long = "wallet-payout-minor-per-100-coins", env = "TOKULY_WALLET_PAYOUT_MINOR_PER_100_COINS", default_value_t = 100)]
    pub payout_minor_per_100_coins: i64,

    /// Minutes a purchase may stay unpaid before it expires.
    #[arg(id = "wallet_purchase_ttl_minutes", long = "wallet-purchase-ttl-minutes", env = "TOKULY_WALLET_PURCHASE_TTL_MINUTES", default_value_t = 60)]
    pub purchase_ttl_minutes: i64,
}

impl Default for WalletConfig {
    fn default() -> Self {
        Self {
            dev_provider: None,
            platform_fee_percent: 30,
            daily_bonus_coins: "10,10,15,15,20,20,50".to_owned(),
            payout_min_coins: 1000,
            payout_currency: "JPY".to_owned(),
            payout_minor_per_100_coins: 100,
            purchase_ttl_minutes: 60,
        }
    }
}

impl WalletConfig {
    /// Whether the development payment provider is on for this deployment.
    pub fn dev_provider_enabled(&self, production: bool) -> bool {
        self.dev_provider.unwrap_or(!production)
    }

    /// The daily bonus ladder as numbers; empty when the bonus is disabled.
    pub fn daily_bonus_cycle(&self) -> anyhow::Result<Vec<i64>> {
        self.daily_bonus_coins
            .split(',')
            .map(str::trim)
            .filter(|step| !step.is_empty())
            .map(|step| match step.parse::<i64>() {
                Ok(coins) if (1..=1_000_000).contains(&coins) => Ok(coins),
                _ => anyhow::bail!("TOKULY_WALLET_DAILY_BONUS_COINS: `{step}` is not a coin amount between 1 and 1,000,000"),
            })
            .collect()
    }

    /// Rejects settings that would corrupt money handling. Called once at startup.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.platform_fee_percent <= 100, "TOKULY_WALLET_PLATFORM_FEE_PERCENT must be 0-100");
        anyhow::ensure!(self.payout_min_coins >= 1, "TOKULY_WALLET_PAYOUT_MIN_COINS must be at least 1");
        anyhow::ensure!(self.payout_minor_per_100_coins >= 0, "TOKULY_WALLET_PAYOUT_MINOR_PER_100_COINS cannot be negative");
        anyhow::ensure!(is_currency_code(&self.payout_currency), "TOKULY_WALLET_PAYOUT_CURRENCY must be a 3-letter ISO 4217 code");
        anyhow::ensure!(self.purchase_ttl_minutes >= 1, "TOKULY_WALLET_PURCHASE_TTL_MINUTES must be at least 1");
        self.daily_bonus_cycle().map(|_| ())
    }
}

/// Shape check for ISO 4217 codes ("JPY", "USD").
pub(crate) fn is_currency_code(code: &str) -> bool {
    code.len() == 3 && code.bytes().all(|b| b.is_ascii_uppercase())
}

impl WalletConfig {
    /// Settings whose empty value means something (none, off): an empty
    /// environment variable is that value, not "unset".
    pub const EMPTY_IS_A_VALUE: &'static [&'static str] = &["daily_bonus_coins"];

    /// The settings the staff console (「サーバー設定」) may change, by
    /// `tokuly.toml` key. Everything else is set in tokuly.toml or the
    /// environment only: addresses, credentials, egress guards, paths,
    /// money and privacy floors never belong here (backend/README.md,
    /// "Declaring a setting").
    pub const CONSOLE_EDITABLE: &'static [&'static str] = &[
        "purchase_ttl_minutes",
    ];
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        wallet: WalletConfig,
    }

    /// Environment variables would leak into these parses; none of the
    /// wallet's are set by the dev infrastructure.
    fn parse(args: &[&str]) -> WalletConfig {
        Cli::parse_from(std::iter::once("tokuly").chain(args.iter().copied())).wallet
    }

    #[test]
    fn command_line_defaults_match_the_default_impl() {
        let (parsed, default) = (parse(&[]), WalletConfig::default());
        assert_eq!(parsed.dev_provider, default.dev_provider);
        assert_eq!(parsed.platform_fee_percent, default.platform_fee_percent);
        assert_eq!(parsed.daily_bonus_coins, default.daily_bonus_coins);
        assert_eq!(parsed.payout_min_coins, default.payout_min_coins);
        assert_eq!(parsed.payout_currency, default.payout_currency);
        assert_eq!(parsed.payout_minor_per_100_coins, default.payout_minor_per_100_coins);
        assert_eq!(parsed.purchase_ttl_minutes, default.purchase_ttl_minutes);
    }

    #[test]
    fn flags_are_prefixed_and_parse() {
        let cfg = parse(&["--wallet-dev-provider", "false", "--wallet-platform-fee-percent", "20", "--wallet-daily-bonus-coins", "5,5,30", "--wallet-payout-currency", "USD"]);
        assert_eq!(cfg.dev_provider, Some(false));
        assert_eq!((cfg.platform_fee_percent, cfg.payout_currency.as_str()), (20, "USD"));
        assert_eq!(cfg.daily_bonus_cycle().unwrap(), vec![5, 5, 30]);
    }

    #[test]
    fn dev_provider_defaults_to_on_outside_production() {
        let mut cfg = WalletConfig::default();
        assert!(cfg.dev_provider_enabled(false));
        assert!(!cfg.dev_provider_enabled(true));
        cfg.dev_provider = Some(true);
        assert!(cfg.dev_provider_enabled(true), "an explicit setting wins over the environment");
        cfg.dev_provider = Some(false);
        assert!(!cfg.dev_provider_enabled(false));
    }

    #[test]
    fn daily_bonus_ladder_parses_and_rejects_garbage() {
        let mut cfg = WalletConfig::default();
        assert_eq!(cfg.daily_bonus_cycle().unwrap(), vec![10, 10, 15, 15, 20, 20, 50]);
        cfg.daily_bonus_coins = " 5 , 8 ,".to_owned();
        assert_eq!(cfg.daily_bonus_cycle().unwrap(), vec![5, 8]);
        cfg.daily_bonus_coins = String::new();
        assert!(cfg.daily_bonus_cycle().unwrap().is_empty(), "empty disables the bonus");
        for bad in ["ten", "0", "-5", "10,,x"] {
            cfg.daily_bonus_coins = bad.to_owned();
            assert!(cfg.validate().is_err(), "{bad}");
        }
    }

    #[test]
    fn validate_guards_money_settings() {
        assert!(WalletConfig::default().validate().is_ok());
        let broken: [fn(&mut WalletConfig); 4] = [
            |c| c.platform_fee_percent = 101,
            |c| c.payout_min_coins = 0,
            |c| c.payout_currency = "yen".to_owned(),
            |c| c.purchase_ttl_minutes = 0,
        ];
        for mutate in broken {
            let mut cfg = WalletConfig::default();
            mutate(&mut cfg);
            assert!(cfg.validate().is_err());
        }
    }
}
