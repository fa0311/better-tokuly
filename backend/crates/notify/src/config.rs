//! Notify-specific settings.
use clap::Args;

/// Push services browsers hand out endpoints for: Chrome/Edge/Opera/Brave
/// (FCM), Firefox (Mozilla autopush), Edge legacy (WNS) and Safari.
pub const DEFAULT_WEBPUSH_HOSTS: &str =
    "fcm.googleapis.com,updates.push.services.mozilla.com,notify.windows.com,push.apple.com";

#[derive(Clone, Args)]
pub struct NotifyConfig {
    /// VAPID private key (raw 32-byte P-256 scalar, base64url) for Web Push.
    /// Empty derives one from `TOKULY_SECRET`. Changing the key invalidates
    /// every existing browser subscription.
    #[arg(
        long,
        env = "TOKULY_VAPID_PRIVATE_KEY",
        default_value = "",
        hide_env_values = true
    )]
    pub vapid_private_key: String,

    /// Contact URI (`mailto:` or `https:`) push services may use to reach the
    /// operator. Empty uses the public URL when it is https, otherwise the
    /// address of `TOKULY_MAIL_FROM`.
    #[arg(long, env = "TOKULY_VAPID_SUBJECT", default_value = "")]
    pub vapid_subject: String,

    /// Hosts a Web Push endpoint may point to, comma separated; subdomains are
    /// included. The server POSTs to these URLs, so the list is what keeps a
    /// crafted subscription from reaching internal services. `*` allows any
    /// https host with a public-looking name. Either way the server only
    /// connects to public addresses of those names.
    #[arg(long, env = "TOKULY_WEBPUSH_ALLOWED_HOSTS", default_value = DEFAULT_WEBPUSH_HOSTS)]
    pub webpush_allowed_hosts: String,

    /// Development and tests only: accept `http://` Web Push endpoints on any
    /// host (a local stand-in for a push service). Refused in production.
    #[arg(long, env = "TOKULY_WEBPUSH_ALLOW_INSECURE", default_value_t = false, action = clap::ArgAction::Set)]
    pub webpush_allow_insecure: bool,

    /// Content of the Firebase service-account JSON file. Empty disables FCM.
    #[arg(
        long,
        env = "TOKULY_FCM_SERVICE_ACCOUNT_JSON",
        default_value = "",
        hide_env_values = true
    )]
    pub fcm_service_account_json: String,

    /// APNs token-signing key (the `.p8` file's content; `\n` escapes are
    /// accepted). APNs is enabled when the key, key ID, team ID and topic are
    /// all set.
    #[arg(
        long,
        env = "TOKULY_APNS_KEY_P8",
        default_value = "",
        hide_env_values = true
    )]
    pub apns_key_p8: String,
    #[arg(long, env = "TOKULY_APNS_KEY_ID", default_value = "")]
    pub apns_key_id: String,
    #[arg(long, env = "TOKULY_APNS_TEAM_ID", default_value = "")]
    pub apns_team_id: String,
    /// The iOS app's bundle ID.
    #[arg(long, env = "TOKULY_APNS_TOPIC", default_value = "")]
    pub apns_topic: String,

    /// How long a push service keeps a notification for an offline device.
    #[arg(long, env = "TOKULY_PUSH_TTL_SECONDS", default_value_t = 86_400)]
    pub push_ttl_seconds: u32,

    /// Push notifications one user may receive per minute. Beyond it
    /// notifications are still recorded in the inbox but not pushed.
    #[arg(long, env = "TOKULY_PUSH_RATE_PER_MINUTE", default_value_t = 30)]
    pub push_rate_per_minute: u32,

    /// Push notifications one person may cause for one recipient per
    /// minute, so that one sender cannot use up the recipient's whole
    /// budget. Urgent notifications have a budget of the same size of their
    /// own. 0 = no cap.
    #[arg(long, env = "TOKULY_PUSH_RATE_PER_SENDER_PER_MINUTE", default_value_t = 10)]
    pub push_rate_per_sender_per_minute: u32,

    /// Days an inbox entry is kept.
    #[arg(long, env = "TOKULY_NOTIFY_RETENTION_DAYS", default_value_t = 90)]
    pub notify_retention_days: u32,
}

/// Keys and credentials never appear in logs, even when the whole
/// configuration is printed.
impl std::fmt::Debug for NotifyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let secret = |value: &str| {
            if value.trim().is_empty() {
                "(not set)"
            } else {
                "(set)"
            }
        };
        f.debug_struct("NotifyConfig")
            .field("vapid_private_key", &secret(&self.vapid_private_key))
            .field("vapid_subject", &self.vapid_subject)
            .field("webpush_allowed_hosts", &self.webpush_allowed_hosts)
            .field("webpush_allow_insecure", &self.webpush_allow_insecure)
            .field(
                "fcm_service_account_json",
                &secret(&self.fcm_service_account_json),
            )
            .field("apns_key_p8", &secret(&self.apns_key_p8))
            .field("apns_key_id", &self.apns_key_id)
            .field("apns_team_id", &self.apns_team_id)
            .field("apns_topic", &self.apns_topic)
            .field("push_ttl_seconds", &self.push_ttl_seconds)
            .field("push_rate_per_minute", &self.push_rate_per_minute)
            .field(
                "push_rate_per_sender_per_minute",
                &self.push_rate_per_sender_per_minute,
            )
            .field("notify_retention_days", &self.notify_retention_days)
            .finish()
    }
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            vapid_private_key: String::new(),
            vapid_subject: String::new(),
            webpush_allowed_hosts: DEFAULT_WEBPUSH_HOSTS.to_owned(),
            webpush_allow_insecure: false,
            fcm_service_account_json: String::new(),
            apns_key_p8: String::new(),
            apns_key_id: String::new(),
            apns_team_id: String::new(),
            apns_topic: String::new(),
            push_ttl_seconds: 86_400,
            push_rate_per_minute: 30,
            push_rate_per_sender_per_minute: 10,
            notify_retention_days: 90,
        }
    }
}

impl NotifyConfig {
    /// Refuses settings that are only safe on a developer's machine.
    pub fn validate(&self, production: bool) -> anyhow::Result<()> {
        anyhow::ensure!(
            !(production && self.webpush_allow_insecure),
            "TOKULY_WEBPUSH_ALLOW_INSECURE is for development and tests; it must be off in production"
        );
        Ok(())
    }

    pub fn fcm_enabled(&self) -> bool {
        !self.fcm_service_account_json.trim().is_empty()
    }

    pub fn apns_enabled(&self) -> bool {
        [
            &self.apns_key_p8,
            &self.apns_key_id,
            &self.apns_team_id,
            &self.apns_topic,
        ]
        .iter()
        .all(|v| !v.trim().is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insecure_endpoints_are_refused_in_production() {
        let insecure = NotifyConfig {
            webpush_allow_insecure: true,
            ..Default::default()
        };
        assert!(insecure.validate(false).is_ok());
        assert!(insecure.validate(true).is_err());
        assert!(NotifyConfig::default().validate(true).is_ok());
    }

    #[test]
    fn debug_output_hides_keys_and_credentials() {
        let cfg = NotifyConfig {
            vapid_private_key: "vapid-secret-value".into(),
            fcm_service_account_json: r#"{"private_key":"fcm-secret-value"}"#.into(),
            apns_key_p8: "apns-secret-value".into(),
            apns_key_id: "KEY1234567".into(),
            ..Default::default()
        };
        let printed = format!("{cfg:?}");
        assert!(!printed.contains("secret-value"), "{printed}");
        assert!(printed.contains("KEY1234567") && printed.contains("(set)"));
    }

    #[test]
    fn native_channels_need_complete_credentials() {
        let cfg = NotifyConfig::default();
        assert!(!cfg.fcm_enabled() && !cfg.apns_enabled());
        let partial = NotifyConfig {
            apns_key_p8: "key".into(),
            apns_key_id: "id".into(),
            ..Default::default()
        };
        assert!(
            !partial.apns_enabled(),
            "all four APNs settings are required"
        );
        let complete = NotifyConfig {
            apns_team_id: "team".into(),
            apns_topic: "app.tokuly".into(),
            ..partial
        };
        assert!(complete.apns_enabled());
        assert!(
            NotifyConfig {
                fcm_service_account_json: "{}".into(),
                ..Default::default()
            }
            .fcm_enabled()
        );
    }
}
