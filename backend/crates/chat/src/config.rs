//! Chat-specific settings.
//!
//! The server flattens every domain's settings into one command line, where
//! both the flag and clap's internal argument ID (by default the field name)
//! must be unique. Every argument therefore carries a `chat` prefix in both.
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct ChatConfig {
    // ── calls (LiveKit) ──
    /// LiveKit server API (Twirp) endpoint, e.g. `http://livekit:7880`.
    /// Calls are disabled unless this, the public URL, the key and the secret
    /// are all set.
    #[arg(id = "chat_livekit_url", long = "chat-livekit-url", env = "TOKULY_LIVEKIT_URL", default_value = "")]
    pub livekit_url: String,

    /// LiveKit URL clients connect to (`ws://` / `wss://`).
    #[arg(
        id = "chat_livekit_public_url",
        long = "chat-livekit-public-url",
        env = "TOKULY_LIVEKIT_PUBLIC_URL",
        default_value = ""
    )]
    pub livekit_public_url: String,

    #[arg(
        id = "chat_livekit_api_key",
        long = "chat-livekit-api-key",
        env = "TOKULY_LIVEKIT_API_KEY",
        default_value = "",
        hide_env_values = true
    )]
    pub livekit_api_key: String,

    #[arg(
        id = "chat_livekit_api_secret",
        long = "chat-livekit-api-secret",
        env = "TOKULY_LIVEKIT_API_SECRET",
        default_value = "",
        hide_env_values = true
    )]
    pub livekit_api_secret: String,

    /// Seconds an unanswered call rings.
    #[arg(id = "chat_ring_seconds", long = "chat-ring-seconds", env = "TOKULY_CHAT_RING_SECONDS", default_value_t = 45)]
    pub ring_seconds: i64,

    // ── AI assistant (Anthropic) ──
    /// Anthropic API key. Empty disables the AI assistant.
    #[arg(
        id = "chat_anthropic_api_key",
        long = "chat-anthropic-api-key",
        env = "TOKULY_ANTHROPIC_API_KEY",
        default_value = "",
        hide_env_values = true
    )]
    pub anthropic_api_key: String,

    /// Base URL of the Anthropic API (tests point this at a local stub).
    #[arg(
        id = "chat_anthropic_base_url",
        long = "chat-anthropic-base-url",
        env = "TOKULY_ANTHROPIC_BASE_URL",
        default_value = "https://api.anthropic.com"
    )]
    pub anthropic_base_url: String,

    /// Value of the `anthropic-version` header.
    #[arg(
        id = "chat_anthropic_version",
        long = "chat-anthropic-version",
        env = "TOKULY_ANTHROPIC_VERSION",
        default_value = "2023-06-01"
    )]
    pub anthropic_version: String,

    /// Model used by the AI assistant.
    #[arg(id = "chat_ai_model", long = "chat-ai-model", env = "TOKULY_AI_MODEL", default_value = "claude-opus-5-5")]
    pub ai_model: String,

    /// `output_config.effort` sent with every request (`low`, `medium`,
    /// `high`…). Rewrites and translations are short, simple tasks. Empty
    /// omits the parameter, for models that do not accept it.
    #[arg(id = "chat_ai_effort", long = "chat-ai-effort", env = "TOKULY_AI_EFFORT", default_value = "low")]
    pub ai_effort: String,

    /// Ask the API to retry a request its safety classifiers declined on
    /// the fallback model Anthropic recommends (`fallbacks: "default"`).
    /// Switch off for models or platforms without server-side fallbacks.
    #[arg(
        id = "chat_ai_refusal_fallback",
        long = "chat-ai-refusal-fallback",
        env = "TOKULY_AI_REFUSAL_FALLBACK",
        default_value_t = true,
        action = clap::ArgAction::Set
    )]
    pub ai_refusal_fallback: bool,

    /// AI assistant requests one user may make per hour.
    #[arg(id = "chat_ai_hourly_limit", long = "chat-ai-hourly-limit", env = "TOKULY_AI_HOURLY_LIMIT", default_value_t = 60)]
    pub ai_hourly_limit: u32,

    /// Seconds to wait for one answer of the AI service.
    #[arg(
        id = "chat_ai_timeout_seconds",
        long = "chat-ai-timeout-seconds",
        env = "TOKULY_AI_TIMEOUT_SECONDS",
        default_value_t = 45
    )]
    pub ai_timeout_seconds: u64,

    // ── limits ──
    /// Members a group may have.
    #[arg(
        id = "chat_max_group_members",
        long = "chat-max-group-members",
        env = "TOKULY_CHAT_MAX_GROUP_MEMBERS",
        default_value_t = 100
    )]
    pub max_group_members: i32,

    /// Seconds after sending during which a message's text can be edited.
    #[arg(
        id = "chat_edit_window_seconds",
        long = "chat-edit-window-seconds",
        env = "TOKULY_CHAT_EDIT_WINDOW_SECONDS",
        default_value_t = 900
    )]
    pub edit_window_seconds: i64,

    /// Messages a non-friend may send before their request is accepted.
    #[arg(
        id = "chat_request_message_limit",
        long = "chat-request-message-limit",
        env = "TOKULY_CHAT_REQUEST_MESSAGE_LIMIT",
        default_value_t = 3
    )]
    pub request_message_limit: i32,

    /// Messages one user may send per minute (all conversations together).
    #[arg(
        id = "chat_messages_per_minute",
        long = "chat-messages-per-minute",
        env = "TOKULY_CHAT_MESSAGES_PER_MINUTE",
        default_value_t = 120
    )]
    pub messages_per_minute: u32,

    /// Seconds a POV request can be answered.
    #[arg(
        id = "chat_pov_answer_seconds",
        long = "chat-pov-answer-seconds",
        env = "TOKULY_CHAT_POV_ANSWER_SECONDS",
        default_value_t = 7200
    )]
    pub pov_answer_seconds: i64,

    /// Seconds an unsaved POV answer lives.
    #[arg(
        id = "chat_pov_reply_seconds",
        long = "chat-pov-reply-seconds",
        env = "TOKULY_CHAT_POV_REPLY_SECONDS",
        default_value_t = 86400
    )]
    pub pov_reply_seconds: i64,
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            livekit_url: String::new(),
            livekit_public_url: String::new(),
            livekit_api_key: String::new(),
            livekit_api_secret: String::new(),
            ring_seconds: 45,
            anthropic_api_key: String::new(),
            anthropic_base_url: "https://api.anthropic.com".to_owned(),
            anthropic_version: "2023-06-01".to_owned(),
            ai_model: "claude-opus-5-5".to_owned(),
            ai_effort: "low".to_owned(),
            ai_refusal_fallback: true,
            ai_hourly_limit: 60,
            ai_timeout_seconds: 45,
            max_group_members: 100,
            edit_window_seconds: 900,
            request_message_limit: 3,
            messages_per_minute: 120,
            pov_answer_seconds: 7200,
            pov_reply_seconds: 86400,
        }
    }
}

impl ChatConfig {
    /// Settings read from the environment (`TOKULY_LIVEKIT_*`,
    /// `TOKULY_ANTHROPIC_*`…), with defaults for everything else. For tests
    /// and tools that have no command line of their own.
    pub fn from_env() -> Self {
        #[derive(clap::Parser)]
        struct Env {
            #[command(flatten)]
            chat: ChatConfig,
        }
        <Env as clap::Parser>::parse_from(["tokuly-chat"]).chat
    }

    /// Whether every LiveKit setting is present.
    pub fn calls_enabled(&self) -> bool {
        !self.livekit_url.is_empty()
            && !self.livekit_public_url.is_empty()
            && !self.livekit_api_key.is_empty()
            && !self.livekit_api_secret.is_empty()
    }

    pub fn assist_enabled(&self) -> bool {
        !self.anthropic_api_key.is_empty()
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (2..=1000).contains(&self.max_group_members),
            "TOKULY_CHAT_MAX_GROUP_MEMBERS must be between 2 and 1000"
        );
        anyhow::ensure!((5..=300).contains(&self.ring_seconds), "TOKULY_CHAT_RING_SECONDS must be between 5 and 300");
        anyhow::ensure!(self.edit_window_seconds >= 0, "TOKULY_CHAT_EDIT_WINDOW_SECONDS must not be negative");
        anyhow::ensure!(self.request_message_limit >= 1, "TOKULY_CHAT_REQUEST_MESSAGE_LIMIT must be at least 1");
        anyhow::ensure!(self.messages_per_minute >= 1, "TOKULY_CHAT_MESSAGES_PER_MINUTE must be at least 1");
        anyhow::ensure!(
            self.pov_answer_seconds > 0 && self.pov_reply_seconds > 0,
            "the POV answer and reply lifetimes must be positive"
        );
        anyhow::ensure!(self.ai_timeout_seconds >= 1, "TOKULY_AI_TIMEOUT_SECONDS must be at least 1");
        let livekit =
            [&self.livekit_url, &self.livekit_public_url, &self.livekit_api_key, &self.livekit_api_secret];
        let set = livekit.iter().filter(|value| !value.is_empty()).count();
        anyhow::ensure!(
            set == 0 || set == livekit.len(),
            "calls need all of TOKULY_LIVEKIT_URL, TOKULY_LIVEKIT_PUBLIC_URL, TOKULY_LIVEKIT_API_KEY and TOKULY_LIVEKIT_API_SECRET (or none, to disable calls)"
        );
        if self.assist_enabled() {
            anyhow::ensure!(!self.ai_model.is_empty(), "TOKULY_AI_MODEL must not be empty");
            anyhow::ensure!(
                self.anthropic_base_url.starts_with("http://") || self.anthropic_base_url.starts_with("https://"),
                "TOKULY_ANTHROPIC_BASE_URL must be an http(s) URL"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        chat: ChatConfig,
    }

    #[test]
    fn command_line_defaults_match_default_impl() {
        Cli::command_for_tests().debug_assert();
        // The environment may carry real LiveKit / Anthropic settings; compare
        // only what no environment variable of the dev setup touches.
        let parsed = Cli::parse_from(["test", "--chat-ring-seconds", "45"]).chat;
        let default = ChatConfig::default();
        assert_eq!(parsed.max_group_members, default.max_group_members);
        assert_eq!(parsed.edit_window_seconds, default.edit_window_seconds);
        assert_eq!(parsed.request_message_limit, default.request_message_limit);
        assert_eq!(parsed.pov_answer_seconds, default.pov_answer_seconds);
        assert_eq!(parsed.pov_reply_seconds, default.pov_reply_seconds);
        assert_eq!(parsed.ai_refusal_fallback, default.ai_refusal_fallback);
    }

    impl Cli {
        fn command_for_tests() -> clap::Command {
            <Cli as clap::CommandFactory>::command()
        }
    }

    #[test]
    fn features_need_their_whole_configuration() {
        let mut cfg = ChatConfig::default();
        assert!(!cfg.calls_enabled() && !cfg.assist_enabled());
        assert!(cfg.validate().is_ok());
        cfg.livekit_url = "http://127.0.0.1:7880".into();
        assert!(cfg.validate().is_err(), "a partial LiveKit configuration is a mistake, not a way to disable calls");
        cfg.livekit_public_url = "ws://127.0.0.1:7880".into();
        cfg.livekit_api_key = "key".into();
        cfg.livekit_api_secret = "secret".into();
        assert!(cfg.validate().is_ok() && cfg.calls_enabled());
        cfg.anthropic_api_key = "sk-test".into();
        assert!(cfg.assist_enabled() && cfg.validate().is_ok());
        cfg.anthropic_base_url = "ftp://example".into();
        assert!(cfg.validate().is_err());
    }
}
