//! Background work of the notify domain: the event consumer, the push job
//! runner, the retention sweep, account deletion and data export.
use std::time::Duration;

use async_trait::async_trait;
use tokuly_core::{
    events::{self, ConsumeOptions},
    export::DataExporter,
    prelude::*,
};

use crate::{
    Notify,
    consumer::{CONSUMER_NAME, TOPICS},
    delivery::{PUSH_JOB, PushJob, job_options},
    devices,
};

const RETENTION_INTERVAL: Duration = Duration::from_secs(3600);
/// Rows deleted per statement, so a large backlog never holds one long lock.
const SWEEP_BATCH: i64 = 5000;
/// Handled-event markers are kept a day longer than the event stream keeps
/// events (7 days): after that a redelivery is impossible.
const PROCESSED_EVENT_DAYS: i32 = 8;

/// What one retention sweep removed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RetentionReport {
    pub notifications: u64,
    pub processed_events: u64,
    /// Markers of withdrawn content (`notify_withdrawn`).
    pub withdrawn: u64,
    pub devices: u64,
}

impl Notify {
    /// Starts the notify workers (worker processes only):
    /// the event consumer, the `notify.push` job runner and the hourly
    /// retention sweep.
    pub fn spawn_workers(&self) {
        let notify = self.clone();
        tokio::spawn(async move {
            let handler = notify.clone();
            events::consume(
                notify.core.bus.clone(),
                CONSUMER_NAME,
                TOPICS,
                ConsumeOptions::default(),
                notify.core.shutdown.clone(),
                move |event| {
                    let notify = handler.clone();
                    async move { notify.handle_event(event).await }
                },
            )
            .await;
        });

        let notify = self.clone();
        tokio::spawn(async move {
            let handler = notify.clone();
            notify
                .core
                .jobs
                .run(
                    PUSH_JOB,
                    job_options(),
                    notify.core.shutdown.clone(),
                    move |job: PushJob| {
                        let notify = handler.clone();
                        async move { notify.run_push_job(job).await }
                    },
                )
                .await;
        });

        let notify = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(RETENTION_INTERVAL);
            loop {
                tokio::select! {
                    _ = notify.core.shutdown.cancelled() => break,
                    _ = tick.tick() => match notify.run_retention().await {
                        Ok(report) if report == RetentionReport::default() => {}
                        Ok(report) => tracing::info!(?report, "notify retention sweep"),
                        Err(e) => tracing::error!(error = %e, "notify retention sweep failed"),
                    },
                }
            }
        });
    }

    /// Removes what is past its keep-by date: inbox entries older than the
    /// retention period, markers of long-handled events, and push
    /// registrations whose session ended.
    pub async fn run_retention(&self) -> AppResult<RetentionReport> {
        let db = &self.core.db;
        let mut report = RetentionReport::default();
        loop {
            let done = sqlx::query(
                "DELETE FROM notifications WHERE id IN (
                     SELECT id FROM notifications WHERE occurred_at < now() - make_interval(days => $1) LIMIT $2
                 )",
            )
            .bind(self.cfg.notify_retention_days as i32)
            .bind(SWEEP_BATCH)
            .execute(db)
            .await?;
            report.notifications += done.rows_affected();
            if done.rows_affected() < SWEEP_BATCH as u64 {
                break;
            }
        }
        loop {
            let done = sqlx::query(
                "DELETE FROM notify_processed_events WHERE event_id IN (
                     SELECT event_id FROM notify_processed_events WHERE processed_at < now() - make_interval(days => $1) LIMIT $2
                 )",
            )
            .bind(PROCESSED_EVENT_DAYS)
            .bind(SWEEP_BATCH)
            .execute(db)
            .await?;
            report.processed_events += done.rows_affected();
            if done.rows_affected() < SWEEP_BATCH as u64 {
                break;
            }
        }
        // A request or push about withdrawn content cannot arrive later
        // than any other event, so the markers are kept as long.
        loop {
            let done = sqlx::query(
                "DELETE FROM notify_withdrawn WHERE ref_id IN (
                     SELECT ref_id FROM notify_withdrawn WHERE withdrawn_at < now() - make_interval(days => $1) LIMIT $2
                 )",
            )
            .bind(PROCESSED_EVENT_DAYS)
            .bind(SWEEP_BATCH)
            .execute(db)
            .await?;
            report.withdrawn += done.rows_affected();
            if done.rows_affected() < SWEEP_BATCH as u64 {
                break;
            }
        }
        report.devices = devices::sweep_ended_sessions(&self.core).await?;
        Ok(report)
    }

    /// Handles `identity.user.deleted`: removes everything about the user,
    /// including what they caused in other people's inboxes (those entries
    /// carry their name).
    pub async fn purge_user(&self, user_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        sqlx::query("DELETE FROM notifications WHERE user_id = $1 OR actor_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM notify_mutes WHERE user_id = $1 OR muted_user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM notify_category_prefs WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM notify_settings WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM push_devices WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}

/// The notify section of a personal data export.
pub(crate) struct NotifyExporter {
    pub core: Core,
}

/// Runs a query that returns one JSON document.
async fn json(core: &Core, sql: &'static str, user_id: Uuid) -> AppResult<serde_json::Value> {
    Ok(sqlx::query_scalar(sql)
        .bind(user_id)
        .fetch_one(&core.db)
        .await?)
}

#[async_trait]
impl DataExporter for NotifyExporter {
    fn name(&self) -> &'static str {
        "notify"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        let core = &self.core;
        let notifications = json(
            core,
            "SELECT COALESCE(jsonb_agg(to_jsonb(n) ORDER BY n.occurred_at DESC), '[]'::jsonb) FROM (
                 SELECT id, kind, category, actor_id, title_ja, title_en, body_ja, body_en, image_url, path,
                        collapse_key, data, urgent, group_count, read_at, occurred_at
                 FROM notifications WHERE user_id = $1
             ) n",
            user_id,
        )
        .await?;
        let settings = json(
            core,
            "SELECT COALESCE((
                 SELECT to_jsonb(s) FROM (
                     SELECT push_paused, push_paused_until, quiet_enabled, quiet_start_minute, quiet_end_minute
                     FROM notify_settings WHERE user_id = $1
                 ) s
             ), '{}'::jsonb)",
            user_id,
        )
        .await?;
        let categories = json(
            core,
            "SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY c.category), '[]'::jsonb) FROM (
                 SELECT category, inbox, push FROM notify_category_prefs WHERE user_id = $1
             ) c",
            user_id,
        )
        .await?;
        let mutes = json(
            core,
            "SELECT COALESCE(jsonb_agg(to_jsonb(m) ORDER BY m.muted_at DESC), '[]'::jsonb) FROM (
                 SELECT muted_user_id, created_at AS muted_at FROM notify_mutes WHERE user_id = $1
             ) m",
            user_id,
        )
        .await?;
        // Tokens and keys are credentials for reaching a device, not data
        // about the user; they stay out of the archive.
        let devices = json(
            core,
            "SELECT COALESCE(jsonb_agg(to_jsonb(d) ORDER BY d.created_at DESC), '[]'::jsonb) FROM (
                 SELECT id, channel, platform, locale, device_name, created_at, last_delivered_at
                 FROM push_devices WHERE user_id = $1
             ) d",
            user_id,
        )
        .await?;
        Ok(serde_json::json!({
            "notifications": notifications,
            "settings": settings,
            "category_preferences": categories,
            "muted_users": mutes,
            "push_devices": devices,
        }))
    }
}
