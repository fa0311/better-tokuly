//! Background work and event payloads of the identity domain.
//!
//! Events emitted (see `docs/events.md`):
//! - `identity.user.created` — [`UserCreated`]
//! - `identity.user.birthdate_given` — [`BirthdateGiven`]
//! - `identity.profile.updated` — [`crate::extras::ProfileUpdated`]
//! - `identity.user.updated`, `identity.user.status_changed`,
//!   `identity.user.privacy_changed`, `identity.user.deletion_requested`,
//!   `identity.user.email_verified`, `identity.user.deleted` — [`UserRef`]
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokuly_core::{
    dead_letters,
    events::{self, ConsumeOptions, Event},
    jobs::JobOptions,
    prelude::*,
    storage::Bucket,
};

use crate::Identity;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserCreated {
    pub user_id: Uuid,
    pub handle: String,
    /// Invite code entered at sign-up; empty when none.
    pub invite_code: String,
    /// Under 18 on the day of sign-up: start the account with the defaults
    /// for minors (`crate::age`). Absent in events from before the field.
    #[serde(default)]
    pub minor: bool,
}

/// An account made before sign-up asked for a birthdate has given one. A
/// domain that starts new minors with defaults of its own applies them now,
/// by `minor` (the age on that day), as it would have at sign-up.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BirthdateGiven {
    pub user_id: Uuid,
    pub minor: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRef {
    pub user_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportJob {
    pub export_id: Uuid,
    pub user_id: Uuid,
}

/// Topics identity's own consumer follows.
pub const ACCOUNT_TOPICS: &[&str] = &["identity.user.banned", "identity.user.deleted"];

/// How long after an erasure the hourly sweep still looks for dead letters
/// about the account: a message in flight at the erasure is retried for up
/// to a couple of hours before it is buried.
const LATE_LETTERS: &str = "1 day";

impl Identity {
    /// Handles one of [`ACCOUNT_TOPICS`]; the same result when an event
    /// arrives twice or late.
    pub async fn on_account_event(&self, event: &Event) -> AppResult<()> {
        match event.topic.as_str() {
            "identity.user.banned" => {
                self.sign_out_banned(event.data::<UserRef>()?.user_id, event.occurred_at).await?;
                Ok(())
            }
            "identity.user.deleted" => {
                dead_letters::erase_users(&self.core.bus, &[event.data::<UserRef>()?.user_id]).await?;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Starts the identity workers: the data-export job, the consumer that
    /// signs banned accounts out and removes erased accounts' dead letters
    /// ([`Identity::on_account_event`]), the profile link check (job and
    /// six-hourly sweep) and the hourly purge of accounts whose deletion
    /// grace period ran out ([`Identity::erase_late_dead_letters`] with it).
    pub fn spawn_workers(&self) {
        let identity = self.clone();
        let (bus, cancel) = (self.core.bus.clone(), self.core.shutdown.clone());
        tokio::spawn(async move {
            events::consume(
                bus,
                "identity-sessions",
                ACCOUNT_TOPICS,
                ConsumeOptions::default(),
                cancel,
                move |event| {
                    let identity = identity.clone();
                    async move { identity.on_account_event(&event).await }
                },
            )
            .await;
        });

        let identity = self.clone();
        tokio::spawn(async move {
            let handler_identity = identity.clone();
            identity
                .core
                .jobs
                .run(
                    "identity.export",
                    JobOptions { concurrency: 2, lease: Duration::from_secs(120), max_deliver: 3 },
                    identity.core.shutdown.clone(),
                    move |job: ExportJob| {
                        let identity = handler_identity.clone();
                        async move { identity.run_export(job).await }
                    },
                )
                .await;
        });

        let identity = self.clone();
        tokio::spawn(async move {
            let handler_identity = identity.clone();
            identity
                .core
                .jobs
                .run(
                    crate::extras::LINK_CHECK_JOB,
                    JobOptions { concurrency: 4, lease: Duration::from_secs(120), max_deliver: 3 },
                    identity.core.shutdown.clone(),
                    move |job: crate::link_check::LinkCheckJob| {
                        let identity = handler_identity.clone();
                        async move { identity.check_profile_links(job.user_id).await.map(|_| ()) }
                    },
                )
                .await;
        });

        let identity = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(crate::link_check::SWEEP_INTERVAL);
            loop {
                tokio::select! {
                    _ = identity.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        if let Err(e) = identity.sweep_profile_links().await {
                            tracing::error!(error = %e, "profile link sweep failed");
                        }
                    }
                }
            }
        });

        let identity = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(3600));
            loop {
                tokio::select! {
                    _ = identity.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        match identity.purge_deleted_accounts().await {
                            Ok(0) => {}
                            Ok(n) => tracing::info!(count = n, "purged deleted accounts"),
                            Err(e) => tracing::error!(error = %e, "account purge failed"),
                        }
                        if let Err(e) = identity.erase_late_dead_letters().await {
                            tracing::error!(error = %e, "dead letter erasure failed");
                        }
                    }
                }
            }
        });
    }

    /// Erases accounts whose deletion grace period has passed. The row stays
    /// as an anonymous tombstone so references from other users' content
    /// (messages, comments) keep resolving to "deleted user"; every domain
    /// removes its own data on `identity.user.deleted`.
    pub async fn purge_deleted_accounts(&self) -> AppResult<u64> {
        let due: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM users WHERE deletion_scheduled_at <= now() AND deleted_at IS NULL ORDER BY deletion_scheduled_at LIMIT 100",
        )
        .fetch_all(&self.core.db)
        .await?;
        let mut purged = 0;
        for user_id in due {
            let mut tx = self.core.db.begin().await?;
            let handle: Option<String> = sqlx::query_scalar("SELECT handle::text FROM users WHERE id = $1 FOR UPDATE")
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await?;
            let done = sqlx::query(
                "UPDATE users SET
                    deleted_at = now(), handle = ('deleted-' || replace(id::text, '-', ''))::citext, display_name = '',
                    email = NULL, email_verified_at = NULL, password_hash = NULL, bio = '', avatar = NULL,
                    avatar_video_url = NULL, banner = NULL, birthdate = NULL, roles = '{}', verified = false, school_name = '',
                    status_emoji = '', status_text = '', status_set_at = NULL, status_expires_at = NULL,
                    totp_secret = NULL, totp_enabled_at = NULL, recovery_code_hashes = '{}',
                    privacy = '{}', appearance = '{}', invited_by = NULL, attrs = '{}'
                 WHERE id = $1 AND deleted_at IS NULL AND deletion_scheduled_at <= now()",
            )
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
            if done.rows_affected() == 0 {
                continue; // cancelled in the meantime
            }
            let sessions: Vec<Uuid> = sqlx::query_scalar("DELETE FROM sessions WHERE user_id = $1 RETURNING id")
                .bind(user_id)
                .fetch_all(&mut *tx)
                .await?;
            for table in ["passkeys", "email_tokens", "security_events", "connections", "data_exports"] {
                sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table} WHERE user_id = $1")))
                    .bind(user_id)
                    .execute(&mut *tx)
                    .await?;
            }
            crate::extras::erase(&mut tx, user_id).await?;
            for slot in ["avatar", "banner", "avatar-video"] {
                tokuly_media::assets::detach_holder(&mut *tx, &crate::account_svc::profile_asset_holder(slot, user_id))
                    .await?;
            }
            sqlx::query("DELETE FROM profile_views WHERE viewed_id = $1 OR viewer_id = $1")
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
            events::emit(&mut *tx, "identity.user.deleted", Some(user_id), &UserRef { user_id }).await?;
            if let Some(handle) = &handle {
                // Nobody gets to answer to the departed account's handle right away.
                crate::users::hold_handle(&self.core, handle, user_id).await?;
            }
            tx.commit().await?;
            // Devices still open are signed out now, not when their access
            // token runs out: it would keep writing to the erased account.
            for session_id in sessions {
                self.core.revoke_session(session_id).await;
            }
            purged += 1;
        }
        Ok(purged)
    }

    /// Removes dead letters about accounts erased in the last day: those
    /// buried after the `identity.user.deleted` consumer looked.
    pub async fn erase_late_dead_letters(&self) -> AppResult<usize> {
        let erased: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE deleted_at > now() - $1::interval")
            .bind(LATE_LETTERS)
            .fetch_all(&self.core.db)
            .await?;
        dead_letters::erase_users(&self.core.bus, &erased).await
    }

    /// Builds the personal data archive and mails a download link.
    async fn run_export(&self, job: ExportJob) -> AppResult<()> {
        let core = &self.core;
        let result: AppResult<(String, i64)> = async {
            let user = crate::users::get(core, job.user_id).await?;
            let me = self.me(&user).await?;
            let sessions = self.list_sessions(job.user_id, Uuid::nil()).await?;
            let settings = crate::settings::settings_from_json(&user.privacy, &user.appearance, me.minor);
            let mut sections = serde_json::Map::new();
            let to_json =
                |v: &dyn Fn() -> Result<serde_json::Value, serde_json::Error>| v().unwrap_or(serde_json::Value::Null);
            sections.insert("account".into(), to_json(&|| serde_json::to_value(&me)));
            sections.insert("settings".into(), to_json(&|| serde_json::to_value(&settings)));
            sections.insert("sessions".into(), to_json(&|| serde_json::to_value(&sessions)));
            for exporter in core.exporters.all() {
                match exporter.export(job.user_id).await {
                    Ok(data) => {
                        sections.insert(exporter.name().to_owned(), data);
                    }
                    Err(e) => {
                        tracing::warn!(section = exporter.name(), error = %e, "export section failed");
                        sections.insert(
                            exporter.name().to_owned(),
                            serde_json::json!({ "error": "export of this section failed" }),
                        );
                    }
                }
            }
            let archive = serde_json::json!({
                "format": "tokuly-export/1",
                "exported_at": chrono::Utc::now().to_rfc3339(),
                "user_id": job.user_id,
                "data": sections,
            });
            let bytes = serde_json::to_vec_pretty(&archive)
                .map_err(|e| AppError::internal(anyhow::anyhow!("export encode: {e}")))?;
            let size = bytes.len() as i64;
            let key = format!("exports/{}/{}.json", job.user_id, job.export_id);
            core.storage.put(Bucket::Private, &key, bytes, "application/json").await?;
            if let Some(email) = &user.email {
                let url = core.storage.presign_get(Bucket::Private, &key, Duration::from_secs(7 * 24 * 3600)).await?;
                self.send_data_export_mail(email, &user.locale, &url).await?;
            }
            Ok((key, size))
        }
        .await;
        match result {
            Ok((key, size)) => {
                sqlx::query("UPDATE data_exports SET status = 'ready', object_key = $2, size_bytes = $3, completed_at = now() WHERE id = $1")
                    .bind(job.export_id)
                    .bind(&key)
                    .bind(size)
                    .execute(&core.db)
                    .await?;
                core.emit(
                    "identity.export.ready",
                    Some(job.user_id),
                    &serde_json::json!({ "user_id": job.user_id, "export_id": job.export_id }),
                )
                .await?;
                Ok(())
            }
            Err(e) => {
                sqlx::query("UPDATE data_exports SET status = 'failed', completed_at = now() WHERE id = $1")
                    .bind(job.export_id)
                    .execute(&core.db)
                    .await?;
                Err(e)
            }
        }
    }
}
