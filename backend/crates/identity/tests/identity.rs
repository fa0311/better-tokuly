//! End-to-end tests of the identity RPCs against real PostgreSQL, Valkey and NATS.
use tokuly_proto::identity::*;
use tokuly_testkit::{TestApp, TestUser};

const MIGRATIONS: &[&str] = &["0001", "0100", "0400"];

async fn app() -> TestApp {
    TestApp::start(MIGRATIONS, |_core, identity, router| identity.register(router)).await
}

fn sign_up_request(tag: &str) -> SignUpRequest {
    SignUpRequest {
        email: format!("{tag}@example.com"),
        password: "correct horse battery".into(),
        handle: tag.to_owned(),
        display_name: "テスト".into(),
        birthdate: "2000-04-02".into(),
        accept_terms: true,
        ..Default::default()
    }
}

fn unique(prefix: &str) -> String {
    format!("{prefix}{}", tokuly_core::ids::random_base62(8).to_ascii_lowercase())
}

async fn me(app: &TestApp, user: &TestUser) -> Me {
    let response: GetMeResponse = app.call("tokuly.identity.v1.AccountService/GetMe", Some(user), &GetMeRequest::default()).await.unwrap();
    response.me.into_option().unwrap()
}

#[tokio::test]
async fn sign_up_creates_account_session_and_event() {
    let app = app().await;
    let tag = unique("su");
    let auth: AuthResponse = app.call("tokuly.identity.v1.AuthService/SignUp", None, &sign_up_request(&tag)).await.unwrap();
    assert!(!auth.access_token.is_empty() && !auth.refresh_token.is_empty());
    let account = auth.me.as_option().unwrap();
    assert_eq!(account.user.as_option().unwrap().handle, tag);
    assert!(!account.email_verified && account.has_password);

    let user_id: uuid::Uuid = account.user.as_option().unwrap().id.parse().unwrap();
    let events = app.events_by("identity.user.created", user_id).await;
    assert_eq!(events.len(), 1);

    // Same handle or email again is a conflict with a machine-readable reason.
    let dup = app.call::<_, AuthResponse>("tokuly.identity.v1.AuthService/SignUp", None, &sign_up_request(&tag)).await.unwrap_err();
    assert_eq!((dup.code.as_str(), dup.reason.as_str()), ("already_exists", "HANDLE_TAKEN"));
    let mut other = sign_up_request(&unique("su"));
    other.email = format!("{tag}@example.com");
    let dup = app.call::<_, AuthResponse>("tokuly.identity.v1.AuthService/SignUp", None, &other).await.unwrap_err();
    assert_eq!(dup.reason, "EMAIL_TAKEN");
}

#[tokio::test]
async fn sign_up_validates_input() {
    let app = app().await;
    let cases: Vec<(&str, Box<dyn Fn(&mut SignUpRequest)>)> = vec![
        ("TERMS_REQUIRED", Box::new(|r| r.accept_terms = false)),
        ("EMAIL_INVALID", Box::new(|r| r.email = "nope".into())),
        ("PASSWORD_TOO_SHORT", Box::new(|r| r.password = "short".into())),
        ("HANDLE_INVALID", Box::new(|r| r.handle = "a b".into())),
        ("HANDLE_RESERVED", Box::new(|r| r.handle = "admin".into())),
        ("DISPLAY_NAME_INVALID", Box::new(|r| r.display_name = "".into())),
        ("AGE_RESTRICTED", Box::new(|r| r.birthdate = "2020-01-01".into())),
        ("BIRTHDATE_INVALID", Box::new(|r| r.birthdate = "soon".into())),
    ];
    for (reason, mutate) in cases {
        let mut request = sign_up_request(&unique("v"));
        mutate(&mut request);
        let err = app.call::<_, AuthResponse>("tokuly.identity.v1.AuthService/SignUp", None, &request).await.unwrap_err();
        assert_eq!(err.reason, reason, "{err}");
    }
}

#[tokio::test]
async fn sign_in_with_email_or_handle_and_reject_wrong_password() {
    let app = app().await;
    let tag = unique("si");
    let _: AuthResponse = app.call("tokuly.identity.v1.AuthService/SignUp", None, &sign_up_request(&tag)).await.unwrap();
    for identifier in [tag.clone(), format!("@{}", tag.to_uppercase()), format!("{tag}@example.com")] {
        let auth: AuthResponse = app
            .call("tokuly.identity.v1.AuthService/SignIn", None, &SignInRequest { identifier, password: "correct horse battery".into(), ..Default::default() })
            .await
            .unwrap();
        assert!(!auth.access_token.is_empty());
    }
    let err = app
        .call::<_, AuthResponse>("tokuly.identity.v1.AuthService/SignIn", None, &SignInRequest { identifier: tag, password: "wrong wrong wrong".into(), ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!((err.status, err.reason.as_str()), (401, "INVALID_CREDENTIALS"));
    let err = app
        .call::<_, AuthResponse>("tokuly.identity.v1.AuthService/SignIn", None, &SignInRequest { identifier: unique("ghost"), password: "whatever it is".into(), ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!(err.reason, "INVALID_CREDENTIALS", "unknown users look the same as wrong passwords");
}

#[tokio::test]
async fn password_guesses_are_limited_per_account_however_it_is_named() {
    let app = app().await;
    let tag = unique("guessed");
    let _: AuthResponse = app.call("tokuly.identity.v1.AuthService/SignUp", None, &sign_up_request(&tag)).await.unwrap();
    // `@h`, `@@h`, `H` and the address all name the same account: every
    // spelling must draw from one budget, not a fresh one each.
    let spellings = (0..20).map(|i| match i % 4 {
        0 => format!("{}{tag}", "@".repeat(i + 1)),
        1 => tag.to_uppercase(),
        2 => format!(" {tag}@EXAMPLE.com "),
        _ => format!("@{}", tag.to_uppercase()),
    });
    let mut reasons = Vec::new();
    for identifier in spellings {
        let err = app
            .call::<_, AuthResponse>("tokuly.identity.v1.AuthService/SignIn", None, &SignInRequest { identifier, password: "wrong wrong wrong".into(), ..Default::default() })
            .await
            .unwrap_err();
        reasons.push(err.reason);
    }
    assert!(reasons.iter().any(|r| r == "RATE_LIMITED"), "{reasons:?}");
    let err = sign_in_with_code(&app, &format!("{}{tag}", "@".repeat(30)), String::new()).await.unwrap_err();
    assert_eq!(err.reason, "RATE_LIMITED", "the right password is not let through while the account is limited");

    // Unknown names are limited too, by their normalized form (from another
    // address, so the per-address limit does not hide it).
    let app = self::app().await;
    let ghost = unique("ghost");
    let mut reasons = Vec::new();
    for i in 0..15 {
        let err = app
            .call::<_, AuthResponse>("tokuly.identity.v1.AuthService/SignIn", None, &SignInRequest { identifier: format!("{}{ghost}", "@".repeat(i)), password: "whatever it is".into(), ..Default::default() })
            .await
            .unwrap_err();
        reasons.push(err.reason);
    }
    assert!(reasons.iter().any(|r| r == "RATE_LIMITED"), "{reasons:?}");
}

#[tokio::test]
async fn refresh_rotates_and_detects_replay() {
    let app = app().await;
    let auth: AuthResponse = app.call("tokuly.identity.v1.AuthService/SignUp", None, &sign_up_request(&unique("rf"))).await.unwrap();
    let first = auth.refresh_token.clone();

    let second: AuthResponse = app.call("tokuly.identity.v1.AuthService/RefreshToken", None, &RefreshTokenRequest { refresh_token: first.clone(), ..Default::default() }).await.unwrap();
    assert_ne!(second.refresh_token, first, "the refresh token rotates");

    // A racing request that still holds the old token converges on the current one.
    let raced: AuthResponse = app.call("tokuly.identity.v1.AuthService/RefreshToken", None, &RefreshTokenRequest { refresh_token: first.clone(), ..Default::default() }).await.unwrap();
    assert_eq!(raced.refresh_token, second.refresh_token);

    let third: AuthResponse = app.call("tokuly.identity.v1.AuthService/RefreshToken", None, &RefreshTokenRequest { refresh_token: second.refresh_token.clone(), ..Default::default() }).await.unwrap();

    // `first` is now two generations old: presenting it is a replay and kills the session.
    let err = app.call::<_, AuthResponse>("tokuly.identity.v1.AuthService/RefreshToken", None, &RefreshTokenRequest { refresh_token: first, ..Default::default() }).await.unwrap_err();
    assert_eq!(err.reason, "REFRESH_INVALID");
    let err = app.call::<_, AuthResponse>("tokuly.identity.v1.AuthService/RefreshToken", None, &RefreshTokenRequest { refresh_token: third.refresh_token, ..Default::default() }).await.unwrap_err();
    assert_eq!(err.status, 401, "the whole session was revoked");

    let err = app.call::<_, AuthResponse>("tokuly.identity.v1.AuthService/RefreshToken", None, &RefreshTokenRequest { refresh_token: "garbage".into(), ..Default::default() }).await.unwrap_err();
    assert_eq!(err.reason, "REFRESH_INVALID");
}

#[tokio::test]
async fn authentication_is_required_and_bad_tokens_are_distinguished() {
    let app = app().await;
    let err = app.call::<_, GetMeResponse>("tokuly.identity.v1.AccountService/GetMe", None, &GetMeRequest::default()).await.unwrap_err();
    assert_eq!((err.status, err.reason.as_str()), (401, "SIGN_IN_REQUIRED"));
    let mut user = app.user("tok").await;
    user.token = "not-a-jwt".into();
    let err = app.call::<_, GetMeResponse>("tokuly.identity.v1.AccountService/GetMe", Some(&user), &GetMeRequest::default()).await.unwrap_err();
    assert_eq!(err.reason, "TOKEN_INVALID");
}

#[tokio::test]
async fn sign_out_revokes_the_access_token_immediately() {
    let app = app().await;
    let user = app.user("out").await;
    me(&app, &user).await;
    let _: SignOutResponse = app.call("tokuly.identity.v1.AuthService/SignOut", Some(&user), &SignOutRequest::default()).await.unwrap();
    let err = app.call::<_, GetMeResponse>("tokuly.identity.v1.AccountService/GetMe", Some(&user), &GetMeRequest::default()).await.unwrap_err();
    assert_eq!(err.reason, "SESSION_REVOKED");
}

#[tokio::test]
async fn profile_updates_and_handle_uniqueness() {
    let app = app().await;
    let (alice, bob) = (app.user("alice").await, app.user("bob").await);
    let new_handle = unique("renamed");
    let response: UpdateProfileResponse = app
        .call(
            "tokuly.identity.v1.AccountService/UpdateProfile",
            Some(&alice),
            &UpdateProfileRequest { display_name: Some("ありす".into()), bio: Some("こんにちは！".into()), handle: Some(new_handle.clone()), time_zone: Some("Europe/Paris".into()), ..Default::default() },
        )
        .await
        .unwrap();
    let account = response.me.as_option().unwrap();
    assert_eq!(account.user.as_option().unwrap().display_name, "ありす");
    assert_eq!(account.user.as_option().unwrap().handle, new_handle);
    assert_eq!((account.bio.as_str(), account.time_zone.as_str()), ("こんにちは！", "Europe/Paris"));
    assert_eq!(app.events_by("identity.user.updated", alice.id).await.len(), 1);

    let err = app
        .call::<_, UpdateProfileResponse>("tokuly.identity.v1.AccountService/UpdateProfile", Some(&bob), &UpdateProfileRequest { handle: Some(new_handle), ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!(err.reason, "HANDLE_TAKEN");
    let err = app
        .call::<_, UpdateProfileResponse>("tokuly.identity.v1.AccountService/UpdateProfile", Some(&bob), &UpdateProfileRequest { time_zone: Some("Mars/Olympus".into()), ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!(err.reason, "TIME_ZONE_INVALID");
}

#[tokio::test]
async fn a_released_handle_is_kept_for_its_previous_owner() {
    let app = app().await;
    let (famous, squatter) = (app.user("famous").await, app.user("squatter").await);
    let rename = |user: &TestUser, handle: String| {
        let (app, user) = (&app, user.clone());
        async move {
            app.call::<_, UpdateProfileResponse>("tokuly.identity.v1.AccountService/UpdateProfile", Some(&user), &UpdateProfileRequest { handle: Some(handle), ..Default::default() })
                .await
        }
    };
    // Links, mentions and screenshots still say @famous: whoever took the
    // handle the moment it was let go would receive all of it.
    let old = famous.handle.clone();
    rename(&famous, unique("famous")).await.unwrap();
    assert_eq!(rename(&squatter, old.clone()).await.unwrap_err().reason, "HANDLE_TAKEN");
    let check: CheckHandleResponse = app.call("tokuly.identity.v1.AuthService/CheckHandle", None, &CheckHandleRequest { handle: old.clone(), ..Default::default() }).await.unwrap();
    assert_eq!((check.available, check.reason.as_str()), (false, "TAKEN"));
    let err = app.call::<_, AuthResponse>("tokuly.identity.v1.AuthService/SignUp", None, &sign_up_request(&old)).await.unwrap_err();
    assert_eq!(err.reason, "HANDLE_TAKEN");
    // The previous owner can still go back to it.
    rename(&famous, old.clone()).await.unwrap();

    // A purged account's handle is not up for grabs either.
    let leaving = app.user("leaving").await;
    sqlx::query("UPDATE users SET deletion_scheduled_at = now() - interval '1 minute' WHERE id = $1").bind(leaving.id).execute(&app.core.db).await.unwrap();
    app.identity.purge_deleted_accounts().await.unwrap();
    assert_eq!(rename(&squatter, leaving.handle.clone()).await.unwrap_err().reason, "HANDLE_TAKEN");
}

#[tokio::test]
async fn one_account_cannot_tie_up_the_database_pool() {
    let app = app().await;
    let user = app.user("burst").await;
    let current: GetSettingsResponse = app.call("tokuly.identity.v1.AccountService/GetSettings", Some(&user), &GetSettingsRequest::default()).await.unwrap();
    let appearance = current.settings.as_option().unwrap().appearance.clone();
    // Twice as many writes at once as the test pool has connections, all
    // from one account, through every RPC that opens a transaction.
    let calls = (0..10).map(|i| {
        let (app, user, appearance) = (&app, user.clone(), appearance.clone());
        async move {
            match i % 3 {
                0 => app.call::<_, UpdateProfileResponse>("tokuly.identity.v1.AccountService/UpdateProfile", Some(&user), &UpdateProfileRequest { bio: Some(format!("{i}")), ..Default::default() }).await.map(drop),
                1 => app.call::<_, SetStatusResponse>("tokuly.identity.v1.AccountService/SetStatus", Some(&user), &SetStatusRequest { emoji: "🍜".into(), ..Default::default() }).await.map(drop),
                _ => app
                    .call::<_, UpdateSettingsResponse>("tokuly.identity.v1.AccountService/UpdateSettings", Some(&user), &UpdateSettingsRequest { appearance, ..Default::default() })
                    .await
                    .map(drop),
            }
        }
    });
    let results = tokio::time::timeout(std::time::Duration::from_secs(10), futures::future::join_all(calls)).await.expect("the burst finishes well within the pool's acquire timeout");
    assert!(results.iter().all(Result::is_ok), "{results:?}");
}

#[tokio::test]
async fn settings_default_then_persist() {
    let app = app().await;
    let user = app.user("set").await;
    let response: GetSettingsResponse = app.call("tokuly.identity.v1.AccountService/GetSettings", Some(&user), &GetSettingsRequest::default()).await.unwrap();
    let settings = response.settings.into_option().unwrap();
    let mut privacy = settings.privacy.into_option().unwrap();
    let mut appearance = settings.appearance.into_option().unwrap();
    assert!(privacy.discoverable && privacy.record_history);
    assert_eq!(appearance.dopamine_level, 3);

    privacy.discoverable = false;
    privacy.location_history_days = 30;
    appearance.dopamine_level = 5;
    appearance.theme = "dark".into();
    let updated: UpdateSettingsResponse = app
        .call("tokuly.identity.v1.AccountService/UpdateSettings", Some(&user), &UpdateSettingsRequest { privacy: privacy.into(), appearance: appearance.clone().into(), ..Default::default() })
        .await
        .unwrap();
    let settings = updated.settings.into_option().unwrap();
    assert!(!settings.privacy.as_option().unwrap().discoverable);
    assert_eq!(settings.privacy.as_option().unwrap().location_history_days, 30);
    assert_eq!(settings.appearance.as_option().unwrap().dopamine_level, 5);
    assert_eq!(app.events_by("identity.user.privacy_changed", user.id).await.len(), 1);

    appearance.dopamine_level = 11;
    let err = app
        .call::<_, UpdateSettingsResponse>("tokuly.identity.v1.AccountService/UpdateSettings", Some(&user), &UpdateSettingsRequest { appearance: appearance.into(), ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!(err.reason, "SETTINGS_INVALID");
}

#[tokio::test]
async fn status_can_be_set_and_cleared() {
    let app = app().await;
    let user = app.user("st").await;
    let set: SetStatusResponse = app
        .call("tokuly.identity.v1.AccountService/SetStatus", Some(&user), &SetStatusRequest { emoji: "🍜".into(), text: "ラーメン中".into(), expires_in_seconds: 3600, ..Default::default() })
        .await
        .unwrap();
    assert!(set.status.as_option().unwrap().expires_at.is_set());
    assert_eq!(me(&app, &user).await.status.as_option().unwrap().text, "ラーメン中");
    let cleared: SetStatusResponse = app.call("tokuly.identity.v1.AccountService/SetStatus", Some(&user), &SetStatusRequest::default()).await.unwrap();
    assert!(cleared.status.is_unset());
    assert!(me(&app, &user).await.status.is_unset());
}

#[tokio::test]
async fn totp_setup_then_sign_in_requires_code_and_recovery_code_works_once() {
    let app = app().await;
    let tag = unique("mfa");
    let auth: AuthResponse = app.call("tokuly.identity.v1.AuthService/SignUp", None, &sign_up_request(&tag)).await.unwrap();
    let user_id: uuid::Uuid = auth.me.as_option().unwrap().user.as_option().unwrap().id.parse().unwrap();
    let user = app.sign_in(user_id).await;

    let setup: BeginTotpSetupResponse = app.call("tokuly.identity.v1.AccountService/BeginTotpSetup", Some(&user), &BeginTotpSetupRequest::default()).await.unwrap();
    assert!(setup.otpauth_url.starts_with("otpauth://totp/"));
    let err = app.call::<_, EnableTotpResponse>("tokuly.identity.v1.AccountService/EnableTotp", Some(&user), &EnableTotpRequest { code: "000000".into(), current_password: "correct horse battery".into(), ..Default::default() }).await;
    // (000000 could be the right code once in a million runs; only assert on the common case.)
    if let Err(err) = err {
        assert_eq!(err.reason, "INVALID_TOTP");
    }
    let code = totp_at(&setup.secret, -1);
    let enabled: EnableTotpResponse = app.call("tokuly.identity.v1.AccountService/EnableTotp", Some(&user), &EnableTotpRequest { code, current_password: "correct horse battery".into(), ..Default::default() }).await.unwrap();
    assert_eq!(enabled.recovery_codes.len(), 10);
    assert!(me(&app, &user).await.totp_enabled);

    let sign_in = |totp_code: String| {
        let tag = tag.clone();
        let app = &app;
        async move {
            app.call::<_, AuthResponse>("tokuly.identity.v1.AuthService/SignIn", None, &SignInRequest { identifier: tag, password: "correct horse battery".into(), totp_code, ..Default::default() }).await
        }
    };
    let pending = sign_in(String::new()).await.unwrap();
    assert!(pending.mfa_required && pending.access_token.is_empty());
    assert_eq!(sign_in("12".into()).await.unwrap_err().reason, "INVALID_TOTP");
    assert!(!sign_in(totp_now(&setup.secret)).await.unwrap().access_token.is_empty());

    let recovery = enabled.recovery_codes[0].clone();
    assert!(!sign_in(recovery.to_uppercase()).await.unwrap().access_token.is_empty(), "recovery codes are case-insensitive");
    assert_eq!(sign_in(recovery).await.unwrap_err().reason, "INVALID_TOTP", "a recovery code works only once");
}

/// Current TOTP code for a base32 secret (RFC 6238, SHA-1, 30 s, 6 digits).
fn totp_now(secret_base32: &str) -> String {
    totp_at(secret_base32, 0)
}

/// The TOTP code `steps` time steps away from now (the server accepts one
/// step of clock skew either way, and every step only once).
fn totp_at(secret_base32: &str, steps: i64) -> String {
    use totp_rs::{Builder, Secret};
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    Builder::new()
        .with_secret(Secret::try_from_base32(secret_base32).unwrap())
        .with_account_name("test".to_owned())
        .with_issuer(Some("Tokuly"))
        .build()
        .unwrap()
        .generate((now + steps * 30) as u64)
        .to_string()
}

/// Signs up a fresh account with TOTP turned on; returns its handle, its
/// signed-in test user and the TOTP secret. Enabling spends the code of the
/// previous time step, so the current one is still unused.
async fn user_with_totp(app: &TestApp, prefix: &str) -> (String, TestUser, String) {
    let tag = unique(prefix);
    let auth: AuthResponse = app.call("tokuly.identity.v1.AuthService/SignUp", None, &sign_up_request(&tag)).await.unwrap();
    let user_id: uuid::Uuid = auth.me.as_option().unwrap().user.as_option().unwrap().id.parse().unwrap();
    let user = app.sign_in(user_id).await;
    let setup: BeginTotpSetupResponse = app.call("tokuly.identity.v1.AccountService/BeginTotpSetup", Some(&user), &BeginTotpSetupRequest::default()).await.unwrap();
    let _: EnableTotpResponse = app
        .call("tokuly.identity.v1.AccountService/EnableTotp", Some(&user), &EnableTotpRequest { code: totp_at(&setup.secret, -1), current_password: "correct horse battery".into(), ..Default::default() })
        .await
        .unwrap();
    (tag, user, setup.secret)
}

async fn sign_in_with_code(app: &TestApp, identifier: &str, totp_code: String) -> Result<AuthResponse, tokuly_testkit::RpcError> {
    app.call(
        "tokuly.identity.v1.AuthService/SignIn",
        None,
        &SignInRequest { identifier: identifier.to_owned(), password: "correct horse battery".into(), totp_code, ..Default::default() },
    )
    .await
}

#[tokio::test]
async fn a_totp_code_is_accepted_only_once() {
    let app = app().await;
    let (tag, user, secret) = user_with_totp(&app, "replay").await;

    // RFC 6238 §5.2: a code that was seen once (shoulder-surfed, phished
    // through a proxy) must not sign anyone in a second time.
    let code = totp_now(&secret);
    assert!(!sign_in_with_code(&app, &tag, code.clone()).await.unwrap().access_token.is_empty());
    assert_eq!(sign_in_with_code(&app, &tag, code.clone()).await.unwrap_err().reason, "INVALID_TOTP", "the same code again");
    let err = app
        .call::<_, DisableTotpResponse>("tokuly.identity.v1.AccountService/DisableTotp", Some(&user), &DisableTotpRequest { code, ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!(err.reason, "INVALID_TOTP", "a spent code cannot turn two-step verification off either");
    // The code of the next step is still good.
    assert!(!sign_in_with_code(&app, &tag, totp_at(&secret, 1)).await.unwrap().access_token.is_empty());
}

#[tokio::test]
async fn two_sign_ins_racing_with_one_code_let_one_through() {
    let app = app().await;
    let (tag, _user, secret) = user_with_totp(&app, "race").await;
    let code = totp_now(&secret);
    let (a, b) = tokio::join!(sign_in_with_code(&app, &tag, code.clone()), sign_in_with_code(&app, &tag, code));
    assert_eq!(a.is_ok() as u8 + b.is_ok() as u8, 1, "{a:?} / {b:?}");
}

#[tokio::test]
async fn totp_guesses_are_capped_per_day_not_only_per_five_minutes() {
    let app = app().await;
    let (tag, user, secret) = user_with_totp(&app, "guess").await;
    // 8 guesses per 5 minutes alone allow ~2300 a day: with three codes
    // valid at any time, someone who has the password gets in within a month
    // about one time in five. Pretend today's budget is already spent.
    app.core.cache.set_string(&format!("rl:totp-day:{}", user.id), "30", Some(std::time::Duration::from_secs(600))).await.unwrap();
    let err = sign_in_with_code(&app, &tag, totp_now(&secret)).await.unwrap_err();
    assert_eq!(err.reason, "RATE_LIMITED");
}

/// Stores a mailed token as `ChangeEmail` / `RequestPasswordReset` would and
/// returns it (the real ones only travel by mail).
async fn mailed_token(app: &TestApp, user_id: uuid::Uuid, purpose: &str, email: &str) -> String {
    let token = tokuly_core::keys::random_token(32);
    sqlx::query("INSERT INTO email_tokens (token_hash, user_id, purpose, email, expires_at) VALUES ($1, $2, $3, $4::citext, now() + interval '1 hour')")
        .bind(app.core.keys.token_hash(&token))
        .bind(user_id)
        .bind(purpose)
        .bind(email)
        .execute(&app.core.db)
        .await
        .unwrap();
    token
}

#[tokio::test]
async fn securing_the_password_kills_a_pending_email_change() {
    let app = app().await;
    for securing in ["ChangePassword", "ResetPassword"] {
        let tag = unique("hijack");
        let auth: AuthResponse = app.call("tokuly.identity.v1.AuthService/SignUp", None, &sign_up_request(&tag)).await.unwrap();
        let user_id: uuid::Uuid = auth.me.as_option().unwrap().user.as_option().unwrap().id.parse().unwrap();
        let owner = app.sign_in(user_id).await;
        // Someone who learned the password asks to move the account to their
        // address; the confirmation link lands in their own inbox.
        let change = mailed_token(&app, user_id, "change", &format!("{tag}@attacker.example")).await;
        // The owner notices and secures the account.
        if securing == "ChangePassword" {
            let _: ChangePasswordResponse = app
                .call(
                    "tokuly.identity.v1.AccountService/ChangePassword",
                    Some(&owner),
                    &ChangePasswordRequest { current_password: "correct horse battery".into(), new_password: "a much better secret".into(), ..Default::default() },
                )
                .await
                .unwrap();
        } else {
            let reset = mailed_token(&app, user_id, "reset", &format!("{tag}@example.com")).await;
            let _: ResetPasswordResponse = app
                .call("tokuly.identity.v1.AuthService/ResetPassword", None, &ResetPasswordRequest { token: reset, new_password: "a much better secret".into(), ..Default::default() })
                .await
                .unwrap();
        }
        let err = app
            .call::<_, VerifyEmailResponse>("tokuly.identity.v1.AuthService/VerifyEmail", None, &VerifyEmailRequest { token: change, ..Default::default() })
            .await
            .unwrap_err();
        assert_eq!(err.reason, "TOKEN_INVALID", "{securing} must end the pending address change");
        let owner = app.sign_in(user_id).await;
        assert_eq!(me(&app, &owner).await.email, format!("{tag}@example.com"));
    }
}

#[tokio::test]
async fn a_new_sign_in_credential_takes_the_password() {
    let app = app().await;
    let tag = unique("cred");
    let auth: AuthResponse = app.call("tokuly.identity.v1.AuthService/SignUp", None, &sign_up_request(&tag)).await.unwrap();
    let user_id: uuid::Uuid = auth.me.as_option().unwrap().user.as_option().unwrap().id.parse().unwrap();
    // Someone holding only an access token (15 minutes, from an unlocked
    // phone or a script on the page) must not be able to add a passkey of
    // their own (it signs in without the second factor and survives a
    // password reset) or lock the owner out behind an authenticator they hold.
    let stolen = app.sign_in(user_id).await;
    for password in ["", "wrong wrong wrong"] {
        let err = app
            .call::<_, BeginPasskeyRegistrationResponse>(
                "tokuly.identity.v1.AuthService/BeginPasskeyRegistration",
                Some(&stolen),
                &BeginPasskeyRegistrationRequest { name: "mine now".into(), current_password: password.into(), ..Default::default() },
            )
            .await
            .unwrap_err();
        assert_eq!(err.reason, "INVALID_CREDENTIALS", "passkey with password {password:?}");

        let setup: BeginTotpSetupResponse = app.call("tokuly.identity.v1.AccountService/BeginTotpSetup", Some(&stolen), &BeginTotpSetupRequest::default()).await.unwrap();
        let err = app
            .call::<_, EnableTotpResponse>(
                "tokuly.identity.v1.AccountService/EnableTotp",
                Some(&stolen),
                &EnableTotpRequest { code: totp_now(&setup.secret), current_password: password.into(), ..Default::default() },
            )
            .await
            .unwrap_err();
        assert_eq!(err.reason, "INVALID_CREDENTIALS", "TOTP with password {password:?}");
    }
    assert!(!me(&app, &stolen).await.totp_enabled);

    // With the password both go ahead.
    let begun: BeginPasskeyRegistrationResponse = app
        .call(
            "tokuly.identity.v1.AuthService/BeginPasskeyRegistration",
            Some(&stolen),
            &BeginPasskeyRegistrationRequest { name: "Laptop".into(), current_password: "correct horse battery".into(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(!begun.challenge_id.is_empty());
    let setup: BeginTotpSetupResponse = app.call("tokuly.identity.v1.AccountService/BeginTotpSetup", Some(&stolen), &BeginTotpSetupRequest::default()).await.unwrap();
    let _: EnableTotpResponse = app
        .call(
            "tokuly.identity.v1.AccountService/EnableTotp",
            Some(&stolen),
            &EnableTotpRequest { code: totp_at(&setup.secret, 1), current_password: "correct horse battery".into(), ..Default::default() },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn changing_the_email_does_not_tell_whose_address_is_registered() {
    let app = app().await;
    let (taken, free) = (unique("known"), unique("unknown"));
    let _: AuthResponse = app.call("tokuly.identity.v1.AuthService/SignUp", None, &sign_up_request(&taken)).await.unwrap();
    let curious = app.user("curious").await;
    // Asking for someone else's address and for an unused one must look the
    // same, or ChangeEmail answers "is this person on Tokuly?" for free.
    for address in [format!("{taken}@example.com"), format!("{free}@example.com")] {
        let answer = app
            .call::<_, ChangeEmailResponse>("tokuly.identity.v1.AccountService/ChangeEmail", Some(&curious), &ChangeEmailRequest { new_email: address.clone(), ..Default::default() })
            .await;
        assert!(answer.is_ok(), "{address}: {answer:?}");
    }
    // Nothing was sent for the registered address, so it cannot be taken over either.
    let pending: Vec<String> = sqlx::query_scalar("SELECT email::text FROM email_tokens WHERE user_id = $1 AND purpose = 'change'")
        .bind(curious.id)
        .fetch_all(&app.core.db)
        .await
        .unwrap();
    assert!(!pending.contains(&format!("{taken}@example.com")), "{pending:?}");
}

/// Stands in for the social domain's blocks: `can_view` is false for a
/// blocked pair, either way round.
#[derive(Default)]
struct Blocks(std::sync::Mutex<Vec<(uuid::Uuid, uuid::Uuid)>>);

#[async_trait::async_trait]
impl tokuly_identity::ProfileExtension for Blocks {
    async fn can_view(&self, viewer: uuid::Uuid, owner: uuid::Uuid) -> tokuly_core::error::AppResult<bool> {
        Ok(!self.0.lock().unwrap().iter().any(|&(a, b)| (a, b) == (viewer, owner) || (a, b) == (owner, viewer)))
    }
}

#[tokio::test]
async fn profile_views_leave_out_blocked_and_suspended_people() {
    let blocks = std::sync::Arc::new(Blocks::default());
    let extension = blocks.clone();
    let app = TestApp::start(MIGRATIONS, move |_core, identity, router| {
        identity.add_profile_extension(extension);
        identity.register(router)
    })
    .await;
    let (owner, stalker, friend, spammer) = (app.user("owner").await, app.user("stalker").await, app.user("friend").await, app.user("spammer").await);
    let view = |viewer: &TestUser| {
        let (app, viewer, owner) = (&app, viewer.clone(), owner.id);
        async move {
            let _: RecordProfileViewResponse = app
                .call("tokuly.identity.v1.ProfileService/RecordProfileView", Some(&viewer), &RecordProfileViewRequest { user_id: owner.to_string(), ..Default::default() })
                .await
                .unwrap();
        }
    };
    // The owner blocked the stalker; blocking hides both from each other everywhere.
    blocks.0.lock().unwrap().push((owner.id, stalker.id));
    view(&stalker).await;
    view(&friend).await;
    view(&spammer).await;
    app.identity.set_banned(spammer.id, Some("spam")).await.unwrap();

    let views: GetProfileViewsResponse = app.call("tokuly.identity.v1.ProfileService/GetProfileViews", Some(&owner), &GetProfileViewsRequest::default()).await.unwrap();
    let named: Vec<String> = views.viewers.iter().map(|v| v.user.as_option().unwrap().handle.clone()).collect();
    assert_eq!(named, [friend.handle.clone()], "only people the owner may see are named");
    assert_eq!(views.total, 2, "a blocked person's look is not even counted");

    // Someone blocked after looking drops out of the list as well.
    blocks.0.lock().unwrap().push((friend.id, owner.id));
    let views: GetProfileViewsResponse = app.call("tokuly.identity.v1.ProfileService/GetProfileViews", Some(&owner), &GetProfileViewsRequest::default()).await.unwrap();
    assert!(views.viewers.is_empty(), "{:?}", views.viewers);
}

#[tokio::test]
async fn only_the_operator_account_gets_an_official_looking_handle() {
    let operator = unique("tokuly_");
    let cfg = tokuly_identity::IdentityConfig { operator_account: format!("@{}", operator.to_uppercase()), ..Default::default() };
    let app = TestApp::start(MIGRATIONS, move |core, _identity, router| tokuly_identity::Identity::new(core.clone(), cfg).unwrap().register(router)).await;
    let check = |handle: String| {
        let app = &app;
        async move {
            app.call::<_, CheckHandleResponse>("tokuly.identity.v1.AuthService/CheckHandle", None, &CheckHandleRequest { handle, ..Default::default() }).await.unwrap()
        }
    };
    let impostor = unique("t0kuly_official");
    let answer = check(impostor.clone()).await;
    assert_eq!((answer.available, answer.reason.as_str()), (false, "RESERVED"));
    let err = app.call::<_, AuthResponse>("tokuly.identity.v1.AuthService/SignUp", None, &sign_up_request(&impostor)).await.unwrap_err();
    assert_eq!(err.reason, "HANDLE_RESERVED");

    assert!(check(operator.clone()).await.available);
    let _: AuthResponse = app.call("tokuly.identity.v1.AuthService/SignUp", None, &sign_up_request(&operator)).await.unwrap();
}

#[tokio::test]
async fn device_login_flow_issues_tokens_once() {
    let app = app().await;
    let approver = app.user("phone").await;
    let start: StartDeviceLoginResponse = app
        .call(
            "tokuly.identity.v1.AuthService/StartDeviceLogin",
            None,
            &StartDeviceLoginRequest { client: ClientInfo { device_name: "Tokuly Studio".into(), ..Default::default() }.into(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(start.verification_url.contains(&start.user_code));
    let poll = || async {
        app.call::<_, PollDeviceLoginResponse>("tokuly.identity.v1.AuthService/PollDeviceLogin", None, &PollDeviceLoginRequest { device_code: start.device_code.clone(), ..Default::default() })
            .await
            .unwrap()
    };
    assert_eq!(poll().await.status, DeviceLoginStatus::DEVICE_LOGIN_STATUS_PENDING);

    let seen: GetDeviceLoginResponse = app
        .call("tokuly.identity.v1.AuthService/GetDeviceLogin", Some(&approver), &GetDeviceLoginRequest { user_code: start.user_code.to_lowercase().replace('-', " "), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(seen.client.as_option().unwrap().device_name, "Tokuly Studio");
    let _: ApproveDeviceLoginResponse = app
        .call("tokuly.identity.v1.AuthService/ApproveDeviceLogin", Some(&approver), &ApproveDeviceLoginRequest { user_code: start.user_code.clone(), approve: true, ..Default::default() })
        .await
        .unwrap();

    let approved = poll().await;
    assert_eq!(approved.status, DeviceLoginStatus::DEVICE_LOGIN_STATUS_APPROVED);
    let auth = approved.auth.as_option().unwrap();
    assert_eq!(auth.me.as_option().unwrap().user.as_option().unwrap().id, approver.id.to_string());
    assert!(!auth.refresh_token.is_empty());
    assert_eq!(poll().await.status, DeviceLoginStatus::DEVICE_LOGIN_STATUS_EXPIRED, "tokens are handed out exactly once");
}

#[tokio::test]
async fn device_codes_cannot_be_guessed_without_limit() {
    let app = app().await;
    let guesser = app.user("guesser").await;
    // Approving a code someone else is waiting on signs their device into
    // the guesser's account (their uploads, their messages). Codes are hard
    // to hit, but nothing stopped trying them all day.
    let mut reasons = Vec::new();
    for _ in 0..40 {
        let err = app
            .call::<_, GetDeviceLoginResponse>("tokuly.identity.v1.AuthService/GetDeviceLogin", Some(&guesser), &GetDeviceLoginRequest { user_code: tokuly_core::ids::random_base62(8), ..Default::default() })
            .await
            .unwrap_err();
        reasons.push(err.reason);
    }
    assert!(reasons.iter().any(|r| r == "RATE_LIMITED"), "{reasons:?}");
    let err = app
        .call::<_, ApproveDeviceLoginResponse>("tokuly.identity.v1.AuthService/ApproveDeviceLogin", Some(&guesser), &ApproveDeviceLoginRequest { user_code: "ABCD-2345".into(), approve: true, ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!(err.reason, "RATE_LIMITED", "approving counts against the same limit");
}

#[tokio::test]
async fn profiles_search_and_views() {
    let app = app().await;
    let name = unique("Zephyrine");
    let (owner, viewer) = (app.user(&name).await, app.user("viewer").await);

    let by_handle: GetProfileResponse = app
        .call("tokuly.identity.v1.ProfileService/GetProfile", None, &GetProfileRequest { by: Some(get_profile_request::By::Handle(format!("@{}", owner.handle))), ..Default::default() })
        .await
        .unwrap();
    let profile = by_handle.profile.as_option().unwrap();
    assert_eq!(profile.user.as_option().unwrap().id, owner.id.to_string());
    assert_eq!(profile.relationship, Relationship::RELATIONSHIP_NONE);

    let own: GetProfileResponse = app
        .call("tokuly.identity.v1.ProfileService/GetProfile", Some(&owner), &GetProfileRequest { by: Some(get_profile_request::By::UserId(owner.id.to_string())), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(own.profile.as_option().unwrap().relationship, Relationship::RELATIONSHIP_SELF);

    let missing = app
        .call::<_, GetProfileResponse>("tokuly.identity.v1.ProfileService/GetProfile", None, &GetProfileRequest { by: Some(get_profile_request::By::Handle(unique("nobody"))), ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!((missing.status, missing.reason.as_str()), (404, "USER_NOT_FOUND"));

    // Search finds by display name fragment and by handle.
    let found: SearchUsersResponse = app
        .call("tokuly.identity.v1.ProfileService/SearchUsers", Some(&viewer), &SearchUsersRequest { query: name[..9].to_lowercase(), ..Default::default() })
        .await
        .unwrap();
    assert!(found.results.iter().any(|r| r.user.as_option().unwrap().id == owner.id.to_string()));

    // Views are counted once per viewer per day and never for yourself.
    for who in [&viewer, &viewer, &owner] {
        let _: RecordProfileViewResponse = app
            .call("tokuly.identity.v1.ProfileService/RecordProfileView", Some(who), &RecordProfileViewRequest { user_id: owner.id.to_string(), ..Default::default() })
            .await
            .unwrap();
    }
    let views: GetProfileViewsResponse = app.call("tokuly.identity.v1.ProfileService/GetProfileViews", Some(&owner), &GetProfileViewsRequest::default()).await.unwrap();
    assert_eq!((views.total, views.last_24h), (1, 1));
    assert_eq!(views.viewers.len(), 1);
}

#[tokio::test]
async fn account_deletion_is_scheduled_cancellable_and_purged_when_due() {
    let app = app().await;
    let user = app.user("bye").await;
    let scheduled: RequestAccountDeletionResponse = app
        .call("tokuly.identity.v1.AccountService/RequestAccountDeletion", Some(&user), &RequestAccountDeletionRequest::default())
        .await
        .unwrap();
    assert!(scheduled.deletion_scheduled_at.is_set());
    assert!(me(&app, &user).await.deletion_scheduled_at.is_set());
    let _: CancelAccountDeletionResponse = app.call("tokuly.identity.v1.AccountService/CancelAccountDeletion", Some(&user), &CancelAccountDeletionRequest::default()).await.unwrap();
    assert!(me(&app, &user).await.deletion_scheduled_at.is_unset());

    // Schedule again, pretend the grace period passed, and purge.
    let _: RequestAccountDeletionResponse = app.call("tokuly.identity.v1.AccountService/RequestAccountDeletion", Some(&user), &RequestAccountDeletionRequest::default()).await.unwrap();
    sqlx::query("UPDATE users SET deletion_scheduled_at = now() - interval '1 minute' WHERE id = $1").bind(user.id).execute(&app.core.db).await.unwrap();
    app.identity.purge_deleted_accounts().await.unwrap();

    let summary = tokuly_identity::summary(&app.core, user.id).await.unwrap();
    assert!(summary.deleted && summary.display_name.is_empty());
    assert_eq!(app.events_by("identity.user.deleted", user.id).await.len(), 1);
    let gone = app
        .call::<_, GetProfileResponse>("tokuly.identity.v1.ProfileService/GetProfile", None, &GetProfileRequest { by: Some(get_profile_request::By::UserId(user.id.to_string())), ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!(gone.status, 404);

    // A device still open at that moment is signed out at once: its access
    // token must not keep writing data back after every domain purged it.
    let err = app
        .call::<_, UpdateProfileResponse>("tokuly.identity.v1.AccountService/UpdateProfile", Some(&user), &UpdateProfileRequest { display_name: Some("still here".into()), ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!((err.status, err.reason.as_str()), (401, "SESSION_REVOKED"));
}

#[tokio::test]
async fn profile_images_hold_their_uploads() {
    use tokuly_media::{assets, testing};
    use tokuly_proto::media::AssetPurpose;

    let app = app().await;
    let (owner, stranger) = (app.user("pic").await, app.user("thief").await);
    let first = testing::ready_image(&app.core, owner.id, AssetPurpose::ASSET_PURPOSE_AVATAR).await.unwrap();
    let second = testing::ready_image(&app.core, owner.id, AssetPurpose::ASSET_PURPOSE_AVATAR).await.unwrap();
    let set_avatar = |user: &TestUser, asset_id: String| {
        let (app, user) = (&app, user.clone());
        async move {
            app.call::<_, UpdateProfileResponse>(
                "tokuly.identity.v1.AccountService/UpdateProfile",
                Some(&user),
                &UpdateProfileRequest { avatar_asset_id: Some(asset_id), ..Default::default() },
            )
            .await
        }
    };

    // Setting the avatar copies the processed image and holds the upload.
    let updated = set_avatar(&owner, first.id.to_string()).await.unwrap();
    let avatar = updated.me.as_option().unwrap().user.as_option().unwrap().avatar.as_option().unwrap().clone();
    assert_eq!(avatar.asset_id, first.id.to_string());
    assert!(assets::is_attached(&app.core, first.id).await.unwrap());

    // Replacing it releases the old upload and holds the new one.
    set_avatar(&owner, second.id.to_string()).await.unwrap();
    assert!(!assets::is_attached(&app.core, first.id).await.unwrap());
    assert!(assets::is_attached(&app.core, second.id).await.unwrap());

    // Someone else's upload cannot be used, and an empty ID clears the avatar.
    let err = set_avatar(&stranger, second.id.to_string()).await.unwrap_err();
    assert_eq!(err.reason, "ASSET_NOT_FOUND");
    let cleared = set_avatar(&owner, String::new()).await.unwrap();
    assert!(cleared.me.as_option().unwrap().user.as_option().unwrap().avatar.is_unset());
    assert!(!assets::is_attached(&app.core, second.id).await.unwrap());
}
