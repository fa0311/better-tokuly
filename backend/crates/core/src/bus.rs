//! NATS: the message bus between services.
//!
//! - Core NATS subjects (`rt.>`, `sys.>`) carry ephemeral fan-out traffic such
//!   as realtime frames and cache invalidations.
//! - JetStream streams carry durable work: `EVENTS` (domain events from the
//!   outbox), `JOBS` (background jobs), `ANALYTICS` (product analytics rows on
//!   their way to ClickHouse) and `LOCATIONS` (location samples for the geo
//!   pipeline).
use std::time::Duration;

use async_nats::jetstream::{
    self,
    consumer::{self, pull},
    stream::{self, RetentionPolicy, StorageType},
};
pub use async_nats::connection::State as ConnectionState;
use bytes::Bytes;

use crate::error::{AppError, AppResult};

pub const STREAM_EVENTS: &str = "EVENTS";
pub const STREAM_JOBS: &str = "JOBS";
pub const STREAM_ANALYTICS: &str = "ANALYTICS";
pub const STREAM_LOCATIONS: &str = "LOCATIONS";

#[derive(Clone)]
pub struct Bus {
    /// The raw client. Prefer the methods below: they apply the namespace.
    pub client: async_nats::Client,
    pub js: jetstream::Context,
    /// Namespace for subjects and streams (`TOKULY_NATS_PREFIX`), so several
    /// deployments or test runs can share one NATS server. Empty by default.
    prefix: String,
}

impl Bus {
    pub async fn connect(url: &str, prefix: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(
            prefix.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "TOKULY_NATS_PREFIX may only contain letters, digits and underscores"
        );
        let client = async_nats::ConnectOptions::new()
            .name("tokuly")
            .retry_on_initial_connect()
            .connection_timeout(Duration::from_secs(5))
            .connect(url)
            .await
            .map_err(|e| anyhow::anyhow!("cannot connect to NATS at {url}: {e}"))?;
        let js = jetstream::new(client.clone());
        Ok(Self {
            client,
            js,
            prefix: prefix.to_owned(),
        })
    }

    /// The namespaced form of a subject.
    pub fn subject(&self, subject: &str) -> String {
        if self.prefix.is_empty() {
            subject.to_owned()
        } else {
            format!("{}.{subject}", self.prefix)
        }
    }

    /// The namespaced form of a stream name.
    pub fn stream_name(&self, name: &str) -> String {
        if self.prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{}_{name}", self.prefix)
        }
    }

    /// Creates the JetStream streams if they do not exist yet. Idempotent.
    pub async fn ensure_streams(&self) -> anyhow::Result<()> {
        let day = Duration::from_secs(24 * 3600);
        let specs = [
            (
                STREAM_EVENTS,
                vec!["evt.>"],
                RetentionPolicy::Limits,
                7 * day,
            ),
            (
                STREAM_JOBS,
                vec!["job.>"],
                RetentionPolicy::WorkQueue,
                14 * day,
            ),
            (
                STREAM_ANALYTICS,
                vec!["an.>"],
                RetentionPolicy::Limits,
                3 * day,
            ),
            (
                STREAM_LOCATIONS,
                vec!["loc.>"],
                RetentionPolicy::Limits,
                day,
            ),
        ];
        for (name, subjects, retention, max_age) in specs {
            // Create-or-update, so a release that changes a stream's settings
            // takes effect on deployments that already have the stream.
            self.js
                .create_or_update_stream(stream::Config {
                    name: self.stream_name(name),
                    subjects: subjects.into_iter().map(|s| self.subject(s)).collect(),
                    retention,
                    max_age,
                    storage: StorageType::File,
                    duplicate_window: Duration::from_secs(120),
                    ..Default::default()
                })
                .await
                .map_err(|e| anyhow::anyhow!("cannot create JetStream stream {name}: {e}"))?;
        }
        Ok(())
    }

    /// Fire-and-forget publish on core NATS.
    pub async fn publish(
        &self,
        subject: impl Into<String>,
        payload: impl Into<Bytes>,
    ) -> AppResult<()> {
        self.client
            .publish(self.subject(&subject.into()), payload.into())
            .await
            .map_err(|e| {
                AppError::unavailable("BUS_UNAVAILABLE", format!("bus publish failed: {e}"))
            })
    }

    /// Core NATS subscription (every subscriber receives every message).
    pub async fn subscribe(&self, subject: impl Into<String>) -> AppResult<async_nats::Subscriber> {
        self.client.subscribe(self.subject(&subject.into())).await.map_err(|e| {
            AppError::unavailable("BUS_UNAVAILABLE", format!("bus subscribe failed: {e}"))
        })
    }

    /// Queue subscription: one member of `group` receives each message.
    pub async fn queue_subscribe(
        &self,
        subject: impl Into<String>,
        group: impl Into<String>,
    ) -> AppResult<async_nats::Subscriber> {
        self.client
            .queue_subscribe(self.subject(&subject.into()), group.into())
            .await
            .map_err(|e| {
                AppError::unavailable("BUS_UNAVAILABLE", format!("bus subscribe failed: {e}"))
            })
    }

    /// Durable publish: resolves once JetStream has stored the message.
    /// `dedupe_id` makes retries of the same logical message idempotent.
    pub async fn publish_durable(
        &self,
        subject: impl Into<String>,
        payload: impl Into<Bytes>,
        dedupe_id: Option<&str>,
    ) -> AppResult<()> {
        let subject = self.subject(&subject.into());
        let unavailable = |e: String| {
            AppError::unavailable("BUS_UNAVAILABLE", format!("durable publish failed: {e}"))
        };
        let ack = match dedupe_id {
            Some(id) => {
                let mut headers = async_nats::HeaderMap::new();
                headers.insert("Nats-Msg-Id", id);
                self.js
                    .publish_with_headers(subject, headers, payload.into())
                    .await
            }
            None => self.js.publish(subject, payload.into()).await,
        }
        .map_err(|e| unavailable(e.to_string()))?;
        ack.await.map_err(|e| unavailable(e.to_string()))?;
        Ok(())
    }

    /// A durable pull consumer on `stream_name`. It is created if missing and
    /// brought up to date if it exists with other settings: a consumer keeps
    /// its position, so a release that subscribes it to more topics starts
    /// receiving them without losing anything.
    pub async fn durable_consumer(
        &self,
        stream_name: &str,
        durable: &str,
        filter_subjects: Vec<String>,
        ack_wait: Duration,
        max_deliver: i64,
    ) -> AppResult<consumer::Consumer<pull::Config>> {
        let unavailable = |e: String| {
            AppError::unavailable("BUS_UNAVAILABLE", format!("consumer setup failed: {e}"))
        };
        let stream = self
            .js
            .get_stream(self.stream_name(stream_name))
            .await
            .map_err(|e| unavailable(e.to_string()))?;
        stream
            .create_consumer(pull::Config {
                durable_name: Some(durable.to_owned()),
                filter_subjects: filter_subjects.iter().map(|s| self.subject(s)).collect(),
                ack_wait,
                max_deliver,
                ..Default::default()
            })
            .await
            .map_err(|e| unavailable(e.to_string()))
    }
}
