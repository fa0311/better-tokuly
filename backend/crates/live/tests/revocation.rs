//! Losing access to a stream ends the subscription to its topic at once
//! (issue #51), on a running gateway.
mod common;

use std::time::Duration;

use common::*;
use tokuly_proto::{
    common::Visibility,
    live::{LiveEvent, UpdateStreamRequest, UpdateStreamResponse},
};
use tokuly_realtime::testing::TestGateway;

const PROMPTLY: Duration = Duration::from_secs(1);

#[tokio::test]
async fn a_viewer_the_streamer_blocks_stops_hearing_the_stream() {
    let ctx = start().await;
    let gateway = TestGateway::start(&ctx.app.core).await;
    let streamer = ctx.app.user("streamer").await;
    let viewer = ctx.app.user("viewer").await;
    let fan = ctx.app.user("fan").await;
    let channel = ctx.channel(&streamer, "配信チャンネル").await;
    let stream = ctx.stream(&streamer, &channel, "ブロックのテスト").await;
    let topic = stream.realtime_topic.clone();

    let mut viewers = gateway.client(Some(&viewer.token)).await;
    let mut fans = gateway.client(Some(&fan.token)).await;
    viewers.subscribe(&[&topic]).await.unwrap();
    fans.subscribe(&[&topic]).await.unwrap();

    ctx.block(&streamer, &viewer).await;
    ctx.deliver("social.user.blocked", streamer.id).await;
    assert_eq!(viewers.unsubscribed(&topic, PROMPTLY).await.as_deref(), Some("TOPIC_FORBIDDEN"));

    ctx.app.core.rt.to_topic(&topic, &LiveEvent::default()).await.unwrap();
    assert!(fans.next_event_on::<LiveEvent>(&topic, PROMPTLY).await.is_some(), "others keep watching");
    assert!(viewers.hears_nothing_on(&topic, QUIET).await);
    assert_eq!(viewers.subscribe(&[&topic]).await, Err("TOPIC_FORBIDDEN".into()));
}

#[tokio::test]
async fn a_stream_narrowed_to_friends_lets_the_others_go() {
    let ctx = start().await;
    let gateway = TestGateway::start(&ctx.app.core).await;
    let streamer = ctx.app.user("streamer").await;
    let friend = ctx.app.user("friend").await;
    ctx.befriend(&streamer, &friend).await;
    let channel = ctx.channel(&streamer, "友達だけ").await;
    let stream = ctx.stream(&streamer, &channel, "あとで友達限定").await;
    let topic = stream.realtime_topic.clone();

    let mut friends = gateway.client(Some(&friend.token)).await;
    let mut guest = gateway.client(None).await;
    friends.subscribe(&[&topic]).await.unwrap();
    guest.subscribe(&[&topic]).await.unwrap();

    let narrow = UpdateStreamRequest {
        stream_id: card(&stream).id.clone(),
        visibility: Some(Visibility::VISIBILITY_FRIENDS.into()),
        ..Default::default()
    };
    let _: UpdateStreamResponse = ctx.call(STREAMS, "UpdateStream", Some(&streamer), &narrow).await.unwrap();
    assert_eq!(guest.unsubscribed(&topic, PROMPTLY).await.as_deref(), Some("TOPIC_FORBIDDEN"));
    ctx.app.core.rt.to_topic(&topic, &LiveEvent::default()).await.unwrap();
    assert!(friends.next_event_on::<LiveEvent>(&topic, PROMPTLY).await.is_some());

    // Then the friendship ends: the former friend goes too.
    ctx.unfriend(&streamer, &friend).await;
    assert_eq!(friends.unsubscribed(&topic, PROMPTLY).await.as_deref(), Some("TOPIC_FORBIDDEN"));
    assert_eq!(friends.subscribe(&[&topic]).await, Err("TOPIC_FORBIDDEN".into()));
}
