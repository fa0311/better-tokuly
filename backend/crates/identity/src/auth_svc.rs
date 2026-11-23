//! `AuthService`: sign-up, sign-in, token refresh, passkeys, email flows and
//! device login.
use std::time::Duration;

use chrono::{DateTime, Utc};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use serde::{Deserialize, Serialize};
use tokuly_core::{
    analytics::AnalyticsEvent, auth::ClientMeta, events, keys::random_token, prelude::*,
};
use tokuly_proto::identity::*;

use crate::{
    Identity,
    sessions::{Issued, REFRESH_COOKIE, cookie_value},
    totp,
    users::{self, UserRow},
};

const VERIFY_TTL_HOURS: i64 = 24;
const RESET_TTL_MINUTES: i64 = 60;
const DEVICE_LOGIN_TTL: Duration = Duration::from_secs(600);
const DEVICE_POLL_INTERVAL_SECONDS: i32 = 3;

/// Pending device login, stored in Valkey under the hash of the device code.
#[derive(Serialize, Deserialize)]
struct DeviceLogin {
    user_code: String,
    platform: i32,
    device_name: String,
    app_version: String,
    device_id: String,
    /// "pending" | "approved" | "denied"
    status: String,
    user_id: Option<Uuid>,
    expires_at: DateTime<Utc>,
}

impl DeviceLogin {
    fn client(&self) -> ClientInfo {
        ClientInfo {
            platform: self.platform.into(),
            device_name: self.device_name.clone(),
            app_version: self.app_version.clone(),
            device_id: self.device_id.clone(),
            ..Default::default()
        }
    }

    fn status(&self) -> DeviceLoginStatus {
        match self.status.as_str() {
            "approved" => DeviceLoginStatus::DEVICE_LOGIN_STATUS_APPROVED,
            "denied" => DeviceLoginStatus::DEVICE_LOGIN_STATUS_DENIED,
            _ => DeviceLoginStatus::DEVICE_LOGIN_STATUS_PENDING,
        }
    }
}

/// `ABCD-2345`: uppercase without look-alike characters.
fn new_user_code() -> String {
    use rand::RngExt as _;
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut rng = rand::rng();
    let mut pick = || ALPHABET[rng.random_range(0..ALPHABET.len())] as char;
    let mut code = String::with_capacity(9);
    for i in 0..8 {
        if i == 4 {
            code.push('-');
        }
        code.push(pick());
    }
    code
}

fn normalize_user_code(input: &str) -> String {
    let compact: String = input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if compact.len() == 8 {
        format!("{}-{}", &compact[..4], &compact[4..])
    } else {
        compact
    }
}

impl Identity {
    /// Builds the response for a successful authentication. Web clients get
    /// the refresh token as an HttpOnly cookie instead of in the body.
    async fn auth_response(
        &self,
        issued: Issued,
        user: &UserRow,
        meta: &ClientMeta,
    ) -> AppResult<Response<AuthResponse>> {
        let body = AuthResponse {
            access_token: issued.access_token,
            refresh_token: if meta.is_web {
                String::new()
            } else {
                issued.refresh_token.clone()
            },
            access_token_expires_at: ts(issued.access_expires_at),
            me: self.me(user).await?.into(),
            session_id: issued.session_id.to_string(),
            ..Default::default()
        };
        let response = Response::new(body);
        Ok(if meta.is_web {
            response.with_header("set-cookie", self.refresh_cookie(&issued.refresh_token))
        } else {
            response
        })
    }

    async fn create_email_token(
        &self,
        user_id: Uuid,
        purpose: &str,
        email: &str,
        ttl: chrono::Duration,
    ) -> AppResult<String> {
        let token = random_token(32);
        // Only the newest token of a kind stays usable.
        sqlx::query("DELETE FROM email_tokens WHERE user_id = $1 AND purpose = $2")
            .bind(user_id)
            .bind(purpose)
            .execute(&self.core.db)
            .await?;
        sqlx::query("INSERT INTO email_tokens (token_hash, user_id, purpose, email, expires_at) VALUES ($1, $2, $3, $4::citext, $5)")
            .bind(self.core.keys.token_hash(&token))
            .bind(user_id)
            .bind(purpose)
            .bind(email)
            .bind(Utc::now() + ttl)
            .execute(&self.core.db)
            .await?;
        Ok(token)
    }

    /// Invalidates the mailed links of a user that change the address or the
    /// password. Called when the password changes: a link someone requested
    /// while they knew the old password must not outlive it.
    pub(crate) async fn drop_account_links(&self, user_id: Uuid) -> AppResult<()> {
        sqlx::query("DELETE FROM email_tokens WHERE user_id = $1 AND purpose IN ('change', 'reset')")
            .bind(user_id)
            .execute(&self.core.db)
            .await?;
        Ok(())
    }

    /// Marks an email token used and returns `(user_id, purpose, email)`.
    async fn consume_email_token(
        &self,
        token: &str,
        purposes: &[&str],
    ) -> AppResult<(Uuid, String, String)> {
        let row: Option<(Uuid, String, String)> = sqlx::query_as(
            "UPDATE email_tokens SET used_at = now()
             WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now() AND purpose = ANY($2)
             RETURNING user_id, purpose, email::text",
        )
        .bind(self.core.keys.token_hash(token))
        .bind(purposes.iter().map(|p| p.to_string()).collect::<Vec<_>>())
        .fetch_optional(&self.core.db)
        .await?;
        row.ok_or_else(|| AppError::invalid("TOKEN_INVALID", "this link is invalid or has expired"))
    }

    /// Sends (or re-sends) the address verification mail without blocking the caller.
    pub(crate) fn spawn_verification_mail(
        &self,
        user_id: Uuid,
        email: String,
        locale: String,
        purpose: &'static str,
    ) {
        let identity = self.clone();
        tokio::spawn(async move {
            let result = async {
                let token = identity
                    .create_email_token(
                        user_id,
                        purpose,
                        &email,
                        chrono::Duration::hours(VERIFY_TTL_HOURS),
                    )
                    .await?;
                identity
                    .send_verification_mail(&email, &locale, &token, purpose == "change")
                    .await
            }
            .await;
            if let Err(e) = result {
                tracing::warn!(%user_id, error = %e, "verification mail failed");
            }
        });
    }

    /// The pending login behind a user code that `user_id` typed in. Lookups
    /// are limited per account (20 per 10 minutes): approving a code someone
    /// else is waiting on would sign their device into this account.
    async fn device_login_by_user_code(&self, user_id: Uuid, user_code: &str) -> AppResult<(String, DeviceLogin)> {
        self.core
            .cache
            .rate_limit(&format!("devcode:user:{user_id}"), 20, Duration::from_secs(600))
            .await?;
        let code = normalize_user_code(user_code);
        let not_found =
            || AppError::not_found("DEVICE_CODE_INVALID", "this code is invalid or has expired");
        let key = self
            .core
            .cache
            .get_string(&format!("id:devcode:{code}"))
            .await?
            .ok_or_else(not_found)?;
        let login: DeviceLogin = self
            .core
            .cache
            .get_json(&format!("id:devlogin:{key}"))
            .await?
            .ok_or_else(not_found)?;
        Ok((key, login))
    }
}

impl AuthService for Identity {
    async fn sign_up(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SignUpRequest>,
    ) -> ServiceResult<AuthResponse> {
        handle(async {
        let req = request.to_owned_message();
        let meta = ctx.client();
        let core = &self.core;
        ensure!(self.cfg.registration_open, denied, "REGISTRATION_CLOSED", "sign-ups are currently closed");
        core.cache.rate_limit(&format!("signup:{}", meta.ip), 10, Duration::from_secs(3600)).await?;
        ensure!(req.accept_terms, invalid, "TERMS_REQUIRED", "you must accept the terms of service");

        let email = users::validate_email(&req.email)?;
        users::validate_password(&req.password)?;
        let handle = users::handle_error(self.claimable_handle(&req.handle))?;
        let display_name = users::validate_display_name(&req.display_name)?;
        let birthdate = users::validate_birthdate(&req.birthdate, self.cfg.min_age)?;
        let client = req.client.as_option();
        let locale = client
            .map(|c| c.locale.as_str())
            .filter(|l| !l.is_empty())
            .or(Some(meta.language.as_str()).filter(|l| !l.is_empty()))
            .and_then(|l| users::validate_locale(l).ok())
            .unwrap_or_else(|| "ja".to_owned());
        let time_zone = client
            .and_then(|c| users::validate_time_zone(&c.time_zone).ok())
            .unwrap_or_else(|| "Asia/Tokyo".to_owned());

        let (handle_taken, email_taken): (bool, bool) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM users WHERE handle = $1::citext), EXISTS(SELECT 1 FROM users WHERE email = $2::citext)",
        )
        .bind(&handle)
        .bind(&email)
        .fetch_one(&core.db)
        .await?;
        ensure!(!handle_taken, already_exists, "HANDLE_TAKEN", "this handle is already in use");
        let handle_held = users::handle_held_from(core, &handle, None).await?;
        ensure!(!handle_held, already_exists, "HANDLE_TAKEN", "this handle is already in use");
        ensure!(!email_taken, already_exists, "EMAIL_TAKEN", "an account with this email already exists");

        let password_hash = crate::password::hash(&req.password).await?;
        let invite_code = req.invite_code.trim().to_owned();
        let mut tx = core.db.begin().await?;
        let user: UserRow = sqlx::query_as(concat!(
            "INSERT INTO users (handle, display_name, email, password_hash, birthdate, locale, time_zone, attrs)
             VALUES ($1::citext, $2, $3::citext, $4, $5, $6, $7, $8) RETURNING ",
            users::user_columns!()
        ))
        .bind(&handle)
        .bind(&display_name)
        .bind(&email)
        .bind(&password_hash)
        .bind(birthdate)
        .bind(&locale)
        .bind(&time_zone)
        .bind(serde_json::json!({ "invite_code": invite_code }))
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            let e = AppError::from(e);
            if e.is_already_exists() { AppError::already_exists("ACCOUNT_EXISTS", "this handle or email is already in use") } else { e }
        })?;
        events::emit(
            &mut *tx,
            "identity.user.created",
            Some(user.id),
            &crate::maintenance::UserCreated { user_id: user.id, handle: handle.clone(), invite_code },
        )
        .await?;
        tx.commit().await?;

        self.spawn_verification_mail(user.id, email, locale, "verify");
        let issued = self.create_session(&user, client, &meta).await?;
        self.log_security_event(user.id, "SIGN_UP", &meta, "", serde_json::json!({})).await;
        core.analytics.track(AnalyticsEvent::new("user.signup").user(Some(user.id))).await;
        self.auth_response(issued, &user, &meta).await
        })
        .await
    }

    async fn sign_in(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SignInRequest>,
    ) -> ServiceResult<AuthResponse> {
        handle(async {
            let req = request.to_owned_message();
            let meta = ctx.client();
            let core = &self.core;
            let identifier = users::normalize_identifier(&req.identifier);
            ensure!(
                !identifier.is_empty() && !req.password.is_empty(),
                invalid,
                "INVALID_CREDENTIALS",
                "enter your email or handle and password"
            );
            core.cache
                .rate_limit(
                    &format!("signin:ip:{}", meta.ip),
                    30,
                    Duration::from_secs(300),
                )
                .await?;
            core.cache
                .rate_limit(
                    &format!("signin:id:{identifier}"),
                    10,
                    Duration::from_secs(300),
                )
                .await?;

            let user = users::find_by_identifier(core, &identifier).await?;
            if let Some(user) = &user {
                // However the account was named (handle, address), one budget.
                core.cache
                    .rate_limit(&format!("signin:user:{}", user.id), 10, Duration::from_secs(300))
                    .await?;
            }
            let password_ok = crate::password::verify(
                &req.password,
                user.as_ref().and_then(|u| u.password_hash.as_deref()),
            )
            .await;
            let Some(user) = user.filter(|_| password_ok) else {
                if let Ok(Some(existing)) = users::find_by_identifier(core, &identifier).await {
                    self.log_security_event(
                        existing.id,
                        "SIGN_IN_FAILED",
                        &meta,
                        "",
                        serde_json::json!({}),
                    )
                    .await;
                }
                bail!(
                    unauthenticated,
                    "INVALID_CREDENTIALS",
                    "wrong email, handle or password"
                );
            };
            ensure!(
                user.banned_at.is_none(),
                denied,
                "ACCOUNT_BANNED",
                "this account has been suspended"
            );

            if user.totp_enabled_at.is_some() {
                if req.totp_code.trim().is_empty() {
                    return ok(AuthResponse {
                        mfa_required: true,
                        ..Default::default()
                    });
                }
                self.limit_totp_attempts(user.id).await?;
                let secret = user
                    .totp_secret
                    .as_deref()
                    .and_then(|s| totp::open_secret(&core.keys, s));
                let code_ok = match secret.as_deref() {
                    Some(secret) => self.accept_totp(user.id, secret, &req.totp_code).await?,
                    None => false,
                };
                if !code_ok {
                    // A recovery code works once.
                    let used: Option<Uuid> = sqlx::query_scalar(
                    "UPDATE users SET recovery_code_hashes = array_remove(recovery_code_hashes, $2)
                     WHERE id = $1 AND $2 = ANY(recovery_code_hashes) RETURNING id",
                )
                .bind(user.id)
                .bind(totp::hash_recovery_code(&core.keys, &req.totp_code))
                .fetch_optional(&core.db)
                .await?;
                    if used.is_none() {
                        self.log_security_event(
                            user.id,
                            "SIGN_IN_FAILED",
                            &meta,
                            "",
                            serde_json::json!({ "factor": "totp" }),
                        )
                        .await;
                        bail!(unauthenticated, "INVALID_TOTP", "wrong authentication code");
                    }
                    self.log_security_event(
                        user.id,
                        "RECOVERY_CODE_USED",
                        &meta,
                        "",
                        serde_json::json!({}),
                    )
                    .await;
                }
            }

            let issued = self
                .create_session(&user, req.client.as_option(), &meta)
                .await?;
            self.log_security_event(
                user.id,
                "SIGN_IN",
                &meta,
                req.client
                    .as_option()
                    .map(|c| c.device_name.as_str())
                    .unwrap_or(""),
                serde_json::json!({ "method": "password" }),
            )
            .await;
            self.auth_response(issued, &user, &meta).await
        })
        .await
    }

    async fn refresh_token(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RefreshTokenRequest>,
    ) -> ServiceResult<AuthResponse> {
        let meta = ctx.client();
        let presented = if request.refresh_token.is_empty() {
            cookie_value(&meta.cookies, REFRESH_COOKIE)
                .unwrap_or_default()
                .to_owned()
        } else {
            request.refresh_token.to_owned()
        };
        match self.refresh_session(&presented, &meta).await {
            Ok((issued, user)) => Ok(self.auth_response(issued, &user, &meta).await?),
            Err(e) => {
                let mut err = connectrpc::ConnectError::from(e);
                if meta.is_web {
                    // Drop a dead cookie so the client stops presenting it.
                    if let Ok(value) = http::HeaderValue::from_str(&self.clear_refresh_cookie()) {
                        err.response_headers_mut().insert("set-cookie", value);
                    }
                }
                Err(err)
            }
        }
    }

    async fn sign_out(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SignOutRequest>,
    ) -> ServiceResult<SignOutResponse> {
        handle(async {
        let meta = ctx.client();
        // Signing out must work even when the access token already expired.
        if let Ok(Some(auth)) = ctx.maybe_auth() {
            if request.all_sessions {
                self.revoke_all_sessions(auth.user_id, None).await?;
            } else {
                self.revoke_session(auth.user_id, auth.session_id).await?;
            }
            self.log_security_event(auth.user_id, "SIGN_OUT", &meta, "", serde_json::json!({ "all": request.all_sessions })).await;
        } else if let Some(token) = cookie_value(&meta.cookies, REFRESH_COOKIE) {
            let row: Option<(Uuid, Uuid)> = sqlx::query_as("SELECT id, user_id FROM sessions WHERE refresh_hash = $1 AND revoked_at IS NULL")
                .bind(self.core.keys.token_hash(token))
                .fetch_optional(&self.core.db)
                .await?;
            if let Some((session_id, user_id)) = row {
                self.revoke_session(user_id, session_id).await?;
            }
        }
        let response = Response::new(SignOutResponse::default());
        Ok(if meta.is_web { response.with_header("set-cookie", self.clear_refresh_cookie()) } else { response })
        })
        .await
    }

    async fn check_handle(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, CheckHandleRequest>,
    ) -> ServiceResult<CheckHandleResponse> {
        handle(async {
            let normalized = match self.claimable_handle(request.handle) {
                Ok(h) => h,
                Err(reason) => {
                    return ok(CheckHandleResponse {
                        available: false,
                        reason: reason.to_owned(),
                        ..Default::default()
                    });
                }
            };
            let taken: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE handle = $1::citext)")
                    .bind(&normalized)
                    .fetch_one(&self.core.db)
                    .await
                    .map_err(AppError::from)?
                    || users::handle_held_from(&self.core, &normalized, None).await?;
            ok(CheckHandleResponse {
                available: !taken,
                normalized,
                reason: if taken {
                    "TAKEN".to_owned()
                } else {
                    String::new()
                },
                ..Default::default()
            })
        })
        .await
    }

    async fn begin_passkey_registration(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BeginPasskeyRegistrationRequest>,
    ) -> ServiceResult<BeginPasskeyRegistrationResponse> {
        handle(async {
            let user = users::get(&self.core, ctx.user_id()?).await?;
            self.confirm_password(&user, request.current_password).await?;
            let (challenge_id, options_json) =
                Identity::begin_passkey_registration(self, &user, request.name).await?;
            ok(BeginPasskeyRegistrationResponse {
                challenge_id,
                options_json,
                ..Default::default()
            })
        })
        .await
    }

    async fn finish_passkey_registration(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, FinishPasskeyRegistrationRequest>,
    ) -> ServiceResult<FinishPasskeyRegistrationResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let passkey = Identity::finish_passkey_registration(
                self,
                user_id,
                request.challenge_id,
                request.credential_json,
            )
            .await?;
            self.log_security_event(
                user_id,
                "PASSKEY_ADDED",
                &ctx.client(),
                "",
                serde_json::json!({ "name": passkey.name }),
            )
            .await;
            ok(FinishPasskeyRegistrationResponse {
                passkey: passkey.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn begin_passkey_sign_in(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, BeginPasskeySignInRequest>,
    ) -> ServiceResult<BeginPasskeySignInResponse> {
        handle(async {
            self.core
                .cache
                .rate_limit(
                    &format!("passkey:ip:{}", ctx.client().ip),
                    60,
                    Duration::from_secs(300),
                )
                .await?;
            let (challenge_id, options_json) = Identity::begin_passkey_sign_in(self).await?;
            ok(BeginPasskeySignInResponse {
                challenge_id,
                options_json,
                ..Default::default()
            })
        })
        .await
    }

    async fn finish_passkey_sign_in(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, FinishPasskeySignInRequest>,
    ) -> ServiceResult<AuthResponse> {
        handle(async {
            let req = request.to_owned_message();
            let meta = ctx.client();
            let user =
                Identity::finish_passkey_sign_in(self, &req.challenge_id, &req.credential_json)
                    .await?;
            ensure!(
                user.banned_at.is_none(),
                denied,
                "ACCOUNT_BANNED",
                "this account has been suspended"
            );
            // A passkey is possession + user verification, so no TOTP step follows.
            let issued = self
                .create_session(&user, req.client.as_option(), &meta)
                .await?;
            self.log_security_event(
                user.id,
                "SIGN_IN",
                &meta,
                "",
                serde_json::json!({ "method": "passkey" }),
            )
            .await;
            self.auth_response(issued, &user, &meta).await
        })
        .await
    }

    async fn request_email_verification(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, RequestEmailVerificationRequest>,
    ) -> ServiceResult<RequestEmailVerificationResponse> {
        handle(async {
            let user = users::get(&self.core, ctx.user_id()?).await?;
            ensure!(
                user.email_verified_at.is_none(),
                precondition,
                "EMAIL_ALREADY_VERIFIED",
                "your email is already verified"
            );
            let email = user
                .email
                .clone()
                .or_not_found("EMAIL_MISSING", "no email address on this account")?;
            self.core
                .cache
                .rate_limit(
                    &format!("verify-mail:{}", user.id),
                    5,
                    Duration::from_secs(3600),
                )
                .await?;
            self.spawn_verification_mail(user.id, email, user.locale.clone(), "verify");
            ok(RequestEmailVerificationResponse::default())
        })
        .await
    }

    async fn verify_email(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, VerifyEmailRequest>,
    ) -> ServiceResult<VerifyEmailResponse> {
        handle(async {
        let (user_id, purpose, email) = self.consume_email_token(request.token, &["verify", "change"]).await?;
        if purpose == "change" {
            let taken: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE email = $1::citext AND id <> $2)")
                .bind(&email)
                .bind(user_id)
                .fetch_one(&self.core.db)
                .await
                .map_err(AppError::from)?;
            ensure!(!taken, already_exists, "EMAIL_TAKEN", "an account with this email already exists");
            sqlx::query("UPDATE users SET email = $2::citext, email_verified_at = now() WHERE id = $1")
                .bind(user_id)
                .bind(&email)
                .execute(&self.core.db)
                .await
                .map_err(AppError::from)?;
            self.log_security_event(user_id, "EMAIL_CHANGED", &ClientMeta::default(), "", serde_json::json!({})).await;
        } else {
            sqlx::query("UPDATE users SET email_verified_at = now() WHERE id = $1 AND email = $2::citext")
                .bind(user_id)
                .bind(&email)
                .execute(&self.core.db)
                .await
                .map_err(AppError::from)?;
        }
        ok(VerifyEmailResponse::default())
        })
        .await
    }

    async fn request_password_reset(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RequestPasswordResetRequest>,
    ) -> ServiceResult<RequestPasswordResetResponse> {
        handle(async {
        let meta = ctx.client();
        self.core.cache.rate_limit(&format!("reset:ip:{}", meta.ip), 10, Duration::from_secs(3600)).await?;
        // Always succeed so the response does not reveal whether the address is registered.
        if let Ok(email) = users::validate_email(request.email) {
            if let Some(user) = users::find_by_identifier(&self.core, &email).await? {
                let limited = self.core.cache.rate_limit(&format!("reset:user:{}", user.id), 3, Duration::from_secs(3600)).await.is_err();
                if !limited && user.banned_at.is_none() {
                    let identity = self.clone();
                    tokio::spawn(async move {
                        let result = async {
                            let token = identity.create_email_token(user.id, "reset", &email, chrono::Duration::minutes(RESET_TTL_MINUTES)).await?;
                            identity.send_password_reset_mail(&email, &user.locale, &token).await
                        }
                        .await;
                        if let Err(e) = result {
                            tracing::warn!(user_id = %user.id, error = %e, "password reset mail failed");
                        }
                    });
                }
            }
        }
        ok(RequestPasswordResetResponse::default())
        })
        .await
    }

    async fn reset_password(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ResetPasswordRequest>,
    ) -> ServiceResult<ResetPasswordResponse> {
        handle(async {
        users::validate_password(request.new_password)?;
        let hash = crate::password::hash(request.new_password).await?;
        let (user_id, _, email) = self.consume_email_token(request.token, &["reset"]).await?;
        // Receiving the reset mail proves control of the address.
        sqlx::query("UPDATE users SET password_hash = $2, email_verified_at = COALESCE(email_verified_at, CASE WHEN email = $3::citext THEN now() END) WHERE id = $1")
            .bind(user_id)
            .bind(hash)
            .bind(&email)
            .execute(&self.core.db)
            .await
            .map_err(AppError::from)?;
        self.drop_account_links(user_id).await?;
        self.revoke_all_sessions(user_id, None).await?;
        self.log_security_event(user_id, "PASSWORD_RESET", &ctx.client(), "", serde_json::json!({})).await;
        ok(ResetPasswordResponse::default())
        })
        .await
    }

    async fn start_device_login(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, StartDeviceLoginRequest>,
    ) -> ServiceResult<StartDeviceLoginResponse> {
        handle(async {
            let req = request.to_owned_message();
            let meta = ctx.client();
            self.core
                .cache
                .rate_limit(
                    &format!("devlogin:ip:{}", meta.ip),
                    20,
                    Duration::from_secs(600),
                )
                .await?;
            let client = req.client.as_option().cloned().unwrap_or_default();
            let device_code = random_token(32);
            let key = self.core.keys.token_hash(&device_code);
            let user_code = new_user_code();
            let login = DeviceLogin {
                user_code: user_code.clone(),
                platform: client.platform.to_i32(),
                device_name: if client.device_name.is_empty() {
                    crate::sessions::device_name_from_user_agent(&meta.user_agent)
                } else {
                    client.device_name.chars().take(80).collect()
                },
                app_version: client.app_version.chars().take(32).collect(),
                device_id: client.device_id.chars().take(128).collect(),
                status: "pending".to_owned(),
                user_id: None,
                expires_at: Utc::now() + chrono::Duration::from_std(DEVICE_LOGIN_TTL).unwrap(),
            };
            self.core
                .cache
                .set_json(
                    &format!("id:devlogin:{key}"),
                    &login,
                    Some(DEVICE_LOGIN_TTL),
                )
                .await?;
            self.core
                .cache
                .set_string(
                    &format!("id:devcode:{user_code}"),
                    &key,
                    Some(DEVICE_LOGIN_TTL),
                )
                .await?;
            ok(StartDeviceLoginResponse {
                device_code,
                verification_url: self.core.web_url(&format!("device?code={user_code}")),
                user_code,
                expires_in_seconds: DEVICE_LOGIN_TTL.as_secs() as i32,
                poll_interval_seconds: DEVICE_POLL_INTERVAL_SECONDS,
                ..Default::default()
            })
        })
        .await
    }

    async fn poll_device_login(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, PollDeviceLoginRequest>,
    ) -> ServiceResult<PollDeviceLoginResponse> {
        handle(async {
            let meta = ctx.client();
            let key = self.core.keys.token_hash(request.device_code);
            self.core
                .cache
                .rate_limit(&format!("devpoll:{key}"), 120, Duration::from_secs(600))
                .await?;
            let Some(login) = self
                .core
                .cache
                .get_json::<DeviceLogin>(&format!("id:devlogin:{key}"))
                .await?
            else {
                return ok(PollDeviceLoginResponse {
                    status: DeviceLoginStatus::DEVICE_LOGIN_STATUS_EXPIRED.into(),
                    ..Default::default()
                });
            };
            let status = login.status();
            if status != DeviceLoginStatus::DEVICE_LOGIN_STATUS_APPROVED {
                return ok(PollDeviceLoginResponse {
                    status: status.into(),
                    ..Default::default()
                });
            }
            // Approved: this poll claims the login exactly once.
            let claimed: Option<DeviceLogin> = self
                .core
                .cache
                .take_json(&format!("id:devlogin:{key}"))
                .await?;
            let Some(login) = claimed else {
                return ok(PollDeviceLoginResponse {
                    status: DeviceLoginStatus::DEVICE_LOGIN_STATUS_EXPIRED.into(),
                    ..Default::default()
                });
            };
            self.core
                .cache
                .del(&format!("id:devcode:{}", login.user_code))
                .await?;
            let user_id = login
                .user_id
                .or_not_found("DEVICE_CODE_INVALID", "this code is invalid or has expired")?;
            let user = users::get(&self.core, user_id).await?;
            ensure!(
                user.is_active(),
                denied,
                "ACCOUNT_BANNED",
                "this account cannot sign in"
            );
            let client = login.client();
            let issued = self.create_session(&user, Some(&client), &meta).await?;
            self.log_security_event(
                user.id,
                "SIGN_IN",
                &meta,
                &login.device_name,
                serde_json::json!({ "method": "device" }),
            )
            .await;
            // Device logins are never web clients; tokens go in the body.
            let auth = AuthResponse {
                access_token: issued.access_token,
                refresh_token: issued.refresh_token,
                access_token_expires_at: ts(issued.access_expires_at),
                me: self.me(&user).await?.into(),
                session_id: issued.session_id.to_string(),
                ..Default::default()
            };
            ok(PollDeviceLoginResponse {
                status: DeviceLoginStatus::DEVICE_LOGIN_STATUS_APPROVED.into(),
                auth: auth.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_device_login(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetDeviceLoginRequest>,
    ) -> ServiceResult<GetDeviceLoginResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let (_, login) = self.device_login_by_user_code(user_id, request.user_code).await?;
            ok(GetDeviceLoginResponse {
                client: login.client().into(),
                status: login.status().into(),
                expires_at: ts(login.expires_at),
                ..Default::default()
            })
        })
        .await
    }

    async fn approve_device_login(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ApproveDeviceLoginRequest>,
    ) -> ServiceResult<ApproveDeviceLoginResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let (key, mut login) = self.device_login_by_user_code(user_id, request.user_code).await?;
            ensure!(
                login.status == "pending",
                precondition,
                "DEVICE_CODE_USED",
                "this code was already answered"
            );
            login.status = if request.approve {
                "approved".to_owned()
            } else {
                "denied".to_owned()
            };
            login.user_id = request.approve.then_some(user_id);
            let remaining = (login.expires_at - Utc::now())
                .to_std()
                .unwrap_or(Duration::from_secs(1));
            self.core
                .cache
                .set_json(&format!("id:devlogin:{key}"), &login, Some(remaining))
                .await?;
            let kind = if request.approve {
                "DEVICE_LOGIN_APPROVED"
            } else {
                "DEVICE_LOGIN_DENIED"
            };
            self.log_security_event(
                user_id,
                kind,
                &ctx.client(),
                &login.device_name,
                serde_json::json!({}),
            )
            .await;
            ok(ApproveDeviceLoginResponse::default())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_codes_are_readable_and_normalize() {
        let code = new_user_code();
        assert_eq!(code.len(), 9);
        assert_eq!(code.as_bytes()[4], b'-');
        assert!(
            code.chars()
                .all(|c| c == '-' || (c.is_ascii_alphanumeric() && !"IO01".contains(c)))
        );
        assert_eq!(normalize_user_code("abcd 2345"), "ABCD-2345");
        assert_eq!(normalize_user_code("ABCD-2345"), "ABCD-2345");
        assert_eq!(normalize_user_code("x"), "X");
    }
}
