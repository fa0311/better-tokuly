//! The handle through which the rest of the gateway reaches one connection.
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU64, Ordering},
};

use bytes::Bytes;
use parking_lot::Mutex;
use tokio::sync::Notify;
use tokuly_core::realtime::{Control, TopicPattern};
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

/// The most control messages one connection queues; beyond that they are
/// folded into a single "re-check everything".
const MAX_QUEUED_CONTROLS: usize = 256;

/// An instruction for a connection's session about its subscriptions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionControl {
    /// Ask the guard again, now, for the subscriptions matching the pattern
    /// (one user's: a handful of topics at most).
    Recheck(TopicPattern),
    /// Ask the guard again about this topic soon, within the gateway's
    /// re-check budget: a topic-wide re-check reaches every subscriber at
    /// once, which for a big audience must not mean every guard query at once.
    RecheckSoon(String),
    /// Drop this topic without asking.
    Close(String),
}

impl SessionControl {
    /// What a control message received on a route means for its members.
    /// `topic` is the route's topic (`None` for a user route).
    pub(crate) fn from_bus(control: Control, topic: Option<&str>) -> Option<Self> {
        match (control, topic) {
            (Control::Recheck(pattern), None) => Some(SessionControl::Recheck(pattern)),
            (Control::RecheckTopic, Some(topic)) => Some(SessionControl::RecheckSoon(topic.to_owned())),
            (Control::CloseTopic, Some(topic)) => Some(SessionControl::Close(topic.to_owned())),
            // An instruction sent to the wrong kind of subject.
            _ => None,
        }
    }
}

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
    /// Instructions for the session, in arrival order (see [`Self::control`]).
    controls: Mutex<Vec<SessionControl>>,
    /// Wakes the session when `controls` gained an entry.
    controls_ready: Notify,
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
            controls: Mutex::new(Vec::new()),
            controls_ready: Notify::new(),
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

    /// Hands an instruction to the session, which applies it between two
    /// client frames: so a revocation that arrives while a `Subscribe` is
    /// being checked is applied after that subscription exists. Never
    /// blocks; a flood is folded into one re-check of every subscription.
    pub(crate) fn control(&self, control: SessionControl) {
        let mut controls = self.controls.lock();
        if controls.contains(&control) {
            return;
        }
        if controls.len() >= MAX_QUEUED_CONTROLS {
            let everything = SessionControl::Recheck(TopicPattern::Prefix(String::new()));
            // Closing is decided without the guard; keep those, re-check the rest.
            controls.retain(|queued| matches!(queued, SessionControl::Close(_)));
            if matches!(control, SessionControl::Close(_)) && controls.len() < MAX_QUEUED_CONTROLS {
                controls.push(control);
            }
            controls.truncate(MAX_QUEUED_CONTROLS - 1);
            controls.push(everything);
        } else {
            controls.push(control);
        }
        drop(controls);
        self.controls_ready.notify_one();
    }

    /// Resolves when an instruction may be waiting.
    pub(crate) async fn controls_ready(&self) {
        self.controls_ready.notified().await;
    }

    /// Takes the queued instructions.
    pub(crate) fn take_controls(&self) -> Vec<SessionControl> {
        std::mem::take(&mut *self.controls.lock())
    }

    /// Queues one of the gateway's own replies (always reliable).
    pub(crate) fn reply(&self, frame: Bytes) {
        self.send(Delivery::Reliable, frame);
    }
}
