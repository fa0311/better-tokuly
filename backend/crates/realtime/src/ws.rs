//! The WebSocket transport: `GET /rt/ws`, one frame per binary message.
//!
//! This is the fallback for clients and networks without WebTransport
//! (HTTP/3 over UDP). It normally sits behind the edge proxy, which
//! terminates TLS and forwards the upgrade.
use std::net::SocketAddr;

use axum::{
    Router,
    extract::{
        ConnectInfo, State, WebSocketUpgrade,
        ws::{CloseFrame, Message, WebSocket},
    },
    response::{IntoResponse, Response},
    routing::{any, get},
};
use bytes::Bytes;
use futures::{
    SinkExt as _, StreamExt as _,
    stream::{SplitSink, SplitStream},
};
use http::{HeaderMap, StatusCode, header};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::{
    Gateway,
    client::Transport,
    connection::{self, FrameSink, FrameSource, Inbound, RecvError, SendFailed, Sent},
    frames::Delivery,
    outbox::CloseReason,
};

pub(crate) const PATH: &str = "/rt/ws";

/// Serves WebSocket upgrades and `/healthz` until `stop` is cancelled.
pub(crate) async fn serve(gateway: Gateway, listener: TcpListener, stop: CancellationToken) -> std::io::Result<()> {
    let app = Router::new()
        // `any`, not `get`: over HTTP/2 a WebSocket is opened with CONNECT.
        .route(PATH, any(upgrade))
        .route("/healthz", get(healthz))
        .with_state(gateway);
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(stop.cancelled_owned())
        .await
}

async fn healthz() -> &'static str {
    "ok"
}

async fn upgrade(State(gateway): State<Gateway>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, upgrade: WebSocketUpgrade) -> Response {
    let header = |name: header::HeaderName| headers.get(name).and_then(|value| value.to_str().ok());
    let refuse = |status: StatusCode, why: &'static str| {
        gateway.counters.connections_refused.incr();
        (status, why).into_response()
    };
    if !gateway.origins.allows(header(header::ORIGIN), header(header::HOST)) {
        tracing::debug!(origin = header(header::ORIGIN), "WebSocket connection from a foreign origin refused");
        return refuse(StatusCode::FORBIDDEN, "origin not allowed");
    }
    if gateway.at_capacity() {
        return refuse(StatusCode::SERVICE_UNAVAILABLE, "too many connections");
    }
    // Behind the edge proxy the TCP peer is the proxy; the client is in the header.
    let client_ip = gateway.trusted_proxies.client_of(peer.ip(), &headers);
    let Some(permit) = gateway.ip_limiter.admit(client_ip) else {
        tracing::debug!(%client_ip, "WebSocket connection over the per-address limit refused");
        return refuse(StatusCode::TOO_MANY_REQUESTS, "too many connections from this address");
    };
    let remote = SocketAddr::new(client_ip, if client_ip == peer.ip().to_canonical() { peer.port() } else { 0 });
    let max_frame_bytes = gateway.cfg.max_frame_bytes;
    let connections = gateway.connections.clone();
    upgrade
        // Twice our own limit, so an oversized frame is answered with a
        // proper `Error` by the session instead of a bare protocol failure.
        .max_message_size(max_frame_bytes * 2)
        .max_frame_size(max_frame_bytes * 2)
        .on_upgrade(move |socket| {
            connections.track_future(async move {
                // Held for as long as the connection lives (and released
                // unused if the upgrade never completes).
                let _permit = permit;
                let (sink, stream) = socket.split();
                connection::serve(gateway.inner(), Transport::WebSocket, WsSource { stream, max_frame_bytes }, WsSink { sink }, remote).await;
            })
        })
}

struct WsSource {
    stream: SplitStream<WebSocket>,
    max_frame_bytes: usize,
}

impl FrameSource for WsSource {
    async fn recv(&mut self) -> Result<Option<Inbound>, RecvError> {
        loop {
            return match self.stream.next().await {
                Some(Ok(Message::Binary(frame))) if frame.len() > self.max_frame_bytes => Err(RecvError::FrameTooLarge),
                Some(Ok(Message::Binary(frame))) => Ok(Some(Inbound { frame, datagram: false })),
                Some(Ok(Message::Text(_))) => Err(RecvError::NotBinary),
                // Protocol-level pings are answered by the WebSocket layer.
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                Some(Ok(Message::Close(_))) | None => Ok(None),
                Some(Err(_)) => Err(RecvError::Lost),
            };
        }
    }
}

struct WsSink {
    sink: SplitSink<WebSocket, Message>,
}

impl FrameSink for WsSink {
    async fn send(&mut self, _delivery: Delivery, frame: Bytes) -> Result<Sent, SendFailed> {
        self.sink.send(Message::Binary(frame)).await.map(|()| Sent::Reliably).map_err(|_| SendFailed)
    }

    async fn close(mut self, reason: CloseReason) {
        let frame = CloseFrame { code: reason.code(), reason: reason.label().into() };
        let _ = self.sink.send(Message::Close(Some(frame))).await;
        let _ = self.sink.close().await;
    }
}
