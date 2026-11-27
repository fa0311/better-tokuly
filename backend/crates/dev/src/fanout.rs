//! From domain events to webhook deliveries.
//!
//! One durable consumer hears the topics the catalogue draws on. For each
//! event it works out which webhook event types it is and for which
//! accounts, and stores a delivery for every active webhook of those
//! accounts that subscribed to the type.
//!
//! Almost all events concern nobody with a webhook, so the common case is
//! made free: the set of event types that *any* active webhook subscribes
//! to is kept in memory, and an event of no subscribed type is dropped
//! without touching the database.
use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::RwLock;
use tokuly_core::{
    events::Event,
    outbound::{NOTIFY_TOPIC, NotificationRequest},
    prelude::*,
};

use crate::{
    Dev,
    delivery::envelope,
    webhooks::{EVENT_TYPES, EventType, Source},
};

/// How long the in-memory set of subscribed types is trusted. A new
/// subscription made on another server instance starts receiving within
/// this time; on the same instance, at once.
const SUBSCRIBED_TTL: Duration = Duration::from_secs(10);

/// The event types at least one active webhook subscribes to.
#[derive(Default)]
pub struct SubscribedTypes {
    cached: RwLock<Option<(Instant, Arc<HashSet<String>>)>>,
}

impl SubscribedTypes {
    /// Forgets the set: subscriptions changed.
    pub(crate) fn invalidate(&self) {
        *self.cached.write() = None;
    }

    fn fresh(&self) -> Option<Arc<HashSet<String>>> {
        self.cached
            .read()
            .as_ref()
            .filter(|(loaded, _)| loaded.elapsed() < SUBSCRIBED_TTL)
            .map(|(_, types)| types.clone())
    }
}

/// The topics the fan-out consumes: every source of the catalogue.
pub fn source_topics() -> Vec<&'static str> {
    let mut topics = vec![NOTIFY_TOPIC];
    for event_type in EVENT_TYPES {
        if let Source::Topic { topic, .. } = event_type.source {
            if !topics.contains(&topic) {
                topics.push(topic);
            }
        }
    }
    topics
}

/// Whether a notification of `kind` is of the kind an event type names.
fn kind_matches(wanted: &str, kind: &str) -> bool {
    wanted.is_empty() || kind == wanted || kind.strip_prefix(wanted).is_some_and(|rest| rest.starts_with('.'))
}

/// The catalogue entries an event of this topic (and, for notifications,
/// kind) feeds.
fn types_for(topic: &str, notification_kind: Option<&str>) -> Vec<&'static EventType> {
    EVENT_TYPES
        .iter()
        .filter(|event_type| match event_type.source {
            Source::Topic { topic: source, .. } => source == topic,
            Source::Notification { kind } => {
                topic == NOTIFY_TOPIC && notification_kind.is_some_and(|actual| kind_matches(kind, actual))
            }
        })
        .collect()
}

/// The user IDs in the named fields of a payload (each a UUID or a list of them).
fn accounts_in(data: &serde_json::Value, fields: &[&str]) -> Vec<Uuid> {
    let mut accounts = Vec::new();
    let mut add = |value: &serde_json::Value| {
        if let Some(id) = value.as_str().and_then(|text| Uuid::parse_str(text).ok()) {
            if !accounts.contains(&id) {
                accounts.push(id);
            }
        }
    };
    for field in fields {
        match data.get(field) {
            Some(serde_json::Value::Array(values)) => values.iter().for_each(&mut add),
            Some(value) => add(value),
            None => {}
        }
    }
    accounts
}

/// What a webhook is told about a notification.
fn notification_data(request: &NotificationRequest) -> serde_json::Value {
    serde_json::json!({
        "kind": request.kind,
        "actor_id": request.actor_id,
        "title": { "ja": request.title.ja, "en": request.title.en },
        "body": { "ja": request.body.ja, "en": request.body.en },
        "path": request.path,
        "image_url": request.image_url,
        "subject": request.subject.map(|(subject_type, id)| serde_json::json!({ "type": subject_type, "id": id })),
        "data": request.data,
    })
}

impl Dev {
    /// Whether one more delivery may go to the host of `url` on behalf of
    /// `owner` this minute. All webhooks of one person — their bots'
    /// included — share the budget per host, so that subscribing many of
    /// them to a busy event cannot turn Tokuly into a flood aimed at
    /// somebody's server. Events over the budget are dropped, not queued.
    async fn within_delivery_budget(&self, owner: Uuid, url: &str) -> AppResult<bool> {
        if self.cfg.webhook_rate_per_minute == 0 {
            return Ok(true);
        }
        let host =
            url::Url::parse(url).ok().and_then(|url| url.host_str().map(str::to_ascii_lowercase)).unwrap_or_default();
        match self
            .core
            .cache
            .rate_limit(
                &format!("dev-webhook-out:{owner}:{host}"),
                self.cfg.webhook_rate_per_minute,
                Duration::from_secs(60),
            )
            .await
        {
            Ok(()) => Ok(true),
            Err(e) if e.code == connectrpc::ErrorCode::ResourceExhausted => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// The event types somebody subscribes to, from memory when recent.
    async fn subscribed_types(&self) -> AppResult<Arc<HashSet<String>>> {
        if let Some(types) = self.subscribed.fresh() {
            return Ok(types);
        }
        let types: Vec<String> =
            sqlx::query_scalar("SELECT DISTINCT unnest(event_types) FROM dev_webhooks WHERE status = 'active'")
                .fetch_all(&self.core.db)
                .await?;
        let types = Arc::new(types.into_iter().collect::<HashSet<String>>());
        *self.subscribed.cached.write() = Some((Instant::now(), types.clone()));
        Ok(types)
    }

    /// Handles one domain event: stores a delivery for every webhook that
    /// wants it, and returns how many. Idempotent: an event delivered twice
    /// by the bus is stored once per webhook.
    pub async fn fan_out(&self, event: &Event) -> AppResult<usize> {
        let kind =
            (event.topic == NOTIFY_TOPIC).then(|| event.data.get("kind").and_then(|kind| kind.as_str())).flatten();
        let candidates = types_for(&event.topic, kind);
        if candidates.is_empty() {
            return Ok(0);
        }
        let subscribed = self.subscribed_types().await?;
        let mut stored = 0;
        for event_type in candidates.into_iter().filter(|event_type| subscribed.contains(event_type.name)) {
            let (accounts, data) = match event_type.source {
                Source::Topic { accounts, .. } => (accounts_in(&event.data, accounts), event.data.clone()),
                Source::Notification { .. } => match event.data::<NotificationRequest>() {
                    Ok(request) => (vec![request.user_id], notification_data(&request)),
                    // Not ours to fix; the notify domain reports malformed requests.
                    Err(_) => continue,
                },
            };
            if accounts.is_empty() {
                continue;
            }
            // Suspended and deleted accounts, and bots that are switched off
            // or whose owner is suspended, stay silent; so do accounts under
            // the minimum age and their bots.
            let webhooks: Vec<(Uuid, String, Uuid, Option<chrono::NaiveDate>, String)> = sqlx::query_as(
                "SELECT w.id, w.url, COALESCE(b.owner_id, w.account_id), COALESCE(o.birthdate, u.birthdate), COALESCE(o.time_zone, u.time_zone)
                 FROM dev_webhooks w
                 JOIN users u ON u.id = w.account_id
                 LEFT JOIN dev_bots b ON b.user_id = w.account_id
                 LEFT JOIN users o ON o.id = b.owner_id
                 WHERE w.account_id = ANY($1) AND w.status = 'active' AND w.event_types @> ARRAY[$2]
                   AND u.banned_at IS NULL AND u.deleted_at IS NULL AND b.disabled_at IS NULL AND o.banned_at IS NULL",
            )
            .bind(&accounts)
            .bind(event_type.name)
            .fetch_all(&self.core.db)
            .await?;
            let payload = envelope(event.id, event_type.name, event.occurred_at, data);
            for (webhook_id, url, owner, birthdate, time_zone) in webhooks {
                if self.identity.too_young(birthdate, &time_zone) {
                    continue;
                }
                if !self.within_delivery_budget(owner, &url).await? {
                    tracing::debug!(%webhook_id, "webhook delivery dropped: too many events for this host in a minute");
                    continue;
                }
                if self.queue_delivery(webhook_id, event.id, event_type.name, &payload, None).await?.is_some() {
                    stored += 1;
                }
            }
        }
        Ok(stored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_map_to_catalogue_entries() {
        let names =
            |topic: &str, kind: Option<&str>| types_for(topic, kind).into_iter().map(|t| t.name).collect::<Vec<_>>();
        assert_eq!(names("social.friend.requested", None), ["friend.request_received"]);
        assert_eq!(names("social.friend.accepted", None), ["friend.added"]);
        // A notification is always `notification.created`, and whatever its kind adds.
        assert_eq!(names(NOTIFY_TOPIC, Some("chat.message")), ["notification.created", "chat.message_received"]);
        assert_eq!(names(NOTIFY_TOPIC, Some("geo.place_alert")), ["notification.created", "place.alert"]);
        assert_eq!(names(NOTIFY_TOPIC, Some("video.published")), ["notification.created", "channel.video_published"]);
        assert_eq!(names(NOTIFY_TOPIC, Some("social.buzz")), ["notification.created"]);
        assert!(names(NOTIFY_TOPIC, None).is_empty());
        assert!(names("chat.message.deleted", None).is_empty() && names("identity.user.deleted", None).is_empty());
    }

    #[test]
    fn notification_kinds_match_whole_segments() {
        assert!(kind_matches("geo.place_alert", "geo.place_alert"));
        assert!(kind_matches("chat", "chat.message") && kind_matches("", "anything.at_all"));
        // `geo.place_alert_response` is another kind, not a sub-kind.
        assert!(!kind_matches("geo.place_alert", "geo.place_alert_response"));
        assert!(!kind_matches("chat.message", "chat") && !kind_matches("chat.message", "chat.mention"));
    }

    #[test]
    fn recipients_come_from_the_named_payload_fields() {
        let (a, b, c) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let data = serde_json::json!({ "user_id": a, "friend_id": b, "user_ids": [a, c, "junk"], "other": c, "nothing": null });
        assert_eq!(accounts_in(&data, &["user_id", "friend_id"]), [a, b]);
        assert_eq!(accounts_in(&data, &["user_ids", "user_id"]), [a, c], "lists are read and duplicates dropped");
        assert!(accounts_in(&data, &["missing", "nothing"]).is_empty());
    }

    #[test]
    fn every_source_topic_is_consumed_once() {
        let topics = source_topics();
        assert!(topics.contains(&NOTIFY_TOPIC) && topics.contains(&"social.friend.requested"));
        let mut unique = topics.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), topics.len());
    }

    #[test]
    fn a_webhook_sees_the_notification_in_both_languages() {
        use tokuly_core::outbound::Localized;
        let request = NotificationRequest::new(
            Uuid::now_v7(),
            "chat.message",
            Localized::new("太郎", "Taro"),
            Localized::new("やあ", "Hi"),
            "/chat/1",
        )
        .actor(Uuid::now_v7());
        let data = notification_data(&request);
        assert_eq!(data["kind"], "chat.message");
        assert_eq!((data["title"]["ja"].as_str(), data["body"]["en"].as_str()), (Some("太郎"), Some("Hi")));
        assert!(data.get("user_id").is_none() && data["subject"].is_null());
    }
}
