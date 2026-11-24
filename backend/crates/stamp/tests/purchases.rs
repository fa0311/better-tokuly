//! Buying paid packs: coins move once, whatever fails or is retried.
mod common;

use common::*;
use tokuly_proto::stamp::*;
use tokuly_wallet::ledger::Balance;

#[tokio::test]
async fn buying_a_pack_moves_coins_once_and_installs_it() {
    let ctx = start().await;
    let (creator, buyer) = (ctx.app.user("creator").await, ctx.app.user("buyer").await);
    let pack = ctx.published(&creator, "ゆうりょうパック", 100, &[], 2).await;
    let id = id_of(&pack);
    let stamp = stamp_uuid(&pack, 0);
    let buy = PurchasePackRequest { pack_id: id.clone(), expected_price: 100, ..Default::default() };

    assert_eq!(ctx.reason("Library", "PurchasePack", Some(&buyer), &buy).await, "INSUFFICIENT_COINS");
    assert!(
        !ctx.get_pack(Some(&buyer), &id).await.unwrap().summary.viewer.owned,
        "a failed payment leaves nothing behind"
    );
    ctx.grant_coins(&buyer, 250).await;
    let stale = PurchasePackRequest { expected_price: 80, ..buy.clone() };
    assert_eq!(ctx.reason("Library", "PurchasePack", Some(&buyer), &stale).await, "PRICE_CHANGED");
    assert_eq!(
        tokuly_stamp::ensure_usable(&ctx.app.core, buyer.id, stamp).await.unwrap_err().reason,
        "STAMP_NOT_USABLE"
    );

    let bought: PurchasePackResponse = ctx.call("Library", "PurchasePack", Some(&buyer), &buy).await.unwrap();
    assert_eq!(bought.charged, 100);
    let viewer = bought.pack.viewer.as_option().unwrap();
    assert!(viewer.owned && viewer.installed && viewer.usable);
    assert_eq!(bought.pack.install_count, 1);
    assert_eq!(ctx.balance(&buyer).await, Balance { coins: 150, earnings: 0 });
    assert_eq!(
        ctx.balance(&creator).await,
        Balance { coins: 0, earnings: 70 },
        "the creator earns the price minus the 30% fee"
    );
    assert!(tokuly_stamp::ensure_usable(&ctx.app.core, buyer.id, stamp).await.is_ok());

    // Buying again succeeds without charging.
    let again: PurchasePackResponse = ctx.call("Library", "PurchasePack", Some(&buyer), &buy).await.unwrap();
    assert_eq!(again.charged, 0);
    assert_eq!(ctx.balance(&buyer).await.coins, 150);

    let purchases = ctx.app.events_by("stamp.pack.purchased", buyer.id).await;
    assert_eq!(purchases.len(), 1);
    assert_eq!((purchases[0].data["price"].as_i64(), purchases[0].data["net"].as_i64()), (Some(100), Some(70)));
    let sold = ctx.notifications(&creator, "stamp.pack.sold").await;
    assert_eq!(sold.len(), 1);
    assert!(sold[0]["body"]["ja"].as_str().unwrap().contains("+70コイン"));

    // Ownership survives uninstalling, and the pack can be installed again for free.
    ctx.uninstall(&buyer, &id).await;
    let after_uninstall = ctx.get_pack(Some(&buyer), &id).await.unwrap();
    assert!(after_uninstall.summary.viewer.owned && !after_uninstall.summary.viewer.installed);
    let owned: ListPurchasedPacksResponse =
        ctx.call("Library", "ListPurchasedPacks", Some(&buyer), &ListPurchasedPacksRequest::default()).await.unwrap();
    assert_eq!(pack_names(&owned.packs), vec!["ゆうりょうパック"]);
    assert!(ctx.install(&buyer, &id).await.viewer.installed);
    assert_eq!(ctx.balance(&buyer).await.coins, 150);

    // Not everything is for sale.
    assert_eq!(ctx.reason("Library", "PurchasePack", Some(&creator), &buy).await, "PACK_NOT_FOR_SALE");
    let free = id_of(&ctx.published(&creator, "むりょう", 0, &[], 1).await);
    let buy_free = PurchasePackRequest { pack_id: free, expected_price: 0, ..Default::default() };
    assert_eq!(ctx.reason("Library", "PurchasePack", Some(&buyer), &buy_free).await, "PACK_NOT_FOR_SALE");
    assert_eq!(ctx.reason("Library", "PurchasePack", None, &buy).await, "SIGN_IN_REQUIRED");
    let missing =
        PurchasePackRequest { pack_id: uuid::Uuid::now_v7().to_string(), expected_price: 100, ..Default::default() };
    assert_eq!(ctx.reason("Library", "PurchasePack", Some(&buyer), &missing).await, "PACK_NOT_FOUND");
}

#[tokio::test]
async fn a_retried_purchase_recognises_the_earlier_payment() {
    let ctx = start().await;
    let (creator, buyer) = (ctx.app.user("creator").await, ctx.app.user("buyer").await);
    let pack = ctx.published(&creator, "とちゅうで失敗", 300, &[], 1).await;
    let id = id_of(&pack);
    ctx.grant_coins(&buyer, 300).await;

    // The state after a crash between the payment and the purchase row: the
    // ledger has the entry, the stamp domain only the intent.
    let key = format!("stamp-pack:{id}:{}:first", buyer.id);
    let pack_uuid: uuid::Uuid = id.parse().unwrap();
    tokuly_wallet::ledger::transfer(
        &ctx.app.core,
        buyer.id,
        creator.id,
        300,
        30,
        "stamp.pack",
        &key,
        serde_json::json!({}),
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO stamp_purchase_intents (user_id, pack_id, price, payment_key) VALUES ($1, $2, 300, $3)")
        .bind(buyer.id)
        .bind(pack_uuid)
        .bind(&key)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(ctx.balance(&buyer).await.coins, 0);

    // The creator cannot delete the pack out from under the unfinished purchase.
    assert_eq!(
        ctx.reason(
            "Studio",
            "DeletePack",
            Some(&creator),
            &DeletePackRequest { pack_id: id.clone(), ..Default::default() }
        )
        .await,
        "PACK_HAS_BUYERS"
    );

    let retry = PurchasePackRequest { pack_id: id.clone(), expected_price: 300, ..Default::default() };
    let bought: PurchasePackResponse = ctx.call("Library", "PurchasePack", Some(&buyer), &retry).await.unwrap();
    assert_eq!(bought.charged, 0, "the coins moved the first time");
    assert!(bought.pack.viewer.owned && bought.pack.viewer.installed);
    assert_eq!(ctx.balance(&buyer).await.coins, 0);
    assert_eq!(ctx.balance(&creator).await.earnings, 210);
    let intents: i64 = sqlx::query_scalar("SELECT count(*) FROM stamp_purchase_intents WHERE pack_id = $1")
        .bind(pack_uuid)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(intents, 0);
}

#[tokio::test]
async fn concurrent_purchases_of_the_same_pack_charge_once() {
    let ctx = start().await;
    let (creator, buyer) = (ctx.app.user("creator").await, ctx.app.user("buyer").await);
    let id = id_of(&ctx.published(&creator, "れんだ", 100, &[], 1).await);
    ctx.grant_coins(&buyer, 500).await;

    // An impatient user taps "buy" five times.
    let taps = (0..5).map(|_| ctx.buy(&buyer, &id, 100));
    let charged: Vec<i32> =
        futures::future::join_all(taps).await.into_iter().map(|response| response.charged).collect();
    assert_eq!(charged.iter().sum::<i32>(), 100, "exactly one tap was charged: {charged:?}");
    assert_eq!(ctx.balance(&buyer).await.coins, 400);
    assert_eq!(ctx.balance(&creator).await.earnings, 70);
    let pack = ctx.get_pack(Some(&buyer), &id).await.unwrap();
    assert_eq!(pack.summary.install_count, 1);
    assert!(pack.summary.viewer.owned && pack.summary.viewer.installed);
    assert_eq!(ctx.app.events_by("stamp.pack.purchased", buyer.id).await.len(), 1, "the sale is announced once");
    assert_eq!(ctx.notifications(&creator, "stamp.pack.sold").await.len(), 1);
}

#[tokio::test]
async fn official_paid_packs_are_paid_to_the_platform() {
    let ctx = start().await;
    let (admin, buyer) = (ctx.app.user_with_roles("admin", &["admin"]).await, ctx.app.user("buyer").await);
    let request =
        CreatePackRequest { name: "こうしき有料".into(), price: 50, official: true, ..Default::default() };
    let created: CreatePackResponse = ctx.call("Studio", "CreatePack", Some(&admin), &request).await.unwrap();
    let id = created.pack.summary.id.clone();
    ctx.add_stamps(&admin, &id, "s", 1).await;
    ctx.submit(&admin, &id).await;

    ctx.grant_coins(&buyer, 60).await;
    let bought = ctx.buy(&buyer, &id, 50).await;
    assert_eq!(bought.charged, 50);
    assert!(bought.pack.viewer.owned);
    assert_eq!(ctx.balance(&buyer).await.coins, 10);
    let purchases = ctx.app.events_by("stamp.pack.purchased", buyer.id).await;
    assert!(purchases[0].data["creator_id"].is_null());
    assert_eq!(purchases[0].data["net"].as_i64(), Some(0));
}

/// The state a crash between the payment and the purchase row leaves: the
/// ledger has moved `price` under the order's key, the stamp domain only
/// holds the order, made `minutes_ago`.
async fn paid_but_unrecorded(
    ctx: &Ctx,
    buyer: &tokuly_testkit::TestUser,
    creator: &tokuly_testkit::TestUser,
    pack_id: &str,
    price: i32,
    minutes_ago: i32,
) -> String {
    // Paid first, then the order written: other tests' worker passes share
    // the database and must never see this order unpaid.
    let key = format!("stamp-pack:{pack_id}:{}:{}", buyer.id, uuid::Uuid::now_v7().simple());
    tokuly_wallet::ledger::transfer(
        &ctx.app.core,
        buyer.id,
        creator.id,
        price as i64,
        30,
        "stamp.pack",
        &key,
        serde_json::json!({}),
    )
    .await
    .unwrap();
    insert_order(ctx, buyer, pack_id, price, minutes_ago, &key, false).await;
    key
}

/// An order as `PurchasePack` writes it before paying, made `minutes_ago`.
/// Returns its ledger key.
async fn open_order(
    ctx: &Ctx,
    buyer: &tokuly_testkit::TestUser,
    pack_id: &str,
    price: i32,
    minutes_ago: i32,
) -> String {
    let key = format!("stamp-pack:{pack_id}:{}:{}", buyer.id, uuid::Uuid::now_v7().simple());
    insert_order(ctx, buyer, pack_id, price, minutes_ago, &key, false).await;
    key
}

/// Writes an order; `leased` as if a settler held it for the next minutes.
async fn insert_order(
    ctx: &Ctx,
    buyer: &tokuly_testkit::TestUser,
    pack_id: &str,
    price: i32,
    minutes_ago: i32,
    key: &str,
    leased: bool,
) {
    sqlx::query(
        "INSERT INTO stamp_purchase_intents (user_id, pack_id, price, payment_key, created_at, leased_until)
         VALUES ($1, $2, $3, $4, now() - make_interval(mins => $5), CASE WHEN $6 THEN now() + interval '5 minutes' END)",
    )
    .bind(buyer.id)
    .bind(pack_id.parse::<uuid::Uuid>().unwrap())
    .bind(price)
    .bind(key)
    .bind(minutes_ago)
    .bind(leased)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
}

async fn recorded_price(ctx: &Ctx, buyer: &tokuly_testkit::TestUser, pack_id: &str) -> Option<i32> {
    sqlx::query_scalar("SELECT price FROM stamp_pack_purchases WHERE user_id = $1 AND pack_id = $2")
        .bind(buyer.id)
        .bind(pack_id.parse::<uuid::Uuid>().unwrap())
        .fetch_optional(&ctx.app.core.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_paid_purchase_is_finished_at_the_price_paid_after_the_pack_changed() {
    let ctx = start().await;
    let (creator, buyer, other) =
        (ctx.app.user("creator").await, ctx.app.user("buyer").await, ctx.app.user("other").await);

    // The price went up after the payment: the retry finishes at the old one.
    let id = id_of(&ctx.published(&creator, "ねあげ", 300, &[], 1).await);
    ctx.grant_coins(&buyer, 1000).await;
    paid_but_unrecorded(&ctx, &buyer, &creator, &id, 300, 0).await;
    sqlx::query("UPDATE stamp_packs SET price = 500 WHERE id = $1")
        .bind(id.parse::<uuid::Uuid>().unwrap())
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let retry = PurchasePackRequest { pack_id: id.clone(), expected_price: 500, ..Default::default() };
    let bought: PurchasePackResponse = ctx.call("Library", "PurchasePack", Some(&buyer), &retry).await.unwrap();
    assert_eq!(bought.charged, 0);
    assert!(bought.pack.viewer.owned);
    assert_eq!(recorded_price(&ctx, &buyer, &id).await, Some(300), "what was paid, not today's price");
    assert_eq!(ctx.balance(&buyer).await.coins, 700);

    // The pack left the store after the payment: the buyer still gets it.
    let id = id_of(&ctx.published(&creator, "うりきれ", 200, &[], 1).await);
    paid_but_unrecorded(&ctx, &buyer, &creator, &id, 200, 0).await;
    ctx.unpublish(&creator, &id).await;
    let retry = PurchasePackRequest { pack_id: id.clone(), expected_price: 200, ..Default::default() };
    let bought: PurchasePackResponse = ctx.call("Library", "PurchasePack", Some(&buyer), &retry).await.unwrap();
    assert!(bought.pack.viewer.owned && bought.charged == 0);
    assert_eq!(ctx.balance(&buyer).await.coins, 500);
    // Nobody else can buy it, though.
    ctx.grant_coins(&other, 500).await;
    assert_eq!(ctx.reason("Library", "PurchasePack", Some(&other), &retry).await, "PACK_NOT_FOUND");
}

#[tokio::test]
async fn an_unfinished_purchase_is_finished_without_the_buyer_and_keeps_the_pack() {
    let ctx = start().await;
    let (creator, buyer, broke) =
        (ctx.app.user("creator").await, ctx.app.user("buyer").await, ctx.app.user("broke").await);
    let id = id_of(&ctx.published(&creator, "ほうちされた", 300, &[], 1).await);
    ctx.grant_coins(&buyer, 300).await;
    paid_but_unrecorded(&ctx, &buyer, &creator, &id, 300, 120).await;
    // An order whose payment never went through (the buyer had no coins).
    open_order(&ctx, &broke, &id, 300, 120).await;

    // Hours later, the paid purchase still keeps the pack from being deleted.
    let delete = DeletePackRequest { pack_id: id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Studio", "DeletePack", Some(&creator), &delete).await, "PACK_HAS_BUYERS");

    settle_orders_of(&ctx, &id).await;
    let pack = ctx.get_pack(Some(&buyer), &id).await.unwrap();
    assert!(pack.summary.viewer.owned && pack.summary.viewer.installed);
    assert_eq!(ctx.balance(&buyer).await.coins, 0, "charged once");
    assert!(!ctx.get_pack(Some(&broke), &id).await.unwrap().summary.viewer.owned);
    assert_eq!(ctx.balance(&broke).await.coins, 0);
    let intents: i64 = sqlx::query_scalar("SELECT count(*) FROM stamp_purchase_intents WHERE pack_id = $1")
        .bind(id.parse::<uuid::Uuid>().unwrap())
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(intents, 0, "both are settled");
}

#[tokio::test]
async fn a_pack_whose_creator_just_left_is_not_for_sale() {
    let ctx = start().await;
    let (creator, buyer) = (ctx.app.user("creator").await, ctx.app.user("buyer").await);
    let id = id_of(&ctx.published(&creator, "さよなら", 100, &[], 1).await);
    ctx.grant_coins(&buyer, 100).await;
    // The account is erased; the stamp domain has not heard of it yet.
    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1")
        .bind(creator.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();

    let buy = PurchasePackRequest { pack_id: id.clone(), expected_price: 100, ..Default::default() };
    assert_eq!(ctx.reason("Library", "PurchasePack", Some(&buyer), &buy).await, "PACK_NOT_FOR_SALE");
    assert_eq!(ctx.balance(&buyer).await.coins, 100);
    let intents: i64 = sqlx::query_scalar("SELECT count(*) FROM stamp_purchase_intents WHERE user_id = $1")
        .bind(buyer.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(intents, 0, "nothing was paid, so nothing is left to settle");
}

async fn open_orders(ctx: &Ctx, pack_id: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM stamp_purchase_intents WHERE pack_id = $1")
        .bind(pack_id.parse::<uuid::Uuid>().unwrap())
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap()
}

/// Runs worker passes until the pack's orders are settled. Tests share a
/// database, so another test's pass may hold one of them for a moment.
async fn settle_orders_of(ctx: &Ctx, pack_id: &str) {
    for _ in 0..50 {
        ctx.stamp.settle_unfinished_purchases().await.unwrap();
        if open_orders(ctx, pack_id).await == 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the orders of {pack_id} were not settled");
}

async fn set_pack(ctx: &Ctx, pack_id: &str, assignment: &str) {
    sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE stamp_packs SET {assignment} WHERE id = $1")))
        .bind(pack_id.parse::<uuid::Uuid>().unwrap())
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
}

#[tokio::test]
async fn an_order_ends_cleanly_whatever_happened_to_the_pack() {
    let ctx = start().await;
    let (creator, buyer) = (ctx.app.user("creator").await, ctx.app.user("buyer").await);
    ctx.grant_coins(&buyer, 1000).await;
    let buy = |id: &str, price: i32| PurchasePackRequest {
        pack_id: id.to_owned(),
        expected_price: price,
        ..Default::default()
    };

    // Never paid, and the pack left the store: the order ends, nothing is charged.
    let unsold = id_of(&ctx.published(&creator, "みはらい", 100, &[], 1).await);
    open_order(&ctx, &buyer, &unsold, 100, 0).await;
    ctx.unpublish(&creator, &unsold).await;
    assert_eq!(ctx.reason("Library", "PurchasePack", Some(&buyer), &buy(&unsold, 100)).await, "PACK_NOT_FOR_SALE");
    assert_eq!((ctx.balance(&buyer).await.coins, open_orders(&ctx, &unsold).await), (1000, 0));
    ctx.delete_pack(&creator, &unsold).await;

    // Paid, then made free: the buyer gets what they paid for.
    let freed = id_of(&ctx.published(&creator, "むりょうに", 100, &[], 1).await);
    paid_but_unrecorded(&ctx, &buyer, &creator, &freed, 100, 0).await;
    set_pack(&ctx, &freed, "price = 0").await;
    let bought: PurchasePackResponse =
        ctx.call("Library", "PurchasePack", Some(&buyer), &buy(&freed, 0)).await.unwrap();
    assert!(bought.pack.viewer.owned && bought.charged == 0);
    assert_eq!(recorded_price(&ctx, &buyer, &freed).await, Some(100));
    assert_eq!(ctx.balance(&buyer).await.coins, 900);

    // Paid, then taken down by moderators: refunded once, and the order is over.
    let removed = id_of(&ctx.published(&creator, "けされた", 200, &[], 1).await);
    paid_but_unrecorded(&ctx, &buyer, &creator, &removed, 200, 0).await;
    assert_eq!(ctx.balance(&buyer).await.coins, 700);
    set_pack(&ctx, &removed, "status = 5").await;
    assert_eq!(ctx.reason("Library", "PurchasePack", Some(&buyer), &buy(&removed, 200)).await, "PACK_NOT_FOR_SALE");
    assert_eq!(ctx.balance(&buyer).await.coins, 900, "refunded");
    assert_eq!(open_orders(&ctx, &removed).await, 0);
    assert_eq!(ctx.reason("Library", "PurchasePack", Some(&buyer), &buy(&removed, 200)).await, "PACK_NOT_FOUND");
    ctx.stamp.settle_unfinished_purchases().await.unwrap();
    assert_eq!(ctx.balance(&buyer).await.coins, 900, "and only once");

    // A later order of the same pack pays under a key of its own.
    let again = id_of(&ctx.published(&creator, "もういちど", 100, &[], 1).await);
    open_order(&ctx, &buyer, &again, 100, 0).await;
    let bought: PurchasePackResponse =
        ctx.call("Library", "PurchasePack", Some(&buyer), &buy(&again, 100)).await.unwrap();
    assert!(bought.pack.viewer.owned && bought.charged == 100);
    assert_eq!(ctx.balance(&buyer).await.coins, 800);
}

#[tokio::test]
async fn retries_and_the_worker_settle_a_paid_order_once() {
    let ctx = start().await;
    let (creator, buyer) = (ctx.app.user("creator").await, ctx.app.user("buyer").await);
    let id = id_of(&ctx.published(&creator, "どうじに", 100, &[], 1).await);
    ctx.grant_coins(&buyer, 300).await;
    paid_but_unrecorded(&ctx, &buyer, &creator, &id, 100, 20).await;
    // The pack left the store between the payment and the retries.
    ctx.unpublish(&creator, &id).await;

    let retry = PurchasePackRequest { pack_id: id.clone(), expected_price: 100, ..Default::default() };
    let retries = (0..4).map(|_| ctx.call::<_, PurchasePackResponse>("Library", "PurchasePack", Some(&buyer), &retry));
    let (answers, worker) = tokio::join!(futures::future::join_all(retries), ctx.stamp.settle_unfinished_purchases());
    worker.unwrap();
    for answer in answers {
        let answer = answer.unwrap();
        assert!(answer.pack.viewer.owned && answer.charged == 0);
    }
    let purchases: i64 = sqlx::query_scalar("SELECT count(*) FROM stamp_pack_purchases WHERE pack_id = $1")
        .bind(id.parse::<uuid::Uuid>().unwrap())
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(purchases, 1);
    assert_eq!((ctx.balance(&buyer).await.coins, ctx.balance(&creator).await.earnings), (200, 70), "paid once, kept");
    assert_eq!(ctx.app.events_by("stamp.pack.purchased", buyer.id).await.len(), 1);
    assert_eq!(open_orders(&ctx, &id).await, 0);
}

#[tokio::test]
async fn orders_that_cannot_finish_do_not_hold_up_the_worker() {
    let ctx = start().await;
    let (creator, buyer) = (ctx.app.user("creator").await, ctx.app.user("buyer").await);
    let stuck_pack = id_of(&ctx.published(&creator, "つまり", 100, &[], 1).await);
    // A hundred orders a previous pass could not finish: they wait out their back-off.
    for n in 0..100 {
        let user = ctx.app.user(&format!("stuck{n}")).await;
        let key = format!("stuck:{}", user.id);
        insert_order(&ctx, &user, &stuck_pack, 100, 60, &key, true).await;
    }
    // One order some other settler holds a row lock on right now.
    let busy = ctx.app.user("busy").await;
    let busy_key = open_order(&ctx, &busy, &stuck_pack, 100, 60).await;
    let mut other = ctx.app.core.db.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM stamp_purchase_intents WHERE payment_key = $1 FOR UPDATE")
        .bind(&busy_key)
        .execute(&mut *other)
        .await
        .unwrap();
    // And the one that can be settled.
    let id = id_of(&ctx.published(&creator, "まってた", 100, &[], 1).await);
    ctx.grant_coins(&buyer, 100).await;
    paid_but_unrecorded(&ctx, &buyer, &creator, &id, 100, 60).await;

    let settled = tokio::time::timeout(std::time::Duration::from_secs(5), ctx.stamp.settle_unfinished_purchases())
        .await
        .expect("the worker does not wait for a locked order")
        .unwrap();
    assert!(settled >= 1);
    settle_orders_of(&ctx, &id).await;
    assert!(ctx.get_pack(Some(&buyer), &id).await.unwrap().summary.viewer.owned);
    other.rollback().await.unwrap();
    assert!(open_orders(&ctx, &stuck_pack).await >= 100, "the stuck ones are left for later");
}

#[tokio::test]
async fn an_erased_creators_pack_goes_with_its_last_unpaid_order() {
    let ctx = start().await;
    let (creator, buyer) = (ctx.app.user("creator").await, ctx.app.user("buyer").await);
    let id = id_of(&ctx.published(&creator, "のこされた", 100, &[], 1).await);
    open_order(&ctx, &buyer, &id, 100, 60).await;
    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1")
        .bind(creator.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.stamp.handle_event(event("identity.user.deleted", serde_json::json!({ "user_id": creator.id }))).await.unwrap();
    assert!(
        tokuly_stamp::packs::find_by_id(&ctx.app.core.db, id.parse().unwrap()).await.unwrap().is_some(),
        "kept while an order may have paid"
    );

    settle_orders_of(&ctx, &id).await;
    assert!(
        tokuly_stamp::packs::find_by_id(&ctx.app.core.db, id.parse().unwrap()).await.unwrap().is_none(),
        "then erased"
    );
}
