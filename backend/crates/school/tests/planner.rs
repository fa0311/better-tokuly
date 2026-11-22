//! `SchoolService` (settings), `ExamService` and `AssignmentService`.
//! (Reminders are sent by the sweep: see `tests/sweep.rs`.)
mod common;

use common::*;
use tokuly_proto::{common::PageRequest, school::*};
use tokuly_testkit::TestUser;

async fn settings_of(ctx: &Ctx, user: &TestUser) -> SchoolSettings {
    let response: GetSchoolSettingsResponse =
        ctx.ok(SETTINGS, "GetSchoolSettings", user, &GetSchoolSettingsRequest::default()).await;
    response.settings.as_option().cloned().unwrap()
}

#[tokio::test]
async fn settings_start_cautious_and_change_one_by_one() {
    let ctx = start().await;
    let (me, other) = (ctx.app.user("me").await, ctx.app.user("other").await);

    let defaults = settings_of(&ctx, &me).await;
    assert_eq!(defaults.class_soon_minutes, 0, "timetable notifications are opt-in");
    assert_eq!(defaults.study_visibility, StudyVisibility::STUDY_VISIBILITY_FRIENDS);
    assert!(
        defaults.share_study_activity
            && !defaults.join_ranking
            && !defaults.cheer_notifications
            && defaults.assignment_reminders
    );
    assert_eq!(defaults.daily_goal_minutes, 0);
    let pomodoro = defaults.pomodoro.as_option().unwrap();
    assert_eq!(
        (pomodoro.work_minutes, pomodoro.break_minutes, pomodoro.long_break_minutes, pomodoro.rounds),
        (25, 5, 15, 4)
    );

    let changed = ctx
        .update_settings(
            &me,
            UpdateSchoolSettingsRequest {
                class_soon_minutes: Some(10),
                join_ranking: Some(true),
                daily_goal_minutes: Some(120),
                pomodoro: PomodoroConfig { work_minutes: 50, ..Default::default() }.into(),
                ..Default::default()
            },
        )
        .await;
    assert_eq!((changed.class_soon_minutes, changed.join_ranking, changed.daily_goal_minutes), (10, true, 120));
    assert_eq!(changed.pomodoro.as_option().unwrap().work_minutes, 50);
    assert_eq!(changed.pomodoro.as_option().unwrap().break_minutes, 5, "lengths left out keep their value");
    // A second change leaves the first alone.
    let again = ctx
        .update_settings(
            &me,
            UpdateSchoolSettingsRequest {
                share_study_activity: Some(false),
                assignment_reminders: Some(false),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(
        (again.class_soon_minutes, again.share_study_activity, again.assignment_reminders, again.join_ranking),
        (10, false, false, true)
    );
    assert_eq!(settings_of(&ctx, &me).await, again);
    assert_eq!(settings_of(&ctx, &other).await, defaults, "settings are per account");

    for request in [
        UpdateSchoolSettingsRequest { class_soon_minutes: Some(121), ..Default::default() },
        UpdateSchoolSettingsRequest { class_soon_minutes: Some(-1), ..Default::default() },
        UpdateSchoolSettingsRequest { daily_goal_minutes: Some(1441), ..Default::default() },
        UpdateSchoolSettingsRequest {
            study_visibility: Some(StudyVisibility::STUDY_VISIBILITY_UNSPECIFIED.into()),
            ..Default::default()
        },
        UpdateSchoolSettingsRequest {
            pomodoro: PomodoroConfig { work_minutes: 999, ..Default::default() }.into(),
            ..Default::default()
        },
    ] {
        assert_eq!(
            ctx.reason(SETTINGS, "UpdateSchoolSettings", &me, &request).await,
            "SETTINGS_INVALID",
            "{request:?}"
        );
    }
    assert_eq!(settings_of(&ctx, &me).await, again, "a rejected change changes nothing");
}

async fn exam_of(ctx: &Ctx, user: &TestUser, exam_id: &str) -> Exam {
    let response: GetExamResponse =
        ctx.ok(EXAMS, "GetExam", user, &GetExamRequest { exam_id: exam_id.to_owned(), ..Default::default() }).await;
    response.exam.as_option().cloned().unwrap()
}

async fn create_exam(ctx: &Ctx, user: &TestUser, name: &str, start: &str, end: &str) -> Exam {
    let request =
        CreateExamRequest { name: name.into(), start_date: start.into(), end_date: end.into(), ..Default::default() };
    let response: CreateExamResponse = ctx.ok(EXAMS, "CreateExam", user, &request).await;
    response.exam.as_option().cloned().unwrap()
}

#[tokio::test]
async fn exams_count_down_and_keep_private_results() {
    let ctx = start().await;
    let (me, other) = (ctx.app.user("me").await, ctx.app.user("other").await);
    let (math, english) = (ctx.subject(&me, "数学").await, ctx.subject(&me, "英語").await);
    ctx.at(2026, 6, 20, 12, 0);

    let exam = create_exam(&ctx, &me, "1学期期末テスト", "2026-07-01", "2026-07-03").await;
    assert_eq!(
        (exam.status.as_known().unwrap(), exam.days_until_start, exam.days_left),
        (ExamStatus::EXAM_STATUS_UPCOMING, 11, 14)
    );
    assert!(!exam.show_on_profile && exam.subjects.is_empty());
    assert_eq!(
        create_exam(&ctx, &me, "模試", "2026-06-28", "").await.end_date,
        "2026-06-28",
        "one day when no end is given"
    );
    let create = |name: &str, start: &str, end: &str| CreateExamRequest {
        name: name.into(),
        start_date: start.into(),
        end_date: end.into(),
        ..Default::default()
    };
    for (request, reason) in [
        (create("", "2026-07-01", "2026-07-03"), "EXAM_NAME_INVALID"),
        (create("x", "2026-07-03", "2026-07-01"), "DATE_RANGE_INVALID"),
        (create("x", "2026-07-01", "2026-09-30"), "DATE_RANGE_INVALID"),
        (create("x", "July", ""), "INVALID_DATE"),
        (CreateExamRequest { memo: "x".repeat(501), ..create("x", "2026-07-01", "") }, "TEXT_TOO_LONG"),
    ] {
        assert_eq!(ctx.reason(EXAMS, "CreateExam", &me, &request).await, reason, "{request:?}");
    }

    // Subjects with their day, period and scope.
    let put = |fill: fn(&mut PutExamSubjectRequest)| {
        let mut request = PutExamSubjectRequest::default();
        fill(&mut request);
        request
    };
    let in_exam = |mut request: PutExamSubjectRequest, subject: &Subject| {
        request.exam_id = exam.id.clone();
        request.subject_id = subject.id.clone();
        request
    };
    let added: PutExamSubjectResponse = ctx
        .ok(
            EXAMS,
            "PutExamSubject",
            &me,
            &in_exam(
                put(|r| {
                    r.date = "2026-07-02".into();
                    r.period = 2;
                    r.scope = "教科書 p.1-80".into();
                }),
                &math,
            ),
        )
        .await;
    let math_test = added.exam_subject.as_option().cloned().unwrap();
    assert_eq!(
        (
            math_test.subject.as_option().unwrap().name.as_str(),
            math_test.date.as_str(),
            math_test.period,
            math_test.scope.as_str()
        ),
        ("数学", "2026-07-02", 2, "教科書 p.1-80")
    );
    assert!(math_test.result.as_option().is_none());
    let _: PutExamSubjectResponse = ctx.ok(EXAMS, "PutExamSubject", &me, &in_exam(put(|_| {}), &english)).await;
    let foreign = ctx.subject(&other, "他人").await;
    for (request, reason) in [
        (in_exam(put(|_| {}), &math), "EXAM_SUBJECT_EXISTS"),
        (in_exam(put(|r| r.date = "2026-07-04".into()), &english), "DATE_OUTSIDE_EXAM"),
        (in_exam(put(|r| r.period = 13), &english), "PERIOD_INVALID"),
        (in_exam(put(|_| {}), &foreign), "SUBJECT_NOT_FOUND"),
        (PutExamSubjectRequest { exam_id: exam.id.clone(), ..Default::default() }, "SUBJECT_NOT_FOUND"),
        (
            PutExamSubjectRequest { exam_subject_id: uuid::Uuid::now_v7().to_string(), ..in_exam(put(|_| {}), &math) },
            "EXAM_SUBJECT_NOT_FOUND",
        ),
        (
            PutExamSubjectRequest {
                exam_id: uuid::Uuid::now_v7().to_string(),
                subject_id: math.id.clone(),
                ..Default::default()
            },
            "EXAM_NOT_FOUND",
        ),
    ] {
        assert_eq!(ctx.reason(EXAMS, "PutExamSubject", &me, &request).await, reason, "{request:?}");
    }
    assert_eq!(ctx.reason(EXAMS, "PutExamSubject", &other, &in_exam(put(|_| {}), &foreign)).await, "EXAM_NOT_FOUND");
    // Changing one: the test moves to the first day.
    let moved: PutExamSubjectResponse = ctx
        .ok(
            EXAMS,
            "PutExamSubject",
            &me,
            &PutExamSubjectRequest {
                exam_id: exam.id.clone(),
                exam_subject_id: math_test.id.clone(),
                date: "2026-07-01".into(),
                period: 1,
                ..Default::default()
            },
        )
        .await;
    let moved = moved.exam_subject.as_option().unwrap();
    assert_eq!(
        (moved.id.as_str(), moved.date.as_str(), moved.period, moved.subject.as_option().unwrap().name.as_str()),
        (math_test.id.as_str(), "2026-07-01", 1, "数学")
    );

    // A private result, recorded and cleared.
    let result = |fill: fn(&mut ExamResult)| {
        let mut result = ExamResult::default();
        fill(&mut result);
        SetExamResultRequest { exam_subject_id: math_test.id.clone(), result: result.into(), ..Default::default() }
    };
    let recorded: SetExamResultResponse = ctx
        .ok(
            EXAMS,
            "SetExamResult",
            &me,
            &result(|r| {
                r.score = Some(82);
                r.average = Some(61.5);
                r.rank = Some(12);
                r.memo = "計算ミス2問".into();
            }),
        )
        .await;
    let stored = recorded.exam_subject.as_option().unwrap().result.as_option().cloned().unwrap();
    assert_eq!(
        (stored.score, stored.max_score, stored.average, stored.rank, stored.memo.as_str()),
        (Some(82), 100, Some(61.5), Some(12), "計算ミス2問")
    );
    for request in [
        result(|r| r.score = Some(101)),
        result(|r| r.max_score = 20_000),
        result(|r| r.average = Some(-1.0)),
        result(|r| r.rank = Some(0)),
        result(|r| r.score = Some(-5)),
    ] {
        assert_eq!(ctx.reason(EXAMS, "SetExamResult", &me, &request).await, "RESULT_INVALID", "{request:?}");
    }
    assert_eq!(
        ctx.reason(EXAMS, "SetExamResult", &other, &result(|r| r.score = Some(1))).await,
        "EXAM_SUBJECT_NOT_FOUND"
    );

    // The exam with its subjects: by day, tests without a day last.
    let loaded = exam_of(&ctx, &me, &exam.id).await;
    assert_eq!(
        loaded
            .subjects
            .iter()
            .map(|s| (s.subject.as_option().unwrap().name.clone(), s.date.clone()))
            .collect::<Vec<_>>(),
        vec![("数学".to_owned(), "2026-07-01".to_owned()), ("英語".to_owned(), String::new())]
    );
    assert_eq!(loaded.subjects[0].result.as_option().unwrap().score, Some(82));
    assert_eq!(
        ctx.reason(EXAMS, "GetExam", &other, &GetExamRequest { exam_id: exam.id.clone(), ..Default::default() }).await,
        "EXAM_NOT_FOUND"
    );
    let cleared: SetExamResultResponse = ctx
        .ok(
            EXAMS,
            "SetExamResult",
            &me,
            &SetExamResultRequest { exam_subject_id: math_test.id.clone(), ..Default::default() },
        )
        .await;
    assert!(cleared.exam_subject.as_option().unwrap().result.as_option().is_none());

    // Changing the exam; its dates must keep covering the tests.
    let update = |exam: &Exam| UpdateExamRequest { exam_id: exam.id.clone(), ..Default::default() };
    let renamed: UpdateExamResponse = ctx
        .ok(
            EXAMS,
            "UpdateExam",
            &me,
            &UpdateExamRequest {
                name: Some("期末".into()),
                memo: Some("提出物も".into()),
                end_date: Some("2026-07-06".into()),
                ..update(&exam)
            },
        )
        .await;
    let renamed = renamed.exam.as_option().unwrap();
    assert_eq!(
        (
            renamed.name.as_str(),
            renamed.memo.as_str(),
            renamed.end_date.as_str(),
            renamed.days_left,
            renamed.subjects.len()
        ),
        ("期末", "提出物も", "2026-07-06", 17, 2)
    );
    for (user, request, reason) in [
        (&me, UpdateExamRequest { start_date: Some("2026-07-02".into()), ..update(&exam) }, "DATE_OUTSIDE_EXAM"),
        (&me, UpdateExamRequest { end_date: Some("2026-06-30".into()), ..update(&exam) }, "DATE_RANGE_INVALID"),
        (&me, UpdateExamRequest { name: Some(" ".into()), ..update(&exam) }, "EXAM_NAME_INVALID"),
        (&other, UpdateExamRequest { name: Some("x".into()), ..update(&exam) }, "EXAM_NOT_FOUND"),
    ] {
        assert_eq!(ctx.reason(EXAMS, "UpdateExam", user, &request).await, reason, "{request:?}");
    }

    // The list: what is coming first, soonest first; then what is over, latest first.
    create_exam(&ctx, &me, "中間テスト", "2026-05-20", "2026-05-22").await;
    create_exam(&ctx, &me, "学年末テスト", "2026-03-01", "2026-03-03").await;
    let listed: ListExamsResponse = ctx.ok(EXAMS, "ListExams", &me, &ListExamsRequest::default()).await;
    assert_eq!(
        listed.exams.iter().map(|e| (e.name.as_str(), e.status.as_known().unwrap())).collect::<Vec<_>>(),
        vec![
            ("模試", ExamStatus::EXAM_STATUS_UPCOMING),
            ("期末", ExamStatus::EXAM_STATUS_UPCOMING),
            ("中間テスト", ExamStatus::EXAM_STATUS_FINISHED),
            ("学年末テスト", ExamStatus::EXAM_STATUS_FINISHED),
        ]
    );
    assert_eq!(listed.exams[1].subjects.len(), 2, "listed with their subjects");
    let theirs: ListExamsResponse = ctx.ok(EXAMS, "ListExams", &other, &ListExamsRequest::default()).await;
    assert!(theirs.exams.is_empty());
    // During the exam.
    ctx.at(2026, 7, 2, 8, 0);
    let during = exam_of(&ctx, &me, &exam.id).await;
    assert_eq!(
        (during.status.as_known().unwrap(), during.days_until_start, during.days_left),
        (ExamStatus::EXAM_STATUS_ONGOING, 0, 5)
    );

    // Removing a subject, then the exam.
    let delete_subject = DeleteExamSubjectRequest { exam_subject_id: math_test.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(EXAMS, "DeleteExamSubject", &other, &delete_subject).await, "EXAM_SUBJECT_NOT_FOUND");
    let _: DeleteExamSubjectResponse = ctx.ok(EXAMS, "DeleteExamSubject", &me, &delete_subject).await;
    assert_eq!(ctx.reason(EXAMS, "DeleteExamSubject", &me, &delete_subject).await, "EXAM_SUBJECT_NOT_FOUND");
    assert_eq!(exam_of(&ctx, &me, &exam.id).await.subjects.len(), 1);
    let delete = DeleteExamRequest { exam_id: exam.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(EXAMS, "DeleteExam", &other, &delete).await, "EXAM_NOT_FOUND");
    let _: DeleteExamResponse = ctx.ok(EXAMS, "DeleteExam", &me, &delete).await;
    assert_eq!(ctx.reason(EXAMS, "DeleteExam", &me, &delete).await, "EXAM_NOT_FOUND");
    assert_eq!(
        ctx.reason(EXAMS, "GetExam", &me, &GetExamRequest { exam_id: exam.id.clone(), ..Default::default() }).await,
        "EXAM_NOT_FOUND"
    );
}

#[tokio::test]
async fn the_exam_period_shows_on_the_profile_when_asked_to() {
    let ctx = start().await;
    let (me, friend, stranger) =
        (ctx.app.user("me").await, ctx.app.user("friend").await, ctx.app.user("stranger").await);
    ctx.befriend(&me, &friend).await;
    ctx.at(2026, 6, 20, 12, 0);
    let exam = create_exam(&ctx, &me, "期末テスト", "2026-07-01", "2026-07-03").await;
    assert!(ctx.profile_card(&friend, &me, "school.exam").await.is_none(), "not shown unless asked to");

    let shown: UpdateExamResponse = ctx
        .ok(
            EXAMS,
            "UpdateExam",
            &me,
            &UpdateExamRequest { exam_id: exam.id.clone(), show_on_profile: Some(true), ..Default::default() },
        )
        .await;
    assert!(shown.exam.as_option().unwrap().show_on_profile);
    let card = ctx.profile_card(&friend, &me, "school.exam").await.expect("friends see the countdown");
    assert_eq!(
        (card.title.as_option().unwrap().ja.as_str(), card.text.as_option().unwrap().ja.as_str()),
        ("テスト期間", "期末テスト · あと11日")
    );
    assert_eq!(
        (card.data["status"].as_str(), card.data["days_until_start"].as_str(), card.path.as_str()),
        ("upcoming", "11", "")
    );
    assert!(ctx.profile_card(&stranger, &me, "school.exam").await.is_none());
    assert_eq!(
        ctx.profile_card(&me, &me, "school.exam").await.unwrap().path,
        format!("/school/exams/{}", exam.id),
        "the owner's card leads to the exam"
    );

    // More than two weeks ahead it is too early; during the exam it says so; afterwards it is gone.
    ctx.at(2026, 6, 16, 12, 0);
    assert!(ctx.profile_card(&friend, &me, "school.exam").await.is_none());
    ctx.at(2026, 7, 2, 12, 0);
    let card = ctx.profile_card(&friend, &me, "school.exam").await.unwrap();
    assert_eq!(
        (card.text.as_option().unwrap().ja.as_str(), card.data["status"].as_str()),
        ("期末テスト · テスト期間中（あと2日）", "ongoing")
    );
    ctx.at(2026, 7, 4, 12, 0);
    assert!(ctx.profile_card(&friend, &me, "school.exam").await.is_none());
}

async fn assignments(ctx: &Ctx, user: &TestUser, request: ListAssignmentsRequest) -> ListAssignmentsResponse {
    ctx.ok(ASSIGNMENTS, "ListAssignments", user, &request).await
}

async fn create_assignment(ctx: &Ctx, user: &TestUser, request: CreateAssignmentRequest) -> Assignment {
    let response: CreateAssignmentResponse = ctx.ok(ASSIGNMENTS, "CreateAssignment", user, &request).await;
    response.assignment.as_option().cloned().unwrap()
}

fn titles(response: &ListAssignmentsResponse) -> Vec<&str> {
    response.assignments.iter().map(|a| a.title.as_str()).collect()
}

#[tokio::test]
async fn assignments_are_listed_by_urgency() {
    let ctx = start().await;
    let (me, other) = (ctx.app.user("me").await, ctx.app.user("other").await);
    let math = ctx.subject(&me, "数学").await;
    ctx.at(2026, 6, 8, 12, 0); // a Monday
    let new = |title: &str, due: chrono::DateTime<chrono::Utc>| CreateAssignmentRequest {
        title: title.into(),
        due_at: stamp(due),
        ..Default::default()
    };

    let report = create_assignment(
        &ctx,
        &me,
        CreateAssignmentRequest {
            subject_id: math.id.clone(),
            memo: "A4 2枚".into(),
            ..new("数学レポート", tokyo(2026, 6, 10, 9, 0))
        },
    )
    .await;
    assert_eq!(
        (report.title.as_str(), report.subject.as_option().unwrap().name.as_str(), report.memo.as_str()),
        ("数学レポート", "数学", "A4 2枚")
    );
    assert_eq!(
        (report.urgency.as_known().unwrap(), report.done, report.remind),
        (AssignmentUrgency::ASSIGNMENT_URGENCY_THIS_WEEK, false, true)
    );
    assert_eq!(instant(&report.due_at), tokyo(2026, 6, 10, 9, 0));
    create_assignment(&ctx, &me, new("昨日まで", tokyo(2026, 6, 7, 17, 0))).await;
    create_assignment(&ctx, &me, new("今日中", tokyo(2026, 6, 8, 18, 0))).await;
    create_assignment(&ctx, &me, new("明日の朝", tokyo(2026, 6, 9, 8, 0))).await;
    create_assignment(&ctx, &me, new("月末", tokyo(2026, 6, 30, 17, 0))).await;

    for (request, reason) in [
        (new(" ", tokyo(2026, 6, 10, 9, 0)), "ASSIGNMENT_TITLE_INVALID"),
        (new(&"あ".repeat(81), tokyo(2026, 6, 10, 9, 0)), "ASSIGNMENT_TITLE_INVALID"),
        (CreateAssignmentRequest { title: "x".into(), ..Default::default() }, "DUE_REQUIRED"),
        (new("x", tokyo(2029, 6, 10, 9, 0)), "DUE_INVALID"),
        (
            CreateAssignmentRequest {
                subject_id: uuid::Uuid::now_v7().to_string(),
                ..new("x", tokyo(2026, 6, 10, 9, 0))
            },
            "SUBJECT_NOT_FOUND",
        ),
        (CreateAssignmentRequest { memo: "x".repeat(501), ..new("x", tokyo(2026, 6, 10, 9, 0)) }, "TEXT_TOO_LONG"),
    ] {
        assert_eq!(ctx.reason(ASSIGNMENTS, "CreateAssignment", &me, &request).await, reason, "{request:?}");
    }

    // Open assignments, the most pressing first.
    let open = assignments(&ctx, &me, ListAssignmentsRequest::default()).await;
    assert_eq!(titles(&open), vec!["昨日まで", "今日中", "明日の朝", "数学レポート", "月末"]);
    use AssignmentUrgency::*;
    assert_eq!(
        open.assignments.iter().map(|a| a.urgency.as_known().unwrap()).collect::<Vec<_>>(),
        vec![
            ASSIGNMENT_URGENCY_OVERDUE,
            ASSIGNMENT_URGENCY_TODAY,
            ASSIGNMENT_URGENCY_TOMORROW,
            ASSIGNMENT_URGENCY_THIS_WEEK,
            ASSIGNMENT_URGENCY_LATER
        ]
    );
    assert_eq!((open.open_count, open.overdue_count), (5, 1));
    assert!(assignments(&ctx, &other, ListAssignmentsRequest::default()).await.assignments.is_empty());

    // Ticking one off moves it to the finished ones.
    let update = |assignment: &Assignment| UpdateAssignmentRequest {
        assignment_id: assignment.id.clone(),
        ..Default::default()
    };
    let late = open.assignments[0].clone();
    let done: UpdateAssignmentResponse = ctx
        .ok(ASSIGNMENTS, "UpdateAssignment", &me, &UpdateAssignmentRequest { done: Some(true), ..update(&late) })
        .await;
    let done = done.assignment.as_option().cloned().unwrap();
    assert_eq!(
        (done.done, done.urgency.as_known().unwrap(), instant(&done.done_at)),
        (true, ASSIGNMENT_URGENCY_DONE, tokyo(2026, 6, 8, 12, 0))
    );
    ctx.pass(1);
    let _: UpdateAssignmentResponse = ctx
        .ok(
            ASSIGNMENTS,
            "UpdateAssignment",
            &me,
            &UpdateAssignmentRequest { done: Some(true), ..update(&open.assignments[1]) },
        )
        .await;
    let open = assignments(&ctx, &me, ListAssignmentsRequest::default()).await;
    assert_eq!((titles(&open), open.open_count, open.overdue_count), (vec!["明日の朝", "数学レポート", "月末"], 3, 0));
    let finished = assignments(
        &ctx,
        &me,
        ListAssignmentsRequest { filter: AssignmentFilter::ASSIGNMENT_FILTER_DONE.into(), ..Default::default() },
    )
    .await;
    assert_eq!(titles(&finished), vec!["今日中", "昨日まで"], "most recently finished first");
    let all = assignments(
        &ctx,
        &me,
        ListAssignmentsRequest { filter: AssignmentFilter::ASSIGNMENT_FILTER_ALL.into(), ..Default::default() },
    )
    .await;
    assert_eq!(titles(&all), vec!["明日の朝", "数学レポート", "月末", "今日中", "昨日まで"]);

    // Pages run through the open ones into the finished ones.
    let page = |cursor: String| ListAssignmentsRequest {
        filter: AssignmentFilter::ASSIGNMENT_FILTER_ALL.into(),
        page: PageRequest { limit: 2, cursor, ..Default::default() }.into(),
        ..Default::default()
    };
    let mut seen = Vec::new();
    let mut cursor = String::new();
    for expected in [2, 2, 1] {
        let response = assignments(&ctx, &me, page(cursor.clone())).await;
        assert_eq!(response.assignments.len(), expected);
        seen.extend(response.assignments.iter().map(|a| a.title.clone()));
        cursor = response.page.as_option().unwrap().next_cursor.clone();
    }
    assert_eq!(seen, vec!["明日の朝", "数学レポート", "月末", "今日中", "昨日まで"]);
    assert!(cursor.is_empty());
    let of_math = assignments(
        &ctx,
        &me,
        ListAssignmentsRequest {
            subject_id: math.id.clone(),
            filter: AssignmentFilter::ASSIGNMENT_FILTER_ALL.into(),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(titles(&of_math), vec!["数学レポート"]);

    // Changing one, and undoing "done".
    let changed: UpdateAssignmentResponse = ctx
        .ok(
            ASSIGNMENTS,
            "UpdateAssignment",
            &me,
            &UpdateAssignmentRequest {
                title: Some("数学レポート（再提出）".into()),
                subject_id: Some("".into()),
                due_at: stamp(tokyo(2026, 6, 9, 9, 0)),
                memo: Some("".into()),
                remind: Some(false),
                ..update(&report)
            },
        )
        .await;
    let changed = changed.assignment.as_option().cloned().unwrap();
    assert_eq!(
        (changed.title.as_str(), changed.subject.as_option().is_none(), changed.memo.as_str(), changed.remind),
        ("数学レポート（再提出）", true, "", false)
    );
    assert_eq!(
        (instant(&changed.due_at), changed.urgency.as_known().unwrap()),
        (tokyo(2026, 6, 9, 9, 0), ASSIGNMENT_URGENCY_TOMORROW)
    );
    let reopened: UpdateAssignmentResponse = ctx
        .ok(ASSIGNMENTS, "UpdateAssignment", &me, &UpdateAssignmentRequest { done: Some(false), ..update(&late) })
        .await;
    let reopened = reopened.assignment.as_option().unwrap();
    assert_eq!(
        (reopened.done, reopened.done_at.as_option().is_none(), reopened.urgency.as_known().unwrap()),
        (false, true, ASSIGNMENT_URGENCY_OVERDUE)
    );
    for (user, request, reason) in [
        (&me, UpdateAssignmentRequest { title: Some("".into()), ..update(&report) }, "ASSIGNMENT_TITLE_INVALID"),
        (&me, UpdateAssignmentRequest { due_at: stamp(tokyo(2030, 1, 1, 0, 0)), ..update(&report) }, "DUE_INVALID"),
        (
            &me,
            UpdateAssignmentRequest { subject_id: Some(uuid::Uuid::now_v7().to_string()), ..update(&report) },
            "SUBJECT_NOT_FOUND",
        ),
        (&other, UpdateAssignmentRequest { done: Some(true), ..update(&report) }, "ASSIGNMENT_NOT_FOUND"),
    ] {
        assert_eq!(ctx.reason(ASSIGNMENTS, "UpdateAssignment", user, &request).await, reason, "{request:?}");
    }

    let delete = DeleteAssignmentRequest { assignment_id: report.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(ASSIGNMENTS, "DeleteAssignment", &other, &delete).await, "ASSIGNMENT_NOT_FOUND");
    let _: DeleteAssignmentResponse = ctx.ok(ASSIGNMENTS, "DeleteAssignment", &me, &delete).await;
    assert_eq!(ctx.reason(ASSIGNMENTS, "DeleteAssignment", &me, &delete).await, "ASSIGNMENT_NOT_FOUND");
    assert_eq!(assignments(&ctx, &me, ListAssignmentsRequest::default()).await.open_count, 3);
}
