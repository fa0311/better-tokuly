//! What the worker reports of the event platform's backlog, against the dev
//! infrastructure.
use std::time::Duration;

use tokuly_core::{
    backlog,
    bus::{STREAM_EVENTS, StartAt},
    dead_letters, events,
};
use tokuly_testkit::TestApp;

#[tokio::test]
async fn the_backlog_counts_the_outbox_the_consumers_and_the_dead() {
    let app = TestApp::start(&["0001", "0100", "0400"], |_, _, router| router).await;
    let (db, bus) = (&app.core.db, &app.core.bus);
    let before = backlog::measure(db, bus).await.unwrap();

    // No relay runs in tests: what is emitted waits in the outbox.
    events::emit(db, "backlogtest.thing.created", None, &serde_json::json!({})).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    // A consumer with two events it has not been given.
    bus.durable_consumer(STREAM_EVENTS, "backlog-test", vec!["evt.backlogtest.>".to_owned()], Duration::from_secs(30), 3, StartAt::All)
        .await
        .unwrap();
    for _ in 0..2 {
        bus.publish_durable("evt.backlogtest.thing.created", "{}", None).await.unwrap();
    }
    dead_letters::bury(bus, "backlog-test", "evt.backlogtest.thing.created", b"{}", "it broke", 3).await.unwrap();

    let now = backlog::measure(db, bus).await.unwrap();
    assert!(now.oldest_outbox_age.is_some_and(|age| age >= Duration::from_secs(1)), "{:?}", now.oldest_outbox_age);
    let consumer = now.consumers.iter().find(|c| c.consumer == "backlog-test").expect("the consumer is measured");
    assert_eq!((consumer.stream, consumer.pending, consumer.unacknowledged), (STREAM_EVENTS, 2, 0));
    assert_eq!(now.dead_letters, before.dead_letters + 1);
}
