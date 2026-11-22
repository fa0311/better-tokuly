//! `TimetableService`: subjects, terms, the weekly grid, one-off changes,
//! day overrides, and what a day resolves to. (Sharing is in `tests/sharing.rs`.)
mod common;

use common::*;
use tokuly_proto::school::*;

use ClassStatus::*;

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[tokio::test]
async fn layout_presets_cover_high_school_and_university() {
    let ctx = start().await;
    let user = ctx.app.user("preset").await;
    let response: ListLayoutPresetsResponse =
        ctx.ok(TIMETABLE, "ListLayoutPresets", &user, &ListLayoutPresetsRequest::default()).await;
    assert_eq!(response.presets.len(), 6);
    let high_school = response.presets.iter().find(|p| p.preset == LayoutPreset::LAYOUT_PRESET_HIGH_SCHOOL_6).unwrap();
    let layout = high_school.layout.as_option().unwrap();
    assert_eq!(layout.periods.len(), 6);
    assert_eq!((layout.periods[0].start_minute, layout.periods[0].end_minute), (8 * 60 + 50, 9 * 60 + 40));
    assert_eq!(layout.weekdays.len(), 5);
    assert!(!high_school.name.as_option().unwrap().ja.is_empty());
    let university = response.presets.iter().find(|p| p.preset == LayoutPreset::LAYOUT_PRESET_UNIVERSITY_90).unwrap();
    assert!(university.layout.as_option().unwrap().periods.iter().all(|p| p.end_minute - p.start_minute == 90));
}

#[tokio::test]
async fn subjects_are_a_private_list_with_unique_names() {
    let ctx = start().await;
    let (me, other) = (ctx.app.user("me").await, ctx.app.user("other").await);

    let math: CreateSubjectResponse = ctx
        .ok(
            TIMETABLE,
            "CreateSubject",
            &me,
            &CreateSubjectRequest {
                name: " 数学Ⅱ ".into(),
                short_name: "数Ⅱ".into(),
                teacher: "田中".into(),
                room: "2-A".into(),
                emoji: "📐".into(),
                ..Default::default()
            },
        )
        .await;
    let math = math.subject.as_option().cloned().unwrap();
    assert_eq!(
        (math.name.as_str(), math.short_name.as_str(), math.teacher.as_str(), math.room.as_str()),
        ("数学Ⅱ", "数Ⅱ", "田中", "2-A")
    );
    assert!(
        math.color.starts_with('#') && math.color.len() == 7,
        "a colour is picked from the palette: {}",
        math.color
    );
    let english = ctx.subject(&me, "English").await;
    assert_ne!(english.color, math.color, "the palette rotates");

    for (request, reason) in [
        (CreateSubjectRequest { name: "english".into(), ..Default::default() }, "SUBJECT_NAME_TAKEN"),
        (CreateSubjectRequest { name: "  ".into(), ..Default::default() }, "SUBJECT_NAME_INVALID"),
        (CreateSubjectRequest { name: "あ".repeat(41), ..Default::default() }, "SUBJECT_NAME_INVALID"),
        (CreateSubjectRequest { name: "色".into(), color: "red".into(), ..Default::default() }, "COLOR_INVALID"),
        (CreateSubjectRequest { name: "長".into(), short_name: "あ".repeat(9), ..Default::default() }, "TEXT_TOO_LONG"),
        (CreateSubjectRequest { name: "絵".into(), emoji: "ab".into(), ..Default::default() }, "TEXT_TOO_LONG"),
    ] {
        assert_eq!(ctx.reason(TIMETABLE, "CreateSubject", &me, &request).await, reason, "{request:?}");
    }
    // Names are unique per account, not across accounts.
    ctx.subject(&other, "English").await;

    let update = |subject: &Subject| UpdateSubjectRequest { subject_id: subject.id.clone(), ..Default::default() };
    let renamed: UpdateSubjectResponse = ctx
        .ok(
            TIMETABLE,
            "UpdateSubject",
            &me,
            &UpdateSubjectRequest {
                name: Some("英語".into()),
                color: Some("#112233".into()),
                room: Some("LL教室".into()),
                ..update(&english)
            },
        )
        .await;
    let renamed = renamed.subject.as_option().unwrap();
    assert_eq!((renamed.name.as_str(), renamed.color.as_str(), renamed.room.as_str()), ("英語", "#112233", "LL教室"));
    assert_eq!(
        ctx.reason(
            TIMETABLE,
            "UpdateSubject",
            &me,
            &UpdateSubjectRequest { name: Some("数学Ⅱ".into()), ..update(&english) }
        )
        .await,
        "SUBJECT_NAME_TAKEN"
    );
    assert_eq!(
        ctx.reason(
            TIMETABLE,
            "UpdateSubject",
            &me,
            &UpdateSubjectRequest { color: Some("".into()), ..update(&english) }
        )
        .await,
        "COLOR_INVALID"
    );
    assert_eq!(
        ctx.reason(
            TIMETABLE,
            "UpdateSubject",
            &other,
            &UpdateSubjectRequest { name: Some("x".into()), ..update(&english) }
        )
        .await,
        "SUBJECT_NOT_FOUND"
    );

    // Archived subjects leave the picker but stay available on request.
    let _: UpdateSubjectResponse =
        ctx.ok(TIMETABLE, "UpdateSubject", &me, &UpdateSubjectRequest { archived: Some(true), ..update(&math) }).await;
    let listed: ListSubjectsResponse = ctx.ok(TIMETABLE, "ListSubjects", &me, &ListSubjectsRequest::default()).await;
    assert_eq!(listed.subjects.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["英語"]);
    let all: ListSubjectsResponse = ctx
        .ok(TIMETABLE, "ListSubjects", &me, &ListSubjectsRequest { include_archived: true, ..Default::default() })
        .await;
    assert_eq!(all.subjects.len(), 2);
    let theirs: ListSubjectsResponse = ctx.ok(TIMETABLE, "ListSubjects", &other, &ListSubjectsRequest::default()).await;
    assert_eq!(theirs.subjects.len(), 1);
}

#[tokio::test]
async fn deleting_a_subject_clears_timetables_and_keeps_history() {
    let ctx = start().await;
    let (me, other) = (ctx.app.user("me").await, ctx.app.user("other").await);
    let (math, art) = (ctx.subject(&me, "数学").await, ctx.subject(&me, "美術").await);
    let term = ctx.term(&me, |_| {}).await;
    ctx.put(
        &me,
        &term,
        vec![
            class(Weekday::WEEKDAY_MONDAY, 1, &math),
            class(Weekday::WEEKDAY_TUESDAY, 2, &math),
            class(Weekday::WEEKDAY_MONDAY, 2, &art),
        ],
    )
    .await;

    let delete = |subject: &Subject| DeleteSubjectRequest { subject_id: subject.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(TIMETABLE, "DeleteSubject", &other, &delete(&math)).await, "SUBJECT_NOT_FOUND");

    // Art is used by the timetable only: it goes for good, with its class.
    let gone: DeleteSubjectResponse = ctx.ok(TIMETABLE, "DeleteSubject", &me, &delete(&art)).await;
    assert_eq!((gone.archived, gone.removed_entry_count), (false, 1));

    // Math has study time logged: it leaves the timetable but is kept, archived.
    ctx.at(2026, 4, 13, 20, 0);
    ctx.log(&me, tokyo(2026, 4, 13, 18, 0), tokyo(2026, 4, 13, 19, 0), Some(&math)).await.unwrap();
    let kept: DeleteSubjectResponse = ctx.ok(TIMETABLE, "DeleteSubject", &me, &delete(&math)).await;
    assert_eq!((kept.archived, kept.removed_entry_count), (true, 2));
    let term: GetTermResponse =
        ctx.ok(TIMETABLE, "GetTerm", &me, &GetTermRequest { term_id: term.id.clone(), ..Default::default() }).await;
    assert!(term.entries.is_empty());
    let all: ListSubjectsResponse = ctx
        .ok(TIMETABLE, "ListSubjects", &me, &ListSubjectsRequest { include_archived: true, ..Default::default() })
        .await;
    assert_eq!(all.subjects.iter().map(|s| (s.name.as_str(), s.archived)).collect::<Vec<_>>(), vec![("数学", true)]);
    assert_eq!(ctx.sessions(&me).await[0].subject.as_option().unwrap().name, "数学", "the log still names the subject");
}

#[tokio::test]
async fn terms_have_one_current_and_are_checked() {
    let ctx = start().await;
    let (me, friend, stranger) =
        (ctx.app.user("me").await, ctx.app.user("friend").await, ctx.app.user("stranger").await);
    ctx.befriend(&me, &friend).await;

    let first = ctx.term(&me, |_| {}).await;
    assert!(first.current, "the first term is the current one");
    assert_eq!(first.audience, TimetableAudience::TIMETABLE_AUDIENCE_NOBODY, "private by default");
    assert!(first.observe_public_holidays && !first.alternating_weeks);
    assert_eq!(first.layout.as_option().unwrap().preset, LayoutPreset::LAYOUT_PRESET_HIGH_SCHOOL_6);
    assert_eq!(first.owner_id, me.id.to_string());

    let second = ctx
        .term(&me, |term| {
            term.name = "2学期".into();
            term.start_date = "2026-09-01".into();
            term.end_date = "2026-12-24".into();
            term.layout = PeriodLayout {
                preset: LayoutPreset::LAYOUT_PRESET_UNIVERSITY_90.into(),
                weekdays: vec![Weekday::WEEKDAY_SATURDAY.into(), Weekday::WEEKDAY_MONDAY.into()],
                ..Default::default()
            }
            .into();
            term.alternating_weeks = true;
            term.week_a_date = "2026-09-01".into();
            term.audience = TimetableAudience::TIMETABLE_AUDIENCE_CHOSEN.into();
            term.shared_with_user_ids = vec![friend.id.to_string()];
        })
        .await;
    assert!(!second.current);
    assert_eq!(second.layout.as_option().unwrap().periods.len(), 5);
    assert_eq!(second.layout.as_option().unwrap().weekdays, vec![Weekday::WEEKDAY_MONDAY, Weekday::WEEKDAY_SATURDAY]);
    assert_eq!(second.shared_with.iter().map(|u| u.id.clone()).collect::<Vec<_>>(), vec![friend.id.to_string()]);

    let create = |fill: fn(&mut CreateTermRequest)| {
        let mut request = CreateTermRequest {
            name: "x".into(),
            start_date: "2026-04-01".into(),
            end_date: "2026-07-01".into(),
            ..Default::default()
        };
        fill(&mut request);
        request
    };
    let stranger_id = stranger.id.to_string();
    for (request, reason) in [
        (create(|t| t.name = "".into()), "TERM_NAME_INVALID"),
        (create(|t| t.end_date = "2026-03-01".into()), "DATE_RANGE_INVALID"),
        (create(|t| t.end_date = "2027-06-01".into()), "DATE_RANGE_INVALID"),
        (create(|t| t.start_date = "April".into()), "INVALID_DATE"),
        (create(|t| t.alternating_weeks = true), "WEEK_A_DATE_REQUIRED"),
        (
            create(|t| {
                t.layout = PeriodLayout {
                    periods: vec![Period { start_minute: 600, end_minute: 500, ..Default::default() }],
                    ..Default::default()
                }
                .into()
            }),
            "LAYOUT_INVALID",
        ),
        (CreateTermRequest { shared_with_user_ids: vec![stranger_id.clone()], ..create(|_| {}) }, "NOT_FRIENDS"),
        (CreateTermRequest { shared_with_user_ids: strings(&["nope"]), ..create(|_| {}) }, "INVALID_ID"),
    ] {
        assert_eq!(ctx.reason(TIMETABLE, "CreateTerm", &me, &request).await, reason, "{request:?}");
    }

    // Making another term current takes the flag from the first.
    let update = |term: &Term| UpdateTermRequest { term_id: term.id.clone(), ..Default::default() };
    let updated: UpdateTermResponse = ctx
        .ok(
            TIMETABLE,
            "UpdateTerm",
            &me,
            &UpdateTermRequest { current: Some(true), name: Some("後期".into()), ..update(&second) },
        )
        .await;
    assert!(updated.term.as_option().unwrap().current);
    let listed: ListTermsResponse = ctx.ok(TIMETABLE, "ListTerms", &me, &ListTermsRequest::default()).await;
    assert_eq!(
        listed.terms.iter().map(|t| (t.name.as_str(), t.current)).collect::<Vec<_>>(),
        vec![("後期", true), ("1学期", false)],
        "newest first"
    );

    assert_eq!(
        ctx.reason(
            TIMETABLE,
            "UpdateTerm",
            &me,
            &UpdateTermRequest { end_date: Some("2026-08-01".into()), ..update(&second) }
        )
        .await,
        "DATE_RANGE_INVALID"
    );
    assert_eq!(
        ctx.reason(
            TIMETABLE,
            "UpdateTerm",
            &me,
            &UpdateTermRequest { week_a_date: Some("".into()), ..update(&second) }
        )
        .await,
        "WEEK_A_DATE_REQUIRED"
    );
    assert_eq!(
        ctx.reason(
            TIMETABLE,
            "UpdateTerm",
            &me,
            &UpdateTermRequest {
                shared_with: UserIdList { user_ids: vec![stranger_id], ..Default::default() }.into(),
                ..update(&second)
            }
        )
        .await,
        "NOT_FRIENDS"
    );
    // Someone else's term does not exist.
    assert_eq!(
        ctx.reason(TIMETABLE, "UpdateTerm", &friend, &UpdateTermRequest { name: Some("x".into()), ..update(&second) })
            .await,
        "TERM_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason(TIMETABLE, "GetTerm", &friend, &GetTermRequest { term_id: second.id.clone(), ..Default::default() })
            .await,
        "TERM_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason(
            TIMETABLE,
            "DeleteTerm",
            &friend,
            &DeleteTermRequest { term_id: second.id.clone(), ..Default::default() }
        )
        .await,
        "TERM_NOT_FOUND"
    );

    // Deleting the current term hands the flag to the newest one left.
    let _: DeleteTermResponse = ctx
        .ok(TIMETABLE, "DeleteTerm", &me, &DeleteTermRequest { term_id: second.id.clone(), ..Default::default() })
        .await;
    let listed: ListTermsResponse = ctx.ok(TIMETABLE, "ListTerms", &me, &ListTermsRequest::default()).await;
    assert_eq!(listed.terms.iter().map(|t| (t.name.as_str(), t.current)).collect::<Vec<_>>(), vec![("1学期", true)]);
}

#[tokio::test]
async fn the_grid_holds_one_class_per_slot_and_week() {
    let ctx = start().await;
    let (me, other) = (ctx.app.user("me").await, ctx.app.user("other").await);
    let (math, english, art) =
        (ctx.subject(&me, "数学").await, ctx.subject(&me, "英語").await, ctx.subject(&me, "美術").await);
    let term = ctx
        .term(&me, |term| {
            term.alternating_weeks = true;
            term.week_a_date = "2026-04-08".into();
        })
        .await;

    let stored = ctx
        .put(
            &me,
            &term,
            vec![
                TimetableEntryInput {
                    teacher: "佐藤".into(),
                    room: "3-B".into(),
                    memo: "小テストあり".into(),
                    color: "#ABCDEF".into(),
                    ..class(Weekday::WEEKDAY_MONDAY, 1, &math)
                },
                class(Weekday::WEEKDAY_MONDAY, 2, &english),
            ],
        )
        .await;
    assert_eq!(stored.entries.len(), 2);
    assert!(stored.replaced_entry_ids.is_empty());
    let first = &stored.entries[0];
    assert_eq!(
        (first.teacher.as_str(), first.room.as_str(), first.memo.as_str(), first.color.as_str()),
        ("佐藤", "3-B", "小テストあり", "#abcdef")
    );
    assert_eq!(first.week_pattern, WeekPattern::WEEK_PATTERN_EVERY);

    // A week-A class displaces the every-week class of its slot; week B then fills the other half.
    let week = |pattern: WeekPattern, subject: &Subject| TimetableEntryInput {
        week_pattern: pattern.into(),
        ..class(Weekday::WEEKDAY_MONDAY, 1, subject)
    };
    let a = ctx.put(&me, &term, vec![week(WeekPattern::WEEK_PATTERN_A, &art)]).await;
    assert_eq!(a.replaced_entry_ids, vec![first.id.clone()]);
    let b = ctx.put(&me, &term, vec![week(WeekPattern::WEEK_PATTERN_B, &math)]).await;
    assert!(b.replaced_entry_ids.is_empty());
    let loaded: GetTermResponse =
        ctx.ok(TIMETABLE, "GetTerm", &me, &GetTermRequest { term_id: term.id.clone(), ..Default::default() }).await;
    assert_eq!(loaded.entries.len(), 3);
    assert_eq!(loaded.term.as_option().unwrap().entry_count, 3);
    // An every-week class displaces both halves.
    let every = ctx.put(&me, &term, vec![class(Weekday::WEEKDAY_MONDAY, 1, &english)]).await;
    assert_eq!(every.replaced_entry_ids.len(), 2);

    let put = |entries: Vec<TimetableEntryInput>| PutTimetableEntriesRequest {
        term_id: term.id.clone(),
        entries,
        ..Default::default()
    };
    let foreign = ctx.subject(&other, "他人の科目").await;
    for (request, reason) in [
        (put(vec![]), "NO_ENTRIES"),
        (put(vec![class(Weekday::WEEKDAY_SATURDAY, 1, &math)]), "SLOT_INVALID"),
        (put(vec![class(Weekday::WEEKDAY_MONDAY, 7, &math)]), "SLOT_INVALID"),
        (put(vec![class(Weekday::WEEKDAY_UNSPECIFIED, 1, &math)]), "SLOT_INVALID"),
        (put(vec![class(Weekday::WEEKDAY_FRIDAY, 1, &math), class(Weekday::WEEKDAY_FRIDAY, 1, &art)]), "SLOT_INVALID"),
        (
            put(vec![
                class(Weekday::WEEKDAY_FRIDAY, 1, &math),
                week(WeekPattern::WEEK_PATTERN_A, &art),
                TimetableEntryInput {
                    weekday: Weekday::WEEKDAY_FRIDAY.into(),
                    ..week(WeekPattern::WEEK_PATTERN_A, &art)
                },
            ]),
            "SLOT_INVALID",
        ),
        (put(vec![class(Weekday::WEEKDAY_FRIDAY, 1, &foreign)]), "SUBJECT_NOT_FOUND"),
        (
            put(vec![TimetableEntryInput { color: "blue".into(), ..class(Weekday::WEEKDAY_FRIDAY, 1, &math) }]),
            "COLOR_INVALID",
        ),
        (
            put(vec![TimetableEntryInput { room: "x".repeat(41), ..class(Weekday::WEEKDAY_FRIDAY, 1, &math) }]),
            "TEXT_TOO_LONG",
        ),
        (put((0..101).map(|_| class(Weekday::WEEKDAY_FRIDAY, 1, &math)).collect()), "TOO_MANY_ENTRIES"),
    ] {
        assert_eq!(ctx.reason(TIMETABLE, "PutTimetableEntries", &me, &request).await, reason);
    }
    assert_eq!(
        ctx.reason(TIMETABLE, "PutTimetableEntries", &other, &put(vec![class(Weekday::WEEKDAY_FRIDAY, 1, &foreign)]))
            .await,
        "TERM_NOT_FOUND"
    );
    // Nothing of a rejected call is stored.
    let loaded: GetTermResponse =
        ctx.ok(TIMETABLE, "GetTerm", &me, &GetTermRequest { term_id: term.id.clone(), ..Default::default() }).await;
    assert_eq!(loaded.entries.len(), 2);

    // A/B classes need a term that alternates.
    let plain = ctx.term(&me, |term| term.name = "夏期講習".into()).await;
    let request = PutTimetableEntriesRequest {
        term_id: plain.id.clone(),
        entries: vec![week(WeekPattern::WEEK_PATTERN_A, &art)],
        ..Default::default()
    };
    assert_eq!(ctx.reason(TIMETABLE, "PutTimetableEntries", &me, &request).await, "WEEK_PATTERN_INVALID");

    let delete = DeleteTimetableEntriesRequest {
        term_id: term.id.clone(),
        entry_ids: vec![every.entries[0].id.clone(), uuid::Uuid::now_v7().to_string()],
        ..Default::default()
    };
    assert_eq!(ctx.reason(TIMETABLE, "DeleteTimetableEntries", &other, &delete).await, "TERM_NOT_FOUND");
    let deleted: DeleteTimetableEntriesResponse = ctx.ok(TIMETABLE, "DeleteTimetableEntries", &me, &delete).await;
    assert_eq!(deleted.deleted_count, 1, "unknown IDs are ignored");
}

#[tokio::test]
async fn changing_a_terms_shape_fits_its_contents() {
    let ctx = start().await;
    let me = ctx.app.user("me").await;
    let (math, art) = (ctx.subject(&me, "数学").await, ctx.subject(&me, "美術").await);
    let term = ctx
        .term(&me, |term| {
            term.alternating_weeks = true;
            term.week_a_date = "2026-04-08".into();
        })
        .await;
    let week = |pattern: WeekPattern, period: i32, subject: &Subject| TimetableEntryInput {
        week_pattern: pattern.into(),
        ..class(Weekday::WEEKDAY_MONDAY, period, subject)
    };
    ctx.put(
        &me,
        &term,
        vec![
            week(WeekPattern::WEEK_PATTERN_A, 1, &math),
            week(WeekPattern::WEEK_PATTERN_B, 1, &art),
            week(WeekPattern::WEEK_PATTERN_B, 2, &art),
            class(Weekday::WEEKDAY_FRIDAY, 6, &math),
        ],
    )
    .await;
    let pin = CreateDayOverrideRequest {
        term_id: term.id.clone(),
        start_date: "2026-04-13".into(),
        end_date: "2026-04-17".into(),
        kind: DayOverrideKind::DAY_OVERRIDE_KIND_WEEK_PATTERN.into(),
        week_pattern: WeekPattern::WEEK_PATTERN_A.into(),
        ..Default::default()
    };
    let _: CreateDayOverrideResponse = ctx.ok(TIMETABLE, "CreateDayOverride", &me, &pin).await;
    let update = || UpdateTermRequest { term_id: term.id.clone(), ..Default::default() };

    // One timetable for every week again: week A's class wins the shared
    // slot, week B's lone class stays, the pinned week is forgotten.
    let flat: UpdateTermResponse =
        ctx.ok(TIMETABLE, "UpdateTerm", &me, &UpdateTermRequest { alternating_weeks: Some(false), ..update() }).await;
    assert_eq!(flat.removed_entry_count, 1);
    let loaded: GetTermResponse =
        ctx.ok(TIMETABLE, "GetTerm", &me, &GetTermRequest { term_id: term.id.clone(), ..Default::default() }).await;
    let grid: Vec<(Weekday, i32, String, WeekPattern)> = loaded
        .entries
        .iter()
        .map(|e| {
            (
                e.weekday.as_known().unwrap(),
                e.period,
                e.subject.as_option().unwrap().name.clone(),
                e.week_pattern.as_known().unwrap(),
            )
        })
        .collect();
    use WeekPattern::WEEK_PATTERN_EVERY as EVERY;
    assert_eq!(
        grid,
        vec![
            (Weekday::WEEKDAY_MONDAY, 1, "数学".to_owned(), EVERY),
            (Weekday::WEEKDAY_MONDAY, 2, "美術".to_owned(), EVERY),
            (Weekday::WEEKDAY_FRIDAY, 6, "数学".to_owned(), EVERY),
        ]
    );
    assert!(loaded.day_overrides.is_empty());

    // A shorter day and a shorter week drop the classes that no longer fit.
    let shorter = PeriodLayout {
        periods: (0..4)
            .map(|i| Period { start_minute: 540 + i * 60, end_minute: 590 + i * 60, ..Default::default() })
            .collect(),
        weekdays: vec![Weekday::WEEKDAY_MONDAY.into(), Weekday::WEEKDAY_TUESDAY.into()],
        ..Default::default()
    };
    let shrunk: UpdateTermResponse =
        ctx.ok(TIMETABLE, "UpdateTerm", &me, &UpdateTermRequest { layout: shorter.into(), ..update() }).await;
    assert_eq!(shrunk.removed_entry_count, 1);
    let layout = shrunk.term.as_option().unwrap().layout.as_option().cloned().unwrap();
    assert_eq!((layout.preset.as_known().unwrap(), layout.periods.len()), (LayoutPreset::LAYOUT_PRESET_CUSTOM, 4));
    assert_eq!(shrunk.term.as_option().unwrap().entry_count, 2);
}

/// The class in progress and the next one, as `(date, period, subject)`.
async fn now(
    ctx: &Ctx,
    user: &tokuly_testkit::TestUser,
) -> (Option<(String, i32, String)>, Option<(String, i32, String)>) {
    let response: GetNowResponse = ctx.ok(TIMETABLE, "GetNow", user, &GetNowRequest::default()).await;
    let now = response.now.as_option().cloned().unwrap();
    let name = |class: Option<&ScheduledClass>| {
        class.map(|c| (c.date.clone(), c.period, c.subject.as_option().unwrap().name.clone()))
    };
    (name(now.current.as_option()), name(now.next.as_option()))
}

/// Monday: 1 math, 2 English; Tuesday: 1 PE. April 13, 2026 is a Monday.
async fn school_week(ctx: &Ctx, me: &tokuly_testkit::TestUser) -> (Term, Subject, Subject, Subject) {
    let (math, english, pe) =
        (ctx.subject(me, "数学").await, ctx.subject(me, "英語").await, ctx.subject(me, "体育").await);
    let term = ctx.term(me, |_| {}).await;
    ctx.put(
        me,
        &term,
        vec![
            TimetableEntryInput {
                room: "2-A".into(),
                memo: "ノート提出".into(),
                ..class(Weekday::WEEKDAY_MONDAY, 1, &math)
            },
            class(Weekday::WEEKDAY_MONDAY, 2, &english),
            class(Weekday::WEEKDAY_TUESDAY, 1, &pe),
        ],
    )
    .await;
    (term, math, english, pe)
}

#[tokio::test]
async fn a_day_is_the_grid_with_holidays_and_changes_applied() {
    let ctx = start().await;
    let (me, other) = (ctx.app.user("me").await, ctx.app.user("other").await);
    let (term, _math, _english, pe) = school_week(&ctx, &me).await;

    let monday = ctx.day(&me, "2026-04-13").await;
    assert_eq!(
        (monday.kind.as_known().unwrap(), monday.weekday.as_known().unwrap()),
        (DayKind::DAY_KIND_SCHOOL_DAY, Weekday::WEEKDAY_MONDAY)
    );
    assert_eq!((monday.term_id.as_str(), monday.term_name.as_str()), (term.id.as_str(), "1学期"));
    assert_eq!(
        classes(&monday),
        vec![(1, "数学".to_owned(), CLASS_STATUS_SCHEDULED), (2, "英語".to_owned(), CLASS_STATUS_SCHEDULED)]
    );
    assert_eq!(monday.periods.len(), 6);
    let first = &monday.classes[0];
    assert_eq!((first.room.as_str(), first.memo.as_str(), first.start_minute), ("2-A", "ノート提出", 8 * 60 + 50));
    assert_eq!(instant(&first.starts_at), tokyo(2026, 4, 13, 8, 50));
    assert_eq!(instant(&first.ends_at), tokyo(2026, 4, 13, 9, 40));

    // Weekend, public holiday, before the term.
    assert_eq!(ctx.day(&me, "2026-04-18").await.kind, DayKind::DAY_KIND_DAY_OFF);
    let showa = ctx.day(&me, "2026-04-29").await;
    assert_eq!(
        (showa.kind.as_known().unwrap(), showa.label.as_str(), showa.classes.len()),
        (DayKind::DAY_KIND_HOLIDAY, "昭和の日", 0)
    );
    let before = ctx.day(&me, "2026-04-01").await;
    assert_eq!(
        (before.kind.as_known().unwrap(), before.term_id.as_str(), before.periods.len()),
        (DayKind::DAY_KIND_OUT_OF_TERM, "", 0)
    );

    // One-off changes on Monday the 13th: math is cancelled, English moves
    // to Thursday's third period, PE is added in the fifth.
    let change = |date: &str, period: i32, kind: ChangeKind| CreateTimetableChangeRequest {
        term_id: term.id.clone(),
        date: date.into(),
        period,
        kind: kind.into(),
        ..Default::default()
    };
    let cancelled: CreateTimetableChangeResponse = ctx
        .ok(
            TIMETABLE,
            "CreateTimetableChange",
            &me,
            &CreateTimetableChangeRequest {
                memo: "先生出張".into(),
                ..change("2026-04-13", 1, ChangeKind::CHANGE_KIND_CANCELLED)
            },
        )
        .await;
    let cancelled = cancelled.change.as_option().cloned().unwrap();
    assert_eq!(
        (cancelled.kind.as_known().unwrap(), cancelled.memo.as_str()),
        (ChangeKind::CHANGE_KIND_CANCELLED, "先生出張")
    );
    let _: CreateTimetableChangeResponse = ctx
        .ok(
            TIMETABLE,
            "CreateTimetableChange",
            &me,
            &CreateTimetableChangeRequest {
                to_date: "2026-04-16".into(),
                to_period: 3,
                ..change("2026-04-13", 2, ChangeKind::CHANGE_KIND_MOVED)
            },
        )
        .await;
    let extra: CreateTimetableChangeResponse = ctx
        .ok(
            TIMETABLE,
            "CreateTimetableChange",
            &me,
            &CreateTimetableChangeRequest {
                subject_id: pe.id.clone(),
                room: "体育館".into(),
                ..change("2026-04-13", 5, ChangeKind::CHANGE_KIND_EXTRA)
            },
        )
        .await;
    assert_eq!(extra.change.as_option().unwrap().subject.as_option().unwrap().name, "体育");

    let monday = ctx.day(&me, "2026-04-13").await;
    assert_eq!(
        classes(&monday),
        vec![
            (1, "数学".to_owned(), CLASS_STATUS_CANCELLED),
            (2, "英語".to_owned(), CLASS_STATUS_MOVED_AWAY),
            (5, "体育".to_owned(), CLASS_STATUS_EXTRA)
        ]
    );
    assert_eq!(
        (monday.classes[0].memo.as_str(), monday.classes[0].change_id.as_str()),
        ("先生出張", cancelled.id.as_str())
    );
    assert_eq!((monday.classes[1].other_date.as_str(), monday.classes[1].other_period), ("2026-04-16", 3));
    assert_eq!(monday.classes[2].room, "体育館");
    let thursday = ctx.day(&me, "2026-04-16").await;
    assert_eq!(classes(&thursday), vec![(3, "英語".to_owned(), CLASS_STATUS_MOVED_HERE)]);
    assert_eq!(
        (thursday.classes[0].other_date.as_str(), thursday.classes[0].other_period, thursday.classes[0].start_minute),
        ("2026-04-13", 2, 10 * 60 + 50)
    );

    // A new change for the same slot and date replaces the old one.
    let replaced: CreateTimetableChangeResponse = ctx
        .ok(
            TIMETABLE,
            "CreateTimetableChange",
            &me,
            &CreateTimetableChangeRequest {
                subject_id: pe.id.clone(),
                ..change("2026-04-13", 1, ChangeKind::CHANGE_KIND_REPLACED)
            },
        )
        .await;
    assert_eq!(replaced.change.as_option().unwrap().id, cancelled.id);
    let monday = ctx.day(&me, "2026-04-13").await;
    assert_eq!(classes(&monday)[0], (1, "体育".to_owned(), CLASS_STATUS_REPLACED));
    assert_eq!(monday.classes[0].original_subject.as_option().unwrap().name, "数学");

    // The term lists its changes; a week lists every day.
    let loaded: GetTermResponse =
        ctx.ok(TIMETABLE, "GetTerm", &me, &GetTermRequest { term_id: term.id.clone(), ..Default::default() }).await;
    assert_eq!(loaded.changes.len(), 3);
    let week: ListDaysResponse = ctx
        .ok(
            TIMETABLE,
            "ListDays",
            &me,
            &ListDaysRequest { start_date: "2026-04-13".into(), end_date: "2026-04-19".into(), ..Default::default() },
        )
        .await;
    assert_eq!(week.days.len(), 7);
    assert_eq!(week.days.iter().map(|day| day.classes.len()).collect::<Vec<_>>(), vec![3, 1, 0, 1, 0, 0, 0]);
    assert_eq!(week.days[6].kind, DayKind::DAY_KIND_DAY_OFF);

    for (request, reason) in [
        (change("2026-04-13", 1, ChangeKind::CHANGE_KIND_UNSPECIFIED), "CHANGE_KIND_REQUIRED"),
        (change("2026-08-01", 1, ChangeKind::CHANGE_KIND_CANCELLED), "DATE_OUTSIDE_TERM"),
        (change("2026-04-13", 9, ChangeKind::CHANGE_KIND_CANCELLED), "SLOT_INVALID"),
        (change("2026-04-15", 1, ChangeKind::CHANGE_KIND_CANCELLED), "NO_CLASS_IN_SLOT"),
        (change("2026-04-29", 1, ChangeKind::CHANGE_KIND_CANCELLED), "NO_CLASS_IN_SLOT"),
        (change("2026-04-14", 2, ChangeKind::CHANGE_KIND_EXTRA), "SUBJECT_REQUIRED"),
        (change("2026-04-14", 1, ChangeKind::CHANGE_KIND_REPLACED), "SUBJECT_REQUIRED"),
        (change("2026-04-14", 1, ChangeKind::CHANGE_KIND_MOVED), "MOVE_TARGET_REQUIRED"),
        (
            CreateTimetableChangeRequest {
                to_date: "2026-04-14".into(),
                to_period: 1,
                ..change("2026-04-14", 1, ChangeKind::CHANGE_KIND_MOVED)
            },
            "MOVE_TARGET_REQUIRED",
        ),
        (
            CreateTimetableChangeRequest {
                to_date: "2026-08-14".into(),
                to_period: 1,
                ..change("2026-04-14", 1, ChangeKind::CHANGE_KIND_MOVED)
            },
            "DATE_OUTSIDE_TERM",
        ),
        (
            CreateTimetableChangeRequest {
                to_date: "2026-04-15".into(),
                to_period: 8,
                ..change("2026-04-14", 1, ChangeKind::CHANGE_KIND_MOVED)
            },
            "SLOT_INVALID",
        ),
        (
            CreateTimetableChangeRequest {
                subject_id: uuid::Uuid::now_v7().to_string(),
                ..change("2026-04-14", 2, ChangeKind::CHANGE_KIND_EXTRA)
            },
            "SUBJECT_NOT_FOUND",
        ),
        (
            CreateTimetableChangeRequest {
                memo: "x".repeat(501),
                ..change("2026-04-14", 1, ChangeKind::CHANGE_KIND_CANCELLED)
            },
            "TEXT_TOO_LONG",
        ),
    ] {
        assert_eq!(ctx.reason(TIMETABLE, "CreateTimetableChange", &me, &request).await, reason, "{request:?}");
    }
    assert_eq!(
        ctx.reason(
            TIMETABLE,
            "CreateTimetableChange",
            &other,
            &change("2026-04-14", 1, ChangeKind::CHANGE_KIND_CANCELLED)
        )
        .await,
        "TERM_NOT_FOUND"
    );
    // A room change is a replacement without a subject.
    let _: CreateTimetableChangeResponse = ctx
        .ok(
            TIMETABLE,
            "CreateTimetableChange",
            &me,
            &CreateTimetableChangeRequest {
                room: "グラウンド".into(),
                ..change("2026-04-14", 1, ChangeKind::CHANGE_KIND_REPLACED)
            },
        )
        .await;
    let tuesday = ctx.day(&me, "2026-04-14").await;
    assert_eq!(
        (classes(&tuesday)[0].clone(), tuesday.classes[0].room.as_str()),
        ((1, "体育".to_owned(), CLASS_STATUS_REPLACED), "グラウンド")
    );

    // Withdrawing a change puts the class back.
    let delete = DeleteTimetableChangeRequest { change_id: cancelled.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(TIMETABLE, "DeleteTimetableChange", &other, &delete).await, "CHANGE_NOT_FOUND");
    let _: DeleteTimetableChangeResponse = ctx.ok(TIMETABLE, "DeleteTimetableChange", &me, &delete).await;
    assert_eq!(ctx.reason(TIMETABLE, "DeleteTimetableChange", &me, &delete).await, "CHANGE_NOT_FOUND");
    assert_eq!(classes(&ctx.day(&me, "2026-04-13").await)[0], (1, "数学".to_owned(), CLASS_STATUS_SCHEDULED));

    assert_eq!(
        ctx.reason(
            TIMETABLE,
            "ListDays",
            &me,
            &ListDaysRequest { start_date: "2026-04-13".into(), end_date: "2026-04-12".into(), ..Default::default() }
        )
        .await,
        "DATE_RANGE_INVALID"
    );
    assert_eq!(
        ctx.reason(
            TIMETABLE,
            "ListDays",
            &me,
            &ListDaysRequest { start_date: "2026-04-13".into(), end_date: "2026-06-15".into(), ..Default::default() }
        )
        .await,
        "DATE_RANGE_INVALID"
    );
    assert_eq!(
        ctx.reason(
            TIMETABLE,
            "ListDays",
            &me,
            &ListDaysRequest { start_date: "2026-04-13".into(), ..Default::default() }
        )
        .await,
        "INVALID_DATE"
    );
    assert_eq!(
        ctx.reason(TIMETABLE, "GetDay", &me, &GetDayRequest { date: "13/04/2026".into(), ..Default::default() }).await,
        "INVALID_DATE"
    );
}

#[tokio::test]
async fn day_overrides_make_holidays_class_days_and_pinned_weeks() {
    let ctx = start().await;
    let (me, other) = (ctx.app.user("me").await, ctx.app.user("other").await);
    let (term, math, _english, _pe) = school_week(&ctx, &me).await;
    let day_override = |start: &str, kind: DayOverrideKind| CreateDayOverrideRequest {
        term_id: term.id.clone(),
        start_date: start.into(),
        kind: kind.into(),
        ..Default::default()
    };

    // 創立記念日 on a Monday; summer holidays as a range with one class day inside.
    let founding: CreateDayOverrideResponse = ctx
        .ok(
            TIMETABLE,
            "CreateDayOverride",
            &me,
            &CreateDayOverrideRequest {
                name: "創立記念日".into(),
                ..day_override("2026-04-20", DayOverrideKind::DAY_OVERRIDE_KIND_HOLIDAY)
            },
        )
        .await;
    let founding = founding.day_override.as_option().cloned().unwrap();
    assert_eq!((founding.start_date.as_str(), founding.end_date.as_str()), ("2026-04-20", "2026-04-20"));
    let day = ctx.day(&me, "2026-04-20").await;
    assert_eq!(
        (day.kind.as_known().unwrap(), day.label.as_str(), day.classes.len()),
        (DayKind::DAY_KIND_HOLIDAY, "創立記念日", 0)
    );

    let _: CreateDayOverrideResponse = ctx
        .ok(
            TIMETABLE,
            "CreateDayOverride",
            &me,
            &CreateDayOverrideRequest {
                end_date: "2026-07-20".into(),
                name: "夏休み".into(),
                ..day_override("2026-07-06", DayOverrideKind::DAY_OVERRIDE_KIND_HOLIDAY)
            },
        )
        .await;
    let _: CreateDayOverrideResponse = ctx
        .ok(
            TIMETABLE,
            "CreateDayOverride",
            &me,
            &CreateDayOverrideRequest {
                name: "登校日".into(),
                ..day_override("2026-07-13", DayOverrideKind::DAY_OVERRIDE_KIND_CLASS_DAY)
            },
        )
        .await;
    assert_eq!(ctx.day(&me, "2026-07-07").await.label, "夏休み");
    let school_day = ctx.day(&me, "2026-07-13").await;
    assert_eq!(
        (school_day.kind.as_known().unwrap(), school_day.label.as_str(), school_day.classes.len()),
        (DayKind::DAY_KIND_SCHOOL_DAY, "登校日", 2)
    );

    // 月曜授業 on the public holiday of Wednesday, April 29.
    let _: CreateDayOverrideResponse = ctx
        .ok(
            TIMETABLE,
            "CreateDayOverride",
            &me,
            &CreateDayOverrideRequest {
                name: "月曜授業".into(),
                follows_weekday: Weekday::WEEKDAY_MONDAY.into(),
                ..day_override("2026-04-29", DayOverrideKind::DAY_OVERRIDE_KIND_CLASS_DAY)
            },
        )
        .await;
    let swapped = ctx.day(&me, "2026-04-29").await;
    assert_eq!(
        (swapped.kind.as_known().unwrap(), swapped.follows_weekday.as_known().unwrap()),
        (DayKind::DAY_KIND_SCHOOL_DAY, Weekday::WEEKDAY_MONDAY)
    );
    assert_eq!(classes(&swapped).len(), 2);

    for (request, reason) in [
        (day_override("2026-04-21", DayOverrideKind::DAY_OVERRIDE_KIND_UNSPECIFIED), "OVERRIDE_KIND_REQUIRED"),
        (day_override("2026-03-21", DayOverrideKind::DAY_OVERRIDE_KIND_HOLIDAY), "DATE_OUTSIDE_TERM"),
        (
            CreateDayOverrideRequest {
                end_date: "2026-08-01".into(),
                ..day_override("2026-07-01", DayOverrideKind::DAY_OVERRIDE_KIND_HOLIDAY)
            },
            "DATE_OUTSIDE_TERM",
        ),
        (
            CreateDayOverrideRequest {
                end_date: "2026-04-20".into(),
                ..day_override("2026-04-21", DayOverrideKind::DAY_OVERRIDE_KIND_HOLIDAY)
            },
            "DATE_RANGE_INVALID",
        ),
        // This term has no A/B weeks to pin.
        (
            CreateDayOverrideRequest {
                week_pattern: WeekPattern::WEEK_PATTERN_A.into(),
                ..day_override("2026-04-21", DayOverrideKind::DAY_OVERRIDE_KIND_WEEK_PATTERN)
            },
            "WEEK_PATTERN_INVALID",
        ),
        (day_override("2026-04-21", DayOverrideKind::DAY_OVERRIDE_KIND_WEEK_PATTERN), "WEEK_PATTERN_INVALID"),
        (
            CreateDayOverrideRequest {
                name: "x".repeat(41),
                ..day_override("2026-04-21", DayOverrideKind::DAY_OVERRIDE_KIND_HOLIDAY)
            },
            "TEXT_TOO_LONG",
        ),
    ] {
        assert_eq!(ctx.reason(TIMETABLE, "CreateDayOverride", &me, &request).await, reason, "{request:?}");
    }
    assert_eq!(
        ctx.reason(
            TIMETABLE,
            "CreateDayOverride",
            &other,
            &day_override("2026-04-21", DayOverrideKind::DAY_OVERRIDE_KIND_HOLIDAY)
        )
        .await,
        "TERM_NOT_FOUND"
    );

    // Pinning a week: in an alternating term the week of April 13 is B by itself.
    let ab = ctx
        .term(&me, |term| {
            term.name = "AB".into();
            term.alternating_weeks = true;
            term.week_a_date = "2026-04-08".into();
            term.current = true;
        })
        .await;
    ctx.put(
        &me,
        &ab,
        vec![TimetableEntryInput {
            week_pattern: WeekPattern::WEEK_PATTERN_A.into(),
            ..class(Weekday::WEEKDAY_WEDNESDAY, 1, &math)
        }],
    )
    .await;
    assert_eq!(ctx.day(&me, "2026-04-08").await.week_pattern, WeekPattern::WEEK_PATTERN_A);
    let wednesday = ctx.day(&me, "2026-04-15").await;
    assert_eq!(
        (wednesday.week_pattern.as_known().unwrap(), wednesday.classes.len(), wednesday.term_id.as_str()),
        (WeekPattern::WEEK_PATTERN_B, 0, ab.id.as_str())
    );
    let pinned: CreateDayOverrideResponse = ctx
        .ok(
            TIMETABLE,
            "CreateDayOverride",
            &me,
            &CreateDayOverrideRequest {
                term_id: ab.id.clone(),
                start_date: "2026-04-13".into(),
                end_date: "2026-04-17".into(),
                kind: DayOverrideKind::DAY_OVERRIDE_KIND_WEEK_PATTERN.into(),
                week_pattern: WeekPattern::WEEK_PATTERN_A.into(),
                ..Default::default()
            },
        )
        .await;
    let wednesday = ctx.day(&me, "2026-04-15").await;
    assert_eq!((wednesday.week_pattern.as_known().unwrap(), wednesday.classes.len()), (WeekPattern::WEEK_PATTERN_A, 1));

    let delete = DeleteDayOverrideRequest {
        day_override_id: pinned.day_override.as_option().unwrap().id.clone(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(TIMETABLE, "DeleteDayOverride", &other, &delete).await, "OVERRIDE_NOT_FOUND");
    let _: DeleteDayOverrideResponse = ctx.ok(TIMETABLE, "DeleteDayOverride", &me, &delete).await;
    assert_eq!(ctx.reason(TIMETABLE, "DeleteDayOverride", &me, &delete).await, "OVERRIDE_NOT_FOUND");
    assert_eq!(ctx.day(&me, "2026-04-15").await.week_pattern, WeekPattern::WEEK_PATTERN_B);
}

#[tokio::test]
async fn now_is_the_class_in_progress_and_the_next_one() {
    let ctx = start().await;
    let me = ctx.app.user("me").await;
    school_week(&ctx, &me).await;
    let class = |date: &str, period: i32, subject: &str| Some((date.to_owned(), period, subject.to_owned()));

    // Before school on Monday.
    ctx.at(2026, 4, 13, 7, 30);
    assert_eq!(now(&ctx, &me).await, (None, class("2026-04-13", 1, "数学")));
    assert_eq!(ctx.day(&me, "").await.date, "2026-04-13", "an empty date means today");
    // During the first period, then in the break after it.
    ctx.at(2026, 4, 13, 9, 0);
    assert_eq!(now(&ctx, &me).await, (class("2026-04-13", 1, "数学"), class("2026-04-13", 2, "英語")));
    ctx.at(2026, 4, 13, 9, 45);
    assert_eq!(now(&ctx, &me).await, (None, class("2026-04-13", 2, "英語")));
    // During the last class of the day: next is tomorrow's first.
    ctx.at(2026, 4, 13, 10, 0);
    assert_eq!(now(&ctx, &me).await, (class("2026-04-13", 2, "英語"), class("2026-04-14", 1, "体育")));
    // After school on Tuesday: the next class is Monday's, across the weekend.
    ctx.at(2026, 4, 14, 16, 0);
    assert_eq!(now(&ctx, &me).await, (None, class("2026-04-20", 1, "数学")));
    // Just before midnight in Tokyo is still Tuesday there.
    ctx.at(2026, 4, 14, 23, 59);
    assert_eq!(ctx.day(&me, "").await.date, "2026-04-14");

    // After the term nothing is in sight.
    ctx.at(2026, 7, 21, 9, 0);
    assert_eq!(now(&ctx, &me).await, (None, None));
}

#[tokio::test]
async fn class_times_are_in_the_owners_time_zone() {
    let ctx = start().await;
    let me = ctx.app.user("newyorker").await;
    ctx.set_time_zone(&me, "America/New_York").await;
    school_week(&ctx, &me).await;

    // 8:50 in New York on April 13 (daylight time, UTC-4).
    let monday = ctx.day(&me, "2026-04-13").await;
    assert_eq!(
        instant(&monday.classes[0].starts_at),
        chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 4, 13, 12, 50, 0).unwrap()
    );
    // 22:00 in Tokyo on Monday is 9:00 on Monday in New York: first period.
    ctx.at(2026, 4, 13, 22, 0);
    let response: GetNowResponse = ctx.ok(TIMETABLE, "GetNow", &me, &GetNowRequest::default()).await;
    assert_eq!(response.now.as_option().unwrap().current.as_option().unwrap().period, 1);
    // 9:00 in Tokyo on Monday is still Sunday evening in New York.
    ctx.at(2026, 4, 13, 9, 0);
    assert_eq!(ctx.day(&me, "").await.date, "2026-04-12");
}
