//! Process configuration.
//!
//! Every setting is an environment variable (prefixed `TOKULY_`) and also a
//! command-line flag, with defaults that work for local development. Domain
//! crates define their own `clap::Args` structs for domain-specific settings
//! and the server binary flattens them next to this one.
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct Config {
    /// Public origin of the web app (scheme + host [+ port], no trailing slash).
    #[arg(
        long,
        env = "TOKULY_PUBLIC_URL",
        default_value = "http://localhost:8080"
    )]
    pub public_url: String,

    /// Path prefix under which the edge proxy exposes the API to browsers
    /// (it strips the prefix before forwarding). Cookies scoped to RPC paths
    /// must include it. Empty when the API is served at the origin root.
    #[arg(long, env = "TOKULY_API_PATH_PREFIX", default_value = "/api")]
    pub api_path_prefix: String,

    /// Master secret (>= 32 chars). Signing keys, token peppers and cursor MACs
    /// are derived from it, so every service must share the same value.
    #[arg(long, env = "TOKULY_SECRET", hide_env_values = true)]
    pub secret: String,

    /// Deployment environment name: "development" | "production".
    #[arg(long, env = "TOKULY_ENV", default_value = "development")]
    pub env: String,

    /// Enforce the per-user and per-address request limits that guard against
    /// abuse (sign-ups per hour, messages per minute, ...). Turn off only for
    /// local development, automated browser tests and load tests, where one
    /// address legitimately creates many accounts.
    #[arg(long, env = "TOKULY_RATE_LIMITS", default_value_t = true, action = clap::ArgAction::Set)]
    pub rate_limits: bool,

    /// Proxies in front of the API whose `X-Forwarded-For` header names the
    /// real client (addresses or CIDR ranges, comma separated). From any
    /// other peer the header is ignored. The default covers an edge proxy on
    /// the same host or container network; add the ranges of a CDN in front
    /// of it, or set it empty if the API is exposed directly.
    #[arg(long, env = "TOKULY_TRUSTED_PROXIES", value_delimiter = ',', default_value = crate::proxies::DEFAULT_TRUSTED_PROXIES)]
    pub trusted_proxies: Vec<String>,

    // ── PostgreSQL ──
    /// PostgreSQL connection URL (with the password).
    #[arg(
        long,
        env = "TOKULY_DATABASE_URL",
        default_value = "postgres://tokuly:tokuly@localhost:5432/tokuly",
        hide_env_values = true
    )]
    pub database_url: String,
    /// Connections each process keeps in its PostgreSQL pool.
    #[arg(long, env = "TOKULY_DATABASE_MAX_CONNECTIONS", default_value_t = 20)]
    pub database_max_connections: u32,

    // ── Valkey ──
    /// Valkey (Redis protocol) URL: caches, rate limits, short-lived tokens.
    #[arg(
        long,
        env = "TOKULY_VALKEY_URL",
        default_value = "redis://localhost:6379",
        hide_env_values = true
    )]
    pub valkey_url: String,

    // ── NATS ──
    /// NATS URL: events, jobs and realtime fan-out.
    #[arg(
        long,
        env = "TOKULY_NATS_URL",
        default_value = "nats://localhost:4222",
        hide_env_values = true
    )]
    pub nats_url: String,

    /// Namespace for NATS subjects and streams, so several deployments can
    /// share one NATS server. Letters, digits and underscores only.
    #[arg(long, env = "TOKULY_NATS_PREFIX", default_value = "")]
    pub nats_prefix: String,

    // ── Object storage (S3 API) ──
    /// Endpoint the backend uses to reach the object store.
    #[arg(
        long,
        env = "TOKULY_S3_ENDPOINT",
        default_value = "http://localhost:9000"
    )]
    pub s3_endpoint: String,
    /// Endpoint browsers and apps use for presigned uploads/downloads.
    /// Defaults to `TOKULY_S3_ENDPOINT`.
    #[arg(long, env = "TOKULY_S3_PUBLIC_ENDPOINT")]
    pub s3_public_endpoint: Option<String>,
    /// Region the object store signs requests for.
    #[arg(long, env = "TOKULY_S3_REGION", default_value = "us-east-1")]
    pub s3_region: String,
    /// Access key of the object store.
    #[arg(
        long,
        env = "TOKULY_S3_ACCESS_KEY",
        default_value = "tokuly",
        hide_env_values = true
    )]
    pub s3_access_key: String,
    /// Secret key of the object store.
    #[arg(
        long,
        env = "TOKULY_S3_SECRET_KEY",
        default_value = "tokuly-secret",
        hide_env_values = true
    )]
    pub s3_secret_key: String,
    /// Bucket for processed, publicly served media.
    #[arg(long, env = "TOKULY_S3_MEDIA_BUCKET", default_value = "tokuly-media")]
    pub s3_media_bucket: String,
    /// Bucket for original uploads and private files (presigned access only).
    #[arg(
        long,
        env = "TOKULY_S3_PRIVATE_BUCKET",
        default_value = "tokuly-private"
    )]
    pub s3_private_bucket: String,
    /// Base URL under which the media bucket is served to clients.
    /// Defaults to `{public_url}/media`.
    #[arg(long, env = "TOKULY_MEDIA_URL")]
    pub media_url: Option<String>,

    // ── ClickHouse ──
    /// ClickHouse HTTP URL (product analytics).
    #[arg(
        long,
        env = "TOKULY_CLICKHOUSE_URL",
        default_value = "http://localhost:8123"
    )]
    pub clickhouse_url: String,
    /// ClickHouse user.
    #[arg(long, env = "TOKULY_CLICKHOUSE_USER", default_value = "tokuly")]
    pub clickhouse_user: String,
    /// ClickHouse password.
    #[arg(
        long,
        env = "TOKULY_CLICKHOUSE_PASSWORD",
        default_value = "tokuly",
        hide_env_values = true
    )]
    pub clickhouse_password: String,
    /// ClickHouse database holding the analytics tables.
    #[arg(long, env = "TOKULY_CLICKHOUSE_DATABASE", default_value = "tokuly")]
    pub clickhouse_database: String,

    // ── Meilisearch ──
    /// Meilisearch URL (search).
    #[arg(
        long,
        env = "TOKULY_MEILI_URL",
        default_value = "http://localhost:7700"
    )]
    pub meili_url: String,
    /// Meilisearch master key.
    #[arg(
        long,
        env = "TOKULY_MEILI_KEY",
        default_value = "tokuly-meili-key",
        hide_env_values = true
    )]
    pub meili_key: String,

    // ── Mail ──
    /// SMTP URL, e.g. `smtp://mailpit:1025` or `smtps://user:pass@smtp.example.com`.
    /// Empty disables outgoing mail (messages are logged instead).
    #[arg(
        long,
        env = "TOKULY_SMTP_URL",
        default_value = "",
        hide_env_values = true
    )]
    pub smtp_url: String,
    /// Sender of outgoing mail (`Name <address>`).
    #[arg(
        long,
        env = "TOKULY_MAIL_FROM",
        default_value = "Tokuly <no-reply@tokuly.localhost>"
    )]
    pub mail_from: String,

    // ── LiveKit (voice and video) ──
    /// LiveKit server API (Twirp) endpoint, e.g. `http://livekit:7880`.
    /// Voice and video are off unless this, the public URL, the key and the
    /// secret are all set (see `tokuly_core::livekit`).
    #[arg(long, env = "TOKULY_LIVEKIT_URL", default_value = "")]
    pub livekit_url: String,
    /// LiveKit URL clients connect to (`ws://` / `wss://`).
    #[arg(long, env = "TOKULY_LIVEKIT_PUBLIC_URL", default_value = "")]
    pub livekit_public_url: String,
    /// LiveKit API key.
    #[arg(long, env = "TOKULY_LIVEKIT_API_KEY", default_value = "", hide_env_values = true)]
    pub livekit_api_key: String,
    /// LiveKit API secret.
    #[arg(long, env = "TOKULY_LIVEKIT_API_SECRET", default_value = "", hide_env_values = true)]
    pub livekit_api_secret: String,

    // ── Telemetry ──
    /// OTLP gRPC endpoint (e.g. `http://otel:4317`). Empty disables export.
    #[arg(long, env = "TOKULY_OTLP_ENDPOINT", default_value = "")]
    pub otlp_endpoint: String,
    /// "pretty" | "json".
    #[arg(long, env = "TOKULY_LOG_FORMAT", default_value = "pretty")]
    pub log_format: String,
}

impl Config {
    pub fn is_production(&self) -> bool {
        self.env == "production"
    }

    /// The API path prefix without a trailing slash (`""` or e.g. `"/api"`).
    pub fn api_path_prefix(&self) -> &str {
        self.api_path_prefix.trim_end_matches('/')
    }

    pub fn public_url(&self) -> &str {
        self.public_url.trim_end_matches('/')
    }

    /// Base URL of publicly served media, without trailing slash.
    pub fn media_url(&self) -> String {
        match &self.media_url {
            Some(url) if !url.is_empty() => url.trim_end_matches('/').to_owned(),
            _ => format!("{}/media", self.public_url()),
        }
    }

    pub fn s3_public_endpoint(&self) -> &str {
        match &self.s3_public_endpoint {
            Some(url) if !url.is_empty() => url,
            _ => &self.s3_endpoint,
        }
    }

    /// Host part of the public URL (the WebAuthn relying party ID).
    pub fn public_host(&self) -> String {
        url::Url::parse(self.public_url())
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_else(|| "localhost".to_owned())
    }

    /// The parsed `TOKULY_TRUSTED_PROXIES`.
    pub fn trusted_proxies(&self) -> anyhow::Result<crate::proxies::TrustedProxies> {
        crate::proxies::TrustedProxies::parse(&self.trusted_proxies).map_err(|e| anyhow::anyhow!("TOKULY_TRUSTED_PROXIES: {e}"))
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        self.trusted_proxies()?;
        anyhow::ensure!(
            self.secret.len() >= 32,
            "TOKULY_SECRET must be at least 32 characters (generate one with `openssl rand -hex 32`)"
        );
        url::Url::parse(self.public_url())
            .map_err(|e| anyhow::anyhow!("TOKULY_PUBLIC_URL is not a valid URL: {e}"))?;
        // What `Core::connect` builds from the settings without the network,
        // built here too: a console change is checked with this before it is
        // saved, and a value the server cannot start with must be refused.
        crate::mail::Mailer::new(&self.smtp_url, &self.mail_from)?;
        crate::livekit::LiveKit::from_config(self)?;
        Ok(())
    }
}

impl Config {
    /// The settings the staff console (「サーバー設定」) may change, by
    /// `tokuly.toml` key. Everything else is set in tokuly.toml or the
    /// environment only: addresses, credentials, egress guards, paths,
    /// money and privacy floors never belong here (backend/README.md,
    /// "Declaring a setting").
    pub const CONSOLE_EDITABLE: &'static [&'static str] = &[
        "mail_from",
    ];
}
