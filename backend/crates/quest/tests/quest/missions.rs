//! Missions: what is drawn for whom, progress, claims, the daily bonus and
//! the reroll.
use std::collections::HashSet;

use chrono::{Duration, NaiveDate, Utc};
use serde_json::json;
use tokuly_core::prelude::*;
use tokuly_proto::quest::{quest_event::Event as Push, *};
use tokuly_quest::{metrics::Metric, missions::pool, rules};
use tokuly_testkit::TestUser;

use crate::common::*;

/// A finished (or open) mission of an earlier day, stored directly.
async fn past_mission(ctx: &Ctx, user: &TestUser, day: NaiveDate, slot: i16, completed: bool) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO quest_missions (user_id, period, period_start, slot, mission_key, metric, target, progress, reward_coins, reward_xp, completed_at)
         VALUES ($1, 1, $2, $5, 'stamp_1', 'stamps_sent', 1, $3, 5, 10, CASE WHEN $4 THEN now() - interval '1 day' END) RETURNING id",
    )
    .bind(user.id)
    .bind(day)
    .bind(i64::from(completed))
    .bind(completed)
    .bind(slot)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap()
}

#[tokio::test]
async fn someone_without_friends_gets_missions_they_can_do() {
    let ctx = start().await;
    let loner = ctx.app.user("loner").await;

    let board = ctx.missions(&loner).await;
    assert!(!board.daily.is_empty() && board.daily.len() <= 3);
    let weekly = board.weekly.as_option().expect("a weekly mission");
    assert_eq!(weekly.period, MissionPeriod::MISSION_PERIOD_WEEKLY);
    assert_eq!(board.rerolls_left, 1);
    let bonus = board.daily_bonus.as_option().unwrap();
    assert_eq!((bonus.claimed, bonus.required, bonus.granted), (0, board.daily.len() as i32, false));
    assert_eq!(bonus.reward.as_option().unwrap().coins, 15);
    assert!(board.unclaimed.is_empty());
    assert!(tokuly_core::time::from_ts(board.daily_resets_at.as_option().unwrap()) > Utc::now());
    assert!(
        tokuly_core::time::from_ts(board.weekly_resets_at.as_option().unwrap())
            >= tokuly_core::time::from_ts(board.daily_resets_at.as_option().unwrap())
    );

    // What is live on this server right now, as the rule table reports it.
    let table: ListEarningRulesResponse =
        ctx.call(QUEST, "ListEarningRules", None, &ListEarningRulesRequest::default()).await.unwrap();
    let live: HashSet<&str> = table.rules.iter().filter(|rule| rule.live).map(|rule| rule.key.as_str()).collect();

    let mut counted = HashSet::new();
    for mission in board.daily.iter().chain(std::iter::once(weekly)) {
        let def = pool::find(&mission.key).unwrap_or_else(|| panic!("{} is not in the pool", mission.key));
        assert_eq!(def.friends, 0, "{} needs friends the player does not have", mission.key);
        assert!(def.experienced.is_none(), "{} is for people who have done it before", mission.key);
        // Fed by something that happens on this server, or by the quest domain itself.
        let feeders: Vec<&str> = rules::rules_counting(def.metric).map(|rule| rule.key).collect();
        assert!(
            feeders.is_empty() || feeders.iter().any(|rule| live.contains(rule)),
            "{} counts {:?}, which nothing feeds",
            mission.key,
            def.metric
        );
        assert_eq!(mission.state, MissionState::MISSION_STATE_ACTIVE);
        assert_eq!((mission.progress, mission.target), (0, def.target));
        assert!(!mission.title.as_option().unwrap().ja.is_empty());
        assert!(mission.reward.as_option().unwrap().coins > 0 && mission.reward.as_option().unwrap().xp > 0);
        if mission.period == MissionPeriod::MISSION_PERIOD_DAILY {
            assert!(
                counted.insert(def.metric) || def.metric == Metric::Xp,
                "two daily missions count {:?}",
                def.metric
            );
            assert_eq!(mission.period_start, today().to_string());
        }
    }

    // The set is drawn once: asking again shows the same missions.
    let again = ctx.missions(&loner).await;
    assert_eq!(
        again.daily.iter().map(|mission| mission.id.clone()).collect::<Vec<_>>(),
        board.daily.iter().map(|mission| mission.id.clone()).collect::<Vec<_>>()
    );
    assert_eq!(again.weekly.as_option().unwrap().id, weekly.id);

    assert_error(
        ctx.call::<_, ListMissionsResponse>(MISSIONS, "ListMissions", None, &ListMissionsRequest::default()).await,
        "unauthenticated",
        "SIGN_IN_REQUIRED",
    );
}

#[tokio::test]
async fn missions_are_drawn_with_the_first_earnings_of_the_day() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    ctx.send_stamps(&alice, 1).await;
    ctx.flush(&[&alice]).await;
    // Nobody opened the missions screen, and yet they are there.
    let drawn: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM quest_missions WHERE user_id = $1 AND period = 1 AND period_start = $2",
    )
    .bind(alice.id)
    .bind(today())
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert!(drawn >= 1);
}

#[tokio::test]
async fn progress_completes_a_mission_and_the_claim_pays_once() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    ctx.known_set(&alice).await;
    let mut listener = ctx.app.listen_user(alice.id).await;

    ctx.send_stamps(&alice, 3).await;
    let board = ctx.missions(&alice).await;
    let stamps = &board.daily[0];
    assert_eq!(
        (stamps.key.as_str(), stamps.progress, stamps.target, stamps.state.as_known()),
        ("stamps_5", 3, 5, Some(MissionState::MISSION_STATE_ACTIVE))
    );
    assert_error(ctx.claim(&alice, &stamps.id).await, "failed_precondition", "MISSION_NOT_COMPLETED");

    ctx.send_stamps(&alice, 4).await;
    ctx.flush(&[&alice]).await;
    let completed: Vec<Mission> = pushes(&mut listener)
        .await
        .into_iter()
        .filter_map(|push| if let Push::MissionCompleted(mission) = push { Some(*mission) } else { None })
        .collect();
    assert_eq!(
        completed.iter().map(|mission| mission.key.as_str()).collect::<Vec<_>>(),
        vec!["stamps_5"],
        "pushed once, when the target was reached"
    );

    let board = ctx.missions(&alice).await;
    let stamps = &board.daily[0];
    assert_eq!(
        (stamps.progress, stamps.state.as_known()),
        (5, Some(MissionState::MISSION_STATE_COMPLETED)),
        "progress stops at the target"
    );
    assert!(stamps.completed_at.as_option().is_some());
    // The week counts the same stamps.
    let weekly = board.weekly.as_option().unwrap();
    assert_eq!((weekly.key.as_str(), weekly.progress, weekly.target), ("w_stamps_30", 7, 30));

    let xp_before = ctx.progress(&alice).await.level.as_option().unwrap().total_xp;
    let claimed = ctx.claim(&alice, &stamps.id).await.unwrap();
    assert!(claimed.granted);
    let reward = claimed.reward.as_option().unwrap();
    assert_eq!((reward.coins, reward.xp), (10, 20));
    assert_eq!(claimed.mission.as_option().unwrap().state, MissionState::MISSION_STATE_CLAIMED);
    assert!(claimed.bonus.as_option().is_none());
    let bonus = claimed.daily_bonus.as_option().unwrap();
    assert_eq!((bonus.claimed, bonus.required, bonus.granted), (1, 3, false));
    assert_eq!(claimed.progress.as_option().unwrap().level.as_option().unwrap().total_xp, xp_before + 20);
    assert_eq!(ctx.coins(&alice).await, 10);
    assert_eq!(ctx.source_today(&alice, "missions").await, (20, false));

    // Claiming again changes nothing.
    let repeated = ctx.claim(&alice, &stamps.id).await.unwrap();
    assert!(!repeated.granted);
    assert_eq!(repeated.progress.as_option().unwrap().level.as_option().unwrap().total_xp, xp_before + 20);
    assert_eq!(ctx.coins(&alice).await, 10);
    assert_eq!(ctx.counter(&alice, "missions_claimed").await, 1);
}

#[tokio::test]
async fn claiming_the_whole_day_adds_the_bonus() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    ctx.known_set(&alice).await;

    ctx.send_stamps(&alice, 5).await;
    ctx.post_note(&alice).await;
    ctx.like_notes(&alice, 3).await;
    let board = ctx.missions(&alice).await;
    assert!(board.daily.iter().all(|mission| mission.state == MissionState::MISSION_STATE_COMPLETED));

    let first = ctx.claim(&alice, &board.daily[0].id).await.unwrap();
    let second = ctx.claim(&alice, &board.daily[1].id).await.unwrap();
    assert!(first.bonus.as_option().is_none() && second.bonus.as_option().is_none());
    let last = ctx.claim(&alice, &board.daily[2].id).await.unwrap();
    let bonus = last.bonus.as_option().expect("the last claim completes the day");
    assert_eq!((bonus.coins, bonus.xp), (15, 30));
    let day = last.daily_bonus.as_option().unwrap();
    assert_eq!((day.claimed, day.required, day.granted), (3, 3, true));

    // 10 + 5 + 5 coins for the missions, 15 for the day.
    assert_eq!(ctx.coins(&alice).await, 35);
    // 20 + 10 + 10 XP for the missions, 30 for the day.
    assert_eq!(ctx.source_today(&alice, "missions").await, (70, false));
    assert_eq!(ctx.counter(&alice, "perfect_days").await, 1);
    assert!(ctx.badge_keys(&alice).await.contains(&"perfect_bronze".to_owned()), "a perfect day is a badge");

    // The bonus is granted once, however often the last claim is repeated.
    let repeated = ctx.claim(&alice, &board.daily[2].id).await.unwrap();
    assert!(!repeated.granted && repeated.bonus.as_option().is_none());
    assert_eq!(ctx.coins(&alice).await, 35);
    assert!(ctx.missions(&alice).await.daily_bonus.as_option().unwrap().granted);
}

#[tokio::test]
async fn a_mission_is_claimed_by_its_owner_only() {
    let ctx = start().await;
    let (alice, mallory) = (ctx.app.user("alice").await, ctx.app.user("mallory").await);
    ctx.known_set(&alice).await;
    ctx.send_stamps(&alice, 5).await;
    let board = ctx.missions(&alice).await;
    let mission_id = board.daily[0].id.clone();

    assert_error(ctx.claim(&mallory, &mission_id).await, "not_found", "MISSION_NOT_FOUND");
    assert_error(ctx.reroll(&mallory, &board.daily[1].id).await, "not_found", "MISSION_NOT_FOUND");
    assert_error(ctx.claim(&alice, &Uuid::now_v7().to_string()).await, "not_found", "MISSION_NOT_FOUND");
    assert_error(ctx.claim(&alice, "not-an-id").await, "invalid_argument", "INVALID_ID");
    assert_error(
        ctx.call::<_, ClaimMissionResponse>(
            MISSIONS,
            "ClaimMission",
            None,
            &ClaimMissionRequest { mission_id: mission_id.clone(), ..Default::default() },
        )
        .await,
        "unauthenticated",
        "SIGN_IN_REQUIRED",
    );
    assert_error(
        ctx.call::<_, RerollMissionResponse>(
            MISSIONS,
            "RerollMission",
            None,
            &RerollMissionRequest { mission_id: mission_id.clone(), ..Default::default() },
        )
        .await,
        "unauthenticated",
        "SIGN_IN_REQUIRED",
    );
    assert_eq!(ctx.coins(&mallory).await, 0);
    assert!(ctx.claim(&alice, &mission_id).await.unwrap().granted);
}

#[tokio::test]
async fn one_reroll_a_day_brings_a_mission_that_starts_from_zero() {
    let ctx = start().await;
    // Somebody else has been busy on this server, so several rules are live.
    let busy = ctx.app.user("busy").await;
    ctx.send_stamps(&busy, 1).await;
    ctx.post_note(&busy).await;
    ctx.like_notes(&busy, 1).await;
    ctx.feed(event(
        "engagement.comment.published",
        Some(busy.id),
        json!({ "comment_id": Uuid::now_v7(), "author_id": busy.id, "subject_owner_id": null }),
    ))
    .await;
    ctx.feed(event(
        "engagement.share.recorded",
        Some(busy.id),
        json!({ "user_id": busy.id, "subject_type": 3, "subject_id": Uuid::now_v7(), "target": 1 }),
    ))
    .await;
    ctx.feed(event(
        "wallet.balance.changed",
        Some(busy.id),
        json!({ "user_id": busy.id, "coins": 10, "earnings": 0, "entry_id": Uuid::now_v7(), "reason": "daily_bonus" }),
    ))
    .await;

    let alice = ctx.app.user("alice").await;
    ctx.known_set(&alice).await;
    // Alice did all of those things today as well, before the reroll.
    ctx.send_stamps(&alice, 2).await;
    ctx.like_notes(&alice, 1).await;
    ctx.feed(event(
        "engagement.comment.published",
        Some(alice.id),
        json!({ "comment_id": Uuid::now_v7(), "author_id": alice.id, "subject_owner_id": null }),
    ))
    .await;
    ctx.feed(event(
        "engagement.share.recorded",
        Some(alice.id),
        json!({ "user_id": alice.id, "subject_type": 3, "subject_id": Uuid::now_v7(), "target": 1 }),
    ))
    .await;
    ctx.feed(event(
        "wallet.balance.changed",
        Some(alice.id),
        json!({ "user_id": alice.id, "coins": 10, "earnings": 0, "entry_id": Uuid::now_v7(), "reason": "daily_bonus" }),
    ))
    .await;

    let board = ctx.missions(&alice).await;
    assert_eq!((board.daily[0].key.as_str(), board.daily[0].progress, board.rerolls_left), ("stamps_5", 2, 1));

    let rerolled = ctx.reroll(&alice, &board.daily[0].id).await.unwrap();
    let new = rerolled.mission.as_option().unwrap();
    assert_eq!(rerolled.rerolls_left, 0);
    assert_eq!(new.id, board.daily[0].id, "the mission keeps its slot");
    assert_ne!(new.key, "stamps_5");
    assert!(new.rerolled);
    assert_eq!(
        (new.progress, new.state.as_known()),
        (0, Some(MissionState::MISSION_STATE_ACTIVE)),
        "what was done before the reroll does not count"
    );
    let def = pool::find(&new.key).unwrap();
    assert!(
        ![Metric::StampsSent, Metric::NotesPosted, Metric::LikesGiven].contains(&def.metric),
        "{} counts what the set already counts",
        new.key
    );
    assert_eq!(def.friends, 0);
    assert_eq!(new.reward.as_option().unwrap().coins, def.payout().coins);

    // It stays at zero until something new is done, and the others are untouched.
    let board = ctx.missions(&alice).await;
    assert_eq!((board.daily[0].key.as_str(), board.daily[0].progress, board.rerolls_left), (new.key.as_str(), 0, 0));
    assert_eq!((board.daily[2].key.as_str(), board.daily[2].progress), ("likes_3", 1));

    // One reroll a day.
    assert_error(ctx.reroll(&alice, &board.daily[1].id).await, "failed_precondition", "NO_REROLL_LEFT");
    // A completed mission is not rerolled, whatever the allowance.
    ctx.post_note(&alice).await;
    ctx.flush(&[&alice]).await;
    assert_error(ctx.reroll(&alice, &board.daily[1].id).await, "failed_precondition", "MISSION_NOT_ACTIVE");
    assert_error(ctx.reroll(&alice, &Uuid::now_v7().to_string()).await, "not_found", "MISSION_NOT_FOUND");
}

#[tokio::test]
async fn a_finished_mission_can_be_claimed_for_a_week() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    // Yesterday: one mission finished, one not. Ten days ago: one finished.
    let yesterday = past_mission(&ctx, &alice, today() - Duration::days(1), 0, true).await;
    let unfinished = past_mission(&ctx, &alice, today() - Duration::days(1), 1, false).await;
    let long_ago = past_mission(&ctx, &alice, today() - Duration::days(10), 0, true).await;

    let board = ctx.missions(&alice).await;
    assert_eq!(
        board.unclaimed.iter().map(|mission| mission.id.clone()).collect::<Vec<_>>(),
        vec![yesterday.to_string()]
    );
    assert_eq!(board.unclaimed[0].state, MissionState::MISSION_STATE_COMPLETED);

    let claimed = ctx.claim(&alice, &yesterday.to_string()).await.unwrap();
    assert!(claimed.granted);
    assert_eq!(ctx.coins(&alice).await, 5);
    // Yesterday was not completed, so there is no bonus for it.
    assert!(claimed.bonus.as_option().is_none());
    assert!(ctx.missions(&alice).await.unclaimed.is_empty());

    assert_error(ctx.claim(&alice, &unfinished.to_string()).await, "failed_precondition", "MISSION_NOT_COMPLETED");
    assert_error(ctx.reroll(&alice, &unfinished.to_string()).await, "failed_precondition", "MISSION_NOT_ACTIVE");
    assert_error(ctx.claim(&alice, &long_ago.to_string()).await, "not_found", "MISSION_NOT_FOUND");
}

#[tokio::test]
async fn coins_left_unpaid_by_a_stopped_process_are_paid_by_the_worker() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    // A claim whose process stopped before the wallet was reached.
    let mission = past_mission(&ctx, &alice, today(), 0, true).await;
    sqlx::query("UPDATE quest_missions SET claimed_at = now() - interval '5 minutes' WHERE id = $1")
        .bind(mission)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO quest_mission_days (user_id, day, bonus_granted_at, bonus_coins) VALUES ($1, $2, now() - interval '5 minutes', 15)",
    )
    .bind(alice.id)
    .bind(today())
    .execute(&ctx.app.core.db)
    .await
    .unwrap();

    ctx.quest.pay_stragglers().await.unwrap();
    assert_eq!(ctx.coins(&alice).await, 5 + 15);
    // Paid means paid: another round adds nothing.
    ctx.quest.pay_stragglers().await.unwrap();
    assert_eq!(ctx.coins(&alice).await, 20);
    let paid: bool = sqlx::query_scalar("SELECT paid_at IS NOT NULL FROM quest_missions WHERE id = $1")
        .bind(mission)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(paid);

    // The player's own next visit pays as well, without waiting for the worker.
    let bob = ctx.app.user("bob").await;
    let mission = past_mission(&ctx, &bob, today() - Duration::days(1), 0, true).await;
    sqlx::query("UPDATE quest_missions SET claimed_at = now() WHERE id = $1")
        .bind(mission)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.missions(&bob).await;
    assert_eq!(ctx.coins(&bob).await, 5);
}
