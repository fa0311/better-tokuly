//! The settings of the binary: which sections exist, and resolving their
//! layers at start-up (see `tokuly_core::settings`).
use std::{collections::HashMap, ffi::OsString, path::PathBuf, sync::Arc};

use clap::{Args, CommandFactory as _, FromArgMatches as _};
use tokuly_admin::AdminConfig;
use tokuly_ask::AskConfig;
use tokuly_chat::ChatConfig;
use tokuly_community::CommunityConfig;
use tokuly_core::{
    config::Config,
    keys::Keys,
    settings::{self, Layers, Process, Registry, SectionSpec, Source, State},
};
use tokuly_dev::DevConfig;
use tokuly_emoji::EmojiConfig;
use tokuly_engagement::EngagementConfig;
use tokuly_feed::FeedConfig;
use tokuly_geo::GeoConfig;
use tokuly_identity::IdentityConfig;
use tokuly_live::LiveConfig;
use tokuly_media::MediaConfig;
use tokuly_notes::NotesConfig;
use tokuly_notify::NotifyConfig;
use tokuly_plans::PlansConfig;
use tokuly_quest::QuestConfig;
use tokuly_realtime::RealtimeConfig;
use tokuly_school::SchoolConfig;
use tokuly_search::SearchConfig;
use tokuly_shorts::ShortsConfig;
use tokuly_social::SocialConfig;
use tokuly_spaces::SpacesConfig;
use tokuly_stamp::StampConfig;
use tokuly_video::VideoConfig;
use tokuly_wallet::WalletConfig;

use crate::Cli;

/// The variable naming the `tokuly.toml` to read.
pub const CONFIG_ENV: &str = "TOKULY_CONFIG";

/// Settings of the API listener.
#[derive(Args, Clone, Debug)]
pub struct ApiConfig {
    /// Address the API listens on.
    #[arg(long, env = "TOKULY_API_LISTEN", default_value = "0.0.0.0:8081")]
    pub listen: std::net::SocketAddr,
    /// Extra origins allowed to call the API from browsers (comma separated).
    /// The public URL is always allowed; in development so is any localhost port.
    #[arg(long, env = "TOKULY_CORS_ORIGINS", value_delimiter = ',', default_value = "")]
    pub cors_origins: Vec<String>,
}

const ALL: &[Process] = &[Process::Api, Process::Realtime, Process::Worker];

/// What the console must not change in `[core]`: where the infrastructure
/// is, its credentials and the master secret.
const CORE_BOOTSTRAP: &[&str] = &[
    "public_url",
    "api_path_prefix",
    "secret",
    "env",
    "database_url",
    "database_max_connections",
    "valkey_url",
    "nats_url",
    "nats_prefix",
    "s3_endpoint",
    "s3_public_endpoint",
    "s3_region",
    "s3_access_key",
    "s3_secret_key",
    "s3_media_bucket",
    "s3_private_bucket",
    "media_url",
    "clickhouse_url",
    "clickhouse_user",
    "clickhouse_password",
    "clickhouse_database",
    "meili_url",
    "meili_key",
];

/// The AI provider settings, which the staff console edits on its own
/// screen (with the environment as initial values).
const CHAT_AI_SCREEN: &[(&str, &str)] = &[
    ("anthropic_api_key", "/admin/chat-ai"),
    ("anthropic_base_url", "/admin/chat-ai"),
    ("ai_model", "/admin/chat-ai"),
    ("ai_default_provider", "/admin/chat-ai"),
    ("openai_api_key", "/admin/chat-ai"),
    ("openai_base_url", "/admin/chat-ai"),
    ("openai_model", "/admin/chat-ai"),
    ("openai_compat_", "/admin/chat-ai"),
];

/// The live media limits: read per upload and per transcode.
const MEDIA_LIVE: &[&str] = &[
    "max_avatar_mb",
    "max_image_mb",
    "max_clip_mb",
    "max_video_mb",
    "max_short_mb",
    "max_audio_mb",
    "max_file_mb",
    "max_unused_gb",
    "daily_upload_gb",
    "max_video_hours",
    "transcodes_per_account",
    "daily_transcode_minutes",
];

/// A section check calling the domain's `validate()`.
macro_rules! check {
    ($config:ty) => {
        |matches, _production| <$config>::from_arg_matches(matches)?.validate()
    };
    ($config:ty, production) => {
        |matches, production| <$config>::from_arg_matches(matches)?.validate(production)
    };
}

/// Every section, in the order of the example file and the console.
pub fn sections() -> Vec<SectionSpec> {
    vec![
        SectionSpec::new("core", Config::augment_args).processes(ALL).live(&["rate_limits"]).bootstrap(CORE_BOOTSTRAP).check(check!(Config)),
        SectionSpec::new("api", ApiConfig::augment_args).processes(&[Process::Api]).bootstrap(&["listen"]),
        SectionSpec::new("realtime", RealtimeConfig::augment_args)
            .env_prefix("RT")
            .processes(&[Process::Realtime])
            .bootstrap(&["wt_listen", "ws_listen", "tls_cert", "tls_key"])
            .check(check!(RealtimeConfig)),
        SectionSpec::new("identity", IdentityConfig::augment_args),
        SectionSpec::new("media", MediaConfig::augment_args).live(MEDIA_LIVE).check(check!(MediaConfig)),
        SectionSpec::new("social", SocialConfig::augment_args),
        SectionSpec::new("geo", GeoConfig::augment_args).check(check!(GeoConfig)),
        SectionSpec::new("stamp", StampConfig::augment_args),
        SectionSpec::new("wallet", WalletConfig::augment_args).check(check!(WalletConfig)),
        SectionSpec::new("notify", NotifyConfig::augment_args).check(check!(NotifyConfig, production)),
        SectionSpec::new("admin", AdminConfig::augment_args).bootstrap(&["user"]).check(check!(AdminConfig)),
        SectionSpec::new("feed", FeedConfig::augment_args).check(check!(FeedConfig, production)),
        SectionSpec::new("chat", ChatConfig::augment_args).managed(CHAT_AI_SCREEN).check(check!(ChatConfig)),
        SectionSpec::new("notes", NotesConfig::augment_args).check(check!(NotesConfig)),
        SectionSpec::new("video", VideoConfig::augment_args).check(check!(VideoConfig)),
        SectionSpec::new("search", SearchConfig::augment_args).check(check!(SearchConfig)),
        SectionSpec::new("emoji", EmojiConfig::augment_args).check(check!(EmojiConfig)),
        SectionSpec::new("ask", AskConfig::augment_args).check(check!(AskConfig)),
        SectionSpec::new("plans", PlansConfig::augment_args),
        SectionSpec::new("school", SchoolConfig::augment_args),
        SectionSpec::new("quest", QuestConfig::augment_args).check(check!(QuestConfig)),
        SectionSpec::new("live", LiveConfig::augment_args).check(check!(LiveConfig)),
        SectionSpec::new("shorts", ShortsConfig::augment_args).check(check!(ShortsConfig)),
        SectionSpec::new("spaces", SpacesConfig::augment_args).check(check!(SpacesConfig)),
        SectionSpec::new("community", CommunityConfig::augment_args).check(check!(CommunityConfig)),
        SectionSpec::new("dev", DevConfig::augment_args).check(check!(DevConfig, production)),
        SectionSpec::new("engagement", EngagementConfig::augment_args),
    ]
}

pub fn registry() -> anyhow::Result<Registry> {
    Registry::new(sections())
}

/// The command line, the environment and `tokuly.toml`, and the console
/// values as read (checked when the state is made).
pub struct Resolved {
    pub registry: Arc<Registry>,
    pub layers: Layers,
    pub file: Option<PathBuf>,
    pub console: HashMap<String, String>,
}

impl Resolved {
    /// Reads every layer but the console's. `argv` is the process's arguments.
    pub fn read(argv: &[OsString]) -> anyhow::Result<Self> {
        let registry = Arc::new(registry()?);
        let mut layers = Layers::default();
        // Lenient: only to learn which settings the command line gives.
        let lenient = Cli::command().mut_args(|a| a.required(false)).ignore_errors(true);
        if let Ok(matches) = lenient.try_get_matches_from(argv) {
            layers.read_cli(&registry, &matches);
        }
        layers.read_env(&registry, |name| std::env::var(name).ok());
        let file = std::env::var_os(CONFIG_ENV).filter(|p| !p.is_empty()).map(PathBuf::from);
        let file = match file {
            Some(path) => match std::fs::read_to_string(&path) {
                Ok(text) => {
                    layers.file = settings::file::parse(&registry, &text, &path.display().to_string())?;
                    Some(path)
                }
                // Optional: Compose names the file whether or not the operator wrote one.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    eprintln!("tokuly: {} does not exist; running without a settings file", path.display());
                    None
                }
                Err(e) => anyhow::bail!("cannot read {} ({CONFIG_ENV}): {e}", path.display()),
            },
            None => None,
        };
        Ok(Self { registry, layers, file, console: HashMap::new() })
    }

    /// The value in force of a `[core]` setting, before the console layer.
    fn core_value(&self, key: &str) -> Option<String> {
        let def = self.registry.get("core", key)?;
        self.layers.effective(def).0.map(str::to_owned)
    }

    /// Adds the console layer from the database (absent before the first migration).
    pub async fn read_console(&mut self) -> anyhow::Result<()> {
        let (Some(url), Some(secret)) = (self.core_value("database_url"), self.core_value("secret")) else {
            return Ok(());
        };
        let db = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(15))
            .connect(&url)
            .await
            .map_err(|e| anyhow::anyhow!("cannot connect to PostgreSQL to read the console settings: {e}"))?;
        self.console = settings::store::load(&db, &self.registry, &Keys::new(&secret)).await?;
        db.close().await;
        Ok(())
    }

    fn production(&self) -> bool {
        self.core_value("env").as_deref() == Some("production")
    }

    /// What the process starts with: the console values of a section that
    /// does not pass its checks with them are set aside (and logged).
    pub fn state(&self) -> State {
        State::with_console(self.registry.clone(), self.layers.clone(), self.console.clone(), self.file.clone(), self.production())
    }

    /// The last resort when the server does not start with the console
    /// values at all: every one of them set aside, with `reason`.
    pub fn state_without_console(&self, reason: &str) -> State {
        let mut state = State::new(self.registry.clone(), self.layers.clone(), self.file.clone(), self.production());
        state.rejected_sections = self.console.keys().filter_map(|id| self.registry.by_id(id)).map(|d| (d.section.to_owned(), reason.to_owned())).collect();
        state.rejected = self.console.clone();
        state
    }
}

/// Parses the command line with every layer of `state` below it as defaults.
pub fn parse(argv: &[OsString], state: &State) -> Cli {
    let cmd = settings::apply(Cli::command(), &state.registry, &state.layers, false);
    let matches = cmd.try_get_matches_from(argv).unwrap_or_else(|e| e.exit());
    Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit())
}

impl Resolved {
    /// The settings in force (secrets left out) as a `tokuly.toml` with the source of each.
    pub fn show(&self) -> String {
        let values = self
            .registry
            .defs()
            .iter()
            .filter(|d| !d.secret)
            .filter_map(|d| match self.layers.effective(d) {
                (Some(v), source) if source != Source::Default => Some((d.id.clone(), (v.to_owned(), format!("from {}", source.label())))),
                _ => None,
            })
            .collect();
        settings::file::export(
            &self.registry,
            &values,
            "The settings in force from the command line, the environment and tokuly.toml\n(console values and secrets are not shown).",
        )
    }
}

#[cfg(test)]
mod tests {
    use tokuly_core::settings::Kind;

    use super::*;

    /// Environment variables that hold credentials: each must be a secret
    /// (`hide_env_values`), so the console never shows or exports it.
    #[test]
    fn credentials_are_secrets() {
        let registry = registry().unwrap();
        let credential = |env: &str| ["SECRET", "PASSWORD", "API_KEY", "PRIVATE_KEY", "KEY_P8", "SERVICE_ACCOUNT", "MEILI_KEY", "DATABASE_URL", "SMTP_URL"].iter().any(|w| env.contains(w));
        let missing: Vec<&str> = registry.defs().iter().filter(|d| credential(&d.env) && d.kind == Kind::Text && !d.secret).map(|d| d.env.as_str()).collect();
        assert!(missing.is_empty(), "add `hide_env_values = true` to {missing:?}");
    }

    /// The start-up path: a console value the mailer would refuse does
    /// not reach Core::connect; its section's console values are set aside.
    #[test]
    fn console_values_the_server_cannot_start_with_are_set_aside() {
        let mut resolved = Resolved { registry: Arc::new(registry().unwrap()), layers: Layers::default(), file: None, console: HashMap::new() };
        resolved.layers.cli.insert("secret".into(), "0123456789abcdef0123456789abcdef".into());
        resolved.console.insert("mail_from".into(), "not a mailbox".into());
        resolved.console.insert("max_video_hours".into(), "4".into());
        let state = resolved.state();
        assert_eq!(state.layers.console.get("max_video_hours").map(String::as_str), Some("4"));
        assert!(!state.layers.console.contains_key("mail_from"));
        let mail_from = state.registry.get("core", "mail_from").unwrap();
        assert!(state.rejection(mail_from).is_some_and(|r| r.contains("TOKULY_MAIL_FROM")), "{:?}", state.rejection(mail_from));
        // And the command line parses without it.
        let argv: Vec<OsString> = ["tokuly", "api"].iter().map(Into::into).collect();
        let cli = parse(&argv, &state);
        assert_eq!(cli.core.mail_from, "Tokuly <no-reply@tokuly.localhost>");
        assert_eq!(cli.domains.media.max_video_hours, 4);
    }

    #[test]
    fn every_setting_has_a_description() {
        let registry = registry().unwrap();
        let missing: Vec<&str> = registry.defs().iter().filter(|d| d.help.is_empty()).map(|d| d.env.as_str()).collect();
        assert!(missing.is_empty(), "document {missing:?}");
    }

    /// `tokuly.example.toml` is what `tokuly config example` writes (regenerate with
    /// `cargo run -p tokuly-server -- config example > ../tokuly.example.toml`).
    #[test]
    fn the_example_file_is_current() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../tokuly.example.toml");
        let current = std::fs::read_to_string(path).unwrap_or_default();
        let generated = settings::file::example(&registry().unwrap());
        assert!(current == generated, "tokuly.example.toml is out of date: run `tokuly config example > tokuly.example.toml` from the repository root");
    }

    #[test]
    fn the_example_file_parses_when_uncommented() {
        let registry = registry().unwrap();
        let example = settings::file::example(&registry);
        // Each setting's last comment line is `# key = default`.
        let mut text = String::new();
        let mut previous = "";
        for line in example.lines() {
            if line.starts_with('[') {
                text.push_str(line);
                text.push('\n');
            }
            let setting = |l: &str| l.starts_with("# ") && !l.starts_with("# (") && l.contains(" = ") && !l.ends_with("# not set by default");
            if line.is_empty() && setting(previous) {
                text.push_str(previous.trim_start_matches("# "));
                text.push('\n');
            }
            previous = line;
        }
        if previous.starts_with("# ") && !previous.ends_with("# not set by default") {
            text.push_str(previous.trim_start_matches("# "));
        }
        let values = settings::file::parse(&registry, &text, "example").unwrap();
        assert!(values.len() > 300, "{} settings parsed", values.len());
    }
}
