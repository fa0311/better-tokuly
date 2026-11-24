//! How the gateway answers a subscription: the `Ack` of a `Subscribe` (and
//! the `Welcome` of a Hello naming topics) means the subscription is in
//! effect, so a client that reads a snapshot after it misses nothing; a
//! refusal names the guard's verdict, as `Unsubscribed` does.
use std::sync::Arc;

use tokuly_core::{AppError, AppResult, realtime::TopicGuard};
use tokuly_proto::{
    geo::{GeoEvent, geo_event::Event as GeoEventKind},
    realtime::{ServerFrame, server_frame::Body},
};
use uuid::Uuid;

mod support;
use support::*;

fn marker(text: &str) -> GeoEvent {
    GeoEvent { event: Some(GeoEventKind::LocationRemovedUserId(text.to_owned())), ..Default::default() }
}

fn marker_text(frame: &ServerFrame) -> Option<String> {
    match frame.event::<GeoEvent>()?.event {
        Some(GeoEventKind::LocationRemovedUserId(text)) => Some(text),
        _ => None,
    }
}

/// Says no to everyone.
struct Nobody;

#[async_trait::async_trait]
impl TopicGuard for Nobody {
    async fn may_subscribe(&self, _subscriber: Option<Uuid>, _topic: &str) -> AppResult<bool> {
        Ok(false)
    }
}

/// Cannot decide (its database is away).
struct Broken;

#[async_trait::async_trait]
impl TopicGuard for Broken {
    async fn may_subscribe(&self, _subscriber: Option<Uuid>, _topic: &str) -> AppResult<bool> {
        Err(AppError::unavailable("GUARD_DOWN", "the guard's database is away"))
    }
}

#[tokio::test]
async fn a_frame_published_right_after_the_ack_arrives() {
    let harness = Harness::start().await;
    let rt = &harness.app.core.rt;
    let mut first = harness.ws_as(None, true).await;
    let mut second = harness.ws_as(None, true).await;
    // Fresh topics: each Subscribe makes a route and its bus subscription,
    // which the Ack has to wait for. A second connection joins a route
    // that is already there.
    for round in 0..10 {
        let topic = unique_topic("live");
        assert!(is_ack(&first.request(subscribe_body(&[&topic])).await));
        rt.to_topic(&topic, &marker(&format!("first {round}"))).await.unwrap();
        let frame = first.next_where(|frame| marker_text(frame).is_some()).await;
        assert_eq!(marker_text(&frame), Some(format!("first {round}")), "nothing published after the Ack is lost");

        assert!(is_ack(&second.request(subscribe_body(&[&topic])).await));
        rt.to_topic(&topic, &marker(&format!("second {round}"))).await.unwrap();
        let frame = second.next_where(|frame| marker_text(frame).is_some()).await;
        assert_eq!(marker_text(&frame), Some(format!("second {round}")));
    }
}

#[tokio::test]
async fn a_frame_published_right_after_welcome_arrives_for_the_hellos_topics() {
    let harness = Harness::start().await;
    for round in 0..10 {
        let topic = unique_topic("live");
        let mut client = harness.ws().await;
        let reply = client.request(hello_body("", true, &[&topic])).await;
        assert!(matches!(reply.body, Some(Body::Welcome(_))));
        harness.app.core.rt.to_topic(&topic, &marker(&format!("round {round}"))).await.unwrap();
        let frame = client.next_where(|frame| marker_text(frame).is_some()).await;
        assert_eq!(marker_text(&frame), Some(format!("round {round}")));
    }
}

#[tokio::test]
async fn a_refused_subscribe_says_why_and_subscribes_nothing() {
    let harness = Harness::start().await;
    harness.app.core.topic_guards.register("closed", Arc::new(Nobody));
    harness.app.core.topic_guards.register("broken", Arc::new(Broken));
    let rt = &harness.app.core.rt;
    let mut client = harness.ws_as(None, true).await;

    // The guard said no: the same reason `Unsubscribed` gives when access ends later.
    let (closed, open) = (unique_topic("closed"), unique_topic("live"));
    let error = error_of(&client.request(subscribe_body(&[&open, &closed])).await).clone();
    assert_eq!(
        (error.code.as_str(), error.message.as_str(), error.fatal),
        ("permission_denied", "TOPIC_FORBIDDEN", false)
    );
    // The guard could not decide: not a refusal of access, the client may try again.
    let broken = unique_topic("broken");
    let error = error_of(&client.request(subscribe_body(&[&broken])).await).clone();
    assert_eq!(
        (error.code.as_str(), error.message.as_str(), error.fatal),
        ("unavailable", "TOPIC_CHECK_FAILED", false)
    );

    // All or nothing: not even the open topic of the first request was subscribed.
    for topic in [&open, &closed, &broken] {
        rt.to_topic(topic, &marker("not for you")).await.unwrap();
    }
    assert!(client.stays_quiet().await);
}

#[tokio::test]
async fn a_hello_tells_its_refused_topics_before_welcome() {
    let harness = Harness::start().await;
    harness.app.core.topic_guards.register("closed", Arc::new(Nobody));
    let (closed, open) = (unique_topic("closed"), unique_topic("live"));
    let mut client = harness.ws().await;
    let id = client.send(hello_body("", true, &[&closed, &open])).await;

    // Whatever was not refused before `Welcome` is subscribed when it arrives.
    let mut before_welcome = Vec::new();
    loop {
        let frame = client.next_where(|frame| !is_topic_stats(frame)).await;
        match frame.body {
            Some(Body::Welcome(_)) => {
                assert_eq!(frame.reply_to, id);
                break;
            }
            Some(Body::Unsubscribed(unsubscribed)) => {
                before_welcome.push((unsubscribed.topic.clone(), unsubscribed.reason.clone()))
            }
            other => panic!("unexpected frame before Welcome: {other:?}"),
        }
    }
    assert_eq!(before_welcome, vec![(closed.clone(), "TOPIC_FORBIDDEN".to_owned())]);
    harness.app.core.rt.to_topic(&open, &marker("open")).await.unwrap();
    assert_eq!(marker_text(&client.next_where(|frame| marker_text(frame).is_some()).await).as_deref(), Some("open"));
    assert!(client.stays_quiet().await, "no second Unsubscribed after Welcome");
}
