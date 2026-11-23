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

    /// The deployment's own account (the same setting as the admin domain's
    /// bootstrap admin). Its handle may look official (`tokuly_admin`); every
    /// other account is refused handles that pass for Tokuly or its staff.
    #[arg(
        long = "identity-operator-account",
        id = "identity_operator_account",
        env = "TOKULY_ADMIN_USER",
        default_value = ""
    )]
    pub operator_account: String,

    /// Spotify application credentials for the "now playing" connection.
    /// Leave empty to disable the integration.
    #[arg(long, env = "TOKULY_SPOTIFY_CLIENT_ID", default_value = "")]
    pub spotify_client_id: String,
    #[arg(
        long,
        env = "TOKULY_SPOTIFY_CLIENT_SECRET",
        default_value = "",
        hide_env_values = true
    )]
    pub spotify_client_secret: String,
}

impl Default for IdentityConfig {
    fn default() -> Self {
        Self {
            registration_open: true,
            min_age: 13,
            session_days: 60,
            deletion_grace_days: 30,
            operator_account: String::new(),
            spotify_client_id: String::new(),
            spotify_client_secret: String::new(),
        }
    }
}

impl IdentityConfig {
    pub fn spotify_enabled(&self) -> bool {
        !self.spotify_client_id.is_empty() && !self.spotify_client_secret.is_empty()
    }
}
