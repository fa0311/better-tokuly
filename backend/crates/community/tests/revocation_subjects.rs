//! Removal from a community ends the removed person's subscriptions to the
//! `subject:` topics (comments, counters) of its threads too (issue #51).
//! A test binary of its own: it needs the engagement domain's tables, and
//! all tests of one binary share a database.
mod common;

use std::time::Duration;

use common::*;
use pb::{RemoveMemberRequest, RemoveMemberResponse};
use tokuly_community::Community;
use tokuly_engagement::{Engagement, EngagementConfig};
use tokuly_proto::{
    common::{SubjectRef, SubjectType},
    engagement::{
        CreateCommentRequest, CreateCommentResponse, EngagementEvent, engagement_event::Event as EngagementPush,
    },
};
use tokuly_realtime::testing::TestGateway;
use tokuly_social::{Social, SocialConfig};
use tokuly_stamp::{Stamp, StampConfig};
use tokuly_testkit::TestApp;

const PROMPTLY: Duration = Duration::from_secs(1);

#[tokio::test]
async fn a_removed_member_stops_hearing_the_comments_of_its_threads() {
    // The community with the engagement domain beside it, as the server runs them.
    let mut domain = None;
    let mut migrations = MIGRATIONS.to_vec();
    migrations.push("1100");
    migrations.sort();
    let app = TestApp::start(&migrations, |core, identity, router| {
        let social = Social::new(core.clone(), identity.clone(), SocialConfig::default()).expect("social domain");
        let stamp =
            Stamp::new(core.clone(), identity.clone(), StampConfig { seed_official: false, ..Default::default() })
                .expect("stamp domain");
        let community = Community::new(core.clone(), identity.clone(), test_config()).expect("community domain");
        let engagement =
            Engagement::new(core.clone(), identity.clone(), EngagementConfig::default()).expect("engagement domain");
        let router =
            engagement.register(community.register(stamp.register(social.register(identity.register(router)))));
        domain = Some(community);
        router
    })
    .await;
    let ctx = Ctx { app, community: domain.expect("register ran") };
    let gateway = TestGateway::start(&ctx.app.core).await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.restricted(&owner).await;
    let removed = ctx.member(&owner, &community, "Removed").await;
    let stays = ctx.member(&owner, &community, "Stays").await;
    let general = ctx.general(&owner, &community).await;
    let thread = ctx.thread(&owner, &general, "コメントのあるスレ", "本文").await;
    let subject = SubjectRef {
        r#type: SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD.into(),
        id: thread.id.clone(),
        ..Default::default()
    };
    let topic = format!("subject:{}:{}", SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD as i32, thread.id);

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

    let comment =
        CreateCommentRequest {
            subject: subject.into(), text: "メンバーだけに見えるよ".into(), ..Default::default()
        };
    let _: CreateCommentResponse =
        ctx.app.call("tokuly.engagement.v1.CommentService/CreateComment", Some(&owner), &comment).await.unwrap();
    let heard = stayers.next_event_on::<EngagementEvent>(&topic, PROMPTLY).await.and_then(|event| event.event);
    assert!(matches!(heard, Some(EngagementPush::CommentAdded(_))), "{heard:?}");
    assert!(removeds.hears_nothing_on(&topic, Duration::from_millis(400)).await);
    assert_eq!(removeds.subscribe(&[&topic]).await, Err("TOPIC_FORBIDDEN".into()));
}
