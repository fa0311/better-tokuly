//! Access tokens and the per-request authentication state.
//!
//! Access tokens are short-lived EdDSA (Ed25519) JWTs. The signing key is
//! derived from the master secret, so the API and the realtime gateway verify
//! each other's tokens without sharing key files. Revoked sessions are held in
//! a small in-memory set (fed over the bus by the identity service) for the
//! lifetime of an access token.
use std::{sync::Arc, time::Duration};

use axum::{extract::Request, middleware::Next, response::Response};
use chrono::Utc;
use dashmap::DashMap;
use ed25519_dalek::{SigningKey, pkcs8::EncodePrivateKey as _};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
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
    pub session_id: Uuid,
    pub roles: Vec<String>,
    /// When the access token that proved this identity stops being valid.
    pub expires_at: Time,
}

impl AuthContext {
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role || r == "admin")
    }

    pub fn is_staff(&self) -> bool {
        self.roles.iter().any(|r| r == "admin" || r == "moderator" || r == "staff")
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
}

struct TokensInner {
    encoding: EncodingKey,
    decoding: DecodingKey,
    validation: Validation,
    /// Revoked session IDs → instant after which the entry can be dropped.
    revoked: DashMap<Uuid, Time>,
    public_key: [u8; 32],
}

impl Tokens {
    pub fn new(keys: &Keys) -> anyhow::Result<Self> {
        let signing = SigningKey::from_bytes(&keys.derive("jwt/ed25519"));
        let pkcs8 = signing.to_pkcs8_der().map_err(|e| anyhow::anyhow!("failed to encode signing key: {e}"))?;
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
            }),
        })
    }

    /// Raw Ed25519 public key (for publishing as a JWK).
    pub fn public_key(&self) -> [u8; 32] {
        self.inner.public_key
    }

    /// Issues an access token; returns it with its expiry.
    pub fn issue(&self, user_id: Uuid, session_id: Uuid, roles: &[String]) -> AppResult<(String, Time)> {
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
        let token = jsonwebtoken::encode(&Header::new(Algorithm::EdDSA), &claims, &self.inner.encoding)
            .map_err(|e| AppError::internal(anyhow::anyhow!("jwt encode: {e}")))?;
        Ok((token, expires))
    }

    /// Verifies a token and returns who it belongs to.
    pub fn verify(&self, token: &str) -> Result<AuthContext, &'static str> {
        let data =
            jsonwebtoken::decode::<Claims>(token, &self.inner.decoding, &self.inner.validation).map_err(|e| match e
                .kind()
            {
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
        })
    }

    /// Marks a session as revoked for the remaining lifetime of its tokens.
    pub fn revoke_session(&self, session_id: Uuid) {
        self.inner.revoked.insert(session_id, Utc::now() + ACCESS_TOKEN_TTL + Duration::from_secs(30));
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

    /// Extracts and verifies a bearer token from an `Authorization` header value.
    pub fn authenticate_header(&self, header: Option<&http::HeaderValue>) -> AuthState {
        let Some(value) = header.and_then(|v| v.to_str().ok()) else {
            return AuthState::Anonymous;
        };
        let Some(token) = value.strip_prefix("Bearer ").or_else(|| value.strip_prefix("bearer ")) else {
            return AuthState::Invalid("TOKEN_INVALID");
        };
        match self.verify(token.trim()) {
            Ok(ctx) => AuthState::Authenticated(ctx),
            Err(reason) => AuthState::Invalid(reason),
        }
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

/// What [`auth_middleware`] needs: the token verifier, and the proxies whose
/// word is taken for a client's address.
#[derive(Clone)]
pub struct AuthLayer {
    tokens: Tokens,
    trusted_proxies: std::sync::Arc<crate::proxies::TrustedProxies>,
}

impl AuthLayer {
    pub fn new(tokens: Tokens, trusted_proxies: crate::proxies::TrustedProxies) -> Self {
        Self { tokens, trusted_proxies: std::sync::Arc::new(trusted_proxies) }
    }
}

/// Axum middleware: authenticates the request (never rejects) and records
/// [`AuthState`] and [`ClientMeta`] for handlers.
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
    let state = layer.tokens.authenticate_header(headers.get(http::header::AUTHORIZATION));
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
        cookies: text("cookie"),
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
    req.extensions_mut().insert(state);
    req.extensions_mut().insert(meta);
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(matches!(t.authenticate_header(Some(&bad)), AuthState::Invalid("TOKEN_INVALID")));
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
            request =
                request.extension(axum::extract::ConnectInfo(std::net::SocketAddr::new(peer.parse().unwrap(), 40_000)));
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
}
