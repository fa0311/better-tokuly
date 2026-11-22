//! What must not be possible: learning something from an error, getting a
//! late event to revoke the wrong thing, repeating a costly request without
//! limit, and making the person behind a bot or an application untraceable.
use tokuly_dev::{DevConfig, secrets};
use tokuly_proto::dev::*;
use uuid::Uuid;

use crate::common::*;

const CONFIDENTIAL: AppClientType = AppClientType::APP_CLIENT_TYPE_CONFIDENTIAL;
const PUBLIC: AppClientType = AppClientType::APP_CLIENT_TYPE_PUBLIC;

type User = tokuly_testkit::TestUser;

fn stale(topic: &str, user: &User, age: chrono::Duration) -> tokuly_core::events::Event {
    let mut event = event(topic, serde_json::json!({ "user_id": user.id }));
    event.occurred_at = chrono::Utc::now() - age;
    event
}

async fn create_bot(ctx: &Ctx, owner: &User, name: &str) -> Result<String, String> {
    let handle = format!("{}_{}", name.to_lowercase(), tokuly_core::ids::random_base62(6).to_lowercase());
    let request = CreateBotRequest { handle, display_name: name.into(), ..Default::default() };
    let created = ctx
        .app
        .call::<_, CreateBotResponse>(&format!("{BOT_SERVICE}/CreateBot"), Some(owner), &request)
        .await
        .map_err(|e| e.reason)?;
    Ok(created.bot.as_option().unwrap().user.as_option().unwrap().id.clone())
}

#[tokio::test]
async fn a_client_learns_nothing_from_how_its_authentication_fails() {
    let ctx = start().await;
    let (dana, erin, user) = (ctx.app.user("dana").await, ctx.app.user("erin").await, ctx.app.user("user").await);
    let app = create_app(&ctx, &dana, &["identity:read"], CONFIDENTIAL).await;
    let native = create_app(&ctx, &dana, &["identity:read"], PUBLIC).await;
    let disabled = create_app(&ctx, &dana, &["identity:read"], CONFIDENTIAL).await;
    let orphaned = create_app(&ctx, &erin, &["identity:read"], CONFIDENTIAL).await;
    let removed = serde_json::json!({ "subject_type": tokuly_proto::common::SubjectType::SUBJECT_TYPE_APP as i32, "subject_id": disabled.id, "reason": "abuse" });
    ctx.dev.on_account_event(&event("admin.content.removed", &removed)).await.unwrap();
    ctx.app.identity.set_banned(erin.id, Some("spam")).await.unwrap();

    let other_secret = secrets::generate(secrets::SecretKind::ClientSecret);
    let unknown = Uuid::now_v7().to_string();
    let attempts: [(&str, &str); 6] = [
        // A client that does not exist; one that does, with the wrong secret, or with no secret.
        (&unknown, &other_secret),
        (&app.id, &other_secret),
        (&app.id, "not even a secret"),
        // A public client that sends a secret; a disabled one and a suspended developer's, with the right secret.
        (&native.id, &other_secret),
        (&disabled.id, &disabled.secret),
        (&orphaned.id, &orphaned.secret),
    ];
    let mut answers = Vec::new();
    for (client_id, secret) in attempts {
        for path in ["/oauth/token", "/oauth/revoke", "/oauth/device_authorization"] {
            let reply = ctx
                .form(
                    path,
                    Some((client_id, secret)),
                    &[("grant_type", "refresh_token"), ("refresh_token", "x"), ("token", "x")],
                )
                .await;
            answers.push((reply.status, reply.json(), reply.header("www-authenticate").to_owned()));
        }
    }
    assert_eq!(answers[0].0, 401);
    assert!(answers.iter().all(|answer| answer == &answers[0]), "{answers:?}");

    // The consent screen answers alike for a client that never existed, one that was disabled, and a suspended developer's.
    let (_, challenge) = pkce();
    let mut refusals = Vec::new();
    for client in [&unknown, &disabled.id, &orphaned.id] {
        let request = AuthorizationRequest {
            client_id: client.to_string(),
            ..authorization_request(&app, "identity:read", &challenge)
        };
        let refused = ctx
            .app
            .call::<_, GetConsentResponse>(
                &format!("{AUTHORIZATION_SERVICE}/GetConsent"),
                Some(&user),
                &GetConsentRequest { request: request.into(), ..Default::default() },
            )
            .await
            .unwrap_err();
        refusals.push((refused.status, refused.code, refused.reason, refused.message));
        let card = ctx
            .app
            .call::<_, GetPublicAppResponse>(
                &format!("{AUTHORIZATION_SERVICE}/GetPublicApp"),
                None,
                &GetPublicAppRequest { app_id: client.to_string(), ..Default::default() },
            )
            .await
            .unwrap_err();
        assert_eq!(card.reason, "APP_NOT_FOUND");
    }
    assert_eq!(refusals[0].2, "CLIENT_NOT_FOUND");
    assert!(refusals.iter().all(|refusal| refusal == &refusals[0]), "{refusals:?}");
}

#[tokio::test]
async fn a_suspended_developers_application_stops_until_the_suspension_ends() {
    let ctx = start().await;
    let (dana, user) = (ctx.app.user("dana").await, ctx.app.user("user").await);
    let app = create_app(&ctx, &dana, &["identity:read"], CONFIDENTIAL).await;
    let (access, refresh_token) = obtain_tokens(&ctx, &user, &app, "identity:read").await;
    let caller = with_token(&user, &access);
    assert!(whoami(&ctx, &caller).await.is_ok());

    ctx.app.identity.set_banned(dana.id, Some("abuse")).await.unwrap();
    ctx.dev.on_account_event(&event("identity.user.banned", serde_json::json!({ "user_id": dana.id }))).await.unwrap();
    // At once, although the token was in use a moment ago.
    assert_eq!(whoami(&ctx, &caller).await.unwrap_err().reason, "TOKEN_REVOKED");
    let refreshed = ctx
        .form("/oauth/token", app.basic(), &[("grant_type", "refresh_token"), ("refresh_token", &refresh_token)])
        .await;
    assert_eq!(refreshed.error(), "invalid_client");

    // The users' authorizations were not touched: when the suspension ends, the application works again.
    ctx.app.identity.set_banned(dana.id, None).await.unwrap();
    ctx.dev
        .on_account_event(&event("identity.user.unbanned", serde_json::json!({ "user_id": dana.id })))
        .await
        .unwrap();
    assert!(whoami(&ctx, &caller).await.is_ok());
}

#[tokio::test]
async fn an_event_that_arrives_late_revokes_only_what_existed_when_it_happened() {
    let ctx = start().await;
    let (alice, dana) = (ctx.app.user("alice").await, ctx.app.user("dana").await);
    let app = create_app(&ctx, &dana, &["identity:read"], CONFIDENTIAL).await;
    let (_, pat) = personal_token(&ctx, &alice, &["identity:read"]).await;
    let (access, _) = obtain_tokens(&ctx, &alice, &app, "identity:read").await;
    let alive = |secret: &str| {
        let (ctx, caller) = (&ctx, with_token(&alice, secret));
        async move { whoami(ctx, &caller).await.is_ok() }
    };

    // A suspension and a password change that both happened an hour ago —
    // before these credentials existed — reach this domain only now.
    let hour = chrono::Duration::hours(1);
    ctx.dev.on_account_event(&stale("identity.user.banned", &alice, hour)).await.unwrap();
    ctx.dev.on_account_event(&stale("identity.user.password_changed", &alice, hour)).await.unwrap();
    assert!(alive(&pat).await && alive(&access).await, "credentials made after the fact are not its business");

    // The same events, for something that happened just now.
    ctx.dev.on_account_event(&stale("identity.user.password_changed", &alice, chrono::Duration::zero())).await.unwrap();
    assert!(
        !alive(&pat).await && alive(&access).await,
        "a password change ends personal tokens, not what applications were granted"
    );
    ctx.dev.on_account_event(&stale("identity.user.banned", &alice, chrono::Duration::zero())).await.unwrap();
    assert!(!alive(&access).await);
    // Twice is the same as once.
    ctx.dev.on_account_event(&stale("identity.user.banned", &alice, chrono::Duration::zero())).await.unwrap();
    assert_eq!(
        ctx.app
            .events("dev.authorization.revoked")
            .await
            .iter()
            .filter(|e| e.data["user_id"] == alice.id.to_string())
            .count(),
        1
    );
}

#[tokio::test]
async fn a_token_made_after_a_password_change_outlives_the_ones_made_before() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let (_, before) = personal_token(&ctx, &alice, &["identity:read"]).await;
    sqlx::query("UPDATE users SET password_hash = 'the-new-argon2-hash' WHERE id = $1")
        .bind(alice.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let (_, after) = personal_token(&ctx, &alice, &["identity:read"]).await;
    assert!(whoami(&ctx, &with_token(&alice, &after)).await.is_ok());
    // The old token trips the check: it goes, the new one stays.
    assert_eq!(whoami(&ctx, &with_token(&alice, &before)).await.unwrap_err().reason, "TOKEN_REVOKED");
    assert!(whoami(&ctx, &with_token(&alice, &after)).await.is_ok());
}

#[tokio::test]
async fn a_verification_racing_a_revocation_cannot_bring_the_token_back() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let (token, secret) = personal_token(&ctx, &alice, &["identity:read"]).await;
    let caller = with_token(&alice, &secret);
    assert!(whoami(&ctx, &caller).await.is_ok());
    // What a request holds that read the database just before the revocation...
    let key = format!("dev:tok:{}", ctx.app.core.keys.token_hash(&secret));
    let valid: String = ctx.app.core.cache.get_string(&key).await.unwrap().expect("the answer is cached");
    ctx.app
        .call::<_, RevokeTokenResponse>(
            &format!("{TOKEN_SERVICE}/RevokeToken"),
            Some(&alice),
            &RevokeTokenRequest { token_id: token.id, ..Default::default() },
        )
        .await
        .unwrap();
    // ...and writes to the cache just after it.
    ctx.app.core.cache.set_string(&key, &valid, Some(std::time::Duration::from_secs(60))).await.unwrap();
    for _ in 0..3 {
        assert_eq!(whoami(&ctx, &caller).await.unwrap_err().reason, "TOKEN_REVOKED");
    }
}

#[tokio::test]
async fn a_read_scope_does_not_reach_actions_with_a_reading_name() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let (_, reader) = personal_token(&ctx, &alice, &["geo:read", "media:read"]).await;
    let (_, writer) = personal_token(&ctx, &alice, &["geo:write", "media:write"]).await;
    // These domains are not part of this test app: the middleware decides before any routing.
    for procedure in [
        "tokuly.geo.v1.PlaceService/CheckIn",
        "tokuly.geo.v1.SafetyService/ResolveSos",
        "tokuly.media.v1.MediaService/GetUploadParts",
    ] {
        let refused = ctx
            .app
            .call::<_, serde_json::Value>(procedure, Some(&with_token(&alice, &reader)), &serde_json::json!({}))
            .await
            .unwrap_err();
        assert_eq!((refused.status, refused.reason.as_str()), (403, "INSUFFICIENT_SCOPE"), "{procedure}");
        let passed = ctx
            .app
            .call::<_, serde_json::Value>(procedure, Some(&with_token(&alice, &writer)), &serde_json::json!({}))
            .await
            .unwrap_err();
        assert_ne!(passed.reason, "INSUFFICIENT_SCOPE", "{procedure} is open to the write scope");
    }
    // What does only read stays readable.
    let read = ctx
        .app
        .call::<_, serde_json::Value>(
            "tokuly.geo.v1.LocationService/ListFriendLocations",
            Some(&with_token(&alice, &reader)),
            &serde_json::json!({}),
        )
        .await
        .unwrap_err();
    assert_ne!(read.reason, "INSUFFICIENT_SCOPE");
}

#[tokio::test]
async fn who_owned_a_deleted_bot_stays_on_record_for_a_while() {
    let ctx = start().await;
    let owen = ctx.app.user("owen").await;
    let bot_id = create_bot(&ctx, &owen, "Pest").await.unwrap();
    let bot = Uuid::parse_str(&bot_id).unwrap();
    assert_eq!(ctx.dev.bot_owner(bot).await.unwrap(), Some(owen.id));

    ctx.app
        .call::<_, DeleteBotResponse>(
            &format!("{BOT_SERVICE}/DeleteBot"),
            Some(&owen),
            &DeleteBotRequest { bot_id: bot_id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    // The bot is gone for its owner and as an account, but a report about what it did still leads somewhere.
    let listed: ListBotsResponse =
        ctx.app.call(&format!("{BOT_SERVICE}/ListBots"), Some(&owen), &ListBotsRequest::default()).await.unwrap();
    assert!(listed.bots.is_empty());
    assert_eq!(ctx.dev.bot_owner(bot).await.unwrap(), Some(owen.id));
    // The account's deletion event, which follows, does not erase the record either.
    ctx.dev.on_account_event(&event("identity.user.deleted", serde_json::json!({ "user_id": bot }))).await.unwrap();
    assert_eq!(ctx.dev.bot_owner(bot).await.unwrap(), Some(owen.id));

    // The record is kept for the retention, counted from the deletion — not from when the bot was made.
    sqlx::query("UPDATE dev_bots SET created_at = now() - interval '2 years' WHERE user_id = $1")
        .bind(bot)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.dev.purge_bot_records().await.unwrap();
    assert_eq!(ctx.dev.bot_owner(bot).await.unwrap(), Some(owen.id));
    sqlx::query("UPDATE dev_bots SET deleted_at = now() - interval '181 days' WHERE user_id = $1")
        .bind(bot)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.dev.purge_bot_records().await.unwrap();
    assert_eq!(ctx.dev.bot_owner(bot).await.unwrap(), None);
}

#[tokio::test]
async fn a_suspended_bot_cannot_be_deleted_away() {
    let ctx = start_with(DevConfig { max_bots: 1, ..config() }).await;
    let owen = ctx.app.user("owen").await;
    let bot_id = create_bot(&ctx, &owen, "Pest").await.unwrap();
    // Moderation suspends the bot's account.
    ctx.app.identity.set_banned(Uuid::parse_str(&bot_id).unwrap(), Some("harassment")).await.unwrap();

    let got: GetBotResponse = ctx
        .app
        .call(
            &format!("{BOT_SERVICE}/GetBot"),
            Some(&owen),
            &GetBotRequest { bot_id: bot_id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(got.bot.as_option().unwrap().suspended);
    let refused = ctx
        .app
        .call::<_, DeleteBotResponse>(
            &format!("{BOT_SERVICE}/DeleteBot"),
            Some(&owen),
            &DeleteBotRequest { bot_id: bot_id.clone(), ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!((refused.code.as_str(), refused.reason.as_str()), ("failed_precondition", "BOT_SUSPENDED"));
    // It keeps its place in the owner's allowance, so a replacement cannot be made either.
    assert_eq!(create_bot(&ctx, &owen, "Replacement").await.unwrap_err(), "TOO_MANY_BOTS");
    // Once the suspension is lifted the owner may delete it.
    ctx.app.identity.set_banned(Uuid::parse_str(&bot_id).unwrap(), None).await.unwrap();
    ctx.app
        .call::<_, DeleteBotResponse>(
            &format!("{BOT_SERVICE}/DeleteBot"),
            Some(&owen),
            &DeleteBotRequest { bot_id, ..Default::default() },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn consent_and_device_requests_have_a_budget() {
    let ctx = start().await;
    let (dana, user) = (ctx.app.user("dana").await, ctx.app.user("user").await);
    let app = create_app(&ctx, &dana, &["identity:read"], CONFIDENTIAL).await;
    let cli = create_app(&ctx, &dana, &["identity:read"], PUBLIC).await;
    let (_, challenge) = pkce();
    let approve = ApproveAuthorizationRequest {
        request: authorization_request(&app, "identity:read", &challenge).into(),
        ..Default::default()
    };
    let call = || async {
        ctx.app
            .call::<_, ApproveAuthorizationResponse>(
                &format!("{AUTHORIZATION_SERVICE}/ApproveAuthorization"),
                Some(&user),
                &approve,
            )
            .await
    };
    for _ in 0..30 {
        call().await.unwrap();
    }
    let limited = call().await.unwrap_err();
    assert_eq!(
        (limited.status, limited.code.as_str(), limited.reason.as_str()),
        (429, "resource_exhausted", "RATE_LIMITED")
    );

    // Starting device flows, per address.
    for _ in 0..20 {
        assert_eq!(ctx.form("/oauth/device_authorization", None, &[("client_id", &cli.id)]).await.status, 200);
    }
    let limited = ctx.form("/oauth/device_authorization", None, &[("client_id", &cli.id)]).await;
    assert_eq!((limited.status, limited.error().as_str()), (429, "slow_down"));
}

#[tokio::test]
async fn pings_usage_and_the_reference_have_a_budget() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let receiver = Receiver::start().await;
    // Pings are counted per person, across their webhooks.
    let (first, _) = create_webhook(&ctx, &alice, &receiver.url, &["friend.added"]).await;
    let (second, _) = create_webhook(&ctx, &alice, &receiver.url, &["friend.added"]).await;
    let ping = |webhook: &Webhook| {
        let (ctx, alice, request) =
            (&ctx, &alice, PingWebhookRequest { webhook_id: webhook.id.clone(), ..Default::default() });
        async move {
            ctx.app
                .call::<_, PingWebhookResponse>(&format!("{WEBHOOK_SERVICE}/PingWebhook"), Some(alice), &request)
                .await
        }
    };
    for i in 0..20 {
        ping(if i % 2 == 0 { &first } else { &second }).await.unwrap();
    }
    assert_eq!(ping(&first).await.unwrap_err().reason, "RATE_LIMITED");
    assert_eq!(receiver.hits().len(), 20);

    // Usage: each call aggregates over the analytics store.
    let (token, _) = personal_token(&ctx, &alice, &["identity:read"]).await;
    let usage = GetUsageRequest { subject: Some(get_usage_request::Subject::TokenId(token.id)), ..Default::default() };
    let mut answers = Vec::new();
    for _ in 0..61 {
        answers.push(
            ctx.app
                .call::<_, GetUsageResponse>("tokuly.dev.v1.UsageService/GetUsage", Some(&alice), &usage)
                .await
                .map(|_| ())
                .map_err(|e| e.reason),
        );
    }
    assert!(answers[..60].iter().all(|answer| answer.is_ok()));
    assert_eq!(answers[60], Err("RATE_LIMITED".to_owned()));

    // The reference is open to anyone, so it is counted per address.
    let mut answers = Vec::new();
    for _ in 0..121 {
        let request = GetMethodRequest { path: "/tokuly.dev.v1.TokenService/CreateToken".into(), ..Default::default() };
        answers.push(
            ctx.app
                .call::<_, GetMethodResponse>("tokuly.dev.v1.ReferenceService/GetMethod", None, &request)
                .await
                .map(|_| ())
                .map_err(|e| e.reason),
        );
    }
    assert!(answers[..120].iter().all(|answer| answer.is_ok()));
    assert_eq!(answers[120], Err("RATE_LIMITED".to_owned()));
    // So is the descriptor download.
    for _ in 0..30 {
        assert_eq!(ctx.get("/reference/descriptor.binpb").await.status, 200);
    }
    assert_eq!(ctx.get("/reference/descriptor.binpb").await.status, 429);
}

#[tokio::test]
async fn one_persons_webhooks_cannot_flood_a_host() {
    let ctx = start_with(DevConfig { webhook_rate_per_minute: 3, ..config() }).await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let receiver = Receiver::start().await;
    // Two webhooks of Alice's and one of her bot's, all aimed at the same host.
    create_webhook(&ctx, &alice, &receiver.url, &["friend.request_received"]).await;
    create_webhook(&ctx, &alice, &format!("{}/other", receiver.url), &["friend.request_received"]).await;
    let bot_id = create_bot(&ctx, &alice, "Echo").await.unwrap();
    let for_bot = CreateWebhookRequest {
        url: receiver.url.clone(),
        event_types: vec!["friend.request_received".into()],
        bot_id: bot_id.clone(),
        ..Default::default()
    };
    ctx.app
        .call::<_, CreateWebhookResponse>(&format!("{WEBHOOK_SERVICE}/CreateWebhook"), Some(&alice), &for_bot)
        .await
        .unwrap();
    let request_to = |account: Uuid| {
        event(
            "social.friend.requested",
            serde_json::json!({ "request_id": Uuid::now_v7(), "from_user_id": bob.id, "to_user_id": account }),
        )
    };

    // Three deliveries a minute to that host, whoever of hers they are for.
    assert_eq!(ctx.dev.fan_out(&request_to(alice.id)).await.unwrap(), 2);
    assert_eq!(ctx.dev.fan_out(&request_to(Uuid::parse_str(&bot_id).unwrap())).await.unwrap(), 1);
    assert_eq!(ctx.dev.fan_out(&request_to(alice.id)).await.unwrap(), 0, "over the budget: dropped, not queued");
    assert_eq!(ctx.dev.fan_out(&request_to(Uuid::parse_str(&bot_id).unwrap())).await.unwrap(), 0);
    // Somebody else's webhook to the same host has a budget of its own.
    create_webhook(&ctx, &bob, &receiver.url, &["friend.request_received"]).await;
    let to_bob = event(
        "social.friend.requested",
        serde_json::json!({ "request_id": Uuid::now_v7(), "from_user_id": alice.id, "to_user_id": bob.id }),
    );
    assert_eq!(ctx.dev.fan_out(&to_bob).await.unwrap(), 1);
}
