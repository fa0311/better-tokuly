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
    for procedure in ["tokuly.geo.v1.PlaceService/CheckIn", "tokuly.media.v1.MediaService/GetUploadParts"] {
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
    let cli = create_cli(&ctx, &dana, &["identity:read"]).await;
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
async fn an_rpc_without_a_line_in_the_table_is_closed_to_tokens() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let (_, secret) = personal_token(&ctx, &alice, &["notes:write", "wallet:write", "identity:write"]).await;
    let caller = with_token(&alice, &secret);
    // Names that sound like reading, a service that does not exist, an RPC that does not exist.
    for procedure in [
        "tokuly.notes.v1.NoteService/BatchDeleteNotes",
        "tokuly.wallet.v1.WalletService/CheckoutCart",
        "tokuly.live.v1.LiveService/GetStreamKey",
        "tokuly.identity.v1.AccountService/GetEverything",
    ] {
        let refused =
            ctx.app.call::<_, serde_json::Value>(procedure, Some(&caller), &serde_json::json!({})).await.unwrap_err();
        assert_eq!((refused.status, refused.reason.as_str()), (403, "TOKEN_NOT_ALLOWED"), "{procedure}");
    }
    // Scopes for domains that are not classified cannot even be asked for.
    for scope in ["live:admin", "admin:read", "dev:write"] {
        let request = CreateTokenRequest { name: "x".into(), scopes: vec![scope.into()], ..Default::default() };
        let refused = ctx
            .app
            .call::<_, CreateTokenResponse>(&format!("{TOKEN_SERVICE}/CreateToken"), Some(&alice), &request)
            .await
            .unwrap_err();
        assert_eq!(refused.reason, "SCOPE_UNKNOWN", "{scope}");
    }
}

#[tokio::test]
async fn location_and_messages_take_their_own_scope_wherever_they_appear() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let (_, plain) = personal_token(&ctx, &alice, &["feed:read", "notify:read", "identity:read", "social:write"]).await;
    let plain = with_token(&alice, &plain);
    let missing = |procedure: &'static str| {
        let (ctx, plain) = (&ctx, &plain);
        async move {
            let refused =
                ctx.app.call::<_, serde_json::Value>(procedure, Some(plain), &serde_json::json!({})).await.unwrap_err();
            (refused.reason, refused.message)
        }
    };
    let lacks = |scope: &str| ("INSUFFICIENT_SCOPE".to_owned(), format!("this token lacks the scope {scope}"));
    // Friends' coordinates in the feed, distance on a profile, people nearby.
    assert_eq!(missing("tokuly.feed.v1.MapStoryService/ListMapStories").await, lacks("geo:read"));
    assert_eq!(missing("tokuly.feed.v1.ActivityService/ListFriendsActivity").await, lacks("geo:read"));
    assert_eq!(missing("tokuly.identity.v1.ProfileService/GetProfile").await, lacks("geo:read"));
    assert_eq!(missing("tokuly.social.v1.FriendService/DiscoverNearby").await, lacks("geo:read"));
    // The inbox quotes messages and place alerts.
    assert_eq!(missing("tokuly.notify.v1.InboxService/ListNotifications").await, lacks("chat:read"));
    // What is neither stays open: news, the unread count, looking people up.
    for procedure in ["tokuly.feed.v1.NewsService/ListNews", "tokuly.notify.v1.InboxService/GetUnreadCount"] {
        assert_ne!(missing(procedure).await.0, "INSUFFICIENT_SCOPE", "{procedure}");
    }
    let lookup =
        tokuly_proto::identity::BatchGetUsersRequest { user_ids: vec![bob.id.to_string()], ..Default::default() };
    assert!(
        ctx.app
            .call::<_, tokuly_proto::identity::BatchGetUsersResponse>(
                "tokuly.identity.v1.ProfileService/BatchGetUsers",
                Some(&plain),
                &lookup
            )
            .await
            .is_ok()
    );
    // With the location scope the profile is readable.
    let (_, located) = personal_token(&ctx, &alice, &["identity:read", "geo:read"]).await;
    let profile = tokuly_proto::identity::GetProfileRequest {
        by: Some(tokuly_proto::identity::get_profile_request::By::UserId(bob.id.to_string())),
        ..Default::default()
    };
    assert!(
        ctx.app
            .call::<_, tokuly_proto::identity::GetProfileResponse>(
                "tokuly.identity.v1.ProfileService/GetProfile",
                Some(&with_token(&alice, &located)),
                &profile
            )
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn spending_coins_takes_the_wallet_scope_and_reports_take_the_chat_scope() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let (_, plain) = personal_token(&ctx, &alice, &["stamp:write", "ask:write", "engagement:read"]).await;
    let (_, paying) =
        personal_token(&ctx, &alice, &["stamp:write", "ask:write", "wallet:write", "engagement:read", "chat:read"])
            .await;
    for (procedure, scope) in [
        ("tokuly.stamp.v1.StampLibraryService/PurchasePack", "wallet:write"),
        ("tokuly.ask.v1.ComplimentService/BuyHint", "wallet:write"),
        ("tokuly.engagement.v1.ReportService/ListMyReports", "chat:read"),
    ] {
        let refused = ctx
            .app
            .call::<_, serde_json::Value>(procedure, Some(&with_token(&alice, &plain)), &serde_json::json!({}))
            .await
            .unwrap_err();
        assert_eq!(
            (refused.reason.as_str(), refused.message.as_str()),
            ("INSUFFICIENT_SCOPE", format!("this token lacks the scope {scope}").as_str()),
            "{procedure}"
        );
        let passed = ctx
            .app
            .call::<_, serde_json::Value>(procedure, Some(&with_token(&alice, &paying)), &serde_json::json!({}))
            .await
            .unwrap_err();
        assert_ne!(passed.reason, "INSUFFICIENT_SCOPE", "{procedure}");
    }
    // Free stamp actions and a round's own reward stay with their domain's scope.
    for procedure in ["tokuly.stamp.v1.StampLibraryService/InstallPack", "tokuly.ask.v1.ComplimentService/Vote"] {
        let passed = ctx
            .app
            .call::<_, serde_json::Value>(procedure, Some(&with_token(&alice, &plain)), &serde_json::json!({}))
            .await
            .unwrap_err();
        assert_ne!(passed.reason, "INSUFFICIENT_SCOPE", "{procedure}");
    }
}

#[tokio::test]
async fn a_device_code_dies_when_its_application_gives_the_device_flow_up() {
    let ctx = start_with(DevConfig { device_poll_seconds: 0, ..config() }).await;
    let (dana, user) = (ctx.app.user("dana").await, ctx.app.user("user").await);
    let cli = create_cli(&ctx, &dana, &["identity:read"]).await;
    let start_flow = || async { ctx.form("/oauth/device_authorization", None, &[("client_id", &cli.id)]).await.json() };
    let (approved, waiting) = (start_flow().await, start_flow().await);
    let approve =
        ApproveDeviceRequest { user_code: approved["user_code"].as_str().unwrap().to_owned(), ..Default::default() };
    ctx.app
        .call::<_, ApproveDeviceResponse>(&format!("{AUTHORIZATION_SERVICE}/ApproveDevice"), Some(&user), &approve)
        .await
        .unwrap();

    // The developer switches the device flow off.
    let update = UpdateAppRequest {
        app_id: cli.id.clone(),
        name: "Test CLI".into(),
        redirect_uris: vec![REDIRECT_URI.into()],
        scopes: vec!["identity:read".into()],
        device_flow: false,
        ..Default::default()
    };
    ctx.app.call::<_, UpdateAppResponse>(&format!("{APP_SERVICE}/UpdateApp"), Some(&dana), &update).await.unwrap();

    // Neither the code that was already approved nor the one still waiting is worth anything.
    for device in [&approved, &waiting] {
        let poll = ctx
            .form(
                "/oauth/token",
                None,
                &[
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("device_code", device["device_code"].as_str().unwrap()),
                    ("client_id", &cli.id),
                ],
            )
            .await;
        assert_eq!((poll.status, poll.error().as_str()), (400, "expired_token"));
    }
    let late =
        ApproveDeviceRequest { user_code: waiting["user_code"].as_str().unwrap().to_owned(), ..Default::default() };
    assert_eq!(
        ctx.app
            .call::<_, ApproveDeviceResponse>(&format!("{AUTHORIZATION_SERVICE}/ApproveDevice"), Some(&user), &late)
            .await
            .unwrap_err()
            .reason,
        "DEVICE_CODE_INVALID"
    );
    let none: i64 = sqlx::query_scalar("SELECT count(*) FROM dev_oauth_tokens o JOIN dev_authorizations a ON a.id = o.authorization_id WHERE a.user_id = $1").bind(user.id).fetch_one(&ctx.app.core.db).await.unwrap();
    assert_eq!(none, 0);
}

#[tokio::test]
async fn linked_services_and_sos_take_a_session() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let (_, secret) = personal_token(&ctx, &alice, &["identity:write", "geo:write", "wallet:write"]).await;
    for procedure in [
        "tokuly.identity.v1.AccountService/BeginConnection",
        "tokuly.identity.v1.AccountService/FinishConnection",
        "tokuly.identity.v1.AccountService/DeleteConnection",
        "tokuly.identity.v1.AccountService/ListConnections",
        "tokuly.geo.v1.SafetyService/TriggerSos",
        "tokuly.geo.v1.SafetyService/ResolveSos",
        "tokuly.wallet.v1.WalletService/StartPurchase",
        "tokuly.wallet.v1.EarningsService/RequestPayout",
    ] {
        let refused = ctx
            .app
            .call::<_, serde_json::Value>(procedure, Some(&with_token(&alice, &secret)), &serde_json::json!({}))
            .await
            .unwrap_err();
        assert_eq!((refused.status, refused.reason.as_str()), (403, "SESSION_REQUIRED"), "{procedure}");
    }
}

#[tokio::test]
async fn a_bot_token_holds_no_money_and_addresses_nobody() {
    let ctx = start().await;
    let owen = ctx.app.user("owen").await;
    let bot_id = create_bot(&ctx, &owen, "Miner").await.unwrap();
    let token = |scopes: &[&str]| {
        let request = CreateBotTokenRequest {
            bot_id: bot_id.clone(),
            name: "t".into(),
            scopes: scopes.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        let (ctx, owen) = (&ctx, &owen);
        async move {
            ctx.app
                .call::<_, CreateBotTokenResponse>(&format!("{BOT_SERVICE}/CreateBotToken"), Some(owen), &request)
                .await
        }
    };
    for scopes in [
        &["wallet:write"][..],
        &["wallet:read"],
        &["identity:read", "chat:write"],
        &["social:write"],
        &["notes:write"],
        &["engagement:write"],
    ] {
        let refused = token(scopes).await.unwrap_err();
        assert_eq!(
            (refused.code.as_str(), refused.reason.as_str()),
            ("invalid_argument", "SCOPE_NOT_FOR_BOTS"),
            "{scopes:?}"
        );
    }
    let created = token(&["identity:write", "chat:read", "media:write"]).await.unwrap();
    let caller = with_token(&owen, &created.secret);
    // The daily bonus, spending, and messages are out of its reach.
    for procedure in [
        "tokuly.wallet.v1.WalletService/ClaimDailyBonus",
        "tokuly.wallet.v1.WalletService/GetBalance",
        "tokuly.chat.v1.MessageService/SendMessage",
        "tokuly.social.v1.FriendService/SendFriendRequest",
    ] {
        let refused =
            ctx.app.call::<_, serde_json::Value>(procedure, Some(&caller), &serde_json::json!({})).await.unwrap_err();
        assert_eq!(refused.reason, "INSUFFICIENT_SCOPE", "{procedure}");
    }
    // A token row that says otherwise (made before the rule) is cut down when it is verified.
    sqlx::query("UPDATE dev_tokens SET scopes = '{wallet:write,chat:write,identity:read}' WHERE id = $1")
        .bind(Uuid::parse_str(&created.token.as_option().unwrap().id).unwrap())
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let (_, fresh) = (created.token, created.secret);
    // Disabling and enabling makes the token be judged afresh.
    for disabled in [true, false] {
        let request = SetBotDisabledRequest { bot_id: bot_id.clone(), disabled, ..Default::default() };
        ctx.app
            .call::<_, SetBotDisabledResponse>(&format!("{BOT_SERVICE}/SetBotDisabled"), Some(&owen), &request)
            .await
            .unwrap();
    }
    assert_eq!(whoami(&ctx, &with_token(&owen, &fresh)).await.unwrap().scopes, ["identity:read"]);
    // Nothing announced the bot as a person who signed up.
    assert!(ctx.app.events_by("identity.user.created", Uuid::parse_str(&bot_id).unwrap()).await.is_empty());
}

/// `TOKULY_DEV_DEMO_OPERATORS`: the bots of the accounts named there that
/// also know `TOKULY_DEV_DEMO_PASSWORD` write to other domains (the demo
/// simulator's bots chat and post), still without a wallet; nobody else's
/// bots do (not even an account that took an operator's address first), and
/// the allowance ends with the address.
#[tokio::test]
async fn the_bots_of_a_demo_operator_address_people_and_still_hold_no_money() {
    const PASSWORD: &str = "the demo's own password";
    let address = format!("demo-{}@demo.tokuly.localhost", Uuid::new_v4().simple());
    let squatted = format!("squatted-{address}");
    let ctx = start_with(DevConfig {
        demo_operators: vec![address.to_uppercase(), squatted.clone()],
        demo_password: PASSWORD.into(),
        ..config()
    })
    .await;
    let with_address = |name: &'static str, email: String, password: &'static str| {
        let ctx = &ctx;
        async move {
            let user = ctx.app.user(name).await;
            sqlx::query("UPDATE users SET email = $2::citext, password_hash = $3 WHERE id = $1")
                .bind(user.id)
                .bind(email)
                .bind(tokuly_identity::password::hash(password).await.unwrap())
                .execute(&ctx.app.core.db)
                .await
                .unwrap();
            user
        }
    };
    let operator = with_address("operator", address.clone(), PASSWORD).await;
    // Signed up at an operator's address before the simulator did, without its password.
    let squatter = with_address("squatter", squatted, "a guess, not the password").await;
    let somebody = ctx.app.user("somebody").await;
    let token = |owner: &User, bot_id: &str, scopes: &[&str]| {
        let request = CreateBotTokenRequest {
            bot_id: bot_id.to_owned(),
            name: "t".into(),
            scopes: scopes.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        let (ctx, owner) = (&ctx, owner.clone());
        async move {
            ctx.app
                .call::<_, CreateBotTokenResponse>(&format!("{BOT_SERVICE}/CreateBotToken"), Some(&owner), &request)
                .await
        }
    };
    let demo_bot = create_bot(&ctx, &operator, "Demo").await.unwrap();
    let plain_bot = create_bot(&ctx, &somebody, "Plain").await.unwrap();

    let squatters_bot = create_bot(&ctx, &squatter, "Squat").await.unwrap();

    let writes = ["chat:write", "social:write", "notes:write", "geo:write", "engagement:write"];
    assert_eq!(token(&somebody, &plain_bot, &writes).await.unwrap_err().reason, "SCOPE_NOT_FOR_BOTS");
    assert_eq!(token(&squatter, &squatters_bot, &writes).await.unwrap_err().reason, "SCOPE_NOT_FOR_BOTS");
    for scopes in [&["wallet:write"][..], &["chat:write", "wallet:read"]] {
        assert_eq!(token(&operator, &demo_bot, scopes).await.unwrap_err().reason, "SCOPE_NOT_FOR_BOTS", "{scopes:?}");
    }
    let created = token(&operator, &demo_bot, &writes).await.unwrap();
    let caller = with_token(&operator, &created.secret);
    let mut held = whoami(&ctx, &caller).await.unwrap().scopes;
    held.sort();
    assert_eq!(held, ["chat:write", "engagement:write", "geo:write", "notes:write", "social:write"]);

    // The address no longer names the account: its bots are ordinary bots
    // again, whatever their token rows say (judged afresh after a switch).
    sqlx::query("UPDATE users SET email = $2::citext WHERE id = $1")
        .bind(operator.id)
        .bind(format!("moved-{address}"))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    for disabled in [true, false] {
        let request = SetBotDisabledRequest { bot_id: demo_bot.clone(), disabled, ..Default::default() };
        ctx.app
            .call::<_, SetBotDisabledResponse>(&format!("{BOT_SERVICE}/SetBotDisabled"), Some(&operator), &request)
            .await
            .unwrap();
    }
    assert!(whoami(&ctx, &caller).await.unwrap().scopes.is_empty());
    assert_eq!(token(&operator, &demo_bot, &writes).await.unwrap_err().reason, "SCOPE_NOT_FOR_BOTS");
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
