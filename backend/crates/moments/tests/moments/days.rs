//! Drawing days ahead and sending each region's prompt exactly once.
use chrono::{DateTime, Utc};
use tokuly_moments::{KIND_PROMPT, Sent};
use tokuly_proto::moments::{
    MomentsEvent, UpdateMomentSettingsRequest, UpdateMomentSettingsResponse, moments_event::Event as Push,
};
use uuid::Uuid;

use crate::common::*;

#[tokio::test]
async fn a_prompt_reaches_everyone_of_its_region_once() {
    let ctx = start().await;
    let (a, b, c) = (ctx.app.user("あや").await, ctx.app.user("べに").await, ctx.app.user("ちか").await);
    let quiet = ctx.app.user("しずか").await;
    let elsewhere = ctx.app.user("よそ").await;
    let region = ctx.region(&[&a, &b, &c, &quiet]).await;
    ctx.region(&[&elsewhere]).await;
    let _: UpdateMomentSettingsResponse = ctx
        .call(
            "UpdateMomentSettings",
            &quiet,
            &UpdateMomentSettingsRequest { prompts: Some(false), ..Default::default() },
        )
        .await
        .unwrap();
    let mut pushes = ctx.app.listen_user(a.id).await;

    let day = ctx.due_day(&region).await;
    // The job may run twice, or in two processes at once.
    let runs = futures::future::join_all((0..4).map(|_| ctx.moments.send_prompt(day))).await;
    let told: usize = runs
        .into_iter()
        .map(|run| match run.unwrap() {
            Sent::Told(count) => count,
            other => panic!("{other:?}"),
        })
        .sum();
    assert_eq!(told, 3);
    assert_eq!(ctx.moments.send_prompt(day).await.unwrap(), Sent::Told(0));

    for user in [&a, &b, &c] {
        assert_eq!(ctx.notifications(user, KIND_PROMPT).await.len(), 1, "{}", user.handle);
    }
    assert!(ctx.notifications(&quiet, KIND_PROMPT).await.is_empty(), "prompts turned off");
    assert!(ctx.notifications(&elsewhere, KIND_PROMPT).await.is_empty(), "another region");
    let sent: Vec<_> = ctx
        .app
        .events("moments.prompt.sent")
        .await
        .into_iter()
        .filter(|event| event.data["day_id"] == day.to_string())
        .collect();
    assert_eq!(sent.len(), 1);

    let Some(Push::Prompt(shown)) = pushes.next_event::<MomentsEvent>(SOON).await.and_then(|e| e.event) else {
        panic!("no prompt pushed")
    };
    assert_eq!(shown.region, region);
    let today = ctx.today(&a).await;
    assert_eq!(today.day.as_option().unwrap().date, shown.date);
    assert!(!today.posted && !today.unlocked);
}

#[tokio::test]
async fn a_prompt_too_late_or_overtaken_never_starts_its_day() {
    let ctx = start().await;
    let a = ctx.app.user("あや").await;
    let region = ctx.region(&[&a]).await;

    // Workers were down for two hours: nobody is woken for it now.
    let stale = ctx.due_day(&region).await;
    ctx.age_day(stale, 2 * 3600).await;
    sqlx::query("UPDATE moment_days SET announced_at = NULL WHERE id = $1")
        .bind(stale)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(ctx.moments.send_prompt(stale).await.unwrap(), Sent::Skipped);
    assert!(ctx.today(&a).await.day.as_option().is_none());
    assert!(ctx.notifications(&a, KIND_PROMPT).await.is_empty());

    // A later date went out first: the earlier one is skipped.
    let later = ctx.prompt(&region).await;
    let earlier: Uuid = sqlx::query_scalar(
        "INSERT INTO moment_days (region, date, prompt_at) VALUES ($1, DATE '2025-01-01', now()) RETURNING id",
    )
    .bind(&region)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(ctx.moments.send_prompt(earlier).await.unwrap(), Sent::Skipped);
    let current: Uuid = sqlx::query_scalar("SELECT current_day_id FROM moment_regions WHERE region = $1")
        .bind(&region)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(current, later);

    // A day whose time has not come does nothing.
    let ahead: Uuid = sqlx::query_scalar(
        "INSERT INTO moment_days (region, date, prompt_at) VALUES ($1, DATE '2030-01-01', now() + interval '1 hour') RETURNING id",
    )
    .bind(&region)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(ctx.moments.send_prompt(ahead).await.unwrap(), Sent::Nothing);
    assert_eq!(ctx.moments.send_prompt(Uuid::now_v7()).await.unwrap(), Sent::Nothing);
}

#[tokio::test]
async fn the_planner_draws_today_and_tomorrow_once() {
    let ctx = start().await;
    let a = ctx.app.user("あや").await;
    let region = ctx.region(&[&a]).await;
    ctx.moments.refresh_regions().await.unwrap();
    let tz: chrono_tz::Tz = region.parse().unwrap();

    // 10:00 local on a day of its own: today keeps what is left of the window.
    let now: DateTime<Utc> =
        chrono::TimeZone::with_ymd_and_hms(&tz, 2031, 5, 10, 10, 0, 0).unwrap().with_timezone(&Utc);
    ctx.moments.draw_days(now).await.unwrap();
    let drawn = |ctx: &Ctx| {
        let region = region.clone();
        let db = ctx.app.core.db.clone();
        async move {
            sqlx::query_as::<_, (chrono::NaiveDate, Option<DateTime<Utc>>)>(
                "SELECT date, prompt_at FROM moment_days WHERE region = $1 AND date >= DATE '2031-05-10' ORDER BY date",
            )
            .bind(region)
            .fetch_all(&db)
            .await
            .unwrap()
        }
    };
    let first = drawn(&ctx).await;
    assert_eq!(first.iter().map(|(date, _)| date.to_string()).collect::<Vec<_>>(), ["2031-05-10", "2031-05-11"]);
    let today = first[0].1.expect("a prompt today").with_timezone(&tz);
    assert!(today >= now.with_timezone(&tz) && chrono::Timelike::hour(&today) < 22, "{today}");
    let tomorrow = first[1].1.expect("a prompt tomorrow").with_timezone(&tz);
    assert!((9..22).contains(&chrono::Timelike::hour(&tomorrow)), "{tomorrow}");

    // The first draw stays: nobody can redraw a day until it suits them.
    ctx.moments.draw_days(now).await.unwrap();
    assert_eq!(drawn(&ctx).await, first);

    // A region first seen after its window gets no prompt that day.
    let late = ctx.app.user("おそ").await;
    let late_region = ctx.region(&[&late]).await;
    ctx.moments.refresh_regions().await.unwrap();
    let ltz: chrono_tz::Tz = late_region.parse().unwrap();
    let night = chrono::TimeZone::with_ymd_and_hms(&ltz, 2031, 6, 1, 23, 0, 0).unwrap().with_timezone(&Utc);
    ctx.moments.draw_days(night).await.unwrap();
    let prompt: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT prompt_at FROM moment_days WHERE region = $1 AND date = DATE '2031-06-01'")
            .bind(&late_region)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(prompt, None);
}

#[tokio::test]
async fn too_young_and_suspended_accounts_are_not_prompted() {
    let ctx = start().await;
    let (child, banned, ok) =
        (ctx.app.user("こども").await, ctx.app.user("ていし").await, ctx.app.user("ふつう").await);
    let region = ctx.region(&[&child, &banned, &ok]).await;
    sqlx::query("UPDATE users SET birthdate = current_date - interval '10 years' WHERE id = $1")
        .bind(child.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET banned_at = now() WHERE id = $1")
        .bind(banned.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.prompt(&region).await;
    assert_eq!(ctx.notifications(&ok, KIND_PROMPT).await.len(), 1);
    assert!(ctx.notifications(&child, KIND_PROMPT).await.is_empty());
    assert!(ctx.notifications(&banned, KIND_PROMPT).await.is_empty());
}
