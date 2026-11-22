//! Publishing realtime events to connected clients.
//!
//! Any service can push an event to a user (all of their devices) or to a
//! topic. A domain hands over its own event message and nothing else:
//!
//! ```ignore
//! core.rt.notify_user(user_id, &WalletEvent { event: Some(change.into()), ..Default::default() }).await;
//! ```
//!
//! [`Realtime`] puts it into the gateway's envelope (a `ServerFrame` whose
//! body is an [`Event`]: the message's full name plus its encoded bytes),
//! encodes that once and sends it over core NATS to whichever gateway
//! instances hold the matching connections; the gateways forward the bytes
//! untouched. Delivery is best-effort — anything that must not be lost also
//! lives in the database and is fetched on (re)connect.
//!
//! Reading a frame back (tests, a streaming RPC that follows the bus):
//! `frame.event::<WalletEvent>()`.
use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use buffa::{Message, MessageName};
use bytes::Bytes;
use parking_lot::RwLock;
use tokuly_proto::realtime::{Event, PresenceEvent, ServerFrame, server_frame::Body};
use uuid::Uuid;

use crate::{bus::Bus, error::AppResult};

/// Decides who may subscribe to a kind of realtime topic.
///
/// Topics are named `<kind>:<id>` (`live:<stream>`, `party:<id>`...). A kind
/// without a guard is public: anyone, even anonymous connections, may
/// subscribe. The domain that owns a kind with restricted audiences registers
/// a guard for it in [`TopicGuards`] when it is constructed, and the gateway
/// asks it on every subscribe.
#[async_trait]
pub trait TopicGuard: Send + Sync + 'static {
    /// Whether `subscriber` (`None` = anonymous) may follow `topic`.
    async fn may_subscribe(&self, subscriber: Option<Uuid>, topic: &str) -> AppResult<bool>;
}

/// Registry of topic guards by topic kind.
#[derive(Default)]
pub struct TopicGuards {
    guards: RwLock<HashMap<String, Arc<dyn TopicGuard>>>,
}

impl TopicGuards {
    /// Registers the guard for a topic kind (the part before the first `:`).
    pub fn register(&self, kind: &str, guard: Arc<dyn TopicGuard>) {
        self.guards.write().insert(kind.to_owned(), guard);
    }

    /// Whether `subscriber` may follow `topic`; unguarded kinds are public.
    pub async fn may_subscribe(&self, subscriber: Option<Uuid>, topic: &str) -> AppResult<bool> {
        let kind = topic.split(':').next().unwrap_or_default();
        let guard = self.guards.read().get(kind).cloned();
        match guard {
            Some(guard) => guard.may_subscribe(subscriber, topic).await,
            None => Ok(true),
        }
    }
}

#[derive(Clone)]
pub struct Realtime {
    bus: Bus,
}

/// NATS subject for frames addressed to one user.
pub fn user_subject(user_id: Uuid) -> String {
    format!("rt.u.{}", user_id.simple())
}

/// NATS subject for frames addressed to a topic. Topic names may contain
/// `:`, letters, digits, `-` and `_`; anything else is replaced.
pub fn topic_subject(topic: &str) -> String {
    let safe: String = topic
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("rt.t.{safe}")
}

/// The encoded frame that carries `body` to `topic` (empty: to a user).
fn encode(topic: &str, body: Body) -> Bytes {
    let frame = ServerFrame { topic: topic.to_owned(), body: Some(body), ..Default::default() };
    Bytes::from(frame.encode_to_vec())
}

/// The encoded frame that carries a domain's event.
fn encode_event<E: Message + MessageName>(topic: &str, event: &E) -> Bytes {
    encode(topic, Body::Event(Box::new(Event::pack(event))))
}

impl Realtime {
    pub fn new(bus: Bus) -> Self {
        Self { bus }
    }

    async fn publish_to_users(&self, user_ids: impl IntoIterator<Item = Uuid>, frame: Bytes) -> AppResult<()> {
        for user_id in user_ids {
            self.bus.publish(user_subject(user_id), frame.clone()).await?;
        }
        Ok(())
    }

    /// Sends an event to every connection of one user.
    pub async fn to_user<E: Message + MessageName>(&self, user_id: Uuid, event: &E) -> AppResult<()> {
        self.bus.publish(user_subject(user_id), encode_event("", event)).await
    }

    /// Sends the same event to several users.
    pub async fn to_users<E: Message + MessageName>(
        &self,
        user_ids: impl IntoIterator<Item = Uuid>,
        event: &E,
    ) -> AppResult<()> {
        self.publish_to_users(user_ids, encode_event("", event)).await
    }

    /// Sends an event to every connection subscribed to `topic`.
    pub async fn to_topic<E: Message + MessageName>(&self, topic: &str, event: &E) -> AppResult<()> {
        self.bus.publish(topic_subject(topic), encode_event(topic, event)).await
    }

    /// Like [`Self::to_user`] but never fails the caller: realtime delivery is
    /// an optimization on top of state that is already committed.
    pub async fn notify_user<E: Message + MessageName>(&self, user_id: Uuid, event: &E) {
        if let Err(e) = self.to_user(user_id, event).await {
            tracing::warn!(%user_id, error = %e, "realtime publish failed");
        }
    }

    /// Like [`Self::to_users`] but never fails the caller.
    pub async fn notify_users<E: Message + MessageName>(&self, user_ids: impl IntoIterator<Item = Uuid>, event: &E) {
        if let Err(e) = self.to_users(user_ids, event).await {
            tracing::warn!(error = %e, "realtime publish failed");
        }
    }

    /// Like [`Self::to_topic`] but never fails the caller.
    pub async fn notify_topic<E: Message + MessageName>(&self, topic: &str, event: &E) {
        if let Err(e) = self.to_topic(topic, event).await {
            tracing::warn!(topic, error = %e, "realtime publish failed");
        }
    }

    /// Tells users that someone they follow came online or went offline.
    /// Presence is part of the gateway's own protocol (`ServerFrame.presence`)
    /// rather than a domain's event, so it has its own way out.
    pub async fn presence_to_users(
        &self,
        user_ids: impl IntoIterator<Item = Uuid>,
        presence: PresenceEvent,
    ) -> AppResult<()> {
        self.publish_to_users(user_ids, encode("", Body::Presence(Box::new(presence)))).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_travel_by_name_and_decode_as_their_type() {
        let pong = tokuly_proto::realtime::Pong { nonce: 7, ..Default::default() };
        let frame = ServerFrame::decode_from_slice(&encode_event("live:1", &pong)).unwrap();
        assert_eq!(frame.topic, "live:1");
        let Some(Body::Event(event)) = &frame.body else { panic!("expected an event body") };
        assert_eq!(event.r#type, "tokuly.realtime.v1.Pong");
        assert_eq!(frame.event::<tokuly_proto::realtime::Pong>().map(|pong| pong.nonce), Some(7));
        // Asking for another type finds nothing, and neither does a frame of the gateway itself.
        assert!(frame.event::<PresenceEvent>().is_none());
        let presence = encode("", Body::Presence(Box::new(PresenceEvent::default())));
        assert!(ServerFrame::decode_from_slice(&presence).unwrap().event::<PresenceEvent>().is_none());
    }

    #[test]
    fn subjects_are_valid_nats_tokens() {
        let user = Uuid::now_v7();
        assert_eq!(user_subject(user), format!("rt.u.{}", user.simple()));
        assert_eq!(topic_subject("live:abc-123"), "rt.t.live:abc-123");
        assert_eq!(topic_subject("a b.c*>"), "rt.t.a_b_c__");
    }
}
