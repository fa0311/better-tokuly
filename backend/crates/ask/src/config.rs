//! Settings of the ask domain.
//!
//! Flags and argument IDs carry an `ask` prefix because the server flattens
//! every domain's settings into one command line.
use clap::Args;

/// Words that hold a question in the FILTERED folder on every box, unless a
/// deployment replaces the list: insults, calls to self-harm and sexual
/// solicitation, in Japanese and English. Latin words match whole words,
/// everything else matches as a substring (see `filter.rs`).
pub const DEFAULT_FILTER_WORDS: &str = "死ね,死んで,氏ね,タヒね,殺す,殺して,ころすぞ,消えろ,きえろ,自殺しろ,くたばれ,きもい,キモい,キモイ,気持ち悪い,うざい,ウザい,ウザイ,ブス,デブ,ガイジ,クズ,嫌われてる,住所教えて,裸,下着,エッチ,やらせて,\
kys,kill yourself,go die,slut,whore,fag,retard,ugly,nudes,loser,nobody likes you";

#[derive(Debug, Clone, Args)]
pub struct AskConfig {
    /// Words (comma-separated) that hold a question or prompt response in the
    /// recipient's FILTERED folder, on top of each owner's own muted words.
    /// Empty switches the server-wide list off.
    #[arg(id = "ask_filter_words", long = "ask-filter-words", env = "TOKULY_ASK_FILTER_WORDS", default_value = DEFAULT_FILTER_WORDS)]
    pub filter_words: String,

    /// Questions and prompt responses one account may send per hour.
    #[arg(
        id = "ask_sends_per_hour",
        long = "ask-sends-per-hour",
        env = "TOKULY_ASK_SENDS_PER_HOUR",
        default_value_t = 30
    )]
    pub sends_per_hour: u32,

    /// Questions and prompt responses one account may send to the same
    /// person per day.
    #[arg(
        id = "ask_pair_sends_per_day",
        long = "ask-pair-sends-per-day",
        env = "TOKULY_ASK_PAIR_SENDS_PER_DAY",
        default_value_t = 10
    )]
    pub pair_sends_per_day: u32,

    /// Questions and prompt responses one person may receive per hour, from
    /// everyone together: the brake on a pile-on from many accounts.
    #[arg(
        id = "ask_box_receives_per_hour",
        long = "ask-box-receives-per-hour",
        env = "TOKULY_ASK_BOX_RECEIVES_PER_HOUR",
        default_value_t = 120
    )]
    pub box_receives_per_hour: u32,

    /// Polls in a compliment round.
    #[arg(id = "ask_round_polls", long = "ask-round-polls", env = "TOKULY_ASK_ROUND_POLLS", default_value_t = 12)]
    pub round_polls: u32,

    /// Seconds between finishing a round and the next one opening.
    #[arg(
        id = "ask_round_cooldown_seconds",
        long = "ask-round-cooldown-seconds",
        env = "TOKULY_ASK_ROUND_COOLDOWN_SECONDS",
        default_value_t = 3600
    )]
    pub round_cooldown_seconds: u64,

    /// Coins for a round in which every poll was voted on; skipped polls
    /// earn nothing (a round of 12 with 9 votes pays three quarters).
    #[arg(
        id = "ask_round_reward_coins",
        long = "ask-round-reward-coins",
        env = "TOKULY_ASK_ROUND_REWARD_COINS",
        default_value_t = 12
    )]
    pub round_reward_coins: i64,

    /// Times the four friends of one poll can be replaced.
    #[arg(
        id = "ask_shuffles_per_poll",
        long = "ask-shuffles-per-poll",
        env = "TOKULY_ASK_SHUFFLES_PER_POLL",
        default_value_t = 3
    )]
    pub shuffles_per_poll: u32,

    /// Price in coins of the first and the second hint about who sent a
    /// flame, separated by a comma. Empty sells no hints.
    #[arg(id = "ask_hint_prices", long = "ask-hint-prices", env = "TOKULY_ASK_HINT_PRICES", default_value = "30,60")]
    pub hint_prices: String,

    /// Days a deleted, withdrawn or dropped question is kept for moderators
    /// before it is erased. Unanswered questions leave the sender's "asked"
    /// list after the same time, so the erasure itself tells the sender
    /// nothing.
    #[arg(
        id = "ask_retention_days",
        long = "ask-retention-days",
        env = "TOKULY_ASK_RETENTION_DAYS",
        default_value_t = 180
    )]
    pub retention_days: u32,
}

impl Default for AskConfig {
    fn default() -> Self {
        Self {
            filter_words: DEFAULT_FILTER_WORDS.to_owned(),
            sends_per_hour: 30,
            pair_sends_per_day: 10,
            box_receives_per_hour: 120,
            round_polls: 12,
            round_cooldown_seconds: 3600,
            round_reward_coins: 12,
            shuffles_per_poll: 3,
            hint_prices: "30,60".to_owned(),
            retention_days: 180,
        }
    }
}

/// Hints sold per flame at most: the first letter, then school or months.
pub const MAX_HINTS: usize = 2;

impl AskConfig {
    /// The hint price ladder as numbers; empty when hints are not sold.
    pub fn hint_price_list(&self) -> anyhow::Result<Vec<i64>> {
        let prices: Vec<i64> = self
            .hint_prices
            .split(',')
            .map(str::trim)
            .filter(|price| !price.is_empty())
            .map(|price| match price.parse::<i64>() {
                Ok(coins) if (1..=1_000_000).contains(&coins) => Ok(coins),
                _ => anyhow::bail!("TOKULY_ASK_HINT_PRICES: `{price}` is not a price between 1 and 1,000,000 coins"),
            })
            .collect::<anyhow::Result<_>>()?;
        anyhow::ensure!(prices.len() <= MAX_HINTS, "TOKULY_ASK_HINT_PRICES: at most {MAX_HINTS} hints exist per flame");
        Ok(prices)
    }

    /// The server-wide filter words, trimmed, without empty entries.
    pub fn filter_word_list(&self) -> Vec<&str> {
        self.filter_words.split(',').map(str::trim).filter(|word| !word.is_empty()).collect()
    }

    /// Rejects settings the domain cannot run with. Called once at startup.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.sends_per_hour > 0, "TOKULY_ASK_SENDS_PER_HOUR must be positive");
        anyhow::ensure!(self.pair_sends_per_day > 0, "TOKULY_ASK_PAIR_SENDS_PER_DAY must be positive");
        anyhow::ensure!(self.box_receives_per_hour > 0, "TOKULY_ASK_BOX_RECEIVES_PER_HOUR must be positive");
        anyhow::ensure!((1..=50).contains(&self.round_polls), "TOKULY_ASK_ROUND_POLLS must be between 1 and 50");
        anyhow::ensure!(
            (0..=1_000_000).contains(&self.round_reward_coins),
            "TOKULY_ASK_ROUND_REWARD_COINS must be between 0 and 1,000,000"
        );
        anyhow::ensure!(self.retention_days >= 1, "TOKULY_ASK_RETENTION_DAYS must be at least 1");
        self.hint_price_list().map(|_| ())
    }
}

impl AskConfig {
    /// Settings whose empty value means something (none, off): an empty
    /// environment variable is that value, not "unset".
    pub const EMPTY_IS_A_VALUE: &'static [&'static str] = &["filter_words", "hint_prices"];

    /// The settings the staff console (「サーバー設定」) may change, by
    /// `tokuly.toml` key. Everything else is set in tokuly.toml or the
    /// environment only: addresses, credentials, egress guards, paths,
    /// money and privacy floors never belong here (backend/README.md,
    /// "Declaring a setting").
    pub const CONSOLE_EDITABLE: &'static [&'static str] = &[
        "filter_words",
        "sends_per_hour",
        "pair_sends_per_day",
        "box_receives_per_hour",
        "round_polls",
        "round_cooldown_seconds",
        "shuffles_per_poll",
        "retention_days",
    ];
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        ask: AskConfig,
    }

    /// Environment variables would leak into these parses; none of this
    /// domain's are set by the dev infrastructure.
    fn parse(args: &[&str]) -> AskConfig {
        Cli::parse_from(std::iter::once("tokuly").chain(args.iter().copied())).ask
    }

    #[test]
    fn command_line_defaults_match_the_default_impl() {
        let (parsed, default) = (parse(&[]), AskConfig::default());
        assert_eq!(parsed.filter_words, default.filter_words);
        assert_eq!(parsed.sends_per_hour, default.sends_per_hour);
        assert_eq!(parsed.pair_sends_per_day, default.pair_sends_per_day);
        assert_eq!(parsed.box_receives_per_hour, default.box_receives_per_hour);
        assert_eq!(parsed.round_polls, default.round_polls);
        assert_eq!(parsed.round_cooldown_seconds, default.round_cooldown_seconds);
        assert_eq!(parsed.round_reward_coins, default.round_reward_coins);
        assert_eq!(parsed.shuffles_per_poll, default.shuffles_per_poll);
        assert_eq!(parsed.hint_prices, default.hint_prices);
        assert_eq!(parsed.retention_days, default.retention_days);
        assert!(default.validate().is_ok());
    }

    #[test]
    fn flags_are_prefixed_and_parse() {
        let cfg = parse(&["--ask-round-polls", "6", "--ask-hint-prices", "5, 9", "--ask-filter-words", " a , ,b "]);
        assert_eq!(cfg.round_polls, 6);
        assert_eq!(cfg.hint_price_list().unwrap(), vec![5, 9]);
        assert_eq!(cfg.filter_word_list(), vec!["a", "b"]);
    }

    #[test]
    fn hint_prices_parse_and_reject_garbage() {
        let mut cfg = AskConfig::default();
        assert_eq!(cfg.hint_price_list().unwrap(), vec![30, 60]);
        cfg.hint_prices = String::new();
        assert!(cfg.hint_price_list().unwrap().is_empty(), "empty sells no hints");
        for bad in ["free", "0", "-3", "10,20,30"] {
            cfg.hint_prices = bad.to_owned();
            assert!(cfg.validate().is_err(), "{bad}");
        }
    }

    #[test]
    fn validate_guards_the_limits() {
        let broken: [fn(&mut AskConfig); 5] = [
            |c| c.sends_per_hour = 0,
            |c| c.box_receives_per_hour = 0,
            |c| c.round_polls = 0,
            |c| c.round_reward_coins = -1,
            |c| c.retention_days = 0,
        ];
        for mutate in broken {
            let mut cfg = AskConfig::default();
            mutate(&mut cfg);
            assert!(cfg.validate().is_err());
        }
    }
}
