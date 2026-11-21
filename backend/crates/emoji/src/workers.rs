//! Background work of the emoji domain: reacting to other domains' events,
//! the import and export jobs, the usage worker and periodic upkeep.
use std::time::Duration;

use tokuly_core::{
    events::{self, ConsumeOptions, Event},
    jobs::JobOptions,
    prelude::*,
};
use tokuly_identity::maintenance::UserRef;
use tokuly_proto::common::SubjectType;

use crate::{
    Emoji, catalog,
    emojis::{self, Effects},
    events::{
        ASSET_DELETED, ASSET_FAILED, ASSET_READY, AssetEvent, CONTENT_REMOVED, ContentRemoved, ExportJob, ImportJob,
        JOB_EXPORT, JOB_IMPORT, RemovedBecause, USER_DELETED,
    },
    model::{EmojiRow, PICTURE_PURPOSE, emoji_columns},
    usage,
    validate::REASON_MAX_CHARS,
};

/// How often the cache is reconciled and imports and exports are swept.
const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(60);
/// Maintenance rounds between two prunings (once an hour).
const PRUNE_EVERY: u64 = 60;
/// The note on an emoji taken down without a stated reason.
const TAKEDOWN_WITHOUT_REASON: &str = "通報により非公開になりました (removed after a report)";

impl Emoji {
    /// Starts the emoji workers (worker processes only):
    /// - the consumer of `identity.user.deleted`, `media.asset.*` and
    ///   `admin.content.removed`;
    /// - the `emoji.import` and `emoji.export` jobs;
    /// - the usage worker, every `usage_flush_seconds`;
    /// - maintenance every minute, pruning every hour.
    pub fn spawn_workers(&self) {
        let emoji = self.clone();
        tokio::spawn(async move {
            let handler = emoji.clone();
            events::consume(
                emoji.core.bus.clone(),
                "emoji",
                &[USER_DELETED, ASSET_READY, ASSET_FAILED, ASSET_DELETED, CONTENT_REMOVED],
                ConsumeOptions::default(),
                emoji.core.shutdown.clone(),
                move |event| {
                    let emoji = handler.clone();
                    async move { emoji.handle_event(event).await }
                },
            )
            .await;
        });

        let emoji = self.clone();
        tokio::spawn(async move {
            let handler = emoji.clone();
            // A pack is read by one job at a time; its pictures are
            // processed by the media workers.
            let options = JobOptions { concurrency: 1, lease: Duration::from_secs(120), max_deliver: 3 };
            emoji
                .core
                .jobs
                .run(JOB_IMPORT, options, emoji.core.shutdown.clone(), move |job: ImportJob| {
                    let emoji = handler.clone();
                    async move { emoji.run_import(job.import_id).await }
                })
                .await;
        });

        let emoji = self.clone();
        tokio::spawn(async move {
            let handler = emoji.clone();
            let options = JobOptions { concurrency: 1, lease: Duration::from_secs(120), max_deliver: 3 };
            emoji
                .core
                .jobs
                .run(JOB_EXPORT, options, emoji.core.shutdown.clone(), move |job: ExportJob| {
                    let emoji = handler.clone();
                    async move { emoji.run_export(job.export_id).await }
                })
                .await;
        });

        let emoji = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(emoji.cfg.usage_flush_seconds.max(5)));
            loop {
                tokio::select! {
                    _ = emoji.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        if let Err(e) = emoji.aggregate_usage().await {
                            tracing::error!(error = %e, "emoji usage aggregation failed");
                        }
                    }
                }
            }
        });

        let emoji = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(MAINTENANCE_INTERVAL);
            let mut round: u64 = 0;
            loop {
                tokio::select! {
                    _ = emoji.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        if let Err(e) = emoji.maintain().await {
                            tracing::error!(error = %e, "emoji maintenance failed");
                        }
                        if round.is_multiple_of(PRUNE_EVERY)
                            && let Err(e) = emoji.prune().await
                        {
                            tracing::error!(error = %e, "emoji pruning failed");
                        }
                        round += 1;
                    }
                }
            }
        });
    }

    /// Dispatches a consumed event. Public so tests can deliver events
    /// without a running relay.
    pub async fn handle_event(&self, event: Event) -> AppResult<()> {
        match event.topic.as_str() {
            USER_DELETED => self.on_user_deleted(event.data::<UserRef>()?.user_id).await,
            ASSET_READY | ASSET_FAILED => {
                let upload: AssetEvent = event.data()?;
                // Every upload in the system passes by here; only pictures
                // can belong to an import.
                if upload.purpose == 0 || upload.purpose == PICTURE_PURPOSE as i32 {
                    self.settle_asset(upload.asset_id).await?;
                }
                Ok(())
            }
            ASSET_DELETED => self.on_asset_deleted(event.data::<AssetEvent>()?.asset_id).await,
            CONTENT_REMOVED => self.on_content_removed(event.data()?).await,
            _ => Ok(()),
        }
    }

    /// One round of the usage worker: the use counters move from Valkey
    /// into the database and the popularity scores are recomputed. Returns
    /// the uses written.
    pub async fn aggregate_usage(&self) -> AppResult<u64> {
        let written = usage::flush(&self.core).await?;
        usage::refresh_popularity(&self.core, self.cfg.popular_days).await?;
        Ok(written)
    }

    /// One round of maintenance: the catalogue cache and its announcements
    /// are brought in line with the database, imports and exports are swept.
    pub async fn maintain(&self) -> AppResult<()> {
        catalog::reconcile(&self.core).await?;
        self.sweep_imports().await?;
        self.sweep_exports().await
    }

    /// Hourly: forgets removals and usage rows past their retention.
    pub async fn prune(&self) -> AppResult<()> {
        let removals = catalog::prune_removals(&self.core, self.cfg.removal_retention_days).await?;
        let usage_rows = usage::prune(&self.core).await?;
        if removals + usage_rows > 0 {
            tracing::info!(removals, usage_rows, "pruned emoji bookkeeping");
        }
        Ok(())
    }

    /// `identity.user.deleted`: what was theirs alone goes — submissions
    /// that never made it into the catalogue, their deck, their recently
    /// used list, their trusted flag. Emoji the community uses stay, without
    /// the credit; they keep their upload, which the media domain spares
    /// for as long as it is held.
    pub(crate) async fn on_user_deleted(&self, user_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let mut fx = Effects::default();
        let unpublished: Vec<EmojiRow> = sqlx::query_as(concat!(
            "SELECT ",
            emoji_columns!(),
            " FROM emojis e WHERE e.author_id = $1 AND e.state IN (1, 3) ORDER BY e.id FOR UPDATE"
        ))
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?;
        for emoji in &unpublished {
            emojis::delete(&mut tx, &mut fx, emoji, RemovedBecause::Deleted, None).await?;
        }
        let kept: Vec<Uuid> =
            sqlx::query_scalar("UPDATE emojis SET author_id = NULL WHERE author_id = $1 RETURNING id")
                .bind(user_id)
                .fetch_all(&mut *tx)
                .await?;
        // Search hides documents of deleted owners; these have none now.
        emojis::reindex(&mut tx, &kept).await?;

        for table in ["emoji_decks", "emoji_recents", "emoji_submitters"] {
            sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table} WHERE user_id = $1")))
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        }
        for (table, column) in [
            ("emoji_submitters", "trusted_by"),
            ("emoji_reviews", "actor_id"),
            ("emoji_imports", "requested_by"),
            ("emoji_exports", "requested_by"),
        ] {
            sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE {table} SET {column} = NULL WHERE {column} = $1")))
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        }
        self.commit(tx, fx).await
    }

    /// `media.asset.deleted`: the files of an upload are gone, so the emoji
    /// showing it is deleted rather than left with a broken picture.
    ///
    /// This is a safety net. An emoji holds its upload, so neither its
    /// uploader nor the media janitor can delete it; the event only reaches
    /// an emoji when some domain deleted the upload by force.
    pub(crate) async fn on_asset_deleted(&self, asset_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let mut fx = Effects::default();
        let broken: Vec<EmojiRow> = sqlx::query_as(concat!(
            "SELECT ",
            emoji_columns!(),
            " FROM emojis e WHERE e.asset_id = $1 ORDER BY e.id FOR UPDATE"
        ))
        .bind(asset_id)
        .fetch_all(&mut *tx)
        .await?;
        if broken.is_empty() {
            return Ok(());
        }
        for emoji in &broken {
            emojis::delete(&mut tx, &mut fx, emoji, RemovedBecause::AssetDeleted, None).await?;
        }
        self.commit(tx, fx).await
    }

    /// `admin.content.removed`: a report against an emoji was upheld. The
    /// emoji is disabled — out of the catalogue, its name kept so nothing
    /// else can pass for it. An emoji that is unknown or already out of the
    /// catalogue is left as it is: the event may arrive twice.
    pub(crate) async fn on_content_removed(&self, removed: ContentRemoved) -> AppResult<()> {
        if removed.subject_type != SubjectType::SUBJECT_TYPE_CUSTOM_EMOJI as i32 {
            return Ok(());
        }
        let mut tx = self.core.db.begin().await?;
        let mut fx = Effects::default();
        let target: Option<EmojiRow> =
            sqlx::query_as(concat!("SELECT ", emoji_columns!(), " FROM emojis e WHERE e.id = $1 FOR UPDATE"))
                .bind(removed.subject_id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some(emoji) = target.filter(EmojiRow::is_approved) else { return Ok(()) };
        let reason: String = match removed.reason.trim() {
            "" => TAKEDOWN_WITHOUT_REASON.to_owned(),
            reason => reason.chars().take(REASON_MAX_CHARS).collect(),
        };
        emojis::disable(&mut tx, &mut fx, &emoji, removed.moderator_id, &reason, true).await?;
        self.commit(tx, fx).await
    }
}
