//! Requests and events that arrive at the same time.
//!
//! Every place that checks and then writes is run here with several real
//! concurrent callers — more of them than the test pool has connections
//! (5), so a lock holder that needed a second connection would show as a
//! stall, and a missing lock as a double reward.
use std::time::Duration;

use futures::future::{BoxFuture, join_all};
use serde_json::json;
use tokuly_core::prelude::*;
use tokuly_proto::quest::*;
use tokuly_quest::badges;
use tokuly_testkit::RpcError;

use crate::common::*;

/// Concurrent callers per test: more than the pool's 5 connections.
const CALLERS: usize = 8;

/// Runs the calls together; none may take longer than this in total.
async fn together<T>(calls: Vec<BoxFuture<'_, T>>) -> Vec<T> {
    tokio::time::timeout(Duration::from_secs(60), join_all(calls)).await.expect("concurrent calls stalled")
}

#[tokio::test]
async fn a_mission_claimed_by_many_requests_at_once_pays_once() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    ctx.known_set(&alice).await;
    ctx.send_stamps(&alice, 5).await;
    let mission = ctx.missions(&alice).await.daily[0].id.clone();

    let claims: Vec<Result<ClaimMissionResponse, RpcError>> =
        together((0..CALLERS).map(|_| Box::pin(ctx.claim(&alice, &mission)) as BoxFuture<'_, _>).collect()).await;
    let granted = claims.iter().filter(|claim| claim.as_ref().is_ok_and(|claim| claim.granted)).count();
    assert!(claims.iter().all(Result::is_ok), "{claims:?}");
    assert_eq!(granted, 1, "exactly one request collects the reward");
    assert_eq!(ctx.coins(&alice).await, 10);
    assert_eq!(ctx.source_today(&alice, "missions").await, (20, false));
    assert_eq!(ctx.counter(&alice, "missions_claimed").await, 1);
}

#[tokio::test]
async fn a_day_claimed_from_all_sides_at_once_grants_one_bonus() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    ctx.known_set(&alice).await;
    ctx.send_stamps(&alice, 5).await;
    ctx.post_note(&alice).await;
    ctx.like_notes(&alice, 3).await;
    let daily: Vec<String> = ctx.missions(&alice).await.daily.iter().map(|mission| mission.id.clone()).collect();

    // Each of the three missions, three times over, all at once.
    let calls =
        daily.iter().cycle().take(9).map(|mission| Box::pin(ctx.claim(&alice, mission)) as BoxFuture<'_, _>).collect();
    let claims: Vec<Result<ClaimMissionResponse, RpcError>> = together(calls).await;
    assert!(claims.iter().all(Result::is_ok), "{claims:?}");
    let bonuses =
        claims.iter().filter(|claim| claim.as_ref().is_ok_and(|claim| claim.bonus.as_option().is_some())).count();
    assert_eq!(bonuses, 1, "the bonus comes with exactly one claim");
    assert_eq!(claims.iter().filter(|claim| claim.as_ref().is_ok_and(|claim| claim.granted)).count(), 3);
    // 10 + 5 + 5 for the missions, 15 for the day; 20 + 10 + 10 + 30 XP.
    assert_eq!(ctx.coins(&alice).await, 35);
    assert_eq!(ctx.source_today(&alice, "missions").await, (70, false));
    assert_eq!(ctx.counter(&alice, "perfect_days").await, 1);
}

#[tokio::test]
async fn rerolls_at_once_use_the_days_one_allowance() {
    let ctx = start().await;
    // Enough is going on for other missions to be on offer.
    let busy = ctx.app.user("busy").await;
    ctx.send_stamps(&busy, 1).await;
    ctx.post_note(&busy).await;
    ctx.like_notes(&busy, 1).await;
    ctx.feed(event("engagement.share.recorded", Some(busy.id), json!({ "user_id": busy.id }))).await;
    ctx.feed(event("engagement.comment.published", Some(busy.id), json!({ "author_id": busy.id }))).await;

    let alice = ctx.app.user("alice").await;
    ctx.known_set(&alice).await;
    let daily: Vec<String> = ctx.missions(&alice).await.daily.iter().map(|mission| mission.id.clone()).collect();

    // All three missions, over and over, at the same moment.
    let calls = daily
        .iter()
        .cycle()
        .take(CALLERS)
        .map(|mission| Box::pin(ctx.reroll(&alice, mission)) as BoxFuture<'_, _>)
        .collect();
    let rerolls: Vec<Result<RerollMissionResponse, RpcError>> = together(calls).await;
    assert_eq!(rerolls.iter().filter(|reroll| reroll.is_ok()).count(), 1, "{rerolls:?}");
    for refused in rerolls.iter().filter_map(|reroll| reroll.as_ref().err()) {
        assert_eq!(refused.reason, "NO_REROLL_LEFT", "{refused}");
    }
    let board = ctx.missions(&alice).await;
    assert_eq!(board.rerolls_left, 0);
    assert_eq!(board.daily.iter().filter(|mission| mission.rerolled).count(), 1);
    // The set still counts three different things.
    let keys: std::collections::HashSet<&str> = board.daily.iter().map(|mission| mission.key.as_str()).collect();
    assert_eq!(keys.len(), 3);
}

#[tokio::test]
async fn a_reroll_racing_a_claim_ends_one_way_or_the_other() {
    let ctx = start().await;
    let busy = ctx.app.user("busy").await;
    ctx.post_note(&busy).await;
    ctx.like_notes(&busy, 1).await;
    ctx.feed(event("engagement.share.recorded", Some(busy.id), json!({ "user_id": busy.id }))).await;

    for round in 0..4 {
        let alice = ctx.app.user("alice").await;
        ctx.known_set(&alice).await;
        let mission = ctx.missions(&alice).await.daily[0].id.clone();
        // The fifth stamp is on its way while both requests arrive: neither
        // has seen the mission complete yet.
        ctx.send_stamps(&alice, 5).await;
        let (claimed, rerolled) = tokio::join!(ctx.claim(&alice, &mission), ctx.reroll(&alice, &mission));

        let after = ctx.missions(&alice).await;
        let slot = &after.daily[0];
        match (&claimed, &rerolled) {
            // Completed first: it is claimed and stays what it was.
            (Ok(claim), Err(refused)) => {
                assert!(claim.granted);
                assert_eq!(refused.reason, "MISSION_NOT_ACTIVE");
                assert_eq!(
                    (slot.key.as_str(), slot.state.as_known(), ctx.coins(&alice).await),
                    ("stamps_5", Some(MissionState::MISSION_STATE_CLAIMED), 10)
                );
            }
            // Swapped first: there was nothing to claim, and nothing was paid.
            (Err(refused), Ok(_)) => {
                assert_eq!(refused.reason, "MISSION_NOT_COMPLETED");
                assert_ne!(slot.key, "stamps_5");
                assert_eq!(ctx.coins(&alice).await, 0);
            }
            other => panic!("round {round}: a mission was both claimed and rerolled, or neither: {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_set_is_drawn_once_however_many_requests_ask_for_it() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let boards: Vec<ListMissionsResponse> =
        together((0..CALLERS).map(|_| Box::pin(ctx.missions(&alice)) as BoxFuture<'_, _>).collect()).await;
    let ids = |board: &ListMissionsResponse| -> Vec<String> {
        board.daily.iter().chain(board.weekly.as_option()).map(|mission| mission.id.clone()).collect()
    };
    for board in &boards {
        assert_eq!(ids(board), ids(&boards[0]), "two requests saw different sets");
    }
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM quest_missions WHERE user_id = $1")
        .bind(alice.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(stored as usize, ids(&boards[0]).len());
}

#[tokio::test]
async fn an_event_delivered_many_times_at_once_counts_once() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    let bumped = event(
        "social.bump.created",
        Some(alice.id),
        json!({ "bump_id": Uuid::now_v7(), "user_ids": [alice.id, bob.id], "friends": true }),
    );

    let deliveries =
        together((0..CALLERS).map(|_| Box::pin(ctx.quest.handle_event(bumped.clone())) as BoxFuture<'_, _>).collect())
            .await;
    assert!(deliveries.iter().all(Result::is_ok), "{deliveries:?}");
    // Flushed by several workers at once as well.
    let flushes = together(
        [&alice, &bob]
            .iter()
            .cycle()
            .take(CALLERS)
            .map(|user| Box::pin(ctx.quest.flush_user(user.id)) as BoxFuture<'_, _>)
            .collect(),
    )
    .await;
    assert!(flushes.iter().all(Result::is_ok), "{flushes:?}");

    for user in [&alice, &bob] {
        // One bump (15 XP), and the streak day both parts of it completed (5 XP).
        assert_eq!(ctx.source_today(user, "friends").await, (15, false));
        assert_eq!(ctx.source_today(user, "streaks").await, (5, false));
        assert_eq!(
            (
                ctx.counter(user, "bumps").await,
                ctx.counter(user, "streak_days").await,
                ctx.counter(user, "friends_reached").await
            ),
            (1, 1, 1)
        );
    }
    assert_eq!(ctx.streak(&alice, &bob).await.days, 1);
}

#[tokio::test]
async fn earnings_written_by_many_workers_at_once_are_written_once() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    // Stamps keep arriving while several flushes run.
    let mut calls: Vec<BoxFuture<'_, AppResult<()>>> = Vec::new();
    for _ in 0..4 {
        calls.push(Box::pin(async {
            ctx.send_stamps(&alice, 2).await;
            Ok(())
        }));
        calls.push(Box::pin(ctx.quest.flush_user(alice.id)));
    }
    let results = together(calls).await;
    assert!(results.iter().all(Result::is_ok), "{results:?}");
    ctx.flush(&[&alice]).await;
    assert_eq!(ctx.counter(&alice, "stamps_sent").await, 8);
    assert_eq!(ctx.source_today(&alice, "stamps").await, (16, false));
    assert_eq!(ctx.progress(&alice).await.level.as_option().unwrap().total_xp, 16);
}

#[tokio::test]
async fn a_badge_awarded_from_many_places_at_once_is_awarded_once() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let awards = together(
        (0..CALLERS)
            .map(|_| Box::pin(badges::award(&ctx.app.core, alice.id, "bug_hunter")) as BoxFuture<'_, _>)
            .collect(),
    )
    .await;
    assert_eq!(awards.iter().filter(|award| matches!(award, Ok(true))).count(), 1, "{awards:?}");
    assert_eq!(awards.iter().filter(|award| matches!(award, Ok(false))).count(), CALLERS - 1);
    assert_eq!(ctx.notifications(&alice, "quest.badge").await.len(), 1);
}

#[tokio::test]
async fn a_restore_asked_for_many_times_at_once_is_charged_once() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    ctx.seed_streak(&alice, &bob, 5, 2).await;
    ctx.fund(&alice, 300).await;

    let restores: Vec<Result<RestoreStreakResponse, RpcError>> =
        together((0..CALLERS).map(|_| Box::pin(ctx.restore(&alice, &bob)) as BoxFuture<'_, _>).collect()).await;
    assert!(restores.iter().all(Result::is_ok), "{restores:?}");
    let charged: i32 = restores.iter().map(|restore| restore.as_ref().unwrap().coins_spent).sum();
    assert_eq!(charged, 100, "the calls report one charge between them");
    assert_eq!(ctx.coins(&alice).await, 200);
    assert!(restores.iter().all(|restore| restore.as_ref().unwrap().streak.as_option().unwrap().days == 5));
    let orders: Vec<(bool, i32)> =
        sqlx::query_as("SELECT applied_at IS NOT NULL, restored_count FROM quest_streak_restores WHERE user_id = $1")
            .bind(alice.id)
            .fetch_all(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(orders, vec![(true, 5)]);
    assert_eq!(ctx.notifications(&bob, "quest.streak_restored").await.len(), 1);
}

#[tokio::test]
async fn two_friends_restoring_the_same_streak_at_once_pay_once_between_them() {
    let ctx = start().await;
    for round in 0..3 {
        let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
        ctx.befriend(&alice, &bob).await;
        ctx.seed_streak(&alice, &bob, 6, 2).await;
        ctx.fund(&alice, 100).await;
        ctx.fund(&bob, 100).await;

        let (hers, his) = tokio::join!(ctx.restore(&alice, &bob), ctx.restore(&bob, &alice));
        // Whoever was second finds the streak running: nothing to do, nothing charged.
        for result in [&hers, &his] {
            match result {
                Ok(restored) => assert_eq!(restored.streak.as_option().unwrap().days, 6),
                Err(refused) => assert_eq!(refused.reason, "STREAK_NOT_LOST", "round {round}: {refused}"),
            }
        }
        assert_eq!(ctx.streak(&alice, &bob).await.days, 6);
        assert_eq!(ctx.coins(&alice).await + ctx.coins(&bob).await, 100, "round {round}: one of them paid, once");
        let orders: i64 = sqlx::query_scalar("SELECT count(*) FROM quest_streak_restores WHERE user_id IN ($1, $2)")
            .bind(alice.id)
            .bind(bob.id)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
        assert_eq!(orders, 1, "round {round}: the one who did not restore keeps their month's restore");
    }
}

#[tokio::test]
async fn a_restore_that_stopped_half_way_is_finished_from_wherever_the_player_looks_next() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    ctx.seed_streak(&alice, &bob, 5, 2).await;
    ctx.fund(&alice, 100).await;
    // Three days ago the streak ended, Alice ordered its restore in time —
    // and the process stopped: nothing charged, nothing restored.
    let (lo, hi) = if alice.id < bob.id { (alice.id, bob.id) } else { (bob.id, alice.id) };
    let lost_at: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("UPDATE quest_streaks SET count = 0, expires_at = NULL, lost_count = 5, lost_at = now() - interval '3 days' WHERE user_lo = $1 AND user_hi = $2 RETURNING lost_at")
            .bind(lo)
            .bind(hi)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    sqlx::query("INSERT INTO quest_streak_restores (user_id, month, friend_id, cost, lost_at, created_at) VALUES ($1, date_trunc('month', $2::date)::date, $3, 100, $4, now() - interval '3 days')")
        .bind(alice.id)
        .bind(today())
        .bind(bob.id)
        .bind(lost_at)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();

    // Days later — long past the restore window — Alice opens her streaks.
    let streak = ctx.streak(&alice, &bob).await;
    assert_eq!(
        (streak.days, streak.state.as_known() != Some(StreakState::STREAK_STATE_LOST)),
        (5, true),
        "what was ordered in time is delivered"
    );
    assert_eq!(ctx.coins(&alice).await, 0);

    // An order whose streak is gone (the two are no longer friends) is refunded instead.
    let (carol, dave) = (ctx.app.user("carol").await, ctx.app.user("dave").await);
    ctx.fund(&carol, 100).await;
    let order: Uuid = sqlx::query_scalar("INSERT INTO quest_streak_restores (user_id, month, friend_id, cost, lost_at, created_at) VALUES ($1, date_trunc('month', $2::date)::date, $3, 100, now(), now() - interval '5 minutes') RETURNING id")
        .bind(carol.id)
        .bind(today())
        .bind(dave.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    // Charged by the half that ran before the stop.
    tokuly_wallet::ledger::spend(
        &ctx.app.core,
        carol.id,
        100,
        "quest.streak_restore",
        &format!("quest.streak_restore:{order}"),
        json!({}),
    )
    .await
    .unwrap();
    assert_eq!(ctx.coins(&carol).await, 0);
    ctx.quest.settle_pending_restores().await.unwrap();
    assert_eq!(ctx.coins(&carol).await, 100, "the coins came back");
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM quest_streak_restores WHERE user_id = $1")
        .bind(carol.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(left, 0, "and so did the month's restore");
}

/// A settle that refunded an undeliverable restore and stopped before
/// withdrawing the order leaves an order whose charge is refunded. The next
/// settle finishes the withdrawal once — without charging again, even though
/// the streak could be restored now — and nothing keeps retrying it.
#[tokio::test]
async fn a_restore_interrupted_between_refund_and_withdrawal_settles_once() {
    let ctx = start().await;
    let (erin, frank) = (ctx.app.user("erin").await, ctx.app.user("frank").await);
    ctx.befriend(&erin, &frank).await;
    ctx.seed_streak(&erin, &frank, 5, 2).await;
    ctx.fund(&erin, 100).await;
    let (lo, hi) = if erin.id < frank.id { (erin.id, frank.id) } else { (frank.id, erin.id) };
    let lost_at: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("UPDATE quest_streaks SET count = 0, expires_at = NULL, lost_count = 5, lost_at = now() - interval '1 hour' WHERE user_lo = $1 AND user_hi = $2 RETURNING lost_at")
            .bind(lo)
            .bind(hi)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    let order: Uuid = sqlx::query_scalar("INSERT INTO quest_streak_restores (user_id, month, friend_id, cost, lost_at, created_at) VALUES ($1, date_trunc('month', $2::date)::date, $3, 100, $4, now() - interval '5 minutes') RETURNING id")
        .bind(erin.id)
        .bind(today())
        .bind(frank.id)
        .bind(lost_at)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    // The half that ran before the stop: charged, then refunded — but the order was not withdrawn.
    let (entry, _) = tokuly_wallet::ledger::spend(&ctx.app.core, erin.id, 100, "quest.streak_restore", &format!("quest.streak_restore:{order}"), json!({}))
        .await
        .unwrap();
    tokuly_wallet::ledger::refund(&ctx.app.core, entry, "quest.streak_restore.refund").await.unwrap();
    assert_eq!(ctx.coins(&erin).await, 100);

    // Erin looks at her streaks: the order is finished as undone, not charged again.
    let streak = ctx.streak(&erin, &frank).await;
    assert_eq!(streak.state.as_known(), Some(StreakState::STREAK_STATE_LOST), "an undone restore restores nothing");
    assert_eq!(ctx.coins(&erin).await, 100, "and charges nothing");
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM quest_streak_restores WHERE id = $1").bind(order).fetch_one(&ctx.app.core.db).await.unwrap();
    assert_eq!(left, 0, "the withdrawal was finished");
    // Nothing is left for the background settle to retry.
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM quest_streak_restores WHERE user_id = $1 AND applied_at IS NULL").bind(erin.id).fetch_one(&ctx.app.core.db).await.unwrap();
    assert_eq!(pending, 0);
    ctx.quest.settle_pending_restores().await.unwrap();
    assert_eq!(ctx.coins(&erin).await, 100);
    let charges: i64 = sqlx::query_scalar("SELECT count(*) FROM wallet_entries WHERE from_user_id = $1 AND kind = 'spend'").bind(erin.id).fetch_one(&ctx.app.core.db).await.unwrap();
    assert_eq!(charges, 1, "the one charge that was refunded");

    // The month's restore is free again, as after any withdrawn order: asking for it is a new, paid restore.
    let restored = ctx.restore(&erin, &frank).await.unwrap();
    assert_eq!((restored.coins_spent, restored.streak.as_option().unwrap().days), (100, 5));
    assert_eq!(ctx.coins(&erin).await, 0);
}

#[tokio::test]
async fn a_season_finalized_by_several_workers_at_once_awards_once() {
    let ctx = start().await;
    let may = chrono::NaiveDate::from_ymd_opt(2003, 5, 1).unwrap();
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);
    ctx.befriend(&alice, &bob).await;
    ctx.befriend(&alice, &carol).await;
    for (user, xp) in [(&alice, 300), (&bob, 200), (&carol, 100)] {
        sqlx::query("INSERT INTO quest_xp_days (user_id, day, source, xp) VALUES ($1, '2003-05-10', 'chat', $2)")
            .bind(user.id)
            .bind(xp)
            .execute(&ctx.app.core.db)
            .await
            .unwrap();
    }

    let runs = together((0..4).map(|_| Box::pin(ctx.quest.finalize_season(may)) as BoxFuture<'_, _>).collect()).await;
    assert!(runs.iter().all(Result::is_ok), "{runs:?}");

    let times: i32 =
        sqlx::query_scalar("SELECT times FROM quest_badges WHERE user_id = $1 AND badge_key = 'season_05'")
            .bind(alice.id)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(times, 1);
    assert_eq!(ctx.counter(&alice, "seasons_won").await, 1);
    assert!(ctx.badge_keys(&alice).await.contains(&"season_champion".to_owned()));
    for user in [&alice, &bob, &carol] {
        assert_eq!(ctx.notifications(user, "quest.season").await.len(), 1);
    }
    let results: Vec<(i32, bool)> =
        sqlx::query_as("SELECT rank, badge_won FROM quest_season_results WHERE month = $1 ORDER BY xp DESC")
            .bind(may)
            .fetch_all(&ctx.app.core.db)
            .await
            .unwrap();
    // Alice ranks among three; Bob and Carol each only know Alice.
    assert_eq!(results, vec![(1, true), (2, false), (2, false)]);
}

#[tokio::test]
async fn sweeps_at_once_remind_once() {
    let ctx = start().await;
    let zone = evening_zone();
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    ctx.running_today(&alice, &bob, &zone, 5, None).await;

    let sweeps = together((0..4).map(|_| Box::pin(ctx.quest.sweep_streaks()) as BoxFuture<'_, _>).collect()).await;
    assert!(sweeps.iter().all(Result::is_ok), "{sweeps:?}");
    for user in [&alice, &bob] {
        assert_eq!(ctx.notifications(user, "quest.streak_expiring").await.len(), 1);
    }
}

#[tokio::test]
async fn a_title_never_outlives_a_badge_that_was_revoked_at_the_same_moment() {
    let ctx = start().await;
    let admin = ctx.app.user_with_roles("admin", &["admin"]).await;
    for round in 0..6 {
        let alice = ctx.app.user("alice").await;
        badges::award(&ctx.app.core, alice.id, "bug_hunter").await.unwrap();
        let revoke = RevokeBadgeRequest {
            user_id: alice.id.to_string(),
            badge_key: "bug_hunter".into(),
            reason: "race test".into(),
            ..Default::default()
        };
        let title = SetTitleRequest { badge_key: "bug_hunter".into(), ..Default::default() };
        let showcase = SetShowcaseRequest { badge_keys: vec!["bug_hunter".into()], ..Default::default() };
        let (revoked, _, _) = tokio::join!(
            ctx.call::<_, RevokeBadgeResponse>(ADMIN, "RevokeBadge", Some(&admin), &revoke),
            ctx.call::<_, SetTitleResponse>(BADGES, "SetTitle", Some(&alice), &title),
            ctx.call::<_, SetShowcaseResponse>(BADGES, "SetShowcase", Some(&alice), &showcase),
        );
        assert!(revoked.unwrap().revoked);
        // Whichever came first, a badge that is gone is neither title nor showcase.
        let (title_badge, showcase): (String, Vec<String>) =
            sqlx::query_as("SELECT title_badge, showcase FROM quest_profiles WHERE user_id = $1")
                .bind(alice.id)
                .fetch_one(&ctx.app.core.db)
                .await
                .unwrap();
        assert_eq!((title_badge.as_str(), showcase.len()), ("", 0), "round {round}");
    }
}

#[tokio::test]
async fn a_removal_delivered_again_or_late_ends_only_the_streak_it_was_about() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    ctx.seed_streak(&alice, &bob, 4, 1).await;
    ctx.unfriend(&alice, &bob).await;
    let removed = ctx.app.events_by("social.friend.removed", alice.id).await.pop().unwrap();

    // They make up and start over.
    ctx.befriend(&alice, &bob).await;
    ctx.map_reaction(&alice, &bob).await;
    ctx.map_reaction(&bob, &alice).await;
    assert_eq!(ctx.streak(&alice, &bob).await.days, 1);

    // The removal is delivered once more, after the new streak began.
    ctx.feed(removed.clone()).await;
    ctx.feed(removed).await;
    let streak = ctx.streak(&alice, &bob).await;
    assert_eq!((streak.days, streak.best_days), (1, 1), "the new streak is not the one that ended");

    // A removal from before a pair's streak existed (handled out of order) leaves it alone too.
    let (carol, dave) = (ctx.app.user("carol").await, ctx.app.user("dave").await);
    ctx.befriend(&carol, &dave).await;
    ctx.map_reaction(&carol, &dave).await;
    ctx.map_reaction(&dave, &carol).await;
    let stale = event_at(
        "social.friend.removed",
        Some(carol.id),
        json!({ "user_id": carol.id, "friend_id": dave.id, "blocked": false }),
        chrono::Utc::now() - chrono::Duration::hours(2),
    );
    ctx.feed(stale).await;
    assert_eq!(ctx.streak(&carol, &dave).await.days, 1);
}

/// One call of the stall test: its name and how it ended.
type Outcome = (&'static str, Result<(), RpcError>);

#[tokio::test]
async fn one_account_cannot_stall_the_api_for_everyone() {
    let ctx = start().await;
    let (alice, bob, other) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("other").await);
    ctx.befriend(&alice, &bob).await;
    ctx.seed_streak(&alice, &bob, 5, 2).await;
    ctx.fund(&alice, 100).await;
    ctx.known_set(&alice).await;
    ctx.send_stamps(&alice, 5).await;
    ctx.post_note(&alice).await;
    let daily: Vec<String> = ctx.missions(&alice).await.daily.iter().map(|mission| mission.id.clone()).collect();
    // Earnings are waiting to be written when the burst arrives.
    ctx.send_stamps(&alice, 2).await;

    fn done<T>(name: &'static str, result: Result<T, RpcError>) -> Outcome {
        (name, result.map(|_| ()))
    }
    // Every RPC that takes a lock or opens a transaction, four times over,
    // from one account: twenty-eight requests against five connections.
    let mut calls: Vec<BoxFuture<'_, Outcome>> = Vec::new();
    for round in 0..4 {
        let mission = &daily[round % daily.len()];
        calls.push(Box::pin(async { done("ClaimMission", ctx.claim(&alice, mission).await) }));
        calls.push(Box::pin(async { done("RerollMission", ctx.reroll(&alice, &daily[2]).await) }));
        calls.push(Box::pin(async {
            done(
                "ListMissions",
                ctx.call::<_, ListMissionsResponse>(
                    MISSIONS,
                    "ListMissions",
                    Some(&alice),
                    &ListMissionsRequest::default(),
                )
                .await,
            )
        }));
        calls.push(Box::pin(async {
            done(
                "GetProgress",
                ctx.call::<_, GetProgressResponse>(QUEST, "GetProgress", Some(&alice), &GetProgressRequest::default())
                    .await,
            )
        }));
        calls.push(Box::pin(async {
            done(
                "ListBadgeCatalog",
                ctx.call::<_, ListBadgeCatalogResponse>(
                    BADGES,
                    "ListBadgeCatalog",
                    Some(&alice),
                    &ListBadgeCatalogRequest::default(),
                )
                .await,
            )
        }));
        calls.push(Box::pin(async { done("RestoreStreak", ctx.restore(&alice, &bob).await) }));
        calls.push(Box::pin(async {
            done(
                "SetTitle",
                ctx.call::<_, SetTitleResponse>(BADGES, "SetTitle", Some(&alice), &SetTitleRequest::default()).await,
            )
        }));
        // Somebody else, meanwhile, just wants to see their progress.
        calls.push(Box::pin(async {
            done(
                "GetProgress (other account)",
                ctx.call::<_, GetProgressResponse>(QUEST, "GetProgress", Some(&other), &GetProgressRequest::default())
                    .await,
            )
        }));
    }
    let started = std::time::Instant::now();
    let outcomes = tokio::time::timeout(Duration::from_secs(25), join_all(calls))
        .await
        .expect("the burst left requests waiting for a connection");
    assert!(started.elapsed() < Duration::from_secs(25));

    for (name, outcome) in &outcomes {
        match outcome {
            Ok(()) => {}
            // Refusals that follow from the state are fine; running out of anything is not.
            Err(refused) => assert!(
                matches!(
                    refused.reason.as_str(),
                    "NO_REROLL_LEFT" | "MISSION_NOT_ACTIVE" | "NO_OTHER_MISSION" | "MISSION_NOT_COMPLETED"
                ),
                "{name} failed: {refused}"
            ),
        }
    }
    assert!(
        outcomes.iter().filter(|(name, _)| *name == "GetProgress (other account)").all(|(_, outcome)| outcome.is_ok())
    );
    // And the burst left consistent books: two missions claimed once each, one restore paid once.
    assert_eq!(ctx.counter(&alice, "missions_claimed").await, 2);
    assert_eq!(ctx.coins(&alice).await, 100 - 100 + 10 + 5);
    assert_eq!(ctx.streak(&alice, &bob).await.days, 5);
}
