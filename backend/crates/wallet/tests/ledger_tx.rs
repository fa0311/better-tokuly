//! The transactional ledger operations against a real database: atomicity
//! with the caller's transaction, idempotency, balances that cannot go
//! negative, and postings that always balance.
mod common;

use common::*;
use tokuly_testkit::{TestApp, TestUser};
use tokuly_wallet::{
    ledger::{self, Balance},
    ledger_tx::{self, Account},
};
use uuid::Uuid;

fn no_memo() -> serde_json::Value {
    serde_json::json!({})
}

async fn ledger_balance(app: &TestApp, user: &TestUser) -> Balance {
    ledger::balance(&app.core, user.id).await.unwrap()
}

/// A user with `net` coins of earnings.
async fn creator_with_earnings(app: &TestApp, net: i64) -> TestUser {
    let creator = app.user("creator").await;
    earn(app, &creator, net * 10 / 7, "live.gift").await;
    assert_eq!(ledger_balance(app, &creator).await.earnings, net);
    creator
}

/// Takes a payout hold in its own transaction and returns the hold entry.
async fn hold(app: &TestApp, user: &TestUser, amount: i64, hold_key: &str) -> tokuly_core::AppResult<ledger_tx::Posted> {
    let mut tx = app.core.db.begin().await?;
    let posted = ledger_tx::hold_payout(&mut tx, user.id, amount, hold_key, &no_memo()).await?;
    tx.commit().await?;
    Ok(posted)
}

/// Net postings of `account` within one entry.
async fn posted(app: &TestApp, entry_id: Uuid, account: &str) -> i64 {
    sqlx::query_scalar("SELECT COALESCE(sum(delta), 0)::bigint FROM wallet_postings WHERE entry_id = $1 AND account = $2")
        .bind(entry_id)
        .bind(account)
        .fetch_one(&app.core.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_grant_lives_and_dies_with_the_callers_transaction() {
    let (app, _) = app().await;
    let user = app.user("granted").await;
    let grant_key = key("grant");

    // Rolled back: no coins, no entry, no event.
    let mut tx = app.core.db.begin().await.unwrap();
    let rolled_back = ledger_tx::grant(&mut tx, user.id, 500, "purchase", &grant_key, &no_memo()).await.unwrap();
    assert!(rolled_back.applied);
    tx.rollback().await.unwrap();
    assert_eq!(ledger_balance(&app, &user).await.coins, 0);
    assert!(balance_events(&app, &user).await.is_empty());

    // Committed: coins, a balanced entry and one event.
    let mut tx = app.core.db.begin().await.unwrap();
    let granted = ledger_tx::grant(&mut tx, user.id, 500, "purchase", &grant_key, &no_memo()).await.unwrap();
    tx.commit().await.unwrap();
    assert!(granted.applied);
    assert_eq!(ledger_balance(&app, &user).await.coins, 500);
    assert_eq!(posted(&app, granted.entry_id, "system:issued").await, -500);
    assert_eq!(posted(&app, granted.entry_id, &Account::Coins.of(user.id)).await, 500);
    assert_eq!(balance_events(&app, &user).await.len(), 1);

    // The same key again — through either ledger API — moves nothing.
    let mut tx = app.core.db.begin().await.unwrap();
    let repeated = ledger_tx::grant(&mut tx, user.id, 500, "purchase", &grant_key, &no_memo()).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!((repeated.entry_id, repeated.applied), (granted.entry_id, false));
    assert_eq!(ledger::grant(&app.core, user.id, 500, "purchase", &grant_key, no_memo()).await.unwrap(), (granted.entry_id, false));
    assert_eq!(ledger_balance(&app, &user).await.coins, 500);

    let mut tx = app.core.db.begin().await.unwrap();
    assert_eq!(ledger_tx::grant(&mut tx, user.id, 0, "x", &key("grant"), &no_memo()).await.unwrap_err().reason, "AMOUNT_INVALID");
    assert_eq!(ledger_tx::grant(&mut tx, user.id, 5, "x", "", &no_memo()).await.unwrap_err().reason, "IDEMPOTENCY_KEY_INVALID");
}

#[tokio::test]
async fn a_payout_hold_locks_earnings_until_released() {
    let (app, _) = app().await;
    let creator = creator_with_earnings(&app, 2100).await;
    let hold_key = key("hold");

    let held = hold(&app, &creator, 1500, &hold_key).await.unwrap();
    assert!(held.applied);
    assert_eq!(ledger_balance(&app, &creator).await, Balance { coins: 0, earnings: 600 });
    assert_eq!(posted(&app, held.entry_id, &Account::Earnings.of(creator.id)).await, -1500);
    assert_eq!(posted(&app, held.entry_id, "system:payout_hold").await, 1500);
    let repeated = hold(&app, &creator, 1500, &hold_key).await.unwrap();
    assert_eq!((repeated.entry_id, repeated.applied), (held.entry_id, false));
    assert_eq!(ledger_balance(&app, &creator).await.earnings, 600);

    // Earnings cannot be held twice, and spendable coins are not earnings.
    assert_eq!(hold(&app, &creator, 601, &key("hold")).await.unwrap_err().reason, "INSUFFICIENT_EARNINGS");
    fund(&app, &creator, 5000).await;
    assert_eq!(hold(&app, &creator, 601, &key("hold")).await.unwrap_err().reason, "INSUFFICIENT_EARNINGS");
    assert_eq!(hold(&app, &creator, 0, &key("hold")).await.unwrap_err().reason, "AMOUNT_INVALID");

    // Releasing puts the coins back, once.
    let mut tx = app.core.db.begin().await.unwrap();
    let refund = ledger_tx::release_payout(&mut tx, held.entry_id, "payout.rejected").await.unwrap().expect("first release posts a refund");
    tx.commit().await.unwrap();
    assert_eq!(ledger_balance(&app, &creator).await.earnings, 2100);
    assert_eq!(posted(&app, refund, "system:payout_hold").await, -1500);
    assert_eq!(posted(&app, refund, &Account::Earnings.of(creator.id)).await, 1500);
    let (kind, refund_of): (String, Option<Uuid>) = sqlx::query_as("SELECT kind, refund_of FROM wallet_entries WHERE id = $1").bind(refund).fetch_one(&app.core.db).await.unwrap();
    assert_eq!((kind.as_str(), refund_of), ("refund", Some(held.entry_id)));
    let mut tx = app.core.db.begin().await.unwrap();
    assert_eq!(ledger_tx::release_payout(&mut tx, held.entry_id, "payout.rejected").await.unwrap(), None, "a second release is a no-op");
    // A returned hold can no longer be paid out.
    assert_eq!(ledger_tx::settle_payout(&mut tx, held.entry_id, &no_memo()).await.unwrap_err().reason, "PAYOUT_ALREADY_RETURNED");
    drop(tx);
    assert_eq!(ledger_balance(&app, &creator).await.earnings, 2100);
    assert_eq!(unbalanced_entries(&app).await, 0);
}

#[tokio::test]
async fn settling_a_hold_moves_it_to_paid_out_for_good() {
    let (app, _) = app().await;
    let creator = creator_with_earnings(&app, 2100).await;
    let held = hold(&app, &creator, 2000, &key("hold")).await.unwrap();

    let mut tx = app.core.db.begin().await.unwrap();
    let settled = ledger_tx::settle_payout(&mut tx, held.entry_id, &no_memo()).await.unwrap();
    tx.commit().await.unwrap();
    assert!(settled.applied);
    assert_eq!(posted(&app, settled.entry_id, "system:payout_hold").await, -2000);
    assert_eq!(posted(&app, settled.entry_id, "system:paid_out").await, 2000);
    assert_eq!(ledger_balance(&app, &creator).await.earnings, 100, "settling touches no user balance");
    let (from, to): (Option<Uuid>, Option<Uuid>) = sqlx::query_as("SELECT from_user_id, to_user_id FROM wallet_entries WHERE id = $1").bind(settled.entry_id).fetch_one(&app.core.db).await.unwrap();
    assert_eq!((from, to), (None, None), "the settlement is not part of anyone's history");

    let mut tx = app.core.db.begin().await.unwrap();
    let again = ledger_tx::settle_payout(&mut tx, held.entry_id, &no_memo()).await.unwrap();
    assert_eq!((again.entry_id, again.applied), (settled.entry_id, false));
    // Money that was sent cannot flow back into earnings.
    assert_eq!(ledger_tx::release_payout(&mut tx, held.entry_id, "payout.rejected").await.unwrap_err().reason, "PAYOUT_ALREADY_SETTLED");
    drop(tx);

    // Only payout holds can be released or settled.
    let (_, gift) = earn(&app, &creator, 100, "live.gift").await;
    let mut tx = app.core.db.begin().await.unwrap();
    assert_eq!(ledger_tx::release_payout(&mut tx, gift, "x").await.unwrap_err().reason, "NOT_A_PAYOUT_HOLD");
    assert_eq!(ledger_tx::settle_payout(&mut tx, gift, &no_memo()).await.unwrap_err().reason, "NOT_A_PAYOUT_HOLD");
    assert_eq!(ledger_tx::settle_payout(&mut tx, Uuid::now_v7(), &no_memo()).await.unwrap_err().reason, "ENTRY_NOT_FOUND");
    drop(tx);
    assert_eq!(unbalanced_entries(&app).await, 0);
}

#[tokio::test]
async fn concurrent_holds_never_overdraw_earnings() {
    let (app, _) = app().await;
    let creator = creator_with_earnings(&app, 700).await;
    let attempts = (0..12).map(|_| {
        let app = &app;
        let creator = &creator;
        async move { hold(app, creator, 300, &key("race")).await.is_ok() }
    });
    let succeeded = futures::future::join_all(attempts).await.into_iter().filter(|ok| *ok).count();
    assert_eq!(succeeded, 2, "exactly the affordable number of holds succeed");
    assert_eq!(ledger_balance(&app, &creator).await.earnings, 100);
}

#[tokio::test]
async fn a_hold_and_its_release_racing_a_settlement_end_in_one_outcome() {
    let (app, _) = app().await;
    let creator = creator_with_earnings(&app, 700).await;
    let held = hold(&app, &creator, 700, &key("hold")).await.unwrap();
    let release = async {
        let mut tx = app.core.db.begin().await?;
        ledger_tx::release_payout(&mut tx, held.entry_id, "payout.rejected").await?;
        tx.commit().await?;
        tokuly_core::AppResult::Ok(())
    };
    let settle = async {
        let mut tx = app.core.db.begin().await?;
        ledger_tx::settle_payout(&mut tx, held.entry_id, &no_memo()).await?;
        tx.commit().await?;
        tokuly_core::AppResult::Ok(())
    };
    let (released, settled) = futures::future::join(release, settle).await;
    assert!(released.is_ok() != settled.is_ok(), "exactly one of them wins: {released:?} / {settled:?}");
    let earnings = ledger_balance(&app, &creator).await.earnings;
    assert_eq!(earnings, if released.is_ok() { 700 } else { 0 });
    assert_eq!(unbalanced_entries(&app).await, 0);
}

#[tokio::test]
async fn adjustments_credit_and_debit_either_balance() {
    let (app, _) = app().await;
    let user = app.user("adjusted").await;
    let adjust = |account: Account, delta: i64, adjust_key: String| {
        let app = &app;
        let user = &user;
        async move {
            let mut tx = app.core.db.begin().await?;
            let posted = ledger_tx::adjust(&mut tx, user.id, account, delta, &adjust_key, &no_memo()).await?;
            tx.commit().await?;
            tokuly_core::AppResult::Ok(posted)
        }
    };

    let credit_key = key("adj");
    let credit = adjust(Account::Coins, 400, credit_key.clone()).await.unwrap();
    assert_eq!(posted(&app, credit.entry_id, "system:adjustments").await, -400);
    assert_eq!(adjust(Account::Coins, 400, credit_key).await.unwrap(), ledger_tx::Posted { entry_id: credit.entry_id, applied: false });
    let debit = adjust(Account::Coins, -150, key("adj")).await.unwrap();
    assert_eq!(posted(&app, debit.entry_id, "system:adjustments").await, 150);
    adjust(Account::Earnings, 90, key("adj")).await.unwrap();
    assert_eq!(ledger_balance(&app, &user).await, Balance { coins: 250, earnings: 90 });

    assert_eq!(adjust(Account::Coins, -251, key("adj")).await.unwrap_err().reason, "INSUFFICIENT_COINS");
    assert_eq!(adjust(Account::Earnings, -91, key("adj")).await.unwrap_err().reason, "INSUFFICIENT_EARNINGS");
    assert_eq!(adjust(Account::Coins, 0, key("adj")).await.unwrap_err().reason, "AMOUNT_INVALID");
    assert_eq!(adjust(Account::Coins, i64::MIN, key("adj")).await.unwrap_err().reason, "AMOUNT_INVALID");
    assert_eq!(adjust(Account::Coins, 100_000_001, key("adj")).await.unwrap_err().reason, "AMOUNT_INVALID");
    assert_eq!(ledger_balance(&app, &user).await, Balance { coins: 250, earnings: 90 }, "failed adjustments change nothing");

    // Debits and credits name the user on the right side of the entry.
    let sides = |entry_id: Uuid| {
        let app = &app;
        async move { sqlx::query_as::<_, (Option<Uuid>, Option<Uuid>)>("SELECT from_user_id, to_user_id FROM wallet_entries WHERE id = $1").bind(entry_id).fetch_one(&app.core.db).await.unwrap() }
    };
    assert_eq!(sides(credit.entry_id).await, (None, Some(user.id)));
    assert_eq!(sides(debit.entry_id).await, (Some(user.id), None));
    assert_eq!(unbalanced_entries(&app).await, 0);
}

#[tokio::test]
async fn closing_an_account_forfeits_what_is_left() {
    let (app, _) = app().await;
    let leaver = creator_with_earnings(&app, 700).await;
    fund(&app, &leaver, 300).await;
    let held = hold(&app, &leaver, 500, &key("hold")).await.unwrap();

    let mut tx = app.core.db.begin().await.unwrap();
    ledger_tx::unlink_user(&mut tx, leaver.id).await.unwrap();
    // The hold lost its owner, so releasing it forfeits the coins.
    let forfeited_hold = ledger_tx::release_payout(&mut tx, held.entry_id, "payout.cancelled").await.unwrap().unwrap();
    let closing = ledger_tx::close_account(&mut tx, leaver.id).await.unwrap().expect("a balance was left");
    tx.commit().await.unwrap();

    assert_eq!(posted(&app, forfeited_hold, "system:forfeited").await, 500);
    assert_eq!(posted(&app, closing, "system:forfeited").await, 500, "300 coins + 200 earnings");
    assert_eq!(posted(&app, closing, &Account::Coins.of(leaver.id)).await, -300);
    assert_eq!(posted(&app, closing, &Account::Earnings.of(leaver.id)).await, -200);
    assert_eq!(account_sum(&app, &Account::Coins.of(leaver.id)).await, 0);
    assert_eq!(account_sum(&app, &Account::Earnings.of(leaver.id)).await, 0);
    assert_eq!(ledger_balance(&app, &leaver).await, Balance { coins: 0, earnings: 0 });
    let linked: i64 = sqlx::query_scalar("SELECT count(*) FROM wallet_entries WHERE from_user_id = $1 OR to_user_id = $1").bind(leaver.id).fetch_one(&app.core.db).await.unwrap();
    assert_eq!(linked, 0);

    // Nothing left to close the second time; an account that never existed is fine too.
    let mut tx = app.core.db.begin().await.unwrap();
    assert_eq!(ledger_tx::close_account(&mut tx, leaver.id).await.unwrap(), None);
    assert_eq!(ledger_tx::close_account(&mut tx, Uuid::now_v7()).await.unwrap(), None);
    tx.commit().await.unwrap();
    assert_eq!(unbalanced_entries(&app).await, 0);
    assert_eq!(postings_sum(&app).await, 0);
}
