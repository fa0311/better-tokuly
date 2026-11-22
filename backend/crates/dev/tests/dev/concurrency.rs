//! Requests that arrive together. Limits are counted under a lock,
//! single-use secrets are taken by one atomic step, and attempts at a
//! webhook settle one after the other — these tests really run the requests
//! at the same time, more of them than the database pool has connections.
use tokuly_dev::DevConfig;
use tokuly_proto::dev::*;
use uuid::Uuid;

use crate::common::*;

const CONFIDENTIAL: AppClientType = AppClientType::APP_CLIENT_TYPE_CONFIDENTIAL;
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// How many of the calls succeeded, and the reasons of those that did not.
fn tally<T>(results: Vec<Result<T, tokuly_testkit::RpcError>>) -> (usize, Vec<String>) {
    let mut reasons: Vec<String> = results.iter().filter_map(|r| r.as_ref().err().map(|e| e.reason.clone())).collect();
    reasons.sort();
    reasons.dedup();
    (results.iter().filter(|r| r.is_ok()).count(), reasons)
}

/// Token pairs ever issued to `user` for `app`, and how many of them still work.
async fn token_pairs(ctx: &Ctx, app: &OAuthApp, user: &tokuly_testkit::TestUser) -> (i64, i64) {
    sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE o.revoked_at IS NULL AND o.rotated_at IS NULL)
         FROM dev_oauth_tokens o JOIN dev_authorizations a ON a.id = o.authorization_id WHERE a.app_id = $1 AND a.user_id = $2",
    )
    .bind(Uuid::parse_str(&app.id).unwrap())
    .bind(user.id)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap()
}

#[tokio::test]
async fn limits_hold_when_one_account_floods_and_nobody_else_waits() {
    let ctx = start_with(DevConfig { max_tokens: 2, max_apps: 2, max_bots: 2, max_webhooks: 2, ..config() }).await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    // More at once than the pool has connections, each taking the same lock.
    let burst = POOL_SIZE + 3;
    let (ctx, alice) = (&ctx, &alice);

    let tokens = at_once(burst, |i| async move {
        let request =
            CreateTokenRequest { name: format!("token {i}"), scopes: vec!["notes:read".into()], ..Default::default() };
        ctx.app.call::<_, CreateTokenResponse>(&format!("{TOKEN_SERVICE}/CreateToken"), Some(alice), &request).await
    });
    let apps = at_once(burst, |i| async move {
        let request = app_request(&format!("App {i}"), &["notes:read"], CONFIDENTIAL);
        ctx.app.call::<_, CreateAppResponse>(&format!("{APP_SERVICE}/CreateApp"), Some(alice), &request).await
    });
    let bots = at_once(burst, |i| async move {
        let handle = format!("flood{i}_{}", tokuly_core::ids::random_base62(6).to_lowercase());
        let request = CreateBotRequest { handle, display_name: format!("Bot {i}"), ..Default::default() };
        ctx.app.call::<_, CreateBotResponse>(&format!("{BOT_SERVICE}/CreateBot"), Some(alice), &request).await
    });
    let webhooks = at_once(burst, |i| async move {
        let request = CreateWebhookRequest {
            url: format!("http://127.0.0.1:9/hook/{i}"),
            event_types: vec!["friend.added".into()],
            ..Default::default()
        };
        ctx.app
            .call::<_, CreateWebhookResponse>(&format!("{WEBHOOK_SERVICE}/CreateWebhook"), Some(alice), &request)
            .await
    });
    // Meanwhile somebody else uses the API and is answered.
    let bystander = async {
        for _ in 0..10 {
            ctx.app
                .call::<_, ListTokensResponse>(
                    &format!("{TOKEN_SERVICE}/ListTokens"),
                    Some(&bob),
                    &ListTokensRequest::default(),
                )
                .await
                .expect("others are served");
        }
    };
    let (tokens, apps, bots, webhooks, ()) = tokio::join!(tokens, apps, bots, webhooks, bystander);

    assert_eq!(tally(tokens), (2, vec!["TOO_MANY_TOKENS".to_owned()]));
    assert_eq!(tally(apps), (2, vec!["TOO_MANY_APPS".to_owned()]));
    assert_eq!(tally(bots), (2, vec!["TOO_MANY_BOTS".to_owned()]));
    assert_eq!(tally(webhooks), (2, vec!["TOO_MANY_WEBHOOKS".to_owned()]));
    let stored: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM dev_tokens WHERE user_id = $1), (SELECT count(*) FROM dev_apps WHERE owner_id = $1),
                (SELECT count(*) FROM dev_bots WHERE owner_id = $1), (SELECT count(*) FROM dev_webhooks WHERE account_id = $1)",
    )
    .bind(alice.id)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(stored, (2, 2, 2, 2));
}

#[tokio::test]
async fn an_authorization_code_presented_twice_at_once_buys_one_pair_and_loses_it() {
    let ctx = start().await;
    let (dana, user) = (ctx.app.user("dana").await, ctx.app.user("user").await);
    let app = create_app(&ctx, &dana, &["identity:read"], CONFIDENTIAL).await;
    let (code, verifier) = authorize(&ctx, &user, &app, "identity:read").await;

    let replies = at_once(POOL_SIZE + 3, |_| exchange(&ctx, &app, &code, &verifier)).await;
    let winners: Vec<&HttpReply> = replies.iter().filter(|reply| reply.status == 200).collect();
    assert!(winners.len() <= 1, "{} requests were given tokens for one code", winners.len());
    assert!(
        replies
            .iter()
            .filter(|reply| reply.status != 200)
            .all(|reply| reply.status == 400 && reply.error() == "invalid_grant")
    );
    // Exactly one of them took the code; a code seen twice was intercepted, so what it bought is revoked.
    assert_eq!(token_pairs(&ctx, &app, &user).await, (1, 0));
    if let Some(winner) = winners.first() {
        let access = winner.json()["access_token"].as_str().unwrap().to_owned();
        assert_eq!(whoami(&ctx, &with_token(&user, &access)).await.unwrap_err().reason, "TOKEN_REVOKED");
    }
}

#[tokio::test]
async fn an_approved_device_code_is_collected_by_exactly_one_poll() {
    let ctx = start_with(DevConfig { device_poll_seconds: 0, ..config() }).await;
    let (dana, user) = (ctx.app.user("dana").await, ctx.app.user("user").await);
    let cli = create_cli(&ctx, &dana, &["identity:read"]).await;
    let device = ctx.form("/oauth/device_authorization", None, &[("client_id", &cli.id)]).await.json();
    let (device_code, user_code) =
        (device["device_code"].as_str().unwrap().to_owned(), device["user_code"].as_str().unwrap().to_owned());

    // The user answers from two tabs at once, yes in some and no in others: one answer stands.
    let answers = at_once(6, |i| {
        let (ctx, user, user_code) = (&ctx, &user, user_code.clone());
        async move {
            if i % 2 == 0 {
                ctx.app
                    .call::<_, ApproveDeviceResponse>(
                        &format!("{AUTHORIZATION_SERVICE}/ApproveDevice"),
                        Some(user),
                        &ApproveDeviceRequest { user_code, ..Default::default() },
                    )
                    .await
                    .map(|_| true)
            } else {
                ctx.app
                    .call::<_, DenyDeviceResponse>(
                        &format!("{AUTHORIZATION_SERVICE}/DenyDevice"),
                        Some(user),
                        &DenyDeviceRequest { user_code, ..Default::default() },
                    )
                    .await
                    .map(|_| false)
            }
        }
    })
    .await;
    let accepted: Vec<bool> = answers.iter().filter_map(|answer| answer.as_ref().ok().copied()).collect();
    assert_eq!(accepted.len(), 1, "exactly one answer is taken");
    assert!(answers.iter().filter_map(|answer| answer.as_ref().err()).all(|e| e.reason == "DEVICE_CODE_INVALID"));

    let polls = at_once(POOL_SIZE + 3, |_| async {
        ctx.form(
            "/oauth/token",
            None,
            &[("grant_type", DEVICE_GRANT), ("device_code", &device_code), ("client_id", &cli.id)],
        )
        .await
    })
    .await;
    if accepted[0] {
        let collected: Vec<&HttpReply> = polls.iter().filter(|reply| reply.status == 200).collect();
        assert_eq!(collected.len(), 1, "one poll collects the tokens");
        assert!(polls.iter().filter(|reply| reply.status != 200).all(|reply| reply.error() == "expired_token"));
        assert_eq!(token_pairs(&ctx, &cli, &user).await, (1, 1));
        assert!(whoami(&ctx, &with_token(&user, collected[0].json()["access_token"].as_str().unwrap())).await.is_ok());
    } else {
        assert_eq!(
            polls.iter().filter(|reply| reply.error() == "access_denied").count(),
            1,
            "one poll hears the refusal"
        );
        assert!(polls.iter().all(|reply| reply.status == 400));
        assert_eq!(token_pairs(&ctx, &cli, &user).await, (0, 0));
    }
}

#[tokio::test]
async fn a_refresh_token_used_twice_at_once_rotates_once_and_then_the_family_is_revoked() {
    let ctx = start().await;
    let (dana, user) = (ctx.app.user("dana").await, ctx.app.user("user").await);
    let app = create_app(&ctx, &dana, &["identity:read"], CONFIDENTIAL).await;
    let (first_access, refresh_token) = obtain_tokens(&ctx, &user, &app, "identity:read").await;

    let replies = at_once(POOL_SIZE + 3, |_| async {
        ctx.form("/oauth/token", app.basic(), &[("grant_type", "refresh_token"), ("refresh_token", &refresh_token)])
            .await
    })
    .await;
    let rotated: Vec<&HttpReply> = replies.iter().filter(|reply| reply.status == 200).collect();
    assert_eq!(rotated.len(), 1, "one request rotates the token");
    assert!(
        replies
            .iter()
            .filter(|reply| reply.status != 200)
            .all(|reply| reply.status == 400 && reply.error() == "invalid_grant")
    );
    // The others presented a token that had just been retired: that is reuse, and the family dies —
    // the pair the winner was handed included.
    assert_eq!(token_pairs(&ctx, &app, &user).await, (2, 0));
    let new_access = rotated[0].json()["access_token"].as_str().unwrap().to_owned();
    for access in [&first_access, &new_access] {
        assert_eq!(whoami(&ctx, &with_token(&user, access)).await.unwrap_err().reason, "TOKEN_REVOKED");
    }
    let again = ctx
        .form(
            "/oauth/token",
            app.basic(),
            &[("grant_type", "refresh_token"), ("refresh_token", rotated[0].json()["refresh_token"].as_str().unwrap())],
        )
        .await;
    assert_eq!(again.error(), "invalid_grant");
}

#[tokio::test]
async fn consent_given_many_times_at_once_is_one_authorization() {
    let ctx = start().await;
    let (dana, user) = (ctx.app.user("dana").await, ctx.app.user("user").await);
    let app = create_app(&ctx, &dana, &["identity:read"], CONFIDENTIAL).await;
    let codes = at_once(POOL_SIZE + 3, |_| authorize(&ctx, &user, &app, "identity:read")).await;
    assert_eq!(codes.len(), POOL_SIZE + 3);
    let authorizations: i64 = sqlx::query_scalar("SELECT count(*) FROM dev_authorizations WHERE user_id = $1")
        .bind(user.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(authorizations, 1);
    assert_eq!(ctx.notifications(user.id, "dev.app_authorized").await.len(), 1, "the user is told once");
}

#[tokio::test]
async fn one_event_and_one_attempt_however_often_they_are_handled_at_once() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let receiver = Receiver::start().await;
    let (webhook, _) = create_webhook(&ctx, &alice, &receiver.url, &["friend.request_received"]).await;
    // The bus delivers the same event to several workers.
    let request = event(
        "social.friend.requested",
        serde_json::json!({ "request_id": Uuid::now_v7(), "from_user_id": bob.id, "to_user_id": alice.id }),
    );
    let stored = at_once(POOL_SIZE + 3, |_| ctx.dev.fan_out(&request)).await;
    assert_eq!(stored.into_iter().map(|n| n.unwrap()).sum::<usize>(), 1);
    let ids = delivery_ids(&ctx, &webhook.id).await;
    assert_eq!(ids.len(), 1);
    // The job for it runs more than once.
    let attempts = at_once(POOL_SIZE + 3, |_| ctx.dev.deliver(ids[0])).await;
    assert_eq!(attempts.into_iter().filter(|attempt| attempt.as_ref().unwrap().is_some()).count(), 1);
    assert_eq!(receiver.hits().len(), 1, "the endpoint was called once");
}

#[tokio::test]
async fn deliveries_failing_at_once_disable_the_webhook_exactly_once() {
    let ctx =
        start_with(DevConfig { webhook_disable_after_failures: 2, webhook_disable_after_hours: 0, ..config() }).await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let receiver = Receiver::start().await;
    let (webhook, _) = create_webhook(&ctx, &alice, &receiver.url, &["friend.request_received"]).await;
    receiver.answer_with(500);
    let count = POOL_SIZE + 3;
    for _ in 0..count {
        let request = event(
            "social.friend.requested",
            serde_json::json!({ "request_id": Uuid::now_v7(), "from_user_id": bob.id, "to_user_id": alice.id }),
        );
        assert_eq!(ctx.dev.fan_out(&request).await.unwrap(), 1);
    }
    let ids = delivery_ids(&ctx, &webhook.id).await;
    // Every attempt is under way while one of them crosses the threshold.
    let settled = at_once(count, |i| ctx.dev.deliver(ids[i])).await;
    assert!(settled.iter().all(|result| result.is_ok()));

    let state: (String, i32) = sqlx::query_as("SELECT status, consecutive_failures FROM dev_webhooks WHERE id = $1")
        .bind(Uuid::parse_str(&webhook.id).unwrap())
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(state.0, "disabled");
    // Told once, however many attempts found the webhook failing.
    assert_eq!(ctx.notifications(alice.id, "dev.webhook_disabled").await.len(), 1);
    assert_eq!(
        ctx.app.events("dev.webhook.disabled").await.iter().filter(|e| e.data["webhook_id"] == webhook.id).count(),
        1
    );
    // Nothing is left to retry against a disabled endpoint: not what was waiting, not what was in flight.
    let unsettled: i64 = sqlx::query_scalar("SELECT count(*) FROM dev_webhook_deliveries WHERE webhook_id = $1 AND (status <> 'failed' OR next_attempt_at IS NOT NULL OR claimed_at IS NOT NULL)")
        .bind(Uuid::parse_str(&webhook.id).unwrap())
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(unsettled, 0);
    assert_eq!(ctx.dev.queue_due_deliveries().await.unwrap(), 0);
}
