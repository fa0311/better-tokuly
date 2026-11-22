//! `StudyRoomService`: opening, entering and leaving rooms, the shared
//! timer, audiences and invitations, reactions, and closing.
mod common;

use common::*;
use tokuly_proto::school::{school_event::Event as Push, *};
use tokuly_testkit::TestUser;

fn room_of(response: &CreateStudyRoomResponse) -> StudyRoom {
    response.room.as_option().cloned().expect("the room")
}

fn member_names(room: &StudyRoom) -> Vec<String> {
    room.members.iter().map(|member| member.user.as_option().unwrap().display_name.clone()).collect()
}

async fn leave(ctx: &Ctx, user: &TestUser) -> StudySession {
    let response: LeaveStudyRoomResponse =
        ctx.ok(ROOMS, "LeaveStudyRoom", user, &LeaveStudyRoomRequest::default()).await;
    response.session.as_option().cloned().expect("the session")
}

async fn list(ctx: &Ctx, viewer: &TestUser, scope: RoomScope) -> Vec<String> {
    let response: ListStudyRoomsResponse = ctx
        .ok(ROOMS, "ListStudyRooms", viewer, &ListStudyRoomsRequest { scope: scope.into(), ..Default::default() })
        .await;
    response.rooms.iter().map(|room| room.id.clone()).collect()
}

const CLASSIC: PomodoroConfig = PomodoroConfig {
    work_minutes: 25,
    break_minutes: 5,
    long_break_minutes: 15,
    rounds: 4,
    __buffa_unknown_fields: buffa::UnknownFields::new(),
};

#[tokio::test]
async fn a_room_is_opened_entered_and_left() {
    let ctx = start().await;
    let (host, friend, stranger) =
        (ctx.app.user("host").await, ctx.app.user("friend").await, ctx.app.user("stranger").await);
    ctx.befriend(&host, &friend).await;
    ctx.at(2026, 4, 13, 18, 0);

    let created = ctx.create_room(&host, |room| room.title = " テスト前もくもく ".into()).await;
    let room = room_of(&created);
    assert_eq!(
        (room.title.as_str(), room.host.as_option().unwrap().id.as_str()),
        ("テスト前もくもく", host.id.to_string().as_str())
    );
    assert_eq!(room.audience, RoomAudience::ROOM_AUDIENCE_FRIENDS, "friends of the host by default");
    assert_eq!((room.member_count, room.member_cap, room.joined, room.host_present), (1, 20, true, true));
    assert_eq!(room.topic, format!("study:{}", room.id));
    assert_eq!((member_names(&room), room.members[0].host), (vec!["host".to_owned()], true));
    let timer = room.timer.as_option().unwrap();
    assert_eq!(
        (timer.mode.as_known().unwrap(), timer.state.as_known().unwrap(), timer.round),
        (StudyMode::STUDY_MODE_POMODORO, TimerState::TIMER_STATE_IDLE, 1),
        "the timer waits for the host"
    );
    let session = created.session.as_option().unwrap();
    assert_eq!(
        (session.source.as_known().unwrap(), session.room_id.as_str(), created.heartbeat_interval_seconds),
        (SessionSource::SESSION_SOURCE_ROOM, room.id.as_str(), 60)
    );
    let opened = ctx.app.events_by("school.room.opened", host.id).await;
    assert_eq!((opened.len(), opened[0].data["room_id"].as_str()), (1, Some(room.id.as_str())));

    let create = |fill: fn(&mut CreateStudyRoomRequest)| {
        let mut request = CreateStudyRoomRequest { title: "x".into(), ..Default::default() };
        fill(&mut request);
        request
    };
    assert_eq!(ctx.reason(ROOMS, "CreateStudyRoom", &host, &create(|_| {})).await, "ALREADY_HOSTING");
    for (request, reason) in [
        (create(|r| r.title = " ".into()), "ROOM_TITLE_INVALID"),
        (create(|r| r.member_cap = 1), "MEMBER_CAP_INVALID"),
        (create(|r| r.member_cap = 51), "MEMBER_CAP_INVALID"),
        (
            create(|r| r.pomodoro = PomodoroConfig { work_minutes: 999, ..Default::default() }.into()),
            "POMODORO_INVALID",
        ),
        (create(|r| r.subject_id = uuid::Uuid::now_v7().to_string()), "SUBJECT_NOT_FOUND"),
        (CreateStudyRoomRequest { invite_user_ids: vec![host.id.to_string()], ..create(|_| {}) }, "NOT_FRIENDS"),
    ] {
        assert_eq!(ctx.reason(ROOMS, "CreateStudyRoom", &stranger, &request).await, reason, "{request:?}");
    }

    // A friend of the host may look and enter; to a stranger the room does not exist.
    assert!(!ctx.room(&friend, &room.id).await.unwrap().joined);
    assert_eq!(ctx.room(&stranger, &room.id).await.unwrap_err().reason, "ROOM_NOT_FOUND");
    assert_eq!(ctx.join(&stranger, &room.id).await.unwrap_err().reason, "ROOM_NOT_FOUND");
    assert_eq!(ctx.join(&friend, &uuid::Uuid::now_v7().to_string()).await.unwrap_err().reason, "ROOM_NOT_FOUND");

    // The friend was studying on their own: that session ends and is logged.
    ctx.start_stopwatch(&friend).await;
    ctx.pass(10);
    ctx.heartbeat(&friend).await;
    let mut topic = ctx.app.listen_topic(&room.topic).await;
    let joined = ctx.join(&friend, &room.id).await.unwrap();
    let seen = joined.room.as_option().cloned().unwrap();
    assert_eq!(
        (seen.member_count, seen.joined, member_names(&seen)),
        (2, true, vec!["host".to_owned(), "friend".to_owned()])
    );
    assert_eq!(joined.session.as_option().unwrap().room_id, room.id);
    assert_eq!(ctx.sessions(&friend).await.iter().map(|s| s.studied_seconds).collect::<Vec<_>>(), vec![600]);
    let heard_in_room = heard(&mut topic).await;
    assert_eq!(kinds(&heard_in_room), vec!["room_member_joined"]);
    let Push::RoomMemberJoined(member) = &heard_in_room[0] else { unreachable!() };
    assert_eq!((member.user.as_option().unwrap().display_name.as_str(), member.host), ("friend", false));
    // Entering twice is entering once.
    let again = ctx.join(&friend, &room.id).await.unwrap();
    assert_eq!(again.session.as_option().unwrap().id, joined.session.as_option().unwrap().id);
    assert_eq!(again.room.as_option().unwrap().member_count, 2);

    // Leaving ends the stay. The timer never ran, so there is nothing to log.
    let left = leave(&ctx, &friend).await;
    assert_eq!((left.state.as_known().unwrap(), left.studied_seconds), (SessionState::SESSION_STATE_ENDED, 0));
    let heard_in_room = heard(&mut topic).await;
    assert_eq!(kinds(&heard_in_room), vec!["room_member_left"]);
    let Push::RoomMemberLeft(gone) = &heard_in_room[0] else { unreachable!() };
    assert_eq!(
        (gone.user_id.as_str(), gone.reason.as_known().unwrap(), gone.member_count),
        (friend.id.to_string().as_str(), RoomLeaveReason::ROOM_LEAVE_REASON_LEFT, 1)
    );
    assert_eq!(ctx.sessions(&friend).await.len(), 1);
    assert_eq!(ctx.reason(ROOMS, "LeaveStudyRoom", &friend, &LeaveStudyRoomRequest::default()).await, "NOT_IN_ROOM");
    assert_eq!(ctx.reason(ROOMS, "LeaveStudyRoom", &stranger, &LeaveStudyRoomRequest::default()).await, "NOT_IN_ROOM");
    assert_eq!(ctx.room(&host, &room.id).await.unwrap().member_count, 1);
}

#[tokio::test]
async fn the_shared_timer_counts_for_everyone_inside() {
    let ctx = start().await;
    let (host, friend, stranger) =
        (ctx.app.user("host").await, ctx.app.user("friend").await, ctx.app.user("stranger").await);
    ctx.befriend(&host, &friend).await;
    ctx.at(2026, 4, 13, 18, 0);
    let room = room_of(&ctx.create_room(&host, |room| room.pomodoro = CLASSIC.into()).await);
    ctx.join(&friend, &room.id).await.unwrap();
    let mut topic = ctx.app.listen_topic(&room.topic).await;
    let both = [&host, &friend];

    // Only the host controls the timer.
    assert_eq!(ctx.control(&friend, &room.id, TimerAction::TIMER_ACTION_START).await.unwrap_err().reason, "NOT_HOST");
    assert_eq!(
        ctx.control(&stranger, &room.id, TimerAction::TIMER_ACTION_START).await.unwrap_err().reason,
        "ROOM_NOT_FOUND"
    );
    assert_eq!(
        ctx.control(&host, &room.id, TimerAction::TIMER_ACTION_UNSPECIFIED).await.unwrap_err().reason,
        "TIMER_ACTION_REQUIRED"
    );
    for action in [TimerAction::TIMER_ACTION_PAUSE, TimerAction::TIMER_ACTION_RESUME, TimerAction::TIMER_ACTION_SKIP] {
        assert_eq!(
            ctx.control(&host, &room.id, action).await.unwrap_err().reason,
            "TIMER_ACTION_INVALID",
            "{action:?} before the start"
        );
    }

    let running = ctx.control(&host, &room.id, TimerAction::TIMER_ACTION_START).await.unwrap();
    assert_eq!(
        (running.state.as_known().unwrap(), running.phase.as_known().unwrap(), running.round),
        (TimerState::TIMER_STATE_RUNNING, TimerPhase::TIMER_PHASE_WORK, 1)
    );
    assert_eq!(
        (instant(&running.phase_ends_at), instant(&running.server_time)),
        (tokyo(2026, 4, 13, 18, 25), tokyo(2026, 4, 13, 18, 0))
    );
    let heard_in_room = heard(&mut topic).await;
    assert_eq!(kinds(&heard_in_room), vec!["room_timer"]);
    let Push::RoomTimer(pushed) = &heard_in_room[0] else { unreachable!() };
    assert_eq!(**pushed, running, "everyone gets the snapshot the host got, server time included");
    assert_eq!(
        ctx.control(&host, &room.id, TimerAction::TIMER_ACTION_START).await.unwrap_err().reason,
        "TIMER_ACTION_INVALID"
    );

    // Ten minutes of work count for both.
    ctx.pass(10);
    assert_eq!(ctx.heartbeat(&friend).await.studied_seconds, 600);
    assert_eq!(ctx.heartbeat(&host).await.studied_seconds, 600);
    // A pause counts for nobody.
    let paused = ctx.control(&host, &room.id, TimerAction::TIMER_ACTION_PAUSE).await.unwrap();
    assert_eq!(
        (paused.state.as_known().unwrap(), paused.phase_remaining_seconds),
        (TimerState::TIMER_STATE_PAUSED, 900)
    );
    ctx.pass(5);
    ctx.beat(&both).await;
    assert_eq!(ctx.heartbeat(&friend).await.studied_seconds, 600);
    let resumed = ctx.control(&host, &room.id, TimerAction::TIMER_ACTION_RESUME).await.unwrap();
    assert_eq!(instant(&resumed.phase_ends_at), tokyo(2026, 4, 13, 18, 30), "the phase ends five minutes later");

    // The remaining fifteen minutes of the round, then two minutes of break.
    ctx.pass(10);
    ctx.beat(&both).await;
    ctx.pass(7);
    ctx.beat(&both).await;
    let session = ctx.heartbeat(&friend).await;
    assert_eq!(session.studied_seconds, 1500, "the break does not count");
    assert_eq!(
        session.timer.as_option().unwrap().phase,
        TimerPhase::TIMER_PHASE_BREAK,
        "a room session shows the room's timer"
    );
    let seen = ctx.room(&friend, &room.id).await.unwrap();
    assert_eq!(seen.members.iter().map(|m| m.studied_seconds).collect::<Vec<_>>(), vec![1500, 1500]);
    assert_eq!(seen.timer.as_option().unwrap().phase, TimerPhase::TIMER_PHASE_BREAK);

    // Skipping the break starts round two; resetting goes back to the start.
    let skipped = ctx.control(&host, &room.id, TimerAction::TIMER_ACTION_SKIP).await.unwrap();
    assert_eq!((skipped.phase.as_known().unwrap(), skipped.round), (TimerPhase::TIMER_PHASE_WORK, 2));
    ctx.pass(3);
    ctx.beat(&both).await;
    let reset = ctx.control(&host, &room.id, TimerAction::TIMER_ACTION_RESET).await.unwrap();
    assert_eq!(
        (reset.state.as_known().unwrap(), reset.round, reset.elapsed_seconds),
        (TimerState::TIMER_STATE_IDLE, 1, 0)
    );
    ctx.pass(5);
    ctx.beat(&both).await;

    // What the room's timer counted is the member's study session.
    let ended = ctx.stop(&friend).await.session.as_option().cloned().unwrap();
    assert_eq!(
        (ended.studied_seconds, ended.source.as_known().unwrap(), ended.room_id.as_str()),
        (1680, SessionSource::SESSION_SOURCE_ROOM, room.id.as_str())
    );
    let events = ctx.app.events_by("school.study.session_ended", friend.id).await;
    assert_eq!(
        (
            events.len(),
            events[0].data["minutes"].as_i64(),
            events[0].data["source"].as_i64(),
            events[0].data["room_id"].as_str()
        ),
        (1, Some(28), Some(2), Some(room.id.as_str()))
    );

    // In a room the host's timer is the only one: members cannot pause or skip.
    for method in ["PauseStudySession", "ResumeStudySession", "SkipStudyPhase"] {
        assert_eq!(ctx.reason(STUDY, method, &host, &serde_json::json!({})).await, "ROOM_SESSION", "{method}");
    }
}

#[tokio::test]
async fn an_open_ended_room_has_no_phases() {
    let ctx = start().await;
    let host = ctx.app.user("host").await;
    ctx.at(2026, 4, 13, 18, 0);
    let room = room_of(&ctx.create_room(&host, |room| room.mode = StudyMode::STUDY_MODE_STOPWATCH.into()).await);
    let timer = room.timer.as_option().unwrap();
    assert_eq!(
        (timer.mode.as_known().unwrap(), timer.pomodoro.as_option().is_none()),
        (StudyMode::STUDY_MODE_STOPWATCH, true)
    );
    ctx.control(&host, &room.id, TimerAction::TIMER_ACTION_START).await.unwrap();
    assert_eq!(
        ctx.control(&host, &room.id, TimerAction::TIMER_ACTION_SKIP).await.unwrap_err().reason,
        "TIMER_ACTION_INVALID"
    );
    for _ in 0..4 {
        ctx.pass(10);
        ctx.heartbeat(&host).await;
    }
    assert_eq!(ctx.stop(&host).await.session.as_option().unwrap().studied_seconds, 2400, "everything counts");
}

#[tokio::test]
async fn a_room_keeps_going_without_its_host() {
    let ctx = start().await;
    let (host, guest, late) = (ctx.app.user("host").await, ctx.app.user("guest").await, ctx.app.user("late").await);
    ctx.at(2026, 4, 13, 18, 0);
    let room = room_of(&ctx.create_room(&host, |room| room.audience = RoomAudience::ROOM_AUDIENCE_PUBLIC.into()).await);
    ctx.join(&guest, &room.id).await.unwrap();
    let mut topic = ctx.app.listen_topic(&room.topic).await;

    // The host walks out before starting the timer: it starts by itself.
    leave(&ctx, &host).await;
    let heard_in_room = heard(&mut topic).await;
    assert_eq!(kinds(&heard_in_room), vec!["room_member_left", "room_timer"]);
    let Push::RoomTimer(timer) = &heard_in_room[1] else { unreachable!() };
    assert_eq!(timer.state, TimerState::TIMER_STATE_RUNNING);
    let seen = ctx.room(&guest, &room.id).await.unwrap();
    assert_eq!((seen.host_present, seen.member_count), (false, 1));
    ctx.pass(10);
    assert_eq!(ctx.heartbeat(&guest).await.studied_seconds, 600);
    // The host cannot open a second room while people are in the first.
    assert_eq!(
        ctx.reason(
            ROOMS,
            "CreateStudyRoom",
            &host,
            &CreateStudyRoomRequest { title: "二部屋目".into(), ..Default::default() }
        )
        .await,
        "ALREADY_HOSTING"
    );

    // The last one out: the timer pauses and the room drops out of the listing.
    assert!(list(&ctx, &late, RoomScope::ROOM_SCOPE_PUBLIC).await.contains(&room.id));
    leave(&ctx, &guest).await;
    let empty = ctx.room(&late, &room.id).await.unwrap();
    assert_eq!(
        (empty.member_count, empty.timer.as_option().unwrap().state.as_known().unwrap()),
        (0, TimerState::TIMER_STATE_PAUSED)
    );
    assert!(!list(&ctx, &late, RoomScope::ROOM_SCOPE_PUBLIC).await.contains(&room.id));

    // Someone coming into the empty room, host still away: it runs again.
    ctx.pass(3);
    let joined = ctx.join(&late, &room.id).await.unwrap();
    assert_eq!(joined.room.as_option().unwrap().timer.as_option().unwrap().state, TimerState::TIMER_STATE_RUNNING);
    ctx.pass(5);
    assert_eq!(ctx.heartbeat(&late).await.studied_seconds, 300);
    // The host coming back changes nothing about the timer.
    let back = ctx.join(&host, &room.id).await.unwrap();
    let seen = back.room.as_option().unwrap();
    assert_eq!(
        (seen.host_present, seen.member_count, seen.timer.as_option().unwrap().state.as_known().unwrap()),
        (true, 2, TimerState::TIMER_STATE_RUNNING)
    );

    // A host whose room stands empty may open a new one; the old one closes.
    leave(&ctx, &host).await;
    leave(&ctx, &late).await;
    let second = room_of(&ctx.create_room(&host, |_| {}).await);
    assert_ne!(second.id, room.id);
    assert_eq!(ctx.room(&guest, &room.id).await.unwrap_err().reason, "ROOM_NOT_FOUND");
    assert!(
        ctx.room(&host, &room.id).await.unwrap().closed_at.as_option().is_some(),
        "the host still sees their closed room"
    );
}

#[tokio::test]
async fn invitations_let_people_in_and_removal_keeps_them_out() {
    let ctx = start().await;
    let (host, friend, shy, stranger) = (
        ctx.app.user("host").await,
        ctx.app.user("friend").await,
        ctx.app.user("shy").await,
        ctx.app.user("stranger").await,
    );
    ctx.befriend(&host, &friend).await;
    ctx.befriend(&host, &shy).await;
    ctx.at(2026, 4, 13, 18, 0);
    let room =
        room_of(&ctx.create_room(&host, |room| room.audience = RoomAudience::ROOM_AUDIENCE_INVITED.into()).await);
    let invite = |users: &[&TestUser]| InviteToStudyRoomRequest {
        room_id: room.id.clone(),
        user_ids: users.iter().map(|u| u.id.to_string()).collect(),
        ..Default::default()
    };

    // Invitation only: being a friend is not enough.
    assert_eq!(ctx.room(&friend, &room.id).await.unwrap_err().reason, "ROOM_NOT_FOUND");
    assert!(!list(&ctx, &friend, RoomScope::ROOM_SCOPE_FRIENDS).await.contains(&room.id));

    let mut friend_rt = ctx.app.listen_user(friend.id).await;
    let invited: InviteToStudyRoomResponse = ctx.ok(ROOMS, "InviteToStudyRoom", &host, &invite(&[&friend, &shy])).await;
    assert_eq!(invited.invitees.len(), 2);
    let heard_by_friend = heard(&mut friend_rt).await;
    assert_eq!(kinds(&heard_by_friend), vec!["room_invitation"]);
    let Push::RoomInvitation(shown) = &heard_by_friend[0] else { unreachable!() };
    assert_eq!((shown.id.as_str(), shown.invited, shown.joined), (room.id.as_str(), true, false));
    let notified = ctx.notifications_for(friend.id, "school.room_invite").await;
    assert_eq!(
        (notified.len(), notified[0].actor_id, notified[0].data["path"].as_str()),
        (1, Some(host.id), Some(format!("/school/rooms/{}", room.id).as_str()))
    );
    // Inviting again notifies nobody twice.
    let _: InviteToStudyRoomResponse = ctx.ok(ROOMS, "InviteToStudyRoom", &host, &invite(&[&friend])).await;
    assert_eq!(ctx.notifications_for(friend.id, "school.room_invite").await.len(), 1);

    let seen = ctx.room(&friend, &room.id).await.unwrap();
    assert!(seen.invited && seen.invitees.is_empty(), "who else is invited is the host's business");
    assert_eq!(ctx.room(&host, &room.id).await.unwrap().invitees.len(), 2);
    assert!(list(&ctx, &friend, RoomScope::ROOM_SCOPE_FRIENDS).await.contains(&room.id));
    ctx.join(&friend, &room.id).await.unwrap();

    for (user, request, reason) in [
        (&host, invite(&[&stranger]), "NOT_FRIENDS"),
        (&host, invite(&[]), "NO_USERS"),
        (&friend, invite(&[&shy]), "NOT_HOST"),
        (&stranger, invite(&[&shy]), "ROOM_NOT_FOUND"),
    ] {
        assert_eq!(ctx.reason(ROOMS, "InviteToStudyRoom", user, &request).await, reason);
    }

    // An invitation can be withdrawn.
    let uninvite = |user: &TestUser| UninviteFromStudyRoomRequest {
        room_id: room.id.clone(),
        user_id: user.id.to_string(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(ROOMS, "UninviteFromStudyRoom", &friend, &uninvite(&shy)).await, "NOT_HOST");
    let _: UninviteFromStudyRoomResponse = ctx.ok(ROOMS, "UninviteFromStudyRoom", &host, &uninvite(&shy)).await;
    assert_eq!(ctx.join(&shy, &room.id).await.unwrap_err().reason, "ROOM_NOT_FOUND");

    // The host removes the friend: out for good, and told so.
    let remove = |user: &TestUser| RemoveStudyRoomMemberRequest {
        room_id: room.id.clone(),
        user_id: user.id.to_string(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(ROOMS, "RemoveStudyRoomMember", &friend, &remove(&host)).await, "NOT_HOST");
    assert_eq!(ctx.reason(ROOMS, "RemoveStudyRoomMember", &host, &remove(&host)).await, "CANNOT_REMOVE_SELF");
    assert_eq!(ctx.reason(ROOMS, "RemoveStudyRoomMember", &host, &remove(&shy)).await, "USER_NOT_IN_ROOM");
    let mut topic = ctx.app.listen_topic(&room.topic).await;
    let mut friend_rt = ctx.app.listen_user(friend.id).await;
    let _: RemoveStudyRoomMemberResponse = ctx.ok(ROOMS, "RemoveStudyRoomMember", &host, &remove(&friend)).await;
    let heard_in_room = heard(&mut topic).await;
    let Some(Push::RoomMemberLeft(gone)) = heard_in_room.first() else {
        panic!("the room hears of the removal: {:?}", kinds(&heard_in_room))
    };
    assert_eq!(gone.reason, RoomLeaveReason::ROOM_LEAVE_REASON_REMOVED);
    let heard_by_friend = heard(&mut friend_rt).await;
    assert!(kinds(&heard_by_friend).contains(&"room_left"), "{:?}", kinds(&heard_by_friend));
    assert_eq!(ctx.join(&friend, &room.id).await.unwrap_err().reason, "ROOM_NOT_FOUND");
    assert_eq!(ctx.room(&friend, &room.id).await.unwrap_err().reason, "ROOM_NOT_FOUND");
    assert_eq!(ctx.reason(ROOMS, "LeaveStudyRoom", &friend, &LeaveStudyRoomRequest::default()).await, "NOT_IN_ROOM");
}

#[tokio::test]
async fn a_full_room_and_a_block_keep_people_out() {
    let ctx = start().await;
    let (host, first, second, blocked) = (
        ctx.app.user("host").await,
        ctx.app.user("first").await,
        ctx.app.user("second").await,
        ctx.app.user("blocked").await,
    );
    ctx.at(2026, 4, 13, 18, 0);
    let room = room_of(
        &ctx.create_room(&host, |room| {
            room.audience = RoomAudience::ROOM_AUDIENCE_PUBLIC.into();
            room.member_cap = 2;
        })
        .await,
    );
    ctx.join(&first, &room.id).await.unwrap();
    assert_eq!(ctx.join(&second, &room.id).await.unwrap_err().reason, "ROOM_FULL");
    leave(&ctx, &first).await;

    // Blocked by the host: the public room is gone for them, listing included.
    assert!(list(&ctx, &blocked, RoomScope::ROOM_SCOPE_PUBLIC).await.contains(&room.id));
    ctx.block(&host, &blocked).await;
    assert_eq!(ctx.room(&blocked, &room.id).await.unwrap_err().reason, "ROOM_NOT_FOUND");
    assert_eq!(ctx.join(&blocked, &room.id).await.unwrap_err().reason, "ROOM_NOT_FOUND");
    assert!(!list(&ctx, &blocked, RoomScope::ROOM_SCOPE_PUBLIC).await.contains(&room.id));

    // Blocked by someone inside: the same, as long as they are in there.
    ctx.join(&second, &room.id).await.unwrap();
    let other = ctx.app.user("other").await;
    ctx.block(&second, &other).await;
    assert_eq!(ctx.join(&other, &room.id).await.unwrap_err().reason, "ROOM_NOT_FOUND");
    assert!(!list(&ctx, &other, RoomScope::ROOM_SCOPE_PUBLIC).await.contains(&room.id));
    leave(&ctx, &second).await;
    assert!(ctx.join(&other, &room.id).await.is_ok());
}

#[tokio::test]
async fn rooms_are_listed_by_scope() {
    let ctx = start().await;
    let (me, pal, inviter, stranger) = (
        ctx.app.user("me").await,
        ctx.app.user("pal").await,
        ctx.app.user("inviter").await,
        ctx.app.user("stranger").await,
    );
    ctx.befriend(&me, &pal).await;
    ctx.befriend(&me, &inviter).await;
    ctx.at(2026, 4, 13, 18, 0);
    let public =
        room_of(&ctx.create_room(&stranger, |room| room.audience = RoomAudience::ROOM_AUDIENCE_PUBLIC.into()).await);
    let of_pal = room_of(&ctx.create_room(&pal, |_| {}).await);
    let invited = room_of(
        &ctx.create_room(&inviter, |room| {
            room.audience = RoomAudience::ROOM_AUDIENCE_INVITED.into();
            room.invite_user_ids = vec![me.id.to_string()];
        })
        .await,
    );
    let secret = room_of(
        &ctx.create_room(&ctx.app.user("secretive").await, |room| {
            room.audience = RoomAudience::ROOM_AUDIENCE_INVITED.into()
        })
        .await,
    );

    // Other tests of this binary open rooms too: look for these four.
    let has = |listed: &[String], room: &StudyRoom| listed.contains(&room.id);
    let listed = list(&ctx, &me, RoomScope::ROOM_SCOPE_PUBLIC).await;
    assert_eq!(
        (has(&listed, &public), has(&listed, &of_pal), has(&listed, &invited), has(&listed, &secret)),
        (true, false, false, false)
    );
    let listed = list(&ctx, &me, RoomScope::ROOM_SCOPE_FRIENDS).await;
    assert_eq!(
        (has(&listed, &public), has(&listed, &of_pal), has(&listed, &invited), has(&listed, &secret)),
        (false, true, true, false)
    );
    assert!(list(&ctx, &me, RoomScope::ROOM_SCOPE_MINE).await.is_empty());
    assert_eq!(list(&ctx, &pal, RoomScope::ROOM_SCOPE_MINE).await, vec![of_pal.id.clone()]);
    ctx.join(&me, &public.id).await.unwrap();
    assert_eq!(list(&ctx, &me, RoomScope::ROOM_SCOPE_MINE).await, vec![public.id.clone()]);
    assert_eq!(
        ctx.reason(ROOMS, "ListStudyRooms", &me, &ListStudyRoomsRequest::default()).await,
        "ROOM_SCOPE_REQUIRED"
    );

    // A listed room shows who is in it and whether the caller is.
    let response: ListStudyRoomsResponse = ctx
        .ok(
            ROOMS,
            "ListStudyRooms",
            &me,
            &ListStudyRoomsRequest { scope: RoomScope::ROOM_SCOPE_MINE.into(), ..Default::default() },
        )
        .await;
    assert_eq!(
        (response.rooms[0].joined, response.rooms[0].member_count, member_names(&response.rooms[0])),
        (true, 2, vec!["stranger".to_owned(), "me".to_owned()])
    );

    // Studying in a room shows it to friends who may enter it.
    let response: ListStudyingFriendsResponse =
        ctx.ok(STUDY, "ListStudyingFriends", &pal, &ListStudyingFriendsRequest::default()).await;
    let mine = response
        .friends
        .iter()
        .find(|f| f.user.as_option().unwrap().id == me.id.to_string())
        .expect("pal sees me studying");
    assert_eq!((mine.room_id.as_str(), mine.room_title.as_str()), (public.id.as_str(), "もくもく"));
}

#[tokio::test]
async fn a_room_is_changed_reacted_in_and_closed() {
    let ctx = start().await;
    let (host, friend, stranger) =
        (ctx.app.user("host").await, ctx.app.user("friend").await, ctx.app.user("stranger").await);
    ctx.befriend(&host, &friend).await;
    let (math, english) = (ctx.subject(&friend, "数学").await, ctx.subject(&friend, "英語").await);
    ctx.at(2026, 4, 13, 18, 0);
    let room = room_of(&ctx.create_room(&host, |room| room.mode = StudyMode::STUDY_MODE_STOPWATCH.into()).await);
    let joined: JoinStudyRoomResponse = ctx
        .ok(
            ROOMS,
            "JoinStudyRoom",
            &friend,
            &JoinStudyRoomRequest { room_id: room.id.clone(), subject_id: math.id.clone(), ..Default::default() },
        )
        .await;
    assert_eq!(joined.room.as_option().unwrap().members[1].subject.as_option().unwrap().name, "数学");
    ctx.control(&host, &room.id, TimerAction::TIMER_ACTION_START).await.unwrap();
    let both = [&host, &friend];
    let mut topic = ctx.app.listen_topic(&room.topic).await;

    // Saying what one studies now: the time so far is logged under the old subject.
    ctx.pass(10);
    ctx.beat(&both).await;
    let switched: SetStudyRoomSubjectResponse = ctx
        .ok(
            ROOMS,
            "SetStudyRoomSubject",
            &friend,
            &SetStudyRoomSubjectRequest { subject_id: english.id.clone(), ..Default::default() },
        )
        .await;
    let session = switched.session.as_option().cloned().unwrap();
    assert_eq!(
        (session.subject.as_option().unwrap().name.as_str(), session.studied_seconds, session.room_id.as_str()),
        ("英語", 0, room.id.as_str())
    );
    assert_ne!(session.id, joined.session.as_option().unwrap().id);
    let logged = ctx.sessions(&friend).await;
    assert_eq!(
        logged.iter().map(|s| (s.subject.as_option().unwrap().name.clone(), s.studied_seconds)).collect::<Vec<_>>(),
        vec![("数学".to_owned(), 600)]
    );
    let heard_in_room = heard(&mut topic).await;
    assert_eq!(kinds(&heard_in_room), vec!["room_member_updated"]);
    let Push::RoomMemberUpdated(member) = &heard_in_room[0] else { unreachable!() };
    assert_eq!(member.subject.as_option().unwrap().name, "英語");
    assert_eq!(instant(&member.joined_at), tokyo(2026, 4, 13, 18, 0), "still in the room since they came in");
    assert_eq!(ctx.room(&host, &room.id).await.unwrap().member_count, 2);
    let foreign =
        SetStudyRoomSubjectRequest { subject_id: ctx.subject(&host, "物理").await.id, ..Default::default() };
    assert_eq!(ctx.reason(ROOMS, "SetStudyRoomSubject", &friend, &foreign).await, "SUBJECT_NOT_FOUND");
    assert_eq!(
        ctx.reason(ROOMS, "SetStudyRoomSubject", &stranger, &SetStudyRoomSubjectRequest::default()).await,
        "NOT_IN_ROOM"
    );
    let relabel = UpdateStudySessionRequest {
        session_id: session.id.clone(),
        subject_id: Some(math.id.clone()),
        ..Default::default()
    };
    assert_eq!(ctx.reason(STUDY, "UpdateStudySession", &friend, &relabel).await, "ROOM_SESSION");

    // Reactions instead of chat.
    let react = |reaction: RoomReaction, to: Option<&TestUser>| SendStudyRoomReactionRequest {
        reaction: reaction.into(),
        to_user_id: to.map(|u| u.id.to_string()).unwrap_or_default(),
        ..Default::default()
    };
    let _: SendStudyRoomReactionResponse =
        ctx.ok(ROOMS, "SendStudyRoomReaction", &friend, &react(RoomReaction::ROOM_REACTION_CHEER, None)).await;
    let _: SendStudyRoomReactionResponse =
        ctx.ok(ROOMS, "SendStudyRoomReaction", &host, &react(RoomReaction::ROOM_REACTION_COFFEE, Some(&friend))).await;
    let reactions: Vec<(String, RoomReaction, String)> = heard(&mut topic)
        .await
        .into_iter()
        .filter_map(|push| match push {
            Push::RoomReaction(sent) => Some((
                sent.from.as_option().unwrap().display_name.clone(),
                sent.reaction.as_known().unwrap(),
                sent.to_user_id.clone(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        reactions,
        vec![
            ("friend".to_owned(), RoomReaction::ROOM_REACTION_CHEER, String::new()),
            ("host".to_owned(), RoomReaction::ROOM_REACTION_COFFEE, friend.id.to_string())
        ]
    );
    for (user, request, reason) in [
        (&friend, react(RoomReaction::ROOM_REACTION_UNSPECIFIED, None), "REACTION_REQUIRED"),
        (&friend, react(RoomReaction::ROOM_REACTION_WAVE, Some(&stranger)), "USER_NOT_IN_ROOM"),
        (&stranger, react(RoomReaction::ROOM_REACTION_WAVE, None), "NOT_IN_ROOM"),
    ] {
        assert_eq!(ctx.reason(ROOMS, "SendStudyRoomReaction", user, &request).await, reason);
    }

    // The host changes the room; everyone following it hears.
    let update = |fill: fn(&mut UpdateStudyRoomRequest)| {
        let mut request = UpdateStudyRoomRequest::default();
        fill(&mut request);
        request
    };
    let with_room = |mut request: UpdateStudyRoomRequest| {
        request.room_id = room.id.clone();
        request
    };
    let changed: UpdateStudyRoomResponse = ctx
        .ok(
            ROOMS,
            "UpdateStudyRoom",
            &host,
            &with_room(update(|r| {
                r.title = Some("期末対策".into());
                r.audience = RoomAudience::ROOM_AUDIENCE_PUBLIC.into();
                r.member_cap = Some(5);
                r.mode = StudyMode::STUDY_MODE_POMODORO.into();
                r.pomodoro = PomodoroConfig { work_minutes: 50, break_minutes: 10, ..Default::default() }.into();
            })),
        )
        .await;
    let changed = changed.room.as_option().cloned().unwrap();
    assert_eq!(
        (changed.title.as_str(), changed.audience.as_known().unwrap(), changed.member_cap),
        ("期末対策", RoomAudience::ROOM_AUDIENCE_PUBLIC, 5)
    );
    let timer = changed.timer.as_option().unwrap();
    assert_eq!(
        (
            timer.mode.as_known().unwrap(),
            timer.pomodoro.as_option().unwrap().work_minutes,
            timer.state.as_known().unwrap(),
            timer.elapsed_seconds
        ),
        (StudyMode::STUDY_MODE_POMODORO, 50, TimerState::TIMER_STATE_RUNNING, 0)
    );
    assert_eq!(kinds(&heard(&mut topic).await), vec!["room_updated", "room_timer"]);
    assert!(ctx.room(&stranger, &room.id).await.is_ok(), "public now");
    for (user, request, reason) in [
        (&friend, with_room(update(|r| r.title = Some("x".into()))), "NOT_HOST"),
        (&host, with_room(update(|r| r.title = Some("".into()))), "ROOM_TITLE_INVALID"),
        (&host, with_room(update(|r| r.member_cap = Some(100))), "MEMBER_CAP_INVALID"),
        (
            &host,
            with_room(update(|r| r.pomodoro = PomodoroConfig { rounds: 50, ..Default::default() }.into())),
            "POMODORO_INVALID",
        ),
        (&host, update(|r| r.room_id = uuid::Uuid::now_v7().to_string()), "ROOM_NOT_FOUND"),
    ] {
        assert_eq!(ctx.reason(ROOMS, "UpdateStudyRoom", user, &request).await, reason);
    }

    // Closing: everyone is out, their time is logged, and the room is gone.
    for _ in 0..2 {
        ctx.pass(10);
        ctx.beat(&both).await;
    }
    let close = CloseStudyRoomRequest { room_id: room.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(ROOMS, "CloseStudyRoom", &friend, &close).await, "NOT_HOST");
    assert_eq!(
        ctx.reason(
            ROOMS,
            "CloseStudyRoom",
            &ctx.app.user("nobody").await,
            &CloseStudyRoomRequest { room_id: uuid::Uuid::now_v7().to_string(), ..Default::default() }
        )
        .await,
        "ROOM_NOT_FOUND"
    );
    let mut friend_rt = ctx.app.listen_user(friend.id).await;
    let _: CloseStudyRoomResponse = ctx.ok(ROOMS, "CloseStudyRoom", &host, &close).await;
    let heard_in_room = heard(&mut topic).await;
    assert_eq!(kinds(&heard_in_room), vec!["room_closed"]);
    let Push::RoomClosed(closed) = &heard_in_room[0] else { unreachable!() };
    assert_eq!(closed.reason, RoomCloseReason::ROOM_CLOSE_REASON_HOST);
    let heard_by_friend = heard(&mut friend_rt).await;
    let Some(Push::RoomLeft(left)) = heard_by_friend.iter().find(|push| kind(push) == "room_left") else {
        panic!("{:?}", kinds(&heard_by_friend))
    };
    assert_eq!(
        (left.reason.as_known().unwrap(), left.session.as_option().unwrap().studied_seconds),
        (RoomLeaveReason::ROOM_LEAVE_REASON_CLOSED, 1200)
    );
    assert_eq!(ctx.sessions(&friend).await.iter().map(|s| s.studied_seconds).collect::<Vec<_>>(), vec![1200, 600]);
    assert_eq!(
        ctx.sessions(&host).await.iter().map(|s| s.studied_seconds).collect::<Vec<_>>(),
        vec![1800],
        "the host's one stay, before and after the change of lengths"
    );
    let events = ctx.app.events_by("school.room.closed", host.id).await;
    assert_eq!(
        (events.len(), events[0].data["room_id"].as_str(), events[0].data["reason"].as_i64()),
        (1, Some(room.id.as_str()), Some(1))
    );

    // A closed room is gone for everyone but its host, and nothing works on it.
    assert_eq!(ctx.room(&friend, &room.id).await.unwrap_err().reason, "ROOM_NOT_FOUND");
    assert!(ctx.room(&host, &room.id).await.unwrap().closed_at.as_option().is_some());
    assert_eq!(ctx.join(&friend, &room.id).await.unwrap_err().reason, "ROOM_CLOSED");
    assert_eq!(ctx.control(&host, &room.id, TimerAction::TIMER_ACTION_PAUSE).await.unwrap_err().reason, "ROOM_CLOSED");
    assert_eq!(
        ctx.reason(ROOMS, "UpdateStudyRoom", &host, &with_room(update(|r| r.title = Some("x".into())))).await,
        "ROOM_CLOSED"
    );
    let invite = InviteToStudyRoomRequest {
        room_id: room.id.clone(),
        user_ids: vec![friend.id.to_string()],
        ..Default::default()
    };
    assert_eq!(ctx.reason(ROOMS, "InviteToStudyRoom", &host, &invite).await, "ROOM_CLOSED");
    let _: CloseStudyRoomResponse = ctx.ok(ROOMS, "CloseStudyRoom", &host, &close).await;
    assert!(list(&ctx, &host, RoomScope::ROOM_SCOPE_MINE).await.is_empty());
    // The host is free to open another.
    ctx.create_room(&host, |_| {}).await;
}
