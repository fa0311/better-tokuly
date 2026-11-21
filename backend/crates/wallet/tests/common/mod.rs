//! Shared setup for the wallet's integration tests.
#![allow(dead_code)] // each test binary uses its own subset

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use tokuly_core::{events::Event, prelude::*};
use tokuly_proto::wallet::*;
use tokuly_testkit::{TestApp, TestUser};
use tokuly_wallet::{Checkout, PaymentProvider, PaymentState, PurchaseOrder, Wallet, WalletConfig, ledger};

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0400", "1400"];

pub const WALLET: &str = "tokuly.wallet.v1.WalletService";
pub const EARNINGS: &str = "tokuly.wallet.v1.EarningsService";
pub const ADMIN: &str = "tokuly.wallet.v1.WalletAdminService";

/// An app with the wallet's RPCs, plus the domain itself for the things
/// tests drive directly (event handling, maintenance, provider webhooks).
pub async fn app_with(cfg: WalletConfig) -> (TestApp, Wallet) {
    let mut wallet = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        let domain = Wallet::new(core.clone(), identity.clone(), cfg).expect("wallet");
        let router = domain.register(router);
        wallet = Some(domain);
        router
    })
    .await;
    (app, wallet.expect("register ran"))
}

pub async fn app() -> (TestApp, Wallet) {
    app_with(WalletConfig::default()).await
}

/// A unique idempotency key.
pub fn key(tag: &str) -> String {
    format!("{tag}:{}", tokuly_core::ids::random_base62(10))
}

/// Gives `user` spendable coins.
pub async fn fund(app: &TestApp, user: &TestUser, coins: i64) {
    ledger::grant(&app.core, user.id, coins, "test.fund", &key("fund"), serde_json::json!({})).await.unwrap();
}

/// Makes `creator` earn: a funded fan sends `amount` with a 30% fee.
/// Returns the fan and the transfer's ledger entry.
pub async fn earn(app: &TestApp, creator: &TestUser, amount: i64, reason: &str) -> (TestUser, Uuid) {
    let fan = app.user("fan").await;
    fund(app, &fan, amount).await;
    let receipt = ledger::transfer(&app.core, fan.id, creator.id, amount, 30, reason, &key("gift"), serde_json::json!({})).await.unwrap();
    (fan, receipt.entry_id)
}

pub async fn balance(app: &TestApp, user: &TestUser) -> Balance {
    let response: GetBalanceResponse = app.call(&format!("{WALLET}/GetBalance"), Some(user), &GetBalanceRequest::default()).await.unwrap();
    response.balance.into_option().unwrap()
}

pub async fn history(app: &TestApp, user: &TestUser) -> Vec<LedgerEntry> {
    let response: ListLedgerEntriesResponse = app.call(&format!("{WALLET}/ListLedgerEntries"), Some(user), &ListLedgerEntriesRequest::default()).await.unwrap();
    response.entries
}

/// Sum of all postings: zero whenever the ledger is balanced.
pub async fn postings_sum(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT COALESCE(sum(delta), 0)::bigint FROM wallet_postings").fetch_one(&app.core.db).await.unwrap()
}

/// Number of ledger entries whose own postings do not sum to zero. Always 0:
/// stricter than [`postings_sum`], where two wrong entries could cancel out.
pub async fn unbalanced_entries(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM (SELECT entry_id FROM wallet_postings GROUP BY entry_id HAVING sum(delta) <> 0) broken").fetch_one(&app.core.db).await.unwrap()
}

/// Net postings of one account, e.g. `system:payout_hold` or `user:<id>:coins`.
pub async fn account_sum(app: &TestApp, account: &str) -> i64 {
    sqlx::query_scalar("SELECT COALESCE(sum(delta), 0)::bigint FROM wallet_postings WHERE account = $1").bind(account).fetch_one(&app.core.db).await.unwrap()
}

/// Notification requests of `kind` addressed to `user`, oldest first.
pub async fn notifications(app: &TestApp, user: &TestUser, kind: &str) -> Vec<serde_json::Value> {
    app.events("notify.requested")
        .await
        .into_iter()
        .map(|event| event.data)
        .filter(|data| data["user_id"] == user.id.to_string() && data["kind"] == kind)
        .collect()
}

/// `wallet.balance.changed` events for `user`, oldest first.
pub async fn balance_events(app: &TestApp, user: &TestUser) -> Vec<Event> {
    app.events_by("wallet.balance.changed", user.id).await
}

/// A payment provider with a hosted checkout whose payments tests settle by
/// hand: every checkout starts unpaid, and `set` decides what the provider
/// answers from then on.
#[derive(Default)]
pub struct FakePay {
    states: Mutex<HashMap<String, PaymentState>>,
    /// Makes `confirm` fail, as an unreachable provider would.
    pub down: Mutex<bool>,
}

impl FakePay {
    pub const NAME: &'static str = "fakepay";

    pub fn set(&self, reference: &str, state: PaymentState) {
        self.states.lock().unwrap().insert(reference.to_owned(), state);
    }
}

#[async_trait::async_trait]
impl PaymentProvider for FakePay {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    async fn create_checkout(&self, order: &PurchaseOrder) -> AppResult<Checkout> {
        let reference = format!("fp_{}", order.purchase_id.simple());
        self.set(&reference, PaymentState::Pending);
        Ok(Checkout { checkout_url: Some(format!("https://pay.example/checkout/{reference}")), reference, state: PaymentState::Pending })
    }

    async fn confirm(&self, _order: &PurchaseOrder, reference: &str, _client_proof: &str) -> AppResult<PaymentState> {
        if *self.down.lock().unwrap() {
            return Err(AppError::unavailable("FAKEPAY_DOWN", "fakepay is down"));
        }
        Ok(self.states.lock().unwrap().get(reference).cloned().unwrap_or(PaymentState::Pending))
    }
}

/// Registers a [`FakePay`] on the wallet and returns a handle to steer it.
pub fn with_fakepay(wallet: &Wallet) -> Arc<FakePay> {
    let provider = Arc::new(FakePay::default());
    wallet.add_payment_provider(provider.clone());
    provider
}

/// The provider's reference of a purchase (not exposed through the API).
pub async fn provider_ref(app: &TestApp, purchase_id: &str) -> String {
    sqlx::query_scalar("SELECT provider_ref FROM wallet_purchases WHERE id = $1::uuid").bind(purchase_id).fetch_one(&app.core.db).await.unwrap()
}

/// A coin package from the seeded line-up.
pub async fn package(app: &TestApp, sku: &str) -> CoinPackage {
    let response: ListCoinPackagesResponse = app.call(&format!("{WALLET}/ListCoinPackages"), None, &ListCoinPackagesRequest::default()).await.unwrap();
    response.packages.into_iter().find(|package| package.sku == sku).unwrap_or_else(|| panic!("package {sku} is on sale"))
}
