//! Social-specific settings.
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct SocialConfig {
    /// Most friends one account can have. Bounds every per-friend fan-out
    /// (presence, locations, "friends bumped").
    #[arg(long = "social-max-friends", env = "TOKULY_SOCIAL_MAX_FRIENDS", default_value_t = 1000)]
    pub max_friends: i64,

    /// Most accounts one user can follow (pending follow requests included).
    #[arg(long = "social-max-following", env = "TOKULY_SOCIAL_MAX_FOLLOWING", default_value_t = 5000)]
    pub max_following: i64,

    /// How many friends the automatic best-friend ranking keeps per user.
    #[arg(long = "social-best-friend-count", env = "TOKULY_SOCIAL_BEST_FRIEND_COUNT", default_value_t = 8)]
    pub best_friend_count: i32,

    /// Seconds between runs of the best-friend ranking worker.
    #[arg(
        id = "social_ranking_interval_seconds",
        long = "social-ranking-interval-seconds",
        env = "TOKULY_SOCIAL_RANKING_INTERVAL_SECONDS",
        default_value_t = 300
    )]
    pub ranking_interval_seconds: u64,

    /// Radius of nearby discovery, in meters.
    #[arg(long = "social-nearby-radius-meters", env = "TOKULY_SOCIAL_NEARBY_RADIUS_METERS", default_value_t = 50.0)]
    pub nearby_radius_meters: f64,

    /// How far apart two shaking phones may be to count as a bump, in meters.
    #[arg(long = "social-bump-radius-meters", env = "TOKULY_SOCIAL_BUMP_RADIUS_METERS", default_value_t = 30.0)]
    pub bump_radius_meters: f64,

    /// How far apart in time two shakes may be to count as a bump, in milliseconds.
    #[arg(long = "social-bump-window-ms", env = "TOKULY_SOCIAL_BUMP_WINDOW_MS", default_value_t = 4000)]
    pub bump_window_ms: i64,

    /// Seconds during which further shakes of the same two people return the
    /// bump they just made instead of counting a new one.
    #[arg(long = "social-bump-cooldown-seconds", env = "TOKULY_SOCIAL_BUMP_COOLDOWN_SECONDS", default_value_t = 30)]
    pub bump_cooldown_seconds: u64,

    /// How close two friends must be for a buzz, in meters.
    #[arg(long = "social-buzz-radius-meters", env = "TOKULY_SOCIAL_BUZZ_RADIUS_METERS", default_value_t = 200.0)]
    pub buzz_radius_meters: f64,
}

impl Default for SocialConfig {
    fn default() -> Self {
        Self {
            max_friends: 1000,
            max_following: 5000,
            best_friend_count: 8,
            ranking_interval_seconds: 300,
            nearby_radius_meters: 50.0,
            bump_radius_meters: 30.0,
            bump_window_ms: 4000,
            bump_cooldown_seconds: 30,
            buzz_radius_meters: 200.0,
        }
    }
}

impl SocialConfig {
    /// The settings the staff console (「サーバー設定」) may change, by
    /// `tokuly.toml` key. Everything else is set in tokuly.toml or the
    /// environment only: addresses, credentials, egress guards, paths,
    /// money and privacy floors never belong here (backend/README.md,
    /// "Declaring a setting").
    pub const CONSOLE_EDITABLE: &'static [&'static str] = &[
        "max_friends",
        "max_following",
        "best_friend_count",
        "ranking_interval_seconds",
        "nearby_radius_meters",
        "bump_radius_meters",
        "bump_window_ms",
        "bump_cooldown_seconds",
        "buzz_radius_meters",
    ];
}
