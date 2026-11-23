//! A share link's topic closes for everyone following it when the share
//! ends (issue #51), on a running gateway. (Friends' positions travel to
//! each friend's own connections, not over a topic: an unfriending already
//! stopped them before; `locations.rs` covers that.)
mod common;

use std::time::Duration;

use common::*;
use tokuly_proto::geo::*;
use tokuly_realtime::testing::TestGateway;

const PROMPTLY: Duration = Duration::from_secs(1);

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
