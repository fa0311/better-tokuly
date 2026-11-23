//! Removal from a study room ends the subscription to its topic at once
//! (issue #51), on a running gateway.
mod common;

use std::time::Duration;

use common::*;
use tokuly_proto::school::*;
use tokuly_realtime::testing::TestGateway;

const PROMPTLY: Duration = Duration::from_secs(1);

#[tokio::test]
async fn a_removed_member_stops_hearing_the_room() {
    let ctx = start().await;
    let gateway = TestGateway::start(&ctx.app.core).await;
    let host = ctx.app.user("host").await;
    let member = ctx.app.user("member").await;
    let created = ctx.create_room(&host, |request| request.audience = RoomAudience::ROOM_AUDIENCE_PUBLIC.into()).await;
    let room = created.room.as_option().cloned().expect("the room");
    ctx.join(&member, &room.id).await.unwrap();

    let mut hosts = gateway.client(Some(&host.token)).await;
    let mut members = gateway.client(Some(&member.token)).await;
    hosts.subscribe(&[&room.topic]).await.unwrap();
    members.subscribe(&[&room.topic]).await.unwrap();

    let remove =
        RemoveStudyRoomMemberRequest { room_id: room.id.clone(), user_id: member.id.to_string(), ..Default::default() };
    let _: RemoveStudyRoomMemberResponse = ctx.ok(ROOMS, "RemoveStudyRoomMember", &host, &remove).await;
    assert_eq!(members.unsubscribed(&room.topic, PROMPTLY).await.as_deref(), Some("TOPIC_FORBIDDEN"));

    ctx.app.core.rt.to_topic(&room.topic, &SchoolEvent::default()).await.unwrap();
    assert!(hosts.next_event_on::<SchoolEvent>(&room.topic, PROMPTLY).await.is_some());
    assert!(members.hears_nothing_on(&room.topic, Duration::from_millis(400)).await);
    assert_eq!(members.subscribe(&[&room.topic]).await, Err("TOPIC_FORBIDDEN".into()));
}
