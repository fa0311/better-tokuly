//! Bots: accounts owned by a person, acting through their own tokens.
use tokuly_dev::{DevConfig, secrets};
use tokuly_proto::{dev::*, identity};
use uuid::Uuid;

use crate::common::*;

async fn create_bot(ctx: &Ctx, owner: &tokuly_testkit::TestUser, name: &str) -> Bot {
    let handle = format!("{}_{}", name.to_lowercase(), tokuly_core::ids::random_base62(6).to_lowercase());
    let request =
        CreateBotRequest { handle, display_name: name.into(), bio: "I am a bot".into(), ..Default::default() };
    let created: CreateBotResponse =
        ctx.app.call(&format!("{BOT_SERVICE}/CreateBot"), Some(owner), &request).await.expect("bot is created");
    created.bot.into_option().expect("bot")
}

fn bot_id(bot: &Bot) -> String {
    bot.user.as_option().expect("a bot has an account").id.clone()
}

async fn bot_token(ctx: &Ctx, owner: &tokuly_testkit::TestUser, bot: &Bot, scopes: &[&str]) -> (AccessToken, String) {
    let request = CreateBotTokenRequest {
        bot_id: bot_id(bot),
        name: "runner".into(),
        scopes: scopes.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    let created: CreateBotTokenResponse = ctx
        .app
        .call(&format!("{BOT_SERVICE}/CreateBotToken"), Some(owner), &request)
        .await
        .expect("bot token is created");
    (created.token.into_option().expect("token"), created.secret)
}

/// A caller presenting a bot's token (the identity fields are not used by the test client).
fn as_bot(owner: &tokuly_testkit::TestUser, secret: &str) -> tokuly_testkit::TestUser {
    with_token(owner, secret)
}

#[tokio::test]
async fn a_bot_is_a_real_account_owned_by_its_creator() {
    let ctx = start_with(DevConfig { max_bots: 2, ..config() }).await;
    let (owen, mallory) = (ctx.app.user("owen").await, ctx.app.user("mallory").await);
    let bot = create_bot(&ctx, &owen, "Helper").await;
    let id = bot_id(&bot);
    let account = bot.user.as_option().unwrap();
    assert_eq!(
        (account.display_name.as_str(), bot.bio.as_str(), bot.disabled, bot.token_count),
        ("Helper", "I am a bot", false, 0)
    );
    assert_eq!(bot.owner.as_option().unwrap().id, owen.id.to_string());

    // An account like any other as far as identity is concerned — one nobody can sign in to.
    let user = tokuly_identity::users::get(&ctx.app.core, Uuid::parse_str(&id).unwrap()).await.unwrap();
    assert!(user.is_active() && user.email.is_none() && user.password_hash.is_none());
    let profile: identity::GetProfileResponse = ctx
        .app
        .call(
            "tokuly.identity.v1.ProfileService/GetProfile",
            Some(&mallory),
            &identity::GetProfileRequest {
                by: Some(identity::get_profile_request::By::UserId(id.clone())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(profile.profile.as_option().unwrap().user.as_option().unwrap().handle, account.handle);
    // It is announced as a bot, not as a person who signed up.
    assert!(ctx.app.events("identity.user.created").await.iter().all(|e| e.data["user_id"] != id));
    assert_eq!(ctx.app.events_by("dev.bot.created", owen.id).await.len(), 1);

    // Its owner finds it; nobody else does.
    let get = GetBotRequest { bot_id: id.clone(), ..Default::default() };
    assert!(ctx.app.call::<_, GetBotResponse>(&format!("{BOT_SERVICE}/GetBot"), Some(&owen), &get).await.is_ok());
    let hidden =
        ctx.app.call::<_, GetBotResponse>(&format!("{BOT_SERVICE}/GetBot"), Some(&mallory), &get).await.unwrap_err();
    assert_eq!((hidden.code.as_str(), hidden.reason.as_str()), ("not_found", "BOT_NOT_FOUND"));
    let listed: ListBotsResponse =
        ctx.app.call(&format!("{BOT_SERVICE}/ListBots"), Some(&owen), &ListBotsRequest::default()).await.unwrap();
    assert_eq!((listed.bots.len(), listed.max_bots), (1, 2));
    let none: ListBotsResponse =
        ctx.app.call(&format!("{BOT_SERVICE}/ListBots"), Some(&mallory), &ListBotsRequest::default()).await.unwrap();
    assert!(none.bots.is_empty());

    // Handles are the same namespace as people's; the number of bots is capped.
    let create = |request: CreateBotRequest| {
        let (ctx, owen) = (&ctx, &owen);
        async move {
            ctx.app
                .call::<_, CreateBotResponse>(&format!("{BOT_SERVICE}/CreateBot"), Some(owen), &request)
                .await
                .map_err(|e| e.reason)
        }
    };
    let request = |handle: &str, name: &str| CreateBotRequest {
        handle: handle.into(),
        display_name: name.into(),
        ..Default::default()
    };
    assert_eq!(create(request(&account.handle, "Twin")).await.unwrap_err(), "HANDLE_TAKEN");
    assert_eq!(create(request(&owen.handle, "Me")).await.unwrap_err(), "HANDLE_TAKEN");
    assert_eq!(create(request("no", "Short")).await.unwrap_err(), "HANDLE_INVALID");
    assert_eq!(create(request("admin", "Admin")).await.unwrap_err(), "HANDLE_RESERVED");
    assert_eq!(create(request("fine_handle_1", "")).await.unwrap_err(), "DISPLAY_NAME_INVALID");
    assert_eq!(
        create(CreateBotRequest { bio: "b".repeat(301), ..request("fine_handle_2", "Wordy") }).await.unwrap_err(),
        "BIO_TOO_LONG"
    );
    create_bot(&ctx, &owen, "Second").await;
    assert_eq!(
        create(request(&format!("third_{}", tokuly_core::ids::random_base62(6).to_lowercase()), "Third"))
            .await
            .unwrap_err(),
        "TOO_MANY_BOTS"
    );
    let anonymous = ctx
        .app
        .call::<_, CreateBotResponse>(&format!("{BOT_SERVICE}/CreateBot"), None, &request("anon_bot", "Anon"))
        .await
        .unwrap_err();
    assert_eq!(anonymous.reason, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn a_bot_token_acts_as_the_bot() {
    let ctx = start().await;
    let (owen, mallory) = (ctx.app.user("owen").await, ctx.app.user("mallory").await);
    let bot = create_bot(&ctx, &owen, "Helper").await;
    let id = bot_id(&bot);
    let (token, secret) = bot_token(&ctx, &owen, &bot, &["identity:write"]).await;
    assert!(secret.starts_with("tkl_bot_") && secrets::is(&secret, secrets::SecretKind::Bot));
    assert_eq!(ctx.security_events(owen.id, "BOT_TOKEN_CREATED").await, 1);
    let caller = as_bot(&owen, &secret);

    let me = whoami(&ctx, &caller).await.unwrap();
    assert_eq!(me.kind.as_known(), Some(CredentialKind::CREDENTIAL_KIND_BOT));
    assert_eq!(me.user.as_option().unwrap().id, id, "the bot, not its owner");
    // It reads and writes as itself in other domains.
    let got: identity::GetMeResponse = ctx
        .app
        .call("tokuly.identity.v1.AccountService/GetMe", Some(&caller), &identity::GetMeRequest::default())
        .await
        .unwrap();
    assert_eq!(got.me.as_option().unwrap().user.as_option().unwrap().id, id);
    let status = identity::SetStatusRequest { emoji: "🤖".into(), text: "running".into(), ..Default::default() };
    ctx.app
        .call::<_, identity::SetStatusResponse>("tokuly.identity.v1.AccountService/SetStatus", Some(&caller), &status)
        .await
        .unwrap();
    let owner_status: String = sqlx::query_scalar("SELECT status_text FROM users WHERE id = $1")
        .bind(owen.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(owner_status, "", "the owner's account is untouched");
    // A bot cannot manage bots or tokens, its own included.
    let denied = ctx
        .app
        .call::<_, ListBotsResponse>(&format!("{BOT_SERVICE}/ListBots"), Some(&caller), &ListBotsRequest::default())
        .await
        .unwrap_err();
    assert_eq!(denied.reason, "SESSION_REQUIRED");

    // Tokens of a bot: listed and revoked by the owner only.
    let listed: ListBotTokensResponse = ctx
        .app
        .call(
            &format!("{BOT_SERVICE}/ListBotTokens"),
            Some(&owen),
            &ListBotTokensRequest { bot_id: id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(listed.tokens.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), [token.id.as_str()]);
    let theirs = ctx
        .app
        .call::<_, ListBotTokensResponse>(
            &format!("{BOT_SERVICE}/ListBotTokens"),
            Some(&mallory),
            &ListBotTokensRequest { bot_id: id.clone(), ..Default::default() },
        )
        .await;
    assert_eq!(theirs.unwrap_err().reason, "BOT_NOT_FOUND");
    let foreign = CreateBotTokenRequest {
        bot_id: id.clone(),
        name: "theirs".into(),
        scopes: vec!["chat:write".into()],
        ..Default::default()
    };
    assert_eq!(
        ctx.app
            .call::<_, CreateBotTokenResponse>(&format!("{BOT_SERVICE}/CreateBotToken"), Some(&mallory), &foreign)
            .await
            .unwrap_err()
            .reason,
        "BOT_NOT_FOUND"
    );
    let unscoped = CreateBotTokenRequest { scopes: vec![], ..foreign.clone() };
    assert_eq!(
        ctx.app
            .call::<_, CreateBotTokenResponse>(&format!("{BOT_SERVICE}/CreateBotToken"), Some(&owen), &unscoped)
            .await
            .unwrap_err()
            .reason,
        "SCOPES_REQUIRED"
    );
    // A bot token is not a personal token: it is not in the owner's list, nor revocable as one.
    let personal: ListTokensResponse =
        ctx.app.call(&format!("{TOKEN_SERVICE}/ListTokens"), Some(&owen), &ListTokensRequest::default()).await.unwrap();
    assert!(personal.tokens.is_empty());
    let as_personal = ctx
        .app
        .call::<_, RevokeTokenResponse>(
            &format!("{TOKEN_SERVICE}/RevokeToken"),
            Some(&owen),
            &RevokeTokenRequest { token_id: token.id.clone(), ..Default::default() },
        )
        .await;
    assert_eq!(as_personal.unwrap_err().reason, "TOKEN_NOT_FOUND");

    let revoke = RevokeBotTokenRequest { token_id: token.id.clone(), ..Default::default() };
    assert_eq!(
        ctx.app
            .call::<_, RevokeBotTokenResponse>(&format!("{BOT_SERVICE}/RevokeBotToken"), Some(&mallory), &revoke)
            .await
            .unwrap_err()
            .reason,
        "TOKEN_NOT_FOUND"
    );
    ctx.app
        .call::<_, RevokeBotTokenResponse>(&format!("{BOT_SERVICE}/RevokeBotToken"), Some(&owen), &revoke)
        .await
        .unwrap();
    assert_eq!(whoami(&ctx, &caller).await.unwrap_err().reason, "TOKEN_REVOKED");
}

#[tokio::test]
async fn the_owner_renames_a_bot_and_changes_its_avatar() {
    let ctx = start().await;
    let (owen, mallory) = (ctx.app.user("owen").await, ctx.app.user("mallory").await);
    let bot = create_bot(&ctx, &owen, "Helper").await;
    let id = bot_id(&bot);
    let avatar = ctx.images.upload(owen.id);
    let new_handle = format!("renamed_{}", tokuly_core::ids::random_base62(6).to_lowercase());
    let update = UpdateBotRequest {
        bot_id: id.clone(),
        display_name: Some("Butler".into()),
        handle: Some(new_handle.clone()),
        bio: Some("At your service".into()),
        avatar_asset_id: Some(avatar.to_string()),
        ..Default::default()
    };
    let hidden = ctx
        .app
        .call::<_, UpdateBotResponse>(&format!("{BOT_SERVICE}/UpdateBot"), Some(&mallory), &update)
        .await
        .unwrap_err();
    assert_eq!(hidden.reason, "BOT_NOT_FOUND");
    let updated: UpdateBotResponse =
        ctx.app.call(&format!("{BOT_SERVICE}/UpdateBot"), Some(&owen), &update).await.unwrap();
    let changed = updated.bot.as_option().unwrap();
    let account = changed.user.as_option().unwrap();
    assert_eq!(
        (account.display_name.as_str(), account.handle.as_str(), changed.bio.as_str()),
        ("Butler", new_handle.as_str(), "At your service")
    );
    assert_eq!(account.avatar.as_option().unwrap().url, format!("https://media.test/{avatar}.webp"));
    assert_eq!(ctx.images.held_by(&format!("dev:bot-avatar:{id}")), Some(avatar));
    // The rest of the app hears of it the way it hears of any profile change.
    let updates = ctx.app.events_by("dev.bot.updated", owen.id).await;
    assert_eq!((updates.len(), updates[0].data["bot_id"].as_str()), (1, Some(id.as_str())));

    // Unset fields stay; an empty avatar is removed.
    let partial = UpdateBotRequest { bot_id: id.clone(), avatar_asset_id: Some(String::new()), ..Default::default() };
    let updated: UpdateBotResponse =
        ctx.app.call(&format!("{BOT_SERVICE}/UpdateBot"), Some(&owen), &partial).await.unwrap();
    let account = updated.bot.as_option().unwrap().user.as_option().unwrap();
    assert_eq!(account.display_name, "Butler");
    assert!(account.avatar.as_option().is_none() && ctx.images.held_by(&format!("dev:bot-avatar:{id}")).is_none());

    let update_with = |change: UpdateBotRequest| {
        let (ctx, owen) = (&ctx, &owen);
        async move {
            ctx.app
                .call::<_, UpdateBotResponse>(&format!("{BOT_SERVICE}/UpdateBot"), Some(owen), &change)
                .await
                .unwrap_err()
                .reason
        }
    };
    let base = || UpdateBotRequest { bot_id: id.clone(), ..Default::default() };
    assert_eq!(update_with(UpdateBotRequest { handle: Some(owen.handle.clone()), ..base() }).await, "HANDLE_TAKEN");
    assert_eq!(update_with(UpdateBotRequest { handle: Some("x".into()), ..base() }).await, "HANDLE_INVALID");
    assert_eq!(
        update_with(UpdateBotRequest { display_name: Some(String::new()), ..base() }).await,
        "DISPLAY_NAME_INVALID"
    );
    // An image somebody else uploaded is not the owner's to use.
    assert_eq!(
        update_with(UpdateBotRequest { avatar_asset_id: Some(ctx.images.upload(mallory.id).to_string()), ..base() })
            .await,
        "ASSET_NOT_FOUND"
    );
}

#[tokio::test]
async fn a_disabled_bot_is_silent_until_it_is_enabled_again() {
    let ctx = start().await;
    let (owen, mallory) = (ctx.app.user("owen").await, ctx.app.user("mallory").await);
    let bot = create_bot(&ctx, &owen, "Helper").await;
    let (_, secret) = bot_token(&ctx, &owen, &bot, &["identity:read"]).await;
    let caller = as_bot(&owen, &secret);
    assert!(whoami(&ctx, &caller).await.is_ok());

    let set = |disabled: bool, by: &tokuly_testkit::TestUser| {
        let (ctx, by, request) =
            (&ctx, by.clone(), SetBotDisabledRequest { bot_id: bot_id(&bot), disabled, ..Default::default() });
        async move {
            ctx.app
                .call::<_, SetBotDisabledResponse>(&format!("{BOT_SERVICE}/SetBotDisabled"), Some(&by), &request)
                .await
        }
    };
    assert_eq!(set(true, &mallory).await.unwrap_err().reason, "BOT_NOT_FOUND");
    assert!(set(true, &owen).await.unwrap().bot.as_option().unwrap().disabled);
    // At once, although the token was in use a moment ago.
    assert_eq!(whoami(&ctx, &caller).await.unwrap_err().reason, "TOKEN_REVOKED");
    assert!(!set(false, &owen).await.unwrap().bot.as_option().unwrap().disabled);
    assert!(whoami(&ctx, &caller).await.is_ok(), "the same token works again");
}

#[tokio::test]
async fn deleting_a_bot_deletes_its_account() {
    let ctx = start().await;
    let (owen, mallory) = (ctx.app.user("owen").await, ctx.app.user("mallory").await);
    let bot = create_bot(&ctx, &owen, "Helper").await;
    let id = bot_id(&bot);
    let account_id = Uuid::parse_str(&id).unwrap();
    let (_, secret) = bot_token(&ctx, &owen, &bot, &["identity:read"]).await;
    let caller = as_bot(&owen, &secret);
    // Its owner subscribed a webhook for it.
    let for_bot = CreateWebhookRequest {
        url: "http://127.0.0.1:9/hook".into(),
        event_types: vec!["account.updated".into()],
        bot_id: id.clone(),
        ..Default::default()
    };
    let created: CreateWebhookResponse =
        ctx.app.call(&format!("{WEBHOOK_SERVICE}/CreateWebhook"), Some(&owen), &for_bot).await.unwrap();
    assert_eq!(created.webhook.as_option().unwrap().account_id, id);

    let delete = DeleteBotRequest { bot_id: id.clone(), ..Default::default() };
    assert_eq!(
        ctx.app
            .call::<_, DeleteBotResponse>(&format!("{BOT_SERVICE}/DeleteBot"), Some(&mallory), &delete)
            .await
            .unwrap_err()
            .reason,
        "BOT_NOT_FOUND"
    );
    ctx.app.call::<_, DeleteBotResponse>(&format!("{BOT_SERVICE}/DeleteBot"), Some(&owen), &delete).await.unwrap();

    // Gone here...
    assert_eq!(whoami(&ctx, &caller).await.unwrap_err().reason, "TOKEN_INVALID");
    let listed: ListBotsResponse =
        ctx.app.call(&format!("{BOT_SERVICE}/ListBots"), Some(&owen), &ListBotsRequest::default()).await.unwrap();
    assert!(listed.bots.is_empty());
    let left: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM dev_bots WHERE user_id = $1 AND deleted_at IS NULL), (SELECT count(*) FROM dev_tokens WHERE user_id = $1), (SELECT count(*) FROM dev_webhooks WHERE account_id = $1)",
    )
    .bind(account_id)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(left, (0, 0, 0));
    // ...and as an account: it is due for deletion, and identity's own purge
    // (hourly; run here by the test) erases it and tells every domain.
    let user = tokuly_identity::users::get(&ctx.app.core, account_id).await.unwrap();
    assert!(user.deleted_at.is_none() && user.deletion_scheduled_at.is_some());
    ctx.app.identity.purge_deleted_accounts().await.unwrap();
    let user = tokuly_identity::users::get(&ctx.app.core, account_id).await.unwrap();
    assert!(user.deleted_at.is_some() && user.display_name.is_empty());
    assert_eq!(ctx.app.events_by("identity.user.deleted", account_id).await.len(), 1);
    assert_eq!(ctx.app.events_by("dev.bot.deleted", owen.id).await.len(), 1);
    assert_eq!(
        ctx.app
            .call::<_, DeleteBotResponse>(&format!("{BOT_SERVICE}/DeleteBot"), Some(&owen), &delete)
            .await
            .unwrap_err()
            .reason,
        "BOT_NOT_FOUND"
    );
    // The handle's deletion event reaching this domain later changes nothing.
    ctx.dev
        .on_account_event(&event("identity.user.deleted", serde_json::json!({ "user_id": account_id })))
        .await
        .unwrap();

    // Usage of a bot is the owner's to see (none here).
    let another = create_bot(&ctx, &owen, "Counter").await;
    let usage =
        GetUsageRequest { subject: Some(get_usage_request::Subject::BotId(bot_id(&another))), ..Default::default() };
    let seen: GetUsageResponse =
        ctx.app.call("tokuly.dev.v1.UsageService/GetUsage", Some(&owen), &usage).await.unwrap();
    assert_eq!(seen.requests, 0);
    assert_eq!(
        ctx.app
            .call::<_, GetUsageResponse>("tokuly.dev.v1.UsageService/GetUsage", Some(&mallory), &usage)
            .await
            .unwrap_err()
            .reason,
        "BOT_NOT_FOUND"
    );
}

#[tokio::test]
async fn a_bot_cannot_take_a_handle_held_for_the_person_who_gave_it_up() {
    let ctx = start().await;
    let (owen, carol) = (ctx.app.user("owen").await, ctx.app.user("carol").await);
    let given_up = carol.handle.clone();
    let renamed = format!("carol_{}", tokuly_core::ids::random_base62(6).to_lowercase());
    let _: identity::UpdateProfileResponse = ctx
        .app
        .call(
            "tokuly.identity.v1.AccountService/UpdateProfile",
            Some(&carol),
            &identity::UpdateProfileRequest { handle: Some(renamed), ..Default::default() },
        )
        .await
        .expect("carol renames");

    let request = CreateBotRequest { handle: given_up.clone(), display_name: "Squatter".into(), ..Default::default() };
    let refused = ctx
        .app
        .call::<_, CreateBotResponse>(&format!("{BOT_SERVICE}/CreateBot"), Some(&owen), &request)
        .await
        .unwrap_err();
    assert_eq!(refused.reason, "HANDLE_TAKEN", "the handle is held for carol");

    let bot = create_bot(&ctx, &owen, "Helper").await;
    let rename = UpdateBotRequest { bot_id: bot_id(&bot), handle: Some(given_up), ..Default::default() };
    let refused = ctx
        .app
        .call::<_, UpdateBotResponse>(&format!("{BOT_SERVICE}/UpdateBot"), Some(&owen), &rename)
        .await
        .unwrap_err();
    assert_eq!(refused.reason, "HANDLE_TAKEN");
}

/// The birthdate of someone who turns `years` today.
fn born(years: u32) -> chrono::NaiveDate {
    chrono::Utc::now().date_naive().checked_sub_months(chrono::Months::new(12 * years)).unwrap()
}

/// An account under the minimum age cannot use Tokuly through a session, and
/// no more through what acts for it: its tokens, its bots, the applications
/// it let in. They stop with the answer and work again from that birthday.
#[tokio::test]
async fn the_credentials_of_an_account_under_the_minimum_age_stop_until_that_birthday() {
    let ctx = start().await;
    let (kid, dana) = (ctx.app.user("kid").await, ctx.app.user("dana").await);
    let (_, pat) = personal_token(&ctx, &kid, &["identity:read"]).await;
    let bot = create_bot(&ctx, &kid, "Helper").await;
    let (_, bot_secret) = bot_token(&ctx, &kid, &bot, &["identity:read"]).await;
    let app = create_app(&ctx, &dana, &["identity:read"], AppClientType::APP_CLIENT_TYPE_CONFIDENTIAL).await;
    let (access, _) = obtain_tokens(&ctx, &kid, &app, "identity:read").await;
    let (_, others) = personal_token(&ctx, &dana, &["identity:read"]).await;
    let credentials = [("personal", pat), ("bot", bot_secret), ("oauth", access)];
    for (kind, secret) in &credentials {
        assert!(whoami(&ctx, &with_token(&kid, secret)).await.is_ok(), "{kind}");
    }

    let answer = |birthdate: chrono::NaiveDate| {
        let (ctx, kid) = (&ctx, &kid);
        async move {
            sqlx::query("UPDATE users SET birthdate = $2 WHERE id = $1")
                .bind(kid.id)
                .bind(birthdate)
                .execute(&ctx.app.core.db)
                .await
                .unwrap();
            let given = event("identity.user.birthdate_given", serde_json::json!({ "user_id": kid.id, "minor": true }));
            ctx.dev.on_account_event(&given).await.unwrap();
        }
    };
    answer(born(12)).await;
    for (kind, secret) in &credentials {
        // At once, although the token was in use a moment ago; and as any
        // revoked token: the application does not learn why.
        assert_eq!(whoami(&ctx, &with_token(&kid, secret)).await.unwrap_err().reason, "TOKEN_REVOKED", "{kind}");
    }
    assert!(whoami(&ctx, &with_token(&dana, &others)).await.is_ok());

    answer(born(13)).await;
    for (kind, secret) in &credentials {
        assert!(whoami(&ctx, &with_token(&kid, secret)).await.is_ok(), "{kind}: nothing was revoked");
    }
}
