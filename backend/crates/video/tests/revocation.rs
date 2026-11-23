//! Leaving a watch party, or its end, ends the subscriptions to its topics
//! at once (issue #51), on a running gateway.
mod common;

use std::time::Duration;

use common::*;
use tokuly_proto::video::*;
use tokuly_realtime::testing::TestGateway;

const PROMPTLY: Duration = Duration::from_secs(1);

async fn join(ctx: &Ctx, user: Option<&tokuly_testkit::TestUser>, code: &str, guest_name: &str) -> PartySession {
    let request = JoinPartyRequest { code: code.to_owned(), guest_name: guest_name.to_owned(), ..Default::default() };
    let response: JoinPartyResponse = ctx.call(PARTIES, "JoinParty", user, &request).await.expect("join party");
    response.session.as_option().cloned().unwrap()
}

#[tokio::test]
async fn members_and_guests_who_leave_and_everyone_at_the_end_are_let_go() {
    let ctx = start().await;
    let gateway = TestGateway::start(&ctx.app.core).await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let channel = ctx.channel(&alice, "ch").await;
    let video = ctx.public_video(&alice, &channel, "みんなで見よう").await;
    let created: CreatePartyResponse = ctx
        .call(
            PARTIES,
            "CreateParty",
            Some(&alice),
            &CreatePartyRequest { video_id: card(&video).id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let host = created.session.as_option().cloned().unwrap();
    let party_id = host.party.id.clone();
    let member = join(&ctx, Some(&bob), &host.party.code, "").await;
    let guest = join(&ctx, None, &host.party.code, "ゲスト").await;

    let mut alices = gateway.client(Some(&alice.token)).await;
    let mut bobs = gateway.client(Some(&bob.token)).await;
    let mut guests = gateway.client(None).await;
    alices.subscribe(&[&host.realtime_topic]).await.unwrap();
    bobs.subscribe(&[&member.realtime_topic]).await.unwrap();
    guests.subscribe(&[&guest.realtime_topic]).await.unwrap();

    // A member leaves: their subscription goes, and they cannot come back to the topic.
    let _: LeavePartyResponse = ctx
        .call(
            PARTIES,
            "LeaveParty",
            Some(&bob),
            &LeavePartyRequest { party_id: party_id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(bobs.unsubscribed(&member.realtime_topic, PROMPTLY).await.as_deref(), Some("TOPIC_FORBIDDEN"));
    assert_eq!(bobs.subscribe(&[&member.realtime_topic]).await, Err("TOPIC_FORBIDDEN".into()));

    // A guest leaves: their own topic closes.
    let leave =
        LeavePartyRequest { party_id: party_id.clone(), guest_token: guest.guest_token.clone(), ..Default::default() };
    let _: LeavePartyResponse = ctx.call(PARTIES, "LeaveParty", None, &leave).await.unwrap();
    assert_eq!(guests.unsubscribed(&guest.realtime_topic, PROMPTLY).await.as_deref(), Some("TOPIC_CLOSED"));

    // The party ends: the host's topic closes after `party_ended`.
    let _: EndPartyResponse = ctx
        .call(PARTIES, "EndParty", Some(&alice), &EndPartyRequest { party_id: party_id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(alices.unsubscribed(&host.realtime_topic, PROMPTLY).await.as_deref(), Some("TOPIC_CLOSED"));
}
