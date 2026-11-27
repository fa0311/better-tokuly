//! Personal access tokens and bot tokens (`dev_tokens`), and `TokenService`.
//!
//! A token is shown once, when it is created, and stored only as a keyed
//! hash. It dies when it is revoked, when it expires, when the account it
//! acts as or its creator is banned or deleted, and when its creator's
//! password changes (see `verify.rs` for how that is noticed).
use std::time::Duration;

use chrono::Utc;
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use sqlx::PgConnection;
use tokuly_core::{
    api_tokens::{ApiTokenKind, Scopes},
    events,
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::dev::*;

use crate::{
    Dev, clean_name,
    events::TokenEvent,
    lock_limit, scopes,
    secrets::{self, SecretKind},
};

const MAX_PAGE_SIZE: i64 = 100;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct TokenRow {
    pub id: Uuid,
    pub kind: String,
    pub user_id: Uuid,
    pub name: String,
    pub hint: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<Time>,
    pub last_used_at: Option<Time>,
    pub last_used_ip: String,
    pub created_at: Time,
}

macro_rules! token_columns {
    () => {
        "id, kind, user_id, name, hint, scopes, expires_at, last_used_at, last_used_ip, created_at"
    };
}

impl TokenRow {
    pub(crate) fn to_proto(&self) -> AccessToken {
        AccessToken {
            id: self.id.to_string(),
            name: self.name.clone(),
            scopes: self.scopes.clone(),
            hint: self.hint.clone(),
            created_at: ts(self.created_at),
            expires_at: opt_ts(self.expires_at),
            last_used_at: opt_ts(self.last_used_at),
            last_used_ip: self.last_used_ip.clone(),
            expired: self.expires_at.is_some_and(|at| at <= Utc::now()),
            ..Default::default()
        }
    }
}

/// When a token created now with this choice expires.
pub(crate) fn expiry_time(expiry: buffa::EnumValue<TokenExpiry>) -> AppResult<Option<Time>> {
    let days = match expiry.as_known() {
        Some(TokenExpiry::TOKEN_EXPIRY_7_DAYS) => 7,
        Some(TokenExpiry::TOKEN_EXPIRY_UNSPECIFIED | TokenExpiry::TOKEN_EXPIRY_30_DAYS) => 30,
        Some(TokenExpiry::TOKEN_EXPIRY_90_DAYS) => 90,
        Some(TokenExpiry::TOKEN_EXPIRY_1_YEAR) => 365,
        Some(TokenExpiry::TOKEN_EXPIRY_NEVER) => return Ok(None),
        None => bail!(invalid, "EXPIRY_INVALID", "unknown expiry"),
    };
    Ok(Some(Utc::now() + chrono::Duration::days(days)))
}

/// What a new token is to be.
pub(crate) struct NewToken<'a> {
    pub kind: SecretKind,
    /// The account it acts as.
    pub user_id: Uuid,
    /// The person creating it.
    pub created_by: Uuid,
    pub name: &'a str,
    pub scopes: &'a [String],
    pub expiry: buffa::EnumValue<TokenExpiry>,
    /// How many live tokens the account may have.
    pub limit: i64,
}

impl Dev {
    /// Creates a personal or bot token and returns it with its secret.
    pub(crate) async fn create_token(&self, tx: &mut PgConnection, new: NewToken<'_>) -> AppResult<(TokenRow, String)> {
        let name = clean_name(new.name, 60, "TOKEN_NAME_INVALID", "the token name")?;
        let scopes = scopes::validate(new.scopes)?;
        if new.kind == SecretKind::Bot {
            let (owner_email, owner_password): (Option<String>, Option<String>) =
                sqlx::query_as("SELECT email::text, password_hash FROM users WHERE id = $1")
                    .bind(new.created_by)
                    .fetch_one(&mut *tx)
                    .await?;
            // Anyone can sign up with an address nobody verified, so the
            // address alone does not make a demo operator: the account must
            // also have the demo's password. (Its tokens die when the
            // password changes, and `verify` checks the address on every use.)
            let demo = self.cfg.is_demo_operator(owner_email.as_deref())
                && tokuly_identity::password::verify(&self.cfg.demo_password, owner_password.as_deref()).await;
            if let Some(scope) = scopes.as_slice().iter().find(|scope| !scopes::allowed_for_bots(scope, demo)) {
                bail!(invalid, "SCOPE_NOT_FOR_BOTS", "a bot token cannot hold {scope}");
            }
        }
        let expires_at = expiry_time(new.expiry)?;
        let kind = if new.kind == SecretKind::Bot { "bot" } else { "pat" };
        lock_limit(&mut *tx, "dev-tokens", new.user_id).await?;
        let live: i64 = sqlx::query_scalar("SELECT count(*) FROM dev_tokens WHERE user_id = $1 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now())")
            .bind(new.user_id)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(
            live < new.limit,
            precondition,
            "TOO_MANY_TOKENS",
            "at most {} tokens at a time; revoke one first",
            new.limit
        );
        let creator_password: Option<String> = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
            .bind(new.created_by)
            .fetch_one(&mut *tx)
            .await?;
        let secret = secrets::generate(new.kind);
        let row: TokenRow = sqlx::query_as(concat!(
            "INSERT INTO dev_tokens (kind, user_id, created_by, name, token_hash, hint, scopes, password_mark, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING ",
            token_columns!()
        ))
        .bind(kind)
        .bind(new.user_id)
        .bind(new.created_by)
        .bind(&name)
        .bind(self.core.keys.token_hash(&secret))
        .bind(secrets::hint(&secret))
        .bind(scopes.as_slice())
        .bind(self.password_mark(creator_password.as_deref()))
        .bind(expires_at)
        .fetch_one(&mut *tx)
        .await?;
        let event = TokenEvent { token_id: row.id, user_id: row.user_id, kind: kind.to_owned(), reason: String::new() };
        events::emit(&mut *tx, "dev.token.created", Some(new.created_by), &event).await?;
        Ok((row, secret))
    }

    /// Revokes the live tokens matched by `filter` (a condition on
    /// `dev_tokens` taking `$1`), at once. `made_before` limits it to tokens
    /// that existed at that moment: an event that arrives late must not
    /// revoke what was created after the thing it reports.
    async fn revoke_tokens_where(
        &self,
        filter: &'static str,
        id: Uuid,
        reason: &str,
        actor: Option<Uuid>,
        made_before: Option<Time>,
    ) -> AppResult<Vec<TokenRow>> {
        let mut tx = self.core.db.begin().await?;
        let rows = Self::revoke_tokens_in(&mut tx, filter, id, reason, actor, made_before).await?;
        tx.commit().await?;
        let (hashes, tokens): (Vec<String>, Vec<TokenRow>) = rows.into_iter().unzip();
        self.forget_tokens(&hashes).await?;
        Ok(tokens)
    }

    /// The revocation of [`Self::revoke_tokens_where`] on a transaction of
    /// the caller's, with its events; returns each token's hash and row. The
    /// caller forgets the hashes ([`Self::forget_tokens`]) after the commit.
    pub(crate) async fn revoke_tokens_in(
        tx: &mut sqlx::PgConnection,
        filter: &'static str,
        id: Uuid,
        reason: &str,
        actor: Option<Uuid>,
        made_before: Option<Time>,
    ) -> AppResult<Vec<(String, TokenRow)>> {
        let rows: Vec<(String, TokenRow)> = sqlx::query_as::<_, RevokedRow>(sqlx::AssertSqlSafe(format!(
            "UPDATE dev_tokens SET revoked_at = now(), revoked_reason = $2
             WHERE {filter} AND revoked_at IS NULL AND ($3::timestamptz IS NULL OR created_at <= $3) RETURNING token_hash, {}",
            token_columns!()
        )))
        .bind(id)
        .bind(reason)
        .bind(made_before)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|row| (row.token_hash, row.token))
        .collect();
        for (_, token) in &rows {
            let event = TokenEvent {
                token_id: token.id,
                user_id: token.user_id,
                kind: token.kind.clone(),
                reason: reason.to_owned(),
            };
            events::emit(&mut *tx, "dev.token.revoked", actor, &event).await?;
        }
        Ok(rows)
    }

    /// Revokes every live token that existed at `made_before`, a few
    /// hundred per statement (`tokuly sessions revoke-all`); returns how many.
    pub(crate) async fn revoke_every_api_token(&self, made_before: Time) -> AppResult<u64> {
        const NEXT_BATCH: &str = "id IN (SELECT id FROM dev_tokens
             WHERE id > $1 AND revoked_at IS NULL AND created_at <= $3 AND (expires_at IS NULL OR expires_at > now())
             ORDER BY id LIMIT 500)";
        let (mut after, mut revoked) = (Uuid::nil(), 0);
        loop {
            let batch = self.revoke_tokens_where(NEXT_BATCH, after, "operator", None, Some(made_before)).await?;
            let Some(last) = batch.iter().map(|token| token.id).max() else { return Ok(revoked) };
            (after, revoked) = (last, revoked + batch.len() as u64);
        }
    }

    /// Revokes one token on its owner's say-so.
    pub(crate) async fn revoke_token(&self, token_id: Uuid, actor: Uuid) -> AppResult<Option<TokenRow>> {
        Ok(self.revoke_tokens_where("id = $1", token_id, "owner", Some(actor), None).await?.pop())
    }

    /// Revokes every token `user_id` created — their own and their bots' —
    /// that existed at `made_before` (all of them when `None`).
    pub(crate) async fn revoke_tokens_created_by(
        &self,
        user_id: Uuid,
        reason: &str,
        made_before: Option<Time>,
    ) -> AppResult<usize> {
        Ok(self.revoke_tokens_where("created_by = $1", user_id, reason, None, made_before).await?.len())
    }

    /// Revokes the tokens `creator` made under another password than the
    /// current one (`current_mark`, see `Dev::password_mark`). Tokens made
    /// since the change carry the current mark and are left alone.
    pub(crate) async fn revoke_tokens_with_stale_password(
        &self,
        creator: Uuid,
        current_mark: Option<String>,
    ) -> AppResult<usize> {
        let mut tx = self.core.db.begin().await?;
        let rows: Vec<RevokedRow> = sqlx::query_as(concat!(
            "UPDATE dev_tokens SET revoked_at = now(), revoked_reason = 'password_changed'
             WHERE created_by = $1 AND revoked_at IS NULL AND password_mark IS DISTINCT FROM $2 RETURNING token_hash, ",
            token_columns!()
        ))
        .bind(creator)
        .bind(current_mark)
        .fetch_all(&mut *tx)
        .await?;
        for row in &rows {
            let event = TokenEvent {
                token_id: row.token.id,
                user_id: row.token.user_id,
                kind: row.token.kind.clone(),
                reason: "password_changed".to_owned(),
            };
            events::emit(&mut *tx, "dev.token.revoked", None, &event).await?;
        }
        tx.commit().await?;
        let hashes: Vec<String> = rows.into_iter().map(|row| row.token_hash).collect();
        self.forget_tokens(&hashes).await?;
        Ok(hashes.len())
    }

    /// Revokes every token that acts as `user_id`.
    pub(crate) async fn revoke_tokens_of(
        &self,
        user_id: Uuid,
        reason: &str,
        made_before: Option<Time>,
    ) -> AppResult<usize> {
        Ok(self.revoke_tokens_where("user_id = $1", user_id, reason, None, made_before).await?.len())
    }

    /// The live tokens of `kind` acting as `user_id`, newest first.
    pub(crate) async fn list_tokens(&self, user_id: Uuid, kind: &str, page: &Page) -> AppResult<Vec<TokenRow>> {
        let after: Option<Uuid> = page.decode()?;
        Ok(sqlx::query_as(concat!(
            "SELECT ",
            token_columns!(),
            " FROM dev_tokens WHERE user_id = $1 AND kind = $2 AND revoked_at IS NULL AND ($3::uuid IS NULL OR id < $3) ORDER BY id DESC LIMIT $4"
        ))
        .bind(user_id)
        .bind(kind)
        .bind(after)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?)
    }

    /// A live token of `kind` that `owner` created.
    pub(crate) async fn owned_token(&self, token_id: Uuid, owner: Uuid, kind: &str) -> AppResult<TokenRow> {
        sqlx::query_as(concat!(
            "SELECT ",
            token_columns!(),
            " FROM dev_tokens WHERE id = $1 AND created_by = $2 AND kind = $3 AND revoked_at IS NULL"
        ))
        .bind(token_id)
        .bind(owner)
        .bind(kind)
        .fetch_optional(&self.core.db)
        .await?
        .or_not_found("TOKEN_NOT_FOUND", "token not found")
    }
}

#[derive(sqlx::FromRow)]
struct RevokedRow {
    token_hash: String,
    #[sqlx(flatten)]
    token: TokenRow,
}

/// Tells the account that a token now exists for it: the one message that
/// reaches the owner if someone else created it.
fn token_created_notice(user_id: Uuid, token: &TokenRow) -> NotificationRequest {
    NotificationRequest::new(
        user_id,
        "dev.token_created",
        Localized::new("アクセストークンが作成されました", "An access token was created"),
        Localized::new(
            format!(
                "「{}」があなたのアカウントにアクセスできます。心当たりがない場合はすぐに取り消してください！",
                token.name
            ),
            format!("\"{}\" can now access your account. If this was not you, revoke it right away.", token.name),
        ),
        "/settings/developer/tokens",
    )
    .data(serde_json::json!({ "token_id": token.id, "scopes": token.scopes }))
}

impl TokenService for Dev {
    async fn list_scopes(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, ListScopesRequest>,
    ) -> ServiceResult<ListScopesResponse> {
        handle(async { ok(ListScopesResponse { scopes: scopes::catalogue(), ..Default::default() }) }).await
    }

    async fn create_token(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateTokenRequest>,
    ) -> ServiceResult<CreateTokenResponse> {
        handle(async {
            let user_id = ctx.require_session()?.user_id;
            let req = request.to_owned_message();
            self.core.cache.rate_limit(&format!("dev-token-create:{user_id}"), 20, Duration::from_secs(3600)).await?;
            let mut tx = self.core.db.begin().await?;
            let new = NewToken {
                kind: SecretKind::Personal,
                user_id,
                created_by: user_id,
                name: &req.name,
                scopes: &req.scopes,
                expiry: req.expiry,
                limit: self.cfg.max_tokens,
            };
            let (token, secret) = self.create_token(&mut tx, new).await?;
            outbound::notify(&mut *tx, &token_created_notice(user_id, &token)).await?;
            tx.commit().await?;
            let attrs = serde_json::json!({ "token_id": token.id, "name": token.name, "scopes": token.scopes });
            self.identity.log_security_event(user_id, "API_TOKEN_CREATED", &ctx.client(), "", attrs).await;
            ok(CreateTokenResponse { token: token.to_proto().into(), secret, ..Default::default() })
        })
        .await
    }

    async fn list_tokens(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListTokensRequest>,
    ) -> ServiceResult<ListTokensResponse> {
        handle(async {
            let user_id = ctx.require_session()?.user_id;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE_SIZE);
            let mut rows = self.list_tokens(user_id, "pat", &page).await?;
            let info = page.finish(&mut rows, |row| row.id);
            ok(ListTokensResponse {
                tokens: rows.iter().map(TokenRow::to_proto).collect(),
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn revoke_token(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RevokeTokenRequest>,
    ) -> ServiceResult<RevokeTokenResponse> {
        handle(async {
            let user_id = ctx.require_session()?.user_id;
            let token = self.owned_token(parse_id(request.token_id)?, user_id, "pat").await?;
            self.revoke_token(token.id, user_id).await?;
            let attrs = serde_json::json!({ "token_id": token.id, "name": token.name });
            self.identity.log_security_event(user_id, "API_TOKEN_REVOKED", &ctx.client(), "", attrs).await;
            ok(RevokeTokenResponse::default())
        })
        .await
    }

    async fn get_current_token(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetCurrentTokenRequest>,
    ) -> ServiceResult<GetCurrentTokenResponse> {
        handle(async {
            let auth = ctx.auth()?;
            let user = tokuly_identity::summary(&self.core, auth.user_id).await?;
            let Some(token) = &auth.api_token else {
                return ok(GetCurrentTokenResponse {
                    kind: CredentialKind::CREDENTIAL_KIND_SESSION.into(),
                    user: user.into(),
                    ..Default::default()
                });
            };
            let (kind, app) = match token.kind {
                ApiTokenKind::Personal => (CredentialKind::CREDENTIAL_KIND_PERSONAL_TOKEN, None),
                ApiTokenKind::Bot => (CredentialKind::CREDENTIAL_KIND_BOT, None),
                ApiTokenKind::OAuth => {
                    let app = match token.app_id {
                        Some(app_id) => self.find_public_app(app_id).await?,
                        None => None,
                    };
                    (CredentialKind::CREDENTIAL_KIND_APP, app)
                }
            };
            ok(GetCurrentTokenResponse {
                kind: kind.into(),
                user: user.into(),
                scopes: Scopes::as_slice(&token.scopes).to_vec(),
                expires_at: opt_ts(token.expires_at),
                app: app.into(),
                rate_limit_per_minute: token.rate.per_minute.min(i32::MAX as u32) as i32,
                credential_id: token.id.to_string(),
                ..Default::default()
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_choices_map_to_days() {
        let days = |expiry: TokenExpiry| {
            expiry_time(expiry.into()).unwrap().map(|at| (at - Utc::now()).num_hours() as f64 / 24.0)
        };
        let close = |actual: Option<f64>, expected: f64| actual.is_some_and(|d| (d - expected).abs() < 0.1);
        assert!(close(days(TokenExpiry::TOKEN_EXPIRY_7_DAYS), 7.0));
        assert!(close(days(TokenExpiry::TOKEN_EXPIRY_30_DAYS), 30.0));
        assert!(close(days(TokenExpiry::TOKEN_EXPIRY_UNSPECIFIED), 30.0), "unspecified means 30 days, not forever");
        assert!(close(days(TokenExpiry::TOKEN_EXPIRY_90_DAYS), 90.0));
        assert!(close(days(TokenExpiry::TOKEN_EXPIRY_1_YEAR), 365.0));
        assert_eq!(days(TokenExpiry::TOKEN_EXPIRY_NEVER), None);
        assert_eq!(expiry_time(buffa::EnumValue::from(99)).unwrap_err().reason, "EXPIRY_INVALID");
    }
}
