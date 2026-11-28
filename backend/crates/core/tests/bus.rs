//! The bus against a real NATS server (`TOKULY_NATS_URL`, as exported by
//! `tools/dev/infra.sh env`).
use std::time::Duration;

use async_nats::jetstream::consumer::{Consumer, pull};
use futures::StreamExt as _;
use tokuly_core::bus::{Bus, STREAM_ANALYTICS, STREAM_DEAD, STREAM_EVENTS, STREAM_JOBS, STREAM_LOCATIONS, StartAt};

const ACK_WAIT: Duration = Duration::from_secs(30);

fn nats_url() -> String {
    std::env::var("TOKULY_NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_owned())
}

/// A bus in a namespace of its own, so the test neither sees nor disturbs
/// the streams of other tests or of a running server.
async fn isolated_bus() -> Bus {
    let bus = Bus::connect(&nats_url(), &format!("bustest_{}", uuid::Uuid::now_v7().simple())).await.expect("NATS is reachable");
    bus.ensure_streams().await.expect("streams are created");
    bus
}

async fn remove_streams(bus: &Bus) {
    for name in [STREAM_EVENTS, STREAM_JOBS, STREAM_ANALYTICS, STREAM_LOCATIONS, STREAM_DEAD] {
        bus.js.delete_stream(bus.stream_name(name)).await.expect("stream is deleted");
    }
}

/// Everything waiting for the consumer right now, acknowledged, as text.
async fn drain(consumer: &Consumer<pull::Config>) -> Vec<String> {
    let mut messages = consumer.fetch().max_messages(20).expires(Duration::from_millis(400)).messages().await.expect("fetch");
    let mut received = Vec::new();
    while let Some(message) = messages.next().await {
        let message = message.expect("message");
        message.ack().await.expect("ack");
        received.push(String::from_utf8_lossy(&message.payload).into_owned());
    }
    received
}

#[tokio::test]
async fn a_durable_consumer_follows_its_topic_list_across_releases() {
    let bus = isolated_bus().await;

    // The first release of a worker listens to one topic.
    let consumer = bus.durable_consumer(STREAM_EVENTS, "worker", vec!["evt.user.created".to_owned()], ACK_WAIT, 5, StartAt::New).await.unwrap();
    bus.publish_durable("evt.user.created", "first", None).await.unwrap();
    assert_eq!(drain(&consumer).await, ["first"]);

    // The next release listens to a second topic under the same durable name.
    // It must start receiving that topic and keep its place in the stream.
    let topics = vec!["evt.user.created".to_owned(), "evt.user.deleted".to_owned()];
    let consumer = bus.durable_consumer(STREAM_EVENTS, "worker", topics.clone(), ACK_WAIT, 5, StartAt::New).await.unwrap();
    bus.publish_durable("evt.user.deleted", "second", None).await.unwrap();
    bus.publish_durable("evt.user.created", "third", None).await.unwrap();
    assert_eq!(drain(&consumer).await, ["second", "third"], "the new topic arrives and nothing is replayed");

    // Starting again with the same settings changes nothing.
    let consumer = bus.durable_consumer(STREAM_EVENTS, "worker", topics, ACK_WAIT, 5, StartAt::New).await.unwrap();
    assert!(drain(&consumer).await.is_empty());

    remove_streams(&bus).await;
}

#[tokio::test]
async fn setting_up_streams_twice_is_harmless_and_keeps_messages() {
    let bus = isolated_bus().await;
    let consumer = bus.durable_consumer(STREAM_EVENTS, "worker", vec!["evt.>".to_owned()], ACK_WAIT, 5, StartAt::New).await.unwrap();
    bus.publish_durable("evt.note.posted", "kept", None).await.unwrap();

    bus.ensure_streams().await.unwrap();
    assert_eq!(drain(&consumer).await, ["kept"]);

    remove_streams(&bus).await;
}

#[tokio::test]
async fn a_new_consumer_starts_from_now_unless_it_asks_for_history() {
    // A release that came before published something.
    let prefix = format!("bustest_{}", uuid::Uuid::now_v7().simple());
    let earlier = Bus::connect(&nats_url(), &prefix).await.unwrap();
    earlier.ensure_streams().await.unwrap();
    earlier.publish_durable("evt.note.posted", "before", None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let bus = Bus::connect(&nats_url(), &prefix).await.unwrap();
    // This process published something before its consumer existed.
    bus.publish_durable("evt.note.posted", "since", None).await.unwrap();

    let fresh = bus.durable_consumer(STREAM_EVENTS, "fresh", vec!["evt.>".to_owned()], ACK_WAIT, 5, StartAt::New).await.unwrap();
    let catching_up = bus.durable_consumer(STREAM_EVENTS, "catching-up", vec!["evt.>".to_owned()], ACK_WAIT, 5, StartAt::All).await.unwrap();
    bus.publish_durable("evt.note.posted", "after", None).await.unwrap();

    assert_eq!(drain(&fresh).await, ["since", "after"]);
    assert_eq!(drain(&catching_up).await, ["before", "since", "after"]);

    remove_streams(&bus).await;
}

#[tokio::test]
async fn a_consumer_from_before_start_positions_keeps_its_place() {
    let bus = isolated_bus().await;
    // What every release so far created: a consumer reading from the start.
    let old = bus.durable_consumer(STREAM_EVENTS, "worker", vec!["evt.>".to_owned()], ACK_WAIT, 5, StartAt::All).await.unwrap();
    bus.publish_durable("evt.note.posted", "handled", None).await.unwrap();
    assert_eq!(drain(&old).await, ["handled"]);
    bus.publish_durable("evt.note.posted", "waiting", None).await.unwrap();

    // The next release asks for its default; the consumer must still start and lose nothing.
    let consumer = bus.durable_consumer(STREAM_EVENTS, "worker", vec!["evt.>".to_owned()], ACK_WAIT, 5, StartAt::New).await.unwrap();
    assert_eq!(drain(&consumer).await, ["waiting"]);

    remove_streams(&bus).await;
}
