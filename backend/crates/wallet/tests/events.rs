//! What happens after the ledger moved coins: realtime pushes, "coins
//! received" notifications, account deletion and the data export.
mod common;

use std::time::Duration;

use common::*;
use tokuly_core::events::Event;
use tokuly_proto::{realtime::server_frame::Body, wallet::*};
use tokuly_testkit::{FrameListener, TestApp, TestUser};
use tokuly_wallet::{Wallet, ledger};

/// Feeds every `wallet.balance.changed` event of `user` to the consumer, as
/// the event stream would.
async fn deliver_balance_events(app: &TestApp, wallet: &Wallet, user: &TestUser) {
    for event in balance_events(app, user).await {
        wallet.handle_event(event).await.unwrap();
    }
}

async fn next_balance_push(frames: &mut FrameListener) -> BalanceChanged {
    let frame = frames.next(Duration::from_secs(2)).await.expect("a wallet frame");
    let Some(Body::Wallet(event)) = frame.body else { panic!("not a wallet frame") };
    let Some(wallet_event::Event::BalanceChanged(change)) = event.event else { panic!("not a balance change") };
    *change
}

fn user_deleted(user: &TestUser) -> Event {
    Event { id: uuid::Uuid::now_v7(), topic: "identity.user.deleted".into(), actor_id: Some(user.id), occurred_at: chrono::Utc::now(), data: serde_json::json!({ "user_id": user.id }) }
}

#[tokio::test]
async fn a_received_gift_is_pushed_and_announced_once() {
    let (app, wallet) = app().await;
    let (fan, streamer) = (app.user("ファン").await, app.user("streamer").await);
    fund(&app, &fan, 1000).await;
    let mut streamer_frames = app.listen_user(streamer.id).await;
    let mut fan_frames = app.listen_user(fan.id).await;
    let gift = ledger::transfer(&app.core, fan.id, streamer.id, 300, 30, "live.gift", &key("gift"), serde_json::json!({ "stream_id": "s1" })).await.unwrap();

    // The streamer's devices get the new balance with the entry that caused it.
    deliver_balance_events(&app, &wallet, &streamer).await;
    let pushed = next_balance_push(&mut streamer_frames).await;
    let pushed_balance = pushed.balance.as_option().unwrap();
    assert_eq!((pushed_balance.coins, pushed_balance.earnings), (0, 210));
    assert_eq!(pushed_balance.revision, balance(&app, &streamer).await.revision, "the push and the RPC agree on the revision");
    let entry = pushed.entry.as_option().unwrap();
    assert_eq!((entry.id.as_str(), entry.earnings_delta, entry.description.as_str()), (gift.entry_id.to_string().as_str(), 210, "ライブギフトを受け取りました"));
    assert_eq!(entry.counterpart.as_option().unwrap().id, fan.id.to_string());

    // ...and a notification from the fan.
    let notices = notifications(&app, &streamer, "wallet.received").await;
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0]["actor_id"], fan.id.to_string());
    assert_eq!(notices[0]["title"]["ja"], "コインを受け取りました!");
    assert_eq!(notices[0]["body"]["ja"], "ファンさんから +210コイン（ライブギフトを受け取りました）");
    assert_eq!(notices[0]["body"]["en"], "+210 coins from ファン (Received a live gift)");
    assert_eq!(notices[0]["data"]["coins"], 210);

    // The fan's devices see the coins leave; paying is not news to the payer.
    deliver_balance_events(&app, &wallet, &fan).await;
    let funded = next_balance_push(&mut fan_frames).await;
    let sent = next_balance_push(&mut fan_frames).await;
    assert_eq!((funded.balance.as_option().unwrap().coins, sent.balance.as_option().unwrap().coins), (1000, 700));
    assert!(sent.balance.as_option().unwrap().revision > funded.balance.as_option().unwrap().revision);
    assert_eq!(sent.entry.as_option().unwrap().coins_delta, -300);
    assert!(notifications(&app, &fan, "wallet.received").await.is_empty());

    // Redelivery pushes again (harmless: same revision) but never notifies twice.
    deliver_balance_events(&app, &wallet, &streamer).await;
    assert_eq!(next_balance_push(&mut streamer_frames).await.balance.as_option().unwrap().revision, pushed_balance.revision);
    assert_eq!(notifications(&app, &streamer, "wallet.received").await.len(), 1);
}

#[tokio::test]
async fn refunds_and_staff_credits_are_announced_but_own_actions_are_not() {
    let (app, wallet) = app().await;
    let (fan, streamer, admin) = (app.user("fan").await, app.user("streamer").await, app.user_with_roles("boss", &["admin"]).await);

    // Own actions: a purchase and the daily bonus.
    let coins_1000 = package(&app, "coins_1000").await;
    let _: StartPurchaseResponse = app.call(&format!("{WALLET}/StartPurchase"), Some(&fan), &StartPurchaseRequest { package_id: coins_1000.id, ..Default::default() }).await.unwrap();
    let _: ClaimDailyBonusResponse = app.call(&format!("{WALLET}/ClaimDailyBonus"), Some(&fan), &ClaimDailyBonusRequest::default()).await.unwrap();
    deliver_balance_events(&app, &wallet, &fan).await;
    assert!(notifications(&app, &fan, "wallet.received").await.is_empty());

    // A refunded gift: the fan is told the coins came back; the streamer loses earnings quietly.
    let gift = ledger::transfer(&app.core, fan.id, streamer.id, 300, 30, "live.gift", &key("gift"), serde_json::json!({})).await.unwrap();
    ledger::refund(&app.core, gift.entry_id, "live.gift.returned").await.unwrap();
    deliver_balance_events(&app, &wallet, &fan).await;
    deliver_balance_events(&app, &wallet, &streamer).await;
    let refunds = notifications(&app, &fan, "wallet.received").await;
    assert_eq!(refunds.len(), 1);
    assert_eq!((refunds[0]["title"]["en"].as_str(), refunds[0]["body"]["en"].as_str()), (Some("Coins refunded"), Some("300 coins are back in your wallet.")));
    assert!(refunds[0]["actor_id"].is_null(), "a system notice");
    assert_eq!(notifications(&app, &streamer, "wallet.received").await.len(), 1, "only the gift itself was announced");

    // A credit from staff is announced; a debit is not.
    let credit = AdjustBalanceRequest { user_id: streamer.id.to_string(), account: LedgerAccount::LEDGER_ACCOUNT_EARNINGS.into(), delta: 500, reason: "キャンペーン賞金".into(), ..Default::default() };
    let _: AdjustBalanceResponse = app.call(&format!("{ADMIN}/AdjustBalance"), Some(&admin), &credit).await.unwrap();
    let debit = AdjustBalanceRequest { delta: -100, ..credit.clone() };
    let _: AdjustBalanceResponse = app.call(&format!("{ADMIN}/AdjustBalance"), Some(&admin), &debit).await.unwrap();
    deliver_balance_events(&app, &wallet, &streamer).await;
    let notices = notifications(&app, &streamer, "wallet.received").await;
    assert_eq!(notices.len(), 2);
    assert_eq!(notices[1]["body"]["en"], "Tokuly staff added 500 coins to your earnings.");
    assert!(!notices[1].to_string().contains("キャンペーン賞金"), "the internal reason stays internal");
}

#[tokio::test]
async fn events_for_unknown_topics_and_vanished_users_are_ignored() {
    let (app, wallet) = app().await;
    let stray = Event { id: uuid::Uuid::now_v7(), topic: "chat.message.sent".into(), actor_id: None, occurred_at: chrono::Utc::now(), data: serde_json::json!({}) };
    wallet.handle_event(stray).await.unwrap();
    let ghost = uuid::Uuid::now_v7();
    let orphan = Event {
        id: uuid::Uuid::now_v7(),
        topic: "wallet.balance.changed".into(),
        actor_id: Some(ghost),
        occurred_at: chrono::Utc::now(),
        data: serde_json::json!({ "user_id": ghost, "coins": 1, "earnings": 0, "entry_id": uuid::Uuid::now_v7(), "reason": "x" }),
    };
    wallet.handle_event(orphan).await.unwrap();
    let malformed = Event { id: uuid::Uuid::now_v7(), topic: "wallet.balance.changed".into(), actor_id: None, occurred_at: chrono::Utc::now(), data: serde_json::json!({ "nope": true }) };
    assert!(wallet.handle_event(malformed).await.is_err(), "a payload that does not parse is retried, then dropped by the consumer");
    drop(app);
}

#[tokio::test]
async fn deleting_a_user_closes_the_wallet_but_keeps_the_books() {
    let (app, wallet) = app().await;
    let fakepay = with_fakepay(&wallet);
    let (leaver, friend) = (app.user("leaver").await, app.user("friend").await);

    // The leaver bought coins, sent a gift, received gifts, has a payout request and an unpaid checkout.
    fund(&app, &leaver, 800).await;
    let sent = ledger::transfer(&app.core, leaver.id, friend.id, 300, 30, "live.gift", &key("gift"), serde_json::json!({})).await.unwrap();
    fund(&app, &friend, 5000).await;
    let received = ledger::transfer(&app.core, friend.id, leaver.id, 3000, 30, "live.gift", &key("gift"), serde_json::json!({})).await.unwrap();
    let payout = RequestPayoutRequest { coins: 1500, method: PayoutMethod::PAYOUT_METHOD_BANK_TRANSFER.into(), destination: "銀行 1234567".into(), note: "お願いします".into(), ..Default::default() };
    let payout: RequestPayoutResponse = app.call(&format!("{EARNINGS}/RequestPayout"), Some(&leaver), &payout).await.unwrap();
    let payout_id = payout.payout.as_option().unwrap().id.clone();
    let coins_100 = package(&app, "coins_100").await;
    let checkout: StartPurchaseResponse = app.call(&format!("{WALLET}/StartPurchase"), Some(&leaver), &StartPurchaseRequest { package_id: coins_100.id, provider: "fakepay".into(), ..Default::default() }).await.unwrap();
    let purchase_id = checkout.purchase.as_option().unwrap().id.clone();
    let _: ClaimDailyBonusResponse = app.call(&format!("{WALLET}/ClaimDailyBonus"), Some(&leaver), &ClaimDailyBonusRequest::default()).await.unwrap();
    let before = balance(&app, &leaver).await;
    assert_eq!((before.coins, before.earnings), (510, 600));

    wallet.handle_event(user_deleted(&leaver)).await.unwrap();

    // Personal state is gone...
    let count = |sql: &'static str| {
        let app = &app;
        let leaver = &leaver;
        async move { sqlx::query_scalar::<_, i64>(sql).bind(leaver.id).fetch_one(&app.core.db).await.unwrap() }
    };
    assert_eq!(count("SELECT count(*) FROM wallet_accounts WHERE user_id = $1").await, 0);
    assert_eq!(count("SELECT count(*) FROM wallet_daily_bonuses WHERE user_id = $1").await, 0);
    assert_eq!(count("SELECT count(*) FROM wallet_entries WHERE from_user_id = $1 OR to_user_id = $1").await, 0, "no ledger entry points at the user any more");
    assert_eq!(count("SELECT count(*) FROM wallet_purchases WHERE user_id = $1").await, 0);
    assert_eq!(count("SELECT count(*) FROM wallet_payout_requests WHERE user_id = $1").await, 0);
    let (status, destination, note, owner): (String, Option<Vec<u8>>, String, Option<uuid::Uuid>) =
        sqlx::query_as("SELECT status, destination, note, user_id FROM wallet_payout_requests WHERE id = $1::uuid").bind(&payout_id).fetch_one(&app.core.db).await.unwrap();
    assert_eq!((status.as_str(), destination, note.as_str(), owner), ("cancelled", None, "", None), "the request stays as a record, without bank details");
    let purchase_status: String = sqlx::query_scalar("SELECT status FROM wallet_purchases WHERE id = $1::uuid").bind(&purchase_id).fetch_one(&app.core.db).await.unwrap();
    assert_eq!(purchase_status, "cancelled");

    // ...the books still balance: everything the user had (510 + 600 + 1500 on hold) was forfeited...
    assert_eq!(postings_sum(&app).await, 0);
    assert_eq!(account_sum(&app, &format!("user:{}:coins", leaver.id)).await, 0);
    assert_eq!(account_sum(&app, &format!("user:{}:earnings", leaver.id)).await, 0);
    let forfeited: i64 = sqlx::query_scalar(
        "SELECT COALESCE(sum(p.delta), 0)::bigint FROM wallet_postings p
         WHERE p.account = 'system:forfeited' AND p.entry_id IN (SELECT entry_id FROM wallet_postings WHERE account LIKE 'user:' || $1 || ':%' OR entry_id IN (SELECT id FROM wallet_entries WHERE refund_of = (SELECT hold_entry_id FROM wallet_payout_requests WHERE id = $2::uuid)))",
    )
    .bind(leaver.id.to_string())
    .bind(&payout_id)
    .fetch_one(&app.core.db)
    .await
    .unwrap();
    assert_eq!(forfeited, 510 + 600 + 1500);

    // ...and the friend keeps their side of both gifts, now without a name on them.
    let entries = history(&app, &friend).await;
    let gift_in = entries.iter().find(|entry| entry.id == sent.entry_id.to_string()).unwrap();
    let gift_out = entries.iter().find(|entry| entry.id == received.entry_id.to_string()).unwrap();
    assert_eq!((gift_in.earnings_delta, gift_out.coins_delta), (210, -3000));
    assert!(gift_in.counterpart.as_option().is_none() && gift_out.counterpart.as_option().is_none());
    assert_eq!(balance(&app, &friend).await.earnings, 210);
    // A gift to someone who left can no longer be refunded.
    assert_eq!(ledger::refund(&app.core, received.entry_id, "live.gift.returned").await.unwrap_err().reason, "REFUND_IMPOSSIBLE");

    // A payment for the dead checkout is refused rather than granted to nobody.
    fakepay.set(&provider_ref(&app, &purchase_id).await, tokuly_wallet::PaymentState::Paid);
    let err = wallet.settle_purchase("fakepay", &provider_ref(&app, &purchase_id).await, tokuly_wallet::PaymentState::Paid).await.unwrap_err();
    assert_eq!(err.reason, "PURCHASE_ORPHANED");

    // Doing it all again changes nothing.
    wallet.handle_event(user_deleted(&leaver)).await.unwrap();
    assert_eq!(postings_sum(&app).await, 0);
    assert_eq!(balance(&app, &friend).await.earnings, 210);
}

#[tokio::test]
async fn deleting_a_user_without_a_wallet_is_a_no_op() {
    let (app, wallet) = app().await;
    let user = app.user("walletless").await;
    wallet.handle_event(user_deleted(&user)).await.unwrap();
    let closures: i64 = sqlx::query_scalar("SELECT count(*) FROM wallet_postings WHERE account LIKE 'user:' || $1 || ':%'").bind(user.id.to_string()).fetch_one(&app.core.db).await.unwrap();
    assert_eq!(closures, 0);
}

#[tokio::test]
async fn the_data_export_contains_the_users_wallet() {
    let (app, _) = app().await;
    let (creator, fan) = (app.user("creator").await, app.user("fan").await);
    fund(&app, &fan, 5000).await;
    ledger::transfer(&app.core, fan.id, creator.id, 3000, 30, "live.gift", &key("gift"), serde_json::json!({ "stream_id": "s9" })).await.unwrap();
    let coins_100 = package(&app, "coins_100").await;
    let _: StartPurchaseResponse = app.call(&format!("{WALLET}/StartPurchase"), Some(&creator), &StartPurchaseRequest { package_id: coins_100.id, ..Default::default() }).await.unwrap();
    let payout = RequestPayoutRequest { coins: 1000, method: PayoutMethod::PAYOUT_METHOD_GIFT_CARD.into(), destination: "creator@example.com".into(), ..Default::default() };
    let _: RequestPayoutResponse = app.call(&format!("{EARNINGS}/RequestPayout"), Some(&creator), &payout).await.unwrap();
    let _: ClaimDailyBonusResponse = app.call(&format!("{WALLET}/ClaimDailyBonus"), Some(&creator), &ClaimDailyBonusRequest::default()).await.unwrap();

    let exporter = app.core.exporters.all().into_iter().find(|exporter| exporter.name() == "wallet").expect("the wallet registers an exporter");
    let export = exporter.export(creator.id).await.unwrap();
    assert_eq!(export["balance"], serde_json::json!({ "coins": 110, "earnings": 1100 }));
    let ledger_entries = export["ledger"].as_array().unwrap();
    assert_eq!(ledger_entries.len(), 4, "gift, purchase, payout hold, bonus");
    let gift = ledger_entries.iter().find(|entry| entry["reason"] == "live.gift").unwrap();
    assert_eq!((gift["earnings_delta"].as_i64(), gift["amount"].as_i64(), gift["fee"].as_i64()), (Some(2100), Some(3000), Some(900)));
    assert_eq!(gift["from_user_id"], fan.id.to_string());
    assert_eq!(gift["memo"]["stream_id"], "s9");
    assert_eq!(export["ledger_truncated"], false);
    assert_eq!(export["purchases"][0]["sku"], "coins_100");
    assert_eq!((export["payout_requests"][0]["coins"].as_i64(), export["payout_requests"][0]["destination"].as_str()), (Some(1000), Some("creator@example.com")));
    assert_eq!(export["daily_bonus"]["streak"], 1);

    // Someone else's export has none of it.
    let other = exporter.export(fan.id).await.unwrap();
    assert_eq!(other["ledger"].as_array().unwrap().len(), 2);
    assert!(other["payout_requests"].as_array().unwrap().is_empty() && other["daily_bonus"].is_null());
}
