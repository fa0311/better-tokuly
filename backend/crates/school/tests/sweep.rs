//! The periodic sweep: sessions nobody is behind anymore, study room phases,
//! empty rooms, and the reminders for classes and assignments.
//!
//! The sweep acts on everything in the database, and every test has its own
//! clock, so the tests of this file run one at a time ([`SWEEPS`]) and look
//! only at what they made themselves.
mod common;

use common::*;
use tokio::sync::Mutex;
use tokuly_proto::school::{school_event::Event as Push, *};

static SWEEPS: Mutex<()> = Mutex::const_new(());

const CLASSIC: PomodoroConfig = PomodoroConfig {
    work_minutes: 25,
    break_minutes: 5,
    long_break_minutes: 15,
    rounds: 4,
    __buffa_unknown_fields: buffa::UnknownFields::new(),
};

/// Runs the class reminder step; returns every class reminder `user` has got so far.
async fn sent(ctx: &Ctx, user: uuid::Uuid) -> Vec<String> {
    ctx.school.send_class_reminders(ctx.school.now()).await.unwrap();
    ctx.notifications_for(user, "school.class_soon")
        .await
        .iter()
        .map(|n| n.data["body"]["ja"].as_str().unwrap().to_owned())
        .collect()
}

/// Runs the assignment reminder step; returns every assignment reminder
/// `user` has got so far, as `(text, which reminder)`.
async fn reminders(ctx: &Ctx, user: uuid::Uuid) -> Vec<(String, String)> {
    ctx.school.send_assignment_reminders(ctx.school.now()).await.unwrap();
    ctx.notifications_for(user, "school.assignment_due")
        .await
        .iter()
        .map(|n| {
            (n.data["body"]["ja"].as_str().unwrap().to_owned(), n.data["data"]["reminder"].as_str().unwrap().to_owned())
        })
        .collect()
}

#[tokio::test]
async fn sessions_nobody_is_behind_are_ended_at_their_last_heartbeat() {
    let _one_at_a_time = SWEEPS.lock().await;
    let ctx = start().await;
    let (me, friend, host, guest) = (
        ctx.app.user("me").await,
        ctx.app.user("friend").await,
        ctx.app.user("host").await,
        ctx.app.user("guest").await,
    );
    ctx.befriend(&me, &friend).await;
    ctx.at(2026, 4, 13, 18, 0);

    // A stopwatch, and a room with its timer running.
    ctx.start_stopwatch(&me).await;
    let created = ctx
        .create_room(&host, |room| {
            room.audience = RoomAudience::ROOM_AUDIENCE_PUBLIC.into();
            room.pomodoro = CLASSIC.into();
        })
        .await;
    let room = created.room.as_option().cloned().unwrap();
    ctx.join(&guest, &room.id).await.unwrap();
    ctx.control(&host, &room.id, TimerAction::TIMER_ACTION_START).await.unwrap();
    ctx.pass(5);
    ctx.heartbeat(&me).await; // the last anyone hears of `me`
    ctx.pass(5);
    ctx.beat(&[&host, &guest]).await; // the last anyone hears of `guest`

    let mut my_devices = ctx.app.listen_user(me.id).await;
    let mut friend_rt = ctx.app.listen_user(friend.id).await;
    // 18:22: `me` has been silent for 17 minutes, `guest` for 12.
    ctx.pass(12);
    ctx.heartbeat(&host).await;
    let report = ctx.school.sweep(ctx.school.now()).await;
    assert!(report.sessions_ended >= 1, "{report:?}");
    let logged = ctx.sessions(&me).await;
    assert_eq!(
        logged.iter().map(|s| (s.studied_seconds, s.auto_stopped)).collect::<Vec<_>>(),
        vec![(300, true)],
        "counted up to the last heartbeat"
    );
    assert_eq!(instant(&logged[0].ended_at), tokyo(2026, 4, 13, 18, 5));
    let heard_by_me = heard(&mut my_devices).await;
    let Some(Push::Session(ended)) = heard_by_me.first() else {
        panic!("my devices hear that the session ended: {:?}", kinds(&heard_by_me))
    };
    assert_eq!((ended.state.as_known().unwrap(), ended.auto_stopped), (SessionState::SESSION_STATE_ENDED, true));
    let heard_by_friend = heard(&mut friend_rt).await;
    assert!(
        matches!(heard_by_friend.first(), Some(Push::FriendStudying(event)) if !event.studying),
        "{:?}",
        kinds(&heard_by_friend)
    );
    assert_eq!(ctx.room(&host, &room.id).await.unwrap().member_count, 2, "the guest is still within the limit");

    // The host pauses and resumes: everyone's time is counted up to that
    // moment — the silent guest's too, for now.
    ctx.control(&host, &room.id, TimerAction::TIMER_ACTION_PAUSE).await.unwrap();
    ctx.control(&host, &room.id, TimerAction::TIMER_ACTION_RESUME).await.unwrap();
    assert_eq!(ctx.room(&host, &room.id).await.unwrap().members[1].studied_seconds, 1320);

    // 18:27: the guest has been silent for 17 minutes. Out of the room, and
    // back to what was counted when they were last heard from.
    let mut topic = ctx.app.listen_topic(&room.topic).await;
    let mut guest_rt = ctx.app.listen_user(guest.id).await;
    ctx.pass(5);
    ctx.heartbeat(&host).await;
    assert!(ctx.school.end_overdue_sessions(ctx.school.now()).await.unwrap() >= 1);
    let heard_in_room = heard(&mut topic).await;
    let Some(Push::RoomMemberLeft(gone)) = heard_in_room.first() else {
        panic!("the room hears that the guest is gone: {:?}", kinds(&heard_in_room))
    };
    assert_eq!(
        (gone.user_id.as_str(), gone.reason.as_known().unwrap(), gone.member_count),
        (guest.id.to_string().as_str(), RoomLeaveReason::ROOM_LEAVE_REASON_IDLE, 1)
    );
    let heard_by_guest = heard(&mut guest_rt).await;
    let Some(Push::RoomLeft(left)) = heard_by_guest.iter().find(|push| kind(push) == "room_left") else {
        panic!("{:?}", kinds(&heard_by_guest))
    };
    assert_eq!(
        (left.reason.as_known().unwrap(), left.session.as_option().unwrap().studied_seconds),
        (RoomLeaveReason::ROOM_LEAVE_REASON_IDLE, 600)
    );
    assert_eq!(
        ctx.sessions(&guest).await.iter().map(|s| (s.studied_seconds, s.auto_stopped)).collect::<Vec<_>>(),
        vec![(600, true)]
    );
    // The host, heard from all along, is untouched.
    let seen = ctx.room(&host, &room.id).await.unwrap();
    assert_eq!((seen.member_count, seen.host_present), (1, true));
    assert_eq!(ctx.heartbeat(&host).await.state, SessionState::SESSION_STATE_RUNNING);
}

#[tokio::test]
async fn phases_that_end_by_themselves_are_announced() {
    let _one_at_a_time = SWEEPS.lock().await;
    let ctx = start().await;
    let host = ctx.app.user("host").await;
    ctx.at(2026, 4, 13, 18, 0);
    let room = ctx.create_room(&host, |room| room.pomodoro = CLASSIC.into()).await.room.as_option().cloned().unwrap();
    ctx.control(&host, &room.id, TimerAction::TIMER_ACTION_START).await.unwrap();
    let mut topic = ctx.app.listen_topic(&room.topic).await;
    let timers = |pushes: Vec<Push>| -> Vec<(TimerPhase, i32)> {
        pushes
            .into_iter()
            .filter_map(|push| match push {
                Push::RoomTimer(timer) => Some((timer.phase.as_known().unwrap(), timer.round)),
                _ => None,
            })
            .collect()
    };

    // Nothing to say while the first round is on.
    ctx.pass(24);
    ctx.school.announce_room_phases(ctx.school.now()).await.unwrap();
    assert!(heard(&mut topic).await.is_empty());
    // The round is over: the break is announced, once.
    ctx.pass(1);
    assert!(ctx.school.announce_room_phases(ctx.school.now()).await.unwrap() >= 1);
    let heard_in_room = heard(&mut topic).await;
    let Some(Push::RoomTimer(timer)) = heard_in_room.first() else { panic!("{:?}", kinds(&heard_in_room)) };
    assert_eq!(
        (timer.phase.as_known().unwrap(), timer.round, instant(&timer.phase_ends_at), instant(&timer.server_time)),
        (TimerPhase::TIMER_PHASE_BREAK, 2, tokyo(2026, 4, 13, 18, 30), tokyo(2026, 4, 13, 18, 25))
    );
    ctx.school.announce_room_phases(ctx.school.now()).await.unwrap();
    assert!(heard(&mut topic).await.is_empty());
    // And the second round after the break.
    ctx.pass(5);
    ctx.school.announce_room_phases(ctx.school.now()).await.unwrap();
    assert_eq!(timers(heard(&mut topic).await), vec![(TimerPhase::TIMER_PHASE_WORK, 2)]);

    // A paused timer has no phase ending.
    ctx.control(&host, &room.id, TimerAction::TIMER_ACTION_PAUSE).await.unwrap();
    heard(&mut topic).await;
    ctx.pass(60);
    ctx.school.announce_room_phases(ctx.school.now()).await.unwrap();
    assert!(heard(&mut topic).await.is_empty());
}

#[tokio::test]
async fn rooms_close_after_standing_empty() {
    let _one_at_a_time = SWEEPS.lock().await;
    let ctx = start().await;
    let (host, friend) = (ctx.app.user("host").await, ctx.app.user("friend").await);
    ctx.befriend(&host, &friend).await;
    ctx.at(2026, 4, 13, 18, 0);
    let room = ctx.create_room(&host, |_| {}).await.room.as_option().cloned().unwrap();
    let _: LeaveStudyRoomResponse = ctx.ok(ROOMS, "LeaveStudyRoom", &host, &LeaveStudyRoomRequest::default()).await;
    let closed = |room: StudyRoom| room.closed_at.as_option().is_some();

    // Empty for nine minutes: still open (rooms close after ten).
    ctx.pass(9);
    ctx.school.close_empty_rooms(ctx.school.now()).await.unwrap();
    assert!(!closed(ctx.room(&host, &room.id).await.unwrap()));
    // Someone looking in starts the wait over.
    ctx.join(&friend, &room.id).await.unwrap();
    let _: LeaveStudyRoomResponse = ctx.ok(ROOMS, "LeaveStudyRoom", &friend, &LeaveStudyRoomRequest::default()).await;
    ctx.pass(9);
    ctx.school.close_empty_rooms(ctx.school.now()).await.unwrap();
    assert!(!closed(ctx.room(&host, &room.id).await.unwrap()));

    let mut topic = ctx.app.listen_topic(&room.topic).await;
    ctx.pass(2);
    assert!(ctx.school.close_empty_rooms(ctx.school.now()).await.unwrap() >= 1);
    assert!(closed(ctx.room(&host, &room.id).await.unwrap()));
    let heard_in_room = heard(&mut topic).await;
    let Some(Push::RoomClosed(event)) = heard_in_room.first() else { panic!("{:?}", kinds(&heard_in_room)) };
    assert_eq!(event.reason, RoomCloseReason::ROOM_CLOSE_REASON_EMPTY);
    let events = ctx.app.events_by("school.room.closed", host.id).await;
    assert_eq!((events.len(), events[0].data["reason"].as_i64()), (1, Some(2)));
    assert_eq!(ctx.join(&friend, &room.id).await.unwrap_err().reason, "ROOM_CLOSED");
}

#[tokio::test]
async fn the_class_reminder_comes_before_the_first_class_for_those_who_asked() {
    let _one_at_a_time = SWEEPS.lock().await;
    let ctx = start().await;
    let (me, other) = (ctx.app.user("me").await, ctx.app.user("other").await);
    ctx.at(2026, 4, 13, 7, 0); // a Monday morning
    let mut terms = Vec::new();
    for user in [&me, &other] {
        // Monday: first period math in 2-A. Tuesday: second period English.
        let (math, english) = (ctx.subject(user, "数学").await, ctx.subject(user, "英語").await);
        let term = ctx.term(user, |_| {}).await;
        ctx.put(
            user,
            &term,
            vec![
                TimetableEntryInput { room: "2-A".into(), ..class(Weekday::WEEKDAY_MONDAY, 1, &math) },
                class(Weekday::WEEKDAY_TUESDAY, 2, &english),
            ],
        )
        .await;
        terms.push((term, english));
    }
    ctx.update_settings(&me, UpdateSchoolSettingsRequest { class_soon_minutes: Some(10), ..Default::default() }).await;
    let me_id = me.id;

    // Not yet at 8:30; at 8:40, ten minutes before the first class; and only once.
    ctx.at(2026, 4, 13, 8, 30);
    assert!(sent(&ctx, me_id).await.is_empty());
    ctx.at(2026, 4, 13, 8, 40);
    assert_eq!(sent(&ctx, me_id).await, vec!["1限 数学（2-A） 8:50〜"]);
    assert_eq!(sent(&ctx, me_id).await.len(), 1);
    ctx.at(2026, 4, 13, 9, 0);
    assert_eq!(sent(&ctx, me_id).await.len(), 1);
    let notified = ctx.notifications_for(me.id, "school.class_soon").await;
    assert_eq!(
        (notified[0].data["title"]["ja"].as_str(), notified[0].data["data"]["period"].as_i64(), notified[0].actor_id),
        (Some("もうすぐ授業!"), Some(1), None)
    );

    // Tuesday's first class is the second period.
    ctx.at(2026, 4, 14, 9, 39);
    assert_eq!(sent(&ctx, me_id).await.len(), 1);
    ctx.at(2026, 4, 14, 9, 40);
    assert_eq!(sent(&ctx, me_id).await.last().unwrap(), "2限 英語 9:50〜");

    // Next Monday the first class is cancelled: no class, no reminder...
    let (term, english) = &terms[0];
    let cancel = CreateTimetableChangeRequest {
        term_id: term.id.clone(),
        date: "2026-04-20".into(),
        period: 1,
        kind: ChangeKind::CHANGE_KIND_CANCELLED.into(),
        ..Default::default()
    };
    let _: CreateTimetableChangeResponse = ctx.ok(TIMETABLE, "CreateTimetableChange", &me, &cancel).await;
    ctx.at(2026, 4, 20, 8, 45);
    assert_eq!(sent(&ctx, me_id).await.len(), 2);
    // ...until the timetable gets a later class that day.
    ctx.put(&me, term, vec![class(Weekday::WEEKDAY_MONDAY, 2, english)]).await;
    ctx.at(2026, 4, 20, 9, 41);
    assert_eq!(sent(&ctx, me_id).await, vec!["1限 数学（2-A） 8:50〜", "2限 英語 9:50〜", "2限 英語 9:50〜"]);

    // Switching it off stops it. The classmate who never asked got nothing.
    ctx.update_settings(&me, UpdateSchoolSettingsRequest { class_soon_minutes: Some(0), ..Default::default() }).await;
    ctx.at(2026, 4, 21, 9, 45);
    assert_eq!(sent(&ctx, me_id).await.len(), 3);
    assert!(ctx.notifications_for(other.id, "school.class_soon").await.is_empty());
}

#[tokio::test]
async fn assignment_reminders_come_the_evening_before_and_an_hour_before() {
    let _one_at_a_time = SWEEPS.lock().await;
    let ctx = start().await;
    let (me, quiet) = (ctx.app.user("me").await, ctx.app.user("quiet").await);
    ctx.at(2026, 6, 8, 12, 0);
    let create = |title: &str, due: chrono::DateTime<chrono::Utc>| CreateAssignmentRequest {
        title: title.into(),
        due_at: stamp(due),
        ..Default::default()
    };
    let due = tokyo(2026, 6, 10, 9, 0);

    let _: CreateAssignmentResponse = ctx.ok(ASSIGNMENTS, "CreateAssignment", &me, &create("数学レポート", due)).await;
    let _: CreateAssignmentResponse = ctx
        .ok(
            ASSIGNMENTS,
            "CreateAssignment",
            &me,
            &CreateAssignmentRequest { remind: Some(false), ..create("静かな課題", due) },
        )
        .await;
    let finished: CreateAssignmentResponse =
        ctx.ok(ASSIGNMENTS, "CreateAssignment", &me, &create("もう終わった課題", due)).await;
    let done = UpdateAssignmentRequest {
        assignment_id: finished.assignment.as_option().unwrap().id.clone(),
        done: Some(true),
        ..Default::default()
    };
    let _: UpdateAssignmentResponse = ctx.ok(ASSIGNMENTS, "UpdateAssignment", &me, &done).await;
    ctx.update_settings(
        &quiet,
        UpdateSchoolSettingsRequest { assignment_reminders: Some(false), ..Default::default() },
    )
    .await;
    let _: CreateAssignmentResponse = ctx.ok(ASSIGNMENTS, "CreateAssignment", &quiet, &create("通知なし", due)).await;

    // The evening before, at seven; once.
    ctx.at(2026, 6, 9, 18, 59);
    assert!(reminders(&ctx, me.id).await.is_empty());
    ctx.at(2026, 6, 9, 19, 0);
    assert_eq!(reminders(&ctx, me.id).await, vec![("明日9:00まで: 数学レポート".to_owned(), "evening".to_owned())]);
    assert_eq!(reminders(&ctx, me.id).await.len(), 1);
    // An hour before; once.
    ctx.at(2026, 6, 10, 8, 0);
    assert_eq!(
        reminders(&ctx, me.id).await.last().unwrap(),
        &("あと1時間: 数学レポート".to_owned(), "hour".to_owned())
    );
    ctx.at(2026, 6, 10, 8, 30);
    assert_eq!(
        reminders(&ctx, me.id).await.len(),
        2,
        "nothing for the assignment without reminders or the finished one"
    );
    assert!(reminders(&ctx, quiet.id).await.is_empty(), "nothing for someone who switched reminders off");

    // Switching reminders on schedules what is still ahead.
    ctx.at(2026, 6, 11, 12, 0);
    let _: CreateAssignmentResponse =
        ctx.ok(ASSIGNMENTS, "CreateAssignment", &quiet, &create("やっぱり通知", tokyo(2026, 6, 12, 9, 0))).await;
    ctx.update_settings(&quiet, UpdateSchoolSettingsRequest { assignment_reminders: Some(true), ..Default::default() })
        .await;
    ctx.at(2026, 6, 11, 19, 0);
    assert_eq!(
        reminders(&ctx, quiet.id).await,
        vec![("明日9:00まで: やっぱり通知".to_owned(), "evening".to_owned())],
        "the one already past due stays silent"
    );

    // The worker was away over both moments: one reminder, the later one.
    let _: CreateAssignmentResponse =
        ctx.ok(ASSIGNMENTS, "CreateAssignment", &me, &create("英語の課題", tokyo(2026, 6, 13, 9, 0))).await;
    ctx.at(2026, 6, 13, 8, 10);
    let all = reminders(&ctx, me.id).await;
    assert_eq!((all.len(), all.last().unwrap()), (3, &("あと1時間: 英語の課題".to_owned(), "hour".to_owned())));
    assert_eq!(reminders(&ctx, me.id).await.len(), 3);
    // Away past the due time: reminding now would be noise.
    let _: CreateAssignmentResponse =
        ctx.ok(ASSIGNMENTS, "CreateAssignment", &me, &create("間に合わなかった", tokyo(2026, 6, 14, 9, 0))).await;
    ctx.at(2026, 6, 14, 9, 30);
    assert_eq!(reminders(&ctx, me.id).await.len(), 3);
}
