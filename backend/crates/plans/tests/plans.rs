//! `PlanService`: plans, invitations, RSVP with a waitlist, roles and the checklist.
mod common;

use chrono::{DateTime, Duration, Utc};
use common::*;
use tokuly_plans::PlansConfig;
use tokuly_proto::{
    common::{LatLng, PageRequest, SubjectType},
    plans::{plans_event::Event as Push, *},
};
use tokuly_testkit::TestUser;
use uuid::Uuid;

fn in_days(days: i64) -> DateTime<Utc> {
    Utc::now() + Duration::days(days)
}

fn plan_ref(plan: &Plan) -> GetPlanRequest {
    GetPlanRequest { plan_id: plan.id.clone(), ..Default::default() }
}

async fn list(ctx: &Ctx, user: &TestUser, request: ListPlansRequest) -> ListPlansResponse {
    ctx.call(PLANS, "ListPlans", user, &request).await.expect("list plans")
}

fn titles(plans: &[Plan]) -> Vec<&str> {
    plans.iter().map(|plan| plan.title.as_str()).collect()
}

async fn update(ctx: &Ctx, user: &TestUser, request: UpdatePlanRequest) -> Plan {
    let response: UpdatePlanResponse = ctx.call(PLANS, "UpdatePlan", user, &request).await.expect("update plan");
    response.plan.as_option().cloned().expect("a plan")
}

async fn set_role(ctx: &Ctx, host: &TestUser, plan: &Plan, user: &TestUser, role: PlanRole) -> Plan {
    let request = SetPlanRoleRequest {
        plan_id: plan.id.clone(),
        user_id: user.id.to_string(),
        role: role.into(),
        ..Default::default()
    };
    let response: SetPlanRoleResponse = ctx.call(PLANS, "SetPlanRole", host, &request).await.expect("set role");
    response.plan.as_option().cloned().expect("a plan")
}

#[tokio::test]
async fn a_plan_brings_friends_together() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let outsider = ctx.app.user("Outsider").await;

    let start = in_days(3);
    let request = CreatePlanRequest {
        emoji: "🎤".into(),
        description: "2時間くらい歌おう".into(),
        place: PlanPlace {
            name: "駅前のカラオケ".into(),
            location: LatLng { lat: 35.66, lng: 139.7, ..Default::default() }.into(),
            place_id: "place-123".into(),
            ..Default::default()
        }
        .into(),
        ..plan_at("カラオケ", start, &[yui, sota])
    };
    let plan = ctx.create_plan(mika, request).await;
    assert_eq!(
        (plan.title.as_str(), plan.emoji.as_str(), plan.description.as_str()),
        ("カラオケ", "🎤", "2時間くらい歌おう")
    );
    assert_eq!(plan.status, PlanStatus::PLAN_STATUS_ACTIVE);
    assert_eq!(plan.host_user_id, mika.id.to_string());
    let time = plan.time.as_option().unwrap();
    assert!(matches!(&time.start, Some(plan_time::Start::StartAt(at)) if at.seconds == start.timestamp()));
    assert_eq!(time.time_zone, "Asia/Tokyo");
    let place = plan.place.as_option().unwrap();
    assert_eq!(
        (place.name.as_str(), place.place_id.as_str(), place.location.as_option().unwrap().lat),
        ("駅前のカラオケ", "place-123", 35.66)
    );
    // Defaults: reminders the day before and an hour before, yen, no limit.
    assert_eq!(
        (plan.remind_day_before, plan.remind_minutes_before, plan.currency.as_str(), plan.capacity),
        (true, 60, "JPY", 0)
    );
    assert!(!plan.over && !plan.guests_can_invite);

    // The host is going; the invitees have not answered.
    assert_eq!((plan.my_role, plan.my_rsvp), (PlanRole::PLAN_ROLE_HOST.into(), Rsvp::RSVP_GOING.into()));
    let counts = plan.counts.as_option().unwrap();
    assert_eq!((counts.going, counts.invited, counts.maybe, counts.declined, counts.waitlisted), (1, 2, 0, 0, 0));
    assert_eq!(plan.participants.len(), 3);
    assert_eq!(plan.participants[0].user.as_option().unwrap().id, mika.id.to_string());
    let invitee = participant(&plan, yui);
    assert_eq!(
        (invitee.role, invitee.rsvp, invitee.invited_by_user_id.clone()),
        (PlanRole::PLAN_ROLE_GUEST.into(), Rsvp::RSVP_INVITED.into(), mika.id.to_string())
    );

    // Invitees see the plan as their own invitation; nobody else sees it at all.
    let seen = ctx.get_plan(yui, &plan.id).await;
    assert_eq!((seen.my_role, seen.my_rsvp), (PlanRole::PLAN_ROLE_GUEST.into(), Rsvp::RSVP_INVITED.into()));
    assert_eq!(ctx.refusal(PLANS, "GetPlan", &outsider, &plan_ref(&plan)).await, "PLAN_NOT_FOUND");
    assert_eq!(
        ctx.refusal(
            PLANS,
            "GetPlan",
            mika,
            &GetPlanRequest { plan_id: Uuid::now_v7().to_string(), ..Default::default() }
        )
        .await,
        "PLAN_NOT_FOUND"
    );

    // Each invitee is told.
    for invited in [yui, sota] {
        let notices = ctx.notifications_for(invited.id, "plans.invited").await;
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].data["title"]["ja"], serde_json::json!("Mikaさんから予定のお誘い!"));
        assert!(notices[0].data["body"]["ja"].as_str().unwrap().starts_with("「🎤 カラオケ」"));
        assert_eq!(notices[0].data["path"], serde_json::json!(format!("/plans/{}", plan.id)));
        assert_eq!(notices[0].actor_id, Some(mika.id));
    }
    assert!(ctx.notifications_for(mika.id, "plans.invited").await.is_empty());

    // Other domains hear of the plan; the host's log gets a private entry.
    let created = ctx.app.events_by("plans.plan.created", mika.id).await;
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].data["plan_id"], serde_json::json!(plan.id));
    assert_eq!(created[0].data["invitee_ids"].as_array().unwrap().len(), 2);
    let logged = ctx.activities_of(mika.id, "plans.created").await;
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0].data["audience"], serde_json::json!("private"));
    assert_eq!(logged[0].data["subject"], serde_json::json!([SubjectType::SUBJECT_TYPE_PLAN as i32, plan.id]));
}

#[tokio::test]
async fn a_plan_can_be_on_a_day_or_still_undecided() {
    let ctx = start().await;
    let mika = ctx.app.user("Mika").await;
    let day = (Utc::now() + Duration::days(10)).format("%Y-%m-%d").to_string();
    let next_day = (Utc::now() + Duration::days(11)).format("%Y-%m-%d").to_string();

    let trip = CreatePlanRequest {
        title: "旅行".into(),
        time: PlanTime {
            start: Some(plan_time::Start::StartDate(day.clone())),
            end: Some(plan_time::End::EndDate(next_day.clone())),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };
    let trip = ctx.create_plan(&mika, trip).await;
    let time = trip.time.as_option().unwrap();
    assert_eq!(time.start, Some(plan_time::Start::StartDate(day)));
    assert_eq!(time.end, Some(plan_time::End::EndDate(next_day)));
    // The caller's time zone is the default.
    assert_eq!(time.time_zone, "Asia/Tokyo");

    let someday = ctx.create_plan(&mika, CreatePlanRequest { title: "いつか焼肉".into(), ..Default::default() }).await;
    let time = someday.time.as_option().unwrap();
    assert!(time.start.is_none() && time.end.is_none());
    assert!(!someday.over);
}

#[tokio::test]
async fn invalid_plans_are_rejected() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui"]).await;
    let (mika, yui) = (&users[0], &users[1]);
    let stranger = ctx.app.user("Stranger").await;
    let base = || plan_at("カラオケ", in_days(3), &[]);
    let at = |start: DateTime<Utc>, end: DateTime<Utc>| PlanTime {
        start: Some(plan_time::Start::StartAt(Box::new(start.into()))),
        end: Some(plan_time::End::EndAt(Box::new(end.into()))),
        ..Default::default()
    };

    let cases = [
        (CreatePlanRequest { title: "   ".into(), ..base() }, "TITLE_REQUIRED"),
        (CreatePlanRequest { title: "あ".repeat(81), ..base() }, "TITLE_TOO_LONG"),
        (CreatePlanRequest { emoji: "karaoke".into(), ..base() }, "EMOJI_INVALID"),
        (CreatePlanRequest { description: "あ".repeat(2001), ..base() }, "DESCRIPTION_TOO_LONG"),
        (CreatePlanRequest { time: at(in_days(3), in_days(2)).into(), ..base() }, "TIME_INVALID"),
        (CreatePlanRequest { time: at(in_days(-60), in_days(-59)).into(), ..base() }, "TIME_INVALID"),
        (
            CreatePlanRequest {
                time: PlanTime { time_zone: "Mars/Olympus".into(), ..Default::default() }.into(),
                ..base()
            },
            "TIME_ZONE_INVALID",
        ),
        (
            CreatePlanRequest {
                place: PlanPlace {
                    name: "月".into(),
                    location: LatLng { lat: 100.0, lng: 0.0, ..Default::default() }.into(),
                    ..Default::default()
                }
                .into(),
                ..base()
            },
            "PLACE_INVALID",
        ),
        (CreatePlanRequest { capacity: -1, ..base() }, "CAPACITY_INVALID"),
        (CreatePlanRequest { remind_minutes_before: Some(20_000), ..base() }, "REMINDER_INVALID"),
        (CreatePlanRequest { currency: "yen".into(), ..base() }, "CURRENCY_INVALID"),
        (CreatePlanRequest { invitee_ids: ids(&[yui, &stranger]), ..base() }, "NOT_FRIENDS"),
        (CreatePlanRequest { invitee_ids: vec!["somebody".into()], ..base() }, "INVALID_ID"),
    ];
    for (request, reason) in cases {
        assert_eq!(ctx.refusal(PLANS, "CreatePlan", mika, &request).await, reason);
    }
    assert!(list(&ctx, mika, ListPlansRequest::default()).await.plans.is_empty());

    // Every RPC of the service needs a signed-in caller.
    let methods = [
        "CreatePlan",
        "GetPlan",
        "UpdatePlan",
        "CancelPlan",
        "ListPlans",
        "InviteToPlan",
        "RemovePlanParticipant",
        "LeavePlan",
        "SetPlanRole",
        "RespondToPlan",
        "AddChecklistItem",
        "UpdateChecklistItem",
        "RemoveChecklistItem",
        "AddExpense",
        "UpdateExpense",
        "RemoveExpense",
        "ListExpenses",
        "GetSplit",
        "SettleTransfer",
        "RemoveSettlement",
    ];
    for method in methods {
        let error = ctx
            .app
            .call::<_, serde_json::Value>(&format!("{PLANS}/{method}"), None, &serde_json::json!({}))
            .await
            .unwrap_err();
        assert_eq!((error.code.as_str(), error.reason.as_str()), ("unauthenticated", "SIGN_IN_REQUIRED"), "{method}");
    }
}

#[tokio::test]
async fn hosts_change_and_cancel_plans() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let outsider = ctx.app.user("Outsider").await;
    let plan = ctx.create_plan(mika, plan_at("花火", in_days(3), &[yui, sota])).await;
    // Sota cannot come: he hears no more about changes.
    assert_eq!(ctx.rsvp(sota, &plan.id, Rsvp::RSVP_DECLINED).await, Rsvp::RSVP_DECLINED);
    let change = || UpdatePlanRequest { plan_id: plan.id.clone(), ..Default::default() };

    // Only the host and co-hosts change a plan.
    assert_eq!(
        ctx.refusal(PLANS, "UpdatePlan", yui, &UpdatePlanRequest { title: Some("ぼくの花火".into()), ..change() })
            .await,
        "NOT_A_HOST"
    );
    assert_eq!(ctx.refusal(PLANS, "UpdatePlan", &outsider, &change()).await, "PLAN_NOT_FOUND");
    assert_eq!(
        ctx.refusal(PLANS, "UpdatePlan", mika, &UpdatePlanRequest { title: Some(String::new()), ..change() }).await,
        "TITLE_REQUIRED"
    );

    // A new title or description is not worth a notification.
    let renamed = update(
        &ctx,
        mika,
        UpdatePlanRequest {
            title: Some("花火大会".into()),
            emoji: Some("🎆".into()),
            description: Some("浴衣で!".into()),
            ..change()
        },
    )
    .await;
    assert_eq!(
        (renamed.title.as_str(), renamed.emoji.as_str(), renamed.description.as_str()),
        ("花火大会", "🎆", "浴衣で!")
    );
    assert!(ctx.notifications_for(yui.id, "plans.updated").await.is_empty());

    // A new time is: everyone who has not declined is told.
    let later = in_days(4);
    let moved = update(
        &ctx,
        mika,
        UpdatePlanRequest {
            time: PlanTime { start: Some(plan_time::Start::StartAt(Box::new(later.into()))), ..Default::default() }
                .into(),
            ..change()
        },
    )
    .await;
    assert!(
        matches!(&moved.time.as_option().unwrap().start, Some(plan_time::Start::StartAt(at)) if at.seconds == later.timestamp())
    );
    let told = ctx.notifications_for(yui.id, "plans.updated").await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].data["collapse_key"], serde_json::json!(format!("plans.updated:{}", plan.id)));
    assert!(told[0].data["body"]["ja"].as_str().unwrap().starts_with("「🎆 花火大会」が"));
    assert!(ctx.notifications_for(sota.id, "plans.updated").await.is_empty());
    assert!(ctx.notifications_for(mika.id, "plans.updated").await.is_empty());

    // So is a new place.
    let placed = update(
        &ctx,
        mika,
        UpdatePlanRequest { place: PlanPlace { name: "河川敷".into(), ..Default::default() }.into(), ..change() },
    )
    .await;
    assert_eq!(placed.place.as_option().unwrap().name, "河川敷");
    let told = ctx.notifications_for(yui.id, "plans.updated").await;
    assert_eq!(told.len(), 2);
    assert_eq!(told[1].data["body"]["ja"], serde_json::json!("「🎆 花火大会」の場所が河川敷に変更になったよ!"));

    // An empty time makes the plan "to be decided" again; an empty place name removes the place.
    let open = update(
        &ctx,
        mika,
        UpdatePlanRequest { time: PlanTime::default().into(), place: PlanPlace::default().into(), ..change() },
    )
    .await;
    assert!(open.time.as_option().unwrap().start.is_none() && open.place.is_unset());

    // Cancelling is the host's call.
    let cancel =
        CancelPlanRequest { plan_id: plan.id.clone(), reason: "雨予報のため".into(), ..Default::default() };
    assert_eq!(ctx.refusal(PLANS, "CancelPlan", yui, &cancel).await, "NOT_THE_HOST");
    assert_eq!(
        ctx.refusal(PLANS, "CancelPlan", mika, &CancelPlanRequest { reason: "あ".repeat(201), ..cancel.clone() }).await,
        "REASON_TOO_LONG"
    );
    let cancelled: CancelPlanResponse = ctx.call(PLANS, "CancelPlan", mika, &cancel).await.unwrap();
    let cancelled = cancelled.plan.as_option().unwrap();
    assert_eq!(
        (cancelled.status, cancelled.cancel_reason.as_str()),
        (PlanStatus::PLAN_STATUS_CANCELLED.into(), "雨予報のため")
    );
    assert!(cancelled.cancelled_at.is_set());
    let told = ctx.notifications_for(yui.id, "plans.cancelled").await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].data["body"]["ja"], serde_json::json!("「🎆 花火大会」が中止になったよ:雨予報のため"));
    assert!(ctx.notifications_for(sota.id, "plans.cancelled").await.is_empty());
    assert_eq!(ctx.app.events_by("plans.plan.cancelled", mika.id).await.len(), 1);

    // A cancelled plan stays readable and is otherwise closed.
    assert_eq!(ctx.get_plan(yui, &plan.id).await.status, PlanStatus::PLAN_STATUS_CANCELLED);
    assert_eq!(ctx.refusal(PLANS, "CancelPlan", mika, &cancel).await, "PLAN_CANCELLED");
    assert_eq!(
        ctx.refusal(PLANS, "UpdatePlan", mika, &UpdatePlanRequest { title: Some("やっぱりやる".into()), ..change() })
            .await,
        "PLAN_CANCELLED"
    );
    let going = RespondToPlanRequest { plan_id: plan.id.clone(), rsvp: Rsvp::RSVP_GOING.into(), ..Default::default() };
    assert_eq!(ctx.refusal(PLANS, "RespondToPlan", yui, &going).await, "PLAN_CANCELLED");
}

#[tokio::test]
async fn plans_are_listed_by_where_they_stand() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui"]).await;
    let (mika, yui) = (&users[0], &users[1]);

    let soon = ctx.create_plan(mika, plan_at("あさって", in_days(2), &[yui])).await;
    let later = ctx.create_plan(mika, plan_at("来週", in_days(7), &[yui])).await;
    ctx.create_plan(mika, CreatePlanRequest { title: "いつか".into(), invitee_ids: ids(&[yui]), ..Default::default() })
        .await;
    // Written down after the fact, and one that was called off.
    ctx.create_plan(mika, plan_at("きのう", in_days(-1), &[yui])).await;
    let called_off = ctx.create_plan(mika, plan_at("中止", in_days(5), &[yui])).await;
    let _: CancelPlanResponse = ctx
        .call(PLANS, "CancelPlan", mika, &CancelPlanRequest { plan_id: called_off.id.clone(), ..Default::default() })
        .await
        .unwrap();

    // Upcoming: soonest first, the undated one last.
    let upcoming = list(&ctx, yui, ListPlansRequest::default()).await;
    assert_eq!(titles(&upcoming.plans), ["あさって", "来週", "いつか"]);
    assert!(!upcoming.page.as_option().unwrap().has_more);
    // Lists carry a preview: the host, no checklist.
    assert_eq!(upcoming.plans[0].participants.len(), 1);
    assert_eq!(upcoming.plans[0].participants[0].role, PlanRole::PLAN_ROLE_HOST);
    assert_eq!(upcoming.plans[0].counts.as_option().unwrap().invited, 1);

    // All three wait for Yui's answer; answering takes a plan off that list.
    let waiting = |request: ListPlansRequest| ListPlansRequest {
        filter: PlanListFilter::PLAN_LIST_FILTER_INVITED.into(),
        ..request
    };
    assert_eq!(
        titles(&list(&ctx, yui, waiting(ListPlansRequest::default())).await.plans),
        ["あさって", "来週", "いつか"]
    );
    ctx.rsvp(yui, &soon.id, Rsvp::RSVP_GOING).await;
    assert_eq!(titles(&list(&ctx, yui, waiting(ListPlansRequest::default())).await.plans), ["来週", "いつか"]);
    assert!(list(&ctx, mika, waiting(ListPlansRequest::default())).await.plans.is_empty());

    // A declined plan leaves the upcoming list unless asked for.
    ctx.rsvp(yui, &later.id, Rsvp::RSVP_DECLINED).await;
    assert_eq!(titles(&list(&ctx, yui, ListPlansRequest::default()).await.plans), ["あさって", "いつか"]);
    assert_eq!(
        titles(&list(&ctx, yui, ListPlansRequest { include_declined: true, ..Default::default() }).await.plans),
        ["あさって", "来週", "いつか"]
    );
    assert_eq!(titles(&list(&ctx, mika, ListPlansRequest::default()).await.plans), ["あさって", "来週", "いつか"]);

    // Past: over or cancelled, most recent first.
    let past = list(
        &ctx,
        yui,
        ListPlansRequest { filter: PlanListFilter::PLAN_LIST_FILTER_PAST.into(), ..Default::default() },
    )
    .await;
    assert_eq!(titles(&past.plans), ["中止", "きのう"]);
    assert!(past.plans[1].over && !past.plans[0].over);

    // Pages follow each other without gaps or repeats.
    let mut seen = Vec::new();
    let mut cursor = String::new();
    loop {
        let page = list(
            &ctx,
            mika,
            ListPlansRequest {
                page: PageRequest { limit: 2, cursor, ..Default::default() }.into(),
                ..Default::default()
            },
        )
        .await;
        seen.extend(page.plans.iter().map(|plan| plan.title.clone()));
        let info = page.page.as_option().unwrap();
        if !info.has_more {
            break;
        }
        cursor = info.next_cursor.clone();
    }
    assert_eq!(seen, ["あさって", "来週", "いつか"]);
    let bad = ListPlansRequest {
        page: PageRequest { cursor: "!!".into(), ..Default::default() }.into(),
        ..Default::default()
    };
    assert_eq!(ctx.refusal(PLANS, "ListPlans", mika, &bad).await, "INVALID_CURSOR");
}

#[tokio::test]
async fn invitations_follow_the_hosts_rules() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    // Hana is Yui's friend, not Mika's.
    let hana = ctx.app.user("Hana").await;
    ctx.befriend(yui, &hana).await;
    let stranger = ctx.app.user("Stranger").await;
    let plan = ctx.create_plan(mika, plan_at("ピクニック", in_days(3), &[yui])).await;
    let invite = |invitees: &[&TestUser]| InviteToPlanRequest {
        plan_id: plan.id.clone(),
        user_ids: ids(invitees),
        ..Default::default()
    };

    // A guest may not bring friends unless the host allows it.
    assert_eq!(ctx.refusal(PLANS, "InviteToPlan", yui, &invite(&[&hana])).await, "CANNOT_INVITE");
    assert_eq!(ctx.refusal(PLANS, "InviteToPlan", &stranger, &invite(&[])).await, "PLAN_NOT_FOUND");

    // The host invites their own friends; people already in the plan are skipped.
    let invited: InviteToPlanResponse = ctx.call(PLANS, "InviteToPlan", mika, &invite(&[yui, sota])).await.unwrap();
    assert_eq!(invited.invited_count, 1);
    assert_eq!(invited.plan.as_option().unwrap().participants.len(), 3);
    assert_eq!(ctx.notifications_for(sota.id, "plans.invited").await.len(), 1);
    assert_eq!(ctx.notifications_for(yui.id, "plans.invited").await.len(), 1);
    // Hana is not Mika's friend.
    assert_eq!(ctx.refusal(PLANS, "InviteToPlan", mika, &invite(&[&hana])).await, "NOT_FRIENDS");

    // Once the host allows it, a guest brings their own friends.
    update(
        &ctx,
        mika,
        UpdatePlanRequest { plan_id: plan.id.clone(), guests_can_invite: Some(true), ..Default::default() },
    )
    .await;
    let mut hanas_phone = ctx.app.listen_user(hana.id).await;
    let invited: InviteToPlanResponse = ctx.call(PLANS, "InviteToPlan", yui, &invite(&[&hana])).await.unwrap();
    assert_eq!(invited.invited_count, 1);
    let brought = ctx.get_plan(mika, &plan.id).await;
    assert_eq!(participant(&brought, &hana).invited_by_user_id, yui.id.to_string());
    let told = ctx.notifications_for(hana.id, "plans.invited").await;
    assert_eq!((told.len(), told[0].actor_id), (1, Some(yui.id)));
    assert!(
        matches!(next_push(&mut hanas_phone).await, Some(Push::PlanChanged(change)) if change.change == PlanChange::PLAN_CHANGE_INVITED && change.plan_id == plan.id)
    );
    assert_eq!(ctx.refusal(PLANS, "InviteToPlan", yui, &invite(&[&stranger])).await, "NOT_FRIENDS");

    // Someone who declined does not send invitations.
    ctx.rsvp(yui, &plan.id, Rsvp::RSVP_DECLINED).await;
    assert_eq!(ctx.refusal(PLANS, "InviteToPlan", yui, &invite(&[&hana])).await, "CANNOT_INVITE");
}

#[tokio::test]
async fn a_plan_holds_a_limited_number_of_people() {
    // The host and two more.
    let ctx = start_with(PlansConfig { max_participants: 3, ..PlansConfig::default() }).await;
    let users = ctx.group(&["Mika", "Yui", "Sota", "Hana"]).await;
    let (mika, yui, sota, hana) = (&users[0], &users[1], &users[2], &users[3]);

    assert_eq!(
        ctx.refusal(PLANS, "CreatePlan", mika, &plan_at("満員", in_days(3), &[yui, sota, hana])).await,
        "TOO_MANY_PARTICIPANTS"
    );
    let plan = ctx.create_plan(mika, plan_at("満員", in_days(3), &[yui, sota])).await;
    let one_more = InviteToPlanRequest { plan_id: plan.id.clone(), user_ids: ids(&[hana]), ..Default::default() };
    assert_eq!(ctx.refusal(PLANS, "InviteToPlan", mika, &one_more).await, "TOO_MANY_PARTICIPANTS");
}

#[tokio::test]
async fn answers_fill_the_plan_and_then_the_waitlist() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota", "Hana"]).await;
    let (mika, yui, sota, hana) = (&users[0], &users[1], &users[2], &users[3]);
    // Room for two: the host and one more.
    let plan = ctx
        .create_plan(mika, CreatePlanRequest { capacity: 2, ..plan_at("ボードゲーム", in_days(3), &[yui, sota, hana]) })
        .await;
    let mut mikas_phone = ctx.app.listen_user(mika.id).await;

    let answer = RespondToPlanRequest {
        plan_id: plan.id.clone(),
        rsvp: Rsvp::RSVP_GOING.into(),
        comment: "たのしみ!".into(),
        ..Default::default()
    };
    let first: RespondToPlanResponse = ctx.call(PLANS, "RespondToPlan", yui, &answer).await.unwrap();
    assert_eq!(first.rsvp, Rsvp::RSVP_GOING);
    let seen = first.plan.as_option().unwrap();
    assert_eq!((participant(seen, yui).comment.as_str(), seen.my_rsvp), ("たのしみ!", Rsvp::RSVP_GOING.into()));
    assert!(participant(seen, yui).responded_at.is_set());

    // Everyone in the plan sees the answer arrive, with the new counts.
    match next_push(&mut mikas_phone).await {
        Some(Push::PlanRsvpChanged(change)) => {
            assert_eq!(
                (change.plan_id.clone(), change.user_id.clone(), change.rsvp),
                (plan.id.clone(), yui.id.to_string(), Rsvp::RSVP_GOING.into())
            );
            let counts = change.counts.as_option().unwrap();
            assert_eq!((counts.going, counts.invited), (2, 2));
        }
        other => panic!("expected the answer to be pushed, got {other:?}"),
    }

    // The plan is full now: the next ones wait, in the order they asked.
    assert_eq!(ctx.rsvp(sota, &plan.id, Rsvp::RSVP_GOING).await, Rsvp::RSVP_WAITLISTED);
    assert_eq!(ctx.rsvp(hana, &plan.id, Rsvp::RSVP_GOING).await, Rsvp::RSVP_WAITLISTED);
    // Asking again does not lose one's place in the queue.
    assert_eq!(ctx.rsvp(sota, &plan.id, Rsvp::RSVP_GOING).await, Rsvp::RSVP_WAITLISTED);
    let full = ctx.get_plan(mika, &plan.id).await;
    let counts = full.counts.as_option().unwrap();
    assert_eq!((counts.going, counts.waitlisted, counts.invited), (2, 2, 0));
    assert_eq!(
        (
            participant(&full, sota).waitlist_position,
            participant(&full, hana).waitlist_position,
            participant(&full, yui).waitlist_position
        ),
        (1, 2, 0)
    );

    // The host hears about every answer, in one notification per plan.
    let heard = ctx.notifications_for(mika.id, "plans.rsvp").await;
    assert_eq!(heard.len(), 3);
    assert!(
        heard.iter().all(|notice| notice.data["collapse_key"] == serde_json::json!(format!("plans.rsvp:{}", plan.id)))
    );
    assert_eq!(heard[0].data["title"]["ja"], serde_json::json!("Yuiさんが「参加」と回答!"));
    assert_eq!(heard[0].data["body"]["ja"], serde_json::json!("「ボードゲーム」:たのしみ!"));
    assert_eq!(heard[1].data["title"]["ja"], serde_json::json!("Sotaさんが「キャンセル待ち」と回答!"));

    // A place that frees up goes to whoever waited longest, and they are told.
    assert_eq!(ctx.rsvp(yui, &plan.id, Rsvp::RSVP_MAYBE).await, Rsvp::RSVP_MAYBE);
    let moved = ctx.get_plan(mika, &plan.id).await;
    assert_eq!(
        (participant(&moved, sota).rsvp, participant(&moved, hana).rsvp),
        (Rsvp::RSVP_GOING.into(), Rsvp::RSVP_WAITLISTED.into())
    );
    assert_eq!(participant(&moved, hana).waitlist_position, 1);
    let promoted = ctx.notifications_for(sota.id, "plans.promoted").await;
    assert_eq!(promoted.len(), 1);
    assert!(ctx.notifications_for(hana.id, "plans.promoted").await.is_empty());
    // Other domains see each change of answer.
    let changes = ctx.app.events_by("plans.plan.rsvp_changed", sota.id).await;
    assert_eq!(
        changes.iter().map(|event| event.data["rsvp"].as_i64().unwrap()).collect::<Vec<_>>(),
        [Rsvp::RSVP_WAITLISTED as i64, Rsvp::RSVP_GOING as i64]
    );

    // More room lets the rest in.
    let roomier =
        update(&ctx, mika, UpdatePlanRequest { plan_id: plan.id.clone(), capacity: Some(0), ..Default::default() })
            .await;
    assert_eq!(participant(&roomier, hana).rsvp, Rsvp::RSVP_GOING);
    assert_eq!(ctx.notifications_for(hana.id, "plans.promoted").await.len(), 1);

    // What is not an answer, and who cannot change theirs.
    let respond =
        |rsvp: Rsvp| RespondToPlanRequest { plan_id: plan.id.clone(), rsvp: rsvp.into(), ..Default::default() };
    for not_an_answer in [Rsvp::RSVP_UNSPECIFIED, Rsvp::RSVP_INVITED, Rsvp::RSVP_WAITLISTED] {
        assert_eq!(ctx.refusal(PLANS, "RespondToPlan", yui, &respond(not_an_answer)).await, "RSVP_INVALID");
    }
    assert_eq!(ctx.refusal(PLANS, "RespondToPlan", mika, &respond(Rsvp::RSVP_DECLINED)).await, "HOST_IS_GOING");
    assert_eq!(
        ctx.refusal(
            PLANS,
            "RespondToPlan",
            yui,
            &RespondToPlanRequest { comment: "あ".repeat(141), ..respond(Rsvp::RSVP_GOING) }
        )
        .await,
        "COMMENT_TOO_LONG"
    );
    let outsider = ctx.app.user("Outsider").await;
    assert_eq!(ctx.refusal(PLANS, "RespondToPlan", &outsider, &respond(Rsvp::RSVP_GOING)).await, "PLAN_NOT_FOUND");

    // Once the plan is over, answers are closed.
    ctx.sql("UPDATE plans SET over_at = now() - interval '1 minute' WHERE id = $1", plan.id.parse().unwrap()).await;
    assert_eq!(ctx.refusal(PLANS, "RespondToPlan", yui, &respond(Rsvp::RSVP_GOING)).await, "PLAN_OVER");
}

#[tokio::test]
async fn roles_decide_who_runs_the_plan() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota", "Hana"]).await;
    let (mika, yui, sota, hana) = (&users[0], &users[1], &users[2], &users[3]);
    let plan = ctx.create_plan(mika, plan_at("文化祭の打ち上げ", in_days(3), &[yui, sota, hana])).await;
    let role = |user: &TestUser, role: PlanRole| SetPlanRoleRequest {
        plan_id: plan.id.clone(),
        user_id: user.id.to_string(),
        role: role.into(),
        ..Default::default()
    };
    let remove = |user: &TestUser| RemovePlanParticipantRequest {
        plan_id: plan.id.clone(),
        user_id: user.id.to_string(),
        ..Default::default()
    };

    // Only the host hands out roles.
    assert_eq!(ctx.refusal(PLANS, "SetPlanRole", yui, &role(sota, PlanRole::PLAN_ROLE_CO_HOST)).await, "NOT_THE_HOST");
    assert_eq!(
        ctx.refusal(PLANS, "SetPlanRole", mika, &role(yui, PlanRole::PLAN_ROLE_UNSPECIFIED)).await,
        "ROLE_INVALID"
    );
    assert_eq!(ctx.refusal(PLANS, "SetPlanRole", mika, &role(mika, PlanRole::PLAN_ROLE_GUEST)).await, "ROLE_INVALID");
    let outsider = ctx.app.user("Outsider").await;
    assert_eq!(
        ctx.refusal(PLANS, "SetPlanRole", mika, &role(&outsider, PlanRole::PLAN_ROLE_CO_HOST)).await,
        "PARTICIPANT_NOT_FOUND"
    );

    // A co-host edits the plan and invites, like the host.
    let with_co_host = set_role(&ctx, mika, &plan, yui, PlanRole::PLAN_ROLE_CO_HOST).await;
    assert_eq!(participant(&with_co_host, yui).role, PlanRole::PLAN_ROLE_CO_HOST);
    update(
        &ctx,
        yui,
        UpdatePlanRequest {
            plan_id: plan.id.clone(), description: Some("19時に校門前".into()), ..Default::default()
        },
    )
    .await;

    // Co-hosts manage the guests, but neither the host nor each other.
    assert_eq!(ctx.refusal(PLANS, "RemovePlanParticipant", sota, &remove(hana)).await, "NOT_A_HOST");
    assert_eq!(ctx.refusal(PLANS, "RemovePlanParticipant", yui, &remove(mika)).await, "CANNOT_REMOVE_HOST");
    set_role(&ctx, mika, &plan, hana, PlanRole::PLAN_ROLE_CO_HOST).await;
    assert_eq!(ctx.refusal(PLANS, "RemovePlanParticipant", yui, &remove(hana)).await, "NOT_THE_HOST");
    assert_eq!(ctx.refusal(PLANS, "RemovePlanParticipant", yui, &remove(&outsider)).await, "PARTICIPANT_NOT_FOUND");
    let mut sotas_phone = ctx.app.listen_user(sota.id).await;
    let removed: RemovePlanParticipantResponse =
        ctx.call(PLANS, "RemovePlanParticipant", yui, &remove(sota)).await.unwrap();
    assert_eq!(removed.plan.as_option().unwrap().participants.len(), 3);
    assert!(
        matches!(next_push(&mut sotas_phone).await, Some(Push::PlanChanged(change)) if change.change == PlanChange::PLAN_CHANGE_REMOVED)
    );
    assert_eq!(ctx.refusal(PLANS, "GetPlan", sota, &plan_ref(&plan)).await, "PLAN_NOT_FOUND");

    // A co-host can be made a guest again; guests leave on their own.
    let demoted = set_role(&ctx, mika, &plan, hana, PlanRole::PLAN_ROLE_GUEST).await;
    assert_eq!(participant(&demoted, hana).role, PlanRole::PLAN_ROLE_GUEST);
    let leave = LeavePlanRequest { plan_id: plan.id.clone(), ..Default::default() };
    let _: LeavePlanResponse = ctx.call(PLANS, "LeavePlan", hana, &leave).await.unwrap();
    assert_eq!(ctx.refusal(PLANS, "GetPlan", hana, &plan_ref(&plan)).await, "PLAN_NOT_FOUND");
    assert_eq!(ctx.refusal(PLANS, "LeavePlan", hana, &leave).await, "PLAN_NOT_FOUND");

    // The host cannot simply walk away: the plan is handed over first.
    assert_eq!(ctx.refusal(PLANS, "LeavePlan", mika, &leave).await, "HOST_CANNOT_LEAVE");
    let handed_over = set_role(&ctx, mika, &plan, yui, PlanRole::PLAN_ROLE_HOST).await;
    assert_eq!(handed_over.host_user_id, yui.id.to_string());
    assert_eq!(
        (participant(&handed_over, yui).role, participant(&handed_over, yui).rsvp),
        (PlanRole::PLAN_ROLE_HOST.into(), Rsvp::RSVP_GOING.into())
    );
    assert_eq!(
        (participant(&handed_over, mika).role, handed_over.my_role),
        (PlanRole::PLAN_ROLE_CO_HOST.into(), PlanRole::PLAN_ROLE_CO_HOST.into())
    );
    // The new host now has the host's rights, the old one no longer.
    assert_eq!(
        ctx.refusal(PLANS, "CancelPlan", mika, &CancelPlanRequest { plan_id: plan.id.clone(), ..Default::default() })
            .await,
        "NOT_THE_HOST"
    );
    let _: LeavePlanResponse = ctx.call(PLANS, "LeavePlan", mika, &leave).await.unwrap();
    assert_eq!(ctx.get_plan(yui, &plan.id).await.participants.len(), 1);
}

#[tokio::test]
async fn the_checklist_is_shared_by_everyone_going() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let outsider = ctx.app.user("Outsider").await;
    let plan = ctx.create_plan(mika, plan_at("お花見", in_days(3), &[yui, sota])).await;
    let add = |kind: ChecklistKind, text: &str| AddChecklistItemRequest {
        plan_id: plan.id.clone(),
        kind: kind.into(),
        text: text.to_owned(),
        ..Default::default()
    };
    let change = |item: &ChecklistItem| UpdateChecklistItemRequest { item_id: item.id.clone(), ..Default::default() };
    let item_of = |response: Result<UpdateChecklistItemResponse, _>| -> ChecklistItem {
        response.expect("update item").item.as_option().cloned().unwrap()
    };

    // Someone who has not said they are going does not touch the list.
    assert_eq!(
        ctx.refusal(PLANS, "AddChecklistItem", yui, &add(ChecklistKind::CHECKLIST_KIND_BRING, "お菓子")).await,
        "NOT_GOING"
    );
    assert_eq!(
        ctx.refusal(PLANS, "AddChecklistItem", &outsider, &add(ChecklistKind::CHECKLIST_KIND_BRING, "お菓子")).await,
        "PLAN_NOT_FOUND"
    );
    ctx.rsvp(yui, &plan.id, Rsvp::RSVP_GOING).await;
    ctx.rsvp(sota, &plan.id, Rsvp::RSVP_GOING).await;

    let mut yuis_phone = ctx.app.listen_user(yui.id).await;
    let sheet: AddChecklistItemResponse = ctx
        .call(PLANS, "AddChecklistItem", mika, &add(ChecklistKind::CHECKLIST_KIND_UNSPECIFIED, " レジャーシート "))
        .await
        .unwrap();
    let sheet = sheet.item.as_option().cloned().unwrap();
    // "Bring" is the default kind.
    assert_eq!(
        (sheet.text.as_str(), sheet.kind, sheet.created_by_user_id.clone()),
        ("レジャーシート", ChecklistKind::CHECKLIST_KIND_BRING.into(), mika.id.to_string())
    );
    assert!(
        matches!(next_push(&mut yuis_phone).await, Some(Push::ChecklistChanged(change)) if change.item.as_option().is_some_and(|item| item.id == sheet.id))
    );
    let snacks: AddChecklistItemResponse =
        ctx.call(PLANS, "AddChecklistItem", yui, &add(ChecklistKind::CHECKLIST_KIND_TODO, "場所取り")).await.unwrap();
    let snacks = snacks.item.as_option().cloned().unwrap();
    assert_eq!(snacks.kind, ChecklistKind::CHECKLIST_KIND_TODO);
    assert_eq!(
        ctx.refusal(PLANS, "AddChecklistItem", mika, &add(ChecklistKind::CHECKLIST_KIND_BRING, "  ")).await,
        "TEXT_REQUIRED"
    );
    assert_eq!(
        ctx.refusal(PLANS, "AddChecklistItem", mika, &add(ChecklistKind::CHECKLIST_KIND_BRING, &"あ".repeat(101)))
            .await,
        "TEXT_TOO_LONG"
    );

    // "I'll bring it": one person per item.
    let claimed = item_of(
        ctx.call(
            PLANS,
            "UpdateChecklistItem",
            yui,
            &UpdateChecklistItemRequest { claimed: Some(true), ..change(&sheet) },
        )
        .await,
    );
    assert_eq!(claimed.claimed_by_user_id, yui.id.to_string());
    assert_eq!(
        ctx.refusal(
            PLANS,
            "UpdateChecklistItem",
            sota,
            &UpdateChecklistItemRequest { claimed: Some(true), ..change(&sheet) }
        )
        .await,
        "ALREADY_CLAIMED"
    );
    assert_eq!(
        ctx.refusal(
            PLANS,
            "UpdateChecklistItem",
            sota,
            &UpdateChecklistItemRequest { claimed: Some(false), ..change(&sheet) }
        )
        .await,
        "NOT_ALLOWED"
    );
    // A host can release someone's claim; so can they themselves.
    let released = item_of(
        ctx.call(
            PLANS,
            "UpdateChecklistItem",
            mika,
            &UpdateChecklistItemRequest { claimed: Some(false), ..change(&sheet) },
        )
        .await,
    );
    assert!(released.claimed_by_user_id.is_empty());

    // Anyone going ticks items off (and un-ticks them).
    let ticked = item_of(
        ctx.call(
            PLANS,
            "UpdateChecklistItem",
            sota,
            &UpdateChecklistItemRequest { done: Some(true), ..change(&sheet) },
        )
        .await,
    );
    assert!(ticked.done && ticked.done_at.is_set());
    assert_eq!(ticked.done_by_user_id, sota.id.to_string());
    let overview = ctx.get_plan(yui, &plan.id).await;
    assert_eq!((overview.checklist.len(), overview.checklist_count, overview.checklist_done_count), (2, 2, 1));
    assert_eq!(overview.checklist[0].id, sheet.id);
    let unticked = item_of(
        ctx.call(
            PLANS,
            "UpdateChecklistItem",
            yui,
            &UpdateChecklistItemRequest { done: Some(false), ..change(&sheet) },
        )
        .await,
    );
    assert!(!unticked.done && unticked.done_by_user_id.is_empty());

    // Rewriting and deleting are for the author and the hosts.
    let rewrite =
        |item: &ChecklistItem, text: &str| UpdateChecklistItemRequest { text: Some(text.to_owned()), ..change(item) };
    assert_eq!(ctx.refusal(PLANS, "UpdateChecklistItem", sota, &rewrite(&sheet, "ブルーシート")).await, "NOT_ALLOWED");
    assert_eq!(ctx.refusal(PLANS, "UpdateChecklistItem", yui, &rewrite(&snacks, "")).await, "TEXT_REQUIRED");
    let rewritten = item_of(
        ctx.call(
            PLANS,
            "UpdateChecklistItem",
            mika,
            &UpdateChecklistItemRequest {
                kind: Some(ChecklistKind::CHECKLIST_KIND_BRING.into()),
                ..rewrite(&snacks, "おやつ")
            },
        )
        .await,
    );
    assert_eq!((rewritten.text.as_str(), rewritten.kind), ("おやつ", ChecklistKind::CHECKLIST_KIND_BRING.into()));
    let delete = |item: &ChecklistItem| RemoveChecklistItemRequest { item_id: item.id.clone(), ..Default::default() };
    assert_eq!(ctx.refusal(PLANS, "RemoveChecklistItem", sota, &delete(&snacks)).await, "NOT_ALLOWED");
    assert_eq!(ctx.refusal(PLANS, "RemoveChecklistItem", &outsider, &delete(&snacks)).await, "ITEM_NOT_FOUND");
    assert_eq!(ctx.refusal(PLANS, "UpdateChecklistItem", &outsider, &change(&snacks)).await, "ITEM_NOT_FOUND");
    let _: RemoveChecklistItemResponse = ctx.call(PLANS, "RemoveChecklistItem", yui, &delete(&snacks)).await.unwrap();
    assert_eq!(ctx.refusal(PLANS, "RemoveChecklistItem", yui, &delete(&snacks)).await, "ITEM_NOT_FOUND");
    assert_eq!(ctx.get_plan(mika, &plan.id).await.checklist.len(), 1);

    // Someone who backs out no longer edits the list.
    ctx.rsvp(sota, &plan.id, Rsvp::RSVP_DECLINED).await;
    assert_eq!(
        ctx.refusal(
            PLANS,
            "UpdateChecklistItem",
            sota,
            &UpdateChecklistItemRequest { done: Some(true), ..change(&sheet) }
        )
        .await,
        "NOT_GOING"
    );
}

#[tokio::test]
async fn a_block_hides_the_person_but_keeps_the_plan() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let plan = ctx.create_plan(mika, plan_at("クラス会", in_days(3), &[yui, sota])).await;
    let going = |comment: &str| RespondToPlanRequest {
        plan_id: plan.id.clone(),
        rsvp: Rsvp::RSVP_GOING.into(),
        comment: comment.to_owned(),
        ..Default::default()
    };
    let _: RespondToPlanResponse = ctx.call(PLANS, "RespondToPlan", yui, &going("行きます")).await.unwrap();
    let _: RespondToPlanResponse = ctx.call(PLANS, "RespondToPlan", sota, &going("おれも")).await.unwrap();

    ctx.block(yui, sota).await;

    // Each of the two still sees that someone is there, and nothing more.
    for (viewer, hidden) in [(yui, sota), (sota, yui)] {
        let seen = ctx.get_plan(viewer, &plan.id).await;
        assert_eq!(seen.participants.len(), 3);
        let entry = participant(&seen, hidden);
        let user = entry.user.as_option().unwrap();
        assert!(entry.hidden && entry.comment.is_empty());
        assert!(user.handle.is_empty() && user.display_name.is_empty() && user.avatar.is_unset());
        // The answer itself is part of the plan.
        assert_eq!(entry.rsvp, Rsvp::RSVP_GOING);
        assert!(!participant(&seen, mika).hidden);
    }
    // Everyone else sees both as before.
    let seen = ctx.get_plan(mika, &plan.id).await;
    assert!(!participant(&seen, yui).hidden && !participant(&seen, sota).hidden);
    assert_eq!(participant(&seen, sota).comment, "おれも");

    // Nothing one of the two does reaches the other. Sota blocks the host:
    // her changes are no longer announced to him, nor his answers to her.
    ctx.block(sota, mika).await;
    let heard_before = ctx.notifications_for(mika.id, "plans.rsvp").await.len();
    let moved = UpdatePlanRequest {
        plan_id: plan.id.clone(),
        time: PlanTime { start: Some(plan_time::Start::StartAt(Box::new(in_days(5).into()))), ..Default::default() }
            .into(),
        ..Default::default()
    };
    update(&ctx, mika, moved).await;
    assert_eq!(ctx.notifications_for(yui.id, "plans.updated").await.len(), 1);
    assert!(ctx.notifications_for(sota.id, "plans.updated").await.is_empty());
    assert_eq!(ctx.rsvp(sota, &plan.id, Rsvp::RSVP_MAYBE).await, Rsvp::RSVP_MAYBE);
    assert_eq!(ctx.notifications_for(mika.id, "plans.rsvp").await.len(), heard_before);
    // The cost split still works between them — the numbers are the plan's — but silently.
    let expense = ExpenseInput {
        title: "会費".into(),
        amount: 1000,
        method: SplitMethod::SPLIT_METHOD_EQUAL.into(),
        shares: [mika, sota]
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
    let paid = SettleTransferRequest {
        plan_id: plan.id.clone(),
        from_user_id: sota.id.to_string(),
        to_user_id: mika.id.to_string(),
        amount: 500,
        ..Default::default()
    };
    let _: SettleTransferResponse = ctx.call(PLANS, "SettleTransfer", sota, &paid).await.unwrap();
    assert!(ctx.notifications_for(mika.id, "plans.settled").await.is_empty());
    let cancel = CancelPlanRequest { plan_id: plan.id.clone(), ..Default::default() };
    let _: CancelPlanResponse = ctx.call(PLANS, "CancelPlan", mika, &cancel).await.unwrap();
    assert_eq!(ctx.notifications_for(yui.id, "plans.cancelled").await.len(), 1);
    assert!(ctx.notifications_for(sota.id, "plans.cancelled").await.is_empty());
}

#[tokio::test]
async fn plans_are_subjects_for_comments_and_reports() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui"]).await;
    let (mika, yui) = (&users[0], &users[1]);
    let outsider = ctx.app.user("Outsider").await;
    let plan = ctx
        .create_plan(mika, CreatePlanRequest { emoji: "🎳".into(), ..plan_at("ボウリング", in_days(3), &[yui]) })
        .await;
    let subjects = &ctx.app.core.subjects;
    let plan_id: Uuid = plan.id.parse().unwrap();

    let info = subjects.resolve(SubjectType::SUBJECT_TYPE_PLAN, &plan.id, Some(yui.id)).await.unwrap();
    assert_eq!((info.id, info.owner_id, info.title.as_str()), (plan_id, mika.id, "🎳 ボウリング"));
    assert_eq!(info.path, format!("/plans/{}", plan.id));
    assert!(info.interactions_enabled);

    // Only the people in the plan can attach anything to it.
    for viewer in [Some(outsider.id), None] {
        assert_eq!(
            subjects.resolve(SubjectType::SUBJECT_TYPE_PLAN, &plan.id, viewer).await.unwrap_err().reason,
            "SUBJECT_NOT_FOUND"
        );
    }
    assert_eq!(
        subjects.resolve(SubjectType::SUBJECT_TYPE_PLAN, "not-an-id", Some(mika.id)).await.unwrap_err().reason,
        "SUBJECT_NOT_FOUND"
    );

    // The comment counter is kept on the plan.
    subjects.counter_changed(SubjectType::SUBJECT_TYPE_PLAN, plan_id, "comment_count", 2).await.unwrap();
    subjects.counter_changed(SubjectType::SUBJECT_TYPE_PLAN, plan_id, "like_count", 5).await.unwrap();
    assert_eq!(ctx.get_plan(yui, &plan.id).await.comment_count, 2);
    subjects.counter_changed(SubjectType::SUBJECT_TYPE_PLAN, plan_id, "comment_count", -5).await.unwrap();
    assert_eq!(ctx.get_plan(yui, &plan.id).await.comment_count, 0);

    // A cancelled plan can still be read, but the conversation is closed.
    let _: CancelPlanResponse = ctx
        .call(PLANS, "CancelPlan", mika, &CancelPlanRequest { plan_id: plan.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert!(
        !subjects.resolve(SubjectType::SUBJECT_TYPE_PLAN, &plan.id, Some(yui.id)).await.unwrap().interactions_enabled
    );
}
