//! Shared helpers for the gateway tests: a gateway on ephemeral ports plus
//! minimal WebSocket and WebTransport clients speaking the frame protocol.
#![allow(dead_code)]

use std::{net::SocketAddr, time::Duration};

use buffa::Message as _;
use bytes::Bytes;
use ed25519_dalek::{Signer as _, SigningKey};
use futures::{SinkExt as _, StreamExt as _};
use serde::de::DeserializeOwned;
use tokio::{net::TcpStream, task::JoinHandle};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest as _, protocol::CloseFrame},
};
use tokuly_core::keys::b64url;
use tokuly_proto::realtime::{
    AppState, ClientFrame, Error as ErrorFrame, GetEndpointsRequest, GetEndpointsResponse, Hello, Ping, ServerFrame, Subscribe, Unsubscribe, Welcome,
    client_frame::Body as ClientBody, server_frame::Body,
};
use tokuly_realtime::{Gateway, ListenAddrs, RealtimeConfig};
use tokuly_testkit::{TestApp, TestUser};
use uuid::Uuid;
use wtransport::{ClientConfig, Connection, Endpoint, RecvStream, SendStream, tls::Sha256Digest};

/// What the gateway puts on the bus for the geo pipeline.
pub type LocationReport = tokuly_core::presence::LocationReport<tokuly_proto::geo::LocationSample>;

/// How long a test waits for something that should happen promptly.
pub const WAIT: Duration = Duration::from_secs(5);
/// How long a test listens to be reasonably sure nothing arrives.
pub const QUIET: Duration = Duration::from_millis(400);

const MIGRATIONS: &[&str] = &["0001", "0100", "0400"];

/// A config with listeners on ephemeral loopback ports and timers short
/// enough for tests; override what a test is about.
pub fn test_config() -> RealtimeConfig {
    RealtimeConfig {
        wt_listen: "127.0.0.1:0".into(),
        ws_listen: "127.0.0.1:0".parse().unwrap(),
        presence_heartbeat: Duration::from_millis(300),
        audience_refresh: Duration::from_millis(500),
        topic_stats_interval: Duration::from_millis(100),
        revocation_check: Duration::from_millis(200),
        shutdown_grace: Duration::from_secs(2),
        ..Default::default()
    }
}

/// A running gateway and the app it belongs to.
pub struct Harness {
    pub app: TestApp,
    pub gateway: Gateway,
    pub addrs: ListenAddrs,
    /// Resolves when `Gateway::run` returns.
    pub running: JoinHandle<anyhow::Result<()>>,
}

impl Harness {
    pub async fn start() -> Self {
        Self::start_with(test_config()).await
    }

    pub async fn start_with(cfg: RealtimeConfig) -> Self {
        let mut created = None;
        let app = TestApp::start(MIGRATIONS, |core, _identity, router| {
            let gateway = Gateway::new(core.clone(), cfg).expect("gateway configuration");
            let router = gateway.register(router);
            created = Some(gateway);
            router
        })
        .await;
        let gateway = created.expect("gateway was created");
        let (addrs, running) = run(&gateway).await;
        Self { app, gateway, addrs, running }
    }

    /// A second gateway process on the same infrastructure.
    pub async fn another_gateway(&self, cfg: RealtimeConfig) -> (Gateway, ListenAddrs) {
        let gateway = Gateway::new(self.app.core.clone(), cfg).expect("gateway configuration");
        let (addrs, _running) = run(&gateway).await;
        (gateway, addrs)
    }

    pub async fn ws(&self) -> WsClient {
        WsClient::connect(self.addrs.websocket).await
    }

    /// A WebSocket connection that said `Hello` as `user` (anonymous if `None`).
    pub async fn ws_as(&self, user: Option<&TestUser>, foreground: bool) -> WsClient {
        let mut client = self.ws().await;
        client.hello(user.map(|user| user.token.as_str()).unwrap_or_default(), foreground).await;
        client
    }

    pub async fn endpoints(&self) -> GetEndpointsResponse {
        self.app.call("tokuly.realtime.v1.RealtimeService/GetEndpoints", None, &GetEndpointsRequest::default()).await.expect("GetEndpoints")
    }

    /// Waits for `Gateway::run` to return (after `core.shutdown` was cancelled).
    pub async fn stopped(&mut self) -> anyhow::Result<()> {
        tokio::time::timeout(WAIT * 2, &mut self.running).await.expect("Gateway::run returns after shutdown").expect("Gateway::run does not panic")
    }

    pub fn wt_url(&self) -> String {
        format!("https://127.0.0.1:{}/rt", self.addrs.webtransport.expect("WebTransport is enabled").port())
    }

    /// A WebTransport connection pinned to the published certificate hashes.
    pub async fn wt(&self) -> WtClient {
        let hashes = self.endpoints().await.webtransport_cert_hashes;
        WtClient::connect(&self.wt_url(), &hashes).await.expect("WebTransport connection")
    }
}

async fn run(gateway: &Gateway) -> (ListenAddrs, JoinHandle<anyhow::Result<()>>) {
    let running = tokio::spawn({
        let gateway = gateway.clone();
        async move { gateway.run().await }
    });
    let addrs = tokio::time::timeout(WAIT, gateway.listening()).await.expect("gateway starts listening");
    (addrs, running)
}

/// A topic name no other test uses.
pub fn unique_topic(kind: &str) -> String {
    format!("{kind}:{}", Uuid::now_v7())
}

/// Polls until `check` holds; panics with `what` after [`WAIT`].
pub async fn eventually(what: &str, mut check: impl AsyncFnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !check().await {
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting until {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// The next JSON message on a bus subscription that satisfies `wanted`.
pub async fn next_on_bus<T: DeserializeOwned>(subscriber: &mut async_nats::Subscriber, wait: Duration, wanted: impl Fn(&T) -> bool) -> Option<T> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let message = tokio::time::timeout_at(deadline, subscriber.next()).await.ok()??;
        if let Ok(value) = serde_json::from_slice::<T>(&message.payload)
            && wanted(&value)
        {
            return Some(value);
        }
    }
}

/// An access token for `user` that the gateway accepts but that expires in
/// `lifetime_seconds`, signed like the identity service does (the test
/// shares the master secret).
pub fn short_lived_token(app: &TestApp, user: &TestUser, lifetime_seconds: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let header = b64url(br#"{"alg":"EdDSA","typ":"JWT"}"#);
    let claims = serde_json::json!({ "sub": user.id, "sid": user.session_id, "iat": now, "exp": now + lifetime_seconds, "iss": "tokuly" });
    let signing_input = format!("{header}.{}", b64url(claims.to_string().as_bytes()));
    let key = SigningKey::from_bytes(&app.core.keys.derive("jwt/ed25519"));
    let signature = key.sign(signing_input.as_bytes());
    format!("{signing_input}.{}", b64url(&signature.to_bytes()))
}

fn encode(frame: &ClientFrame) -> Bytes {
    Bytes::from(frame.encode_to_vec())
}

fn decode(bytes: &[u8]) -> ServerFrame {
    ServerFrame::decode_from_slice(bytes).expect("the gateway sends valid frames")
}

pub fn hello_body(token: &str, foreground: bool, topics: &[&str]) -> ClientBody {
    ClientBody::Hello(Box::new(Hello {
        access_token: token.to_owned(),
        foreground,
        topics: topics.iter().map(|topic| topic.to_string()).collect(),
        app_version: "test".into(),
        ..Default::default()
    }))
}

pub fn subscribe_body(topics: &[&str]) -> ClientBody {
    ClientBody::Subscribe(Box::new(Subscribe { topics: topics.iter().map(|topic| topic.to_string()).collect(), ..Default::default() }))
}

pub fn unsubscribe_body(topics: &[&str]) -> ClientBody {
    ClientBody::Unsubscribe(Box::new(Unsubscribe { topics: topics.iter().map(|topic| topic.to_string()).collect(), ..Default::default() }))
}

pub fn ping_body(nonce: u64) -> ClientBody {
    ClientBody::Ping(Box::new(Ping { nonce, ..Default::default() }))
}

pub fn app_state_body(foreground: bool) -> ClientBody {
    ClientBody::AppState(Box::new(AppState { foreground, ..Default::default() }))
}

/// The `Error` in a frame; panics if the frame is something else.
pub fn error_of(frame: &ServerFrame) -> &ErrorFrame {
    match &frame.body {
        Some(Body::Error(error)) => error,
        other => panic!("expected an Error frame, got {other:?}"),
    }
}

pub fn is_ack(frame: &ServerFrame) -> bool {
    matches!(frame.body, Some(Body::Ack(_)))
}

pub fn is_topic_stats(frame: &ServerFrame) -> bool {
    matches!(frame.body, Some(Body::TopicStats(_)))
}

// ───────────────────────────── WebSocket client ─────────────────────────────

/// What a test may vary about a WebSocket connection attempt.
#[derive(Default, Clone, Copy)]
pub struct WsOptions<'a> {
    /// The `Origin` header (browsers send one, native apps do not).
    pub origin: Option<&'a str>,
    /// The `X-Forwarded-For` header, as an edge proxy would set it.
    pub forwarded_for: Option<&'a str>,
    /// Size of the socket's receive buffer (small = stalls the sender soon).
    pub receive_buffer: Option<u32>,
}

pub struct WsClient {
    stream: WebSocketStream<TcpStream>,
    next_id: u64,
}

impl WsClient {
    pub async fn connect(addr: SocketAddr) -> Self {
        Self::connect_with(addr, WsOptions::default()).await.expect("WebSocket connection")
    }

    /// Connects with extra request headers or socket settings. Errors carry
    /// the HTTP status the upgrade was refused with.
    pub async fn connect_with(addr: SocketAddr, options: WsOptions<'_>) -> Result<Self, String> {
        let socket = tokio::net::TcpSocket::new_v4().expect("socket");
        if let Some(size) = options.receive_buffer {
            socket.set_recv_buffer_size(size).expect("receive buffer size");
        }
        let tcp = socket.connect(addr).await.expect("TCP connection to the gateway");
        let mut request = format!("ws://{addr}/rt/ws").into_client_request().expect("request");
        for (name, value) in [("origin", options.origin), ("x-forwarded-for", options.forwarded_for)] {
            if let Some(value) = value {
                request.headers_mut().insert(name, value.parse().expect("header value"));
            }
        }
        match tokio_tungstenite::client_async(request, tcp).await {
            Ok((stream, _response)) => Ok(Self { stream, next_id: 1 }),
            Err(error) => Err(error.to_string()),
        }
    }

    /// Sends a frame with a fresh non-zero ID and returns that ID.
    pub async fn send(&mut self, body: ClientBody) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send_frame(ClientFrame { id, body: Some(body), ..Default::default() }).await;
        id
    }

    /// Sends a frame with ID 0 (no answer wanted).
    pub async fn send_unanswered(&mut self, body: ClientBody) {
        self.send_frame(ClientFrame { id: 0, body: Some(body), ..Default::default() }).await;
    }

    pub async fn send_frame(&mut self, frame: ClientFrame) {
        self.send_raw(Message::Binary(encode(&frame))).await;
    }

    pub async fn send_raw(&mut self, message: Message) {
        self.stream.send(message).await.expect("WebSocket send");
    }

    /// The next frame within `wait`; `None` on timeout or when the connection ended.
    pub async fn next_within(&mut self, wait: Duration) -> Option<ServerFrame> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            match tokio::time::timeout_at(deadline, self.stream.next()).await.ok()?? {
                Ok(Message::Binary(bytes)) => return Some(decode(&bytes)),
                Ok(Message::Close(_)) | Err(_) => return None,
                Ok(_) => {}
            }
        }
    }

    pub async fn next(&mut self) -> ServerFrame {
        self.next_within(WAIT).await.expect("a frame from the gateway")
    }

    /// The next frame satisfying `wanted`, skipping others.
    pub async fn next_where(&mut self, wanted: impl Fn(&ServerFrame) -> bool) -> ServerFrame {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let frame = self.next_within(remaining).await.expect("the awaited frame from the gateway");
            if wanted(&frame) {
                return frame;
            }
        }
    }

    /// The answer to the frame sent with `id`, skipping pushed events.
    pub async fn reply_to(&mut self, id: u64) -> ServerFrame {
        self.next_where(|frame| frame.reply_to == id).await
    }

    /// Sends a frame and returns its answer.
    pub async fn request(&mut self, body: ClientBody) -> ServerFrame {
        let id = self.send(body).await;
        self.reply_to(id).await
    }

    pub async fn hello(&mut self, token: &str, foreground: bool) -> Welcome {
        let reply = self.request(hello_body(token, foreground, &[])).await;
        match reply.body {
            Some(Body::Welcome(welcome)) => *welcome,
            other => panic!("expected Welcome, got {other:?}"),
        }
    }

    /// Whether anything other than `TopicStats` arrives within [`QUIET`].
    pub async fn stays_quiet(&mut self) -> bool {
        let deadline = tokio::time::Instant::now() + QUIET;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            match self.next_within(remaining).await {
                Some(frame) if is_topic_stats(&frame) => {}
                Some(_) => return false,
                None => return true,
            }
        }
    }

    /// Waits for the server to close the connection; returns its close frame.
    pub async fn closed(&mut self) -> Option<CloseFrame> {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            match tokio::time::timeout_at(deadline, self.stream.next()).await.expect("the gateway closes the connection") {
                Some(Ok(Message::Close(frame))) => return frame,
                Some(Ok(_)) => {}
                Some(Err(_)) | None => return None,
            }
        }
    }

    pub async fn close(mut self) {
        let _ = self.stream.close(None).await;
    }
}

// ──────────────────────────── WebTransport client ────────────────────────────

pub struct WtClient {
    pub connection: Connection,
    send: SendStream,
    recv: RecvStream,
    next_id: u64,
}

impl WtClient {
    /// Connects the way a browser does with `serverCertificateHashes`:
    /// only a certificate with one of these hashes is accepted.
    pub async fn connect(url: &str, cert_hashes: &[Vec<u8>]) -> Result<Self, String> {
        let hashes = cert_hashes.iter().map(|hash| Sha256Digest::new(hash.as_slice().try_into().expect("a SHA-256 hash is 32 bytes")));
        let config = ClientConfig::builder().with_bind_address("127.0.0.1:0".parse().unwrap()).with_server_certificate_hashes(hashes).build();
        let endpoint = Endpoint::client(config).map_err(|e| e.to_string())?;
        let connection = tokio::time::timeout(WAIT, endpoint.connect(url)).await.map_err(|_| "timed out".to_owned())?.map_err(|e| e.to_string())?;
        // The one bidirectional stream that carries reliable frames.
        let (send, recv) = connection.open_bi().await.map_err(|e| e.to_string())?.await.map_err(|e| e.to_string())?;
        Ok(Self { connection, send, recv, next_id: 1 })
    }

    /// SHA-256 of the certificate the gateway presented.
    pub fn presented_cert_hash(&self) -> Vec<u8> {
        let chain = self.connection.peer_identity().expect("the server presented a certificate");
        chain.as_slice()[0].hash().as_ref().to_vec()
    }

    pub async fn send(&mut self, body: ClientBody) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send_raw(&encode(&ClientFrame { id, body: Some(body), ..Default::default() })).await;
        id
    }

    /// Writes one length-prefixed frame to the stream.
    pub async fn send_raw(&mut self, frame: &[u8]) {
        self.send.write_all(&(frame.len() as u32).to_be_bytes()).await.expect("stream write");
        self.send.write_all(frame).await.expect("stream write");
    }

    pub fn send_datagram(&self, id: u64, body: ClientBody) {
        self.connection.send_datagram(encode(&ClientFrame { id, body: Some(body), ..Default::default() })).expect("datagram send");
    }

    /// The next frame on the stream within `wait`.
    pub async fn next_within(&mut self, wait: Duration) -> Option<ServerFrame> {
        let read = async {
            let mut prefix = [0u8; 4];
            self.recv.read_exact(&mut prefix).await.ok()?;
            let mut frame = vec![0u8; u32::from_be_bytes(prefix) as usize];
            self.recv.read_exact(&mut frame).await.ok()?;
            Some(decode(&frame))
        };
        tokio::time::timeout(wait, read).await.ok()?
    }

    pub async fn next(&mut self) -> ServerFrame {
        self.next_within(WAIT).await.expect("a frame on the stream")
    }

    pub async fn reply_to(&mut self, id: u64) -> ServerFrame {
        loop {
            let frame = self.next().await;
            if frame.reply_to == id {
                return frame;
            }
        }
    }

    pub async fn request(&mut self, body: ClientBody) -> ServerFrame {
        let id = self.send(body).await;
        self.reply_to(id).await
    }

    pub async fn hello(&mut self, token: &str, foreground: bool) -> Welcome {
        let reply = self.request(hello_body(token, foreground, &[])).await;
        match reply.body {
            Some(Body::Welcome(welcome)) => *welcome,
            other => panic!("expected Welcome, got {other:?}"),
        }
    }

    /// The next frame arriving as a datagram within `wait`.
    pub async fn next_datagram_within(&self, wait: Duration) -> Option<ServerFrame> {
        let datagram = tokio::time::timeout(wait, self.connection.receive_datagram()).await.ok()?.ok()?;
        Some(decode(&datagram.payload()))
    }

    pub async fn next_datagram(&self) -> ServerFrame {
        self.next_datagram_within(WAIT).await.expect("a datagram from the gateway")
    }
}
