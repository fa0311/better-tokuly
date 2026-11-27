//! `AuthService`: sign-up, sign-in, token refresh, passkeys, email flows and
//! device login.
use std::time::Duration;

use chrono::{DateTime, Utc};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use serde::{Deserialize, Serialize};
use tokuly_core::{analytics::AnalyticsEvent, auth::ClientMeta, events, keys::random_token, prelude::*};
use tokuly_proto::identity::*;

use crate::{
    Identity,
    sessions::{Issued, REFRESH_COOKIE, cookie_value},
    users::{self, UserRow},
};

const VERIFY_TTL_HOURS: i64 = 24;
/// How long the previous address can undo an e-mail change.
const REVERT_TTL_DAYS: i64 = 7;
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

/// Valkey key holding when the address behind an undo link was verified.
fn revert_verified_key(token_hash: &str) -> String {
    format!("id:revert:verified:{token_hash}")
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
    let compact: String = input.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect();
    if compact.len() == 8 { format!("{}-{}", &compact[..4], &compact[4..]) } else { compact }
}

impl Identity {
    /// Builds the response for a successful authentication. Web clients get
    /// the refresh token as an HttpOnly cookie instead of in the body.
    pub(crate) async fn auth_response(
        &self,
        issued: Issued,
        user: &UserRow,
        meta: &ClientMeta,
    ) -> AppResult<Response<AuthResponse>> {
        let body = AuthResponse {
            access_token: issued.access_token,
            refresh_token: if meta.is_web { String::new() } else { issued.refresh_token.clone() },
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

    /// Signs `user` in on this device after their credentials were checked:
    /// with `add_account`, beside the accounts already there (the active one
    /// is parked, the cap applies) instead of in their place. Logs the
    /// sign-in and builds the response.
    async fn establish_session(
        &self,
        ctx: &RequestContext,
        user: &UserRow,
        client: Option<&ClientInfo>,
        method: &str,
        mfa: bool,
        add_account: bool,
    ) -> AppResult<Response<AuthResponse>> {
        let meta = ctx.client();
        let current = ctx.maybe_auth().ok().flatten().map(|auth| auth.session_id);
        let device_id = client.map(|c| c.device_id.as_str()).unwrap_or_default();
        let (parked, others) =
            if add_account { self.make_room_for(user, &meta, current, device_id).await? } else { (Vec::new(), false) };
        let origin = crate::sessions::SessionOrigin { mfa, added_by_switch: others, ..Default::default() };
        let issued = self.create_session_with(user, client, &meta, origin).await?;
        let kind = if method == "signup" { "SIGN_UP" } else { "SIGN_IN" };
        let device_name = client.map(|c| c.device_name.as_str()).unwrap_or("");
        self.log_security_event(
            user.id,
            kind,
            &meta,
            device_name,
            serde_json::json!({ "method": method, "added_account": others }),
        )
        .await;
        let response = self.auth_response(issued, user, &meta).await?;
        Ok(if add_account && meta.is_web {
            response.with_header("set-cookie", self.parked_cookie(&parked))
        } else {
            response
        })
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

    /// Refuses a token [`Self::consume_email_token`] would refuse, with the
    /// same answer and without using it up: the cheap check before work
    /// that is only worth doing for a real link.
    async fn require_live_email_token(&self, token: &str, purposes: &[&str]) -> AppResult<()> {
        let live: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM email_tokens
                           WHERE token_hash = ANY($1) AND used_at IS NULL AND expires_at > now() AND purpose = ANY($2))",
        )
        .bind(self.core.keys.token_hashes(token))
        .bind(purposes.iter().map(|p| p.to_string()).collect::<Vec<_>>())
        .fetch_one(&self.core.db)
        .await?;
        ensure!(live, invalid, "TOKEN_INVALID", "this link is invalid or has expired");
        Ok(())
    }

    /// Marks an email token used and returns `(user_id, purpose, email)`.
    async fn consume_email_token(&self, token: &str, purposes: &[&str]) -> AppResult<(Uuid, String, String)> {
        let row: Option<(Uuid, String, String)> = sqlx::query_as(
            "UPDATE email_tokens SET used_at = now()
             WHERE token_hash = ANY($1) AND used_at IS NULL AND expires_at > now() AND purpose = ANY($2)
             RETURNING user_id, purpose, email::text",
        )
        .bind(self.core.keys.token_hashes(token))
        .bind(purposes.iter().map(|p| p.to_string()).collect::<Vec<_>>())
        .fetch_optional(&self.core.db)
        .await?;
        row.ok_or_else(|| AppError::invalid("TOKEN_INVALID", "this link is invalid or has expired"))
    }

    /// Mails a security notice to the account's address, without blocking
    /// the caller; a failure is logged.
    pub(crate) fn notify_security_change(&self, user: &UserRow, notice: crate::mailer::SecurityNotice) {
        let Some(email) = user.email.clone() else { return };
        let (identity, locale, user_id) = (self.clone(), user.locale.clone(), user.id);
        tokio::spawn(async move {
            if let Err(e) = identity.send_security_notice(&email, &locale, notice).await {
                tracing::warn!(%user_id, ?notice, error = %e, "security notice mail failed");
            }
        });
    }

    /// The address of `user_id` changed from `previous`, verified at
    /// `verified_at`, to `current`: the previous address gets a link that
    /// moves the account back (7 days, once), and stays reserved for it
    /// meanwhile ([`users::email_unavailable`]). Older such links stay valid,
    /// so a second change cannot take the first undo away. Only a verified
    /// address is offered one: an undo must not hand an account an address it
    /// never proved.
    async fn offer_email_revert(
        &self,
        user_id: Uuid,
        previous: &str,
        verified_at: DateTime<Utc>,
        current: &str,
        locale: &str,
    ) -> AppResult<()> {
        let token = random_token(32);
        let token_hash = self.core.keys.token_hash(&token);
        let ttl = chrono::Duration::days(REVERT_TTL_DAYS);
        // The undo restores the address as it was, verified when it was.
        self.core
            .cache
            .set_string(&revert_verified_key(&token_hash), &verified_at.to_rfc3339(), ttl.to_std().ok())
            .await?;
        sqlx::query("INSERT INTO email_tokens (token_hash, user_id, purpose, email, expires_at) VALUES ($1, $2, 'revert', $3::citext, $4)")
            .bind(&token_hash)
            .bind(user_id)
            .bind(previous)
            .bind(Utc::now() + ttl)
            .execute(&self.core.db)
            .await?;
        let (identity, previous, current, locale) =
            (self.clone(), previous.to_owned(), current.to_owned(), locale.to_owned());
        tokio::spawn(async move {
            if let Err(e) = identity.send_email_changed_mail(&previous, &locale, &current, &token).await {
                tracing::warn!(%user_id, error = %e, "email changed mail failed");
            }
        });
        Ok(())
    }

    /// Sends (or re-sends) the address verification mail without blocking the caller.
    pub(crate) fn spawn_verification_mail(&self, user_id: Uuid, email: String, locale: String, purpose: &'static str) {
        let identity = self.clone();
        tokio::spawn(async move {
            let result = async {
                let token = identity
                    .create_email_token(user_id, purpose, &email, chrono::Duration::hours(VERIFY_TTL_HOURS))
                    .await?;
                identity.send_verification_mail(&email, &locale, &token, purpose == "change").await
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
        self.core.cache.rate_limit(&format!("devcode:user:{user_id}"), 20, Duration::from_secs(600)).await?;
        let code = normalize_user_code(user_code);
        let not_found = || AppError::not_found("DEVICE_CODE_INVALID", "this code is invalid or has expired");
        let key = self.core.cache.get_string(&format!("id:devcode:{code}")).await?.ok_or_else(not_found)?;
        let login: DeviceLogin =
            self.core.cache.get_json(&format!("id:devlogin:{key}")).await?.ok_or_else(not_found)?;
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
        let handle = users::handle_error(users::claimable_handle(&req.handle))?;
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

        let handle_taken: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE handle = $1::citext)")
            .bind(&handle)
            .fetch_one(&core.db)
            .await?;
        let email_taken = users::email_unavailable(core, &email, None).await?;
        ensure!(!handle_taken, already_exists, "HANDLE_TAKEN", "this handle is already in use");
        let handle_held = users::handle_held_from(core, &handle, None).await?;
        ensure!(!handle_held, already_exists, "HANDLE_TAKEN", "this handle is already in use");
        ensure!(!email_taken, already_exists, "EMAIL_TAKEN", "an account with this email already exists");

        let password_hash = crate::password::hash(&req.password).await?;
        let invite_code = req.invite_code.trim().to_owned();
        let mut tx = core.db.begin().await?;
        let user: UserRow = sqlx::query_as(concat!(
            "INSERT INTO users (handle, display_name, email, password_hash, birthdate, locale, time_zone, attrs, privacy)
             VALUES ($1::citext, $2, $3::citext, $4, $5, $6, $7, $8, $9) RETURNING ",
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
        .bind(crate::settings::sign_up_privacy(birthdate, Utc::now().date_naive()))
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
        core.analytics.track(AnalyticsEvent::new("user.signup").user(Some(user.id))).await;
        self.establish_session(&ctx, &user, client, "signup", false, req.add_account).await
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
            core.cache.rate_limit(&format!("signin:ip:{}", meta.ip), 30, Duration::from_secs(300)).await?;
            core.cache.rate_limit(&format!("signin:id:{identifier}"), 10, Duration::from_secs(300)).await?;

            let user = users::find_by_identifier(core, &identifier).await?;
            if let Some(user) = &user {
                // However the account was named (handle, address), one budget.
                core.cache.rate_limit(&format!("signin:user:{}", user.id), 10, Duration::from_secs(300)).await?;
            }
            let password_ok =
                crate::password::verify(&req.password, user.as_ref().and_then(|u| u.password_hash.as_deref())).await;
            let Some(user) = user.filter(|_| password_ok) else {
                if let Ok(Some(existing)) = users::find_by_identifier(core, &identifier).await {
                    self.log_security_event(existing.id, "SIGN_IN_FAILED", &meta, "", serde_json::json!({})).await;
                }
                bail!(unauthenticated, "INVALID_CREDENTIALS", "wrong email, handle or password");
            };
            ensure!(user.banned_at.is_none(), denied, "ACCOUNT_BANNED", "this account has been suspended");

            let mfa = user.totp_enabled_at.is_some();
            if mfa {
                if req.totp_code.trim().is_empty() {
                    return ok(AuthResponse { mfa_required: true, ..Default::default() });
                }
                ensure!(
                    self.second_factor(&user, &req.totp_code, &meta).await?,
                    unauthenticated,
                    "INVALID_TOTP",
                    "wrong authentication code"
                );
            }
            self.establish_session(&ctx, &user, req.client.as_option(), "password", mfa, req.add_account).await
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
            cookie_value(&meta.cookies, REFRESH_COOKIE).unwrap_or_default().to_owned()
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
                self.log_security_event(
                    auth.user_id,
                    "SIGN_OUT",
                    &meta,
                    "",
                    serde_json::json!({ "all": request.all_sessions }),
                )
                .await;
            } else if let Some(token) = cookie_value(&meta.cookies, REFRESH_COOKIE) {
                let row: Option<(Uuid, Uuid)> =
                    sqlx::query_as("SELECT id, user_id FROM sessions WHERE refresh_hash = ANY($1) AND revoked_at IS NULL")
                        .bind(self.core.keys.token_hashes(token))
                        .fetch_optional(&self.core.db)
                        .await?;
                if let Some((session_id, user_id)) = row {
                    self.revoke_session(user_id, session_id).await?;
                }
            }
            // A shared computer: every account of this device goes, the parked ones too.
            if request.all_accounts_on_device {
                let parked: Vec<String> = if meta.is_web {
                    crate::device_accounts::DeviceTokens::from_cookies(&meta).parked
                } else {
                    request.parked_refresh_tokens.iter().take(16).map(|t| t.to_string()).collect()
                };
                for session in self.live_sessions(&parked).await? {
                    self.revoke_session(session.user_id, session.id).await?;
                    self.log_security_event(
                        session.user_id,
                        "SIGN_OUT",
                        &meta,
                        "",
                        serde_json::json!({ "all": false }),
                    )
                    .await;
                }
            }
            let response = Response::new(SignOutResponse::default());
            Ok(if meta.is_web {
                let response = response.with_header("set-cookie", self.clear_refresh_cookie());
                if request.all_accounts_on_device {
                    response.with_header("set-cookie", self.parked_cookie(&[]))
                } else {
                    response
                }
            } else {
                response
            })
        })
        .await
    }

    async fn check_handle(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CheckHandleRequest>,
    ) -> ServiceResult<CheckHandleResponse> {
        handle(async {
            // Asked while someone types a handle; far more than that is
            // someone listing which handles exist. Counted per account when
            // signed in (choosing a new handle in settings), so that a
            // school's or a mobile carrier's shared address does not put
            // its members in each other's way.
            let limiter = match ctx.maybe_user_id()? {
                Some(user_id) => format!("user:{user_id}"),
                None => format!("ip:{}", ctx.client().ip),
            };
            self.core.cache.rate_limit(&format!("handle:check:{limiter}"), 120, Duration::from_secs(60)).await?;
            let normalized = match users::claimable_handle(request.handle) {
                Ok(h) => h,
                Err(reason) => {
                    return ok(CheckHandleResponse {
                        available: false,
                        reason: reason.to_owned(),
                        ..Default::default()
                    });
                }
            };
            let taken: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE handle = $1::citext)")
                .bind(&normalized)
                .fetch_one(&self.core.db)
                .await
                .map_err(AppError::from)?
                || users::handle_held_from(&self.core, &normalized, None).await?;
            ok(CheckHandleResponse {
                available: !taken,
                normalized,
                reason: if taken { "TAKEN".to_owned() } else { String::new() },
                ..Default::default()
            })
        })
        .await
    }

    async fn get_auth_capabilities(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, GetAuthCapabilitiesRequest>,
    ) -> ServiceResult<GetAuthCapabilitiesResponse> {
        let capabilities = AuthCapabilities { passkeys_available: self.passkeys_available(), ..Default::default() };
        handle(async { ok(GetAuthCapabilitiesResponse { capabilities: capabilities.into(), ..Default::default() }) })
            .await
    }

    async fn begin_passkey_registration(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BeginPasskeyRegistrationRequest>,
    ) -> ServiceResult<BeginPasskeyRegistrationResponse> {
        handle(async {
            let user = users::get(&self.core, ctx.user_id()?).await?;
            // Before the password check: on a server without passkeys there is
            // nothing to confirm the password for.
            self.relying_party()?;
            self.confirm_password(&user, request.current_password).await?;
            let (challenge_id, options_json) = Identity::begin_passkey_registration(self, &user, request.name).await?;
            ok(BeginPasskeyRegistrationResponse { challenge_id, options_json, ..Default::default() })
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
            let passkey =
                Identity::finish_passkey_registration(self, user_id, request.challenge_id, request.credential_json)
                    .await?;
            self.log_security_event(
                user_id,
                "PASSKEY_ADDED",
                &ctx.client(),
                "",
                serde_json::json!({ "name": passkey.name }),
            )
            .await;
            let user = users::get(&self.core, user_id).await?;
            self.notify_security_change(&user, crate::mailer::SecurityNotice::PasskeyAdded);
            ok(FinishPasskeyRegistrationResponse { passkey: passkey.into(), ..Default::default() })
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
                .rate_limit(&format!("passkey:ip:{}", ctx.client().ip), 60, Duration::from_secs(300))
                .await?;
            let (challenge_id, options_json) = Identity::begin_passkey_sign_in(self).await?;
            ok(BeginPasskeySignInResponse { challenge_id, options_json, ..Default::default() })
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
            self.core
                .cache
                .rate_limit(&format!("passkey:finish:ip:{}", ctx.client().ip), 60, Duration::from_secs(300))
                .await?;
            let user = Identity::finish_passkey_sign_in(self, &req.challenge_id, &req.credential_json).await?;
            ensure!(user.banned_at.is_none(), denied, "ACCOUNT_BANNED", "this account has been suspended");
            // A passkey is possession + user verification, so no TOTP step
            // follows, and it counts as the second factor.
            self.establish_session(&ctx, &user, req.client.as_option(), "passkey", true, req.add_account).await
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
            let email = user.email.clone().or_not_found("EMAIL_MISSING", "no email address on this account")?;
            self.core.cache.rate_limit(&format!("verify-mail:{}", user.id), 5, Duration::from_secs(3600)).await?;
            self.spawn_verification_mail(user.id, email, user.locale.clone(), "verify");
            ok(RequestEmailVerificationResponse::default())
        })
        .await
    }

    async fn verify_email(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, VerifyEmailRequest>,
    ) -> ServiceResult<VerifyEmailResponse> {
        handle(async {
        let meta = ctx.client();
        // Look before spending the link: a change or undo to an address that
        // cannot be had fails without using it up.
        let pending: Option<(Uuid, String, String, String)> = sqlx::query_as(
            "SELECT user_id, purpose, email::text, token_hash FROM email_tokens
             WHERE token_hash = ANY($1) AND used_at IS NULL AND expires_at > now() AND purpose IN ('verify', 'change', 'revert')",
        )
        .bind(self.core.keys.token_hashes(request.token))
        .fetch_optional(&self.core.db)
        .await?;
        // As stored: the link may be from before the secret was replaced.
        let token_hash = pending.as_ref().map_or_else(|| self.core.keys.token_hash(request.token), |row| row.3.clone());
        if let Some((owner, purpose, email, _)) = &pending
            && purpose != "verify"
        {
            let unavailable = users::email_unavailable(&self.core, email, Some(*owner)).await?;
            ensure!(!unavailable, already_exists, "EMAIL_TAKEN", "an account with this email already exists");
        }
        let (user_id, purpose, email) = self.consume_email_token(request.token, &["verify", "change", "revert"]).await?;
        // The person opening the link may be signed in on this device; that
        // session survives a change (not an undo).
        let opener = ctx.maybe_auth().ok().flatten().filter(|auth| auth.user_id == user_id).map(|auth| auth.session_id);
        let mut reverted = false;
        if purpose == "change" || purpose == "revert" {
            let before = users::get(&self.core, user_id).await?;
            // A change is proven by its link now; an undo puts back the
            // address as it was, verified when it was (never "now").
            let verified_at = if purpose == "change" {
                Some(Utc::now())
            } else {
                let key = revert_verified_key(&token_hash);
                let stored = self.core.cache.get_string(&key).await?;
                self.core.cache.del(&key).await?;
                stored.and_then(|t| DateTime::parse_from_rfc3339(&t).ok()).map(|t| t.with_timezone(&Utc))
            };
            let mut tx = self.core.db.begin().await?;
            sqlx::query("UPDATE users SET email = $2::citext, email_verified_at = $3 WHERE id = $1")
                .bind(user_id)
                .bind(&email)
                .bind(verified_at)
                .execute(&mut *tx)
                .await
                .map_err(AppError::from)?;
            if purpose == "revert" {
                // Undo links issued after this one point at addresses the
                // account only had since; they go.
                sqlx::query(
                    "DELETE FROM email_tokens WHERE user_id = $1 AND purpose = 'revert'
                     AND created_at > (SELECT created_at FROM email_tokens WHERE token_hash = $2)",
                )
                .bind(user_id)
                .bind(&token_hash)
                .execute(&mut *tx)
                .await?;
            }
            if verified_at.is_some() {
                events::emit(&mut *tx, "identity.user.email_verified", Some(user_id), &crate::maintenance::UserRef { user_id }).await?;
            }
            tx.commit().await?;
            // Whoever had the account under the other address loses it:
            // pending reset and change links die, and devices sign out.
            self.drop_account_links(user_id).await?;
            if purpose == "revert" {
                reverted = true;
                self.revoke_all_sessions(user_id, None).await?;
                self.log_security_event(user_id, "EMAIL_CHANGE_REVERTED", &meta, "", serde_json::json!({})).await;
            } else {
                self.revoke_all_sessions(user_id, opener).await?;
                self.log_security_event(user_id, "EMAIL_CHANGED", &meta, "", serde_json::json!({})).await;
                // An unverified previous address is not told and gets no undo:
                // it was never shown to belong to this account.
                if let (Some(previous), Some(verified_at)) = (before.email.as_deref(), before.email_verified_at)
                    && !previous.eq_ignore_ascii_case(&email)
                {
                    self.offer_email_revert(user_id, previous, verified_at, &email, &before.locale).await?;
                }
            }
        } else {
            let mut tx = self.core.db.begin().await?;
            let done = sqlx::query("UPDATE users SET email_verified_at = now() WHERE id = $1 AND email = $2::citext AND email_verified_at IS NULL")
                .bind(user_id)
                .bind(&email)
                .execute(&mut *tx)
                .await
                .map_err(AppError::from)?;
            if done.rows_affected() == 1 {
                events::emit(&mut *tx, "identity.user.email_verified", Some(user_id), &crate::maintenance::UserRef { user_id }).await?;
            }
            tx.commit().await?;
        }
        ok(VerifyEmailResponse { email_change_reverted: reverted, ..Default::default() })
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
                    let limited = self
                        .core
                        .cache
                        .rate_limit(&format!("reset:user:{}", user.id), 3, Duration::from_secs(3600))
                        .await
                        .is_err();
                    if !limited && user.banned_at.is_none() {
                        let identity = self.clone();
                        tokio::spawn(async move {
                            let result = async {
                                let token = identity
                                    .create_email_token(
                                        user.id,
                                        "reset",
                                        &email,
                                        chrono::Duration::minutes(RESET_TTL_MINUTES),
                                    )
                                    .await?;
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
        self.core.cache.rate_limit(&format!("reset:finish:ip:{}", ctx.client().ip), 30, Duration::from_secs(3600)).await?;
        users::validate_password(request.new_password)?;
        // Hashing a password is the most expensive thing an anonymous caller
        // can ask for: only a link that would be accepted gets that far.
        self.require_live_email_token(request.token, &["reset"]).await?;
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
            self.core.cache.rate_limit(&format!("devlogin:ip:{}", meta.ip), 20, Duration::from_secs(600)).await?;
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
            self.core.cache.set_json(&format!("id:devlogin:{key}"), &login, Some(DEVICE_LOGIN_TTL)).await?;
            self.core.cache.set_string(&format!("id:devcode:{user_code}"), &key, Some(DEVICE_LOGIN_TTL)).await?;
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
            self.core.cache.rate_limit(&format!("devpoll:{key}"), 120, Duration::from_secs(600)).await?;
            let Some(login) = self.core.cache.get_json::<DeviceLogin>(&format!("id:devlogin:{key}")).await? else {
                return ok(PollDeviceLoginResponse {
                    status: DeviceLoginStatus::DEVICE_LOGIN_STATUS_EXPIRED.into(),
                    ..Default::default()
                });
            };
            let status = login.status();
            if status != DeviceLoginStatus::DEVICE_LOGIN_STATUS_APPROVED {
                return ok(PollDeviceLoginResponse { status: status.into(), ..Default::default() });
            }
            // Approved: this poll claims the login exactly once.
            let claimed: Option<DeviceLogin> = self.core.cache.take_json(&format!("id:devlogin:{key}")).await?;
            let Some(login) = claimed else {
                return ok(PollDeviceLoginResponse {
                    status: DeviceLoginStatus::DEVICE_LOGIN_STATUS_EXPIRED.into(),
                    ..Default::default()
                });
            };
            self.core.cache.del(&format!("id:devcode:{}", login.user_code)).await?;
            let user_id = login.user_id.or_not_found("DEVICE_CODE_INVALID", "this code is invalid or has expired")?;
            let user = users::get(&self.core, user_id).await?;
            ensure!(user.is_active(), denied, "ACCOUNT_BANNED", "this account cannot sign in");
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
            ensure!(login.status == "pending", precondition, "DEVICE_CODE_USED", "this code was already answered");
            login.status = if request.approve { "approved".to_owned() } else { "denied".to_owned() };
            login.user_id = request.approve.then_some(user_id);
            let remaining = (login.expires_at - Utc::now()).to_std().unwrap_or(Duration::from_secs(1));
            self.core.cache.set_json(&format!("id:devlogin:{key}"), &login, Some(remaining)).await?;
            let kind = if request.approve { "DEVICE_LOGIN_APPROVED" } else { "DEVICE_LOGIN_DENIED" };
            self.log_security_event(user_id, kind, &ctx.client(), &login.device_name, serde_json::json!({})).await;
            ok(ApproveDeviceLoginResponse::default())
        })
        .await
    }

    async fn list_device_accounts(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListDeviceAccountsRequest>,
    ) -> ServiceResult<ListDeviceAccountsResponse> {
        handle(async {
            let meta = ctx.client();
            let current = ctx.maybe_auth().ok().flatten().filter(|a| a.api_token.is_none()).map(|a| a.session_id);
            let native: Vec<String> = request.refresh_tokens.iter().map(|t| t.to_string()).collect();
            let (accounts, cookie) = self.device_accounts(&meta, current, &native).await?;
            let response = Response::new(ListDeviceAccountsResponse {
                accounts,
                limit: self.cfg.device_account_limit,
                ..Default::default()
            });
            Ok(match cookie {
                Some(cookie) => response.with_header("set-cookie", cookie),
                None => response,
            })
        })
        .await
    }

    async fn switch_account(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SwitchAccountRequest>,
    ) -> ServiceResult<AuthResponse> {
        handle(async {
            let meta = ctx.client();
            let current = ctx.maybe_auth().ok().flatten().filter(|a| a.api_token.is_none()).map(|a| a.session_id);
            self.switch_account(&meta, current, request.session_id, request.refresh_token, request.totp_code).await
        })
        .await
    }

    async fn remove_device_account(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveDeviceAccountRequest>,
    ) -> ServiceResult<RemoveDeviceAccountResponse> {
        handle(async {
            let meta = ctx.client();
            let cookie = self.remove_device_account(&meta, request.session_id, request.refresh_token).await?;
            let response = Response::new(RemoveDeviceAccountResponse::default());
            Ok(match cookie {
                Some(cookie) => response.with_header("set-cookie", cookie),
                None => response,
            })
        })
        .await
    }

    async fn create_web_handoff(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateWebHandoffRequest>,
    ) -> ServiceResult<CreateWebHandoffResponse> {
        self.create_web_handoff_rpc(ctx, request).await
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
        assert!(code.chars().all(|c| c == '-' || (c.is_ascii_alphanumeric() && !"IO01".contains(c))));
        assert_eq!(normalize_user_code("abcd 2345"), "ABCD-2345");
        assert_eq!(normalize_user_code("ABCD-2345"), "ABCD-2345");
        assert_eq!(normalize_user_code("x"), "X");
    }
}
