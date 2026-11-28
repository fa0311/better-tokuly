//! Background work of the moments domain (worker processes only).
//!
//! | What | How often |
//! |---|---|
//! | event consumer `moments` (see `cleanup`) | — |
//! | planner: time zones to bands, days ahead, prompts due (see `days`) | `TOKULY_MOMENTS_WORKER_INTERVAL_SECONDS` (20 s) |
//! | job `moments.prompt`: announce a day and tell its region | queued by the planner, or by an admin's `SendPromptNow` |
//! | purge of what was kept for reports | hourly |
//!
//! Everything is safe to run in several processes at once: a day is
//! announced and told under its row lock, and the purge skips rows another
//! process holds.
use std::time::Duration;

use tokuly_core::{
    events::{self, ConsumeOptions},
    jobs::JobOptions,
    prelude::*,
};

use crate::{
    Moments,
    days::{JOB_PROMPT, PromptJob},
    events::{ASSET_DELETED, BIRTHDATE_GIVEN, CONTENT_REMOVED, USER_CREATED, USER_DELETED},
};

const PURGE_INTERVAL: Duration = Duration::from_secs(3600);

impl Moments {
    /// Starts the workers. Call once, in worker processes.
    pub fn spawn_workers(&self) {
        let moments = self.clone();
        tokio::spawn(async move {
            let handler = moments.clone();
            events::consume(
                moments.core.bus.clone(),
                "moments",
                &[USER_CREATED, BIRTHDATE_GIVEN, USER_DELETED, CONTENT_REMOVED, ASSET_DELETED],
                ConsumeOptions::default(),
                moments.core.shutdown.clone(),
                move |event| {
                    let moments = handler.clone();
                    async move { moments.handle_event(&event).await }
                },
            )
            .await;
        });

        let moments = self.clone();
        tokio::spawn(async move {
            let handler = moments.clone();
            // Telling a large region takes a while; the lease is renewed while it runs.
            let options = JobOptions { concurrency: 2, lease: Duration::from_secs(120), max_deliver: 20 };
            moments
                .core
                .jobs
                .run(JOB_PROMPT, options, moments.core.shutdown.clone(), move |job: PromptJob| {
                    let moments = handler.clone();
                    async move { moments.send_prompt(job.day_id).await.map(|_| ()) }
                })
                .await;
        });

        let every = Duration::from_secs(self.cfg.worker_interval_seconds);
        self.spawn_periodic("planner", every, |moments| async move { moments.plan().await.map(|_| ()) });
        self.spawn_periodic("purge", PURGE_INTERVAL, |moments| async move { moments.purge().await.map(|_| ()) });
    }

    /// Runs `job` every `every` until shutdown, logging failures.
    fn spawn_periodic<F, Fut>(&self, name: &'static str, every: Duration, job: F)
    where
        F: Fn(Moments) -> Fut + Send + 'static,
        Fut: Future<Output = AppResult<()>> + Send + 'static,
    {
        let moments = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(every);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = moments.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        if let Err(e) = job(moments.clone()).await {
                            tracing::error!(job = name, error = %e, "periodic moments job failed");
                        }
                    }
                }
            }
        });
    }
}
