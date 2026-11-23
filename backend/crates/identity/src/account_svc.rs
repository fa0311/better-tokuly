//! `AccountService`: everything the signed-in user does to their own account.
use std::time::Duration;

use chrono::{DateTime, Utc};
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{events, prelude::*};
use tokuly_media::assets;
use tokuly_proto::{identity::*, media::AssetKind, media::AssetPurpose};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::{
    Identity,
    maintenance::{ExportJob, UserRef},
    settings, totp,
    users::{self, UserRow},
};

impl Identity {
    /// Confirms the caller knows the account password. Accounts without a
    /// password (passkey-only) pass, since a signed-in passkey session is the
    /// strongest proof they can give.
    async fn confirm_password(&self, user: &UserRow, password: &str) -> AppResult<()> {
        if user.password_hash.is_none() {
            return Ok(());
        }
        self.core
            .cache
            .rate_limit(
                &format!("confirm-pw:{}", user.id),
                10,
                Duration::from_secs(300),
            )
            .await?;
        ensure!(
            crate::password::verify(password, user.password_hash.as_deref()).await,
            unauthenticated,
            "INVALID_CREDENTIALS",
            "wrong password"
        );
        Ok(())
    }
}

/// Name under which a profile image slot holds its media asset.
pub(crate) fn profile_asset_holder(slot: &str, user_id: Uuid) -> String {
    format!("identity:{slot}:{user_id}")
}

impl AccountService for Identity {
    async fn get_me(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetMeRequest>,
    ) -> ServiceResult<GetMeResponse> {
        handle(async {
            let user = users::get(&self.core, ctx.user_id()?).await?;
            ok(GetMeResponse {
                me: self.me(&user).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_profile(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateProfileRequest>,
    ) -> ServiceResult<UpdateProfileResponse> {
        handle(async {
        let user_id = ctx.user_id()?;
        let core = &self.core;
        let current = users::get(core, user_id).await?;

        let display_name = request.display_name.map(users::validate_display_name).transpose()?;
        let bio = request.bio.map(users::validate_bio).transpose()?;
        let locale = request.locale.map(users::validate_locale).transpose()?;
        let time_zone = request.time_zone.map(users::validate_time_zone).transpose()?;
        let school_name = request.school_name.map(users::validate_school_name).transpose()?;
        let handle = match request.handle {
            Some(h) => {
                let handle = users::validate_handle(h)?;
                if handle == current.handle {
                    None
                } else {
                    core.cache.rate_limit(&format!("handle-change:{user_id}"), 3, Duration::from_secs(7 * 24 * 3600)).await?;
                    let taken: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE handle = $1::citext AND id <> $2)")
                        .bind(&handle)
                        .bind(user_id)
                        .fetch_one(&core.db)
                        .await
                        .map_err(AppError::from)?;
                    ensure!(!taken, already_exists, "HANDLE_TAKEN", "this handle is already in use");
                    Some(handle)
                }
            }
            None => None,
        };

        // `Some(None)` clears an image, `Some(Some(json))` replaces it, `None` leaves it.
        let avatar: Option<Option<serde_json::Value>> = match request.avatar_asset_id {
            None => None,
            Some("") => Some(None),
            Some(id) => Some(Some(assets::require_image_json(core, parse_id(id)?, user_id, &[AssetPurpose::ASSET_PURPOSE_AVATAR]).await?)),
        };
        let banner: Option<Option<serde_json::Value>> = match request.banner_asset_id {
            None => None,
            Some("") => Some(None),
            Some(id) => Some(Some(assets::require_image_json(core, parse_id(id)?, user_id, &[AssetPurpose::ASSET_PURPOSE_BANNER]).await?)),
        };
        let avatar_video: Option<Option<String>> = match request.avatar_video_asset_id {
            None => None,
            Some("") => Some(None),
            Some(id) => {
                let asset = assets::require_ready(core, parse_id(id)?, user_id, &[AssetPurpose::ASSET_PURPOSE_AVATAR_VIDEO]).await?;
                ensure!(asset.kind == AssetKind::ASSET_KIND_VIDEO as i32, invalid, "ASSET_NOT_VIDEO", "a video is required");
                let url = asset.video().map(|v| v.mp4_url).filter(|u| !u.is_empty()).unwrap_or(asset.url);
                Some(Some(url))
            }
        };

        let mut tx = core.db.begin().await?;
        let user: UserRow = sqlx::query_as(concat!(
            "UPDATE users SET
                display_name     = COALESCE($2, display_name),
                handle           = COALESCE($3::citext, handle),
                bio              = COALESCE($4, bio),
                locale           = COALESCE($5, locale),
                time_zone        = COALESCE($6, time_zone),
                avatar           = CASE WHEN $7 THEN $8 ELSE avatar END,
                banner           = CASE WHEN $9 THEN $10 ELSE banner END,
                avatar_video_url = CASE WHEN $11 THEN $12 ELSE avatar_video_url END,
                school_name      = COALESCE($13, school_name)
             WHERE id = $1 RETURNING ",
            users::user_columns!()
        ))
        .bind(user_id)
        .bind(display_name)
        .bind(handle)
        .bind(bio)
        .bind(locale)
        .bind(time_zone)
        .bind(avatar.is_some())
        .bind(avatar.flatten())
        .bind(banner.is_some())
        .bind(banner.flatten())
        .bind(avatar_video.is_some())
        .bind(avatar_video.flatten())
        .bind(school_name)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            let e = AppError::from(e);
            if e.is_already_exists() { AppError::already_exists("HANDLE_TAKEN", "this handle is already in use") } else { e }
        })?;
        // Keep the media assets behind the profile images alive (and release
        // the ones they replace): unheld uploads are garbage-collected.
        let slots = [
            ("avatar", request.avatar_asset_id),
            ("banner", request.banner_asset_id),
            ("avatar-video", request.avatar_video_asset_id),
        ];
        for (slot, requested) in slots {
            let Some(asset_id) = requested else { continue };
            let holder = profile_asset_holder(slot, user_id);
            assets::detach_holder(&mut *tx, &holder).await?;
            if !asset_id.is_empty() {
                assets::attach(&mut *tx, parse_id(asset_id)?, &holder).await?;
            }
        }
        events::emit(&mut *tx, "identity.user.updated", Some(user_id), &UserRef { user_id }).await?;
        tx.commit().await?;
        ok(UpdateProfileResponse { me: self.me(&user).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn set_status(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetStatusRequest>,
    ) -> ServiceResult<SetStatusResponse> {
        handle(async {
        let user_id = ctx.user_id()?;
        let emoji = request.emoji.trim();
        let text = request.text.trim();
        ensure!(emoji.graphemes(true).count() <= 2, invalid, "STATUS_INVALID", "status emoji must be one or two emoji");
        ensure!(text.graphemes(true).count() <= 60, invalid, "STATUS_INVALID", "status text must be at most 60 characters");
        ensure!((0..=30 * 24 * 3600).contains(&request.expires_in_seconds), invalid, "STATUS_INVALID", "status can last at most 30 days");
        let cleared = emoji.is_empty() && text.is_empty();
        let expires_at = (!cleared && request.expires_in_seconds > 0).then(|| Utc::now() + chrono::Duration::seconds(request.expires_in_seconds as i64));
        let mut tx = self.core.db.begin().await?;
        sqlx::query("UPDATE users SET status_emoji = $2, status_text = $3, status_set_at = $4, status_expires_at = $5 WHERE id = $1")
            .bind(user_id)
            .bind(emoji)
            .bind(text)
            .bind((!cleared).then(Utc::now))
            .bind(expires_at)
            .execute(&mut *tx)
            .await
            .map_err(AppError::from)?;
        events::emit(&mut *tx, "identity.user.status_changed", Some(user_id), &UserRef { user_id }).await?;
        tx.commit().await.map_err(AppError::from)?;
        let status = users::status_of(emoji, text, Some(Utc::now()), expires_at);
        ok(SetStatusResponse { status: status.into(), ..Default::default() })
        })
        .await
    }

    async fn get_settings(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetSettingsRequest>,
    ) -> ServiceResult<GetSettingsResponse> {
        handle(async {
            let user = users::get(&self.core, ctx.user_id()?).await?;
            ok(GetSettingsResponse {
                settings: settings::settings_from_json(&user.privacy, &user.appearance).into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_settings(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateSettingsRequest>,
    ) -> ServiceResult<UpdateSettingsResponse> {
        handle(async {
        let user_id = ctx.user_id()?;
        let req = request.to_owned_message();
        let encode = |v: &dyn Fn() -> Result<serde_json::Value, serde_json::Error>| v().map_err(|e| AppError::internal(anyhow::anyhow!("settings encode: {e}")));
        let privacy = match req.privacy.as_option() {
            Some(p) => {
                settings::validate_privacy(p)?;
                Some(encode(&|| serde_json::to_value(p))?)
            }
            None => None,
        };
        let appearance = match req.appearance.as_option() {
            Some(a) => {
                settings::validate_appearance(a)?;
                Some(encode(&|| serde_json::to_value(a))?)
            }
            None => None,
        };
        let mut tx = self.core.db.begin().await?;
        let (privacy, appearance): (serde_json::Value, serde_json::Value) = sqlx::query_as(
            "UPDATE users SET privacy = COALESCE($2, privacy), appearance = COALESCE($3, appearance) WHERE id = $1 RETURNING privacy, appearance",
        )
        .bind(user_id)
        .bind(&privacy)
        .bind(appearance)
        .fetch_one(&mut *tx)
        .await
        .map_err(AppError::from)?;
        if req.privacy.is_set() {
            // Location visibility, history retention etc. depend on these.
            events::emit(&mut *tx, "identity.user.privacy_changed", Some(user_id), &UserRef { user_id }).await?;
        }
        tx.commit().await.map_err(AppError::from)?;
        ok(UpdateSettingsResponse { settings: settings::settings_from_json(&privacy, &appearance).into(), ..Default::default() })
        })
        .await
    }

    async fn change_password(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ChangePasswordRequest>,
    ) -> ServiceResult<ChangePasswordResponse> {
        handle(async {
            let auth = ctx.auth()?.clone();
            let user = users::get(&self.core, auth.user_id).await?;
            self.confirm_password(&user, request.current_password)
                .await?;
            users::validate_password(request.new_password)?;
            let hash = crate::password::hash(request.new_password).await?;
            sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
                .bind(user.id)
                .bind(hash)
                .execute(&self.core.db)
                .await
                .map_err(AppError::from)?;
            // Other devices must sign in again with the new password.
            self.revoke_all_sessions(user.id, Some(auth.session_id))
                .await?;
            self.log_security_event(
                user.id,
                "PASSWORD_CHANGED",
                &ctx.client(),
                "",
                serde_json::json!({}),
            )
            .await;
            ok(ChangePasswordResponse::default())
        })
        .await
    }

    async fn change_email(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ChangeEmailRequest>,
    ) -> ServiceResult<ChangeEmailResponse> {
        handle(async {
            let user = users::get(&self.core, ctx.user_id()?).await?;
            self.confirm_password(&user, request.current_password)
                .await?;
            let email = users::validate_email(request.new_email)?;
            ensure!(
                user.email.as_deref() != Some(email.as_str()),
                invalid,
                "EMAIL_UNCHANGED",
                "this is already your email address"
            );
            let taken: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE email = $1::citext)")
                    .bind(&email)
                    .fetch_one(&self.core.db)
                    .await
                    .map_err(AppError::from)?;
            ensure!(
                !taken,
                already_exists,
                "EMAIL_TAKEN",
                "an account with this email already exists"
            );
            self.core
                .cache
                .rate_limit(
                    &format!("email-change:{}", user.id),
                    5,
                    Duration::from_secs(3600),
                )
                .await?;
            // The address only changes once the new one is verified.
            self.spawn_verification_mail(user.id, email, user.locale.clone(), "change");
            ok(ChangeEmailResponse::default())
        })
        .await
    }

    async fn list_sessions(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListSessionsRequest>,
    ) -> ServiceResult<ListSessionsResponse> {
        handle(async {
            let auth = ctx.auth()?;
            let sessions = Identity::list_sessions(self, auth.user_id, auth.session_id).await?;
            ok(ListSessionsResponse {
                sessions,
                ..Default::default()
            })
        })
        .await
    }

    async fn revoke_session(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RevokeSessionRequest>,
    ) -> ServiceResult<RevokeSessionResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let session_id = parse_id(request.session_id)?;
            let existed = Identity::revoke_session(self, user_id, session_id).await?;
            ensure!(existed, not_found, "SESSION_NOT_FOUND", "session not found");
            self.log_security_event(
                user_id,
                "SESSION_REVOKED",
                &ctx.client(),
                "",
                serde_json::json!({ "session_id": session_id }),
            )
            .await;
            ok(RevokeSessionResponse::default())
        })
        .await
    }

    async fn list_passkeys(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListPasskeysRequest>,
    ) -> ServiceResult<ListPasskeysResponse> {
        handle(async {
        let rows: Vec<(Uuid, String, DateTime<Utc>, Option<DateTime<Utc>>)> =
            sqlx::query_as("SELECT id, name, created_at, last_used_at FROM passkeys WHERE user_id = $1 ORDER BY id")
                .bind(ctx.user_id()?)
                .fetch_all(&self.core.db)
                .await
                .map_err(AppError::from)?;
        let passkeys = rows
            .into_iter()
            .map(|(id, name, created_at, last_used_at)| Passkey {
                id: id.to_string(),
                name,
                created_at: ts(created_at),
                last_used_at: opt_ts(last_used_at),
                ..Default::default()
            })
            .collect();
        ok(ListPasskeysResponse { passkeys, ..Default::default() })
        })
        .await
    }

    async fn delete_passkey(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeletePasskeyRequest>,
    ) -> ServiceResult<DeletePasskeyResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let user = users::get(&self.core, user_id).await?;
            let passkey_id = parse_id(request.passkey_id)?;
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM passkeys WHERE user_id = $1")
                .bind(user_id)
                .fetch_one(&self.core.db)
                .await
                .map_err(AppError::from)?;
            ensure!(
                user.password_hash.is_some() || count > 1,
                precondition,
                "LAST_CREDENTIAL",
                "set a password before removing your only passkey"
            );
            let done = sqlx::query("DELETE FROM passkeys WHERE id = $1 AND user_id = $2")
                .bind(passkey_id)
                .bind(user_id)
                .execute(&self.core.db)
                .await
                .map_err(AppError::from)?;
            ensure!(
                done.rows_affected() == 1,
                not_found,
                "PASSKEY_NOT_FOUND",
                "passkey not found"
            );
            self.log_security_event(
                user_id,
                "PASSKEY_REMOVED",
                &ctx.client(),
                "",
                serde_json::json!({}),
            )
            .await;
            ok(DeletePasskeyResponse::default())
        })
        .await
    }

    async fn begin_totp_setup(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, BeginTotpSetupRequest>,
    ) -> ServiceResult<BeginTotpSetupResponse> {
        handle(async {
            let user = users::get(&self.core, ctx.user_id()?).await?;
            ensure!(
                user.totp_enabled_at.is_none(),
                precondition,
                "TOTP_ALREADY_ENABLED",
                "two-factor authentication is already on"
            );
            let account = user.email.clone().unwrap_or_else(|| user.handle.clone());
            let (secret, otpauth_url) = totp::generate(&account)?;
            self.core
                .cache
                .set_string(
                    &format!("id:totp:setup:{}", user.id),
                    &totp::seal_secret(&self.core.keys, &secret),
                    Some(Duration::from_secs(600)),
                )
                .await?;
            ok(BeginTotpSetupResponse {
                secret,
                otpauth_url,
                ..Default::default()
            })
        })
        .await
    }

    async fn enable_totp(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, EnableTotpRequest>,
    ) -> ServiceResult<EnableTotpResponse> {
        handle(async {
        let user_id = ctx.user_id()?;
        let key = format!("id:totp:setup:{user_id}");
        let sealed = self.core.cache.get_string(&key).await?.or_not_found("TOTP_SETUP_EXPIRED", "start the setup again")?;
        let secret = totp::open_secret(&self.core.keys, &sealed).or_not_found("TOTP_SETUP_EXPIRED", "start the setup again")?;
        self.limit_totp_attempts(user_id).await?;
        ensure!(self.accept_totp(user_id, &secret, request.code).await?, invalid, "INVALID_TOTP", "wrong authentication code");
        let codes = totp::generate_recovery_codes();
        let hashes: Vec<String> = codes.iter().map(|c| totp::hash_recovery_code(&self.core.keys, c)).collect();
        sqlx::query("UPDATE users SET totp_secret = $2, totp_enabled_at = now(), recovery_code_hashes = $3 WHERE id = $1")
            .bind(user_id)
            .bind(sealed)
            .bind(hashes)
            .execute(&self.core.db)
            .await
            .map_err(AppError::from)?;
        self.core.cache.del(&key).await?;
        self.log_security_event(user_id, "TOTP_ENABLED", &ctx.client(), "", serde_json::json!({})).await;
        ok(EnableTotpResponse { recovery_codes: codes, ..Default::default() })
        })
        .await
    }

    async fn disable_totp(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DisableTotpRequest>,
    ) -> ServiceResult<DisableTotpResponse> {
        handle(async {
        let user = users::get(&self.core, ctx.user_id()?).await?;
        ensure!(user.totp_enabled_at.is_some(), precondition, "TOTP_NOT_ENABLED", "two-factor authentication is not on");
        self.limit_totp_attempts(user.id).await?;
        let secret = user.totp_secret.as_deref().and_then(|s| totp::open_secret(&self.core.keys, s));
        let code_ok = match secret.as_deref() {
            Some(secret) => self.accept_totp(user.id, secret, request.code).await?,
            None => false,
        };
        let recovery_ok = if code_ok {
            false
        } else {
            let hit: bool = sqlx::query_scalar("SELECT $2 = ANY(recovery_code_hashes) FROM users WHERE id = $1")
                .bind(user.id)
                .bind(totp::hash_recovery_code(&self.core.keys, request.code))
                .fetch_one(&self.core.db)
                .await
                .map_err(AppError::from)?;
            hit
        };
        ensure!(code_ok || recovery_ok, invalid, "INVALID_TOTP", "wrong authentication code");
        sqlx::query("UPDATE users SET totp_secret = NULL, totp_enabled_at = NULL, recovery_code_hashes = '{}' WHERE id = $1")
            .bind(user.id)
            .execute(&self.core.db)
            .await
            .map_err(AppError::from)?;
        self.log_security_event(user.id, "TOTP_DISABLED", &ctx.client(), "", serde_json::json!({})).await;
        ok(DisableTotpResponse::default())
        })
        .await
    }

    async fn list_security_events(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListSecurityEventsRequest>,
    ) -> ServiceResult<ListSecurityEventsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let page = Page::from_view(request.page.as_option(), 100);
            let before: Option<Uuid> = page.decode()?;
            let mut rows: Vec<(Uuid, String, String, String, String, DateTime<Utc>)> =
                sqlx::query_as(
                    "SELECT id, type, ip, device_name, location, created_at FROM security_events
             WHERE user_id = $1 AND ($2::uuid IS NULL OR id < $2) ORDER BY id DESC LIMIT $3",
                )
                .bind(user_id)
                .bind(before)
                .bind(page.fetch())
                .fetch_all(&self.core.db)
                .await
                .map_err(AppError::from)?;
            let info = page.finish(&mut rows, |r| r.0);
            let events = rows
                .into_iter()
                .map(
                    |(id, r#type, ip, device_name, location, created_at)| SecurityEvent {
                        id: id.to_string(),
                        r#type,
                        ip,
                        device_name,
                        location,
                        created_at: ts(created_at),
                        ..Default::default()
                    },
                )
                .collect();
            ok(ListSecurityEventsResponse {
                events,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn request_account_deletion(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RequestAccountDeletionRequest>,
    ) -> ServiceResult<RequestAccountDeletionResponse> {
        handle(async {
            let user = users::get(&self.core, ctx.user_id()?).await?;
            self.confirm_password(&user, request.current_password)
                .await?;
            let scheduled = Utc::now() + chrono::Duration::days(self.cfg.deletion_grace_days);
            let mut tx = self.core.db.begin().await?;
            sqlx::query("UPDATE users SET deletion_scheduled_at = $2 WHERE id = $1")
                .bind(user.id)
                .bind(scheduled)
                .execute(&mut *tx)
                .await
                .map_err(AppError::from)?;
            events::emit(
                &mut *tx,
                "identity.user.deletion_requested",
                Some(user.id),
                &UserRef { user_id: user.id },
            )
            .await?;
            tx.commit().await.map_err(AppError::from)?;
            let reason: String = request.reason.chars().take(500).collect();
            self.log_security_event(
                user.id,
                "DELETION_REQUESTED",
                &ctx.client(),
                "",
                serde_json::json!({ "reason": reason }),
            )
            .await;
            ok(RequestAccountDeletionResponse {
                deletion_scheduled_at: ts(scheduled),
                ..Default::default()
            })
        })
        .await
    }

    async fn cancel_account_deletion(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, CancelAccountDeletionRequest>,
    ) -> ServiceResult<CancelAccountDeletionResponse> {
        handle(async {
        let user_id = ctx.user_id()?;
        let done = sqlx::query("UPDATE users SET deletion_scheduled_at = NULL WHERE id = $1 AND deletion_scheduled_at IS NOT NULL AND deleted_at IS NULL")
            .bind(user_id)
            .execute(&self.core.db)
            .await
            .map_err(AppError::from)?;
        if done.rows_affected() == 1 {
            self.log_security_event(user_id, "DELETION_CANCELLED", &ctx.client(), "", serde_json::json!({})).await;
        }
        ok(CancelAccountDeletionResponse::default())
        })
        .await
    }

    async fn request_data_export(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, RequestDataExportRequest>,
    ) -> ServiceResult<RequestDataExportResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.core
                .cache
                .rate_limit(
                    &format!("export:{user_id}"),
                    2,
                    Duration::from_secs(24 * 3600),
                )
                .await?;
            let export_id: Uuid =
                sqlx::query_scalar("INSERT INTO data_exports (user_id) VALUES ($1) RETURNING id")
                    .bind(user_id)
                    .fetch_one(&self.core.db)
                    .await
                    .map_err(AppError::from)?;
            self.core
                .jobs
                .enqueue_unique(
                    "identity.export",
                    &export_id.to_string(),
                    &ExportJob { export_id, user_id },
                )
                .await?;
            ok(RequestDataExportResponse {
                export_id: export_id.to_string(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_connections(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListConnectionsRequest>,
    ) -> ServiceResult<ListConnectionsResponse> {
        handle(async {
            let connections = self.connections_of(ctx.user_id()?).await?;
            ok(ListConnectionsResponse {
                connections,
                available_providers: self.available_providers(),
                ..Default::default()
            })
        })
        .await
    }

    async fn begin_connection(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BeginConnectionRequest>,
    ) -> ServiceResult<BeginConnectionResponse> {
        handle(async {
            let (authorization_url, state) = self
                .begin_provider_connection(ctx.user_id()?, request.provider, request.redirect_uri)
                .await?;
            ok(BeginConnectionResponse {
                authorization_url,
                state,
                ..Default::default()
            })
        })
        .await
    }

    async fn finish_connection(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, FinishConnectionRequest>,
    ) -> ServiceResult<FinishConnectionResponse> {
        handle(async {
            let connection = self
                .finish_provider_connection(
                    ctx.user_id()?,
                    request.provider,
                    request.code,
                    request.state,
                )
                .await?;
            ok(FinishConnectionResponse {
                connection: connection.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_connection(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteConnectionRequest>,
    ) -> ServiceResult<DeleteConnectionResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            sqlx::query("DELETE FROM connections WHERE user_id = $1 AND provider = $2")
                .bind(user_id)
                .bind(request.provider)
                .execute(&self.core.db)
                .await
                .map_err(AppError::from)?;
            self.core.cache.del(&format!("id:np:{user_id}")).await?;
            ok(DeleteConnectionResponse::default())
        })
        .await
    }
}
