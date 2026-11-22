//! `FreeService`: the "ひま" status, its audiences, "あそぼ!" and alerts.
mod common;

use chrono::{Duration, Utc};
use common::*;
use tokuly_proto::{
    common::{LatLng, PageRequest},
    plans::{plans_event::Event as Push, *},
};
use tokuly_testkit::TestUser;
use uuid::Uuid;

async fn set_status(ctx: &Ctx, user: &TestUser, request: SetFreeStatusRequest) -> FreeStatus {
    let response: SetFreeStatusResponse =
        ctx.call(FREE, "SetFreeStatus", user, &request).await.expect("set free status");
    response.status.as_option().cloned().expect("a status")
}

async fn free_friends(ctx: &Ctx, user: &TestUser) -> ListFreeFriendsResponse {
    ctx.call(FREE, "ListFreeFriends", user, &ListFreeFriendsRequest::default()).await.expect("list free friends")
}

async fn status_of(ctx: &Ctx, viewer: &TestUser, owner: &TestUser) -> Option<FreeStatus> {
    let response: GetFreeStatusResponse = ctx
        .call(
            FREE,
            "GetFreeStatus",
            viewer,
            &GetFreeStatusRequest { user_id: owner.id.to_string(), ..Default::default() },
        )
        .await
        .expect("get free status");
    response.status.as_option().cloned()
}

fn now_for(audience: FreeAudience) -> SetFreeStatusRequest {
    SetFreeStatusRequest { window: FreeWindow::FREE_WINDOW_NOW.into(), audience: audience.into(), ..Default::default() }
}

fn owner_id(status: &FreeStatus) -> String {
    status.user.as_option().expect("an owner").id.clone()
}

#[tokio::test]
async fn friends_see_who_is_free() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui"]).await;
    let (mika, yui) = (&users[0], &users[1]);
    let stranger = ctx.app.user("Stranger").await;

    let request = SetFreeStatusRequest {
        note: "  カラオケ行きたい ".into(),
        area: LatLng { lat: 35.658_034, lng: 139.701_636, ..Default::default() }.into(),
        area_name: "渋谷あたり".into(),
        ..now_for(FreeAudience::FREE_AUDIENCE_FRIENDS)
    };
    let mine = set_status(&ctx, mika, request).await;
    assert!(mine.free_now);
    assert_eq!(mine.note, "カラオケ行きたい");
    // "Now" lasts three hours by default.
    assert_eq!(time_of(&mine.ends_at) - time_of(&mine.starts_at), Duration::hours(3));
    // The area is rounded to about a kilometre: never a precise position.
    let area = mine.area.as_option().expect("an area");
    assert_eq!((area.lat, area.lng), (35.66, 139.7));
    // The owner sees who it is for.
    assert_eq!(mine.audience, FreeAudience::FREE_AUDIENCE_FRIENDS);
    assert_eq!(mine.answer_count, 0);

    // A friend finds it in their list, without the owner-only fields.
    let listed = free_friends(&ctx, yui).await;
    assert_eq!(listed.statuses.len(), 1);
    let seen = &listed.statuses[0];
    assert_eq!(owner_id(seen), mika.id.to_string());
    assert_eq!((seen.note.as_str(), seen.area_name.as_str()), ("カラオケ行きたい", "渋谷あたり"));
    assert_eq!(seen.audience, FreeAudience::FREE_AUDIENCE_UNSPECIFIED);
    assert!(!seen.answered_by_me && listed.mine.is_unset());
    assert_eq!(status_of(&ctx, yui, mika).await.map(|status| status.id), Some(mine.id.clone()));

    // The owner's own list holds their friends' statuses, and their own on the side.
    let own = free_friends(&ctx, mika).await;
    assert!(own.statuses.is_empty());
    assert_eq!(own.mine.as_option().map(|status| status.id.clone()), Some(mine.id.clone()));

    // Someone who is not a friend sees nothing at all.
    assert!(free_friends(&ctx, &stranger).await.statuses.is_empty());
    assert!(status_of(&ctx, &stranger, mika).await.is_none());

    // Other domains hear about it, without the note or the area.
    let events = ctx.app.events_by("plans.free.set", mika.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data["status_id"], serde_json::json!(mine.id));
    assert_eq!(events[0].data["window"], serde_json::json!(FreeWindow::FREE_WINDOW_NOW as i32));
    assert!(events[0].data.get("note").is_none() && events[0].data.get("area_lat").is_none());
    // Setting a status notifies nobody by default.
    assert!(ctx.notifications_for(yui.id, "plans.friend_free").await.is_empty());
}

#[tokio::test]
async fn a_status_is_only_seen_by_its_audience() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota", "Hana"]).await;
    let (mika, yui, sota, hana) = (&users[0], &users[1], &users[2], &users[3]);
    // Mika starred Yui: Yui is one of Mika's best friends.
    ctx.star(mika, yui).await;

    let chosen = SetFreeStatusRequest { chosen_user_ids: ids(&[sota]), ..now_for(FreeAudience::FREE_AUDIENCE_CHOSEN) };
    let status = set_status(&ctx, mika, chosen.clone()).await;
    assert_eq!(status.chosen.iter().map(|user| user.id.clone()).collect::<Vec<_>>(), ids(&[sota]));
    assert_eq!(free_friends(&ctx, sota).await.statuses.len(), 1);
    for other in [yui, hana] {
        assert!(free_friends(&ctx, other).await.statuses.is_empty());
        assert!(status_of(&ctx, other, mika).await.is_none());
    }

    // Best friends only: the starred friend sees it, the others do not.
    set_status(&ctx, mika, now_for(FreeAudience::FREE_AUDIENCE_BEST_FRIENDS)).await;
    assert_eq!(free_friends(&ctx, yui).await.statuses.len(), 1);
    assert!(status_of(&ctx, yui, mika).await.is_some());
    for other in [sota, hana] {
        assert!(free_friends(&ctx, other).await.statuses.is_empty());
        assert!(status_of(&ctx, other, mika).await.is_none());
    }

    // The end of a friendship takes effect at once, before any event is handled.
    set_status(&ctx, mika, chosen).await;
    assert!(status_of(&ctx, sota, mika).await.is_some());
    ctx.unfriend(mika, sota).await;
    assert!(status_of(&ctx, sota, mika).await.is_none());
    assert!(free_friends(&ctx, sota).await.statuses.is_empty());
}

#[tokio::test]
async fn windows_are_resolved_by_the_server() {
    let ctx = start().await;
    let mika = ctx.app.user("Mika").await;
    let before = Utc::now();

    let today = set_status(
        &ctx,
        &mika,
        SetFreeStatusRequest {
            window: FreeWindow::FREE_WINDOW_TODAY.into(),
            ..now_for(FreeAudience::FREE_AUDIENCE_FRIENDS)
        },
    )
    .await;
    // Until midnight in the owner's time zone (Tokyo by default): at most 24 hours away, on a full hour in UTC.
    let ends = time_of(&today.ends_at);
    assert!(ends > before && ends <= before + Duration::hours(24));
    assert_eq!(ends.format("%H:%M:%S").to_string(), "15:00:00");

    // A custom window in the future is announced but not "now" yet.
    let (starts, ends) = (before + Duration::days(2), before + Duration::days(2) + Duration::hours(5));
    let custom = SetFreeStatusRequest {
        window: FreeWindow::FREE_WINDOW_CUSTOM.into(),
        starts_at: timestamp(starts).into(),
        ends_at: timestamp(ends).into(),
        ..now_for(FreeAudience::FREE_AUDIENCE_FRIENDS)
    };
    let later = set_status(&ctx, &mika, custom).await;
    assert!(!later.free_now);
    assert_eq!(
        (time_of(&later.starts_at).timestamp(), time_of(&later.ends_at).timestamp()),
        (starts.timestamp(), ends.timestamp())
    );
}

#[tokio::test]
async fn invalid_statuses_are_rejected() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui"]).await;
    let (mika, yui) = (&users[0], &users[1]);
    let stranger = ctx.app.user("Stranger").await;
    let base = || now_for(FreeAudience::FREE_AUDIENCE_FRIENDS);

    let cases = [
        (SetFreeStatusRequest { window: FreeWindow::FREE_WINDOW_UNSPECIFIED.into(), ..base() }, "WINDOW_REQUIRED"),
        (SetFreeStatusRequest { window: FreeWindow::FREE_WINDOW_CUSTOM.into(), ..base() }, "WINDOW_INVALID"),
        (
            SetFreeStatusRequest {
                window: FreeWindow::FREE_WINDOW_CUSTOM.into(),
                ends_at: timestamp(Utc::now() - Duration::hours(1)).into(),
                ..base()
            },
            "WINDOW_INVALID",
        ),
        (SetFreeStatusRequest { note: "あ".repeat(61), ..base() }, "NOTE_TOO_LONG"),
        (
            SetFreeStatusRequest { area: LatLng { lat: 95.0, lng: 0.0, ..Default::default() }.into(), ..base() },
            "AREA_INVALID",
        ),
        (SetFreeStatusRequest { area_name: "あ".repeat(41), ..base() }, "AREA_NAME_TOO_LONG"),
        (
            SetFreeStatusRequest { audience: FreeAudience::FREE_AUDIENCE_UNSPECIFIED.into(), ..base() },
            "AUDIENCE_REQUIRED",
        ),
        (now_for(FreeAudience::FREE_AUDIENCE_CHOSEN), "CHOSEN_REQUIRED"),
        // Choosing only oneself is choosing nobody.
        (
            SetFreeStatusRequest { chosen_user_ids: ids(&[mika]), ..now_for(FreeAudience::FREE_AUDIENCE_CHOSEN) },
            "CHOSEN_REQUIRED",
        ),
        (
            SetFreeStatusRequest {
                chosen_user_ids: (0..51).map(|_| Uuid::now_v7().to_string()).collect(),
                ..now_for(FreeAudience::FREE_AUDIENCE_CHOSEN)
            },
            "TOO_MANY_CHOSEN",
        ),
        (
            SetFreeStatusRequest {
                chosen_user_ids: ids(&[yui, &stranger]),
                ..now_for(FreeAudience::FREE_AUDIENCE_CHOSEN)
            },
            "NOT_FRIENDS",
        ),
        (
            SetFreeStatusRequest {
                chosen_user_ids: vec!["nobody".into()],
                ..now_for(FreeAudience::FREE_AUDIENCE_CHOSEN)
            },
            "INVALID_ID",
        ),
    ];
    for (request, reason) in cases {
        assert_eq!(ctx.refusal(FREE, "SetFreeStatus", mika, &request).await, reason);
    }
    // Nothing was stored along the way.
    assert!(status_of(&ctx, mika, mika).await.is_none());

    // Every RPC of the service needs a signed-in caller.
    for method in [
        "SetFreeStatus",
        "ClearFreeStatus",
        "GetFreeStatus",
        "ListFreeFriends",
        "AnswerFreeStatus",
        "ListFreeAnswers",
        "SetFreeAlert",
        "ListFreeAlerts",
    ] {
        let error = ctx
            .app
            .call::<_, serde_json::Value>(&format!("{FREE}/{method}"), None, &serde_json::json!({}))
            .await
            .unwrap_err();
        assert_eq!((error.code.as_str(), error.reason.as_str()), ("unauthenticated", "SIGN_IN_REQUIRED"), "{method}");
    }
}

#[tokio::test]
async fn asobo_answers_a_status_in_one_tap() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let stranger = ctx.app.user("Stranger").await;
    let status = set_status(
        &ctx,
        mika,
        SetFreeStatusRequest {
            note: "カフェ行こ".into(),
            area_name: "駅前".into(),
            ..now_for(FreeAudience::FREE_AUDIENCE_FRIENDS)
        },
    )
    .await;

    let mut mikas_phone = ctx.app.listen_user(mika.id).await;
    let request =
        AnswerFreeStatusRequest { user_id: mika.id.to_string(), message: "行く行く!".into(), ..Default::default() };
    let answered: AnswerFreeStatusResponse = ctx.call(FREE, "AnswerFreeStatus", yui, &request).await.unwrap();
    assert_eq!(answered.answer.as_option().unwrap().message, "行く行く!");
    let answered_status = answered.status.as_option().unwrap();
    assert_eq!((answered_status.id.clone(), answered_status.answered_by_me), (status.id.clone(), true));
    // What the client needs to open a chat (the friend) or start a plan (a draft for the two).
    assert_eq!(owner_id(answered_status), mika.id.to_string());
    let draft = answered.plan_draft.as_option().unwrap();
    assert_eq!((draft.title.as_str(), draft.invitee_ids.clone()), ("カフェ行こ", ids(&[mika])));
    assert_eq!(draft.place.as_option().unwrap().name, "駅前");
    assert!(matches!(draft.time.as_option().unwrap().start, Some(plan_time::Start::StartAt(_))));

    // The owner is told, once per sender, and sees it live.
    let nudges = ctx.notifications_for(mika.id, "plans.nudge").await;
    assert_eq!(nudges.len(), 1);
    assert_eq!(nudges[0].data["collapse_key"], serde_json::json!(format!("plans.nudge:{}", yui.id)));
    assert_eq!(nudges[0].actor_id, Some(yui.id));
    assert_eq!(nudges[0].data["body"]["ja"], serde_json::json!("行く行く!"));
    match next_push(&mut mikas_phone).await {
        Some(Push::FreeAnswer(answer)) => assert_eq!(answer.user.as_option().unwrap().id, yui.id.to_string()),
        other => panic!("expected the answer to be pushed, got {other:?}"),
    }

    // Answering again replaces the message without notifying again.
    let again = AnswerFreeStatusRequest {
        user_id: mika.id.to_string(),
        message: "やっぱり17時から!".into(),
        ..Default::default()
    };
    let _: AnswerFreeStatusResponse = ctx.call(FREE, "AnswerFreeStatus", yui, &again).await.unwrap();
    let _: AnswerFreeStatusResponse = ctx
        .call(
            FREE,
            "AnswerFreeStatus",
            sota,
            &AnswerFreeStatusRequest { user_id: mika.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(ctx.notifications_for(mika.id, "plans.nudge").await.len(), 2);

    let answers: ListFreeAnswersResponse =
        ctx.call(FREE, "ListFreeAnswers", mika, &ListFreeAnswersRequest::default()).await.unwrap();
    let who: Vec<(String, String)> = answers
        .answers
        .iter()
        .map(|answer| (answer.user.as_option().unwrap().id.clone(), answer.message.clone()))
        .collect();
    assert_eq!(who.len(), 2);
    assert!(
        who.contains(&(yui.id.to_string(), "やっぱり17時から!".to_owned()))
            && who.contains(&(sota.id.to_string(), String::new()))
    );
    assert_eq!(status_of(&ctx, mika, mika).await.unwrap().answer_count, 2);
    // Nobody else reads the answers: Yui has no status, so her list is empty.
    let none: ListFreeAnswersResponse =
        ctx.call(FREE, "ListFreeAnswers", yui, &ListFreeAnswersRequest::default()).await.unwrap();
    assert!(none.answers.is_empty());

    let answer_to = |user: &TestUser| AnswerFreeStatusRequest { user_id: user.id.to_string(), ..Default::default() };
    assert_eq!(ctx.refusal(FREE, "AnswerFreeStatus", mika, &answer_to(mika)).await, "CANNOT_ANSWER_SELF");
    // A friend without a status, and a status one may not see, look the same.
    assert_eq!(ctx.refusal(FREE, "AnswerFreeStatus", mika, &answer_to(yui)).await, "FREE_STATUS_NOT_FOUND");
    assert_eq!(ctx.refusal(FREE, "AnswerFreeStatus", &stranger, &answer_to(mika)).await, "FREE_STATUS_NOT_FOUND");
    let long = AnswerFreeStatusRequest { message: "あ".repeat(101), ..answer_to(mika) };
    assert_eq!(ctx.refusal(FREE, "AnswerFreeStatus", yui, &long).await, "MESSAGE_TOO_LONG");
}

#[tokio::test]
async fn friends_who_asked_are_told_once_per_window() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let stranger = ctx.app.user("Stranger").await;

    let alert = |user: &TestUser, enabled: bool| SetFreeAlertRequest {
        user_id: user.id.to_string(),
        enabled,
        ..Default::default()
    };
    let _: SetFreeAlertResponse = ctx.call(FREE, "SetFreeAlert", yui, &alert(mika, true)).await.unwrap();
    // Asking twice is harmless.
    let _: SetFreeAlertResponse = ctx.call(FREE, "SetFreeAlert", yui, &alert(mika, true)).await.unwrap();
    assert_eq!(ctx.refusal(FREE, "SetFreeAlert", &stranger, &alert(mika, true)).await, "NOT_FRIENDS");
    let alerts: ListFreeAlertsResponse =
        ctx.call(FREE, "ListFreeAlerts", yui, &ListFreeAlertsRequest::default()).await.unwrap();
    assert_eq!(alerts.users.iter().map(|user| user.id.clone()).collect::<Vec<_>>(), ids(&[mika]));

    let told = || async { ctx.notifications_for(yui.id, "plans.friend_free").await };
    set_status(
        &ctx,
        mika,
        SetFreeStatusRequest {
            window: FreeWindow::FREE_WINDOW_TODAY.into(),
            note: "ひま〜".into(),
            ..now_for(FreeAudience::FREE_AUDIENCE_FRIENDS)
        },
    )
    .await;
    let first = told().await;
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].data["title"]["ja"], serde_json::json!("Mikaさんがひましてるよ!"));
    assert_eq!(first[0].data["body"]["ja"], serde_json::json!("今日ひま!「ひま〜」"));
    assert_eq!(first[0].actor_id, Some(mika.id));
    // Only those who asked are told.
    assert!(ctx.notifications_for(sota.id, "plans.friend_free").await.is_empty());

    // Changing the status within the same window does not announce it again.
    set_status(&ctx, mika, now_for(FreeAudience::FREE_AUDIENCE_FRIENDS)).await;
    assert_eq!(told().await.len(), 1);

    // Another window does.
    let later = |days: i64, audience: FreeAudience| SetFreeStatusRequest {
        window: FreeWindow::FREE_WINDOW_CUSTOM.into(),
        starts_at: timestamp(Utc::now() + Duration::days(days)).into(),
        ends_at: timestamp(Utc::now() + Duration::days(days) + Duration::hours(4)).into(),
        ..now_for(audience)
    };
    // ...but not right away: however many windows Mika sets, Yui hears from
    // her at most once an hour.
    set_status(&ctx, mika, later(3, FreeAudience::FREE_AUDIENCE_FRIENDS)).await;
    assert_eq!(told().await.len(), 1);
    ctx.sql("UPDATE free_alerts SET notified_at = now() - interval '61 minutes' WHERE user_id = $1", yui.id).await;
    set_status(&ctx, mika, later(12, FreeAudience::FREE_AUDIENCE_FRIENDS)).await;
    assert_eq!(told().await.len(), 2);
    ctx.sql("UPDATE free_alerts SET notified_at = now() - interval '61 minutes' WHERE user_id = $1", yui.id).await;

    // A status the friend may not see is not announced to them.
    set_status(
        &ctx,
        mika,
        SetFreeStatusRequest { chosen_user_ids: ids(&[sota]), ..later(6, FreeAudience::FREE_AUDIENCE_CHOSEN) },
    )
    .await;
    assert_eq!(told().await.len(), 2);

    // Opting out again.
    let _: SetFreeAlertResponse = ctx.call(FREE, "SetFreeAlert", yui, &alert(mika, false)).await.unwrap();
    let alerts: ListFreeAlertsResponse =
        ctx.call(FREE, "ListFreeAlerts", yui, &ListFreeAlertsRequest::default()).await.unwrap();
    assert!(alerts.users.is_empty());
    set_status(&ctx, mika, later(9, FreeAudience::FREE_AUDIENCE_FRIENDS)).await;
    assert_eq!(told().await.len(), 2);
}

#[tokio::test]
async fn a_status_ends_when_cleared_or_when_its_window_is_over() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui"]).await;
    let (mika, yui) = (&users[0], &users[1]);

    set_status(&ctx, mika, now_for(FreeAudience::FREE_AUDIENCE_FRIENDS)).await;
    let _: ClearFreeStatusResponse =
        ctx.call(FREE, "ClearFreeStatus", mika, &ClearFreeStatusRequest::default()).await.unwrap();
    assert!(free_friends(&ctx, yui).await.statuses.is_empty());
    assert!(status_of(&ctx, mika, mika).await.is_none());
    // Clearing nothing is fine.
    let _: ClearFreeStatusResponse =
        ctx.call(FREE, "ClearFreeStatus", mika, &ClearFreeStatusRequest::default()).await.unwrap();

    // A window that is over is gone for everyone, even before the worker removes the row...
    set_status(&ctx, mika, now_for(FreeAudience::FREE_AUDIENCE_FRIENDS)).await;
    ctx.sql("UPDATE free_statuses SET starts_at = now() - interval '4 hours', ends_at = now() - interval '1 minute' WHERE user_id = $1", mika.id).await;
    assert!(free_friends(&ctx, yui).await.statuses.is_empty());
    assert!(status_of(&ctx, mika, mika).await.is_none());
    // ...which it does on its next pass.
    assert_eq!(ctx.work().await.statuses_expired, 1);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM free_statuses WHERE user_id = $1")
        .bind(mika.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn friends_lists_update_live() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let mut yuis_phone = ctx.app.listen_user(yui.id).await;
    let mut sotas_phone = ctx.app.listen_user(sota.id).await;

    // Set for all friends: both get the status.
    let status = set_status(&ctx, mika, now_for(FreeAudience::FREE_AUDIENCE_FRIENDS)).await;
    for phone in [&mut yuis_phone, &mut sotas_phone] {
        match next_push(phone).await {
            Some(Push::FreeStatusChanged(change)) => {
                assert_eq!(change.user_id, mika.id.to_string());
                let pushed = change.status.as_option().expect("the new status");
                // What is pushed is the friends' view: no audience, no chosen list.
                assert_eq!((pushed.id.clone(), pushed.audience.to_i32()), (status.id.clone(), 0));
            }
            other => panic!("expected the status to be pushed, got {other:?}"),
        }
    }

    // Narrowed to Sota: he gets the new one, Yui sees the old one end.
    set_status(
        &ctx,
        mika,
        SetFreeStatusRequest { chosen_user_ids: ids(&[sota]), ..now_for(FreeAudience::FREE_AUDIENCE_CHOSEN) },
    )
    .await;
    assert!(
        matches!(next_push(&mut sotas_phone).await, Some(Push::FreeStatusChanged(change)) if change.status.is_set())
    );
    assert!(
        matches!(next_push(&mut yuis_phone).await, Some(Push::FreeStatusChanged(change)) if change.status.is_unset())
    );

    // Cleared: only the people who saw it are told.
    let _: ClearFreeStatusResponse =
        ctx.call(FREE, "ClearFreeStatus", mika, &ClearFreeStatusRequest::default()).await.unwrap();
    assert!(
        matches!(next_push(&mut sotas_phone).await, Some(Push::FreeStatusChanged(change)) if change.status.is_unset())
    );
    assert!(pushes(&mut yuis_phone, QUIET).await.is_empty());
}

#[tokio::test]
async fn the_profile_shows_the_status_to_its_audience() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let stranger = ctx.app.user("Stranger").await;
    assert!(ctx.cards(yui, mika).await.is_empty());

    let status = set_status(
        &ctx,
        mika,
        SetFreeStatusRequest { note: "だれかあそぼ".into(), ..now_for(FreeAudience::FREE_AUDIENCE_FRIENDS) },
    )
    .await;
    for viewer in [yui, mika] {
        let cards = ctx.cards(viewer, mika).await;
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].kind, "plans.free");
        assert_eq!(cards[0].text.as_option().unwrap().ja, "今ひま!「だれかあそぼ」");
        assert_eq!(cards[0].text.as_option().unwrap().en, "Free now: “だれかあそぼ”");
        assert_eq!((cards[0].path.as_str(), cards[0].data["status_id"].as_str()), ("/plans/free", status.id.as_str()));
    }
    // Not for strangers, and not on other people's profiles.
    assert!(ctx.cards(&stranger, mika).await.is_empty());
    assert!(ctx.cards(mika, yui).await.is_empty());

    // Only for the audience.
    set_status(
        &ctx,
        mika,
        SetFreeStatusRequest { chosen_user_ids: ids(&[sota]), ..now_for(FreeAudience::FREE_AUDIENCE_CHOSEN) },
    )
    .await;
    assert!(ctx.cards(yui, mika).await.is_empty());
    assert_eq!(ctx.cards(sota, mika).await.len(), 1);
}

#[tokio::test]
async fn lists_come_in_pages() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let one = |cursor: &str| PageRequest { limit: 1, cursor: cursor.to_owned(), ..Default::default() };

    // Two friends are free, both answered Mika's status, and Mika asked to be told about both.
    set_status(&ctx, mika, now_for(FreeAudience::FREE_AUDIENCE_FRIENDS)).await;
    for friend in [yui, sota] {
        set_status(&ctx, friend, now_for(FreeAudience::FREE_AUDIENCE_FRIENDS)).await;
        let answer = AnswerFreeStatusRequest { user_id: mika.id.to_string(), ..Default::default() };
        let _: AnswerFreeStatusResponse = ctx.call(FREE, "AnswerFreeStatus", friend, &answer).await.unwrap();
        let alert = SetFreeAlertRequest { user_id: friend.id.to_string(), enabled: true, ..Default::default() };
        let _: SetFreeAlertResponse = ctx.call(FREE, "SetFreeAlert", mika, &alert).await.unwrap();
    }

    // Who is free: the status that started first comes first; "mine" rides on the first page only.
    let first: ListFreeFriendsResponse = ctx
        .call(FREE, "ListFreeFriends", mika, &ListFreeFriendsRequest { page: one("").into(), ..Default::default() })
        .await
        .unwrap();
    let info = first.page.as_option().unwrap();
    assert!(info.has_more && first.mine.is_set());
    assert_eq!(first.statuses.iter().map(owner_id).collect::<Vec<_>>(), ids(&[yui]));
    let second: ListFreeFriendsResponse = ctx
        .call(
            FREE,
            "ListFreeFriends",
            mika,
            &ListFreeFriendsRequest { page: one(&info.next_cursor).into(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(!second.page.as_option().unwrap().has_more && second.mine.is_unset());
    assert_eq!(second.statuses.iter().map(owner_id).collect::<Vec<_>>(), ids(&[sota]));

    // Answers: newest first.
    let first: ListFreeAnswersResponse = ctx
        .call(FREE, "ListFreeAnswers", mika, &ListFreeAnswersRequest { page: one("").into(), ..Default::default() })
        .await
        .unwrap();
    let info = first.page.as_option().unwrap();
    assert!(info.has_more);
    assert_eq!(first.answers[0].user.as_option().unwrap().id, sota.id.to_string());
    let second: ListFreeAnswersResponse = ctx
        .call(
            FREE,
            "ListFreeAnswers",
            mika,
            &ListFreeAnswersRequest { page: one(&info.next_cursor).into(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(!second.page.as_option().unwrap().has_more);
    assert_eq!(
        second.answers.iter().map(|answer| answer.user.as_option().unwrap().id.clone()).collect::<Vec<_>>(),
        ids(&[yui])
    );

    // Alerts: most recently added first.
    let first: ListFreeAlertsResponse = ctx
        .call(FREE, "ListFreeAlerts", mika, &ListFreeAlertsRequest { page: one("").into(), ..Default::default() })
        .await
        .unwrap();
    let info = first.page.as_option().unwrap();
    assert!(info.has_more);
    assert_eq!(first.users.iter().map(|user| user.id.clone()).collect::<Vec<_>>(), ids(&[sota]));
    let second: ListFreeAlertsResponse = ctx
        .call(
            FREE,
            "ListFreeAlerts",
            mika,
            &ListFreeAlertsRequest { page: one(&info.next_cursor).into(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(!second.page.as_option().unwrap().has_more);
    assert_eq!(second.users.iter().map(|user| user.id.clone()).collect::<Vec<_>>(), ids(&[yui]));
}
