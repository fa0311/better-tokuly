//! The outbox relay of the worker process, against the dev infrastructure.
use std::time::Duration;

use futures::StreamExt as _;
use tokuly_core::{
    bus::{STREAM_EVENTS, StartAt},
    events::{self, Event},
};
use tokuly_testkit::TestApp;
use uuid::Uuid;

#[tokio::test]
async fn the_relay_moves_batches_in_order_and_drops_only_what_the_bus_cannot_take() {
    let app = TestApp::start(&["0001", "0100", "0400"], |_, _, router| router).await;
    let core = &app.core;
    let consumer = core
        .bus
        .durable_consumer(STREAM_EVENTS, "relay-test", vec!["evt.relaytest.>".to_owned()], Duration::from_secs(30), 1, StartAt::All)
        .await
        .unwrap();

    // More than two batches, with one event too large for the bus in the middle.
    let max_payload = core.bus.client.server_info().max_payload;
    for n in 0..450 {
        if n == 250 {
            let huge = "x".repeat(max_payload);
            events::emit(&core.db, "relaytest.thing.huge", None, &serde_json::json!({ "huge": huge })).await.unwrap();
        }
        events::emit(&core.db, "relaytest.thing.created", None, &serde_json::json!({ "n": n })).await.unwrap();
    }
    tokio::spawn(events::run_outbox_relay(core.db.clone(), core.bus.clone(), core.shutdown.clone()));

    let mut received = Vec::new();
    let started = std::time::Instant::now();
    while received.len() < 450 {
        assert!(started.elapsed() < Duration::from_secs(20), "only {} events arrived", received.len());
        let mut batch = consumer.fetch().max_messages(500).expires(Duration::from_millis(500)).messages().await.unwrap();
        while let Some(message) = batch.next().await {
            let message = message.unwrap();
            message.ack().await.unwrap();
            let event: Event = serde_json::from_slice(&message.payload).unwrap();
            received.push(event);
        }
    }
    let numbers: Vec<i64> = received.iter().map(|e| e.data["n"].as_i64().expect("only the small events")).collect();
    assert_eq!(numbers, (0..450).collect::<Vec<_>>(), "every event, once, in the order emitted");
    let mut ids: Vec<Uuid> = received.iter().map(|e| e.id).collect();
    ids.dedup();
    assert_eq!(ids.len(), 450);
    // The relay commits a batch once the bus has it, so the last may still be on its way out.
    loop {
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox WHERE topic LIKE 'relaytest.%'").fetch_one(&core.db).await.unwrap();
        if left == 0 {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(20), "{left} events stay in the outbox");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    core.shutdown.cancel();
}
