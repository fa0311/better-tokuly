//! Background jobs on a JetStream work queue.
//!
//! `enqueue` stores a JSON payload under `job.<name>`; workers `run` a handler
//! per job name. A running job keeps its lease alive with progress acks, so
//! long transcodes are not redelivered while healthy, and failed jobs retry
//! with exponential backoff up to `max_deliver` attempts. A job that failed
//! its last attempt is buried ([`crate::dead_letters`]), not dropped.
use std::{future::Future, sync::Arc, time::Duration};

use async_nats::jetstream::{self, AckKind};
use futures::StreamExt as _;
use serde::{Serialize, de::DeserializeOwned};
use tokio_util::sync::CancellationToken;

use crate::{
    bus::{Bus, STREAM_JOBS, StartAt},
    dead_letters,
    error::{AppError, AppResult},
};

#[derive(Clone)]
pub struct Jobs {
    bus: Bus,
}

#[derive(Debug, Clone)]
pub struct JobOptions {
    /// Jobs of this kind processed in parallel by one worker process.
    pub concurrency: usize,
    /// Lease duration; renewed automatically while the handler runs.
    pub lease: Duration,
    /// Attempts before the job is given up on and buried.
    pub max_deliver: i64,
}

impl Default for JobOptions {
    fn default() -> Self {
        Self {
            concurrency: 4,
            lease: Duration::from_secs(60),
            max_deliver: 5,
        }
    }
}

impl Jobs {
    pub fn new(bus: Bus) -> Self {
        Self { bus }
    }

    /// Queues a job. `name` is a dotted lowercase identifier, e.g. `media.transcode`.
    pub async fn enqueue<T: Serialize + ?Sized>(&self, name: &str, payload: &T) -> AppResult<()> {
        self.enqueue_inner(name, payload, None).await
    }

    /// Queues a job at most once per `dedupe_id` within the dedupe window
    /// (2 minutes) — safe to call from retried requests.
    pub async fn enqueue_unique<T: Serialize + ?Sized>(
        &self,
        name: &str,
        dedupe_id: &str,
        payload: &T,
    ) -> AppResult<()> {
        self.enqueue_inner(name, payload, Some(dedupe_id)).await
    }

    async fn enqueue_inner<T: Serialize + ?Sized>(
        &self,
        name: &str,
        payload: &T,
        dedupe_id: Option<&str>,
    ) -> AppResult<()> {
        let body = serde_json::to_vec(payload)
            .map_err(|e| AppError::internal(anyhow::anyhow!("job encode: {e}")))?;
        let dedupe = dedupe_id.map(|id| format!("{name}:{id}"));
        self.bus
            .publish_durable(format!("job.{name}"), body, dedupe.as_deref())
            .await
    }

    /// Processes jobs named `name` until cancelled.
    pub async fn run<T, F, Fut>(
        &self,
        name: &str,
        options: JobOptions,
        cancel: CancellationToken,
        handler: F,
    ) where
        T: DeserializeOwned + Send + 'static,
        F: Fn(T) -> Fut + Send + Sync + Clone + 'static,
        Fut: Future<Output = AppResult<()>> + Send + 'static,
    {
        let durable = format!("job-{}", name.replace('.', "-"));
        let subject = format!("job.{name}");
        loop {
            if cancel.is_cancelled() {
                return;
            }
            let consumer = match self
                .bus
                .durable_consumer(
                    STREAM_JOBS,
                    &durable,
                    vec![subject.clone()],
                    options.lease,
                    options.max_deliver,
                    // A queue: jobs enqueued before the first worker are work too.
                    StartAt::All,
                )
                .await
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(job = name, error = %e, "job consumer setup failed; retrying");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };
            let mut messages = match consumer.messages().await {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!(job = name, error = %e, "job stream failed; retrying");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };
            tracing::info!(
                job = name,
                concurrency = options.concurrency,
                "job worker started"
            );
            let limiter = Arc::new(tokio::sync::Semaphore::new(options.concurrency.max(1)));
            loop {
                // Take a slot first so we never hold more leases than we can work on.
                let permit = tokio::select! {
                    _ = cancel.cancelled() => return,
                    permit = limiter.clone().acquire_owned() => permit.expect("semaphore is never closed"),
                };
                let next = tokio::select! {
                    _ = cancel.cancelled() => return,
                    next = messages.next() => next,
                };
                let message = match next {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => {
                        tracing::warn!(job = name, error = %e, "job stream error; reconnecting");
                        break;
                    }
                    None => break,
                };
                let handler = handler.clone();
                let name = name.to_owned();
                let options = options.clone();
                let (bus, durable) = (self.bus.clone(), durable.clone());
                tokio::spawn(async move {
                    let _permit = permit;
                    run_one(&bus, &durable, message, &name, &options, handler).await;
                });
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

async fn run_one<T, F, Fut>(
    bus: &Bus,
    durable: &str,
    message: jetstream::Message,
    name: &str,
    options: &JobOptions,
    handler: F,
) where
    T: DeserializeOwned,
    F: Fn(T) -> Fut,
    Fut: Future<Output = AppResult<()>>,
{
    let delivered = message.info().map(|i| i.delivered).unwrap_or(1);
    let payload: T = match serde_json::from_slice(&message.payload) {
        Ok(p) => p,
        Err(e) => {
            dead_letters::bury_or_log(bus, durable, &message.subject, &message.payload, &format!("undecodable job: {e}"), delivered).await;
            let _ = message.ack_with(AckKind::Term).await;
            return;
        }
    };
    let started = std::time::Instant::now();
    let work = handler(payload);
    tokio::pin!(work);
    let mut keepalive = tokio::time::interval(options.lease / 3);
    keepalive.tick().await; // first tick fires immediately
    let result = loop {
        tokio::select! {
            result = &mut work => break result,
            _ = keepalive.tick() => {
                let _ = message.ack_with(AckKind::Progress).await;
            }
        }
    };
    match result {
        Ok(()) => {
            tracing::debug!(
                job = name,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "job done"
            );
            if let Err(e) = message.ack().await {
                tracing::warn!(job = name, error = %e, "job ack failed");
            }
        }
        Err(e) if delivered >= options.max_deliver => {
            dead_letters::bury_or_log(bus, durable, &message.subject, &message.payload, &e.to_string(), delivered).await;
            let _ = message.ack_with(AckKind::Term).await;
        }
        Err(e) => {
            let delay = Duration::from_secs((5u64 << delivered.clamp(0, 6)).min(600));
            tracing::warn!(job = name, delivered, retry_in = ?delay, error = %e, "job failed; will retry");
            let _ = message.ack_with(AckKind::Nak(Some(delay))).await;
        }
    }
}
