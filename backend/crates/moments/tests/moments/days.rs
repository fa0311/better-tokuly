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
async fn the_planner_draws_today_and_tomorrow_of_every_band_once() {
    let ctx = start().await;
    let db = ctx.app.core.db.clone();
    ctx.moments.refresh_regions().await.unwrap();
    let band_of = |zone: &'static str| {
        let db = db.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT region FROM moment_zones WHERE time_zone = $1")
                .bind(zone)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    assert_eq!(band_of("Asia/Tokyo").await, "utc+7..+11");
    assert_eq!(band_of("Asia/Seoul").await, "utc+7..+11", "Tokyo and Seoul are prompted together");
    assert_eq!(band_of("Europe/London").await, "utc-1..+3");

    // 15:00 in Tokyo, on days far ahead that no other test draws.
    let at = |rfc3339: &str| DateTime::parse_from_rfc3339(rfc3339).unwrap().with_timezone(&Utc);
    let now = at("2031-05-10T15:00:00+09:00");
    ctx.moments.draw_days(now).await.unwrap();
    let drawn = |from: &'static str, until: &'static str| {
        let db = db.clone();
        async move {
            sqlx::query_as::<_, (String, chrono::NaiveDate, Option<DateTime<Utc>>)>(
                "SELECT region, date, prompt_at FROM moment_days
                 WHERE region LIKE 'utc%' AND date BETWEEN $1::date AND $2::date ORDER BY region, date",
            )
            .bind(from)
            .bind(until)
            .fetch_all(&db)
            .await
            .unwrap()
        }
    };
    let first = drawn("2031-05-09", "2031-05-11").await;
    assert_eq!(first.len(), 12, "today and tomorrow of six bands: {first:?}");
    let tokyo: Vec<_> = first.iter().filter(|(region, _, _)| region == "utc+7..+11").collect();
    assert_eq!(tokyo.iter().map(|(_, date, _)| date.to_string()).collect::<Vec<_>>(), ["2031-05-10", "2031-05-11"]);
    // The window shared from Bangkok to Sydney is 11:00 to 20:00 in Tokyo;
    // today keeps what is left of it.
    let today = tokyo[0].2.expect("a prompt today");
    assert!(today >= now && today < at("2031-05-10T20:00:00+09:00"), "{today}");
    let tomorrow = tokyo[1].2.expect("a prompt tomorrow");
    assert!(tomorrow >= at("2031-05-11T11:00:00+09:00") && tomorrow < at("2031-05-11T20:00:00+09:00"), "{tomorrow}");

    // The first draw stays: nobody can redraw a day until it suits them.
    ctx.moments.draw_days(now).await.unwrap();
    assert_eq!(drawn("2031-05-09", "2031-05-11").await, first);

    // Drawn after its window, a day has no prompt.
    ctx.moments.draw_days(at("2031-06-01T21:00:00+09:00")).await.unwrap();
    let late: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT prompt_at FROM moment_days WHERE region = 'utc+7..+11' AND date = DATE '2031-06-01'",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(late, None);
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

#[tokio::test]
async fn a_prompt_run_that_died_half_way_is_picked_up_where_it_stopped() {
    let ctx = start().await;
    let mut people = Vec::new();
    for name in ["あや", "べに", "ちか"] {
        people.push(ctx.app.user(name).await);
    }
    people.sort_by_key(|user| user.id);
    let region = ctx.region(&people.iter().collect::<Vec<_>>()).await;
    let day = ctx.due_day(&region).await;

    // What a run leaves when it dies after announcing the day and telling
    // the first page (here: the first account, in ID order).
    let db = &ctx.app.core.db;
    sqlx::query("UPDATE moment_days SET announced_at = now(), told_through = $2 WHERE id = $1")
        .bind(day)
        .bind(people[0].id)
        .execute(db)
        .await
        .unwrap();
    sqlx::query("UPDATE moment_regions SET current_day_id = $1 WHERE region = $2")
        .bind(day)
        .bind(&region)
        .execute(db)
        .await
        .unwrap();

    assert_eq!(ctx.moments.send_prompt(day).await.unwrap(), Sent::Told(2));
    assert_eq!(ctx.moments.send_prompt(day).await.unwrap(), Sent::Told(0));
    assert!(ctx.notifications(&people[0], KIND_PROMPT).await.is_empty(), "told by the run that died");
    for user in &people[1..] {
        assert_eq!(ctx.notifications(user, KIND_PROMPT).await.len(), 1, "{}", user.handle);
    }
    let announced =
        ctx.app.events("moments.prompt.sent").await.into_iter().filter(|e| e.data["day_id"] == day.to_string()).count();
    assert_eq!(announced, 0, "the day was announced by the run that died, not again");
}

#[tokio::test]
async fn a_new_time_zone_counts_from_the_next_day() {
    let ctx = start().await;
    let (late, punctual) = (ctx.app.user("おそ").await, ctx.app.user("まじめ").await);
    let home = ctx.region(&[&late, &punctual]).await;
    let away = ctx.region(&[]).await;
    let first = ctx.prompt(&home).await;
    ctx.post(&punctual).await;
    // `late` missed the prompt by three hours.
    ctx.age_day(first, 3 * 3600).await;

    // Both move to a region whose prompt of the same date goes out now.
    ctx.move_to(&away, &[&late, &punctual]).await;
    ctx.prompt(&away).await;
    for user in [&late, &punctual] {
        assert_eq!(ctx.notifications(user, KIND_PROMPT).await.len(), 1, "one prompt a day: {}", user.handle);
    }
    let today = ctx.today(&late).await;
    assert_eq!(today.day.as_option().unwrap().region, home, "the day is still the one the first prompt brought");
    let moment = ctx.post(&late).await;
    assert!(moment.late_seconds > 3 * 3600 - 200, "not on time in the new zone: {}", moment.late_seconds);
    assert_eq!(moment.day.as_option().unwrap().region, home);
    for user in [&late, &punctual] {
        assert_eq!(ctx.try_post(user).await.unwrap_err().reason, "ALREADY_POSTED", "{}", user.handle);
    }

    // From the next day on, the new zone's prompts are theirs.
    ctx.prompt(&away).await;
    for user in [&late, &punctual] {
        assert_eq!(ctx.notifications(user, KIND_PROMPT).await.len(), 2, "{}", user.handle);
        let moment = ctx.post(user).await;
        assert_eq!((moment.late_seconds, moment.day.as_option().unwrap().region.as_str()), (0, away.as_str()));
    }
}

#[tokio::test]
async fn two_prompts_never_reach_one_person_in_a_day() {
    let ctx = start().await;
    let a = ctx.app.user("あや").await;
    let (east, west) = (ctx.region(&[&a]).await, ctx.region(&[]).await);
    // Back and forth between two regions, each prompted for each date: the
    // prompt of a date reaches `a` once, wherever it is when it goes out.
    for _ in 0..2 {
        ctx.prompt(&east).await;
        ctx.move_to(&west, &[&a]).await;
        ctx.prompt(&west).await;
        ctx.move_to(&east, &[&a]).await;
    }
    assert_eq!(ctx.notifications(&a, KIND_PROMPT).await.len(), 2);
    let days: Vec<(String, chrono::NaiveDate)> = sqlx::query_as(
        "SELECT d.region, d.date FROM moment_days d JOIN moment_people p ON p.day_id = d.id WHERE p.user_id = $1",
    )
    .bind(a.id)
    .fetch_all(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(days.len(), 1);
    assert_eq!(days[0].0, east, "the second date reached `a` in the east first");
}

#[tokio::test]
async fn posting_while_the_prompt_is_on_its_way_fixes_the_day() {
    let ctx = start().await;
    let mut people = Vec::new();
    for name in ["あや", "べに", "ちか", "でん", "えみ", "ふみ"] {
        people.push(ctx.app.user(name).await);
    }
    let region = ctx.region(&people.iter().collect::<Vec<_>>()).await;
    let early = &people[0];
    // The day went out; its fan-out has not reached anyone yet.
    let day = ctx.due_day(&region).await;
    let db = &ctx.app.core.db;
    sqlx::query("UPDATE moment_days SET announced_at = now() WHERE id = $1").bind(day).execute(db).await.unwrap();
    sqlx::query("UPDATE moment_regions SET current_day_id = $1 WHERE region = $2")
        .bind(day)
        .bind(&region)
        .execute(db)
        .await
        .unwrap();
    let today = ctx.today(early).await;
    assert_eq!(today.day.as_option().unwrap().region, region, "a friend said it is time");
    assert_eq!(ctx.post(early).await.late_seconds, 0);

    // The rest post while the fan-out runs, from several processes at once.
    let mut requests = Vec::new();
    for user in &people[1..] {
        requests.push((user, ctx.pair(user).await));
    }
    let posts = futures::future::join_all(
        requests.iter().map(|(user, request)| ctx.call::<_, serde_json::Value>("PostMoment", user, request)),
    );
    let tells = futures::future::join_all((0..2).map(|_| ctx.moments.send_prompt(day)));
    let (posts, tells) = tokio::join!(posts, tells);
    assert!(posts.iter().all(Result::is_ok), "{posts:?}");
    assert!(tells.iter().all(|told| matches!(told, Ok(Sent::Told(_)))), "{tells:?}");

    assert!(ctx.notifications(early, KIND_PROMPT).await.is_empty(), "posted already: no prompt after it");
    for user in &people {
        assert!(ctx.notifications(user, KIND_PROMPT).await.len() <= 1, "{}", user.handle);
        assert_eq!(ctx.try_post(user).await.unwrap_err().reason, "ALREADY_POSTED", "{}", user.handle);
    }
}
