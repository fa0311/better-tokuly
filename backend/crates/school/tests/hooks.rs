//! What the school domain owes the rest of the backend: every RPC needs a
//! signed-in caller; deleted accounts, ended friendships, blocks and
//! takedowns are cleaned up after; study rooms are reportable subjects and
//! guarded realtime topics; the data export has a `school` section.
mod common;

use common::*;
use tokuly_proto::{
    common::SubjectType,
    school::{school_event::Event as Push, *},
};
use tokuly_testkit::TestUser;

#[tokio::test]
async fn every_rpc_needs_a_signed_in_caller() {
    let ctx = start().await;
    let rpcs: [(&str, &[&str]); 6] = [
        (SETTINGS, &["GetSchoolSettings", "UpdateSchoolSettings"]),
        (
            TIMETABLE,
            &[
                "ListLayoutPresets",
                "ListSubjects",
                "CreateSubject",
                "UpdateSubject",
                "DeleteSubject",
                "ListTerms",
                "GetTerm",
                "CreateTerm",
                "UpdateTerm",
                "DeleteTerm",
                "PutTimetableEntries",
                "DeleteTimetableEntries",
                "CreateTimetableChange",
                "DeleteTimetableChange",
                "CreateDayOverride",
                "DeleteDayOverride",
                "GetDay",
                "ListDays",
                "GetNow",
                "ListFriendTimetables",
                "GetFriendTimetable",
                "CopyFriendTimetable",
                "FindCommonFreePeriods",
            ],
        ),
        (
            STUDY,
            &[
                "StartStudySession",
                "PauseStudySession",
                "ResumeStudySession",
                "StopStudySession",
                "HeartbeatStudySession",
                "SkipStudyPhase",
                "GetCurrentStudySession",
                "LogStudySession",
                "UpdateStudySession",
                "DeleteStudySession",
                "ListStudySessions",
                "GetStudySummary",
                "GetStudyProgress",
                "GetStudyHeatmap",
                "ListStudyingFriends",
                "GetStudyRanking",
                "CheerStudy",
                "ListStudyCheers",
            ],
        ),
        (
            ROOMS,
            &[
                "CreateStudyRoom",
                "GetStudyRoom",
                "UpdateStudyRoom",
                "CloseStudyRoom",
                "ListStudyRooms",
                "JoinStudyRoom",
                "LeaveStudyRoom",
                "SetStudyRoomSubject",
                "ControlStudyRoomTimer",
                "SendStudyRoomReaction",
                "InviteToStudyRoom",
                "UninviteFromStudyRoom",
                "RemoveStudyRoomMember",
            ],
        ),
        (
            EXAMS,
            &[
                "ListExams",
                "GetExam",
                "CreateExam",
                "UpdateExam",
                "DeleteExam",
                "PutExamSubject",
                "DeleteExamSubject",
                "SetExamResult",
            ],
        ),
        (ASSIGNMENTS, &["ListAssignments", "CreateAssignment", "UpdateAssignment", "DeleteAssignment"]),
    ];
    let mut checked = 0;
    for (service, methods) in rpcs {
        for method in methods {
            let error = ctx
                .app
                .call::<_, serde_json::Value>(&format!("{service}/{method}"), None, &serde_json::json!({}))
                .await
                .unwrap_err();
            assert_eq!(
                (error.code.as_str(), error.reason.as_str()),
                ("unauthenticated", "SIGN_IN_REQUIRED"),
                "{service}/{method}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 68, "every RPC of the six services");
}

/// How many rows a table holds about a user.
async fn rows(ctx: &Ctx, table: &str, column: &str, user: &TestUser) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table} WHERE {column} = $1")))
        .bind(user.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap()
}

const TABLES: [(&str, &str); 18] = [
    ("school_settings", "user_id"),
    ("school_subjects", "user_id"),
    ("school_terms", "user_id"),
    ("school_term_shares", "user_id"),
    ("school_entries", "user_id"),
    ("school_changes", "user_id"),
    ("school_day_overrides", "user_id"),
    ("school_rooms", "host_id"),
    ("school_room_invites", "user_id"),
    ("school_room_bans", "user_id"),
    ("school_room_visits", "user_id"),
    ("school_study_sessions", "user_id"),
    ("school_study_segments", "user_id"),
    ("school_study_cheers", "from_user_id"),
    ("school_study_cheers", "to_user_id"),
    ("school_exams", "user_id"),
    ("school_exam_subjects", "user_id"),
    ("school_assignments", "user_id"),
];

#[tokio::test]
async fn a_deleted_account_leaves_nothing_behind() {
    let ctx = start().await;
    let (gone, friend, guest) = (ctx.app.user("gone").await, ctx.app.user("friend").await, ctx.app.user("guest").await);
    ctx.befriend(&gone, &friend).await;
    ctx.befriend(&gone, &guest).await;
    ctx.at(2026, 4, 13, 18, 0);

    // Something of everything: settings, a shared term with a class, a
    // change and a holiday, an exam with a subject, an assignment, a log.
    ctx.update_settings(&gone, UpdateSchoolSettingsRequest { daily_goal_minutes: Some(30), ..Default::default() })
        .await;
    let math = ctx.subject(&gone, "数学").await;
    let term = ctx
        .term(&gone, |t| {
            t.audience = TimetableAudience::TIMETABLE_AUDIENCE_CHOSEN.into();
            t.shared_with_user_ids = vec![];
        })
        .await;
    let share = UpdateTermRequest {
        term_id: term.id.clone(),
        shared_with: UserIdList { user_ids: vec![friend.id.to_string()], ..Default::default() }.into(),
        ..Default::default()
    };
    let _: UpdateTermResponse = ctx.ok(TIMETABLE, "UpdateTerm", &gone, &share).await;
    ctx.put(&gone, &term, vec![class(Weekday::WEEKDAY_MONDAY, 1, &math)]).await;
    let cancel = CreateTimetableChangeRequest {
        term_id: term.id.clone(),
        date: "2026-04-20".into(),
        period: 1,
        kind: ChangeKind::CHANGE_KIND_CANCELLED.into(),
        ..Default::default()
    };
    let _: CreateTimetableChangeResponse = ctx.ok(TIMETABLE, "CreateTimetableChange", &gone, &cancel).await;
    let holiday = CreateDayOverrideRequest {
        term_id: term.id.clone(),
        start_date: "2026-04-21".into(),
        kind: DayOverrideKind::DAY_OVERRIDE_KIND_HOLIDAY.into(),
        ..Default::default()
    };
    let _: CreateDayOverrideResponse = ctx.ok(TIMETABLE, "CreateDayOverride", &gone, &holiday).await;
    let exam: CreateExamResponse = ctx
        .ok(
            EXAMS,
            "CreateExam",
            &gone,
            &CreateExamRequest { name: "期末".into(), start_date: "2026-07-01".into(), ..Default::default() },
        )
        .await;
    let put = PutExamSubjectRequest {
        exam_id: exam.exam.as_option().unwrap().id.clone(),
        subject_id: math.id.clone(),
        ..Default::default()
    };
    let _: PutExamSubjectResponse = ctx.ok(EXAMS, "PutExamSubject", &gone, &put).await;
    let homework =
        CreateAssignmentRequest {
            title: "宿題".into(), due_at: stamp(tokyo(2026, 4, 20, 9, 0)), ..Default::default()
        };
    let _: CreateAssignmentResponse = ctx.ok(ASSIGNMENTS, "CreateAssignment", &gone, &homework).await;
    ctx.log(&gone, tokyo(2026, 4, 13, 16, 0), tokyo(2026, 4, 13, 17, 0), Some(&math)).await.unwrap();

    // Ties to other people: the friend shares a term with them and copied
    // theirs; they cheered the friend on; the friend invited them to a room.
    let friends_term = ctx
        .term(&friend, |t| {
            t.audience = TimetableAudience::TIMETABLE_AUDIENCE_CHOSEN.into();
            t.shared_with_user_ids = vec![];
        })
        .await;
    let share = UpdateTermRequest {
        term_id: friends_term.id.clone(),
        shared_with: UserIdList { user_ids: vec![gone.id.to_string()], ..Default::default() }.into(),
        ..Default::default()
    };
    let _: UpdateTermResponse = ctx.ok(TIMETABLE, "UpdateTerm", &friend, &share).await;
    let copied: CopyFriendTimetableResponse = ctx
        .ok(
            TIMETABLE,
            "CopyFriendTimetable",
            &friend,
            &CopyFriendTimetableRequest { user_id: gone.id.to_string(), ..Default::default() },
        )
        .await;
    let copy = copied.term.as_option().cloned().unwrap();
    assert!(copy.copied_from.as_option().is_some());
    let friends_room = ctx
        .create_room(&friend, |room| room.invite_user_ids = vec![gone.id.to_string()])
        .await
        .room
        .as_option()
        .cloned()
        .unwrap();
    let _: CheerStudyResponse = ctx
        .ok(STUDY, "CheerStudy", &gone, &CheerStudyRequest { user_id: friend.id.to_string(), ..Default::default() })
        .await;

    // And a room of their own, with a guest studying in it.
    let room = ctx
        .create_room(&gone, |room| room.audience = RoomAudience::ROOM_AUDIENCE_PUBLIC.into())
        .await
        .room
        .as_option()
        .cloned()
        .unwrap();
    ctx.join(&guest, &room.id).await.unwrap();
    ctx.control(&gone, &room.id, TimerAction::TIMER_ACTION_START).await.unwrap();
    ctx.pass(10);
    ctx.beat(&[&gone, &guest, &friend]).await;
    let mut before = 0;
    for (table, column) in TABLES {
        before += rows(&ctx, table, column, &gone).await;
    }
    assert!(before >= 14, "the account has rows all over the schema: {before}");

    let mut guest_rt = ctx.app.listen_user(guest.id).await;
    ctx.deliver_event("identity.user.deleted", serde_json::json!({ "user_id": gone.id })).await;

    for (table, column) in TABLES {
        assert_eq!(rows(&ctx, table, column, &gone).await, 0, "{table}.{column}");
    }
    // Their room is gone; the guest's time in it is logged, and the guest is told.
    assert_eq!(ctx.room(&guest, &room.id).await.unwrap_err().reason, "ROOM_NOT_FOUND");
    assert_eq!(ctx.sessions(&guest).await.iter().map(|s| s.studied_seconds).collect::<Vec<_>>(), vec![600]);
    let heard_by_guest = heard(&mut guest_rt).await;
    let Some(Push::RoomLeft(left)) = heard_by_guest.iter().find(|push| kind(push) == "room_left") else {
        panic!("{:?}", kinds(&heard_by_guest))
    };
    assert_eq!(left.reason, RoomLeaveReason::ROOM_LEAVE_REASON_CLOSED);
    // Nothing of the friend's points at them anymore.
    let loaded: GetTermResponse = ctx
        .ok(TIMETABLE, "GetTerm", &friend, &GetTermRequest { term_id: friends_term.id.clone(), ..Default::default() })
        .await;
    assert!(loaded.term.as_option().unwrap().shared_with.is_empty());
    let loaded: GetTermResponse =
        ctx.ok(TIMETABLE, "GetTerm", &friend, &GetTermRequest { term_id: copy.id.clone(), ..Default::default() }).await;
    assert!(loaded.term.as_option().unwrap().copied_from.as_option().is_none());
    assert_eq!(loaded.entries.len(), 1, "the copy itself is the friend's");
    let seen = ctx.room(&friend, &friends_room.id).await.unwrap();
    assert!(seen.invitees.is_empty());
    assert_eq!(ctx.heartbeat(&friend).await.cheer_count, 0);

    // Events are delivered at least once: a second time changes nothing.
    ctx.deliver_event("identity.user.deleted", serde_json::json!({ "user_id": gone.id })).await;
    assert_eq!(ctx.sessions(&guest).await.len(), 1);
}

#[tokio::test]
async fn ended_friendships_and_blocks_take_people_out_of_rooms() {
    let ctx = start().await;
    let (host, friend, pal) = (ctx.app.user("host").await, ctx.app.user("friend").await, ctx.app.user("pal").await);
    ctx.befriend(&host, &friend).await;
    ctx.befriend(&host, &pal).await;
    ctx.at(2026, 4, 13, 18, 0);

    // A friends-only room: the friend is inside, the pal is invited.
    let room = ctx
        .create_room(&host, |room| room.invite_user_ids = vec![pal.id.to_string()])
        .await
        .room
        .as_option()
        .cloned()
        .unwrap();
    ctx.join(&friend, &room.id).await.unwrap();
    let mut friend_rt = ctx.app.listen_user(friend.id).await;
    ctx.unfriend(&friend, &host).await;
    let seen = ctx.room(&host, &room.id).await.unwrap();
    assert_eq!((seen.member_count, seen.invitees.len()), (1, 1));
    let heard_by_friend = heard(&mut friend_rt).await;
    let Some(Push::RoomLeft(left)) = heard_by_friend.iter().find(|push| kind(push) == "room_left") else {
        panic!("{:?}", kinds(&heard_by_friend))
    };
    assert_eq!(left.reason, RoomLeaveReason::ROOM_LEAVE_REASON_REMOVED);
    assert_eq!(ctx.room(&friend, &room.id).await.unwrap_err().reason, "ROOM_NOT_FOUND");
    // The invitation goes with the friendship.
    ctx.unfriend(&host, &pal).await;
    assert!(ctx.room(&host, &room.id).await.unwrap().invitees.is_empty());
    assert_eq!(ctx.room(&pal, &room.id).await.unwrap_err().reason, "ROOM_NOT_FOUND");

    // A public room takes no friendship to be in: unfriending changes nothing, a block does.
    let (owner, visitor) = (ctx.app.user("owner").await, ctx.app.user("visitor").await);
    ctx.befriend(&owner, &visitor).await;
    let public = ctx
        .create_room(&owner, |room| room.audience = RoomAudience::ROOM_AUDIENCE_PUBLIC.into())
        .await
        .room
        .as_option()
        .cloned()
        .unwrap();
    ctx.join(&visitor, &public.id).await.unwrap();
    ctx.unfriend(&owner, &visitor).await;
    assert_eq!(ctx.room(&owner, &public.id).await.unwrap().member_count, 2);
    ctx.block(&owner, &visitor).await;
    assert_eq!(ctx.room(&owner, &public.id).await.unwrap().member_count, 1);
    assert_eq!(ctx.join(&visitor, &public.id).await.unwrap_err().reason, "ROOM_NOT_FOUND");

    // Two guests of someone else's room: the one who was blocked leaves.
    let (blocker, blocked) = (ctx.app.user("blocker").await, ctx.app.user("blocked").await);
    ctx.join(&blocker, &public.id).await.unwrap();
    ctx.join(&blocked, &public.id).await.unwrap();
    ctx.block(&blocker, &blocked).await;
    let seen = ctx.room(&owner, &public.id).await.unwrap();
    let mut names: Vec<String> =
        seen.members.iter().map(|m| m.user.as_option().unwrap().display_name.clone()).collect();
    names.sort();
    assert_eq!(names, vec!["blocker", "owner"]);
    assert_eq!(ctx.reason(ROOMS, "LeaveStudyRoom", &blocked, &LeaveStudyRoomRequest::default()).await, "NOT_IN_ROOM");
}

#[tokio::test]
async fn a_room_taken_down_by_moderation_closes() {
    let ctx = start().await;
    let (host, guest) = (ctx.app.user("host").await, ctx.app.user("guest").await);
    ctx.at(2026, 4, 13, 18, 0);
    let room = ctx
        .create_room(&host, |room| room.audience = RoomAudience::ROOM_AUDIENCE_PUBLIC.into())
        .await
        .room
        .as_option()
        .cloned()
        .unwrap();
    ctx.join(&guest, &room.id).await.unwrap();
    let mut topic = ctx.app.listen_topic(&room.topic).await;
    let removed = |subject_type: SubjectType, subject_id: &str| serde_json::json!({ "subject_type": subject_type as i32, "subject_id": subject_id, "reason": "不適切なタイトル", "moderator_id": uuid::Uuid::now_v7() });

    // Takedowns of other things, or of rooms that do not exist, change nothing.
    ctx.deliver_event("admin.content.removed", removed(SubjectType::SUBJECT_TYPE_NOTE, &room.id)).await;
    ctx.deliver_event(
        "admin.content.removed",
        removed(SubjectType::SUBJECT_TYPE_STUDY_ROOM, &uuid::Uuid::now_v7().to_string()),
    )
    .await;
    assert_eq!(ctx.room(&guest, &room.id).await.unwrap().member_count, 2);

    ctx.deliver_event("admin.content.removed", removed(SubjectType::SUBJECT_TYPE_STUDY_ROOM, &room.id)).await;
    assert_eq!(ctx.room(&guest, &room.id).await.unwrap_err().reason, "ROOM_NOT_FOUND");
    assert!(ctx.room(&host, &room.id).await.unwrap().closed_at.as_option().is_some());
    let heard_in_room = heard(&mut topic).await;
    let Some(Push::RoomClosed(closed)) = heard_in_room.first() else { panic!("{:?}", kinds(&heard_in_room)) };
    assert_eq!(closed.reason, RoomCloseReason::ROOM_CLOSE_REASON_REMOVED);
    assert_eq!(ctx.app.events_by("school.room.closed", host.id).await[0].data["reason"].as_i64(), Some(3));
    assert_eq!(ctx.reason(ROOMS, "LeaveStudyRoom", &guest, &LeaveStudyRoomRequest::default()).await, "NOT_IN_ROOM");
    // Delivered again: nothing left to do.
    ctx.deliver_event("admin.content.removed", removed(SubjectType::SUBJECT_TYPE_STUDY_ROOM, &room.id)).await;
}

#[tokio::test]
async fn rooms_are_subjects_and_topics_for_those_who_may_enter() {
    let ctx = start().await;
    let (host, friend, stranger) =
        (ctx.app.user("host").await, ctx.app.user("friend").await, ctx.app.user("stranger").await);
    ctx.befriend(&host, &friend).await;
    let core = &ctx.app.core;
    let room = ctx.create_room(&host, |room| room.title = "静かに勉強".into()).await.room.as_option().cloned().unwrap();
    let study_room = SubjectType::SUBJECT_TYPE_STUDY_ROOM;
    assert!(core.subjects.is_registered(study_room));

    // Reporting a room goes through the subject resolver.
    let info = core.subjects.resolve(study_room, &room.id, Some(friend.id)).await.unwrap();
    assert_eq!(
        (info.id, info.owner_id, info.title.as_str(), info.path.as_str()),
        (id(&room.id), host.id, "静かに勉強", format!("/school/rooms/{}", room.id).as_str())
    );
    assert!(!info.interactions_enabled, "rooms take reports, not comments");
    for (subject, viewer) in
        [(room.id.as_str(), Some(stranger.id)), (room.id.as_str(), None), ("not-a-room", Some(friend.id))]
    {
        assert_eq!(core.subjects.resolve(study_room, subject, viewer).await.unwrap_err().reason, "SUBJECT_NOT_FOUND");
    }

    // The room's topic is for the same people.
    let guards = &core.topic_guards;
    assert!(guards.may_subscribe(Some(friend.id), &room.topic).await.unwrap());
    assert!(guards.may_subscribe(Some(host.id), &room.topic).await.unwrap());
    assert!(!guards.may_subscribe(Some(stranger.id), &room.topic).await.unwrap());
    assert!(!guards.may_subscribe(None, &room.topic).await.unwrap());
    assert!(!guards.may_subscribe(Some(friend.id), "study:not-a-room").await.unwrap());
    assert!(!guards.may_subscribe(Some(friend.id), &format!("study:{}", uuid::Uuid::now_v7())).await.unwrap());

    // A public room is open to anyone signed in — and to nobody anonymous.
    let public = ctx
        .create_room(&stranger, |room| room.audience = RoomAudience::ROOM_AUDIENCE_PUBLIC.into())
        .await
        .room
        .as_option()
        .cloned()
        .unwrap();
    assert!(guards.may_subscribe(Some(friend.id), &public.topic).await.unwrap());
    assert!(!guards.may_subscribe(None, &public.topic).await.unwrap());
    assert!(core.subjects.resolve(study_room, &public.id, Some(friend.id)).await.is_ok());

    // A closed room is neither.
    let _: CloseStudyRoomResponse = ctx
        .ok(ROOMS, "CloseStudyRoom", &host, &CloseStudyRoomRequest { room_id: room.id.clone(), ..Default::default() })
        .await;
    assert!(!guards.may_subscribe(Some(friend.id), &room.topic).await.unwrap());
    assert!(core.subjects.resolve(study_room, &room.id, Some(friend.id)).await.is_err());
}

#[tokio::test]
async fn the_data_export_has_a_school_section() {
    let ctx = start().await;
    let (me, other) = (ctx.app.user("me").await, ctx.app.user("other").await);
    ctx.at(2026, 4, 13, 18, 0);
    ctx.update_settings(&me, UpdateSchoolSettingsRequest { daily_goal_minutes: Some(45), ..Default::default() }).await;
    let math = ctx.subject(&me, "数学").await;
    let term = ctx.term(&me, |_| {}).await;
    ctx.put(
        &me,
        &term,
        vec![TimetableEntryInput { memo: "わたしのメモ".into(), ..class(Weekday::WEEKDAY_MONDAY, 1, &math) }],
    )
    .await;
    ctx.log(&me, tokyo(2026, 4, 13, 16, 0), tokyo(2026, 4, 13, 17, 0), Some(&math)).await.unwrap();
    let exam: CreateExamResponse = ctx
        .ok(
            EXAMS,
            "CreateExam",
            &me,
            &CreateExamRequest { name: "期末".into(), start_date: "2026-07-01".into(), ..Default::default() },
        )
        .await;
    let put = PutExamSubjectRequest {
        exam_id: exam.exam.as_option().unwrap().id.clone(),
        subject_id: math.id.clone(),
        ..Default::default()
    };
    let test: PutExamSubjectResponse = ctx.ok(EXAMS, "PutExamSubject", &me, &put).await;
    let result = SetExamResultRequest {
        exam_subject_id: test.exam_subject.as_option().unwrap().id.clone(),
        result: ExamResult { score: Some(82), ..Default::default() }.into(),
        ..Default::default()
    };
    let _: SetExamResultResponse = ctx.ok(EXAMS, "SetExamResult", &me, &result).await;
    let homework =
        CreateAssignmentRequest {
            title: "宿題".into(), due_at: stamp(tokyo(2026, 4, 20, 9, 0)), ..Default::default()
        };
    let _: CreateAssignmentResponse = ctx.ok(ASSIGNMENTS, "CreateAssignment", &me, &homework).await;

    let exporter = ctx
        .app
        .core
        .exporters
        .all()
        .into_iter()
        .find(|exporter| exporter.name() == "school")
        .expect("the school exporter is registered");
    let mine = exporter.export(me.id).await.unwrap();
    assert_eq!(mine["settings"]["daily_goal_minutes"], 45);
    assert_eq!(mine["subjects"][0]["name"], "数学");
    assert_eq!(
        (mine["terms"][0]["name"].as_str(), mine["terms"][0]["entries"][0]["memo"].as_str()),
        (Some("1学期"), Some("わたしのメモ"))
    );
    assert_eq!(
        (mine["study_sessions"][0]["studied_seconds"].as_i64(), mine["study_segments"].as_array().unwrap().len()),
        (Some(3600), 1)
    );
    assert_eq!(mine["exams"][0]["subjects"][0]["result"]["score"], 82);
    assert_eq!(mine["assignments"][0]["title"], "宿題");
    for section in
        ["cheers_sent", "cheers_received", "study_rooms_hosted", "study_room_invitations", "timetables_shared_with_me"]
    {
        assert_eq!(mine[section], serde_json::json!([]), "{section}");
    }

    // Someone who never used any of it exports empty sections, and nobody else's data.
    let theirs = exporter.export(other.id).await.unwrap();
    assert!(theirs["settings"].is_null());
    for section in ["subjects", "terms", "study_sessions", "exams", "assignments"] {
        assert_eq!(theirs[section], serde_json::json!([]), "{section}");
    }
}

#[tokio::test]
async fn a_room_can_be_reported_by_whoever_it_was_put_in_front_of() {
    let ctx = start().await;
    let (host, invitee, member, stranger) = (
        ctx.app.user("host").await,
        ctx.app.user("invitee").await,
        ctx.app.user("member").await,
        ctx.app.user("stranger").await,
    );
    ctx.befriend(&host, &invitee).await;
    ctx.befriend(&host, &member).await;
    let core = &ctx.app.core;
    let study_room = SubjectType::SUBJECT_TYPE_STUDY_ROOM;
    let reportable = |room: &StudyRoom, viewer: &TestUser| {
        let (room_id, viewer) = (room.id.clone(), viewer.id);
        async move { core.subjects.resolve(study_room, &room_id, Some(viewer)).await.is_ok() }
    };

    // An invitation-only room with a title someone may want to report; one
    // friend is invited, the other comes in and is thrown out.
    let room = ctx
        .create_room(&host, |room| {
            room.title = "ひどいタイトル".into();
            room.audience = RoomAudience::ROOM_AUDIENCE_INVITED.into();
            room.invite_user_ids = vec![invitee.id.to_string(), member.id.to_string()];
        })
        .await
        .room
        .as_option()
        .cloned()
        .unwrap();
    assert_eq!(ctx.notifications_for(invitee.id, "school.room_invite").await.len(), 1, "the title was delivered");
    ctx.join(&member, &room.id).await.unwrap();
    assert!(reportable(&room, &invitee).await && reportable(&room, &member).await);

    // Taking the invitation back, or throwing the member out, hides the room
    // from them — and leaves the report button where it was.
    let uninvite = UninviteFromStudyRoomRequest {
        room_id: room.id.clone(),
        user_id: invitee.id.to_string(),
        ..Default::default()
    };
    let _: UninviteFromStudyRoomResponse = ctx.ok(ROOMS, "UninviteFromStudyRoom", &host, &uninvite).await;
    let remove =
        RemoveStudyRoomMemberRequest { room_id: room.id.clone(), user_id: member.id.to_string(), ..Default::default() };
    let _: RemoveStudyRoomMemberResponse = ctx.ok(ROOMS, "RemoveStudyRoomMember", &host, &remove).await;
    for user in [&invitee, &member] {
        assert_eq!(ctx.room(user, &room.id).await.unwrap_err().reason, "ROOM_NOT_FOUND");
        assert!(reportable(&room, user).await, "{} can still report the room", user.handle);
    }
    // So does ending the friendship, and closing the room.
    ctx.unfriend(&host, &invitee).await;
    let _: CloseStudyRoomResponse = ctx
        .ok(ROOMS, "CloseStudyRoom", &host, &CloseStudyRoomRequest { room_id: room.id.clone(), ..Default::default() })
        .await;
    assert!(reportable(&room, &invitee).await && reportable(&room, &member).await);
    let info = core.subjects.resolve(study_room, &room.id, Some(invitee.id)).await.unwrap();
    assert_eq!((info.owner_id, info.title.as_str()), (host.id, "ひどいタイトル"));
    // Someone the room was never shown to still has nothing to report.
    assert!(!reportable(&room, &stranger).await);
    // A withdrawn invitation does not come back by itself: not visible, not joinable.
    assert_eq!(ctx.join(&invitee, &room.id).await.unwrap_err().reason, "ROOM_NOT_FOUND");
}

/// The names in the weekly ranking as `viewer` sees it, in order.
async fn ranking(ctx: &Ctx, viewer: &TestUser) -> Vec<String> {
    let response: GetStudyRankingResponse =
        ctx.ok(STUDY, "GetStudyRanking", viewer, &GetStudyRankingRequest::default()).await;
    response.entries.iter().map(|e| e.user.as_option().unwrap().display_name.clone()).collect()
}

/// How many friends `viewer` sees studying.
async fn studying(ctx: &Ctx, viewer: &TestUser) -> usize {
    let response: ListStudyingFriendsResponse =
        ctx.ok(STUDY, "ListStudyingFriends", viewer, &ListStudyingFriendsRequest::default()).await;
    response.friends.len()
}

#[tokio::test]
async fn a_suspended_account_is_present_nowhere_and_shares_nothing() {
    let ctx = start().await;
    let (suspended, friend, guest) =
        (ctx.app.user("suspended").await, ctx.app.user("friend").await, ctx.app.user("guest").await);
    ctx.befriend(&suspended, &friend).await;
    ctx.at(2026, 4, 13, 9, 0); // Monday, first period

    // A shared timetable, study time, a place in the ranking, and a room
    // with a guest in it.
    let math = ctx.subject(&suspended, "数学").await;
    let term = ctx.term(&suspended, |t| t.audience = TimetableAudience::TIMETABLE_AUDIENCE_FRIENDS.into()).await;
    ctx.put(&suspended, &term, vec![class(Weekday::WEEKDAY_MONDAY, 1, &math)]).await;
    ctx.log(&suspended, tokyo(2026, 4, 13, 7, 0), tokyo(2026, 4, 13, 8, 0), None).await.unwrap();
    for user in [&suspended, &friend] {
        ctx.update_settings(user, UpdateSchoolSettingsRequest { join_ranking: Some(true), ..Default::default() }).await;
    }
    let room = ctx
        .create_room(&suspended, |room| room.audience = RoomAudience::ROOM_AUDIENCE_PUBLIC.into())
        .await
        .room
        .as_option()
        .cloned()
        .unwrap();
    ctx.join(&guest, &room.id).await.unwrap();
    ctx.control(&suspended, &room.id, TimerAction::TIMER_ACTION_START).await.unwrap();
    ctx.pass(10);
    ctx.beat(&[&suspended, &guest]).await;

    let of_them = suspended.id.to_string();
    let heatmap = GetStudyHeatmapRequest { user_id: of_them.clone(), ..Default::default() };
    // Before: the friend sees all of it.
    assert!(ctx.day_of(&friend, &suspended, "2026-04-13").await.is_ok());
    assert!(ctx.call::<_, GetStudyHeatmapResponse>(STUDY, "GetStudyHeatmap", &friend, &heatmap).await.is_ok());
    assert_eq!(ranking(&ctx, &friend).await, vec!["suspended", "friend"]);
    assert_eq!(studying(&ctx, &friend).await, 1);

    // A late "suspended" event for an account that is not suspended changes nothing.
    ctx.deliver_event("identity.user.banned", serde_json::json!({ "user_id": suspended.id })).await;
    assert_eq!(ctx.room(&guest, &room.id).await.unwrap().member_count, 2);

    ctx.app.identity.set_banned(suspended.id, Some("spam")).await.unwrap();
    ctx.deliver_event("identity.user.banned", serde_json::json!({ "user_id": suspended.id })).await;

    // Their room closed; the guest's time in it is logged.
    assert_eq!(ctx.room(&guest, &room.id).await.unwrap_err().reason, "ROOM_NOT_FOUND");
    assert_eq!(ctx.sessions(&guest).await.iter().map(|s| s.studied_seconds).collect::<Vec<_>>(), vec![600]);
    // What they shared reads exactly as if they had stopped sharing it.
    assert_eq!(ctx.day_of(&friend, &suspended, "2026-04-13").await.unwrap_err().reason, "TIMETABLE_NOT_SHARED");
    assert_eq!(
        ctx.reason(
            TIMETABLE,
            "GetFriendTimetable",
            &friend,
            &GetFriendTimetableRequest { user_id: of_them.clone(), ..Default::default() }
        )
        .await,
        "TIMETABLE_NOT_SHARED"
    );
    let listed: ListFriendTimetablesResponse =
        ctx.ok(TIMETABLE, "ListFriendTimetables", &friend, &ListFriendTimetablesRequest::default()).await;
    assert!(listed.timetables.is_empty());
    assert_eq!(ctx.reason(STUDY, "GetStudyHeatmap", &friend, &heatmap).await, "STUDY_NOT_SHARED");
    assert_eq!(ranking(&ctx, &friend).await, vec!["friend"]);
    assert_eq!(studying(&ctx, &friend).await, 0);
    assert_eq!(
        ctx.reason(STUDY, "CheerStudy", &friend, &CheerStudyRequest { user_id: of_them.clone(), ..Default::default() })
            .await,
        "NOT_STUDYING"
    );
    let free = FindCommonFreePeriodsRequest {
        start_date: "2026-04-13".into(),
        user_ids: vec![of_them.clone()],
        ..Default::default()
    };
    let response: FindCommonFreePeriodsResponse = ctx.ok(TIMETABLE, "FindCommonFreePeriods", &friend, &free).await;
    assert_eq!(response.not_shared_user_ids, vec![of_them.clone()]);

    // Nothing was deleted: when the suspension is lifted it is all back.
    ctx.app.identity.set_banned(suspended.id, None).await.unwrap();
    assert!(ctx.day_of(&friend, &suspended, "2026-04-13").await.is_ok());
    assert_eq!(ranking(&ctx, &friend).await, vec!["suspended", "friend"]);
}
