//! Background work of the notes domain: reacting to other domains' events,
//! fetching link previews, expiring stories and ranking hashtags.
use std::time::Duration;

use tokuly_core::{
    events::{self, ConsumeOptions, Event},
    jobs::JobOptions,
    prelude::*,
};
use tokuly_identity::maintenance::UserRef;
use tokuly_social::events::BlockChanged;

use crate::{
    Notes,
    events::{
        ASSET_DELETED, AssetDeleted, CONTENT_REMOVED, FOLLOW_SETTINGS_CHANGED, STORY_EXPIRED, StoryExpired,
        USER_BLOCKED, USER_DELETED,
    },
    posting::{AntennaMatchJob, JOB_ANTENNA_MATCH, JOB_LINK_PREVIEW, LinkPreviewJob},
};

/// Stories archived per pass of the expiry worker.
const EXPIRY_BATCH: i64 = 500;

impl Notes {
    /// Starts the notes workers (worker processes only):
    /// - the consumer of `identity.user.deleted`, `social.user.blocked`,
    ///   `media.asset.deleted` and `admin.content.removed`;
    /// - the `notes.link_preview` and `notes.antenna_match` jobs;
    /// - the story expiry pass, every `story_expiry_interval_seconds`;
    /// - the trending hashtags refresh, every `trend_interval_seconds`.
    pub fn spawn_workers(&self) {
        let notes = self.clone();
        tokio::spawn(async move {
            let handler = notes.clone();
            events::consume(
                notes.core.bus.clone(),
                "notes",
                &[USER_DELETED, USER_BLOCKED, FOLLOW_SETTINGS_CHANGED, ASSET_DELETED, CONTENT_REMOVED],
                // Erasing a prolific account takes a while.
                ConsumeOptions { ack_wait: Duration::from_secs(300), ..Default::default() },
                notes.core.shutdown.clone(),
                move |event| {
                    let notes = handler.clone();
                    async move { notes.handle_event(event).await }
                },
            )
            .await;
        });

        let notes = self.clone();
        tokio::spawn(async move {
            let handler = notes.clone();
            notes
                .core
                .jobs
                .run(
                    JOB_LINK_PREVIEW,
                    // A page that does not answer is not worth many attempts.
                    JobOptions { concurrency: 4, lease: Duration::from_secs(60), max_deliver: 3 },
                    notes.core.shutdown.clone(),
                    move |job: LinkPreviewJob| {
                        let notes = handler.clone();
                        async move { notes.resolve_link_preview(job.note_id).await }
                    },
                )
                .await;
        });

        let notes = self.clone();
        tokio::spawn(async move {
            let handler = notes.clone();
            notes
                .core
                .jobs
                .run(
                    JOB_ANTENNA_MATCH,
                    JobOptions { concurrency: 4, lease: Duration::from_secs(60), max_deliver: 5 },
                    notes.core.shutdown.clone(),
                    move |job: AntennaMatchJob| {
                        let notes = handler.clone();
                        async move { notes.match_antennas(job.note_id).await.map(|_| ()) }
                    },
                )
                .await;
        });

        self.spawn_periodic(
            "story expiry",
            Duration::from_secs(self.cfg.story_expiry_interval_seconds),
            |notes| async move {
                match notes.expire_stories().await? {
                    0 => {}
                    expired => tracing::info!(expired, "stories moved to the archive"),
                }
                Ok(())
            },
        );
        self.spawn_periodic(
            "trending hashtags",
            Duration::from_secs(self.cfg.trend_interval_seconds),
            |notes| async move { notes.refresh_trends().await.map(|_| ()) },
        );
    }

    /// Runs `job` every `every` until shutdown, logging failures.
    fn spawn_periodic<F, Fut>(&self, name: &'static str, every: Duration, job: F)
    where
        F: Fn(Notes) -> Fut + Send + 'static,
        Fut: Future<Output = AppResult<()>> + Send,
    {
        let notes = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(every);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = notes.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        if let Err(e) = job(notes.clone()).await {
                            tracing::error!(job = name, error = %e, "periodic job failed");
                        }
                    }
                }
            }
        });
    }

    /// Dispatches a consumed event (see [`cleanup`](crate::cleanup) for the
    /// reactions). Public so tests can deliver events without a running relay.
    pub async fn handle_event(&self, event: Event) -> AppResult<()> {
        match event.topic.as_str() {
            USER_DELETED => self.on_user_deleted(event.data::<UserRef>()?.user_id).await,
            USER_BLOCKED => {
                let block: BlockChanged = event.data()?;
                self.on_user_blocked(block.user_id, block.blocked_id).await
            }
            // Which way the setting went is read again from the social
            // domain: events may arrive late or twice.
            FOLLOW_SETTINGS_CHANGED => self.sync_author(event.data::<UserRef>()?.user_id).await.map(|_| ()),
            ASSET_DELETED => self.on_asset_deleted(event.data::<AssetDeleted>()?.asset_id).await,
            CONTENT_REMOVED => self.on_content_removed(event.data()?).await,
            _ => Ok(()),
        }
    }

    /// Marks stories whose day is over as archived and announces each as
    /// `notes.story.expired` (the story expiry worker; public so tools and
    /// tests can run a pass). Returns how many expired.
    ///
    /// Who sees a story never depends on this pass — `expires_at` decides
    /// that the moment it is reached. The pass exists so other domains hear
    /// about the expiry, exactly once per story.
    pub async fn expire_stories(&self) -> AppResult<u64> {
        let mut total = 0;
        loop {
            let mut tx = self.core.db.begin().await?;
            let expired: Vec<(Uuid, Uuid)> = sqlx::query_as(
                "UPDATE stories SET archived_at = now()
                 WHERE id IN (
                     SELECT id FROM stories WHERE archived_at IS NULL AND expires_at <= now()
                     ORDER BY expires_at LIMIT $1 FOR UPDATE SKIP LOCKED
                 )
                 RETURNING id, owner_id",
            )
            .bind(EXPIRY_BATCH)
            .fetch_all(&mut *tx)
            .await?;
            for (story_id, owner_id) in &expired {
                events::emit(&mut *tx, STORY_EXPIRED, None, &StoryExpired { story_id: *story_id, owner_id: *owner_id })
                    .await?;
            }
            tx.commit().await?;
            total += expired.len() as u64;
            if (expired.len() as i64) < EXPIRY_BATCH {
                return Ok(total);
            }
        }
    }
}
