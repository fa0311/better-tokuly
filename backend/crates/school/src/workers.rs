//! Background work of the school domain: the consumer of other domains'
//! events and the periodic sweep.
use std::time::Duration;

use tokuly_core::{
    events::{self, ConsumeOptions},
    prelude::*,
};

use crate::{
    School,
    events::{CONTENT_REMOVED, FRIEND_REMOVED, USER_BLOCKED, USER_DELETED},
};

/// What one pass of the sweep did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Study rooms whose timer moved on to its next phase.
    pub phases_announced: usize,
    /// Sessions ended because their client fell silent or a limit was reached.
    pub sessions_ended: usize,
    /// Rooms closed after standing empty.
    pub rooms_closed: usize,
    pub class_reminders: usize,
    pub assignment_reminders: usize,
}

impl School {
    /// Starts the school workers (worker processes only):
    /// - the consumer of `identity.user.deleted`, `social.friend.removed`,
    ///   `social.user.blocked` and `admin.content.removed`;
    /// - the sweep, every `sweep_interval_seconds`.
    pub fn spawn_workers(&self) {
        let school = self.clone();
        tokio::spawn(async move {
            let handler = school.clone();
            events::consume(
                school.core.bus.clone(),
                "school",
                &[USER_DELETED, FRIEND_REMOVED, USER_BLOCKED, CONTENT_REMOVED],
                ConsumeOptions::default(),
                school.core.shutdown.clone(),
                move |event| {
                    let school = handler.clone();
                    async move { school.handle_event(&event).await }
                },
            )
            .await;
        });

        let school = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(school.cfg.sweep_interval_seconds));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = school.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        school.sweep(school.now()).await;
                    }
                }
            }
        });
    }

    /// One pass over everything that happens by the clock rather than by a
    /// request: study room phases that ended, sessions nobody is behind
    /// anymore, rooms standing empty, and reminders that are due. Each step
    /// is safe to run in several processes at once; a step that fails is
    /// logged and tried again on the next pass.
    pub async fn sweep(&self, at: Time) -> SweepReport {
        fn done(step: &'static str, result: AppResult<usize>) -> usize {
            result.unwrap_or_else(|e| {
                tracing::error!(step, error = %e, "school sweep step failed");
                0
            })
        }
        SweepReport {
            phases_announced: done("room phases", self.announce_room_phases(at).await),
            sessions_ended: done("overdue sessions", self.end_overdue_sessions(at).await),
            rooms_closed: done("empty rooms", self.close_empty_rooms(at).await),
            class_reminders: done("class reminders", self.send_class_reminders(at).await),
            assignment_reminders: done("assignment reminders", self.send_assignment_reminders(at).await),
        }
    }
}
