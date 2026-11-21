//! Contracts between the realtime gateway and the domains.
//!
//! The gateway knows nothing about friends, conversations or ghost mode. It
//! records who is connected and forwards what clients say onto the bus; the
//! owning domain reacts:
//!
//! | Subject (core NATS unless noted) | Payload | Consumer |
//! |---|---|---|
//! | `sys.presence` | [`PresenceChange`] | social → `PresenceEvent` to friends |
//! | `loc.sample.<user>` (JetStream `LOCATIONS`) | [`LocationReport`] | geo pipeline |
//! | `sys.typing` | [`TypingSignal`] | chat → typing indicators |
//! | `sys.conversation_presence` | [`ConversationPresenceSignal`] | chat → co-presence |
//!
//! Online state itself lives in Valkey and is readable by anyone through
//! [`is_online`] / [`online_among`]; topic audience sizes through
//! [`topic_audience`].
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokuly_proto::geo::LocationSample;
use uuid::Uuid;

use crate::{bus::Bus, cache::Cache, error::AppResult};

pub const PRESENCE_SUBJECT: &str = "sys.presence";
pub const TYPING_SUBJECT: &str = "sys.typing";
pub const CONVERSATION_PRESENCE_SUBJECT: &str = "sys.conversation_presence";
/// Prefix of the durable location subjects; the full subject ends with the user ID.
pub const LOCATION_SUBJECT_PREFIX: &str = "loc.sample";

/// How long an "online" marker survives without a heartbeat from the gateway.
pub const ONLINE_TTL_SECONDS: u64 = 75;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceChange {
    pub user_id: Uuid,
    /// At least one connection has the app in the foreground.
    pub online: bool,
    pub at: DateTime<Utc>,
}

/// A location sample received from a device, on its way to the geo pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocationReport {
    pub user_id: Uuid,
    pub received_at: DateTime<Utc>,
    pub sample: LocationSample,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypingSignal {
    pub user_id: Uuid,
    pub conversation_id: Uuid,
    pub typing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationPresenceSignal {
    pub user_id: Uuid,
    pub conversation_id: Uuid,
    pub present: bool,
}

fn online_key(user_id: Uuid) -> String {
    format!("rt:online:{}", user_id.simple())
}

fn last_seen_key(user_id: Uuid) -> String {
    format!("rt:seen:{}", user_id.simple())
}

fn topic_key(topic: &str) -> String {
    format!("rt:topic:{topic}")
}

/// Marks a user online (or refreshes the marker). Called by the gateway on
/// connect, on foreground transitions and on every heartbeat.
pub async fn mark_online(cache: &Cache, user_id: Uuid) -> AppResult<()> {
    cache.set_string(&online_key(user_id), "1", Some(std::time::Duration::from_secs(ONLINE_TTL_SECONDS))).await?;
    cache.set_string(&last_seen_key(user_id), &Utc::now().timestamp().to_string(), Some(std::time::Duration::from_secs(90 * 24 * 3600))).await
}

/// Marks a user offline (last foreground connection went away).
pub async fn mark_offline(cache: &Cache, user_id: Uuid) -> AppResult<()> {
    cache.del(&online_key(user_id)).await?;
    cache.set_string(&last_seen_key(user_id), &Utc::now().timestamp().to_string(), Some(std::time::Duration::from_secs(90 * 24 * 3600))).await
}

/// Whether the user has the app open right now.
pub async fn is_online(cache: &Cache, user_id: Uuid) -> AppResult<bool> {
    cache.exists(&online_key(user_id)).await
}

/// The subset of `user_ids` that is online right now.
pub async fn online_among(cache: &Cache, user_ids: &[Uuid]) -> AppResult<Vec<Uuid>> {
    if user_ids.is_empty() {
        return Ok(Vec::new());
    }
    let keys: Vec<String> = user_ids.iter().map(|id| online_key(*id)).collect();
    let values: Vec<Option<String>> = redis::cmd("MGET").arg(&keys).query_async(&mut cache.conn()).await?;
    Ok(user_ids.iter().zip(values).filter(|(_, v)| v.is_some()).map(|(id, _)| *id).collect())
}

/// When the user was last seen online (now, if they are online).
pub async fn last_seen(cache: &Cache, user_id: Uuid) -> AppResult<Option<DateTime<Utc>>> {
    Ok(cache.get_string(&last_seen_key(user_id)).await?.and_then(|s| s.parse::<i64>().ok()).and_then(|s| DateTime::from_timestamp(s, 0)))
}

/// Publishes a presence transition for the social domain to fan out.
pub async fn announce(bus: &Bus, change: &PresenceChange) -> AppResult<()> {
    bus.publish(PRESENCE_SUBJECT, serde_json::to_vec(change).unwrap_or_default()).await
}

/// Hands a location sample to the geo pipeline (durable).
pub async fn submit_location(bus: &Bus, report: &LocationReport) -> AppResult<()> {
    let subject = format!("{LOCATION_SUBJECT_PREFIX}.{}", report.user_id.simple());
    bus.publish(subject, serde_json::to_vec(report).unwrap_or_default()).await
}

/// How long one gateway's audience report stays valid without a refresh.
/// A gateway that died stops refreshing, so its viewers age out by themselves.
pub const AUDIENCE_TTL_SECONDS: i64 = 90;

/// Reports how many connections on one gateway instance are subscribed to a
/// topic. Gateways refresh their reports periodically; a count of zero (or
/// below) withdraws the report.
pub async fn set_topic_audience(cache: &Cache, topic: &str, instance: &str, count: i64) -> AppResult<()> {
    let key = topic_key(topic);
    let mut conn = cache.conn();
    if count <= 0 {
        redis::cmd("HDEL").arg(&key).arg(instance).query_async::<()>(&mut conn).await?;
    } else {
        // Each report carries its own timestamp so readers can ignore stale instances.
        let value = format!("{count}:{}", Utc::now().timestamp());
        redis::pipe()
            .cmd("HSET").arg(&key).arg(instance).arg(value).ignore()
            .cmd("EXPIRE").arg(&key).arg(AUDIENCE_TTL_SECONDS * 2).ignore()
            .query_async::<()>(&mut conn)
            .await?;
    }
    Ok(())
}

/// How many connections are subscribed to a topic across all live gateways
/// (e.g. `live:<stream_id>` → concurrent viewers). Reports from gateways that
/// stopped refreshing are ignored and cleaned up.
pub async fn topic_audience(cache: &Cache, topic: &str) -> AppResult<i64> {
    let key = topic_key(topic);
    let mut conn = cache.conn();
    let reports: Vec<(String, String)> = redis::cmd("HGETALL").arg(&key).query_async(&mut conn).await?;
    let now = Utc::now().timestamp();
    let mut total = 0;
    let mut stale = Vec::new();
    for (instance, value) in reports {
        match parse_audience_report(&value) {
            Some((count, reported_at)) if now - reported_at <= AUDIENCE_TTL_SECONDS => total += count,
            _ => stale.push(instance),
        }
    }
    if !stale.is_empty() {
        redis::cmd("HDEL").arg(&key).arg(&stale).query_async::<()>(&mut conn).await?;
    }
    Ok(total)
}

/// Parses a `"<count>:<unix seconds>"` audience report.
fn parse_audience_report(value: &str) -> Option<(i64, i64)> {
    let (count, reported_at) = value.split_once(':')?;
    Some((count.parse().ok()?, reported_at.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audience_reports_parse_or_are_rejected() {
        assert_eq!(parse_audience_report("12:1800000000"), Some((12, 1_800_000_000)));
        assert_eq!(parse_audience_report("12"), None);
        assert_eq!(parse_audience_report("x:1"), None);
        assert_eq!(parse_audience_report(""), None);
    }
}
