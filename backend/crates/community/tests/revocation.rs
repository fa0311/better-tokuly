//! Removal from a community that is not public ends the subscription to its
//! topic at once (issue #51), on a running gateway.
mod common;

use std::time::Duration;

use common::*;
use pb::{CommunityEvent, RemoveMemberRequest, RemoveMemberResponse};
use tokuly_realtime::testing::TestGateway;

const PROMPTLY: Duration = Duration::from_secs(1);

#[tokio::test]
async fn a_removed_member_stops_hearing_the_community() {
    let ctx = start().await;
    let gateway = TestGateway::start(&ctx.app.core).await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.restricted(&owner).await;
    let removed = ctx.member(&owner, &community, "Removed").await;
    let stays = ctx.member(&owner, &community, "Stays").await;
    let topic = format!("community:{}", community.id);

    let mut removeds = gateway.client(Some(&removed.token)).await;
    let mut stayers = gateway.client(Some(&stays.token)).await;
    removeds.subscribe(&[&topic]).await.unwrap();
    stayers.subscribe(&[&topic]).await.unwrap();

    let kick = RemoveMemberRequest {
        community_id: community.id.clone(),
        user_id: removed.id.to_string(),
        ..Default::default()
    };
    let _: RemoveMemberResponse =
        ctx.call("CommunityModerationService", "RemoveMember", Some(&owner), &kick).await.unwrap();
    assert_eq!(removeds.unsubscribed(&topic, PROMPTLY).await.as_deref(), Some("TOPIC_FORBIDDEN"));

    // A new thread is announced to the members only.
    let general = ctx.general(&owner, &community).await;
    ctx.thread(&owner, &general, "退出後のスレ", "本文").await;
    assert!(stayers.next_event_on::<CommunityEvent>(&topic, PROMPTLY).await.is_some());
    assert!(removeds.hears_nothing_on(&topic, Duration::from_millis(400)).await);
    assert_eq!(removeds.subscribe(&[&topic]).await, Err("TOPIC_FORBIDDEN".into()));
}

#[tokio::test]
async fn a_community_that_stops_being_public_lets_outsiders_go() {
    let ctx = start().await;
    let gateway = TestGateway::start(&ctx.app.core).await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let member = ctx.member(&owner, &community, "Member").await;
    let topic = format!("community:{}", community.id);
    let mut guest = gateway.client(None).await;
    let mut members = gateway.client(Some(&member.token)).await;
    guest.subscribe(&[&topic]).await.unwrap();
    members.subscribe(&[&topic]).await.unwrap();

    let narrow = pb::UpdateCommunityRequest {
        community_id: community.id.clone(),
        visibility: Some(pb::CommunityVisibility::COMMUNITY_VISIBILITY_RESTRICTED.into()),
        ..Default::default()
    };
    let _: pb::UpdateCommunityResponse =
        ctx.call("CommunityService", "UpdateCommunity", Some(&owner), &narrow).await.unwrap();
    assert_eq!(guest.unsubscribed(&topic, PROMPTLY).await.as_deref(), Some("TOPIC_FORBIDDEN"));
    assert!(members.unsubscribed(&topic, Duration::from_millis(400)).await.is_none(), "members stay");
}

#[tokio::test]
async fn a_closed_community_lets_everyone_go() {
    let ctx = start().await;
    let gateway = TestGateway::start(&ctx.app.core).await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let topic = format!("community:{}", community.id);
    let mut guest = gateway.client(None).await;
    guest.subscribe(&[&topic]).await.unwrap();
    let delete = pb::DeleteCommunityRequest { community_id: community.id.clone(), ..Default::default() };
    let _: pb::DeleteCommunityResponse =
        ctx.call("CommunityService", "DeleteCommunity", Some(&owner), &delete).await.unwrap();
    assert_eq!(guest.unsubscribed(&topic, PROMPTLY).await.as_deref(), Some("TOPIC_CLOSED"));
}
