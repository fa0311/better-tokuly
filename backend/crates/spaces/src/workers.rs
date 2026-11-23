//! Background work of the spaces domain (worker processes only).
//!
//! | What | How often |
//! |---|---|
//! | event consumer `spaces` (see [`crate::cleanup`]) | — |
//! | reconciliation with LiveKit, then hand-over / end of rooms whose host is away or that are empty | `TOKULY_SPACES_RECONCILE_INTERVAL_SECONDS` (20 s) |
//! | housekeeping: host nudges, rooms nobody started, LiveKit rooms left open, old history | 60 s |
//!
//! Every job is safe to run in several processes at once: each change is
//! made under the lock of the room or hangout it concerns.
use std::time::Duration;

use tokuly_core::{
    events::{self, ConsumeOptions},
    prelude::*,
};

use crate::{
    Spaces,
    events::{CONTENT_REMOVED, FRIEND_REMOVED, ROOM_STARTED, USER_BANNED, USER_BLOCKED, USER_DELETED},
};

const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

impl Spaces {
    /// Starts the workers. Call once, in worker processes.
    pub fn spawn_workers(&self) {
        let spaces = self.clone();
        tokio::spawn(async move {
            let handler = spaces.clone();
            events::consume(
                spaces.core.bus.clone(),
                "spaces",
                &[USER_DELETED, USER_BANNED, USER_BLOCKED, FRIEND_REMOVED, CONTENT_REMOVED, ROOM_STARTED],
                // Telling a thousand friends that a room started takes a moment.
                ConsumeOptions { ack_wait: Duration::from_secs(120), ..Default::default() },
                spaces.core.shutdown.clone(),
                move |event| {
                    let spaces = handler.clone();
                    async move { spaces.handle_event(&event).await }
                },
            )
            .await;
        });

        let every = Duration::from_secs(self.cfg.reconcile_interval_seconds);
        self.spawn_periodic("presence reconciliation", every, |spaces| async move {
            spaces.reconcile().await?;
            spaces.settle_rooms().await?;
            Ok(())
        });
        self.spawn_periodic("housekeeping", SWEEP_INTERVAL, |spaces| async move { spaces.sweep().await });
    }

    /// Runs `job` every `every` until shutdown, logging failures.
    fn spawn_periodic<F, Fut>(&self, name: &'static str, every: Duration, job: F)
    where
        F: Fn(Spaces) -> Fut + Send + 'static,
        Fut: Future<Output = AppResult<()>> + Send + 'static,
    {
        let spaces = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(every);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = spaces.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        if let Err(e) = job(spaces.clone()).await {
                            tracing::error!(job = name, error = %e, "periodic spaces job failed");
                        }
                    }
                }
            }
        });
    }
}
