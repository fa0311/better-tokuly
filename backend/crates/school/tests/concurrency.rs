//! Requests that arrive at the same moment.
//!
//! Two things are checked wherever the domain takes a lock or relies on a
//! unique index:
//! - a burst from one account never stalls the API. The test pool has five
//!   connections and every burst here is more than twice that; a handler
//!   that held its transaction while asking the pool for a second connection
//!   would leave all of them waiting out the pool's timeout and answer with
//!   a server error, which [`outcomes`] rejects;
//! - whatever must happen once happens once: one session, one room, one
//!   membership, one log entry, one cheer.
mod common;

use std::collections::BTreeMap;

use common::*;
use futures::future::join_all;
use serde::de::DeserializeOwned;
use tokuly_proto::school::*;
use tokuly_testkit::{RpcError, TestUser};

/// More than twice the connections of the test pool.
const BURST: usize = 12;

/// Fires `BURST` identical calls from one account at once.
async fn burst<Req: serde::Serialize, Resp: DeserializeOwned>(
    ctx: &Ctx,
    service: &str,
    method: &str,
    user: &TestUser,
    request: &Req,
) -> Vec<Result<Resp, RpcError>> {
    join_all((0..BURST).map(|_| ctx.call::<_, Resp>(service, method, user, request))).await
}

/// How many calls succeeded, and how many failed with each reason. A server
/// error — which is what a starved connection pool produces — fails the test.
fn outcomes<Resp>(results: &[Result<Resp, RpcError>]) -> (usize, BTreeMap<String, usize>) {
    let mut reasons = BTreeMap::new();
    for error in results.iter().filter_map(|result| result.as_ref().err()) {
        assert!(error.status < 500, "a request failed on the server's side: {error}");
        *reasons.entry(error.reason.clone()).or_insert(0) += 1;
    }
    (results.iter().filter(|result| result.is_ok()).count(), reasons)
}

/// All calls succeeded.
fn all_ok<Resp>(results: &[Result<Resp, RpcError>]) {
    let (succeeded, reasons) = outcomes(results);
    assert_eq!((succeeded, reasons.len()), (BURST, 0), "{reasons:?}");
}

/// Exactly one call succeeded; the others were turned away with `reason`.
fn one_ok<Resp>(results: &[Result<Resp, RpcError>], reason: &str) {
    let (succeeded, reasons) = outcomes(results);
    assert_eq!(succeeded, 1, "{reasons:?}");
    assert_eq!(reasons, BTreeMap::from([(reason.to_owned(), BURST - 1)]));
}

#[tokio::test]
async fn a_burst_from_one_account_neither_stalls_the_api_nor_doubles_anything() {
    let ctx = start().await;
    let (me, classmate) = (ctx.app.user("me").await, ctx.app.user("classmate").await);
    ctx.befriend(&me, &classmate).await;
    ctx.at(2026, 4, 13, 18, 0);

    // The timer: one session however many starts arrive.
    let start = StartStudySessionRequest { mode: StudyMode::STUDY_MODE_STOPWATCH.into(), ..Default::default() };
    one_ok(
        &burst::<_, StartStudySessionResponse>(&ctx, STUDY, "StartStudySession", &me, &start).await,
        "SESSION_ALREADY_ACTIVE",
    );
    ctx.pass(10);
    // Heartbeats and edits of the running session, interleaved.
    all_ok(
        &burst::<_, HeartbeatStudySessionResponse>(
            &ctx,
            STUDY,
            "HeartbeatStudySession",
            &me,
            &HeartbeatStudySessionRequest::default(),
        )
        .await,
    );
    let current: GetCurrentStudySessionResponse =
        ctx.ok(STUDY, "GetCurrentStudySession", &me, &GetCurrentStudySessionRequest::default()).await;
    let session = current.session.as_option().cloned().unwrap();
    assert_eq!(session.studied_seconds, 600);
    let relabel =
        UpdateStudySessionRequest {
            session_id: session.id.clone(), memo: Some("同時に".into()), ..Default::default()
        };
    let beat = HeartbeatStudySessionRequest::default();
    let (beats, edits) = tokio::join!(
        burst::<_, HeartbeatStudySessionResponse>(&ctx, STUDY, "HeartbeatStudySession", &me, &beat),
        burst::<_, UpdateStudySessionResponse>(&ctx, STUDY, "UpdateStudySession", &me, &relabel),
    );
    all_ok(&beats);
    all_ok(&edits);
    // Pausing: one pause; the others find the timer paused already.
    one_ok(
        &burst::<_, PauseStudySessionResponse>(
            &ctx,
            STUDY,
            "PauseStudySession",
            &me,
            &PauseStudySessionRequest::default(),
        )
        .await,
        "SESSION_NOT_RUNNING",
    );
    // Stopping: the session is logged once.
    one_ok(
        &burst::<_, StopStudySessionResponse>(
            &ctx,
            STUDY,
            "StopStudySession",
            &me,
            &StopStudySessionRequest::default(),
        )
        .await,
        "NO_ACTIVE_SESSION",
    );
    assert_eq!(ctx.sessions(&me).await.iter().map(|s| s.studied_seconds).collect::<Vec<_>>(), vec![600]);
    assert_eq!(ctx.app.events_by("school.study.session_ended", me.id).await.len(), 1);

    // Logging the same hour by hand: once.
    let log = LogStudySessionRequest {
        started_at: stamp(tokyo(2026, 4, 13, 15, 0)),
        ended_at: stamp(tokyo(2026, 4, 13, 16, 0)),
        ..Default::default()
    };
    one_ok(&burst::<_, LogStudySessionResponse>(&ctx, STUDY, "LogStudySession", &me, &log).await, "SESSION_OVERLAPS");

    // Subjects: one per name. Terms: all created, exactly one current.
    let subject = CreateSubjectRequest { name: "数学".into(), ..Default::default() };
    one_ok(
        &burst::<_, CreateSubjectResponse>(&ctx, TIMETABLE, "CreateSubject", &me, &subject).await,
        "SUBJECT_NAME_TAKEN",
    );
    let term = CreateTermRequest {
        name: "1学期".into(),
        start_date: "2026-04-08".into(),
        end_date: "2026-07-20".into(),
        ..Default::default()
    };
    all_ok(&burst::<_, CreateTermResponse>(&ctx, TIMETABLE, "CreateTerm", &me, &term).await);
    let terms: ListTermsResponse = ctx.ok(TIMETABLE, "ListTerms", &me, &ListTermsRequest::default()).await;
    assert_eq!((terms.terms.len(), terms.terms.iter().filter(|t| t.current).count()), (BURST, 1));

    // The grid: twelve writes to one slot leave one class in it.
    let subjects: ListSubjectsResponse = ctx.ok(TIMETABLE, "ListSubjects", &me, &ListSubjectsRequest::default()).await;
    let term_id = terms.terms[0].id.clone();
    let put = PutTimetableEntriesRequest {
        term_id: term_id.clone(),
        entries: vec![class(Weekday::WEEKDAY_MONDAY, 1, &subjects.subjects[0])],
        ..Default::default()
    };
    all_ok(&burst::<_, PutTimetableEntriesResponse>(&ctx, TIMETABLE, "PutTimetableEntries", &me, &put).await);
    let loaded: GetTermResponse =
        ctx.ok(TIMETABLE, "GetTerm", &me, &GetTermRequest { term_id: term_id.clone(), ..Default::default() }).await;
    assert_eq!(loaded.entries.len(), 1);
    // One-off changes for one slot and date: one change.
    let cancel = CreateTimetableChangeRequest {
        term_id: term_id.clone(),
        date: "2026-04-13".into(),
        period: 1,
        kind: ChangeKind::CHANGE_KIND_CANCELLED.into(),
        ..Default::default()
    };
    all_ok(&burst::<_, CreateTimetableChangeResponse>(&ctx, TIMETABLE, "CreateTimetableChange", &me, &cancel).await);
    let loaded: GetTermResponse =
        ctx.ok(TIMETABLE, "GetTerm", &me, &GetTermRequest { term_id, ..Default::default() }).await;
    assert_eq!(loaded.changes.len(), 1);

    // Copying a classmate's term: every copy is made (two locks and several writes each).
    ctx.subject(&classmate, "英語").await;
    ctx.term(&classmate, |t| t.audience = TimetableAudience::TIMETABLE_AUDIENCE_FRIENDS.into()).await;
    let copy = CopyFriendTimetableRequest { user_id: classmate.id.to_string(), ..Default::default() };
    all_ok(&burst::<_, CopyFriendTimetableResponse>(&ctx, TIMETABLE, "CopyFriendTimetable", &me, &copy).await);

    // Settings, exams, assignments.
    let goal = UpdateSchoolSettingsRequest {
        daily_goal_minutes: Some(60),
        class_soon_minutes: Some(10),
        ..Default::default()
    };
    all_ok(&burst::<_, UpdateSchoolSettingsResponse>(&ctx, SETTINGS, "UpdateSchoolSettings", &me, &goal).await);
    let exam: CreateExamResponse = ctx
        .ok(
            EXAMS,
            "CreateExam",
            &me,
            &CreateExamRequest { name: "期末".into(), start_date: "2026-07-01".into(), ..Default::default() },
        )
        .await;
    let test = PutExamSubjectRequest {
        exam_id: exam.exam.as_option().unwrap().id.clone(),
        subject_id: subjects.subjects[0].id.clone(),
        ..Default::default()
    };
    one_ok(&burst::<_, PutExamSubjectResponse>(&ctx, EXAMS, "PutExamSubject", &me, &test).await, "EXAM_SUBJECT_EXISTS");
    let homework =
        CreateAssignmentRequest {
            title: "宿題".into(), due_at: stamp(tokyo(2026, 4, 20, 9, 0)), ..Default::default()
        };
    let created = burst::<_, CreateAssignmentResponse>(&ctx, ASSIGNMENTS, "CreateAssignment", &me, &homework).await;
    all_ok(&created);
    let done = UpdateAssignmentRequest {
        assignment_id: created[0].as_ref().unwrap().assignment.as_option().unwrap().id.clone(),
        done: Some(true),
        ..Default::default()
    };
    all_ok(&burst::<_, UpdateAssignmentResponse>(&ctx, ASSIGNMENTS, "UpdateAssignment", &me, &done).await);
}

#[tokio::test]
async fn rooms_hold_one_of_everything_under_concurrent_requests() {
    let ctx = start().await;
    let (host, guest) = (ctx.app.user("host").await, ctx.app.user("guest").await);
    ctx.at(2026, 4, 13, 18, 0);

    // Twelve requests to open a room: one room, with its host inside. (Ten
    // rooms an hour is the limit, so two of the twelve stop there.)
    let open = CreateStudyRoomRequest {
        title: "同時に".into(),
        audience: RoomAudience::ROOM_AUDIENCE_PUBLIC.into(),
        ..Default::default()
    };
    let results = burst::<_, CreateStudyRoomResponse>(&ctx, ROOMS, "CreateStudyRoom", &host, &open).await;
    let (succeeded, reasons) = outcomes(&results);
    assert_eq!(succeeded, 1, "{reasons:?}");
    assert!(reasons.keys().all(|reason| reason == "ALREADY_HOSTING" || reason == "RATE_LIMITED"), "{reasons:?}");
    let room = results.iter().find_map(|result| result.as_ref().ok()).unwrap().room.as_option().cloned().unwrap();
    let mine: ListStudyRoomsResponse = ctx
        .ok(
            ROOMS,
            "ListStudyRooms",
            &host,
            &ListStudyRoomsRequest { scope: RoomScope::ROOM_SCOPE_MINE.into(), ..Default::default() },
        )
        .await;
    assert_eq!(
        mine.rooms.iter().map(|r| (r.id.as_str(), r.joined, r.member_count, r.host_present)).collect::<Vec<_>>(),
        vec![(room.id.as_str(), true, 1, true)]
    );
    assert_eq!(ctx.app.events_by("school.room.opened", host.id).await.len(), 1);

    // One person entering twelve times at once is in the room once.
    let join = JoinStudyRoomRequest { room_id: room.id.clone(), ..Default::default() };
    let joins = burst::<_, JoinStudyRoomResponse>(&ctx, ROOMS, "JoinStudyRoom", &guest, &join).await;
    all_ok(&joins);
    let mut sessions: Vec<String> =
        joins.iter().map(|join| join.as_ref().unwrap().session.as_option().unwrap().id.clone()).collect();
    sessions.dedup();
    assert_eq!(sessions.len(), 1, "every request got the same stay");
    let seen = ctx.room(&host, &room.id).await.unwrap();
    assert_eq!((seen.member_count, seen.members.len()), (2, 2));
    // The host's timer, pushed twelve times at once: started once.
    let start = ControlStudyRoomTimerRequest {
        room_id: room.id.clone(),
        action: TimerAction::TIMER_ACTION_START.into(),
        ..Default::default()
    };
    one_ok(
        &burst::<_, ControlStudyRoomTimerResponse>(&ctx, ROOMS, "ControlStudyRoomTimer", &host, &start).await,
        "TIMER_ACTION_INVALID",
    );
    // Leaving twelve times at once: out once, the stay logged once.
    ctx.pass(10);
    ctx.beat(&[&host, &guest]).await;
    one_ok(
        &burst::<_, LeaveStudyRoomResponse>(&ctx, ROOMS, "LeaveStudyRoom", &guest, &LeaveStudyRoomRequest::default())
            .await,
        "NOT_IN_ROOM",
    );
    assert_eq!(ctx.sessions(&guest).await.iter().map(|s| s.studied_seconds).collect::<Vec<_>>(), vec![600]);
    assert_eq!(ctx.room(&host, &room.id).await.unwrap().member_count, 1);
}

#[tokio::test]
async fn a_room_with_one_place_left_takes_one_of_many() {
    let ctx = start().await;
    let host = ctx.app.user("host").await;
    ctx.at(2026, 4, 13, 18, 0);
    let created = ctx
        .create_room(&host, |room| {
            room.audience = RoomAudience::ROOM_AUDIENCE_PUBLIC.into();
            room.member_cap = 2;
        })
        .await;
    let room = created.room.as_option().cloned().unwrap();
    let mut hopefuls = Vec::new();
    for n in 0..8 {
        hopefuls.push(ctx.app.user(&format!("hopeful{n}")).await);
    }
    let results = join_all(hopefuls.iter().map(|user| ctx.join(user, &room.id))).await;
    let (got_in, reasons) = outcomes(&results);
    assert_eq!((got_in, reasons), (1, BTreeMap::from([("ROOM_FULL".to_owned(), 7)])));
    let seen = ctx.room(&host, &room.id).await.unwrap();
    assert_eq!((seen.member_count, seen.members.len()), (2, 2));
}

#[tokio::test]
async fn cheers_and_invitations_do_not_multiply() {
    let ctx = start().await;
    let (me, friend) = (ctx.app.user("me").await, ctx.app.user("friend").await);
    ctx.befriend(&me, &friend).await;
    ctx.at(2026, 4, 13, 18, 0);

    // Twelve cheers at once: one gets through, the friend's phone stays quiet.
    ctx.start_stopwatch(&friend).await;
    let cheer = CheerStudyRequest { user_id: friend.id.to_string(), ..Default::default() };
    one_ok(&burst::<_, CheerStudyResponse>(&ctx, STUDY, "CheerStudy", &me, &cheer).await, "RATE_LIMITED");
    assert_eq!(ctx.heartbeat(&friend).await.cheer_count, 1);

    // Twelve invitations of the same friend at once: invited once, notified once.
    let room = ctx.create_room(&me, |_| {}).await.room.as_option().cloned().unwrap();
    let invite = InviteToStudyRoomRequest {
        room_id: room.id.clone(),
        user_ids: vec![friend.id.to_string()],
        ..Default::default()
    };
    all_ok(&burst::<_, InviteToStudyRoomResponse>(&ctx, ROOMS, "InviteToStudyRoom", &me, &invite).await);
    assert_eq!(ctx.room(&me, &room.id).await.unwrap().invitees.len(), 1);
    assert_eq!(ctx.notifications_for(friend.id, "school.room_invite").await.len(), 1);

    // Withdrawing and inviting again, over and over, rings a few times at most.
    let uninvite =
        UninviteFromStudyRoomRequest { room_id: room.id.clone(), user_id: friend.id.to_string(), ..Default::default() };
    for _ in 0..6 {
        let _: UninviteFromStudyRoomResponse = ctx.ok(ROOMS, "UninviteFromStudyRoom", &me, &uninvite).await;
        let _: InviteToStudyRoomResponse = ctx.ok(ROOMS, "InviteToStudyRoom", &me, &invite).await;
    }
    assert_eq!(ctx.notifications_for(friend.id, "school.room_invite").await.len(), 3, "three an hour per friend");
    assert!(ctx.room(&friend, &room.id).await.unwrap().invited, "the invitation itself stands");
}
