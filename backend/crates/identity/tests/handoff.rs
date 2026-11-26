//! Opening the web signed in from an app: `CreateWebHandoff` and
//! `GET /auth/handoff`.
mod common;

use std::time::Duration;

use common::{Browser, Ext, start};
use serde_json::json;
use tokuly_identity::IdentityConfig;
use tokuly_proto::identity::*;
use tokuly_testkit::TestUser;

const CREATE: &str = "tokuly.identity.v1.AuthService/CreateWebHandoff";
const VERIFIER_HEADER: &str = "x-tokuly-handoff-verifier";

/// What the app holds after asking for a code: the route its web view loads
/// (below the API's prefix, which the edge strips) and the verifier only it knows.
struct Handoff {
    route: String,
    verifier: String,
}

impl std::ops::Deref for Handoff {
    type Target = str;

    fn deref(&self) -> &str {
        &self.route
    }
}

/// A request for `path` bound to a fresh verifier.
fn request(path: &str) -> (CreateWebHandoffRequest, String) {
    let verifier = tokuly_core::keys::random_token(32);
    let verifier_hash = tokuly_core::keys::sha256_hex(verifier.as_bytes());
    (CreateWebHandoffRequest { path: path.into(), verifier_hash, ..Default::default() }, verifier)
}

/// A code for `path`, as the app of `user`.
async fn handoff(ext: &Ext, user: &TestUser, path: &str) -> Handoff {
    let (request, verifier) = request(path);
    let made: CreateWebHandoffResponse = ext.app.call(CREATE, Some(user), &request).await.unwrap();
    let prefix = ext.app.core.cfg.api_path_prefix();
    let route = made.url.strip_prefix(prefix).expect("the URL is below the API prefix").to_owned();
    Handoff { route, verifier }
}

/// The app's web view loading a hand-off: its first load carries the verifier.
trait WebView {
    async fn open(&mut self, handoff: &Handoff) -> (u16, Option<String>, Vec<String>);
}

impl WebView for Browser {
    async fn open(&mut self, handoff: &Handoff) -> (u16, Option<String>, Vec<String>) {
        self.get_with(&handoff.route, &[(VERIFIER_HEADER, &handoff.verifier)]).await
    }
}

fn code_of(route: &str) -> &str {
    route.split_once("code=").unwrap().1
}

/// The web sessions linked to an app's session that are still live.
async fn live_web_sessions(ext: &Ext, app_session: uuid::Uuid) -> Vec<uuid::Uuid> {
    sqlx::query_scalar("SELECT id FROM sessions WHERE linked_to = $1 AND revoked_at IS NULL")
        .bind(app_session)
        .fetch_all(&ext.app.core.db)
        .await
        .unwrap()
}

/// Whether the browser's refresh cookie still gets it an access token; the token.
async fn web_refresh(browser: &mut Browser) -> Result<String, String> {
    let answer = browser.call("tokuly.identity.v1.AuthService/RefreshToken", None, json!({})).await;
    if answer.status == 200 { Ok(answer.body["accessToken"].as_str().unwrap().to_owned()) } else { Err(answer.reason) }
}

async fn get_me_status(browser: &mut Browser, token: &str) -> (u16, String) {
    let answer = browser.call("tokuly.identity.v1.AccountService/GetMe", Some(token), json!({})).await;
    (answer.status, answer.reason)
}

#[tokio::test]
async fn a_code_signs_the_web_view_in_once() {
    let ext = start(IdentityConfig::default()).await;
    let app_user = ext.app.user("handoff").await;
    let route = handoff(&ext, &app_user, "/settings").await;
    assert!(route.starts_with("/auth/handoff?code="), "{}", route.route);
    // 256 random bits, base64url.
    assert!(code_of(&route).len() >= 43);

    let mut web_view = Browser::new(&ext);
    let (status, location, cookies) = web_view.open(&route).await;
    assert_eq!(status, 303);
    assert_eq!(location.as_deref(), Some("/settings"));
    let cookie = cookies.iter().find(|c| c.starts_with("tokuly_rt=")).expect("the refresh cookie");
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax"), "{cookie}");
    assert!(!cookie.contains("Secure"), "plain-HTTP test server: {cookie}");
    // The web app's hint that a session waits (readable, nothing secret in it).
    assert!(cookies.iter().any(|c| c.starts_with("tokuly_session_hint=1;") && !c.contains("HttpOnly")));
    // The session's token never shows in a URL.
    let refresh = web_view.jar["tokuly_rt"].clone();
    assert!(!location.unwrap().contains(&refresh));

    // A web session of the same account, linked to the app's session.
    let token = web_refresh(&mut web_view).await.expect("the web session works");
    let me = web_view.call("tokuly.identity.v1.AccountService/GetMe", Some(&token), json!({})).await;
    assert_eq!(me.body["me"]["user"]["id"], json!(app_user.id.to_string()));
    let web_sessions = live_web_sessions(&ext, app_user.session_id).await;
    assert_eq!(web_sessions.len(), 1);
    assert_ne!(web_sessions[0], app_user.session_id);
    let platform: i32 = sqlx::query_scalar("SELECT platform FROM sessions WHERE id = $1")
        .bind(web_sessions[0])
        .fetch_one(&ext.app.core.db)
        .await
        .unwrap();
    assert_eq!(platform, tokuly_proto::common::Platform::PLATFORM_WEB as i32);
    let logged: Vec<String> = sqlx::query_scalar("SELECT type FROM security_events WHERE user_id = $1")
        .bind(app_user.id)
        .fetch_all(&ext.app.core.db)
        .await
        .unwrap();
    assert!(logged.contains(&"WEB_SIGN_IN_FROM_APP".to_owned()), "{logged:?}");

    // Used: a second load signs nobody in.
    let mut again = Browser::new(&ext);
    let (status, location, cookies) = again.open(&route).await;
    assert_eq!((status, location.as_deref()), (303, Some("/login")));
    assert!(cookies.is_empty(), "{cookies:?}");
}

#[tokio::test]
async fn unknown_expired_and_malformed_codes_sign_nobody_in() {
    let ext = start(IdentityConfig::default()).await;
    let app_user = ext.app.user("expired").await;
    let mut web_view = Browser::new(&ext);
    for route in [
        "/auth/handoff".to_owned(),
        "/auth/handoff?code=".to_owned(),
        format!("/auth/handoff?code={}", tokuly_core::keys::random_token(32)),
        "/auth/handoff?code=%0d%0aSet-Cookie:%20tokuly_rt=x".to_owned(),
    ] {
        let (status, location, cookies) = web_view.get(&route).await;
        assert_eq!((status, location.as_deref()), (303, Some("/login")), "{route}");
        assert!(cookies.is_empty(), "{route}: {cookies:?}");
    }

    // A code lives for 60 seconds.
    let route = handoff(&ext, &app_user, "/notes").await;
    let key = format!("id:handoff:{}", ext.app.core.keys.token_hash(code_of(&route)));
    let ttl: i64 = redis::cmd("TTL").arg(&key).query_async(&mut ext.app.core.cache.conn()).await.unwrap();
    assert!((1..=60).contains(&ttl), "{ttl}");
    // ...and then it is gone.
    let _: () = redis::cmd("PEXPIRE").arg(&key).arg(1).query_async(&mut ext.app.core.cache.conn()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let (status, location, cookies) = web_view.open(&route).await;
    assert_eq!((status, location.as_deref()), (303, Some("/login")));
    assert!(cookies.is_empty());
    assert!(live_web_sessions(&ext, app_user.session_id).await.is_empty());
}

#[tokio::test]
async fn two_loads_of_one_code_at_once_sign_in_one() {
    let ext = start(IdentityConfig::default()).await;
    let app_user = ext.app.user("race").await;
    let route = handoff(&ext, &app_user, "/").await;
    let (mut first, mut second) = (Browser::new(&ext), Browser::new(&ext));
    let (a, b) = tokio::join!(first.open(&route), second.open(&route));
    let signed_in = [&a, &b].iter().filter(|(_, _, cookies)| !cookies.is_empty()).count();
    assert_eq!(signed_in, 1, "{a:?} {b:?}");
    let locations: Vec<_> = [a.1, b.1].into_iter().flatten().collect();
    assert!(locations.contains(&"/".to_owned()) && locations.contains(&"/login".to_owned()), "{locations:?}");
}

#[tokio::test]
async fn only_paths_of_the_web_app_are_accepted() {
    let ext = start(IdentityConfig::default()).await;
    let app_user = ext.app.user("redirect").await;
    for evil in [
        "//evil.example",
        "https://evil.example",
        "http:evil.example",
        "/\\evil.example",
        "/%2F%2Fevil.example",
        "/%5Cevil.example",
        "/%252F%252Fevil.example",
        "/..//evil.example",
        "evil.example",
        "/login?redirect=//evil.example",
        "/oauth/authorize",
        "/not-a-screen",
    ] {
        let err = ext
            .app
            .call::<_, CreateWebHandoffResponse>(
                CREATE,
                Some(&app_user),
                &request(evil).0,
            )
            .await
            .unwrap_err();
        assert_eq!(err.reason, "INVALID_PATH", "{evil}");
    }
    // Normalised as a browser would: the redirect is what was checked.
    let route = handoff(&ext, &app_user, "/settings/../notes?tab=home").await;
    let (_, location, _) = Browser::new(&ext).open(&route).await;
    assert_eq!(location.as_deref(), Some("/notes?tab=home"));

    let err = ext
        .app
        .call::<_, CreateWebHandoffResponse>(CREATE, None, &CreateWebHandoffRequest::default())
        .await
        .unwrap_err();
    assert_eq!(err.code, "unauthenticated");
}

#[tokio::test]
async fn ending_the_app_session_ends_the_web_one() {
    let ext = start(IdentityConfig::default()).await;
    let app_user = ext.app.user("revoke").await;
    let mut web_view = Browser::new(&ext);
    web_view.open(&handoff(&ext, &app_user, "/settings").await).await;
    let web_token = web_refresh(&mut web_view).await.unwrap();
    assert_eq!(get_me_status(&mut web_view, &web_token).await.0, 200);

    // The app signs out: the web session's refresh cookie and its access token stop at once.
    let _: SignOutResponse =
        ext.app.call("tokuly.identity.v1.AuthService/SignOut", Some(&app_user), &SignOutRequest::default()).await.unwrap();
    assert_eq!(web_refresh(&mut web_view).await.unwrap_err(), "SESSION_EXPIRED");
    assert_eq!(get_me_status(&mut web_view, &web_token).await, (401, "SESSION_REVOKED".to_owned()));
    assert!(live_web_sessions(&ext, app_user.session_id).await.is_empty());

    // "Sign out everywhere" from another device ends it too.
    let other_app = ext.app.sign_in(app_user.id).await;
    let mut web_view = Browser::new(&ext);
    web_view.open(&handoff(&ext, &other_app, "/notes").await).await;
    let web_token = web_refresh(&mut web_view).await.unwrap();
    let elsewhere = ext.app.sign_in(app_user.id).await;
    let _: SignOutResponse = ext
        .app
        .call(
            "tokuly.identity.v1.AuthService/SignOut",
            Some(&elsewhere),
            &SignOutRequest { all_sessions: true, ..Default::default() },
        )
        .await
        .unwrap();
    assert!(web_refresh(&mut web_view).await.is_err());
    assert_eq!(get_me_status(&mut web_view, &web_token).await.0, 401);

    // A ban ends it (the identity.user.banned consumer).
    let banned_app = ext.app.sign_in(app_user.id).await;
    let mut web_view = Browser::new(&ext);
    web_view.open(&handoff(&ext, &banned_app, "/").await).await;
    let web_token = web_refresh(&mut web_view).await.unwrap();
    let banned_at: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("UPDATE users SET banned_at = now() WHERE id = $1 RETURNING banned_at")
            .bind(app_user.id)
            .fetch_one(&ext.app.core.db)
            .await
            .unwrap();
    ext.identity.sign_out_banned(app_user.id, banned_at).await.unwrap();
    assert!(web_refresh(&mut web_view).await.is_err());
    assert_eq!(get_me_status(&mut web_view, &web_token).await.0, 401);
}

#[tokio::test]
async fn the_web_session_lives_while_its_app_account_is_the_active_one() {
    let ext = start(IdentityConfig::default()).await;
    let app_user = ext.app.user("parked").await;
    let mut web_view = Browser::new(&ext);
    web_view.open(&handoff(&ext, &app_user, "/").await).await;
    web_refresh(&mut web_view).await.unwrap();

    // The app switched to another account: this one is parked there.
    sqlx::query("UPDATE sessions SET parked_at = now() WHERE id = $1")
        .bind(app_user.session_id)
        .execute(&ext.app.core.db)
        .await
        .unwrap();
    assert_eq!(web_refresh(&mut web_view).await.unwrap_err(), "SESSION_EXPIRED");

    // A code made just before the switch is no good after it either.
    sqlx::query("UPDATE sessions SET parked_at = NULL WHERE id = $1")
        .bind(app_user.session_id)
        .execute(&ext.app.core.db)
        .await
        .unwrap();
    let route = handoff(&ext, &app_user, "/settings").await;
    sqlx::query("UPDATE sessions SET parked_at = now() WHERE id = $1")
        .bind(app_user.session_id)
        .execute(&ext.app.core.db)
        .await
        .unwrap();
    let (_, location, cookies) = Browser::new(&ext).open(&route).await;
    assert_eq!(location.as_deref(), Some("/login"));
    assert!(cookies.is_empty());
}

#[tokio::test]
async fn a_newer_hand_off_replaces_the_web_session_before() {
    let ext = start(IdentityConfig::default()).await;
    let app_user = ext.app.user("replace").await;
    let mut first = Browser::new(&ext);
    first.open(&handoff(&ext, &app_user, "/").await).await;
    let first_token = web_refresh(&mut first).await.unwrap();
    let mut second = Browser::new(&ext);
    second.open(&handoff(&ext, &app_user, "/").await).await;
    web_refresh(&mut second).await.unwrap();
    assert!(web_refresh(&mut first).await.is_err());
    assert_eq!(get_me_status(&mut first, &first_token).await.0, 401);
    assert_eq!(live_web_sessions(&ext, app_user.session_id).await.len(), 1);
}

#[tokio::test]
async fn changing_the_password_in_the_web_view_keeps_the_app_signed_in() {
    let ext = start(IdentityConfig::default()).await;
    let app_user = ext.app.user("password").await;
    let other_device = ext.app.sign_in(app_user.id).await;
    let mut web_view = Browser::new(&ext);
    web_view.open(&handoff(&ext, &app_user, "/settings").await).await;
    let web_token = web_refresh(&mut web_view).await.unwrap();
    let web_session: uuid::Uuid = live_web_sessions(&ext, app_user.session_id).await[0];

    // What a password change does: every session but the one it was made in.
    ext.identity.revoke_all_sessions(app_user.id, Some(web_session)).await.unwrap();
    assert_eq!(get_me_status(&mut web_view, &web_token).await.0, 200);
    let me: Result<GetMeResponse, _> =
        ext.app.call("tokuly.identity.v1.AccountService/GetMe", Some(&app_user), &GetMeRequest::default()).await;
    assert!(me.is_ok(), "the app around the web view stays signed in");
    let other: Result<GetMeResponse, _> =
        ext.app.call("tokuly.identity.v1.AccountService/GetMe", Some(&other_device), &GetMeRequest::default()).await;
    assert_eq!(other.unwrap_err().reason, "SESSION_REVOKED");
}

#[tokio::test]
async fn suspended_and_deleted_accounts_cannot_redeem() {
    let ext = start(IdentityConfig::default()).await;
    for column in ["banned_at", "deleted_at"] {
        let app_user = ext.app.user("gone").await;
        let route = handoff(&ext, &app_user, "/").await;
        let sql = format!("UPDATE users SET {column} = now() WHERE id = $1");
        sqlx::query(sqlx::AssertSqlSafe(sql)).bind(app_user.id).execute(&ext.app.core.db).await.unwrap();
        let (_, location, cookies) = Browser::new(&ext).open(&route).await;
        assert_eq!(location.as_deref(), Some("/login"), "{column}");
        assert!(cookies.is_empty(), "{column}");
        assert!(live_web_sessions(&ext, app_user.session_id).await.is_empty(), "{column}");
    }
}

#[tokio::test]
async fn a_web_session_cannot_open_another() {
    let ext = start(IdentityConfig::default()).await;
    let app_user = ext.app.user("chain").await;
    let mut web_view = Browser::new(&ext);
    web_view.open(&handoff(&ext, &app_user, "/").await).await;
    let web_token = web_refresh(&mut web_view).await.unwrap();
    let chained = web_view.call(CREATE, Some(&web_token), json!({ "path": "/", "verifierHash": "a".repeat(64) })).await;
    assert_eq!(chained.reason, "NOT_AVAILABLE");
}

/// Login CSRF: someone sends a victim a link with a code of their own (or a
/// code leaked from an app). Without the app's verifier it signs nobody in,
/// so the victim's browser is never signed in as the sender's account (whose
/// map would then get the victim's location).
#[tokio::test]
async fn a_code_without_the_apps_verifier_signs_nobody_in() {
    let ext = start(IdentityConfig::default()).await;
    let attacker = ext.app.user("attacker").await;
    let leaked = handoff(&ext, &attacker, "/map").await;

    // A plain browser following the link, signed out or signed in as itself.
    let victim_user = ext.app.user("victim").await;
    let mut victim = Browser::new(&ext);
    victim.open(&handoff(&ext, &victim_user, "/").await).await;
    let victim_cookie = victim.jar["tokuly_rt"].clone();
    for browser in [&mut Browser::new(&ext), &mut victim] {
        let (status, location, cookies) = browser.get(&leaked).await;
        assert_eq!((status, location.as_deref()), (303, Some("/login")));
        assert!(cookies.is_empty(), "{cookies:?}");
    }
    assert_eq!(victim.jar["tokuly_rt"], victim_cookie);
    assert_eq!(web_refresh(&mut victim).await.map(|_| ()), Ok(()));
    let me = victim.call("tokuly.identity.v1.AuthService/RefreshToken", None, json!({})).await;
    assert_eq!(me.body["me"]["user"]["id"], json!(victim_user.id.to_string()));
    assert!(live_web_sessions(&ext, attacker.session_id).await.is_empty());

    // A wrong verifier: refused, and the code is spent.
    let mut guesser = Browser::new(&ext);
    let (_, location, cookies) =
        guesser.get_with(&leaked, &[(VERIFIER_HEADER, &tokuly_core::keys::random_token(32))]).await;
    assert_eq!(location.as_deref(), Some("/login"));
    assert!(cookies.is_empty());
    let (_, location, _) = Browser::new(&ext).open(&leaked).await;
    assert_eq!(location.as_deref(), Some("/login"));
    assert!(live_web_sessions(&ext, attacker.session_id).await.is_empty());
}

#[tokio::test]
async fn a_link_without_the_verifier_leaves_the_code_to_the_app() {
    let ext = start(IdentityConfig::default()).await;
    let app_user = ext.app.user("prefetch").await;
    let made = handoff(&ext, &app_user, "/settings").await;
    // Something loads the URL without the header first (a link preview, a prefetch).
    Browser::new(&ext).get(&made).await;
    let (_, location, cookies) = Browser::new(&ext).open(&made).await;
    assert_eq!(location.as_deref(), Some("/settings"));
    assert!(cookies.iter().any(|c| c.starts_with("tokuly_rt=")));
}

#[tokio::test]
async fn the_verifier_hash_must_be_a_sha256() {
    let ext = start(IdentityConfig::default()).await;
    let app_user = ext.app.user("verifier").await;
    for hash in ["", "abc", &"g".repeat(64), &"A".repeat(64)] {
        let err = ext
            .app
            .call::<_, CreateWebHandoffResponse>(
                CREATE,
                Some(&app_user),
                &CreateWebHandoffRequest { path: "/".into(), verifier_hash: hash.into(), ..Default::default() },
            )
            .await
            .unwrap_err();
        assert_eq!(err.reason, "INVALID_VERIFIER", "{hash}");
    }
}
