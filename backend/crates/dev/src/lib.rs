//! Developer platform: the API opened to the people who use Tokuly.
//!
//! A map of the crate, by file:
//!
//! - **Credentials**: every secret has one format (`secrets.rs`). Personal
//!   access tokens (`tokens.rs`), OAuth access tokens and bot tokens are
//!   bearer tokens for the whole API; `verify.rs` is the verifier that
//!   core's auth middleware asks on every request made with one
//!   (`tokuly_core::api_tokens`), answering from Valkey. What a token may
//!   do is its scopes; `scopes.rs` is the catalogue with the wording people
//!   agree to.
//! - **OAuth 2.1** (`apps.rs` registers applications; `oauth.rs` holds the
//!   protocol: authorization requests, codes with PKCE, token issue and
//!   rotation, the device flow; `consent_svc.rs` is the user's side over
//!   RPC, `oauth_http.rs` the endpoints OAuth libraries call).
//! - **Bots** (`bots.rs`): accounts owned by a person; `transfers.rs`
//!   hands one over to another person.
//! - **Webhooks** (`webhooks.rs` subscriptions and the event catalogue,
//!   `fanout.rs` from domain events to deliveries, `delivery.rs` sending,
//!   signing, retries and the guard against requests into private networks).
//! - **API reference** (`reference.rs`, from the descriptor set the server
//!   embeds) and usage statistics (`usage.rs`, from ClickHouse).
//! - `cleanup.rs`: account bans and deletions, moderation takedowns, the
//!   data export and the report subject; `workers.rs` starts the loops.
//!
//! Wiring, done by the server: [`Dev::new`], [`Dev::register`],
//! [`Dev::http_routes`] merged into the HTTP router (inside the auth
//! middleware), [`Dev::spawn_workers`], and [`Dev::set_images`] bridged to
//! the media domain. See `docs/events/dev.md`.
use std::{ops::Deref, sync::Arc};

use parking_lot::RwLock;
use tokuly_core::prelude::*;
use tokuly_identity::Identity;

mod apps;
mod bots;
mod cleanup;
pub mod config;
mod consent_svc;
pub mod delivery;
pub mod events;
pub mod fanout;
pub mod images;
mod oauth;
mod oauth_http;
pub mod reference;
pub mod scopes;
pub mod secrets;
mod tokens;
mod transfers;
mod usage;
mod verify;
pub mod webhooks;
mod workers;

pub use config::DevConfig;
pub use images::ImageStore;

pub struct DevInner {
    pub core: Core,
    pub identity: Identity,
    pub cfg: DevConfig,
    images: RwLock<Option<Arc<dyn ImageStore>>>,
    /// The API as its descriptor set describes it.
    reference: reference::Reference,
    /// Sends webhook deliveries; never follows redirects, never connects
    /// to a private address.
    webhook_http: reqwest::Client,
    /// Event types somebody subscribed to, so the rest costs nothing.
    subscribed: fanout::SubscribedTypes,
}

/// The developer platform. Cheap to clone.
#[derive(Clone)]
pub struct Dev(Arc<DevInner>);

impl Deref for Dev {
    type Target = DevInner;

    fn deref(&self) -> &DevInner {
        &self.0
    }
}

impl Dev {
    /// Builds the domain and registers its hooks: the API token verifier
    /// (from here on `tkl_…` bearer tokens authenticate), the report subject
    /// for applications and the data exporter.
    pub fn new(core: Core, identity: Identity, cfg: DevConfig) -> anyhow::Result<Self> {
        cfg.validate(core.cfg.is_production())?;
        let dev = Self(Arc::new(DevInner {
            reference: reference::Reference::load(tokuly_proto::FILE_DESCRIPTOR_SET)?,
            webhook_http: delivery::http_client(&cfg)?,
            images: RwLock::new(None),
            subscribed: fanout::SubscribedTypes::default(),
            core,
            identity,
            cfg,
        }));
        dev.core.api_tokens.register(Arc::new(verify::Verifier(dev.clone())));
        dev.core
            .subjects
            .register(tokuly_proto::common::SubjectType::SUBJECT_TYPE_APP, Arc::new(cleanup::AppSubjects(dev.clone())));
        dev.core.exporters.register(Arc::new(cleanup::DevExporter(dev.clone())));
        Ok(dev)
    }

    /// Connects application icons and bot avatars to the media domain.
    /// Until this is called they cannot be set (`ICONS_UNAVAILABLE`).
    pub fn set_images(&self, images: Arc<dyn ImageStore>) {
        *self.images.write() = Some(images);
    }

    pub(crate) fn images(&self) -> AppResult<Arc<dyn ImageStore>> {
        self.images
            .read()
            .clone()
            .ok_or_else(|| AppError::unimplemented("ICONS_UNAVAILABLE", "images cannot be set on this server"))
    }

    /// Registers this domain's RPC services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::dev::{
            AppServiceExt, AuthorizationServiceExt, BotServiceExt, ReferenceServiceExt, TokenServiceExt,
            UsageServiceExt, WebhookServiceExt,
        };
        let me = Arc::new(self.clone());
        let router = TokenServiceExt::register(me.clone(), router);
        let router = AppServiceExt::register(me.clone(), router);
        let router = AuthorizationServiceExt::register(me.clone(), router);
        let router = BotServiceExt::register(me.clone(), router);
        let router = WebhookServiceExt::register(me.clone(), router);
        let router = ReferenceServiceExt::register(me.clone(), router);
        UsageServiceExt::register(me, router)
    }

    /// The plain HTTP endpoints next to the RPCs: `POST /oauth/token`,
    /// `POST /oauth/revoke`, `POST /oauth/device_authorization`,
    /// `GET /.well-known/oauth-authorization-server` and
    /// `GET /reference/descriptor.binpb`. Merge it into the API router so
    /// the auth middleware (client address) wraps it.
    pub fn http_routes(&self) -> axum::Router {
        oauth_http::routes(self.clone())
    }

    /// Sets the rate tier (`standard` or `elevated`) of an application.
    /// For operators; there is no RPC for it.
    pub async fn set_app_rate_tier(&self, app_id: Uuid, tier: &str) -> AppResult<()> {
        ensure!(matches!(tier, "standard" | "elevated"), invalid, "RATE_TIER_UNKNOWN", "unknown rate tier {tier}");
        let done = sqlx::query("UPDATE dev_apps SET rate_tier = $2 WHERE id = $1")
            .bind(app_id)
            .bind(tier)
            .execute(&self.core.db)
            .await?;
        ensure!(done.rows_affected() == 1, not_found, "APP_NOT_FOUND", "application not found");
        self.forget_app_tokens(app_id).await
    }
}

/// Makes "count, then insert" safe against two requests at once: a
/// transaction-level advisory lock per account and kind of thing. The lock
/// is held until the transaction ends, so everything up to the commit must
/// run on that same transaction — nothing through the pool.
pub(crate) async fn lock_limit(tx: &mut sqlx::PgConnection, what: &str, account: Uuid) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1 || ':' || $2::text, 0))")
        .bind(what)
        .bind(account)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Text limits shared by the services: trims, counts characters and
/// refuses control characters.
pub(crate) fn clean_text(input: &str, max_chars: usize, reason: &'static str, what: &str) -> AppResult<String> {
    let text = input.trim();
    ensure!(text.chars().count() <= max_chars, invalid, reason, "{what} must be at most {max_chars} characters");
    ensure!(!text.chars().any(|c| c.is_control() && c != '\n'), invalid, reason, "{what} contains control characters");
    Ok(text.to_owned())
}

/// Like [`clean_text`] for a required single-line name.
pub(crate) fn clean_name(input: &str, max_chars: usize, reason: &'static str, what: &str) -> AppResult<String> {
    let name = clean_text(input, max_chars, reason, what)?;
    ensure!(
        !name.is_empty() && !name.contains('\n'),
        invalid,
        reason,
        "{what} must be 1-{max_chars} characters on one line"
    );
    Ok(name)
}
