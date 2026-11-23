//! What ends on a running gateway when location sharing ends (issue #51):
//! a friend's positions travel to each friend's own connections, so an
//! unfriending stops them for the former friend; a share link's topic
//! closes for everyone following it when the share ends.
mod common;

use std::time::Duration;

use common::*;
use tokuly_proto::{
    geo::{geo_event::Event as Push, *},
    realtime::ServerFrame,
};
use tokuly_realtime::testing::{TestClient, TestGateway};

const PROMPTLY: Duration = Duration::from_secs(1);

/// The geo pushes reaching a connection within `window`, user-scoped ones only.
async fn geo_pushes(client: &mut TestClient, window: Duration) -> Vec<Push> {
    let deadline = tokio::time::Instant::now() + window;
    let mut pushes = Vec::new();
    while let Some(frame) = client.next_within(deadline.saturating_duration_since(tokio::time::Instant::now())).await {
        if let Some(event) = ServerFrame::event::<GeoEvent>(&frame).and_then(|event| event.event) {
            pushes.push(event);
        }
    }
    pushes
}

#[tokio::test]
async fn an_unfriended_friend_stops_getting_positions_at_once() {
    let ctx = start().await;
    let gateway = TestGateway::start(&ctx.app.core).await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    let mut rens = gateway.client(Some(&ren.token)).await;
    ctx.at(&aoi, home).await;
    assert!(
        geo_pushes(&mut rens, PROMPTLY)
            .await
            .iter()
            .any(|push| matches!(push, Push::Location(pin) if pin.user_id == aoi.id.to_string())),
        "a friend's position arrives"
    );

    ctx.unfriend(&aoi, &ren).await;
    let after = geo_pushes(&mut rens, PROMPTLY).await;
    assert!(
        after.iter().any(|push| matches!(push, Push::LocationRemovedUserId(id) if *id == aoi.id.to_string())),
        "{after:?}"
    );
    ctx.at(&aoi, offset(home, 0.0, 80.0)).await;
    assert!(
        !geo_pushes(&mut rens, Duration::from_millis(400)).await.iter().any(|push| matches!(push, Push::Location(_))),
        "nothing more of the former friend"
    );
}

#[tokio::test]
async fn a_revoked_share_lets_its_followers_go() {
    let ctx = start().await;
    let gateway = TestGateway::start(&ctx.app.core).await;
    let aoi = ctx.app.user("aoi").await;
    ctx.at(&aoi, somewhere()).await;
    let created: CreateLiveShareResponse = ctx
        .location("CreateLiveShare", &aoi, &CreateLiveShareRequest { duration_seconds: 600, ..Default::default() })
        .await
        .unwrap();
    let share = created.share.into_option().expect("share");
    let token = share.url.rsplit('/').next().expect("a token").to_owned();
    let topic = format!("share:{token}");
    let mut guest = gateway.client(None).await;
    guest.subscribe(&[&topic]).await.unwrap();

    let _: RevokeLiveShareResponse = ctx
        .location("RevokeLiveShare", &aoi, &RevokeLiveShareRequest { share_id: share.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(guest.unsubscribed(&topic, PROMPTLY).await.as_deref(), Some("TOPIC_CLOSED"));
    ctx.at(&aoi, offset(somewhere(), 90.0, 100.0)).await;
    assert!(guest.hears_nothing_on(&topic, Duration::from_millis(400)).await);
}
