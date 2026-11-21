//! The WebTransport transport: an HTTP/3 session at `/rt`.
//!
//! Reliable frames travel on one bidirectional stream opened by the client,
//! each prefixed with its length as 4 bytes, big-endian. Datagrams carry one
//! frame each, without a prefix: location samples and pings upwards, friends'
//! positions (and the answers to datagram pings) downwards.
use std::{net::SocketAddr, time::Duration};

use bytes::{Buf as _, Bytes, BytesMut};
use tokio::io::AsyncReadExt as _;
use tokio_util::sync::CancellationToken;
use wtransport::{
    Connection, Endpoint, Identity, RecvStream, SendStream, ServerConfig, VarInt,
    config::QuicTransportConfig,
    endpoint::{IncomingSession, endpoint_side::Server},
    error::SendDatagramError,
    quinn,
};

use crate::{
    Gateway,
    client::Transport,
    config::RealtimeConfig,
    connection::{self, FrameSink, FrameSource, Inbound, RecvError, SendFailed, Sent},
    frames::Delivery,
    outbox::CloseReason,
};

pub(crate) const PATH: &str = "/rt";

const LENGTH_PREFIX: usize = 4;
/// Streams a client may open at once. The session needs three
/// unidirectional HTTP/3 streams and two bidirectional ones (the CONNECT
/// request and the frame stream); the rest is slack for browser internals.
const MAX_CLIENT_STREAMS: u32 = 16;
/// Bytes a client may have in flight, per stream and in total. Frames are
/// small, so this bounds what an idle or hostile connection can pin.
const STREAM_RECEIVE_WINDOW: u32 = 256 * 1024;
const CONNECTION_RECEIVE_WINDOW: u32 = 1024 * 1024;
const DATAGRAM_BUFFER_BYTES: usize = 256 * 1024;
/// QUIC keep-alives from our side: browsers give up on a silent connection
/// after about 30 s, and NAT bindings for UDP are short-lived.
const KEEP_ALIVE: Duration = Duration::from_secs(10);

/// The QUIC settings of the listener, kept to rebuild its configuration
/// when the certificate changes.
#[derive(Clone, Copy)]
struct Tuning {
    idle_timeout: Duration,
    keep_alive: Duration,
}

pub(crate) struct WtListener {
    endpoint: Endpoint<Server>,
    local_addr: SocketAddr,
    tuning: Tuning,
}

impl WtListener {
    pub(crate) fn bind(addr: SocketAddr, identity: Identity, cfg: &RealtimeConfig) -> anyhow::Result<Self> {
        let tuning = Tuning { idle_timeout: cfg.idle_timeout, keep_alive: KEEP_ALIVE.min(cfg.ping_interval) };
        let endpoint = Endpoint::server(server_config(addr, identity, tuning)?)
            .map_err(|e| anyhow::anyhow!("cannot listen for WebTransport on udp/{addr}: {e}"))?;
        let local_addr = endpoint.local_addr()?;
        Ok(Self { endpoint, local_addr, tuning })
    }

    pub(crate) fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Presents `identity` to connections accepted from now on. Open
    /// connections keep running on the certificate they were set up with.
    pub(crate) fn use_identity(&self, identity: Identity) -> anyhow::Result<()> {
        self.endpoint.reload_config(server_config(self.local_addr, identity, self.tuning)?, false)?;
        Ok(())
    }

    /// Accepts sessions until `stop` is cancelled.
    pub(crate) async fn serve(&self, gateway: Gateway, stop: CancellationToken) {
        loop {
            let incoming = tokio::select! {
                _ = stop.cancelled() => return,
                incoming = self.endpoint.accept() => incoming,
            };
            gateway.connections.spawn(accept(gateway.clone(), incoming));
        }
    }

    /// Closes whatever is still open and waits (briefly) for the peers to
    /// learn about it.
    pub(crate) async fn close(&self, grace: Duration) {
        self.endpoint.close(close_code(CloseReason::ShuttingDown), CloseReason::ShuttingDown.label().as_bytes());
        let _ = tokio::time::timeout(grace, self.endpoint.wait_idle()).await;
    }
}

fn server_config(addr: SocketAddr, identity: Identity, tuning: Tuning) -> anyhow::Result<ServerConfig> {
    let mut transport = QuicTransportConfig::default();
    transport
        .max_concurrent_bidi_streams(quinn::VarInt::from_u32(MAX_CLIENT_STREAMS))
        .max_concurrent_uni_streams(quinn::VarInt::from_u32(MAX_CLIENT_STREAMS))
        .stream_receive_window(quinn::VarInt::from_u32(STREAM_RECEIVE_WINDOW))
        .receive_window(quinn::VarInt::from_u32(CONNECTION_RECEIVE_WINDOW))
        .datagram_receive_buffer_size(Some(DATAGRAM_BUFFER_BYTES))
        .datagram_send_buffer_size(DATAGRAM_BUFFER_BYTES)
        .max_idle_timeout(Some(tuning.idle_timeout.try_into().map_err(|_| anyhow::anyhow!("TOKULY_RT_IDLE_TIMEOUT is too long for QUIC"))?))
        .keep_alive_interval(Some(tuning.keep_alive));
    Ok(ServerConfig::builder().with_bind_address(addr).with_custom_transport(identity, transport).build())
}

fn close_code(reason: CloseReason) -> VarInt {
    VarInt::from_u32(u32::from(reason.code()))
}

/// Takes one incoming QUIC connection through the handshake, the session
/// request and the opening of the frame stream, then serves it.
async fn accept(gateway: Gateway, incoming: IncomingSession) {
    let remote = incoming.remote_address();
    // An address already at its limit is turned away before the handshake
    // costs anything. (Its slot is only taken after the handshake, when
    // the address is proven: a forged source cannot use up someone's slots.)
    if gateway.ip_limiter.is_full(remote.ip()) {
        gateway.counters.connections_refused.incr();
        return incoming.refuse();
    }
    let setup_time = gateway.cfg.hello_timeout;
    let Ok(Ok(request)) = tokio::time::timeout(setup_time, incoming).await else { return };

    let path = request.path().split('?').next().unwrap_or_default();
    if path != PATH {
        gateway.counters.connections_refused.incr();
        return request.not_found().await;
    }
    if !gateway.origins.allows(request.origin(), Some(request.authority())) {
        gateway.counters.connections_refused.incr();
        tracing::debug!(origin = request.origin(), "WebTransport session from a foreign origin refused");
        return request.forbidden().await;
    }
    if gateway.at_capacity() {
        gateway.counters.connections_refused.incr();
        return request.too_many_requests().await;
    }
    // Held until the connection ends.
    let Some(_permit) = gateway.ip_limiter.admit(remote.ip()) else {
        gateway.counters.connections_refused.incr();
        tracing::debug!(client_ip = %remote.ip(), "WebTransport session over the per-address limit refused");
        return request.too_many_requests().await;
    };
    let Ok(connection) = request.accept().await else { return };

    // The stream only becomes visible once the client writes to it, which a
    // well-behaved client does at once (its `Hello`).
    let Ok(Ok((send, recv))) = tokio::time::timeout(setup_time, connection.accept_bi()).await else {
        connection.close(close_code(CloseReason::Fatal), b"no frame stream opened");
        return;
    };
    let source = WtSource { connection: connection.clone(), recv, frames: FrameBuffer::new(gateway.cfg.max_frame_bytes) };
    let sink = WtSink { connection, send };
    connection::serve(gateway.inner(), Transport::WebTransport, source, sink, remote).await;
}

/// Reassembles length-prefixed frames from stream bytes.
struct FrameBuffer {
    bytes: BytesMut,
    max_frame_bytes: usize,
}

impl FrameBuffer {
    fn new(max_frame_bytes: usize) -> Self {
        Self { bytes: BytesMut::with_capacity(4096), max_frame_bytes }
    }

    /// The next complete frame, if the buffer holds one.
    fn next_frame(&mut self) -> Result<Option<Bytes>, RecvError> {
        let Some(prefix) = self.bytes.first_chunk::<LENGTH_PREFIX>() else { return Ok(None) };
        let len = u32::from_be_bytes(*prefix) as usize;
        if len > self.max_frame_bytes {
            return Err(RecvError::FrameTooLarge);
        }
        if self.bytes.len() < LENGTH_PREFIX + len {
            return Ok(None);
        }
        self.bytes.advance(LENGTH_PREFIX);
        Ok(Some(self.bytes.split_to(len).freeze()))
    }
}

struct WtSource {
    connection: Connection,
    recv: RecvStream,
    frames: FrameBuffer,
}

impl FrameSource for WtSource {
    async fn recv(&mut self) -> Result<Option<Inbound>, RecvError> {
        loop {
            if let Some(frame) = self.frames.next_frame()? {
                return Ok(Some(Inbound { frame, datagram: false }));
            }
            // Cancel-safe: stream bytes only leave the stream when `read_buf`
            // completes, and they are in the buffer by then.
            tokio::select! {
                datagram = self.connection.receive_datagram() => {
                    let datagram = datagram.map_err(|_| RecvError::Lost)?;
                    return Ok(Some(Inbound { frame: datagram.payload(), datagram: true }));
                }
                read = self.recv.read_buf(&mut self.frames.bytes) => match read {
                    // The client finished its stream: it is done with us.
                    Ok(0) => return Ok(None),
                    Ok(_) => {}
                    Err(_) => return Err(RecvError::Lost),
                },
            }
        }
    }
}

struct WtSink {
    connection: Connection,
    send: SendStream,
}

impl FrameSink for WtSink {
    async fn send(&mut self, delivery: Delivery, frame: Bytes) -> Result<Sent, SendFailed> {
        if delivery == Delivery::Droppable && self.connection.max_datagram_size().is_some_and(|max| frame.len() <= max) {
            match self.connection.send_datagram(&frame) {
                Ok(()) => return Ok(Sent::AsDatagram),
                Err(SendDatagramError::NotConnected) => return Err(SendFailed),
                // The path MTU shrank or the peer takes no datagrams: use the stream.
                Err(SendDatagramError::TooLarge | SendDatagramError::UnsupportedByPeer) => {}
            }
        }
        let prefix = (frame.len() as u32).to_be_bytes();
        self.send.write_all(&prefix).await.map_err(|_| SendFailed)?;
        self.send.write_all(&frame).await.map_err(|_| SendFailed)?;
        Ok(Sent::Reliably)
    }

    async fn close(mut self, reason: CloseReason) {
        // Resolves once the client acknowledged everything written, so a
        // final `Error` frame is not cut off by the close below.
        let _ = self.send.finish().await;
        self.connection.close(close_code(reason), reason.label().as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefixed(payload: &[u8]) -> Vec<u8> {
        let mut bytes = (payload.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn frames_are_reassembled_across_reads() {
        let mut buffer = FrameBuffer::new(1024);
        let mut wire = prefixed(b"hello");
        wire.extend(prefixed(b""));
        wire.extend(prefixed(b"world!"));

        // Fed one byte at a time, frames appear exactly when complete.
        let mut frames = Vec::new();
        for byte in wire {
            buffer.bytes.extend_from_slice(&[byte]);
            while let Ok(Some(frame)) = buffer.next_frame() {
                frames.push(frame);
            }
        }
        assert_eq!(frames, vec![Bytes::from_static(b"hello"), Bytes::new(), Bytes::from_static(b"world!")]);
        assert!(buffer.bytes.is_empty());
    }

    #[test]
    fn several_frames_in_one_read_come_out_in_order() {
        let mut buffer = FrameBuffer::new(1024);
        buffer.bytes.extend_from_slice(&prefixed(b"a"));
        buffer.bytes.extend_from_slice(&prefixed(b"bc"));
        buffer.bytes.extend_from_slice(&[0, 0]); // half a prefix
        assert_eq!(buffer.next_frame().ok().flatten().unwrap(), Bytes::from_static(b"a"));
        assert_eq!(buffer.next_frame().ok().flatten().unwrap(), Bytes::from_static(b"bc"));
        assert!(matches!(buffer.next_frame(), Ok(None)));
    }

    #[test]
    fn oversized_frames_are_refused_from_the_prefix_alone() {
        let mut buffer = FrameBuffer::new(8);
        buffer.bytes.extend_from_slice(&9u32.to_be_bytes());
        assert!(matches!(buffer.next_frame(), Err(RecvError::FrameTooLarge)));
        let mut exact = FrameBuffer::new(8);
        exact.bytes.extend_from_slice(&prefixed(b"12345678"));
        assert!(matches!(exact.next_frame(), Ok(Some(_))));
    }
}
