//! Keeping "who is inside" true: the comparison with LiveKit (people who
//! dropped off, connections without a seat, permissions that drifted), the
//! webhook that triggers it, hand-over and automatic end, and the slow
//! housekeeping.
//!
//! Every test holds [`SWEEPS`]: some of the workers look at every room in
//! the database, and must not do another test's comparing for it.
mod common;

use common::*;
use tokuly_core::livekit::RoomOptions;
use tokuly_proto::spaces::{spaces_event::Event as Push, *};
use tokuly_spaces::{Reconciled, SpacesConfig, WEBHOOK_PATH};
use tokuly_testkit::TestUser;
use tower::ServiceExt as _;

/// No patience with people who are not connected: gone at the first look.
fn impatient() -> SpacesConfig {
    SpacesConfig { connect_grace_seconds: 0, reconnect_grace_seconds: 0, ..Default::default() }
}

fn credentials_of(joined: &JoinSpaceResponse) -> AudioCredentials {
    joined.credentials.as_option().expect("credentials").clone()
}

/// Runs a statement against the test database, to make time pass.
async fn sql(ctx: &Ctx, statement: &'static str, space_id: &str) {
    sqlx::query(statement).bind(space_id).execute(&ctx.app.core.db).await.expect("test SQL");
}

async fn end_request(ctx: &Ctx, host: &TestUser, space_id: &str) {
    let _: EndSpaceResponse = ctx
        .call("Space", "EndSpace", host, &EndSpaceRequest { space_id: space_id.to_owned(), ..Default::default() })
        .await
        .expect("end room");
}

#[tokio::test]
async fn someone_who_never_connects_is_taken_out() {
    let _sweeps = SWEEPS.lock().await;
    let ctx = start_with(impatient()).await;
    let host = ctx.app.user("taro").await;
    let ghost = ctx.app.user("ghost").await;
    let (room, host_credentials) = ctx.open_room(&host, "だれかいる?", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;
    let _host_connection = ctx.connect(&host_credentials).await;
    // Entered, got a token, and never used it (the app crashed right there).
    ctx.join(&ghost, &room.id).await;
    assert_eq!(ctx.get(&host, &room.id).await.listener_count, 2);

    let mut ghost_events = ctx.listen(&ghost).await;
    let mut room_events = ctx.listen_room(&room.id).await;
    let changed = ctx.spaces.reconcile_venue(Ctx::venue(&room.id)).await.unwrap();
    assert_eq!(changed, Reconciled { ghosts: 1, strays: 0, repaired: 0 });
    let after = ctx.get(&host, &room.id).await;
    assert_eq!((after.listener_count, after.total_listeners), (1, 2));
    assert_eq!(ctx.role_of(&host, &ghost, &room.id).await, None);
    assert_eq!(ctx.role_of(&host, &host, &room.id).await, Some(SpaceRole::SPACE_ROLE_HOST), "the connected host stays");
    // The person is told (should a device of theirs still be listening), the room gets a new count.
    assert!(ctx.events(&mut ghost_events).await.iter().any(|event| matches!(
        event,
        Push::ParticipantLeft(left) if left.reason == SpaceLeaveReason::SPACE_LEAVE_REASON_DISCONNECTED
    )));
    assert!(
        ctx.events(&mut room_events)
            .await
            .iter()
            .any(|event| matches!(event, Push::Counts(counts) if counts.listener_count == 1))
    );
    // Looking again changes nothing.
    assert_eq!(ctx.spaces.reconcile_venue(Ctx::venue(&room.id)).await.unwrap(), Reconciled::default());
}

#[tokio::test]
async fn people_get_time_to_connect_and_to_reconnect() {
    let _sweeps = SWEEPS.lock().await;
    let ctx = start().await;
    let host = ctx.app.user("taro").await;
    let listener = ctx.app.user("hanako").await;
    let (room, host_credentials) = ctx.open_room(&host, "電波わるい", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;
    let _host_connection = ctx.connect(&host_credentials).await;
    let venue = Ctx::venue(&room.id);

    // Just entered, not connected yet: that is what the first 45 seconds are for.
    let credentials = credentials_of(&ctx.join(&listener, &room.id).await);
    assert_eq!(ctx.spaces.reconcile_venue(venue).await.unwrap(), Reconciled::default());
    assert_eq!(ctx.get(&host, &room.id).await.listener_count, 2);
    // …but not forever.
    sql(
        &ctx,
        "UPDATE space_participants SET joined_at = now() - interval '46 seconds' WHERE space_id = $1::uuid AND role = 1",
        &room.id,
    )
    .await;
    assert_eq!(ctx.spaces.reconcile_venue(venue).await.unwrap().ghosts, 1);

    // Connected, then the connection drops without a word.
    ctx.join(&listener, &room.id).await;
    let connection = ctx.connect(&credentials).await;
    assert_eq!(ctx.spaces.reconcile_venue(venue).await.unwrap(), Reconciled::default());
    connection.drop_dead();
    ctx.wait_until_connected(&credentials.room, &listener.id.to_string(), false).await;
    // The first look starts the clock; the person stays inside for now.
    assert_eq!(ctx.spaces.reconcile_venue(venue).await.unwrap(), Reconciled::default());
    assert_eq!(ctx.get(&host, &room.id).await.listener_count, 2);
    // They make it back in time: the clock is forgotten.
    let connection = ctx.connect(&credentials).await;
    assert_eq!(ctx.spaces.reconcile_venue(venue).await.unwrap(), Reconciled::default());
    let missing: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT missing_since FROM space_participants WHERE space_id = $1::uuid AND user_id = $2")
            .bind(&room.id)
            .bind(listener.id)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert!(missing.is_none());

    // The next time they do not come back within the twenty seconds.
    connection.drop_dead();
    ctx.wait_until_connected(&credentials.room, &listener.id.to_string(), false).await;
    assert_eq!(ctx.spaces.reconcile_venue(venue).await.unwrap(), Reconciled::default());
    sql(
        &ctx,
        "UPDATE space_participants SET missing_since = now() - interval '21 seconds' WHERE space_id = $1::uuid AND missing_since IS NOT NULL",
        &room.id,
    )
    .await;
    assert_eq!(ctx.spaces.reconcile_venue(venue).await.unwrap(), Reconciled { ghosts: 1, strays: 0, repaired: 0 });
    assert_eq!(ctx.get(&host, &room.id).await.listener_count, 1);
}

#[tokio::test]
async fn a_connection_without_a_seat_is_cut() {
    let _sweeps = SWEEPS.lock().await;
    let ctx = start().await;
    let host = ctx.app.user("taro").await;
    let leaver = ctx.app.user("hanako").await;
    let (room, host_credentials) = ctx.open_room(&host, "出入り自由", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;
    let _host_connection = ctx.connect(&host_credentials).await;
    let venue = Ctx::venue(&room.id);

    let credentials = credentials_of(&ctx.join(&leaver, &room.id).await);
    ctx.leave(&leaver, &room.id).await;
    // The token is still good for a few minutes: LiveKit lets them back in.
    let _stale = ctx.connect(&credentials).await;
    assert!(ctx.connected(&credentials.room, &leaver.id.to_string()).await.is_some());

    let changed = ctx.spaces.reconcile_venue(venue).await.unwrap();
    assert_eq!(changed, Reconciled { ghosts: 0, strays: 1, repaired: 0 });
    ctx.wait_until_connected(&credentials.room, &leaver.id.to_string(), false).await;
    assert!(ctx.connected(&credentials.room, &host.id.to_string()).await.is_some(), "the host is untouched");
    assert_eq!(ctx.get(&host, &room.id).await.listener_count, 1);
}

#[tokio::test]
async fn permissions_that_drifted_are_repaired() {
    let _sweeps = SWEEPS.lock().await;
    let ctx = start().await;
    let host = ctx.app.user("taro").await;
    let listener = ctx.app.user("hanako").await;
    let (room, host_credentials) = ctx.open_room(&host, "権限のずれ", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;
    let _host_connection = ctx.connect(&host_credentials).await;
    let credentials = credentials_of(&ctx.join(&listener, &room.id).await);
    let _connection = ctx.connect(&credentials).await;
    let venue = Ctx::venue(&room.id);
    assert_eq!(ctx.spaces.reconcile_venue(venue).await.unwrap(), Reconciled::default(), "everything is in line");

    // A promotion whose update never reached LiveKit (it was down for a moment).
    sql(&ctx, "UPDATE space_participants SET role = 2 WHERE space_id = $1::uuid AND role = 1", &room.id).await;
    let before = ctx.connected(&credentials.room, &listener.id.to_string()).await.unwrap();
    assert!(!before.permission.unwrap().can_publish);

    assert_eq!(ctx.spaces.reconcile_venue(venue).await.unwrap(), Reconciled { ghosts: 0, strays: 0, repaired: 1 });
    let after = ctx.connected(&credentials.room, &listener.id.to_string()).await.unwrap();
    assert!(after.permission.unwrap().can_publish);
    assert_eq!(after.metadata, r#"{"role":"speaker"}"#);
    assert_eq!(ctx.spaces.reconcile_venue(venue).await.unwrap(), Reconciled::default());
}

async fn post_webhook(ctx: &Ctx, authorization: Option<&str>, body: &str) -> u16 {
    let mut request =
        http::Request::builder().method("POST").uri(WEBHOOK_PATH).header("content-type", "application/webhook+json");
    if let Some(authorization) = authorization {
        request = request.header("authorization", authorization);
    }
    let request = request.body(axum::body::Body::from(body.to_owned())).unwrap();
    ctx.spaces.http_routes().oneshot(request).await.expect("the router answers").status().as_u16()
}

#[tokio::test]
async fn the_webhook_triggers_the_comparison() {
    let _sweeps = SWEEPS.lock().await;
    let ctx = start().await;
    let host = ctx.app.user("taro").await;
    let banned = ctx.app.user("jiro").await;
    let (room, host_credentials) = ctx.open_room(&host, "出禁", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;
    let _host_connection = ctx.connect(&host_credentials).await;
    let credentials = credentials_of(&ctx.join(&banned, &room.id).await);
    let remove = RemoveParticipantRequest {
        space_id: room.id.clone(),
        user_id: banned.id.to_string(),
        ban: true,
        ..Default::default()
    };
    let _: RemoveParticipantResponse = ctx.call("Space", "RemoveParticipant", &host, &remove).await.unwrap();
    // Banned, but the token in their hands still opens the door at LiveKit.
    let _sneaking = ctx.connect(&credentials).await;

    // What LiveKit posts when someone connects.
    let body = serde_json::json!({
        "event": "participant_joined",
        "room": { "sid": "RM_test", "name": credentials.room },
        "participant": { "sid": "PA_test", "identity": banned.id.to_string(), "state": "ACTIVE" },
        "id": "EV_test",
        "createdAt": "1795281571",
    })
    .to_string();
    let signature = ctx.livekit().sign_webhook(body.as_bytes()).unwrap();

    // Only LiveKit's own signature over this very body is accepted.
    assert_eq!(post_webhook(&ctx, None, &body).await, 401);
    assert_eq!(post_webhook(&ctx, Some("not-a-token"), &body).await, 401);
    let other_body = body.replace("participant_joined", "participant_left");
    assert_eq!(post_webhook(&ctx, Some(&signature), &other_body).await, 401, "the signature covers the body");
    let join_token = credentials.token.clone();
    assert_eq!(post_webhook(&ctx, Some(&join_token), &body).await, 401, "a participant's token signs nothing");
    assert!(ctx.connected(&credentials.room, &banned.id.to_string()).await.is_some(), "nothing happened so far");

    assert_eq!(post_webhook(&ctx, Some(&signature), &body).await, 200);
    ctx.wait_until_connected(&credentials.room, &banned.id.to_string(), false).await;
    assert!(ctx.connected(&credentials.room, &host.id.to_string()).await.is_some());

    // Signed, but not an event.
    let nonsense = "[1, 2, 3]";
    let signed_nonsense = ctx.livekit().sign_webhook(nonsense.as_bytes()).unwrap();
    assert_eq!(post_webhook(&ctx, Some(&signed_nonsense), nonsense).await, 400);
    // Rooms of other domains on the same LiveKit server are none of our business.
    let call = serde_json::json!({ "event": "participant_left", "room": { "name": "tokuly-call-0123456789abcdef0123456789abcdef" } })
        .to_string();
    assert_eq!(post_webhook(&ctx, Some(&ctx.livekit().sign_webhook(call.as_bytes()).unwrap()), &call).await, 200);
    let track = serde_json::json!({ "event": "track_published", "room": { "name": credentials.room } }).to_string();
    assert_eq!(post_webhook(&ctx, Some(&ctx.livekit().sign_webhook(track.as_bytes()).unwrap()), &track).await, 200);
    // The route answers POST only.
    let get = http::Request::builder().method("GET").uri(WEBHOOK_PATH).body(axum::body::Body::empty()).unwrap();
    assert_eq!(ctx.spaces.http_routes().oneshot(get).await.unwrap().status().as_u16(), 405);
}

#[tokio::test]
async fn a_cohost_takes_over_when_the_host_stays_away() {
    let _sweeps = SWEEPS.lock().await;
    let ctx = start().await;
    let host = ctx.app.user("taro").await;
    let (deputy, listener) = (ctx.app.user("deputy").await, ctx.app.user("listener").await);
    let room = ctx.public_room(&host, "生放送").await;
    let deputy_credentials = credentials_of(&ctx.join(&deputy, &room.id).await);
    let _deputy_connection = ctx.connect(&deputy_credentials).await;
    ctx.join(&listener, &room.id).await;
    ctx.make_cohost(&host, &deputy, &room.id).await;

    // The host leaves (or their app died and the comparison took them out).
    ctx.leave(&host, &room.id).await;
    assert_eq!(ctx.spaces.settle_rooms().await.unwrap(), 0, "the room waits three minutes for its host");
    assert_eq!(ctx.get(&listener, &room.id).await.host.id, host.id.to_string());

    sql(&ctx, "UPDATE spaces SET host_absent_since = now() - interval '181 seconds' WHERE id = $1::uuid", &room.id)
        .await;
    let mut room_events = ctx.listen_room(&room.id).await;
    assert_eq!(ctx.spaces.settle_rooms().await.unwrap(), 1);
    let space = ctx.get(&listener, &room.id).await;
    assert_eq!(space.state, SpaceState::SPACE_STATE_LIVE);
    assert_eq!(space.host.id, deputy.id.to_string());
    assert_eq!(ctx.role_of(&listener, &deputy, &room.id).await, Some(SpaceRole::SPACE_ROLE_HOST));
    assert!(ctx.events(&mut room_events).await.iter().any(|event| matches!(
        event,
        Push::SpaceUpdated(updated) if updated.space.host.id == deputy.id.to_string()
    )));
    let connected = ctx.connected(&deputy_credentials.room, &deputy.id.to_string()).await.unwrap();
    assert_eq!(connected.metadata, r#"{"role":"host"}"#);
    assert_eq!(ctx.spaces.settle_rooms().await.unwrap(), 0, "settled");

    // The former host comes back as a co-host of what is now someone else's room.
    assert_eq!(ctx.get(&host, &room.id).await.viewer.role, SpaceRole::SPACE_ROLE_COHOST);
    assert_eq!(ctx.join(&host, &room.id).await.me.role, SpaceRole::SPACE_ROLE_COHOST);
    end_request(&ctx, &deputy, &room.id).await;
}

#[tokio::test]
async fn a_room_without_host_or_cohost_ends() {
    let _sweeps = SWEEPS.lock().await;
    let ctx = start().await;
    let host = ctx.app.user("taro").await;
    let listener = ctx.app.user("hanako").await;
    let room = ctx.public_room(&host, "ひとり語り").await;
    ctx.join(&listener, &room.id).await;

    // A host who is back in time keeps the room.
    ctx.leave(&host, &room.id).await;
    ctx.join(&host, &room.id).await;
    sql(&ctx, "UPDATE spaces SET host_absent_since = now() - interval '181 seconds' WHERE id = $1::uuid AND host_absent_since IS NOT NULL", &room.id).await;
    assert_eq!(ctx.spaces.settle_rooms().await.unwrap(), 0);
    assert_eq!(ctx.get(&listener, &room.id).await.state, SpaceState::SPACE_STATE_LIVE);

    // A host who stays away does not.
    ctx.leave(&host, &room.id).await;
    sql(&ctx, "UPDATE spaces SET host_absent_since = now() - interval '181 seconds' WHERE id = $1::uuid", &room.id)
        .await;
    let mut listener_events = ctx.listen(&listener).await;
    assert_eq!(ctx.spaces.settle_rooms().await.unwrap(), 1);
    let ended = ctx.get(&listener, &room.id).await;
    assert_eq!(ended.state, SpaceState::SPACE_STATE_ENDED);
    assert_eq!(ended.end_reason, SpaceEndReason::SPACE_END_REASON_HOST_LEFT);
    assert!(ctx.events(&mut listener_events).await.iter().any(|event| matches!(
        event,
        Push::ParticipantLeft(left) if left.reason == SpaceLeaveReason::SPACE_LEAVE_REASON_ENDED
    )));
    let event = ctx.app.events_by("spaces.room.ended", host.id).await.pop().unwrap();
    assert_eq!(event.data["reason"].as_i64(), Some(2));

    // A room nobody is in ends even sooner.
    let lonely = ctx.public_room(&host, "だれもいない").await;
    ctx.leave(&host, &lonely.id).await;
    assert_eq!(ctx.spaces.settle_rooms().await.unwrap(), 0);
    sql(&ctx, "UPDATE spaces SET empty_since = now() - interval '121 seconds' WHERE id = $1::uuid", &lonely.id).await;
    assert_eq!(ctx.spaces.settle_rooms().await.unwrap(), 1);
    let ended = ctx.get(&host, &lonely.id).await;
    assert_eq!(
        (ended.state, ended.end_reason),
        (SpaceState::SPACE_STATE_ENDED.into(), SpaceEndReason::SPACE_END_REASON_EMPTY.into())
    );
}

#[tokio::test]
async fn upcoming_rooms_nudge_their_host_and_expire() {
    let _sweeps = SWEEPS.lock().await;
    let ctx = start().await;
    let host = ctx.app.user("taro").await;
    let room = ctx.schedule(&host, "忘れてない?", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;
    ctx.spaces.nudge_hosts().await.unwrap();
    assert!(ctx.notifications(&host).await.is_empty(), "its time has not come");

    // The planned time arrives: the host is told, once.
    sql(&ctx, "UPDATE spaces SET scheduled_at = now() - interval '1 minute' WHERE id = $1::uuid", &room.id).await;
    ctx.spaces.nudge_hosts().await.unwrap();
    ctx.spaces.nudge_hosts().await.unwrap();
    let nudges = ctx.notifications(&host).await;
    assert_eq!(nudges.len(), 1);
    assert_eq!(nudges[0].kind, "spaces.reminder");
    assert_eq!(nudges[0].title.ja, "ルームの時間だよ!");
    assert_eq!(nudges[0].actor_id, None);
    // Rescheduling arms the nudge again.
    let update = UpdateSpaceRequest {
        space_id: room.id.clone(),
        scheduled_at: tokuly_core::time::ts(chrono::Utc::now() + chrono::Duration::hours(2)),
        ..Default::default()
    };
    let _: UpdateSpaceResponse = ctx.call("Space", "UpdateSpace", &host, &update).await.unwrap();
    sql(&ctx, "UPDATE spaces SET scheduled_at = now() - interval '1 minute' WHERE id = $1::uuid", &room.id).await;
    ctx.spaces.nudge_hosts().await.unwrap();
    assert_eq!(ctx.notifications(&host).await.len(), 2);

    // A day later nobody has started it: it is called off.
    assert_eq!(ctx.spaces.cancel_rooms_nobody_started().await.unwrap(), 0);
    sql(&ctx, "UPDATE spaces SET scheduled_at = now() - interval '25 hours' WHERE id = $1::uuid", &room.id).await;
    let mut room_events = ctx.listen_room(&room.id).await;
    assert_eq!(ctx.spaces.cancel_rooms_nobody_started().await.unwrap(), 1);
    let cancelled = ctx.get(&host, &room.id).await;
    assert_eq!(cancelled.state, SpaceState::SPACE_STATE_CANCELLED);
    assert_eq!(cancelled.end_reason, SpaceEndReason::SPACE_END_REASON_NEVER_STARTED);
    assert!(matches!(ctx.events(&mut room_events).await.as_slice(), [Push::SpaceUpdated(_)]));
    assert!(ctx.search_changes(&room.id).await.pop().unwrap().doc.is_none());
}

#[tokio::test]
async fn the_sweep_closes_what_was_left_open_and_forgets_old_rooms() {
    let _sweeps = SWEEPS.lock().await;
    let ctx = start().await;
    let host = ctx.app.user("taro").await;
    let (room, credentials) = ctx.open_room(&host, "おわった話", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;
    end_request(&ctx, &host, &room.id).await;
    let closed_at = || async {
        sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
            "SELECT room_closed_at FROM spaces WHERE id = $1::uuid",
        )
        .bind(&room.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap()
    };
    assert!(closed_at().await.is_some(), "ending the room closed its LiveKit room");

    // As if LiveKit had been unreachable at that moment, and someone reconnected since.
    sql(
        &ctx,
        "UPDATE spaces SET room_closed_at = NULL, ended_at = now() - interval '1 minute' WHERE id = $1::uuid",
        &room.id,
    )
    .await;
    // The room LiveKit kept (it does not make rooms on its own).
    ctx.livekit().create_room(&RoomOptions::named(&credentials.room)).await.unwrap();
    let _lingering = ctx.connect(&credentials).await;
    ctx.spaces.sweep().await.unwrap();
    assert!(closed_at().await.is_some());
    assert!(ctx.livekit().list_rooms(std::slice::from_ref(&credentials.room)).await.unwrap().is_empty());

    // History is kept for ninety days.
    sql(&ctx, "UPDATE spaces SET ended_at = now() - interval '89 days' WHERE id = $1::uuid", &room.id).await;
    ctx.spaces.sweep().await.unwrap();
    assert_eq!(ctx.get(&host, &room.id).await.title, "おわった話");
    // Deleting it hides it, but does not make it go sooner: for thirty days
    // from the deletion it stays on record, however old the room is.
    let _: DeleteSpaceResponse = ctx
        .call("Space", "DeleteSpace", &host, &DeleteSpaceRequest { space_id: room.id.clone(), ..Default::default() })
        .await
        .unwrap();
    sql(&ctx, "UPDATE spaces SET ended_at = now() - interval '91 days' WHERE id = $1::uuid", &room.id).await;
    ctx.spaces.sweep().await.unwrap();
    let on_record = || async {
        sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM spaces WHERE id = $1::uuid)")
            .bind(&room.id)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap()
    };
    assert!(on_record().await, "a report filed just before the deletion can still be looked into");
    sql(&ctx, "UPDATE spaces SET deleted_at = now() - interval '31 days' WHERE id = $1::uuid", &room.id).await;
    ctx.spaces.sweep().await.unwrap();
    assert!(!on_record().await);

    // A room nobody deleted goes after its ninety days.
    let (old, _) = ctx.open_room(&host, "むかしの話", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;
    end_request(&ctx, &host, &old.id).await;
    sql(&ctx, "UPDATE spaces SET ended_at = now() - interval '91 days' WHERE id = $1::uuid", &old.id).await;
    ctx.spaces.sweep().await.unwrap();
    let request = GetSpaceRequest { space_id: old.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Space", "GetSpace", &host, &request).await, "SPACE_NOT_FOUND");
}

#[tokio::test]
async fn the_full_pass_covers_rooms_hangouts_and_what_just_ended() {
    let _sweeps = SWEEPS.lock().await;
    let ctx = start().await;
    let (host, friend) = ctx.friends("taro", "hanako").await;
    let listener = ctx.app.user("jiro").await;

    // A live room with a connection that has no seat.
    let (live, host_credentials) = ctx.open_room(&host, "つづいてる", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;
    let _host_connection = ctx.connect(&host_credentials).await;
    let left = credentials_of(&ctx.join(&listener, &live.id).await);
    ctx.leave(&listener, &live.id).await;
    let _stale = ctx.connect(&left).await;

    // A room that just ended, and someone who reconnects with the token they still hold.
    let other_host = ctx.app.user("saburo").await;
    let (ended, _) = ctx.open_room(&other_host, "おわったはず", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;
    let lingering = credentials_of(&ctx.join(&friend, &ended.id).await);
    end_request(&ctx, &other_host, &ended.id).await;
    // LiveKit does not make rooms on its own (`room.auto_create: false`), so
    // a reconnect into a closed room is refused: this is the room LiveKit
    // kept because closing it did not reach it.
    ctx.livekit().create_room(&RoomOptions::named(&lingering.room)).await.unwrap();
    let _lingerer = ctx.connect(&lingering).await;

    // A hangout someone left while their token is still good.
    let hangout = ctx.hangout(&host, "いつメン", &[&friend]).await;
    let in_hangout = ctx.join_hangout(&friend, &hangout.id).await.credentials.as_option().unwrap().clone();
    let leave = LeaveHangoutRequest { hangout_id: hangout.id.clone(), ..Default::default() };
    let _: LeaveHangoutResponse = ctx.call("Hangout", "LeaveHangout", &friend, &leave).await.unwrap();
    let _back = ctx.connect(&in_hangout).await;

    let changed = ctx.spaces.reconcile().await.unwrap();
    assert!(changed.strays >= 3, "{changed:?}");
    ctx.wait_until_connected(&left.room, &listener.id.to_string(), false).await;
    ctx.wait_until_connected(&lingering.room, &friend.id.to_string(), false).await;
    ctx.wait_until_connected(&in_hangout.room, &friend.id.to_string(), false).await;
    assert!(ctx.connected(&host_credentials.room, &host.id.to_string()).await.is_some());
    end_request(&ctx, &host, &live.id).await;
}
