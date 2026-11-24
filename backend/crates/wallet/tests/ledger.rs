//! The ledger against a real database: balances, idempotency, refunds.
use tokuly_testkit::TestApp;
use tokuly_wallet::ledger::{self, Balance};

const MIGRATIONS: &[&str] = &["0001", "0100", "0400", "1400"];

fn key(tag: &str) -> String {
    format!("{tag}:{}", tokuly_core::ids::random_base62(10))
}

async fn postings_sum(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT COALESCE(sum(delta), 0)::bigint FROM wallet_postings").fetch_one(&app.core.db).await.unwrap()
}

#[tokio::test]
async fn grant_spend_and_idempotency() {
    let app = TestApp::start(MIGRATIONS, |_, _, router| router).await;
    let user = app.user("spender").await;
    assert_eq!(ledger::balance(&app.core, user.id).await.unwrap(), Balance { coins: 0, earnings: 0 });

    let grant_key = key("grant");
    let (entry, applied) = ledger::grant(&app.core, user.id, 500, "purchase", &grant_key, serde_json::json!({})).await.unwrap();
    assert!(applied);
    let (again, applied_again) = ledger::grant(&app.core, user.id, 500, "purchase", &grant_key, serde_json::json!({})).await.unwrap();
    assert_eq!((again, applied_again), (entry, false), "same key returns the same entry and moves nothing");
    assert_eq!(ledger::balance(&app.core, user.id).await.unwrap().coins, 500);

    let spend_key = key("spend");
    ledger::spend(&app.core, user.id, 120, "stamp.pack", &spend_key, serde_json::json!({ "pack": "x" })).await.unwrap();
    ledger::spend(&app.core, user.id, 120, "stamp.pack", &spend_key, serde_json::json!({})).await.unwrap();
    assert_eq!(ledger::balance(&app.core, user.id).await.unwrap().coins, 380);

    let err = ledger::spend(&app.core, user.id, 381, "stamp.pack", &key("spend"), serde_json::json!({})).await.unwrap_err();
    assert_eq!(err.reason, "INSUFFICIENT_COINS");
    assert_eq!(ledger::balance(&app.core, user.id).await.unwrap().coins, 380, "a failed spend changes nothing");
    assert!(ledger::spend(&app.core, user.id, 0, "x", &key("spend"), serde_json::json!({})).await.is_err());
    assert_eq!(app.events_by("wallet.balance.changed", user.id).await.len(), 2);
}

#[tokio::test]
async fn transfer_takes_fee_and_refund_reverses_everything() {
    let app = TestApp::start(MIGRATIONS, |_, _, router| router).await;
    let (fan, streamer) = (app.user("fan").await, app.user("streamer").await);
    ledger::grant(&app.core, fan.id, 1000, "purchase", &key("grant"), serde_json::json!({})).await.unwrap();

    let gift_key = key("gift");
    let receipt = ledger::transfer(&app.core, fan.id, streamer.id, 300, 30, "live.gift", &gift_key, serde_json::json!({})).await.unwrap();
    assert_eq!((receipt.fee, receipt.net, receipt.applied), (90, 210, true));
    let repeat = ledger::transfer(&app.core, fan.id, streamer.id, 300, 30, "live.gift", &gift_key, serde_json::json!({})).await.unwrap();
    assert_eq!((repeat.entry_id, repeat.applied), (receipt.entry_id, false));
    assert_eq!(ledger::balance(&app.core, fan.id).await.unwrap(), Balance { coins: 700, earnings: 0 });
    assert_eq!(ledger::balance(&app.core, streamer.id).await.unwrap(), Balance { coins: 0, earnings: 210 });

    assert_eq!(ledger::transfer(&app.core, fan.id, fan.id, 1, 0, "x", &key("t"), serde_json::json!({})).await.unwrap_err().reason, "SELF_TRANSFER");
    assert_eq!(ledger::transfer(&app.core, fan.id, streamer.id, 5000, 0, "x", &key("t"), serde_json::json!({})).await.unwrap_err().reason, "INSUFFICIENT_COINS");

    ledger::refund(&app.core, receipt.entry_id, "live.gift.returned").await.unwrap();
    ledger::refund(&app.core, receipt.entry_id, "live.gift.returned").await.unwrap();
    assert_eq!(ledger::balance(&app.core, fan.id).await.unwrap(), Balance { coins: 1000, earnings: 0 });
    assert_eq!(ledger::balance(&app.core, streamer.id).await.unwrap(), Balance { coins: 0, earnings: 0 });
    assert_eq!(postings_sum(&app).await, 0, "the ledger always balances");
}

#[tokio::test]
async fn concurrent_spends_never_overdraw() {
    let app = TestApp::start(MIGRATIONS, |_, _, router| router).await;
    let user = app.user("racer").await;
    ledger::grant(&app.core, user.id, 100, "purchase", &key("grant"), serde_json::json!({})).await.unwrap();
    let attempts = (0..20).map(|_| {
        let core = app.core.clone();
        let k = key("race");
        async move { ledger::spend(&core, user.id, 10, "race", &k, serde_json::json!({})).await.is_ok() }
    });
    let succeeded = futures::future::join_all(attempts).await.into_iter().filter(|ok| *ok).count();
    assert_eq!(succeeded, 10, "exactly the affordable number of spends succeed");
    assert_eq!(ledger::balance(&app.core, user.id).await.unwrap().coins, 0);
}

/// An idempotency key names one movement: one kind, payer, recipient and
/// amount. A call that reuses it for anything else is refused instead of
/// being answered with the first movement as if it were a retry — that
/// would hand the second caller whatever it pays for without paying.
#[tokio::test]
async fn a_key_cannot_be_reused_for_another_movement() {
    let app = TestApp::start(MIGRATIONS, |_, _, router| router).await;
    let (alice, bob, carol) = (app.user("alice").await, app.user("bob").await, app.user("carol").await);
    for user in [&alice, &bob] {
        ledger::grant(&app.core, user.id, 1000, "purchase", &key("grant"), serde_json::json!({})).await.unwrap();
    }
    let reused = |result: Result<(uuid::Uuid, bool), tokuly_core::error::AppError>| result.unwrap_err().reason;

    let spend_key = key("pack");
    let (spent, _) = ledger::spend(&app.core, alice.id, 100, "stamp.pack", &spend_key, serde_json::json!({})).await.unwrap();
    assert_eq!(reused(ledger::spend(&app.core, bob.id, 100, "stamp.pack", &spend_key, serde_json::json!({})).await), "IDEMPOTENCY_KEY_REUSED", "another payer");
    assert_eq!(ledger::balance(&app.core, bob.id).await.unwrap().coins, 1000, "bob was not charged, and got nothing for free");
    assert_eq!(reused(ledger::spend(&app.core, alice.id, 500, "stamp.pack", &spend_key, serde_json::json!({})).await), "IDEMPOTENCY_KEY_REUSED", "another amount");
    assert_eq!(reused(ledger::grant(&app.core, alice.id, 100, "purchase", &spend_key, serde_json::json!({})).await), "IDEMPOTENCY_KEY_REUSED", "another kind");
    assert_eq!(ledger::spend(&app.core, alice.id, 100, "stamp.pack", &spend_key, serde_json::json!({})).await.unwrap(), (spent, false), "a true retry");

    let gift_key = key("gift");
    let gift = ledger::transfer(&app.core, alice.id, carol.id, 300, 30, "live.gift", &gift_key, serde_json::json!({})).await.unwrap();
    let transfer = |from: &tokuly_testkit::TestUser, to: &tokuly_testkit::TestUser, amount: i64, fee_percent: u32| {
        let (core, key, from, to) = (app.core.clone(), gift_key.clone(), from.id, to.id);
        async move { ledger::transfer(&core, from, to, amount, fee_percent, "live.gift", &key, serde_json::json!({})).await }
    };
    assert_eq!(transfer(&alice, &bob, 300, 30).await.unwrap_err().reason, "IDEMPOTENCY_KEY_REUSED", "another recipient");
    assert_eq!(transfer(&bob, &carol, 300, 30).await.unwrap_err().reason, "IDEMPOTENCY_KEY_REUSED", "another payer");
    assert_eq!(transfer(&alice, &carol, 500, 30).await.unwrap_err().reason, "IDEMPOTENCY_KEY_REUSED", "another amount");
    // A retry is answered with what the original moved, even if the fee setting changed since.
    let retry = transfer(&alice, &carol, 300, 0).await.unwrap();
    assert_eq!((retry.entry_id, retry.fee, retry.net, retry.applied), (gift.entry_id, 90, 210, false));

    // Two payers racing with one key: one movement, one refusal.
    let race_key = key("race");
    let (a, b) = tokio::join!(
        ledger::spend(&app.core, alice.id, 1, "race", &race_key, serde_json::json!({})),
        ledger::spend(&app.core, bob.id, 1, "race", &race_key, serde_json::json!({})),
    );
    let refused: Vec<String> = [a, b].into_iter().filter_map(|result| result.err().map(|e| e.reason.into_owned())).collect();
    assert_eq!(refused, vec!["IDEMPOTENCY_KEY_REUSED".to_owned()]);
    let paid = ledger::balance(&app.core, alice.id).await.unwrap().coins + ledger::balance(&app.core, bob.id).await.unwrap().coins;
    assert_eq!(paid, 2000 - 100 - 300 - 1, "exactly one of the two paid");
    assert_eq!(postings_sum(&app).await, 0);
}

/// Once identity has erased an account (`users.deleted_at`), its wallet is
/// closed for good: nothing moves into it or out of it, whether or not the
/// wallet's own clean-up has run yet. Otherwise a late gift or refund
/// re-opens the closed account and strands the coins on a tombstone.
#[tokio::test]
async fn coins_do_not_move_to_or_from_a_deleted_account() {
    let app = TestApp::start(MIGRATIONS, |_, _, router| router).await;
    let (fan, leaver) = (app.user("fan").await, app.user("leaver").await);
    ledger::grant(&app.core, fan.id, 1000, "purchase", &key("grant"), serde_json::json!({})).await.unwrap();
    ledger::grant(&app.core, leaver.id, 1000, "purchase", &key("grant"), serde_json::json!({})).await.unwrap();
    let sent = ledger::transfer(&app.core, leaver.id, fan.id, 100, 0, "live.gift", &key("gift"), serde_json::json!({})).await.unwrap();
    let pack_key = key("pack");
    let (spent, _) = ledger::spend(&app.core, leaver.id, 100, "stamp.pack", &pack_key, serde_json::json!({})).await.unwrap();
    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1").bind(leaver.id).execute(&app.core.db).await.unwrap();
    // A retry of what did happen before is still answered as a retry.
    assert_eq!(ledger::spend(&app.core, leaver.id, 100, "stamp.pack", &pack_key, serde_json::json!({})).await.unwrap(), (spent, false));

    let gift = ledger::transfer(&app.core, fan.id, leaver.id, 300, 30, "live.gift", &key("gift"), serde_json::json!({})).await;
    assert_eq!(gift.unwrap_err().reason, "ACCOUNT_CLOSED", "a gift to a deleted account");
    assert_eq!(ledger::balance(&app.core, fan.id).await.unwrap().coins, 1000, "the fan keeps the coins");
    let grant = ledger::grant(&app.core, leaver.id, 50, "quest.mission", &key("reward"), serde_json::json!({})).await;
    assert_eq!(grant.unwrap_err().reason, "ACCOUNT_CLOSED");
    let spend = ledger::spend(&app.core, leaver.id, 50, "stamp.pack", &key("pack"), serde_json::json!({})).await;
    assert_eq!(spend.unwrap_err().reason, "ACCOUNT_CLOSED");
    // Refunds cannot go back to a deleted payer, or take back what a deleted recipient earned.
    assert_eq!(ledger::refund(&app.core, sent.entry_id, "live.gift.returned").await.unwrap_err().reason, "REFUND_IMPOSSIBLE");
    assert_eq!(ledger::refund(&app.core, spent, "stamp.pack.refund").await.unwrap_err().reason, "REFUND_IMPOSSIBLE");
    assert_eq!(ledger::balance(&app.core, fan.id).await.unwrap(), Balance { coins: 1000, earnings: 100 });
    assert_eq!(ledger::balance(&app.core, leaver.id).await.unwrap(), Balance { coins: 800, earnings: 0 }, "left for the clean-up to forfeit");
    assert_eq!(postings_sum(&app).await, 0);
}

/// Transfers both ways between the same two accounts, and refunds of
/// them, all at once and more of them than the pool has connections: no
/// deadlock, no request left waiting for a connection, and every coin
/// accounted for.
#[tokio::test]
async fn opposite_transfers_and_refunds_at_once_neither_deadlock_nor_starve() {
    let app = TestApp::start(MIGRATIONS, |_, _, router| router).await;
    let (a, b) = (app.user("a").await, app.user("b").await);
    for user in [&a, &b] {
        ledger::grant(&app.core, user.id, 1000, "purchase", &key("grant"), serde_json::json!({})).await.unwrap();
    }
    let earlier = ledger::transfer(&app.core, a.id, b.id, 100, 0, "live.gift", &key("gift"), serde_json::json!({})).await.unwrap();
    let moves = (0..12).map(|i| {
        let core = app.core.clone();
        let (from, to) = if i % 2 == 0 { (a.id, b.id) } else { (b.id, a.id) };
        async move {
            if i % 4 == 3 {
                ledger::refund(&core, earlier.entry_id, "live.gift.returned").await.map(|_| ())
            } else {
                ledger::transfer(&core, from, to, 10, 30, "live.gift", &key("gift"), serde_json::json!({})).await.map(|_| ())
            }
        }
    });
    let results = tokio::time::timeout(std::time::Duration::from_secs(8), futures::future::join_all(moves)).await.expect("no request waits out the pool");
    assert!(results.iter().all(Result::is_ok), "{:?}", results.iter().filter_map(|r| r.as_ref().err()).collect::<Vec<_>>());
    let (a_after, b_after) = (ledger::balance(&app.core, a.id).await.unwrap(), ledger::balance(&app.core, b.id).await.unwrap());
    let fees: i64 = sqlx::query_scalar("SELECT COALESCE(sum(delta), 0)::bigint FROM wallet_postings p JOIN wallet_entries e ON e.id = p.entry_id WHERE p.account = 'system:fees' AND (e.from_user_id IN ($1, $2) OR e.to_user_id IN ($1, $2))")
        .bind(a.id)
        .bind(b.id)
        .fetch_one(&app.core.db)
        .await
        .unwrap();
    assert_eq!(a_after.coins + a_after.earnings + b_after.coins + b_after.earnings + fees, 2000, "every coin is somewhere");
    assert_eq!(postings_sum(&app).await, 0);
}

/// A retry of a movement that was refunded since is not answered "done":
/// the coins are back with the payer, and a caller that took the answer as
/// "paid" would hand out what it sells for nothing.
#[tokio::test]
async fn a_retry_of_a_refunded_movement_is_refused() {
    let app = TestApp::start(MIGRATIONS, |_, _, router| router).await;
    let (fan, streamer) = (app.user("fan").await, app.user("streamer").await);
    ledger::grant(&app.core, fan.id, 1000, "purchase", &key("grant"), serde_json::json!({})).await.unwrap();
    let (pack_key, gift_key) = (key("pack"), key("gift"));
    let (spent, _) = ledger::spend(&app.core, fan.id, 100, "stamp.pack", &pack_key, serde_json::json!({})).await.unwrap();
    let gift = ledger::transfer(&app.core, fan.id, streamer.id, 300, 30, "live.gift", &gift_key, serde_json::json!({})).await.unwrap();
    ledger::refund(&app.core, spent, "stamp.pack.refund").await.unwrap();
    ledger::refund(&app.core, gift.entry_id, "live.gift.returned").await.unwrap();

    let err = ledger::spend(&app.core, fan.id, 100, "stamp.pack", &pack_key, serde_json::json!({})).await.unwrap_err();
    assert_eq!((err.code, err.reason.as_ref()), (connectrpc::ErrorCode::FailedPrecondition, "ENTRY_REFUNDED"));
    let err = ledger::transfer(&app.core, fan.id, streamer.id, 300, 30, "live.gift", &gift_key, serde_json::json!({})).await.unwrap_err();
    assert_eq!(err.reason, "ENTRY_REFUNDED");
    assert_eq!(ledger::balance(&app.core, fan.id).await.unwrap(), Balance { coins: 1000, earnings: 0 }, "nothing moved again");
    // A different movement under the key is still a reuse, refunded or not.
    assert_eq!(ledger::spend(&app.core, streamer.id, 100, "stamp.pack", &pack_key, serde_json::json!({})).await.unwrap_err().reason, "IDEMPOTENCY_KEY_REUSED");
}

#[tokio::test]
async fn a_movement_is_found_by_its_key() {
    let app = TestApp::start(MIGRATIONS, |_, _, router| router).await;
    let (payer, payee) = (app.user("payer").await, app.user("payee").await);
    ledger::grant(&app.core, payer.id, 100, "purchase", &key("grant"), serde_json::json!({})).await.unwrap();
    let transfer_key = key("transfer");
    assert_eq!(ledger::find_by_key(&app.core, &transfer_key).await.unwrap(), None, "nothing moved yet");

    let receipt = ledger::transfer(&app.core, payer.id, payee.id, 100, 30, "stamp.pack", &transfer_key, serde_json::json!({})).await.unwrap();
    let found = ledger::find_by_key(&app.core, &transfer_key).await.unwrap().unwrap();
    assert_eq!((found.entry_id, found.amount, found.fee, found.refunded), (receipt.entry_id, 100, 30, false));

    ledger::refund(&app.core, receipt.entry_id, "stamp.pack.refund").await.unwrap();
    assert!(ledger::find_by_key(&app.core, &transfer_key).await.unwrap().unwrap().refunded);
}
