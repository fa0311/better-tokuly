//! Several accounts on one device and switching between them, as a browser
//! (cookies) and as a native app (tokens in the body).
mod common;

use common::{Browser, Ext, start, unique};
use serde_json::json;
use tokuly_identity::IdentityConfig;
use tokuly_proto::identity::*;
use tokuly_testkit::TestUser;

const PASSWORD: &str = "correct horse battery";

fn sign_up_json(tag: &str, add_account: bool) -> serde_json::Value {
    json!({
        "email": format!("{tag}@example.com"), "password": PASSWORD, "handle": tag, "displayName": "テスト",
        "birthdate": "2000-04-02", "acceptTerms": true, "addAccount": add_account,
    })
}

fn sign_up_request(tag: &str) -> SignUpRequest {
    SignUpRequest {
        email: format!("{tag}@example.com"),
        password: PASSWORD.into(),
        handle: tag.to_owned(),
        display_name: "テスト".into(),
        birthdate: "2000-04-02".into(),
        accept_terms: true,
        ..Default::default()
    }
}

async fn security_log(ext: &Ext, user_id: &str) -> Vec<String> {
    sqlx::query_scalar("SELECT type FROM security_events WHERE user_id = $1::uuid ORDER BY id")
        .bind(user_id)
        .fetch_all(&ext.app.core.db)
        .await
        .unwrap()
}

fn user_id(answer: &serde_json::Value) -> String {
    answer["me"]["user"]["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn a_browser_holds_several_accounts_and_switches_between_them() {
    let ext = start(IdentityConfig::default()).await;
    let mut browser = Browser::new(&ext);
    let (a, b) = (unique("main"), unique("ura"));

    let first = browser.call("tokuly.identity.v1.AuthService/SignUp", None, sign_up_json(&a, false)).await;
    assert_eq!(first.status, 200, "{}", first.reason);
    let a_id = user_id(&first.body);
    let a_token = first.body["accessToken"].as_str().unwrap().to_owned();
    let a_refresh = browser.jar["tokuly_rt"].clone();

    // A second account, added beside the first: the first is parked.
    let second = browser.call("tokuly.identity.v1.AuthService/SignUp", Some(&a_token), sign_up_json(&b, true)).await;
    assert_eq!(second.status, 200, "{}", second.reason);
    let b_id = user_id(&second.body);
    assert_eq!(second.cookie("tokuly_ra").as_deref(), Some(a_refresh.as_str()));
    assert_ne!(browser.jar["tokuly_rt"], a_refresh);
    let b_token = second.body["accessToken"].as_str().unwrap().to_owned();

    // The parked token is not good for a plain refresh.
    let mut thief = Browser::new(&ext);
    thief.jar.insert("tokuly_rt".into(), a_refresh.clone());
    let refused = thief.call("tokuly.identity.v1.AuthService/RefreshToken", None, json!({})).await;
    assert_eq!(refused.reason, "SWITCH_REQUIRED");

    let listed = browser.call("tokuly.identity.v1.AuthService/ListDeviceAccounts", None, json!({})).await;
    let accounts = listed.body["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 2);
    assert_eq!(accounts[0]["user"]["id"], json!(b_id));
    assert_eq!(accounts[0]["active"], json!(true));
    assert_eq!(accounts[1]["user"]["id"], json!(a_id));
    assert_eq!(listed.body["limit"], json!(5));
    let a_session = accounts[1]["sessionId"].as_str().unwrap().to_owned();

    // Switch back to A, no password asked.
    let switched = browser
        .call("tokuly.identity.v1.AuthService/SwitchAccount", Some(&b_token), json!({ "sessionId": a_session }))
        .await;
    assert_eq!(switched.status, 200, "{}", switched.reason);
    assert_eq!(user_id(&switched.body), a_id);
    assert_eq!(switched.body["sessionId"], json!(a_session), "the same session, rotated");
    assert_ne!(browser.jar["tokuly_rt"], a_refresh);
    let refreshed = browser.call("tokuly.identity.v1.AuthService/RefreshToken", None, json!({})).await;
    assert_eq!(user_id(&refreshed.body), a_id);
    assert!(security_log(&ext, &a_id).await.contains(&"ACCOUNT_SWITCHED".to_owned()));
    let a_access = refreshed.body["accessToken"].as_str().unwrap().to_owned();

    // A's device list shows the switch; nothing of A names B.
    let a_user = TestUser {
        id: a_id.parse().unwrap(),
        handle: a.clone(),
        token: a_access.clone(),
        session_id: a_session.parse().unwrap(),
    };
    let sessions: ListSessionsResponse = ext
        .app
        .call("tokuly.identity.v1.AccountService/ListSessions", Some(&a_user), &ListSessionsRequest::default())
        .await
        .unwrap();
    assert_eq!(sessions.sessions.len(), 1);
    assert!(sessions.sessions[0].last_switched_at.as_option().is_some() && !sessions.sessions[0].parked);
    let a_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM security_events WHERE user_id = $1::uuid AND (attrs::text LIKE '%' || $2 || '%' OR attrs::text LIKE '%' || $3 || '%')",
    )
    .bind(&a_id)
    .bind(&b_id)
    .bind(&b)
    .fetch_one(&ext.app.core.db)
    .await
    .unwrap();
    assert_eq!(a_rows, 0, "A's security log does not name B");
    let b_sessions: Vec<bool> =
        sqlx::query_scalar("SELECT added_by_switch FROM sessions WHERE user_id = $1::uuid AND revoked_at IS NULL")
            .bind(&b_id)
            .fetch_all(&ext.app.core.db)
            .await
            .unwrap();
    assert_eq!(b_sessions, [true], "B shows it was added beside another account");

    // Signing A out leaves B on the device.
    let _ = browser.call("tokuly.identity.v1.AuthService/SignOut", Some(&a_access), json!({})).await;
    let listed = browser.call("tokuly.identity.v1.AuthService/ListDeviceAccounts", None, json!({})).await;
    let accounts = listed.body["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0]["user"]["id"], json!(b_id));
    assert!(!accounts[0]["active"].as_bool().unwrap_or(false));
    let b_session = accounts[0]["sessionId"].as_str().unwrap().to_owned();
    let switched =
        browser.call("tokuly.identity.v1.AuthService/SwitchAccount", None, json!({ "sessionId": b_session })).await;
    assert_eq!(user_id(&switched.body), b_id);

    // Removing a parked account signs it out of this device only.
    let c = unique("creator");
    let third = browser.call("tokuly.identity.v1.AuthService/SignUp", None, sign_up_json(&c, true)).await;
    let c_id = user_id(&third.body);
    let listed = browser.call("tokuly.identity.v1.AuthService/ListDeviceAccounts", None, json!({})).await;
    let parked_b = listed.body["accounts"].as_array().unwrap().iter().find(|a| a["user"]["id"] == json!(b_id)).unwrap()
        ["sessionId"]
        .clone();
    let removed = browser
        .call("tokuly.identity.v1.AuthService/RemoveDeviceAccount", None, json!({ "sessionId": parked_b }))
        .await;
    assert_eq!(removed.status, 200, "{}", removed.reason);
    let listed = browser.call("tokuly.identity.v1.AuthService/ListDeviceAccounts", None, json!({})).await;
    let ids: Vec<&str> =
        listed.body["accounts"].as_array().unwrap().iter().map(|a| a["user"]["id"].as_str().unwrap()).collect();
    assert_eq!(ids, [c_id.as_str()]);
    let again =
        browser.call("tokuly.identity.v1.AuthService/SwitchAccount", None, json!({ "sessionId": parked_b })).await;
    assert_eq!(again.reason, "ACCOUNT_NOT_ON_DEVICE");
}

#[tokio::test]
async fn a_device_holds_at_most_the_limit() {
    let ext = start(IdentityConfig { device_account_limit: 2, ..Default::default() }).await;
    let mut browser = Browser::new(&ext);
    assert_eq!(
        browser.call("tokuly.identity.v1.AuthService/SignUp", None, sign_up_json(&unique("one"), true)).await.status,
        200
    );
    assert_eq!(
        browser.call("tokuly.identity.v1.AuthService/SignUp", None, sign_up_json(&unique("two"), true)).await.status,
        200
    );
    let third = unique("three");
    let _: AuthResponse =
        ext.app.call("tokuly.identity.v1.AuthService/SignUp", None, &sign_up_request(&third)).await.unwrap();
    let refused = browser
        .call(
            "tokuly.identity.v1.AuthService/SignIn",
            None,
            json!({ "identifier": third, "password": PASSWORD, "addAccount": true }),
        )
        .await;
    assert_eq!(refused.reason, "TOO_MANY_ACCOUNTS");
    // Signing in to an account already on the device replaces its session instead.
    let listed = browser.call("tokuly.identity.v1.AuthService/ListDeviceAccounts", None, json!({})).await;
    let handle = listed.body["accounts"][1]["user"]["handle"].as_str().unwrap().to_owned();
    let again = browser
        .call(
            "tokuly.identity.v1.AuthService/SignIn",
            None,
            json!({ "identifier": handle, "password": PASSWORD, "addAccount": true }),
        )
        .await;
    assert_eq!(again.status, 200, "{}", again.reason);
    let listed = browser.call("tokuly.identity.v1.AuthService/ListDeviceAccounts", None, json!({})).await;
    assert_eq!(listed.body["accounts"].as_array().unwrap().len(), 2);
}

/// TOTP code for a base32 secret, `steps` time steps from now.
fn totp_at(secret_base32: &str, steps: i64) -> String {
    use totp_rs::{Builder, Secret};
    let now_secs = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    if now_secs() % 30 >= 27 {
        std::thread::sleep(std::time::Duration::from_secs(4));
    }
    Builder::new()
        .with_secret(Secret::try_from_base32(secret_base32).unwrap())
        .with_account_name("test".to_owned())
        .with_issuer(Some("Tokuly"))
        .build()
        .unwrap()
        .generate((now_secs() + steps * 30) as u64)
        .to_string()
}

#[tokio::test]
async fn switching_to_an_account_with_two_factor_asks_for_it_when_stale() {
    let ext = start(IdentityConfig::default()).await;
    // A native app: it holds the refresh tokens itself.
    let main = unique("native");
    let first: AuthResponse =
        ext.app.call("tokuly.identity.v1.AuthService/SignUp", None, &sign_up_request(&main)).await.unwrap();
    let main_user = TestUser {
        id: first.me.as_option().unwrap().user.as_option().unwrap().id.parse().unwrap(),
        handle: main.clone(),
        token: first.access_token.clone(),
        session_id: first.session_id.parse().unwrap(),
    };
    // The second account turns TOTP on (on its own device first).
    let secure = unique("secure");
    let _: AuthResponse =
        ext.app.call("tokuly.identity.v1.AuthService/SignUp", None, &sign_up_request(&secure)).await.unwrap();
    let secure_id: uuid::Uuid = sqlx::query_scalar("SELECT id FROM users WHERE handle = $1::citext")
        .bind(&secure)
        .fetch_one(&ext.app.core.db)
        .await
        .unwrap();
    let elsewhere = ext.app.sign_in(secure_id).await;
    let setup: BeginTotpSetupResponse = ext
        .app
        .call("tokuly.identity.v1.AccountService/BeginTotpSetup", Some(&elsewhere), &BeginTotpSetupRequest::default())
        .await
        .unwrap();
    let _: EnableTotpResponse = ext
        .app
        .call(
            "tokuly.identity.v1.AccountService/EnableTotp",
            Some(&elsewhere),
            &EnableTotpRequest {
                code: totp_at(&setup.secret, -1),
                current_password: PASSWORD.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    // Added on the app with its password and code: the session passed the second factor.
    let added: AuthResponse = ext
        .app
        .call(
            "tokuly.identity.v1.AuthService/SignIn",
            Some(&main_user),
            &SignInRequest {
                identifier: secure.clone(),
                password: PASSWORD.into(),
                totp_code: totp_at(&setup.secret, 0),
                add_account: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let secure_user = TestUser {
        id: secure_id,
        handle: secure.clone(),
        token: added.access_token.clone(),
        session_id: added.session_id.parse().unwrap(),
    };

    // The main account is parked; the app lists both from its tokens.
    let err = ext
        .app
        .call::<_, AuthResponse>(
            "tokuly.identity.v1.AuthService/RefreshToken",
            None,
            &RefreshTokenRequest { refresh_token: first.refresh_token.clone(), ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!(err.reason, "SWITCH_REQUIRED");
    let listed: ListDeviceAccountsResponse = ext
        .app
        .call(
            "tokuly.identity.v1.AuthService/ListDeviceAccounts",
            Some(&secure_user),
            &ListDeviceAccountsRequest { refresh_tokens: vec![first.refresh_token.clone()], ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(listed.accounts.len(), 2);
    assert!(listed.accounts[0].active && listed.accounts[0].user.as_option().unwrap().handle == secure);

    let switch = |token: &str, code: &str, auth: &TestUser| {
        let request =
            SwitchAccountRequest { refresh_token: token.to_owned(), totp_code: code.to_owned(), ..Default::default() };
        let (app, auth) = (&ext.app, auth.clone());
        async move {
            app.call::<_, AuthResponse>("tokuly.identity.v1.AuthService/SwitchAccount", Some(&auth), &request).await
        }
    };
    // To the main account (no two-factor): no code asked.
    let back = switch(&first.refresh_token, "", &secure_user).await.unwrap();
    assert!(!back.mfa_required && !back.access_token.is_empty());
    let main_now = TestUser { token: back.access_token.clone(), ..main_user.clone() };
    // Back to the secure one: its session passed TOTP just now, so no code either.
    let to_secure = switch(&added.refresh_token, "", &main_now).await.unwrap();
    assert!(!to_secure.mfa_required);
    let secure_now = TestUser { token: to_secure.access_token.clone(), ..secure_user.clone() };
    let back = switch(&back.refresh_token, "", &secure_now).await.unwrap();
    let main_now = TestUser { token: back.access_token.clone(), ..main_user.clone() };

    // Half a day later the secure account asks again.
    sqlx::query("UPDATE sessions SET mfa_at = now() - interval '13 hours' WHERE id = $1")
        .bind(secure_user.session_id)
        .execute(&ext.app.core.db)
        .await
        .unwrap();
    let asked = switch(&to_secure.refresh_token, "", &main_now).await.unwrap();
    assert!(asked.mfa_required && asked.access_token.is_empty());
    assert_eq!(switch(&to_secure.refresh_token, "000000", &main_now).await.unwrap_err().reason, "INVALID_TOTP");
    let passed = switch(&to_secure.refresh_token, &totp_at(&setup.secret, 1), &main_now).await.unwrap();
    assert!(!passed.mfa_required && !passed.access_token.is_empty());
    let log = security_log(&ext, &secure_id.to_string()).await;
    assert!(log.contains(&"ACCOUNT_SWITCHED".to_owned()) && log.contains(&"SIGN_IN_FAILED".to_owned()));
}

#[tokio::test]
async fn a_password_change_takes_the_parked_session_off_the_device() {
    let ext = start(IdentityConfig::default()).await;
    let mut browser = Browser::new(&ext);
    let first = browser.call("tokuly.identity.v1.AuthService/SignUp", None, sign_up_json(&unique("pa"), false)).await;
    let a_id = user_id(&first.body);
    let second = browser.call("tokuly.identity.v1.AuthService/SignUp", None, sign_up_json(&unique("pb"), true)).await;
    assert_eq!(second.status, 200);
    // A changes the password on another device: every other session of A ends, the parked one too.
    let elsewhere = ext.app.sign_in(a_id.parse().unwrap()).await;
    let _: ChangePasswordResponse = ext
        .app
        .call(
            "tokuly.identity.v1.AccountService/ChangePassword",
            Some(&elsewhere),
            &ChangePasswordRequest {
                current_password: PASSWORD.into(),
                new_password: "another long secret".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let listed = browser.call("tokuly.identity.v1.AuthService/ListDeviceAccounts", None, json!({})).await;
    let accounts = listed.body["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 1, "{accounts:?}");
    assert_ne!(accounts[0]["user"]["id"], json!(a_id));
    assert_eq!(listed.cookie("tokuly_ra").as_deref(), Some(""), "the dead token leaves the cookie");
}

#[tokio::test]
async fn concurrent_switches_settle_on_one_active_account() {
    let ext = start(IdentityConfig::default()).await;
    let mut browser = Browser::new(&ext);
    browser.call("tokuly.identity.v1.AuthService/SignUp", None, sign_up_json(&unique("ca"), false)).await;
    browser.call("tokuly.identity.v1.AuthService/SignUp", None, sign_up_json(&unique("cb"), true)).await;
    let listed = browser.call("tokuly.identity.v1.AuthService/ListDeviceAccounts", None, json!({})).await;
    let parked = listed.body["accounts"][1]["sessionId"].as_str().unwrap().to_owned();
    // Two tabs press the same switch at once (as many requests as the pool has connections).
    let jar = browser.jar.clone();
    let tabs =
        (0..5).map(|_| {
            let mut tab = Browser::new(&ext);
            tab.jar = jar.clone();
            let parked = parked.clone();
            async move {
                tab.call("tokuly.identity.v1.AuthService/SwitchAccount", None, json!({ "sessionId": parked })).await
            }
        });
    let answers = futures::future::join_all(tabs).await;
    assert!(
        answers.iter().any(|a| a.status == 200),
        "{:?}",
        answers.iter().map(|a| a.reason.clone()).collect::<Vec<_>>()
    );
    for answer in &answers {
        assert!(answer.status == 200 || !answer.reason.is_empty(), "every answer is a clean one");
    }
    let parked_now: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sessions WHERE id = $1::uuid AND parked_at IS NULL AND revoked_at IS NULL",
    )
    .bind(&parked)
    .fetch_one(&ext.app.core.db)
    .await
    .unwrap();
    assert_eq!(parked_now, 1, "the target ends up active");
}

#[tokio::test]
async fn a_shared_computer_signs_every_account_out_at_once() {
    let ext = start(IdentityConfig::default()).await;
    let mut browser = Browser::new(&ext);
    let first = browser.call("tokuly.identity.v1.AuthService/SignUp", None, sign_up_json(&unique("sa"), false)).await;
    let first_id = user_id(&first.body);
    let second = browser.call("tokuly.identity.v1.AuthService/SignUp", None, sign_up_json(&unique("sb"), true)).await;
    let token = second.body["accessToken"].as_str().unwrap().to_owned();
    let out = browser
        .call("tokuly.identity.v1.AuthService/SignOut", Some(&token), json!({ "allAccountsOnDevice": true }))
        .await;
    assert_eq!(out.status, 200, "{}", out.reason);
    assert_eq!(out.cookie("tokuly_rt").as_deref(), Some(""));
    assert_eq!(out.cookie("tokuly_ra").as_deref(), Some(""));
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sessions WHERE user_id IN ($1::uuid, $2::uuid) AND revoked_at IS NULL",
    )
    .bind(&first_id)
    .bind(user_id(&second.body))
    .fetch_one(&ext.app.core.db)
    .await
    .unwrap();
    assert_eq!(live, 0, "the parked account is signed out too");
    let listed = browser.call("tokuly.identity.v1.AuthService/ListDeviceAccounts", None, json!({})).await;
    assert!(listed.body["accounts"].as_array().is_none_or(|a| a.is_empty()));
}

#[tokio::test]
async fn cookies_split_over_several_header_fields_keep_every_account() {
    // The Compose edge reaches the API over HTTP/2, which may carry each
    // cookie in a header field of its own. The active account's refresh
    // cookie must be found whichever comes first.
    let ext = start(IdentityConfig::default()).await;
    let mut browser = Browser::new(&ext);
    browser.split_cookies = true;
    let first = browser.call("tokuly.identity.v1.AuthService/SignUp", None, sign_up_json(&unique("ha"), false)).await;
    let second = browser.call("tokuly.identity.v1.AuthService/SignUp", None, sign_up_json(&unique("hb"), true)).await;
    assert_eq!(second.status, 200, "{}", second.reason);
    let refreshed = browser.call("tokuly.identity.v1.AuthService/RefreshToken", None, json!({})).await;
    assert_eq!(refreshed.status, 200, "{}", refreshed.reason);
    assert_eq!(user_id(&refreshed.body), user_id(&second.body));
    let listed = browser.call("tokuly.identity.v1.AuthService/ListDeviceAccounts", None, json!({})).await;
    let accounts = listed.body["accounts"].as_array().cloned().unwrap_or_default();
    assert_eq!(accounts.len(), 2, "{accounts:?}");
    assert_eq!(accounts[0]["user"]["id"], json!(user_id(&second.body)));
    assert_eq!(accounts[1]["user"]["id"], json!(user_id(&first.body)));
}
