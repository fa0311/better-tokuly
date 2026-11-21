//! The workers end to end: outbox relay → event stream → wallet consumer.
//!
//! This test runs a real outbox relay, which drains the outbox of the whole
//! test database — that is why it lives alone in its own test binary.
mod common;

use std::time::Duration;

use common::*;
use tokuly_proto::{realtime::server_frame::Body, wallet::*};
use tokuly_wallet::ledger;

#[tokio::test]
async fn spawned_workers_push_balance_changes_and_close_deleted_wallets() {
    let (app, wallet) = app().await;
    tokio::spawn(tokuly_core::events::run_outbox_relay(app.core.db.clone(), app.core.bus.clone(), app.core.shutdown.clone()));
    wallet.spawn_workers();

    let (fan, streamer) = (app.user("fan").await, app.user("streamer").await);
    let mut frames = app.listen_user(streamer.id).await;
    fund(&app, &fan, 1000).await;
    ledger::transfer(&app.core, fan.id, streamer.id, 300, 30, "live.gift", &key("gift"), serde_json::json!({})).await.unwrap();

    // The consumer picks the event up from the stream and pushes the balance.
    let frame = frames.next(Duration::from_secs(20)).await.expect("the consumer pushed the balance change");
    let Some(Body::Wallet(event)) = frame.body else { panic!("not a wallet frame") };
    let Some(wallet_event::Event::BalanceChanged(change)) = event.event else { panic!("not a balance change") };
    assert_eq!(change.balance.as_option().unwrap().earnings, 210);

    // `identity.user.deleted` reaches the same consumer.
    tokuly_core::events::emit(&app.core.db, "identity.user.deleted", Some(streamer.id), &serde_json::json!({ "user_id": streamer.id })).await.unwrap();
    let mut closed = false;
    for _ in 0..100 {
        let accounts: i64 = sqlx::query_scalar("SELECT count(*) FROM wallet_accounts WHERE user_id = $1").bind(streamer.id).fetch_one(&app.core.db).await.unwrap();
        if accounts == 0 {
            closed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(closed, "the deleted user's wallet was closed by the worker");
    assert_eq!(postings_sum(&app).await, 0);
    app.core.shutdown.cancel();
}
