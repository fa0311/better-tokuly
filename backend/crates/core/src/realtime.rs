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
//!
//! A topic frame can leave some users out ([`Realtime::to_topic_except`]):
//! the list travels in the [`EXCEPT_HEADER`] of the bus message, beside the
//! frame and never inside it, and each gateway skips the connections signed
//! in as one of them. This is how a live chat line stays away from people
//! in a block relation with its author, at the cost of one publish.
//!
//! ## Ending subscriptions
//!
//! A topic's guard runs when a connection subscribes. When access ends later
//! (a ban, a removal, a block, an unfriending, leaving, an audience narrowed
//! to members, a deleted room), the owning domain tells the gateways, after
//! its change is committed:
//!
//! - [`Realtime::revoke`] / [`Realtime::revoke_prefix`]: one user's
//!   subscriptions to a topic (or to every topic under a prefix) are checked
//!   against the guard again, on all of that user's connections, and dropped
//!   where the guard now refuses. Calling it when access did not change is
//!   harmless, so a domain may call it on every event that *might* end
//!   access, and twice.
//! - [`Realtime::recheck_topic`]: the same for every subscriber of a topic,
//!   anonymous ones included (an audience narrowed to friends or members).
//! - [`Realtime::close_topic`]: every subscriber is dropped without asking
//!   the guard (the room is gone, the stream or share is over).
//!
//! Each connection hears `ServerFrame.unsubscribed { topic, reason }` for a
//! dropped subscription. These are control messages on the same bus
//! subjects as the frames (`rt.u.<user>` and `rt.t.<topic>`), told apart by
//! the [`CONTROL_HEADER`] and carrying an empty payload, so whoever else
//! follows those subjects (a streaming RPC, the testkit) decodes them as an
//! empty frame and skips them. They are best-effort like every push: the
//! gateway also re-checks long-lived subscriptions against their guards
//! periodically, which heals a revocation that got lost.
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use async_trait::async_trait;
use buffa::Message as _;
use bytes::Bytes;
use parking_lot::RwLock;
use tokuly_proto::{
    RealtimeEvent,
    realtime::{Event, PresenceEvent, ServerFrame, server_frame::Body},
};
use uuid::Uuid;

use crate::{
    bus::Bus,
    error::{AppError, AppResult},
};

/// Bus header of a topic frame: the users (simple UUIDs, comma-separated)
/// whose connections must not get it.
pub const EXCEPT_HEADER: &str = "Tokuly-Except";

/// The most users one topic frame can leave out. The header then stays
/// under 33 KB, well inside a bus message; a publish that would need more
/// is refused (`TOO_MANY_EXCEPTIONS`) rather than sent to everyone.
pub const MAX_EXCEPT: usize = 1000;

/// Bus header that makes a message on `rt.u.<user>` / `rt.t.<topic>` an
/// instruction for the gateways instead of a frame for the clients. Values:
/// `recheck` (with [`CONTROL_TOPIC_HEADER`] on a user subject: that topic;
/// on a topic subject: the whole topic), `recheck-prefix` (user subject,
/// every topic under the prefix in [`CONTROL_TOPIC_HEADER`]) and `close`
/// (topic subject).
pub const CONTROL_HEADER: &str = "Tokuly-Control";

/// Bus header of a control message on a user subject: the topic, or the
/// prefix, it is about.
pub const CONTROL_TOPIC_HEADER: &str = "Tokuly-Topic";

/// Which subscriptions a user-scoped control message is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopicPattern {
    /// Exactly this topic.
    Exact(String),
    /// This topic and every topic below it (`party:<id>` covers
    /// `party:<id>:<key>`; `live` covers every `live:` topic; the empty
    /// prefix covers everything).
    Prefix(String),
}

impl TopicPattern {
    pub fn matches(&self, topic: &str) -> bool {
        match self {
            TopicPattern::Exact(exact) => topic == exact,
            TopicPattern::Prefix(prefix) => {
                prefix.is_empty()
                    || topic.strip_prefix(prefix.as_str()).is_some_and(|rest| rest.is_empty() || rest.starts_with(':'))
            }
        }
    }
}

/// What a control message asks the gateways to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Control {
    /// Re-run the guard for the user's subscriptions matching the pattern
    /// (only on a user subject).
    Recheck(TopicPattern),
    /// Re-run the guard for every subscriber of the topic (only on a topic
    /// subject).
    RecheckTopic,
    /// Drop every subscriber of the topic (only on a topic subject).
    CloseTopic,
}

/// The instruction a bus message carries, if it is a control message.
/// Unknown or malformed instructions read as `Some(None)`: a control
/// message all the same (never forwarded), but nothing to do.
pub fn control_of(headers: Option<&async_nats::HeaderMap>) -> Option<Option<Control>> {
    let headers = headers?;
    let kind = headers.get(CONTROL_HEADER)?.as_str();
    let topic = headers.get(CONTROL_TOPIC_HEADER).map(|value| value.as_str().to_owned());
    Some(match (kind, topic) {
        ("recheck", Some(topic)) => Some(Control::Recheck(TopicPattern::Exact(topic))),
        ("recheck", None) => Some(Control::RecheckTopic),
        ("recheck-prefix", Some(prefix)) => Some(Control::Recheck(TopicPattern::Prefix(prefix))),
        ("close", None) => Some(Control::CloseTopic),
        _ => None,
    })
}

/// The users a bus message's [`EXCEPT_HEADER`] names (empty without one).
/// Malformed entries are ignored.
pub fn except_of(headers: Option<&async_nats::HeaderMap>) -> HashSet<Uuid> {
    headers
        .and_then(|headers| headers.get(EXCEPT_HEADER))
        .map(|value| value.as_str().split(',').filter_map(|id| Uuid::parse_str(id.trim()).ok()).collect())
        .unwrap_or_default()
}

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
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_') { c } else { '_' })
        .collect();
    format!("rt.t.{safe}")
}

/// The encoded frame that carries `body` to `topic` (empty: to a user).
fn encode(topic: &str, body: Body) -> Bytes {
    let frame = ServerFrame { topic: topic.to_owned(), body: Some(body), ..Default::default() };
    Bytes::from(frame.encode_to_vec())
}

/// The encoded frame that carries a domain's event.
fn encode_event<E: RealtimeEvent>(topic: &str, event: &E) -> Bytes {
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
    pub async fn to_user<E: RealtimeEvent>(&self, user_id: Uuid, event: &E) -> AppResult<()> {
        self.bus.publish(user_subject(user_id), encode_event("", event)).await
    }

    /// Sends the same event to several users.
    pub async fn to_users<E: RealtimeEvent>(
        &self,
        user_ids: impl IntoIterator<Item = Uuid>,
        event: &E,
    ) -> AppResult<()> {
        self.publish_to_users(user_ids, encode_event("", event)).await
    }

    /// Sends an event to every connection subscribed to `topic`.
    pub async fn to_topic<E: RealtimeEvent>(&self, topic: &str, event: &E) -> AppResult<()> {
        self.bus.publish(topic_subject(topic), encode_event(topic, event)).await
    }

    /// Sends an event to every connection subscribed to `topic`, except the
    /// connections signed in as one of `except` (at most [`MAX_EXCEPT`];
    /// duplicates count once). One bus message, whatever the list holds.
    pub async fn to_topic_except<E: RealtimeEvent>(&self, topic: &str, event: &E, except: &[Uuid]) -> AppResult<()> {
        let except: HashSet<&Uuid> = except.iter().collect();
        if except.is_empty() {
            return self.to_topic(topic, event).await;
        }
        if except.len() > MAX_EXCEPT {
            return Err(AppError::precondition(
                "TOO_MANY_EXCEPTIONS",
                format!("a topic frame can leave out at most {MAX_EXCEPT} users"),
            ));
        }
        let list: Vec<String> = except.iter().map(|id| id.simple().to_string()).collect();
        let mut headers = async_nats::HeaderMap::new();
        headers.insert(EXCEPT_HEADER, list.join(",").as_str());
        self.bus.publish_with_headers(topic_subject(topic), headers, encode_event(topic, event)).await
    }

    /// Like [`Self::to_topic_except`] but never fails the caller.
    pub async fn notify_topic_except<E: RealtimeEvent>(&self, topic: &str, event: &E, except: &[Uuid]) {
        if let Err(e) = self.to_topic_except(topic, event, except).await {
            tracing::warn!(topic, error = %e, "realtime publish failed");
        }
    }

    /// Like [`Self::to_user`] but never fails the caller: realtime delivery is
    /// an optimization on top of state that is already committed.
    pub async fn notify_user<E: RealtimeEvent>(&self, user_id: Uuid, event: &E) {
        if let Err(e) = self.to_user(user_id, event).await {
            tracing::warn!(%user_id, error = %e, "realtime publish failed");
        }
    }

    /// Like [`Self::to_users`] but never fails the caller.
    pub async fn notify_users<E: RealtimeEvent>(&self, user_ids: impl IntoIterator<Item = Uuid>, event: &E) {
        if let Err(e) = self.to_users(user_ids, event).await {
            tracing::warn!(error = %e, "realtime publish failed");
        }
    }

    /// Like [`Self::to_topic`] but never fails the caller.
    pub async fn notify_topic<E: RealtimeEvent>(&self, topic: &str, event: &E) {
        if let Err(e) = self.to_topic(topic, event).await {
            tracing::warn!(topic, error = %e, "realtime publish failed");
        }
    }

    /// Publishes a control message (see [`CONTROL_HEADER`]). Never fails
    /// the caller: a lost one is healed by the gateways' periodic re-check.
    async fn control(&self, subject: String, kind: &str, topic: Option<&str>) {
        let mut headers = async_nats::HeaderMap::new();
        headers.insert(CONTROL_HEADER, kind);
        if let Some(topic) = topic {
            headers.insert(CONTROL_TOPIC_HEADER, topic);
        }
        if let Err(e) = self.bus.publish_with_headers(subject, headers, Bytes::new()).await {
            tracing::warn!(kind, topic, error = %e, "realtime control publish failed");
        }
    }

    /// Ends `user_id`'s subscriptions to `topic`, on every connection and
    /// gateway, unless the topic's guard still admits them. Call it after
    /// the change that ends access is committed; calling it when nothing
    /// changed costs one guard check per affected connection and drops
    /// nothing.
    pub async fn revoke(&self, user_id: Uuid, topic: &str) {
        self.control(user_subject(user_id), "recheck", Some(topic)).await;
    }

    /// [`Self::revoke`] for every topic under `prefix`: `"live"` for all of
    /// the user's `live:` subscriptions, `"party:<id>"` for a party's
    /// member and guest topics. Use it when the change may affect topics the
    /// domain cannot list cheaply (a block between two people and every
    /// stream of one of them).
    pub async fn revoke_prefix(&self, user_id: Uuid, prefix: &str) {
        self.control(user_subject(user_id), "recheck-prefix", Some(prefix)).await;
    }

    /// [`Self::revoke`] for several users.
    pub async fn revoke_users(&self, user_ids: impl IntoIterator<Item = Uuid>, topic: &str) {
        for user_id in user_ids {
            self.revoke(user_id, topic).await;
        }
    }

    /// Re-checks every subscriber of `topic` (anonymous connections too)
    /// against its guard and drops those it refuses: for an audience
    /// narrowed to friends or members.
    pub async fn recheck_topic(&self, topic: &str) {
        self.control(topic_subject(topic), "recheck", None).await;
    }

    /// Drops every subscriber of `topic` without asking the guard: the room
    /// was deleted, the stream or share is over.
    pub async fn close_topic(&self, topic: &str) {
        self.control(topic_subject(topic), "close", None).await;
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
        use tokuly_proto::{
            notify::{NotificationsRead, NotifyEvent},
            wallet::WalletEvent,
        };
        let read = NotificationsRead { bulk: true, ..Default::default() };
        let sent = NotifyEvent { event: Some(read.into()), ..Default::default() };
        let frame = ServerFrame::decode_from_slice(&encode_event("live:1", &sent)).unwrap();
        assert_eq!(frame.topic, "live:1");
        let Some(Body::Event(event)) = &frame.body else { panic!("expected an event body") };
        assert_eq!(event.r#type, "tokuly.notify.v1.NotifyEvent");
        assert_eq!(frame.event::<NotifyEvent>(), Some(sent));
        // Asking for another type finds nothing, and neither does a frame of the gateway itself.
        assert!(frame.event::<WalletEvent>().is_none());
        let presence = encode("", Body::Presence(Box::new(PresenceEvent::default())));
        assert!(ServerFrame::decode_from_slice(&presence).unwrap().event::<NotifyEvent>().is_none());
    }

    #[test]
    fn the_except_header_reads_back_as_the_users_it_names() {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let mut headers = async_nats::HeaderMap::new();
        headers.insert(EXCEPT_HEADER, format!("{},{}, nonsense", a.simple(), b.simple()).as_str());
        assert_eq!(except_of(Some(&headers)), HashSet::from([a, b]));
        assert!(except_of(None).is_empty());
        assert!(except_of(Some(&async_nats::HeaderMap::new())).is_empty());
    }

    #[test]
    fn patterns_match_whole_segments() {
        let exact = TopicPattern::Exact("party:1".into());
        assert!(exact.matches("party:1"));
        assert!(!exact.matches("party:1:key") && !exact.matches("party:12"));
        let prefix = TopicPattern::Prefix("party:1".into());
        assert!(prefix.matches("party:1") && prefix.matches("party:1:key"));
        assert!(!prefix.matches("party:12") && !prefix.matches("live:1"));
        assert!(TopicPattern::Prefix("live".into()).matches("live:abc"));
        assert!(!TopicPattern::Prefix("live".into()).matches("liver:abc"));
        assert!(TopicPattern::Prefix(String::new()).matches("anything:at-all"));
    }

    #[test]
    fn control_headers_read_back() {
        let headers = |kind: &str, topic: Option<&str>| {
            let mut headers = async_nats::HeaderMap::new();
            headers.insert(CONTROL_HEADER, kind);
            if let Some(topic) = topic {
                headers.insert(CONTROL_TOPIC_HEADER, topic);
            }
            headers
        };
        assert_eq!(
            control_of(Some(&headers("recheck", Some("live:1")))),
            Some(Some(Control::Recheck(TopicPattern::Exact("live:1".into()))))
        );
        assert_eq!(
            control_of(Some(&headers("recheck-prefix", Some("live")))),
            Some(Some(Control::Recheck(TopicPattern::Prefix("live".into()))))
        );
        assert_eq!(control_of(Some(&headers("recheck", None))), Some(Some(Control::RecheckTopic)));
        assert_eq!(control_of(Some(&headers("close", None))), Some(Some(Control::CloseTopic)));
        // A control message from a newer server: not a frame, but nothing to do.
        assert_eq!(control_of(Some(&headers("evict", None))), Some(None));
        assert_eq!(control_of(Some(&async_nats::HeaderMap::new())), None);
        assert_eq!(control_of(None), None);
    }

    #[test]
    fn subjects_are_valid_nats_tokens() {
        let user = Uuid::now_v7();
        assert_eq!(user_subject(user), format!("rt.u.{}", user.simple()));
        assert_eq!(topic_subject("live:abc-123"), "rt.t.live:abc-123");
        assert_eq!(topic_subject("a b.c*>"), "rt.t.a_b_c__");
    }
}
