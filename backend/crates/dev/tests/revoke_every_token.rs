//! `tokuly sessions revoke-all` (docs/configuration.md, a leaked secret):
//! every API and OAuth token stops working at once. A test binary of its
//! own: it revokes every token of the database.
#[allow(dead_code)]
#[path = "dev/common.rs"]
mod common;

use chrono::Utc;
use common::*;
use tokuly_proto::dev::{ApproveDeviceRequest, ApproveDeviceResponse};

const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const CONFIDENTIAL: tokuly_proto::dev::AppClientType = tokuly_proto::dev::AppClientType::APP_CLIENT_TYPE_CONFIDENTIAL;

async fn refresh(ctx: &Ctx, app: &OAuthApp, refresh_token: &str) -> HttpReply {
    ctx.form("/oauth/token", app.basic(), &[("grant_type", "refresh_token"), ("refresh_token", refresh_token)]).await
}

#[tokio::test]
async fn every_token_that_existed_stops_working_and_one_made_meanwhile_does_not() {
    let (ctx, elsewhere) = (start().await, start().await);
    let (alice, dana, bulk) = (ctx.app.user("alice").await, ctx.app.user("dana").await, ctx.app.user("bulk").await);
    let (_, personal) = personal_token(&ctx, &alice, &["identity:read"]).await;
    let app = create_app(&ctx, &dana, &["identity:read"], CONFIDENTIAL).await;
    let (access, refresh_token) = obtain_tokens(&ctx, &alice, &app, "identity:read").await;
    let (_, refreshed_meanwhile) = obtain_tokens(&ctx, &alice, &app, "identity:read").await;
    // Approved by alice's session before, collected only afterwards.
    let (code, verifier) = authorize(&ctx, &alice, &app, "identity:read").await;
    let cli = create_cli(&ctx, &dana, &["identity:read"]).await;
    let device = ctx.form("/oauth/device_authorization", None, &[("client_id", &cli.id)]).await.json();
    let device_code = device["device_code"].as_str().unwrap().to_owned();
    let _: ApproveDeviceResponse = ctx
        .app
        .call(
            &format!("{AUTHORIZATION_SERVICE}/ApproveDevice"),
            Some(&alice),
            &ApproveDeviceRequest { user_code: device["user_code"].as_str().unwrap().to_owned(), ..Default::default() },
        )
        .await
        .unwrap();
    // More than one statement's worth, so that the batches are walked through.
    sqlx::query(
        "INSERT INTO dev_tokens (kind, user_id, created_by, name, token_hash, hint, scopes)
         SELECT 'pat', $1, $1, 'bulk', 'bulk-' || gen_random_uuid(), 'tk_…', '{}' FROM generate_series(1, 1100)",
    )
    .bind(bulk.id)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
    for secret in [&personal, &access] {
        assert!(whoami(&elsewhere, &with_token(&alice, secret)).await.is_ok(), "the answer is cached");
    }

    let started = Utc::now();
    // What the command does first (`Identity::sign_everyone_out`).
    ctx.app.core.revoke_sessions_made_before(started).await.unwrap();
    // Who signs in while it runs makes a token, and keeps it. (Alice's old
    // session can make none any more.)
    let carol = ctx.app.user("carol").await;
    let (_, meanwhile) = personal_token(&ctx, &carol, &["identity:read"]).await;
    // An application (or whoever holds its refresh token) renewing while it
    // runs: the new pair is made after the start, from one made before.
    let renewed = refresh(&ctx, &app, &refreshed_meanwhile).await.json();
    let (renewed_access, renewed_refresh) =
        (renewed["access_token"].as_str().unwrap().to_owned(), renewed["refresh_token"].as_str().unwrap().to_owned());
    let revoked = ctx.dev.revoke_every_token(started).await.unwrap();
    assert!(revoked.api_tokens >= 1101 && revoked.oauth_tokens >= 1, "{revoked:?}");

    for secret in [&personal, &access, &renewed_access] {
        assert_eq!(whoami(&elsewhere, &with_token(&alice, secret)).await.unwrap_err().reason, "TOKEN_REVOKED");
    }
    for refresh_token in [&refresh_token, &renewed_refresh] {
        assert_eq!(refresh(&ctx, &app, refresh_token).await.error(), "invalid_grant", "the application asks its user again");
    }
    assert_eq!(exchange(&ctx, &app, &code, &verifier).await.error(), "invalid_grant", "a code alice's old session got");
    let collected = ctx
        .form(
            "/oauth/token",
            None,
            &[("grant_type", DEVICE_GRANT), ("device_code", &device_code), ("client_id", &cli.id)],
        )
        .await;
    assert_eq!(collected.error(), "expired_token", "a device alice's old session let in");
    assert!(whoami(&elsewhere, &with_token(&carol, &meanwhile)).await.is_ok(), "made while it ran");
    let told = ctx.app.events("dev.token.revoked").await;
    assert!(
        told.iter().any(|event| event.data["user_id"] == alice.id.to_string() && event.data["reason"] == "operator")
    );
}
