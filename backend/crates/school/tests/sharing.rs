//! Timetables between friends: audiences, reading a friend's timetable,
//! copying a classmate's, free periods in common, and the profile card.
mod common;

use common::*;
use tokuly_proto::school::*;
use tokuly_testkit::TestUser;

async fn set_audience(
    ctx: &Ctx,
    owner: &TestUser,
    term: &Term,
    audience: TimetableAudience,
    chosen: &[&TestUser],
) -> Term {
    let request = UpdateTermRequest {
        term_id: term.id.clone(),
        audience: audience.into(),
        shared_with: UserIdList {
            user_ids: chosen.iter().map(|user| user.id.to_string()).collect(),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };
    let response: UpdateTermResponse = ctx.ok(TIMETABLE, "UpdateTerm", owner, &request).await;
    response.term.as_option().cloned().unwrap()
}

async fn find(ctx: &Ctx, user: &TestUser, request: FindCommonFreePeriodsRequest) -> FindCommonFreePeriodsResponse {
    ctx.ok(TIMETABLE, "FindCommonFreePeriods", user, &request).await
}

async fn friend_timetables(ctx: &Ctx, viewer: &TestUser) -> Vec<FriendTimetable> {
    let response: ListFriendTimetablesResponse =
        ctx.ok(TIMETABLE, "ListFriendTimetables", viewer, &ListFriendTimetablesRequest::default()).await;
    response.timetables
}

#[tokio::test]
async fn a_term_is_seen_by_its_audience_only() {
    let ctx = start().await;
    let (owner, friend, chosen, stranger) = (
        ctx.app.user("owner").await,
        ctx.app.user("friend").await,
        ctx.app.user("chosen").await,
        ctx.app.user("stranger").await,
    );
    ctx.befriend(&owner, &friend).await;
    ctx.befriend(&owner, &chosen).await;
    let math = ctx.subject(&owner, "数学").await;
    let term = ctx.term(&owner, |_| {}).await;
    ctx.put(
        &owner,
        &term,
        vec![TimetableEntryInput {
            memo: "秘密のメモ".into(),
            room: "2-A".into(),
            ..class(Weekday::WEEKDAY_MONDAY, 1, &math)
        }],
    )
    .await;
    ctx.at(2026, 4, 13, 9, 0); // Monday, first period

    // Private by default: nobody else gets anything.
    let of_owner = owner.id.to_string();
    for viewer in [&friend, &chosen, &stranger] {
        assert_eq!(ctx.day_of(viewer, &owner, "2026-04-13").await.unwrap_err().reason, "TIMETABLE_NOT_SHARED");
        assert_eq!(
            ctx.reason(TIMETABLE, "GetNow", viewer, &GetNowRequest { user_id: of_owner.clone(), ..Default::default() })
                .await,
            "TIMETABLE_NOT_SHARED"
        );
        let days = ListDaysRequest {
            start_date: "2026-04-13".into(),
            end_date: "2026-04-14".into(),
            user_id: of_owner.clone(),
            ..Default::default()
        };
        assert_eq!(ctx.reason(TIMETABLE, "ListDays", viewer, &days).await, "TIMETABLE_NOT_SHARED");
        assert_eq!(
            ctx.reason(
                TIMETABLE,
                "GetFriendTimetable",
                viewer,
                &GetFriendTimetableRequest { user_id: of_owner.clone(), ..Default::default() }
            )
            .await,
            "TIMETABLE_NOT_SHARED"
        );
        assert!(friend_timetables(&ctx, viewer).await.is_empty());
        assert!(ctx.profile_card(viewer, &owner, "school.now").await.is_none());
    }

    // Chosen friends: the chosen one sees the day, without the private memo.
    let shared = set_audience(&ctx, &owner, &term, TimetableAudience::TIMETABLE_AUDIENCE_CHOSEN, &[&chosen]).await;
    assert_eq!(shared.shared_with.iter().map(|u| u.id.clone()).collect::<Vec<_>>(), vec![chosen.id.to_string()]);
    let seen = ctx.day_of(&chosen, &owner, "2026-04-13").await.unwrap();
    assert_eq!(classes(&seen), vec![(1, "数学".to_owned(), ClassStatus::CLASS_STATUS_SCHEDULED)]);
    assert_eq!((seen.classes[0].memo.as_str(), seen.classes[0].room.as_str()), ("", "2-A"));
    assert_eq!(ctx.day(&owner, "2026-04-13").await.classes[0].memo, "秘密のメモ");
    assert_eq!(ctx.day_of(&friend, &owner, "2026-04-13").await.unwrap_err().reason, "TIMETABLE_NOT_SHARED");
    assert_eq!(friend_timetables(&ctx, &chosen).await.len(), 1);
    assert!(friend_timetables(&ctx, &friend).await.is_empty());

    // All friends: both friends see it, a stranger still does not.
    set_audience(&ctx, &owner, &term, TimetableAudience::TIMETABLE_AUDIENCE_FRIENDS, &[]).await;
    assert!(ctx.day_of(&friend, &owner, "2026-04-13").await.is_ok());
    assert_eq!(ctx.day_of(&stranger, &owner, "2026-04-13").await.unwrap_err().reason, "TIMETABLE_NOT_SHARED");

    let theirs: GetFriendTimetableResponse = ctx
        .ok(
            TIMETABLE,
            "GetFriendTimetable",
            &friend,
            &GetFriendTimetableRequest { user_id: of_owner.clone(), ..Default::default() },
        )
        .await;
    let seen_term = theirs.term.as_option().unwrap();
    assert_eq!((seen_term.id.as_str(), seen_term.owner_id.as_str()), (term.id.as_str(), of_owner.as_str()));
    assert!(seen_term.shared_with.is_empty(), "who else a term is shared with is the owner's business");
    assert_eq!(
        (theirs.entries.len(), theirs.entries[0].memo.as_str(), theirs.entries[0].room.as_str()),
        (1, "", "2-A")
    );
    let wrong_term = GetFriendTimetableRequest {
        user_id: of_owner.clone(),
        term_id: uuid::Uuid::now_v7().to_string(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(TIMETABLE, "GetFriendTimetable", &friend, &wrong_term).await, "TIMETABLE_NOT_SHARED");

    let listed = friend_timetables(&ctx, &friend).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].user.as_option().unwrap().id, of_owner);
    assert_eq!(listed[0].now.as_option().unwrap().current.as_option().unwrap().period, 1);
    let now: GetNowResponse =
        ctx.ok(TIMETABLE, "GetNow", &friend, &GetNowRequest { user_id: of_owner.clone(), ..Default::default() }).await;
    assert_eq!(now.now.as_option().unwrap().current.as_option().unwrap().subject.as_option().unwrap().name, "数学");

    // The class in progress shows on the profile for the audience.
    let card = ctx.profile_card(&friend, &owner, "school.now").await.expect("the friend sees the class");
    assert_eq!(card.data["state"], "current");
    assert_eq!(card.data["room"], "2-A");
    assert!(
        card.text.as_option().unwrap().ja.contains("数学") && card.title.as_option().unwrap().ja == "授業中",
        "{card:?}"
    );
    assert!(ctx.profile_card(&stranger, &owner, "school.now").await.is_none());
    assert!(ctx.profile_card(&owner, &owner, "school.now").await.is_some());
    // Between classes with nothing left today, there is nothing to show.
    ctx.at(2026, 4, 13, 15, 0);
    assert!(ctx.profile_card(&friend, &owner, "school.now").await.is_none());
    // Before the first class it is the next one.
    ctx.at(2026, 4, 13, 8, 0);
    let card = ctx.profile_card(&friend, &owner, "school.now").await.unwrap();
    assert_eq!((card.data["state"].as_str(), card.title.as_option().unwrap().ja.as_str()), ("next", "次の授業"));

    // A second, private term stays invisible: its dates are "out of term" to friends.
    let second = ctx
        .term(&owner, |t| {
            t.name = "2学期".into();
            t.start_date = "2026-09-01".into();
            t.end_date = "2026-12-24".into();
        })
        .await;
    ctx.put(&owner, &second, vec![class(Weekday::WEEKDAY_MONDAY, 1, &math)]).await;
    assert_eq!(ctx.day(&owner, "2026-09-07").await.classes.len(), 1);
    assert_eq!(ctx.day_of(&friend, &owner, "2026-09-07").await.unwrap().kind, DayKind::DAY_KIND_OUT_OF_TERM);
    let theirs: GetFriendTimetableResponse = ctx
        .ok(
            TIMETABLE,
            "GetFriendTimetable",
            &friend,
            &GetFriendTimetableRequest { user_id: of_owner, ..Default::default() },
        )
        .await;
    assert!(theirs.other_terms.is_empty());
}

#[tokio::test]
async fn sharing_ends_with_the_friendship() {
    let ctx = start().await;
    let (owner, friend, blocked) =
        (ctx.app.user("owner").await, ctx.app.user("friend").await, ctx.app.user("blocked").await);
    ctx.befriend(&owner, &friend).await;
    ctx.befriend(&owner, &blocked).await;
    let math = ctx.subject(&owner, "数学").await;
    let term = ctx.term(&owner, |_| {}).await;
    ctx.put(&owner, &term, vec![class(Weekday::WEEKDAY_MONDAY, 1, &math)]).await;
    set_audience(&ctx, &owner, &term, TimetableAudience::TIMETABLE_AUDIENCE_CHOSEN, &[&friend, &blocked]).await;
    assert!(ctx.day_of(&friend, &owner, "2026-04-13").await.is_ok());

    ctx.unfriend(&friend, &owner).await;
    assert_eq!(ctx.day_of(&friend, &owner, "2026-04-13").await.unwrap_err().reason, "TIMETABLE_NOT_SHARED");
    ctx.block(&owner, &blocked).await;
    assert_eq!(ctx.day_of(&blocked, &owner, "2026-04-13").await.unwrap_err().reason, "TIMETABLE_NOT_SHARED");
    let listed: ListTermsResponse = ctx.ok(TIMETABLE, "ListTerms", &owner, &ListTermsRequest::default()).await;
    assert!(listed.terms[0].shared_with.is_empty(), "neither is among the chosen friends anymore");

    // Becoming friends again does not bring the old choice back.
    ctx.befriend(&owner, &friend).await;
    assert_eq!(ctx.day_of(&friend, &owner, "2026-04-13").await.unwrap_err().reason, "TIMETABLE_NOT_SHARED");

    // Events can arrive late and more than once. The owner chooses the
    // friend again; the old "friendship ended" turning up after that must
    // not take the new choice away.
    set_audience(&ctx, &owner, &term, TimetableAudience::TIMETABLE_AUDIENCE_CHOSEN, &[&friend]).await;
    ctx.deliver("social.friend.removed", friend.id).await;
    assert!(ctx.day_of(&friend, &owner, "2026-04-13").await.is_ok());
    // The same for a block that was lifted, followed by a new friendship.
    let unblock = tokuly_proto::social::UnblockUserRequest { user_id: blocked.id.to_string(), ..Default::default() };
    let _: tokuly_proto::social::UnblockUserResponse =
        ctx.app.call("tokuly.social.v1.FriendService/UnblockUser", Some(&owner), &unblock).await.unwrap();
    ctx.befriend(&owner, &blocked).await;
    set_audience(&ctx, &owner, &term, TimetableAudience::TIMETABLE_AUDIENCE_CHOSEN, &[&friend, &blocked]).await;
    ctx.deliver("social.user.blocked", owner.id).await;
    ctx.deliver("social.friend.removed", owner.id).await;
    assert!(ctx.day_of(&blocked, &owner, "2026-04-13").await.is_ok());
    assert!(ctx.day_of(&friend, &owner, "2026-04-13").await.is_ok());
}

#[tokio::test]
async fn a_classmates_timetable_is_copied_as_a_starting_point() {
    let ctx = start().await;
    let (me, classmate, stranger) =
        (ctx.app.user("me").await, ctx.app.user("classmate").await, ctx.app.user("stranger").await);
    ctx.befriend(&me, &classmate).await;

    // The classmate's term: three classes, a school holiday, a one-off change.
    let math: CreateSubjectResponse = ctx
        .ok(
            TIMETABLE,
            "CreateSubject",
            &classmate,
            &CreateSubjectRequest {
                name: "数学".into(),
                teacher: "田中".into(),
                color: "#112233".into(),
                ..Default::default()
            },
        )
        .await;
    let math = math.subject.as_option().cloned().unwrap();
    let english = ctx.subject(&classmate, "英語").await;
    let theirs = ctx.term(&classmate, |t| t.audience = TimetableAudience::TIMETABLE_AUDIENCE_FRIENDS.into()).await;
    ctx.put(
        &classmate,
        &theirs,
        vec![
            TimetableEntryInput {
                memo: "わたしのメモ".into(),
                room: "視聴覚室".into(),
                ..class(Weekday::WEEKDAY_MONDAY, 1, &math)
            },
            class(Weekday::WEEKDAY_MONDAY, 2, &english),
            class(Weekday::WEEKDAY_TUESDAY, 1, &math),
        ],
    )
    .await;
    let holiday = CreateDayOverrideRequest {
        term_id: theirs.id.clone(),
        start_date: "2026-04-20".into(),
        kind: DayOverrideKind::DAY_OVERRIDE_KIND_HOLIDAY.into(),
        name: "創立記念日".into(),
        ..Default::default()
    };
    let _: CreateDayOverrideResponse = ctx.ok(TIMETABLE, "CreateDayOverride", &classmate, &holiday).await;
    let cancelled = CreateTimetableChangeRequest {
        term_id: theirs.id.clone(),
        date: "2026-04-13".into(),
        period: 1,
        kind: ChangeKind::CHANGE_KIND_CANCELLED.into(),
        ..Default::default()
    };
    let _: CreateTimetableChangeResponse = ctx.ok(TIMETABLE, "CreateTimetableChange", &classmate, &cancelled).await;

    // I already have a subject called 数学, in my own colour.
    let my_math: CreateSubjectResponse = ctx
        .ok(
            TIMETABLE,
            "CreateSubject",
            &me,
            &CreateSubjectRequest { name: "数学".into(), color: "#ff0000".into(), ..Default::default() },
        )
        .await;
    let my_math = my_math.subject.as_option().cloned().unwrap();

    let copy = |fill: fn(&mut CopyFriendTimetableRequest)| {
        let mut request = CopyFriendTimetableRequest { user_id: classmate.id.to_string(), ..Default::default() };
        fill(&mut request);
        request
    };
    let copied: CopyFriendTimetableResponse = ctx.ok(TIMETABLE, "CopyFriendTimetable", &me, &copy(|_| {})).await;
    assert_eq!((copied.copied_entry_count, copied.skipped_entry_count, copied.created_subject_count), (3, 0, 1));
    let mine = copied.term.as_option().cloned().unwrap();
    assert_eq!(
        (mine.name.as_str(), mine.start_date.as_str(), mine.end_date.as_str()),
        ("1学期", "2026-04-08", "2026-07-20")
    );
    assert!(mine.current, "my first term");
    assert_eq!(mine.audience, TimetableAudience::TIMETABLE_AUDIENCE_NOBODY, "a copy starts private");
    assert_eq!(mine.copied_from.as_option().unwrap().id, classmate.id.to_string());
    assert_ne!(mine.id, theirs.id);
    let monday_first = copied.entries.iter().find(|e| e.weekday == Weekday::WEEKDAY_MONDAY && e.period == 1).unwrap();
    assert_eq!(monday_first.subject.as_option().unwrap().id, my_math.id, "matched to my subject by name");
    assert_eq!(
        (monday_first.memo.as_str(), monday_first.room.as_str()),
        ("", "視聴覚室"),
        "memos stay with their owner"
    );
    let subjects: ListSubjectsResponse = ctx.ok(TIMETABLE, "ListSubjects", &me, &ListSubjectsRequest::default()).await;
    assert_eq!(subjects.subjects.iter().map(|s| (s.name.as_str(), s.color.as_str())).collect::<Vec<_>>().len(), 2);
    assert_eq!(
        subjects.subjects.iter().find(|s| s.name == "数学").unwrap().color,
        "#ff0000",
        "my own subject is untouched"
    );

    // The school's holiday came along; the classmate's one-off change did not.
    let loaded: GetTermResponse =
        ctx.ok(TIMETABLE, "GetTerm", &me, &GetTermRequest { term_id: mine.id.clone(), ..Default::default() }).await;
    assert_eq!((loaded.day_overrides.len(), loaded.changes.len()), (1, 0));
    assert_eq!(ctx.day(&me, "2026-04-20").await.label, "創立記念日");
    assert_eq!(classes(&ctx.day(&me, "2026-04-13").await)[0].2, ClassStatus::CLASS_STATUS_SCHEDULED);

    // The copy is mine: what the classmate does afterwards does not reach it.
    ctx.put(&classmate, &theirs, vec![class(Weekday::WEEKDAY_FRIDAY, 6, &english)]).await;
    let loaded: GetTermResponse =
        ctx.ok(TIMETABLE, "GetTerm", &me, &GetTermRequest { term_id: mine.id.clone(), ..Default::default() }).await;
    assert_eq!(loaded.entries.len(), 3);

    // Copying into my term keeps my own classes: I replaced Monday's first
    // period with art and emptied Tuesday's.
    let art = ctx.subject(&me, "美術").await;
    ctx.put(&me, &mine, vec![class(Weekday::WEEKDAY_MONDAY, 1, &art)]).await;
    let tuesday = copied.entries.iter().find(|e| e.weekday == Weekday::WEEKDAY_TUESDAY).unwrap();
    let _: DeleteTimetableEntriesResponse = ctx
        .ok(
            TIMETABLE,
            "DeleteTimetableEntries",
            &me,
            &DeleteTimetableEntriesRequest {
                term_id: mine.id.clone(),
                entry_ids: vec![tuesday.id.clone()],
                ..Default::default()
            },
        )
        .await;
    let into_mine = CopyFriendTimetableRequest { target_term_id: mine.id.clone(), ..copy(|_| {}) };
    let merged: CopyFriendTimetableResponse = ctx.ok(TIMETABLE, "CopyFriendTimetable", &me, &into_mine).await;
    assert_eq!(
        (merged.copied_entry_count, merged.skipped_entry_count, merged.created_subject_count),
        (2, 2, 0),
        "Tuesday's first and Friday's sixth are new"
    );
    assert_eq!(merged.term.as_option().unwrap().id, mine.id);
    let monday_first = merged.entries.iter().find(|e| e.weekday == Weekday::WEEKDAY_MONDAY && e.period == 1).unwrap();
    assert_eq!(monday_first.subject.as_option().unwrap().name, "美術", "my own class is kept");
    // With `overwrite` the classmate's classes win.
    let overwritten: CopyFriendTimetableResponse = ctx
        .ok(TIMETABLE, "CopyFriendTimetable", &me, &CopyFriendTimetableRequest { overwrite: true, ..into_mine.clone() })
        .await;
    assert_eq!((overwritten.copied_entry_count, overwritten.skipped_entry_count), (4, 0));
    let monday_first =
        overwritten.entries.iter().find(|e| e.weekday == Weekday::WEEKDAY_MONDAY && e.period == 1).unwrap();
    assert_eq!(monday_first.subject.as_option().unwrap().name, "数学");
    assert_eq!(overwritten.entries.len(), 4);

    // A named copy; what cannot be copied.
    let named: CopyFriendTimetableResponse =
        ctx.ok(TIMETABLE, "CopyFriendTimetable", &me, &copy(|r| r.name = "コピー".into())).await;
    assert_eq!(
        (named.term.as_option().unwrap().name.as_str(), named.term.as_option().unwrap().current),
        ("コピー", false)
    );
    assert_eq!(
        ctx.reason(TIMETABLE, "CopyFriendTimetable", &me, &copy(|r| r.name = "あ".repeat(41))).await,
        "TERM_NAME_INVALID"
    );
    assert_eq!(
        ctx.reason(
            TIMETABLE,
            "CopyFriendTimetable",
            &me,
            &copy(|r| r.target_term_id = uuid::Uuid::now_v7().to_string())
        )
        .await,
        "TERM_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason(TIMETABLE, "CopyFriendTimetable", &me, &copy(|r| r.term_id = uuid::Uuid::now_v7().to_string()))
            .await,
        "TIMETABLE_NOT_SHARED"
    );
    assert_eq!(ctx.reason(TIMETABLE, "CopyFriendTimetable", &stranger, &copy(|_| {})).await, "TIMETABLE_NOT_SHARED");
    let from_stranger = CopyFriendTimetableRequest { user_id: stranger.id.to_string(), ..Default::default() };
    assert_eq!(ctx.reason(TIMETABLE, "CopyFriendTimetable", &me, &from_stranger).await, "TIMETABLE_NOT_SHARED");
}

#[tokio::test]
async fn free_periods_are_compared_with_friends_who_share() {
    let ctx = start().await;
    let (me, busy, free, private, elsewhere, stranger) = (
        ctx.app.user("me").await,
        ctx.app.user("busy").await,
        ctx.app.user("free").await,
        ctx.app.user("private").await,
        ctx.app.user("elsewhere").await,
        ctx.app.user("stranger").await,
    );
    for friend in [&busy, &free, &private, &elsewhere] {
        ctx.befriend(&me, friend).await;
    }
    let shared = |t: &mut CreateTermRequest| t.audience = TimetableAudience::TIMETABLE_AUDIENCE_FRIENDS.into();

    // Me on Mondays: classes in periods 1 and 4.
    let my_math = ctx.subject(&me, "数学").await;
    let my_term = ctx.term(&me, |_| {}).await;
    ctx.put(
        &me,
        &my_term,
        vec![class(Weekday::WEEKDAY_MONDAY, 1, &my_math), class(Weekday::WEEKDAY_MONDAY, 4, &my_math)],
    )
    .await;
    // `busy` has a class in period 2; `free` has school but no classes on Mondays.
    let their_math = ctx.subject(&busy, "数学").await;
    let busy_term = ctx.term(&busy, shared).await;
    ctx.put(&busy, &busy_term, vec![class(Weekday::WEEKDAY_MONDAY, 2, &their_math)]).await;
    ctx.term(&free, shared).await;
    // `private` keeps their timetable to themselves; `elsewhere` shares a
    // term that does not cover April.
    ctx.term(&private, |_| {}).await;
    ctx.term(&elsewhere, |t| {
        t.start_date = "2026-09-01".into();
        t.end_date = "2026-12-24".into();
        t.audience = TimetableAudience::TIMETABLE_AUDIENCE_FRIENDS.into();
    })
    .await;

    let names = |period: &FreePeriod| {
        let mut names: Vec<String> = period.free_friends.iter().map(|user| user.display_name.clone()).collect();
        names.sort();
        names
    };
    let monday =
        find(&ctx, &me, FindCommonFreePeriodsRequest { start_date: "2026-04-13".into(), ..Default::default() }).await;
    assert_eq!(
        monday.periods.iter().map(|p| (p.date.as_str(), p.period, p.between_classes)).collect::<Vec<_>>(),
        vec![("2026-04-13", 2, true), ("2026-04-13", 3, true), ("2026-04-13", 5, false), ("2026-04-13", 6, false),]
    );
    assert_eq!(names(&monday.periods[0]), vec!["free"], "busy has a class in period 2; nobody knows about elsewhere");
    assert_eq!(names(&monday.periods[1]), vec!["busy", "free"]);
    assert_eq!(instant(&monday.periods[0].starts_at), tokyo(2026, 4, 13, 9, 50));
    assert!(monday.not_shared_user_ids.is_empty(), "nobody was asked for by name");

    // Asking for people by name says who could not be compared.
    let asked = find(
        &ctx,
        &me,
        FindCommonFreePeriodsRequest {
            start_date: "2026-04-13".into(),
            user_ids: vec![busy.id.to_string(), private.id.to_string(), stranger.id.to_string(), me.id.to_string()],
            ..Default::default()
        },
    )
    .await;
    assert_eq!(names(&asked.periods[1]), vec!["busy"]);
    let mut not_shared = asked.not_shared_user_ids.clone();
    not_shared.sort();
    let mut expected = vec![private.id.to_string(), stranger.id.to_string()];
    expected.sort();
    assert_eq!(not_shared, expected);

    // A cancelled class frees its period; two days list both; a holiday has no periods.
    let cancel = CreateTimetableChangeRequest {
        term_id: my_term.id.clone(),
        date: "2026-04-13".into(),
        period: 1,
        kind: ChangeKind::CHANGE_KIND_CANCELLED.into(),
        ..Default::default()
    };
    let _: CreateTimetableChangeResponse = ctx.ok(TIMETABLE, "CreateTimetableChange", &me, &cancel).await;
    let two_days = find(
        &ctx,
        &me,
        FindCommonFreePeriodsRequest { start_date: "2026-04-13".into(), days: 2, ..Default::default() },
    )
    .await;
    assert_eq!(
        two_days.periods.iter().filter(|p| p.date == "2026-04-13").map(|p| p.period).collect::<Vec<_>>(),
        vec![1, 2, 3, 5, 6]
    );
    assert_eq!(two_days.periods.iter().filter(|p| p.date == "2026-04-14").count(), 6);
    assert!(
        find(&ctx, &me, FindCommonFreePeriodsRequest { start_date: "2026-04-29".into(), ..Default::default() })
            .await
            .periods
            .is_empty()
    );
    // Without a date it is today.
    ctx.at(2026, 4, 17, 12, 0);
    assert!(
        find(&ctx, &me, FindCommonFreePeriodsRequest::default()).await.periods.iter().all(|p| p.date == "2026-04-17")
    );

    assert_eq!(
        ctx.reason(
            TIMETABLE,
            "FindCommonFreePeriods",
            &me,
            &FindCommonFreePeriodsRequest { days: 8, ..Default::default() }
        )
        .await,
        "DATE_RANGE_INVALID"
    );
    let crowd = FindCommonFreePeriodsRequest {
        user_ids: (0..51).map(|_| uuid::Uuid::now_v7().to_string()).collect(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(TIMETABLE, "FindCommonFreePeriods", &me, &crowd).await, "TOO_MANY_USERS");
}
