//! Personal access tokens: creation, the scopes that limit them, and
//! everything that ends them.
use tokuly_dev::{DevConfig, secrets};
use tokuly_proto::{dev::*, identity};
use uuid::Uuid;

use crate::common::*;

const GET_ME: &str = "tokuly.identity.v1.AccountService/GetMe";
const UPDATE_PROFILE: &str = "tokuly.identity.v1.AccountService/UpdateProfile";

fn create_request(name: &str, scopes: &[&str], expiry: TokenExpiry) -> CreateTokenRequest {
    CreateTokenRequest {
        name: name.into(),
        scopes: scopes.iter().map(|s| s.to_string()).collect(),
        expiry: expiry.into(),
        ..Default::default()
    }
}

async fn list(ctx: &Ctx, user: &tokuly_testkit::TestUser) -> Vec<AccessToken> {
    let listed: ListTokensResponse =
        ctx.app.call(&format!("{TOKEN_SERVICE}/ListTokens"), Some(user), &ListTokensRequest::default()).await.unwrap();
    listed.tokens
}

#[tokio::test]
async fn the_scope_catalogue_is_public_and_worded_for_people() {
    let ctx = start().await;
    let listed: ListScopesResponse =
        ctx.app.call(&format!("{TOKEN_SERVICE}/ListScopes"), None, &ListScopesRequest::default()).await.unwrap();
    let geo = listed.scopes.iter().find(|s| s.scope == "geo:read").expect("location has a scope of its own");
    assert!(geo.sensitive && !geo.write);
    assert!(geo.description.as_option().unwrap().en.contains("live location"));
    assert!(listed.scopes.iter().any(|s| s.scope == "chat:write" && s.write));
    assert!(!listed.scopes.iter().any(|s| s.domain == "admin"));
}

#[tokio::test]
async fn a_token_is_shown_once_stored_hashed_and_announced() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let request = create_request("backup script", &["notes:read", "identity:read"], TokenExpiry::TOKEN_EXPIRY_90_DAYS);
    let created: CreateTokenResponse =
        ctx.app.call(&format!("{TOKEN_SERVICE}/CreateToken"), Some(&alice), &request).await.unwrap();
    let token = created.token.as_option().unwrap();

    // Recognisable, checksummed, and never stored.
    assert!(created.secret.starts_with("tkl_pat_") && secrets::is(&created.secret, secrets::SecretKind::Personal));
    assert_eq!(token.hint, format!("tkl_pat_…{}", &created.secret[created.secret.len() - 4..]));
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM dev_tokens t WHERE to_jsonb(t)::text LIKE '%' || $1 || '%'")
            .bind(&created.secret)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(stored, 0, "the secret is not in the database");

    assert_eq!(
        (token.name.as_str(), token.scopes.as_slice()),
        ("backup script", ["identity:read".to_owned(), "notes:read".to_owned()].as_slice())
    );
    let days = (tokuly_core::time::from_ts(token.expires_at.as_option().unwrap()) - chrono::Utc::now()).num_days();
    assert!((89..=90).contains(&days) && !token.expired && token.last_used_at.as_option().is_none());

    // The list shows the token, not the secret.
    let listed = list(&ctx, &alice).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, token.id);
    assert!(!serde_json::to_string(&listed).unwrap().contains(&created.secret));

    // The owner is told, three ways.
    let notices = ctx.notifications(alice.id, "dev.token_created").await;
    assert_eq!(notices.len(), 1);
    assert!(notices[0]["body"]["ja"].as_str().unwrap().contains("backup script"));
    assert_eq!(ctx.security_events(alice.id, "API_TOKEN_CREATED").await, 1);
    let events = ctx.app.events_by("dev.token.created", alice.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(
        (events[0].data["token_id"].as_str().unwrap(), events[0].data["kind"].as_str().unwrap()),
        (token.id.as_str(), "pat")
    );
}

#[tokio::test]
async fn a_token_acts_as_its_owner_within_its_scopes() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let (token, secret) = personal_token(&ctx, &alice, &["identity:read"]).await;
    let caller = with_token(&alice, &secret);

    // What the token says about itself.
    let me = whoami(&ctx, &caller).await.unwrap();
    assert_eq!(me.kind.as_known(), Some(CredentialKind::CREDENTIAL_KIND_PERSONAL_TOKEN));
    assert_eq!(
        (me.user.as_option().unwrap().id.as_str(), me.credential_id.as_str()),
        (alice.id.to_string().as_str(), token.id.as_str())
    );
    assert_eq!((me.scopes.as_slice(), me.rate_limit_per_minute), (["identity:read".to_owned()].as_slice(), 600));
    // A session has no scopes to report.
    let session = whoami(&ctx, &alice).await.unwrap();
    assert_eq!(session.kind.as_known(), Some(CredentialKind::CREDENTIAL_KIND_SESSION));
    assert!(session.scopes.is_empty());

    // Another domain's RPCs, through the real middleware: reading is allowed...
    let got: identity::GetMeResponse =
        ctx.app.call(GET_ME, Some(&caller), &identity::GetMeRequest::default()).await.unwrap();
    assert_eq!(got.me.as_option().unwrap().user.as_option().unwrap().handle, alice.handle);
    // ...writing is not...
    let update = identity::UpdateProfileRequest { bio: Some("changed by a token".into()), ..Default::default() };
    let denied =
        ctx.app.call::<_, identity::UpdateProfileResponse>(UPDATE_PROFILE, Some(&caller), &update).await.unwrap_err();
    assert_eq!(
        (denied.status, denied.code.as_str(), denied.reason.as_str()),
        (403, "permission_denied", "INSUFFICIENT_SCOPE")
    );
    assert!(denied.message.contains("identity:write"), "{}", denied.message);
    // ...and what takes a person at a screen is closed whatever the scopes.
    let (_, all) = personal_token(&ctx, &alice, &["identity:write", "dev:read"]).await;
    let powerful = with_token(&alice, &all);
    let password = identity::ChangePasswordRequest {
        current_password: "x".into(),
        new_password: "a new password".into(),
        ..Default::default()
    };
    let denied = ctx
        .app
        .call::<_, identity::ChangePasswordResponse>(
            "tokuly.identity.v1.AccountService/ChangePassword",
            Some(&powerful),
            &password,
        )
        .await
        .unwrap_err();
    assert_eq!(denied.reason, "SESSION_REQUIRED");
    // A token cannot mint, list or revoke tokens.
    let request = create_request("escalation", &["chat:write"], TokenExpiry::TOKEN_EXPIRY_NEVER);
    for procedure in ["CreateToken", "ListTokens", "RevokeToken"] {
        let denied = ctx
            .app
            .call::<_, serde_json::Value>(&format!("{TOKEN_SERVICE}/{procedure}"), Some(&powerful), &request)
            .await
            .unwrap_err();
        assert_eq!(denied.reason, "SESSION_REQUIRED", "{procedure}");
    }
    // With the scope, the write goes through as the owner.
    let updated: identity::UpdateProfileResponse =
        ctx.app.call(UPDATE_PROFILE, Some(&powerful), &update).await.unwrap();
    assert_eq!(updated.me.as_option().unwrap().bio, "changed by a token");
}

#[tokio::test]
async fn token_requests_are_validated() {
    let ctx = start_with(DevConfig { max_tokens: 2, ..config() }).await;
    let alice = ctx.app.user("alice").await;
    let create = |request: CreateTokenRequest| {
        let (ctx, alice) = (&ctx, &alice);
        async move {
            ctx.app.call::<_, CreateTokenResponse>(&format!("{TOKEN_SERVICE}/CreateToken"), Some(alice), &request).await
        }
    };
    let long_name = "n".repeat(61);
    for (request, reason) in [
        (create_request("", &["notes:read"], TokenExpiry::TOKEN_EXPIRY_7_DAYS), "TOKEN_NAME_INVALID"),
        (create_request(&long_name, &["notes:read"], TokenExpiry::TOKEN_EXPIRY_7_DAYS), "TOKEN_NAME_INVALID"),
        (create_request("no scopes", &[], TokenExpiry::TOKEN_EXPIRY_7_DAYS), "SCOPES_REQUIRED"),
        (create_request("made up", &["notes:delete"], TokenExpiry::TOKEN_EXPIRY_7_DAYS), "SCOPE_UNKNOWN"),
        (create_request("moderator", &["admin:write"], TokenExpiry::TOKEN_EXPIRY_7_DAYS), "SCOPE_UNKNOWN"),
        (
            CreateTokenRequest {
                expiry: buffa::EnumValue::from(42),
                ..create_request("odd expiry", &["notes:read"], TokenExpiry::TOKEN_EXPIRY_7_DAYS)
            },
            "EXPIRY_INVALID",
        ),
    ] {
        assert_eq!(create(request).await.unwrap_err().reason, reason);
    }
    let anonymous = ctx
        .app
        .call::<_, CreateTokenResponse>(
            &format!("{TOKEN_SERVICE}/CreateToken"),
            None,
            &create_request("x", &["notes:read"], TokenExpiry::TOKEN_EXPIRY_7_DAYS),
        )
        .await;
    assert_eq!(anonymous.unwrap_err().reason, "SIGN_IN_REQUIRED");

    // No expiry choice means 30 days, "never" means none.
    let default =
        create(create_request("default", &["notes:read"], TokenExpiry::TOKEN_EXPIRY_UNSPECIFIED)).await.unwrap();
    assert!(default.token.as_option().unwrap().expires_at.as_option().is_some());
    let forever = create(create_request("forever", &["notes:read"], TokenExpiry::TOKEN_EXPIRY_NEVER)).await.unwrap();
    assert!(forever.token.as_option().unwrap().expires_at.as_option().is_none());
    // The limit counts live tokens: revoking one makes room.
    let full = create(create_request("third", &["notes:read"], TokenExpiry::TOKEN_EXPIRY_7_DAYS)).await.unwrap_err();
    assert_eq!((full.code.as_str(), full.reason.as_str()), ("failed_precondition", "TOO_MANY_TOKENS"));
    let revoke = RevokeTokenRequest { token_id: forever.token.as_option().unwrap().id.clone(), ..Default::default() };
    ctx.app
        .call::<_, RevokeTokenResponse>(&format!("{TOKEN_SERVICE}/RevokeToken"), Some(&alice), &revoke)
        .await
        .unwrap();
    assert!(create(create_request("third", &["notes:read"], TokenExpiry::TOKEN_EXPIRY_7_DAYS)).await.is_ok());
}

#[tokio::test]
async fn revoking_a_token_ends_it_at_once() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let (token, secret) = personal_token(&ctx, &alice, &["identity:read"]).await;
    let caller = with_token(&alice, &secret);
    // In use (and so cached) before it is revoked.
    assert!(whoami(&ctx, &caller).await.is_ok());

    let revoke = RevokeTokenRequest { token_id: token.id.clone(), ..Default::default() };
    // Not by somebody else, who is not even told it exists.
    let not_theirs = ctx
        .app
        .call::<_, RevokeTokenResponse>(&format!("{TOKEN_SERVICE}/RevokeToken"), Some(&bob), &revoke)
        .await
        .unwrap_err();
    assert_eq!((not_theirs.code.as_str(), not_theirs.reason.as_str()), ("not_found", "TOKEN_NOT_FOUND"));
    assert!(whoami(&ctx, &caller).await.is_ok());

    ctx.app
        .call::<_, RevokeTokenResponse>(&format!("{TOKEN_SERVICE}/RevokeToken"), Some(&alice), &revoke)
        .await
        .unwrap();
    let refused = whoami(&ctx, &caller).await.unwrap_err();
    assert_eq!(
        (refused.status, refused.code.as_str(), refused.reason.as_str()),
        (401, "unauthenticated", "TOKEN_REVOKED")
    );
    assert!(list(&ctx, &alice).await.is_empty());
    assert_eq!(ctx.app.events_by("dev.token.revoked", alice.id).await.len(), 1);
    assert_eq!(ctx.security_events(alice.id, "API_TOKEN_REVOKED").await, 1);
    // Twice is not found.
    let again = ctx
        .app
        .call::<_, RevokeTokenResponse>(&format!("{TOKEN_SERVICE}/RevokeToken"), Some(&alice), &revoke)
        .await
        .unwrap_err();
    assert_eq!(again.reason, "TOKEN_NOT_FOUND");
}

#[tokio::test]
async fn malformed_unknown_and_expired_tokens_are_told_apart() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let refused = |secret: String| {
        let (ctx, alice) = (&ctx, &alice);
        async move { whoami(ctx, &with_token(alice, &secret)).await.unwrap_err().reason }
    };
    // Well formed but never issued; a typo; other kinds of secrets.
    assert_eq!(refused(secrets::generate(secrets::SecretKind::Personal)).await, "TOKEN_INVALID");
    assert_eq!(refused("tkl_pat_notatoken".to_owned()).await, "TOKEN_INVALID");
    assert_eq!(refused(secrets::generate(secrets::SecretKind::Refresh)).await, "TOKEN_INVALID");
    assert_eq!(refused(secrets::generate(secrets::SecretKind::ClientSecret)).await, "TOKEN_INVALID");

    let (token, secret) = personal_token(&ctx, &alice, &["identity:read"]).await;
    sqlx::query("UPDATE dev_tokens SET expires_at = now() - interval '1 second' WHERE id = $1")
        .bind(Uuid::parse_str(&token.id).unwrap())
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(refused(secret).await, "TOKEN_EXPIRED");
    assert!(list(&ctx, &alice).await[0].expired);
}

#[tokio::test]
async fn using_a_token_records_when_and_from_where() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let (_, secret) = personal_token(&ctx, &alice, &["identity:read"]).await;
    assert!(whoami(&ctx, &with_token(&alice, &secret)).await.is_ok());
    eventually("the token's last use is recorded", || async {
        let token = &list(&ctx, &alice).await[0];
        token.last_used_at.as_option().is_some() && token.last_used_ip.starts_with("10.")
    })
    .await;
}

#[tokio::test]
async fn a_password_change_revokes_the_tokens_made_before_it() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let (_, used) = personal_token(&ctx, &alice, &["identity:read"]).await;
    let (_, unused) = personal_token(&ctx, &alice, &["identity:read"]).await;
    assert!(whoami(&ctx, &with_token(&alice, &used)).await.is_ok());

    // Identity changes the password (it has no event for it yet): the next
    // look at the database notices.
    sqlx::query("UPDATE users SET password_hash = 'a-new-argon2-hash' WHERE id = $1")
        .bind(alice.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(whoami(&ctx, &with_token(&alice, &unused)).await.unwrap_err().reason, "TOKEN_REVOKED");
    // That revoked every token made before the change, cached or not.
    assert_eq!(whoami(&ctx, &with_token(&alice, &used)).await.unwrap_err().reason, "TOKEN_REVOKED");
    let reasons: Vec<String> = sqlx::query_scalar("SELECT revoked_reason FROM dev_tokens WHERE user_id = $1")
        .bind(alice.id)
        .fetch_all(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(reasons, ["password_changed", "password_changed"]);
    // A token made with the new password works.
    let (_, fresh) = personal_token(&ctx, &alice, &["identity:read"]).await;
    assert!(whoami(&ctx, &with_token(&alice, &fresh)).await.is_ok());

    // And the event, once identity emits it, does the same without waiting.
    ctx.dev
        .on_account_event(&event("identity.user.password_changed", serde_json::json!({ "user_id": alice.id })))
        .await
        .unwrap();
    assert_eq!(whoami(&ctx, &with_token(&alice, &fresh)).await.unwrap_err().reason, "TOKEN_REVOKED");
}

#[tokio::test]
async fn a_banned_account_loses_its_tokens_for_good() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let (_, secret) = personal_token(&ctx, &alice, &["identity:read"]).await;
    ctx.app.identity.set_banned(alice.id, Some("spam")).await.unwrap();
    // Refused from the database alone, before any event arrives.
    assert_eq!(whoami(&ctx, &with_token(&alice, &secret)).await.unwrap_err().reason, "TOKEN_REVOKED");

    ctx.dev.on_account_event(&event("identity.user.banned", serde_json::json!({ "user_id": alice.id }))).await.unwrap();
    ctx.app.identity.set_banned(alice.id, None).await.unwrap();
    let revoked: Vec<String> = sqlx::query_scalar("SELECT revoked_reason FROM dev_tokens WHERE user_id = $1")
        .bind(alice.id)
        .fetch_all(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(revoked, ["banned"], "lifting the ban does not bring the token back");
}

#[tokio::test]
async fn a_token_is_held_to_the_rate_of_its_tier() {
    let ctx = start_with(DevConfig { rate_standard: 3, ..config() }).await;
    let alice = ctx.app.user("alice").await;
    let (_, secret) = personal_token(&ctx, &alice, &["identity:read"]).await;
    let caller = with_token(&alice, &secret);
    for _ in 0..3 {
        assert!(whoami(&ctx, &caller).await.is_ok());
    }
    let limited = whoami(&ctx, &caller).await.unwrap_err();
    assert_eq!(
        (limited.status, limited.code.as_str(), limited.reason.as_str()),
        (429, "resource_exhausted", "RATE_LIMITED")
    );
    // The session of the same person is not the token.
    assert!(whoami(&ctx, &alice).await.is_ok());
}

#[tokio::test]
async fn usage_is_reported_per_token_by_day_and_method() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let (token, secret) = personal_token(&ctx, &alice, &["identity:read", "dev:read"]).await;
    let caller = with_token(&alice, &secret);
    for _ in 0..3 {
        ctx.app
            .call::<_, identity::GetMeResponse>(GET_ME, Some(&caller), &identity::GetMeRequest::default())
            .await
            .unwrap();
    }
    let update = identity::UpdateProfileRequest { bio: Some("no".into()), ..Default::default() };
    ctx.app.call::<_, identity::UpdateProfileResponse>(UPDATE_PROFILE, Some(&caller), &update).await.unwrap_err();

    let usage_of = |user: &tokuly_testkit::TestUser| {
        let (ctx, user, request) = (
            &ctx,
            user.clone(),
            GetUsageRequest {
                subject: Some(get_usage_request::Subject::TokenId(token.id.clone())),
                ..Default::default()
            },
        );
        async move {
            ctx.app.call::<_, GetUsageResponse>("tokuly.dev.v1.UsageService/GetUsage", Some(&user), &request).await
        }
    };
    // Usage is tracked after each response and written in batches.
    eventually("the calls reach the analytics store", || async {
        ctx.app.flush_analytics().await;
        usage_of(&alice).await.is_ok_and(|usage| usage.requests >= 4)
    })
    .await;
    let usage = usage_of(&alice).await.unwrap();
    assert_eq!((usage.requests, usage.errors), (4, 1));
    assert_eq!(usage.days.len(), 1);
    assert_eq!((usage.days[0].requests, usage.days[0].errors), (4, 1));
    let methods: Vec<(&str, i64, i64)> =
        usage.methods.iter().map(|m| (m.procedure.as_str(), m.requests, m.errors)).collect();
    assert_eq!(
        methods,
        [("tokuly.identity.v1.AccountService/GetMe", 3, 0), ("tokuly.identity.v1.AccountService/UpdateProfile", 1, 1)]
    );

    // The token itself may read its usage (dev:read); nobody else's token or session may.
    assert!(usage_of(&caller).await.is_ok());
    assert_eq!(usage_of(&bob).await.unwrap_err().reason, "TOKEN_NOT_FOUND");
    let nothing = ctx
        .app
        .call::<_, GetUsageResponse>("tokuly.dev.v1.UsageService/GetUsage", Some(&alice), &GetUsageRequest::default())
        .await
        .unwrap_err();
    assert_eq!(nothing.reason, "USAGE_SUBJECT_REQUIRED");
}
