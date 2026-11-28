//! How far behind the event platform is: the age of the oldest outbox row,
//! what each durable consumer has still to handle, and how many messages
//! are buried (`DEAD`). The worker reports it as metrics ([`report`]); the
//! staff console shows it next to the dead letters.
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::TryStreamExt as _;
use opentelemetry::KeyValue;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use crate::{
    bus::{Bus, STREAM_ANALYTICS, STREAM_DEAD, STREAM_EVENTS, STREAM_JOBS, STREAM_LOCATIONS},
    error::{AppError, AppResult},
};

/// The streams whose consumers are measured.
const CONSUMED: [&str; 4] = [STREAM_EVENTS, STREAM_JOBS, STREAM_ANALYTICS, STREAM_LOCATIONS];

/// How often the worker measures.
const REPORT_EVERY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Backlog {
    /// How long the oldest row has waited in the outbox; `None` when it is empty.
    pub oldest_outbox_age: Option<Duration>,
    pub consumers: Vec<ConsumerBacklog>,
    /// Messages on the `DEAD` stream.
    pub dead_letters: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumerBacklog {
    pub stream: &'static str,
    pub consumer: String,
    /// Messages it has not been given yet.
    pub pending: u64,
    /// Messages it was given and has not acknowledged.
    pub unacknowledged: u64,
}

/// Measures the backlog now.
pub async fn measure(db: &PgPool, bus: &Bus) -> AppResult<Backlog> {
    let unavailable = |e: String| AppError::unavailable("BUS_UNAVAILABLE", format!("backlog cannot be read: {e}"));
    // The oldest row by the primary key (UUIDv7, in insertion order): no scan.
    let oldest: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT occurred_at FROM outbox ORDER BY id LIMIT 1").fetch_optional(db).await?;
    let mut consumers = Vec::new();
    for stream in CONSUMED {
        let handle = bus.js.get_stream(bus.stream_name(stream)).await.map_err(|e| unavailable(e.to_string()))?;
        let infos: Vec<_> = handle.consumers().try_collect().await.map_err(|e| unavailable(e.to_string()))?;
        consumers.extend(infos.into_iter().map(|info| ConsumerBacklog {
            stream,
            consumer: info.name,
            pending: info.num_pending,
            unacknowledged: info.num_ack_pending as u64,
        }));
    }
    consumers.sort_by(|a, b| (a.stream, &a.consumer).cmp(&(b.stream, &b.consumer)));
    let mut dead = bus.js.get_stream(bus.stream_name(STREAM_DEAD)).await.map_err(|e| unavailable(e.to_string()))?;
    let dead_letters = dead.info().await.map_err(|e| unavailable(e.to_string()))?.state.messages;
    Ok(Backlog {
        oldest_outbox_age: oldest.map(|at| (Utc::now() - at).to_std().unwrap_or_default()),
        consumers,
        dead_letters,
    })
}

/// Measures the backlog every half minute until cancelled and records it as
/// metrics (exported with the traces, `TOKULY_OTLP_ENDPOINT`):
///
/// - `tokuly.outbox.oldest_age` (seconds; 0 when the outbox is empty)
/// - `tokuly.consumer.pending` and `tokuly.consumer.unacknowledged`
///   (attributes `stream`, `consumer`)
/// - `tokuly.dead_letters`
pub async fn report(db: PgPool, bus: Bus, cancel: CancellationToken) {
    let meter = opentelemetry::global::meter("tokuly");
    let oldest = meter.f64_gauge("tokuly.outbox.oldest_age").with_unit("s").with_description("Age of the oldest outbox row").build();
    let pending = meter.u64_gauge("tokuly.consumer.pending").with_description("Messages a consumer has not been given yet").build();
    let unacknowledged =
        meter.u64_gauge("tokuly.consumer.unacknowledged").with_description("Messages a consumer was given and has not acknowledged").build();
    let dead = meter.u64_gauge("tokuly.dead_letters").with_description("Messages on the DEAD stream").build();
    let mut every = tokio::time::interval(REPORT_EVERY);
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = every.tick() => {}
        }
        let backlog = match measure(&db, &bus).await {
            Ok(backlog) => backlog,
            Err(e) => {
                tracing::warn!(error = %e, "backlog not measured");
                continue;
            }
        };
        oldest.record(backlog.oldest_outbox_age.unwrap_or_default().as_secs_f64(), &[]);
        for consumer in &backlog.consumers {
            let attributes = [KeyValue::new("stream", consumer.stream), KeyValue::new("consumer", consumer.consumer.clone())];
            pending.record(consumer.pending, &attributes);
            unacknowledged.record(consumer.unacknowledged, &attributes);
        }
        dead.record(backlog.dead_letters, &[]);
    }
}
