//! One client connection, from accepted transport to cleanup.
//!
//! The two transports differ only in how frames get on and off the wire.
//! Each provides a [`FrameSource`] and a [`FrameSink`]; everything else —
//! the protocol ([`Session`]), the outbox and its writer — is shared.
use std::{future::Future, net::SocketAddr, sync::Arc, time::Duration};

use bytes::Bytes;

use crate::{
    GatewayInner,
    client::{Client, Transport},
    frames::Delivery,
    outbox::{CloseReason, Next, OutboxLimits},
    session::Session,
};

/// How long closing a transport may take once its last frame is written.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);
/// The same when the peer is gone or not reading: a courtesy, not a wait.
const ABORT_CLOSE_TIMEOUT: Duration = Duration::from_millis(500);

/// One frame received from a client.
pub(crate) struct Inbound {
    pub frame: Bytes,
    /// Arrived as a datagram rather than on the reliable channel.
    pub datagram: bool,
}

pub(crate) enum RecvError {
    /// The frame exceeds `max_frame_bytes`.
    FrameTooLarge,
    /// A WebSocket text message; frames are binary.
    NotBinary,
    /// The connection broke.
    Lost,
}

/// The receiving half of a transport.
pub(crate) trait FrameSource: Send {
    /// The next frame, or `None` once the peer closed the connection.
    /// Must be cancel-safe: the session polls it alongside its timers.
    fn recv(&mut self) -> impl Future<Output = Result<Option<Inbound>, RecvError>> + Send;
}

/// How a frame left the gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sent {
    Reliably,
    AsDatagram,
}

/// The connection broke while writing.
pub(crate) struct SendFailed;

/// The sending half of a transport.
pub(crate) trait FrameSink: Send + 'static {
    fn send(&mut self, delivery: Delivery, frame: Bytes) -> impl Future<Output = Result<Sent, SendFailed>> + Send;
    /// Closes the connection, telling the peer why.
    fn close(self, reason: CloseReason) -> impl Future<Output = ()> + Send;
}

/// Runs a connection to its end. Returns when the peer is gone and
/// everything the connection held (routes, presence, counters) is released.
pub(crate) async fn serve(gateway: Arc<GatewayInner>, transport: Transport, mut source: impl FrameSource, sink: impl FrameSink, remote: SocketAddr) {
    let limits = OutboxLimits {
        reliable_frames: gateway.cfg.queue_frames,
        reliable_bytes: gateway.cfg.queue_bytes,
        droppable_frames: gateway.cfg.queue_droppable_frames,
    };
    let client = Client::new(transport, limits, gateway.counters.clone());
    let open_connections = match transport {
        Transport::WebTransport => &gateway.counters.webtransport_connections,
        Transport::WebSocket => &gateway.counters.websocket_connections,
    };
    open_connections.incr();
    gateway.counters.connections_accepted.incr();
    tracing::debug!(connection = %client.id, transport = transport.as_str(), %remote, "realtime connection opened");

    let writer = tokio::spawn(write_frames(gateway.clone(), client.clone(), sink));
    let mut session = Session::new(&gateway, client.clone());
    let reason = session.run(&mut source).await;
    session.finish();

    match reason {
        // Nobody is listening anymore: do not bother flushing.
        CloseReason::PeerClosed | CloseReason::SlowConsumer => client.outbox.abort(reason),
        // Let the queued explanation (a fatal `Error` frame) go out first.
        CloseReason::ShuttingDown | CloseReason::Fatal | CloseReason::Idle => client.outbox.close(reason),
    }
    let mut writer = writer;
    if tokio::time::timeout(gateway.cfg.write_timeout + CLOSE_TIMEOUT, &mut writer).await.is_err() {
        writer.abort();
    }
    open_connections.decr();
    tracing::debug!(connection = %client.id, transport = transport.as_str(), reason = reason.label(), "realtime connection closed");
}

/// Writes the client's outbox to the wire until the connection closes.
/// This is the only place that waits on a client's network speed.
async fn write_frames(gateway: Arc<GatewayInner>, client: Arc<Client>, mut sink: impl FrameSink) {
    loop {
        match client.outbox.next().await {
            Next::Frame(delivery, frame) => {
                let written = tokio::select! {
                    // The connection was given up on (e.g. its backlog
                    // overflowed while this very write was stalled).
                    _ = client.outbox.aborted() => continue,
                    written = tokio::time::timeout(gateway.cfg.write_timeout, sink.send(delivery, frame)) => written,
                };
                match written {
                    Ok(Ok(sent)) => {
                        gateway.counters.frames_out.incr();
                        if sent == Sent::AsDatagram {
                            gateway.counters.datagrams_out.incr();
                        }
                    }
                    Ok(Err(SendFailed)) => client.outbox.abort(CloseReason::PeerClosed),
                    Err(_stalled) => {
                        gateway.counters.slow_consumer_disconnects.incr();
                        client.outbox.abort(CloseReason::SlowConsumer);
                    }
                }
            }
            Next::Close(reason) => {
                let patience = if client.outbox.is_aborted() { ABORT_CLOSE_TIMEOUT } else { CLOSE_TIMEOUT };
                let _ = tokio::time::timeout(patience, sink.close(reason)).await;
                return;
            }
        }
    }
}
