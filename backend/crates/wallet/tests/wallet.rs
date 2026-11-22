//! `WalletService`: balance and ledger history.
mod common;

use common::*;
use tokuly_proto::{
    common::PageRequest,
    wallet::*,
};
use tokuly_wallet::ledger;

#[tokio::test]
async fn balance_starts_at_zero_and_requires_sign_in() {
    let (app, _) = app().await;
    let err = app.call::<_, GetBalanceResponse>(&format!("{WALLET}/GetBalance"), None, &GetBalanceRequest::default()).await.unwrap_err();
    assert_eq!((err.status, err.reason.as_str()), (401, "SIGN_IN_REQUIRED"));

    let user = app.user("newbie").await;
    let empty = balance(&app, &user).await;
    assert_eq!((empty.coins, empty.earnings, empty.revision), (0, 0, 0));
}

#[tokio::test]
async fn balance_revision_grows_with_every_movement() {
    let (app, _) = app().await;
    let user = app.user("rev").await;
    fund(&app, &user, 500).await;
    let first = balance(&app, &user).await;
    assert_eq!(first.coins, 500);
    assert!(first.revision > 0);

    ledger::spend(&app.core, user.id, 120, "stamp.pack", &key("spend"), serde_json::json!({})).await.unwrap();
    let second = balance(&app, &user).await;
    assert_eq!(second.coins, 380);
    assert!(second.revision > first.revision, "a newer snapshot always carries a higher revision");
}

#[tokio::test]
async fn history_shows_each_side_its_own_view() {
    let (app, _) = app().await;
    let (fan, streamer) = (app.user("fan").await, app.user("streamer").await);
    fund(&app, &fan, 1000).await;
    ledger::spend(&app.core, fan.id, 120, "stamp.pack", &key("spend"), serde_json::json!({ "pack_id": "p1" })).await.unwrap();
    let gift = ledger::transfer(&app.core, fan.id, streamer.id, 300, 30, "live.gift", &key("gift"), serde_json::json!({ "stream_id": "s1" })).await.unwrap();

    // The fan: newest first, coins going out, the streamer named on the gift.
    let entries = history(&app, &fan).await;
    assert_eq!(entries.len(), 3);
    let sent = &entries[0];
    assert_eq!(sent.id, gift.entry_id.to_string());
    assert_eq!(sent.kind.as_known(), Some(EntryKind::ENTRY_KIND_TRANSFER));
    assert_eq!((sent.reason.as_str(), sent.description.as_str()), ("live.gift", "ライブギフトを送りました"));
    assert_eq!((sent.coins_delta, sent.earnings_delta, sent.amount, sent.fee), (-300, 0, 300, 90));
    assert_eq!(sent.counterpart.as_option().unwrap().id, streamer.id.to_string());
    assert_eq!(sent.memo_json, r#"{"stream_id":"s1"}"#);
    assert!(!sent.refunded);
    assert_eq!((entries[1].kind.as_known(), entries[1].coins_delta), (Some(EntryKind::ENTRY_KIND_SPEND), -120));
    assert!(entries[1].counterpart.as_option().is_none(), "spending is between the user and the platform");
    assert_eq!((entries[2].kind.as_known(), entries[2].coins_delta), (Some(EntryKind::ENTRY_KIND_GRANT), 1000));

    // The streamer: the same entry, as earnings net of the fee, from the fan.
    let entries = history(&app, &streamer).await;
    assert_eq!(entries.len(), 1);
    let received = &entries[0];
    assert_eq!(received.id, gift.entry_id.to_string());
    assert_eq!(received.description, "ライブギフトを受け取りました");
    assert_eq!((received.coins_delta, received.earnings_delta, received.amount, received.fee), (0, 210, 300, 90));
    assert_eq!(received.counterpart.as_option().unwrap().handle, fan.handle);

    // A refund marks the original and adds a reversing entry on both sides.
    ledger::refund(&app.core, gift.entry_id, "live.gift.returned").await.unwrap();
    let entries = history(&app, &fan).await;
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0].kind.as_known(), Some(EntryKind::ENTRY_KIND_REFUND));
    assert_eq!((entries[0].coins_delta, entries[0].refund_of_entry_id.as_str(), entries[0].description.as_str()), (300, gift.entry_id.to_string().as_str(), "返金されました"));
    assert!(entries[1].refunded);
    let entries = history(&app, &streamer).await;
    assert_eq!((entries[0].earnings_delta, entries[0].description.as_str()), (-210, "返金のため売上が取り消されました"));
}

#[tokio::test]
async fn history_is_worded_in_the_users_language() {
    let (app, _) = app().await;
    let user = app.user("english").await;
    sqlx::query("UPDATE users SET locale = 'en-US' WHERE id = $1").bind(user.id).execute(&app.core.db).await.unwrap();
    fund(&app, &user, 100).await;
    ledger::spend(&app.core, user.id, 40, "video.rental", &key("spend"), serde_json::json!({})).await.unwrap();
    let entries = history(&app, &user).await;
    assert_eq!(entries[0].description, "Coins spent", "unknown reasons fall back to the kind");
    assert_eq!(entries[1].description, "Coins received");
}

#[tokio::test]
async fn history_filters_by_account_and_paginates() {
    let (app, _) = app().await;
    let creator = app.user("creator").await;
    fund(&app, &creator, 100).await;
    for _ in 0..3 {
        earn(&app, &creator, 100, "live.gift").await;
    }
    let list = |account: LedgerAccount, page: Option<PageRequest>| {
        let app = &app;
        let creator = &creator;
        async move {
            let request = ListLedgerEntriesRequest { account: account.into(), page: page.into(), ..Default::default() };
            app.call::<_, ListLedgerEntriesResponse>(&format!("{WALLET}/ListLedgerEntries"), Some(creator), &request).await
        }
    };

    let coins = list(LedgerAccount::LEDGER_ACCOUNT_COINS, None).await.unwrap();
    assert_eq!(coins.entries.len(), 1);
    assert_eq!(coins.entries[0].coins_delta, 100);
    let earnings = list(LedgerAccount::LEDGER_ACCOUNT_EARNINGS, None).await.unwrap();
    assert_eq!(earnings.entries.len(), 3);
    assert!(earnings.entries.iter().all(|entry| entry.earnings_delta == 70));

    // Two pages of two cover all four entries without overlap, newest first.
    let first = list(LedgerAccount::LEDGER_ACCOUNT_UNSPECIFIED, Some(PageRequest { limit: 2, ..Default::default() })).await.unwrap();
    let info = first.page.as_option().unwrap();
    assert_eq!(first.entries.len(), 2);
    assert!(info.has_more && !info.next_cursor.is_empty());
    let second = list(LedgerAccount::LEDGER_ACCOUNT_UNSPECIFIED, Some(PageRequest { limit: 2, cursor: info.next_cursor.clone(), ..Default::default() })).await.unwrap();
    assert_eq!(second.entries.len(), 2);
    assert!(!second.page.as_option().unwrap().has_more);
    let mut ids: Vec<String> = first.entries.iter().chain(&second.entries).map(|entry| entry.id.clone()).collect();
    assert!(ids.is_sorted_by(|a, b| a > b), "newest first across pages");
    ids.dedup();
    assert_eq!(ids.len(), 4);
    assert_eq!(second.entries[1].kind.as_known(), Some(EntryKind::ENTRY_KIND_GRANT), "the oldest entry comes last");

    let err = list(LedgerAccount::LEDGER_ACCOUNT_UNSPECIFIED, Some(PageRequest { cursor: "garbage!".into(), ..Default::default() })).await.unwrap_err();
    assert_eq!(err.reason, "INVALID_CURSOR");
    let err = app.call::<_, ListLedgerEntriesResponse>(&format!("{WALLET}/ListLedgerEntries"), None, &ListLedgerEntriesRequest::default()).await.unwrap_err();
    assert_eq!(err.reason, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn transfer_with_fee_uses_the_configured_percentage() {
    let (app, wallet) = app().await;
    let (fan, creator) = (app.user("fan").await, app.user("creator").await);
    fund(&app, &fan, 1000).await;
    assert_eq!(wallet.platform_fee_percent(), 30);
    let receipt = wallet.transfer_with_fee(fan.id, creator.id, 200, "stamp.pack", &key("sale"), serde_json::json!({})).await.unwrap();
    assert_eq!((receipt.fee, receipt.net, receipt.applied), (60, 140, true));
    assert_eq!(balance(&app, &creator).await.earnings, 140);
}

#[tokio::test]
async fn an_entry_touching_both_balances_is_listed_once() {
    let (app, _) = app().await;
    let creator = app.user("both").await;
    fund(&app, &creator, 100).await;
    earn(&app, &creator, 100, "live.gift").await;
    // No wallet operation moves coins between a user's own balances today;
    // post such an entry by hand to pin down how history treats one.
    let (coins_account, earnings_account) = (format!("user:{}:coins", creator.id), format!("user:{}:earnings", creator.id));
    let both: uuid::Uuid = sqlx::query_scalar(
        "WITH entry AS (
             INSERT INTO wallet_entries (kind, reason, idempotency_key, from_user_id, to_user_id, amount)
             VALUES ('adjustment', 'test.convert', $1, $2, $2, 50) RETURNING id
         ), postings AS (
             INSERT INTO wallet_postings (entry_id, account, delta)
             SELECT entry.id, account, delta FROM entry, (VALUES ($3, -50::bigint), ($4, 50::bigint)) AS p (account, delta)
         )
         SELECT id FROM entry",
    )
    .bind(key("convert"))
    .bind(creator.id)
    .bind(&earnings_account)
    .bind(&coins_account)
    .fetch_one(&app.core.db)
    .await
    .unwrap();
    fund(&app, &creator, 7).await;

    let page_through = |account: LedgerAccount| {
        let app = &app;
        let creator = &creator;
        async move {
            let (mut entries, mut cursor) = (Vec::new(), String::new());
            loop {
                let request = ListLedgerEntriesRequest { account: account.into(), page: Some(PageRequest { limit: 1, cursor, ..Default::default() }).into(), ..Default::default() };
                let page: ListLedgerEntriesResponse = app.call(&format!("{WALLET}/ListLedgerEntries"), Some(creator), &request).await.unwrap();
                entries.extend(page.entries);
                let info = page.page.into_option().unwrap();
                if !info.has_more {
                    return entries;
                }
                cursor = info.next_cursor;
            }
        }
    };

    // One line per entry, however the pages fall, with both deltas on it.
    let all = page_through(LedgerAccount::LEDGER_ACCOUNT_UNSPECIFIED).await;
    assert_eq!(all.len(), 4);
    assert_eq!(all.iter().filter(|entry| entry.id == both.to_string()).count(), 1);
    assert_eq!(all[0].coins_delta, 7, "newest first");
    assert_eq!((all[1].id.as_str(), all[1].coins_delta, all[1].earnings_delta), (both.to_string().as_str(), 50, -50));

    // It belongs to the history of either balance.
    let coins = page_through(LedgerAccount::LEDGER_ACCOUNT_COINS).await;
    assert_eq!(coins.iter().map(|entry| entry.coins_delta).collect::<Vec<_>>(), vec![7, 50, 100]);
    let earnings = page_through(LedgerAccount::LEDGER_ACCOUNT_EARNINGS).await;
    assert_eq!(earnings.iter().map(|entry| entry.earnings_delta).collect::<Vec<_>>(), vec![-50, 70]);
}

/// A transfer to a hidden recipient (a gift to a channel pays the channel's
/// owner): the payer never learns which account was paid, in the history,
/// in the export or through the refund; the recipient sees the payer.
#[tokio::test]
async fn a_hidden_recipient_is_not_shown_to_the_payer() {
    let (app, wallet) = app().await;
    let (fan, streamer) = (app.user("fan").await, app.user("streamer").await);
    fund(&app, &fan, 1000).await;
    let memo = serde_json::json!({ "channel_id": "c1", "stream_id": "s1" });
    let gift_key = key("gift");
    let gift = wallet
        .transfer_with_fee_hiding_recipient(fan.id, streamer.id, 300, "live.gift", &gift_key, memo.clone())
        .await
        .unwrap();
    assert!(gift.applied);
    // Idempotent like any transfer.
    let again = wallet
        .transfer_with_fee_hiding_recipient(fan.id, streamer.id, 300, "live.gift", &gift_key, memo)
        .await
        .unwrap();
    assert_eq!((again.entry_id, again.applied), (gift.entry_id, false));
    assert_eq!(balance(&app, &fan).await.coins, 700);

    let sent = &history(&app, &fan).await[0];
    assert_eq!(sent.id, gift.entry_id.to_string());
    assert!(sent.counterpart.as_option().is_none(), "the payer is not told which account was paid");
    assert!(sent.counterpart_hidden);
    assert!(sent.memo_json.contains("c1"), "what was paid is in the memo");
    let streamer_id = streamer.id.to_string();
    assert!(!serde_json::to_string(sent).unwrap().contains(&streamer_id));
    let received = &history(&app, &streamer).await[0];
    assert_eq!(received.counterpart.as_option().unwrap().id, fan.id.to_string());
    assert!(!received.counterpart_hidden);

    // The refund runs the other way and keeps the same account hidden.
    ledger::refund(&app.core, gift.entry_id, "live.gift.returned").await.unwrap();
    let entries = history(&app, &fan).await;
    assert!(entries[0].counterpart.as_option().is_none() && entries[0].counterpart_hidden);
    assert_eq!(entries[0].coins_delta, 300);
    assert_eq!(history(&app, &streamer).await[0].counterpart.as_option().unwrap().id, fan.id.to_string());
    assert_eq!(balance(&app, &fan).await.coins, 1000);

    // Nor does the payer's data export name the account.
    let exporter = app.core.exporters.all().into_iter().find(|exporter| exporter.name() == "wallet").unwrap();
    let export = exporter.export(fan.id).await.unwrap().to_string();
    assert!(!export.contains(&streamer_id), "the export leaks the hidden recipient");
    assert!(exporter.export(streamer.id).await.unwrap().to_string().contains(&fan.id.to_string()));
    // An ordinary transfer still names both sides.
    let plain =
        ledger::transfer(&app.core, fan.id, streamer.id, 100, 30, "stamp.pack", &key("sale"), serde_json::json!({}))
            .await
            .unwrap();
    let entry = history(&app, &fan).await.into_iter().find(|entry| entry.id == plain.entry_id.to_string()).unwrap();
    assert_eq!(entry.counterpart.as_option().unwrap().id, streamer_id);
    assert!(!entry.counterpart_hidden);
}
