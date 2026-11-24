//! Background work of the admin domain (worker processes only):
//!
//! - an event consumer filing reports into the moderation queue, erasing
//!   personal data of deleted accounts and promoting the configured
//!   first-deployment admin when that address is verified;
//! - the `admin.announcement.broadcast` job;
//! - a once-a-minute timer that lifts expired suspensions, lets go of the
//!   evidence of cases resolved long ago and queues the broadcast of
//!   announcements whose publish time has come.
use std::time::Duration;

use tokuly_core::{
    events::{self as core_events, ConsumeOptions, Event},
    jobs::JobOptions,
    prelude::*,
};

use crate::{
    Admin,
    announcements::{BROADCAST_JOB, BroadcastJob},
    events::{REPORT_CREATED, USER_CREATED, USER_DELETED, USER_EMAIL_VERIFIED},
};

/// Durable consumer name on the `EVENTS` stream.
const EVENT_CONSUMER: &str = "admin";

const TIMER_INTERVAL: Duration = Duration::from_secs(60);

impl Admin {
    /// Handles one event from the stream. Public so tests (and tools) can
    /// feed events directly; unknown topics are ignored.
    pub async fn handle_event(&self, event: Event) -> AppResult<()> {
        match event.topic.as_str() {
            REPORT_CREATED => self.on_report_created(&event).await,
            USER_CREATED | USER_EMAIL_VERIFIED => self.grant_bootstrap_admin().await.map(drop),
            USER_DELETED => self.on_user_deleted(&event).await,
            _ => Ok(()),
        }
    }

    /// One pass of the periodic work. Each part logs its own failure so one
    /// cannot starve the other.
    pub async fn run_maintenance(&self) {
        match self.expire_due_suspensions().await {
            Ok(0) => {}
            Ok(lifted) => tracing::info!(lifted, "expired suspensions lifted"),
            Err(e) => tracing::error!(error = %e, "lifting expired suspensions failed"),
        }
        match self.release_due_evidence().await {
            Ok(0) => {}
            Ok(released) => tracing::info!(released, "evidence of old cases released"),
            Err(e) => tracing::error!(error = %e, "releasing the evidence of old cases failed"),
        }
        match self.queue_due_broadcasts().await {
            Ok(0) => {}
            Ok(queued) => tracing::info!(queued, "announcement broadcasts queued"),
            Err(e) => tracing::error!(error = %e, "queueing announcement broadcasts failed"),
        }
    }

    /// Starts the admin workers. Call once, in worker processes.
    pub fn spawn_workers(&self) {
        let admin = self.clone();
        tokio::spawn(async move {
            let handler = admin.clone();
            core_events::consume(
                admin.core.bus.clone(),
                EVENT_CONSUMER,
                &[REPORT_CREATED, USER_CREATED, USER_EMAIL_VERIFIED, USER_DELETED],
                ConsumeOptions::default(),
                admin.core.shutdown.clone(),
                move |event| {
                    let admin = handler.clone();
                    async move { admin.handle_event(event).await }
                },
            )
            .await;
        });

        let admin = self.clone();
        tokio::spawn(async move {
            let handler = admin.clone();
            // One broadcast at a time per process: each is a long, chunked
            // loop and there is no hurry. The lease is renewed while it runs.
            let options = JobOptions { concurrency: 1, lease: Duration::from_secs(120), max_deliver: 5 };
            admin
                .core
                .jobs
                .run(BROADCAST_JOB, options, admin.core.shutdown.clone(), move |job: BroadcastJob| {
                    let admin = handler.clone();
                    async move { admin.run_broadcast(job).await }
                })
                .await;
        });

        let admin = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(TIMER_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = admin.core.shutdown.cancelled() => break,
                    _ = tick.tick() => admin.run_maintenance().await,
                }
            }
        });
    }
}
