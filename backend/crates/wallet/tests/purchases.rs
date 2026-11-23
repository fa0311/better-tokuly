//! Buying coins: packages, the provider flow, idempotency and expiry.
mod common;

use std::time::Duration;

use common::*;
use tokuly_proto::wallet::*;
use tokuly_testkit::{TestApp, TestUser};
use tokuly_wallet::{PaymentState, WalletConfig};

const COMPLETED: Option<PurchaseStatus> = Some(PurchaseStatus::PURCHASE_STATUS_COMPLETED);
const PENDING: Option<PurchaseStatus> = Some(PurchaseStatus::PURCHASE_STATUS_PENDING);

async fn start(app: &TestApp, user: &TestUser, package_id: &str, provider: &str, idempotency_key: &str) -> Result<StartPurchaseResponse, tokuly_testkit::RpcError> {
    let request = StartPurchaseRequest { package_id: package_id.to_owned(), provider: provider.to_owned(), idempotency_key: idempotency_key.to_owned(), ..Default::default() };
    app.call(&format!("{WALLET}/StartPurchase"), Some(user), &request).await
}

async fn confirm(app: &TestApp, user: &TestUser, purchase_id: &str) -> Result<ConfirmPurchaseResponse, tokuly_testkit::RpcError> {
    let request = ConfirmPurchaseRequest { purchase_id: purchase_id.to_owned(), ..Default::default() };
    app.call(&format!("{WALLET}/ConfirmPurchase"), Some(user), &request).await
}

async fn get(app: &TestApp, user: &TestUser, purchase_id: &str) -> Purchase {
    let request = GetPurchaseRequest { purchase_id: purchase_id.to_owned(), ..Default::default() };
    let response: GetPurchaseResponse = app.call(&format!("{WALLET}/GetPurchase"), Some(user), &request).await.unwrap();
    response.purchase.into_option().unwrap()
}

#[tokio::test]
async fn coin_packages_are_listed_for_everyone() {
    let (app, _) = app().await;
    let response: ListCoinPackagesResponse = app.call(&format!("{WALLET}/ListCoinPackages"), None, &ListCoinPackagesRequest::default()).await.unwrap();
    assert_eq!(response.providers, vec!["development"]);
    let skus: Vec<&str> = response.packages.iter().map(|package| package.sku.as_str()).collect();
    for sku in ["coins_100", "coins_500", "coins_1000", "coins_3000", "coins_5000", "coins_10000"] {
        assert!(skus.contains(&sku), "{sku} is on sale");
    }
    assert!(response.packages.is_sorted_by_key(|package| package.sort_order), "shop order");
    let popular = response.packages.iter().find(|package| package.sku == "coins_1000").unwrap();
    assert_eq!((popular.coins, popular.bonus_coins, popular.price_minor, popular.currency.as_str()), (1000, 60, 1000, "JPY"));
    assert_eq!(popular.badge.as_known(), Some(PackageBadge::PACKAGE_BADGE_POPULAR));
    assert!(popular.active);
}

#[tokio::test]
async fn development_purchase_completes_on_the_spot() {
    let (app, _) = app().await;
    let buyer = app.user("buyer").await;
    let package = package(&app, "coins_1000").await;
    let mut frames = app.listen_user(buyer.id).await;

    let response = start(&app, &buyer, &package.id, "", "").await.unwrap();
    let purchase = response.purchase.into_option().unwrap();
    assert_eq!(purchase.status.as_known(), COMPLETED);
    assert_eq!((purchase.provider.as_str(), purchase.sku.as_str(), purchase.coins, purchase.bonus_coins, purchase.price_minor), ("development", "coins_1000", 1000, 60, 1000));
    assert!(purchase.checkout_url.is_empty() && !purchase.entry_id.is_empty());
    assert!(purchase.completed_at.as_option().is_some());
    assert_eq!(response.balance.as_option().unwrap().coins, 1060, "coins plus bonus");

    // The grant is an ordinary ledger entry.
    let entries = history(&app, &buyer).await;
    assert_eq!(entries.len(), 1);
    assert_eq!((entries[0].id.as_str(), entries[0].reason.as_str(), entries[0].coins_delta, entries[0].description.as_str()), (purchase.entry_id.as_str(), "purchase", 1060, "コインを購入しました"));

    // A receipt lands in the inbox (no push: the buyer watched it happen)...
    let notices = notifications(&app, &buyer, "wallet.purchase").await;
    assert_eq!(notices.len(), 1);
    assert_eq!((notices[0]["inbox"].as_bool(), notices[0]["push"].as_bool()), (Some(true), Some(false)));
    assert_eq!(notices[0]["body"]["en"], "1,060 coins were added to your wallet.");
    // ...and the buyer's other devices hear about the purchase.
    let frame = frames.next(Duration::from_secs(2)).await.expect("purchase pushed");
    let event: WalletEvent = frame.event().expect("not a wallet frame");
    let Some(wallet_event::Event::PurchaseUpdated(pushed)) = event.event else { panic!("not a purchase update") };
    assert_eq!((pushed.id.as_str(), pushed.status.as_known()), (purchase.id.as_str(), COMPLETED));

    assert_eq!(get(&app, &buyer, &purchase.id).await.status.as_known(), COMPLETED);
    let list: ListPurchasesResponse = app.call(&format!("{WALLET}/ListPurchases"), Some(&buyer), &ListPurchasesRequest::default()).await.unwrap();
    assert_eq!(list.purchases.len(), 1);
    assert_eq!(list.purchases[0].id, purchase.id);
    assert_eq!(postings_sum(&app).await, 0);
}

#[tokio::test]
async fn starting_again_with_the_same_key_returns_the_same_purchase() {
    let (app, _) = app().await;
    let buyer = app.user("retry").await;
    let (small, large) = (package(&app, "coins_100").await, package(&app, "coins_500").await);
    let retry_key = key("buy");

    let first = start(&app, &buyer, &small.id, "", &retry_key).await.unwrap();
    let second = start(&app, &buyer, &small.id, "", &retry_key).await.unwrap();
    assert_eq!(first.purchase.as_option().unwrap().id, second.purchase.as_option().unwrap().id);
    assert_eq!(balance(&app, &buyer).await.coins, 100, "the coins were granted once");

    let err = start(&app, &buyer, &large.id, "", &retry_key).await.unwrap_err();
    assert_eq!((err.code.as_str(), err.reason.as_str()), ("already_exists", "IDEMPOTENCY_KEY_REUSED"));

    // Without a key every start is a new purchase; keys are per user.
    start(&app, &buyer, &small.id, "", "").await.unwrap();
    assert_eq!(balance(&app, &buyer).await.coins, 200);
    let other = app.user("other").await;
    let theirs = start(&app, &other, &small.id, "", &retry_key).await.unwrap();
    assert_ne!(theirs.purchase.as_option().unwrap().id, first.purchase.as_option().unwrap().id);
}

#[tokio::test]
async fn start_purchase_validates_its_input() {
    let (app, _) = app().await;
    let buyer = app.user("picky").await;
    let package = package(&app, "coins_100").await;

    let cases: Vec<(&str, String, &str, String)> = vec![
        ("INVALID_ID", "not-a-uuid".into(), "", String::new()),
        ("PACKAGE_NOT_FOUND", uuid::Uuid::now_v7().to_string(), "", String::new()),
        ("PROVIDER_UNKNOWN", package.id.clone(), "paypal", String::new()),
        ("IDEMPOTENCY_KEY_INVALID", package.id.clone(), "", "k".repeat(201)),
    ];
    for (reason, package_id, provider, idempotency_key) in cases {
        let err = start(&app, &buyer, &package_id, provider, &idempotency_key).await.unwrap_err();
        assert_eq!(err.reason, reason, "{err}");
    }
    let request = StartPurchaseRequest { package_id: package.id.clone(), ..Default::default() };
    let err = app.call::<_, StartPurchaseResponse>(&format!("{WALLET}/StartPurchase"), None, &request).await.unwrap_err();
    assert_eq!(err.reason, "SIGN_IN_REQUIRED");
    assert_eq!(balance(&app, &buyer).await.coins, 0);
}

#[tokio::test]
async fn hosted_checkout_completes_when_the_provider_confirms() {
    let (app, wallet) = app().await;
    let fakepay = with_fakepay(&wallet);
    let buyer = app.user("hosted").await;
    let package = package(&app, "coins_500").await;

    let listed: ListCoinPackagesResponse = app.call(&format!("{WALLET}/ListCoinPackages"), None, &ListCoinPackagesRequest::default()).await.unwrap();
    assert_eq!(listed.providers, vec!["development", "fakepay"], "the default provider comes first");

    let started = start(&app, &buyer, &package.id, "fakepay", "").await.unwrap();
    let purchase = started.purchase.into_option().unwrap();
    assert_eq!(purchase.status.as_known(), PENDING);
    assert!(purchase.checkout_url.starts_with("https://pay.example/checkout/fp_"));
    assert!(purchase.entry_id.is_empty() && purchase.expires_at.as_option().is_some());
    assert_eq!(started.balance.as_option().unwrap().coins, 0, "nothing is granted before the payment");

    // Asking before the money arrived changes nothing.
    let unpaid = confirm(&app, &buyer, &purchase.id).await.unwrap();
    assert_eq!(unpaid.purchase.as_option().unwrap().status.as_known(), PENDING);

    fakepay.set(&provider_ref(&app, &purchase.id).await, PaymentState::Paid);
    let paid = confirm(&app, &buyer, &purchase.id).await.unwrap();
    assert_eq!(paid.purchase.as_option().unwrap().status.as_known(), COMPLETED);
    assert_eq!(paid.balance.as_option().unwrap().coins, 520);

    // Confirming again never grants twice.
    let again = confirm(&app, &buyer, &purchase.id).await.unwrap();
    assert_eq!(again.balance.as_option().unwrap().coins, 520);
    assert_eq!(history(&app, &buyer).await.len(), 1);

    // Purchases are private to their buyer.
    let stranger = app.user("stranger").await;
    assert_eq!(confirm(&app, &stranger, &purchase.id).await.unwrap_err().reason, "PURCHASE_NOT_FOUND");
    let request = GetPurchaseRequest { purchase_id: purchase.id.clone(), ..Default::default() };
    let err = app.call::<_, GetPurchaseResponse>(&format!("{WALLET}/GetPurchase"), Some(&stranger), &request).await.unwrap_err();
    assert_eq!((err.code.as_str(), err.reason.as_str()), ("not_found", "PURCHASE_NOT_FOUND"));
    let err = app.call::<_, GetPurchaseResponse>(&format!("{WALLET}/GetPurchase"), None, &request).await.unwrap_err();
    assert_eq!(err.reason, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn declined_and_cancelled_purchases_grant_nothing_unless_paid_after_all() {
    let (app, wallet) = app().await;
    let fakepay = with_fakepay(&wallet);
    let buyer = app.user("declined").await;
    let package = package(&app, "coins_100").await;
    let cancel = |purchase_id: String, user: TestUser| {
        let app = &app;
        async move { app.call::<_, CancelPurchaseResponse>(&format!("{WALLET}/CancelPurchase"), Some(&user), &CancelPurchaseRequest { purchase_id, ..Default::default() }).await }
    };

    // Declined by the provider.
    let declined = start(&app, &buyer, &package.id, "fakepay", "").await.unwrap().purchase.into_option().unwrap();
    fakepay.set(&provider_ref(&app, &declined.id).await, PaymentState::Failed { reason: "card_declined".into() });
    let failed = confirm(&app, &buyer, &declined.id).await.unwrap().purchase.into_option().unwrap();
    assert_eq!((failed.status.as_known(), failed.failure_reason.as_str()), (Some(PurchaseStatus::PURCHASE_STATUS_FAILED), "card_declined"));
    assert_eq!(cancel(declined.id.clone(), buyer.clone()).await.unwrap_err().reason, "PURCHASE_NOT_PENDING");

    // Cancelled by the buyer; only they can, and only once.
    let abandoned = start(&app, &buyer, &package.id, "fakepay", "").await.unwrap().purchase.into_option().unwrap();
    let stranger = app.user("stranger").await;
    assert_eq!(cancel(abandoned.id.clone(), stranger).await.unwrap_err().reason, "PURCHASE_NOT_FOUND");
    let cancelled = cancel(abandoned.id.clone(), buyer.clone()).await.unwrap().purchase.into_option().unwrap();
    assert_eq!(cancelled.status.as_known(), Some(PurchaseStatus::PURCHASE_STATUS_CANCELLED));
    assert_eq!(cancel(abandoned.id.clone(), buyer.clone()).await.unwrap_err().reason, "PURCHASE_NOT_PENDING");
    assert_eq!(balance(&app, &buyer).await.coins, 0);

    // Money that was taken always becomes coins, even for a cancelled purchase.
    fakepay.set(&provider_ref(&app, &abandoned.id).await, PaymentState::Paid);
    let rescued = confirm(&app, &buyer, &abandoned.id).await.unwrap();
    assert_eq!(rescued.purchase.as_option().unwrap().status.as_known(), COMPLETED);
    assert_eq!(rescued.balance.as_option().unwrap().coins, 100);
    // The same goes for a declined payment the buyer retried successfully.
    fakepay.set(&provider_ref(&app, &declined.id).await, PaymentState::Paid);
    let retried = confirm(&app, &buyer, &declined.id).await.unwrap();
    assert_eq!(retried.purchase.as_option().unwrap().status.as_known(), COMPLETED);
    assert!(retried.purchase.as_option().unwrap().failure_reason.is_empty());
    assert_eq!(retried.balance.as_option().unwrap().coins, 200);
}

#[tokio::test]
async fn provider_webhooks_settle_purchases() {
    let (app, wallet) = app().await;
    let _fakepay = with_fakepay(&wallet);
    let buyer = app.user("webhook").await;
    let package = package(&app, "coins_500").await;
    let purchase = start(&app, &buyer, &package.id, "fakepay", "").await.unwrap().purchase.into_option().unwrap();
    let reference = provider_ref(&app, &purchase.id).await;

    wallet.settle_purchase("fakepay", &reference, PaymentState::Pending).await.unwrap();
    assert_eq!(get(&app, &buyer, &purchase.id).await.status.as_known(), PENDING);
    wallet.settle_purchase("fakepay", &reference, PaymentState::Paid).await.unwrap();
    wallet.settle_purchase("fakepay", &reference, PaymentState::Paid).await.unwrap();
    assert_eq!(get(&app, &buyer, &purchase.id).await.status.as_known(), COMPLETED);
    assert_eq!(balance(&app, &buyer).await.coins, 520, "a repeated webhook grants nothing more");

    // Completed behind the buyer's back: this time they get a push notification too.
    let notices = notifications(&app, &buyer, "wallet.purchase").await;
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0]["push"].as_bool(), Some(true));

    assert_eq!(wallet.settle_purchase("fakepay", "fp_unknown", PaymentState::Paid).await.unwrap_err().reason, "PURCHASE_NOT_FOUND");
    assert_eq!(wallet.settle_purchase("development", &reference, PaymentState::Paid).await.unwrap_err().reason, "PURCHASE_NOT_FOUND", "references are per provider");
}

#[tokio::test]
async fn overdue_purchases_expire_after_a_last_look_at_the_provider() {
    let (app, wallet) = app().await;
    let fakepay = with_fakepay(&wallet);
    let buyer = app.user("late").await;
    let package = package(&app, "coins_100").await;
    let overdue = |purchase_id: String| {
        let app = &app;
        async move {
            sqlx::query("UPDATE wallet_purchases SET expires_at = now() - interval '1 minute' WHERE id = $1::uuid").bind(purchase_id).execute(&app.core.db).await.unwrap();
        }
    };

    let unpaid = start(&app, &buyer, &package.id, "fakepay", "").await.unwrap().purchase.into_option().unwrap();
    let paid_silently = start(&app, &buyer, &package.id, "fakepay", "").await.unwrap().purchase.into_option().unwrap();
    let unreachable = start(&app, &buyer, &package.id, "fakepay", "").await.unwrap().purchase.into_option().unwrap();
    let in_time = start(&app, &buyer, &package.id, "fakepay", "").await.unwrap().purchase.into_option().unwrap();
    fakepay.set(&provider_ref(&app, &paid_silently.id).await, PaymentState::Paid);
    overdue(unpaid.id.clone()).await;
    overdue(paid_silently.id.clone()).await;

    wallet.run_maintenance(false).await;
    assert_eq!(get(&app, &buyer, &unpaid.id).await.status.as_known(), Some(PurchaseStatus::PURCHASE_STATUS_EXPIRED));
    assert_eq!(get(&app, &buyer, &paid_silently.id).await.status.as_known(), COMPLETED, "a payment nobody reported is still honoured");
    assert_eq!(get(&app, &buyer, &in_time.id).await.status.as_known(), PENDING, "purchases with time left are untouched");
    assert_eq!(balance(&app, &buyer).await.coins, 100);

    // While the provider cannot be asked, an overdue purchase waits rather than being written off.
    *fakepay.down.lock().unwrap() = true;
    overdue(unreachable.id.clone()).await;
    wallet.run_maintenance(false).await;
    assert_eq!(get(&app, &buyer, &unreachable.id).await.status.as_known(), PENDING);
    let err = confirm(&app, &buyer, &unreachable.id).await.unwrap_err();
    assert_eq!((err.code.as_str(), err.reason.as_str()), ("unavailable", "PAYMENT_PROVIDER_ERROR"));
    *fakepay.down.lock().unwrap() = false;
    wallet.run_maintenance(false).await;
    assert_eq!(get(&app, &buyer, &unreachable.id).await.status.as_known(), Some(PurchaseStatus::PURCHASE_STATUS_EXPIRED));
}

#[tokio::test]
async fn without_a_provider_coins_cannot_be_bought() {
    let (app, wallet) = app_with(WalletConfig { dev_provider: Some(false), ..Default::default() }).await;
    assert!(wallet.payment_providers().is_empty());
    let buyer = app.user("nobuy").await;
    let listed: ListCoinPackagesResponse = app.call(&format!("{WALLET}/ListCoinPackages"), None, &ListCoinPackagesRequest::default()).await.unwrap();
    assert!(listed.providers.is_empty());
    assert!(!listed.packages.is_empty(), "prices are still shown");
    let package_id = listed.packages[0].id.clone();
    for provider in ["", "development"] {
        let err = start(&app, &buyer, &package_id, provider, "").await.unwrap_err();
        assert_eq!((err.code.as_str(), err.reason.as_str()), ("unimplemented", "PURCHASES_UNAVAILABLE"));
    }
}

/// A provider whose checkouts take a moment to open, recording every
/// checkout it opened and every one it was asked to void.
#[derive(Default)]
struct SlowPay {
    opened: std::sync::Mutex<Vec<String>>,
    voided: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl tokuly_wallet::PaymentProvider for SlowPay {
    fn name(&self) -> &'static str {
        "slowpay"
    }

    async fn create_checkout(&self, _order: &tokuly_wallet::PurchaseOrder) -> tokuly_core::AppResult<tokuly_wallet::Checkout> {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let reference = format!("sp_{}", tokuly_core::ids::random_base62(12));
        self.opened.lock().unwrap().push(reference.clone());
        Ok(tokuly_wallet::Checkout { checkout_url: Some(format!("https://pay.example/{reference}")), reference, state: PaymentState::Pending })
    }

    async fn confirm(&self, _order: &tokuly_wallet::PurchaseOrder, _reference: &str, _client_proof: &str) -> tokuly_core::AppResult<PaymentState> {
        Ok(PaymentState::Pending)
    }

    async fn cancel_checkout(&self, _order: &tokuly_wallet::PurchaseOrder, reference: &str) -> tokuly_core::AppResult<()> {
        self.voided.lock().unwrap().push(reference.to_owned());
        Ok(())
    }
}

/// A client that retries StartPurchase while its first call is still
/// opening the checkout must not end up with two payable checkouts for one
/// purchase: money paid into the one the purchase does not remember would
/// never become coins.
#[tokio::test]
async fn a_retry_during_checkout_leaves_one_payable_checkout() {
    let (app, wallet) = app().await;
    let slowpay = std::sync::Arc::new(SlowPay::default());
    wallet.add_payment_provider(slowpay.clone());
    let buyer = app.user("impatient").await;
    let coins_500 = package(&app, "coins_500").await;
    let retry_key = key("buy");

    let (first, second) = tokio::join!(start(&app, &buyer, &coins_500.id, "slowpay", &retry_key), start(&app, &buyer, &coins_500.id, "slowpay", &retry_key));
    let (first, second) = (first.unwrap().purchase.into_option().unwrap(), second.unwrap().purchase.into_option().unwrap());
    assert_eq!(first.id, second.id, "one purchase");
    assert_eq!(first.checkout_url, second.checkout_url, "both calls send the buyer to the same checkout");
    let kept = provider_ref(&app, &first.id).await;
    assert!(first.checkout_url.ends_with(&kept));
    let opened = slowpay.opened.lock().unwrap().clone();
    let voided = slowpay.voided.lock().unwrap().clone();
    for reference in opened.iter().filter(|reference| **reference != kept) {
        assert!(voided.contains(reference), "checkout {reference} was opened, forgotten and left payable");
    }
    assert!(!voided.contains(&kept));
}
