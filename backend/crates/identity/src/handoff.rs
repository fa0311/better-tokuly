//! Opening the web signed in from an app: a one-time hand-off.
//!
//! The app (Bearer tokens) shows the screens it does not have yet in a web
//! view. It must not hand the web its own tokens, and nothing that signs in
//! may travel in a URL for longer than a moment. So:
//!
//! 1. The app makes a random verifier and sends its SHA-256 with
//!    `CreateWebHandoff` (as its session), which makes a code: 256 random
//!    bits, kept in Valkey only as its keyed hash, for 60 seconds, bound to
//!    that session, to a path of the web app checked here and to the
//!    verifier's hash.
//! 2. The web view loads `GET <api prefix>/auth/handoff?code=…` with the
//!    verifier in the [`VERIFIER_HEADER`] of that first request. That
//!    request takes the code (GETDEL: once, whoever comes second finds
//!    nothing), and only when the verifier matches makes a **new** web
//!    session of the same account linked to the app's session
//!    (`sessions.linked_to`), sets the refresh cookie as a sign-in on the
//!    web would and redirects to the path.
//!
//! The verifier is what makes a code useless to anyone but the app that
//! asked for it, the way PKCE binds an OAuth code. A code is a link: whoever
//! gets one (sent by its owner, leaked from a log or a screenshot) could
//! otherwise sign a victim's browser in as the code's account — login CSRF,
//! and the web map would then share the victim's location with that account,
//! their posts and messages would go out as it. A browser following a link
//! sends no such header, and no page can make it send one on a navigation,
//! so a load without the verifier signs nobody in (and leaves the code to
//! the app); a wrong one spends the code.
//!
//! The web session is the app session's: whatever ends the app's session
//! (signing out, removing the account from the device, "sign out
//! everywhere", a password change elsewhere, a ban, deletion) ends it in the
//! same statement (the `sessions_revoke_linked` trigger), and it refreshes
//! only while the app's session is the active account of its device
//! ([`Identity::app_session_in_use`]). An app session has at most one: a
//! newer hand-off replaces the one before. Each hand-off is written to the
//! account's security log (`WEB_SIGN_IN_FROM_APP`).
//!
//! Only sessions make codes: API tokens and OAuth apps cannot
//! (`api_access`), and neither can a web session that came from a hand-off
//! (no chains).
use std::time::Duration;

use axum::{
    Extension, Router,
    extract::{RawQuery, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::Utc;
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use serde::{Deserialize, Serialize};
use tokuly_core::{
    auth::ClientMeta,
    keys::{random_token, secure_eq},
    prelude::*,
};
use tokuly_proto::{
    common::Platform,
    identity::{ClientInfo, CreateWebHandoffRequest, CreateWebHandoffResponse},
};

use crate::{
    Identity,
    sessions::SessionOrigin,
    users,
};

/// How long a code works.
const CODE_TTL: Duration = Duration::from_secs(60);
/// The route that redeems codes, below the API's path prefix.
pub const HANDOFF_ROUTE: &str = "/auth/handoff";
/// Codes one account may make per ten minutes: one per web screen opened.
const CODES_PER_ACCOUNT: u32 = 60;
/// How often a redeem tries again after losing the app session's one slot
/// to a redeem at the same moment.
const LINK_ATTEMPTS: u32 = 3;
/// The request header the app's web view sends the verifier in.
pub const VERIFIER_HEADER: &str = "x-tokuly-handoff-verifier";
/// Redemptions one address may try per minute.
const REDEEMS_PER_ADDRESS: u32 = 60;
/// Where a code that does not work leads: the web's own sign-in.
const SIGN_IN_PATH: &str = "/login";
/// A readable cookie that tells the web app a session is waiting in its
/// HttpOnly cookie (a fresh web view has no hint of its own that it should
/// try to resume one). It carries nothing secret; the web removes it.
pub const SESSION_HINT_COOKIE: &str = "tokuly_session_hint";

/// The first path segments of the web app's screens: where a hand-off may
/// lead. Everything else (its sign-in and e-mail flows, OAuth consent, the
/// API documentation, anything unknown) is refused.
const WEB_ROUTES: &[&str] = &[
    "activity",
    "admin",
    "announcements",
    "ask",
    "bookmarks",
    "c",
    "chat",
    "circles",
    "clip",
    "communities",
    "developers",
    "emoji",
    "footprints",
    "friends",
    "g",
    "hangouts",
    "history",
    "live",
    "map",
    "me",
    "news",
    "notes",
    "notifications",
    "party",
    "places",
    "plans",
    "playlist",
    "quest",
    "recap",
    "school",
    "search",
    "settings",
    "shorts",
    "sound",
    "spaces",
    "stamps",
    "stories",
    "studio",
    "tag",
    "u",
    "wallet",
    "watch",
];

/// A code waiting to be redeemed, in Valkey under the keyed hash of the code.
#[derive(Serialize, Deserialize)]
struct PendingHandoff {
    /// The app's session the web session will belong to.
    session_id: Uuid,
    user_id: Uuid,
    /// Where the web opens, already checked.
    path: String,
    /// The keyed hash again, compared in constant time after the lookup.
    check: String,
    /// SHA-256 (hex) of the app's verifier.
    verifier_hash: String,
}

fn pending_key(hash: &str) -> String {
    format!("id:handoff:{hash}")
}

/// The path a hand-off may redirect to, normalised, or `None`. Only a path
/// of this origin's web app: no scheme, host or `//` (another origin), no
/// backslash (which browsers read as a slash), no control characters, and a
/// first segment of [`WEB_ROUTES`] once dot segments and percent-encoding
/// are resolved the way a browser resolves them.
pub fn web_path(input: &str) -> Option<String> {
    if input.is_empty() {
        return Some("/".to_owned());
    }
    if input.len() > 2048
        || !input.starts_with('/')
        || input.starts_with("//")
        || input.chars().any(|c| c == '\\' || c.is_control() || c.is_whitespace())
    {
        return None;
    }
    let decoded = percent_decoded(input)?;
    if decoded.starts_with("//") || decoded.chars().any(|c| c == '\\' || c.is_control()) {
        return None;
    }
    // Resolve it against a placeholder origin: whatever the browser would
    // make of it must stay on that origin.
    let base = url::Url::parse("https://handoff.invalid/").ok()?;
    let resolved = base.join(input).ok()?;
    if resolved.origin() != base.origin() || !resolved.username().is_empty() || resolved.password().is_some() {
        return None;
    }
    let path = resolved.path();
    // `/..//host` resolves to `//host`: another origin again.
    if path.starts_with("//") {
        return None;
    }
    let first = path.trim_start_matches('/').split('/').next().unwrap_or_default();
    if !first.is_empty() && !WEB_ROUTES.contains(&first) {
        return None;
    }
    let mut out = path.to_owned();
    if let Some(query) = resolved.query() {
        out.push('?');
        out.push_str(query);
    }
    if let Some(fragment) = resolved.fragment() {
        out.push('#');
        out.push_str(fragment);
    }
    Some(out)
}

/// `input` with its percent-escapes decoded (repeatedly, so a double
/// encoding cannot hide a `//` or a backslash), or `None` when it is not
/// text.
fn percent_decoded(input: &str) -> Option<String> {
    let mut current = input.to_owned();
    for _ in 0..4 {
        let next = url::form_urlencoded::parse(format!("x={}", current.replace('+', "%2B")).as_bytes())
            .next()
            .map(|(_, value)| value.into_owned())?;
        if next == current {
            return Some(current);
        }
        current = next;
    }
    Some(current)
}

/// A code as it may appear in a query: what [`random_token`] makes.
fn plausible_code(code: &str) -> bool {
    (16..=128).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// A verifier hash as `CreateWebHandoff` takes it: SHA-256, lowercase hex.
fn valid_verifier_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A verifier as RFC 7636 has it: 32 to 128 unreserved characters (ours are
/// longer than PKCE's 43 minimum need be: 256 random bits in base64url or hex).
fn plausible_verifier(verifier: &str) -> bool {
    (32..=128).contains(&verifier.len())
        && verifier.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

/// What a redemption came to.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Redeemed {
    /// Signed in: the cookie to set and where to go.
    SignedIn { cookie: String, path: String },
    /// No verifier, a wrong one, or the code is unknown, used, expired, or
    /// its session or account is gone.
    Refused,
}

impl Identity {
    pub(crate) async fn create_web_handoff_rpc(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateWebHandoffRequest>,
    ) -> ServiceResult<CreateWebHandoffResponse> {
        handle(async {
            let auth = ctx.auth()?;
            // The access table already keeps tokens away; a hand-off is a session's alone.
            ensure!(auth.api_token.is_none(), denied, "SESSION_REQUIRED", "only a signed-in app can open the web");
            ensure!(
                valid_verifier_hash(request.verifier_hash),
                invalid,
                "INVALID_VERIFIER",
                "verifier_hash must be the SHA-256 of a verifier, in lowercase hex"
            );
            let Some(path) = web_path(request.path) else {
                bail!(invalid, "INVALID_PATH", "not a path of the web app");
            };
            self.core
                .cache
                .rate_limit(&format!("handoff:{}", auth.user_id), CODES_PER_ACCOUNT, Duration::from_secs(600))
                .await?;
            let linked: Option<Option<Uuid>> = sqlx::query_scalar(
                "SELECT linked_to FROM sessions WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL AND expires_at > now()",
            )
            .bind(auth.session_id)
            .bind(auth.user_id)
            .fetch_optional(&self.core.db)
            .await?;
            let Some(linked) = linked else {
                bail!(unauthenticated, "SESSION_EXPIRED", "sign in again");
            };
            ensure!(linked.is_none(), precondition, "NOT_AVAILABLE", "a web session cannot open another one");

            let code = random_token(32);
            let hash = self.core.keys.token_hash(&code);
            let pending = PendingHandoff {
                session_id: auth.session_id,
                user_id: auth.user_id,
                path,
                check: hash.clone(),
                verifier_hash: request.verifier_hash.to_owned(),
            };
            self.core.cache.set_json(&pending_key(&hash), &pending, Some(CODE_TTL)).await?;
            let url = format!("{}{HANDOFF_ROUTE}?code={code}", self.core.cfg.api_path_prefix());
            let expires_at = Utc::now() + chrono::Duration::from_std(CODE_TTL).unwrap_or_default();
            ok(CreateWebHandoffResponse { url, expires_at: ts(expires_at), ..Default::default() })
        })
        .await
    }

    /// Redeems a code once, given the app's verifier: see the module documentation.
    pub(crate) async fn redeem_web_handoff(
        &self,
        code: &str,
        verifier: Option<&str>,
        meta: &ClientMeta,
    ) -> AppResult<Redeemed> {
        let core = &self.core;
        core.cache
            .rate_limit(&format!("handoff:ip:{}", meta.ip), REDEEMS_PER_ADDRESS, Duration::from_secs(60))
            .await?;
        // Not the app's web view (a link, a preview, a prefetch): nothing
        // happens, and the code stays for the app.
        let Some(verifier) = verifier.filter(|v| plausible_verifier(v)) else {
            return Ok(Redeemed::Refused);
        };
        if !plausible_code(code) {
            return Ok(Redeemed::Refused);
        }
        let hash = core.keys.token_hash(code);
        let Some(pending) = core.cache.take_json::<PendingHandoff>(&pending_key(&hash)).await? else {
            return Ok(Redeemed::Refused);
        };
        if !secure_eq(pending.check.as_bytes(), hash.as_bytes()) {
            return Ok(Redeemed::Refused);
        }
        // Taken by whoever knew the code; it works only for the one who also knows the verifier.
        let presented = tokuly_core::keys::sha256_hex(verifier.as_bytes());
        if !secure_eq(pending.verifier_hash.as_bytes(), presented.as_bytes()) {
            return Ok(Redeemed::Refused);
        }
        // Still the app's active account, and an account that may sign in.
        if !self.app_session_in_use(pending.session_id).await? {
            return Ok(Redeemed::Refused);
        }
        let user = match users::get(core, pending.user_id).await {
            Ok(user) if user.is_active() => user,
            _ => return Ok(Redeemed::Refused),
        };

        // One web session per app session (the unique index
        // `sessions_one_linked`): the one before goes. Two redeems at once
        // race for the slot; the one that loses ends the winner's and tries
        // again, so the later one stays, as one after the other would.
        let client = ClientInfo { platform: Platform::PLATFORM_WEB.into(), ..Default::default() };
        let origin = SessionOrigin { linked_to: Some(pending.session_id), ..Default::default() };
        let mut attempt = 0;
        let issued = loop {
            let previous: Vec<Uuid> = sqlx::query_scalar(
                "UPDATE sessions SET revoked_at = now() WHERE linked_to = $1 AND revoked_at IS NULL RETURNING id",
            )
            .bind(pending.session_id)
            .fetch_all(&core.db)
            .await?;
            self.forget_session_tokens(&previous).await?;
            match self.create_session_with(&user, Some(&client), meta, origin).await {
                Ok(issued) => break issued,
                Err(error) if error.is_already_exists() && attempt < LINK_ATTEMPTS => attempt += 1,
                Err(error) if error.is_already_exists() => return Ok(Redeemed::Refused),
                Err(error) => return Err(error),
            }
        };
        // The app's session may have ended while this one was made, too late
        // for the trigger to see it: then this one ends as well.
        if !self.app_session_in_use(pending.session_id).await? {
            self.revoke_session(user.id, issued.session_id).await?;
            return Ok(Redeemed::Refused);
        }
        self.log_security_event(
            user.id,
            "WEB_SIGN_IN_FROM_APP",
            meta,
            "",
            serde_json::json!({ "session_id": issued.session_id, "app_session_id": pending.session_id }),
        )
        .await;
        Ok(Redeemed::SignedIn { cookie: self.refresh_cookie(&issued.refresh_token), path: pending.path })
    }

    /// The plain HTTP route of the hand-off, for the API's router (behind
    /// the auth middleware, which works out the client's address).
    pub fn http_routes(&self) -> Router {
        Router::new().route(HANDOFF_ROUTE, get(redeem)).with_state(self.clone())
    }
}

/// `GET /auth/handoff?code=…` (and the verifier in its header): a redirect,
/// with the cookie when it worked.
async fn redeem(
    State(identity): State<Identity>,
    client: Option<Extension<ClientMeta>>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    let meta = client.map(|Extension(meta)| meta).unwrap_or_default();
    let code = url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default();
    let verifier = headers.get(VERIFIER_HEADER).and_then(|v| v.to_str().ok());
    match identity.redeem_web_handoff(&code, verifier, &meta).await {
        Ok(Redeemed::SignedIn { cookie, path }) => {
            let secure = if identity.core.cfg.public_url.starts_with("https://") { "; Secure" } else { "" };
            let hint = format!("{SESSION_HINT_COOKIE}=1; Path=/; Max-Age=120; SameSite=Lax{secure}");
            redirect(&path, &[&cookie, &hint])
        }
        Ok(Redeemed::Refused) => redirect(SIGN_IN_PATH, &[]),
        Err(error) if error.reason == "RATE_LIMITED" => {
            no_store((StatusCode::TOO_MANY_REQUESTS, "too many requests").into_response())
        }
        Err(error) => {
            tracing::warn!(error = %error, "web hand-off failed");
            redirect(SIGN_IN_PATH, &[])
        }
    }
}

/// A `303 See Other` to a path of this origin (checked by [`web_path`]).
fn redirect(path: &str, cookies: &[&str]) -> Response {
    let mut response = StatusCode::SEE_OTHER.into_response();
    let headers = response.headers_mut();
    if let Ok(location) = HeaderValue::from_str(path) {
        headers.insert(header::LOCATION, location);
    }
    for cookie in cookies.iter().filter_map(|c| HeaderValue::from_str(c).ok()) {
        headers.append(header::SET_COOKIE, cookie);
    }
    // The code is in this URL: it is not kept, and not passed on.
    headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    no_store(response)
}

fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_paths_stay_on_the_web_app() {
        assert_eq!(web_path("").as_deref(), Some("/"));
        assert_eq!(web_path("/").as_deref(), Some("/"));
        assert_eq!(web_path("/settings").as_deref(), Some("/settings"));
        assert_eq!(web_path("/notes?tab=home#top").as_deref(), Some("/notes?tab=home#top"));
        assert_eq!(web_path("/settings/../notes").as_deref(), Some("/notes"));
        for evil in [
            "//evil.example",
            "//evil.example/settings",
            "https://evil.example/settings",
            "http:/evil.example",
            "javascript:alert(1)",
            "settings",
            "/\\evil.example",
            "\\\\evil.example",
            "/%2fevil.example",
            "/%2F%2Fevil.example",
            "/%5cevil.example",
            "/%5Cevil.example",
            "/%252fevil.example",
            "/%09/evil.example",
            "/\t/evil.example",
            "/ /evil.example",
            "/settings\r\nSet-Cookie: x=1",
            "/login",
            "/oauth/authorize",
            "/docs/api",
            "/..//evil.example",
            "/unknown",
            "/%2e%2e/login",
        ] {
            assert_eq!(web_path(evil), None, "{evil}");
        }
    }

    #[test]
    fn codes_look_like_tokens() {
        assert!(plausible_code(&random_token(32)));
        assert!(!plausible_code(""));
        assert!(!plausible_code("short"));
        assert!(!plausible_code(&format!("{}%0d", random_token(32))));
    }
}
