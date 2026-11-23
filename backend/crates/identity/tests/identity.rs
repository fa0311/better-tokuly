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
    let err = app.call::<_, EnableTotpResponse>("tokuly.identity.v1.AccountService/EnableTotp", Some(&user), &EnableTotpRequest { code: "000000".into(), ..Default::default() }).await;
    // (000000 could be the right code once in a million runs; only assert on the common case.)
    if let Err(err) = err {
        assert_eq!(err.reason, "INVALID_TOTP");
    }
    let code = totp_at(&setup.secret, -1);
    let enabled: EnableTotpResponse = app.call("tokuly.identity.v1.AccountService/EnableTotp", Some(&user), &EnableTotpRequest { code, ..Default::default() }).await.unwrap();
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
        .call("tokuly.identity.v1.AccountService/EnableTotp", Some(&user), &EnableTotpRequest { code: totp_at(&setup.secret, -1), ..Default::default() })
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
