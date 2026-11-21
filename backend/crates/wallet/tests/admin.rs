//! `WalletAdminService`: adjustments, support lookups and the coin shop.
//! (Payout review is covered in `payouts.rs`.)
mod common;

use common::*;
use tokuly_proto::wallet::*;
use tokuly_testkit::{RpcError, TestApp, TestUser};

fn adjustment(user: &TestUser, account: LedgerAccount, delta: i64) -> AdjustBalanceRequest {
    AdjustBalanceRequest { user_id: user.id.to_string(), account: account.into(), delta, reason: "サポート対応 #4821".into(), ..Default::default() }
}

async fn adjust(app: &TestApp, admin: &TestUser, request: &AdjustBalanceRequest) -> Result<AdjustBalanceResponse, RpcError> {
    app.call(&format!("{ADMIN}/AdjustBalance"), Some(admin), request).await
}

async fn save_package(app: &TestApp, admin: &TestUser, package: CoinPackage) -> Result<CoinPackage, RpcError> {
    let response: SaveCoinPackageResponse = app.call(&format!("{ADMIN}/SaveCoinPackage"), Some(admin), &SaveCoinPackageRequest { package: package.into(), ..Default::default() }).await?;
    Ok(response.package.into_option().unwrap())
}

fn new_package(sku: &str) -> CoinPackage {
    CoinPackage { sku: sku.to_owned(), coins: 250, bonus_coins: 5, price_minor: 250, currency: "JPY".into(), sort_order: 15, active: true, ..Default::default() }
}

fn unique_sku(tag: &str) -> String {
    format!("{tag}_{}", tokuly_core::ids::random_base62(8).to_ascii_lowercase())
}

#[tokio::test]
async fn adjustments_move_coins_and_are_logged() {
    let (app, _) = app().await;
    let (admin, user) = (app.user_with_roles("boss", &["admin"]).await, app.user("lucky").await);

    // A credit to spendable coins.
    let credited = adjust(&app, &admin, &adjustment(&user, LedgerAccount::LEDGER_ACCOUNT_COINS, 500)).await.unwrap();
    assert_eq!(credited.balance.as_option().unwrap().coins, 500);
    let logged = credited.adjustment.as_option().unwrap();
    assert_eq!((logged.delta, logged.reason.as_str(), logged.account.as_known()), (500, "サポート対応 #4821", Some(LedgerAccount::LEDGER_ACCOUNT_COINS)));
    assert_eq!(logged.user.as_option().unwrap().id, user.id.to_string());
    assert_eq!(logged.admin.as_option().unwrap().id, admin.id.to_string());

    // A debit, and one that would overdraw.
    let debited = adjust(&app, &admin, &adjustment(&user, LedgerAccount::LEDGER_ACCOUNT_COINS, -200)).await.unwrap();
    assert_eq!(debited.balance.as_option().unwrap().coins, 300);
    let err = adjust(&app, &admin, &adjustment(&user, LedgerAccount::LEDGER_ACCOUNT_COINS, -301)).await.unwrap_err();
    assert_eq!((err.code.as_str(), err.reason.as_str()), ("failed_precondition", "INSUFFICIENT_COINS"));

    // Earnings are a separate balance.
    let earned = adjust(&app, &admin, &adjustment(&user, LedgerAccount::LEDGER_ACCOUNT_EARNINGS, 70)).await.unwrap();
    let after = earned.balance.as_option().unwrap();
    assert_eq!((after.coins, after.earnings), (300, 70));
    assert_eq!(adjust(&app, &admin, &adjustment(&user, LedgerAccount::LEDGER_ACCOUNT_EARNINGS, -71)).await.unwrap_err().reason, "INSUFFICIENT_EARNINGS");

    // The user sees neutral history lines (never the internal reason)...
    let entries = history(&app, &user).await;
    assert_eq!(entries.len(), 3);
    assert!(entries.iter().all(|entry| entry.kind.as_known() == Some(EntryKind::ENTRY_KIND_ADJUSTMENT) && entry.description == "運営による残高の調整"));
    assert_eq!(entries.iter().map(|entry| (entry.coins_delta, entry.earnings_delta)).collect::<Vec<_>>(), vec![(0, 70), (-200, 0), (500, 0)]);
    assert!(entries.iter().all(|entry| !entry.memo_json.contains("サポート")));
    assert_eq!(entries[2].id, logged.entry_id);

    // ...and the log keeps who, what and why. A failed adjustment leaves no trace.
    let request = ListAdjustmentsRequest { user_id: user.id.to_string(), ..Default::default() };
    let log: ListAdjustmentsResponse = app.call(&format!("{ADMIN}/ListAdjustments"), Some(&admin), &request).await.unwrap();
    assert_eq!(log.adjustments.iter().map(|a| a.delta).collect::<Vec<_>>(), vec![70, -200, 500], "newest first");
    let everyone: ListAdjustmentsResponse = app.call(&format!("{ADMIN}/ListAdjustments"), Some(&admin), &ListAdjustmentsRequest::default()).await.unwrap();
    assert!(everyone.adjustments.len() >= 3);
    assert_eq!(postings_sum(&app).await, 0);
}

#[tokio::test]
async fn a_retried_adjustment_is_applied_once() {
    let (app, _) = app().await;
    let (admin, user) = (app.user_with_roles("boss", &["admin"]).await, app.user("once").await);
    let keyed = AdjustBalanceRequest { idempotency_key: key("adj"), ..adjustment(&user, LedgerAccount::LEDGER_ACCOUNT_COINS, 100) };

    let first = adjust(&app, &admin, &keyed).await.unwrap();
    let retried = adjust(&app, &admin, &keyed).await.unwrap();
    assert_eq!(first.adjustment.as_option().unwrap().id, retried.adjustment.as_option().unwrap().id);
    assert_eq!(retried.balance.as_option().unwrap().coins, 100);

    let different = AdjustBalanceRequest { delta: 999, ..keyed.clone() };
    let err = adjust(&app, &admin, &different).await.unwrap_err();
    assert_eq!((err.code.as_str(), err.reason.as_str()), ("already_exists", "IDEMPOTENCY_KEY_REUSED"));
    assert_eq!(balance(&app, &user).await.coins, 100);

    // Racing retries still add up to one adjustment.
    let racing = AdjustBalanceRequest { idempotency_key: key("adj"), ..adjustment(&user, LedgerAccount::LEDGER_ACCOUNT_COINS, 7) };
    let results = futures::future::join_all((0..4).map(|_| adjust(&app, &admin, &racing))).await;
    assert!(results.iter().all(Result::is_ok));
    assert_eq!(balance(&app, &user).await.coins, 107);
}

#[tokio::test]
async fn adjustments_need_the_admin_role_and_valid_input() {
    let (app, _) = app().await;
    let (admin, moderator, user) = (app.user_with_roles("boss", &["admin"]).await, app.user_with_roles("mod", &["moderator"]).await, app.user("target").await);
    let valid = adjustment(&user, LedgerAccount::LEDGER_ACCOUNT_COINS, 100);

    // Moderators review payouts but do not hand out coins.
    for (caller, expected) in [(Some(&moderator), "FORBIDDEN"), (Some(&user), "FORBIDDEN"), (None, "SIGN_IN_REQUIRED")] {
        let err = app.call::<_, AdjustBalanceResponse>(&format!("{ADMIN}/AdjustBalance"), caller, &valid).await.unwrap_err();
        assert_eq!(err.reason, expected);
        let err = app.call::<_, ListAdjustmentsResponse>(&format!("{ADMIN}/ListAdjustments"), caller, &ListAdjustmentsRequest::default()).await.unwrap_err();
        assert_eq!(err.reason, expected);
    }

    let cases: Vec<(&str, Box<dyn Fn(&mut AdjustBalanceRequest)>)> = vec![
        ("INVALID_ID", Box::new(|r| r.user_id = "nope".into())),
        ("USER_NOT_FOUND", Box::new(|r| r.user_id = uuid::Uuid::now_v7().to_string())),
        ("ACCOUNT_INVALID", Box::new(|r| r.account = LedgerAccount::LEDGER_ACCOUNT_UNSPECIFIED.into())),
        ("AMOUNT_INVALID", Box::new(|r| r.delta = 0)),
        ("AMOUNT_INVALID", Box::new(|r| r.delta = 100_000_001)),
        ("REASON_REQUIRED", Box::new(|r| r.reason = "  ".into())),
        ("REASON_TOO_LONG", Box::new(|r| r.reason = "x".repeat(501))),
        ("IDEMPOTENCY_KEY_INVALID", Box::new(|r| r.idempotency_key = "k".repeat(201))),
    ];
    for (reason, mutate) in cases {
        let mut request = valid.clone();
        mutate(&mut request);
        let err = adjust(&app, &admin, &request).await.unwrap_err();
        assert_eq!(err.reason, reason, "{err}");
    }
    assert_eq!(balance(&app, &user).await.coins, 0);
    assert_eq!(app.call::<_, ListAdjustmentsResponse>(&format!("{ADMIN}/ListAdjustments"), Some(&admin), &ListAdjustmentsRequest { user_id: "nope".into(), ..Default::default() }).await.unwrap_err().reason, "INVALID_ID");
}

#[tokio::test]
async fn support_can_look_into_a_wallet() {
    let (app, _) = app().await;
    let (moderator, creator) = (app.user_with_roles("mod", &["moderator"]).await, app.user("creator").await);
    let (fan, _) = earn(&app, &creator, 3000, "live.gift").await;
    let coins_1000 = package(&app, "coins_1000").await;
    let _: StartPurchaseResponse = app.call(&format!("{WALLET}/StartPurchase"), Some(&creator), &StartPurchaseRequest { package_id: coins_1000.id, ..Default::default() }).await.unwrap();
    let payout = RequestPayoutRequest { coins: 1000, method: PayoutMethod::PAYOUT_METHOD_PAYPAL.into(), destination: "creator@example.com".into(), ..Default::default() };
    let _: RequestPayoutResponse = app.call(&format!("{EARNINGS}/RequestPayout"), Some(&creator), &payout).await.unwrap();

    let lookup = GetUserWalletRequest { user_id: creator.id.to_string(), ..Default::default() };
    let wallet: GetUserWalletResponse = app.call(&format!("{ADMIN}/GetUserWallet"), Some(&moderator), &lookup).await.unwrap();
    assert_eq!(wallet.user.as_option().unwrap().handle, creator.handle);
    let balance = wallet.balance.as_option().unwrap();
    assert_eq!((balance.coins, balance.earnings), (1060, 1100));
    assert_eq!((wallet.pending_payout, wallet.lifetime_earned, wallet.lifetime_paid_out, wallet.lifetime_purchased), (1000, 2100, 0, 1060));

    let request = ListUserLedgerEntriesRequest { user_id: creator.id.to_string(), ..Default::default() };
    let ledger: ListUserLedgerEntriesResponse = app.call(&format!("{ADMIN}/ListUserLedgerEntries"), Some(&moderator), &request).await.unwrap();
    assert_eq!(ledger.entries.iter().map(|e| e.kind.as_known().unwrap()).collect::<Vec<_>>(), vec![EntryKind::ENTRY_KIND_PAYOUT, EntryKind::ENTRY_KIND_GRANT, EntryKind::ENTRY_KIND_TRANSFER]);
    assert_eq!(ledger.entries[2].counterpart.as_option().unwrap().id, fan.id.to_string(), "counterparts are relative to the wallet's owner");
    let earnings_only = ListUserLedgerEntriesRequest { account: LedgerAccount::LEDGER_ACCOUNT_EARNINGS.into(), ..request.clone() };
    let ledger: ListUserLedgerEntriesResponse = app.call(&format!("{ADMIN}/ListUserLedgerEntries"), Some(&moderator), &earnings_only).await.unwrap();
    assert_eq!(ledger.entries.len(), 2);

    // Unknown users, and callers without the role.
    let ghost = uuid::Uuid::now_v7().to_string();
    let err = app.call::<_, GetUserWalletResponse>(&format!("{ADMIN}/GetUserWallet"), Some(&moderator), &GetUserWalletRequest { user_id: ghost.clone(), ..Default::default() }).await.unwrap_err();
    assert_eq!((err.code.as_str(), err.reason.as_str()), ("not_found", "USER_NOT_FOUND"));
    let err = app.call::<_, ListUserLedgerEntriesResponse>(&format!("{ADMIN}/ListUserLedgerEntries"), Some(&moderator), &ListUserLedgerEntriesRequest { user_id: ghost, ..Default::default() }).await.unwrap_err();
    assert_eq!(err.reason, "USER_NOT_FOUND");
    for (caller, expected) in [(Some(&creator), "FORBIDDEN"), (None, "SIGN_IN_REQUIRED")] {
        let err = app.call::<_, GetUserWalletResponse>(&format!("{ADMIN}/GetUserWallet"), caller, &lookup).await.unwrap_err();
        assert_eq!(err.reason, expected);
        let err = app.call::<_, ListUserLedgerEntriesResponse>(&format!("{ADMIN}/ListUserLedgerEntries"), caller, &request).await.unwrap_err();
        assert_eq!(err.reason, expected);
    }
}

#[tokio::test]
async fn admins_manage_the_coin_shop() {
    let (app, _) = app().await;
    let (admin, buyer) = (app.user_with_roles("boss", &["admin"]).await, app.user("buyer").await);
    let on_sale = || async {
        let listed: ListCoinPackagesResponse = app.call(&format!("{WALLET}/ListCoinPackages"), None, &ListCoinPackagesRequest::default()).await.unwrap();
        listed.packages
    };
    let buy = |package_id: String| {
        let app = &app;
        let buyer = &buyer;
        async move { app.call::<_, StartPurchaseResponse>(&format!("{WALLET}/StartPurchase"), Some(buyer), &StartPurchaseRequest { package_id, ..Default::default() }).await }
    };

    // Create: on sale at once, at its place in the shop order.
    let sku = unique_sku("coins_250");
    let created = save_package(&app, &admin, new_package(&sku)).await.unwrap();
    assert!(!created.id.is_empty());
    assert_eq!((created.sku.as_str(), created.coins, created.bonus_coins, created.price_minor), (sku.as_str(), 250, 5, 250));
    assert!(on_sale().await.iter().any(|package| package.id == created.id));
    assert_eq!(buy(created.id.clone()).await.unwrap().balance.as_option().unwrap().coins, 255);

    // Update: every field is replaced; the purchase already made keeps its snapshot.
    let repriced = CoinPackage { price_minor: 300, bonus_coins: 0, badge: PackageBadge::PACKAGE_BADGE_LIMITED.into(), ..created.clone() };
    let updated = save_package(&app, &admin, repriced).await.unwrap();
    assert_eq!((updated.id.as_str(), updated.price_minor, updated.bonus_coins, updated.badge.as_known()), (created.id.as_str(), 300, 0, Some(PackageBadge::PACKAGE_BADGE_LIMITED)));
    let purchases: ListPurchasesResponse = app.call(&format!("{WALLET}/ListPurchases"), Some(&buyer), &ListPurchasesRequest::default()).await.unwrap();
    assert_eq!((purchases.purchases[0].price_minor, purchases.purchases[0].bonus_coins), (250, 5));

    // Retire: gone from the shop, still in the admin list, no longer purchasable.
    save_package(&app, &admin, CoinPackage { active: false, ..updated.clone() }).await.unwrap();
    assert!(on_sale().await.iter().all(|package| package.id != created.id));
    let all: ListAllCoinPackagesResponse = app.call(&format!("{ADMIN}/ListAllCoinPackages"), Some(&admin), &ListAllCoinPackagesRequest::default()).await.unwrap();
    assert!(all.packages.iter().any(|package| package.id == created.id && !package.active));
    assert_eq!(buy(created.id.clone()).await.unwrap_err().reason, "PACKAGE_NOT_FOUND");

    // Limited-time packages are on sale only inside their window.
    let now = chrono::Utc::now();
    let over = CoinPackage { available_from: tokuly_core::time::ts(now - chrono::Duration::days(2)), available_until: tokuly_core::time::ts(now - chrono::Duration::days(1)), ..new_package(&unique_sku("ended")) };
    let upcoming = CoinPackage { available_from: tokuly_core::time::ts(now + chrono::Duration::days(1)), ..new_package(&unique_sku("soon")) };
    let running = CoinPackage { available_until: tokuly_core::time::ts(now + chrono::Duration::days(1)), ..new_package(&unique_sku("now")) };
    let (over, upcoming, running) = (save_package(&app, &admin, over).await.unwrap(), save_package(&app, &admin, upcoming).await.unwrap(), save_package(&app, &admin, running).await.unwrap());
    let listed: Vec<String> = on_sale().await.into_iter().map(|package| package.id).collect();
    assert!(listed.contains(&running.id) && !listed.contains(&over.id) && !listed.contains(&upcoming.id));
    assert_eq!(buy(upcoming.id.clone()).await.unwrap_err().reason, "PACKAGE_NOT_FOUND");
    // Clean up so the other tests' shop stays as seeded.
    for package in [running, over, upcoming] {
        save_package(&app, &admin, CoinPackage { active: false, ..package }).await.unwrap();
    }
}

#[tokio::test]
async fn coin_packages_are_validated_and_admin_only() {
    let (app, _) = app().await;
    let (admin, moderator) = (app.user_with_roles("boss", &["admin"]).await, app.user_with_roles("mod", &["moderator"]).await);

    for (caller, expected) in [(Some(&moderator), "FORBIDDEN"), (None, "SIGN_IN_REQUIRED")] {
        let request = SaveCoinPackageRequest { package: new_package(&unique_sku("nope")).into(), ..Default::default() };
        let err = app.call::<_, SaveCoinPackageResponse>(&format!("{ADMIN}/SaveCoinPackage"), caller, &request).await.unwrap_err();
        assert_eq!(err.reason, expected);
        let err = app.call::<_, ListAllCoinPackagesResponse>(&format!("{ADMIN}/ListAllCoinPackages"), caller, &ListAllCoinPackagesRequest::default()).await.unwrap_err();
        assert_eq!(err.reason, expected);
    }

    let now = chrono::Utc::now();
    let cases: Vec<(&str, Box<dyn Fn(&mut CoinPackage)>)> = vec![
        ("SKU_INVALID", Box::new(|p| p.sku = "Has Spaces".into())),
        ("SKU_TAKEN", Box::new(|p| p.sku = "coins_100".into())),
        ("COINS_INVALID", Box::new(|p| p.coins = 0)),
        ("BONUS_INVALID", Box::new(|p| p.bonus_coins = -1)),
        ("PRICE_INVALID", Box::new(|p| p.price_minor = 0)),
        ("CURRENCY_INVALID", Box::new(|p| p.currency = "yen".into())),
        ("BADGE_INVALID", Box::new(|p| p.badge = 77.into())),
        ("AVAILABILITY_INVALID", Box::new(move |p| {
            p.available_from = tokuly_core::time::ts(now);
            p.available_until = tokuly_core::time::ts(now - chrono::Duration::hours(1));
        })),
        ("INVALID_ID", Box::new(|p| p.id = "nope".into())),
        ("PACKAGE_NOT_FOUND", Box::new(|p| p.id = uuid::Uuid::now_v7().to_string())),
    ];
    for (reason, mutate) in cases {
        let mut package = new_package(&unique_sku("bad"));
        mutate(&mut package);
        let err = save_package(&app, &admin, package).await.unwrap_err();
        assert_eq!(err.reason, reason, "{err}");
    }
    // A request without a package at all fails like an empty one.
    let err = app.call::<_, SaveCoinPackageResponse>(&format!("{ADMIN}/SaveCoinPackage"), Some(&admin), &SaveCoinPackageRequest::default()).await.unwrap_err();
    assert_eq!(err.reason, "SKU_INVALID");
}
