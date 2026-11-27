//! `StudyService`: the focus timer, the study log, totals, the goal and
//! streak, the heatmap, who is studying, the ranking and cheers.
mod common;

use common::*;
use tokuly_proto::{common::PageRequest, school::*};
use tokuly_school::SchoolConfig;
use tokuly_testkit::TestUser;

async fn current(ctx: &Ctx, user: &TestUser) -> Option<StudySession> {
    let response: GetCurrentStudySessionResponse =
        ctx.ok(STUDY, "GetCurrentStudySession", user, &GetCurrentStudySessionRequest::default()).await;
    response.session.as_option().cloned()
}

async fn summary(ctx: &Ctx, user: &TestUser, period: StatsPeriod, anchor: &str) -> GetStudySummaryResponse {
    ctx.ok(
        STUDY,
        "GetStudySummary",
        user,
        &GetStudySummaryRequest { period: period.into(), anchor_date: anchor.into(), ..Default::default() },
    )
    .await
}

async fn progress(ctx: &Ctx, user: &TestUser) -> StudyProgress {
    let response: GetStudyProgressResponse =
        ctx.ok(STUDY, "GetStudyProgress", user, &GetStudyProgressRequest::default()).await;
    response.progress.as_option().cloned().unwrap()
}

async fn studying_friends(ctx: &Ctx, user: &TestUser) -> Vec<StudyingFriend> {
    let response: ListStudyingFriendsResponse =
        ctx.ok(STUDY, "ListStudyingFriends", user, &ListStudyingFriendsRequest::default()).await;
    response.friends
}

/// A page of the log: the study time of each session, and the page info.
async fn list(
    ctx: &Ctx,
    user: &TestUser,
    request: ListStudySessionsRequest,
) -> (Vec<i32>, tokuly_proto::common::PageInfo) {
    let response: ListStudySessionsResponse = ctx.ok(STUDY, "ListStudySessions", user, &request).await;
    (response.sessions.iter().map(|s| s.studied_seconds).collect(), response.page.as_option().cloned().unwrap())
}

fn timer(session: &StudySession) -> &TimerSnapshot {
    session.timer.as_option().expect("an active session has a timer")
}

#[tokio::test]
async fn a_stopwatch_counts_what_the_server_saw() {
    let ctx = start().await;
    let me = ctx.app.user("me").await;
    let math = ctx.subject(&me, "数学").await;
    ctx.at(2026, 4, 13, 18, 0);
    let mut devices = ctx.app.listen_user(me.id).await;

    let request = StartStudySessionRequest {
        mode: StudyMode::STUDY_MODE_STOPWATCH.into(),
        subject_id: math.id.clone(),
        memo: "p.42".into(),
        ..Default::default()
    };
    let started: StartStudySessionResponse = ctx.ok(STUDY, "StartStudySession", &me, &request).await;
    assert_eq!((started.heartbeat_interval_seconds, started.idle_timeout_seconds), (60, 900));
    let session = started.session.as_option().cloned().unwrap();
    assert_eq!(session.state, SessionState::SESSION_STATE_RUNNING);
    assert_eq!(session.source, SessionSource::SESSION_SOURCE_TIMER);
    assert_eq!(
        (session.studied_seconds, session.subject.as_option().unwrap().name.as_str(), session.memo.as_str()),
        (0, "数学", "p.42")
    );
    assert_eq!(instant(&session.started_at), tokyo(2026, 4, 13, 18, 0));
    assert_eq!(
        (timer(&session).mode.as_known().unwrap(), timer(&session).state.as_known().unwrap()),
        (StudyMode::STUDY_MODE_STOPWATCH, TimerState::TIMER_STATE_RUNNING)
    );
    assert_eq!(
        instant(&timer(&session).server_time),
        tokyo(2026, 4, 13, 18, 0),
        "the timer carries the server's clock"
    );
    assert_eq!(ctx.reason(STUDY, "StartStudySession", &me, &request).await, "SESSION_ALREADY_ACTIVE");

    ctx.pass(10);
    assert_eq!(ctx.heartbeat(&me).await.studied_seconds, 600);
    // Without a heartbeat the timer still reads the right time...
    ctx.pass(1);
    assert_eq!(current(&ctx, &me).await.unwrap().studied_seconds, 660);

    // ...and pausing counts up to the pause.
    let paused: PauseStudySessionResponse =
        ctx.ok(STUDY, "PauseStudySession", &me, &PauseStudySessionRequest::default()).await;
    let paused = paused.session.as_option().cloned().unwrap();
    assert_eq!(
        (paused.state.as_known().unwrap(), paused.studied_seconds, timer(&paused).state.as_known().unwrap()),
        (SessionState::SESSION_STATE_PAUSED, 660, TimerState::TIMER_STATE_PAUSED)
    );
    assert_eq!(
        ctx.reason(STUDY, "PauseStudySession", &me, &PauseStudySessionRequest::default()).await,
        "SESSION_NOT_RUNNING"
    );
    assert_eq!(ctx.reason(STUDY, "SkipStudyPhase", &me, &SkipStudyPhaseRequest::default()).await, "NOT_POMODORO");
    ctx.pass(30);
    assert_eq!(ctx.heartbeat(&me).await.studied_seconds, 660, "a pause counts for nothing");
    let resumed: ResumeStudySessionResponse =
        ctx.ok(STUDY, "ResumeStudySession", &me, &ResumeStudySessionRequest::default()).await;
    assert_eq!(resumed.session.as_option().unwrap().state, SessionState::SESSION_STATE_RUNNING);
    assert_eq!(
        ctx.reason(STUDY, "ResumeStudySession", &me, &ResumeStudySessionRequest::default()).await,
        "SESSION_NOT_PAUSED"
    );

    ctx.pass(5);
    let stopped: StopStudySessionResponse = ctx
        .ok(
            STUDY,
            "StopStudySession",
            &me,
            &StopStudySessionRequest { memo: Some("p.42-58".into()), ..Default::default() },
        )
        .await;
    let ended = stopped.session.as_option().cloned().unwrap();
    assert_eq!(
        (ended.state.as_known().unwrap(), ended.studied_seconds, ended.memo.as_str(), ended.auto_stopped),
        (SessionState::SESSION_STATE_ENDED, 960, "p.42-58", false)
    );
    assert_eq!(instant(&ended.ended_at), tokyo(2026, 4, 13, 18, 46));
    assert!(ended.timer.as_option().is_none());
    assert_eq!(stopped.progress.as_option().unwrap().today_seconds, 960);

    // Every change reached the user's other devices.
    let sessions: Vec<SessionState> = heard(&mut devices)
        .await
        .into_iter()
        .filter_map(|push| match push {
            school_event::Event::Session(session) => session.state.as_known(),
            _ => None,
        })
        .collect();
    use SessionState::*;
    assert_eq!(sessions, vec![SESSION_STATE_RUNNING, SESSION_STATE_PAUSED, SESSION_STATE_RUNNING, SESSION_STATE_ENDED]);

    // The log entry, the event for other domains, the friends' activity entry.
    assert_eq!(
        ctx.sessions(&me).await.iter().map(|s| (s.id.clone(), s.studied_seconds)).collect::<Vec<_>>(),
        vec![(ended.id.clone(), 960)]
    );
    let events = ctx.app.events_by("school.study.session_ended", me.id).await;
    assert_eq!(events.len(), 1);
    let data = &events[0].data;
    assert_eq!(
        (data["minutes"].as_i64(), data["seconds"].as_i64(), data["subject"].as_str()),
        (Some(16), Some(960), Some("数学"))
    );
    assert_eq!(
        (data["manual"].as_bool(), data["auto_stopped"].as_bool(), data["session_id"].as_str()),
        (Some(false), Some(false), Some(ended.id.as_str()))
    );
    let activity = ctx.activities_of(me.id, "school.studied").await;
    assert_eq!(activity.len(), 1);
    assert_eq!(
        (activity[0].data["audience"].as_str(), activity[0].data["summary"]["ja"].as_str()),
        (Some("friends"), Some("数学を16分勉強しました"))
    );

    // Product analytics got the session too.
    assert!(ctx.app.flush_analytics().await >= 1);

    // Nothing is going anymore.
    assert!(current(&ctx, &me).await.is_none());
    for method in
        ["StopStudySession", "PauseStudySession", "ResumeStudySession", "HeartbeatStudySession", "SkipStudyPhase"]
    {
        assert_eq!(ctx.reason(STUDY, method, &me, &serde_json::json!({})).await, "NO_ACTIVE_SESSION", "{method}");
    }
}

#[tokio::test]
async fn a_pomodoro_counts_work_and_not_breaks() {
    let ctx = start().await;
    let (me, other) = (ctx.app.user("me").await, ctx.app.user("other").await);
    ctx.at(2026, 4, 13, 18, 0);
    let classic =
        PomodoroConfig { work_minutes: 25, break_minutes: 5, long_break_minutes: 15, rounds: 4, ..Default::default() };
    let request = StartStudySessionRequest {
        mode: StudyMode::STUDY_MODE_POMODORO.into(),
        pomodoro: classic.into(),
        ..Default::default()
    };
    let started: StartStudySessionResponse = ctx.ok(STUDY, "StartStudySession", &me, &request).await;
    let session = started.session.as_option().cloned().unwrap();
    let t = timer(&session);
    assert_eq!(
        (t.phase.as_known().unwrap(), t.round, t.phase_remaining_seconds),
        (TimerPhase::TIMER_PHASE_WORK, 1, 1500)
    );
    assert_eq!(instant(&t.phase_ends_at), tokyo(2026, 4, 13, 18, 25));

    ctx.pass(10);
    ctx.heartbeat(&me).await;
    ctx.pass(10);
    assert_eq!(ctx.heartbeat(&me).await.studied_seconds, 1200);
    // Two minutes into the first break: the work phase is counted in full, the break not at all.
    ctx.pass(7);
    let resting = ctx.heartbeat(&me).await;
    assert_eq!(resting.studied_seconds, 1500);
    let t = timer(&resting);
    assert_eq!(
        (t.phase.as_known().unwrap(), t.round, t.phase_remaining_seconds),
        (TimerPhase::TIMER_PHASE_BREAK, 2, 180)
    );
    assert_eq!(
        (instant(&t.phase_started_at), instant(&t.phase_ends_at)),
        (tokyo(2026, 4, 13, 18, 25), tokyo(2026, 4, 13, 18, 30))
    );

    // Skipping the rest of the break starts the second round now.
    let skipped: SkipStudyPhaseResponse = ctx.ok(STUDY, "SkipStudyPhase", &me, &SkipStudyPhaseRequest::default()).await;
    let t = timer(skipped.session.as_option().unwrap()).clone();
    assert_eq!((t.phase.as_known().unwrap(), t.round), (TimerPhase::TIMER_PHASE_WORK, 2));
    assert_eq!(instant(&t.phase_ends_at), tokyo(2026, 4, 13, 18, 52));
    ctx.pass(3);
    assert_eq!(ctx.stop(&me).await.session.as_option().unwrap().studied_seconds, 1500 + 180);

    // Lengths left out come from the user's settings.
    ctx.update_settings(
        &other,
        UpdateSchoolSettingsRequest {
            pomodoro: PomodoroConfig { work_minutes: 50, break_minutes: 10, ..Default::default() }.into(),
            ..Default::default()
        },
    )
    .await;
    let request = StartStudySessionRequest { mode: StudyMode::STUDY_MODE_POMODORO.into(), ..Default::default() };
    let started: StartStudySessionResponse = ctx.ok(STUDY, "StartStudySession", &other, &request).await;
    let config = timer(started.session.as_option().unwrap()).pomodoro.as_option().cloned().unwrap();
    assert_eq!((config.work_minutes, config.break_minutes, config.long_break_minutes, config.rounds), (50, 10, 15, 4));
    ctx.stop(&other).await;

    let start = |fill: fn(&mut StartStudySessionRequest)| {
        let mut request =
            StartStudySessionRequest { mode: StudyMode::STUDY_MODE_POMODORO.into(), ..Default::default() };
        fill(&mut request);
        request
    };
    for (request, reason) in [
        (start(|r| r.mode = StudyMode::STUDY_MODE_UNSPECIFIED.into()), "STUDY_MODE_REQUIRED"),
        (start(|r| r.pomodoro = PomodoroConfig { work_minutes: 500, ..Default::default() }.into()), "POMODORO_INVALID"),
        (start(|r| r.pomodoro = PomodoroConfig { rounds: 99, ..Default::default() }.into()), "POMODORO_INVALID"),
        (start(|r| r.subject_id = uuid::Uuid::now_v7().to_string()), "SUBJECT_NOT_FOUND"),
        (start(|r| r.subject_id = "math".into()), "INVALID_ID"),
        (start(|r| r.memo = "x".repeat(501)), "TEXT_TOO_LONG"),
    ] {
        assert_eq!(ctx.reason(STUDY, "StartStudySession", &other, &request).await, reason, "{request:?}");
    }
}

#[tokio::test]
async fn a_silent_client_keeps_only_the_time_up_to_its_last_heartbeat() {
    let ctx = start().await;
    let me = ctx.app.user("me").await;
    ctx.at(2026, 4, 13, 18, 0);
    ctx.start_stopwatch(&me).await;
    ctx.pass(5);
    assert_eq!(ctx.heartbeat(&me).await.studied_seconds, 300);

    // Half an hour of silence (the idle limit is 15 minutes): the session is
    // over, and counted until the last heartbeat only.
    ctx.pass(30);
    assert!(current(&ctx, &me).await.is_none(), "a session nobody is behind is not shown as going");
    let ended = ctx.heartbeat(&me).await;
    assert_eq!(
        (ended.state.as_known().unwrap(), ended.auto_stopped, ended.studied_seconds),
        (SessionState::SESSION_STATE_ENDED, true, 300)
    );
    assert_eq!(instant(&ended.ended_at), tokyo(2026, 4, 13, 18, 5));
    assert_eq!(
        ctx.reason(STUDY, "HeartbeatStudySession", &me, &HeartbeatStudySessionRequest::default()).await,
        "NO_ACTIVE_SESSION"
    );
    let logged = ctx.sessions(&me).await;
    assert_eq!(logged.iter().map(|s| (s.studied_seconds, s.auto_stopped)).collect::<Vec<_>>(), vec![(300, true)]);
    let events = ctx.app.events_by("school.study.session_ended", me.id).await;
    assert_eq!(
        (events.len(), events[0].data["auto_stopped"].as_bool(), events[0].data["minutes"].as_i64()),
        (1, Some(true), Some(5))
    );

    // A session that never counted anything leaves no trace, and does not
    // stand in the way of a new one.
    ctx.start_stopwatch(&me).await;
    ctx.pass(40);
    let fresh = ctx.start_stopwatch(&me).await;
    assert_eq!(ctx.sessions(&me).await.len(), 1);
    assert_eq!(ctx.app.events_by("school.study.session_ended", me.id).await.len(), 1);

    // A pause that never ends: the session ends where it was paused.
    ctx.pass(2);
    ctx.heartbeat(&me).await;
    let _: PauseStudySessionResponse =
        ctx.ok(STUDY, "PauseStudySession", &me, &PauseStudySessionRequest::default()).await;
    ctx.pass(181);
    let ended = ctx.heartbeat(&me).await;
    assert_eq!(
        (ended.id.as_str(), ended.state.as_known().unwrap(), ended.auto_stopped, ended.studied_seconds),
        (fresh.id.as_str(), SessionState::SESSION_STATE_ENDED, true, 120)
    );
    assert_eq!(instant(&ended.ended_at), tokyo(2026, 4, 13, 19, 17));
}

#[tokio::test]
async fn a_silent_pomodoro_is_believed_until_its_phase_ends() {
    let ctx = start().await;
    let (me, friend) = (ctx.app.user("me").await, ctx.app.user("friend").await);
    ctx.befriend(&me, &friend).await;
    ctx.at(2026, 4, 13, 18, 0);
    let request = StartStudySessionRequest {
        mode: StudyMode::STUDY_MODE_POMODORO.into(),
        pomodoro: PomodoroConfig {
            work_minutes: 25,
            break_minutes: 5,
            long_break_minutes: 15,
            rounds: 4,
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };
    let _: StartStudySessionResponse = ctx.ok(STUDY, "StartStudySession", &me, &request).await;
    ctx.pass(2);
    ctx.heartbeat(&me).await;

    // The phone is locked from 18:02. The work phase ends at 18:25, so the
    // session stays going past the 15 minutes a stopwatch would get...
    ctx.pass(18);
    assert!(current(&ctx, &me).await.is_some(), "18:20, inside the phase");
    assert_eq!(studying_friends(&ctx, &friend).await.len(), 1, "and friends still see it");
    ctx.pass(19);
    assert!(current(&ctx, &me).await.is_some(), "18:39, within the grace after the phase");

    // ...until the phase end plus the usual grace, and it is counted up to
    // the end of that phase: not the break, not the next round.
    ctx.pass(2);
    assert!(current(&ctx, &me).await.is_none(), "18:41, past the grace");
    assert!(studying_friends(&ctx, &friend).await.is_empty());
    let ended = ctx.heartbeat(&me).await;
    assert_eq!(
        (ended.state.as_known().unwrap(), ended.auto_stopped, ended.studied_seconds),
        (SessionState::SESSION_STATE_ENDED, true, 1500)
    );
    assert_eq!(instant(&ended.ended_at), tokyo(2026, 4, 13, 18, 25));
}

#[tokio::test]
async fn a_session_ends_when_it_has_lasted_as_long_as_one_may() {
    let ctx = start_with(SchoolConfig { session_max_seconds: 1800, ..Default::default() }).await;
    let me = ctx.app.user("me").await;
    ctx.at(2026, 4, 13, 18, 0);
    ctx.start_stopwatch(&me).await;
    for _ in 0..2 {
        ctx.pass(10);
        ctx.heartbeat(&me).await;
    }
    ctx.pass(10);
    let ended = ctx.heartbeat(&me).await;
    assert_eq!(
        (ended.state.as_known().unwrap(), ended.auto_stopped, ended.studied_seconds),
        (SessionState::SESSION_STATE_ENDED, true, 1200)
    );
}

#[tokio::test]
async fn study_time_can_be_logged_corrected_and_deleted() {
    let ctx = start().await;
    let (me, other) = (ctx.app.user("me").await, ctx.app.user("other").await);
    let (math, english) = (ctx.subject(&me, "数学").await, ctx.subject(&me, "英語").await);
    ctx.at(2026, 4, 15, 20, 0);
    let at = |hour: u32, minute: u32| tokyo(2026, 4, 15, hour, minute);

    let logged = ctx.log(&me, at(16, 0), at(17, 30), Some(&math)).await.unwrap();
    assert_eq!(
        (logged.source.as_known().unwrap(), logged.state.as_known().unwrap(), logged.studied_seconds),
        (SessionSource::SESSION_SOURCE_MANUAL, SessionState::SESSION_STATE_ENDED, 5400)
    );
    let events = ctx.app.events_by("school.study.session_ended", me.id).await;
    assert_eq!(
        (events.len(), events[0].data["manual"].as_bool(), events[0].data["minutes"].as_i64()),
        (1, Some(true), Some(90))
    );
    assert!(
        ctx.activities_of(me.id, "school.studied").await.is_empty(),
        "time logged afterwards is not news for friends"
    );

    for (from, to) in [
        (at(19, 0), at(21, 0)),                                 // not over yet
        (at(19, 0), at(19, 0)),                                 // empty
        (at(19, 0), at(18, 0)),                                 // backwards
        (tokyo(2026, 4, 14, 10, 0), tokyo(2026, 4, 15, 11, 0)), // longer than a day
        (tokyo(2023, 4, 14, 10, 0), tokyo(2023, 4, 14, 11, 0)), // long ago
    ] {
        assert_eq!(ctx.log(&me, from, to, None).await.unwrap_err().reason, "TIME_RANGE_INVALID");
    }
    assert_eq!(
        ctx.reason(STUDY, "LogStudySession", &me, &LogStudySessionRequest::default()).await,
        "TIME_RANGE_INVALID"
    );
    // Time already logged cannot be logged twice; time next to it can.
    assert_eq!(ctx.log(&me, at(17, 0), at(18, 0), None).await.unwrap_err().reason, "SESSION_OVERLAPS");
    assert_eq!(ctx.log(&me, at(15, 0), at(18, 0), None).await.unwrap_err().reason, "SESSION_OVERLAPS");
    let next = ctx.log(&me, at(17, 30), at(18, 0), Some(&english)).await.unwrap();
    let foreign = LogStudySessionRequest {
        started_at: stamp(at(10, 0)),
        ended_at: stamp(at(11, 0)),
        subject_id: ctx.subject(&other, "他人").await.id,
        ..Default::default()
    };
    assert_eq!(ctx.reason(STUDY, "LogStudySession", &me, &foreign).await, "SUBJECT_NOT_FOUND");

    // Correcting a session: its subject, memo and times.
    let update =
        |session: &StudySession| UpdateStudySessionRequest { session_id: session.id.clone(), ..Default::default() };
    let relabelled: UpdateStudySessionResponse = ctx
        .ok(
            STUDY,
            "UpdateStudySession",
            &me,
            &UpdateStudySessionRequest { subject_id: Some("".into()), memo: Some("復習".into()), ..update(&logged) },
        )
        .await;
    let relabelled = relabelled.session.as_option().cloned().unwrap();
    assert!(relabelled.subject.as_option().is_none());
    assert_eq!((relabelled.memo.as_str(), relabelled.studied_seconds), ("復習", 5400));
    let moved: UpdateStudySessionResponse = ctx
        .ok(
            STUDY,
            "UpdateStudySession",
            &me,
            &UpdateStudySessionRequest {
                subject_id: Some(math.id.clone()),
                started_at: stamp(at(16, 10)),
                ended_at: stamp(at(17, 20)),
                ..update(&logged)
            },
        )
        .await;
    let moved = moved.session.as_option().cloned().unwrap();
    assert_eq!(
        (moved.studied_seconds, instant(&moved.started_at), instant(&moved.ended_at)),
        (4200, at(16, 10), at(17, 20))
    );
    assert_eq!(moved.subject.as_option().unwrap().name, "数学");
    for (request, reason) in [
        (UpdateStudySessionRequest { started_at: stamp(at(16, 0)), ..update(&logged) }, "TIME_RANGE_INVALID"),
        (
            UpdateStudySessionRequest { started_at: stamp(at(16, 0)), ended_at: stamp(at(17, 45)), ..update(&logged) },
            "SESSION_OVERLAPS",
        ),
        (
            UpdateStudySessionRequest { started_at: stamp(at(16, 0)), ended_at: stamp(at(23, 0)), ..update(&logged) },
            "TIME_RANGE_INVALID",
        ),
        (
            UpdateStudySessionRequest { subject_id: Some(uuid::Uuid::now_v7().to_string()), ..update(&logged) },
            "SUBJECT_NOT_FOUND",
        ),
        (UpdateStudySessionRequest { memo: Some("x".repeat(501)), ..update(&logged) }, "TEXT_TOO_LONG"),
    ] {
        assert_eq!(ctx.reason(STUDY, "UpdateStudySession", &me, &request).await, reason, "{request:?}");
    }
    // Someone else's session does not exist.
    assert_eq!(
        ctx.reason(
            STUDY,
            "UpdateStudySession",
            &other,
            &UpdateStudySessionRequest { memo: Some("x".into()), ..update(&logged) }
        )
        .await,
        "SESSION_NOT_FOUND"
    );
    let delete =
        |session: &StudySession| DeleteStudySessionRequest { session_id: session.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(STUDY, "DeleteStudySession", &other, &delete(&logged)).await, "SESSION_NOT_FOUND");

    // A session that is still going: no time may be logged over it, its
    // times cannot be edited, and it cannot be deleted — its memo can change.
    let going = ctx.start_stopwatch(&me).await;
    ctx.pass(10);
    assert_eq!(ctx.log(&me, at(20, 0), at(20, 5), None).await.unwrap_err().reason, "SESSION_OVERLAPS");
    assert!(ctx.log(&me, at(19, 0), at(19, 30), None).await.is_ok(), "before it started is fine");
    assert_eq!(
        ctx.reason(
            STUDY,
            "UpdateStudySession",
            &me,
            &UpdateStudySessionRequest { started_at: stamp(at(18, 0)), ended_at: stamp(at(18, 30)), ..update(&going) }
        )
        .await,
        "SESSION_ACTIVE"
    );
    assert_eq!(ctx.reason(STUDY, "DeleteStudySession", &me, &delete(&going)).await, "SESSION_ACTIVE");
    let _: UpdateStudySessionResponse = ctx
        .ok(
            STUDY,
            "UpdateStudySession",
            &me,
            &UpdateStudySessionRequest {
                memo: Some("集中".into()),
                subject_id: Some(english.id.clone()),
                ..update(&going)
            },
        )
        .await;
    let stopped = ctx.stop(&me).await.session.as_option().cloned().unwrap();
    assert_eq!(
        (stopped.memo.as_str(), stopped.subject.as_option().unwrap().name.as_str(), stopped.studied_seconds),
        ("集中", "英語", 600)
    );

    // The log, newest first, in pages and filtered.
    let (all, _) = list(&ctx, &me, ListStudySessionsRequest::default()).await;
    assert_eq!(all, vec![600, 1800, 1800, 4200], "the stopwatch, 19:00, 17:30, 16:10");
    let (first, page) = list(
        &ctx,
        &me,
        ListStudySessionsRequest { page: PageRequest { limit: 3, ..Default::default() }.into(), ..Default::default() },
    )
    .await;
    assert_eq!((first, page.has_more), (vec![600, 1800, 1800], true));
    let (rest, page) = list(
        &ctx,
        &me,
        ListStudySessionsRequest {
            page: PageRequest { limit: 3, cursor: page.next_cursor, ..Default::default() }.into(),
            ..Default::default()
        },
    )
    .await;
    assert_eq!((rest, page.has_more), (vec![4200], false));
    let (of_english, _) =
        list(&ctx, &me, ListStudySessionsRequest { subject_id: english.id.clone(), ..Default::default() }).await;
    assert_eq!(of_english, vec![600, 1800]);
    let (yesterday, _) =
        list(&ctx, &me, ListStudySessionsRequest { to_date: "2026-04-14".into(), ..Default::default() }).await;
    assert!(yesterday.is_empty());
    let (today, _) = list(
        &ctx,
        &me,
        ListStudySessionsRequest { from_date: "2026-04-15".into(), to_date: "2026-04-15".into(), ..Default::default() },
    )
    .await;
    assert_eq!(today.len(), 4);

    let _: DeleteStudySessionResponse = ctx.ok(STUDY, "DeleteStudySession", &me, &delete(&next)).await;
    assert_eq!(ctx.reason(STUDY, "DeleteStudySession", &me, &delete(&next)).await, "SESSION_NOT_FOUND");
    assert_eq!(ctx.sessions(&me).await.len(), 3);
}

#[tokio::test]
async fn totals_are_cut_at_the_local_midnight() {
    let ctx = start().await;
    let (me, new_yorker) = (ctx.app.user("me").await, ctx.app.user("newyorker").await);
    ctx.set_time_zone(&new_yorker, "America/New_York").await;
    let (math, english) = (ctx.subject(&me, "数学").await, ctx.subject(&me, "英語").await);
    ctx.at(2026, 4, 10, 12, 0); // a Friday

    // An hour across midnight between Wednesday the 8th and Thursday the 9th.
    let (late, early) = (tokyo(2026, 4, 8, 23, 30), tokyo(2026, 4, 9, 0, 30));
    ctx.log(&me, late, early, Some(&math)).await.unwrap();
    ctx.log(&me, tokyo(2026, 4, 9, 10, 0), tokyo(2026, 4, 9, 11, 0), Some(&english)).await.unwrap();
    ctx.log(&me, tokyo(2026, 4, 3, 10, 0), tokyo(2026, 4, 3, 10, 30), None).await.unwrap();

    let subjects = |summary: &GetStudySummaryResponse| -> Vec<(String, i32)> {
        summary
            .subjects
            .iter()
            .map(|total| (total.subject.as_option().map(|s| s.name.clone()).unwrap_or_default(), total.seconds))
            .collect()
    };
    let wednesday = summary(&ctx, &me, StatsPeriod::STATS_PERIOD_DAY, "2026-04-08").await;
    assert_eq!(
        (wednesday.total_seconds, wednesday.previous_total_seconds),
        (1800, 0),
        "half of the hour belongs to Wednesday"
    );
    assert_eq!(
        wednesday.days.iter().map(|d| (d.date.as_str(), d.seconds)).collect::<Vec<_>>(),
        vec![("2026-04-08", 1800)]
    );
    assert_eq!(subjects(&wednesday), vec![("数学".to_owned(), 1800)]);
    let thursday = summary(&ctx, &me, StatsPeriod::STATS_PERIOD_DAY, "2026-04-09").await;
    assert_eq!(
        (thursday.total_seconds, thursday.previous_total_seconds),
        (5400, 1800),
        "the other half belongs to Thursday"
    );
    assert_eq!(subjects(&thursday), vec![("英語".to_owned(), 3600), ("数学".to_owned(), 1800)]);

    let week = summary(&ctx, &me, StatsPeriod::STATS_PERIOD_WEEK, "2026-04-09").await;
    assert_eq!((week.start_date.as_str(), week.end_date.as_str()), ("2026-04-06", "2026-04-12"));
    assert_eq!((week.total_seconds, week.previous_total_seconds), (7200, 1800));
    assert_eq!(
        week.days.iter().map(|d| d.seconds).collect::<Vec<_>>(),
        vec![0, 0, 1800, 5400, 0, 0, 0],
        "every day of the week, studied or not"
    );
    let month = summary(&ctx, &me, StatsPeriod::STATS_PERIOD_MONTH, "").await;
    assert_eq!(
        (month.start_date.as_str(), month.end_date.as_str(), month.days.len()),
        ("2026-04-01", "2026-04-30", 30)
    );
    assert_eq!((month.total_seconds, month.previous_total_seconds), (9000, 0));
    let mut by_subject = subjects(&month);
    by_subject.sort();
    assert_eq!(by_subject, vec![(String::new(), 1800), ("数学".to_owned(), 3600), ("英語".to_owned(), 3600)]);
    assert_eq!(
        ctx.reason(STUDY, "GetStudySummary", &me, &GetStudySummaryRequest::default()).await,
        "STATS_PERIOD_REQUIRED"
    );
    assert_eq!(
        ctx.reason(
            STUDY,
            "GetStudySummary",
            &me,
            &GetStudySummaryRequest {
                period: StatsPeriod::STATS_PERIOD_DAY.into(),
                anchor_date: "today".into(),
                ..Default::default()
            }
        )
        .await,
        "INVALID_DATE"
    );

    // The same hour is the middle of Wednesday morning in New York: one day.
    ctx.log(&new_yorker, late, early, None).await.unwrap();
    let theirs = summary(&ctx, &new_yorker, StatsPeriod::STATS_PERIOD_DAY, "2026-04-08").await;
    assert_eq!(theirs.total_seconds, 3600);
    assert_eq!(summary(&ctx, &new_yorker, StatsPeriod::STATS_PERIOD_DAY, "2026-04-09").await.total_seconds, 0);

    // The heatmap: one entry per day with study time.
    let heatmap: GetStudyHeatmapResponse =
        ctx.ok(STUDY, "GetStudyHeatmap", &me, &GetStudyHeatmapRequest::default()).await;
    assert_eq!(
        (heatmap.start_date.as_str(), heatmap.end_date.as_str()),
        ("2025-04-11", "2026-04-10"),
        "a year ending today"
    );
    assert_eq!(
        heatmap.days.iter().map(|d| (d.date.as_str(), d.seconds)).collect::<Vec<_>>(),
        vec![("2026-04-03", 1800), ("2026-04-08", 1800), ("2026-04-09", 5400)]
    );
    assert_eq!((heatmap.max_seconds, heatmap.total_seconds), (5400, 9000));
    let short: GetStudyHeatmapResponse = ctx
        .ok(
            STUDY,
            "GetStudyHeatmap",
            &me,
            &GetStudyHeatmapRequest { end_date: "2026-04-08".into(), days: 7, ..Default::default() },
        )
        .await;
    assert_eq!((short.start_date.as_str(), short.days.len(), short.total_seconds), ("2026-04-02", 2, 3600));
    assert_eq!(
        ctx.reason(STUDY, "GetStudyHeatmap", &me, &GetStudyHeatmapRequest { days: 372, ..Default::default() }).await,
        "DATE_RANGE_INVALID"
    );
}

#[tokio::test]
async fn a_running_timer_across_midnight_counts_towards_both_days() {
    let ctx = start().await;
    let me = ctx.app.user("me").await;
    ctx.at(2026, 4, 8, 23, 40);
    ctx.start_stopwatch(&me).await;
    for _ in 0..4 {
        ctx.pass(10);
        ctx.heartbeat(&me).await;
    }
    // 23:40 to 00:20: twenty minutes on each side of midnight, while it runs.
    assert_eq!(summary(&ctx, &me, StatsPeriod::STATS_PERIOD_DAY, "2026-04-08").await.total_seconds, 1200);
    assert_eq!(summary(&ctx, &me, StatsPeriod::STATS_PERIOD_DAY, "2026-04-09").await.total_seconds, 1200);
    let stopped = ctx.stop(&me).await;
    assert_eq!(stopped.session.as_option().unwrap().studied_seconds, 2400);
    assert_eq!(stopped.progress.as_option().unwrap().today_seconds, 1200, "today is the 9th");
}

#[tokio::test]
async fn the_daily_goal_makes_a_streak() {
    let ctx = start().await;
    let (me, casual) = (ctx.app.user("me").await, ctx.app.user("casual").await);
    ctx.at(2026, 4, 10, 21, 0);
    let settings = ctx
        .update_settings(&me, UpdateSchoolSettingsRequest { daily_goal_minutes: Some(30), ..Default::default() })
        .await;
    assert_eq!(settings.daily_goal_minutes, 30);

    // Wednesday: 40 minutes. Thursday: 20, then 15 more. Friday (today): exactly 30.
    ctx.log(&me, tokyo(2026, 4, 8, 18, 0), tokyo(2026, 4, 8, 18, 40), None).await.unwrap();
    ctx.log(&me, tokyo(2026, 4, 9, 18, 0), tokyo(2026, 4, 9, 18, 20), None).await.unwrap();
    ctx.log(&me, tokyo(2026, 4, 9, 19, 0), tokyo(2026, 4, 9, 19, 15), None).await.unwrap();
    ctx.log(&me, tokyo(2026, 4, 10, 18, 0), tokyo(2026, 4, 10, 18, 30), None).await.unwrap();

    let today = progress(&ctx, &me).await;
    assert_eq!(
        (today.date.as_str(), today.today_seconds, today.goal_minutes, today.goal_reached),
        ("2026-04-10", 1800, 30, true)
    );
    assert_eq!(
        (today.streak_days, today.longest_streak_days, today.week_seconds, today.total_seconds),
        (3, 3, 6300, 6300)
    );

    // The goal was reached once a day, by the session that crossed it.
    let reached: Vec<(String, i64, i64)> = ctx
        .app
        .events_by("school.study.goal_reached", me.id)
        .await
        .iter()
        .map(|e| {
            (
                e.data["date"].as_str().unwrap().to_owned(),
                e.data["minutes"].as_i64().unwrap(),
                e.data["streak_days"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        reached,
        vec![("2026-04-08".to_owned(), 40, 1), ("2026-04-09".to_owned(), 35, 2), ("2026-04-10".to_owned(), 30, 3)]
    );

    // Tomorrow morning the streak still stands; a day without study breaks it.
    ctx.at(2026, 4, 11, 9, 0);
    let tomorrow = progress(&ctx, &me).await;
    assert_eq!((tomorrow.today_seconds, tomorrow.goal_reached, tomorrow.streak_days), (0, false, 3));
    ctx.at(2026, 4, 12, 9, 0);
    let later = progress(&ctx, &me).await;
    assert_eq!(
        (later.streak_days, later.longest_streak_days, later.week_seconds),
        (0, 3, 6300),
        "Sunday is still the same week"
    );

    // Without a goal, any study keeps the streak going.
    ctx.log(&casual, tokyo(2026, 4, 11, 10, 0), tokyo(2026, 4, 11, 10, 5), None).await.unwrap();
    ctx.log(&casual, tokyo(2026, 4, 12, 8, 0), tokyo(2026, 4, 12, 8, 2), None).await.unwrap();
    let theirs = progress(&ctx, &casual).await;
    assert_eq!((theirs.streak_days, theirs.goal_minutes, theirs.goal_reached), (2, 0, false));
    assert!(ctx.app.events_by("school.study.goal_reached", casual.id).await.is_empty());
}

#[tokio::test]
async fn friends_see_who_is_studying_unless_it_is_hidden() {
    let ctx = start().await;
    let (me, friend, stranger) =
        (ctx.app.user("me").await, ctx.app.user("friend").await, ctx.app.user("stranger").await);
    ctx.befriend(&me, &friend).await;
    let math = ctx.subject(&me, "数学").await;
    ctx.at(2026, 4, 13, 18, 0);
    let mut friend_rt = ctx.app.listen_user(friend.id).await;
    let mut stranger_rt = ctx.app.listen_user(stranger.id).await;
    let studying = |pushes: Vec<school_event::Event>| -> Vec<(bool, String)> {
        pushes
            .into_iter()
            .filter_map(|push| match push {
                school_event::Event::FriendStudying(event) => Some((
                    event.studying,
                    event
                        .friend
                        .as_option()
                        .and_then(|f| f.subject.as_option().map(|s| s.name.clone()))
                        .unwrap_or_default(),
                )),
                _ => None,
            })
            .collect()
    };

    let request = StartStudySessionRequest {
        mode: StudyMode::STUDY_MODE_STOPWATCH.into(),
        subject_id: math.id.clone(),
        ..Default::default()
    };
    let _: StartStudySessionResponse = ctx.ok(STUDY, "StartStudySession", &me, &request).await;
    assert_eq!(studying(heard(&mut friend_rt).await), vec![(true, "数学".to_owned())]);
    assert!(heard(&mut stranger_rt).await.is_empty());

    ctx.pass(12);
    ctx.heartbeat(&me).await;
    let seen = studying_friends(&ctx, &friend).await;
    assert_eq!(seen.len(), 1);
    assert_eq!(
        (
            seen[0].user.as_option().unwrap().id.as_str(),
            seen[0].studied_seconds,
            seen[0].subject.as_option().unwrap().name.as_str()
        ),
        (me.id.to_string().as_str(), 720, "数学")
    );
    assert_eq!(
        (seen[0].state.as_known().unwrap(), seen[0].phase.as_known().unwrap()),
        (SessionState::SESSION_STATE_RUNNING, TimerPhase::TIMER_PHASE_WORK)
    );
    assert!(studying_friends(&ctx, &stranger).await.is_empty());
    assert!(studying_friends(&ctx, &me).await.is_empty(), "the list is about friends");

    // On the profile, for friends.
    let card = ctx.profile_card(&friend, &me, "school.studying").await.expect("friends see the card");
    assert_eq!(
        (card.title.as_option().unwrap().ja.as_str(), card.text.as_option().unwrap().ja.as_str()),
        ("勉強中", "数学 · 12分")
    );
    assert_eq!(card.data["studied_seconds"], "720");
    assert!(ctx.profile_card(&stranger, &me, "school.studying").await.is_none());

    // Hidden: friends are told the user stopped, and see nothing anymore.
    let hidden = ctx
        .update_settings(
            &me,
            UpdateSchoolSettingsRequest {
                study_visibility: Some(StudyVisibility::STUDY_VISIBILITY_NOBODY.into()),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(hidden.study_visibility, StudyVisibility::STUDY_VISIBILITY_NOBODY);
    assert_eq!(studying(heard(&mut friend_rt).await), vec![(false, String::new())]);
    assert!(studying_friends(&ctx, &friend).await.is_empty());
    assert!(ctx.profile_card(&friend, &me, "school.studying").await.is_none());
    let _: PauseStudySessionResponse =
        ctx.ok(STUDY, "PauseStudySession", &me, &PauseStudySessionRequest::default()).await;
    assert!(heard(&mut friend_rt).await.is_empty(), "nothing is announced while hidden");

    // Shown again while paused, then stopped.
    ctx.update_settings(
        &me,
        UpdateSchoolSettingsRequest {
            study_visibility: Some(StudyVisibility::STUDY_VISIBILITY_FRIENDS.into()),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(studying(heard(&mut friend_rt).await), vec![(true, "数学".to_owned())]);
    assert_eq!(studying_friends(&ctx, &friend).await[0].state, SessionState::SESSION_STATE_PAUSED);
    assert_eq!(
        ctx.profile_card(&friend, &me, "school.studying").await.unwrap().title.as_option().unwrap().ja,
        "休憩中"
    );
    ctx.stop(&me).await;
    assert_eq!(studying(heard(&mut friend_rt).await), vec![(false, String::new())]);
    assert!(studying_friends(&ctx, &friend).await.is_empty());

    // A client that went silent is not shown as studying.
    ctx.start_stopwatch(&me).await;
    ctx.pass(20);
    assert!(studying_friends(&ctx, &friend).await.is_empty());
}

#[tokio::test]
async fn a_friends_heatmap_follows_their_study_visibility() {
    let ctx = start().await;
    let (owner, friend, stranger) =
        (ctx.app.user("owner").await, ctx.app.user("friend").await, ctx.app.user("stranger").await);
    ctx.befriend(&owner, &friend).await;
    ctx.at(2026, 4, 10, 12, 0);
    ctx.log(&owner, tokyo(2026, 4, 9, 10, 0), tokyo(2026, 4, 9, 11, 0), None).await.unwrap();
    let of_owner = GetStudyHeatmapRequest { user_id: owner.id.to_string(), ..Default::default() };

    let seen: GetStudyHeatmapResponse = ctx.ok(STUDY, "GetStudyHeatmap", &friend, &of_owner).await;
    assert_eq!(seen.days.iter().map(|d| (d.date.as_str(), d.seconds)).collect::<Vec<_>>(), vec![("2026-04-09", 3600)]);
    assert_eq!(ctx.reason(STUDY, "GetStudyHeatmap", &stranger, &of_owner).await, "STUDY_NOT_SHARED");
    ctx.update_settings(
        &owner,
        UpdateSchoolSettingsRequest {
            study_visibility: Some(StudyVisibility::STUDY_VISIBILITY_NOBODY.into()),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(ctx.reason(STUDY, "GetStudyHeatmap", &friend, &of_owner).await, "STUDY_NOT_SHARED");
    let own: GetStudyHeatmapResponse = ctx.ok(STUDY, "GetStudyHeatmap", &owner, &of_owner).await;
    assert_eq!(own.total_seconds, 3600);
}

#[tokio::test]
async fn the_weekly_ranking_is_among_friends_who_joined() {
    let ctx = start().await;
    let (me, top, tied, outsider, stranger) = (
        ctx.app.user("me").await,
        ctx.app.user("top").await,
        ctx.app.user("tied").await,
        ctx.app.user("outsider").await,
        ctx.app.user("stranger").await,
    );
    for friend in [&top, &tied, &outsider] {
        ctx.befriend(&me, friend).await;
    }
    // Timed study (the ranking counts nothing else): `top` 100 minutes last
    // week and 90 this week, `tied` and `me` an hour each, the two who are
    // not in the ranking five hours each.
    for (user, (month, day), minutes) in [
        (&top, (4, 2), 100),
        (&tied, (4, 7), 60),
        (&outsider, (4, 7), 300),
        (&stranger, (4, 7), 300),
        (&me, (4, 8), 60),
        (&top, (4, 9), 90),
    ] {
        ctx.at(2026, month, day, 10, 0);
        ctx.study_for(user, minutes).await;
    }
    ctx.at(2026, 4, 10, 12, 0); // a Friday
    let ranking = |anchor: &str| GetStudyRankingRequest { anchor_date: anchor.into(), ..Default::default() };

    // Until the caller joins there is nothing to see.
    let closed: GetStudyRankingResponse = ctx.ok(STUDY, "GetStudyRanking", &me, &ranking("")).await;
    assert!(!closed.joined && closed.entries.is_empty());
    assert_eq!((closed.start_date.as_str(), closed.end_date.as_str()), ("2026-04-06", "2026-04-12"));

    for user in [&me, &top, &tied] {
        ctx.update_settings(user, UpdateSchoolSettingsRequest { join_ranking: Some(true), ..Default::default() }).await;
    }
    let open: GetStudyRankingResponse = ctx.ok(STUDY, "GetStudyRanking", &me, &ranking("")).await;
    assert!(open.joined);
    let name = |entry: &RankingEntry| entry.user.as_option().unwrap().display_name.clone();
    assert_eq!(
        (name(&open.entries[0]), open.entries[0].rank, open.entries[0].seconds, open.entries[0].me),
        ("top".to_owned(), 1, 5400, false)
    );
    // Equal times share second place; the friend who did not join and the stranger are not listed.
    let mut second: Vec<(String, i32, i32, bool)> =
        open.entries[1..].iter().map(|e| (name(e), e.rank, e.seconds, e.me)).collect();
    second.sort();
    assert_eq!(second, vec![("me".to_owned(), 2, 3600, true), ("tied".to_owned(), 2, 3600, false)]);

    // Hours typed in by hand do not move anyone up: ten of them for `tied`
    // show in their own totals and leave the ranking as it was.
    let typed = ctx.log(&tied, tokyo(2026, 4, 9, 8, 0), tokyo(2026, 4, 9, 18, 0), None).await.unwrap();
    assert_eq!(typed.source, SessionSource::SESSION_SOURCE_MANUAL);
    assert_eq!(summary(&ctx, &tied, StatsPeriod::STATS_PERIOD_WEEK, "").await.total_seconds, 3600 + 36_000);
    let after: GetStudyRankingResponse = ctx.ok(STUDY, "GetStudyRanking", &me, &ranking("")).await;
    assert_eq!(after.entries, open.entries);

    // Last week only `top` studied.
    let last_week: GetStudyRankingResponse = ctx.ok(STUDY, "GetStudyRanking", &me, &ranking("2026-04-02")).await;
    assert_eq!(last_week.start_date, "2026-03-30");
    assert_eq!((name(&last_week.entries[0]), last_week.entries[0].seconds), ("top".to_owned(), 6000));
    assert!(last_week.entries[1..].iter().all(|e| e.seconds == 0 && e.rank == 2));
    // The ranking is each participant's own view: `tied` sees `me` only if we are friends (we are), not `top`.
    let theirs: GetStudyRankingResponse = ctx.ok(STUDY, "GetStudyRanking", &tied, &ranking("")).await;
    let mut names: Vec<String> = theirs.entries.iter().map(name).collect();
    names.sort();
    assert_eq!(names, vec!["me", "tied"]);
}

#[tokio::test]
async fn the_senders_cheer_limit_is_told_from_a_friends_cool_down() {
    let ctx = start().await;
    let (me, busy, other) = (ctx.app.user("me").await, ctx.app.user("busy").await, ctx.app.user("other").await);
    ctx.befriend(&me, &busy).await;
    ctx.befriend(&me, &other).await;
    ctx.at(2026, 4, 13, 18, 0);
    ctx.start_stopwatch(&busy).await;
    ctx.start_stopwatch(&other).await;
    let cheer = |user: &TestUser| CheerStudyRequest { user_id: user.id.to_string(), ..Default::default() };

    // The client shows "sent" on a cool-down, so only the friend's own
    // cool-down may say CHEER_TOO_SOON: 60 a minute in all, then RATE_LIMITED
    // for a friend who was never cheered on.
    let _: CheerStudyResponse = ctx.ok(STUDY, "CheerStudy", &me, &cheer(&busy)).await;
    for _ in 1..60 {
        assert_eq!(ctx.reason(STUDY, "CheerStudy", &me, &cheer(&busy)).await, "CHEER_TOO_SOON");
    }
    assert_eq!(ctx.reason(STUDY, "CheerStudy", &me, &cheer(&other)).await, "RATE_LIMITED");
    assert_eq!(current(&ctx, &other).await.unwrap().cheer_count, 0);
}

#[tokio::test]
async fn a_cheer_is_a_quiet_nudge_between_friends() {
    let ctx = start().await;
    let (me, friend, fan, stranger) = (
        ctx.app.user("me").await,
        ctx.app.user("friend").await,
        ctx.app.user("fan").await,
        ctx.app.user("stranger").await,
    );
    ctx.befriend(&me, &friend).await;
    ctx.befriend(&fan, &friend).await;
    ctx.at(2026, 4, 13, 18, 0);
    let cheer = |user: &TestUser| CheerStudyRequest { user_id: user.id.to_string(), ..Default::default() };

    // Nobody to cheer on yet.
    assert_eq!(ctx.reason(STUDY, "CheerStudy", &me, &cheer(&friend)).await, "NOT_STUDYING");
    let session = ctx.start_stopwatch(&friend).await;
    let mut friend_rt = ctx.app.listen_user(friend.id).await;

    let sent: CheerStudyResponse = ctx.ok(STUDY, "CheerStudy", &me, &cheer(&friend)).await;
    let sent = sent.cheer.as_option().cloned().unwrap();
    assert_eq!(
        (sent.from.as_option().unwrap().id.as_str(), sent.session_id.as_str()),
        (me.id.to_string().as_str(), session.id.as_str())
    );
    let received: Vec<String> = heard(&mut friend_rt)
        .await
        .into_iter()
        .filter_map(|push| match push {
            school_event::Event::Cheer(cheer) => Some(cheer.from.as_option().unwrap().display_name.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(received, vec!["me"]);
    assert!(ctx.notifications_for(friend.id, "school.cheer").await.is_empty(), "no notification unless asked for");
    assert_eq!(current(&ctx, &friend).await.unwrap().cheer_count, 1);

    // One cheer per friend every few minutes; not for oneself; not from strangers.
    assert_eq!(ctx.reason(STUDY, "CheerStudy", &me, &cheer(&friend)).await, "CHEER_TOO_SOON");
    assert_eq!(ctx.reason(STUDY, "CheerStudy", &friend, &cheer(&friend)).await, "CANNOT_CHEER_SELF");
    assert_eq!(ctx.reason(STUDY, "CheerStudy", &stranger, &cheer(&friend)).await, "NOT_STUDYING");
    assert_eq!(ctx.reason(STUDY, "CheerStudy", &friend, &cheer(&me)).await, "NOT_STUDYING");

    // Someone who asked for it gets a notification too.
    ctx.update_settings(&friend, UpdateSchoolSettingsRequest { cheer_notifications: Some(true), ..Default::default() })
        .await;
    let _: CheerStudyResponse = ctx.ok(STUDY, "CheerStudy", &fan, &cheer(&friend)).await;
    let notified = ctx.notifications_for(friend.id, "school.cheer").await;
    assert_eq!(notified.len(), 1);
    assert_eq!((notified[0].actor_id, notified[0].data["title"]["ja"].as_str()), (Some(fan.id), Some("がんばれ!")));

    let list = ListStudyCheersRequest { session_id: session.id.clone(), ..Default::default() };
    let cheers: ListStudyCheersResponse = ctx.ok(STUDY, "ListStudyCheers", &friend, &list).await;
    assert_eq!(
        cheers.cheers.iter().map(|c| c.from.as_option().unwrap().display_name.clone()).collect::<Vec<_>>(),
        vec!["fan", "me"],
        "newest first"
    );
    assert_eq!(ctx.reason(STUDY, "ListStudyCheers", &me, &list).await, "SESSION_NOT_FOUND");
    assert_eq!(ctx.stop(&friend).await.session.as_option().unwrap().cheer_count, 2);

    // Someone who hides their studying cannot be cheered on.
    ctx.update_settings(
        &friend,
        UpdateSchoolSettingsRequest {
            study_visibility: Some(StudyVisibility::STUDY_VISIBILITY_NOBODY.into()),
            ..Default::default()
        },
    )
    .await;
    ctx.start_stopwatch(&friend).await;
    assert_eq!(ctx.reason(STUDY, "CheerStudy", &stranger, &cheer(&friend)).await, "NOT_STUDYING");
    let hidden = ctx.app.user("hidden-cheerer").await;
    ctx.befriend(&hidden, &friend).await;
    assert_eq!(ctx.reason(STUDY, "CheerStudy", &hidden, &cheer(&friend)).await, "NOT_STUDYING");
}

#[tokio::test]
async fn hidden_studying_stays_out_of_friends_feeds() {
    let ctx = start().await;
    let (open, hidden, quiet) = (ctx.app.user("open").await, ctx.app.user("hidden").await, ctx.app.user("quiet").await);
    ctx.at(2026, 4, 13, 18, 0);
    ctx.update_settings(
        &hidden,
        UpdateSchoolSettingsRequest {
            study_visibility: Some(StudyVisibility::STUDY_VISIBILITY_NOBODY.into()),
            ..Default::default()
        },
    )
    .await;
    ctx.update_settings(
        &quiet,
        UpdateSchoolSettingsRequest { share_study_activity: Some(false), ..Default::default() },
    )
    .await;
    for user in [&open, &hidden, &quiet] {
        ctx.study_for(user, 15).await;
    }
    assert_eq!(ctx.activities_of(open.id, "school.studied").await.len(), 1);
    assert!(
        ctx.activities_of(hidden.id, "school.studied").await.is_empty(),
        "nobody sees them studying, so no feed entry says they did"
    );
    assert!(ctx.activities_of(quiet.id, "school.studied").await.is_empty());
    // A short session is not news either.
    ctx.study_for(&open, 5).await;
    assert_eq!(ctx.activities_of(open.id, "school.studied").await.len(), 1);
    // The log and the event for other domains do not depend on any of it.
    assert_eq!(ctx.sessions(&hidden).await.len(), 1);
    assert_eq!(ctx.app.events_by("school.study.session_ended", hidden.id).await.len(), 1);
}

#[tokio::test]
async fn a_timed_session_can_be_edited_shorter_never_longer() {
    let ctx = start().await;
    let me = ctx.app.user("me").await;
    ctx.at(2026, 4, 13, 18, 0);
    // A pomodoro from 18:00: work until 18:25, break until 18:30, work until 18:40.
    let classic =
        PomodoroConfig { work_minutes: 25, break_minutes: 5, long_break_minutes: 15, rounds: 4, ..Default::default() };
    let request = StartStudySessionRequest {
        mode: StudyMode::STUDY_MODE_POMODORO.into(),
        pomodoro: classic.into(),
        ..Default::default()
    };
    let _: StartStudySessionResponse = ctx.ok(STUDY, "StartStudySession", &me, &request).await;
    for _ in 0..4 {
        ctx.pass(10);
        ctx.heartbeat(&me).await;
    }
    let session = ctx.stop(&me).await.session.as_option().cloned().unwrap();
    assert_eq!(session.studied_seconds, 35 * 60);
    ctx.pass(60);
    let at = |hour: u32, minute: u32| tokyo(2026, 4, 13, hour, minute);
    let edit = |from, to| UpdateStudySessionRequest {
        session_id: session.id.clone(),
        started_at: stamp(from),
        ended_at: stamp(to),
        ..Default::default()
    };

    // Not longer, not moved, not re-saved as one stretch that swallows the break.
    for (from, to) in
        [(at(17, 0), at(18, 40)), (at(18, 0), at(19, 0)), (at(17, 30), at(18, 10)), (at(18, 41), at(19, 10))]
    {
        assert_eq!(ctx.reason(STUDY, "UpdateStudySession", &me, &edit(from, to)).await, "TIME_RANGE_INVALID");
    }
    let same: UpdateStudySessionResponse = ctx.ok(STUDY, "UpdateStudySession", &me, &edit(at(18, 0), at(18, 40))).await;
    assert_eq!(same.session.as_option().unwrap().studied_seconds, 35 * 60, "the break is still not study time");
    // Nothing was counted during the break, so that is not a session.
    assert_eq!(ctx.reason(STUDY, "UpdateStudySession", &me, &edit(at(18, 26), at(18, 29))).await, "TIME_RANGE_INVALID");

    // Shorter: from 18:10 to 18:35 is 15 minutes of work, the break, and 5 more.
    let shorter: UpdateStudySessionResponse =
        ctx.ok(STUDY, "UpdateStudySession", &me, &edit(at(18, 10), at(18, 35))).await;
    let shorter = shorter.session.as_option().cloned().unwrap();
    assert_eq!(
        (shorter.studied_seconds, shorter.source.as_known().unwrap()),
        (20 * 60, SessionSource::SESSION_SOURCE_TIMER)
    );
    assert_eq!((instant(&shorter.started_at), instant(&shorter.ended_at)), (at(18, 10), at(18, 35)));
    // What was cut is gone: it cannot be edited back in.
    assert_eq!(ctx.reason(STUDY, "UpdateStudySession", &me, &edit(at(18, 0), at(18, 40))).await, "TIME_RANGE_INVALID");
    assert_eq!(summary(&ctx, &me, StatsPeriod::STATS_PERIOD_DAY, "2026-04-13").await.total_seconds, 20 * 60);
}
