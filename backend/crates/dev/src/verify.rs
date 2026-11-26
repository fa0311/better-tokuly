//! The API token verifier: what core's auth middleware asks on every
//! request made with a `tkl_…` bearer token.
//!
//! The answer comes from Valkey (`dev:tok:<keyed hash>`); the database is
//! read at most once a minute per token, and never for a string that is not
//! a well-formed token (the checksum settles that for free). Everything
//! that ends a token's life — revocation, a ban, a deleted application —
//! removes the cached answer, so it takes effect at once rather than when
//! the entry expires.
//!
//! Removing the entry is not enough on its own: a request that read the
//! database a moment before the revocation would write its "valid" back a
//! moment after. So a change also leaves a marker (`dev:tok-changed:…`)
//! that outlives any cached answer; while it is there, the cache is not
//! believed and the token is judged against the database on every use.
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokuly_core::{
    analytics::AnalyticsEvent,
    api_tokens::{ApiCall, ApiToken, ApiTokenKind, ApiTokenVerifier, Scopes, TokenRejection},
    prelude::*,
};

use crate::{
    Dev,
    secrets::{self, SecretKind},
};

/// How long a valid answer is trusted before the database is asked again.
/// Bounds how long a change that is not announced here (an account
/// suspended by identity, a password change) takes to reach a token.
const VALID_TTL: Duration = Duration::from_secs(60);
/// Unknown strings are remembered briefly, so guessing does not reach the database.
const INVALID_TTL: Duration = Duration::from_secs(30);
/// Expired and revoked tokens stay that way.
const DEAD_TTL: Duration = Duration::from_secs(300);

/// Analytics event recorded for every request made with a token.
pub(crate) const USAGE_EVENT: &str = "dev.api.request";

/// A cached answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Verdict {
    Valid(ApiToken),
    Invalid,
    Expired,
    Revoked,
}

impl Verdict {
    fn ttl(&self) -> Duration {
        match self {
            Self::Valid(token) => match token.expires_at {
                Some(at) => (at - Utc::now()).to_std().unwrap_or_default().clamp(Duration::from_secs(1), VALID_TTL),
                None => VALID_TTL,
            },
            Self::Invalid => INVALID_TTL,
            Self::Expired | Self::Revoked => DEAD_TTL,
        }
    }

    fn into_result(self) -> Result<ApiToken, TokenRejection> {
        match self {
            // The entry may outlive the token by a moment.
            Self::Valid(token) if token.expires_at.is_some_and(|at| at <= Utc::now()) => Err(TokenRejection::Expired),
            Self::Valid(token) => Ok(token),
            Self::Invalid => Err(TokenRejection::Invalid),
            Self::Expired => Err(TokenRejection::Expired),
            Self::Revoked => Err(TokenRejection::Revoked),
        }
    }
}

fn cache_key(token_hash: &str) -> String {
    format!("dev:tok:{token_hash}")
}

/// Says that something about the token changed recently: the cached answer
/// may be older than the change.
fn changed_key(token_hash: &str) -> String {
    format!("dev:tok-changed:{token_hash}")
}

/// How long a changed token is judged against the database on every use.
/// Longer than any cached answer lives, so one written by a request that
/// raced the change has expired by the time the cache is believed again.
const CHANGED_SECONDS: u64 = 120;

#[derive(sqlx::FromRow)]
struct TokenLookup {
    id: Uuid,
    kind: String,
    user_id: Uuid,
    created_by: Uuid,
    scopes: Vec<String>,
    rate_tier: String,
    password_mark: Option<String>,
    expires_at: Option<Time>,
    revoked: bool,
    account_gone: bool,
    creator_gone: bool,
    creator_password_hash: Option<String>,
    /// For a bot token the owner's address (a token of a handed-over bot is
    /// revoked): what makes a bot a demo bot.
    creator_email: Option<String>,
    bot_disabled: bool,
    /// A bot token made by someone who no longer owns the bot (it was handed over).
    creator_not_owner: bool,
}

#[derive(sqlx::FromRow)]
struct AccessLookup {
    authorization_id: Uuid,
    user_id: Uuid,
    app_id: Uuid,
    scopes: Vec<String>,
    access_expires_at: Time,
    revoked: bool,
    app_active: bool,
    rate_tier: String,
    account_gone: bool,
    /// The application's owner is suspended or deleted.
    owner_gone: bool,
}

impl Dev {
    /// A fingerprint of an account's password hash. A token remembers the
    /// one of its creator; a different one later means the password changed.
    pub(crate) fn password_mark(&self, password_hash: Option<&str>) -> Option<String> {
        password_hash.map(|hash| hex::encode(&self.core.keys.mac("dev/password-mark", hash.as_bytes())[..16]))
    }

    /// Judges a personal access token or bot token against the database.
    async fn judge_token(&self, token_hash: &str) -> AppResult<Verdict> {
        let row: Option<TokenLookup> = sqlx::query_as(
            "SELECT t.id, t.kind, t.user_id, t.created_by, t.scopes, t.rate_tier, t.password_mark, t.expires_at,
                    t.revoked_at IS NOT NULL AS revoked,
                    (u.banned_at IS NOT NULL OR u.deleted_at IS NOT NULL) AS account_gone,
                    (c.banned_at IS NOT NULL OR c.deleted_at IS NOT NULL) AS creator_gone,
                    c.password_hash AS creator_password_hash,
                    c.email::text AS creator_email,
                    COALESCE(b.disabled_at IS NOT NULL, false) AS bot_disabled,
                    COALESCE(t.kind = 'bot' AND b.owner_id <> t.created_by, false) AS creator_not_owner
             FROM dev_tokens t
             JOIN users u ON u.id = t.user_id
             JOIN users c ON c.id = t.created_by
             LEFT JOIN dev_bots b ON b.user_id = t.user_id
             WHERE t.token_hash = $1",
        )
        .bind(token_hash)
        .fetch_optional(&self.core.db)
        .await?;
        let Some(row) = row else { return Ok(Verdict::Invalid) };
        if row.revoked || row.account_gone || row.creator_gone || row.bot_disabled || row.creator_not_owner {
            return Ok(Verdict::Revoked);
        }
        if row.expires_at.is_some_and(|at| at <= Utc::now()) {
            return Ok(Verdict::Expired);
        }
        let current_mark = self.password_mark(row.creator_password_hash.as_deref());
        if row.password_mark != current_mark {
            // The creator's password changed since the token was made:
            // whoever made the token may no longer be who holds the account.
            // The same goes for every other token made under the old password.
            self.revoke_tokens_with_stale_password(row.created_by, current_mark).await?;
            return Ok(Verdict::Revoked);
        }
        let is_bot = row.kind == "bot";
        // The rule for bots holds whatever a token row says (it may predate the rule).
        let demo = is_bot && self.cfg.is_demo_operator(row.creator_email.as_deref());
        let scopes = row.scopes.into_iter().filter(|scope| !is_bot || crate::scopes::allowed_for_bots(scope, demo));
        Ok(Verdict::Valid(ApiToken {
            kind: if is_bot { ApiTokenKind::Bot } else { ApiTokenKind::Personal },
            id: row.id,
            user_id: row.user_id,
            app_id: None,
            scopes: Scopes::new(scopes),
            expires_at: row.expires_at,
            rate: self.cfg.rate_tier(&row.rate_tier),
        }))
    }

    /// Judges an OAuth access token against the database.
    async fn judge_access_token(&self, access_hash: &str) -> AppResult<Verdict> {
        let row: Option<AccessLookup> = sqlx::query_as(
            "SELECT a.id AS authorization_id, a.user_id, a.app_id, o.scopes, o.access_expires_at,
                    o.revoked_at IS NOT NULL AS revoked,
                    p.status = 'active' AS app_active, p.rate_tier,
                    (u.banned_at IS NOT NULL OR u.deleted_at IS NOT NULL) AS account_gone,
                    (w.banned_at IS NOT NULL OR w.deleted_at IS NOT NULL) AS owner_gone
             FROM dev_oauth_tokens o
             JOIN dev_authorizations a ON a.id = o.authorization_id
             JOIN dev_apps p ON p.id = a.app_id
             JOIN users u ON u.id = a.user_id
             JOIN users w ON w.id = p.owner_id
             WHERE o.access_hash = $1",
        )
        .bind(access_hash)
        .fetch_optional(&self.core.db)
        .await?;
        let Some(row) = row else { return Ok(Verdict::Invalid) };
        if row.revoked || !row.app_active || row.account_gone || row.owner_gone {
            return Ok(Verdict::Revoked);
        }
        if row.access_expires_at <= Utc::now() {
            return Ok(Verdict::Expired);
        }
        Ok(Verdict::Valid(ApiToken {
            kind: ApiTokenKind::OAuth,
            id: row.authorization_id,
            user_id: row.user_id,
            app_id: Some(row.app_id),
            scopes: Scopes::new(row.scopes),
            expires_at: Some(row.access_expires_at),
            rate: self.cfg.rate_tier(&row.rate_tier),
        }))
    }

    async fn verdict(&self, token: &str) -> AppResult<Verdict> {
        let Some(kind @ (SecretKind::Personal | SecretKind::Bot | SecretKind::Access)) = secrets::kind_of(token) else {
            // Not a bearer token at all (malformed, or a refresh token,
            // client secret...): no need to look anywhere.
            return Ok(Verdict::Invalid);
        };
        let hash = self.core.keys.token_hash(token);
        let key = cache_key(&hash);
        // One round trip for the cached answer and the "do not trust it" marker.
        let (cached, changed): (Option<Vec<u8>>, Option<Vec<u8>>) =
            redis::cmd("MGET").arg(&key).arg(changed_key(&hash)).query_async(&mut self.core.cache.conn()).await?;
        if let (Some(verdict), None) =
            (cached.and_then(|bytes| serde_json::from_slice::<Verdict>(&bytes).ok()), changed)
        {
            return Ok(verdict);
        }
        let verdict = match kind {
            SecretKind::Access => self.judge_access_token(&hash).await?,
            _ => self.judge_token(&hash).await?,
        };
        self.core.cache.set_json(&key, &verdict, Some(verdict.ttl())).await?;
        Ok(verdict)
    }

    /// Makes whatever just changed about these tokens apply to their next
    /// request: drops the cached answers and marks the tokens as changed,
    /// so that an answer cached by a request that raced the change is not
    /// believed either.
    pub(crate) async fn forget_tokens(&self, token_hashes: &[String]) -> AppResult<()> {
        if token_hashes.is_empty() {
            return Ok(());
        }
        let mut pipe = redis::pipe();
        for hash in token_hashes {
            pipe.set_ex(changed_key(hash), "1", CHANGED_SECONDS).ignore().del(cache_key(hash)).ignore();
        }
        pipe.query_async::<()>(&mut self.core.cache.conn()).await?;
        Ok(())
    }

    /// Drops the cached answers for every live access token of an application.
    pub(crate) async fn forget_app_tokens(&self, app_id: Uuid) -> AppResult<()> {
        let hashes: Vec<String> = sqlx::query_scalar(
            "SELECT o.access_hash FROM dev_oauth_tokens o JOIN dev_authorizations a ON a.id = o.authorization_id
             WHERE a.app_id = $1 AND o.access_expires_at > now()",
        )
        .bind(app_id)
        .fetch_all(&self.core.db)
        .await?;
        self.forget_tokens(&hashes).await
    }

    /// Drops the cached answers for every token that acts as, or was created by, `user_id`.
    pub(crate) async fn forget_account_tokens(&self, user_id: Uuid) -> AppResult<()> {
        let mut hashes: Vec<String> = sqlx::query_scalar(
            "SELECT token_hash FROM dev_tokens WHERE (user_id = $1 OR created_by = $1) AND revoked_at IS NULL",
        )
        .bind(user_id)
        .fetch_all(&self.core.db)
        .await?;
        let access: Vec<String> = sqlx::query_scalar(
            "SELECT o.access_hash FROM dev_oauth_tokens o JOIN dev_authorizations a ON a.id = o.authorization_id
             WHERE a.user_id = $1 AND o.access_expires_at > now()",
        )
        .bind(user_id)
        .fetch_all(&self.core.db)
        .await?;
        hashes.extend(access);
        self.forget_tokens(&hashes).await
    }

    /// Notes that a credential was used: at most one write a minute.
    async fn touch(&self, token: &ApiToken, ip: &str) -> AppResult<()> {
        if !self.core.cache.set_nx(&format!("dev:used:{}", token.id), "1", Duration::from_secs(60)).await? {
            return Ok(());
        }
        match token.kind {
            ApiTokenKind::OAuth => {
                sqlx::query("UPDATE dev_authorizations SET last_used_at = now() WHERE id = $1")
                    .bind(token.id)
                    .execute(&self.core.db)
                    .await?
            }
            ApiTokenKind::Personal | ApiTokenKind::Bot => {
                sqlx::query("UPDATE dev_tokens SET last_used_at = now(), last_used_ip = $2 WHERE id = $1")
                    .bind(token.id)
                    .bind(ip)
                    .execute(&self.core.db)
                    .await?
            }
        };
        Ok(())
    }
}

/// What usage statistics are kept per: the personal token, the application
/// (all of its users together) or the bot (all of its tokens together).
pub(crate) fn usage_subject(token: &ApiToken) -> (&'static str, Uuid) {
    match token.kind {
        ApiTokenKind::Personal => ("token", token.id),
        ApiTokenKind::OAuth => ("app", token.app_id.unwrap_or(token.id)),
        ApiTokenKind::Bot => ("bot", token.user_id),
    }
}

pub(crate) struct Verifier(pub Dev);

#[async_trait]
impl ApiTokenVerifier for Verifier {
    async fn verify(&self, token: &str) -> Result<ApiToken, TokenRejection> {
        match self.0.verdict(token).await {
            Ok(verdict) => verdict.into_result(),
            Err(e) => {
                tracing::error!(error = %e, "API token could not be verified");
                Err(TokenRejection::Unavailable)
            }
        }
    }

    async fn record(&self, token: &ApiToken, call: ApiCall) {
        let (subject_type, subject_id) = usage_subject(token);
        let event = AnalyticsEvent::new(USAGE_EVENT)
            .user(Some(token.user_id))
            .subject(subject_type, subject_id)
            .value(call.duration.as_secs_f64() * 1000.0)
            .props(serde_json::json!({ "procedure": call.procedure, "error": call.error.clone().unwrap_or_default() }));
        self.0.core.analytics.track(event).await;
        if let Err(e) = self.0.touch(token, &call.ip).await {
            tracing::debug!(error = %e, "could not record token use");
        }
    }
}

#[cfg(test)]
mod tests {
    use tokuly_core::api_tokens::RateTier;

    use super::*;

    fn token(expires_in: Option<i64>) -> ApiToken {
        ApiToken {
            kind: ApiTokenKind::Personal,
            id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
            app_id: None,
            scopes: Scopes::new(["notes:read"]),
            expires_at: expires_in.map(|seconds| Utc::now() + chrono::Duration::seconds(seconds)),
            rate: RateTier { name: "standard".into(), per_minute: 600 },
        }
    }

    #[test]
    fn a_cached_answer_never_outlives_the_token() {
        assert_eq!(Verdict::Valid(token(None)).ttl(), VALID_TTL);
        assert_eq!(Verdict::Valid(token(Some(3600))).ttl(), VALID_TTL);
        assert!(Verdict::Valid(token(Some(10))).ttl() <= Duration::from_secs(10));
        assert_eq!(Verdict::Valid(token(Some(-5))).ttl(), Duration::from_secs(1));
        assert_eq!(Verdict::Valid(token(Some(-5))).into_result(), Err(TokenRejection::Expired));
        assert!(Verdict::Valid(token(Some(30))).into_result().is_ok());
        assert_eq!(Verdict::Revoked.into_result(), Err(TokenRejection::Revoked));
        assert_eq!(Verdict::Invalid.into_result(), Err(TokenRejection::Invalid));
    }

    #[test]
    fn verdicts_survive_the_cache() {
        let valid = Verdict::Valid(token(Some(60)));
        let json = serde_json::to_string(&valid).unwrap();
        assert!(
            matches!(serde_json::from_str::<Verdict>(&json).unwrap(), Verdict::Valid(t) if Ok(&t) == valid.clone().into_result().as_ref())
        );
        assert!(matches!(
            serde_json::from_str::<Verdict>(&serde_json::to_string(&Verdict::Revoked).unwrap()).unwrap(),
            Verdict::Revoked
        ));
    }

    #[test]
    fn usage_is_kept_per_token_application_or_bot() {
        let personal = token(None);
        assert_eq!(usage_subject(&personal), ("token", personal.id));
        let app = Uuid::now_v7();
        let oauth = ApiToken { kind: ApiTokenKind::OAuth, app_id: Some(app), ..token(None) };
        assert_eq!(usage_subject(&oauth), ("app", app));
        let bot = ApiToken { kind: ApiTokenKind::Bot, ..token(None) };
        assert_eq!(usage_subject(&bot), ("bot", bot.user_id));
    }
}
