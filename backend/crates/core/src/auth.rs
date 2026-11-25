//! Access tokens and the per-request authentication state.
//!
//! Access tokens are short-lived EdDSA (Ed25519) JWTs. The signing key is
//! derived from the master secret, so the API and the realtime gateway verify
//! each other's tokens without sharing key files.
//!
//! A revoked session's tokens stop working at once, not at expiry: the
//! revocation is written to Valkey (`auth:revoked:<session>`, kept for the
//! lifetime of an access token) and every authenticated request asks Valkey
//! about its session (one `EXISTS` on the shared multiplexed connection). A
//! small in-memory set, fed over the bus and loaded from Valkey when a
//! process starts, answers without the round trip for sessions already
//! known to be revoked, and is what the synchronous [`Tokens::verify`] (the
//! realtime gateway) consults.
//!
//! The other kind of bearer credential is an API token (`tkl_…`): opaque,
//! long-lived, limited to its scopes. [`auth_middleware`] hands those to
//! [`crate::api_tokens`], which verifies them and enforces scopes and rate
//! limits before a handler runs.
use std::{sync::Arc, time::Duration};

use axum::{extract::Request, middleware::Next, response::Response};
use chrono::Utc;
use dashmap::DashMap;
use ed25519_dalek::{SigningKey, pkcs8::EncodePrivateKey as _};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    api_tokens::{self, ApiGate, ApiToken, ApiTokens, Guarded, Refusal, Scopes},
    cache::Cache,
    error::{AppError, AppResult},
    keys::Keys,
    time::Time,
};

pub const ACCESS_TOKEN_TTL: Duration = Duration::from_secs(15 * 60);
const ISSUER: &str = "tokuly";

/// The authenticated caller of a request.
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub user_id: Uuid,
    /// The session. For an API token, which has no session, the ID of the
    /// credential ([`ApiToken::id`]): stable and unique, but not a row in
    /// `sessions`.
    pub session_id: Uuid,
    /// Staff roles. Always empty for API tokens.
    pub roles: Vec<String>,
    /// When the access token that proved this identity stops being valid.
    pub expires_at: Time,
    /// The API token making the request, with the scopes that limit it.
    /// `None` for a normal session, which may do everything its user may.
    pub api_token: Option<ApiToken>,
}

impl AuthContext {
    /// The caller an API token stands for. Tokens never carry roles, whoever
    /// their account is.
    pub fn from_api_token(token: ApiToken) -> Self {
        Self {
            user_id: token.user_id,
            session_id: token.id,
            roles: Vec::new(),
            expires_at: token.expires_at.unwrap_or(Time::MAX_UTC),
            api_token: Some(token),
        }
    }

    /// The scopes limiting this caller; `None` for a session (no limit).
    pub fn scopes(&self) -> Option<&Scopes> {
        self.api_token.as_ref().map(|token| &token.scopes)
    }

    /// Whether the caller may use `scope` (`<domain>:read` / `<domain>:write`).
    /// A session always may. The middleware already checked the scope of the
    /// RPC being called; this is for handlers that reach into another
    /// domain's data on the caller's behalf.
    pub fn allows(&self, scope: &str) -> bool {
        self.scopes().is_none_or(|scopes| scopes.allows(scope))
    }

    /// Whether this is a signed-in session rather than an API token.
    pub fn is_session(&self) -> bool {
        self.api_token.is_none()
    }

    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role || r == "admin")
    }

    pub fn is_staff(&self) -> bool {
        self.roles
            .iter()
            .any(|r| r == "admin" || r == "moderator" || r == "staff")
    }
}

/// What the auth middleware learned about a request; stored in request extensions.
#[derive(Debug, Clone)]
pub enum AuthState {
    Anonymous,
    Authenticated(AuthContext),
    /// A token was presented but is expired, malformed or revoked.
    Invalid(&'static str),
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: Uuid,
    sid: Uuid,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    roles: Vec<String>,
    iat: i64,
    exp: i64,
    iss: String,
}

/// Issues and verifies access tokens.
#[derive(Clone)]
pub struct Tokens {
    inner: Arc<TokensInner>,
    /// Where revocations are shared between processes; `None` keeps them in
    /// this process only (unit tests).
    revocations: Option<Cache>,
}

/// How long a revocation must be remembered: until every access token of
/// the session has expired (plus the verification leeway, with margin).
const REVOCATION_TTL: Duration = Duration::from_secs(ACCESS_TOKEN_TTL.as_secs() + 30);
const REVOKED_PREFIX: &str = "auth:revoked:";
/// How long a request waits for Valkey's answer about its session before it
/// goes on with what this process knows.
const REVOCATION_LOOKUP_TIMEOUT: Duration = Duration::from_millis(50);

struct TokensInner {
    encoding: EncodingKey,
    decoding: DecodingKey,
    validation: Validation,
    /// Revoked session IDs → instant after which the entry can be dropped.
    revoked: DashMap<Uuid, Time>,
    public_key: [u8; 32],
    /// When "revocations unreadable" was last logged (Unix seconds).
    unreadable_warned_at: std::sync::atomic::AtomicI64,
}

impl Tokens {
    pub fn new(keys: &Keys) -> anyhow::Result<Self> {
        let signing = SigningKey::from_bytes(&keys.derive("jwt/ed25519"));
        let pkcs8 = signing
            .to_pkcs8_der()
            .map_err(|e| anyhow::anyhow!("failed to encode signing key: {e}"))?;
        let public_key = signing.verifying_key().to_bytes();
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_issuer(&[ISSUER]);
        validation.set_required_spec_claims(&["exp", "iss", "sub"]);
        validation.leeway = 5;
        Ok(Self {
            inner: Arc::new(TokensInner {
                encoding: EncodingKey::from_ed_der(pkcs8.as_bytes()),
                decoding: DecodingKey::from_ed_der(&public_key),
                validation,
                revoked: DashMap::new(),
                public_key,
                unreadable_warned_at: std::sync::atomic::AtomicI64::new(0),
            }),
            revocations: None,
        })
    }

    /// Shares revocations with every other process through `cache`.
    pub fn with_revocations(mut self, cache: Cache) -> Self {
        self.revocations = Some(cache);
        self
    }

    /// Raw Ed25519 public key (for publishing as a JWK).
    pub fn public_key(&self) -> [u8; 32] {
        self.inner.public_key
    }

    /// Issues an access token; returns it with its expiry.
    pub fn issue(
        &self,
        user_id: Uuid,
        session_id: Uuid,
        roles: &[String],
    ) -> AppResult<(String, Time)> {
        let now = Utc::now();
        let expires = now + ACCESS_TOKEN_TTL;
        let claims = Claims {
            sub: user_id,
            sid: session_id,
            roles: roles.to_vec(),
            iat: now.timestamp(),
            exp: expires.timestamp(),
            iss: ISSUER.to_owned(),
        };
        let token = jsonwebtoken::encode(
            &Header::new(Algorithm::EdDSA),
            &claims,
            &self.inner.encoding,
        )
        .map_err(|e| AppError::internal(anyhow::anyhow!("jwt encode: {e}")))?;
        Ok((token, expires))
    }

    /// Verifies a token and returns who it belongs to.
    pub fn verify(&self, token: &str) -> Result<AuthContext, &'static str> {
        let data =
            jsonwebtoken::decode::<Claims>(token, &self.inner.decoding, &self.inner.validation)
                .map_err(|e| match e.kind() {
                    jsonwebtoken::errors::ErrorKind::ExpiredSignature => "TOKEN_EXPIRED",
                    _ => "TOKEN_INVALID",
                })?;
        if self.is_revoked(data.claims.sid) {
            return Err("SESSION_REVOKED");
        }
        Ok(AuthContext {
            user_id: data.claims.sub,
            session_id: data.claims.sid,
            roles: data.claims.roles,
            expires_at: chrono::DateTime::from_timestamp(data.claims.exp, 0).unwrap_or_default(),
            api_token: None,
        })
    }

    /// Revokes a session for the remaining lifetime of its tokens, in this
    /// process and (through Valkey) in every other one, for their next
    /// request. [`crate::Core::revoke_session`] also tells running processes
    /// over the bus.
    pub async fn revoke(&self, session_id: Uuid) {
        self.revoke_session(session_id);
        if let Some(cache) = &self.revocations
            && let Err(e) = cache.set_string(&format!("{REVOKED_PREFIX}{session_id}"), "1", Some(REVOCATION_TTL)).await
        {
            tracing::warn!(error = %e, %session_id, "session revocation not stored in Valkey");
        }
    }

    /// Whether Valkey holds a revocation of the session (another process
    /// revoked it). An unreachable Valkey answers `false`: the bus and the
    /// in-memory set still apply, and signing everyone out while Valkey is
    /// down would be worse.
    async fn revoked_elsewhere(&self, session_id: Uuid) -> bool {
        let Some(cache) = &self.revocations else { return false };
        let key = format!("{REVOKED_PREFIX}{session_id}");
        match tokio::time::timeout(REVOCATION_LOOKUP_TIMEOUT, cache.exists(&key)).await {
            Ok(Ok(revoked)) => {
                if revoked {
                    self.revoke_session(session_id);
                }
                revoked
            }
            Ok(Err(e)) => {
                self.warn_unreadable(&e.to_string());
                false
            }
            Err(_) => {
                self.warn_unreadable("no answer in time");
                false
            }
        }
    }

    /// Logs that Valkey could not be asked, at most once a minute per process.
    fn warn_unreadable(&self, error: &str) {
        use std::sync::atomic::Ordering;
        let now = Utc::now().timestamp();
        let last = self.inner.unreadable_warned_at.load(Ordering::Relaxed);
        if now - last >= 60
            && self.inner.unreadable_warned_at.compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed).is_ok()
        {
            tracing::warn!(error, "session revocations unreadable; checked in memory only (logged once a minute)");
        }
    }

    /// Loads the revocations other processes stored, so that a process that
    /// just started (and missed their broadcasts) knows them too.
    pub async fn load_revocations(&self) -> anyhow::Result<usize> {
        use futures::StreamExt as _;
        let Some(cache) = &self.revocations else { return Ok(0) };
        let mut conn = cache.conn();
        let keys: Vec<String> = {
            let mut iter = redis::AsyncCommands::scan_match::<_, String>(&mut conn, format!("{REVOKED_PREFIX}*")).await?;
            let mut keys = Vec::new();
            while let Some(key) = iter.next().await {
                keys.push(key?);
            }
            keys
        };
        let ids: Vec<Uuid> = keys.iter().filter_map(|key| key.strip_prefix(REVOKED_PREFIX)?.parse().ok()).collect();
        for id in &ids {
            self.revoke_session(*id);
        }
        Ok(ids.len())
    }

    /// Marks a session as revoked in this process only, for the remaining
    /// lifetime of its tokens (what a revocation broadcast does).
    pub fn revoke_session(&self, session_id: Uuid) {
        self.inner.revoked.insert(session_id, Utc::now() + REVOCATION_TTL);
        if self.inner.revoked.len() > 10_000 {
            let now = Utc::now();
            self.inner.revoked.retain(|_, until| *until > now);
        }
    }

    pub fn is_revoked(&self, session_id: Uuid) -> bool {
        match self.inner.revoked.get(&session_id) {
            Some(until) => *until > Utc::now(),
            None => false,
        }
    }

    /// Extracts and verifies a session's bearer token from an `Authorization`
    /// header value. API tokens are not sessions: they are invalid here and
    /// go through [`auth_middleware`].
    pub fn authenticate_header(&self, header: Option<&http::HeaderValue>) -> AuthState {
        match bearer_token(header) {
            Bearer::Absent => AuthState::Anonymous,
            Bearer::Malformed => AuthState::Invalid("TOKEN_INVALID"),
            Bearer::Token(token) => self.authenticate(token),
        }
    }

    fn authenticate(&self, token: &str) -> AuthState {
        match self.verify(token) {
            Ok(ctx) => AuthState::Authenticated(ctx),
            Err(reason) => AuthState::Invalid(reason),
        }
    }

    /// [`Self::authenticate`], and the session's revocation is also looked up
    /// in Valkey: what every request goes through.
    pub async fn authenticate_shared(&self, token: &str) -> AuthState {
        match self.authenticate(token) {
            AuthState::Authenticated(ctx) if self.revoked_elsewhere(ctx.session_id).await => AuthState::Invalid("SESSION_REVOKED"),
            state => state,
        }
    }
}

/// What an `Authorization` header holds.
enum Bearer<'a> {
    Absent,
    /// A header that is not `Bearer <token>`.
    Malformed,
    Token(&'a str),
}

fn bearer_token(header: Option<&http::HeaderValue>) -> Bearer<'_> {
    let Some(value) = header.and_then(|v| v.to_str().ok()) else {
        return Bearer::Absent;
    };
    match value.strip_prefix("Bearer ").or_else(|| value.strip_prefix("bearer ")) {
        Some(token) => Bearer::Token(token.trim()),
        None => Bearer::Malformed,
    }
}

/// Network-level facts about the caller, stored in request extensions.
#[derive(Debug, Clone, Default)]
pub struct ClientMeta {
    /// The client's address: the TCP peer, or the client named by one of our
    /// proxies (see [`crate::proxies`]). Empty when the peer is unknown.
    pub ip: String,
    pub user_agent: String,
    /// `x-tokuly-client: web` — refresh tokens go into an HttpOnly cookie.
    pub is_web: bool,
    /// Raw `Cookie` header, for RPCs that read the refresh cookie.
    pub cookies: String,
    /// Preferred language from `Accept-Language` (first tag, lowercased).
    pub language: String,
}

/// What [`auth_middleware`] needs: the verifiers of session tokens and of
/// API tokens, and the proxies whose word is taken for a client's address.
#[derive(Clone)]
pub struct AuthLayer {
    tokens: Tokens,
    trusted_proxies: std::sync::Arc<crate::proxies::TrustedProxies>,
    api: ApiGate,
}

impl AuthLayer {
    /// A layer that knows sessions only; every API token is invalid until
    /// [`Self::with_api_tokens`] names their registry.
    pub fn new(tokens: Tokens, trusted_proxies: crate::proxies::TrustedProxies) -> Self {
        Self { tokens, trusted_proxies: std::sync::Arc::new(trusted_proxies), api: ApiGate::default() }
    }

    /// Accepts the API tokens of the verifier registered in `api_tokens`.
    /// Their requests are counted in `limiter`; `None` switches the
    /// per-token rate limits off.
    pub fn with_api_tokens(mut self, api_tokens: ApiTokens, limiter: Option<Cache>) -> Self {
        self.api = ApiGate { tokens: api_tokens, limiter, ..self.api };
        self
    }

    /// How often a token is verified again while a response made with it is
    /// still streaming (half a minute unless set). A stream is cut off at
    /// the first check that finds its token revoked or expired.
    pub fn with_stream_recheck(mut self, every: std::time::Duration) -> Self {
        self.api.recheck = every;
        self
    }
}

/// Every `Cookie` header of a request as one `a=1; b=2` string. HTTP/2 (the
/// edge reaches the API over h2c) may send each cookie as a header field of
/// its own (RFC 9113, 8.2.3); reading only the first would lose the rest —
/// the refresh token whenever another cookie happens to come first.
pub fn cookie_header(headers: &http::HeaderMap) -> String {
    headers
        .get_all(http::header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

/// Axum middleware: authenticates the request and records [`AuthState`] and
/// [`ClientMeta`] for handlers.
///
/// A request with a session token, a bad token or none is never rejected
/// here: handlers decide what an anonymous caller may do. A request with an
/// API token is held to the token's rate limit and scopes first, and may end
/// here (see [`crate::api_tokens`]).
///
/// The client address is the TCP peer, or — when the peer is one of our
/// proxies — the client that proxy names in `X-Forwarded-For`. Serve the
/// router with `into_make_service_with_connect_info::<SocketAddr>()` so the
/// peer is known; without it the address stays empty.
pub async fn auth_middleware(
    axum::extract::State(layer): axum::extract::State<AuthLayer>,
    mut req: Request,
    next: Next,
) -> Response {
    let headers = req.headers();
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let text = |name: &str| header(name).unwrap_or_default().to_owned();
    let ip = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|peer| layer.trusted_proxies.client_ip(peer.0.ip(), header("x-forwarded-for")).to_string())
        .unwrap_or_default();
    let meta = ClientMeta {
        ip,
        user_agent: text("user-agent"),
        is_web: text("x-tokuly-client").eq_ignore_ascii_case("web"),
        cookies: cookie_header(headers),
        language: text("accept-language")
            .split(',')
            .next()
            .unwrap_or_default()
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase(),
    };
    let (state, admitted) = match bearer_token(headers.get(http::header::AUTHORIZATION)) {
        Bearer::Absent => (AuthState::Anonymous, None),
        Bearer::Malformed => (AuthState::Invalid("TOKEN_INVALID"), None),
        Bearer::Token(token) if api_tokens::is_api_token(token) => match layer.api.admit(token, req.uri().path(), headers, &meta).await {
            Ok(admitted) => (AuthState::Authenticated(AuthContext::from_api_token(admitted.token.clone())), Some(admitted)),
            Err(Refusal::Unauthenticated(reason)) => (AuthState::Invalid(reason), None),
            Err(Refusal::Respond(response)) => return response,
        },
        Bearer::Token(token) => (layer.tokens.authenticate_shared(token).await, None),
    };
    // A stream opened with a session must not outlive it either.
    let session = match &state {
        AuthState::Authenticated(ctx) if ctx.is_session() => Some((ctx.session_id, ctx.expires_at)),
        _ => None,
    };
    req.extensions_mut().insert(state);
    req.extensions_mut().insert(meta);
    let mut response = next.run(req).await;
    if let Some(admitted) = admitted {
        admitted.respond(&mut response);
    } else if let Some((session_id, expires_at)) = session {
        Guarded::wrap(&mut response, session_outlived(layer.tokens.clone(), session_id, expires_at, layer.api.recheck));
    }
    response
}

/// Resolves when a session's access token stops being valid: it expired, or
/// the session was revoked (here or, through Valkey, elsewhere).
async fn session_outlived(tokens: Tokens, session_id: Uuid, expires_at: Time, recheck: Duration) {
    loop {
        let left = (expires_at - Utc::now()).to_std().unwrap_or_default();
        tokio::time::sleep(left.min(recheck)).await;
        if expires_at <= Utc::now() || tokens.is_revoked(session_id) || tokens.revoked_elsewhere(session_id).await {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies_split_over_several_header_fields_are_all_read() {
        let mut headers = http::HeaderMap::new();
        // As HTTP/2 may deliver them: one field per cookie, in any order.
        headers.append(http::header::COOKIE, "tokuly_ra=parked.token".parse().unwrap());
        headers.append(http::header::COOKIE, "tokuly_rt=active.token; theme=dark".parse().unwrap());
        assert_eq!(cookie_header(&headers), "tokuly_ra=parked.token; tokuly_rt=active.token; theme=dark");
        assert_eq!(cookie_header(&http::HeaderMap::new()), "");
    }

    fn tokens() -> Tokens {
        Tokens::new(&Keys::new("0123456789abcdef0123456789abcdef")).unwrap()
    }

    #[test]
    fn issue_then_verify_roundtrips() {
        let t = tokens();
        let (user, session) = (Uuid::now_v7(), Uuid::now_v7());
        let (token, _) = t.issue(user, session, &["moderator".into()]).unwrap();
        let ctx = t.verify(&token).unwrap();
        assert_eq!((ctx.user_id, ctx.session_id), (user, session));
        assert!(ctx.has_role("moderator") && ctx.is_staff() && !ctx.has_role("staff"));
    }

    #[test]
    fn tokens_from_another_secret_are_rejected() {
        let other = Tokens::new(&Keys::new("ffffffffffffffffffffffffffffffff")).unwrap();
        let (token, _) = other.issue(Uuid::now_v7(), Uuid::now_v7(), &[]).unwrap();
        assert_eq!(tokens().verify(&token).unwrap_err(), "TOKEN_INVALID");
    }

    #[test]
    fn revoked_sessions_fail_verification() {
        let t = tokens();
        let session = Uuid::now_v7();
        let (token, _) = t.issue(Uuid::now_v7(), session, &[]).unwrap();
        t.revoke_session(session);
        assert_eq!(t.verify(&token).unwrap_err(), "SESSION_REVOKED");
    }

    #[test]
    fn header_parsing_distinguishes_anonymous_from_invalid() {
        let t = tokens();
        assert!(matches!(t.authenticate_header(None), AuthState::Anonymous));
        let bad = http::HeaderValue::from_static("Bearer nope");
        assert!(matches!(
            t.authenticate_header(Some(&bad)),
            AuthState::Invalid("TOKEN_INVALID")
        ));
    }

    /// The client address a handler sees for a request from `peer` carrying
    /// this `X-Forwarded-For`, when only 10.0.0.0/8 holds our proxies.
    async fn address_seen(peer: Option<&str>, forwarded_for: Option<&str>) -> String {
        use tower::ServiceExt as _;

        let proxies = crate::proxies::TrustedProxies::parse(&["10.0.0.0/8".to_owned()]).unwrap();
        let app = axum::Router::new()
            .route("/", axum::routing::get(|axum::Extension(meta): axum::Extension<ClientMeta>| async move { meta.ip }))
            .layer(axum::middleware::from_fn_with_state(AuthLayer::new(tokens(), proxies), auth_middleware));
        let mut request = http::Request::builder().uri("/");
        if let Some(forwarded_for) = forwarded_for {
            request = request.header("x-forwarded-for", forwarded_for);
        }
        if let Some(peer) = peer {
            request = request.extension(axum::extract::ConnectInfo(std::net::SocketAddr::new(peer.parse().unwrap(), 40_000)));
        }
        let response = app.oneshot(request.body(axum::body::Body::empty()).unwrap()).await.unwrap();
        String::from_utf8(axum::body::to_bytes(response.into_body(), 256).await.unwrap().to_vec()).unwrap()
    }

    #[tokio::test]
    async fn the_client_address_comes_from_our_proxies_only() {
        // Through the edge proxy: the client it names.
        assert_eq!(address_seen(Some("10.0.0.2"), Some("203.0.113.7")).await, "203.0.113.7");
        // Reached directly: the header is the caller's own claim and is ignored.
        assert_eq!(address_seen(Some("198.51.100.4"), Some("203.0.113.7")).await, "198.51.100.4");
        assert_eq!(address_seen(Some("198.51.100.4"), None).await, "198.51.100.4");
        // Served without peer information: unknown rather than whatever the header says.
        assert_eq!(address_seen(None, Some("203.0.113.7")).await, "");
    }

    // ───────────────────────────── API tokens ─────────────────────────────

    use std::{
        collections::HashMap,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use axum::response::IntoResponse as _;
    use futures::StreamExt as _;

    use crate::api_tokens::{ApiCall, ApiTokenKind, ApiTokenVerifier, RateTier, TokenRejection};

    /// A verifier that knows a fixed set of tokens and remembers what the
    /// middleware reported.
    #[derive(Default)]
    struct FakeVerifier {
        known: HashMap<&'static str, Result<ApiToken, TokenRejection>>,
        calls: parking_lot::Mutex<Vec<(Uuid, ApiCall)>>,
        /// Tokens revoked while the test runs.
        revoked: parking_lot::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl ApiTokenVerifier for FakeVerifier {
        async fn verify(&self, token: &str) -> Result<ApiToken, TokenRejection> {
            if self.revoked.lock().iter().any(|revoked| revoked == token) {
                return Err(TokenRejection::Revoked);
            }
            self.known.get(token).cloned().unwrap_or(Err(TokenRejection::Invalid))
        }

        async fn record(&self, token: &ApiToken, call: ApiCall) {
            self.calls.lock().push((token.id, call));
        }
    }

    impl FakeVerifier {
        /// The calls reported so far, once `count` of them arrived (the hook runs after the response).
        async fn calls(&self, count: usize) -> Vec<(Uuid, ApiCall)> {
            for _ in 0..200 {
                if self.calls.lock().len() >= count {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            self.calls.lock().clone()
        }
    }

    fn api_token(scopes: &[&str]) -> ApiToken {
        ApiToken {
            kind: ApiTokenKind::Personal,
            id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
            app_id: None,
            scopes: Scopes::new(scopes.iter().copied()),
            expires_at: None,
            rate: RateTier { name: "standard".into(), per_minute: 0 },
        }
    }

    /// An app whose only handler says what the middleware made of the caller.
    struct TokenApp {
        router: axum::Router,
        verifier: Arc<FakeVerifier>,
        /// Requests that reached the handler.
        handled: Arc<AtomicUsize>,
    }

    impl TokenApp {
        fn new(known: impl IntoIterator<Item = (&'static str, Result<ApiToken, TokenRejection>)>) -> Self {
            let verifier = Arc::new(FakeVerifier { known: known.into_iter().collect(), ..Default::default() });
            let registry = ApiTokens::default();
            registry.register(verifier.clone());
            let layer = AuthLayer::new(tokens(), crate::proxies::TrustedProxies::parse(&[]).unwrap()).with_api_tokens(registry, None);
            Self::with_layer(layer, verifier)
        }

        fn with_layer(layer: AuthLayer, verifier: Arc<FakeVerifier>) -> Self {
            let handled = Arc::new(AtomicUsize::new(0));
            let counter = handled.clone();
            let describe = move |axum::Extension(state): axum::Extension<AuthState>, uri: http::Uri| async move {
                counter.fetch_add(1, Ordering::SeqCst);
                if uri.path().ends_with("/WatchAsset") {
                    // A server stream: one message, then silence for as long as the client listens.
                    let messages = futures::stream::once(async { Ok::<_, std::convert::Infallible>(bytes::Bytes::from_static(b"first")) }).chain(futures::stream::pending());
                    return axum::body::Body::from_stream(messages).into_response();
                }
                if uri.path().ends_with("/GetNoteThread") {
                    // What a handler's `AppError::not_found` looks like on the way out.
                    return (http::StatusCode::NOT_FOUND, [(crate::error::REASON_HEADER, "THING_NOT_FOUND")]).into_response();
                }
                let seen = match state {
                    AuthState::Anonymous => "anonymous".to_owned(),
                    AuthState::Invalid(reason) => format!("invalid {reason}"),
                    AuthState::Authenticated(auth) => format!(
                        "user {} session {} roles {:?} scopes {}",
                        auth.user_id,
                        auth.session_id,
                        auth.roles,
                        auth.scopes().map(Scopes::to_string).unwrap_or_else(|| "everything".to_owned())
                    ),
                };
                seen.into_response()
            };
            let router = axum::Router::new().fallback(describe).layer(axum::middleware::from_fn_with_state(layer, auth_middleware));
            Self { router, verifier, handled }
        }

        async fn call(&self, path: &str, bearer: Option<&str>, content_type: &str) -> (http::response::Parts, bytes::Bytes) {
            use tower::ServiceExt as _;

            let mut request = http::Request::builder().method("POST").uri(path).header("content-type", content_type).header("user-agent", "curl/8");
            if let Some(bearer) = bearer {
                request = request.header("authorization", format!("Bearer {bearer}"));
            }
            let response = self.router.clone().oneshot(request.body(axum::body::Body::empty()).unwrap()).await.unwrap();
            let (parts, body) = response.into_parts();
            (parts, axum::body::to_bytes(body, 64 * 1024).await.unwrap())
        }

        /// A Connect unary call: status, error reason and body text.
        async fn unary(&self, path: &str, bearer: Option<&str>) -> (u16, String, String) {
            let (parts, body) = self.call(path, bearer, "application/json").await;
            let reason = parts.headers.get(crate::error::REASON_HEADER).and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
            (parts.status.as_u16(), reason, String::from_utf8_lossy(&body).into_owned())
        }
    }

    const LIST_NOTES: &str = "/tokuly.notes.v1.NoteService/GetNote";
    const CREATE_NOTE: &str = "/tokuly.notes.v1.NoteService/CreateNote";

    #[tokio::test]
    async fn an_api_token_acts_as_its_account_within_its_scopes() {
        let token = api_token(&["notes:write", "geo:read"]);
        let app = TokenApp::new([("tkl_pat_reader", Ok(token.clone()))]);
        let expected = format!("user {} session {} roles [] scopes geo:read notes:write", token.user_id, token.id);
        // Writing includes reading.
        for path in [LIST_NOTES, CREATE_NOTE, "/tokuly.geo.v1.LocationService/ListFriendLocations"] {
            assert_eq!(app.unary(path, Some("tkl_pat_reader")).await, (200, String::new(), expected.clone()), "{path}");
        }
    }

    #[tokio::test]
    async fn a_token_without_the_scope_never_reaches_the_handler() {
        let app = TokenApp::new([("tkl_pat_reader", Ok(api_token(&["notes:read"])))]);
        let (parts, body) = app.call(CREATE_NOTE, Some("tkl_pat_reader"), "application/json").await;
        assert_eq!(parts.status, http::StatusCode::FORBIDDEN);
        assert_eq!(parts.headers[crate::error::REASON_HEADER], "INSUFFICIENT_SCOPE");
        assert_eq!(parts.headers["x-tokuly-scope"], "notes:write");
        assert_eq!(parts.headers["www-authenticate"], r#"Bearer error="insufficient_scope", scope="notes:write""#);
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["code"], "permission_denied");
        assert_eq!(error["message"], "this token lacks the scope notes:write");
        // Another domain entirely.
        let (status, reason, _) = app.unary("/tokuly.geo.v1.LocationService/ListFriendLocations", Some("tkl_pat_reader")).await;
        assert_eq!((status, reason.as_str()), (403, "INSUFFICIENT_SCOPE"));
        assert_eq!(app.handled.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn refusals_speak_the_protocol_of_the_request() {
        let app = TokenApp::new([("tkl_pat_reader", Ok(api_token(&["notes:read"])))]);
        let reason = |parts: &http::response::Parts| parts.headers[crate::error::REASON_HEADER].to_str().unwrap().to_owned();

        // Connect streaming: 200 and an end-of-stream envelope carrying the error.
        let (parts, body) = app.call(CREATE_NOTE, Some("tkl_pat_reader"), "application/connect+json").await;
        assert_eq!((parts.status.as_u16(), reason(&parts).as_str()), (200, "INSUFFICIENT_SCOPE"));
        assert_eq!(parts.headers["content-type"], "application/connect+json");
        assert_eq!(body[0], 0x02, "end-of-stream flag");
        let end: serde_json::Value = serde_json::from_slice(&body[5..]).unwrap();
        assert_eq!(end["error"]["code"], "permission_denied");
        assert_eq!(end["metadata"][crate::error::REASON_HEADER][0], "INSUFFICIENT_SCOPE");

        // gRPC: 200 with the status where a trailers-only response puts it.
        let (parts, body) = app.call(CREATE_NOTE, Some("tkl_pat_reader"), "application/grpc").await;
        assert_eq!((parts.status.as_u16(), reason(&parts).as_str()), (200, "INSUFFICIENT_SCOPE"));
        assert_eq!(parts.headers["grpc-status"], "7");
        assert_eq!(parts.headers["grpc-message"], "this token lacks the scope notes:write");
        assert!(body.is_empty());

        // gRPC-Web: 200 with the trailers as the only body frame.
        let (parts, body) = app.call(CREATE_NOTE, Some("tkl_pat_reader"), "application/grpc-web+proto").await;
        assert_eq!((parts.status.as_u16(), reason(&parts).as_str()), (200, "INSUFFICIENT_SCOPE"));
        assert_eq!(body[0], 0x80, "trailer frame flag");
        let trailers = String::from_utf8_lossy(&body[5..]).into_owned();
        assert!(trailers.contains("grpc-status: 7\r\n") && trailers.contains("x-tokuly-error: INSUFFICIENT_SCOPE\r\n"), "{trailers}");
        assert_eq!(app.handled.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn tokens_cannot_reach_session_only_rpcs_or_anything_that_is_not_an_rpc() {
        // Even a token holding every scope of the domain.
        let app = TokenApp::new([("tkl_pat_all", Ok(api_token(&["identity:write", "dev:write", "admin:write"])))]);
        for path in [
            "/tokuly.identity.v1.AuthService/SignIn",
            "/tokuly.identity.v1.AccountService/ChangePassword",
            "/tokuly.identity.v1.AccountService/RequestAccountDeletion",
            "/tokuly.dev.v1.TokenService/CreateToken",
            "/tokuly.admin.v1.UserAdminService/SuspendUser",
        ] {
            let (status, reason, _) = app.unary(path, Some("tkl_pat_all")).await;
            assert_eq!((status, reason.as_str()), (403, "SESSION_REQUIRED"), "{path}");
        }
        for path in ["/oauth/token", "/healthz", "/"] {
            let (status, reason, _) = app.unary(path, Some("tkl_pat_all")).await;
            assert_eq!((status, reason.as_str()), (403, "TOKEN_NOT_ALLOWED"), "{path}");
        }
        assert_eq!(app.handled.load(Ordering::SeqCst), 0);
        // What every token may call, whatever its scopes.
        let (status, _, _) = app.unary("/tokuly.dev.v1.TokenService/GetCurrentToken", Some("tkl_pat_all")).await;
        assert_eq!(status, 200);
    }

    #[tokio::test]
    async fn a_rejected_token_leaves_the_request_unauthenticated_with_the_reason() {
        let app = TokenApp::new([
            ("tkl_pat_revoked", Err(TokenRejection::Revoked)),
            ("tkl_pat_expired", Err(TokenRejection::Expired)),
            ("tkl_pat_outage", Err(TokenRejection::Unavailable)),
        ]);
        for (token, seen) in [("tkl_pat_revoked", "invalid TOKEN_REVOKED"), ("tkl_pat_expired", "invalid TOKEN_EXPIRED"), ("tkl_pat_unknown", "invalid TOKEN_INVALID")] {
            assert_eq!(app.unary(LIST_NOTES, Some(token)).await, (200, String::new(), seen.to_owned()), "{token}");
        }
        // A verifier that cannot answer fails the request instead of demoting the caller to a guest.
        let (status, reason, _) = app.unary(LIST_NOTES, Some("tkl_pat_outage")).await;
        assert_eq!((status, reason.as_str()), (503, "TOKEN_CHECK_UNAVAILABLE"));
        assert!(app.verifier.calls(0).await.is_empty(), "nothing is recorded for tokens that were never valid");

        // Without a verifier no API token is valid.
        let bare = TokenApp::with_layer(AuthLayer::new(tokens(), crate::proxies::TrustedProxies::parse(&[]).unwrap()), Arc::default());
        assert_eq!(bare.unary(LIST_NOTES, Some("tkl_pat_reader")).await.2, "invalid TOKEN_INVALID");
    }

    #[tokio::test]
    async fn sessions_are_unaffected_by_the_token_rules() {
        let app = TokenApp::new([]);
        let (user, session) = (Uuid::now_v7(), Uuid::now_v7());
        let (jwt, _) = tokens().issue(user, session, &["admin".into()]).unwrap();
        let expected = format!("user {user} session {session} roles [\"admin\"] scopes everything");
        // Including the RPCs and paths closed to API tokens.
        for path in [CREATE_NOTE, "/tokuly.identity.v1.AccountService/ChangePassword", "/tokuly.admin.v1.UserAdminService/SuspendUser", "/healthz"] {
            assert_eq!(app.unary(path, Some(&jwt)).await, (200, String::new(), expected.clone()), "{path}");
        }
        assert_eq!(app.unary(CREATE_NOTE, None).await.2, "anonymous");
        assert_eq!(app.unary(CREATE_NOTE, Some("nope")).await.2, "invalid TOKEN_INVALID");
        assert!(app.verifier.calls(0).await.is_empty());

        let auth = tokens().verify(&jwt).unwrap();
        assert!(auth.is_session() && auth.scopes().is_none() && auth.allows("geo:write"));
        let auth = AuthContext::from_api_token(api_token(&["geo:read"]));
        assert!(!auth.is_session() && auth.allows("geo:read") && !auth.allows("geo:write") && !auth.is_staff());
    }

    #[tokio::test]
    async fn every_request_made_with_a_token_is_reported_to_its_verifier() {
        let token = api_token(&["notes:read"]);
        let app = TokenApp::new([("tkl_pat_reader", Ok(token.clone()))]);
        app.unary(LIST_NOTES, Some("tkl_pat_reader")).await;
        app.unary("/tokuly.notes.v1.NoteService/GetNoteThread", Some("tkl_pat_reader")).await;
        app.unary(CREATE_NOTE, Some("tkl_pat_reader")).await;
        let calls = app.verifier.calls(3).await;
        let mut seen: Vec<(String, Option<String>)> = calls.iter().map(|(_, call)| (call.procedure.clone(), call.error.clone())).collect();
        seen.sort();
        assert_eq!(
            seen,
            [
                ("tokuly.notes.v1.NoteService/CreateNote".to_owned(), Some("INSUFFICIENT_SCOPE".to_owned())),
                ("tokuly.notes.v1.NoteService/GetNote".to_owned(), None),
                ("tokuly.notes.v1.NoteService/GetNoteThread".to_owned(), Some("THING_NOT_FOUND".to_owned())),
            ]
        );
        assert!(calls.iter().all(|(id, call)| *id == token.id && call.user_agent == "curl/8"));
    }

    #[tokio::test]
    async fn an_rpc_nobody_classified_is_closed_to_tokens() {
        let app = TokenApp::new([("tkl_pat_all", Ok(api_token(&["notes:write", "live:write", "wallet:write"])))]);
        // Names a prefix rule would have waved through as reads, and a service that does not exist.
        for path in ["/tokuly.notes.v1.NoteService/BatchDeleteNotes", "/tokuly.wallet.v1.WalletService/CheckoutCart", "/tokuly.live.v1.LiveService/GetStreamKey"] {
            let (status, reason, _) = app.unary(path, Some("tkl_pat_all")).await;
            assert_eq!((status, reason.as_str()), (403, "TOKEN_NOT_ALLOWED"), "{path}");
        }
        assert_eq!(app.handled.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn an_rpc_that_needs_several_scopes_names_the_one_that_is_missing() {
        let app = TokenApp::new([("tkl_pat_feed", Ok(api_token(&["feed:read"]))), ("tkl_pat_both", Ok(api_token(&["feed:read", "geo:read"])))]);
        let (parts, _) = app.call("/tokuly.feed.v1.MapStoryService/ListMapStories", Some("tkl_pat_feed"), "application/json").await;
        assert_eq!((parts.status.as_u16(), parts.headers[crate::error::REASON_HEADER].to_str().unwrap()), (403, "INSUFFICIENT_SCOPE"));
        assert_eq!(parts.headers["x-tokuly-scope"], "geo:read");
        assert_eq!(app.unary("/tokuly.feed.v1.MapStoryService/ListMapStories", Some("tkl_pat_both")).await.0, 200);
        assert_eq!(app.unary("/tokuly.feed.v1.NewsService/ListNews", Some("tkl_pat_feed")).await.0, 200);
    }

    /// Reads a streaming response until it ends; `None` if it is still open after `patience`.
    async fn stream_ends_within(mut body: axum::body::BodyDataStream, patience: Duration) -> Option<Vec<bytes::Bytes>> {
        let mut received = Vec::new();
        let read = async {
            while let Some(Ok(chunk)) = body.next().await {
                received.push(chunk);
            }
        };
        tokio::time::timeout(patience, read).await.ok().map(|()| received)
    }

    #[tokio::test]
    async fn a_stream_ends_when_its_token_is_revoked_or_expires() {
        use tower::ServiceExt as _;

        let expiring = ApiToken { expires_at: Some(Utc::now() + chrono::Duration::milliseconds(300)), ..api_token(&["media:read"]) };
        let verifier = Arc::new(FakeVerifier {
            known: [("tkl_pat_stream", Ok(api_token(&["media:read"]))), ("tkl_pat_expiring", Ok(expiring))].into_iter().collect(),
            ..Default::default()
        });
        let registry = ApiTokens::default();
        registry.register(verifier.clone());
        let layer_tokens = tokens();
        let layer = AuthLayer::new(layer_tokens.clone(), crate::proxies::TrustedProxies::parse(&[]).unwrap())
            .with_api_tokens(registry, None)
            .with_stream_recheck(Duration::from_millis(50));
        let app = TokenApp::with_layer(layer, verifier.clone());
        let open = |bearer: String| {
            let router = app.router.clone();
            async move {
                let request = http::Request::builder().method("POST").uri("/tokuly.media.v1.MediaService/WatchAsset").header("content-type", "application/connect+json");
                let request = request.header("authorization", format!("Bearer {bearer}")).body(axum::body::Body::empty()).unwrap();
                router.oneshot(request).await.unwrap().into_body().into_data_stream()
            }
        };

        // While the token is good the stream stays open through many re-checks.
        let stream = open("tkl_pat_stream".to_owned()).await;
        assert!(stream_ends_within(stream, Duration::from_millis(400)).await.is_none());
        // Revoked while a stream is open: the stream ends at the next check.
        let stream = open("tkl_pat_stream".to_owned()).await;
        verifier.revoked.lock().push("tkl_pat_stream".to_owned());
        assert_eq!(stream_ends_within(stream, Duration::from_secs(5)).await.expect("the stream was cut off"), ["first"]);
        // A token that runs out mid-stream ends it too, without the verifier having to say so.
        let stream = open("tkl_pat_expiring".to_owned()).await;
        assert!(stream_ends_within(stream, Duration::from_secs(5)).await.is_some());
        // A session's stream is watched the same way: open while the session
        // lasts, cut off at the next check once it is revoked.
        let session = Uuid::now_v7();
        let (jwt, _) = layer_tokens.issue(Uuid::now_v7(), session, &[]).unwrap();
        assert!(stream_ends_within(open(jwt.clone()).await, Duration::from_millis(300)).await.is_none());
        let stream = open(jwt).await;
        layer_tokens.revoke(session).await;
        assert_eq!(stream_ends_within(stream, Duration::from_secs(5)).await.expect("the stream was cut off"), ["first"]);
    }
}
