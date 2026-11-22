//! `AnniversaryService` and `CalendarService`: countdowns, "days since"
//! counters, birthdays, and the month that shows them all.
mod common;

use chrono::{Datelike as _, Duration, Months, NaiveDate, TimeZone as _, Utc};
use chrono_tz::Asia::Tokyo;
use common::*;
use tokuly_proto::plans::{plans_event::Event as Push, *};
use tokuly_testkit::TestUser;
use uuid::Uuid;

/// Today on a Tokyo calendar (the default time zone of test users).
fn today() -> NaiveDate {
    Utc::now().with_timezone(&Tokyo).date_naive()
}

fn date(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

fn in_days(days: i64) -> String {
    date(today() + Duration::days(days))
}

fn countdown_to(title: &str, date: String) -> CreateCountdownRequest {
    CreateCountdownRequest { title: title.to_owned(), date, ..Default::default() }
}

async fn create_countdown(ctx: &Ctx, user: &TestUser, request: CreateCountdownRequest) -> Countdown {
    let response: CreateCountdownResponse =
        ctx.call(ANNIVERSARIES, "CreateCountdown", user, &request).await.expect("create countdown");
    response.countdown.as_option().cloned().expect("a countdown")
}

async fn countdowns(ctx: &Ctx, user: &TestUser) -> Vec<Countdown> {
    let response: ListCountdownsResponse = ctx
        .call(ANNIVERSARIES, "ListCountdowns", user, &ListCountdownsRequest::default())
        .await
        .expect("list countdowns");
    response.countdowns
}

fn countdown_ref(countdown: &Countdown) -> GetCountdownRequest {
    GetCountdownRequest { countdown_id: countdown.id.clone(), ..Default::default() }
}

async fn anniversaries(ctx: &Ctx, user: &TestUser) -> Vec<Anniversary> {
    let response: ListAnniversariesResponse = ctx
        .call(ANNIVERSARIES, "ListAnniversaries", user, &ListAnniversariesRequest::default())
        .await
        .expect("list anniversaries");
    response.anniversaries
}

async fn update_anniversary(ctx: &Ctx, user: &TestUser, request: UpdateAnniversaryRequest) -> Anniversary {
    let response: UpdateAnniversaryResponse =
        ctx.call(ANNIVERSARIES, "UpdateAnniversary", user, &request).await.expect("update anniversary");
    response.anniversary.as_option().cloned().expect("an anniversary")
}

fn partner_id(anniversary: &Anniversary) -> String {
    anniversary.partner.as_option().expect("a partner").id.clone()
}

#[tokio::test]
async fn a_countdown_counts_the_days_to_a_day() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui"]).await;
    let (mika, yui) = (&users[0], &users[1]);

    let festival = create_countdown(
        &ctx,
        mika,
        CreateCountdownRequest { emoji: "🎪".into(), ..countdown_to(" 文化祭 ", in_days(12)) },
    )
    .await;
    assert_eq!((festival.title.as_str(), festival.emoji.as_str(), festival.days_left), ("文化祭", "🎪", 12));
    assert_eq!((festival.date.clone(), festival.next_date.clone()), (in_days(12), in_days(12)));
    assert_eq!(festival.owner.as_option().unwrap().id, mika.id.to_string());
    // Private unless said otherwise.
    assert_eq!(festival.sharing, CountdownSharing::COUNTDOWN_SHARING_PRIVATE);

    // A day that comes back every year counts down to its next occurrence.
    let long_ago = NaiveDate::from_ymd_opt(2010, today().month(), today().day().min(28)).unwrap() + Duration::days(40);
    let yearly = create_countdown(
        &ctx,
        mika,
        CreateCountdownRequest { repeats_yearly: true, ..countdown_to("推しの誕生日", date(long_ago)) },
    )
    .await;
    assert_eq!(yearly.date, date(long_ago));
    assert!(yearly.repeats_yearly && (0..=366).contains(&yearly.days_left));
    assert!(yearly.next_date >= date(today()));
    let exam = create_countdown(&ctx, mika, countdown_to("期末テスト", in_days(3))).await;

    // The nearest day comes first.
    let listed = countdowns(&ctx, mika).await;
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[0].id, exam.id);
    assert!(listed.windows(2).all(|pair| pair[0].days_left <= pair[1].days_left));
    // In pages of two, the order is the same.
    let page = |cursor: &str| ListCountdownsRequest {
        page: tokuly_proto::common::PageRequest { limit: 2, cursor: cursor.to_owned(), ..Default::default() }.into(),
        ..Default::default()
    };
    let first: ListCountdownsResponse = ctx.call(ANNIVERSARIES, "ListCountdowns", mika, &page("")).await.unwrap();
    let info = first.page.as_option().unwrap();
    assert!(info.has_more);
    let rest: ListCountdownsResponse =
        ctx.call(ANNIVERSARIES, "ListCountdowns", mika, &page(&info.next_cursor)).await.unwrap();
    assert!(!rest.page.as_option().unwrap().has_more);
    let paged: Vec<String> =
        first.countdowns.iter().chain(&rest.countdowns).map(|countdown| countdown.id.clone()).collect();
    assert_eq!(paged, listed.iter().map(|countdown| countdown.id.clone()).collect::<Vec<_>>());

    // Only the owner changes a countdown; to everyone else it does not exist.
    let change =
        |countdown: &Countdown| UpdateCountdownRequest { countdown_id: countdown.id.clone(), ..Default::default() };
    let response: UpdateCountdownResponse = ctx
        .call(
            ANNIVERSARIES,
            "UpdateCountdown",
            mika,
            &UpdateCountdownRequest {
                title: Some("期末テスト最終日".into()),
                date: Some(in_days(5)),
                emoji: Some("📝".into()),
                ..change(&exam)
            },
        )
        .await
        .unwrap();
    let moved = response.countdown.as_option().unwrap();
    assert_eq!((moved.title.as_str(), moved.days_left, moved.emoji.as_str()), ("期末テスト最終日", 5, "📝"));
    assert_eq!(
        ctx.refusal(
            ANNIVERSARIES,
            "UpdateCountdown",
            yui,
            &UpdateCountdownRequest { title: Some("x".into()), ..change(&exam) }
        )
        .await,
        "COUNTDOWN_NOT_FOUND"
    );
    assert_eq!(ctx.refusal(ANNIVERSARIES, "GetCountdown", yui, &countdown_ref(&exam)).await, "COUNTDOWN_NOT_FOUND");
    assert_eq!(
        ctx.refusal(
            ANNIVERSARIES,
            "DeleteCountdown",
            yui,
            &DeleteCountdownRequest { countdown_id: exam.id.clone(), ..Default::default() }
        )
        .await,
        "COUNTDOWN_NOT_FOUND"
    );
    assert_eq!(
        ctx.refusal(
            ANNIVERSARIES,
            "UpdateCountdown",
            mika,
            &UpdateCountdownRequest { date: Some(in_days(-1)), ..change(&exam) }
        )
        .await,
        "DATE_INVALID"
    );
    // Making it yearly makes a past day fine.
    let _: UpdateCountdownResponse = ctx
        .call(
            ANNIVERSARIES,
            "UpdateCountdown",
            mika,
            &UpdateCountdownRequest { date: Some(in_days(-1)), repeats_yearly: Some(true), ..change(&exam) },
        )
        .await
        .unwrap();

    let _: DeleteCountdownResponse = ctx
        .call(
            ANNIVERSARIES,
            "DeleteCountdown",
            mika,
            &DeleteCountdownRequest { countdown_id: exam.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(countdowns(&ctx, mika).await.len(), 2);
    assert_eq!(
        ctx.refusal(
            ANNIVERSARIES,
            "DeleteCountdown",
            mika,
            &DeleteCountdownRequest { countdown_id: exam.id.clone(), ..Default::default() }
        )
        .await,
        "COUNTDOWN_NOT_FOUND"
    );

    let stranger = ctx.app.user("Stranger").await;
    let chosen = |shared: Vec<String>| CreateCountdownRequest {
        sharing: CountdownSharing::COUNTDOWN_SHARING_CHOSEN.into(),
        shared_with_ids: shared,
        ..countdown_to("ライブ", in_days(20))
    };
    let cases = [
        (countdown_to("  ", in_days(5)), "TITLE_REQUIRED"),
        (countdown_to(&"あ".repeat(61), in_days(5)), "TITLE_TOO_LONG"),
        (CreateCountdownRequest { emoji: "live".into(), ..countdown_to("ライブ", in_days(5)) }, "EMOJI_INVALID"),
        (countdown_to("ライブ", "来週".into()), "DATE_INVALID"),
        (countdown_to("ライブ", in_days(-2)), "DATE_INVALID"),
        (chosen(Vec::new()), "SHARED_WITH_REQUIRED"),
        (chosen((0..51).map(|_| Uuid::now_v7().to_string()).collect()), "TOO_MANY_SHARED"),
        (chosen(ids(&[&stranger])), "NOT_FRIENDS"),
    ];
    for (request, reason) in cases {
        assert_eq!(ctx.refusal(ANNIVERSARIES, "CreateCountdown", mika, &request).await, reason);
    }

    // Every RPC of the two services needs a signed-in caller.
    let methods = [
        "CreateCountdown",
        "UpdateCountdown",
        "DeleteCountdown",
        "GetCountdown",
        "ListCountdowns",
        "StartFriendshipCounter",
        "InviteCouple",
        "RespondCoupleInvite",
        "UpdateAnniversary",
        "EndAnniversary",
        "ListAnniversaries",
        "ListBirthdays",
        "GetPlansSettings",
        "UpdatePlansSettings",
    ];
    for procedure in
        methods.iter().map(|method| format!("{ANNIVERSARIES}/{method}")).chain([format!("{CALENDAR}/GetCalendar")])
    {
        let error = ctx.app.call::<_, serde_json::Value>(&procedure, None, &serde_json::json!({})).await.unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.reason.as_str()),
            ("unauthenticated", "SIGN_IN_REQUIRED"),
            "{procedure}"
        );
    }
}

#[tokio::test]
async fn a_countdown_is_shared_with_chosen_friends_or_pinned_to_the_profile() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let stranger = ctx.app.user("Stranger").await;

    // Shared with Yui only.
    let trip = create_countdown(
        &ctx,
        mika,
        CreateCountdownRequest {
            emoji: "✈️".into(),
            sharing: CountdownSharing::COUNTDOWN_SHARING_CHOSEN.into(),
            shared_with_ids: ids(&[yui]),
            ..countdown_to("沖縄旅行", in_days(30))
        },
    )
    .await;
    assert_eq!(trip.shared_with.iter().map(|user| user.id.clone()).collect::<Vec<_>>(), ids(&[yui]));
    let yuis = countdowns(&ctx, yui).await;
    assert_eq!(yuis.len(), 1);
    assert_eq!(
        (yuis[0].id.clone(), yuis[0].owner.as_option().unwrap().id.clone(), yuis[0].days_left),
        (trip.id.clone(), mika.id.to_string(), 30)
    );
    // Who else it is shared with is the owner's business.
    assert!(yuis[0].shared_with.is_empty() && yuis[0].sharing == CountdownSharing::COUNTDOWN_SHARING_UNSPECIFIED);
    let seen: GetCountdownResponse = ctx.call(ANNIVERSARIES, "GetCountdown", yui, &countdown_ref(&trip)).await.unwrap();
    assert_eq!(seen.countdown.as_option().unwrap().title, "沖縄旅行");
    assert!(countdowns(&ctx, sota).await.is_empty());
    assert_eq!(ctx.refusal(ANNIVERSARIES, "GetCountdown", sota, &countdown_ref(&trip)).await, "COUNTDOWN_NOT_FOUND");
    // A shared countdown cannot be changed by the people it is shared with.
    assert_eq!(
        ctx.refusal(
            ANNIVERSARIES,
            "DeleteCountdown",
            yui,
            &DeleteCountdownRequest { countdown_id: trip.id.clone(), ..Default::default() }
        )
        .await,
        "COUNTDOWN_NOT_FOUND"
    );

    // Pinned to the profile: friends see it there, strangers do not.
    let pinned = |title: &str, days: i64| CreateCountdownRequest {
        sharing: CountdownSharing::COUNTDOWN_SHARING_PROFILE.into(),
        ..countdown_to(title, in_days(days))
    };
    let festival =
        create_countdown(&ctx, mika, CreateCountdownRequest { emoji: "🎪".into(), ..pinned("文化祭", 12) }).await;
    for viewer in [sota, mika] {
        let cards = ctx.cards(viewer, mika).await;
        assert_eq!(cards.len(), 1);
        assert_eq!((cards[0].kind.as_str(), cards[0].emoji.as_str()), ("plans.countdown", "🎪"));
        assert_eq!(
            (cards[0].title.as_option().unwrap().ja.as_str(), cards[0].text.as_option().unwrap().ja.as_str()),
            ("文化祭", "あと12日")
        );
        assert_eq!(cards[0].text.as_option().unwrap().en, "12 days to go");
        assert_eq!(
            (cards[0].data["countdown_id"].as_str(), cards[0].data["days_left"].as_str()),
            (festival.id.as_str(), "12")
        );
        assert_eq!(cards[0].path, format!("/plans/countdowns/{}", festival.id));
    }
    assert!(ctx.cards(&stranger, mika).await.is_empty());
    // A friend can open what is pinned; it is not listed among "shared with me".
    let opened: GetCountdownResponse =
        ctx.call(ANNIVERSARIES, "GetCountdown", sota, &countdown_ref(&festival)).await.unwrap();
    assert_eq!(opened.countdown.as_option().unwrap().days_left, 12);
    assert!(countdowns(&ctx, sota).await.is_empty());
    assert_eq!(
        ctx.refusal(ANNIVERSARIES, "GetCountdown", &stranger, &countdown_ref(&festival)).await,
        "COUNTDOWN_NOT_FOUND"
    );

    // At most three pins, shown nearest first; an existing countdown can be pinned too.
    create_countdown(&ctx, mika, pinned("テスト", 3)).await;
    create_countdown(&ctx, mika, pinned("修学旅行", 50)).await;
    assert_eq!(ctx.refusal(ANNIVERSARIES, "CreateCountdown", mika, &pinned("卒業式", 100)).await, "TOO_MANY_PINNED");
    let pin_trip = UpdateCountdownRequest {
        countdown_id: trip.id.clone(),
        sharing: CountdownSharing::COUNTDOWN_SHARING_PROFILE.into(),
        ..Default::default()
    };
    assert_eq!(ctx.refusal(ANNIVERSARIES, "UpdateCountdown", mika, &pin_trip).await, "TOO_MANY_PINNED");
    let cards = ctx.cards(yui, mika).await;
    assert_eq!(
        cards.iter().map(|card| card.title.as_option().unwrap().ja.clone()).collect::<Vec<_>>(),
        ["テスト", "文化祭", "修学旅行"]
    );
    assert!(cards.windows(2).all(|pair| pair[0].order < pair[1].order));

    // A pinned day that has passed leaves the profile by itself.
    ctx.sql("UPDATE plan_countdowns SET target_date = current_date - 3 WHERE id = $1", festival.id.parse().unwrap())
        .await;
    assert_eq!(ctx.cards(yui, mika).await.len(), 2);
    let past = countdowns(&ctx, mika).await;
    // In the list it stays, after the days still ahead.
    assert_eq!(
        past.last().map(|countdown| (countdown.id.clone(), countdown.days_left < 0)),
        Some((festival.id.clone(), true))
    );

    // Un-sharing, and the end of a friendship, take a countdown away again.
    let _: UpdateCountdownResponse = ctx
        .call(
            ANNIVERSARIES,
            "UpdateCountdown",
            mika,
            &UpdateCountdownRequest {
                countdown_id: trip.id.clone(),
                sharing: CountdownSharing::COUNTDOWN_SHARING_PRIVATE.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(countdowns(&ctx, yui).await.is_empty());
    ctx.unfriend(mika, sota).await;
    assert!(ctx.cards(sota, mika).await.is_empty());
}

#[tokio::test]
async fn friends_count_the_days_since_they_met() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui"]).await;
    let (mika, yui) = (&users[0], &users[1]);
    let stranger = ctx.app.user("Stranger").await;
    let with = |user: &TestUser, start: &str| StartFriendshipCounterRequest {
        user_id: user.id.to_string(),
        start_date: start.to_owned(),
        ..Default::default()
    };

    // Without a date the counter starts on the day the two became friends: today.
    let mut yuis_phone = ctx.app.listen_user(yui.id).await;
    let started: StartFriendshipCounterResponse =
        ctx.call(ANNIVERSARIES, "StartFriendshipCounter", mika, &with(yui, "")).await.unwrap();
    let counter = started.anniversary.as_option().cloned().unwrap();
    assert_eq!(
        (counter.kind, counter.status),
        (AnniversaryKind::ANNIVERSARY_KIND_FRIENDSHIP.into(), AnniversaryStatus::ANNIVERSARY_STATUS_ACTIVE.into())
    );
    assert_eq!((partner_id(&counter), counter.start_date.clone()), (yui.id.to_string(), date(today())));
    // The first day is day 1.
    assert_eq!((counter.days, counter.count_start_day), (1, true));
    let next = &counter.upcoming[0];
    assert_eq!((next.kind, next.number, next.days_left), (MilestoneKind::MILESTONE_KIND_DAYS.into(), 100, 99));
    assert_eq!(next.label.as_option().unwrap().ja, "100日記念日");
    assert_eq!(counter.upcoming.len(), 3);

    // It belongs to both: the friend sees it, is told about it (quietly), and gets it live.
    let yuis = anniversaries(&ctx, yui).await;
    assert_eq!(yuis.len(), 1);
    assert_eq!((yuis[0].id.clone(), partner_id(&yuis[0])), (counter.id.clone(), mika.id.to_string()));
    let told = ctx.notifications_for(yui.id, "plans.counter_started").await;
    assert_eq!(told.len(), 1);
    assert_eq!((told[0].data["push"].as_bool(), told[0].data["inbox"].as_bool()), (Some(false), Some(true)));
    assert!(
        matches!(next_push(&mut yuis_phone).await, Some(Push::AnniversaryChanged(change)) if change.anniversary.as_option().is_some_and(|a| a.id == counter.id))
    );
    let events = ctx.app.events_by("plans.anniversary.started", mika.id).await;
    assert_eq!(
        (events.len(), events[0].data["kind"].as_i64()),
        (1, Some(AnniversaryKind::ANNIVERSARY_KIND_FRIENDSHIP as i64))
    );

    // One per pair, whoever asks; and only between friends.
    assert_eq!(ctx.refusal(ANNIVERSARIES, "StartFriendshipCounter", yui, &with(mika, "")).await, "ANNIVERSARY_EXISTS");
    assert_eq!(ctx.refusal(ANNIVERSARIES, "StartFriendshipCounter", mika, &with(&stranger, "")).await, "NOT_FRIENDS");
    assert_eq!(ctx.refusal(ANNIVERSARIES, "StartFriendshipCounter", mika, &with(mika, "")).await, "NOT_FRIENDS");

    // Either of the two can correct the start and the way of counting.
    let change = || UpdateAnniversaryRequest { anniversary_id: counter.id.clone(), ..Default::default() };
    let earlier = update_anniversary(
        &ctx,
        yui,
        UpdateAnniversaryRequest {
            start_date: Some(in_days(-200)),
            title: Some("ずっとも".into()),
            emoji: Some("🫶".into()),
            ..change()
        },
    )
    .await;
    assert_eq!((earlier.days, earlier.title.as_str(), earlier.emoji.as_str()), (201, "ずっとも", "🫶"));
    // Day 300 is next: 99 days from day 201.
    assert_eq!((earlier.upcoming[0].number, earlier.upcoming[0].days_left), (300, 99));
    let elapsed =
        update_anniversary(&ctx, mika, UpdateAnniversaryRequest { count_start_day: Some(false), ..change() }).await;
    assert_eq!(elapsed.days, 200);
    assert_eq!(anniversaries(&ctx, yui).await[0].days, 200);

    let cases = [
        (UpdateAnniversaryRequest { start_date: Some(in_days(1)), ..change() }, "DATE_INVALID"),
        (UpdateAnniversaryRequest { start_date: Some("むかし".into()), ..change() }, "DATE_INVALID"),
        (UpdateAnniversaryRequest { title: Some("あ".repeat(41)), ..change() }, "TITLE_TOO_LONG"),
        (UpdateAnniversaryRequest { emoji: Some("bff".into()), ..change() }, "EMOJI_INVALID"),
        // Friendship counters are private to the two.
        (UpdateAnniversaryRequest { show_on_profile: Some(true), ..change() }, "NOT_A_COUPLE"),
    ];
    for (request, reason) in cases {
        assert_eq!(ctx.refusal(ANNIVERSARIES, "UpdateAnniversary", mika, &request).await, reason);
    }
    assert_eq!(
        ctx.refusal(
            ANNIVERSARIES,
            "UpdateAnniversary",
            &stranger,
            &UpdateAnniversaryRequest { title: Some("x".into()), ..change() }
        )
        .await,
        "ANNIVERSARY_NOT_FOUND"
    );
    assert_eq!(
        ctx.refusal(ANNIVERSARIES, "StartFriendshipCounter", mika, &with(yui, &in_days(-2))).await,
        "ANNIVERSARY_EXISTS"
    );

    // Either can end it; the other sees it go.
    let end = EndAnniversaryRequest { anniversary_id: counter.id.clone(), ..Default::default() };
    assert_eq!(ctx.refusal(ANNIVERSARIES, "EndAnniversary", &stranger, &end).await, "ANNIVERSARY_NOT_FOUND");
    let mut mikas_phone = ctx.app.listen_user(mika.id).await;
    let _: EndAnniversaryResponse = ctx.call(ANNIVERSARIES, "EndAnniversary", yui, &end).await.unwrap();
    assert!(
        matches!(next_push(&mut mikas_phone).await, Some(Push::AnniversaryChanged(change)) if change.ended_anniversary_id == counter.id)
    );
    assert!(anniversaries(&ctx, mika).await.is_empty());
    assert_eq!(ctx.refusal(ANNIVERSARIES, "EndAnniversary", yui, &end).await, "ANNIVERSARY_NOT_FOUND");

    // With a date of one's own, and never in the future.
    assert_eq!(
        ctx.refusal(ANNIVERSARIES, "StartFriendshipCounter", mika, &with(yui, &in_days(1))).await,
        "DATE_INVALID"
    );
    let again: StartFriendshipCounterResponse =
        ctx.call(ANNIVERSARIES, "StartFriendshipCounter", mika, &with(yui, &in_days(-364))).await.unwrap();
    assert_eq!(again.anniversary.as_option().unwrap().days, 365);
}

#[tokio::test]
async fn a_couple_counter_exists_once_both_agree() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let stranger = ctx.app.user("Stranger").await;
    let invite = |user: &TestUser| InviteCoupleRequest {
        user_id: user.id.to_string(),
        start_date: in_days(-50),
        ..Default::default()
    };
    let respond = |anniversary: &Anniversary, accept: bool| RespondCoupleInviteRequest {
        anniversary_id: anniversary.id.clone(),
        accept,
        ..Default::default()
    };

    // An invitation is not a counter yet.
    let sent: InviteCoupleResponse = ctx.call(ANNIVERSARIES, "InviteCouple", mika, &invite(yui)).await.unwrap();
    let invitation = sent.anniversary.as_option().cloned().unwrap();
    assert_eq!(
        (invitation.kind, invitation.status),
        (AnniversaryKind::ANNIVERSARY_KIND_COUPLE.into(), AnniversaryStatus::ANNIVERSARY_STATUS_PENDING.into())
    );
    assert!(invitation.invited_by_me && invitation.days == 0 && invitation.upcoming.is_empty());
    let received = anniversaries(&ctx, yui).await;
    assert_eq!((received.len(), received[0].invited_by_me, partner_id(&received[0])), (1, false, mika.id.to_string()));
    let told = ctx.notifications_for(yui.id, "plans.couple_invite").await;
    assert_eq!((told.len(), told[0].actor_id), (1, Some(mika.id)));
    assert!(ctx.app.events_by("plans.anniversary.started", mika.id).await.is_empty());

    // One invitation at a time, one per pair, among friends.
    assert_eq!(ctx.refusal(ANNIVERSARIES, "InviteCouple", mika, &invite(sota)).await, "INVITE_PENDING");
    assert_eq!(ctx.refusal(ANNIVERSARIES, "InviteCouple", yui, &invite(mika)).await, "ANNIVERSARY_EXISTS");
    assert_eq!(ctx.refusal(ANNIVERSARIES, "InviteCouple", mika, &invite(&stranger)).await, "NOT_FRIENDS");
    assert_eq!(
        ctx.refusal(
            ANNIVERSARIES,
            "InviteCouple",
            sota,
            &InviteCoupleRequest { start_date: in_days(1), ..invite(mika) }
        )
        .await,
        "DATE_INVALID"
    );
    // Only the invited person answers; an invitation is not edited.
    assert_eq!(
        ctx.refusal(ANNIVERSARIES, "RespondCoupleInvite", mika, &respond(&invitation, true)).await,
        "ANNIVERSARY_NOT_FOUND"
    );
    assert_eq!(
        ctx.refusal(ANNIVERSARIES, "RespondCoupleInvite", sota, &respond(&invitation, true)).await,
        "ANNIVERSARY_NOT_FOUND"
    );
    let edit = UpdateAnniversaryRequest {
        anniversary_id: invitation.id.clone(),
        title: Some("らぶ".into()),
        ..Default::default()
    };
    assert_eq!(ctx.refusal(ANNIVERSARIES, "UpdateAnniversary", mika, &edit).await, "ANNIVERSARY_NOT_FOUND");

    // Accepted: it counts from the day they named.
    let accepted: RespondCoupleInviteResponse =
        ctx.call(ANNIVERSARIES, "RespondCoupleInvite", yui, &respond(&invitation, true)).await.unwrap();
    let couple = accepted.anniversary.as_option().cloned().unwrap();
    assert_eq!(
        (couple.status, couple.days, couple.invited_by_me),
        (AnniversaryStatus::ANNIVERSARY_STATUS_ACTIVE.into(), 51, false)
    );
    // Day 100 is 49 days away.
    assert_eq!((couple.upcoming[0].number, couple.upcoming[0].days_left), (100, 49));
    let told = ctx.notifications_for(mika.id, "plans.couple_accepted").await;
    assert_eq!((told.len(), told[0].actor_id), (1, Some(yui.id)));
    assert_eq!(ctx.app.events_by("plans.anniversary.started", yui.id).await.len(), 1);
    assert_eq!(
        ctx.refusal(ANNIVERSARIES, "RespondCoupleInvite", yui, &respond(&invitation, true)).await,
        "ANNIVERSARY_NOT_FOUND"
    );

    // It is private to the two until both choose to show it on their profiles.
    assert!(ctx.cards(sota, mika).await.is_empty());
    let show = |show: bool| UpdateAnniversaryRequest {
        anniversary_id: couple.id.clone(),
        show_on_profile: Some(show),
        ..Default::default()
    };
    let shown = update_anniversary(&ctx, mika, show(true)).await;
    assert!(shown.show_on_profile && !shown.partner_shows_on_profile);
    assert!(ctx.cards(sota, mika).await.is_empty() && ctx.cards(mika, mika).await.is_empty());
    update_anniversary(&ctx, yui, show(true)).await;
    for (owner, partner) in [(mika, yui), (yui, mika)] {
        // Sota is a friend of Mika only, so he sees Mika's profile card; both see their own.
        let viewer = if owner.id == mika.id { sota } else { yui };
        let cards = ctx.cards(viewer, owner).await;
        assert_eq!(cards.len(), 1, "on {}'s profile", owner.handle);
        assert_eq!(cards[0].kind, "plans.anniversary");
        assert_eq!(cards[0].title.as_option().unwrap().ja, "カップル");
        let partner_name = if partner.id == yui.id { "Yui" } else { "Mika" };
        assert_eq!(cards[0].text.as_option().unwrap().ja, format!("{partner_name}さんと51日目"));
        assert_eq!(
            (cards[0].data["partner_id"].as_str(), cards[0].data["days"].as_str()),
            (partner.id.to_string().as_str(), "51")
        );
        assert_eq!(cards[0].path, format!("/u/{}", partner.handle));
    }
    assert!(ctx.cards(&stranger, mika).await.is_empty());
    // One of them changing their mind is enough to take it off both profiles.
    update_anniversary(&ctx, yui, show(false)).await;
    assert!(ctx.cards(sota, mika).await.is_empty());

    // Someone in a couple neither invites nor accepts.
    assert_eq!(ctx.refusal(ANNIVERSARIES, "InviteCouple", mika, &invite(sota)).await, "ALREADY_IN_COUPLE");
    let hopeful: InviteCoupleResponse = ctx.call(ANNIVERSARIES, "InviteCouple", sota, &invite(mika)).await.unwrap();
    let hopeful = hopeful.anniversary.as_option().cloned().unwrap();
    assert_eq!(
        ctx.refusal(ANNIVERSARIES, "RespondCoupleInvite", mika, &respond(&hopeful, true)).await,
        "ALREADY_IN_COUPLE"
    );
    // Declining removes the invitation without a word to the sender.
    let mut sotas_phone = ctx.app.listen_user(sota.id).await;
    let declined: RespondCoupleInviteResponse =
        ctx.call(ANNIVERSARIES, "RespondCoupleInvite", mika, &respond(&hopeful, false)).await.unwrap();
    assert!(declined.anniversary.is_unset());
    assert!(anniversaries(&ctx, sota).await.is_empty());
    assert!(pushes(&mut sotas_phone, QUIET).await.is_empty());
    assert!(ctx.notifications_for(sota.id, "plans.couple_accepted").await.is_empty());

    // Either of the two can end it, and both are free again.
    let _: EndAnniversaryResponse = ctx
        .call(
            ANNIVERSARIES,
            "EndAnniversary",
            mika,
            &EndAnniversaryRequest { anniversary_id: couple.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(anniversaries(&ctx, yui).await.is_empty());
    // The sender can withdraw an invitation, which the invited person sees.
    let second: InviteCoupleResponse = ctx.call(ANNIVERSARIES, "InviteCouple", yui, &invite(mika)).await.unwrap();
    let second = second.anniversary.as_option().cloned().unwrap();
    let mut mikas_phone = ctx.app.listen_user(mika.id).await;
    let _: EndAnniversaryResponse = ctx
        .call(
            ANNIVERSARIES,
            "EndAnniversary",
            yui,
            &EndAnniversaryRequest { anniversary_id: second.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(
        matches!(next_push(&mut mikas_phone).await, Some(Push::AnniversaryChanged(change)) if change.ended_anniversary_id == second.id)
    );
    assert!(anniversaries(&ctx, mika).await.is_empty());
}

#[tokio::test]
async fn birthdays_are_shown_only_when_shared() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let stranger = ctx.app.user("Stranger").await;
    let in_ten_days = today() + Duration::days(10);
    ctx.set_birthdate(yui, NaiveDate::from_ymd_opt(2008, in_ten_days.month(), in_ten_days.day().min(28)).unwrap())
        .await;
    ctx.set_birthdate(sota, NaiveDate::from_ymd_opt(2009, 7, 1).unwrap()).await;
    let birthdays = |user: &TestUser| {
        let user = user.clone();
        let ctx = &ctx;
        async move {
            ctx.call::<_, ListBirthdaysResponse>(
                ANNIVERSARIES,
                "ListBirthdays",
                &user,
                &ListBirthdaysRequest::default(),
            )
            .await
            .unwrap()
            .birthdays
        }
    };

    // Sharing a birthday is off until its owner switches it on.
    let settings: GetPlansSettingsResponse =
        ctx.call(ANNIVERSARIES, "GetPlansSettings", yui, &GetPlansSettingsRequest::default()).await.unwrap();
    assert!(!settings.settings.as_option().unwrap().share_birthday);
    assert!(birthdays(mika).await.is_empty());

    let updated: UpdatePlansSettingsResponse = ctx
        .call(
            ANNIVERSARIES,
            "UpdatePlansSettings",
            yui,
            &UpdatePlansSettingsRequest { share_birthday: Some(true), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(updated.settings.as_option().unwrap().share_birthday);
    // An update that names nothing changes nothing.
    let same: UpdatePlansSettingsResponse =
        ctx.call(ANNIVERSARIES, "UpdatePlansSettings", yui, &UpdatePlansSettingsRequest::default()).await.unwrap();
    assert!(same.settings.as_option().unwrap().share_birthday);

    // Friends see the month and the day — and how long until then — never the year.
    let seen = birthdays(mika).await;
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].user.as_option().unwrap().id, yui.id.to_string());
    assert_eq!((seen[0].month as u32, seen[0].day as u32), (in_ten_days.month(), in_ten_days.day().min(28)));
    assert!((0..=10).contains(&seen[0].days_left));
    assert_eq!(&seen[0].next_date[5..], format!("{:02}-{:02}", in_ten_days.month(), in_ten_days.day().min(28)));
    // Not the friend who did not share, and nobody who is not a friend.
    assert!(birthdays(&stranger).await.is_empty());
    assert!(birthdays(yui).await.is_empty());

    let _: UpdatePlansSettingsResponse = ctx
        .call(
            ANNIVERSARIES,
            "UpdatePlansSettings",
            yui,
            &UpdatePlansSettingsRequest { share_birthday: Some(false), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(birthdays(mika).await.is_empty());
}

#[tokio::test]
async fn the_calendar_gathers_everything_dated() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui"]).await;
    let (mika, yui) = (&users[0], &users[1]);

    // Everything happens next month, so the test does not depend on today's date.
    let first = today().with_day(1).unwrap().checked_add_months(Months::new(1)).unwrap();
    let month = first.format("%Y-%m").to_string();
    let on = |day: u32| first.with_day(day).unwrap();
    let tokyo = |day: u32, hour: u32| {
        Tokyo.from_local_datetime(&on(day).and_hms_opt(hour, 0, 0).unwrap()).unwrap().with_timezone(&Utc)
    };

    // Two plans: one with a time (Yui invited), one on a day. A third one Yui declined.
    let karaoke = ctx
        .create_plan(mika, CreatePlanRequest { emoji: "🎤".into(), ..plan_at("カラオケ", tokyo(10, 18), &[yui]) })
        .await;
    let trip = ctx
        .create_plan(
            mika,
            CreatePlanRequest {
                title: "日帰り旅行".into(),
                time: PlanTime { start: Some(plan_time::Start::StartDate(date(on(20)))), ..Default::default() }.into(),
                ..Default::default()
            },
        )
        .await;
    let declined = ctx.create_plan(mika, plan_at("行けないやつ", tokyo(11, 12), &[yui])).await;
    ctx.rsvp(yui, &declined.id, Rsvp::RSVP_DECLINED).await;
    // A poll whose answers are due on the 5th at noon.
    let poll = ctx
        .create_poll(
            mika,
            CreatePollRequest {
                title: "打ち上げいつ?".into(),
                slots: vec![PollSlotInput {
                    start: Some(poll_slot_input::Start::Date(date(on(25)))),
                    ..Default::default()
                }],
                invitee_ids: ids(&[yui]),
                deadline: timestamp(tokyo(5, 12)).into(),
                ..Default::default()
            },
        )
        .await;
    // Mika's countdown on the 15th; Yui's, shared with Mika, on the 16th; one Yui keeps to herself.
    let exam = create_countdown(
        &ctx,
        mika,
        CreateCountdownRequest { emoji: "📝".into(), ..countdown_to("期末テスト", date(on(15))) },
    )
    .await;
    let live = create_countdown(
        &ctx,
        yui,
        CreateCountdownRequest {
            sharing: CountdownSharing::COUNTDOWN_SHARING_CHOSEN.into(),
            shared_with_ids: ids(&[mika]),
            ..countdown_to("ライブ", date(on(16)))
        },
    )
    .await;
    create_countdown(&ctx, yui, countdown_to("ひみつ", date(on(17)))).await;
    // A friendship counter whose day 100 falls on the 12th.
    let request = StartFriendshipCounterRequest {
        user_id: yui.id.to_string(),
        start_date: date(on(12) - Duration::days(99)),
        ..Default::default()
    };
    let counter: StartFriendshipCounterResponse =
        ctx.call(ANNIVERSARIES, "StartFriendshipCounter", mika, &request).await.unwrap();
    let counter = counter.anniversary.as_option().cloned().unwrap();
    // Yui shares her birthday (the 8th); Mika's own is on the 28th.
    ctx.set_birthdate(yui, NaiveDate::from_ymd_opt(2008, first.month(), 8).unwrap()).await;
    ctx.set_birthdate(mika, NaiveDate::from_ymd_opt(2007, first.month(), 28).unwrap()).await;
    let _: UpdatePlansSettingsResponse = ctx
        .call(
            ANNIVERSARIES,
            "UpdatePlansSettings",
            yui,
            &UpdatePlansSettingsRequest { share_birthday: Some(true), ..Default::default() },
        )
        .await
        .unwrap();

    let calendar = |user: &TestUser, request: GetCalendarRequest| {
        let user = user.clone();
        let ctx = &ctx;
        async move { ctx.call::<_, GetCalendarResponse>(CALENDAR, "GetCalendar", &user, &request).await.unwrap().entries }
    };
    let entries = calendar(mika, GetCalendarRequest { month: month.clone(), ..Default::default() }).await;
    let summary: Vec<(String, CalendarEntryKind, String)> = entries
        .iter()
        .map(|entry| (entry.date.clone(), entry.kind.as_known().unwrap(), entry.ref_id.clone()))
        .collect();
    assert_eq!(
        summary,
        [
            (date(on(5)), CalendarEntryKind::CALENDAR_ENTRY_KIND_POLL_DEADLINE, poll.id.clone()),
            (date(on(8)), CalendarEntryKind::CALENDAR_ENTRY_KIND_BIRTHDAY, String::new()),
            (date(on(10)), CalendarEntryKind::CALENDAR_ENTRY_KIND_PLAN, karaoke.id.clone()),
            (date(on(11)), CalendarEntryKind::CALENDAR_ENTRY_KIND_PLAN, declined.id.clone()),
            (date(on(12)), CalendarEntryKind::CALENDAR_ENTRY_KIND_ANNIVERSARY, counter.id.clone()),
            (date(on(15)), CalendarEntryKind::CALENDAR_ENTRY_KIND_COUNTDOWN, exam.id.clone()),
            (date(on(16)), CalendarEntryKind::CALENDAR_ENTRY_KIND_COUNTDOWN, live.id.clone()),
            (date(on(20)), CalendarEntryKind::CALENDAR_ENTRY_KIND_PLAN, trip.id.clone()),
            (date(on(28)), CalendarEntryKind::CALENDAR_ENTRY_KIND_BIRTHDAY, String::new()),
        ]
    );

    // What each kind of entry carries.
    let deadline = &entries[0];
    assert_eq!(
        (deadline.title.as_str(), deadline.label.as_option().unwrap().ja.as_str()),
        ("打ち上げいつ?", "回答しめきり")
    );
    assert_eq!(
        (time_of(&deadline.start_at), deadline.path.clone()),
        (tokyo(5, 12), format!("/plans/polls/{}", poll.short_id))
    );
    let birthday = &entries[1];
    assert_eq!(
        (birthday.user.as_option().unwrap().id.clone(), birthday.label.as_option().unwrap().ja.as_str()),
        (yui.id.to_string(), "Yuiさんの誕生日")
    );
    assert!(birthday.start_at.is_unset());
    let plan = &entries[2];
    assert_eq!(
        (plan.title.as_str(), plan.emoji.as_str(), plan.rsvp, time_of(&plan.start_at)),
        ("カラオケ", "🎤", Rsvp::RSVP_GOING.into(), tokyo(10, 18))
    );
    assert_eq!(plan.path, format!("/plans/{}", karaoke.id));
    let milestone = &entries[4];
    assert_eq!(
        (milestone.label.as_option().unwrap().ja.as_str(), milestone.user.as_option().unwrap().id.clone()),
        ("Yuiさんと100日記念日", yui.id.to_string())
    );
    let shared = &entries[6];
    assert_eq!((shared.title.as_str(), shared.user.as_option().unwrap().id.clone()), ("ライブ", yui.id.to_string()));
    assert!(entries[5].user.is_unset() && entries[7].start_at.is_unset());
    assert_eq!(entries[8].label.as_option().unwrap().ja, "自分の誕生日");

    // Yui's month: no plan she declined, her own countdowns, no birthday of Mika (not shared).
    let yuis = calendar(yui, GetCalendarRequest { month: month.clone(), ..Default::default() }).await;
    let yuis: Vec<(String, CalendarEntryKind)> =
        yuis.iter().map(|entry| (entry.date.clone(), entry.kind.as_known().unwrap())).collect();
    assert_eq!(
        yuis,
        [
            (date(on(5)), CalendarEntryKind::CALENDAR_ENTRY_KIND_POLL_DEADLINE),
            (date(on(8)), CalendarEntryKind::CALENDAR_ENTRY_KIND_BIRTHDAY),
            (date(on(10)), CalendarEntryKind::CALENDAR_ENTRY_KIND_PLAN),
            (date(on(12)), CalendarEntryKind::CALENDAR_ENTRY_KIND_ANNIVERSARY),
            (date(on(16)), CalendarEntryKind::CALENDAR_ENTRY_KIND_COUNTDOWN),
            (date(on(17)), CalendarEntryKind::CALENDAR_ENTRY_KIND_COUNTDOWN),
        ]
    );

    // Which day a time falls on depends on the time zone asked for: 18:00 in
    // Tokyo on the 10th is still the 9th in Honolulu.
    let abroad = calendar(
        mika,
        GetCalendarRequest { month: month.clone(), time_zone: "Pacific/Honolulu".into(), ..Default::default() },
    )
    .await;
    let karaoke_abroad = abroad.iter().find(|entry| entry.ref_id == karaoke.id).unwrap();
    assert_eq!(karaoke_abroad.date, date(on(9)));
    // Days without a time stay where they were written.
    assert_eq!(abroad.iter().find(|entry| entry.ref_id == trip.id).unwrap().date, date(on(20)));

    // The month after holds what repeats — the birthdays do not fall into it — and nothing else.
    let after = first.checked_add_months(Months::new(1)).unwrap().format("%Y-%m").to_string();
    assert!(calendar(mika, GetCalendarRequest { month: after, ..Default::default() }).await.is_empty());

    for bad in ["", "2026", "2026-13", "next month"] {
        assert_eq!(
            ctx.refusal(CALENDAR, "GetCalendar", mika, &GetCalendarRequest { month: bad.into(), ..Default::default() })
                .await,
            "MONTH_INVALID"
        );
    }
    assert_eq!(
        ctx.refusal(
            CALENDAR,
            "GetCalendar",
            mika,
            &GetCalendarRequest { month, time_zone: "Mars/Olympus".into(), ..Default::default() }
        )
        .await,
        "TIME_ZONE_INVALID"
    );
}
