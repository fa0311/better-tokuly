//! A server without LiveKit: everything that needs audio says so clearly,
//! and everything else keeps working.
mod common;

use common::*;
use tokuly_proto::spaces::*;
use tokuly_spaces::{Reconciled, WEBHOOK_PATH};
use tower::ServiceExt as _;

#[tokio::test]
async fn everything_but_audio_works_without_livekit() {
    let ctx = start_without_audio().await;
    let (host, friend) = ctx.friends("taro", "hanako").await;
    assert!(!ctx.spaces.audio_enabled());
    let config: GetSpacesConfigResponse =
        ctx.anonymous("Space", "GetSpacesConfig", &GetSpacesConfigRequest::default()).await.unwrap();
    assert!(!config.config.audio_enabled, "clients can tell in advance");

    // A room cannot go live…
    let instant = CreateSpaceRequest { title: "いますぐ".into(), ..Default::default() };
    let refused = ctx.call::<_, CreateSpaceResponse>("Space", "CreateSpace", &host, &instant).await.unwrap_err();
    assert_eq!((refused.reason.as_str(), refused.code.as_str()), ("AUDIO_NOT_CONFIGURED", "unavailable"));
    // …but it can be planned, changed, waited for and called off.
    let upcoming = ctx.schedule(&host, "あとで", SpaceAudience::SPACE_AUDIENCE_FRIENDS).await;
    let update =
        UpdateSpaceRequest { space_id: upcoming.id.clone(), title: Some("あとでね".into()), ..Default::default() };
    let _: UpdateSpaceResponse = ctx.call("Space", "UpdateSpace", &host, &update).await.unwrap();
    let remind = SetReminderRequest { space_id: upcoming.id.clone(), remind: true, ..Default::default() };
    let reminded: SetReminderResponse = ctx.call("Space", "SetReminder", &friend, &remind).await.unwrap();
    assert_eq!(reminded.space.reminder_count, 1);
    let listed: ListUpcomingSpacesResponse =
        ctx.call("Space", "ListUpcomingSpaces", &friend, &ListUpcomingSpacesRequest::default()).await.unwrap();
    assert!(listed.spaces.iter().any(|space| space.id == upcoming.id));
    let pin = SetPinRequest { space_id: upcoming.id.clone(), text: "準備中".into(), ..Default::default() };
    let _: SetPinResponse = ctx.call("Space", "SetPin", &host, &pin).await.unwrap();
    let start = StartSpaceRequest { space_id: upcoming.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Space", "StartSpace", &host, &start).await, "AUDIO_NOT_CONFIGURED");
    assert_eq!(ctx.get(&host, &upcoming.id).await.state, SpaceState::SPACE_STATE_SCHEDULED, "nothing half-started");

    // A room that is live all the same (LiveKit was switched off under it).
    sqlx::query("UPDATE spaces SET state = 2, started_at = now() WHERE id = $1::uuid")
        .bind(&upcoming.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let join = JoinSpaceRequest { space_id: upcoming.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Space", "JoinSpace", &friend, &join).await, "AUDIO_NOT_CONFIGURED");
    let mute = MuteSpeakerRequest { space_id: upcoming.id.clone(), user_id: host.id.to_string(), ..Default::default() };
    assert_eq!(ctx.reason("Space", "MuteSpeaker", &host, &mute).await, "AUDIO_NOT_CONFIGURED");
    // It can still be read, listed and ended.
    assert_eq!(ctx.get(&friend, &upcoming.id).await.title, "あとでね");
    let live: ListLiveSpacesResponse = ctx
        .call(
            "Space",
            "ListLiveSpaces",
            &friend,
            &ListLiveSpacesRequest { scope: LiveScope::LIVE_SCOPE_FRIENDS.into(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(live.spaces.is_empty(), "nobody is inside");
    let ended: EndSpaceResponse = ctx
        .call("Space", "EndSpace", &host, &EndSpaceRequest { space_id: upcoming.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(ended.space.state, SpaceState::SPACE_STATE_ENDED);

    // Hangouts: the group works, the voice room does not.
    let hangout = ctx.hangout(&host, "いつメン", &[&friend]).await;
    assert_eq!(hangout.member_count, 2);
    let join = JoinHangoutRequest { hangout_id: hangout.id.clone(), ..Default::default() };
    let refused = ctx.call::<_, JoinHangoutResponse>("Hangout", "JoinHangout", &friend, &join).await.unwrap_err();
    assert_eq!((refused.reason.as_str(), refused.code.as_str()), ("AUDIO_NOT_CONFIGURED", "unavailable"));
    let listed: ListHangoutsResponse =
        ctx.call("Hangout", "ListHangouts", &friend, &ListHangoutsRequest::default()).await.unwrap();
    assert_eq!(listed.hangouts.len(), 1);

    // The workers have nothing to compare with, and the webhook says why it cannot help.
    assert_eq!(ctx.spaces.reconcile().await.unwrap(), Reconciled::default());
    assert_eq!(ctx.spaces.settle_rooms().await.unwrap(), 0);
    ctx.spaces.sweep().await.unwrap();
    let request = http::Request::builder()
        .method("POST")
        .uri(WEBHOOK_PATH)
        .header("authorization", "whatever")
        .body(axum::body::Body::from("{}"))
        .unwrap();
    assert_eq!(ctx.spaces.http_routes().oneshot(request).await.unwrap().status().as_u16(), 503);
}
