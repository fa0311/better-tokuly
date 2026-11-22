//! [`Core`]: the bundle of shared infrastructure handed to every domain.
use std::{ops::Deref, sync::Arc};

use futures::StreamExt as _;
use serde::Serialize;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    analytics::Analytics,
    auth::{AuthLayer, Tokens},
    bus::Bus, cache::Cache, config::Config, db,
    error::AppResult, events, export::Exporters, flags::Flags, jobs::Jobs, keys::Keys,
    livekit::LiveKit,
    mail::Mailer,
    realtime::{Realtime, TopicGuards},
    storage::Storage,
    subject::Subjects,
};

const SESSION_REVOKED_SUBJECT: &str = "sys.session.revoked";

pub struct CoreInner {
    pub cfg: Config,
    pub keys: Keys,
    pub db: PgPool,
    pub cache: Cache,
    pub bus: Bus,
    pub rt: Realtime,
    pub jobs: Jobs,
    pub storage: Storage,
    pub tokens: Tokens,
    /// State for [`crate::auth::auth_middleware`] on every HTTP router serving RPCs.
    pub auth_layer: AuthLayer,
    pub mail: Mailer,
    pub analytics: Analytics,
    pub search: meilisearch_sdk::client::Client,
    pub http: reqwest::Client,
    /// Voice and video rooms; not configured unless the `TOKULY_LIVEKIT_*`
    /// settings are present.
    pub livekit: LiveKit,
    pub subjects: Subjects,
    pub topic_guards: TopicGuards,
    pub exporters: Exporters,
    pub flags: Flags,
    /// Cancelled when the process is asked to stop; background loops watch it.
    pub shutdown: CancellationToken,
}

/// Cheap to clone; every domain service holds one.
#[derive(Clone)]
pub struct Core(Arc<CoreInner>);

impl Deref for Core {
    type Target = CoreInner;

    fn deref(&self) -> &CoreInner {
        &self.0
    }
}

impl Core {
    /// Connects to every backing service. Fails fast with a readable error
    /// when a required dependency is unreachable.
    pub async fn connect(cfg: Config) -> anyhow::Result<Self> {
        cfg.validate()?;
        let keys = Keys::new(&cfg.secret);
        let db = db::connect(&cfg.database_url, cfg.database_max_connections).await?;
        let cache = Cache::connect(&cfg.valkey_url).await?.with_rate_limits(cfg.rate_limits);
        if !cfg.rate_limits {
            tracing::warn!("request rate limits are OFF (TOKULY_RATE_LIMITS=false): never expose this deployment to the internet");
        }
        let bus = Bus::connect(&cfg.nats_url, &cfg.nats_prefix).await?;
        bus.ensure_streams().await?;
        let tokens = Tokens::new(&keys)?;
        let auth_layer = AuthLayer::new(tokens.clone(), cfg.trusted_proxies()?);
        let storage = Storage::new(&cfg);
        let mail = Mailer::new(&cfg.smtp_url, &cfg.mail_from)?;
        let analytics = Analytics::new(bus.clone(), &cfg);
        let search = meilisearch_sdk::client::Client::new(&cfg.meili_url, Some(&cfg.meili_key))
            .map_err(|e| anyhow::anyhow!("invalid Meilisearch configuration: {e}"))?;
        let http = reqwest::Client::builder()
            .user_agent("Tokuly/1.0")
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| anyhow::anyhow!("http client: {e}"))?;
        let livekit = LiveKit::from_config(&cfg)?;
        let core = Self(Arc::new(CoreInner {
            rt: Realtime::new(bus.clone()),
            jobs: Jobs::new(bus.clone()),
            flags: Flags::new(db.clone()),
            exporters: Exporters::default(),
            topic_guards: TopicGuards::default(),
            cfg,
            keys,
            db,
            cache,
            bus,
            storage,
            tokens,
            auth_layer,
            mail,
            analytics,
            search,
            http,
            livekit,
            subjects: Subjects::default(),
            shutdown: CancellationToken::new(),
        }));
        core.spawn_system_listeners().await?;
        Ok(core)
    }

    /// Process-wide listeners every service needs (session revocations).
    async fn spawn_system_listeners(&self) -> anyhow::Result<()> {
        let mut revoked = self
            .bus
            .subscribe(SESSION_REVOKED_SUBJECT)
            .await
            .map_err(|e| anyhow::anyhow!("cannot subscribe to system subjects: {e}"))?;
        let core = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = core.shutdown.cancelled() => break,
                    message = revoked.next() => {
                        let Some(message) = message else { break };
                        if let Some(id) = std::str::from_utf8(&message.payload).ok().and_then(|s| Uuid::parse_str(s).ok()) {
                            core.tokens.revoke_session(id);
                        }
                    }
                }
            }
        });
        Ok(())
    }

    /// Invalidates a session's access tokens on every service instance.
    pub async fn revoke_session(&self, session_id: Uuid) {
        self.tokens.revoke_session(session_id);
        if let Err(e) = self
            .bus
            .publish(SESSION_REVOKED_SUBJECT, session_id.to_string())
            .await
        {
            tracing::warn!(error = %e, "session revocation broadcast failed");
        }
    }

    /// Emits a domain event outside of a transaction. Prefer
    /// [`events::emit`] with the open transaction when one exists.
    pub async fn emit<T: Serialize + ?Sized>(
        &self,
        topic: &str,
        actor_id: Option<Uuid>,
        data: &T,
    ) -> AppResult<()> {
        events::emit(&self.db, topic, actor_id, data).await
    }

    /// Absolute URL of a path in the web app.
    pub fn web_url(&self, path: &str) -> String {
        format!("{}/{}", self.cfg.public_url(), path.trim_start_matches('/'))
    }
}
