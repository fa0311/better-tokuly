//! Wiring: domains, the HTTP application, workers and setup tasks.
use std::{net::SocketAddr, time::Duration};

use axum::{Json, extract::State, http::StatusCode, routing::get};
use clap::Args;
use http::{HeaderName, HeaderValue, Method, header};
use tokuly_admin::{Admin, AdminConfig};
use tokuly_ask::{Ask, AskConfig};
use tokuly_chat::{Chat, ChatConfig};
use tokuly_community::{Community, CommunityConfig};
use tokuly_core::{Core, analytics, auth::auth_middleware, events};
use tokuly_dev::{Dev, DevConfig};
use tokuly_emoji::{Emoji, EmojiConfig};
use tokuly_engagement::{Engagement, EngagementConfig};
use tokuly_feed::{Feed, FeedConfig};
use tokuly_geo::{Geo, GeoConfig};
use tokuly_identity::{Identity, IdentityConfig};
use tokuly_live::{Live, LiveConfig};
use tokuly_media::{Media, MediaConfig};
use tokuly_notes::{Notes, NotesConfig};
use tokuly_notify::{Notify, NotifyConfig};
use tokuly_plans::{Plans, PlansConfig};
use tokuly_quest::{Quest, QuestConfig};
use tokuly_realtime::{Gateway, RealtimeConfig};
use tokuly_school::{School, SchoolConfig};
use tokuly_search::{Search, SearchConfig};
use tokuly_shorts::{Shorts, ShortsConfig};
use tokuly_social::{Social, SocialConfig};
use tokuly_spaces::{Spaces, SpacesConfig};
use tokuly_stamp::{Stamp, StampConfig};
use tokuly_video::{Video, VideoConfig};
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
    pub geo: GeoConfig,
    #[command(flatten)]
    pub stamp: StampConfig,
    #[command(flatten)]
    pub wallet: WalletConfig,
    #[command(flatten)]
    pub notify: NotifyConfig,
    #[command(flatten)]
    pub admin: AdminConfig,
    #[command(flatten)]
    pub feed: FeedConfig,
    #[command(flatten)]
    pub chat: ChatConfig,
    #[command(flatten)]
    pub notes: NotesConfig,
    #[command(flatten)]
    pub video: VideoConfig,
    #[command(flatten)]
    pub search: SearchConfig,
    #[command(flatten)]
    pub emoji: EmojiConfig,
    #[command(flatten)]
    pub ask: AskConfig,
    #[command(flatten)]
    pub plans: PlansConfig,
    #[command(flatten)]
    pub school: SchoolConfig,
    #[command(flatten)]
    pub quest: QuestConfig,
    #[command(flatten)]
    pub live: LiveConfig,
    #[command(flatten)]
    pub shorts: ShortsConfig,
    #[command(flatten)]
    pub spaces: SpacesConfig,
    #[command(flatten)]
    pub community: CommunityConfig,
    #[command(flatten)]
    pub dev: DevConfig,
    #[command(flatten)]
    pub engagement: EngagementConfig,
    #[command(flatten)]
    pub realtime: RealtimeConfig,
}

/// Every domain, constructed once and shared by the API and the workers.
#[derive(Clone)]
pub struct Domains {
    pub media: Media,
    pub identity: Identity,
    pub social: Social,
    pub geo: Geo,
    pub stamp: Stamp,
    pub wallet: Wallet,
    pub notify: Notify,
    pub admin: Admin,
    pub feed: Feed,
    pub chat: Chat,
    pub notes: Notes,
    pub video: Video,
    pub search: Search,
    pub emoji: Emoji,
    pub ask: Ask,
    pub plans: Plans,
    pub school: School,
    pub quest: Quest,
    pub live: Live,
    pub shorts: Shorts,
    pub spaces: Spaces,
    pub community: Community,
    pub dev: Dev,
    pub engagement: Engagement,
    pub gateway: Gateway,
}

impl Domains {
    /// Constructs the domains in dependency order. Construction also registers
    /// each domain's hooks (profile extensions, subject resolvers, exporters).
    pub fn new(core: Core, cfg: DomainConfigs) -> anyhow::Result<Self> {
        let media = Media::new(core.clone(), cfg.media)?;
        let identity = Identity::new(core.clone(), cfg.identity)?;
        let social = Social::new(core.clone(), identity.clone(), cfg.social)?;
        let geo = Geo::new(core.clone(), identity.clone(), cfg.geo, &social)?;
        let wallet = Wallet::new(core.clone(), identity.clone(), cfg.wallet)?;
        let stamp = Stamp::new(core.clone(), identity.clone(), cfg.stamp)?;
        let notify = Notify::new(core.clone(), identity.clone(), cfg.notify)?;
        let admin = Admin::new(core.clone(), identity.clone(), cfg.admin)?;
        let feed = Feed::new(core.clone(), identity.clone(), cfg.feed)?;
        let chat = Chat::new(core.clone(), identity.clone(), cfg.chat)?;
        let notes = Notes::new(core.clone(), identity.clone(), cfg.notes)?;
        let video = Video::new(core.clone(), identity.clone(), cfg.video)?;
        let search = Search::new(core.clone(), identity.clone(), cfg.search)?;
        let emoji = Emoji::new(core.clone(), identity.clone(), cfg.emoji)?;
        let ask = Ask::new(core.clone(), identity.clone(), cfg.ask)?;
        let plans = Plans::new(core.clone(), identity.clone(), cfg.plans, social.clone())?;
        let school = School::new(core.clone(), identity.clone(), cfg.school)?;
        let quest = Quest::new(core.clone(), identity.clone(), cfg.quest)?;
        let live = Live::new(core.clone(), identity.clone(), cfg.live, wallet.clone())?;
        let shorts = Shorts::new(core.clone(), identity.clone(), cfg.shorts)?;
        let spaces = Spaces::new(core.clone(), identity.clone(), cfg.spaces)?;
        let community = Community::new(core.clone(), identity.clone(), cfg.community)?;
        // From here on `tkl_…` bearer tokens authenticate.
        let dev = Dev::new(core.clone(), identity.clone(), cfg.dev)?;
        // After every domain that owns subjects: engagement only fills in the
        // subject types nobody else registered a resolver for.
        let engagement = Engagement::new(core.clone(), identity.clone(), cfg.engagement)?;
        let gateway = Gateway::new(core.clone(), cfg.realtime)?;
        let domains = Self {
            media,
            identity,
            social,
            geo,
            stamp,
            wallet,
            notify,
            admin,
            feed,
            chat,
            notes,
            video,
            search,
            emoji,
            ask,
            plans,
            school,
            quest,
            live,
            shorts,
            spaces,
            community,
            dev,
            engagement,
            gateway,
        };
        crate::bridges::connect(&core, &domains);
        Ok(domains)
    }

    /// Every RPC service of every domain on one Connect router.
    fn connect_router(&self) -> connectrpc::Router {
        let mut router = connectrpc::Router::new();
        router = self.identity.register(router);
        router = self.media.register(router);
        router = self.social.register(router);
        router = self.geo.register(router);
        router = self.stamp.register(router);
        router = self.wallet.register(router);
        router = self.notify.register(router);
        router = self.admin.register(router);
        router = self.feed.register(router);
        router = self.chat.register(router);
        router = self.notes.register(router);
        router = self.video.register(router);
        router = self.search.register(router);
        router = self.emoji.register(router);
        router = self.ask.register(router);
        router = self.plans.register(router);
        router = self.school.register(router);
        router = self.quest.register(router);
        router = self.live.register(router);
        router = self.shorts.register(router);
        router = self.spaces.register(router);
        router = self.community.register(router);
        router = self.dev.register(router);
        router = self.engagement.register(router);
        router = self.gateway.register(router);
        router
    }
}

/// All PostgreSQL migrations, embedded at compile time from `backend/migrations`.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// ClickHouse schema files, embedded from `backend/clickhouse` (applied in
/// order): the shared event tables, then what the domains keep there.
fn clickhouse_schema() -> Vec<(&'static str, &'static str)> {
    let mut schema = vec![("0001_events", include_str!("../../../clickhouse/0001_events.sql"))];
    schema.extend_from_slice(tokuly_geo::CLICKHOUSE_SCHEMA);
    schema
}

/// One-time and idempotent setup: database migrations, analytics schema,
/// buckets, streams, and the first admin.
pub async fn migrate(core: &Core, domains: &Domains) -> anyhow::Result<()> {
    tokuly_core::db::migrate(&core.db, &MIGRATOR).await?;
    tracing::info!("database migrations applied");
    core.bus.ensure_streams().await?;
    core.storage.ensure_buckets().await?;
    tracing::info!("object storage buckets ready");
    match analytics::apply_schema(&core.cfg, &clickhouse_schema()).await {
        Ok(()) => tracing::info!("analytics schema applied"),
        // Analytics is not required to serve users; keep going and say so.
        Err(e) => {
            tracing::warn!(error = %e, "analytics schema not applied; dashboards will be empty until ClickHouse is reachable")
        }
    }
    match domains.search.setup().await {
        Ok(()) => tracing::info!("search indexes ready"),
        // Search is not required to serve users; the indexer creates the indexes when the engine is back.
        Err(e) => {
            tracing::warn!(error = %e, "search indexes not prepared; search stays empty until Meilisearch is reachable")
        }
    }
    // Country and prefecture boundaries for footprints, bundled with the binary.
    domains.geo.seed_regions().await.map_err(|e| anyhow::anyhow!("could not load the map regions: {e}"))?;
    tracing::info!("map regions ready");
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
    domains.geo.spawn_workers();
    domains.stamp.spawn_workers();
    domains.wallet.spawn_workers();
    domains.notify.spawn_workers();
    domains.admin.spawn_workers();
    domains.feed.spawn_workers();
    domains.chat.spawn_workers();
    domains.notes.spawn_workers();
    domains.video.spawn_workers();
    domains.search.spawn_workers();
    domains.emoji.spawn_workers();
    domains.ask.spawn_workers();
    domains.plans.spawn_workers();
    domains.school.spawn_workers();
    domains.quest.spawn_workers();
    domains.live.spawn_workers();
    domains.shorts.spawn_workers();
    domains.spaces.spawn_workers();
    domains.community.spawn_workers();
    domains.dev.spawn_workers();
    domains.engagement.spawn_workers();
    tracing::info!("workers started");
}

fn cors_layer(core: &Core, extra_origins: Vec<String>) -> CorsLayer {
    let mut allowed: Vec<String> = vec![core.cfg.public_url().to_owned()];
    allowed
        .extend(extra_origins.into_iter().map(|o| o.trim().trim_end_matches('/').to_owned()).filter(|o| !o.is_empty()));
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
            // What a client of the developer platform reads: its rate limit,
            // the scope a refused call needed, and when to try again.
            HeaderName::from_static("x-ratelimit-limit"),
            HeaderName::from_static("x-ratelimit-remaining"),
            HeaderName::from_static("x-ratelimit-reset"),
            HeaderName::from_static("x-tokuly-scope"),
            header::WWW_AUTHENTICATE,
            header::RETRY_AFTER,
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
        // The OAuth endpoints and the API descriptor; they read the client
        // address the auth middleware works out.
        .merge(domains.dev.http_routes())
        // The pictures of link previews, fetched on the viewers' behalf.
        .merge(domains.media.http_routes())
        .fallback_service(connect.into_axum_service())
        .layer(axum::middleware::from_fn_with_state(core.auth_layer.clone(), auth_middleware))
        // Our own infrastructure calling in. Both prove who they are in
        // `Authorization` with something that is not a token of ours, so they
        // stay outside the auth middleware; the edge does not pass them on.
        // LiveKit's webhook for audio rooms:
        .merge(domains.spaces.http_routes())
        // The media server asking who may publish and watch a live stream:
        .nest("/hooks/live", domains.live.http_routes())
        .layer(TraceLayer::new_for_http())
        .layer(cors_layer(&core, cors_origins))
}

pub async fn serve_api(
    core: Core,
    domains: Domains,
    listen: SocketAddr,
    cors_origins: Vec<String>,
) -> anyhow::Result<()> {
    let app = api_router(core.clone(), &domains, cors_origins);
    let listener =
        tokio::net::TcpListener::bind(listen).await.map_err(|e| anyhow::anyhow!("cannot listen on {listen}: {e}"))?;
    tracing::info!(%listen, "API listening");
    let shutdown = core.shutdown.clone();
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use tokuly_media::link_preview::image_proxy::{Kind, proxy_url};
    use tokuly_proto::{
        geo::{LocationSample, ReportLocationsRequest, ReportLocationsResponse},
        social::{DiscoverNearbyRequest, DiscoverNearbyResponse},
    };
    use tokuly_testkit::TestApp;
    use tower::ServiceExt as _;

    use super::*;

    #[derive(Parser)]
    struct Settings {
        #[command(flatten)]
        domains: DomainConfigs,
    }

    const MIGRATIONS: &[&str] = &[
        "0001", "0100", "0200", "0300", "0400", "0500", "0600", "0700", "0800", "0900", "1000", "1100", "1200", "1300",
        "1400", "1500", "1600", "1700", "1800", "1900", "2000", "2100", "2200", "2300", "2400",
    ];

    /// The pictures of link previews are served by the API's router, behind
    /// the auth layer: it is what tells the route the client's address.
    #[tokio::test]
    async fn link_preview_pictures_are_routed_with_the_client_address() {
        let app = tokuly_testkit::TestApp::start(MIGRATIONS, |_, _, router| router).await;
        let core = app.core.clone();
        let domains = Domains::new(core.clone(), Settings::parse_from(["tokuly"]).domains).expect("domains");
        let router = api_router(core.clone(), &domains, Vec::new());
        let get = |path: String, peer: std::net::IpAddr| {
            let router = router.clone();
            async move {
                let request = axum::http::Request::get(path)
                    .extension(axum::extract::ConnectInfo(SocketAddr::new(peer, 4000)))
                    .body(axum::body::Body::empty())
                    .unwrap();
                let response = router.oneshot(request).await.unwrap();
                let cache = response.headers().get("cache-control").map(|v| v.to_str().unwrap().to_owned());
                (response.status().as_u16(), cache)
            }
        };
        // A signed URL of a picture on an address the egress guard refuses:
        // answered by the route ("nothing to show", remembered for a while),
        // not by the Connect fallback.
        let target = format!("http://169.254.169.254/{}.png", uuid::Uuid::now_v7());
        let proxied = proxy_url(&core, Kind::Image, &target);
        let path = proxied[proxied.find("/media/link-image/").unwrap()..].to_owned();
        let peer = std::net::IpAddr::from([198, 51, 100, (uuid::Uuid::now_v7().as_u128() % 250) as u8 + 1]);
        assert_eq!(get(path.clone(), peer).await, (404, Some("public, max-age=1800".to_owned())));

        // The client's address reaches the per-client limit: once it is used
        // up, a miss is turned away (and not remembered).
        for _ in 0..120 {
            let _ = core.cache.rate_limit(&format!("media:link-image-client:{peer}"), 120, Duration::from_secs(60)).await;
        }
        let other = format!("http://169.254.169.254/{}.png", uuid::Uuid::now_v7());
        let proxied = proxy_url(&core, Kind::Image, &other);
        let busy = proxied[proxied.find("/media/link-image/").unwrap()..].to_owned();
        assert_eq!(get(busy, peer).await, (503, Some("no-store".to_owned())));
        core.cache.del(&format!("rl:media:link-image-client:{peer}")).await.unwrap();
    }

    /// The server plugs the geo domain's check of claimed positions into the
    /// social domain: without it every nearby session and bump is refused.
    #[tokio::test]
    async fn a_real_fix_lets_a_nearby_session_open_there() {
        let app = TestApp::start(MIGRATIONS, |core, _identity, _router| {
            let cfg = Settings::try_parse_from(["tokuly"]).expect("default settings").domains;
            Domains::new(core.clone(), cfg).expect("domains").connect_router()
        })
        .await;
        let aoi = app.user("aoi").await;
        let spot = (35.6595 + rand_offset(aoi.id), 139.7005);
        let open = |at: (f64, f64)| {
            let request = DiscoverNearbyRequest {
                location: tokuly_proto::common::LatLng { lat: at.0, lng: at.1, ..Default::default() }.into(),
                accuracy_meters: 5.0,
                ..Default::default()
            };
            let (app, aoi) = (&app, aoi.clone());
            async move {
                app.stream::<_, DiscoverNearbyResponse>("tokuly.social.v1.FriendService/DiscoverNearby", Some(&aoi), &request, 1, Duration::from_secs(3)).await
            }
        };
        assert_eq!(open(spot).await.unwrap_err().reason, "LOCATION_NOT_VERIFIED", "no fix yet");

        let sample = LocationSample {
            position: tokuly_proto::common::LatLng { lat: spot.0, lng: spot.1, ..Default::default() }.into(),
            accuracy_meters: 10.0,
            recorded_at: tokuly_core::time::ts(chrono::Utc::now()),
            foreground: true,
            ..Default::default()
        };
        let reported: ReportLocationsResponse = app
            .call("tokuly.geo.v1.LocationService/ReportLocations", Some(&aoi), &ReportLocationsRequest { samples: vec![sample], ..Default::default() })
            .await
            .expect("report a fix");
        assert_eq!(reported.accepted_count, 1);

        let first = open(spot).await.expect("a session where the fix is");
        assert_eq!(first.len(), 1, "the first snapshot arrives at once");
        assert_eq!(open((spot.0 + 0.05, spot.1)).await.unwrap_err().reason, "LOCATION_NOT_VERIFIED", "5 km off the fix");
    }

    /// A latitude offset per run (Valkey is shared with other test runs).
    fn rand_offset(id: uuid::Uuid) -> f64 {
        (id.as_u128() % 10_000) as f64 / 100_000.0
    }
}
