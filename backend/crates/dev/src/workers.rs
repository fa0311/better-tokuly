//! Background loops of the developer platform (worker processes only).
use std::time::Duration;

use tokuly_core::{
    events::{self, ConsumeOptions},
    jobs::JobOptions,
};

use crate::{Dev, cleanup, delivery::DELIVER_JOB, events::DeliverJob, fanout};

/// How often deliveries whose retry is due are queued.
const SWEEP_INTERVAL: Duration = Duration::from_secs(15);
const HOUSEKEEPING_INTERVAL: Duration = Duration::from_secs(3600);

impl Dev {
    /// Starts the event consumers (webhook fan-out; account bans, deletions
    /// and takedowns), the delivery job, the retry sweep and housekeeping.
    pub fn spawn_workers(&self) {
        let core = &self.core;

        let dev = self.clone();
        let (bus, cancel) = (core.bus.clone(), core.shutdown.clone());
        tokio::spawn(async move {
            let topics = fanout::source_topics();
            events::consume(bus, "dev-webhooks", &topics, ConsumeOptions::default(), cancel, move |event| {
                let dev = dev.clone();
                async move { dev.fan_out(&event).await.map(|_| ()) }
            })
            .await;
        });

        let dev = self.clone();
        let (bus, cancel) = (core.bus.clone(), core.shutdown.clone());
        tokio::spawn(async move {
            events::consume(
                bus,
                "dev-accounts",
                cleanup::ACCOUNT_TOPICS,
                ConsumeOptions::default(),
                cancel,
                move |event| {
                    let dev = dev.clone();
                    async move { dev.on_account_event(&event).await }
                },
            )
            .await;
        });

        let dev = self.clone();
        tokio::spawn(async move {
            // The lease outlasts the slowest attempt: a connect and a full timeout.
            let options = JobOptions {
                concurrency: 16,
                lease: dev.cfg.webhook_timeout() * 2 + Duration::from_secs(30),
                max_deliver: 3,
            };
            let handler_dev = dev.clone();
            dev.core
                .jobs
                .run(DELIVER_JOB, options, dev.core.shutdown.clone(), move |job: DeliverJob| {
                    let dev = handler_dev.clone();
                    async move { dev.deliver(job.delivery_id).await.map(|_| ()) }
                })
                .await;
        });

        let dev = self.clone();
        tokio::spawn(async move {
            let mut sweep = tokio::time::interval(SWEEP_INTERVAL);
            let mut housekeeping = tokio::time::interval(HOUSEKEEPING_INTERVAL);
            loop {
                tokio::select! {
                    _ = dev.core.shutdown.cancelled() => break,
                    _ = sweep.tick() => {
                        if let Err(e) = dev.queue_due_deliveries().await {
                            tracing::warn!(error = %e, "webhook retry sweep failed");
                        }
                    }
                    _ = housekeeping.tick() => {
                        if let Err(e) = dev.purge_old_deliveries().await {
                            tracing::warn!(error = %e, "webhook log cleanup failed");
                        }
                        if let Err(e) = dev.purge_dead_oauth_tokens().await {
                            tracing::warn!(error = %e, "OAuth token cleanup failed");
                        }
                        if let Err(e) = dev.purge_bot_records().await {
                            tracing::warn!(error = %e, "bot record cleanup failed");
                        }
                    }
                }
            }
        });
    }
}
