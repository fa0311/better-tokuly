//! Push delivery: one background job per notification, fanned out to the
//! recipient's devices.
//!
//! The event consumer decides *whether* a notification is pushed and queues a
//! [`PushJob`]. This module does the sending: it loads the user's
//! registrations, writes the message in each device's language, hands it to
//! the device's [`PushChannel`](crate::push::PushChannel) and acts on what
//! the push service said.
//!
//! Retries: when a push service is temporarily unreachable the job fails and
//! `core.jobs` runs it again with backoff, up to [`MAX_ATTEMPTS`] times.
//! Devices that already got the message are remembered in Valkey for the
//! lifetime of the job, so a retry only reaches the devices still missing it.
use std::{collections::HashSet, time::Duration};

use redis::AsyncCommands as _;
use serde::{Deserialize, Serialize};
use tokuly_core::{jobs::JobOptions, outbound::Localized, prelude::*};
use tokuly_proto::notify::PushPreview;

use crate::{
    Notify, blocks, categories,
    devices::{self, DeliveryDevice},
    edits::{self, Current},
    inbox, preferences, preview,
    push::{PushMessage, PushOutcome, PushPriority},
    withdraw,
};

pub const PUSH_JOB: &str = "notify.push";
/// Attempts per job, the first one included.
pub const MAX_ATTEMPTS: i64 = 5;
/// How long the "already delivered" set of a job is kept: comfortably longer
/// than all retries of a job take.
const DELIVERED_SET_TTL: Duration = Duration::from_secs(3 * 3600);

/// A notification to push to every device of one user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushJob {
    /// Identifies this push across retries (the ID of the event behind it).
    pub id: Uuid,
    pub user_id: Uuid,
    /// Who caused it, when the recipient can block or mute them: a job is
    /// dropped when that happened while it waited. `None` for system notices.
    #[serde(default)]
    pub actor_id: Option<Uuid>,
    /// The inbox entry, when the notification has one.
    pub notification_id: Option<Uuid>,
    pub kind: String,
    pub category: String,
    pub title: Localized,
    pub body: Localized,
    pub image_url: Option<String>,
    pub path: String,
    pub collapse_key: Option<String>,
    /// Unread inbox entries at the time the notification was recorded.
    pub badge: i64,
    #[serde(default)]
    pub data: serde_json::Value,
    /// What the notification refers to, when it can be withdrawn: a job for
    /// withdrawn content sends nothing.
    #[serde(default)]
    pub refs: Vec<Uuid>,
    /// The producing domain marked the notification urgent (incoming call,
    /// SOS): delivered at the highest priority every channel offers.
    #[serde(default)]
    pub urgent: bool,
    /// The producer's text does not name `actor_id`, so no rendering may.
    #[serde(default)]
    pub actor_unnamed: bool,
    pub created_at: Time,
}

/// What happened to the devices of one job run.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DeliveryReport {
    /// A push service accepted the message.
    pub delivered: usize,
    /// Registrations removed: the token is dead or its session ended.
    pub removed: usize,
    /// Refused for good; not retried.
    pub rejected: usize,
    /// Temporarily unreachable; the job will run again for these.
    pub retry: usize,
    /// Nothing to do: already delivered by an earlier attempt, or the
    /// device's channel is not configured on this server.
    pub skipped: usize,
}

pub(crate) fn job_options() -> JobOptions {
    JobOptions { concurrency: 16, lease: Duration::from_secs(60), max_deliver: MAX_ATTEMPTS }
}

/// How insistently a job's notification is delivered: urgent requests at the
/// top, then the time-critical categories, then everything else.
fn priority_of(job: &PushJob) -> PushPriority {
    if job.urgent {
        PushPriority::Urgent
    } else if categories::find(&job.category).is_some_and(|c| c.high_priority) {
        PushPriority::High
    } else {
        PushPriority::Normal
    }
}

fn delivered_set_key(job_id: Uuid) -> String {
    format!("notify:push:delivered:{job_id}")
}

impl Notify {
    /// Queues a push. Unique per job ID, so a consumer that is retried right
    /// after queuing does not push twice.
    pub(crate) async fn enqueue_push(&self, job: &PushJob) -> AppResult<()> {
        self.core.jobs.enqueue_unique(PUSH_JOB, &job.id.to_string(), job).await
    }

    /// The `notify.push` job handler: delivers, and fails the job when some
    /// device has to be tried again.
    pub async fn run_push_job(&self, job: PushJob) -> AppResult<()> {
        let report = self.deliver_push(&job).await?;
        ensure!(report.retry == 0, unavailable, "PUSH_RETRY", "{} device(s) could not be reached", report.retry);
        Ok(())
    }

    /// One delivery attempt to every device of the job's user that has not
    /// received the message yet.
    pub async fn deliver_push(&self, job: &PushJob) -> AppResult<DeliveryReport> {
        let mut report = DeliveryReport::default();
        let age = now() - job.created_at;
        if age.num_seconds() > i64::from(self.cfg.push_ttl_seconds) {
            // Stuck behind an outage for longer than a push service would
            // have kept it: showing it now would only confuse.
            tracing::warn!(job = %job.id, kind = %job.kind, "push job expired before delivery");
            return Ok(report);
        }
        if withdraw::any_withdrawn(&self.core.db, &job.refs).await?
            || withdraw::hidden_since(&self.core.db, &job.refs, job.user_id, job.created_at).await?
        {
            // Deleted, taken down or hidden from the recipient while the push waited.
            return Ok(report);
        }
        if let Some(actor_id) = job.actor_id
            && blocks::silenced(&self.core.db, job.user_id, actor_id).await?
        {
            // Blocked or muted while the push waited.
            return Ok(report);
        }
        // A comment edited while the push waited: its text now, or nothing
        // for a mention the edit took out.
        let edited;
        let job = match edits::current(&self.core.db, &job.kind, job.user_id, &job.data).await? {
            Current::Unchanged => job,
            current => {
                let mut copy = job.clone();
                if !current.apply(&mut copy.kind, &mut copy.title, &mut copy.body, &mut copy.data) {
                    return Ok(report);
                }
                edited = copy;
                &edited
            }
        };
        let devices = devices::for_delivery(&self.core, job.user_id).await?;
        if devices.is_empty() {
            return Ok(report);
        }
        let account_locale = inbox::locale_of(&self.core, job.user_id).await?;
        // Read now rather than when the push was queued: a change to what
        // pushes show applies to everything not yet sent.
        let preview = preferences::push_preview_of(&self.core, job.user_id).await?;
        // A sender-only push never shows more than the full one: an actor
        // the producer's text leaves unnamed stays unnamed.
        let actor_name = match (preview, job.actor_id) {
            (PushPreview::PUSH_PREVIEW_SENDER_ONLY, Some(actor_id)) if !job.actor_unnamed => self.actor_name(actor_id).await?,
            _ => None,
        };
        let already_delivered = self.delivered_devices(job.id).await?;

        let mut sends = Vec::new();
        for device in devices {
            if device.session_ended {
                // Signed out or revoked: this device must stay silent.
                devices::delete(&self.core, device.id).await?;
                report.removed += 1;
                continue;
            }
            if already_delivered.contains(&device.id) {
                report.skipped += 1;
                continue;
            }
            let Some(channel) = self.channels.get(&device.channel).cloned() else {
                report.skipped += 1;
                continue;
            };
            let locale = if device.locale.is_empty() { account_locale.as_str() } else { device.locale.as_str() };
            let mut message = self.message_for(job, locale);
            preview::apply(&mut message, preview, actor_name.as_deref(), &self.core.web_url(preview::INBOX_PATH));
            sends.push(async move {
                let outcome = channel.send(&device.target(), &message).await;
                (device, outcome)
            });
        }
        // A user has a handful of devices; reach them all at once.
        for (device, outcome) in futures::future::join_all(sends).await {
            self.record_outcome(job, &device, outcome, &mut report).await?;
        }
        Ok(report)
    }

    /// The name a sender-only push shows for `actor_id`: their display name,
    /// else their handle; `None` for an account that is gone.
    async fn actor_name(&self, actor_id: Uuid) -> AppResult<Option<String>> {
        let summary = tokuly_identity::summary(&self.core, actor_id).await?;
        if summary.deleted {
            return Ok(None);
        }
        let name = if summary.display_name.trim().is_empty() { summary.handle } else { summary.display_name };
        Ok(Some(name).filter(|name| !name.trim().is_empty()))
    }

    /// The job's notification written for a device that reads `locale`.
    fn message_for(&self, job: &PushJob, locale: &str) -> PushMessage {
        PushMessage {
            notification_id: job.notification_id,
            kind: job.kind.clone(),
            category: job.category.clone(),
            title: job.title.pick(locale).to_owned(),
            body: job.body.pick(locale).to_owned(),
            image_url: job.image_url.clone(),
            path: job.path.clone(),
            url: self.core.web_url(&job.path),
            locale: locale.to_owned(),
            collapse_key: job.collapse_key.clone(),
            badge: job.badge,
            data: job.data.clone(),
            priority: priority_of(job),
            ttl_seconds: self.cfg.push_ttl_seconds,
            sent_at: job.created_at,
        }
    }

    async fn record_outcome(
        &self,
        job: &PushJob,
        device: &DeliveryDevice,
        outcome: PushOutcome,
        report: &mut DeliveryReport,
    ) -> AppResult<()> {
        let db = &self.core.db;
        match outcome {
            PushOutcome::Delivered => {
                // Remember first: if the bookkeeping below fails and the job
                // is retried, the device must not get the message twice.
                self.mark_delivered(job.id, device.id).await?;
                sqlx::query(
                    "UPDATE push_devices SET last_delivered_at = now(), rejections = 0, last_error = '' WHERE id = $1",
                )
                .bind(device.id)
                .execute(db)
                .await?;
                report.delivered += 1;
            }
            PushOutcome::Gone(reason) => {
                tracing::info!(device = %device.id, %reason, "push registration is gone; removing it");
                devices::delete(&self.core, device.id).await?;
                report.removed += 1;
            }
            PushOutcome::Rejected(reason) => {
                // Not the device's fault as far as we can tell (it may be our
                // credentials), so the registration stays; the counter and the
                // message are there for whoever investigates.
                tracing::warn!(device = %device.id, kind = %job.kind, %reason, "push rejected");
                self.mark_delivered(job.id, device.id).await?;
                sqlx::query("UPDATE push_devices SET rejections = rejections + 1, last_error = $2 WHERE id = $1")
                    .bind(device.id)
                    .bind(&reason)
                    .execute(db)
                    .await?;
                report.rejected += 1;
            }
            PushOutcome::Retry(reason) => {
                tracing::warn!(device = %device.id, kind = %job.kind, %reason, "push failed; will retry");
                sqlx::query("UPDATE push_devices SET last_error = $2 WHERE id = $1")
                    .bind(device.id)
                    .bind(&reason)
                    .execute(db)
                    .await?;
                report.retry += 1;
            }
        }
        Ok(())
    }

    /// Devices an earlier attempt of this job is done with.
    async fn delivered_devices(&self, job_id: Uuid) -> AppResult<HashSet<Uuid>> {
        let members: Vec<String> = self.core.cache.conn().smembers(delivered_set_key(job_id)).await?;
        Ok(members.iter().filter_map(|m| Uuid::parse_str(m).ok()).collect())
    }

    async fn mark_delivered(&self, job_id: Uuid, device_id: Uuid) -> AppResult<()> {
        let key = delivered_set_key(job_id);
        let mut conn = self.core.cache.conn();
        conn.sadd::<_, _, ()>(&key, device_id.to_string()).await?;
        conn.expire::<_, ()>(&key, DELIVERED_SET_TTL.as_secs() as i64).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(kind_category: &str, urgent: bool) -> PushJob {
        PushJob {
            id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
            actor_id: None,
            notification_id: None,
            kind: format!("{kind_category}.something"),
            category: kind_category.to_owned(),
            title: Localized::same("t"),
            body: Localized::same("b"),
            image_url: None,
            path: "/".into(),
            collapse_key: None,
            badge: 0,
            data: serde_json::Value::Null,
            refs: Vec::new(),
            urgent,
            actor_unnamed: false,
            created_at: now(),
        }
    }

    #[test]
    fn priority_comes_from_the_request_then_the_category() {
        assert_eq!(priority_of(&job("video", false)), PushPriority::Normal);
        assert_eq!(priority_of(&job("chat", false)), PushPriority::High);
        assert_eq!(priority_of(&job("chat", true)), PushPriority::Urgent);
        assert_eq!(
            priority_of(&job("video", true)),
            PushPriority::Urgent,
            "urgency is the producer's call, whatever the category"
        );
    }

    #[test]
    fn jobs_queued_before_the_urgent_flag_existed_still_decode() {
        let mut value = serde_json::to_value(job("chat", true)).unwrap();
        value.as_object_mut().unwrap().remove("urgent");
        let decoded: PushJob = serde_json::from_value(value).unwrap();
        assert!(!decoded.urgent);
    }
}
