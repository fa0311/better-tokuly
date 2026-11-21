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
    let key = format!("stamp-pack:{id}:{}", buyer.id);
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
    sqlx::query("INSERT INTO stamp_purchase_intents (user_id, pack_id) VALUES ($1, $2)")
        .bind(buyer.id)
        .bind(pack_uuid)
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
