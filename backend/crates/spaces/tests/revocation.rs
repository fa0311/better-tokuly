//! A ban ends the banned person's subscription to the room's topic at once
//! (issue #51), on a running gateway.
mod common;

use std::time::Duration;

use common::*;
use tokuly_proto::spaces::*;
use tokuly_realtime::testing::TestGateway;

const PROMPTLY: Duration = Duration::from_secs(1);

#[tokio::test]
async fn a_banned_listener_stops_hearing_the_room() {
    let ctx = start().await;
    let gateway = TestGateway::start(&ctx.app.core).await;
    let host = ctx.app.user("taro").await;
    let listener = ctx.app.user("jiro").await;
    let (room, _host_credentials) = ctx.open_room(&host, "出禁テスト", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;
    let topic = format!("space:{}", room.id);
    let _ = ctx.join(&listener, &room.id).await;

    let mut hosts = gateway.client(Some(&host.token)).await;
    let mut listeners = gateway.client(Some(&listener.token)).await;
    hosts.subscribe(&[&topic]).await.unwrap();
    listeners.subscribe(&[&topic]).await.unwrap();

    let remove = RemoveParticipantRequest {
        space_id: room.id.clone(),
        user_id: listener.id.to_string(),
        ban: true,
        ..Default::default()
    };
    let _: RemoveParticipantResponse = ctx.call("Space", "RemoveParticipant", &host, &remove).await.unwrap();
    assert_eq!(listeners.unsubscribed(&topic, PROMPTLY).await.as_deref(), Some("TOPIC_FORBIDDEN"));

    // What the room says from now on reaches the host only.
    ctx.app.core.rt.to_topic(&topic, &SpacesEvent::default()).await.unwrap();
    assert!(hosts.next_event_on::<SpacesEvent>(&topic, PROMPTLY).await.is_some());
    assert!(listeners.hears_nothing_on(&topic, Duration::from_millis(400)).await);
    // And the banned person cannot follow it again.
    assert_eq!(listeners.subscribe(&[&topic]).await, Err("TOPIC_FORBIDDEN".into()));
}
