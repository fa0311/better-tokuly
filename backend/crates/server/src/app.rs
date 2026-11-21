//! Wiring: domains, the HTTP application, workers and setup tasks.
use std::{net::SocketAddr, time::Duration};

use axum::{Json, extract::State, http::StatusCode, routing::get};
use clap::Args;
use http::{HeaderName, HeaderValue, Method, header};
use tokuly_admin::{Admin, AdminConfig};
use tokuly_core::{Core, analytics, auth::auth_middleware, events};
use tokuly_identity::{Identity, IdentityConfig};
use tokuly_media::{Media, MediaConfig};
use tokuly_notify::{Notify, NotifyConfig};
use tokuly_realtime::{Gateway, RealtimeConfig};
use tokuly_social::{Social, SocialConfig};
use tokuly_stamp::{Stamp, StampConfig};
use tokuly_wallet::{Wallet, WalletConfig};
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    trace::TraceLayer,
};

/// Domain-specific settings, flattened into the command line.
#[derive(Args, Clone)]
pub struct DomainConfigs {
    #[command(flatten)]
    pub identity: IdentityConfig,
    #[command(flatten)]
    pub media: MediaConfig,
    #[command(flatten)]
    pub social: SocialConfig,
    #[command(flatten)]
    pub stamp: StampConfig,
    #[command(flatten)]
    pub wallet: WalletConfig,
    #[command(flatten)]
    pub notify: NotifyConfig,
    #[command(flatten)]
    pub admin: AdminConfig,
    #[command(flatten)]
    pub realtime: RealtimeConfig,
}

/// Every domain, constructed once and shared by the API and the workers.
#[derive(Clone)]
pub struct Domains {
    pub media: Media,
    pub identity: Identity,
    pub social: Social,
    pub stamp: Stamp,
    pub wallet: Wallet,
    pub notify: Notify,
    pub admin: Admin,
    pub gateway: Gateway,
}

impl Domains {
    /// Constructs the domains in dependency order. Construction also registers
    /// each domain's hooks (profile extensions, subject resolvers, exporters).
    pub fn new(core: Core, cfg: DomainConfigs) -> anyhow::Result<Self> {
        let media = Media::new(core.clone(), cfg.media)?;
        let identity = Identity::new(core.clone(), cfg.identity)?;
        let social = Social::new(core.clone(), identity.clone(), cfg.social)?;
        let wallet = Wallet::new(core.clone(), identity.clone(), cfg.wallet)?;
        let stamp = Stamp::new(core.clone(), identity.clone(), cfg.stamp)?;
        let notify = Notify::new(core.clone(), identity.clone(), cfg.notify)?;
        let admin = Admin::new(core.clone(), identity.clone(), cfg.admin)?;
        let gateway = Gateway::new(core, cfg.realtime)?;
        Ok(Self { media, identity, social, stamp, wallet, notify, admin, gateway })
    }

    /// Every RPC service of every domain on one Connect router.
    fn connect_router(&self) -> connectrpc::Router {
        let mut router = connectrpc::Router::new();
        router = self.identity.register(router);
        router = self.media.register(router);
        router = self.social.register(router);
        router = self.stamp.register(router);
        router = self.wallet.register(router);
        router = self.notify.register(router);
        router = self.admin.register(router);
        router = self.gateway.register(router);
        router
    }
}

/// All PostgreSQL migrations, embedded at compile time from `backend/migrations`.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// ClickHouse schema files, embedded from `backend/clickhouse` (applied in order).
const CLICKHOUSE_SCHEMA: &[(&str, &str)] = &[("0001_events", include_str!("../../../clickhouse/0001_events.sql"))];

/// One-time and idempotent setup: database migrations, analytics schema,
/// buckets, streams, and the first admin.
pub async fn migrate(core: &Core, domains: &Domains) -> anyhow::Result<()> {
    tokuly_core::db::migrate(&core.db, &MIGRATOR).await?;
    tracing::info!("database migrations applied");
    core.bus.ensure_streams().await?;
    core.storage.ensure_buckets().await?;
    tracing::info!("object storage buckets ready");
    match analytics::apply_schema(&core.cfg, CLICKHOUSE_SCHEMA).await {
        Ok(()) => tracing::info!("analytics schema applied"),
        // Analytics is not required to serve users; keep going and say so.
        Err(e) => tracing::warn!(error = %e, "analytics schema not applied; dashboards will be empty until ClickHouse is reachable"),
    }
    // The account named by TOKULY_ADMIN_USER, if it already exists; otherwise
    // the admin worker promotes it when it signs up.
    if let Err(e) = domains.admin.grant_bootstrap_admin().await {
        tracing::warn!(error = %e, "could not grant the admin role");
    }
    Ok(())
}

/// Starts every background loop of the worker role.
pub fn spawn_workers(core: &Core, domains: &Domains) {
    tokio::spawn(events::run_outbox_relay(core.db.clone(), core.bus.clone(), core.shutdown.clone()));
    let analytics = core.analytics.clone();
    let cancel = core.shutdown.clone();
    tokio::spawn(async move { analytics.run_sink(cancel).await });
    domains.identity.spawn_workers();
    domains.media.spawn_workers();
    domains.social.spawn_workers();
    domains.stamp.spawn_workers();
    domains.wallet.spawn_workers();
    domains.notify.spawn_workers();
    domains.admin.spawn_workers();
    tracing::info!("workers started");
}

fn cors_layer(core: &Core, extra_origins: Vec<String>) -> CorsLayer {
    let mut allowed: Vec<String> = vec![core.cfg.public_url().to_owned()];
    allowed.extend(extra_origins.into_iter().map(|o| o.trim().trim_end_matches('/').to_owned()).filter(|o| !o.is_empty()));
    let development = !core.cfg.is_production();
    let origin = AllowOrigin::predicate(move |origin: &HeaderValue, _| {
        let Ok(origin) = origin.to_str() else { return false };
        if allowed.iter().any(|a| a == origin) {
            return true;
        }
        // Dev servers, Expo web and Tauri run on assorted local ports.
        development
            && (origin.starts_with("http://localhost:")
                || origin.starts_with("http://127.0.0.1:")
                || origin == "http://localhost"
                || origin == "tauri://localhost"
                || origin == "http://tauri.localhost")
    });
    CorsLayer::new()
        .allow_origin(origin)
        .allow_credentials(true)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            header::ACCEPT,
            HeaderName::from_static("connect-protocol-version"),
            HeaderName::from_static("connect-timeout-ms"),
            HeaderName::from_static("connect-accept-encoding"),
            HeaderName::from_static("connect-content-encoding"),
            HeaderName::from_static("grpc-timeout"),
            HeaderName::from_static("x-grpc-web"),
            HeaderName::from_static("x-user-agent"),
            HeaderName::from_static("x-tokuly-client"),
        ])
        .expose_headers([
            HeaderName::from_static("x-tokuly-error"),
            HeaderName::from_static("grpc-status"),
            HeaderName::from_static("grpc-message"),
            HeaderName::from_static("grpc-status-details-bin"),
            HeaderName::from_static("connect-content-encoding"),
        ])
        .max_age(Duration::from_secs(7200))
}

async fn healthz() -> &'static str {
    "ok"
}

/// Readiness: can this instance reach its dependencies?
async fn readyz(State(core): State<Core>) -> (StatusCode, Json<serde_json::Value>) {
    let postgres = tokuly_core::db::ping(&core.db).await;
    let valkey = core.cache.ping().await.is_ok();
    let nats = core.bus.client.connection_state() == tokuly_core::bus::ConnectionState::Connected;
    let ready = postgres && valkey && nats;
    let status = if ready { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (status, Json(serde_json::json!({ "ready": ready, "postgres": postgres, "valkey": valkey, "nats": nats })))
}

pub fn api_router(core: Core, domains: &Domains, cors_origins: Vec<String>) -> axum::Router {
    let connect = domains.connect_router();
    tracing::info!(methods = connect.methods().count(), "RPC methods registered");
    axum::Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(core.clone())
        .fallback_service(connect.into_axum_service())
        .layer(axum::middleware::from_fn_with_state(core.auth_layer.clone(), auth_middleware))
        .layer(TraceLayer::new_for_http())
        .layer(cors_layer(&core, cors_origins))
}

pub async fn serve_api(core: Core, domains: Domains, listen: SocketAddr, cors_origins: Vec<String>) -> anyhow::Result<()> {
    let app = api_router(core.clone(), &domains, cors_origins);
    let listener = tokio::net::TcpListener::bind(listen).await.map_err(|e| anyhow::anyhow!("cannot listen on {listen}: {e}"))?;
    tracing::info!(%listen, "API listening");
    let shutdown = core.shutdown.clone();
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await?;
    Ok(())
}
