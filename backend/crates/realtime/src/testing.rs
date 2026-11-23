//! A gateway and a client for other crates' tests (feature `testing`).
//!
//! ```ignore
//! let gateway = TestGateway::start(&app.core).await;
//! let mut client = gateway.client(Some(&alice.token)).await;
//! client.subscribe(&[&topic]).await.unwrap();
//! domain_does_something().await;
//! assert_eq!(client.unsubscribed(&topic, Duration::from_secs(1)).await.as_deref(), Some("TOPIC_FORBIDDEN"));
//! assert_eq!(client.subscribe(&[&topic]).await, Err("TOPIC_FORBIDDEN".into()));
//! ```
use std::{net::SocketAddr, time::Duration};

use buffa::Message as _;
use futures::{SinkExt as _, StreamExt as _};
use tokio::net::TcpStream;
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};
use tokuly_core::Core;
use tokuly_proto::{
    RealtimeEvent,
    realtime::{ClientFrame, Hello, ServerFrame, Subscribe, client_frame::Body as ClientBody, server_frame::Body},
};

use crate::{Gateway, RealtimeConfig};

/// How long the helpers wait for something that should happen promptly.
const WAIT: Duration = Duration::from_secs(5);

/// A running gateway on loopback ports (WebSocket only).
pub struct TestGateway {
    pub gateway: Gateway,
    pub websocket: SocketAddr,
}

impl TestGateway {
    /// Starts a gateway on `core` with the default timers.
    pub async fn start(core: &Core) -> Self {
        Self::start_with(core, RealtimeConfig::default()).await
    }

    /// Starts a gateway with `cfg`; its listeners are replaced by ephemeral
    /// loopback ports and WebTransport is off.
    pub async fn start_with(core: &Core, cfg: RealtimeConfig) -> Self {
        let cfg =
            RealtimeConfig { wt_listen: String::new(), ws_listen: "127.0.0.1:0".parse().expect("address"), ..cfg };
        let gateway = Gateway::new(core.clone(), cfg).expect("gateway configuration");
        tokio::spawn({
            let gateway = gateway.clone();
            async move { gateway.run().await }
        });
        let addrs = tokio::time::timeout(WAIT, gateway.listening()).await.expect("the gateway starts listening");
        Self { gateway, websocket: addrs.websocket }
    }

    /// A connection that said `Hello` with `token` (anonymous if `None`).
    pub async fn client(&self, token: Option<&str>) -> TestClient {
        let (stream, _) = tokio_tungstenite::connect_async(format!("ws://{}/rt/ws", self.websocket))
            .await
            .expect("WebSocket connection");
        let mut client = TestClient { stream, next_id: 1 };
        let hello =
            Hello { access_token: token.unwrap_or_default().to_owned(), foreground: true, ..Default::default() };
        let reply = client.request(ClientBody::Hello(Box::new(hello))).await;
        assert!(matches!(reply.body, Some(Body::Welcome(_))), "expected Welcome, got {:?}", reply.body);
        client
    }
}

/// One WebSocket connection speaking the frame protocol.
pub struct TestClient {
    stream: WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>,
    next_id: u64,
}

impl TestClient {
    /// The next frame within `wait`; `None` on timeout or when the connection ended.
    pub async fn next_within(&mut self, wait: Duration) -> Option<ServerFrame> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            match tokio::time::timeout_at(deadline, self.stream.next()).await.ok()?? {
                Ok(Message::Binary(bytes)) => {
                    return Some(ServerFrame::decode_from_slice(&bytes).expect("the gateway sends valid frames"));
                }
                Ok(Message::Close(_)) | Err(_) => return None,
                Ok(_) => {}
            }
        }
    }

    async fn request(&mut self, body: ClientBody) -> ServerFrame {
        let id = self.next_id;
        self.next_id += 1;
        let frame = ClientFrame { id, body: Some(body), ..Default::default() };
        self.stream.send(Message::Binary(frame.encode_to_vec().into())).await.expect("WebSocket send");
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let frame = self.next_within(remaining).await.expect("an answer from the gateway");
            if frame.reply_to == id {
                return frame;
            }
        }
    }

    /// Subscribes; `Err` holds the reason the gateway refused with.
    pub async fn subscribe(&mut self, topics: &[&str]) -> Result<(), String> {
        let subscribe =
            Subscribe { topics: topics.iter().map(|topic| topic.to_string()).collect(), ..Default::default() };
        let reply = self.request(ClientBody::Subscribe(Box::new(subscribe))).await;
        match reply.body {
            Some(Body::Ack(_)) => Ok(()),
            Some(Body::Error(error)) => Err(error.message),
            other => panic!("expected Ack or Error, got {other:?}"),
        }
    }

    /// Waits up to `wait` for `Unsubscribed` about `topic`, skipping other
    /// frames; its reason.
    pub async fn unsubscribed(&mut self, topic: &str, wait: Duration) -> Option<String> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let frame = self.next_within(remaining).await?;
            if let Some(Body::Unsubscribed(unsubscribed)) = frame.body
                && unsubscribed.topic == topic
            {
                return Some(unsubscribed.reason);
            }
        }
    }

    /// The next event of type `E` on `topic` within `wait`, skipping other frames.
    pub async fn next_event_on<E: RealtimeEvent>(&mut self, topic: &str, wait: Duration) -> Option<E> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let frame = self.next_within(remaining).await?;
            if frame.topic == topic
                && let Some(event) = frame.event::<E>()
            {
                return Some(event);
            }
        }
    }

    /// Whether no frame for `topic` (audience counts aside) arrives within `wait`.
    pub async fn hears_nothing_on(&mut self, topic: &str, wait: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            match self.next_within(remaining).await {
                Some(frame) if frame.topic == topic && !matches!(frame.body, Some(Body::TopicStats(_))) => {
                    return false;
                }
                Some(_) => {}
                None => return true,
            }
        }
    }
}
