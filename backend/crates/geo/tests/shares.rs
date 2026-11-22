//! Live shares: public links to a live location.
mod common;

use chrono::Utc;
use common::*;
use tokuly_core::time::from_ts;
use tokuly_proto::geo::{geo_event::Event as Push, *};
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

const LOCATIONS: &str = "tokuly.geo.v1.LocationService";
const WATCH: &str = "tokuly.geo.v1.LocationService/WatchLiveShare";

async fn create_share(ctx: &Ctx, user: &TestUser, duration_seconds: i32, label: &str) -> Result<LiveShare, RpcError> {
    let response: CreateLiveShareResponse = ctx
        .location(
            "CreateLiveShare",
            user,
            &CreateLiveShareRequest { duration_seconds, label: label.to_owned(), ..Default::default() },
        )
        .await?;
    Ok(response.share.into_option().expect("share"))
}

async fn my_shares(ctx: &Ctx, user: &TestUser) -> Vec<LiveShare> {
    let response: ListLiveSharesResponse =
        ctx.location("ListLiveShares", user, &ListLiveSharesRequest::default()).await.unwrap();
    response.shares
}

/// What anyone holding the link gets (no account).
async fn open_link(ctx: &Ctx, token: &str) -> Result<GetLiveShareResponse, RpcError> {
    ctx.app
        .call(
            &format!("{LOCATIONS}/GetLiveShare"),
            None,
            &GetLiveShareRequest { token: token.to_owned(), ..Default::default() },
        )
        .await
}

async fn revoke(ctx: &Ctx, user: &TestUser, share_id: &str) -> Result<RevokeLiveShareResponse, RpcError> {
    ctx.location(
        "RevokeLiveShare",
        user,
        &RevokeLiveShareRequest { share_id: share_id.to_owned(), ..Default::default() },
    )
    .await
}

fn token_of(share: &LiveShare) -> String {
    share.url.rsplit('/').next().expect("a URL with a token").to_owned()
}

#[tokio::test]
async fn a_share_link_shows_the_live_pin_to_anyone_who_has_it() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    ctx.report(&aoi, vec![LocationSample { battery_percent: Some(33), ..sample(home, Utc::now()) }]).await;
    // Ghost mode is about friends; a share link is its own, explicit grant.
    ctx.ghost(&aoi, &[], GhostMode::GHOST_MODE_FROZEN, 0).await.unwrap();
    let here = offset(home, 90.0, 1500.0);
    ctx.at(&aoi, here).await;

    let share = create_share(&ctx, &aoi, 600, " 迎えに来て ").await.unwrap();
    assert_eq!(share.label, "迎えに来て");
    let token = token_of(&share);
    assert_eq!(share.url, format!("http://localhost:8080/share/{token}"));
    assert_eq!(token.len(), 32);
    let lifetime =
        (from_ts(share.expires_at.as_option().unwrap()) - from_ts(share.created_at.as_option().unwrap())).num_seconds();
    assert!((599..=601).contains(&lifetime));
    assert_eq!(my_shares(&ctx, &aoi).await, vec![share.clone()]);
    assert!(my_shares(&ctx, &ren).await.is_empty());

    let opened = open_link(&ctx, &token).await.unwrap();
    assert_eq!(
        (opened.user.as_option().unwrap().id.clone(), opened.label.as_str()),
        (aoi.id.to_string(), "迎えに来て")
    );
    assert_eq!(opened.expires_at, share.expires_at);
    let pin = opened.location.as_option().expect("the live pin");
    assert_at!(pin_point(pin), here);
    assert_eq!(pin.battery_percent, Some(33));
    assert!(pin.status.as_option().is_none() && !pin.online, "nothing meant for friends");
    // Each visitor counts once.
    open_link(&ctx, &token).await.unwrap();
    assert_eq!(my_shares(&ctx, &aoi).await[0].view_count, 1);

    // Updates are published on the share's realtime topic.
    let mut topic = ctx.app.listen_topic(&format!("share:{token}")).await;
    let moved = offset(here, 90.0, 200.0);
    ctx.at(&aoi, moved).await;
    let frame = topic.next(SOON).await.expect("a frame on the share topic");
    assert_eq!(frame.topic, format!("share:{token}"));
    let Some(tokuly_proto::realtime::server_frame::Body::Geo(event)) = frame.body else { panic!("a geo event") };
    assert!(matches!(event.event, Some(Push::Location(pin)) if at(pin_point(&pin), moved)));

    // Revoking ends it for everyone, at once.
    assert_eq!(revoke(&ctx, &ren, &share.id).await.unwrap_err().reason, "SHARE_NOT_FOUND");
    assert_eq!(revoke(&ctx, &aoi, "x").await.unwrap_err().reason, "INVALID_ID");
    revoke(&ctx, &aoi, &share.id).await.unwrap();
    let frame = topic.next(SOON).await.expect("the end of the share on its topic");
    let Some(tokuly_proto::realtime::server_frame::Body::Geo(event)) = frame.body else { panic!("a geo event") };
    assert!(matches!(event.event, Some(Push::LocationRemovedUserId(id)) if id == aoi.id.to_string()));
    let gone = open_link(&ctx, &token).await.unwrap_err();
    assert_eq!((gone.code.as_str(), gone.reason.as_str()), ("not_found", "SHARE_NOT_FOUND"));
    assert!(my_shares(&ctx, &aoi).await.is_empty());
    revoke(&ctx, &aoi, &share.id).await.unwrap(); // revoking twice is fine
    ctx.at(&aoi, offset(moved, 90.0, 200.0)).await;
    assert!(topic.next(QUIET).await.is_none(), "nothing more is published");
}

#[tokio::test]
async fn a_share_can_be_watched_as_a_stream_until_it_ends() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    let home = somewhere();
    ctx.at(&aoi, home).await;
    let share = create_share(&ctx, &aoi, 3600, "").await.unwrap();
    let token = token_of(&share);
    let request = WatchLiveShareRequest { token: token.clone(), ..Default::default() };

    // The current pin first, then updates.
    let watch = ctx.app.stream::<_, WatchLiveShareResponse>(WATCH, None, &request, 2, SOON * 3);
    tokio::pin!(watch);
    let mut step = 0;
    let messages = loop {
        tokio::select! {
            messages = &mut watch => break messages.unwrap(),
            _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                step += 1;
                ctx.at(&aoi, offset(home, 0.0, 400.0 + step as f64 * 10.0)).await;
            }
        }
    };
    assert_eq!(messages.len(), 2);
    assert!(messages.iter().all(|message| !message.ended && message.location.as_option().is_some()));
    assert!(distance(pin_point(messages[1].location.as_option().unwrap()), home) >= 400.0);

    // Revoked while someone watches: the stream says so and ends.
    let watch = ctx.app.stream::<_, WatchLiveShareResponse>(WATCH, None, &request, 5, SOON * 3);
    tokio::pin!(watch);
    let messages = loop {
        tokio::select! {
            messages = &mut watch => break messages.unwrap(),
            _ = tokio::time::sleep(std::time::Duration::from_millis(700)) => {
                revoke(&ctx, &aoi, &share.id).await.unwrap();
            }
        }
    };
    let last = messages.last().expect("at least the end");
    assert!(last.ended && last.location.as_option().is_none());
    assert!(messages.len() < 5, "the stream closed after the end");

    let after = ctx.app.stream::<_, WatchLiveShareResponse>(WATCH, None, &request, 1, SOON).await;
    assert_eq!(after.unwrap_err().reason, "SHARE_NOT_FOUND");
}

#[tokio::test]
async fn shares_are_limited_validated_and_expire() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    ctx.at(&aoi, somewhere()).await;
    for (duration, label) in [(60, ""), (0, ""), (100_000, "")] {
        assert_eq!(
            create_share(&ctx, &aoi, duration, label).await.unwrap_err().reason,
            "INVALID_DURATION",
            "{duration} s"
        );
    }
    assert_eq!(create_share(&ctx, &aoi, 600, &"あ".repeat(61)).await.unwrap_err().reason, "LABEL_TOO_LONG");
    let anonymous = ctx
        .app
        .call::<_, CreateLiveShareResponse>(
            &format!("{LOCATIONS}/CreateLiveShare"),
            None,
            &CreateLiveShareRequest { duration_seconds: 600, ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!(anonymous.reason, "SIGN_IN_REQUIRED");

    // Tokens that are not tokens, or not anyone's.
    for token in ["", "short", &"a".repeat(32), "../../../../etc/passwd-and-some-more"] {
        assert_eq!(open_link(&ctx, token).await.unwrap_err().reason, "SHARE_NOT_FOUND", "{token:?}");
    }

    // The shortest and the longest lifetimes, up to ten shares at a time.
    let first = create_share(&ctx, &aoi, 300, "5分").await.unwrap();
    create_share(&ctx, &aoi, 86_400, "1日").await.unwrap();
    for _ in 0..8 {
        create_share(&ctx, &aoi, 600, "").await.unwrap();
    }
    let too_many = create_share(&ctx, &aoi, 600, "").await.unwrap_err();
    assert_eq!((too_many.code.as_str(), too_many.reason.as_str()), ("failed_precondition", "TOO_MANY_SHARES"));
    let listed = my_shares(&ctx, &aoi).await;
    assert_eq!(listed.len(), 10);
    assert!(
        listed
            .windows(2)
            .all(|pair| from_ts(pair[0].created_at.as_option().unwrap())
                >= from_ts(pair[1].created_at.as_option().unwrap())),
        "newest first"
    );
    let tokens: std::collections::HashSet<String> = listed.iter().map(token_of).collect();
    assert_eq!(tokens.len(), 10, "every share has its own token");

    // Time runs out: the link stops working even before the sweeper tidies up, and followers are told.
    let token = token_of(&first);
    let mut topic = ctx.app.listen_topic(&format!("share:{token}")).await;
    sqlx::query("UPDATE geo_live_shares SET expires_at = now() - interval '1 second' WHERE id = $1")
        .bind(Uuid::parse_str(&first.id).unwrap())
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(open_link(&ctx, &token).await.unwrap_err().reason, "SHARE_NOT_FOUND");
    assert_eq!(my_shares(&ctx, &aoi).await.len(), 9);
    assert!(ctx.geo.expire_live_shares().await.unwrap() >= 1);
    let frame = topic.next(SOON).await.expect("the end of the share on its topic");
    let Some(tokuly_proto::realtime::server_frame::Body::Geo(event)) = frame.body else { panic!("a geo event") };
    assert!(matches!(event.event, Some(Push::LocationRemovedUserId(_))));
    // A slot is free again.
    create_share(&ctx, &aoi, 600, "").await.unwrap();
}

#[tokio::test]
async fn a_share_before_any_location_has_no_pin_yet() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    let share = create_share(&ctx, &aoi, 600, "").await.unwrap();
    let opened = open_link(&ctx, &token_of(&share)).await.unwrap();
    assert!(opened.location.as_option().is_none());
    assert_eq!(opened.user.as_option().unwrap().id, aoi.id.to_string());
    // The first sample appears.
    let here = somewhere();
    ctx.at(&aoi, here).await;
    assert_at!(pin_point(open_link(&ctx, &token_of(&share)).await.unwrap().location.as_option().unwrap()), here);
}
