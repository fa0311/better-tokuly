//! Background work of the chat domain (worker processes only).
//!
//! | What | How often |
//! |---|---|
//! | event consumer `chat` (other domains' events, see [`crate::cleanup`]) | — |
//! | typing and co-presence fan-out (gateway signals, queue groups) | — |
//! | `chat.link_preview` job | — |
//! | co-presence sweep (members who went away silently) | 10 s |
//! | ringing time-out of calls | 5 s |
//! | reconciliation of live calls with LiveKit | 30 s |
//! | POV expiry (unanswered requests, unsaved answers) | 60 s |
//! | link previews whose job was lost | 5 min |
//! | texts kept for reports after an edit or unsend, past their 30 days | 1 h |
use std::time::Duration;

use tokuly_core::{
    events::{self, ConsumeOptions},
    jobs::JobOptions,
    prelude::*,
};

use crate::{
    Chat,
    events::{
        ASSET_DELETED, ASSET_FAILED, ASSET_READY, CONTENT_REMOVED, FRIEND_ACCEPTED, USER_BLOCKED, USER_CREATED,
        USER_DELETED, USER_UNBLOCKED,
    },
    send::{JOB_LINK_PREVIEW, LinkPreviewJob},
    server_notices::{JOB_NOTICES, NoticeBatch},
};

const PRESENCE_SWEEP_INTERVAL: Duration = Duration::from_secs(10);
const RINGING_SWEEP_INTERVAL: Duration = Duration::from_secs(5);
const CALL_RECONCILE_INTERVAL: Duration = Duration::from_secs(30);
const POV_SWEEP_INTERVAL: Duration = Duration::from_secs(60);
const LINK_SWEEP_INTERVAL: Duration = Duration::from_secs(300);
const REMOVED_TEXT_PURGE_INTERVAL: Duration = Duration::from_secs(3600);

impl Chat {
    /// Starts the chat workers. Call once, in worker processes.
    pub fn spawn_workers(&self) {
        let chat = self.clone();
        tokio::spawn(async move {
            let handler = chat.clone();
            events::consume(
                chat.core.bus.clone(),
                "chat",
                &[
                    USER_CREATED,
                    USER_DELETED,
                    USER_BLOCKED,
                    USER_UNBLOCKED,
                    FRIEND_ACCEPTED,
                    ASSET_READY,
                    ASSET_FAILED,
                    ASSET_DELETED,
                    CONTENT_REMOVED,
                ],
                // Removing a member of many groups takes a moment.
                ConsumeOptions { ack_wait: Duration::from_secs(120), ..Default::default() },
                chat.core.shutdown.clone(),
                move |event| {
                    let chat = handler.clone();
                    async move { chat.handle_event(&event).await }
                },
            )
            .await;
        });

        let chat = self.clone();
        tokio::spawn(async move { chat.run_typing_fanout().await });
        let chat = self.clone();
        tokio::spawn(async move { chat.run_presence_fanout().await });

        let chat = self.clone();
        tokio::spawn(async move {
            let handler = chat.clone();
            chat.core
                .jobs
                .run(
                    JOB_LINK_PREVIEW,
                    JobOptions { concurrency: 8, lease: Duration::from_secs(60), max_deliver: 3 },
                    chat.core.shutdown.clone(),
                    move |job: LinkPreviewJob| {
                        let chat = handler.clone();
                        async move { chat.resolve_link_preview(job.message_id).await }
                    },
                )
                .await;
        });

        let chat = self.clone();
        tokio::spawn(async move {
            let handler = chat.clone();
            chat.core
                .jobs
                .run(
                    JOB_NOTICES,
                    JobOptions { concurrency: 4, lease: Duration::from_secs(120), max_deliver: 5 },
                    chat.core.shutdown.clone(),
                    move |batch: NoticeBatch| {
                        let chat = handler.clone();
                        async move { chat.post_notice_batch(batch).await }
                    },
                )
                .await;
        });

        self.spawn_periodic("co-presence sweep", PRESENCE_SWEEP_INTERVAL, |chat| async move {
            chat.sweep_presence().await.map(|_| ())
        });
        self.spawn_periodic("call ringing time-out", RINGING_SWEEP_INTERVAL, |chat| async move {
            chat.expire_ringing_calls().await.map(|_| ())
        });
        self.spawn_periodic("call reconciliation", CALL_RECONCILE_INTERVAL, |chat| async move {
            chat.reconcile_calls().await.map(|_| ())
        });
        self.spawn_periodic(
            "POV expiry",
            POV_SWEEP_INTERVAL,
            |chat| async move { chat.expire_povs().await.map(|_| ()) },
        );
        self.spawn_periodic("link preview sweep", LINK_SWEEP_INTERVAL, |chat| async move {
            chat.sweep_link_previews().await.map(|_| ())
        });
        self.spawn_periodic("removed text purge", REMOVED_TEXT_PURGE_INTERVAL, |chat| async move {
            chat.purge_removed_texts().await.map(|_| ())
        });
    }

    /// Runs `job` every `every` until shutdown, logging failures.
    fn spawn_periodic<F, Fut>(&self, name: &'static str, every: Duration, job: F)
    where
        F: Fn(Chat) -> Fut + Send + 'static,
        Fut: Future<Output = AppResult<()>> + Send + 'static,
    {
        let chat = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(every);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = chat.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        if let Err(e) = job(chat.clone()).await {
                            tracing::error!(job = name, error = %e, "periodic chat job failed");
                        }
                    }
                }
            }
        });
    }
}
