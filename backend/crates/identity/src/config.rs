//! Identity-specific settings.
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct IdentityConfig {
    /// Allow new accounts to be created.
    #[arg(long, env = "TOKULY_REGISTRATION_OPEN", default_value_t = true, action = clap::ArgAction::Set)]
    pub registration_open: bool,

    /// Minimum age in years to sign up.
    #[arg(long, env = "TOKULY_MIN_AGE", default_value_t = 13)]
    pub min_age: u32,

    /// Refresh token lifetime in days (sliding: renewed on every refresh).
    #[arg(long, env = "TOKULY_SESSION_DAYS", default_value_t = 60)]
    pub session_days: i64,

    /// Days between an account deletion request and the actual purge.
    #[arg(long, env = "TOKULY_DELETION_GRACE_DAYS", default_value_t = 30)]
    pub deletion_grace_days: i64,

    /// Spotify application credentials for the "now playing" connection.
    /// Leave empty to disable the integration.
    #[arg(long, env = "TOKULY_SPOTIFY_CLIENT_ID", default_value = "")]
    pub spotify_client_id: String,
    /// Spotify app client secret (profile music connection).
    #[arg(long, env = "TOKULY_SPOTIFY_CLIENT_SECRET", default_value = "", hide_env_values = true)]
    pub spotify_client_secret: String,
    /// Base URL of the Spotify Web API (a test double in tests).
    #[arg(long, env = "TOKULY_SPOTIFY_API_URL", default_value = "https://api.spotify.com")]
    pub spotify_api_url: String,

    /// How many accounts one device may hold for quick switching.
    #[arg(long, env = "TOKULY_DEVICE_ACCOUNT_LIMIT", default_value_t = 5)]
    pub device_account_limit: i32,

    /// A switch to an account with two-factor authentication asks for the
    /// code again when its session on that device has not passed it within
    /// this many hours.
    #[arg(long, env = "TOKULY_ACCOUNT_SWITCH_MFA_HOURS", default_value_t = 12)]
    pub switch_mfa_hours: i64,

    /// Let the profile link check reach loopback addresses and other ports
    /// (development and tests only; refused in production).
    #[arg(long, env = "TOKULY_PROFILE_LINKS_ALLOW_LOOPBACK", default_value_t = false, action = clap::ArgAction::Set)]
    pub profile_links_allow_loopback: bool,
}

impl Default for IdentityConfig {
    fn default() -> Self {
        Self {
            registration_open: true,
            min_age: 13,
            session_days: 60,
            deletion_grace_days: 30,
            spotify_client_id: String::new(),
            spotify_client_secret: String::new(),
            spotify_api_url: "https://api.spotify.com".to_owned(),
            device_account_limit: 5,
            switch_mfa_hours: 12,
            profile_links_allow_loopback: false,
        }
    }
}

impl IdentityConfig {
    pub fn spotify_enabled(&self) -> bool {
        !self.spotify_client_id.is_empty() && !self.spotify_client_secret.is_empty()
    }
}
