//! Requests that arrive at the same moment.
//!
//! The first test is about the connection pool. An RPC that holds a
//! transaction (a connection, and usually a lock) must not ask the pool for a
//! second connection: a handful of requests from one account would then leave
//! the lock holder waiting for a connection the others hold, and every user
//! waiting with it. So each RPC that locks is called more often at once than
//! the pool has connections, from one account, while someone else reads.
//!
//! The other tests are about "check, then write". Each sends the same request
//! several times at once and expects what the lock or constraint behind it is
//! there to guarantee: one place is taken once, one poll becomes one plan,
//! one debt is settled once.
mod common;

use std::{
    future::Future,
    pin::Pin,
    time::{Duration as StdDuration, Instant},
};

use chrono::{DateTime, Duration, Utc};
use common::*;
use futures::future::join_all;
use serde::Serialize;
use serde_json::Value;
use tokuly_proto::plans::*;
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

/// Calls at once: more than the 5 connections of the test pool.
const AT_ONCE: usize = 8;
/// The pool gives up on a connection after 10 seconds. Requests that needed
/// anywhere near that long were waiting for one.
const NO_STALL: StdDuration = StdDuration::from_secs(8);

type Answer = Result<Value, RpcError>;
type Call<'a> = Pin<Box<dyn Future<Output = Answer> + 'a>>;

fn call<'a, Req: Serialize + 'a>(
    ctx: &'a Ctx,
    service: &'static str,
    method: &'static str,
    user: &'a TestUser,
    request: Req,
) -> Call<'a> {
    Box::pin(async move { ctx.call(service, method, user, &request).await })
}

/// The same call, [`AT_ONCE`] times.
fn repeated<'a, Req: Serialize + Clone + 'a>(
    ctx: &'a Ctx,
    service: &'static str,
    method: &'static str,
    user: &'a TestUser,
    request: Req,
) -> Vec<Call<'a>> {
    (0..AT_ONCE).map(|_| call(ctx, service, method, user, request.clone())).collect()
}

/// A failure of the server rather than a refusal: an error nobody chose to
/// answer with (a pool that ran dry, a deadlock, a broken constraint).
fn is_failure(error: &RpcError) -> bool {
    error.status >= 500 || error.reason.is_empty() || error.reason == "INTERNAL"
}

fn reasons(answers: &[Answer]) -> Vec<String> {
    let mut reasons: Vec<String> = answers
        .iter()
        .map(|answer| answer.as_ref().err().map(|e| e.reason.clone()).unwrap_or_else(|| "OK".into()))
        .collect();
    reasons.sort();
    reasons
}

fn succeeded(answers: &[Answer]) -> usize {
    answers.iter().filter(|answer| answer.is_ok()).count()
}

/// Runs `calls` side by side with a read by `bystander`. None of them may
/// fail or wait for the pool; the bystander is served.
async fn burst<'a>(
    ctx: &'a Ctx,
    what: &str,
    mut calls: Vec<Call<'a>>,
    bystander: &'a TestUser,
    plan_id: &str,
) -> Vec<Answer> {
    calls.push(call(
        ctx,
        PLANS,
        "GetPlan",
        bystander,
        GetPlanRequest { plan_id: plan_id.to_owned(), ..Default::default() },
    ));
    let started = Instant::now();
    let mut answers = join_all(calls).await;
    let took = started.elapsed();
    assert!(took < NO_STALL, "{what}: {} calls at once took {took:?}", answers.len());
    for answer in &answers {
        if let Err(error) = answer {
            assert!(!is_failure(error), "{what}: {error}");
        }
    }
    let read = answers.pop().expect("the bystander's read");
    assert!(read.is_ok(), "{what}: someone else could not read meanwhile");
    answers
}

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

async fn count(ctx: &Ctx, query: &'static str, id: Uuid) -> i64 {
    sqlx::query_scalar(query).bind(id).fetch_one(&ctx.app.core.db).await.expect("count")
}

fn free_now() -> SetFreeStatusRequest {
    SetFreeStatusRequest {
        window: FreeWindow::FREE_WINDOW_NOW.into(),
        audience: FreeAudience::FREE_AUDIENCE_FRIENDS.into(),
        ..Default::default()
    }
}

fn equal_split(title: &str, amount: i64, payer: &TestUser, among: &[&TestUser]) -> ExpenseInput {
    ExpenseInput {
        title: title.to_owned(),
        amount,
        paid_by_user_id: payer.id.to_string(),
        method: SplitMethod::SPLIT_METHOD_EQUAL.into(),
        shares: among.iter().map(|user| ExpenseShare { user_id: user.id.to_string(), ..Default::default() }).collect(),
        ..Default::default()
    }
}

fn rsvp(plan: &Plan, answer: Rsvp) -> RespondToPlanRequest {
    RespondToPlanRequest { plan_id: plan.id.clone(), rsvp: answer.into(), ..Default::default() }
}

#[tokio::test]
async fn one_account_cannot_stall_the_api() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota", "Hana", "Kenta", "Riko"]).await;
    let (mika, yui, sota, hana, kenta, riko) = (&users[0], &users[1], &users[2], &users[3], &users[4], &users[5]);

    // Mika is the one account. She hosts a plan with Yui and Sota, is a
    // guest in Yui's plan, runs a poll and is invited to Yui's.
    let hers = ctx.create_plan(mika, plan_at("花火", in_days(3), &[yui, sota])).await;
    ctx.rsvp(yui, &hers.id, Rsvp::RSVP_GOING).await;
    let yuis = ctx.create_plan(yui, plan_at("カラオケ", in_days(4), &[mika])).await;
    let poll = ctx
        .create_poll(
            mika,
            CreatePollRequest {
                title: "いつ行く?".into(),
                slots: vec![day(5), day(6)],
                invitee_ids: ids(&[yui]),
                ..Default::default()
            },
        )
        .await;
    ctx.answer_poll(yui, &poll, &[PollAnswer::POLL_ANSWER_YES]).await;
    let yuis_poll = ctx
        .create_poll(
            yui,
            CreatePollRequest {
                title: "映画".into(),
                slots: vec![day(7)],
                invitee_ids: ids(&[mika]),
                ..Default::default()
            },
        )
        .await;
    let _: SetFreeStatusResponse = ctx.call(FREE, "SetFreeStatus", yui, &free_now()).await.unwrap();
    // Yui watches the plan the whole time.
    macro_rules! at_once {
        ($what:literal, $calls:expr) => {
            burst(&ctx, $what, $calls, yui, &hers.id).await
        };
    }

    // ── ひま ──
    at_once!("SetFreeStatus", repeated(&ctx, FREE, "SetFreeStatus", mika, free_now()));
    let answer =
        AnswerFreeStatusRequest { user_id: yui.id.to_string(), message: "あそぼ!".into(), ..Default::default() };
    at_once!("AnswerFreeStatus", repeated(&ctx, FREE, "AnswerFreeStatus", mika, answer));
    let alert = SetFreeAlertRequest { user_id: yui.id.to_string(), enabled: true, ..Default::default() };
    at_once!("SetFreeAlert", repeated(&ctx, FREE, "SetFreeAlert", mika, alert));

    // ── the plan she hosts ──
    let invite = InviteToPlanRequest { plan_id: hers.id.clone(), user_ids: ids(&[hana]), ..Default::default() };
    let invited = at_once!("InviteToPlan", repeated(&ctx, PLANS, "InviteToPlan", mika, invite));
    assert_eq!(succeeded(&invited), AT_ONCE);
    let rename =
        UpdatePlanRequest { plan_id: hers.id.clone(), title: Some("花火大会".into()), ..Default::default() };
    at_once!("UpdatePlan", repeated(&ctx, PLANS, "UpdatePlan", mika, rename));
    let item = AddChecklistItemRequest {
        plan_id: hers.id.clone(),
        kind: ChecklistKind::CHECKLIST_KIND_BRING.into(),
        text: "レジャーシート".into(),
        ..Default::default()
    };
    let items = at_once!("AddChecklistItem", repeated(&ctx, PLANS, "AddChecklistItem", mika, item));
    let item_id = items[0].as_ref().unwrap()["item"]["id"].as_str().unwrap().to_owned();
    let claim = UpdateChecklistItemRequest { item_id: item_id.clone(), claimed: Some(true), ..Default::default() };
    at_once!("UpdateChecklistItem", repeated(&ctx, PLANS, "UpdateChecklistItem", mika, claim));
    let expense = AddExpenseRequest {
        plan_id: hers.id.clone(),
        expense: equal_split("たこ焼き", 800, mika, &[mika, yui]).into(),
        ..Default::default()
    };
    let expenses = at_once!("AddExpense", repeated(&ctx, PLANS, "AddExpense", mika, expense));
    assert_eq!(succeeded(&expenses), AT_ONCE);
    // Yui owes 8 × 400.
    let settle = SettleTransferRequest {
        plan_id: hers.id.clone(),
        from_user_id: yui.id.to_string(),
        to_user_id: mika.id.to_string(),
        amount: 400,
        ..Default::default()
    };
    let settled = at_once!("SettleTransfer", repeated(&ctx, PLANS, "SettleTransfer", mika, settle));
    assert_eq!(succeeded(&settled), AT_ONCE);
    let promote = SetPlanRoleRequest {
        plan_id: hers.id.clone(),
        user_id: sota.id.to_string(),
        role: PlanRole::PLAN_ROLE_CO_HOST.into(),
        ..Default::default()
    };
    at_once!("SetPlanRole", repeated(&ctx, PLANS, "SetPlanRole", mika, promote));
    let remove =
        RemovePlanParticipantRequest { plan_id: hers.id.clone(), user_id: hana.id.to_string(), ..Default::default() };
    let removed = at_once!("RemovePlanParticipant", repeated(&ctx, PLANS, "RemovePlanParticipant", mika, remove));
    assert_eq!(succeeded(&removed), 1, "{:?}", reasons(&removed));

    // ── the plan she is a guest of ──
    at_once!("RespondToPlan", repeated(&ctx, PLANS, "RespondToPlan", mika, rsvp(&yuis, Rsvp::RSVP_MAYBE)));
    let leave = LeavePlanRequest { plan_id: yuis.id.clone(), ..Default::default() };
    let left = at_once!("LeavePlan", repeated(&ctx, PLANS, "LeavePlan", mika, leave));
    assert_eq!(succeeded(&left), 1, "{:?}", reasons(&left));

    // ── polls ──
    let retitle =
        UpdatePollRequest { poll_id: poll.id.clone(), title: Some("いつにする?".into()), ..Default::default() };
    at_once!("UpdatePoll", repeated(&ctx, POLLS, "UpdatePoll", mika, retitle));
    let more = InviteToPollRequest { poll_id: poll.id.clone(), user_ids: ids(&[kenta]), ..Default::default() };
    at_once!("InviteToPoll", repeated(&ctx, POLLS, "InviteToPoll", mika, more));
    let yes = AnswerPollRequest {
        poll_id: yuis_poll.id.clone(),
        answers: vec![SlotAnswer {
            slot_id: yuis_poll.slots[0].id.clone(),
            answer: PollAnswer::POLL_ANSWER_YES.into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let answered = at_once!("AnswerPoll", repeated(&ctx, POLLS, "AnswerPoll", mika, yes));
    assert_eq!(succeeded(&answered), AT_ONCE);
    let remind = RemindPollParticipantsRequest { poll_id: poll.id.clone(), ..Default::default() };
    at_once!("RemindPollParticipants", repeated(&ctx, POLLS, "RemindPollParticipants", mika, remind));
    let close = ClosePollRequest { poll_id: poll.id.clone(), ..Default::default() };
    at_once!("ClosePoll", repeated(&ctx, POLLS, "ClosePoll", mika, close));
    let reopen = ReopenPollRequest { poll_id: poll.id.clone(), ..Default::default() };
    at_once!("ReopenPoll", repeated(&ctx, POLLS, "ReopenPoll", mika, reopen));
    let finalize = FinalizePollRequest { poll_id: poll.id.clone(), ..Default::default() };
    let finalized = at_once!("FinalizePoll", repeated(&ctx, POLLS, "FinalizePoll", mika, finalize));
    assert_eq!(succeeded(&finalized), 1, "{:?}", reasons(&finalized));
    let delete = DeletePollRequest { poll_id: poll.id.clone(), ..Default::default() };
    let deleted = at_once!("DeletePoll", repeated(&ctx, POLLS, "DeletePoll", mika, delete));
    assert_eq!(succeeded(&deleted), 1, "{:?}", reasons(&deleted));

    // ── counters and countdowns: every call waits for the same lock, hers ──
    let friends = [yui, sota, hana, kenta, riko, yui, sota, hana];
    let counters = friends
        .iter()
        .map(|friend| {
            let request = StartFriendshipCounterRequest { user_id: friend.id.to_string(), ..Default::default() };
            call(&ctx, ANNIVERSARIES, "StartFriendshipCounter", mika, request)
        })
        .collect();
    let started = at_once!("StartFriendshipCounter", counters);
    assert_eq!(succeeded(&started), 5, "{:?}", reasons(&started));
    let invitations = friends
        .iter()
        .map(|friend| {
            let request = InviteCoupleRequest { user_id: friend.id.to_string(), ..Default::default() };
            call(&ctx, ANNIVERSARIES, "InviteCouple", mika, request)
        })
        .collect();
    let sent = at_once!("InviteCouple", invitations);
    assert_eq!(succeeded(&sent), 1, "{:?}", reasons(&sent));
    let counter_id = started.iter().find_map(|answer| answer.as_ref().ok()).unwrap()["anniversary"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let name = UpdateAnniversaryRequest {
        anniversary_id: counter_id.clone(),
        title: Some("ズッ友".into()),
        ..Default::default()
    };
    at_once!("UpdateAnniversary", repeated(&ctx, ANNIVERSARIES, "UpdateAnniversary", mika, name));
    let end = EndAnniversaryRequest { anniversary_id: counter_id, ..Default::default() };
    let ended = at_once!("EndAnniversary", repeated(&ctx, ANNIVERSARIES, "EndAnniversary", mika, end));
    assert_eq!(succeeded(&ended), 1, "{:?}", reasons(&ended));
    let countdown = CreateCountdownRequest {
        title: "文化祭".into(),
        date: in_days(30).format("%Y-%m-%d").to_string(),
        ..Default::default()
    };
    let countdowns = at_once!("CreateCountdown", repeated(&ctx, ANNIVERSARIES, "CreateCountdown", mika, countdown));
    assert_eq!(succeeded(&countdowns), AT_ONCE);
    let countdown_id = countdowns[0].as_ref().unwrap()["countdown"]["id"].as_str().unwrap().to_owned();
    let pin = UpdateCountdownRequest {
        countdown_id: countdown_id.clone(),
        sharing: CountdownSharing::COUNTDOWN_SHARING_PROFILE.into(),
        ..Default::default()
    };
    at_once!("UpdateCountdown", repeated(&ctx, ANNIVERSARIES, "UpdateCountdown", mika, pin));
    let drop = DeleteCountdownRequest { countdown_id, ..Default::default() };
    at_once!("DeleteCountdown", repeated(&ctx, ANNIVERSARIES, "DeleteCountdown", mika, drop));

    // ── and the end of the plan ──
    let cancel = CancelPlanRequest { plan_id: hers.id.clone(), reason: "雨".into(), ..Default::default() };
    let cancelled = at_once!("CancelPlan", repeated(&ctx, PLANS, "CancelPlan", mika, cancel));
    assert_eq!(succeeded(&cancelled), 1, "{:?}", reasons(&cancelled));
}

#[tokio::test]
async fn events_arriving_together_do_not_stall_either() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota", "Hana"]).await;
    let (mika, yui, sota, hana) = (&users[0], &users[1], &users[2], &users[3]);
    let plan = ctx.create_plan(mika, plan_at("遠足", in_days(3), &[yui, sota, hana])).await;
    ctx.rsvp(hana, &plan.id, Rsvp::RSVP_GOING).await;

    // Yui and Sota stop being Mika's friends; each event comes several times
    // (delivery is at least once), all of them together.
    ctx.unfriend(mika, yui).await;
    ctx.unfriend(mika, sota).await;
    let event = |friend: &TestUser| tokuly_core::events::Event {
        id: Uuid::now_v7(),
        topic: "social.friend.removed".to_owned(),
        actor_id: Some(mika.id),
        occurred_at: Utc::now(),
        data: serde_json::json!({ "user_id": mika.id, "friend_id": friend.id, "blocked": false }),
    };
    let events: Vec<_> = (0..AT_ONCE).map(|n| event(if n % 2 == 0 { yui } else { sota })).collect();
    let started = Instant::now();
    let handled = join_all(events.iter().map(|event| ctx.plans.handle_event(event))).await;
    assert!(started.elapsed() < NO_STALL);
    assert!(handled.iter().all(|result| result.is_ok()));

    // Their unanswered invitations are withdrawn, once.
    let plan = ctx.get_plan(hana, &plan.id).await;
    assert_eq!(plan.participants.len(), 2);
    assert_eq!(count(&ctx, "SELECT count(*) FROM plan_past_participants WHERE plan_id = $1", id_of(&plan.id)).await, 2);
}

#[tokio::test]
async fn the_last_place_goes_to_one_person() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota", "Hana", "Kenta"]).await;
    let mika = &users[0];
    let guests: Vec<&TestUser> = users[1..].iter().collect();
    // Two places, and the host has one of them.
    let plan =
        ctx.create_plan(mika, CreatePlanRequest { capacity: 2, ..plan_at("個室ごはん", in_days(3), &guests) }).await;

    let answers = join_all(guests.iter().map(|guest| ctx.rsvp(guest, &plan.id, Rsvp::RSVP_GOING))).await;
    assert_eq!(answers.iter().filter(|answer| **answer == Rsvp::RSVP_GOING).count(), 1);
    assert_eq!(answers.iter().filter(|answer| **answer == Rsvp::RSVP_WAITLISTED).count(), 3);
    let seen = ctx.get_plan(mika, &plan.id).await;
    let counts = seen.counts.as_option().unwrap();
    assert_eq!((counts.going, counts.waitlisted), (2, 3));
    // Everyone waiting has a place in the queue of their own.
    let mut queue: Vec<i32> =
        seen.participants.iter().map(|p| p.waitlist_position).filter(|position| *position > 0).collect();
    queue.sort();
    assert_eq!(queue, vec![1, 2, 3]);

    // The guest who got in gives up while one of those waiting settles for
    // "maybe" and another asks again: one place opens, one person moves up.
    let lucky = guests[answers.iter().position(|answer| *answer == Rsvp::RSVP_GOING).unwrap()];
    let waiting: Vec<&TestUser> = guests.iter().copied().filter(|guest| guest.id != lucky.id).collect();
    let changes = join_all([
        ctx.rsvp(lucky, &plan.id, Rsvp::RSVP_DECLINED),
        ctx.rsvp(waiting[0], &plan.id, Rsvp::RSVP_MAYBE),
        ctx.rsvp(waiting[1], &plan.id, Rsvp::RSVP_GOING),
    ])
    .await;
    assert_eq!(changes[0], Rsvp::RSVP_DECLINED);
    let seen = ctx.get_plan(mika, &plan.id).await;
    let counts = seen.counts.as_option().unwrap();
    assert_eq!((counts.going, counts.declined, counts.maybe, counts.waitlisted), (2, 1, 1, 1));
}

#[tokio::test]
async fn a_poll_becomes_one_plan_however_often_it_is_finalized() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let poll = ctx
        .create_poll(
            mika,
            CreatePollRequest {
                title: "打ち上げ".into(),
                slots: vec![day(5), day(6)],
                invitee_ids: ids(&[yui, sota]),
                ..Default::default()
            },
        )
        .await;
    ctx.answer_poll(yui, &poll, &[PollAnswer::POLL_ANSWER_YES, PollAnswer::POLL_ANSWER_NO]).await;
    ctx.answer_poll(sota, &poll, &[PollAnswer::POLL_ANSWER_MAYBE, PollAnswer::POLL_ANSWER_NO]).await;

    let finalize = FinalizePollRequest { poll_id: poll.id.clone(), ..Default::default() };
    let answers = join_all((0..6).map(|_| call(&ctx, POLLS, "FinalizePoll", mika, finalize.clone()))).await;
    assert_eq!(
        reasons(&answers),
        ["OK", "POLL_FINALIZED", "POLL_FINALIZED", "POLL_FINALIZED", "POLL_FINALIZED", "POLL_FINALIZED"]
    );

    let poll_id = id_of(&poll.id);
    assert_eq!(count(&ctx, "SELECT count(*) FROM plans WHERE poll_id = $1", poll_id).await, 1);
    let made = answers.iter().find_map(|answer| answer.as_ref().ok()).unwrap();
    assert_eq!(ctx.get_poll(yui, &poll.id).await.plan_id, made["plan"]["id"].as_str().unwrap());
    // Everyone who could make it is invited to that plan, once.
    for invited in [yui, sota] {
        assert_eq!(ctx.notifications_for(invited.id, "plans.invited").await.len(), 1);
    }
    assert_eq!(ctx.app.events_by("plans.poll.finalized", mika.id).await.len(), 1);
    assert_eq!(ctx.app.events_by("plans.plan.created", mika.id).await.len(), 1);
}

#[tokio::test]
async fn a_debt_is_settled_once() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui"]).await;
    let (mika, yui) = (&users[0], &users[1]);
    let plan = ctx.create_plan(mika, plan_at("焼肉", in_days(3), &[yui])).await;
    ctx.rsvp(yui, &plan.id, Rsvp::RSVP_GOING).await;
    let expense = AddExpenseRequest {
        plan_id: plan.id.clone(),
        expense: equal_split("焼肉", 3000, mika, &[mika, yui]).into(),
        ..Default::default()
    };
    let _: AddExpenseResponse = ctx.call(PLANS, "AddExpense", mika, &expense).await.unwrap();

    // Yui owes 1500. Mika, who got the money, marks it — with a nervous thumb.
    let settle = SettleTransferRequest {
        plan_id: plan.id.clone(),
        from_user_id: yui.id.to_string(),
        to_user_id: mika.id.to_string(),
        amount: 1500,
        ..Default::default()
    };
    let answers = join_all(
        [mika, mika, mika, yui].into_iter().map(|user| call(&ctx, PLANS, "SettleTransfer", user, settle.clone())),
    )
    .await;
    // (Yui cannot mark her own debt as paid at all.)
    assert_eq!(reasons(&answers), ["AMOUNT_EXCEEDS_BALANCE", "AMOUNT_EXCEEDS_BALANCE", "NOT_ALLOWED", "OK"]);

    let split: GetSplitResponse = ctx
        .call(PLANS, "GetSplit", yui, &GetSplitRequest { plan_id: plan.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(split.settlements.len(), 1);
    assert_eq!(split.settlements[0].amount, 1500);
    assert!(split.splits[0].transfers.is_empty());
    assert!(split.splits[0].balances.iter().all(|balance| balance.net == 0));
    // Yui is told, once.
    assert_eq!(ctx.notifications_for(yui.id, "plans.settled").await.len(), 1);

    // The same holds for a debt paid in parts: together they never exceed it.
    let more = AddExpenseRequest {
        plan_id: plan.id.clone(),
        expense: equal_split("二次会", 2000, mika, &[mika, yui]).into(),
        ..Default::default()
    };
    let _: AddExpenseResponse = ctx.call(PLANS, "AddExpense", mika, &more).await.unwrap();
    let part = SettleTransferRequest { amount: 400, ..settle.clone() };
    let answers = join_all((0..6).map(|_| call(&ctx, PLANS, "SettleTransfer", mika, part.clone()))).await;
    assert_eq!(succeeded(&answers), 2, "{:?}", reasons(&answers));
    let split: GetSplitResponse = ctx
        .call(PLANS, "GetSplit", mika, &GetSplitRequest { plan_id: plan.id.clone(), ..Default::default() })
        .await
        .unwrap();
    let owed = split.splits[0].balances.iter().find(|balance| balance.user_id == mika.id.to_string()).unwrap().net;
    assert_eq!(owed, 200);
}

#[tokio::test]
async fn a_checklist_item_is_taken_by_one_person() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota", "Hana"]).await;
    let mika = &users[0];
    let guests: Vec<&TestUser> = users[1..].iter().collect();
    let plan = ctx.create_plan(mika, plan_at("BBQ", in_days(3), &guests)).await;
    for guest in &guests {
        ctx.rsvp(guest, &plan.id, Rsvp::RSVP_GOING).await;
    }
    let item: AddChecklistItemResponse = ctx
        .call(
            PLANS,
            "AddChecklistItem",
            mika,
            &AddChecklistItemRequest {
                plan_id: plan.id.clone(),
                kind: ChecklistKind::CHECKLIST_KIND_BRING.into(),
                text: "炭".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let item_id = item.item.as_option().unwrap().id.clone();

    let claim = UpdateChecklistItemRequest { item_id: item_id.clone(), claimed: Some(true), ..Default::default() };
    let answers =
        join_all(guests.iter().map(|guest| call(&ctx, PLANS, "UpdateChecklistItem", guest, claim.clone()))).await;
    assert_eq!(reasons(&answers), ["ALREADY_CLAIMED", "ALREADY_CLAIMED", "OK"]);
    let winner = guests[answers.iter().position(|answer| answer.is_ok()).unwrap()];
    let seen = ctx.get_plan(mika, &plan.id).await;
    assert_eq!(seen.checklist[0].claimed_by_user_id, winner.id.to_string());

    // Items added at once all land, up to the limit of a checklist.
    let add = AddChecklistItemRequest {
        plan_id: plan.id.clone(),
        kind: ChecklistKind::CHECKLIST_KIND_TODO.into(),
        text: "買い出し".into(),
        ..Default::default()
    };
    let added = join_all(guests.iter().map(|guest| call(&ctx, PLANS, "AddChecklistItem", guest, add.clone()))).await;
    assert_eq!(succeeded(&added), 3);
    assert_eq!(ctx.get_plan(mika, &plan.id).await.checklist_count, 4);
}

#[tokio::test]
async fn a_status_set_twice_at_once_is_one_status_announced_once() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    for friend in [yui, sota] {
        let alert = SetFreeAlertRequest { user_id: mika.id.to_string(), enabled: true, ..Default::default() };
        let _: SetFreeAlertResponse = ctx.call(FREE, "SetFreeAlert", friend, &alert).await.unwrap();
    }

    let answers = join_all((0..5).map(|_| call(&ctx, FREE, "SetFreeStatus", mika, free_now()))).await;
    assert_eq!(succeeded(&answers), 5, "{:?}", reasons(&answers));
    assert_eq!(count(&ctx, "SELECT count(*) FROM free_statuses WHERE user_id = $1", mika.id).await, 1);
    for friend in [yui, sota] {
        assert_eq!(ctx.notifications_for(friend.id, "plans.friend_free").await.len(), 1, "{}", friend.handle);
    }

    // "あそぼ!" tapped several times is one answer and one nudge.
    let asobo =
        AnswerFreeStatusRequest { user_id: mika.id.to_string(), message: "いこ!".into(), ..Default::default() };
    let answers = join_all((0..5).map(|_| call(&ctx, FREE, "AnswerFreeStatus", yui, asobo.clone()))).await;
    assert_eq!(succeeded(&answers), 5, "{:?}", reasons(&answers));
    assert_eq!(ctx.notifications_for(mika.id, "plans.nudge").await.len(), 1);
    let own: GetFreeStatusResponse =
        ctx.call(FREE, "GetFreeStatus", mika, &GetFreeStatusRequest::default()).await.unwrap();
    assert_eq!(own.status.as_option().unwrap().answer_count, 1);

    // Answers that cross a new status either reach a status or find none:
    // none of them fails.
    let mut calls: Vec<Call> = Vec::new();
    for _ in 0..3 {
        calls.push(call(&ctx, FREE, "SetFreeStatus", mika, free_now()));
        calls.push(call(&ctx, FREE, "AnswerFreeStatus", sota, asobo.clone()));
        calls.push(call(&ctx, FREE, "ClearFreeStatus", mika, ClearFreeStatusRequest::default()));
        calls.push(call(&ctx, FREE, "AnswerFreeStatus", yui, asobo.clone()));
    }
    for answer in join_all(calls).await {
        if let Err(error) = answer {
            assert_eq!(error.reason, "FREE_STATUS_NOT_FOUND", "{error}");
        }
    }
    assert!(count(&ctx, "SELECT count(*) FROM free_statuses WHERE user_id = $1", mika.id).await <= 1);
}

#[tokio::test]
async fn a_friend_invited_by_two_people_at_once_is_invited_once() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    ctx.befriend(yui, sota).await;
    let plan = ctx
        .create_plan(mika, CreatePlanRequest { guests_can_invite: true, ..plan_at("お花見", in_days(3), &[yui]) })
        .await;
    ctx.rsvp(yui, &plan.id, Rsvp::RSVP_GOING).await;

    let invite = InviteToPlanRequest { plan_id: plan.id.clone(), user_ids: ids(&[sota]), ..Default::default() };
    let answers = join_all(
        [mika, yui, mika, yui].into_iter().map(|user| call(&ctx, PLANS, "InviteToPlan", user, invite.clone())),
    )
    .await;
    assert_eq!(succeeded(&answers), 4, "{:?}", reasons(&answers));
    let newly: i64 = answers.iter().map(|answer| answer.as_ref().unwrap()["invitedCount"].as_i64().unwrap_or(0)).sum();
    assert_eq!(newly, 1);
    assert_eq!(ctx.get_plan(mika, &plan.id).await.participants.len(), 3);
    assert_eq!(ctx.notifications_for(sota.id, "plans.invited").await.len(), 1);

    // A plan never holds more people than it may, however the invitations arrive.
    let small = start_with(tokuly_plans::PlansConfig { max_participants: 3, ..Default::default() }).await;
    let crowd = small.group(&["Host", "A", "B", "C", "D"]).await;
    let host = &crowd[0];
    let plan = small.create_plan(host, plan_at("せまい部屋", in_days(3), &[])).await;
    let answers = join_all(crowd[1..].iter().map(|guest| {
        let invite = InviteToPlanRequest { plan_id: plan.id.clone(), user_ids: ids(&[guest]), ..Default::default() };
        call(&small, PLANS, "InviteToPlan", host, invite)
    }))
    .await;
    assert_eq!(reasons(&answers), ["OK", "OK", "TOO_MANY_PARTICIPANTS", "TOO_MANY_PARTICIPANTS"]);
    assert_eq!(small.get_plan(host, &plan.id).await.participants.len(), 3);
}

/// How many running counters of a kind `user` has.
async fn active(ctx: &Ctx, user: &TestUser, kind: AnniversaryKind) -> usize {
    let listed: ListAnniversariesResponse =
        ctx.call(ANNIVERSARIES, "ListAnniversaries", user, &ListAnniversariesRequest::default()).await.unwrap();
    listed
        .anniversaries
        .iter()
        .filter(|counter| counter.kind == kind && counter.status == AnniversaryStatus::ANNIVERSARY_STATUS_ACTIVE)
        .count()
}

#[tokio::test]
async fn two_people_end_up_with_one_counter_and_one_couple() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);

    // Both start the friendship counter at the same moment.
    let counter =
        |friend: &TestUser| StartFriendshipCounterRequest { user_id: friend.id.to_string(), ..Default::default() };
    let answers = join_all([
        call(&ctx, ANNIVERSARIES, "StartFriendshipCounter", mika, counter(yui)),
        call(&ctx, ANNIVERSARIES, "StartFriendshipCounter", yui, counter(mika)),
        call(&ctx, ANNIVERSARIES, "StartFriendshipCounter", mika, counter(yui)),
        call(&ctx, ANNIVERSARIES, "StartFriendshipCounter", yui, counter(mika)),
    ])
    .await;
    assert_eq!(reasons(&answers), ["ANNIVERSARY_EXISTS", "ANNIVERSARY_EXISTS", "ANNIVERSARY_EXISTS", "OK"]);
    assert_eq!(active(&ctx, mika, AnniversaryKind::ANNIVERSARY_KIND_FRIENDSHIP).await, 1);
    let started = ctx.app.events_by("plans.anniversary.started", mika.id).await.len()
        + ctx.app.events_by("plans.anniversary.started", yui.id).await.len();
    assert_eq!(started, 1);

    // Mika asked Yui; Sota asked Mika. Yui and Mika say yes at the same time:
    // Mika is in one couple, not two.
    let invite = |friend: &TestUser| InviteCoupleRequest { user_id: friend.id.to_string(), ..Default::default() };
    let to_yui: InviteCoupleResponse = ctx.call(ANNIVERSARIES, "InviteCouple", mika, &invite(yui)).await.unwrap();
    let to_mika: InviteCoupleResponse = ctx.call(ANNIVERSARIES, "InviteCouple", sota, &invite(mika)).await.unwrap();
    let yes = |sent: &InviteCoupleResponse| RespondCoupleInviteRequest {
        anniversary_id: sent.anniversary.as_option().unwrap().id.clone(),
        accept: true,
        ..Default::default()
    };
    let answers = join_all([
        call(&ctx, ANNIVERSARIES, "RespondCoupleInvite", yui, yes(&to_yui)),
        call(&ctx, ANNIVERSARIES, "RespondCoupleInvite", mika, yes(&to_mika)),
        call(&ctx, ANNIVERSARIES, "RespondCoupleInvite", yui, yes(&to_yui)),
        call(&ctx, ANNIVERSARIES, "RespondCoupleInvite", mika, yes(&to_mika)),
    ])
    .await;
    assert_eq!(succeeded(&answers), 1, "{:?}", reasons(&answers));
    for answer in &answers {
        if let Err(error) = answer {
            assert!(matches!(error.reason.as_str(), "ANNIVERSARY_NOT_FOUND" | "ALREADY_IN_COUPLE"), "{error}");
        }
    }
    let couple = AnniversaryKind::ANNIVERSARY_KIND_COUPLE;
    assert_eq!(active(&ctx, mika, couple).await, 1);
    assert_eq!(active(&ctx, yui, couple).await + active(&ctx, sota, couple).await, 1);
    // Mika no longer has an invitation of her own out.
    assert_eq!(
        count(
            &ctx,
            "SELECT count(*) FROM plan_anniversaries WHERE kind = 2 AND status = 1 AND created_by = $1",
            mika.id
        )
        .await,
        0
    );
}

#[tokio::test]
async fn limits_hold_when_everything_is_created_at_once() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);

    // Three countdowns fit on a profile.
    let pinned = CreateCountdownRequest {
        title: "テスト".into(),
        date: in_days(20).format("%Y-%m-%d").to_string(),
        sharing: CountdownSharing::COUNTDOWN_SHARING_PROFILE.into(),
        ..Default::default()
    };
    let answers = join_all((0..6).map(|_| call(&ctx, ANNIVERSARIES, "CreateCountdown", mika, pinned.clone()))).await;
    assert_eq!(reasons(&answers), ["OK", "OK", "OK", "TOO_MANY_PINNED", "TOO_MANY_PINNED", "TOO_MANY_PINNED"]);
    assert_eq!(
        count(&ctx, "SELECT count(*) FROM plan_countdowns WHERE user_id = $1 AND sharing = 3", mika.id).await,
        3
    );

    // One invitation to a couple counter is out at a time.
    let answers = join_all([yui, sota, yui, sota].into_iter().map(|friend| {
        let invite = InviteCoupleRequest { user_id: friend.id.to_string(), ..Default::default() };
        call(&ctx, ANNIVERSARIES, "InviteCouple", mika, invite)
    }))
    .await;
    assert_eq!(succeeded(&answers), 1, "{:?}", reasons(&answers));
    assert_eq!(
        ctx.notifications_for(yui.id, "plans.couple_invite").await.len()
            + ctx.notifications_for(sota.id, "plans.couple_invite").await.len(),
        1
    );

    // Someone who answers a poll through its link joins it once.
    let poll = ctx
        .create_poll(
            mika,
            CreatePollRequest {
                title: "だれでも".into(),
                slots: vec![day(5)],
                open_to_friends: true,
                ..Default::default()
            },
        )
        .await;
    let yes = AnswerPollRequest {
        poll_id: poll.short_id.clone(),
        answers: vec![SlotAnswer {
            slot_id: poll.slots[0].id.clone(),
            answer: PollAnswer::POLL_ANSWER_YES.into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let answers = join_all((0..5).map(|_| call(&ctx, POLLS, "AnswerPoll", yui, yes.clone()))).await;
    assert_eq!(succeeded(&answers), 5, "{:?}", reasons(&answers));
    let seen = ctx.get_poll(mika, &poll.id).await;
    assert_eq!((seen.participant_count, seen.slots[0].yes_count), (2, 1));
    assert_eq!(ctx.notifications_for(mika.id, "plans.poll_answer").await.len(), 1);
}

#[tokio::test]
async fn workers_running_side_by_side_send_everything_once() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let plan = ctx.create_plan(mika, plan_at("ライブ", in_days(3), &[yui, sota])).await;
    let plan_id = id_of(&plan.id);
    ctx.rsvp(yui, &plan.id, Rsvp::RSVP_GOING).await;
    ctx.rsvp(sota, &plan.id, Rsvp::RSVP_GOING).await;

    let passes = ctx
        .make_due_for_workers(
            "UPDATE plans SET day_before_due_at = now() - interval '1 minute', attended_due_at = now() - interval '1 minute' WHERE id = $1",
            &[plan_id],
            4,
        )
        .await;
    assert_eq!(passes.iter().map(|pass| pass.day_before_reminders).sum::<u64>(), 1);
    assert_eq!(passes.iter().map(|pass| pass.plans_attended).sum::<u64>(), 1);
    for user in [mika, yui, sota] {
        assert_eq!(ctx.notifications_for(user.id, "plans.reminder").await.len(), 1, "{}", user.handle);
        assert_eq!(ctx.app.events_by("plans.plan.attended", user.id).await.len(), 1, "{}", user.handle);
    }
}
