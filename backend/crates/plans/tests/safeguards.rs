//! What keeps one account from using the domain against other people:
//! limits on everything that notifies someone, answers that give nothing
//! away, a plan that stays reportable for whoever was shown it, things that
//! happen exactly once, and events that arrive late or twice.
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

fn day(days: i64) -> PollSlotInput {
    let date = in_days(days).with_timezone(&chrono_tz::Asia::Tokyo).format("%Y-%m-%d").to_string();
    PollSlotInput { start: Some(poll_slot_input::Start::Date(date)), ..Default::default() }
}

fn id_of(text: &str) -> Uuid {
    text.parse().expect("an id")
}

fn event(topic: &str, actor: Uuid, data: serde_json::Value) -> Event {
    Event { id: Uuid::now_v7(), topic: topic.to_owned(), actor_id: Some(actor), occurred_at: Utc::now(), data }
}

async fn count(ctx: &Ctx, query: &'static str, id: Uuid) -> i64 {
    sqlx::query_scalar(query).bind(id).fetch_one(&ctx.app.core.db).await.expect("count")
}

fn plan_ref(plan: &Plan) -> GetPlanRequest {
    GetPlanRequest { plan_id: plan.id.clone(), ..Default::default() }
}

fn invite(plan: &Plan, users: &[&TestUser]) -> InviteToPlanRequest {
    InviteToPlanRequest { plan_id: plan.id.clone(), user_ids: ids(users), ..Default::default() }
}

fn remove(plan: &Plan, user: &TestUser) -> RemovePlanParticipantRequest {
    RemovePlanParticipantRequest { plan_id: plan.id.clone(), user_id: user.id.to_string(), ..Default::default() }
}

fn answer(plan: &Plan, rsvp: Rsvp) -> RespondToPlanRequest {
    RespondToPlanRequest { plan_id: plan.id.clone(), rsvp: rsvp.into(), ..Default::default() }
}

#[tokio::test]
async fn nobody_is_notified_over_and_over() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);

    // An answer reaches the host every time it changes: a dozen a minute is plenty.
    let plan = ctx.create_plan(mika, plan_at("お祭り", in_days(3), &[yui])).await;
    for round in 0..12 {
        ctx.rsvp(yui, &plan.id, if round % 2 == 0 { Rsvp::RSVP_GOING } else { Rsvp::RSVP_MAYBE }).await;
    }
    assert_eq!(ctx.refusal(PLANS, "RespondToPlan", yui, &answer(&plan, Rsvp::RSVP_GOING)).await, "RATE_LIMITED");
    assert_eq!(ctx.notifications_for(mika.id, "plans.rsvp").await.len(), 12);

    // Inviting, removing and inviting again: the same friend hears from the
    // same person ten times a day, plans and polls together.
    let _: RemovePlanParticipantResponse =
        ctx.call(PLANS, "RemovePlanParticipant", mika, &remove(&plan, yui)).await.unwrap();
    for _ in 0..8 {
        let _: InviteToPlanResponse = ctx.call(PLANS, "InviteToPlan", mika, &invite(&plan, &[yui])).await.unwrap();
        let _: RemovePlanParticipantResponse =
            ctx.call(PLANS, "RemovePlanParticipant", mika, &remove(&plan, yui)).await.unwrap();
    }
    let poll = ctx
        .create_poll(
            mika,
            CreatePollRequest {
                title: "いつ?".into(),
                slots: vec![day(5)],
                invitee_ids: ids(&[yui]),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(ctx.refusal(PLANS, "InviteToPlan", mika, &invite(&plan, &[yui])).await, "RATE_LIMITED");
    let new_poll = CreatePollRequest {
        title: "また?".into(),
        slots: vec![day(6)],
        invitee_ids: ids(&[yui]),
        ..Default::default()
    };
    assert_eq!(ctx.refusal(POLLS, "CreatePoll", mika, &new_poll).await, "RATE_LIMITED");
    assert_eq!(ctx.notifications_for(yui.id, "plans.invited").await.len(), 9);
    assert_eq!(ctx.notifications_for(yui.id, "plans.poll_invited").await.len(), 1);
    // The limit is per friend: Sota can still be invited.
    let invited: InviteToPlanResponse = ctx.call(PLANS, "InviteToPlan", mika, &invite(&plan, &[sota])).await.unwrap();
    assert_eq!(invited.invited_count, 1);

    // Someone who takes their answer to a poll back and gives it again is
    // announced to the creator once.
    for answers in
        [&[PollAnswer::POLL_ANSWER_YES][..], &[], &[PollAnswer::POLL_ANSWER_NO], &[], &[PollAnswer::POLL_ANSWER_YES]]
    {
        ctx.answer_poll(yui, &poll, answers).await;
    }
    assert_eq!(ctx.notifications_for(mika.id, "plans.poll_answer").await.len(), 1);

    // A couple invitation sent, withdrawn and sent again stops at three a day.
    let ask = InviteCoupleRequest { user_id: sota.id.to_string(), ..Default::default() };
    for _ in 0..3 {
        let sent: InviteCoupleResponse = ctx.call(ANNIVERSARIES, "InviteCouple", mika, &ask).await.unwrap();
        let withdraw = EndAnniversaryRequest {
            anniversary_id: sent.anniversary.as_option().unwrap().id.clone(),
            ..Default::default()
        };
        let _: EndAnniversaryResponse = ctx.call(ANNIVERSARIES, "EndAnniversary", mika, &withdraw).await.unwrap();
    }
    assert_eq!(ctx.refusal(ANNIVERSARIES, "InviteCouple", mika, &ask).await, "RATE_LIMITED");
    assert_eq!(ctx.notifications_for(sota.id, "plans.couple_invite").await.len(), 3);
    // The same for a friendship counter started and ended in turns.
    let count_days = StartFriendshipCounterRequest { user_id: sota.id.to_string(), ..Default::default() };
    for _ in 0..5 {
        let started: StartFriendshipCounterResponse =
            ctx.call(ANNIVERSARIES, "StartFriendshipCounter", mika, &count_days).await.unwrap();
        let end = EndAnniversaryRequest {
            anniversary_id: started.anniversary.as_option().unwrap().id.clone(),
            ..Default::default()
        };
        let _: EndAnniversaryResponse = ctx.call(ANNIVERSARIES, "EndAnniversary", mika, &end).await.unwrap();
    }
    assert_eq!(ctx.refusal(ANNIVERSARIES, "StartFriendshipCounter", mika, &count_days).await, "RATE_LIMITED");
}

async fn counters_of(ctx: &Ctx, user: &TestUser) -> Vec<Anniversary> {
    let listed: ListAnniversariesResponse =
        ctx.call(ANNIVERSARIES, "ListAnniversaries", user, &ListAnniversariesRequest::default()).await.unwrap();
    listed.anniversaries
}

#[tokio::test]
async fn an_answer_tells_nothing_about_someone_elses_counters() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let ask = |friend: &TestUser| InviteCoupleRequest { user_id: friend.id.to_string(), ..Default::default() };
    let respond = |sent: &InviteCoupleResponse, accept: bool| RespondCoupleInviteRequest {
        anniversary_id: sent.anniversary.as_option().unwrap().id.clone(),
        accept,
        ..Default::default()
    };

    // Mika asked Yui. Before Yui answers, Sota asks Mika, and Mika says yes.
    let to_yui: InviteCoupleResponse = ctx.call(ANNIVERSARIES, "InviteCouple", mika, &ask(yui)).await.unwrap();
    let to_mika: InviteCoupleResponse = ctx.call(ANNIVERSARIES, "InviteCouple", sota, &ask(mika)).await.unwrap();
    let mut yuis_phone = ctx.app.listen_user(yui.id).await;
    let accepted: RespondCoupleInviteResponse =
        ctx.call(ANNIVERSARIES, "RespondCoupleInvite", mika, &respond(&to_mika, true)).await.unwrap();
    assert_eq!(accepted.anniversary.as_option().unwrap().status, AnniversaryStatus::ANNIVERSARY_STATUS_ACTIVE);

    // For Yui the invitation is simply gone, exactly as if Mika had taken it
    // back: nothing she can do with it says why.
    let withdrawn = to_yui.anniversary.as_option().unwrap().id.clone();
    assert!(
        matches!(next_push(&mut yuis_phone).await, Some(Push::AnniversaryChanged(change)) if change.ended_anniversary_id == withdrawn)
    );
    assert!(counters_of(&ctx, yui).await.is_empty());
    for accept in [true, false] {
        assert_eq!(
            ctx.refusal(ANNIVERSARIES, "RespondCoupleInvite", yui, &respond(&to_yui, accept)).await,
            "ANNIVERSARY_NOT_FOUND"
        );
    }
    // She can ask Mika herself as before: whether Mika has a couple counter
    // by now is not something an error tells her.
    let asked: InviteCoupleResponse = ctx.call(ANNIVERSARIES, "InviteCouple", yui, &ask(mika)).await.unwrap();
    assert_eq!(asked.anniversary.as_option().unwrap().status, AnniversaryStatus::ANNIVERSARY_STATUS_PENDING);
    // Mika's own list holds her couple counter and that invitation, nothing of the withdrawn one.
    assert_eq!(counters_of(&ctx, mika).await.len(), 2);
}

#[tokio::test]
async fn a_plan_stays_reportable_for_whoever_was_shown_it() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota", "Hana"]).await;
    let (mika, yui, sota, hana) = (&users[0], &users[1], &users[2], &users[3]);
    let stranger = ctx.app.user("Stranger").await;
    let plan = ctx
        .create_plan(
            mika,
            CreatePlanRequest { emoji: "🔥".into(), ..plan_at("いやな誘い", in_days(3), &[yui, sota, hana]) },
        )
        .await;
    ctx.rsvp(sota, &plan.id, Rsvp::RSVP_GOING).await;
    ctx.rsvp(hana, &plan.id, Rsvp::RSVP_MAYBE).await;
    let subjects = &ctx.app.core.subjects;
    let plan_type = SubjectType::SUBJECT_TYPE_PLAN;

    // While they are in the plan it is theirs to comment on and to report.
    let seen = subjects.resolve(plan_type, &plan.id, Some(yui.id)).await.unwrap();
    assert!(seen.interactions_enabled);
    assert!(subjects.resolve(plan_type, &plan.id, Some(stranger.id)).await.is_err());

    // Yui blocks the host: the invitation she never answered is withdrawn...
    ctx.block(yui, mika).await;
    let blocked = event("social.user.blocked", yui.id, serde_json::json!({ "user_id": yui.id, "blocked_id": mika.id }));
    ctx.plans.handle_event(&blocked).await.unwrap();
    assert_eq!(ctx.refusal(PLANS, "GetPlan", yui, &plan_ref(&plan)).await, "PLAN_NOT_FOUND");
    // ...the host removes Sota, and Hana leaves by herself.
    let _: RemovePlanParticipantResponse =
        ctx.call(PLANS, "RemovePlanParticipant", mika, &remove(&plan, sota)).await.unwrap();
    let _: LeavePlanResponse = ctx
        .call(PLANS, "LeavePlan", hana, &LeavePlanRequest { plan_id: plan.id.clone(), ..Default::default() })
        .await
        .unwrap();
    // The host then rewrites what they were shown.
    let rename =
        UpdatePlanRequest {
            plan_id: plan.id.clone(), title: Some("なんでもないよ".into()), ..Default::default()
        };
    let _: UpdatePlanResponse = ctx.call(PLANS, "UpdatePlan", mika, &rename).await.unwrap();

    // None of that takes away the handle for a report: each of them still
    // resolves the plan as they saw it, without a way to write into it.
    for gone in [yui, sota, hana] {
        assert_eq!(ctx.refusal(PLANS, "GetPlan", gone, &plan_ref(&plan)).await, "PLAN_NOT_FOUND", "{}", gone.handle);
        let seen = subjects.resolve(plan_type, &plan.id, Some(gone.id)).await.unwrap();
        assert_eq!(
            (seen.id, seen.owner_id, seen.title.as_str(), seen.interactions_enabled),
            (id_of(&plan.id), mika.id, "🔥 いやな誘い", false),
            "{}",
            gone.handle
        );
    }
    // Someone who never was in it still finds nothing.
    assert!(subjects.resolve(plan_type, &plan.id, Some(stranger.id)).await.is_err());
    assert!(subjects.resolve(plan_type, &plan.id, None).await.is_err());

    // Invited back, Sota is a participant like before.
    let _: InviteToPlanResponse = ctx.call(PLANS, "InviteToPlan", mika, &invite(&plan, &[sota])).await.unwrap();
    let seen = subjects.resolve(plan_type, &plan.id, Some(sota.id)).await.unwrap();
    assert_eq!((seen.title.as_str(), seen.interactions_enabled), ("🔥 なんでもないよ", true));

    // A plan a moderator took down is gone for everyone, past participants included.
    let taken_down = event(
        "admin.content.removed",
        stranger.id,
        serde_json::json!({ "subject_type": plan_type as i32, "subject_id": plan.id, "reason": "harassment", "moderator_id": stranger.id }),
    );
    ctx.plans.handle_event(&taken_down).await.unwrap();
    for gone in [yui, hana, sota] {
        assert!(subjects.resolve(plan_type, &plan.id, Some(gone.id)).await.is_err());
    }
}

#[tokio::test]
async fn attendance_is_counted_once_even_if_the_plan_moves() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui"]).await;
    let (mika, yui) = (&users[0], &users[1]);
    let plan = ctx.create_plan(mika, plan_at("カフェ", in_days(1), &[yui])).await;
    let plan_id = id_of(&plan.id);
    ctx.rsvp(yui, &plan.id, Rsvp::RSVP_GOING).await;
    const OVER: &str = "UPDATE plans SET attended_due_at = now() - interval '1 minute' WHERE id = $1";

    assert_eq!(ctx.make_due(OVER, &[plan_id]).await.plans_attended, 1);
    assert_eq!(ctx.app.events_by("plans.plan.attended", yui.id).await.len(), 1);

    // The host moves the plan to next week. Other domains reward attendance
    // (quests, streaks); doing it again must not pay twice.
    let moved = UpdatePlanRequest {
        plan_id: plan.id.clone(),
        time: PlanTime { start: Some(plan_time::Start::StartAt(Box::new(in_days(7).into()))), ..Default::default() }
            .into(),
        ..Default::default()
    };
    let _: UpdatePlanResponse = ctx.call(PLANS, "UpdatePlan", mika, &moved).await.unwrap();
    assert_eq!(count(&ctx, "SELECT count(*) FROM plans WHERE id = $1 AND attended_due_at IS NULL", plan_id).await, 1);
    // The reminders for the new day are scheduled as usual.
    assert_eq!(count(&ctx, "SELECT count(*) FROM plans WHERE id = $1 AND before_due_at > now()", plan_id).await, 1);
    // Even a pass that finds the plan due again leaves it alone.
    assert_eq!(ctx.make_due(OVER, &[plan_id]).await.plans_attended, 0);
    for attendee in [mika, yui] {
        assert_eq!(ctx.app.events_by("plans.plan.attended", attendee.id).await.len(), 1);
        assert_eq!(ctx.activities_of(attendee.id, "plans.attended").await.len(), 1);
    }
}

#[tokio::test]
async fn events_that_come_late_change_nothing_that_was_set_up_since() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let friendship_over = |friend: &TestUser| {
        event(
            "social.friend.removed",
            mika.id,
            serde_json::json!({ "user_id": mika.id, "friend_id": friend.id, "blocked": false }),
        )
    };

    // Mika and Yui fell out and made up again; the event about the falling
    // out only arrives now, after what they have set up since.
    ctx.unfriend(mika, yui).await;
    ctx.befriend(mika, yui).await;
    let plan = ctx.create_plan(mika, plan_at("仲直りごはん", in_days(3), &[yui])).await;
    let poll = ctx
        .create_poll(
            mika,
            CreatePollRequest {
                title: "次は?".into(),
                slots: vec![day(5)],
                invitee_ids: ids(&[yui, sota]),
                ..Default::default()
            },
        )
        .await;
    let _: StartFriendshipCounterResponse = ctx
        .call(
            ANNIVERSARIES,
            "StartFriendshipCounter",
            mika,
            &StartFriendshipCounterRequest { user_id: yui.id.to_string(), ..Default::default() },
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

    let late = friendship_over(yui);
    ctx.plans.handle_event(&late).await.unwrap();
    ctx.plans.handle_event(&late).await.unwrap();
    assert_eq!(ctx.get_plan(yui, &plan.id).await.my_rsvp, Rsvp::RSVP_INVITED);
    assert_eq!(ctx.get_poll(yui, &poll.id).await.participant_count, 3);
    let counters: ListAnniversariesResponse =
        ctx.call(ANNIVERSARIES, "ListAnniversaries", yui, &ListAnniversariesRequest::default()).await.unwrap();
    assert_eq!(counters.anniversaries.len(), 1);
    let alerts: ListFreeAlertsResponse =
        ctx.call(FREE, "ListFreeAlerts", yui, &ListFreeAlertsRequest::default()).await.unwrap();
    assert_eq!(alerts.users.len(), 1);

    // Sota answered the poll and then stopped being Mika's friend: he
    // leaves it, however the news arrives and however often.
    ctx.answer_poll(sota, &poll, &[PollAnswer::POLL_ANSWER_YES]).await;
    ctx.unfriend(mika, sota).await;
    let block = event("social.user.blocked", sota.id, serde_json::json!({ "user_id": sota.id, "blocked_id": mika.id }));
    ctx.plans.handle_event(&block).await.unwrap();
    ctx.plans.handle_event(&friendship_over(sota)).await.unwrap();
    assert_eq!(
        ctx.refusal(POLLS, "GetPoll", sota, &GetPollRequest { poll_id: poll.id.clone(), ..Default::default() }).await,
        "POLL_NOT_FOUND"
    );
    assert_eq!(ctx.get_poll(mika, &poll.id).await.participant_count, 2);

    // An event about something that no longer exists is not an error.
    let _: DeletePollResponse = ctx
        .call(POLLS, "DeletePoll", mika, &DeletePollRequest { poll_id: poll.id.clone(), ..Default::default() })
        .await
        .unwrap();
    ctx.plans.handle_event(&friendship_over(sota)).await.unwrap();
    let nothing = event(
        "admin.content.removed",
        mika.id,
        serde_json::json!({ "subject_type": SubjectType::SUBJECT_TYPE_PLAN as i32, "subject_id": Uuid::now_v7(), "reason": "spam", "moderator_id": mika.id }),
    );
    ctx.plans.handle_event(&nothing).await.unwrap();
    let nobody = event("identity.user.deleted", mika.id, serde_json::json!({ "user_id": Uuid::now_v7() }));
    ctx.plans.handle_event(&nobody).await.unwrap();
    assert_eq!(ctx.get_plan(mika, &plan.id).await.participants.len(), 2);
}

#[tokio::test]
async fn nobody_outside_the_split_can_write_into_it() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    // Sota is invited and never answers.
    let plan = ctx.create_plan(mika, plan_at("焼肉", in_days(3), &[yui, sota])).await;
    ctx.rsvp(yui, &plan.id, Rsvp::RSVP_GOING).await;
    let settle = |from: &TestUser, to: &TestUser, amount: i64| SettleTransferRequest {
        plan_id: plan.id.clone(),
        from_user_id: from.id.to_string(),
        to_user_id: to.id.to_string(),
        amount,
        ..Default::default()
    };
    let expense = |payer: &TestUser, among: &[&TestUser]| AddExpenseRequest {
        plan_id: plan.id.clone(),
        expense: ExpenseInput {
            title: "お肉".into(),
            amount: 3000,
            paid_by_user_id: payer.id.to_string(),
            method: SplitMethod::SPLIT_METHOD_EQUAL.into(),
            shares: among
                .iter()
                .map(|user| ExpenseShare { user_id: user.id.to_string(), ..Default::default() })
                .collect(),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };
    let split = |viewer: &TestUser| {
        let request = GetSplitRequest { plan_id: plan.id.clone(), ..Default::default() };
        let viewer = viewer.clone();
        let ctx = &ctx;
        async move { ctx.call::<_, GetSplitResponse>(PLANS, "GetSplit", &viewer, &request).await.unwrap() }
    };

    // "I gave the host a million yen": not his to say, and there is no debt.
    assert_eq!(ctx.refusal(PLANS, "SettleTransfer", sota, &settle(sota, mika, 1_000_000)).await, "NOT_ALLOWED");
    assert_eq!(
        ctx.refusal(PLANS, "SettleTransfer", mika, &settle(sota, mika, 1_000_000)).await,
        "AMOUNT_EXCEEDS_BALANCE"
    );
    assert_eq!(ctx.refusal(PLANS, "SettleTransfer", sota, &settle(mika, sota, 1)).await, "AMOUNT_EXCEEDS_BALANCE");
    assert!(split(mika).await.splits.is_empty());

    // Nobody is charged for a plan they have not said yes to.
    assert_eq!(ctx.refusal(PLANS, "AddExpense", mika, &expense(mika, &[mika, yui, sota])).await, "SHARE_NOT_GOING");
    assert_eq!(ctx.refusal(PLANS, "AddExpense", mika, &expense(sota, &[mika, yui])).await, "PAYER_NOT_GOING");
    assert_eq!(ctx.refusal(PLANS, "AddExpense", sota, &expense(sota, &[mika, yui])).await, "NOT_GOING");
    let _: AddExpenseResponse = ctx.call(PLANS, "AddExpense", mika, &expense(mika, &[mika, yui])).await.unwrap();
    // So he is free to go.
    let _: LeavePlanResponse = ctx
        .call(PLANS, "LeavePlan", sota, &LeavePlanRequest { plan_id: plan.id.clone(), ..Default::default() })
        .await
        .unwrap();

    // Yui owes 1500 and blocks Mika. That does not let her mark it paid;
    // Mika marking it received lowers her own claim, and the record shows it.
    ctx.block(yui, mika).await;
    assert_eq!(ctx.refusal(PLANS, "SettleTransfer", yui, &settle(yui, mika, 1500)).await, "NOT_ALLOWED");
    assert_eq!(split(mika).await.splits[0].transfers.len(), 1);
    let done: SettleTransferResponse = ctx.call(PLANS, "SettleTransfer", mika, &settle(yui, mika, 1500)).await.unwrap();
    let seen = split(yui).await;
    assert!(seen.splits[0].transfers.is_empty());
    assert_eq!(seen.settlements[0].recorded_by_user_id, mika.id.to_string());
    // Yui cannot make it disappear, and when Mika takes it back the row stays.
    let take_back = RemoveSettlementRequest {
        settlement_id: done.settlement.as_option().unwrap().id.clone(),
        ..Default::default()
    };
    assert_eq!(ctx.refusal(PLANS, "RemoveSettlement", yui, &take_back).await, "NOT_ALLOWED");
    let _: RemoveSettlementResponse = ctx.call(PLANS, "RemoveSettlement", mika, &take_back).await.unwrap();
    let seen = split(yui).await;
    assert_eq!((seen.settlements.len(), seen.removed_settlements.len()), (0, 1));
    assert_eq!(seen.removed_settlements[0].removed_by_user_id, mika.id.to_string());
    assert_eq!(count(&ctx, "SELECT count(*) FROM plan_settlements WHERE plan_id = $1", id_of(&plan.id)).await, 1);
}

#[tokio::test]
async fn changing_a_plan_does_not_send_what_was_sent_again() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let plan = ctx.create_plan(mika, plan_at("花火", in_days(3), &[yui, sota])).await;
    let plan_id = id_of(&plan.id);
    ctx.rsvp(yui, &plan.id, Rsvp::RSVP_GOING).await;
    ctx.rsvp(sota, &plan.id, Rsvp::RSVP_GOING).await;
    const DAY_BEFORE: &str = "UPDATE plans SET day_before_due_at = now() - interval '1 minute' WHERE id = $1";
    const SOON: &str = "UPDATE plans SET before_due_at = now() - interval '1 second' WHERE id = $1";
    let lead = |minutes: i32| UpdatePlanRequest {
        plan_id: plan.id.clone(),
        remind_minutes_before: Some(minutes),
        ..Default::default()
    };
    let at = |time: DateTime<Utc>| UpdatePlanRequest {
        plan_id: plan.id.clone(),
        time: PlanTime { start: Some(plan_time::Start::StartAt(Box::new(time.into()))), ..Default::default() }.into(),
        ..Default::default()
    };

    // Both reminders go out once.
    ctx.make_due(DAY_BEFORE, &[plan_id]).await;
    ctx.make_due(SOON, &[plan_id]).await;
    assert_eq!(ctx.notifications_for(yui.id, "plans.reminder").await.len(), 2);
    // They speak for the host.
    assert_eq!(ctx.notifications_for(yui.id, "plans.reminder").await[0].actor_id, Some(mika.id));
    assert_eq!(ctx.notifications_for(mika.id, "plans.reminder").await[0].actor_id, None);

    // The host changes the lead time again and again: whatever comes due
    // for the same start time is not sent a second time.
    for minutes in [30, 45, 15, 120] {
        let _: UpdatePlanResponse = ctx.call(PLANS, "UpdatePlan", mika, &lead(minutes)).await.unwrap();
        ctx.make_due(DAY_BEFORE, &[plan_id]).await;
        ctx.make_due(SOON, &[plan_id]).await;
    }
    assert_eq!(ctx.notifications_for(yui.id, "plans.reminder").await.len(), 2);
    assert_eq!(
        count(
            &ctx,
            "SELECT count(*) FROM plans WHERE id = $1 AND day_before_due_at IS NULL AND before_due_at IS NULL",
            plan_id
        )
        .await,
        1
    );

    // A new start time is news, and gets its reminders. Sota has blocked the
    // host by then: what her plan sends no longer reaches him.
    ctx.block(sota, mika).await;
    let _: UpdatePlanResponse = ctx.call(PLANS, "UpdatePlan", mika, &at(in_days(5))).await.unwrap();
    ctx.make_due(DAY_BEFORE, &[plan_id]).await;
    assert_eq!(ctx.notifications_for(yui.id, "plans.reminder").await.len(), 3);
    assert_eq!(ctx.notifications_for(sota.id, "plans.reminder").await.len(), 2);

    // A plan is not moved around all day: six changes of time or place an
    // hour, each of which tells everyone. Other edits are not held up.
    for day in 6..11 {
        let _: UpdatePlanResponse = ctx.call(PLANS, "UpdatePlan", mika, &at(in_days(day))).await.unwrap();
    }
    assert_eq!(ctx.refusal(PLANS, "UpdatePlan", mika, &at(in_days(20))).await, "RATE_LIMITED");
    assert_eq!(ctx.notifications_for(yui.id, "plans.updated").await.len(), 6);
    let renamed =
        UpdatePlanRequest { plan_id: plan.id.clone(), title: Some("花火大会".into()), ..Default::default() };
    let _: UpdatePlanResponse = ctx.call(PLANS, "UpdatePlan", mika, &renamed).await.unwrap();
}

#[tokio::test]
async fn a_poll_can_be_left_and_people_can_be_taken_out() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota", "Hana"]).await;
    let (mika, yui, sota, hana) = (&users[0], &users[1], &users[2], &users[3]);
    let stranger = ctx.app.user("Stranger").await;
    let poll = ctx
        .create_poll(
            mika,
            CreatePollRequest {
                title: "合宿".into(),
                slots: vec![day(5), day(6)],
                invitee_ids: ids(&[yui]),
                open_to_friends: true,
                ..Default::default()
            },
        )
        .await;
    let by_id = GetPollRequest { poll_id: poll.id.clone(), ..Default::default() };
    let by_link = GetPollRequest { poll_id: poll.short_id.clone(), ..Default::default() };
    let leave = LeavePollRequest { poll_id: poll.id.clone(), ..Default::default() };
    let remove = |user: &TestUser| RemovePollParticipantRequest {
        poll_id: poll.id.clone(),
        user_id: user.id.to_string(),
        ..Default::default()
    };
    let invite = |user: &TestUser| InviteToPollRequest {
        poll_id: poll.id.clone(),
        user_ids: ids(&[user]),
        ..Default::default()
    };
    ctx.answer_poll(yui, &poll, &[PollAnswer::POLL_ANSWER_YES]).await;
    ctx.answer_poll(sota, &poll, &[PollAnswer::POLL_ANSWER_YES]).await;
    ctx.answer_poll(hana, &poll, &[PollAnswer::POLL_ANSWER_YES]).await;
    assert_eq!(ctx.get_poll(mika, &poll.id).await.slots[0].yes_count, 3);

    // Yui leaves: her answers go with her, and she is not invited back.
    assert_eq!(ctx.refusal(POLLS, "LeavePoll", mika, &leave).await, "CREATOR_CANNOT_LEAVE");
    assert_eq!(ctx.refusal(POLLS, "LeavePoll", &stranger, &leave).await, "POLL_NOT_FOUND");
    let mut yuis_phone = ctx.app.listen_user(yui.id).await;
    let _: LeavePollResponse = ctx.call(POLLS, "LeavePoll", yui, &leave).await.unwrap();
    assert!(matches!(next_push(&mut yuis_phone).await, Some(Push::PollChanged(change)) if change.deleted));
    let seen = ctx.get_poll(mika, &poll.id).await;
    assert_eq!((seen.participant_count, seen.slots[0].yes_count), (3, 2));
    assert_eq!(ctx.refusal(POLLS, "LeavePoll", yui, &leave).await, "POLL_NOT_FOUND");
    let again: InviteToPollResponse = ctx.call(POLLS, "InviteToPoll", mika, &invite(yui)).await.unwrap();
    assert_eq!(again.invited_count, 0);
    assert_eq!(ctx.notifications_for(yui.id, "plans.poll_invited").await.len(), 1);
    // The poll is open to friends with the link, so she can come back — by herself.
    ctx.answer_poll(yui, &poll, &[PollAnswer::POLL_ANSWER_MAYBE]).await;
    assert_eq!(ctx.get_poll(mika, &poll.id).await.participant_count, 4);

    // The creator takes Sota out: the link no longer lets him in.
    assert_eq!(ctx.refusal(POLLS, "RemovePollParticipant", hana, &remove(sota)).await, "NOT_THE_CREATOR");
    assert_eq!(ctx.refusal(POLLS, "RemovePollParticipant", mika, &remove(mika)).await, "CANNOT_REMOVE_CREATOR");
    assert_eq!(ctx.refusal(POLLS, "RemovePollParticipant", mika, &remove(&stranger)).await, "PARTICIPANT_NOT_FOUND");
    let removed: RemovePollParticipantResponse =
        ctx.call(POLLS, "RemovePollParticipant", mika, &remove(sota)).await.unwrap();
    let seen = removed.poll.as_option().unwrap();
    assert_eq!((seen.participant_count, seen.slots[0].yes_count), (3, 1));
    for request in [&by_id, &by_link] {
        assert_eq!(ctx.refusal(POLLS, "GetPoll", sota, request).await, "POLL_NOT_FOUND");
    }
    let yes = AnswerPollRequest {
        poll_id: poll.short_id.clone(),
        answers: vec![SlotAnswer {
            slot_id: poll.slots[0].id.clone(),
            answer: PollAnswer::POLL_ANSWER_YES.into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert_eq!(ctx.refusal(POLLS, "AnswerPoll", sota, &yes).await, "POLL_NOT_FOUND");
    // Until he is invited again.
    let back: InviteToPollResponse = ctx.call(POLLS, "InviteToPoll", mika, &invite(sota)).await.unwrap();
    assert_eq!(back.invited_count, 1);
    assert_eq!(ctx.get_poll(sota, &poll.id).await.participant_count, 4);

    // Answers stop at the deadline, not at the worker's next pass.
    ctx.sql("UPDATE date_polls SET deadline = now() - interval '1 second' WHERE id = $1", id_of(&poll.id)).await;
    assert_eq!(ctx.refusal(POLLS, "AnswerPoll", hana, &yes).await, "POLL_NOT_OPEN");
    assert_eq!(ctx.refusal(POLLS, "InviteToPoll", mika, &invite(hana)).await, "POLL_NOT_OPEN");
    ctx.work().await;
    assert_eq!(ctx.get_poll(mika, &poll.id).await.status, PollStatus::POLL_STATUS_CLOSED);

    // A closed poll is not edited, but its link can be switched off.
    let retitle = UpdatePollRequest { poll_id: poll.id.clone(), title: Some("合宿!".into()), ..Default::default() };
    assert_eq!(ctx.refusal(POLLS, "UpdatePoll", mika, &retitle).await, "POLL_NOT_OPEN");
    let unlink = UpdatePollRequest { poll_id: poll.id.clone(), open_to_friends: Some(false), ..Default::default() };
    let unlinked: UpdatePollResponse = ctx.call(POLLS, "UpdatePoll", mika, &unlink).await.unwrap();
    assert!(!unlinked.poll.as_option().unwrap().open_to_friends);
    let newcomer = ctx.app.user("Newcomer").await;
    ctx.befriend(mika, &newcomer).await;
    assert_eq!(ctx.refusal(POLLS, "GetPoll", &newcomer, &by_link).await, "POLL_NOT_FOUND");
}

#[tokio::test]
async fn leaving_a_plan_is_for_good() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui"]).await;
    let (mika, yui) = (&users[0], &users[1]);
    let plan = ctx.create_plan(mika, plan_at("飲み会", in_days(3), &[yui])).await;
    let _: LeavePlanResponse = ctx
        .call(PLANS, "LeavePlan", yui, &LeavePlanRequest { plan_id: plan.id.clone(), ..Default::default() })
        .await
        .unwrap();

    // The host asks again, and again: Yui is not brought back and hears nothing.
    for _ in 0..3 {
        let asked: InviteToPlanResponse = ctx.call(PLANS, "InviteToPlan", mika, &invite(&plan, &[yui])).await.unwrap();
        assert_eq!(asked.invited_count, 0);
    }
    assert_eq!(ctx.notifications_for(yui.id, "plans.invited").await.len(), 1);
    assert_eq!(ctx.refusal(PLANS, "GetPlan", yui, &plan_ref(&plan)).await, "PLAN_NOT_FOUND");
    assert_eq!(ctx.get_plan(mika, &plan.id).await.participants.len(), 1);
}

#[tokio::test]
async fn a_page_of_free_friends_holds_only_what_the_caller_may_see() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota", "Hana"]).await;
    let (mika, yui, sota, hana) = (&users[0], &users[1], &users[2], &users[3]);
    let free = |audience: FreeAudience| SetFreeStatusRequest {
        window: FreeWindow::FREE_WINDOW_NOW.into(),
        audience: audience.into(),
        ..Default::default()
    };
    // Yui's status is for her best friends, and Mika is not one of them.
    let _: SetFreeStatusResponse =
        ctx.call(FREE, "SetFreeStatus", yui, &free(FreeAudience::FREE_AUDIENCE_BEST_FRIENDS)).await.unwrap();
    let _: SetFreeStatusResponse =
        ctx.call(FREE, "SetFreeStatus", sota, &free(FreeAudience::FREE_AUDIENCE_FRIENDS)).await.unwrap();
    let page = |cursor: String| ListFreeFriendsRequest {
        page: tokuly_proto::common::PageRequest { limit: 1, cursor, ..Default::default() }.into(),
        ..Default::default()
    };
    let owner = |list: &ListFreeFriendsResponse| list.statuses[0].user.as_option().unwrap().id.clone();

    // The first page is Sota's status, not an empty page pointing at Yui's.
    let first: ListFreeFriendsResponse = ctx.call(FREE, "ListFreeFriends", mika, &page(String::new())).await.unwrap();
    assert_eq!((first.statuses.len(), owner(&first)), (1, sota.id.to_string()));
    let info = first.page.as_option().unwrap();
    assert!(!info.has_more && info.next_cursor.is_empty());

    // With one more friend free there is a second page, reached from a
    // cursor that names what Mika was shown.
    let _: SetFreeStatusResponse =
        ctx.call(FREE, "SetFreeStatus", hana, &free(FreeAudience::FREE_AUDIENCE_FRIENDS)).await.unwrap();
    let first: ListFreeFriendsResponse = ctx.call(FREE, "ListFreeFriends", mika, &page(String::new())).await.unwrap();
    let info = first.page.as_option().unwrap();
    assert!(info.has_more);
    assert_eq!(owner(&first), sota.id.to_string());
    let second: ListFreeFriendsResponse =
        ctx.call(FREE, "ListFreeFriends", mika, &page(info.next_cursor.clone())).await.unwrap();
    assert_eq!((second.statuses.len(), owner(&second)), (1, hana.id.to_string()));
    assert!(!second.page.as_option().unwrap().has_more);
}
