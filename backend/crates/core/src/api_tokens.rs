//! API tokens: opaque bearer credentials for scripts, apps and bots.
//!
//! A signed-in session carries a JWT that core verifies on its own
//! ([`crate::auth::Tokens`]). An API token is an opaque string starting with
//! [`API_TOKEN_PREFIX`] that only the domain which issued it can judge. That
//! domain registers an [`ApiTokenVerifier`] in [`ApiTokens`]
//! (`core.api_tokens`), and [`crate::auth::auth_middleware`] asks it on every
//! request presenting such a token.
//!
//! Unlike a session, a token may do only what its [`Scopes`] say, and the
//! middleware enforces that before any handler runs, so no handler has to
//! know about tokens:
//!
//! 1. **Who**: the verifier turns the string into an [`ApiToken`] (the
//!    account it acts as, the credential and app behind it, its scopes and
//!    its [`RateTier`]), or rejects it. Tokens never carry staff roles.
//! 2. **How often**: requests are counted per token and minute in Valkey;
//!    past the tier's limit the request is refused with `RATE_LIMITED`.
//!    Every response to a token carries `x-ratelimit-*` headers.
//! 3. **What**: the scopes an RPC needs are written down per RPC
//!    ([`crate::api_access`]). A token without them is refused with
//!    `permission_denied` / `INSUFFICIENT_SCOPE`; RPCs that need a person at
//!    a screen (signing in, changing a password, minting tokens) are refused
//!    with `SESSION_REQUIRED`; anything that is not a listed RPC with
//!    `TOKEN_NOT_ALLOWED`.
//! 4. **For how long**: a response that keeps streaming is cut off when its
//!    token is revoked or expires (it is checked again every half minute).
//!
//! Refusals are rendered by [`connectrpc::ConnectError::into_http_response`]
//! in the wire format of the request, so Connect, gRPC and gRPC-Web clients
//! all see a proper error with the reason in `x-tokuly-error`, exactly as if
//! a handler had returned the [`AppError`].
//!
//! After the response the verifier's [`ApiTokenVerifier::record`] hook is
//! told what was called and how it ended (usage statistics, "last used").
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use chrono::Utc;
use connectrpc::ConnectError;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::ClientMeta,
    cache::Cache,
    error::{AppError, AppResult, REASON_HEADER},
    time::Time,
};

/// Every API token starts with this; nothing else presented as a bearer
/// token does (JWTs start with `eyJ`).
pub const API_TOKEN_PREFIX: &str = "tkl_";

/// Whether a bearer credential claims to be an API token.
pub fn is_api_token(bearer: &str) -> bool {
    bearer.starts_with(API_TOKEN_PREFIX)
}

// ───────────────────────────── scopes ─────────────────────────────

/// Whether `scope` has the shape `<domain>:read` or `<domain>:write`.
pub fn is_scope(scope: &str) -> bool {
    scope.split_once(':').is_some_and(|(domain, level)| is_domain(domain) && matches!(level, "read" | "write"))
}

fn is_domain(domain: &str) -> bool {
    domain.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && domain.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// The scopes of a token: a sorted set of `<domain>:read` / `<domain>:write`.
/// Writing includes reading.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Scopes(Vec<String>);

impl Scopes {
    pub fn new(scopes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let mut scopes: Vec<String> = scopes.into_iter().map(Into::into).collect();
        scopes.sort();
        scopes.dedup();
        Self(scopes)
    }

    /// Whether these scopes cover `scope` (`x:write` covers `x:read`).
    pub fn allows(&self, scope: &str) -> bool {
        let has = |wanted: &str| self.0.iter().any(|s| s == wanted);
        has(scope) || scope.strip_suffix(":read").is_some_and(|domain| has(&format!("{domain}:write")))
    }

    /// Whether every one of `other`'s scopes is covered by these.
    pub fn covers(&self, other: &Scopes) -> bool {
        other.0.iter().all(|scope| self.allows(scope))
    }

    pub fn as_slice(&self) -> &[String] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn into_vec(self) -> Vec<String> {
        self.0
    }
}

/// Space separated, as OAuth writes scope lists.
impl std::fmt::Display for Scopes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.join(" "))
    }
}

pub use crate::api_access::{Access, access_for};

// ───────────────────────────── tokens ─────────────────────────────

/// What kind of credential a token is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiTokenKind {
    /// A personal access token: its owner scripting their own account.
    Personal,
    /// An OAuth access token: an application acting for a user who agreed to it.
    OAuth,
    /// A bot token: acts as a bot account.
    Bot,
}

/// How many requests a token may make.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateTier {
    /// Name of the tier, for people (`standard`, `elevated`...).
    pub name: String,
    /// Requests per minute; 0 means unlimited.
    pub per_minute: u32,
}

/// A verified API token: who it acts as and what it may do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiToken {
    pub kind: ApiTokenKind,
    /// The credential, for audit and rate limits: the personal or bot
    /// token, or — for OAuth — the user's authorization of the app (which
    /// outlives the access tokens issued under it).
    pub id: Uuid,
    /// The account the token acts as.
    pub user_id: Uuid,
    /// The application acting, for OAuth tokens.
    pub app_id: Option<Uuid>,
    pub scopes: Scopes,
    /// `None` for tokens that do not expire.
    pub expires_at: Option<Time>,
    pub rate: RateTier,
}

/// Why a verifier turned a token down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenRejection {
    /// Malformed or unknown.
    Invalid,
    Expired,
    /// Revoked, or its account or application is suspended or gone.
    Revoked,
    /// The verifier could not reach its stores. The request fails with
    /// `unavailable` instead of being treated as unauthenticated.
    Unavailable,
}

impl TokenRejection {
    /// The reason clients see in `x-tokuly-error`.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Invalid => "TOKEN_INVALID",
            Self::Expired => "TOKEN_EXPIRED",
            Self::Revoked => "TOKEN_REVOKED",
            Self::Unavailable => "TOKEN_CHECK_UNAVAILABLE",
        }
    }
}

/// One finished request made with an API token.
#[derive(Debug, Clone)]
pub struct ApiCall {
    /// `tokuly.notes.v1.NoteService/CreateNote`; the request path for
    /// anything that is not an RPC.
    pub procedure: String,
    /// The caller's address and user agent (see [`ClientMeta`]).
    pub ip: String,
    pub user_agent: String,
    /// The error reason (`x-tokuly-error`), or `None` when the call succeeded.
    pub error: Option<String>,
    pub duration: Duration,
}

/// Judges API tokens. Implemented by the domain that issues them.
#[async_trait]
pub trait ApiTokenVerifier: Send + Sync + 'static {
    /// Who `token` acts as and what it may do. This runs on every request
    /// made with a token, so the answer should come from a cache.
    async fn verify(&self, token: &str) -> Result<ApiToken, TokenRejection>;

    /// A request made with `token` finished (or was refused by the
    /// middleware). Runs after the response, off the request path.
    async fn record(&self, _token: &ApiToken, _call: ApiCall) {}
}

/// Registry of the API token verifier (`core.api_tokens`). The domain that
/// issues tokens registers itself when it is constructed; without one every
/// API token is invalid.
#[derive(Clone, Default)]
pub struct ApiTokens {
    verifier: Arc<RwLock<Option<Arc<dyn ApiTokenVerifier>>>>,
}

impl ApiTokens {
    pub fn register(&self, verifier: Arc<dyn ApiTokenVerifier>) {
        *self.verifier.write() = Some(verifier);
    }

    fn verifier(&self) -> Option<Arc<dyn ApiTokenVerifier>> {
        self.verifier.read().clone()
    }

    /// Verifies a token outside the HTTP middleware (other entry points,
    /// such as the realtime gateway). Scopes and rate limits are then the
    /// caller's to enforce.
    pub async fn verify(&self, token: &str) -> Result<ApiToken, TokenRejection> {
        match self.verifier() {
            Some(verifier) => verifier.verify(token).await,
            None => Err(TokenRejection::Invalid),
        }
    }
}

// ───────────────────────────── the gate ─────────────────────────────

/// Where a token stands against its rate limit in the current minute.
#[derive(Debug, Clone, Copy)]
struct RateStatus {
    limit: u32,
    remaining: u32,
    /// Seconds until the window ends.
    reset_seconds: u32,
    exceeded: bool,
}

impl RateStatus {
    fn write_headers(&self, headers: &mut http::HeaderMap) {
        headers.insert("x-ratelimit-limit", self.limit.into());
        headers.insert("x-ratelimit-remaining", self.remaining.into());
        headers.insert("x-ratelimit-reset", self.reset_seconds.into());
        if self.exceeded {
            headers.insert(http::header::RETRY_AFTER, self.reset_seconds.into());
        }
    }
}

/// Counts one request against the token's limit (fixed one-minute windows).
async fn count_request(cache: &Cache, token: &ApiToken) -> AppResult<RateStatus> {
    let now = Utc::now().timestamp();
    let key = format!("rl:api:{}:{}", token.id.simple(), now / 60);
    let (count,): (i64,) =
        redis::pipe().atomic().incr(&key, 1).expire(&key, 120).ignore().query_async(&mut cache.conn()).await?;
    let limit = token.rate.per_minute;
    Ok(RateStatus {
        limit,
        remaining: (limit as i64 - count).clamp(0, limit as i64) as u32,
        reset_seconds: (60 - now.rem_euclid(60)) as u32,
        exceeded: count > limit as i64,
    })
}

/// How often a token is verified again while its response is still streaming.
const STREAM_RECHECK: Duration = Duration::from_secs(30);

/// The API-token half of the auth middleware.
#[derive(Clone)]
pub(crate) struct ApiGate {
    pub(crate) tokens: ApiTokens,
    /// Where requests are counted; `None` switches rate limits off.
    pub(crate) limiter: Option<Cache>,
    /// See [`STREAM_RECHECK`].
    pub(crate) recheck: Duration,
}

impl Default for ApiGate {
    fn default() -> Self {
        Self { tokens: ApiTokens::default(), limiter: None, recheck: STREAM_RECHECK }
    }
}

/// Why the gate did not admit a request.
pub(crate) enum Refusal {
    /// The token is not valid: the request continues unauthenticated with
    /// this reason, like one with a bad session token.
    Unauthenticated(&'static str),
    /// The request ends here with this response.
    Respond(axum::response::Response),
}

/// A request admitted with an API token, until its response is known.
pub(crate) struct Admitted {
    verifier: Arc<dyn ApiTokenVerifier>,
    pub(crate) token: ApiToken,
    procedure: String,
    ip: String,
    user_agent: String,
    started: Instant,
    rate: Option<RateStatus>,
    /// The token as presented, to verify it again while a stream is open.
    secret: String,
    recheck: Duration,
}

impl ApiGate {
    /// Verifies `token` and decides whether it may make this request.
    pub(crate) async fn admit(
        &self,
        token: &str,
        path: &str,
        request_headers: &http::HeaderMap,
        client: &ClientMeta,
    ) -> Result<Admitted, Refusal> {
        let Some(verifier) = self.tokens.verifier() else {
            return Err(Refusal::Unauthenticated(TokenRejection::Invalid.reason()));
        };
        let secret = token.to_owned();
        let token = match verifier.verify(token).await {
            Ok(token) => token,
            Err(TokenRejection::Unavailable) => {
                let error = AppError::unavailable(
                    TokenRejection::Unavailable.reason(),
                    "the token could not be checked right now, try again",
                );
                return Err(Refusal::Respond(render(error, request_headers)));
            }
            Err(rejection) => return Err(Refusal::Unauthenticated(rejection.reason())),
        };
        let mut admitted = Admitted {
            verifier,
            token,
            procedure: path.trim_start_matches('/').to_owned(),
            ip: client.ip.clone(),
            user_agent: client.user_agent.clone(),
            started: Instant::now(),
            rate: None,
            secret,
            recheck: self.recheck,
        };
        if let (Some(cache), true) = (&self.limiter, admitted.token.rate.per_minute > 0) {
            match count_request(cache, &admitted.token).await {
                Ok(status) => admitted.rate = Some(status),
                // A counter that cannot be reached must not take the API down with it.
                Err(e) => tracing::warn!(error = %e, "API token rate limit could not be counted; request allowed"),
            }
        }
        if admitted.rate.is_some_and(|rate| rate.exceeded) {
            let error = AppError::rate_limited(
                "RATE_LIMITED",
                format!("this token is limited to {} requests per minute", admitted.token.rate.per_minute),
            );
            return Err(admitted.refuse(error, None, request_headers));
        }
        match access_for(path) {
            Some(Access::Scopes(needed)) => match needed.iter().find(|scope| !admitted.token.scopes.allows(scope)) {
                Some(missing) => {
                    let error = AppError::denied("INSUFFICIENT_SCOPE", format!("this token lacks the scope {missing}"));
                    Err(admitted.refuse(error, Some(missing), request_headers))
                }
                None => Ok(admitted),
            },
            Some(Access::AnyToken) => Ok(admitted),
            Some(Access::SessionOnly) => {
                let error =
                    AppError::denied("SESSION_REQUIRED", "API tokens cannot call this; it takes a signed-in session");
                Err(admitted.refuse(error, None, request_headers))
            }
            None => {
                // Not an RPC, or one nobody has classified: closed to tokens.
                let error = AppError::denied("TOKEN_NOT_ALLOWED", "API tokens cannot call this");
                Err(admitted.refuse(error, None, request_headers))
            }
        }
    }
}

impl Admitted {
    /// Ends the request with `error`, naming the scope that was missing.
    fn refuse(self, error: AppError, missing_scope: Option<&str>, request_headers: &http::HeaderMap) -> Refusal {
        let reason = error.reason.to_string();
        let mut response = render(error, request_headers);
        if let Some(scope) = missing_scope {
            // RFC 6750 section 3.1, for OAuth clients; `x-tokuly-scope` for everyone else.
            let challenge = format!(r#"Bearer error="insufficient_scope", scope="{scope}""#);
            for (name, value) in [("www-authenticate", challenge.as_str()), ("x-tokuly-scope", scope)] {
                if let Ok(value) = http::HeaderValue::from_str(value) {
                    response.headers_mut().insert(name, value);
                }
            }
        }
        self.finish(&mut response, Some(reason));
        Refusal::Respond(response)
    }

    /// Adds the rate limit headers and reports the finished call to the verifier.
    pub(crate) fn respond(self, response: &mut axum::response::Response) {
        let error = error_reason(response);
        self.finish(response, error);
    }

    fn finish(self, response: &mut axum::response::Response, error: Option<String>) {
        if let Some(rate) = &self.rate {
            rate.write_headers(response.headers_mut());
        }
        let call = ApiCall {
            procedure: self.procedure,
            ip: self.ip,
            user_agent: self.user_agent,
            error,
            duration: self.started.elapsed(),
        };
        // A stream opened with the token must not outlive it.
        let watch = outlived(self.verifier.clone(), self.secret, self.token.expires_at, self.recheck);
        Guarded::wrap(response, watch);
        let (verifier, token) = (self.verifier, self.token);
        tokio::spawn(async move { verifier.record(&token, call).await });
    }
}

/// Resolves when the token stops being valid: revoked, or expired.
async fn outlived(verifier: Arc<dyn ApiTokenVerifier>, secret: String, expires_at: Option<Time>, recheck: Duration) {
    loop {
        let until_expiry = expires_at.map(|at| (at - Utc::now()).to_std().unwrap_or_default());
        tokio::time::sleep(until_expiry.map_or(recheck, |left| left.min(recheck))).await;
        if expires_at.is_some_and(|at| at <= Utc::now()) {
            return;
        }
        // A verifier that cannot answer right now is not a revocation.
        if matches!(verifier.verify(&secret).await, Err(rejection) if rejection != TokenRejection::Unavailable) {
            return;
        }
    }
}

/// A response body that ends when its token does. The watch costs a timer
/// per response; a verification only when a response is still open after
/// the re-check interval.
pub(crate) struct Guarded {
    inner: axum::body::Body,
    watch: Option<std::pin::Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl Guarded {
    /// Makes `response` end when `watch` resolves.
    pub(crate) fn wrap(response: &mut axum::response::Response, watch: impl Future<Output = ()> + Send + 'static) {
        let body = std::mem::take(response.body_mut());
        *response.body_mut() = axum::body::Body::new(Guarded { inner: body, watch: Some(Box::pin(watch)) });
    }
}

impl connectrpc::http_body::Body for Guarded {
    type Data = bytes::Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<connectrpc::http_body::Frame<Self::Data>, Self::Error>>> {
        use std::task::Poll;
        let Some(watch) = self.watch.as_mut() else { return Poll::Ready(None) };
        if watch.as_mut().poll(cx).is_ready() {
            // The token is gone: the stream ends here, without the trailers a
            // complete response would carry, so clients see it as cut off.
            self.watch = None;
            return Poll::Ready(None);
        }
        std::pin::Pin::new(&mut self.inner).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.watch.is_none() || self.inner.is_end_stream()
    }

    fn size_hint(&self) -> connectrpc::http_body::SizeHint {
        self.inner.size_hint()
    }
}

/// The error reason of a response, however its protocol reports failure:
/// `None` for a success.
fn error_reason(response: &axum::response::Response) -> Option<String> {
    let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok());
    if let Some(reason) = header(REASON_HEADER) {
        return Some(reason.to_owned());
    }
    // gRPC reports failures that happen before the first message in the headers.
    if let Some(status) = header("grpc-status").filter(|status| *status != "0") {
        return Some(format!("GRPC_{status}"));
    }
    let status = response.status();
    (status.is_client_error() || status.is_server_error()).then(|| format!("HTTP_{}", status.as_u16()))
}

/// Renders `error` as the response a handler returning it would have
/// produced, in the wire format of the request (Connect unary, Connect
/// streaming, gRPC or gRPC-Web).
fn render(error: AppError, request_headers: &http::HeaderMap) -> axum::response::Response {
    ConnectError::from(error).into_http_response(request_headers).map(axum::body::Body::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_are_validated_by_shape() {
        for good in ["geo:read", "chat:write", "a1_b:read"] {
            assert!(is_scope(good), "{good}");
        }
        for bad in ["", "geo", "geo:", ":read", "geo:admin", "Geo:read", "1geo:read", "geo:read:write", "geo read"] {
            assert!(!is_scope(bad), "{bad:?}");
        }
    }

    #[test]
    fn writing_includes_reading_but_not_the_reverse() {
        let scopes = Scopes::new(["notes:write", "geo:read", "geo:read"]);
        assert_eq!(scopes.as_slice(), ["geo:read", "notes:write"]);
        assert_eq!(scopes.to_string(), "geo:read notes:write");
        assert!(scopes.allows("notes:write") && scopes.allows("notes:read") && scopes.allows("geo:read"));
        assert!(!scopes.allows("geo:write") && !scopes.allows("chat:read") && !scopes.allows("notes"));
        assert!(scopes.covers(&Scopes::new(["notes:read", "geo:read"])));
        assert!(!scopes.covers(&Scopes::new(["notes:read", "geo:write"])));
        assert!(Scopes::default().covers(&Scopes::default()) && !Scopes::default().allows("geo:read"));
    }
}
