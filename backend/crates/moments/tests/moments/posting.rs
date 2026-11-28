//! Posting: the day, lateness, one per day, validation, two posts at once.
use tokuly_proto::{
    common::LatLng,
    media::AssetPurpose,
    moments::{DeleteMomentRequest, DeleteMomentResponse, MomentsEvent, moments_event::Event as Push},
};

use crate::common::*;

#[tokio::test]
async fn a_moment_belongs_to_the_current_day_once() {
    let ctx = start().await;
    let a = ctx.app.user("あや").await;
    let region = ctx.region(&[&a]).await;
    assert_eq!(ctx.try_post(&a).await.unwrap_err().reason, "NO_PROMPT_YET");

    ctx.prompt(&region).await;
    let moment = ctx.post(&a).await;
    assert_eq!(moment.late_seconds, 0, "posted in the window");
    assert_eq!((moment.retakes, moment.caption.as_str(), moment.mine, moment.locked), (2, "放課後🍦", true, false));
    assert!(moment.location.is_set(), "the owner sees their own location");
    assert!(!moment.back.url.is_empty() && !moment.front.url.is_empty());
    assert_eq!(ctx.app.events_by("moments.moment.posted", a.id).await.len(), 1);
    assert_eq!(ctx.holders(&moment.back.asset_id).await, 1, "the pictures are held");

    let again = ctx.try_post(&a).await.unwrap_err();
    assert_eq!(again.reason, "ALREADY_POSTED");

    // Deleting does not give the day back.
    let _: DeleteMomentResponse = ctx
        .call("DeleteMoment", &a, &DeleteMomentRequest { moment_id: moment.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(ctx.try_post(&a).await.unwrap_err().reason, "ALREADY_POSTED");
    let today = ctx.today(&a).await;
    assert!(today.posted && !today.unlocked && today.mine.as_option().is_none());

    // The next day is a new chance.
    ctx.prompt(&region).await;
    ctx.post(&a).await;
}

#[tokio::test]
async fn a_late_moment_says_how_late() {
    let ctx = start().await;
    let a = ctx.app.user("あや").await;
    let region = ctx.region(&[&a]).await;
    let day = ctx.prompt(&region).await;
    ctx.age_day(day, 3 * 3600).await;
    let moment = ctx.post(&a).await;
    let expected = 3 * 3600 - 120;
    assert!((expected..expected + 30).contains(&moment.late_seconds), "{}", moment.late_seconds);
}

#[tokio::test]
async fn what_a_moment_is_made_of_is_checked() {
    let ctx = start().await;
    let (a, b) = (ctx.app.user("あや").await, ctx.app.user("べに").await);
    let region = ctx.region(&[&a, &b]).await;
    ctx.prompt(&region).await;
    let (ctx, a) = (&ctx, &a);
    let post =
        |request| async move { ctx.call::<_, serde_json::Value>("PostMoment", a, &request).await.unwrap_err().reason };

    let mut same = ctx.pair(a).await;
    same.front_asset_id = same.back_asset_id.clone();
    assert_eq!(post(same).await, "SAME_PICTURE");
    let mut long = ctx.pair(a).await;
    long.caption = "あ".repeat(101);
    assert_eq!(post(long).await, "INVALID_CAPTION");
    let mut retakes = ctx.pair(a).await;
    retakes.retakes = 100;
    assert_eq!(post(retakes).await, "INVALID_RETAKES");
    let mut nowhere = ctx.pair(a).await;
    nowhere.location = Some(LatLng { lat: 120.0, lng: 0.0, ..Default::default() }).into();
    assert_eq!(post(nowhere).await, "INVALID_LOCATION");
    let mut theirs = ctx.pair(a).await;
    theirs.back_asset_id = ctx.picture(&b, AssetPurpose::ASSET_PURPOSE_MOMENT).await;
    assert_eq!(post(theirs).await, "ASSET_NOT_FOUND");
    let mut avatar = ctx.pair(a).await;
    avatar.front_asset_id = ctx.picture(a, AssetPurpose::ASSET_PURPOSE_AVATAR).await;
    assert_eq!(post(avatar).await, "ASSET_WRONG_PURPOSE");

    // Nothing failed into a used day.
    ctx.post(a).await;
    let anonymous = ctx
        .app
        .call::<_, serde_json::Value>("tokuly.moments.v1.MomentService/GetToday", None, &serde_json::json!({}))
        .await;
    assert_eq!(anonymous.unwrap_err().code, "unauthenticated");
}

#[tokio::test]
async fn two_posts_at_once_make_one_moment() {
    let ctx = start().await;
    let a = ctx.app.user("あや").await;
    let region = ctx.region(&[&a]).await;
    ctx.prompt(&region).await;
    let mut requests = Vec::new();
    for _ in 0..5 {
        requests.push(ctx.pair(&a).await);
    }
    let results = futures::future::join_all(
        requests.iter().map(|request| ctx.call::<_, serde_json::Value>("PostMoment", &a, request)),
    )
    .await;
    let posted = results.iter().filter(|result| result.is_ok()).count();
    assert_eq!(posted, 1, "{results:?}");
    assert!(results.iter().filter_map(|result| result.as_ref().err()).all(|e| e.reason == "ALREADY_POSTED"));
}

#[tokio::test]
async fn friends_hear_of_a_moment_and_of_its_deletion() {
    let ctx = start().await;
    let users = ctx.circle_of(&["あや", "べに"]).await;
    let (a, b) = (&users[0], &users[1]);
    let stranger = ctx.app.user("しらない").await;
    let region = ctx.region(&[a, b, &stranger]).await;
    ctx.prompt(&region).await;
    let mut to_b = ctx.app.listen_user(b.id).await;
    let mut to_stranger = ctx.app.listen_user(stranger.id).await;

    let moment = ctx.post(a).await;
    let Some(Push::FriendPosted(posted)) = to_b.next_event::<MomentsEvent>(SOON).await.and_then(|e| e.event) else {
        panic!("no push to the friend")
    };
    assert_eq!(posted.moment_id, moment.id);
    assert!(to_stranger.events::<MomentsEvent>(QUIET).await.is_empty());

    let _: DeleteMomentResponse = ctx
        .call("DeleteMoment", a, &DeleteMomentRequest { moment_id: moment.id.clone(), ..Default::default() })
        .await
        .unwrap();
    let Some(Push::Deleted(gone)) = to_b.next_event::<MomentsEvent>(SOON).await.and_then(|e| e.event) else {
        panic!("no deletion pushed")
    };
    assert_eq!(gone.moment_id, moment.id);
    assert_eq!(ctx.app.events_by("moments.moment.deleted", a.id).await.len(), 1);
    // Someone else's moment cannot be deleted, and reads as not there.
    let other = ctx.post(b).await;
    let refused = ctx
        .call::<_, DeleteMomentResponse>(
            "DeleteMoment",
            a,
            &DeleteMomentRequest { moment_id: other.id, ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!(refused.reason, "MOMENT_NOT_FOUND");
}
