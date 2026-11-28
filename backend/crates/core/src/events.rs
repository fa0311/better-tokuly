//! Domain events: the transactional outbox and durable consumers.
//!
//! A domain emits an event inside the same database transaction as the state
//! change ([`emit`]). The outbox relay ([`run_outbox_relay`]) moves rows to the
//! `EVENTS` JetStream stream, and any number of services consume them with
//! [`consume`] — notifications, the activity log, search indexing, analytics.
//! Delivery is at-least-once; consumers must be idempotent (event IDs are
//! stable). An event a consumer keeps failing on is retried for about
//! twenty minutes and then buried ([`crate::dead_letters`]), not dropped.
//!
//! Topics are dotted lowercase names `<domain>.<entity>.<verb>`, e.g.
//! `social.friend.accepted`. The payload is JSON; the producing crate exposes
//! the payload struct in its `events` module. See `docs/events.md`.
use std::{
    future::Future,
    sync::atomic::{AtomicI64, Ordering},
    time::Duration,
};

use async_nats::jetstream::{self, AckKind};
use chrono::{DateTime, Utc};
use futures::StreamExt as _;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sqlx::{PgExecutor, PgPool};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    bus::{Bus, STREAM_EVENTS, StartAt},
    dead_letters,
    error::{AppError, AppResult},
};

const OUTBOX_CHANNEL: &str = "tokuly_outbox";

/// When this process's relay last got through a round (Unix milliseconds;
/// 0 before the first): a round that finds nothing to move counts, one
/// that fails does not.
static RELAY_ROUND_AT: AtomicI64 = AtomicI64::new(0);

/// Whether the outbox relay of this process got through a round within
/// `within` (the worker's health check). An idle relay goes round every
/// two seconds.
pub fn relay_running(within: Duration) -> bool {
    let at = RELAY_ROUND_AT.load(Ordering::Relaxed);
    at > 0 && Utc::now().timestamp_millis() - at <= within.as_millis() as i64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: Uuid,
    pub topic: String,
    /// The user who caused the event, when there is one.
    pub actor_id: Option<Uuid>,
    pub occurred_at: DateTime<Utc>,
    pub data: serde_json::Value,
}

impl Event {
    /// Deserializes the payload into the consumer's view of it.
    pub fn data<T: DeserializeOwned>(&self) -> AppResult<T> {
        serde_json::from_value(self.data.clone()).map_err(|e| {
            AppError::internal(anyhow::anyhow!(
                "event {} payload mismatch: {e}",
                self.topic
            ))
        })
    }
}

/// Records an event in the outbox. Pass the open transaction so the event is
/// published if and only if the surrounding change commits.
pub async fn emit<'e, E, T>(
    executor: E,
    topic: &str,
    actor_id: Option<Uuid>,
    data: &T,
) -> AppResult<()>
where
    E: PgExecutor<'e>,
    T: Serialize + ?Sized,
{
    debug_assert!(
        topic.split('.').count() >= 2
            && topic
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_'),
        "event topics look like `domain.entity.verb`: {topic}"
    );
    let data = serde_json::to_value(data)
        .map_err(|e| AppError::internal(anyhow::anyhow!("event serialize: {e}")))?;
    sqlx::query("INSERT INTO outbox (topic, actor_id, data) VALUES ($1, $2, $3)")
        .bind(topic)
        .bind(actor_id)
        .bind(data)
        .execute(executor)
        .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct OutboxRow {
    id: Uuid,
    topic: String,
    actor_id: Option<Uuid>,
    data: serde_json::Value,
    occurred_at: DateTime<Utc>,
}

/// Moves outbox rows to JetStream until cancelled. Several instances may run
/// concurrently (`SKIP LOCKED`); each event is published at least once.
pub async fn run_outbox_relay(db: PgPool, bus: Bus, cancel: CancellationToken) {
    let mut listener = match sqlx::postgres::PgListener::connect_with(&db).await {
        Ok(mut l) => match l.listen(OUTBOX_CHANNEL).await {
            Ok(()) => Some(l),
            Err(e) => {
                tracing::warn!(error = %e, "outbox LISTEN failed; falling back to polling");
                None
            }
        },
        Err(e) => {
            tracing::warn!(error = %e, "outbox listener unavailable; falling back to polling");
            None
        }
    };
    tracing::info!("outbox relay started");
    loop {
        let round = relay_batch(&db, &bus).await;
        if round.is_ok() {
            RELAY_ROUND_AT.store(Utc::now().timestamp_millis(), Ordering::Relaxed);
        }
        match round {
            Ok(n) if n >= BATCH => continue, // more waiting
            Ok(_) => {}
            Err(e) => {
                tracing::error!(error = %e, "outbox relay batch failed");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
        let wake = async {
            match listener.as_mut() {
                Some(l) => {
                    // A lost notification connection just degrades to the poll interval.
                    let _ = tokio::time::timeout(Duration::from_secs(2), l.recv()).await;
                }
                None => tokio::time::sleep(Duration::from_millis(250)).await,
            }
        };
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = wake => {}
        }
    }
    tracing::info!("outbox relay stopped");
}

const BATCH: usize = 200;

/// The largest message NATS takes when the server does not say (its default).
const DEFAULT_MAX_PAYLOAD: usize = 1024 * 1024;
/// Room left for the headers that go with a message.
const HEADER_ROOM: usize = 4096;

/// Who besides an owner may still see content, as producers name them in
/// events that withdraw notifications and feed entries (`visible_to`), at
/// most this many: longer lists are narrowed by the producer (see
/// [`bounded_audience`]), so no event approaches the bus's message limit.
pub const MAX_VISIBLE_TO: usize = 5_000;

/// `members` as a `visible_to` list: unchanged up to [`MAX_VISIBLE_TO`]
/// people; otherwise narrowed to the ones in `told` (who may have something
/// about the content in their inbox; `None` when the producer cannot say),
/// and when even that is too long, nobody. Naming fewer people withdraws
/// more than needed, never less. Sorted.
pub fn bounded_audience(members: Vec<Uuid>, told: Option<&[Uuid]>) -> Vec<Uuid> {
    let mut members = members;
    if members.len() > MAX_VISIBLE_TO {
        members = match told {
            Some(told) => {
                let told: std::collections::HashSet<&Uuid> = told.iter().collect();
                members.into_iter().filter(|id| told.contains(id)).collect()
            }
            None => Vec::new(),
        };
        if members.len() > MAX_VISIBLE_TO {
            members = Vec::new();
        }
    }
    members.sort_unstable();
    members.dedup();
    members
}

/// Whether an encoded event is too large for the bus to take: such a row
/// can never be published, and retrying it would hold back every row after
/// it.
fn too_large(payload_len: usize, max_payload: usize) -> bool {
    let max = if max_payload == 0 {
        DEFAULT_MAX_PAYLOAD
    } else {
        max_payload
    };
    payload_len > max.saturating_sub(HEADER_ROOM)
}

async fn relay_batch(db: &PgPool, bus: &Bus) -> AppResult<usize> {
    let mut tx = db.begin().await?;
    let rows: Vec<OutboxRow> = sqlx::query_as(
        "DELETE FROM outbox WHERE id IN (
             SELECT id FROM outbox ORDER BY id LIMIT $1 FOR UPDATE SKIP LOCKED
         ) RETURNING id, topic, actor_id, data, occurred_at",
    )
    .bind(BATCH as i64)
    .fetch_all(&mut *tx)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }
    let mut rows = rows;
    rows.sort_by_key(|r| r.id);
    let count = rows.len();
    let max_payload = bus.client.server_info().max_payload;
    let mut messages = Vec::with_capacity(count);
    for row in rows {
        let event = Event {
            id: row.id,
            topic: row.topic,
            actor_id: row.actor_id,
            occurred_at: row.occurred_at,
            data: row.data,
        };
        let payload = serde_json::to_vec(&event)
            .map_err(|e| AppError::internal(anyhow::anyhow!("event encode: {e}")))?;
        if too_large(payload.len(), max_payload) {
            // Recorded and dropped: it would fail every time and stop the relay.
            tracing::error!(
                event_id = %event.id,
                topic = %event.topic,
                bytes = payload.len(),
                max_payload,
                "dropping an event too large for the bus"
            );
            continue;
        }
        messages.push((format!("evt.{}", event.topic), payload.into(), event.id.to_string()));
    }
    // One round trip for the batch: the rows stay locked only that long. If
    // publishing fails the transaction rolls back and the rows are retried.
    bus.publish_durable_all(messages).await?;
    tx.commit().await?;
    Ok(count)
}

/// Options for a durable event consumer.
#[derive(Debug, Clone)]
pub struct ConsumeOptions {
    /// How long a handler may run before the event is redelivered.
    pub ack_wait: Duration,
    /// Deliveries before the consumer gives up on the event and buries it
    /// ([`crate::dead_letters`]).
    pub max_deliver: i64,
    /// Events handled concurrently.
    pub concurrency: usize,
    /// Where a consumer that does not exist yet starts: by default with the
    /// events published from now on.
    pub start: StartAt,
}

impl Default for ConsumeOptions {
    fn default() -> Self {
        Self {
            ack_wait: Duration::from_secs(30),
            // With `retry_delay`: about twenty minutes of retries, enough
            // for a restart of the database or the search engine.
            max_deliver: 12,
            concurrency: 8,
            start: StartAt::New,
        }
    }
}

/// Consumes events matching `topics` (NATS wildcards allowed, e.g.
/// `"chat.>"`, `"*.*.created"`) under the durable name `durable` until
/// cancelled. Instances sharing a durable name share the work.
pub async fn consume<F, Fut>(
    bus: Bus,
    durable: &str,
    topics: &[&str],
    options: ConsumeOptions,
    cancel: CancellationToken,
    handler: F,
) where
    F: Fn(Event) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = AppResult<()>> + Send + 'static,
{
    let filters: Vec<String> = topics.iter().map(|t| format!("evt.{t}")).collect();
    loop {
        if cancel.is_cancelled() {
            return;
        }
        let consumer = match bus
            .durable_consumer(
                STREAM_EVENTS,
                durable,
                filters.clone(),
                options.ack_wait,
                options.max_deliver,
                options.start,
            )
            .await
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(durable, error = %e, "event consumer setup failed; retrying");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        let mut messages = match consumer.messages().await {
            Ok(m) => m,
            Err(e) => {
                tracing::error!(durable, error = %e, "event consumer stream failed; retrying");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        tracing::info!(durable, ?topics, "event consumer started");
        let limiter = std::sync::Arc::new(tokio::sync::Semaphore::new(options.concurrency.max(1)));
        loop {
            let next = tokio::select! {
                _ = cancel.cancelled() => return,
                next = messages.next() => next,
            };
            let message = match next {
                Some(Ok(m)) => m,
                Some(Err(e)) => {
                    tracing::warn!(durable, error = %e, "event consumer error; reconnecting");
                    break;
                }
                None => break,
            };
            let permit = limiter
                .clone()
                .acquire_owned()
                .await
                .expect("semaphore is never closed");
            let handler = handler.clone();
            let durable = durable.to_owned();
            let max_deliver = options.max_deliver;
            let bus = bus.clone();
            tokio::spawn(async move {
                let _permit = permit;
                handle_message(&bus, message, &durable, max_deliver, handler).await;
            });
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// How long a failed event waits for its next delivery: 2 s, doubling up
/// to 5 minutes.
fn retry_delay(delivered: i64) -> Duration {
    Duration::from_secs((1u64 << delivered.clamp(1, 9)).min(300))
}

async fn handle_message<F, Fut>(bus: &Bus, message: jetstream::Message, durable: &str, max_deliver: i64, handler: F)
where
    F: Fn(Event) -> Fut,
    Fut: Future<Output = AppResult<()>>,
{
    if dead_letters::redriven_for_another(&message, durable) {
        let _ = message.ack().await;
        return;
    }
    let delivered = message.info().map(|i| i.delivered).unwrap_or(1);
    let event: Event = match serde_json::from_slice(&message.payload) {
        Ok(e) => e,
        Err(e) => {
            dead_letters::bury_or_log(bus, durable, &message.subject, &message.payload, &format!("undecodable event: {e}"), delivered).await;
            let _ = message.ack_with(AckKind::Term).await;
            return;
        }
    };
    let topic = event.topic.clone();
    let id = event.id;
    match handler(event).await {
        Ok(()) => {
            if let Err(e) = message.ack().await {
                tracing::warn!(durable, %topic, error = %e, "event ack failed");
            }
        }
        Err(e) if delivered >= max_deliver => {
            dead_letters::bury_or_log(bus, durable, &message.subject, &message.payload, &e.to_string(), delivered).await;
            let _ = message.ack_with(AckKind::Term).await;
        }
        Err(e) => {
            let delay = retry_delay(delivered);
            tracing::warn!(durable, %topic, %id, delivered, retry_in = ?delay, error = %e, "event handler failed; will retry");
            let _ = message.ack_with(AckKind::Nak(Some(delay))).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_relay_runs_while_its_rounds_are_recent() {
        let now = Utc::now().timestamp_millis();
        RELAY_ROUND_AT.store(now - 5_000, Ordering::Relaxed);
        assert!(relay_running(Duration::from_secs(30)));
        RELAY_ROUND_AT.store(now - 60_000, Ordering::Relaxed);
        assert!(!relay_running(Duration::from_secs(30)), "stuck or failing for a minute");
    }

    #[test]
    fn a_failing_event_is_retried_for_about_twenty_minutes() {
        assert_eq!(retry_delay(1), Duration::from_secs(2));
        assert_eq!(retry_delay(8), Duration::from_secs(256));
        assert_eq!(retry_delay(9), Duration::from_secs(300));
        assert_eq!(retry_delay(40), Duration::from_secs(300));
        let retries = 1..ConsumeOptions::default().max_deliver;
        let waited: Duration = retries.map(retry_delay).sum();
        assert!((Duration::from_secs(15 * 60)..Duration::from_secs(30 * 60)).contains(&waited), "{waited:?}");
    }

    #[test]
    fn events_too_large_for_the_bus_are_told_apart() {
        assert!(!too_large(1_000, 0), "an unknown limit is the NATS default");
        assert!(too_large(DEFAULT_MAX_PAYLOAD, 0));
        assert!(!too_large(500_000, 1024 * 1024));
        assert!(too_large(1024 * 1024 - 10, 1024 * 1024), "headers need room too");
        assert!(too_large(9 * 1024 * 1024, 8 * 1024 * 1024));
    }

    #[test]
    fn long_audiences_are_narrowed_to_who_was_told_or_to_nobody() {
        let few: Vec<Uuid> = (0..3).map(|_| Uuid::now_v7()).collect();
        assert_eq!(bounded_audience(few.clone(), None).len(), 3);
        let many: Vec<Uuid> = (0..MAX_VISIBLE_TO + 1).map(|_| Uuid::now_v7()).collect();
        let told = vec![many[7], many[42], Uuid::now_v7()];
        let mut kept = vec![many[7], many[42]];
        kept.sort_unstable();
        assert_eq!(bounded_audience(many.clone(), Some(&told)), kept, "only the told among them");
        assert!(bounded_audience(many.clone(), None).is_empty(), "nobody: withdraws too much, never too little");
        assert!(bounded_audience(many.clone(), Some(&many)).is_empty(), "still too many: nobody");
        // An event naming the most people allowed stays far under the bus limit.
        let longest: Vec<String> = many[..MAX_VISIBLE_TO].iter().map(Uuid::to_string).collect();
        let bytes = serde_json::to_vec(&serde_json::json!({ "visible_to": longest })).unwrap().len();
        assert!(!too_large(bytes, DEFAULT_MAX_PAYLOAD), "{bytes} bytes");
    }
}
