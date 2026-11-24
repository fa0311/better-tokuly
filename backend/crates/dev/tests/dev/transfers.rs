//! Handing a bot over: an offer, the receiver's answer, and what moves.
use tokuly_dev::DevConfig;
use tokuly_proto::dev::*;
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

use crate::common::*;

async fn create_bot(ctx: &Ctx, owner: &TestUser, name: &str) -> String {
    let handle = format!("{}_{}", name.to_lowercase(), tokuly_core::ids::random_base62(6).to_lowercase());
    let request = CreateBotRequest { handle, display_name: name.into(), ..Default::default() };
    let created: CreateBotResponse =
        ctx.app.call(&format!("{BOT_SERVICE}/CreateBot"), Some(owner), &request).await.expect("bot is created");
    created.bot.as_option().unwrap().user.as_option().unwrap().id.clone()
}

async fn offer(ctx: &Ctx, owner: &TestUser, bot_id: &str, to: &str) -> Result<BotTransfer, RpcError> {
    let request = OfferBotTransferRequest { bot_id: bot_id.into(), user_id: to.into(), ..Default::default() };
    let offered: OfferBotTransferResponse =
        ctx.app.call(&format!("{BOT_SERVICE}/OfferBotTransfer"), Some(owner), &request).await?;
    Ok(offered.transfer.into_option().unwrap())
}

async fn accept(ctx: &Ctx, user: &TestUser, transfer_id: &str) -> Result<Bot, RpcError> {
    let request = AcceptBotTransferRequest { transfer_id: transfer_id.into(), ..Default::default() };
    let accepted: AcceptBotTransferResponse =
        ctx.app.call(&format!("{BOT_SERVICE}/AcceptBotTransfer"), Some(user), &request).await?;
    Ok(accepted.bot.into_option().unwrap())
}

async fn cancel(ctx: &Ctx, user: &TestUser, transfer_id: &str) -> Result<CancelBotTransferResponse, RpcError> {
    let request = CancelBotTransferRequest { transfer_id: transfer_id.into(), ..Default::default() };
    ctx.app.call(&format!("{BOT_SERVICE}/CancelBotTransfer"), Some(user), &request).await
}

async fn transfers(ctx: &Ctx, user: &TestUser) -> ListBotTransfersResponse {
    ctx.app
        .call(&format!("{BOT_SERVICE}/ListBotTransfers"), Some(user), &ListBotTransfersRequest::default())
        .await
        .unwrap()
}

async fn get_bot(ctx: &Ctx, user: &TestUser, bot_id: &str) -> Result<GetBotResponse, RpcError> {
    let request = GetBotRequest { bot_id: bot_id.into(), ..Default::default() };
    ctx.app.call(&format!("{BOT_SERVICE}/GetBot"), Some(user), &request).await
}

#[tokio::test]
async fn a_bot_is_handed_over_once_the_receiver_accepts() {
    let ctx = start().await;
    let (owen, rita, mallory) = (ctx.app.user("owen").await, ctx.app.user("rita").await, ctx.app.user("mallory").await);
    let bot = create_bot(&ctx, &owen, "Helper").await;
    let bot_uuid = Uuid::parse_str(&bot).unwrap();
    let request = CreateBotTokenRequest {
        bot_id: bot.clone(),
        name: "runner".into(),
        scopes: vec!["identity:read".into()],
        ..Default::default()
    };
    let token: CreateBotTokenResponse =
        ctx.app.call(&format!("{BOT_SERVICE}/CreateBotToken"), Some(&owen), &request).await.unwrap();
    let as_bot = with_token(&owen, &token.secret);
    whoami(&ctx, &as_bot).await.expect("the token works before");
    sqlx::query(
        "INSERT INTO dev_webhooks (account_id, url, event_types, secret_sealed, secret_hint)
         VALUES ($1, 'https://owen.example/hook', '{}', '\\x00', 'x')",
    )
    .bind(bot_uuid)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
    let before = chrono::Utc::now();

    let offered = offer(&ctx, &owen, &bot, &rita.id.to_string()).await.unwrap();
    assert_eq!(offered.bot.as_option().unwrap().id, bot);
    assert_eq!(offered.to.as_option().unwrap().id, rita.id.to_string());
    let told = ctx.notifications(rita.id, "dev.bot_transfer_offered").await;
    assert_eq!(told.len(), 1, "the receiver is asked in their inbox");
    assert_eq!(told[0]["data"]["transfer_id"], offered.id);
    let (out, inc) = (transfers(&ctx, &owen).await, transfers(&ctx, &rita).await);
    assert_eq!((out.outgoing.len(), out.incoming.len(), inc.outgoing.len(), inc.incoming.len()), (1, 0, 0, 1));
    assert_eq!(inc.incoming[0].id, offered.id);
    // Nobody else can take it or call it off, and the offerer cannot accept their own.
    for user in [&mallory, &owen] {
        assert_eq!(accept(&ctx, user, &offered.id).await.unwrap_err().reason, "TRANSFER_NOT_FOUND");
    }
    assert_eq!(cancel(&ctx, &mallory, &offered.id).await.unwrap_err().reason, "TRANSFER_NOT_FOUND");

    let taken = accept(&ctx, &rita, &offered.id).await.unwrap();
    assert_eq!(taken.owner.as_option().unwrap().id, rita.id.to_string());
    assert_eq!(taken.token_count, 0, "the previous owner's tokens do not come along");
    assert_eq!(accept(&ctx, &rita, &offered.id).await.unwrap_err().reason, "TRANSFER_NOT_FOUND", "once");
    assert!(whoami(&ctx, &as_bot).await.is_err(), "the previous owner's token is refused");
    assert_eq!(get_bot(&ctx, &owen, &bot).await.unwrap_err().reason, "BOT_NOT_FOUND");
    get_bot(&ctx, &rita, &bot).await.unwrap();
    let hooks: i64 = sqlx::query_scalar("SELECT count(*) FROM dev_webhooks WHERE account_id = $1")
        .bind(bot_uuid)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(hooks, 0, "webhooks pointing at the previous owner's server are gone");
    let revoked: String = sqlx::query_scalar("SELECT revoked_reason FROM dev_tokens WHERE user_id = $1")
        .bind(bot_uuid)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(revoked, "bot_transferred");
    assert_eq!(ctx.app.events_by("dev.bot.transferred", rita.id).await.len(), 1);
    // What the bot did before still leads to who ran it then.
    assert_eq!(ctx.dev.bot_owner_at(bot_uuid, before).await.unwrap(), Some(owen.id));
    assert_eq!(ctx.dev.bot_owner_at(bot_uuid, chrono::Utc::now()).await.unwrap(), Some(rita.id));
    assert!(transfers(&ctx, &rita).await.incoming.is_empty());
    // The previous owner cannot act on it any more, the new one can.
    let switch = SetBotDisabledRequest { bot_id: bot.clone(), disabled: true, ..Default::default() };
    let refused = ctx
        .app
        .call::<_, SetBotDisabledResponse>(&format!("{BOT_SERVICE}/SetBotDisabled"), Some(&owen), &switch)
        .await
        .unwrap_err();
    assert_eq!(refused.reason, "BOT_NOT_FOUND");
    offer(&ctx, &rita, &bot, &owen.id.to_string()).await.expect("and hand it on");
}

#[tokio::test]
async fn an_offer_goes_only_to_a_person_who_can_be_reached_and_can_be_called_off() {
    let ctx = start().await;
    let (owen, rita, sam) = (ctx.app.user("owen").await, ctx.app.user("rita").await, ctx.app.user("sam").await);
    let bot = create_bot(&ctx, &owen, "Helper").await;
    let other_bot = create_bot(&ctx, &owen, "Other").await;
    let banned = ctx.app.user("banned").await;
    sqlx::query("UPDATE users SET banned_at = now() WHERE id = $1")
        .bind(banned.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    for to in
        [owen.id.to_string(), Uuid::now_v7().to_string(), "nobody".into(), other_bot.clone(), banned.id.to_string()]
    {
        assert_eq!(offer(&ctx, &owen, &bot, &to).await.unwrap_err().reason, "RECIPIENT_NOT_FOUND", "{to}");
    }
    assert_eq!(offer(&ctx, &rita, &bot, &sam.id.to_string()).await.unwrap_err().reason, "BOT_NOT_FOUND");

    // A new offer replaces the open one; the receiver declines; the offerer withdraws.
    let first = offer(&ctx, &owen, &bot, &rita.id.to_string()).await.unwrap();
    let second = offer(&ctx, &owen, &bot, &sam.id.to_string()).await.unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(accept(&ctx, &rita, &first.id).await.unwrap_err().reason, "TRANSFER_NOT_FOUND");
    assert!(transfers(&ctx, &rita).await.incoming.is_empty());
    cancel(&ctx, &sam, &second.id).await.expect("the receiver declines");
    assert_eq!(accept(&ctx, &sam, &second.id).await.unwrap_err().reason, "TRANSFER_NOT_FOUND");
    let third = offer(&ctx, &owen, &bot, &sam.id.to_string()).await.unwrap();
    cancel(&ctx, &owen, &third.id).await.expect("the offerer withdraws");
    assert_eq!(cancel(&ctx, &owen, &third.id).await.unwrap_err().reason, "TRANSFER_NOT_FOUND");
    // An offer that ran out is gone, and so is one for a bot that was deleted.
    let fourth = offer(&ctx, &owen, &bot, &sam.id.to_string()).await.unwrap();
    sqlx::query("UPDATE dev_bot_transfers SET expires_at = now() WHERE id = $1")
        .bind(Uuid::parse_str(&fourth.id).unwrap())
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(transfers(&ctx, &sam).await.incoming.is_empty());
    assert_eq!(accept(&ctx, &sam, &fourth.id).await.unwrap_err().reason, "TRANSFER_NOT_FOUND");
    let fifth = offer(&ctx, &owen, &bot, &sam.id.to_string()).await.unwrap();
    let delete = DeleteBotRequest { bot_id: bot.clone(), ..Default::default() };
    let _: DeleteBotResponse = ctx.app.call(&format!("{BOT_SERVICE}/DeleteBot"), Some(&owen), &delete).await.unwrap();
    assert_eq!(accept(&ctx, &sam, &fifth.id).await.unwrap_err().reason, "TRANSFER_NOT_FOUND");
    // Anonymous callers and tokens have no say.
    let anonymous = ctx
        .app
        .call::<_, ListBotTransfersResponse>(
            &format!("{BOT_SERVICE}/ListBotTransfers"),
            None,
            &ListBotTransfersRequest::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(anonymous.reason, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn offers_accepted_at_once_respect_the_receivers_allowance() {
    let ctx = start_with(DevConfig { max_bots: 1, ..config() }).await;
    let rita = ctx.app.user("rita").await;
    let mut offers = Vec::new();
    for n in 0..POOL_SIZE {
        let owner = ctx.app.user(&format!("owner{n}")).await;
        let bot = create_bot(&ctx, &owner, "Gift").await;
        offers.push(offer(&ctx, &owner, &bot, &rita.id.to_string()).await.unwrap().id);
    }
    // As many accepts as the pool has connections, from one account, at once.
    let results = at_once(POOL_SIZE, |n| {
        let (ctx, rita, id) = (&ctx, &rita, offers[n].clone());
        async move { accept(ctx, rita, &id).await }
    })
    .await;
    let taken = results.iter().filter(|result| result.is_ok()).count();
    assert_eq!(taken, 1, "{results:?}");
    assert!(results.iter().filter_map(|r| r.as_ref().err()).all(|e| e.reason == "TOO_MANY_BOTS"));
    // And the same offer accepted twice at once moves the bot once.
    let owner = ctx.app.user("owner_again").await;
    let bot = create_bot(&ctx, &owner, "Twice").await;
    let sam = ctx.app.user("sam").await;
    let id = offer(&ctx, &owner, &bot, &sam.id.to_string()).await.unwrap().id;
    let results = at_once(POOL_SIZE, |_| {
        let (ctx, sam, id) = (&ctx, &sam, id.clone());
        async move { accept(ctx, sam, &id).await }
    })
    .await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
}

async fn block(ctx: &Ctx, blocker: &TestUser, blocked: &TestUser) {
    let request = tokuly_proto::social::BlockUserRequest { user_id: blocked.id.to_string(), ..Default::default() };
    let _: tokuly_proto::social::BlockUserResponse =
        ctx.app.call("tokuly.social.v1.FriendService/BlockUser", Some(blocker), &request).await.expect("block");
}

#[tokio::test]
async fn a_block_either_way_stops_an_offer_and_its_acceptance() {
    let ctx = start().await;
    let (owen, rita, sam, tom) =
        (ctx.app.user("owen").await, ctx.app.user("rita").await, ctx.app.user("sam").await, ctx.app.user("tom").await);
    let bot = create_bot(&ctx, &owen, "Helper").await;

    // Offering: to someone who blocked the owner, or whom the owner blocked.
    block(&ctx, &rita, &owen).await;
    block(&ctx, &owen, &sam).await;
    for to in [&rita, &sam] {
        assert_eq!(offer(&ctx, &owen, &bot, &to.id.to_string()).await.unwrap_err().reason, "RECIPIENT_NOT_FOUND");
    }
    assert!(ctx.notifications(rita.id, "dev.bot_transfer_offered").await.is_empty());

    // Accepting: a block made after the offer, either way, ends it.
    let offered = offer(&ctx, &owen, &bot, &tom.id.to_string()).await.unwrap();
    block(&ctx, &tom, &owen).await;
    assert_eq!(accept(&ctx, &tom, &offered.id).await.unwrap_err().reason, "TRANSFER_NOT_FOUND");
    let other = create_bot(&ctx, &owen, "Other").await;
    let lena = ctx.app.user("lena").await;
    let offered = offer(&ctx, &owen, &other, &lena.id.to_string()).await.unwrap();
    block(&ctx, &owen, &lena).await;
    assert_eq!(accept(&ctx, &lena, &offered.id).await.unwrap_err().reason, "TRANSFER_NOT_FOUND");
    let mine = ListBotsRequest::default();
    let still: ListBotsResponse = ctx.app.call(&format!("{BOT_SERVICE}/ListBots"), Some(&owen), &mine).await.unwrap();
    assert_eq!(still.bots.len(), 2, "both bots stay with their owner");
}

#[tokio::test]
async fn the_bots_conversations_are_told_it_has_a_new_owner() {
    let ctx = start().await;
    let (owen, rita) = (ctx.app.user("owen").await, ctx.app.user("rita").await);
    let bot = create_bot(&ctx, &owen, "Helper").await;
    let bot_uuid = Uuid::parse_str(&bot).unwrap();
    let offered = offer(&ctx, &owen, &bot, &rita.id.to_string()).await.unwrap();
    accept(&ctx, &rita, &offered.id).await.unwrap();
    assert!(ctx.notices.posted.lock().is_empty(), "told by the event consumer, after the commit");

    // The hand-over's event, delivered twice.
    let transferred = ctx.app.events_by("dev.bot.transferred", rita.id).await.pop().expect("the hand-over's event");
    ctx.dev.on_account_event(&transferred).await.unwrap();
    ctx.dev.on_account_event(&transferred).await.unwrap();
    let posted = ctx.notices.posted.lock().clone();
    assert_eq!(posted.len(), 2);
    for (account, key, notice) in &posted {
        assert_eq!(*account, bot_uuid, "the bot's conversations, not its owners'");
        assert_eq!(key, &format!("dev.bot.transferred:{}", transferred.id), "one key per hand-over: posted once");
        assert_eq!((notice.ja.as_str(), notice.en.as_str()), ("このボットの持ち主が変わったよ", "This bot has a new owner"));
    }
}
