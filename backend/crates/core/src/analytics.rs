//! Product analytics.
//!
//! [`Analytics::track`] puts an event on the `ANALYTICS` stream; the sink
//! ([`Analytics::run_sink`], part of the worker role) batches them into the
//! ClickHouse `events` table (`backend/clickhouse/0001_events.sql`), which
//! creator dashboards and admin statistics query through [`Analytics::ch`].
//! Tracking never blocks or fails a request.
//!
//! ClickHouse schema files live in `backend/clickhouse/*.sql` (one statement
//! per file, idempotent) and are applied by [`apply_schema`].
use std::time::Duration;

use async_nats::jetstream::consumer::pull;
use chrono::{DateTime, Utc};
use futures::StreamExt as _;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    bus::{Bus, STREAM_ANALYTICS},
    config::Config,
    error::{AppError, AppResult},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsEvent {
    pub id: Uuid,
    /// Dotted lowercase name, e.g. `video.view`, `short.impression`.
    pub name: String,
    pub user_id: Option<Uuid>,
    /// What the event is about, e.g. ("video", id). Empty string when none.
    pub subject_type: String,
    pub subject_id: Option<Uuid>,
    /// Owner of the subject (creator/channel owner), for per-creator dashboards.
    pub owner_id: Option<Uuid>,
    pub at: DateTime<Utc>,
    /// Numeric measure when the event has one (watch seconds, coins...).
    pub value: f64,
    /// Free-form details.
    pub props: serde_json::Value,
}

impl AnalyticsEvent {
    pub fn new(name: &str) -> Self {
        Self {
            id: Uuid::now_v7(),
            name: name.to_owned(),
            user_id: None,
            subject_type: String::new(),
            subject_id: None,
            owner_id: None,
            at: Utc::now(),
            value: 0.0,
            props: serde_json::Value::Null,
        }
    }

    pub fn user(mut self, user_id: Option<Uuid>) -> Self {
        self.user_id = user_id;
        self
    }

    pub fn subject(mut self, subject_type: &str, subject_id: Uuid) -> Self {
        self.subject_type = subject_type.to_owned();
        self.subject_id = Some(subject_id);
        self
    }

    pub fn owner(mut self, owner_id: Uuid) -> Self {
        self.owner_id = Some(owner_id);
        self
    }

    pub fn value(mut self, value: f64) -> Self {
        self.value = value;
        self
    }

    pub fn props(mut self, props: serde_json::Value) -> Self {
        self.props = props;
        self
    }
}

/// An `events` table row.
#[derive(Debug, clickhouse::Row, Serialize, Deserialize)]
pub struct EventRow {
    #[serde(with = "clickhouse::serde::uuid")]
    pub id: Uuid,
    pub name: String,
    #[serde(with = "clickhouse::serde::uuid")]
    pub user_id: Uuid,
    pub subject_type: String,
    #[serde(with = "clickhouse::serde::uuid")]
    pub subject_id: Uuid,
    #[serde(with = "clickhouse::serde::uuid")]
    pub owner_id: Uuid,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    pub at: DateTime<Utc>,
    pub value: f64,
    pub props: String,
}

impl From<AnalyticsEvent> for EventRow {
    fn from(event: AnalyticsEvent) -> Self {
        Self {
            id: event.id,
            name: event.name,
            user_id: event.user_id.unwrap_or_default(),
            subject_type: event.subject_type,
            subject_id: event.subject_id.unwrap_or_default(),
            owner_id: event.owner_id.unwrap_or_default(),
            at: event.at,
            value: event.value,
            props: if event.props.is_null() { String::new() } else { event.props.to_string() },
        }
    }
}

const SINK_DURABLE: &str = "analytics-sink";
const SINK_BATCH: usize = 1000;

#[derive(Clone)]
pub struct Analytics {
    bus: Bus,
    /// ClickHouse client for analytical queries.
    pub ch: clickhouse::Client,
}

fn ch_err(context: &str, error: impl std::fmt::Display) -> AppError {
    AppError::unavailable("ANALYTICS_UNAVAILABLE", format!("{context}: {error}"))
}

impl Analytics {
    pub fn new(bus: Bus, cfg: &Config) -> Self {
        let ch = clickhouse::Client::default()
            .with_url(&cfg.clickhouse_url)
            .with_user(&cfg.clickhouse_user)
            .with_password(&cfg.clickhouse_password)
            .with_database(&cfg.clickhouse_database);
        Self { bus, ch }
    }

    /// Records an analytics event (best effort).
    pub async fn track(&self, event: AnalyticsEvent) {
        match serde_json::to_vec(&event) {
            Ok(payload) => {
                if let Err(e) = self.bus.publish(format!("an.{}", event.name), payload).await {
                    tracing::debug!(error = %e, "analytics publish failed");
                }
            }
            Err(e) => tracing::debug!(error = %e, "analytics encode failed"),
        }
    }

    /// Writes events straight into ClickHouse.
    pub async fn write(&self, events: Vec<AnalyticsEvent>) -> AppResult<()> {
        if events.is_empty() {
            return Ok(());
        }
        let mut insert = self.ch.insert::<EventRow>("events").await.map_err(|e| ch_err("analytics insert", e))?;
        for event in events {
            insert.write(&EventRow::from(event)).await.map_err(|e| ch_err("analytics insert", e))?;
        }
        insert.end().await.map_err(|e| ch_err("analytics insert", e))
    }

    /// Moves whatever is waiting on the stream into ClickHouse and returns how
    /// many events were written. The sink loop calls this repeatedly; tests
    /// call it once after tracking to make events queryable.
    pub async fn drain(&self) -> AppResult<usize> {
        let consumer = self
            .bus
            .durable_consumer(STREAM_ANALYTICS, SINK_DURABLE, vec!["an.>".to_owned()], Duration::from_secs(60), 5)
            .await?;
        let mut total = 0;
        loop {
            let mut batch = consumer
                .fetch()
                .max_messages(SINK_BATCH)
                .expires(Duration::from_millis(500))
                .messages()
                .await
                .map_err(|e| ch_err("analytics fetch", e))?;
            let mut messages = Vec::new();
            let mut events = Vec::new();
            while let Some(message) = batch.next().await {
                let Ok(message) = message else { break };
                // Undecodable payloads are acknowledged and skipped.
                if let Ok(event) = serde_json::from_slice::<AnalyticsEvent>(&message.payload) {
                    events.push(event);
                }
                messages.push(message);
            }
            if messages.is_empty() {
                return Ok(total);
            }
            let written = events.len();
            // Acknowledge only after the rows are stored; a failed insert is retried.
            self.write(events).await?;
            for message in &messages {
                let _ = message.ack().await;
            }
            total += written;
            if messages.len() < SINK_BATCH {
                return Ok(total);
            }
        }
    }

    /// Runs the sink until cancelled.
    pub async fn run_sink(&self, cancel: CancellationToken) {
        tracing::info!("analytics sink started");
        loop {
            let pause = match self.drain().await {
                Ok(0) => Duration::from_secs(2),
                Ok(_) => Duration::from_millis(200),
                Err(e) => {
                    tracing::warn!(error = %e, "analytics sink failed; retrying");
                    Duration::from_secs(5)
                }
            };
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(pause) => {}
            }
        }
    }
}

// The pull consumer type is named here so `drain` stays readable.
#[allow(dead_code)]
type SinkConsumer = async_nats::jetstream::consumer::Consumer<pull::Config>;

/// Applies ClickHouse schema files (`*.sql`, one idempotent statement each,
/// in file-name order) to the configured database, creating it if needed.
pub async fn apply_schema(cfg: &Config, statements: &[(&str, &str)]) -> anyhow::Result<()> {
    let server = clickhouse::Client::default()
        .with_url(&cfg.clickhouse_url)
        .with_user(&cfg.clickhouse_user)
        .with_password(&cfg.clickhouse_password);
    server
        .query(&format!("CREATE DATABASE IF NOT EXISTS `{}`", cfg.clickhouse_database.replace('`', "")))
        .execute()
        .await
        .map_err(|e| anyhow::anyhow!("cannot create ClickHouse database: {e}"))?;
    let database = server.with_database(&cfg.clickhouse_database);
    for (name, sql) in statements {
        database.query(sql).execute().await.map_err(|e| anyhow::anyhow!("ClickHouse schema {name} failed: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_use_zero_uuid_for_absent_ids_and_empty_props() {
        let subject = Uuid::now_v7();
        let row = EventRow::from(AnalyticsEvent::new("video.view").subject("video", subject).value(12.5));
        assert_eq!(row.user_id, Uuid::nil());
        assert_eq!(row.owner_id, Uuid::nil());
        assert_eq!(row.subject_id, subject);
        assert_eq!((row.value, row.props.as_str()), (12.5, ""));
        let row = EventRow::from(AnalyticsEvent::new("x.y").props(serde_json::json!({ "a": 1 })));
        assert_eq!(row.props, r#"{"a":1}"#);
    }
}
