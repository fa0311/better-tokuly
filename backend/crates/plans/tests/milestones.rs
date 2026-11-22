//! Milestones of "days since" counters and countdowns reaching their day:
//! what the worker announces, to whom, and on which day.
//!
//! These tests run with the announcement hour set to midnight, so that
//! "today" can be announced whenever they run; that is why they have a test
//! binary (and so a database) of their own. Time is moved by editing the
//! `*_due_at` columns and the dates (`Ctx::make_due`).
mod common;

use chrono::{Duration, NaiveDate, Utc};
use common::*;
use tokuly_plans::PlansConfig;
use tokuly_proto::plans::*;
use uuid::Uuid;

fn id_of(text: &str) -> Uuid {
    text.parse().expect("an id")
}

/// Today on the calendar of `zone`.
fn today_in(zone: chrono_tz::Tz) -> NaiveDate {
    Utc::now().with_timezone(&zone).date_naive()
}

async fn count(ctx: &Ctx, query: &'static str, id: Uuid) -> i64 {
    sqlx::query_scalar(query).bind(id).fetch_one(&ctx.app.core.db).await.expect("count")
}

/// Milestones and countdowns are announced from this local hour on; hour 0
/// makes "today" announceable whenever the tests run.
fn all_day() -> PlansConfig {
    PlansConfig { morning_hour: 0, ..PlansConfig::default() }
}

#[tokio::test]
async fn milestones_are_announced_to_each_on_their_own_day() {
    let ctx = start_with(all_day()).await;
    let users = ctx.group(&["Mika", "Yui"]).await;
    let (mika, yui) = (&users[0], &users[1]);
    // The two live a day apart: when it is the 10th for Mika it is still the 9th for Yui.
    ctx.set_time_zone(mika, "Pacific/Kiritimati").await;
    ctx.set_time_zone(yui, "Pacific/Honolulu").await;

    // Today is day 100 for Mika, day 99 for Yui.
    let start = today_in(chrono_tz::Pacific::Kiritimati) - Duration::days(99);
    let request = StartFriendshipCounterRequest {
        user_id: yui.id.to_string(),
        start_date: start.format("%Y-%m-%d").to_string(),
        ..Default::default()
    };
    let counter: StartFriendshipCounterResponse =
        ctx.call(ANNIVERSARIES, "StartFriendshipCounter", mika, &request).await.unwrap();
    let counter = counter.anniversary.as_option().cloned().unwrap();
    assert_eq!(counter.days, 100);
    let counter_id = id_of(&counter.id);
    // The next notice is scheduled, at the latest for Yui's morning within a day.
    assert_eq!(count(&ctx, "SELECT count(*) FROM plan_anniversaries WHERE id = $1 AND notice_due_at > now() AND notice_due_at <= now() + interval '24 hours'", counter_id).await, 1);

    assert_eq!(
        ctx.make_due(
            "UPDATE plan_anniversaries SET notice_due_at = now() - interval '1 minute' WHERE id = $1",
            &[counter_id]
        )
        .await
        .counters_checked,
        1
    );
    let told = ctx.notifications_for(mika.id, "plans.milestone").await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].data["title"]["ja"], serde_json::json!("今日は100日記念日!"));
    assert_eq!(told[0].data["body"]["ja"], serde_json::json!("Yuiさんと友達になって100日記念日! 🎉"));
    assert_eq!(
        (told[0].actor_id, told[0].data["data"]["milestone"].clone()),
        (Some(yui.id), serde_json::json!("d100"))
    );
    // Not yet for Yui: it is not her day.
    assert!(ctx.notifications_for(yui.id, "plans.milestone").await.is_empty());
    assert_eq!(
        count(&ctx, "SELECT count(*) FROM plan_anniversaries WHERE id = $1 AND notice_due_at > now()", counter_id)
            .await,
        1
    );

    // Nobody hears about a milestone twice.
    ctx.make_due(
        "UPDATE plan_anniversaries SET notice_due_at = now() - interval '1 minute' WHERE id = $1",
        &[counter_id],
    )
    .await;
    assert_eq!(ctx.notifications_for(mika.id, "plans.milestone").await.len(), 1);

    // A day later (the counter is moved instead of the clock) it is Yui's turn, and only hers.
    ctx.make_due("UPDATE plan_anniversaries SET start_date = start_date - 1, notice_due_at = now() - interval '1 minute' WHERE id = $1", &[counter_id]).await;
    let told = ctx.notifications_for(yui.id, "plans.milestone").await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].data["body"]["en"], serde_json::json!("100 days of friendship with Mika 🎉"));
    assert_eq!(ctx.notifications_for(mika.id, "plans.milestone").await.len(), 1);

    // A couple's milestone reads differently.
    let pair = ctx.group(&["Sota", "Hana"]).await;
    let (sota, hana) = (&pair[0], &pair[1]);
    let start = today_in(chrono_tz::Asia::Tokyo) - Duration::days(99);
    let invited: InviteCoupleResponse = ctx
        .call(
            ANNIVERSARIES,
            "InviteCouple",
            sota,
            &InviteCoupleRequest {
                user_id: hana.id.to_string(),
                start_date: start.format("%Y-%m-%d").to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let couple = invited.anniversary.as_option().cloned().unwrap();
    // An invitation announces nothing.
    assert_eq!(
        count(
            &ctx,
            "SELECT count(*) FROM plan_anniversaries WHERE id = $1 AND notice_due_at IS NULL",
            id_of(&couple.id)
        )
        .await,
        1
    );
    let _: RespondCoupleInviteResponse = ctx
        .call(
            ANNIVERSARIES,
            "RespondCoupleInvite",
            hana,
            &RespondCoupleInviteRequest { anniversary_id: couple.id.clone(), accept: true, ..Default::default() },
        )
        .await
        .unwrap();
    ctx.make_due(
        "UPDATE plan_anniversaries SET notice_due_at = now() - interval '1 minute' WHERE id = $1",
        &[id_of(&couple.id)],
    )
    .await;
    for (person, partner) in [(sota, "Hana"), (hana, "Sota")] {
        let told = ctx.notifications_for(person.id, "plans.milestone").await;
        assert_eq!(told.len(), 1);
        assert_eq!(
            told[0].data["body"]["ja"],
            serde_json::json!(format!("{partner}さんと付き合って100日記念日! おめでとう 🎉"))
        );
    }

    // Changing the start date makes the milestones fall anew.
    let moved = UpdateAnniversaryRequest {
        anniversary_id: couple.id.clone(),
        start_date: Some((start - Duration::days(100)).format("%Y-%m-%d").to_string()),
        ..Default::default()
    };
    let _: UpdateAnniversaryResponse = ctx.call(ANNIVERSARIES, "UpdateAnniversary", sota, &moved).await.unwrap();
    assert_eq!(
        count(&ctx, "SELECT count(*) FROM plan_anniversary_notices WHERE anniversary_id = $1", id_of(&couple.id)).await,
        0
    );
    ctx.make_due(
        "UPDATE plan_anniversaries SET notice_due_at = now() - interval '1 minute' WHERE id = $1",
        &[id_of(&couple.id)],
    )
    .await;
    let told = ctx.notifications_for(sota.id, "plans.milestone").await;
    assert_eq!(told.len(), 2);
    assert_eq!(told[1].data["title"]["ja"], serde_json::json!("今日は200日記念日!"));
}

#[tokio::test]
async fn a_countdown_is_announced_on_its_day() {
    let ctx = start_with(all_day()).await;
    let mika = ctx.app.user("Mika").await;
    let tomorrow = (today_in(chrono_tz::Asia::Tokyo) + Duration::days(1)).format("%Y-%m-%d").to_string();
    let create = |title: &str, yearly: bool| CreateCountdownRequest {
        title: title.to_owned(),
        emoji: "🎪".into(),
        date: tomorrow.clone(),
        repeats_yearly: yearly,
        ..Default::default()
    };
    let festival: CreateCountdownResponse =
        ctx.call(ANNIVERSARIES, "CreateCountdown", &mika, &create("文化祭", false)).await.unwrap();
    let festival = id_of(&festival.countdown.as_option().unwrap().id);
    let birthday: CreateCountdownResponse =
        ctx.call(ANNIVERSARIES, "CreateCountdown", &mika, &create("推しの誕生日", true)).await.unwrap();
    let birthday = id_of(&birthday.countdown.as_option().unwrap().id);

    // Scheduled for the day itself; nothing before.
    assert_eq!(
        count(&ctx, "SELECT count(*) FROM plan_countdowns WHERE id = $1 AND notify_due_at > now()", festival).await,
        1
    );
    ctx.work().await;
    assert!(ctx.notifications_for(mika.id, "plans.countdown").await.is_empty());

    // The day arrives (the countdowns are moved instead of the clock).
    let moved = ctx.make_due("UPDATE plan_countdowns SET target_date = target_date - 1, notify_due_at = now() - interval '1 minute' WHERE id = $1", &[festival, birthday]).await;
    assert_eq!(moved.countdowns_reached, 2);
    let told = ctx.notifications_for(mika.id, "plans.countdown").await;
    assert_eq!(told.len(), 2);
    let titles: Vec<&str> = told.iter().map(|notice| notice.data["title"]["ja"].as_str().unwrap()).collect();
    assert!(titles.contains(&"今日は「🎪 文化祭」!") && titles.contains(&"今日は「🎪 推しの誕生日」!"));
    // A one-off is done; a yearly one waits for next year.
    assert_eq!(
        count(&ctx, "SELECT count(*) FROM plan_countdowns WHERE id = $1 AND notify_due_at IS NULL", festival).await,
        1
    );
    assert_eq!(
        count(
            &ctx,
            "SELECT count(*) FROM plan_countdowns WHERE id = $1 AND notify_due_at > now() + interval '300 days'",
            birthday
        )
        .await,
        1
    );
    ctx.work().await;
    assert_eq!(ctx.notifications_for(mika.id, "plans.countdown").await.len(), 2);

    // A day that passed while the worker was down is not announced late.
    ctx.make_due("UPDATE plan_countdowns SET target_date = target_date - 3, notify_due_at = now() - interval '3 days' WHERE id = $1", &[festival]).await;
    assert_eq!(ctx.notifications_for(mika.id, "plans.countdown").await.len(), 2);
}
