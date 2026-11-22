//! What happens without anyone calling an RPC: reminders and other things
//! that come due, reactions to other domains' events, and the data export.
//!
//! Time is moved by editing the `*_due_at` columns (`Ctx::make_due`): a test
//! cannot wait for tomorrow. Events are delivered by calling the handler
//! directly (no relay runs in tests); `tests/workers.rs` checks the wiring.
//! Milestones and countdowns have their own file, `tests/milestones.rs`.
mod common;

use chrono::{DateTime, Duration, Utc};
use common::*;
use tokuly_core::events::Event;
use tokuly_proto::{
    common::SubjectType,
    plans::{plans_event::Event as Push, *},
};
use tokuly_testkit::TestUser;
use uuid::Uuid;

fn in_days(days: i64) -> DateTime<Utc> {
    Utc::now() + Duration::days(days)
}

fn event(topic: &str, actor: Uuid, data: serde_json::Value) -> Event {
    Event { id: Uuid::now_v7(), topic: topic.to_owned(), actor_id: Some(actor), occurred_at: Utc::now(), data }
}

fn id_of(text: &str) -> Uuid {
    text.parse().expect("an id")
}

async fn count(ctx: &Ctx, query: &'static str, id: Uuid) -> i64 {
    sqlx::query_scalar(query).bind(id).fetch_one(&ctx.app.core.db).await.expect("count")
}

#[tokio::test]
async fn reminders_go_out_the_day_before_and_shortly_before() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota", "Hana", "Kenta"]).await;
    let (mika, yui, sota, hana, kenta) = (&users[0], &users[1], &users[2], &users[3], &users[4]);
    let plan = ctx
        .create_plan(
            mika,
            CreatePlanRequest {
                emoji: "🎆".into(),
                place: PlanPlace { name: "河川敷".into(), ..Default::default() }.into(),
                ..plan_at("花火", in_days(3), &[yui, sota, hana, kenta])
            },
        )
        .await;
    let plan_id = id_of(&plan.id);
    ctx.rsvp(yui, &plan.id, Rsvp::RSVP_GOING).await;
    ctx.rsvp(sota, &plan.id, Rsvp::RSVP_MAYBE).await;
    ctx.rsvp(hana, &plan.id, Rsvp::RSVP_DECLINED).await;
    // Kenta never answers.

    // Both reminders are scheduled, and neither is due yet.
    assert_eq!(count(&ctx, "SELECT count(*) FROM plans WHERE id = $1 AND day_before_due_at > now() AND before_due_at > day_before_due_at", plan_id).await, 1);
    ctx.work().await;
    assert!(ctx.notifications_for(yui.id, "plans.reminder").await.is_empty());

    // The day before: everyone going or maybe going is reminded, once.
    assert_eq!(
        ctx.make_due("UPDATE plans SET day_before_due_at = now() - interval '1 minute' WHERE id = $1", &[plan_id])
            .await
            .day_before_reminders,
        1
    );
    for reminded in [mika, yui, sota] {
        let notices = ctx.notifications_for(reminded.id, "plans.reminder").await;
        assert_eq!(notices.len(), 1, "{}", reminded.handle);
        assert_eq!(notices[0].data["title"]["ja"], serde_json::json!("明日は「🎆 花火」!"));
        assert_eq!(notices[0].data["collapse_key"], serde_json::json!(format!("plans.reminder:{}", plan.id)));
        assert_eq!(notices[0].data["subject"], serde_json::json!([SubjectType::SUBJECT_TYPE_PLAN as i32, plan.id]));
    }
    for quiet in [hana, kenta] {
        assert!(ctx.notifications_for(quiet.id, "plans.reminder").await.is_empty());
    }
    ctx.work().await;
    assert_eq!(ctx.notifications_for(yui.id, "plans.reminder").await.len(), 1);

    // Shortly before: the configured hour, with the place.
    assert_eq!(ctx.make_due("UPDATE plans SET sort_at = now() + interval '60 minutes', before_due_at = now() - interval '1 second' WHERE id = $1", &[plan_id]).await.soon_reminders, 1);
    let notices = ctx.notifications_for(sota.id, "plans.reminder").await;
    assert_eq!(notices.len(), 2);
    assert_eq!(notices[1].data["title"]["ja"], serde_json::json!("もうすぐ「🎆 花火」!"));
    assert_eq!(notices[1].data["body"]["ja"], serde_json::json!("あと1時間で始まるよ! 場所:河川敷"));

    // Changing the time schedules both again.
    let moved = UpdatePlanRequest {
        plan_id: plan.id.clone(),
        time: PlanTime { start: Some(plan_time::Start::StartAt(Box::new(in_days(5).into()))), ..Default::default() }
            .into(),
        ..Default::default()
    };
    let _: UpdatePlanResponse = ctx.call(PLANS, "UpdatePlan", mika, &moved).await.unwrap();
    assert_eq!(
        count(
            &ctx,
            "SELECT count(*) FROM plans WHERE id = $1 AND day_before_due_at > now() AND before_due_at > now()",
            plan_id
        )
        .await,
        1
    );
    // Switching a reminder off unschedules it.
    let off = UpdatePlanRequest {
        plan_id: plan.id.clone(),
        remind_day_before: Some(false),
        remind_minutes_before: Some(0),
        ..Default::default()
    };
    let _: UpdatePlanResponse = ctx.call(PLANS, "UpdatePlan", mika, &off).await.unwrap();
    assert_eq!(
        count(
            &ctx,
            "SELECT count(*) FROM plans WHERE id = $1 AND day_before_due_at IS NULL AND before_due_at IS NULL",
            plan_id
        )
        .await,
        1
    );

    // A reminder that would come after the start (the worker was down) is dropped.
    let late = ctx.create_plan(mika, plan_at("遅刻", in_days(3), &[yui])).await;
    ctx.rsvp(yui, &late.id, Rsvp::RSVP_GOING).await;
    ctx.make_due("UPDATE plans SET sort_at = now() - interval '1 hour', day_before_due_at = now() - interval '1 day' WHERE id = $1", &[id_of(&late.id)]).await;
    assert_eq!(ctx.notifications_for(yui.id, "plans.reminder").await.len(), 2);
    assert_eq!(
        count(&ctx, "SELECT count(*) FROM plans WHERE id = $1 AND day_before_due_at IS NULL", id_of(&late.id)).await,
        1
    );

    // A cancelled plan reminds nobody; one made too late for "tomorrow" only reminds shortly before.
    let cancelled = ctx.create_plan(mika, plan_at("中止", in_days(3), &[yui])).await;
    let _: CancelPlanResponse = ctx
        .call(PLANS, "CancelPlan", mika, &CancelPlanRequest { plan_id: cancelled.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(count(&ctx, "SELECT count(*) FROM plans WHERE id = $1 AND day_before_due_at IS NULL AND before_due_at IS NULL AND attended_due_at IS NULL", id_of(&cancelled.id)).await, 1);
    let tonight = ctx.create_plan(mika, plan_at("今夜", Utc::now() + Duration::hours(5), &[])).await;
    assert_eq!(
        count(
            &ctx,
            "SELECT count(*) FROM plans WHERE id = $1 AND day_before_due_at IS NULL AND before_due_at > now()",
            id_of(&tonight.id)
        )
        .await,
        1
    );
}

#[tokio::test]
async fn attendance_is_announced_once_the_plan_is_over() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let plan = ctx
        .create_plan(
            mika,
            CreatePlanRequest {
                place: PlanPlace {
                    name: "駅前".into(),
                    location: tokuly_proto::common::LatLng { lat: 35.66, lng: 139.7, ..Default::default() }.into(),
                    ..Default::default()
                }
                .into(),
                ..plan_at("ボウリング", in_days(1), &[yui, sota])
            },
        )
        .await;
    let plan_id = id_of(&plan.id);
    ctx.rsvp(yui, &plan.id, Rsvp::RSVP_GOING).await;
    ctx.rsvp(sota, &plan.id, Rsvp::RSVP_DECLINED).await;

    // Nothing while the plan lies ahead.
    ctx.work().await;
    assert!(ctx.app.events_by("plans.plan.attended", yui.id).await.is_empty());

    assert_eq!(
        ctx.make_due("UPDATE plans SET attended_due_at = now() - interval '1 minute' WHERE id = $1", &[plan_id])
            .await
            .plans_attended,
        1
    );
    for (attendee, other) in [(mika, yui), (yui, mika)] {
        // One event per person who was going...
        let attended = ctx.app.events_by("plans.plan.attended", attendee.id).await;
        assert_eq!(attended.len(), 1);
        assert_eq!(
            (attended[0].data["plan_id"].clone(), attended[0].data["host_id"].clone()),
            (serde_json::json!(plan.id), serde_json::json!(mika.id))
        );
        assert_eq!(attended[0].data["attendee_ids"].as_array().unwrap().len(), 2);
        // ...and an entry in their own log, naming the others: nobody outside the plan sees it.
        let logged = ctx.activities_of(attendee.id, "plans.attended").await;
        assert_eq!(logged.len(), 1);
        assert_eq!(logged[0].data["audience"], serde_json::json!("private"));
        assert_eq!(logged[0].data["with_user_ids"], serde_json::json!([other.id]));
        assert_eq!(logged[0].data["summary"]["ja"], serde_json::json!("「ボウリング」に参加しました"));
        assert_eq!(logged[0].data["location"], serde_json::json!([35.66, 139.7]));
    }
    assert!(ctx.app.events_by("plans.plan.attended", sota.id).await.is_empty());
    // Once.
    ctx.work().await;
    assert_eq!(ctx.app.events_by("plans.plan.attended", yui.id).await.len(), 1);

    // A plan written down after the fact counts nobody as having attended.
    let yesterday = ctx.create_plan(mika, plan_at("きのうのごはん", in_days(-1), &[yui])).await;
    assert_eq!(
        count(&ctx, "SELECT count(*) FROM plans WHERE id = $1 AND attended_due_at IS NULL", id_of(&yesterday.id)).await,
        1
    );
    // Neither does a plan without a time.
    let someday = ctx.create_plan(mika, CreatePlanRequest { title: "いつか".into(), ..Default::default() }).await;
    assert_eq!(
        count(&ctx, "SELECT count(*) FROM plans WHERE id = $1 AND attended_due_at IS NULL", id_of(&someday.id)).await,
        1
    );
}

#[tokio::test]
async fn polls_remind_the_quiet_ones_and_close_at_their_deadline() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let day = |days: i64| PollSlotInput {
        start: Some(poll_slot_input::Start::Date((Utc::now() + Duration::days(days)).format("%Y-%m-%d").to_string())),
        ..Default::default()
    };
    let request = CreatePollRequest {
        title: "忘年会".into(),
        slots: vec![day(10), day(11)],
        invitee_ids: ids(&[yui, sota]),
        deadline: timestamp(in_days(4)).into(),
        ..Default::default()
    };
    let poll = ctx.create_poll(mika, request).await;
    let poll_id = id_of(&poll.id);
    ctx.answer_poll(yui, &poll, &[PollAnswer::POLL_ANSWER_YES, PollAnswer::POLL_ANSWER_NO]).await;

    // The automatic reminder is scheduled for the day before the deadline.
    assert_eq!(
        count(
            &ctx,
            "SELECT count(*) FROM date_polls WHERE id = $1 AND remind_due_at = deadline - interval '24 hours'",
            poll_id
        )
        .await,
        1
    );
    ctx.work().await;
    assert!(ctx.notifications_for(sota.id, "plans.poll_reminder").await.is_empty());

    // When it is due, the invited people who have not answered hear about it, once.
    assert_eq!(
        ctx.make_due("UPDATE date_polls SET remind_due_at = now() - interval '1 minute' WHERE id = $1", &[poll_id])
            .await
            .polls_reminded,
        1
    );
    let reminded = ctx.notifications_for(sota.id, "plans.poll_reminder").await;
    assert_eq!(reminded.len(), 1);
    assert_eq!(reminded[0].data["title"]["ja"], serde_json::json!("日程調整の回答がまだだよ!"));
    assert_eq!(reminded[0].data["path"], serde_json::json!(format!("/plans/polls/{}", poll.short_id)));
    for quiet in [yui, mika] {
        assert!(ctx.notifications_for(quiet.id, "plans.poll_reminder").await.is_empty());
    }
    ctx.work().await;
    assert_eq!(ctx.notifications_for(sota.id, "plans.poll_reminder").await.len(), 1);

    // At the deadline the poll closes by itself and its creator is told to pick a day.
    let mut yuis_phone = ctx.app.listen_user(yui.id).await;
    assert_eq!(
        ctx.make_due("UPDATE date_polls SET deadline = now() - interval '1 minute' WHERE id = $1", &[poll_id])
            .await
            .polls_closed,
        1
    );
    assert_eq!(ctx.get_poll(yui, &poll.id).await.status, PollStatus::POLL_STATUS_CLOSED);
    assert!(
        matches!(next_push(&mut yuis_phone).await, Some(Push::PollChanged(change)) if change.status == PollStatus::POLL_STATUS_CLOSED)
    );
    let told = ctx.notifications_for(mika.id, "plans.poll_closed").await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].data["body"]["ja"], serde_json::json!("「忘年会」日程を決めよう!"));
    let late = AnswerPollRequest { poll_id: poll.id.clone(), ..Default::default() };
    assert_eq!(ctx.refusal(POLLS, "AnswerPoll", sota, &late).await, "POLL_NOT_OPEN");

    // Reopening drops the deadline that has passed, so the poll stays open.
    let reopened: ReopenPollResponse = ctx
        .call(POLLS, "ReopenPoll", mika, &ReopenPollRequest { poll_id: poll.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert!(reopened.poll.as_option().unwrap().deadline.is_unset());
    ctx.work().await;
    assert_eq!(ctx.get_poll(yui, &poll.id).await.status, PollStatus::POLL_STATUS_OPEN);

    // A poll without a deadline reminds two days after it was made.
    let open_ended = ctx
        .create_poll(
            mika,
            CreatePollRequest {
                title: "いつでも".into(),
                slots: vec![day(20)],
                invitee_ids: ids(&[yui]),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(
        count(
            &ctx,
            "SELECT count(*) FROM date_polls WHERE id = $1 AND remind_due_at > now() + interval '47 hours'",
            id_of(&open_ended.id)
        )
        .await,
        1
    );
}

#[tokio::test]
async fn a_deleted_account_leaves_nothing_behind() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    ctx.befriend(yui, sota).await;
    let free = |audience: FreeAudience, chosen: &[&TestUser]| SetFreeStatusRequest {
        window: FreeWindow::FREE_WINDOW_NOW.into(),
        audience: audience.into(),
        chosen_user_ids: ids(chosen),
        ..Default::default()
    };

    // ひま: statuses on both sides, an answer, alerts in both directions.
    let _: SetFreeStatusResponse =
        ctx.call(FREE, "SetFreeStatus", yui, &free(FreeAudience::FREE_AUDIENCE_FRIENDS, &[])).await.unwrap();
    let _: SetFreeStatusResponse =
        ctx.call(FREE, "SetFreeStatus", mika, &free(FreeAudience::FREE_AUDIENCE_CHOSEN, &[yui, sota])).await.unwrap();
    let _: AnswerFreeStatusResponse = ctx
        .call(
            FREE,
            "AnswerFreeStatus",
            yui,
            &AnswerFreeStatusRequest { user_id: mika.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    for (user, friend) in [(yui, mika), (mika, yui)] {
        let _: SetFreeAlertResponse = ctx
            .call(
                FREE,
                "SetFreeAlert",
                user,
                &SetFreeAlertRequest { user_id: friend.id.to_string(), enabled: true, ..Default::default() },
            )
            .await
            .unwrap();
    }

    // Plans: one Yui hosts with a co-host, one where she is part of the cost
    // split, one where she merely takes a place others wait for, one of her own.
    let hosted = ctx.create_plan(yui, plan_at("Yuiの会", in_days(3), &[mika, sota])).await;
    ctx.rsvp(mika, &hosted.id, Rsvp::RSVP_GOING).await;
    let co_host = SetPlanRoleRequest {
        plan_id: hosted.id.clone(),
        user_id: mika.id.to_string(),
        role: PlanRole::PLAN_ROLE_CO_HOST.into(),
        ..Default::default()
    };
    let _: SetPlanRoleResponse = ctx.call(PLANS, "SetPlanRole", yui, &co_host).await.unwrap();
    let split = ctx.create_plan(mika, plan_at("割り勘の会", in_days(3), &[yui])).await;
    let going = RespondToPlanRequest {
        plan_id: split.id.clone(),
        rsvp: Rsvp::RSVP_GOING.into(),
        comment: "払うね".into(),
        ..Default::default()
    };
    let _: RespondToPlanResponse = ctx.call(PLANS, "RespondToPlan", yui, &going).await.unwrap();
    let dinner = ExpenseInput {
        title: "ごはん".into(),
        amount: 2000,
        method: SplitMethod::SPLIT_METHOD_EQUAL.into(),
        shares: [mika, yui]
            .iter()
            .map(|user| ExpenseShare { user_id: user.id.to_string(), ..Default::default() })
            .collect(),
        ..Default::default()
    };
    let _: AddExpenseResponse = ctx
        .call(
            PLANS,
            "AddExpense",
            mika,
            &AddExpenseRequest { plan_id: split.id.clone(), expense: dinner.into(), ..Default::default() },
        )
        .await
        .unwrap();
    let full =
        ctx.create_plan(mika, CreatePlanRequest { capacity: 2, ..plan_at("満員の会", in_days(3), &[yui, sota]) }).await;
    ctx.rsvp(yui, &full.id, Rsvp::RSVP_GOING).await;
    assert_eq!(ctx.rsvp(sota, &full.id, Rsvp::RSVP_GOING).await, Rsvp::RSVP_WAITLISTED);
    let item: AddChecklistItemResponse = ctx
        .call(
            PLANS,
            "AddChecklistItem",
            mika,
            &AddChecklistItemRequest { plan_id: full.id.clone(), text: "お菓子".into(), ..Default::default() },
        )
        .await
        .unwrap();
    let item = item.item.as_option().cloned().unwrap();
    let _: UpdateChecklistItemResponse = ctx
        .call(
            PLANS,
            "UpdateChecklistItem",
            yui,
            &UpdateChecklistItemRequest { item_id: item.id.clone(), claimed: Some(true), ..Default::default() },
        )
        .await
        .unwrap();
    let alone = ctx.create_plan(yui, plan_at("ひとりカラオケ", in_days(3), &[])).await;

    // Polls: one of hers, one of Mika's she answered.
    let day = |days: i64| PollSlotInput {
        start: Some(poll_slot_input::Start::Date((Utc::now() + Duration::days(days)).format("%Y-%m-%d").to_string())),
        ..Default::default()
    };
    let hers = ctx
        .create_poll(
            yui,
            CreatePollRequest {
                title: "Yuiの調整".into(),
                slots: vec![day(5)],
                invitee_ids: ids(&[mika]),
                ..Default::default()
            },
        )
        .await;
    let mikas = ctx
        .create_poll(
            mika,
            CreatePollRequest {
                title: "Mikaの調整".into(),
                slots: vec![day(5)],
                invitee_ids: ids(&[yui]),
                ..Default::default()
            },
        )
        .await;
    ctx.answer_poll(yui, &mikas, &[PollAnswer::POLL_ANSWER_YES]).await;

    // Countdowns, a counter, the birthday setting.
    let shared = CreateCountdownRequest {
        title: "旅行".into(),
        date: (Utc::now() + Duration::days(30)).format("%Y-%m-%d").to_string(),
        sharing: CountdownSharing::COUNTDOWN_SHARING_CHOSEN.into(),
        shared_with_ids: ids(&[yui]),
        ..Default::default()
    };
    let shared: CreateCountdownResponse = ctx.call(ANNIVERSARIES, "CreateCountdown", mika, &shared).await.unwrap();
    let shared = shared.countdown.as_option().cloned().unwrap();
    let _: CreateCountdownResponse = ctx
        .call(
            ANNIVERSARIES,
            "CreateCountdown",
            yui,
            &CreateCountdownRequest { title: "テスト".into(), date: shared.date.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let _: StartFriendshipCounterResponse = ctx
        .call(
            ANNIVERSARIES,
            "StartFriendshipCounter",
            yui,
            &StartFriendshipCounterRequest { user_id: mika.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    let _: UpdatePlansSettingsResponse = ctx
        .call(
            ANNIVERSARIES,
            "UpdatePlansSettings",
            yui,
            &UpdatePlansSettingsRequest { share_birthday: Some(true), ..Default::default() },
        )
        .await
        .unwrap();

    let mut mikas_phone = ctx.app.listen_user(mika.id).await;
    let deleted = event("identity.user.deleted", yui.id, serde_json::json!({ "user_id": yui.id }));
    ctx.plans.handle_event(&deleted).await.unwrap();
    // Events are delivered at least once.
    ctx.plans.handle_event(&deleted).await.unwrap();

    // ひま: her status, her answer, her alerts and her place among the chosen are gone.
    assert_eq!(count(&ctx, "SELECT count(*) FROM free_statuses WHERE user_id = $1", yui.id).await, 0);
    assert_eq!(count(&ctx, "SELECT count(*) FROM free_alerts WHERE user_id = $1 OR friend_id = $1", yui.id).await, 0);
    assert_eq!(count(&ctx, "SELECT count(*) FROM free_answers WHERE user_id = $1", yui.id).await, 0);
    let own: GetFreeStatusResponse =
        ctx.call(FREE, "GetFreeStatus", mika, &GetFreeStatusRequest::default()).await.unwrap();
    let own = own.status.as_option().unwrap();
    assert_eq!(
        (own.chosen.iter().map(|user| user.id.clone()).collect::<Vec<_>>(), own.answer_count),
        (ids(&[sota]), 0)
    );

    // The plan she hosted passed to the co-host.
    let hosted = ctx.get_plan(mika, &hosted.id).await;
    assert_eq!(
        (hosted.host_user_id.clone(), hosted.my_role, hosted.participants.len()),
        (mika.id.to_string(), PlanRole::PLAN_ROLE_HOST.into(), 2)
    );
    // Where she is part of the split her place stays, without a word of hers, so the numbers still add up.
    let split = ctx.get_plan(mika, &split.id).await;
    let kept = participant(&split, yui);
    assert_eq!(
        (kept.rsvp, kept.role, kept.comment.as_str()),
        (Rsvp::RSVP_DECLINED.into(), PlanRole::PLAN_ROLE_GUEST.into(), "")
    );
    let state: GetSplitResponse = ctx
        .call(PLANS, "GetSplit", mika, &GetSplitRequest { plan_id: split.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(state.splits[0].balances.iter().map(|balance| balance.net).collect::<Vec<_>>().iter().sum::<i64>(), 0);
    assert_eq!(state.splits[0].transfers.len(), 1);
    // Elsewhere she is simply gone: her place goes to the waitlist, her claim is released.
    let full = ctx.get_plan(mika, &full.id).await;
    assert_eq!(full.participants.len(), 2);
    assert_eq!(participant(&full, sota).rsvp, Rsvp::RSVP_GOING);
    assert!(full.checklist[0].claimed_by_user_id.is_empty());
    assert_eq!(ctx.notifications_for(sota.id, "plans.promoted").await.len(), 1);
    // A plan with nobody else in it disappears.
    assert_eq!(count(&ctx, "SELECT count(*) FROM plans WHERE id = $1", id_of(&alone.id)).await, 0);

    // Her poll is gone for everyone; in Mika's poll her answers are.
    assert_eq!(
        ctx.refusal(POLLS, "GetPoll", mika, &GetPollRequest { poll_id: hers.id.clone(), ..Default::default() }).await,
        "POLL_NOT_FOUND"
    );
    let mikas = ctx.get_poll(mika, &mikas.id).await;
    assert_eq!((mikas.participant_count, mikas.slots[0].yes_count), (1, 0));

    // Countdowns, counters and settings.
    assert_eq!(count(&ctx, "SELECT count(*) FROM plan_countdowns WHERE user_id = $1", yui.id).await, 0);
    let shared: GetCountdownResponse = ctx
        .call(
            ANNIVERSARIES,
            "GetCountdown",
            mika,
            &GetCountdownRequest { countdown_id: shared.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(shared.countdown.as_option().unwrap().shared_with.is_empty());
    let counters: ListAnniversariesResponse =
        ctx.call(ANNIVERSARIES, "ListAnniversaries", mika, &ListAnniversariesRequest::default()).await.unwrap();
    assert!(counters.anniversaries.is_empty());
    assert_eq!(count(&ctx, "SELECT count(*) FROM plan_settings WHERE user_id = $1", yui.id).await, 0);

    // The people left behind saw it happen.
    let seen = pushes(&mut mikas_phone, QUIET).await;
    assert!(
        seen.iter()
            .any(|push| matches!(push, Push::AnniversaryChanged(change) if !change.ended_anniversary_id.is_empty()))
    );
    assert!(
        seen.iter()
            .any(|push| matches!(push, Push::PollChanged(change) if change.deleted && change.poll_id == hers.id))
    );
    assert!(seen.iter().any(
        |push| matches!(push, Push::PlanChanged(change) if change.change == PlanChange::PLAN_CHANGE_PARTICIPANTS)
    ));
}

#[tokio::test]
async fn an_ended_friendship_or_a_block_separates_two_people() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let day = |days: i64| PollSlotInput {
        start: Some(poll_slot_input::Start::Date((Utc::now() + Duration::days(days)).format("%Y-%m-%d").to_string())),
        ..Default::default()
    };

    // Mika's audiences hold both friends.
    let status = SetFreeStatusRequest {
        window: FreeWindow::FREE_WINDOW_NOW.into(),
        audience: FreeAudience::FREE_AUDIENCE_CHOSEN.into(),
        chosen_user_ids: ids(&[yui, sota]),
        ..Default::default()
    };
    let _: SetFreeStatusResponse = ctx.call(FREE, "SetFreeStatus", mika, &status).await.unwrap();
    let _: AnswerFreeStatusResponse = ctx
        .call(
            FREE,
            "AnswerFreeStatus",
            yui,
            &AnswerFreeStatusRequest { user_id: mika.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    let _: SetFreeAlertResponse = ctx
        .call(
            FREE,
            "SetFreeAlert",
            yui,
            &SetFreeAlertRequest { user_id: mika.id.to_string(), enabled: true, ..Default::default() },
        )
        .await
        .unwrap();
    let countdown = CreateCountdownRequest {
        title: "旅行".into(),
        date: (Utc::now() + Duration::days(30)).format("%Y-%m-%d").to_string(),
        sharing: CountdownSharing::COUNTDOWN_SHARING_CHOSEN.into(),
        shared_with_ids: ids(&[yui, sota]),
        ..Default::default()
    };
    let countdown: CreateCountdownResponse =
        ctx.call(ANNIVERSARIES, "CreateCountdown", mika, &countdown).await.unwrap();
    let countdown = countdown.countdown.as_option().cloned().unwrap();
    // An invitation nobody answered, and a plan Yui said yes to.
    let unanswered = ctx.create_plan(mika, plan_at("返事待ち", in_days(3), &[yui, sota])).await;
    let answered = ctx.create_plan(mika, plan_at("参加予定", in_days(3), &[yui, sota])).await;
    ctx.rsvp(yui, &answered.id, Rsvp::RSVP_GOING).await;
    ctx.rsvp(sota, &answered.id, Rsvp::RSVP_GOING).await;
    // The same with polls.
    let quiet_poll = ctx
        .create_poll(
            mika,
            CreatePollRequest {
                title: "未回答".into(),
                slots: vec![day(5)],
                invitee_ids: ids(&[yui]),
                ..Default::default()
            },
        )
        .await;
    let answered_poll = ctx
        .create_poll(
            mika,
            CreatePollRequest {
                title: "回答済み".into(),
                slots: vec![day(5)],
                invitee_ids: ids(&[yui, sota]),
                ..Default::default()
            },
        )
        .await;
    ctx.answer_poll(yui, &answered_poll, &[PollAnswer::POLL_ANSWER_YES]).await;
    ctx.answer_poll(sota, &answered_poll, &[PollAnswer::POLL_ANSWER_YES]).await;
    // A friendship counter with Yui, a couple invitation to Sota.
    let _: StartFriendshipCounterResponse = ctx
        .call(
            ANNIVERSARIES,
            "StartFriendshipCounter",
            mika,
            &StartFriendshipCounterRequest { user_id: yui.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    let _: InviteCoupleResponse = ctx
        .call(
            ANNIVERSARIES,
            "InviteCouple",
            mika,
            &InviteCoupleRequest { user_id: sota.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();

    // Mika and Yui stop being friends.
    ctx.unfriend(mika, yui).await;
    let mut yuis_phone = ctx.app.listen_user(yui.id).await;
    let removed = event(
        "social.friend.removed",
        mika.id,
        serde_json::json!({ "user_id": mika.id, "friend_id": yui.id, "blocked": false }),
    );
    ctx.plans.handle_event(&removed).await.unwrap();
    ctx.plans.handle_event(&removed).await.unwrap();

    // Yui left Mika's audiences; Sota is still in them.
    let own: GetFreeStatusResponse =
        ctx.call(FREE, "GetFreeStatus", mika, &GetFreeStatusRequest::default()).await.unwrap();
    let own = own.status.as_option().unwrap();
    assert_eq!(
        (own.chosen.iter().map(|user| user.id.clone()).collect::<Vec<_>>(), own.answer_count),
        (ids(&[sota]), 0)
    );
    let alerts: ListFreeAlertsResponse =
        ctx.call(FREE, "ListFreeAlerts", yui, &ListFreeAlertsRequest::default()).await.unwrap();
    assert!(alerts.users.is_empty());
    let shared: GetCountdownResponse = ctx
        .call(
            ANNIVERSARIES,
            "GetCountdown",
            mika,
            &GetCountdownRequest { countdown_id: countdown.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(
        shared.countdown.as_option().unwrap().shared_with.iter().map(|user| user.id.clone()).collect::<Vec<_>>(),
        ids(&[sota])
    );

    // The unanswered invitations are withdrawn; what she answered stays hers.
    assert_eq!(
        ctx.refusal(PLANS, "GetPlan", yui, &GetPlanRequest { plan_id: unanswered.id.clone(), ..Default::default() })
            .await,
        "PLAN_NOT_FOUND"
    );
    assert_eq!(ctx.get_plan(sota, &unanswered.id).await.participants.len(), 2);
    assert_eq!(ctx.get_plan(yui, &answered.id).await.my_rsvp, Rsvp::RSVP_GOING);
    assert_eq!(
        ctx.refusal(POLLS, "GetPoll", yui, &GetPollRequest { poll_id: quiet_poll.id.clone(), ..Default::default() })
            .await,
        "POLL_NOT_FOUND"
    );
    assert!(ctx.get_poll(yui, &answered_poll.id).await.answered_by_me);
    let seen = pushes(&mut yuis_phone, QUIET).await;
    assert!(seen.iter().any(|push| matches!(push, Push::PlanChanged(change) if change.change == PlanChange::PLAN_CHANGE_REMOVED && change.plan_id == unanswered.id)));
    assert!(
        seen.iter()
            .any(|push| matches!(push, Push::PollChanged(change) if change.deleted && change.poll_id == quiet_poll.id))
    );

    // Their counter ended; the invitation to Sota is untouched.
    let counters: ListAnniversariesResponse =
        ctx.call(ANNIVERSARIES, "ListAnniversaries", mika, &ListAnniversariesRequest::default()).await.unwrap();
    assert_eq!(counters.anniversaries.len(), 1);
    assert_eq!(counters.anniversaries[0].partner.as_option().unwrap().id, sota.id.to_string());
    let yuis: ListAnniversariesResponse =
        ctx.call(ANNIVERSARIES, "ListAnniversaries", yui, &ListAnniversariesRequest::default()).await.unwrap();
    assert!(yuis.anniversaries.is_empty());

    // Sota blocks Mika: on top of all that, he leaves her polls even where he answered.
    ctx.block(sota, mika).await;
    let blocked =
        event("social.user.blocked", sota.id, serde_json::json!({ "user_id": sota.id, "blocked_id": mika.id }));
    ctx.plans.handle_event(&blocked).await.unwrap();
    assert_eq!(
        ctx.refusal(
            POLLS,
            "GetPoll",
            sota,
            &GetPollRequest { poll_id: answered_poll.id.clone(), ..Default::default() }
        )
        .await,
        "POLL_NOT_FOUND"
    );
    let poll = ctx.get_poll(mika, &answered_poll.id).await;
    assert_eq!((poll.participant_count, poll.slots[0].yes_count), (2, 1));
    assert_eq!(
        ctx.refusal(PLANS, "GetPlan", sota, &GetPlanRequest { plan_id: unanswered.id.clone(), ..Default::default() })
            .await,
        "PLAN_NOT_FOUND"
    );
    let counters: ListAnniversariesResponse =
        ctx.call(ANNIVERSARIES, "ListAnniversaries", mika, &ListAnniversariesRequest::default()).await.unwrap();
    assert!(counters.anniversaries.is_empty());
    let own: GetFreeStatusResponse =
        ctx.call(FREE, "GetFreeStatus", mika, &GetFreeStatusRequest::default()).await.unwrap();
    assert!(own.status.as_option().unwrap().chosen.is_empty());

    // The plan the two share keeps both, each without the other's details.
    let shared = ctx.get_plan(sota, &answered.id).await;
    assert_eq!(shared.participants.len(), 3);
    assert!(participant(&shared, mika).hidden && !participant(&shared, yui).hidden);
    assert!(participant(&ctx.get_plan(mika, &answered.id).await, sota).hidden);
}

#[tokio::test]
async fn a_plan_taken_down_by_a_moderator_is_gone_for_everyone() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui"]).await;
    let (mika, yui) = (&users[0], &users[1]);
    let plan = ctx.create_plan(mika, plan_at("よくない予定", in_days(3), &[yui])).await;
    let other = ctx.create_plan(mika, plan_at("ふつうの予定", in_days(4), &[yui])).await;
    let moderator = ctx.app.user("Moderator").await;
    let removed = |subject_type: SubjectType, id: &str| {
        event(
            "admin.content.removed",
            moderator.id,
            serde_json::json!({ "subject_type": subject_type as i32, "subject_id": id, "reason": "harassment", "moderator_id": moderator.id }),
        )
    };

    // Somebody else's kind of content with the same ID is none of this domain's business.
    ctx.plans.handle_event(&removed(SubjectType::SUBJECT_TYPE_NOTE, &plan.id)).await.unwrap();
    assert_eq!(ctx.get_plan(yui, &plan.id).await.id, plan.id);

    let mut yuis_phone = ctx.app.listen_user(yui.id).await;
    ctx.plans.handle_event(&removed(SubjectType::SUBJECT_TYPE_PLAN, &plan.id)).await.unwrap();
    ctx.plans.handle_event(&removed(SubjectType::SUBJECT_TYPE_PLAN, &plan.id)).await.unwrap();
    assert!(
        matches!(next_push(&mut yuis_phone).await, Some(Push::PlanChanged(change)) if change.change == PlanChange::PLAN_CHANGE_REMOVED && change.plan_id == plan.id)
    );
    for user in [mika, yui] {
        assert_eq!(
            ctx.refusal(PLANS, "GetPlan", user, &GetPlanRequest { plan_id: plan.id.clone(), ..Default::default() })
                .await,
            "PLAN_NOT_FOUND"
        );
        let listed: ListPlansResponse = ctx.call(PLANS, "ListPlans", user, &ListPlansRequest::default()).await.unwrap();
        assert_eq!(listed.plans.iter().map(|plan| plan.id.clone()).collect::<Vec<_>>(), [other.id.clone()]);
    }
    // Nothing more comes of it: no reminders, and nothing to comment on.
    assert_eq!(count(&ctx, "SELECT count(*) FROM plans WHERE id = $1 AND day_before_due_at IS NULL AND before_due_at IS NULL AND attended_due_at IS NULL", id_of(&plan.id)).await, 1);
    assert!(ctx.app.core.subjects.resolve(SubjectType::SUBJECT_TYPE_PLAN, &plan.id, Some(mika.id)).await.is_err());
}

#[tokio::test]
async fn the_export_holds_what_the_domain_knows_about_a_user() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui"]).await;
    let (mika, yui) = (&users[0], &users[1]);
    let status = SetFreeStatusRequest {
        window: FreeWindow::FREE_WINDOW_NOW.into(),
        audience: FreeAudience::FREE_AUDIENCE_FRIENDS.into(),
        note: "ひま".into(),
        ..Default::default()
    };
    let _: SetFreeStatusResponse = ctx.call(FREE, "SetFreeStatus", mika, &status).await.unwrap();
    let plan = ctx.create_plan(mika, plan_at("カラオケ", in_days(3), &[yui])).await;
    let expense = ExpenseInput {
        title: "部屋代".into(),
        amount: 3000,
        method: SplitMethod::SPLIT_METHOD_EQUAL.into(),
        shares: [mika, yui]
            .iter()
            .map(|user| ExpenseShare { user_id: user.id.to_string(), ..Default::default() })
            .collect(),
        ..Default::default()
    };
    let _: AddExpenseResponse = ctx
        .call(
            PLANS,
            "AddExpense",
            mika,
            &AddExpenseRequest { plan_id: plan.id.clone(), expense: expense.into(), ..Default::default() },
        )
        .await
        .unwrap();
    let day = PollSlotInput {
        start: Some(poll_slot_input::Start::Date((Utc::now() + Duration::days(5)).format("%Y-%m-%d").to_string())),
        ..Default::default()
    };
    let poll = ctx
        .create_poll(
            mika,
            CreatePollRequest {
                title: "いつ?".into(),
                slots: vec![day],
                invitee_ids: ids(&[yui]),
                ..Default::default()
            },
        )
        .await;
    ctx.answer_poll(mika, &poll, &[PollAnswer::POLL_ANSWER_YES]).await;
    let countdown = CreateCountdownRequest {
        title: "テスト".into(),
        date: (Utc::now() + Duration::days(9)).format("%Y-%m-%d").to_string(),
        ..Default::default()
    };
    let _: CreateCountdownResponse = ctx.call(ANNIVERSARIES, "CreateCountdown", mika, &countdown).await.unwrap();
    let _: StartFriendshipCounterResponse = ctx
        .call(
            ANNIVERSARIES,
            "StartFriendshipCounter",
            mika,
            &StartFriendshipCounterRequest { user_id: yui.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();

    let exporter = ctx
        .app
        .core
        .exporters
        .all()
        .into_iter()
        .find(|exporter| exporter.name() == "plans")
        .expect("the plans exporter is registered");
    let export = exporter.export(mika.id).await.unwrap();
    assert_eq!(export["free_status"]["note"], serde_json::json!("ひま"));
    assert_eq!(export["plans"].as_array().unwrap().len(), 1);
    assert_eq!(
        (export["plans"][0]["title"].clone(), export["plans"][0]["host_id"].clone()),
        (serde_json::json!("カラオケ"), serde_json::json!(mika.id))
    );
    assert_eq!(
        (
            export["expenses"][0]["amount"].clone(),
            export["expenses"][0]["my_share"].clone(),
            export["expenses"][0]["paid_by_me"].clone()
        ),
        (serde_json::json!(3000), serde_json::json!(1500), serde_json::json!(true))
    );
    assert_eq!(export["polls"][0]["my_answers"].as_array().unwrap().len(), 1);
    assert_eq!(export["countdowns"][0]["title"], serde_json::json!("テスト"));
    assert_eq!(export["anniversaries"][0]["with_user_id"], serde_json::json!(yui.id));
    assert_eq!(export["settings"]["share_birthday"], serde_json::json!(false));

    // Yui's export is hers: her share of the bill, not Mika's status.
    let yuis = exporter.export(yui.id).await.unwrap();
    assert!(yuis["free_status"].is_null());
    assert_eq!(
        (yuis["expenses"][0]["my_share"].clone(), yuis["expenses"][0]["paid_by_me"].clone()),
        (serde_json::json!(1500), serde_json::json!(false))
    );
    assert_eq!(yuis["plans"][0]["my_role"], serde_json::json!(PlanRole::PLAN_ROLE_GUEST as i32));
    // Someone with nothing here gets empty sections.
    let nobody = ctx.app.user("Nobody").await;
    let empty = exporter.export(nobody.id).await.unwrap();
    assert!(empty["plans"].as_array().unwrap().is_empty() && empty["countdowns"].as_array().unwrap().is_empty());
}
