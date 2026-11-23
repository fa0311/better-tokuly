//! A block ends the blocked person's subscriptions to the blocker's
//! `subject:` topics at once (issue #51), on a running gateway.
mod common;

use std::time::Duration;

use common::*;
use tokuly_proto::engagement::EngagementEvent;
use tokuly_realtime::testing::TestGateway;

const PROMPTLY: Duration = Duration::from_secs(1);

#[tokio::test]
async fn a_blocked_reader_stops_hearing_the_comments() {
    let ctx = start().await;
    let gateway = TestGateway::start(&ctx.app.core).await;
    let owner = ctx.app.user("owner").await;
    let reader = ctx.app.user("reader").await;
    let fan = ctx.app.user("fan").await;
    let note = ctx.note(&owner, "ブロックされる前のノート");
    let topic = note.topic();

    let mut readers = gateway.client(Some(&reader.token)).await;
    let mut fans = gateway.client(Some(&fan.token)).await;
    readers.subscribe(&[&topic]).await.unwrap();
    fans.subscribe(&[&topic]).await.unwrap();

    ctx.block(&owner, &reader).await;
    assert_eq!(readers.unsubscribed(&topic, PROMPTLY).await.as_deref(), Some("TOPIC_FORBIDDEN"));

    ctx.comment(&fan, &note, "まだ見えるよ!").await;
    assert!(fans.next_event_on::<EngagementEvent>(&topic, PROMPTLY).await.is_some());
    assert!(readers.hears_nothing_on(&topic, Duration::from_millis(400)).await);
    assert_eq!(readers.subscribe(&[&topic]).await, Err("TOPIC_FORBIDDEN".into()));
}
