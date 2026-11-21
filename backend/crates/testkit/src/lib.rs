//! Integration test harness for domain crates.
//!
//! ```ignore
//! #[tokio::test]
//! async fn sends_a_message() {
//!     let app = TestApp::start(&["0001", "0100", "0400", "0600"], |core, identity, router| {
//!         Chat::new(core.clone(), identity.clone()).register(router)
//!     })
//!     .await;
//!     let alice = app.user("alice").await;
//!     let reply: SendMessageResponse = app
//!         .call("tokuly.chat.v1.ChatService/SendMessage", Some(&alice), &SendMessageRequest { /* ... */ ..Default::default() })
//!         .await
//!         .unwrap();
//! }
//! ```
//!
//! What you get:
//! - a real [`Core`] connected to the development infrastructure (see
//!   `tools/dev/infra.sh`; settings come from the usual `TOKULY_*` variables);
//! - a private PostgreSQL database per test binary with only the migrations
//!   you list (so another domain's half-written migration cannot break you);
//! - a private NATS namespace, so jobs and events never reach a dev server;
//! - a private ClickHouse database with `backend/clickhouse/*.sql` applied
//!   (`app.flush_analytics()` makes tracked events queryable);
//! - RPC calls dispatched in-process through the same auth middleware the
//!   real server uses.
use std::{
    path::PathBuf,
    sync::OnceLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::body::Body;
use buffa::Message as _;
use bytes::{Buf as _, BufMut as _, Bytes, BytesMut};
use clap::Parser;
use futures::StreamExt as _;
use http_body_util::BodyExt as _;
use serde::{Serialize, de::DeserializeOwned};
use sqlx::{Connection as _, PgConnection};
use tokuly_core::{Core, auth::ClientMeta, config::Config, events::Event};
use tokuly_identity::{Identity, IdentityConfig, UserRow};
use tokuly_proto::realtime::ServerFrame;
use tower::ServiceExt as _;
use uuid::Uuid;

/// Everything shared by the tests of one test binary.
struct TestEnv {
    database_url: String,
    nats_prefix: String,
    /// Private ClickHouse database, when ClickHouse is reachable.
    clickhouse_database: Option<String>,
}

static ENV: OnceLock<TestEnv> = OnceLock::new();

#[derive(Parser)]
struct EnvConfig {
    #[command(flatten)]
    core: Config,
}

fn base_config() -> Config {
    // Reads TOKULY_* from the environment; the secret gets a test default.
    let mut args = vec!["tokuly-test".to_owned()];
    if std::env::var("TOKULY_SECRET").is_err() {
        args.push("--secret".to_owned());
        args.push("test-secret-test-secret-test-secret-test".to_owned());
    }
    EnvConfig::parse_from(args).core
}

fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

fn clickhouse_schema_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../clickhouse")
}

/// Creates a private ClickHouse database with every schema file applied.
/// Returns `None` when ClickHouse is not reachable (tests that need it will
/// then fail on their first analytics query, with a clear connection error).
async fn setup_clickhouse(cfg: &Config, name: &str, now: u64) -> Option<String> {
    let server = clickhouse::Client::default()
        .with_url(&cfg.clickhouse_url)
        .with_user(&cfg.clickhouse_user)
        .with_password(&cfg.clickhouse_password);
    let existing: Vec<String> = server.query("SELECT name FROM system.databases WHERE name LIKE 'tokuly_test_%'").fetch_all().await.ok()?;
    for database in existing {
        let started = database.split('_').nth(2).and_then(|t| t.parse::<u64>().ok()).unwrap_or(0);
        if now.saturating_sub(started) > 20 * 60 {
            let _ = server.query(&format!("DROP DATABASE IF EXISTS `{database}`")).execute().await;
        }
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(clickhouse_schema_dir())
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "sql"))
        .collect();
    files.sort();
    let statements: Vec<(String, String)> = files
        .iter()
        .map(|f| (f.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(f).expect("testkit: read ClickHouse schema")))
        .collect();
    let borrowed: Vec<(&str, &str)> = statements.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();
    let mut test_cfg = cfg.clone();
    test_cfg.clickhouse_database = name.to_owned();
    tokuly_core::analytics::apply_schema(&test_cfg, &borrowed)
        .await
        .unwrap_or_else(|e| panic!("testkit: ClickHouse schema failed: {e}"));
    Some(name.to_owned())
}

fn random_suffix(len: usize) -> String {
    tokuly_core::ids::random_base62(len).to_ascii_lowercase()
}

/// Replaces the database name in a PostgreSQL URL.
fn with_database(url: &str, database: &str) -> String {
    let (base, query) = url.split_once('?').map(|(b, q)| (b, Some(q))).unwrap_or((url, None));
    let prefix = base.rsplit_once('/').map(|(p, _)| p).unwrap_or(base);
    match query {
        Some(q) => format!("{prefix}/{database}?{q}"),
        None => format!("{prefix}/{database}"),
    }
}

async fn setup_env(migrations: Vec<String>) -> TestEnv {
    let cfg = base_config();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let name = format!("tokuly_test_{now}_{}", random_suffix(6));
    let mut admin = PgConnection::connect(&cfg.database_url)
        .await
        .unwrap_or_else(|e| panic!("testkit: cannot connect to PostgreSQL at TOKULY_DATABASE_URL ({e}). Start the dev infrastructure: tools/dev/infra.sh up && eval \"$(tools/dev/infra.sh env)\""));

    // Drop databases left behind by test runs that started more than 20 minutes ago.
    let stale: Vec<String> = sqlx::query_scalar("SELECT datname FROM pg_database WHERE datname LIKE 'tokuly_test_%'")
        .fetch_all(&mut admin)
        .await
        .unwrap_or_default();
    for db in stale {
        let started = db.split('_').nth(2).and_then(|t| t.parse::<u64>().ok()).unwrap_or(0);
        if now.saturating_sub(started) > 20 * 60 {
            let _ = sqlx::query(sqlx::AssertSqlSafe(format!("DROP DATABASE IF EXISTS \"{db}\" WITH (FORCE)"))).execute(&mut admin).await;
        }
    }
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE \"{name}\"")))
        .execute(&mut admin)
        .await
        .expect("testkit: CREATE DATABASE failed");
    drop(admin);

    let database_url = with_database(&cfg.database_url, &name);
    let mut conn = PgConnection::connect(&database_url).await.expect("testkit: connect to the test database");
    let mut files: Vec<PathBuf> = std::fs::read_dir(migrations_dir())
        .expect("testkit: backend/migrations is readable")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "sql"))
        .collect();
    files.sort();
    for wanted in &migrations {
        assert!(
            files.iter().any(|f| f.file_name().unwrap().to_string_lossy().starts_with(&format!("{wanted}_"))),
            "testkit: no migration file starts with `{wanted}_`"
        );
    }
    for file in files {
        let file_name = file.file_name().unwrap().to_string_lossy().into_owned();
        let prefix = file_name.split('_').next().unwrap_or_default();
        if !migrations.iter().any(|m| m == prefix) {
            continue;
        }
        let sql = std::fs::read_to_string(&file).expect("testkit: read migration");
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("testkit: migration {file_name} failed: {e}"));
    }
    let clickhouse_database = setup_clickhouse(&cfg, &name, now).await;
    TestEnv { database_url, nats_prefix: format!("T{now}_{}", random_suffix(6)), clickhouse_database }
}

/// A user created for a test, with a valid access token.
#[derive(Debug, Clone)]
pub struct TestUser {
    pub id: Uuid,
    pub handle: String,
    pub token: String,
    pub session_id: Uuid,
}

/// An RPC failure as a client would see it.
#[derive(Debug, Clone)]
pub struct RpcError {
    /// HTTP status of the response.
    pub status: u16,
    /// Connect code, e.g. `not_found`.
    pub code: String,
    /// Machine-readable reason from the `x-tokuly-error` header, e.g. `HANDLE_TAKEN`.
    pub reason: String,
    pub message: String,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} [{}] {}", self.code, self.reason, self.message)
    }
}

/// Collects realtime frames published to one user or topic.
pub struct FrameListener {
    subscriber: async_nats::Subscriber,
}

impl FrameListener {
    /// The next frame, or `None` if nothing arrives within `timeout`.
    pub async fn next(&mut self, timeout: Duration) -> Option<ServerFrame> {
        let message = tokio::time::timeout(timeout, self.subscriber.next()).await.ok()??;
        ServerFrame::decode_from_slice(&message.payload).ok()
    }

    /// Every frame that arrives within `window`.
    pub async fn drain(&mut self, window: Duration) -> Vec<ServerFrame> {
        let deadline = tokio::time::Instant::now() + window;
        let mut frames = Vec::new();
        while let Ok(Some(message)) = tokio::time::timeout_at(deadline, self.subscriber.next()).await {
            if let Ok(frame) = ServerFrame::decode_from_slice(&message.payload) {
                frames.push(frame);
            }
        }
        frames
    }
}

pub struct TestApp {
    pub core: Core,
    pub identity: Identity,
    router: axum::Router,
    /// Fake client IP for this app instance, so rate limits never collide across tests.
    ip: std::net::IpAddr,
}

impl TestApp {
    /// Starts an app with the given migrations (numeric file prefixes such as
    /// `"0001"`) and RPC services. Always include `"0001"`, `"0100"` and
    /// `"0400"` (core, identity, media) plus your own and your dependencies'.
    ///
    /// All tests of one test binary share a database, so use distinct users
    /// per test ([`Self::user`] makes handles unique for you).
    pub async fn start(
        migrations: &[&str],
        register: impl FnOnce(&Core, &Identity, connectrpc::Router) -> connectrpc::Router,
    ) -> Self {
        let wanted: Vec<String> = migrations.iter().map(|m| m.to_string()).collect();
        let env = ENV.get_or_init(|| {
            // Each #[tokio::test] has its own runtime; do the one-time setup on a dedicated one.
            std::thread::spawn(move || {
                tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime").block_on(setup_env(wanted))
            })
            .join()
            .expect("testkit: environment setup panicked")
        });
        let mut cfg = base_config();
        cfg.database_url = env.database_url.clone();
        cfg.nats_prefix = env.nats_prefix.clone();
        if let Some(database) = &env.clickhouse_database {
            cfg.clickhouse_database = database.clone();
        }
        cfg.database_max_connections = 5;
        cfg.smtp_url = String::new();
        cfg.otlp_endpoint = String::new();
        cfg.public_url = "http://localhost:8080".to_owned();
        let core = Core::connect(cfg)
            .await
            .unwrap_or_else(|e| panic!("testkit: cannot connect to the dev infrastructure ({e}). Run tools/dev/infra.sh up"));
        let identity = Identity::new(core.clone(), IdentityConfig::default()).expect("identity");
        let connect = register(&core, &identity, connectrpc::Router::new());
        let router = axum::Router::new()
            .fallback_service(connect.into_axum_service())
            .layer(axum::middleware::from_fn_with_state(core.auth_layer.clone(), tokuly_core::auth::auth_middleware));
        use rand::RngExt as _;
        let mut rng = rand::rng();
        let ip = std::net::IpAddr::from([10, rng.random_range(0..255u8), rng.random_range(0..255u8), rng.random_range(1..255u8)]);
        Self { core, identity, router, ip }
    }

    /// Creates an active, email-verified user and signs them in. The handle
    /// gets a random suffix so tests sharing the database never collide;
    /// `name` becomes the display name.
    pub async fn user(&self, name: &str) -> TestUser {
        let handle = format!("{}_{}", name.to_ascii_lowercase().replace(|c: char| !c.is_ascii_alphanumeric(), ""), random_suffix(6));
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO users (handle, display_name, email, email_verified_at, birthdate)
             VALUES ($1::citext, $2, $3::citext, now(), '2000-01-01') RETURNING id",
        )
        .bind(&handle)
        .bind(name)
        .bind(format!("{handle}@test.tokuly.localhost"))
        .fetch_one(&self.core.db)
        .await
        .expect("testkit: insert user");
        self.sign_in(id).await
    }

    /// A user holding the given roles (e.g. `"admin"`, `"moderator"`).
    pub async fn user_with_roles(&self, name: &str, roles: &[&str]) -> TestUser {
        let user = self.user(name).await;
        sqlx::query("UPDATE users SET roles = $2 WHERE id = $1")
            .bind(user.id)
            .bind(roles.iter().map(|r| r.to_string()).collect::<Vec<_>>())
            .execute(&self.core.db)
            .await
            .expect("testkit: set roles");
        self.sign_in(user.id).await
    }

    /// A fresh session (and token) for an existing user.
    pub async fn sign_in(&self, user_id: Uuid) -> TestUser {
        let row: UserRow = tokuly_identity::users::get(&self.core, user_id).await.expect("testkit: load user");
        let issued = self.identity.create_session(&row, None, &ClientMeta::default()).await.expect("testkit: create session");
        TestUser { id: row.id, handle: row.handle, token: issued.access_token, session_id: issued.session_id }
    }

    fn request(&self, procedure: &str, user: Option<&TestUser>, content_type: &str, body: Bytes) -> http::Request<Body> {
        let mut builder = http::Request::builder()
            .method("POST")
            .uri(format!("/{}", procedure.trim_start_matches('/')))
            .header("content-type", content_type)
            .header("accept-language", "ja")
            // Requests never cross a socket here, so say who the peer is.
            .extension(axum::extract::ConnectInfo(std::net::SocketAddr::new(self.ip, 0)));
        if let Some(user) = user {
            builder = builder.header("authorization", format!("Bearer {}", user.token));
        }
        builder.body(Body::from(body)).expect("request")
    }

    fn error_from(status: u16, reason: String, body: &[u8]) -> RpcError {
        let json: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
        RpcError {
            status,
            code: json.get("code").and_then(|c| c.as_str()).unwrap_or("unknown").to_owned(),
            reason,
            message: json.get("message").and_then(|m| m.as_str()).unwrap_or_default().to_owned(),
        }
    }

    /// Calls a unary RPC, e.g. `"tokuly.identity.v1.AccountService/GetMe"`.
    /// `user` of `None` makes an anonymous call.
    pub async fn call<Req: Serialize, Resp: DeserializeOwned>(&self, procedure: &str, user: Option<&TestUser>, request: &Req) -> Result<Resp, RpcError> {
        let body = serde_json::to_vec(request).expect("testkit: encode request");
        let response = self
            .router
            .clone()
            .oneshot(self.request(procedure, user, "application/json", Bytes::from(body)))
            .await
            .expect("router is infallible");
        let status = response.status().as_u16();
        let reason = response.headers().get("x-tokuly-error").and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
        let bytes = response.into_body().collect().await.expect("testkit: read body").to_bytes();
        if status != 200 {
            return Err(Self::error_from(status, reason, &bytes));
        }
        serde_json::from_slice(&bytes)
            .map_err(|e| panic!("testkit: {procedure} returned undecodable JSON ({e}): {}", String::from_utf8_lossy(&bytes)))
    }

    /// Calls a server-streaming RPC and returns the first `take` messages
    /// (fewer if the stream ends or `timeout` passes first). The stream is
    /// cancelled afterwards.
    pub async fn stream<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        procedure: &str,
        user: Option<&TestUser>,
        request: &Req,
        take: usize,
        timeout: Duration,
    ) -> Result<Vec<Resp>, RpcError> {
        let json = serde_json::to_vec(request).expect("testkit: encode request");
        let mut body = BytesMut::with_capacity(json.len() + 5);
        body.put_u8(0);
        body.put_u32(json.len() as u32);
        body.extend_from_slice(&json);
        let response = self
            .router
            .clone()
            .oneshot(self.request(procedure, user, "application/connect+json", body.freeze()))
            .await
            .expect("router is infallible");
        let status = response.status().as_u16();
        let reason = response.headers().get("x-tokuly-error").and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
        let mut stream = response.into_body().into_data_stream();
        if status != 200 {
            let mut bytes = Vec::new();
            while let Some(Ok(chunk)) = stream.next().await {
                bytes.extend_from_slice(&chunk);
            }
            return Err(Self::error_from(status, reason, &bytes));
        }
        let mut messages = Vec::new();
        let mut buffer = BytesMut::new();
        let deadline = tokio::time::Instant::now() + timeout;
        while messages.len() < take {
            // Decode every complete envelope in the buffer.
            while buffer.len() >= 5 {
                let flags = buffer[0];
                let len = u32::from_be_bytes([buffer[1], buffer[2], buffer[3], buffer[4]]) as usize;
                if buffer.len() < 5 + len {
                    break;
                }
                buffer.advance(5);
                let payload = buffer.split_to(len);
                if flags & 0x02 != 0 {
                    // End of stream: either clean or carrying an error.
                    let end: serde_json::Value = serde_json::from_slice(&payload).unwrap_or_default();
                    if let Some(error) = end.get("error") {
                        if messages.is_empty() {
                            let reason = end
                                .pointer("/metadata/x-tokuly-error/0")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_owned();
                            return Err(Self::error_from(200, reason, error.to_string().as_bytes()));
                        }
                    }
                    return Ok(messages);
                }
                messages.push(serde_json::from_slice(&payload).expect("testkit: decode stream message"));
                if messages.len() >= take {
                    return Ok(messages);
                }
            }
            match tokio::time::timeout_at(deadline, stream.next()).await {
                Ok(Some(Ok(chunk))) => buffer.extend_from_slice(&chunk),
                _ => break,
            }
        }
        Ok(messages)
    }

    /// Subscribes to realtime frames sent to a user. Subscribe *before* the
    /// action that should produce them.
    pub async fn listen_user(&self, user_id: Uuid) -> FrameListener {
        let subscriber = self.core.bus.subscribe(tokuly_core::realtime::user_subject(user_id)).await.expect("testkit: subscribe");
        FrameListener { subscriber }
    }

    /// Subscribes to realtime frames sent to a topic, e.g. `live:<id>`.
    pub async fn listen_topic(&self, topic: &str) -> FrameListener {
        let subscriber = self.core.bus.subscribe(tokuly_core::realtime::topic_subject(topic)).await.expect("testkit: subscribe");
        FrameListener { subscriber }
    }

    /// Moves analytics events tracked so far into ClickHouse, so queries see
    /// them (no sink runs in tests). Returns how many were written.
    pub async fn flush_analytics(&self) -> usize {
        self.core.analytics.drain().await.expect("testkit: analytics drain")
    }

    /// Domain events currently in the outbox whose topic starts with
    /// `topic_prefix` (no relay runs in tests, so they stay there).
    pub async fn events(&self, topic_prefix: &str) -> Vec<Event> {
        let rows: Vec<(Uuid, String, Option<Uuid>, serde_json::Value, chrono::DateTime<chrono::Utc>)> =
            sqlx::query_as("SELECT id, topic, actor_id, data, occurred_at FROM outbox WHERE topic LIKE $1 || '%' ORDER BY id")
                .bind(topic_prefix)
                .fetch_all(&self.core.db)
                .await
                .expect("testkit: read outbox");
        rows.into_iter().map(|(id, topic, actor_id, data, occurred_at)| Event { id, topic, actor_id, occurred_at, data }).collect()
    }

    /// Events with `topic_prefix` caused by `actor` — use this instead of
    /// [`Self::events`] to ignore other tests sharing the database.
    pub async fn events_by(&self, topic_prefix: &str, actor: Uuid) -> Vec<Event> {
        self.events(topic_prefix).await.into_iter().filter(|e| e.actor_id == Some(actor)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_name_is_swapped_in_urls() {
        assert_eq!(with_database("postgres://u:p@h:5432/tokuly", "t1"), "postgres://u:p@h:5432/t1");
        assert_eq!(with_database("postgres://u@h/tokuly?sslmode=disable", "t1"), "postgres://u@h/t1?sslmode=disable");
    }
}
