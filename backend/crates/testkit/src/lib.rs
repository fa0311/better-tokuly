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
//! - a Valkey database apart from a dev server's (shared by test runs);
//! - private media and private-file buckets, so uploads never mix with a
//!   dev server's;
//! - a private ClickHouse database with `backend/clickhouse/*.sql` applied
//!   (`app.flush_analytics()` makes tracked events queryable);
//! - RPC calls dispatched in-process through the same auth middleware the
//!   real server uses.
//!
//! What the binary creates on the shared infrastructure is removed when it
//! exits, passed or failed; leftovers of killed runs are swept by later runs
//! once they are six hours old (`src/env.rs`).
mod buckets;
mod clickhouse;
mod env;
mod nats;
mod postgres;
mod run;
mod search_indexes;
mod valkey;

use std::time::Duration;

use axum::body::Body;
use buffa::Message as _;
use bytes::{Buf as _, BufMut as _, Bytes, BytesMut};
use futures::StreamExt as _;
use http_body_util::BodyExt as _;
use serde::{Serialize, de::DeserializeOwned};
use tokuly_core::{Core, auth::ClientMeta, events::Event};
use tokuly_identity::{Identity, IdentityConfig, UserRow};
pub use tokuly_proto::realtime::ServerFrame;
use tokuly_proto::{RealtimeEvent, common::SubjectType};
use tower::ServiceExt as _;
use uuid::Uuid;

/// A private prefix for resources owned by this test binary
/// (`tokuly_test_<started>_<id>`), suitable for keys in a shared service.
pub fn test_run_prefix() -> String {
    env::run().name("tokuly_test_", '_')
}

/// The Meilisearch index prefix of this test binary
/// (`tokuly_test_<started>_<id>`), for `SearchConfig::index_prefix`. Every
/// index whose name starts with it is deleted when the binary exits.
pub fn search_index_prefix() -> String {
    search_indexes::prefix(env::run())
}

fn random_suffix(len: usize) -> String {
    tokuly_core::ids::random_base62(len).to_ascii_lowercase()
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
///
/// A domain's pushes are read as its own event message, whatever else
/// travels to the same user or topic:
///
/// ```ignore
/// let mut pushes = app.listen_user(alice.id).await;
/// // ... the action ...
/// let event: WalletEvent = pushes.next_event(WAIT).await.expect("a wallet event");
/// let all: Vec<ChatEvent> = pushes.events(WAIT).await;
/// ```
///
/// [`Self::next`] and [`Self::drain`] return the frames themselves, for
/// tests that care about the envelope (the topic, the gateway's own
/// messages) or about what else was sent; `frame.event::<WalletEvent>()`
/// reads the event out of one.
pub struct FrameListener {
    subscriber: async_nats::Subscriber,
}

impl FrameListener {
    /// The next bus message within `deadline` that is a frame: revocations
    /// for the gateways (`tokuly_core::realtime::CONTROL_HEADER`) travel on
    /// the same subjects and are skipped, as gateways never forward them.
    async fn next_message(&mut self, deadline: tokio::time::Instant) -> Option<async_nats::Message> {
        loop {
            let message = tokio::time::timeout_at(deadline, self.subscriber.next()).await.ok()??;
            if tokuly_core::realtime::control_of(message.headers.as_ref()).is_none() {
                return Some(message);
            }
        }
    }

    /// The next frame, or `None` if nothing arrives within `timeout`.
    pub async fn next(&mut self, timeout: Duration) -> Option<ServerFrame> {
        let message = self.next_message(tokio::time::Instant::now() + timeout).await?;
        ServerFrame::decode_from_slice(&message.payload).ok()
    }

    /// Every frame that arrives within `window`.
    pub async fn drain(&mut self, window: Duration) -> Vec<ServerFrame> {
        let deadline = tokio::time::Instant::now() + window;
        let mut frames = Vec::new();
        while let Some(message) = self.next_message(deadline).await {
            if let Ok(frame) = ServerFrame::decode_from_slice(&message.payload) {
                frames.push(frame);
            }
        }
        frames
    }

    /// The next event of type `E`, skipping frames that carry something
    /// else; `None` if none arrives within `timeout`.
    pub async fn next_event<E: RealtimeEvent>(&mut self, timeout: Duration) -> Option<E> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let message = self.next_message(deadline).await?;
            if let Some(event) = ServerFrame::decode_from_slice(&message.payload).ok().and_then(|frame| frame.event()) {
                return Some(event);
            }
        }
    }

    /// Every event of type `E` that arrives within `window`.
    pub async fn events<E: RealtimeEvent>(&mut self, window: Duration) -> Vec<E> {
        self.drain(window).await.iter().filter_map(ServerFrame::event).collect()
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
        Self::start_with(migrations, |_| {}, register).await
    }

    /// [`Self::start`] with a changed configuration (another `secret`, say).
    pub async fn start_with(
        migrations: &[&str],
        configure: impl FnOnce(&mut tokuly_core::config::Config),
        register: impl FnOnce(&Core, &Identity, connectrpc::Router) -> connectrpc::Router,
    ) -> Self {
        let mut cfg = env::get(migrations).cfg.clone();
        cfg.database_max_connections = 5;
        cfg.smtp_url = String::new();
        cfg.otlp_endpoint = String::new();
        cfg.public_url = "http://localhost:8080".to_owned();
        configure(&mut cfg);
        let core = Core::connect(cfg).await.unwrap_or_else(|e| {
            panic!("testkit: cannot connect to the dev infrastructure ({e}). Run tools/dev/infra.sh up")
        });
        let identity = Identity::new(core.clone(), IdentityConfig::default()).expect("identity");
        let connect = register(&core, &identity, connectrpc::Router::new());
        let router = axum::Router::new()
            .fallback_service(connect.into_axum_service())
            .layer(axum::middleware::from_fn_with_state(core.auth_layer.clone(), tokuly_core::auth::auth_middleware));
        use rand::RngExt as _;
        let mut rng = rand::rng();
        let ip = std::net::IpAddr::from([
            10,
            rng.random_range(0..255u8),
            rng.random_range(0..255u8),
            rng.random_range(1..255u8),
        ]);
        Self { core, identity, router, ip }
    }

    /// Creates an active, email-verified user and signs them in. The handle
    /// gets a random suffix so tests sharing the database never collide;
    /// `name` becomes the display name.
    pub async fn user(&self, name: &str) -> TestUser {
        let handle = format!(
            "{}_{}",
            name.to_ascii_lowercase().replace(|c: char| !c.is_ascii_alphanumeric(), ""),
            random_suffix(6)
        );
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
        let issued =
            self.identity.create_session(&row, None, &ClientMeta::default()).await.expect("testkit: create session");
        TestUser { id: row.id, handle: row.handle, token: issued.access_token, session_id: issued.session_id }
    }

    fn request(
        &self,
        procedure: &str,
        user: Option<&TestUser>,
        content_type: &str,
        body: Bytes,
    ) -> http::Request<Body> {
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
    pub async fn call<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        procedure: &str,
        user: Option<&TestUser>,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        let body = serde_json::to_vec(request).expect("testkit: encode request");
        let response = self
            .router
            .clone()
            .oneshot(self.request(procedure, user, "application/json", Bytes::from(body)))
            .await
            .expect("router is infallible");
        let status = response.status().as_u16();
        let reason =
            response.headers().get("x-tokuly-error").and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
        let bytes = response.into_body().collect().await.expect("testkit: read body").to_bytes();
        if status != 200 {
            return Err(Self::error_from(status, reason, &bytes));
        }
        serde_json::from_slice(&bytes).map_err(|e| {
            panic!("testkit: {procedure} returned undecodable JSON ({e}): {}", String::from_utf8_lossy(&bytes))
        })
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
        let reason =
            response.headers().get("x-tokuly-error").and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
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
        let subscriber =
            self.core.bus.subscribe(tokuly_core::realtime::user_subject(user_id)).await.expect("testkit: subscribe");
        FrameListener { subscriber }
    }

    /// Subscribes to realtime frames sent to a topic, e.g. `live:<id>`.
    pub async fn listen_topic(&self, topic: &str) -> FrameListener {
        let subscriber =
            self.core.bus.subscribe(tokuly_core::realtime::topic_subject(topic)).await.expect("testkit: subscribe");
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
        let rows: Vec<(Uuid, String, Option<Uuid>, serde_json::Value, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
            "SELECT id, topic, actor_id, data, occurred_at FROM outbox WHERE topic LIKE $1 || '%' ORDER BY id",
        )
        .bind(topic_prefix)
        .fetch_all(&self.core.db)
        .await
        .expect("testkit: read outbox");
        rows.into_iter()
            .map(|(id, topic, actor_id, data, occurred_at)| Event { id, topic, actor_id, occurred_at, data })
            .collect()
    }

    /// Whether `subject.deleted` is in the outbox for this subject (`id`:
    /// its UUID, as the API gives it).
    pub async fn subject_deleted(&self, subject_type: SubjectType, id: &str) -> bool {
        let sent = serde_json::json!({ "subject_type": subject_type as i32, "subject_id": id });
        self.events(tokuly_core::outbound::SUBJECT_DELETED_TOPIC).await.iter().any(|event| event.data == sent)
    }

    /// Events with `topic_prefix` caused by `actor` — use this instead of
    /// [`Self::events`] to ignore other tests sharing the database.
    pub async fn events_by(&self, topic_prefix: &str, actor: Uuid) -> Vec<Event> {
        self.events(topic_prefix).await.into_iter().filter(|e| e.actor_id == Some(actor)).collect()
    }
}
