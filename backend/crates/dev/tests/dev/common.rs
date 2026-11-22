//! Shared helpers for the dev integration tests.
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicU16, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use base64::Engine as _;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use sqlx::PgConnection;
use tokuly_core::{AppError, AppResult, events::Event};
use tokuly_dev::{Dev, DevConfig, ImageStore};
use tokuly_proto::dev::*;
use tokuly_testkit::{TestApp, TestUser};
use tower::ServiceExt as _;
use uuid::Uuid;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0400", "2400"];

pub const TOKEN_SERVICE: &str = "tokuly.dev.v1.TokenService";
pub const APP_SERVICE: &str = "tokuly.dev.v1.AppService";
pub const AUTHORIZATION_SERVICE: &str = "tokuly.dev.v1.AuthorizationService";
pub const BOT_SERVICE: &str = "tokuly.dev.v1.BotService";
pub const WEBHOOK_SERVICE: &str = "tokuly.dev.v1.WebhookService";

/// A running app with identity's and this domain's services, the domain
/// object for calling handlers directly, and the plain HTTP routes behind
/// the same auth middleware.
pub struct Ctx {
    pub app: TestApp,
    pub dev: Dev,
    pub images: Arc<FakeImages>,
    http: axum::Router,
    ip: IpAddr,
}

/// The defaults, except that webhooks may reach the local test receiver.
pub fn config() -> DevConfig {
    DevConfig { webhook_allow_private: true, ..Default::default() }
}

pub async fn start() -> Ctx {
    start_with(config()).await
}

pub async fn start_with(cfg: DevConfig) -> Ctx {
    let mut holder = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        let dev = Dev::new(core.clone(), identity.clone(), cfg).expect("dev starts");
        let router = dev.register(identity.register(router));
        holder = Some(dev);
        router
    })
    .await;
    let dev = holder.expect("register ran");
    let images = Arc::new(FakeImages::default());
    dev.set_images(images.clone());
    let http = dev
        .http_routes()
        .layer(axum::middleware::from_fn_with_state(app.core.auth_layer.clone(), tokuly_core::auth::auth_middleware));
    // An address of its own per app, so per-address limits never collide across tests.
    let bytes = Uuid::new_v4().into_bytes();
    let ip = IpAddr::from([10, bytes[0], bytes[1], bytes[2].max(1)]);
    Ctx { app, dev, images, http, ip }
}

/// The same person, calling with an API token instead of their session.
pub fn with_token(user: &TestUser, secret: &str) -> TestUser {
    TestUser { token: secret.to_owned(), ..user.clone() }
}

/// Retries `check` for a few seconds: for effects that happen after the
/// response (the middleware's usage hook).
pub async fn eventually<F, Fut>(what: &str, check: F)
where
    F: Fn() -> Fut,
    Fut: Future<Output = bool>,
{
    for _ in 0..200 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("never happened: {what}");
}

/// Runs `count` copies of a request at the same moment and returns their
/// results. Fails if they do not all finish in good time: a handler that
/// held a connection while waiting for another would hang here.
pub async fn at_once<T, F, Fut>(count: usize, request: F) -> Vec<T>
where
    F: Fn(usize) -> Fut,
    Fut: Future<Output = T>,
{
    tokio::time::timeout(Duration::from_secs(60), futures::future::join_all((0..count).map(request)))
        .await
        .expect("concurrent requests stalled")
}

/// How many times the app's database pool can be taken at once.
pub const POOL_SIZE: usize = 5;

pub fn event(topic: &str, data: impl Serialize) -> Event {
    Event {
        id: Uuid::now_v7(),
        topic: topic.to_owned(),
        actor_id: None,
        occurred_at: chrono::Utc::now(),
        data: serde_json::to_value(data).expect("payload serializes"),
    }
}

// ───────────────────────────── plain HTTP ─────────────────────────────

pub struct HttpReply {
    pub status: u16,
    pub headers: http::HeaderMap,
    pub body: bytes::Bytes,
}

impl HttpReply {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&self.body)))
    }

    pub fn header(&self, name: &str) -> &str {
        self.headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or_default()
    }

    /// The OAuth error code of an error response.
    pub fn error(&self) -> String {
        self.json()["error"].as_str().unwrap_or_default().to_owned()
    }
}

impl Ctx {
    async fn send(&self, request: http::request::Builder, body: Vec<u8>) -> HttpReply {
        let request = request
            .extension(axum::extract::ConnectInfo(SocketAddr::new(self.ip, 0)))
            .body(axum::body::Body::from(body))
            .expect("request");
        let response = self.http.clone().oneshot(request).await.expect("router is infallible");
        let (parts, body) = response.into_parts();
        HttpReply {
            status: parts.status.as_u16(),
            headers: parts.headers,
            body: axum::body::to_bytes(body, 8 * 1024 * 1024).await.expect("body"),
        }
    }

    /// `POST <path>` with a form body, optionally with HTTP Basic client credentials.
    pub async fn form(&self, path: &str, basic: Option<(&str, &str)>, params: &[(&str, &str)]) -> HttpReply {
        let body = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(params).finish();
        let mut request = http::Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/x-www-form-urlencoded");
        if let Some((id, secret)) = basic {
            let credentials = base64::engine::general_purpose::STANDARD.encode(format!("{id}:{secret}"));
            request = request.header("authorization", format!("Basic {credentials}"));
        }
        self.send(request, body.into_bytes()).await
    }

    pub async fn get(&self, path: &str) -> HttpReply {
        self.send(http::Request::builder().method("GET").uri(path), Vec::new()).await
    }

    /// `notify.requested` events of `kind` addressed to `user_id`.
    pub async fn notifications(&self, user_id: Uuid, kind: &str) -> Vec<serde_json::Value> {
        let events = self.app.events("notify.requested").await;
        events
            .into_iter()
            .map(|e| e.data)
            .filter(|data| data["user_id"] == user_id.to_string() && data["kind"] == kind)
            .collect()
    }

    /// How many security events of `kind` the account has.
    pub async fn security_events(&self, user_id: Uuid, kind: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM security_events WHERE user_id = $1 AND type = $2")
            .bind(user_id)
            .bind(kind)
            .fetch_one(&self.app.core.db)
            .await
            .expect("security events")
    }
}

// ───────────────────────────── tokens ─────────────────────────────

/// Creates a personal access token for `user` and returns it with its secret.
pub async fn personal_token(ctx: &Ctx, user: &TestUser, scopes: &[&str]) -> (AccessToken, String) {
    let request = CreateTokenRequest {
        name: "test token".into(),
        scopes: scopes.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    let created: CreateTokenResponse =
        ctx.app.call(&format!("{TOKEN_SERVICE}/CreateToken"), Some(user), &request).await.expect("token is created");
    (created.token.into_option().expect("token"), created.secret)
}

/// What the credential is, as `GetCurrentToken` reports it.
pub async fn whoami(ctx: &Ctx, caller: &TestUser) -> Result<GetCurrentTokenResponse, tokuly_testkit::RpcError> {
    ctx.app.call(&format!("{TOKEN_SERVICE}/GetCurrentToken"), Some(caller), &GetCurrentTokenRequest::default()).await
}

// ───────────────────────────── OAuth ─────────────────────────────

pub const REDIRECT_URI: &str = "https://app.example/callback";

/// A registered application with its credentials.
pub struct OAuthApp {
    pub id: String,
    pub secret: String,
    pub app: App,
}

impl OAuthApp {
    pub fn basic(&self) -> Option<(&str, &str)> {
        Some((&self.id, &self.secret))
    }
}

pub fn app_request(name: &str, scopes: &[&str], client_type: AppClientType) -> CreateAppRequest {
    CreateAppRequest {
        name: name.into(),
        description: "An app for tests".into(),
        homepage_url: "https://app.example".into(),
        redirect_uris: vec![REDIRECT_URI.into()],
        scopes: scopes.iter().map(|s| s.to_string()).collect(),
        client_type: client_type.into(),
        ..Default::default()
    }
}

pub async fn create_app(ctx: &Ctx, owner: &TestUser, scopes: &[&str], client_type: AppClientType) -> OAuthApp {
    let created: CreateAppResponse = ctx
        .app
        .call(&format!("{APP_SERVICE}/CreateApp"), Some(owner), &app_request("Test App", scopes, client_type))
        .await
        .expect("app is created");
    let app = created.app.into_option().expect("app");
    OAuthApp { id: app.id.clone(), secret: created.client_secret, app }
}

/// A PKCE pair: the verifier and its `S256` challenge.
pub fn pkce() -> (String, String) {
    let verifier = tokuly_core::ids::random_base62(64);
    let challenge = tokuly_core::keys::b64url(&Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

pub fn authorization_request(app: &OAuthApp, scope: &str, challenge: &str) -> AuthorizationRequest {
    AuthorizationRequest {
        client_id: app.id.clone(),
        redirect_uri: REDIRECT_URI.into(),
        response_type: "code".into(),
        scope: scope.into(),
        state: "state-123".into(),
        code_challenge: challenge.into(),
        code_challenge_method: "S256".into(),
        ..Default::default()
    }
}

/// The query parameters of a URL.
pub fn query(url: &str) -> HashMap<String, String> {
    url::Url::parse(url).expect("a URL").query_pairs().into_owned().collect()
}

/// `user` approves `app` for `scope`; returns the authorization code and the PKCE verifier.
pub async fn authorize(ctx: &Ctx, user: &TestUser, app: &OAuthApp, scope: &str) -> (String, String) {
    let (verifier, challenge) = pkce();
    let request = ApproveAuthorizationRequest {
        request: authorization_request(app, scope, &challenge).into(),
        ..Default::default()
    };
    let approved: ApproveAuthorizationResponse = ctx
        .app
        .call(&format!("{AUTHORIZATION_SERVICE}/ApproveAuthorization"), Some(user), &request)
        .await
        .expect("approved");
    let code = query(&approved.redirect_url).remove("code").expect("the redirect carries a code");
    (code, verifier)
}

/// Exchanges a code at the token endpoint and returns the JSON answer.
pub async fn exchange(ctx: &Ctx, app: &OAuthApp, code: &str, verifier: &str) -> HttpReply {
    ctx.form(
        "/oauth/token",
        app.basic(),
        &[("grant_type", "authorization_code"), ("code", code), ("code_verifier", verifier)],
    )
    .await
}

/// The whole authorization code flow; returns `(access_token, refresh_token)`.
pub async fn obtain_tokens(ctx: &Ctx, user: &TestUser, app: &OAuthApp, scope: &str) -> (String, String) {
    let (code, verifier) = authorize(ctx, user, app, scope).await;
    let reply = exchange(ctx, app, &code, &verifier).await;
    assert_eq!(reply.status, 200, "{}", String::from_utf8_lossy(&reply.body));
    let tokens = reply.json();
    (tokens["access_token"].as_str().unwrap().to_owned(), tokens["refresh_token"].as_str().unwrap().to_owned())
}

// ───────────────────────────── images ─────────────────────────────

/// Stands in for the media domain: knows which assets exist and remembers
/// what holds them.
#[derive(Default)]
pub struct FakeImages {
    /// asset → uploader
    assets: parking_lot::Mutex<HashMap<Uuid, Uuid>>,
    /// holder → asset
    pub holds: parking_lot::Mutex<HashMap<String, Uuid>>,
}

impl FakeImages {
    /// An image `uploader` uploaded.
    pub fn upload(&self, uploader: Uuid) -> Uuid {
        let asset = Uuid::now_v7();
        self.assets.lock().insert(asset, uploader);
        asset
    }

    pub fn held_by(&self, holder: &str) -> Option<Uuid> {
        self.holds.lock().get(holder).copied()
    }
}

#[async_trait]
impl ImageStore for FakeImages {
    async fn image(&self, uploader: Uuid, asset_id: Uuid) -> AppResult<serde_json::Value> {
        if self.assets.lock().get(&asset_id) != Some(&uploader) {
            return Err(AppError::not_found("ASSET_NOT_FOUND", "asset not found"));
        }
        Ok(
            serde_json::json!({ "assetId": asset_id, "url": format!("https://media.test/{asset_id}.webp"), "width": 256, "height": 256 }),
        )
    }

    async fn hold(&self, _tx: &mut PgConnection, asset_id: Uuid, holder: &str) -> AppResult<()> {
        self.holds.lock().insert(holder.to_owned(), asset_id);
        Ok(())
    }

    async fn release(&self, _tx: &mut PgConnection, holder: &str) -> AppResult<()> {
        self.holds.lock().remove(holder);
        Ok(())
    }
}

// ───────────────────────────── a webhook endpoint ─────────────────────────────

/// One request a [`Receiver`] got.
#[derive(Clone)]
pub struct Hit {
    pub path: String,
    pub headers: http::HeaderMap,
    pub body: String,
}

impl Hit {
    pub fn header(&self, name: &str) -> &str {
        self.headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or_default()
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).expect("deliveries are JSON")
    }
}

/// A local HTTP server standing in for a webhook owner's endpoint.
pub struct Receiver {
    pub url: String,
    hits: Arc<parking_lot::Mutex<Vec<Hit>>>,
    status: Arc<AtomicU16>,
}

impl Receiver {
    pub async fn start() -> Self {
        let hits = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let status = Arc::new(AtomicU16::new(200));
        let (recorded, answer) = (hits.clone(), status.clone());
        let handler = move |uri: http::Uri, headers: http::HeaderMap, body: String| async move {
            recorded.lock().push(Hit { path: uri.path().to_owned(), headers, body });
            let status = http::StatusCode::from_u16(answer.load(Ordering::SeqCst)).expect("a status");
            // A redirect points at this same server, where a followed one would be recorded.
            (status, [(http::header::LOCATION, "/redirected")], "received by the test endpoint")
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a local port");
        let url = format!("http://{}/hook", listener.local_addr().expect("address"));
        tokio::spawn(async move { axum::serve(listener, axum::Router::new().fallback(handler)).await });
        Self { url, hits, status }
    }

    /// What the endpoint answers from now on.
    pub fn answer_with(&self, status: u16) {
        self.status.store(status, Ordering::SeqCst);
    }

    pub fn hits(&self) -> Vec<Hit> {
        self.hits.lock().clone()
    }
}

/// Subscribes `owner` (a session or a token) to `event_types` at `url`.
pub async fn create_webhook(ctx: &Ctx, owner: &TestUser, url: &str, event_types: &[&str]) -> (Webhook, String) {
    let request = CreateWebhookRequest {
        url: url.into(),
        event_types: event_types.iter().map(|t| t.to_string()).collect(),
        ..Default::default()
    };
    let created: CreateWebhookResponse = ctx
        .app
        .call(&format!("{WEBHOOK_SERVICE}/CreateWebhook"), Some(owner), &request)
        .await
        .expect("webhook is created");
    (created.webhook.into_option().expect("webhook"), created.secret)
}

/// The IDs of a webhook's deliveries, oldest first.
pub async fn delivery_ids(ctx: &Ctx, webhook_id: &str) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT id FROM dev_webhook_deliveries WHERE webhook_id = $1 ORDER BY id")
        .bind(Uuid::parse_str(webhook_id).expect("id"))
        .fetch_all(&ctx.app.core.db)
        .await
        .expect("deliveries")
}
