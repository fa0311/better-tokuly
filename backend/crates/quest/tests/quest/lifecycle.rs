//! The edges of a player's data: the export, account deletion, the workers
//! that tidy up, and the analytics trail of what was awarded.
use chrono::Duration;
use serde_json::json;
use tokuly_core::prelude::*;
use tokuly_quest::badges;

use crate::common::*;

async fn rows_of(ctx: &Ctx, table: &str, user: Uuid) -> i64 {
    let column = if table == "quest_streaks" { "user_lo = $1 OR user_hi" } else { "user_id" };
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table} WHERE {column} = $1")))
        .bind(user)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn the_export_holds_what_the_domain_knows_about_a_player() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    ctx.send_stamps(&alice, 2).await;
    ctx.flush(&[&alice]).await;
    badges::award(&ctx.app.core, alice.id, "bug_hunter").await.unwrap();
    ctx.seed_streak(&alice, &bob, 4, 1).await;
    ctx.missions(&alice).await;

    let exporter = ctx
        .app
        .core
        .exporters
        .all()
        .into_iter()
        .find(|exporter| exporter.name() == "quest")
        .expect("the quest section is registered");
    let export = exporter.export(alice.id).await.unwrap();
    assert_eq!(export["profile"]["xp"], 4);
    assert_eq!(export["profile"]["level"], 1);
    assert_eq!(export["counters"]["stamps_sent"], 2);
    assert_eq!(export["xp_by_day"][0]["source"], "stamps");
    assert!(export["badges"].as_array().unwrap().iter().any(|badge| badge["badge"] == "bug_hunter"));
    assert_eq!(export["streaks"][0]["friend_id"], bob.id.to_string());
    assert_eq!(export["streaks"][0]["days"], 4);
    assert!(!export["missions"].as_array().unwrap().is_empty());
    assert!(
        export["seasons"].as_array().unwrap().is_empty() && export["streak_restores"].as_array().unwrap().is_empty()
    );

    // Somebody the domain has never heard of exports as empty, not as an error.
    let nobody = exporter.export(ctx.app.user("nobody").await.id).await.unwrap();
    assert!(nobody["profile"].is_null() && nobody["badges"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn deleting_an_account_erases_its_quest_data() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    for user in [&alice, &bob] {
        ctx.send_stamps(user, 2).await;
        badges::award(&ctx.app.core, user.id, "bug_hunter").await.unwrap();
        ctx.missions(user).await;
    }
    ctx.flush(&[&alice, &bob]).await;
    ctx.seed_streak(&alice, &bob, 4, 1).await;
    // Alice has earnings that were not written yet when her account goes.
    ctx.send_stamps(&alice, 1).await;

    let deleted = event("identity.user.deleted", Some(alice.id), json!({ "user_id": alice.id }));
    ctx.feed(deleted.clone()).await;
    ctx.feed(deleted).await; // redelivery
    // What was still waiting in Valkey is not written afterwards.
    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1")
        .bind(alice.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.quest.flush_user(alice.id).await.unwrap();

    for table in [
        "quest_profiles",
        "quest_xp_days",
        "quest_metric_days",
        "quest_counters",
        "quest_badges",
        "quest_missions",
        "quest_mission_days",
        "quest_streaks",
        "quest_season_results",
        "quest_streak_restores",
    ] {
        assert_eq!(rows_of(&ctx, table, alice.id).await, 0, "{table} still holds rows of the deleted account");
    }
    // Bob keeps everything of his own; only the streak with Alice is gone.
    assert_eq!(rows_of(&ctx, "quest_streaks", bob.id).await, 0);
    assert!(rows_of(&ctx, "quest_badges", bob.id).await >= 1);
    assert_eq!(ctx.progress(&bob).await.level.as_option().unwrap().total_xp, 4);
}

#[tokio::test]
async fn level_rewards_left_behind_are_granted_by_the_worker() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    // Level 3 reached, but the process stopped before the coins of levels 2 and 3 were granted.
    sqlx::query("INSERT INTO quest_profiles (user_id, xp, level, rewarded_level, updated_at) VALUES ($1, 260, 3, 1, now() - interval '5 minutes')").bind(alice.id).execute(&ctx.app.core.db).await.unwrap();

    ctx.quest.reward_stragglers().await.unwrap();
    assert_eq!(ctx.coins(&alice).await, 40);
    let rewarded: i32 = sqlx::query_scalar("SELECT rewarded_level FROM quest_profiles WHERE user_id = $1")
        .bind(alice.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(rewarded, 3);
    ctx.quest.reward_stragglers().await.unwrap();
    assert_eq!(ctx.coins(&alice).await, 40);

    // The tenth level pays the milestone reward.
    let bob = ctx.app.user("bob").await;
    sqlx::query("INSERT INTO quest_profiles (user_id, xp, level, rewarded_level, updated_at) VALUES ($1, 2700, 10, 9, now() - interval '5 minutes')").bind(bob.id).execute(&ctx.app.core.db).await.unwrap();
    ctx.quest.reward_stragglers().await.unwrap();
    assert_eq!(ctx.coins(&bob).await, 100);
}

#[tokio::test]
async fn pruning_removes_what_nothing_reads_any_more() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let old = today() - Duration::days(120);
    for (day, metric) in [(old, "messages"), (today(), "messages")] {
        sqlx::query("INSERT INTO quest_metric_days (user_id, day, metric, value) VALUES ($1, $2, $3, 5)")
            .bind(alice.id)
            .bind(day)
            .bind(metric)
            .execute(&ctx.app.core.db)
            .await
            .unwrap();
    }
    for (day, slot) in [(old, 0_i16), (today() - Duration::days(1), 1)] {
        sqlx::query("INSERT INTO quest_missions (user_id, period, period_start, slot, mission_key, metric, target) VALUES ($1, 1, $2, $3, 'stamp_1', 'stamps_sent', 1)")
            .bind(alice.id)
            .bind(day)
            .bind(slot)
            .execute(&ctx.app.core.db)
            .await
            .unwrap();
    }
    // XP history is kept for good.
    sqlx::query("INSERT INTO quest_xp_days (user_id, day, source, xp) VALUES ($1, $2, 'chat', 7)")
        .bind(alice.id)
        .bind(old)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();

    ctx.quest.prune().await.unwrap();
    assert_eq!(rows_of(&ctx, "quest_metric_days", alice.id).await, 1);
    assert_eq!(rows_of(&ctx, "quest_missions", alice.id).await, 1);
    assert_eq!(rows_of(&ctx, "quest_xp_days", alice.id).await, 1);
}

#[tokio::test]
async fn every_award_leaves_an_analytics_event() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    // XP, a level-up (five friends), a badge (the first friend), a mission
    // claim and the bonus for completing that mission's day.
    for _ in 0..5 {
        ctx.friend_accepted(&alice, &ctx.app.user("friend").await).await;
    }
    ctx.flush(&[&alice]).await;
    let mission: Uuid = sqlx::query_scalar(
        "INSERT INTO quest_missions (user_id, period, period_start, slot, mission_key, metric, target, progress, reward_coins, reward_xp, completed_at)
         VALUES ($1, 1, $2, 0, 'stamp_1', 'stamps_sent', 1, 1, 5, 10, now()) RETURNING id",
    )
    .bind(alice.id)
    .bind(today() - Duration::days(1))
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    let _: tokuly_proto::quest::ClaimMissionResponse = ctx
        .call(
            MISSIONS,
            "ClaimMission",
            Some(&alice),
            &tokuly_proto::quest::ClaimMissionRequest { mission_id: mission.to_string(), ..Default::default() },
        )
        .await
        .unwrap();

    ctx.app.flush_analytics().await;
    let user = alice.id.to_string();
    for (name, at_least) in [
        ("quest.xp", 1_u64),
        ("quest.level_up", 1),
        ("quest.badge", 1),
        ("quest.mission.claim", 1),
        ("quest.mission.daily_bonus", 1),
    ] {
        let count: u64 = ctx
            .app
            .core
            .analytics
            .ch
            .query("SELECT count() FROM events WHERE name = ? AND toString(user_id) = ?")
            .bind(name)
            .bind(&user)
            .fetch_one()
            .await
            .unwrap();
        assert!(count >= at_least, "{name}: {count} events");
    }
    let xp: f64 = ctx
        .app
        .core
        .analytics
        .ch
        .query("SELECT sum(value) FROM events WHERE name = 'quest.xp' AND toString(user_id) = ?")
        .bind(&user)
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(xp, 140.0, "100 XP from friends, 10 from the mission and 30 from the day's bonus");
}
