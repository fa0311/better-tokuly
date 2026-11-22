//! The bus against a real NATS server (`TOKULY_NATS_URL`, as exported by
//! `tools/dev/infra.sh env`).
use std::time::Duration;

use async_nats::jetstream::consumer::{Consumer, pull};
use futures::StreamExt as _;
use tokuly_core::bus::{Bus, STREAM_ANALYTICS, STREAM_EVENTS, STREAM_JOBS, STREAM_LOCATIONS};

const ACK_WAIT: Duration = Duration::from_secs(30);

/// A bus in a namespace of its own, so the test neither sees nor disturbs
/// the streams of other tests or of a running server.
async fn isolated_bus() -> Bus {
    let url = std::env::var("TOKULY_NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_owned());
    let bus =
        Bus::connect(&url, &format!("bustest_{}", uuid::Uuid::now_v7().simple())).await.expect("NATS is reachable");
    bus.ensure_streams().await.expect("streams are created");
    bus
}

async fn remove_streams(bus: &Bus) {
    for name in [STREAM_EVENTS, STREAM_JOBS, STREAM_ANALYTICS, STREAM_LOCATIONS] {
        bus.js.delete_stream(bus.stream_name(name)).await.expect("stream is deleted");
    }
}

/// Everything waiting for the consumer right now, acknowledged, as text.
async fn drain(consumer: &Consumer<pull::Config>) -> Vec<String> {
    let mut messages =
        consumer.fetch().max_messages(20).expires(Duration::from_millis(400)).messages().await.expect("fetch");
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
    let consumer =
        bus.durable_consumer(STREAM_EVENTS, "worker", vec!["evt.user.created".to_owned()], ACK_WAIT, 5).await.unwrap();
    bus.publish_durable("evt.user.created", "first", None).await.unwrap();
    assert_eq!(drain(&consumer).await, ["first"]);

    // The next release listens to a second topic under the same durable name.
    // It must start receiving that topic and keep its place in the stream.
    let topics = vec!["evt.user.created".to_owned(), "evt.user.deleted".to_owned()];
    let consumer = bus.durable_consumer(STREAM_EVENTS, "worker", topics.clone(), ACK_WAIT, 5).await.unwrap();
    bus.publish_durable("evt.user.deleted", "second", None).await.unwrap();
    bus.publish_durable("evt.user.created", "third", None).await.unwrap();
    assert_eq!(drain(&consumer).await, ["second", "third"], "the new topic arrives and nothing is replayed");

    // Starting again with the same settings changes nothing.
    let consumer = bus.durable_consumer(STREAM_EVENTS, "worker", topics, ACK_WAIT, 5).await.unwrap();
    assert!(drain(&consumer).await.is_empty());

    remove_streams(&bus).await;
}

#[tokio::test]
async fn setting_up_streams_twice_is_harmless_and_keeps_messages() {
    let bus = isolated_bus().await;
    let consumer = bus.durable_consumer(STREAM_EVENTS, "worker", vec!["evt.>".to_owned()], ACK_WAIT, 5).await.unwrap();
    bus.publish_durable("evt.note.posted", "kept", None).await.unwrap();

    bus.ensure_streams().await.unwrap();
    assert_eq!(drain(&consumer).await, ["kept"]);

    remove_streams(&bus).await;
}
