//! The monthly spending limit: under 18 at most ¥5,000 a month on coin
//! purchases and coins sent to others together; anyone may set a lower
//! limit of their own, minors never above the ceiling.
mod common;

use common::*;
use tokuly_core::prelude::AppResult;
use tokuly_proto::wallet::*;
use tokuly_testkit::{TestApp, TestUser};
use tokuly_wallet::{PaymentState, ledger};

const CEILING: i64 = 5000;

/// Makes `user` 15 years old today.
async fn make_minor(app: &TestApp, user: &TestUser) {
    set_birthdate(app, user, "current_date - interval '15 years'").await;
}

/// Gives `user` the birthdate the SQL expression computes (or `NULL`).
async fn set_birthdate(app: &TestApp, user: &TestUser, expression: &str) {
    let sql = format!("UPDATE users SET birthdate = {expression} WHERE id = $1");
    sqlx::query(sqlx::AssertSqlSafe(sql)).bind(user.id).execute(&app.core.db).await.unwrap();
}

async fn minor(app: &TestApp, name: &str) -> TestUser {
    let user = app.user(name).await;
    make_minor(app, &user).await;
    user
}

async fn start(app: &TestApp, user: &TestUser, sku: &str, provider: &str) -> Result<Purchase, tokuly_testkit::RpcError> {
    let package = package(app, sku).await;
    let request = StartPurchaseRequest { package_id: package.id, provider: provider.to_owned(), idempotency_key: key("buy"), ..Default::default() };
    let response: StartPurchaseResponse = app.call(&format!("{WALLET}/StartPurchase"), Some(user), &request).await?;
    Ok(response.purchase.into_option().unwrap())
}

async fn gift(app: &TestApp, from: &TestUser, to: &TestUser, coins: i64) -> AppResult<ledger::TransferReceipt> {
    ledger::transfer(&app.core, from.id, to.id, coins, 30, "live.gift", &key("gift"), serde_json::json!({})).await
}

async fn limit(app: &TestApp, user: &TestUser) -> SpendingLimit {
    let response: GetSpendingLimitResponse = app.call(&format!("{WALLET}/GetSpendingLimit"), Some(user), &GetSpendingLimitRequest::default()).await.unwrap();
    response.limit.into_option().unwrap()
}

async fn set_limit(app: &TestApp, user: &TestUser, yen: Option<i64>) -> Result<SpendingLimit, tokuly_testkit::RpcError> {
    let request = SetSpendingLimitRequest { limit_yen: yen, ..Default::default() };
    let response: SetSpendingLimitResponse = app.call(&format!("{WALLET}/SetSpendingLimit"), Some(user), &request).await?;
    Ok(response.limit.into_option().unwrap())
}

async fn purchases_of(app: &TestApp, user: &TestUser) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM wallet_purchases WHERE user_id = $1").bind(user.id).fetch_one(&app.core.db).await.unwrap()
}

#[tokio::test]
async fn a_minor_buys_up_to_the_ceiling_and_no_further() {
    let (app, _) = app().await;
    let buyer = minor(&app, "buyer").await;
    assert_eq!(start(&app, &buyer, "coins_3000", "").await.unwrap().status.as_known(), Some(PurchaseStatus::PURCHASE_STATUS_COMPLETED));
    let refused = start(&app, &buyer, "coins_3000", "").await.unwrap_err();
    assert_eq!((refused.code.as_str(), refused.reason.as_str()), ("failed_precondition", "SPENDING_LIMIT_REACHED"));
    assert_eq!(purchases_of(&app, &buyer).await, 1, "a refused purchase leaves no row and no checkout");
    start(&app, &buyer, "coins_1000", "").await.unwrap();
    start(&app, &buyer, "coins_1000", "").await.unwrap();
    assert_eq!(start(&app, &buyer, "coins_100", "").await.unwrap_err().reason, "SPENDING_LIMIT_REACHED", "¥5,000 is used up");
    assert_eq!(balance(&app, &buyer).await.coins, 3240 + 1060 * 2);

    let shown = limit(&app, &buyer).await;
    assert_eq!((shown.ceiling_yen, shown.limit_yen, shown.spent_yen, shown.remaining_yen), (Some(CEILING), Some(CEILING), 5000, Some(0)));
}

#[tokio::test]
async fn an_account_without_a_birthdate_counts_as_a_minor() {
    let (app, _) = app().await;
    let buyer = app.user("buyer").await;
    set_birthdate(&app, &buyer, "NULL").await;
    assert_eq!(start(&app, &buyer, "coins_10000", "").await.unwrap_err().reason, "SPENDING_LIMIT_REACHED");
    assert_eq!(limit(&app, &buyer).await.ceiling_yen, Some(CEILING));
}

#[tokio::test]
async fn gifts_and_purchases_count_together() {
    let (app, wallet) = app().await;
    let (fan, creator) = (minor(&app, "fan").await, app.user("creator").await);
    start(&app, &fan, "coins_3000", "").await.unwrap();
    gift(&app, &fan, &creator, 2000).await.unwrap();
    let over = gift(&app, &fan, &creator, 1).await.unwrap_err();
    assert_eq!(over.reason, "SPENDING_LIMIT_REACHED");
    let hidden = wallet.transfer_with_fee_hiding_recipient(fan.id, creator.id, 1, "live.gift", &key("gift"), serde_json::json!({})).await.unwrap_err();
    assert_eq!(hidden.reason, "SPENDING_LIMIT_REACHED");
    assert_eq!(start(&app, &fan, "coins_100", "").await.unwrap_err().reason, "SPENDING_LIMIT_REACHED");
    assert_eq!(balance(&app, &fan).await.coins, 3240 - 2000, "nothing moved on a refusal");
    assert_eq!(limit(&app, &fan).await.spent_yen, 5000);
}

#[tokio::test]
async fn bonuses_and_platform_spends_are_not_spending() {
    let (app, _) = app().await;
    let (fan, creator) = (minor(&app, "fan").await, app.user("creator").await);
    fund(&app, &fan, 20_000).await;
    ledger::spend(&app.core, fan.id, 3000, "stamp.pack", &key("pack"), serde_json::json!({})).await.unwrap();
    assert_eq!(limit(&app, &fan).await.spent_yen, 0);
    gift(&app, &fan, &creator, 5000).await.unwrap();
}

#[tokio::test]
async fn refunds_and_unpaid_purchases_stop_counting() {
    let (app, wallet) = app().await;
    let fakepay = with_fakepay(&wallet);
    let (fan, creator) = (minor(&app, "fan").await, app.user("creator").await);
    fund(&app, &fan, 20_000).await;
    let first = gift(&app, &fan, &creator, 5000).await.unwrap();
    assert_eq!(gift(&app, &fan, &creator, 1).await.unwrap_err().reason, "SPENDING_LIMIT_REACHED");
    ledger::refund(&app.core, first.entry_id, "live.gift.returned").await.unwrap();
    assert_eq!(limit(&app, &fan).await.spent_yen, 0);

    // A purchase counts from its start, while it waits for the payment.
    let pending = start(&app, &fan, "coins_5000", FakePay::NAME).await.unwrap();
    assert_eq!(limit(&app, &fan).await.spent_yen, 5000);
    assert_eq!(gift(&app, &fan, &creator, 1).await.unwrap_err().reason, "SPENDING_LIMIT_REACHED");
    fakepay.set(&provider_ref(&app, &pending.id).await, PaymentState::Failed { reason: "card_declined".into() });
    let request = ConfirmPurchaseRequest { purchase_id: pending.id.clone(), ..Default::default() };
    let _: ConfirmPurchaseResponse = app.call(&format!("{WALLET}/ConfirmPurchase"), Some(&fan), &request).await.unwrap();
    assert_eq!(limit(&app, &fan).await.spent_yen, 0, "a failed purchase is not spending");
    gift(&app, &fan, &creator, 5000).await.unwrap();
}

/// Money taken at the provider always becomes coins, even when the payment
/// arrives after the purchase failed and the limit was used up since.
#[tokio::test]
async fn a_purchase_paid_at_the_provider_is_never_refused() {
    let (app, wallet) = app().await;
    let fakepay = with_fakepay(&wallet);
    let (fan, creator) = (minor(&app, "fan").await, app.user("creator").await);
    let purchase = start(&app, &fan, "coins_5000", FakePay::NAME).await.unwrap();
    let reference = provider_ref(&app, &purchase.id).await;
    fakepay.set(&reference, PaymentState::Failed { reason: "timeout".into() });
    let request = ConfirmPurchaseRequest { purchase_id: purchase.id.clone(), ..Default::default() };
    let _: ConfirmPurchaseResponse = app.call(&format!("{WALLET}/ConfirmPurchase"), Some(&fan), &request).await.unwrap();
    fund(&app, &fan, 5000).await;
    gift(&app, &fan, &creator, 5000).await.unwrap();

    fakepay.set(&reference, PaymentState::Paid);
    let confirmed: ConfirmPurchaseResponse = app.call(&format!("{WALLET}/ConfirmPurchase"), Some(&fan), &request).await.unwrap();
    assert_eq!(confirmed.purchase.as_option().unwrap().status.as_known(), Some(PurchaseStatus::PURCHASE_STATUS_COMPLETED));
    assert_eq!(limit(&app, &fan).await.spent_yen, 10_000, "it counts, over the limit");
}

#[tokio::test]
async fn a_retried_start_and_a_retried_gift_are_answered_as_retries() {
    let (app, _) = app().await;
    let (fan, creator) = (minor(&app, "fan").await, app.user("creator").await);
    let package = package(&app, "coins_5000").await;
    let request = StartPurchaseRequest { package_id: package.id, idempotency_key: key("buy"), ..Default::default() };
    let first: StartPurchaseResponse = app.call(&format!("{WALLET}/StartPurchase"), Some(&fan), &request).await.unwrap();
    let again: StartPurchaseResponse = app.call(&format!("{WALLET}/StartPurchase"), Some(&fan), &request).await.unwrap();
    assert_eq!(first.purchase.as_option().unwrap().id, again.purchase.as_option().unwrap().id);

    let (fan, gift_key) = (minor(&app, "fan").await, key("gift"));
    fund(&app, &fan, 5000).await;
    let paid = ledger::transfer(&app.core, fan.id, creator.id, 5000, 30, "live.gift", &gift_key, serde_json::json!({})).await.unwrap();
    let retried = ledger::transfer(&app.core, fan.id, creator.id, 5000, 30, "live.gift", &gift_key, serde_json::json!({})).await.unwrap();
    assert_eq!((retried.entry_id, retried.applied), (paid.entry_id, false));
}

#[tokio::test]
async fn adults_have_no_limit_until_they_choose_one() {
    let (app, _) = app().await;
    let (fan, creator) = (app.user("fan").await, app.user("creator").await);
    let open = limit(&app, &fan).await;
    assert_eq!((open.ceiling_yen, open.own_limit_yen, open.limit_yen, open.remaining_yen), (None, None, None, None));
    start(&app, &fan, "coins_10000", "").await.unwrap();
    assert_eq!(limit(&app, &fan).await.spent_yen, 10_000);

    let chosen = set_limit(&app, &fan, Some(12_000)).await.unwrap();
    assert_eq!((chosen.own_limit_yen, chosen.limit_yen, chosen.remaining_yen), (Some(12_000), Some(12_000), Some(2000)));
    assert_eq!(gift(&app, &fan, &creator, 2001).await.unwrap_err().reason, "SPENDING_LIMIT_REACHED");
    gift(&app, &fan, &creator, 2000).await.unwrap();
    set_limit(&app, &fan, Some(50_000)).await.unwrap();
    gift(&app, &fan, &creator, 1000).await.unwrap();
    assert_eq!(set_limit(&app, &fan, None).await.unwrap().limit_yen, None, "adults may clear it");
}

#[tokio::test]
async fn minors_may_lower_their_limit_but_not_lift_it() {
    let (app, _) = app().await;
    let (fan, creator) = (minor(&app, "fan").await, app.user("creator").await);
    fund(&app, &fan, 10_000).await;
    for (yen, reason) in [(Some(CEILING + 1), "SPENDING_LIMIT_RESTRICTED"), (None, "SPENDING_LIMIT_RESTRICTED"), (Some(-1), "SPENDING_LIMIT_INVALID")] {
        let refused = set_limit(&app, &fan, yen).await.unwrap_err();
        assert_eq!((refused.code.as_str(), refused.reason.as_str()), ("invalid_argument", reason), "{yen:?}");
    }
    let lowered = set_limit(&app, &fan, Some(1200)).await.unwrap();
    assert_eq!((lowered.ceiling_yen, lowered.own_limit_yen, lowered.limit_yen, lowered.remaining_yen), (Some(CEILING), Some(1200), Some(1200), Some(1200)));
    assert_eq!(gift(&app, &fan, &creator, 1201).await.unwrap_err().reason, "SPENDING_LIMIT_REACHED");
    gift(&app, &fan, &creator, 1200).await.unwrap();
    assert_eq!(set_limit(&app, &fan, Some(CEILING)).await.unwrap().remaining_yen, Some(CEILING - 1200), "back up to the ceiling is fine");
    set_limit(&app, &fan, Some(0)).await.unwrap();
    assert_eq!(gift(&app, &fan, &creator, 1).await.unwrap_err().reason, "SPENDING_LIMIT_REACHED");
}

/// The ceiling is a limit: it follows the age today. What the person chose
/// themselves stays when they turn 18, and is theirs to change then.
#[tokio::test]
async fn the_ceiling_ends_at_eighteen_and_the_own_limit_stays() {
    let (app, _) = app().await;
    let (fan, creator) = (minor(&app, "fan").await, app.user("creator").await);
    fund(&app, &fan, 20_000).await;
    set_limit(&app, &fan, Some(2000)).await.unwrap();
    set_birthdate(&app, &fan, "current_date - interval '18 years'").await;
    let grown = limit(&app, &fan).await;
    assert_eq!((grown.ceiling_yen, grown.own_limit_yen, grown.limit_yen), (None, Some(2000), Some(2000)));
    assert_eq!(gift(&app, &fan, &creator, 2001).await.unwrap_err().reason, "SPENDING_LIMIT_REACHED");
    set_limit(&app, &fan, Some(8000)).await.unwrap();
    gift(&app, &fan, &creator, 8000).await.unwrap();
}

#[tokio::test]
async fn last_months_spending_does_not_count() {
    let (app, _) = app().await;
    let (fan, creator) = (minor(&app, "fan").await, app.user("creator").await);
    fund(&app, &fan, 10_000).await;
    gift(&app, &fan, &creator, 5000).await.unwrap();
    sqlx::query("UPDATE wallet_entries SET created_at = created_at - interval '32 days' WHERE from_user_id = $1").bind(fan.id).execute(&app.core.db).await.unwrap();
    assert_eq!(limit(&app, &fan).await.spent_yen, 0);
    gift(&app, &fan, &creator, 5000).await.unwrap();

    let resets_at = limit(&app, &fan).await.resets_at.into_option().unwrap();
    let month_start: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("SELECT (date_trunc('month', now() AT TIME ZONE 'Asia/Tokyo') + interval '1 month') AT TIME ZONE 'Asia/Tokyo'")
        .fetch_one(&app.core.db)
        .await
        .unwrap();
    assert_eq!(resets_at.seconds, month_start.timestamp(), "the first of next month, midnight in Japan");
}

/// Two charges at once are counted one after the other: of purchases and
/// gifts that each fit alone but not together, exactly one passes. As many
/// calls as the test pool has connections, so a lock held while asking the
/// pool for another connection would time out here.
#[tokio::test]
async fn charges_at_once_cannot_both_pass_the_check() {
    let (app, _) = app().await;
    let (fan, creator) = (minor(&app, "fan").await, app.user("creator").await);
    fund(&app, &fan, 30_000).await;
    let package = package(&app, "coins_3000").await;
    let attempts = (0..6).map(|n| {
        let (app, fan, creator, package_id) = (&app, &fan, &creator, package.id.clone());
        async move {
            if n % 2 == 0 {
                gift(app, fan, creator, 3000).await.map(|_| ()).map_err(|e| e.reason.to_string())
            } else {
                let request = StartPurchaseRequest { package_id, idempotency_key: key("buy"), ..Default::default() };
                app.call::<_, StartPurchaseResponse>(&format!("{WALLET}/StartPurchase"), Some(fan), &request).await.map(|_| ()).map_err(|e| e.reason)
            }
        }
    });
    let results = tokio::time::timeout(std::time::Duration::from_secs(8), futures::future::join_all(attempts)).await.expect("no request waits out the pool");
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1, "{results:?}");
    assert!(results.iter().filter_map(|r| r.as_ref().err()).all(|reason| reason == "SPENDING_LIMIT_REACHED"), "{results:?}");
    assert_eq!(limit(&app, &fan).await.spent_yen, 3000);
}

#[tokio::test]
async fn the_limit_is_the_callers_own() {
    let (app, _) = app().await;
    let error = app.call::<_, GetSpendingLimitResponse>(&format!("{WALLET}/GetSpendingLimit"), None, &GetSpendingLimitRequest::default()).await.unwrap_err();
    assert_eq!(error.code, "unauthenticated");
    let request = SetSpendingLimitRequest { limit_yen: Some(100), ..Default::default() };
    let error = app.call::<_, SetSpendingLimitResponse>(&format!("{WALLET}/SetSpendingLimit"), None, &request).await.unwrap_err();
    assert_eq!(error.code, "unauthenticated");
}

/// A purchase counts in the month its money is taken. One started just
/// before midnight on the last day of a month (Japan time) and paid after
/// it holds the new month's room while it waits, and uses it once paid:
/// otherwise a minor could start ¥5,000 at 23:59, buy ¥5,000 more at
/// 00:00 and then pay the first, ¥10,000 in one month.
#[tokio::test]
async fn a_purchase_started_before_the_month_turned_counts_in_the_month_it_is_paid() {
    let (app, wallet) = app().await;
    let fakepay = with_fakepay(&wallet);
    let fan = minor(&app, "fan").await;
    let late = start(&app, &fan, "coins_5000", FakePay::NAME).await.unwrap();
    sqlx::query(
        "UPDATE wallet_purchases SET created_at = (date_trunc('month', now() AT TIME ZONE 'Asia/Tokyo') AT TIME ZONE 'Asia/Tokyo') - interval '1 minute'
         WHERE id = $1::uuid",
    )
    .bind(&late.id)
    .execute(&app.core.db)
    .await
    .unwrap();
    assert_eq!(limit(&app, &fan).await.spent_yen, 5000, "while it waits for its payment");
    assert_eq!(start(&app, &fan, "coins_100", "").await.unwrap_err().reason, "SPENDING_LIMIT_REACHED");

    fakepay.set(&provider_ref(&app, &late.id).await, PaymentState::Paid);
    let request = ConfirmPurchaseRequest { purchase_id: late.id.clone(), ..Default::default() };
    let _: ConfirmPurchaseResponse = app.call(&format!("{WALLET}/ConfirmPurchase"), Some(&fan), &request).await.unwrap();
    assert_eq!(limit(&app, &fan).await.spent_yen, 5000, "paid this month");
    assert_eq!(start(&app, &fan, "coins_100", "").await.unwrap_err().reason, "SPENDING_LIMIT_REACHED");
}

async fn decline(app: &TestApp, fakepay: &FakePay, buyer: &TestUser, purchase: &Purchase) -> Purchase {
    fakepay.set(&provider_ref(app, &purchase.id).await, PaymentState::Failed { reason: "card_declined".into() });
    let request = ConfirmPurchaseRequest { purchase_id: purchase.id.clone(), ..Default::default() };
    let response: ConfirmPurchaseResponse = app.call(&format!("{WALLET}/ConfirmPurchase"), Some(buyer), &request).await.unwrap();
    response.purchase.into_option().unwrap()
}

async fn cancel(app: &TestApp, buyer: &TestUser, purchase: &Purchase) -> Purchase {
    let request = CancelPurchaseRequest { purchase_id: purchase.id.clone(), ..Default::default() };
    let response: CancelPurchaseResponse = app.call(&format!("{WALLET}/CancelPurchase"), Some(buyer), &request).await.unwrap();
    response.purchase.into_option().unwrap()
}

/// A purchase that failed, was cancelled or expired frees its room only
/// once its checkout can no longer be paid: the wallet voids it at the
/// provider. Otherwise a minor could have a card declined on ¥5,000, spend
/// the ¥5,000 elsewhere and then try the card again on the first checkout.
#[tokio::test]
async fn an_unpaid_purchase_frees_its_room_once_its_checkout_is_voided() {
    let (app, wallet) = app().await;
    let fakepay = with_fakepay(&wallet);
    let fan = minor(&app, "fan").await;
    let declined = start(&app, &fan, "coins_5000", FakePay::NAME).await.unwrap();
    assert_eq!(decline(&app, &fakepay, &fan, &declined).await.status.as_known(), Some(PurchaseStatus::PURCHASE_STATUS_FAILED));
    let abandoned = start(&app, &fan, "coins_5000", FakePay::NAME).await.unwrap();
    cancel(&app, &fan, &abandoned).await;
    let voided = fakepay.voided.lock().unwrap().clone();
    for purchase in [&declined, &abandoned] {
        assert!(voided.contains(&provider_ref(&app, &purchase.id).await), "{} is voided", purchase.id);
    }
    assert_eq!(limit(&app, &fan).await.spent_yen, 0);
}

#[tokio::test]
async fn a_checkout_that_could_still_be_paid_keeps_counting() {
    let (app, wallet) = app().await;
    let fakepay = with_fakepay(&wallet);
    let (fan, creator) = (minor(&app, "fan").await, app.user("creator").await);
    fund(&app, &fan, 5000).await;
    *fakepay.refuse_voids.lock().unwrap() = true;
    let declined = start(&app, &fan, "coins_5000", FakePay::NAME).await.unwrap();
    assert_eq!(decline(&app, &fakepay, &fan, &declined).await.status.as_known(), Some(PurchaseStatus::PURCHASE_STATUS_FAILED));
    assert_eq!(limit(&app, &fan).await.spent_yen, 5000, "the card may still be tried again");
    assert_eq!(gift(&app, &fan, &creator, 1).await.unwrap_err().reason, "SPENDING_LIMIT_REACHED");

    // The wallet keeps trying, and the room is free once the void takes.
    *fakepay.refuse_voids.lock().unwrap() = false;
    wallet.run_maintenance(false).await;
    assert_eq!(limit(&app, &fan).await.spent_yen, 0);
    gift(&app, &fan, &creator, 5000).await.unwrap();
}
