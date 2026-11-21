//! Publishing realtime frames to connected clients.
//!
//! Any service can push a [`ServerFrame`] to a user (all of their devices) or
//! to a topic. Frames are encoded once and travel over core NATS to whichever
//! gateway instances hold the matching connections; the gateways forward the
//! bytes untouched. Delivery is best-effort — anything that must not be lost
//! also lives in the database and is fetched on (re)connect.
use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use buffa::Message as _;
use bytes::Bytes;
use parking_lot::RwLock;
use tokuly_proto::realtime::{ServerFrame, server_frame::Body};
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

impl Realtime {
    pub fn new(bus: Bus) -> Self {
        Self { bus }
    }

    fn encode(topic: &str, body: Body) -> Bytes {
        let frame = ServerFrame {
            topic: topic.to_owned(),
            body: Some(body),
            ..Default::default()
        };
        Bytes::from(frame.encode_to_vec())
    }

    /// Sends an event to every connection of one user.
    pub async fn to_user(&self, user_id: Uuid, body: Body) -> AppResult<()> {
        self.bus
            .publish(user_subject(user_id), Self::encode("", body))
            .await
    }

    /// Sends the same event to several users.
    pub async fn to_users(
        &self,
        user_ids: impl IntoIterator<Item = Uuid>,
        body: Body,
    ) -> AppResult<()> {
        let payload = Self::encode("", body);
        for user_id in user_ids {
            self.bus
                .publish(user_subject(user_id), payload.clone())
                .await?;
        }
        Ok(())
    }

    /// Sends an event to every connection subscribed to `topic`.
    pub async fn to_topic(&self, topic: &str, body: Body) -> AppResult<()> {
        self.bus
            .publish(topic_subject(topic), Self::encode(topic, body))
            .await
    }

    /// Like [`Self::to_user`] but never fails the caller: realtime delivery is
    /// an optimization on top of state that is already committed.
    pub async fn notify_user(&self, user_id: Uuid, body: Body) {
        if let Err(e) = self.to_user(user_id, body).await {
            tracing::warn!(%user_id, error = %e, "realtime publish failed");
        }
    }

    /// Like [`Self::to_users`] but never fails the caller.
    pub async fn notify_users(&self, user_ids: impl IntoIterator<Item = Uuid>, body: Body) {
        if let Err(e) = self.to_users(user_ids, body).await {
            tracing::warn!(error = %e, "realtime publish failed");
        }
    }

    /// Like [`Self::to_topic`] but never fails the caller.
    pub async fn notify_topic(&self, topic: &str, body: Body) {
        if let Err(e) = self.to_topic(topic, body).await {
            tracing::warn!(topic, error = %e, "realtime publish failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subjects_are_valid_nats_tokens() {
        let user = Uuid::now_v7();
        assert_eq!(user_subject(user), format!("rt.u.{}", user.simple()));
        assert_eq!(topic_subject("live:abc-123"), "rt.t.live:abc-123");
        assert_eq!(topic_subject("a b.c*>"), "rt.t.a_b_c__");
    }
}
