//! What other domains ask of this one: reacting to bans, password changes,
//! account deletions and moderation takedowns; the report subject for
//! applications; the personal data export.
use async_trait::async_trait;
use tokuly_core::{
    events::{self, Event},
    export::DataExporter,
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
    subject::{SubjectInfo, SubjectResolver},
};
use tokuly_proto::common::SubjectType;

use crate::{
    Dev,
    apps::{AppRow, app_columns},
    events::{AppEvent, ContentRemoved, UserRef},
};

/// Topics the account consumer follows.
pub const ACCOUNT_TOPICS: &[&str] = &[
    "identity.user.banned",
    "identity.user.unbanned",
    "identity.user.password_changed",
    "identity.user.deleted",
    "admin.content.removed",
];

impl Dev {
    /// Handles one of [`ACCOUNT_TOPICS`]. Events arrive at least once and
    /// not always in order, so every branch gives the same result when it
    /// runs twice or late: what a ban or a password change revokes is what
    /// existed when it happened (`occurred_at`), not what exists now.
    pub async fn on_account_event(&self, event: &Event) -> AppResult<()> {
        match event.topic.as_str() {
            "identity.user.banned" => self.on_user_banned(event.data::<UserRef>()?.user_id, event.occurred_at).await,
            "identity.user.unbanned" => self.judge_afresh(event.data::<UserRef>()?.user_id).await,
            "identity.user.password_changed" => {
                self.on_password_changed(event.data::<UserRef>()?.user_id, event.occurred_at).await
            }
            "identity.user.deleted" => self.on_user_deleted(event.data::<UserRef>()?.user_id).await,
            "admin.content.removed" => {
                let removed: ContentRemoved = event.data()?;
                if removed.subject_type == SubjectType::SUBJECT_TYPE_APP as i32 {
                    self.disable_app(removed.subject_id, &removed.reason).await?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// A suspended account loses every credential made in its name up to
    /// then: its own tokens, its bots' tokens, and what it granted to
    /// applications. (Verification refuses them the moment the ban is in
    /// the database; this makes it permanent, so lifting the ban does not
    /// revive them.)
    pub async fn on_user_banned(&self, user_id: Uuid, banned_at: Time) -> AppResult<()> {
        self.revoke_tokens_created_by(user_id, "banned", Some(banned_at)).await?;
        self.revoke_tokens_of(user_id, "banned", Some(banned_at)).await?;
        let apps: Vec<Uuid> =
            sqlx::query_scalar("SELECT app_id FROM dev_authorizations WHERE user_id = $1 AND created_at <= $2")
                .bind(user_id)
                .bind(banned_at)
                .fetch_all(&self.core.db)
                .await?;
        for app_id in apps {
            self.revoke_authorization(app_id, user_id, "banned", None).await?;
        }
        self.judge_afresh(user_id).await
    }

    /// Makes every credential that hangs on an account — the tokens of the
    /// applications it owns included — be judged against the database on
    /// its next use, instead of when the cached answer expires. Run when the
    /// account's standing changes: a suspension stops them at once, and its
    /// end lets what was not revoked work again at once.
    async fn judge_afresh(&self, user_id: Uuid) -> AppResult<()> {
        self.forget_account_tokens(user_id).await?;
        let owned: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM dev_apps WHERE owner_id = $1")
            .bind(user_id)
            .fetch_all(&self.core.db)
            .await?;
        for app_id in owned {
            self.forget_app_tokens(app_id).await?;
        }
        Ok(())
    }

    /// A changed password ends every personal and bot token its owner made
    /// before the change.
    pub async fn on_password_changed(&self, user_id: Uuid, changed_at: Time) -> AppResult<()> {
        self.revoke_tokens_created_by(user_id, "password_changed", Some(changed_at)).await?;
        Ok(())
    }

    /// Erases what this domain holds about a deleted account: its tokens,
    /// applications (with every user's authorization of them), the
    /// authorizations it gave, its webhooks and its bots.
    pub async fn on_user_deleted(&self, user_id: Uuid) -> AppResult<()> {
        self.forget_account_tokens(user_id).await?;
        let bots: Vec<Uuid> =
            sqlx::query_scalar("SELECT user_id FROM dev_bots WHERE owner_id = $1 AND deleted_at IS NULL")
                .bind(user_id)
                .fetch_all(&self.core.db)
                .await?;
        for bot_id in bots {
            self.delete_bot(bot_id, user_id, None).await?;
        }
        let apps: Vec<AppRow> =
            sqlx::query_as(concat!("SELECT ", app_columns!(), " FROM dev_apps p WHERE p.owner_id = $1"))
                .bind(user_id)
                .fetch_all(&self.core.db)
                .await?;
        for app in apps {
            self.delete_app(&app, None).await?;
        }
        let mut tx = self.core.db.begin().await?;
        if let Ok(images) = self.images() {
            // The account may have been a bot.
            images.release(&mut tx, &crate::images::bot_avatar_holder(user_id)).await?;
        }
        // Open offers to or from the account; hand-overs already made stay on record.
        sqlx::query("DELETE FROM dev_bot_transfers WHERE accepted_at IS NULL AND (from_id = $1 OR to_id = $1)")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        for table in [
            "dev_authorizations WHERE user_id",
            "dev_webhooks WHERE account_id",
            "dev_tokens WHERE user_id",
            "dev_tokens WHERE created_by",
        ] {
            sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table} = $1")))
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        }
        // If the account was a bot, who owned it stays on record for a while
        // (see `Dev::delete_bot`): that record is about the owner.
        sqlx::query("UPDATE dev_bots SET deleted_at = COALESCE(deleted_at, now()), disabled_at = COALESCE(disabled_at, now()) WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.subscribed.invalidate();
        Ok(())
    }

    /// Moderation took an application down: nothing of it works until it
    /// is restored, and its owner is told.
    pub async fn disable_app(&self, app_id: Uuid, reason: &str) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let owner: Option<(Uuid, String)> =
            sqlx::query_as("UPDATE dev_apps SET status = 'disabled', disabled_reason = $2 WHERE id = $1 AND status = 'active' RETURNING owner_id, name")
                .bind(app_id)
                .bind(reason)
                .fetch_optional(&mut *tx)
                .await?;
        let Some((owner_id, name)) = owner else { return Ok(()) };
        let notice = NotificationRequest::new(
            owner_id,
            "dev.app_disabled",
            Localized::new("アプリが停止されました", "Your application was disabled"),
            Localized::new(
                format!("「{name}」は利用規約違反の報告により停止されました。トークンはすべて無効になっています。"),
                format!("\"{name}\" was disabled after a moderation review. All of its tokens have stopped working."),
            ),
            format!("/settings/developer/apps/{app_id}"),
        )
        .subject(SubjectType::SUBJECT_TYPE_APP, app_id);
        outbound::notify(&mut *tx, &notice).await?;
        events::emit(&mut *tx, "dev.app.disabled", None, &AppEvent { app_id, owner_id }).await?;
        tx.commit().await?;
        self.forget_app_tokens(app_id).await
    }
}

/// Applications as something people can report.
pub(crate) struct AppSubjects(pub Dev);

#[async_trait]
impl SubjectResolver for AppSubjects {
    async fn resolve(&self, id: &str, _viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let Ok(app_id) = Uuid::parse_str(id) else { return Ok(None) };
        // A disabled application is gone as far as the public is concerned.
        let Some(app) = self.0.find_app(app_id).await?.filter(AppRow::is_usable) else { return Ok(None) };
        let image_url = app
            .icon
            .as_ref()
            .and_then(|icon| icon.get("url"))
            .and_then(|url| url.as_str())
            .filter(|url| !url.is_empty())
            .map(str::to_owned);
        Ok(Some(SubjectInfo {
            id: app.id,
            owner_id: app.owner_id,
            title: app.name,
            path: format!("/apps/{}", app.id),
            image_url,
            interactions_enabled: false,
        }))
    }
}

/// This domain's part of a personal data export. Never includes secrets,
/// not even hashed.
pub(crate) struct DevExporter(pub Dev);

#[async_trait]
impl DataExporter for DevExporter {
    fn name(&self) -> &'static str {
        "dev"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        let db = &self.0.core.db;
        let section = |sql: &'static str| async move {
            let rows: Vec<serde_json::Value> = sqlx::query_scalar(sql).bind(user_id).fetch_all(db).await?;
            AppResult::Ok(serde_json::Value::Array(rows))
        };
        Ok(serde_json::json!({
            "tokens": section(
                "SELECT jsonb_build_object('id', id, 'kind', kind, 'acts_as', user_id, 'name', name, 'scopes', scopes, 'created_at', created_at, 'expires_at', expires_at,
                        'last_used_at', last_used_at, 'last_used_ip', last_used_ip, 'revoked_at', revoked_at)
                 FROM dev_tokens WHERE created_by = $1 ORDER BY id"
            ).await?,
            "applications": section(
                "SELECT jsonb_build_object('id', id, 'name', name, 'description', description, 'homepage_url', homepage_url, 'privacy_policy_url', privacy_policy_url,
                        'redirect_uris', redirect_uris, 'scopes', scopes, 'confidential', confidential, 'status', status, 'created_at', created_at)
                 FROM dev_apps WHERE owner_id = $1 ORDER BY id"
            ).await?,
            "authorized_applications": section(
                "SELECT jsonb_build_object('app_id', a.app_id, 'app_name', p.name, 'scopes', a.scopes, 'authorized_at', a.created_at, 'last_used_at', a.last_used_at)
                 FROM dev_authorizations a JOIN dev_apps p ON p.id = a.app_id WHERE a.user_id = $1 ORDER BY a.id"
            ).await?,
            "bots": section(
                "SELECT jsonb_build_object('id', b.user_id, 'handle', u.handle::text, 'display_name', u.display_name, 'disabled_at', b.disabled_at, 'created_at', b.created_at,
                        'deleted_at', b.deleted_at)
                 FROM dev_bots b JOIN users u ON u.id = b.user_id WHERE b.owner_id = $1 ORDER BY b.created_at"
            ).await?,
            "webhooks": section(
                "SELECT jsonb_build_object('id', w.id, 'account_id', w.account_id, 'url', w.url, 'description', w.description, 'event_types', w.event_types, 'status', w.status,
                        'created_at', w.created_at)
                 FROM dev_webhooks w
                 WHERE w.account_id = $1 OR w.account_id IN (SELECT user_id FROM dev_bots WHERE owner_id = $1 AND deleted_at IS NULL) ORDER BY w.id"
            ).await?,
        }))
    }
}
