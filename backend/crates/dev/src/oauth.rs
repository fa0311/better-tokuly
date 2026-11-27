//! The OAuth 2.1 protocol: authorization requests, codes with PKCE, token
//! issue and rotation, revocation and the device flow.
//!
//! Followed here: draft-ietf-oauth-v2-1 (authorization code with PKCE for
//! every client, exact redirect URI matching, refresh token rotation, no
//! implicit or password grant), RFC 7636 (PKCE, `S256` only), RFC 8252
//! (loopback redirects), RFC 9207 (`iss` in the redirect), RFC 7009
//! (revocation) and RFC 8628 (device flow).
//!
//! State that lives for seconds or minutes — authorization codes and
//! pending device codes — is in Valkey; grants and tokens are in
//! PostgreSQL. An authorization (`dev_authorizations`) is the user's
//! standing permission for an application; each code exchange starts a
//! *family* of token pairs under it, and each refresh retires one pair and
//! adds the next. A refresh token presented after it was retired can only
//! be a copy, so the whole family is revoked.
//!
//! Everything that may be used once is taken by one atomic step, so two
//! requests arriving together cannot both win: a code by a script that
//! takes it and leaves a marker, a device's approval by `GETDEL`, a refresh
//! token by an `UPDATE … WHERE rotated_at IS NULL` whose row count decides.
//! Answers never say more than the client could know: a code, token or
//! device code that is unknown, expired, spent or somebody else's gets the
//! same error.
use std::{collections::HashMap, time::Duration};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use sqlx::PgConnection;
use tokuly_core::{
    api_tokens::Scopes,
    events,
    keys::{b64url, secure_eq},
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::dev::{AuthorizationRequest, Consent};

use crate::{
    Dev,
    apps::{AppRow, redirect_uri_matches},
    events::AuthorizationEvent,
    scopes,
    secrets::{self, SecretKind},
};

pub(crate) const GRANT_DEVICE_CODE: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// An error in the vocabulary of RFC 6749 section 5.2 (and RFC 8628).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OAuthError {
    pub error: &'static str,
    pub description: String,
    /// HTTP status of the error response at the token endpoint.
    pub status: u16,
}

impl OAuthError {
    pub(crate) fn new(error: &'static str, description: impl Into<String>) -> Self {
        // RFC 6749 section 5.2: 401 for a client that failed to authenticate, 400 for everything else.
        let status = if error == "invalid_client" { 401 } else { 400 };
        Self { error, description: description.into(), status }
    }

    fn invalid_grant(description: &str) -> Self {
        Self::new("invalid_grant", description)
    }

    /// An error of ours rather than of the request. Too many requests are
    /// answered as such; anything else is logged and reported without detail.
    fn server(error: AppError) -> Self {
        if error.code == connectrpc::ErrorCode::ResourceExhausted {
            return Self { status: 429, ..Self::new("slow_down", "too many requests") };
        }
        tracing::error!(error = %error, "OAuth request failed");
        Self { status: 500, ..Self::new("server_error", "the request could not be processed") }
    }
}

impl From<AppError> for OAuthError {
    fn from(error: AppError) -> Self {
        Self::server(error)
    }
}

impl From<sqlx::Error> for OAuthError {
    fn from(error: sqlx::Error) -> Self {
        Self::server(error.into())
    }
}

pub(crate) type OAuthResult<T> = Result<T, OAuthError>;

/// What the token endpoint answers on success (RFC 6749 section 5.1).
#[derive(Debug, Clone, Serialize)]
pub(crate) struct TokenResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: i64,
    pub refresh_token: String,
    pub scope: String,
}

/// What `/oauth/device_authorization` answers (RFC 8628 section 3.2).
#[derive(Debug, Clone, Serialize)]
pub(crate) struct DeviceAuthorization {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// A client's credentials as presented to an HTTP endpoint.
#[derive(Debug, Default)]
pub(crate) struct ClientCredentials {
    /// From `Authorization: Basic`.
    pub basic: Option<(String, String)>,
    /// `client_id` / `client_secret` of the form body.
    pub body_id: Option<String>,
    pub body_secret: Option<String>,
}

/// An authorization code waiting to be exchanged.
#[derive(Debug, Serialize, Deserialize)]
struct CodeGrant {
    app_id: Uuid,
    authorization_id: Uuid,
    scopes: Vec<String>,
    /// The redirect URI the code was sent to.
    redirect_uri: String,
    code_challenge: String,
}

/// A device waiting for its user (RFC 8628). Written once; the user's
/// answer and the polling pace live in keys of their own, so that a poll
/// and an answer arriving together cannot overwrite each other.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeviceGrant {
    app_id: Uuid,
    scopes: Vec<String>,
    /// Normalized (no dash).
    user_code: String,
}

/// The user's answer to a waiting device, as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceAnswer {
    Approved { user_id: Uuid },
    Denied,
}

impl DeviceAnswer {
    fn encode(self) -> String {
        match self {
            Self::Approved { user_id } => format!("approved:{user_id}"),
            Self::Denied => "denied".to_owned(),
        }
    }

    fn decode(stored: &str) -> Option<Self> {
        match stored.strip_prefix("approved:") {
            Some(user_id) => Uuid::parse_str(user_id).ok().map(|user_id| Self::Approved { user_id }),
            None => (stored == "denied").then_some(Self::Denied),
        }
    }
}

/// How long a spent code is remembered, to recognise a replay.
const SPENT_CODE_SECONDS: u64 = 600;

fn code_key(hash: &str) -> String {
    format!("dev:oauth:code:{hash}")
}

/// Left behind by the redemption of a code: the token family it bought.
fn spent_code_key(hash: &str) -> String {
    format!("dev:oauth:code-spent:{hash}")
}

/// Set when a spent code is presented again.
fn replayed_code_key(hash: &str) -> String {
    format!("dev:oauth:code-replayed:{hash}")
}

fn device_key(hash: &str) -> String {
    format!("dev:oauth:device:{hash}")
}

/// The user's answer; absent while the device is waiting.
fn device_answer_key(hash: &str) -> String {
    format!("dev:oauth:device-answer:{hash}")
}

/// Seconds the device must leave between polls; grows when it does not.
fn device_interval_key(hash: &str) -> String {
    format!("dev:oauth:device-interval:{hash}")
}

/// Present while the device has to wait before its next poll.
fn device_wait_key(hash: &str) -> String {
    format!("dev:oauth:device-wait:{hash}")
}

fn user_code_key(code: &str) -> String {
    format!("dev:oauth:user-code:{code}")
}

/// Takes a code's grant and leaves the spent marker, in one step: of two
/// requests presenting the same code at once, exactly one gets the grant.
const REDEEM_CODE: &str = r"
local grant = redis.call('GET', KEYS[1])
if not grant then return false end
redis.call('DEL', KEYS[1])
redis.call('SET', KEYS[2], ARGV[1], 'EX', ARGV[2])
return grant";

/// Whether `verifier` is the secret behind an `S256` code challenge.
fn pkce_matches(verifier: &str, challenge: &str) -> bool {
    secure_eq(b64url(&Sha256::digest(verifier.as_bytes())).as_bytes(), challenge.as_bytes())
}

/// RFC 7636 section 4.1: 43-128 characters of `A-Z a-z 0-9 - . _ ~`.
fn is_code_verifier(verifier: &str) -> bool {
    (43..=128).contains(&verifier.len())
        && verifier.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

/// An `S256` challenge is the base64url of 32 bytes: 43 characters.
fn is_code_challenge(challenge: &str) -> bool {
    challenge.len() == 43 && challenge.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// `redirect_uri` with these query parameters added.
fn redirect_to(redirect_uri: &str, params: &[(&str, &str)]) -> String {
    match url::Url::parse(redirect_uri) {
        Ok(mut url) => {
            url.query_pairs_mut().extend_pairs(params.iter().filter(|(_, value)| !value.is_empty()));
            url.into()
        }
        // Registered URIs always parse; nothing sensible can be done otherwise.
        Err(_) => redirect_uri.to_owned(),
    }
}

/// An authorization request whose client and redirect URI are genuine, so
/// its outcome — good or bad — may be sent back to the application.
pub(crate) struct CheckedRequest {
    pub app: AppRow,
    pub redirect_uri: String,
    state: String,
    issuer: String,
    /// The scopes to grant and the PKCE challenge, or the error to redirect with.
    outcome: OAuthResult<(Scopes, String)>,
}

impl CheckedRequest {
    fn redirect(&self, params: &[(&str, &str)]) -> String {
        let mut all = params.to_vec();
        all.extend([("state", self.state.as_str()), ("iss", self.issuer.as_str())]);
        redirect_to(&self.redirect_uri, &all)
    }

    pub(crate) fn error_redirect(&self, error: &OAuthError) -> String {
        self.redirect(&[("error", error.error), ("error_description", &error.description)])
    }

    /// The redirect that tells the application its request was refused by the user.
    pub(crate) fn denied_redirect(&self) -> String {
        self.error_redirect(&OAuthError::new("access_denied", "the user denied the request"))
    }

    pub(crate) fn outcome(&self) -> &OAuthResult<(Scopes, String)> {
        &self.outcome
    }
}

#[derive(sqlx::FromRow)]
struct RefreshRow {
    id: Uuid,
    authorization_id: Uuid,
    family_id: Uuid,
    scopes: Vec<String>,
    refresh_expires_at: Time,
    rotated: bool,
    revoked: bool,
    app_id: Uuid,
    user_id: Uuid,
    account_gone: bool,
}

impl Dev {
    /// The issuer identifier: the API's public base URL.
    pub(crate) fn issuer(&self) -> String {
        format!("{}{}", self.core.cfg.public_url(), self.core.cfg.api_path_prefix())
    }

    /// The scopes a request asks for: the ones named, or all the
    /// application registered. They must be among the registered ones.
    fn requested_scopes(app: &AppRow, scope: &str) -> OAuthResult<Scopes> {
        let requested = scopes::parse_oauth(scope)
            .map_err(|unknown| OAuthError::new("invalid_scope", format!("unknown scope {unknown}")))?;
        if requested.is_empty() {
            return Ok(app.scopes());
        }
        match requested.as_slice().iter().find(|scope| !app.scopes().allows(scope)) {
            Some(extra) => {
                Err(OAuthError::new("invalid_scope", format!("the application is not registered for {extra}")))
            }
            None => Ok(requested),
        }
    }

    /// Checks an authorization request. Fails — to be shown to the user,
    /// never redirected — when the client or the redirect URI is not
    /// genuine; every other defect is in the result's `outcome`.
    pub(crate) async fn check_authorization_request(
        &self,
        request: &AuthorizationRequest,
    ) -> AppResult<CheckedRequest> {
        let app = match Uuid::parse_str(&request.client_id) {
            Ok(app_id) => self.find_app(app_id).await?,
            Err(_) => None,
        }
        // Disabled, or its owner suspended: the same answer as for an ID nobody registered.
        .filter(|app| app.is_usable(&self.identity))
        .or_not_found("CLIENT_NOT_FOUND", "unknown client_id")?;
        let redirect_uri = if request.redirect_uri.is_empty() && app.redirect_uris.len() == 1 {
            app.redirect_uris[0].clone()
        } else {
            ensure!(
                redirect_uri_matches(&app.redirect_uris, &request.redirect_uri),
                invalid,
                "REDIRECT_URI_MISMATCH",
                "redirect_uri is not registered for this application"
            );
            request.redirect_uri.clone()
        };
        let outcome = (|| {
            if request.state.len() > 2000 {
                return Err(OAuthError::new("invalid_request", "state is too long"));
            }
            if request.response_type != "code" {
                return Err(OAuthError::new("unsupported_response_type", "only response_type=code is supported"));
            }
            if request.code_challenge.is_empty() {
                return Err(OAuthError::new("invalid_request", "code_challenge is required (PKCE)"));
            }
            if request.code_challenge_method != "S256" {
                return Err(OAuthError::new("invalid_request", "code_challenge_method must be S256"));
            }
            if !is_code_challenge(&request.code_challenge) {
                return Err(OAuthError::new("invalid_request", "code_challenge is not a base64url SHA-256 digest"));
            }
            Ok((Self::requested_scopes(&app, &request.scope)?, request.code_challenge.clone()))
        })();
        // A state too long to echo is dropped rather than reflected.
        let state = if request.state.len() > 2000 { String::new() } else { request.state.clone() };
        Ok(CheckedRequest { app, redirect_uri, state, issuer: self.issuer(), outcome })
    }

    /// What the consent screen shows for a request of `scopes` by `app`.
    pub(crate) async fn consent(
        &self,
        app: &AppRow,
        scopes: &Scopes,
        user_id: Uuid,
        redirect_uri: Option<&str>,
    ) -> AppResult<Consent> {
        let owner = tokuly_identity::summary(&self.core, app.owner_id).await?;
        let granted: Option<Vec<String>> =
            sqlx::query_scalar("SELECT scopes FROM dev_authorizations WHERE app_id = $1 AND user_id = $2")
                .bind(app.id)
                .bind(user_id)
                .fetch_optional(&self.core.db)
                .await?;
        let redirect_host = redirect_uri
            .and_then(|uri| url::Url::parse(uri).ok())
            .map(|url| url.host_str().map(str::to_owned).unwrap_or_else(|| url.scheme().to_owned()));
        Ok(Consent {
            app: app.to_public(owner).into(),
            scopes: scopes::describe(scopes.as_slice()),
            already_granted: granted.is_some_and(|granted| Scopes::new(granted).covers(scopes)),
            redirect_host: redirect_host.unwrap_or_default(),
            ..Default::default()
        })
    }

    /// Records that `user_id` lets `app` act with `scopes` (added to what
    /// they agreed to before). Tells the user when the application is new
    /// to them.
    async fn grant(&self, tx: &mut PgConnection, app: &AppRow, user_id: Uuid, scopes: &Scopes) -> AppResult<Uuid> {
        let (authorization_id, first): (Uuid, bool) = sqlx::query_as(
            "INSERT INTO dev_authorizations (app_id, user_id, scopes) VALUES ($1, $2, $3)
             ON CONFLICT (app_id, user_id) DO UPDATE
                SET scopes = (SELECT array_agg(DISTINCT s ORDER BY s) FROM unnest(dev_authorizations.scopes || EXCLUDED.scopes) AS s), updated_at = now()
             RETURNING id, (xmax = 0)",
        )
        .bind(app.id)
        .bind(user_id)
        .bind(scopes.as_slice())
        .fetch_one(&mut *tx)
        .await?;
        let event =
            AuthorizationEvent { app_id: app.id, user_id, scopes: scopes.as_slice().to_vec(), reason: String::new() };
        events::emit(&mut *tx, "dev.authorization.granted", Some(user_id), &event).await?;
        if first {
            let notice = NotificationRequest::new(
                user_id,
                "dev.app_authorized",
                Localized::new("アプリにアクセスを許可しました", "You gave an app access"),
                Localized::new(
                    format!("「{}」があなたのアカウントにアクセスできます。設定からいつでも取り消せます。", app.name),
                    format!(
                        "\"{}\" can now access your account. You can withdraw this in settings at any time.",
                        app.name
                    ),
                ),
                "/settings/apps",
            )
            .data(serde_json::json!({ "app_id": app.id, "scopes": scopes.as_slice() }));
            outbound::notify(&mut *tx, &notice).await?;
        }
        Ok(authorization_id)
    }

    /// The user approved: records the grant and returns the redirect that
    /// carries the code.
    pub(crate) async fn approve_authorization(&self, user_id: Uuid, checked: &CheckedRequest) -> AppResult<String> {
        let (scopes, code_challenge) = match &checked.outcome {
            Ok(granted) => granted.clone(),
            Err(e) => bail!(invalid, "AUTHORIZATION_REQUEST_INVALID", "{}: {}", e.error, e.description),
        };
        let mut tx = self.core.db.begin().await?;
        let authorization_id = self.grant(&mut tx, &checked.app, user_id, &scopes).await?;
        tx.commit().await?;
        let code = secrets::generate(SecretKind::Code);
        let grant = CodeGrant {
            app_id: checked.app.id,
            authorization_id,
            scopes: scopes.into_vec(),
            redirect_uri: checked.redirect_uri.clone(),
            code_challenge,
        };
        let ttl = Duration::from_secs(self.cfg.auth_code_seconds);
        self.core.cache.set_json(&code_key(&self.core.keys.token_hash(&code)), &grant, Some(ttl)).await?;
        Ok(checked.redirect(&[("code", &code)]))
    }

    // ───────────────────────────── clients ─────────────────────────────

    /// Authenticates the client of a token, revocation or device request
    /// (RFC 6749 section 2.3): HTTP Basic or the form body for confidential
    /// clients, the bare `client_id` for public ones.
    ///
    /// Every failure reads the same, whether the client does not exist, is
    /// disabled, sent the wrong secret or sent one it should not have.
    pub(crate) async fn authenticate_client(&self, credentials: &ClientCredentials) -> OAuthResult<AppRow> {
        let failed = || OAuthError::new("invalid_client", "client authentication failed");
        let (client_id, secret) = match (&credentials.basic, &credentials.body_id, &credentials.body_secret) {
            (Some(_), _, Some(_)) => {
                return Err(OAuthError::new("invalid_request", "use one client authentication method, not two"));
            }
            (Some((id, _)), Some(body_id), None) if id != body_id => {
                return Err(OAuthError::new("invalid_request", "client_id does not match the Authorization header"));
            }
            (Some((id, secret)), _, None) => (id.as_str(), Some(secret.as_str())),
            (None, Some(id), secret) => (id.as_str(), secret.as_deref()),
            (None, None, _) => return Err(failed()),
        };
        let app = match Uuid::parse_str(client_id) {
            Ok(app_id) => self.find_app(app_id).await?,
            Err(_) => None,
        };
        let Some(app) = app.filter(|app| app.is_usable(&self.identity)) else { return Err(failed()) };
        match (app.confidential, secret) {
            (true, Some(secret)) if app.secret_matches(self, secret) => {
                self.renew_secret_hash(&app, secret).await?;
                Ok(app)
            }
            (false, None) => Ok(app),
            _ => Err(failed()),
        }
    }

    /// A client secret that matched under the previous TOKULY_SECRET is
    /// stored under the current one, so it outlives the previous one.
    async fn renew_secret_hash(&self, app: &AppRow, secret: &str) -> AppResult<()> {
        let hashes = self.core.keys.token_hashes(secret);
        if app.secret_hash.as_ref().is_some_and(|stored| hashes[1..].contains(stored)) {
            sqlx::query("UPDATE dev_apps SET secret_hash = $2 WHERE id = $1 AND secret_hash = ANY($3)")
                .bind(app.id)
                .bind(&hashes[0])
                .bind(&hashes[1..])
                .execute(&self.core.db)
                .await?;
        }
        Ok(())
    }

    // ───────────────────────────── tokens ─────────────────────────────

    /// Issues an access and refresh token pair under an authorization.
    /// `family` is the chain of rotations the pair belongs to; `None` starts
    /// one named after this pair.
    async fn issue(
        &self,
        tx: &mut PgConnection,
        authorization_id: Uuid,
        family: Option<Uuid>,
        scopes: &Scopes,
    ) -> AppResult<TokenResponse> {
        let (access, refresh) = (secrets::generate(SecretKind::Access), secrets::generate(SecretKind::Refresh));
        let id = new_id();
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO dev_oauth_tokens (id, authorization_id, family_id, access_hash, access_expires_at, refresh_hash, refresh_expires_at, scopes)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(id)
        .bind(authorization_id)
        .bind(family.unwrap_or(id))
        .bind(self.core.keys.token_hash(&access))
        .bind(now + chrono::Duration::seconds(self.cfg.access_token_seconds))
        .bind(self.core.keys.token_hash(&refresh))
        .bind(now + chrono::Duration::days(self.cfg.refresh_token_days))
        .bind(scopes.as_slice())
        .execute(&mut *tx)
        .await?;
        Ok(TokenResponse {
            access_token: access,
            token_type: "Bearer",
            expires_in: self.cfg.access_token_seconds,
            refresh_token: refresh,
            scope: scopes.to_string(),
        })
    }

    /// Revokes every token pair of a family, at once.
    async fn revoke_family(&self, family_id: Uuid) -> AppResult<()> {
        let hashes: Vec<String> = sqlx::query_scalar("UPDATE dev_oauth_tokens SET revoked_at = now() WHERE family_id = $1 AND revoked_at IS NULL RETURNING access_hash")
            .bind(family_id)
            .fetch_all(&self.core.db)
            .await?;
        self.forget_tokens(&hashes).await
    }

    /// Withdraws a user's authorization of an application with every
    /// token issued under it. `false` when there was none.
    pub(crate) async fn revoke_authorization(
        &self,
        app_id: Uuid,
        user_id: Uuid,
        reason: &str,
        actor: Option<Uuid>,
    ) -> AppResult<bool> {
        let hashes: Vec<String> = sqlx::query_scalar(
            "SELECT o.access_hash FROM dev_oauth_tokens o JOIN dev_authorizations a ON a.id = o.authorization_id WHERE a.app_id = $1 AND a.user_id = $2",
        )
        .bind(app_id)
        .bind(user_id)
        .fetch_all(&self.core.db)
        .await?;
        let mut tx = self.core.db.begin().await?;
        let scopes: Option<Vec<String>> =
            sqlx::query_scalar("DELETE FROM dev_authorizations WHERE app_id = $1 AND user_id = $2 RETURNING scopes")
                .bind(app_id)
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some(scopes) = scopes else { return Ok(false) };
        events::emit(
            &mut *tx,
            "dev.authorization.revoked",
            actor,
            &AuthorizationEvent { app_id, user_id, scopes, reason: reason.to_owned() },
        )
        .await?;
        tx.commit().await?;
        self.forget_tokens(&hashes).await?;
        Ok(true)
    }

    /// `POST /oauth/token`.
    pub(crate) async fn token_request(
        &self,
        credentials: &ClientCredentials,
        form: &HashMap<String, String>,
    ) -> OAuthResult<TokenResponse> {
        let param = |name: &str| form.get(name).map(String::as_str).filter(|value| !value.is_empty());
        let Some(grant_type) = param("grant_type") else {
            return Err(OAuthError::new("invalid_request", "grant_type is required"));
        };
        if !matches!(grant_type, "authorization_code" | "refresh_token" | GRANT_DEVICE_CODE) {
            return Err(OAuthError::new("unsupported_grant_type", format!("grant type {grant_type} is not supported")));
        }
        let app = self.authenticate_client(credentials).await?;
        let required = |name: &'static str| {
            param(name).ok_or_else(|| OAuthError::new("invalid_request", format!("{name} is required")))
        };
        match grant_type {
            "authorization_code" => {
                self.exchange_code(&app, required("code")?, required("code_verifier")?, param("redirect_uri")).await
            }
            "refresh_token" => self.refresh(&app, required("refresh_token")?, param("scope")).await,
            _ => self.poll_device(&app, required("device_code")?).await,
        }
    }

    /// The authorization code grant with PKCE (RFC 7636 section 4.6).
    ///
    /// A code buys tokens once. One atomic step takes it and leaves a marker
    /// naming the token family it is about to buy; whoever presents the code
    /// after that revokes the family, since a code seen twice was
    /// intercepted. The first redeemer checks for such a replay once its
    /// tokens exist, so the family dies whichever of the two is faster.
    async fn exchange_code(
        &self,
        app: &AppRow,
        code: &str,
        verifier: &str,
        redirect_uri: Option<&str>,
    ) -> OAuthResult<TokenResponse> {
        let bad_code = || OAuthError::invalid_grant("the authorization code is invalid, expired or already used");
        if !secrets::is(code, SecretKind::Code) {
            return Err(bad_code());
        }
        let hash = self.core.keys.token_hash(code);
        let family_id = new_id();
        let mut cache = self.core.cache.conn();
        let taken: Option<String> = redis::Script::new(REDEEM_CODE)
            .key(code_key(&hash))
            .key(spent_code_key(&hash))
            .arg(family_id.to_string())
            .arg(SPENT_CODE_SECONDS)
            .invoke_async(&mut cache)
            .await
            .map_err(AppError::from)?;
        let Some(grant) = taken.and_then(|json| serde_json::from_str::<CodeGrant>(&json).ok()) else {
            if let Some(family) =
                self.core.cache.get_string(&spent_code_key(&hash)).await?.and_then(|id| Uuid::parse_str(&id).ok())
            {
                tracing::warn!(app_id = %app.id, "authorization code replayed; its tokens are revoked");
                // The flag first: the first redeemer may not have stored its tokens yet.
                self.core
                    .cache
                    .set_string(&replayed_code_key(&hash), "1", Some(Duration::from_secs(SPENT_CODE_SECONDS)))
                    .await?;
                self.revoke_family(family).await?;
            }
            return Err(bad_code());
        };
        if grant.app_id != app.id {
            return Err(bad_code());
        }
        if redirect_uri.is_some_and(|uri| uri != grant.redirect_uri) {
            return Err(OAuthError::invalid_grant("redirect_uri does not match the authorization request"));
        }
        if !is_code_verifier(verifier) {
            return Err(OAuthError::new("invalid_request", "code_verifier must be 43-128 unreserved characters"));
        }
        if !pkce_matches(verifier, &grant.code_challenge) {
            return Err(OAuthError::invalid_grant("code_verifier does not match the code challenge"));
        }
        let mut tx = self.core.db.begin().await?;
        // The user may have withdrawn the authorization in the meantime.
        let alive: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM dev_authorizations WHERE id = $1)")
            .bind(grant.authorization_id)
            .fetch_one(&mut *tx)
            .await?;
        if !alive {
            return Err(bad_code());
        }
        let issued = self.issue(&mut tx, grant.authorization_id, Some(family_id), &Scopes::new(grant.scopes)).await?;
        tx.commit().await?;
        if self.core.cache.exists(&replayed_code_key(&hash)).await? {
            self.revoke_family(family_id).await?;
            return Err(bad_code());
        }
        Ok(issued)
    }

    /// The refresh token grant, with rotation and reuse detection.
    async fn refresh(&self, app: &AppRow, refresh_token: &str, scope: Option<&str>) -> OAuthResult<TokenResponse> {
        let bad_token = || OAuthError::invalid_grant("the refresh token is invalid, expired or revoked");
        if !secrets::is(refresh_token, SecretKind::Refresh) {
            return Err(bad_token());
        }
        let row: Option<RefreshRow> = sqlx::query_as(
            "SELECT o.id, o.authorization_id, o.family_id, o.scopes, o.refresh_expires_at,
                    o.rotated_at IS NOT NULL AS rotated, o.revoked_at IS NOT NULL AS revoked, a.app_id, a.user_id,
                    (u.banned_at IS NOT NULL OR u.deleted_at IS NOT NULL) AS account_gone
             FROM dev_oauth_tokens o JOIN dev_authorizations a ON a.id = o.authorization_id JOIN users u ON u.id = a.user_id
             WHERE o.refresh_hash = ANY($1)",
        )
        .bind(self.core.keys.token_hashes(refresh_token))
        .fetch_optional(&self.core.db)
        .await?;
        let Some(row) = row.filter(|row| row.app_id == app.id) else { return Err(bad_token()) };
        if row.revoked || row.account_gone || row.refresh_expires_at <= Utc::now() {
            return Err(bad_token());
        }
        let granted = Scopes::new(row.scopes);
        let scopes = match scope {
            None => granted,
            Some(scope) => {
                let narrowed = scopes::parse_oauth(scope)
                    .map_err(|unknown| OAuthError::new("invalid_scope", format!("unknown scope {unknown}")))?;
                if narrowed.is_empty() || !granted.covers(&narrowed) {
                    return Err(OAuthError::new(
                        "invalid_scope",
                        "a refresh can only keep or narrow the scopes it was granted",
                    ));
                }
                narrowed
            }
        };
        let mut tx = self.core.db.begin().await?;
        // Retiring the row is the test: whoever does it first holds the
        // genuine token; anyone after presents a copy.
        let retired = sqlx::query("UPDATE dev_oauth_tokens SET rotated_at = now() WHERE id = $1 AND rotated_at IS NULL AND revoked_at IS NULL").bind(row.id).execute(&mut *tx).await?;
        if row.rotated || retired.rows_affected() == 0 {
            // Give the connection back before asking for another.
            tx.rollback().await?;
            tracing::warn!(app_id = %app.id, user_id = %row.user_id, "refresh token reuse detected; the token family is revoked");
            self.revoke_family(row.family_id).await?;
            return Err(bad_token());
        }
        let issued = self.issue(&mut tx, row.authorization_id, Some(row.family_id), &scopes).await?;
        tx.commit().await?;
        Ok(issued)
    }

    /// `POST /oauth/revoke` (RFC 7009). A token that is unknown, or was
    /// issued to another client, is not an error and nothing happens: the
    /// answer does not say whether somebody else's token exists.
    pub(crate) async fn revoke_request(
        &self,
        credentials: &ClientCredentials,
        form: &HashMap<String, String>,
    ) -> OAuthResult<()> {
        let app = self.authenticate_client(credentials).await?;
        let Some(token) = form.get("token").filter(|token| !token.is_empty()) else {
            return Err(OAuthError::new("invalid_request", "token is required"));
        };
        let column = match secrets::kind_of(token) {
            Some(SecretKind::Access) => "access_hash",
            Some(SecretKind::Refresh) => "refresh_hash",
            _ => return Ok(()),
        };
        let found: Option<(Uuid, Uuid, Uuid)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT o.id, o.family_id, a.app_id FROM dev_oauth_tokens o JOIN dev_authorizations a ON a.id = o.authorization_id WHERE o.{column} = ANY($1)"
        )))
        .bind(self.core.keys.token_hashes(token))
        .fetch_optional(&self.core.db)
        .await?;
        let Some((id, family_id, _)) = found.filter(|(_, _, app_id)| *app_id == app.id) else { return Ok(()) };
        if column == "refresh_hash" {
            // The refresh token is the grant: everything descended from it goes.
            self.revoke_family(family_id).await?;
        } else {
            let hashes: Vec<String> =
                sqlx::query_scalar("UPDATE dev_oauth_tokens SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL RETURNING access_hash").bind(id).fetch_all(&self.core.db).await?;
            self.forget_tokens(&hashes).await?;
        }
        Ok(())
    }

    // ───────────────────────────── device flow ─────────────────────────────

    /// `POST /oauth/device_authorization` (RFC 8628 section 3.1).
    pub(crate) async fn device_authorization(
        &self,
        credentials: &ClientCredentials,
        form: &HashMap<String, String>,
    ) -> OAuthResult<DeviceAuthorization> {
        let app = self.authenticate_client(credentials).await?;
        // The flow where approval happens away from the client is what
        // phishing uses; an application gets it only by asking at registration.
        if !app.device_flow {
            return Err(OAuthError::new("unauthorized_client", "this client is not registered for the device flow"));
        }
        let scopes = Self::requested_scopes(&app, form.get("scope").map(String::as_str).unwrap_or_default())?;
        let ttl = Duration::from_secs(self.cfg.device_code_seconds);
        let device_code = secrets::generate(SecretKind::Device);
        let hash = self.core.keys.token_hash(&device_code);
        // User codes are short; make sure this one is not waiting already.
        let mut user_code = secrets::generate_user_code();
        for attempt in 0.. {
            if self.core.cache.set_nx(&user_code_key(&secrets::normalize_user_code(&user_code)), &hash, ttl).await? {
                break;
            }
            if attempt == 5 {
                return Err(OAuthError::server(AppError::unavailable(
                    "USER_CODES_EXHAUSTED",
                    "no user code available",
                )));
            }
            user_code = secrets::generate_user_code();
        }
        let grant = DeviceGrant {
            app_id: app.id,
            scopes: scopes.into_vec(),
            user_code: secrets::normalize_user_code(&user_code),
        };
        self.core.cache.set_json(&device_key(&hash), &grant, Some(ttl)).await?;
        self.core
            .cache
            .set_string(&device_interval_key(&hash), &self.cfg.device_poll_seconds.to_string(), Some(ttl))
            .await?;
        let verification_uri = self.core.web_url("/device");
        Ok(DeviceAuthorization {
            verification_uri_complete: format!("{verification_uri}?user_code={user_code}"),
            verification_uri,
            device_code,
            user_code,
            expires_in: self.cfg.device_code_seconds,
            interval: self.cfg.device_poll_seconds,
        })
    }

    /// The waiting device a user code stands for, with the hash its keys
    /// are named after. One answer for every code that cannot be answered:
    /// unknown, expired, answered already, or of an application that is gone.
    async fn pending_device(&self, user_id: Uuid, user_code: &str) -> AppResult<(String, DeviceGrant, AppRow)> {
        // The code space is small on purpose (people type it): guessing is slowed here.
        self.core.cache.rate_limit(&format!("dev-user-code:{user_id}"), 10, Duration::from_secs(300)).await?;
        let not_found =
            || AppError::not_found("DEVICE_CODE_INVALID", "this code is invalid, expired or already answered");
        let code = secrets::normalize_user_code(user_code);
        let hash = self.core.cache.get_string(&user_code_key(&code)).await?.ok_or_else(not_found)?;
        let grant: DeviceGrant = self.core.cache.get_json(&device_key(&hash)).await?.ok_or_else(not_found)?;
        if grant.user_code != code || self.core.cache.exists(&device_answer_key(&hash)).await? {
            return Err(not_found());
        }
        // An application that gave the device flow up takes its waiting codes with it.
        let app = self
            .find_app(grant.app_id)
            .await?
            .filter(|app| app.is_usable(&self.identity) && app.device_flow)
            .ok_or_else(not_found)?;
        Ok((hash, grant, app))
    }

    /// What the device page shows for a user code.
    pub(crate) async fn device_consent(&self, user_id: Uuid, user_code: &str) -> AppResult<Consent> {
        let (_, grant, app) = self.pending_device(user_id, user_code).await?;
        self.consent(&app, &Scopes::new(grant.scopes), user_id, None).await
    }

    /// The user's answer to a waiting device. The first answer stands: a
    /// second one — a double click, another tab — is told the code is spent.
    /// Nothing is granted here; the authorization is recorded together with
    /// the tokens when the device collects them.
    pub(crate) async fn answer_device(&self, user_id: Uuid, user_code: &str, approve: bool) -> AppResult<()> {
        let (hash, grant, _) = self.pending_device(user_id, user_code).await?;
        let answer = if approve { DeviceAnswer::Approved { user_id } } else { DeviceAnswer::Denied };
        let ttl = Duration::from_secs(self.cfg.device_code_seconds);
        let first = self.core.cache.set_nx(&device_answer_key(&hash), &answer.encode(), ttl).await?;
        ensure!(first, not_found, "DEVICE_CODE_INVALID", "this code is invalid, expired or already answered");
        // The code has done its job.
        self.core.cache.del(&user_code_key(&grant.user_code)).await
    }

    /// Whether the device polled before its interval had passed. Each such
    /// poll makes the interval five seconds longer (RFC 8628 section 3.5).
    async fn polled_too_soon(&self, hash: &str) -> AppResult<Option<u64>> {
        if self.cfg.device_poll_seconds == 0 {
            return Ok(None);
        }
        let interval = self
            .core
            .cache
            .get_string(&device_interval_key(hash))
            .await?
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(self.cfg.device_poll_seconds);
        if self.core.cache.set_nx(&device_wait_key(hash), "1", Duration::from_secs(interval)).await? {
            return Ok(None);
        }
        let longer: u64 =
            redis::cmd("INCRBY").arg(device_interval_key(hash)).arg(5).query_async(&mut self.core.cache.conn()).await?;
        Ok(Some(longer))
    }

    /// The device code grant: the device asks whether its user has answered
    /// (RFC 8628 section 3.4 and 3.5). A code that is unknown, expired,
    /// collected already or another client's gets the same answer.
    async fn poll_device(&self, app: &AppRow, device_code: &str) -> OAuthResult<TokenResponse> {
        let gone = || OAuthError::new("expired_token", "the device code is invalid or has expired");
        if !secrets::is(device_code, SecretKind::Device) {
            return Err(gone());
        }
        // A code issued while the application had the device flow is worth nothing once it has not.
        if !app.device_flow {
            return Err(gone());
        }
        let hash = self.core.keys.token_hash(device_code);
        let Some(grant) =
            self.core.cache.get_json::<DeviceGrant>(&device_key(&hash)).await?.filter(|grant| grant.app_id == app.id)
        else {
            return Err(gone());
        };
        if let Some(interval) = self.polled_too_soon(&hash).await? {
            return Err(OAuthError::new("slow_down", format!("poll every {interval} seconds")));
        }
        let answer = self
            .core
            .cache
            .get_string(&device_answer_key(&hash))
            .await?
            .and_then(|stored| DeviceAnswer::decode(&stored));
        let Some(answer) = answer else {
            // No answer may also mean that another poll collected it a moment
            // ago; that one removed the grant before it took the answer.
            if !self.core.cache.exists(&device_key(&hash)).await? {
                return Err(gone());
            }
            return Err(OAuthError::new("authorization_pending", "the user has not answered yet"));
        };
        // The grant goes first, so that a poll which finds the answer taken
        // also finds the grant gone. Taking the answer is what makes it
        // single-use: of two polls that arrive together, one collects.
        self.core.cache.del(&device_key(&hash)).await?;
        let taken: Option<String> = redis::cmd("GETDEL")
            .arg(device_answer_key(&hash))
            .query_async(&mut self.core.cache.conn())
            .await
            .map_err(AppError::from)?;
        if taken.is_none() {
            return Err(gone());
        }
        let DeviceAnswer::Approved { user_id } = answer else {
            return Err(OAuthError::new("access_denied", "the user denied the request"));
        };
        let scopes = Scopes::new(grant.scopes);
        let mut tx = self.core.db.begin().await?;
        let authorization_id = self.grant(&mut tx, app, user_id, &scopes).await?;
        let issued = self.issue(&mut tx, authorization_id, None, &scopes).await?;
        tx.commit().await?;
        Ok(issued)
    }

    /// Removes token pairs nobody can use any more.
    pub(crate) async fn purge_dead_oauth_tokens(&self) -> AppResult<u64> {
        let done = sqlx::query(
            "DELETE FROM dev_oauth_tokens WHERE refresh_expires_at < now() OR revoked_at < now() - interval '7 days'",
        )
        .execute(&self.core.db)
        .await?;
        Ok(done.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_follows_the_rfc_example() {
        // RFC 7636 appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(is_code_verifier(verifier) && is_code_challenge(challenge));
        assert!(pkce_matches(verifier, challenge));
        assert!(!pkce_matches("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXK", challenge));
        // `plain` is not accepted: the verifier is never its own challenge.
        assert!(!pkce_matches(challenge, challenge));
        assert!(
            !is_code_verifier("short")
                && !is_code_verifier(&"a".repeat(129))
                && !is_code_verifier(&format!("{}!", "a".repeat(43)))
        );
        assert!(
            !is_code_challenge(verifier.trim_end_matches('k'))
                && !is_code_challenge(&"a".repeat(44))
                && !is_code_challenge(&format!("{}=", "a".repeat(42)))
        );
    }

    #[test]
    fn redirects_keep_the_registered_query_and_skip_empty_values() {
        assert_eq!(
            redirect_to("https://app.example/cb", &[("code", "abc"), ("state", "x y&z")]),
            "https://app.example/cb?code=abc&state=x+y%26z"
        );
        assert_eq!(
            redirect_to("https://app.example/cb?tenant=1", &[("error", "access_denied"), ("state", "")]),
            "https://app.example/cb?tenant=1&error=access_denied"
        );
        assert_eq!(redirect_to("com.example.app:/oauth", &[("code", "abc")]), "com.example.app:/oauth?code=abc");
    }

    #[test]
    fn a_device_answer_survives_storage() {
        let user_id = Uuid::now_v7();
        for answer in [DeviceAnswer::Approved { user_id }, DeviceAnswer::Denied] {
            assert_eq!(DeviceAnswer::decode(&answer.encode()), Some(answer));
        }
        assert_eq!(DeviceAnswer::decode("approved:not-a-user"), None);
        assert_eq!(DeviceAnswer::decode("pending"), None);
    }

    #[test]
    fn token_endpoint_errors_have_their_status() {
        assert_eq!(OAuthError::new("invalid_client", "x").status, 401);
        assert_eq!(OAuthError::new("invalid_grant", "x").status, 400);
        assert_eq!(OAuthError::new("authorization_pending", "x").status, 400);
        // Polling too fast is an OAuth answer; hammering the endpoint is HTTP's.
        assert_eq!(OAuthError::new("slow_down", "poll every 10 seconds").status, 400);
        assert_eq!(OAuthError::from(AppError::rate_limited("RATE_LIMITED", "x")).status, 429);
        let internal = OAuthError::from(AppError::internal(anyhow::anyhow!("database on fire")));
        assert_eq!((internal.status, internal.error), (500, "server_error"));
        assert!(!internal.description.contains("fire"), "internals are not disclosed");
    }
}
