//! Requests that arrive together. Everything here really runs
//! concurrently: where the domain relies on a lock, a unique index or an
//! order, two requests passing the same check at the same moment must not
//! both win — and a burst from one account must not hold up everyone else.
mod common;

use std::time::Duration;

use common::*;
use futures::future::join_all;
use tokuly_proto::spaces::{spaces_event::Event as Push, *};
use tokuly_spaces::{SpacesConfig, WEBHOOK_PATH};
use tokuly_testkit::{RpcError, TestUser};
use tower::ServiceExt as _;

/// More calls at once than the test's database pool has connections (5).
const BURST: usize = 8;
/// A burst that needs longer than this is waiting for a connection that a
/// lock holder cannot get (the pool gives up after 30 seconds).
const PROMPTLY: Duration = Duration::from_secs(20);

fn join_request(space_id: &str) -> JoinSpaceRequest {
    JoinSpaceRequest { space_id: space_id.to_owned(), ..Default::default() }
}

/// How many of the results failed with `reason`.
fn refused<T>(results: &[Result<T, RpcError>], reason: &str) -> usize {
    results.iter().filter(|result| result.as_ref().is_err_and(|error| error.reason == reason)).count()
}

fn succeeded<T>(results: &[Result<T, RpcError>]) -> usize {
    results.iter().filter(|result| result.is_ok()).count()
}

/// The burst as one future, failing the test if it does not finish promptly.
async fn promptly<T>(calls: impl Future<Output = Vec<Result<T, RpcError>>>) -> Vec<Result<T, RpcError>> {
    tokio::time::timeout(PROMPTLY, calls).await.expect("the burst starved the connection pool")
}

#[tokio::test]
async fn a_burst_from_one_account_does_not_stall_the_api() {
    let ctx = start().await;
    let (host, friend) = ctx.friends("taro", "hanako").await;
    let room = ctx.public_room(&host, "いっせいに").await;

    // The same person enters the same room many times at once: every call
    // takes the same locks.
    let enter_room = join_request(&room.id);
    let joins: Vec<Result<JoinSpaceResponse, _>> =
        promptly(join_all((0..BURST).map(|_| ctx.call("Space", "JoinSpace", &friend, &enter_room)))).await;
    assert_eq!(succeeded(&joins), BURST, "{:?}", joins.iter().filter_map(|r| r.as_ref().err()).collect::<Vec<_>>());
    assert_eq!(ctx.get(&host, &room.id).await.listener_count, 2, "counted once");
    assert_eq!(ctx.app.events_by("spaces.room.joined", friend.id).await.len(), 1);

    // The host bans someone who is not inside, many times at once (the ban
    // needs to know that the account exists).
    let outsider = ctx.app.user("jiro").await;
    let ban = RemoveParticipantRequest {
        space_id: room.id.clone(),
        user_id: outsider.id.to_string(),
        ban: true,
        ..Default::default()
    };
    let bans: Vec<Result<RemoveParticipantResponse, _>> =
        promptly(join_all((0..BURST).map(|_| ctx.call("Space", "RemoveParticipant", &host, &ban)))).await;
    assert_eq!(succeeded(&bans), BURST);
    let listed: ListBansResponse = ctx
        .call("Space", "ListBans", &host, &ListBansRequest { space_id: room.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(listed.bans.len(), 1);

    // Everyone inside is moved about at once by the host.
    let hand = SetHandRaisedRequest { space_id: room.id.clone(), raised: true, ..Default::default() };
    let hands: Vec<Result<SetHandRaisedResponse, _>> =
        promptly(join_all((0..BURST).map(|_| ctx.call("Space", "SetHandRaised", &friend, &hand)))).await;
    assert_eq!(succeeded(&hands), BURST);
    assert_eq!(ctx.get(&host, &room.id).await.hand_count, 1);

    // A reminder set many times at once counts once.
    let upcoming = ctx.schedule(&friend, "あとで", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;
    let remind = SetReminderRequest { space_id: upcoming.id.clone(), remind: true, ..Default::default() };
    let reminders: Vec<Result<SetReminderResponse, _>> =
        promptly(join_all((0..BURST).map(|_| ctx.call("Space", "SetReminder", &host, &remind)))).await;
    assert_eq!(succeeded(&reminders), BURST);
    assert_eq!(ctx.get(&host, &upcoming.id).await.reminder_count, 1);

    // The owner of a hangout invites the same friend many times at once
    // (the invitation needs the owner's name for the notification)…
    let created: CreateHangoutResponse = ctx
        .call(
            "Hangout",
            "CreateHangout",
            &host,
            &CreateHangoutRequest { name: "いつメン".into(), ..Default::default() },
        )
        .await
        .unwrap();
    let hangout_id = created.hangout.id.clone();
    let invite = InviteToHangoutRequest {
        hangout_id: hangout_id.clone(),
        user_ids: vec![friend.id.to_string()],
        ..Default::default()
    };
    let invitations: Vec<Result<InviteToHangoutResponse, _>> =
        promptly(join_all((0..BURST).map(|_| ctx.call("Hangout", "InviteToHangout", &host, &invite)))).await;
    assert_eq!(succeeded(&invitations), BURST);
    assert_eq!(invitations.iter().map(|result| result.as_ref().unwrap().invited_count).sum::<i32>(), 1, "invited once");
    assert_eq!(ctx.notifications(&friend).await.iter().filter(|n| n.data["hangout_id"] == hangout_id).count(), 1);
    // …the friend accepts twice at once…
    let accept = RespondToHangoutInviteRequest { hangout_id: hangout_id.clone(), accept: true, ..Default::default() };
    let answers: Vec<Result<RespondToHangoutInviteResponse, _>> =
        promptly(join_all((0..2).map(|_| ctx.call("Hangout", "RespondToHangoutInvite", &friend, &accept)))).await;
    assert_eq!((succeeded(&answers), refused(&answers, "NO_INVITATION")), (1, 1));
    // …and enters the voice room many times at once (which looks up their friends).
    let enter = JoinHangoutRequest { hangout_id: hangout_id.clone(), ..Default::default() };
    let entries: Vec<Result<JoinHangoutResponse, _>> =
        promptly(join_all((0..BURST).map(|_| ctx.call("Hangout", "JoinHangout", &friend, &enter)))).await;
    assert_eq!(succeeded(&entries), BURST);
    let hangout = ctx.get_hangout(&host, &hangout_id).await;
    assert_eq!((hangout.member_count, hangout.inside_count), (2, 1));
    assert_eq!(ctx.app.events_by("spaces.hangout.joined", friend.id).await.len(), 1, "entered once");
    assert_eq!(ctx.notifications(&host).await.iter().filter(|n| n.kind == "spaces.hangout_joined").count(), 1);
}

#[tokio::test]
async fn the_last_seat_goes_to_one_person() {
    let cfg = SpacesConfig { max_speakers: 2, max_cohosts: 1, max_listeners: 4, ..Default::default() };
    let ctx = start_with(cfg).await;
    let host = ctx.app.user("taro").await;
    let (first, second, third) = (ctx.app.user("a").await, ctx.app.user("b").await, ctx.app.user("c").await);
    let room = ctx.public_room(&host, "一席だけ").await;
    for user in [&first, &second] {
        ctx.join(user, &room.id).await;
    }

    // Two invitations, one seat: both accept at the same moment.
    for user in [&first, &second] {
        let invite =
            InviteToStageRequest { space_id: room.id.clone(), user_id: user.id.to_string(), ..Default::default() };
        let invited: InviteToStageResponse = ctx.call("Space", "InviteToStage", &host, &invite).await.unwrap();
        assert!(!invited.on_stage);
    }
    let accept = RespondToStageInviteRequest { space_id: room.id.clone(), accept: true, ..Default::default() };
    let (one, other): (Result<RespondToStageInviteResponse, _>, Result<RespondToStageInviteResponse, _>) = tokio::join!(
        ctx.call("Space", "RespondToStageInvite", &first, &accept),
        ctx.call("Space", "RespondToStageInvite", &second, &accept),
    );
    let answers = [one, other];
    assert_eq!((succeeded(&answers), refused(&answers, "STAGE_FULL")), (1, 1));
    let space = ctx.get(&host, &room.id).await;
    assert_eq!((space.speaker_count, space.stage.len()), (2, 2));

    // The one who made it steps down; the host names two co-hosts at once, and may have one.
    for user in [&first, &second] {
        let down = MoveToAudienceRequest { space_id: room.id.clone(), ..Default::default() };
        let _: MoveToAudienceResponse = ctx.call("Space", "MoveToAudience", user, &down).await.unwrap();
    }
    let cohost = |user: &TestUser| SetCohostRequest {
        space_id: room.id.clone(),
        user_id: user.id.to_string(),
        cohost: true,
        ..Default::default()
    };
    let (name_first, name_second) = (cohost(&first), cohost(&second));
    let (one, other): (Result<SetCohostResponse, _>, Result<SetCohostResponse, _>) = tokio::join!(
        ctx.call("Space", "SetCohost", &host, &name_first),
        ctx.call("Space", "SetCohost", &host, &name_second)
    );
    let named = [one, other];
    assert_eq!(succeeded(&named), 1, "{named:?}");
    assert_eq!(refused(&named, "TOO_MANY_COHOSTS") + refused(&named, "STAGE_FULL"), 1);
    assert_eq!(ctx.get(&host, &room.id).await.speaker_count, 2);

    // The room holds four: one place is left, and three people want it.
    let fourth = ctx.app.user("d").await;
    let fifth = ctx.app.user("e").await;
    let enter_room = join_request(&room.id);
    let arrivals: Vec<Result<JoinSpaceResponse, _>> =
        join_all([&third, &fourth, &fifth].map(|user| ctx.call("Space", "JoinSpace", user, &enter_room))).await;
    assert_eq!((succeeded(&arrivals), refused(&arrivals, "SPACE_FULL")), (1, 2));
    assert_eq!(ctx.get(&host, &room.id).await.listener_count, 4);
}

#[tokio::test]
async fn a_ban_racing_a_join_keeps_the_person_out() {
    let ctx = start().await;
    let host = ctx.app.user("taro").await;
    let room = ctx.public_room(&host, "出禁レース").await;
    for round in 0..6 {
        let target = ctx.app.user(&format!("racer{round}")).await;
        let ban = RemoveParticipantRequest {
            space_id: room.id.clone(),
            user_id: target.id.to_string(),
            ban: true,
            ..Default::default()
        };
        let enter_room = join_request(&room.id);
        let (joined, banned): (Result<JoinSpaceResponse, _>, Result<RemoveParticipantResponse, _>) = tokio::join!(
            ctx.call("Space", "JoinSpace", &target, &enter_room),
            ctx.call("Space", "RemoveParticipant", &host, &ban),
        );
        banned.expect("the ban goes through either way");
        if let Err(error) = &joined {
            assert!(matches!(error.reason.as_str(), "BANNED_FROM_SPACE" | "SPACE_NOT_FOUND"), "{error}");
        }
        // Whichever came first: they are not inside, and they stay out.
        assert_eq!(ctx.role_of(&host, &target, &room.id).await, None, "round {round}");
        assert_eq!(ctx.reason("Space", "JoinSpace", &target, &join_request(&room.id)).await, "SPACE_NOT_FOUND");
    }
    assert_eq!(ctx.get(&host, &room.id).await.listener_count, 1);
}

#[tokio::test]
async fn a_host_goes_live_once_and_schedules_within_the_limit() {
    let ctx = start_with(SpacesConfig { max_scheduled: 2, ..Default::default() }).await;
    let host = ctx.app.user("taro").await;

    let instant = CreateSpaceRequest { title: "どっちか".into(), ..Default::default() };
    let (one, other): (Result<CreateSpaceResponse, _>, Result<CreateSpaceResponse, _>) = tokio::join!(
        ctx.call("Space", "CreateSpace", &host, &instant),
        ctx.call("Space", "CreateSpace", &host, &instant)
    );
    let created = [one, other];
    assert_eq!((succeeded(&created), refused(&created, "ALREADY_HOSTING")), (1, 1), "{created:?}");
    let mine: ListMySpacesResponse =
        ctx.call("Space", "ListMySpaces", &host, &ListMySpacesRequest::default()).await.unwrap();
    assert_eq!(mine.spaces.len(), 1);

    let later = CreateSpaceRequest {
        title: "あとで".into(),
        scheduled_at: tokuly_core::time::ts(chrono::Utc::now() + chrono::Duration::hours(1)),
        ..Default::default()
    };
    let scheduled: Vec<Result<CreateSpaceResponse, _>> =
        promptly(join_all((0..BURST).map(|_| ctx.call("Space", "CreateSpace", &host, &later)))).await;
    assert_eq!((succeeded(&scheduled), refused(&scheduled, "TOO_MANY_SPACES")), (2, BURST - 2));
}

#[tokio::test]
async fn a_person_entering_two_rooms_at_once_ends_up_in_one() {
    let ctx = start().await;
    let (first_host, second_host) = (ctx.app.user("taro").await, ctx.app.user("jiro").await);
    let first = ctx.public_room(&first_host, "こっち").await;
    let second = ctx.public_room(&second_host, "あっち").await;
    for round in 0..5 {
        let wanderer = ctx.app.user(&format!("wanderer{round}")).await;
        let (enter_first, enter_second) = (join_request(&first.id), join_request(&second.id));
        let (one, other): (Result<JoinSpaceResponse, _>, Result<JoinSpaceResponse, _>) = tokio::join!(
            ctx.call("Space", "JoinSpace", &wanderer, &enter_first),
            ctx.call("Space", "JoinSpace", &wanderer, &enter_second),
        );
        for result in [&one, &other] {
            if let Err(error) = result {
                assert_eq!(error.reason, "ENTERING_ELSEWHERE", "the only way to lose this race");
            }
        }
        assert!(one.is_ok() || other.is_ok());
        let inside =
            [&first, &second].map(|room| async { ctx.role_of(&wanderer, &wanderer, &room.id).await.is_some() });
        let inside = join_all(inside).await.into_iter().filter(|inside| *inside).count();
        assert_eq!(inside, 1, "round {round}");
    }
    let total =
        ctx.get(&first_host, &first.id).await.listener_count + ctx.get(&second_host, &second.id).await.listener_count;
    assert_eq!(total, 2 + 5, "two hosts and five wanderers, each counted once");
}

#[tokio::test]
async fn a_hangout_opens_once_however_many_arrive() {
    let ctx = start_with(SpacesConfig { max_hangouts: 2, ..Default::default() }).await;
    let owner = ctx.app.user("taro").await;
    let members = [ctx.app.user("a").await, ctx.app.user("b").await, ctx.app.user("c").await];
    for member in &members {
        ctx.befriend(&owner, member).await;
    }
    let hangout = ctx.hangout(&owner, "いつメン", &members.iter().collect::<Vec<_>>()).await;

    // All four walk in at the same moment: one of them opened it.
    let enter = JoinHangoutRequest { hangout_id: hangout.id.clone(), ..Default::default() };
    let everyone: Vec<&TestUser> = members.iter().chain([&owner]).collect();
    let entries: Vec<Result<JoinHangoutResponse, _>> =
        promptly(join_all(everyone.iter().map(|user| ctx.call("Hangout", "JoinHangout", user, &enter)))).await;
    assert_eq!(succeeded(&entries), 4);
    assert_eq!(ctx.get_hangout(&owner, &hangout.id).await.inside_count, 4);
    let mut firsts = 0;
    for user in &everyone {
        firsts += ctx
            .app
            .events_by("spaces.hangout.joined", user.id)
            .await
            .iter()
            .filter(|event| event.data["hangout_id"] == hangout.id && event.data["first"] == true)
            .count();
    }
    assert_eq!(firsts, 1);

    // One person creating hangouts in a burst gets as many as they may own.
    let create = CreateHangoutRequest { name: "もうひとつ".into(), ..Default::default() };
    let created: Vec<Result<CreateHangoutResponse, _>> =
        promptly(join_all((0..BURST).map(|_| ctx.call("Hangout", "CreateHangout", &members[0], &create)))).await;
    assert_eq!((succeeded(&created), refused(&created, "TOO_MANY_HANGOUTS")), (2, BURST - 2));
}

async fn post_webhook(ctx: &Ctx, body: &str) -> u16 {
    let request = http::Request::builder()
        .method("POST")
        .uri(WEBHOOK_PATH)
        .header("authorization", ctx.livekit().sign_webhook(body.as_bytes()).unwrap())
        .body(axum::body::Body::from(body.to_owned()))
        .unwrap();
    ctx.spaces.http_routes().oneshot(request).await.expect("the router answers").status().as_u16()
}

fn webhook(event: &str, room: &str, identity: &str) -> String {
    serde_json::json!({ "event": event, "room": { "name": room }, "participant": { "identity": identity }, "id": uuid::Uuid::now_v7() })
        .to_string()
}

#[tokio::test]
async fn webhooks_twice_and_out_of_order_change_nothing_they_should_not() {
    // No patience: whoever is found missing is taken out at once, so that a
    // webhook that lies would show.
    let cfg = SpacesConfig { connect_grace_seconds: 0, reconnect_grace_seconds: 0, ..Default::default() };
    let ctx = start_with(cfg).await;
    let host = ctx.app.user("taro").await;
    let listener = ctx.app.user("hanako").await;
    let (room, host_credentials) = ctx.open_room(&host, "うわさ", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;
    let _host_connection = ctx.connect(&host_credentials).await;
    let credentials = ctx.join(&listener, &room.id).await.credentials.as_option().unwrap().clone();
    let connection = ctx.connect(&credentials).await;
    let identity = listener.id.to_string();

    // LiveKit says the listener left — late news from a connection they
    // replaced long ago — several times, together with the news of their
    // arrival. What counts is who is connected now.
    let left = webhook("participant_left", &credentials.room, &identity);
    let joined = webhook("participant_joined", &credentials.room, &identity);
    let statuses = join_all([&left, &left, &joined, &left, &joined].map(|body| post_webhook(&ctx, body))).await;
    assert_eq!(statuses, [200; 5]);
    // (The comparison the webhooks asked for runs a moment later.)
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(ctx.get(&host, &room.id).await.listener_count, 2, "still inside");

    // Now they really are gone. The same news arrives twice, and an old
    // "joined" after it: they leave the room once.
    connection.drop_dead();
    ctx.wait_until_connected(&credentials.room, &identity, false).await;
    let mut listener_events = ctx.listen(&listener).await;
    let statuses = join_all([&left, &left, &joined].map(|body| post_webhook(&ctx, body))).await;
    assert_eq!(statuses, [200; 3]);
    let frames = listener_events.drain(Duration::from_millis(2500)).await;
    assert_eq!(ctx.get(&host, &room.id).await.listener_count, 1);
    let told = pushes(frames)
        .iter()
        .filter(|event| matches!(event, Push::ParticipantLeft(left) if left.reason == SpaceLeaveReason::SPACE_LEAVE_REASON_DISCONNECTED))
        .count();
    assert_eq!(told, 1, "taken out once");
    assert!(ctx.connected(&credentials.room, &host.id.to_string()).await.is_some(), "the host is untouched");
}
