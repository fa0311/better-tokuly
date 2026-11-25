//! `PreferencesService`: what the signed-in user wants to be notified about.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use sqlx::PgExecutor;
use tokuly_core::{events, prelude::*};
use tokuly_proto::notify::*;

use crate::{
    Notify,
    events::{MUTE_CHANGED_TOPIC, MuteChanged},
    preferences,
};

const MAX_PAUSE_SECONDS: i32 = 30 * 24 * 3600;
const MAX_MUTES_PAGE_SIZE: i64 = 100;

/// Mutes or unmutes `muted_user_id` for `user_id` as of `at`. Used by the
/// RPCs (`at` = now) and by the consumer that mirrors the social domain's
/// friend preference (`at` = the event's time). A change older than the one
/// the row reflects is ignored, so the latest change wins whatever order
/// they are handled in. Returns whether the state flipped.
pub(crate) async fn set_mute_state<'e, E: PgExecutor<'e>>(
    executor: E,
    user_id: Uuid,
    muted_user_id: Uuid,
    muted: bool,
    at: Time,
) -> AppResult<bool> {
    // The join keeps a change about a user that does not exist from failing
    // on the foreign key (events can outlive accounts).
    let flipped: Option<bool> = sqlx::query_scalar(
        "INSERT INTO notify_mutes AS m (user_id, muted_user_id, muted, changed_at)
         SELECT a.id, b.id, $3, $4 FROM users a, users b WHERE a.id = $1 AND b.id = $2 AND a.id <> b.id
         ON CONFLICT (user_id, muted_user_id) DO UPDATE SET
            muted = EXCLUDED.muted,
            changed_at = EXCLUDED.changed_at,
            created_at = CASE WHEN EXCLUDED.muted AND NOT m.muted THEN now() ELSE m.created_at END
         WHERE m.changed_at <= EXCLUDED.changed_at
         RETURNING new.muted <> COALESCE(old.muted, false)",
    )
    .bind(user_id)
    .bind(muted_user_id)
    .bind(muted)
    .bind(at)
    .fetch_optional(executor)
    .await?;
    Ok(flipped.unwrap_or(false))
}

impl Notify {
    /// Muting and unmuting share one limit: both write a row.
    async fn mute_rate_limit(&self, user_id: Uuid) -> AppResult<()> {
        self.core.cache.rate_limit(&format!("notify-mute:{user_id}"), 120, Duration::from_secs(60)).await
    }

    /// Applies a mute or unmute requested through the API and announces it,
    /// so the friends list can show the same state.
    async fn set_mute(&self, user_id: Uuid, muted_user_id: Uuid, muted: bool) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let changed = set_mute_state(&mut *tx, user_id, muted_user_id, muted, now()).await?;
        if changed {
            events::emit(&mut *tx, MUTE_CHANGED_TOPIC, Some(user_id), &MuteChanged { user_id, muted_user_id, muted })
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

impl PreferencesService for Notify {
    async fn get_preferences(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetPreferencesRequest>,
    ) -> ServiceResult<GetPreferencesResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            ok(GetPreferencesResponse {
                preferences: preferences::load(&self.core, user_id).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_preferences(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdatePreferencesRequest>,
    ) -> ServiceResult<UpdatePreferencesResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let request = request.to_owned_message();
            // Validate everything before changing anything.
            for change in &request.categories {
                let category = preferences::require_category(&change.category)?;
                ensure!(
                    !(category.inbox_locked && change.inbox == Some(false)),
                    invalid,
                    "CATEGORY_LOCKED",
                    "{} notifications always stay in the inbox",
                    category.id
                );
            }
            let quiet_hours = request.quiet_hours.as_option();
            if let Some(quiet) = quiet_hours {
                preferences::validate_quiet_hours(quiet)?;
            }
            let push_preview = match request.push_preview.as_known() {
                Some(PushPreview::PUSH_PREVIEW_UNSPECIFIED) => None,
                Some(preview) => Some(preview as i16),
                None => bail!(invalid, "PUSH_PREVIEW_INVALID", "unknown push preview setting"),
            };

            let mut tx = self.core.db.begin().await?;
            for change in &request.categories {
                // A missing row means both toggles are on, hence the `true` defaults.
                sqlx::query(
                    "INSERT INTO notify_category_prefs (user_id, category, inbox, push)
                     VALUES ($1, $2, COALESCE($3, true), COALESCE($4, true))
                     ON CONFLICT (user_id, category) DO UPDATE SET
                        inbox = COALESCE($3, notify_category_prefs.inbox),
                        push = COALESCE($4, notify_category_prefs.push),
                        updated_at = now()",
                )
                .bind(user_id)
                .bind(&change.category)
                .bind(change.inbox)
                .bind(change.push)
                .execute(&mut *tx)
                .await?;
            }
            if let Some(quiet) = quiet_hours {
                sqlx::query(
                    "INSERT INTO notify_settings (user_id, quiet_enabled, quiet_start_minute, quiet_end_minute)
                     VALUES ($1, $2, $3, $4)
                     ON CONFLICT (user_id) DO UPDATE SET
                        quiet_enabled = EXCLUDED.quiet_enabled,
                        quiet_start_minute = EXCLUDED.quiet_start_minute,
                        quiet_end_minute = EXCLUDED.quiet_end_minute",
                )
                .bind(user_id)
                .bind(quiet.enabled)
                .bind(quiet.start_minute as i16)
                .bind(quiet.end_minute as i16)
                .execute(&mut *tx)
                .await?;
            }
            if let Some(preview) = push_preview {
                sqlx::query(
                    "INSERT INTO notify_settings (user_id, push_preview) VALUES ($1, $2)
                     ON CONFLICT (user_id) DO UPDATE SET push_preview = EXCLUDED.push_preview",
                )
                .bind(user_id)
                .bind(preview)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
            ok(UpdatePreferencesResponse {
                preferences: preferences::load(&self.core, user_id).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn pause_push(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, PausePushRequest>,
    ) -> ServiceResult<PausePushResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let seconds = request.duration_seconds;
            ensure!((0..=MAX_PAUSE_SECONDS).contains(&seconds), invalid, "PAUSE_DURATION_INVALID", "a pause lasts at most 30 days (0 = until resumed)");
            let until = (seconds > 0).then(|| now() + chrono::Duration::seconds(seconds as i64));
            sqlx::query(
                "INSERT INTO notify_settings (user_id, push_paused, push_paused_until) VALUES ($1, true, $2)
                 ON CONFLICT (user_id) DO UPDATE SET push_paused = true, push_paused_until = EXCLUDED.push_paused_until",
            )
            .bind(user_id)
            .bind(until)
            .execute(&self.core.db)
            .await?;
            ok(PausePushResponse { preferences: preferences::load(&self.core, user_id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn resume_push(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ResumePushRequest>,
    ) -> ServiceResult<ResumePushResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            sqlx::query("UPDATE notify_settings SET push_paused = false, push_paused_until = NULL WHERE user_id = $1")
                .bind(user_id)
                .execute(&self.core.db)
                .await?;
            ok(ResumePushResponse {
                preferences: preferences::load(&self.core, user_id).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn mute_user(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, MuteUserRequest>,
    ) -> ServiceResult<MuteUserResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let target_id = parse_id(request.user_id)?;
            ensure!(target_id != user_id, invalid, "CANNOT_MUTE_SELF", "you cannot mute yourself");
            self.mute_rate_limit(user_id).await?;
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL)")
                    .bind(target_id)
                    .fetch_one(&self.core.db)
                    .await?;
            ensure!(exists, not_found, "USER_NOT_FOUND", "user not found");
            self.set_mute(user_id, target_id, true).await?;
            ok(MuteUserResponse::default())
        })
        .await
    }

    async fn unmute_user(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnmuteUserRequest>,
    ) -> ServiceResult<UnmuteUserResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let target_id = parse_id(request.user_id)?;
            self.mute_rate_limit(user_id).await?;
            self.set_mute(user_id, target_id, false).await?;
            ok(UnmuteUserResponse::default())
        })
        .await
    }

    async fn list_muted_users(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListMutedUsersRequest>,
    ) -> ServiceResult<ListMutedUsersResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let page = Page::from_view(request.page.as_option(), MAX_MUTES_PAGE_SIZE);
            let after: Option<(Time, Uuid)> = page.decode()?;
            let mut rows: Vec<(Uuid, Time)> = sqlx::query_as(
                "SELECT muted_user_id, created_at FROM notify_mutes
                 WHERE user_id = $1 AND muted AND ($2::timestamptz IS NULL OR (created_at, muted_user_id) < ($2, $3))
                 ORDER BY created_at DESC, muted_user_id DESC
                 LIMIT $4",
            )
            .bind(user_id)
            .bind(after.map(|(created_at, _)| created_at))
            .bind(after.map(|(_, id)| id))
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let info = page.finish(&mut rows, |(id, created_at)| (*created_at, *id));
            let ids: Vec<Uuid> = rows.iter().map(|(id, _)| *id).collect();
            let summaries = tokuly_identity::summaries(&self.core, &ids).await?;
            let users = rows
                .into_iter()
                .map(|(id, muted_at)| MutedUser {
                    user: tokuly_identity::users::pick(&summaries, id).into(),
                    muted_at: ts(muted_at),
                    ..Default::default()
                })
                .collect();
            ok(ListMutedUsersResponse { users, page: info.into(), ..Default::default() })
        })
        .await
    }
}
