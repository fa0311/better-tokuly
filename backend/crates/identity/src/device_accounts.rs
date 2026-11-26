//! Several accounts on one device, and switching between them (a main
//! account, a 裏垢, a creator account).
//!
//! # Model
//!
//! Every account on a device has a session of its own there, made by a full
//! sign-in to that account (`SignIn` / `SignUp` / `FinishPasskeySignIn` with
//! `add_account`): its own password and its own second factor. One session
//! is *active*; the others are *parked* (`sessions.parked_at`). A switch
//! parks the active session and activates the target, rotating its refresh
//! token: the result is a normal session of the target account, which shows
//! the switch in its security log (`ACCOUNT_SWITCHED`) and in its device
//! list (`Session.added_by_switch`, `parked`, `last_switched_at`). At most
//! [`crate::IdentityConfig::device_account_limit`] accounts share a device.
//!
//! Web clients cannot see their refresh tokens: the active one is the
//! `tokuly_rt` cookie as before, the parked ones travel in a second HttpOnly
//! cookie ([`PARKED_COOKIE`]) scoped like it. Native clients keep every
//! token in their secure storage and hand the parked ones in.
//!
//! # What stays separate
//!
//! The server never records that two accounts share a device: there is no
//! link table, and nothing one account can read — its profile, security
//! log, device list, data export, API tokens — names or counts the others.
//! Each account keeps its own session lifetime: signing one out
//! (`SignOut`, `RemoveDeviceAccount`) leaves the others signed in, and
//! everything that ends an account's sessions (a password change or reset,
//! an address change, a suspension, deletion) ends its parked session too,
//! which then simply drops out of the device's list.
//!
//! # Threat model, and when a switch asks for the second factor
//!
//! - *Someone picks up the unlocked phone* (a friend, a sibling, a parent)
//!   while the main account is open. A one-tap switch would hand them every
//!   account behind it. So switching to an account that has two-factor
//!   authentication on asks for its TOTP or recovery code unless that
//!   account's session on this device passed the second factor in the last
//!   [`crate::IdentityConfig::switch_mfa_hours`] (12 by default):
//!   `sessions.mfa_at`, set by a sign-in with TOTP, a passkey sign-in, a
//!   switch that asked, and turning TOTP on. An account without two-factor
//!   authentication switches without asking, the same as reopening the app
//!   would sign it in: its owner chose a single factor. Accounts that turn
//!   TOTP on later are asked at their next switch.
//! - *A parked token is taken off the device* (a modified client, a backup).
//!   `RefreshToken` refuses a parked session (`SWITCH_REQUIRED`), so the
//!   token is only good through `SwitchAccount`, which applies the rule
//!   above; replaying an old token revokes the session as everywhere else.
//! - *Correlating accounts* (staff, a leaked database, a data export).
//!   Nothing beyond what sessions already record (device name, address)
//!   ties the sessions of a device together; parked tokens are only ever
//!   held by the device.
//! - *Account farming*. The per-device cap and the sign-in limits bound how
//!   many accounts one browser can keep at hand; making accounts is limited
//!   by sign-up's own limits.
use std::collections::HashSet;
use std::time::Duration;

use chrono::{DateTime, Utc};
use connectrpc::Response;
use tokuly_core::{auth::ClientMeta, prelude::*};
use tokuly_proto::identity::{AuthResponse, DeviceAccount};

use crate::{
    Identity,
    sessions::{REFRESH_COOKIE, cookie_value},
    totp,
    users::{self, UserRow},
};

/// HttpOnly cookie with the refresh tokens of a browser's parked accounts.
pub const PARKED_COOKIE: &str = "tokuly_ra";
const TOKEN_SEPARATOR: char = '~';

/// A live session found by its current refresh token.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct LiveSession {
    pub id: Uuid,
    pub user_id: Uuid,
    pub parked_at: Option<DateTime<Utc>>,
    pub mfa_at: Option<DateTime<Utc>>,
    pub last_active_at: DateTime<Utc>,
    #[sqlx(skip)]
    pub token: String,
}

/// The refresh tokens a browser sent: the active one and the parked ones.
#[derive(Debug, Default)]
pub(crate) struct DeviceTokens {
    pub active: Option<String>,
    pub parked: Vec<String>,
}

impl DeviceTokens {
    pub fn from_cookies(meta: &ClientMeta) -> Self {
        let active = cookie_value(&meta.cookies, REFRESH_COOKIE).filter(|t| !t.is_empty()).map(str::to_owned);
        let parked = cookie_value(&meta.cookies, PARKED_COOKIE)
            .map(|value| value.split(TOKEN_SEPARATOR).filter(|t| !t.is_empty()).map(str::to_owned).collect())
            .unwrap_or_default();
        Self { active, parked }
    }
}

/// Where a session came from, for the account's device list and the
/// switch's second-factor rule.
#[derive(Debug, Clone, Copy, Default)]
pub struct SessionOrigin {
    /// The sign-in passed a second factor (TOTP, recovery code, passkey).
    pub mfa: bool,
    /// Signed in beside other accounts of the device.
    pub added_by_switch: bool,
    /// A web session opened from this app session (a hand-off): it ends
    /// with it.
    pub linked_to: Option<Uuid>,
}

/// Stops a web response from carrying a header value that cannot be one.
fn cookie_safe(token: &str) -> bool {
    !token.is_empty() && token.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

impl Identity {
    /// The live (not revoked, not expired) sessions behind `tokens`, by their
    /// current refresh token, in the order given; dead tokens are left out.
    pub(crate) async fn live_sessions(&self, tokens: &[String]) -> AppResult<Vec<LiveSession>> {
        let tokens: Vec<&String> = tokens.iter().filter(|t| cookie_safe(t)).collect();
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        let hashes: Vec<String> = tokens.iter().map(|t| self.core.keys.token_hash(t)).collect();
        let rows: Vec<(String, LiveSession)> =
            sqlx::query_as::<_, (String, Uuid, Uuid, Option<DateTime<Utc>>, Option<DateTime<Utc>>, DateTime<Utc>)>(
                "SELECT refresh_hash, id, user_id, parked_at, mfa_at, last_active_at FROM sessions
             WHERE refresh_hash = ANY($1) AND revoked_at IS NULL AND expires_at > now()",
            )
            .bind(&hashes)
            .fetch_all(&self.core.db)
            .await?
            .into_iter()
            .map(|(hash, id, user_id, parked_at, mfa_at, last_active_at)| {
                (hash, LiveSession { id, user_id, parked_at, mfa_at, last_active_at, token: String::new() })
            })
            .collect();
        let mut seen = HashSet::new();
        Ok(tokens
            .iter()
            .zip(&hashes)
            .filter_map(|(token, hash)| {
                let (_, row) = rows.iter().find(|(h, _)| h == hash)?;
                seen.insert(row.id).then(|| LiveSession { token: (*token).clone(), ..row.clone() })
            })
            .collect())
    }

    pub(crate) async fn park_session(&self, session_id: Uuid) -> AppResult<()> {
        sqlx::query("UPDATE sessions SET parked_at = now() WHERE id = $1 AND revoked_at IS NULL")
            .bind(session_id)
            .execute(&self.core.db)
            .await?;
        Ok(())
    }

    /// `Set-Cookie` for the parked tokens of a browser (clears it when none).
    pub(crate) fn parked_cookie(&self, tokens: &[String]) -> String {
        let value: Vec<&str> = tokens.iter().filter(|t| cookie_safe(t)).map(String::as_str).collect();
        let max_age = if value.is_empty() { 0 } else { self.cfg.session_days * 24 * 3600 };
        self.cookie(PARKED_COOKIE, &value.join(&TOKEN_SEPARATOR.to_string()), max_age)
    }

    /// Whether switching to `session` must ask for the account's second factor.
    pub(crate) fn switch_needs_mfa(&self, user: &UserRow, session: &LiveSession) -> bool {
        let window = chrono::Duration::hours(self.cfg.switch_mfa_hours.max(0));
        user.totp_enabled_at.is_some() && session.mfa_at.is_none_or(|at| Utc::now() - at > window)
    }

    /// Before a sign-in with `add_account`: the accounts already on this
    /// device that stay (the active one is parked), and refuses past the
    /// cap. A session of the same account is signed out (it is being
    /// replaced). Returns the tokens of the browser's parked accounts after
    /// the sign-in (web) and whether other accounts remain on the device.
    pub(crate) async fn make_room_for(
        &self,
        user: &UserRow,
        meta: &ClientMeta,
        current: Option<Uuid>,
        device_id: &str,
    ) -> AppResult<(Vec<String>, bool)> {
        let limit = self.cfg.device_account_limit.max(1) as usize;
        if meta.is_web {
            let tokens = DeviceTokens::from_cookies(meta);
            let mut all: Vec<String> = tokens.active.iter().cloned().collect();
            all.extend(tokens.parked.iter().cloned());
            let live = self.live_sessions(&all).await?;
            let mut keep = Vec::new();
            for session in live {
                if session.user_id == user.id {
                    self.revoke_session(user.id, session.id).await?;
                } else {
                    keep.push(session);
                }
            }
            ensure!(keep.len() < limit, precondition, "TOO_MANY_ACCOUNTS", "this device already has {limit} accounts");
            for session in &keep {
                if session.parked_at.is_none() {
                    self.park_session(session.id).await?;
                }
            }
            let others = !keep.is_empty();
            return Ok((keep.into_iter().map(|s| s.token).collect(), others));
        }
        // Native: the device's other accounts are counted by its install ID.
        if !device_id.is_empty() {
            let others: i64 = sqlx::query_scalar(
                "SELECT count(DISTINCT user_id) FROM sessions
                 WHERE device_id = $1 AND user_id <> $2 AND revoked_at IS NULL AND expires_at > now()",
            )
            .bind(device_id)
            .bind(user.id)
            .fetch_one(&self.core.db)
            .await?;
            ensure!(
                (others as usize) < limit,
                precondition,
                "TOO_MANY_ACCOUNTS",
                "this device already has {limit} accounts"
            );
        }
        let mut others = false;
        if let Some(current) = current {
            let owner: Option<Uuid> =
                sqlx::query_scalar("SELECT user_id FROM sessions WHERE id = $1 AND revoked_at IS NULL")
                    .bind(current)
                    .fetch_optional(&self.core.db)
                    .await?;
            if owner.is_some_and(|owner| owner != user.id) {
                self.park_session(current).await?;
                others = true;
            }
        }
        Ok((Vec::new(), others))
    }

    /// The accounts of this device, the active one first.
    pub(crate) async fn device_accounts(
        &self,
        meta: &ClientMeta,
        current: Option<Uuid>,
        native_parked: &[String],
    ) -> AppResult<(Vec<DeviceAccount>, Option<String>)> {
        self.core.cache.rate_limit(&format!("device-accounts:{}", meta.ip), 120, Duration::from_secs(60)).await?;
        let (tokens, web_cookie) = if meta.is_web {
            let tokens = DeviceTokens::from_cookies(meta);
            let mut all: Vec<String> = tokens.active.iter().cloned().collect();
            all.extend(tokens.parked.iter().cloned());
            (all, Some(tokens))
        } else {
            (native_parked.iter().take(self.cfg.device_account_limit.max(1) as usize).cloned().collect(), None)
        };
        let mut live = self.live_sessions(&tokens).await?;
        let mut rewritten = None;
        if let Some(cookie) = &web_cookie {
            let parked_live: Vec<String> =
                live.iter().filter(|s| Some(&s.token) != cookie.active.as_ref()).map(|s| s.token.clone()).collect();
            if parked_live.len() != cookie.parked.len() {
                rewritten = Some(self.parked_cookie(&parked_live));
            }
        }
        let active_session = match (&web_cookie, current) {
            (Some(cookie), _) => live.iter().find(|s| Some(&s.token) == cookie.active.as_ref()).map(|s| s.id),
            (None, current) => current,
        };
        if web_cookie.is_none()
            && let Some(current) = current
            && !live.iter().any(|s| s.id == current)
        {
            // The native caller itself, from its access token.
            let row: Option<(Uuid, Option<DateTime<Utc>>, DateTime<Utc>)> = sqlx::query_as(
                "SELECT user_id, mfa_at, last_active_at FROM sessions WHERE id = $1 AND revoked_at IS NULL AND expires_at > now()",
            )
            .bind(current)
            .fetch_optional(&self.core.db)
            .await?;
            if let Some((user_id, mfa_at, last_active_at)) = row {
                live.push(LiveSession {
                    id: current,
                    user_id,
                    parked_at: None,
                    mfa_at,
                    last_active_at,
                    token: String::new(),
                });
            }
        }
        let ids: Vec<Uuid> = live.iter().map(|s| s.user_id).collect();
        let summaries = users::summaries(&self.core, &ids).await?;
        let mut users_by_id = std::collections::HashMap::new();
        for id in &ids {
            if let Some(user) = users::find_by_id(&self.core, *id).await? {
                users_by_id.insert(*id, user);
            }
        }
        live.sort_by_key(|s| (Some(s.id) != active_session, std::cmp::Reverse(s.last_active_at)));
        let accounts = live
            .iter()
            .filter_map(|session| {
                let user = users_by_id.get(&session.user_id).filter(|u| u.deleted_at.is_none())?;
                Some(DeviceAccount {
                    session_id: session.id.to_string(),
                    user: summaries.get(&session.user_id).cloned().into(),
                    active: Some(session.id) == active_session,
                    mfa_required: Some(session.id) != active_session && self.switch_needs_mfa(user, session),
                    ..Default::default()
                })
            })
            .collect();
        Ok((accounts, rewritten))
    }

    /// `SwitchAccount`.
    pub(crate) async fn switch_account(
        &self,
        meta: &ClientMeta,
        current: Option<Uuid>,
        session_id: &str,
        native_token: &str,
        totp_code: &str,
    ) -> AppResult<Response<AuthResponse>> {
        let core = &self.core;
        core.cache.rate_limit(&format!("switch:ip:{}", meta.ip), 30, Duration::from_secs(300)).await?;
        let not_on_device = || AppError::not_found("ACCOUNT_NOT_ON_DEVICE", "sign in to that account again");
        let cookies = meta.is_web.then(|| DeviceTokens::from_cookies(meta));
        let target_token = match &cookies {
            Some(cookies) => {
                let wanted = parse_id(session_id).map_err(|_| not_on_device())?;
                let live = self.live_sessions(&cookies.parked).await?;
                live.into_iter().find(|s| s.id == wanted).map(|s| s.token).ok_or_else(not_on_device)?
            }
            None => native_token.to_owned(),
        };
        let target = self
            .live_sessions(std::slice::from_ref(&target_token))
            .await?
            .into_iter()
            .next()
            .ok_or_else(not_on_device)?;
        ensure!(
            Some(target.id) != current,
            precondition,
            "ACCOUNT_ALREADY_ACTIVE",
            "this account is already the active one"
        );
        core.cache.rate_limit(&format!("switch:session:{}", target.id), 10, Duration::from_secs(300)).await?;
        let user = users::get(core, target.user_id).await.map_err(|_| not_on_device())?;
        ensure!(user.deleted_at.is_none(), not_found, "ACCOUNT_NOT_ON_DEVICE", "sign in to that account again");
        ensure!(user.banned_at.is_none(), denied, "ACCOUNT_BANNED", "this account has been suspended");

        let mut mfa_passed = false;
        if self.switch_needs_mfa(&user, &target) {
            if totp_code.trim().is_empty() {
                return Ok(Response::new(AuthResponse { mfa_required: true, ..Default::default() }));
            }
            ensure!(
                self.second_factor(&user, totp_code, meta).await?,
                unauthenticated,
                "INVALID_TOTP",
                "wrong authentication code"
            );
            mfa_passed = true;
        }

        let (issued, user) = self.refresh_session_with(&target_token, meta, true).await?;
        sqlx::query(
            "UPDATE sessions SET parked_at = NULL, switched_at = now(), mfa_at = CASE WHEN $2 THEN now() ELSE mfa_at END
             WHERE id = $1",
        )
        .bind(issued.session_id)
        .bind(mfa_passed)
        .execute(&core.db)
        .await?;

        // The account that was active is parked beside it.
        let mut parked_after: Vec<String> = Vec::new();
        match &cookies {
            Some(cookies) => {
                let mut rest: Vec<String> = cookies.active.iter().cloned().collect();
                rest.extend(cookies.parked.iter().filter(|t| **t != target_token).cloned());
                for session in self.live_sessions(&rest).await? {
                    if session.id == issued.session_id || session.user_id == user.id {
                        continue;
                    }
                    if session.parked_at.is_none() {
                        self.park_session(session.id).await?;
                    }
                    parked_after.push(session.token);
                }
            }
            None => {
                if let Some(current) = current {
                    self.park_session(current).await?;
                }
            }
        }
        self.log_security_event(
            user.id,
            "ACCOUNT_SWITCHED",
            meta,
            "",
            serde_json::json!({ "second_factor": mfa_passed }),
        )
        .await;
        let response = self.auth_response(issued, &user, meta).await?;
        Ok(if meta.is_web { response.with_header("set-cookie", self.parked_cookie(&parked_after)) } else { response })
    }

    /// `RemoveDeviceAccount`: signs a parked account out of this device.
    pub(crate) async fn remove_device_account(
        &self,
        meta: &ClientMeta,
        session_id: &str,
        native_token: &str,
    ) -> AppResult<Option<String>> {
        self.core.cache.rate_limit(&format!("device-accounts:{}", meta.ip), 120, Duration::from_secs(60)).await?;
        let not_on_device = || AppError::not_found("ACCOUNT_NOT_ON_DEVICE", "that account is not on this device");
        if meta.is_web {
            let cookies = DeviceTokens::from_cookies(meta);
            let wanted = parse_id(session_id).map_err(|_| not_on_device())?;
            let live = self.live_sessions(&cookies.parked).await?;
            let target = live.iter().find(|s| s.id == wanted).ok_or_else(not_on_device)?;
            self.revoke_session(target.user_id, target.id).await?;
            self.log_security_event(target.user_id, "SIGN_OUT", meta, "", serde_json::json!({ "all": false })).await;
            let rest: Vec<String> = live.iter().filter(|s| s.id != wanted).map(|s| s.token.clone()).collect();
            return Ok(Some(self.parked_cookie(&rest)));
        }
        let target =
            self.live_sessions(&[native_token.to_owned()]).await?.into_iter().next().ok_or_else(not_on_device)?;
        self.revoke_session(target.user_id, target.id).await?;
        self.log_security_event(target.user_id, "SIGN_OUT", meta, "", serde_json::json!({ "all": false })).await;
        Ok(None)
    }

    /// Checks a TOTP or recovery code of `user` (a recovery code works once),
    /// under the TOTP attempt limits; a wrong code is logged.
    pub(crate) async fn second_factor(&self, user: &UserRow, code: &str, meta: &ClientMeta) -> AppResult<bool> {
        self.limit_totp_attempts(user.id).await?;
        let secret = user.totp_secret.as_deref().and_then(|s| totp::open_secret(&self.core.keys, s));
        if let Some(secret) = secret.as_deref()
            && self.accept_totp(user.id, secret, code).await?
        {
            return Ok(true);
        }
        let used: Option<Uuid> = sqlx::query_scalar(
            "UPDATE users SET recovery_code_hashes = array_remove(recovery_code_hashes, $2)
             WHERE id = $1 AND $2 = ANY(recovery_code_hashes) RETURNING id",
        )
        .bind(user.id)
        .bind(totp::hash_recovery_code(&self.core.keys, code))
        .fetch_optional(&self.core.db)
        .await?;
        if used.is_some() {
            self.log_security_event(user.id, "RECOVERY_CODE_USED", meta, "", serde_json::json!({})).await;
            return Ok(true);
        }
        self.log_security_event(user.id, "SIGN_IN_FAILED", meta, "", serde_json::json!({ "factor": "totp" })).await;
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parked_tokens_come_from_their_cookie() {
        let meta = ClientMeta {
            cookies: "tokuly_rt=aaa.bbb; tokuly_ra=ccc.ddd~eee.fff~; theme=dark".into(),
            is_web: true,
            ..Default::default()
        };
        let tokens = DeviceTokens::from_cookies(&meta);
        assert_eq!(tokens.active.as_deref(), Some("aaa.bbb"));
        assert_eq!(tokens.parked, ["ccc.ddd", "eee.fff"]);
        assert!(DeviceTokens::from_cookies(&ClientMeta::default()).parked.is_empty());
    }

    #[test]
    fn only_token_characters_reach_a_cookie() {
        assert!(cookie_safe("0190abcd.Zx-_9"));
        assert!(!cookie_safe("a;b"));
        assert!(!cookie_safe("a~b"));
        assert!(!cookie_safe(""));
    }
}
