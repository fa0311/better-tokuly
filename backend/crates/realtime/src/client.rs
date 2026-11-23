//! The handle through which the rest of the gateway reaches one connection.
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU64, Ordering},
};

use bytes::Bytes;
use uuid::Uuid;

use crate::{
    frames::Delivery,
    outbox::{Outbox, OutboxLimits, Pushed},
    stats::Counters,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Transport {
    WebTransport,
    WebSocket,
}

impl Transport {
    /// The name reported in `Welcome.transport`.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Transport::WebTransport => "webtransport",
            Transport::WebSocket => "websocket",
        }
    }
}

static NEXT_KEY: AtomicU64 = AtomicU64::new(1);

/// One connected client, as seen by fan-out: an identity and an outbox.
pub(crate) struct Client {
    /// Told to the client in `Welcome.connection_id`; also in every log line.
    pub id: Uuid,
    /// Process-unique key for membership maps (cheaper than the UUID).
    pub key: u64,
    pub transport: Transport,
    pub outbox: Outbox,
    /// The user the connection signed in as; set once, never changed (a
    /// connection may sign in late but not become someone else). Topic
    /// frames that name this user in their exception list skip it.
    pub user: OnceLock<Uuid>,
    counters: Arc<Counters>,
}

impl Client {
    pub(crate) fn new(transport: Transport, limits: OutboxLimits, counters: Arc<Counters>) -> Arc<Self> {
        Arc::new(Self {
            id: Uuid::now_v7(),
            key: NEXT_KEY.fetch_add(1, Ordering::Relaxed),
            transport,
            outbox: Outbox::new(limits),
            user: OnceLock::new(),
            counters,
        })
    }

    /// Queues a frame for this client. Never blocks; a client that cannot
    /// keep up loses droppable frames or, for reliable ones, its connection.
    pub(crate) fn send(&self, delivery: Delivery, frame: Bytes) {
        match self.outbox.push(delivery, frame) {
            Pushed::Queued | Pushed::Closed => {}
            Pushed::ReplacedOldest => self.counters.frames_dropped.incr(),
            Pushed::Overflowed => {
                self.counters.slow_consumer_disconnects.incr();
                tracing::info!(connection = %self.id, transport = self.transport.as_str(), "disconnecting slow consumer");
            }
        }
    }

    /// Queues one of the gateway's own replies (always reliable).
    pub(crate) fn reply(&self, frame: Bytes) {
        self.send(Delivery::Reliable, frame);
    }
}
