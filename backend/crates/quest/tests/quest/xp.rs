//! Events become XP: once per event, capped per source and day, on the
//! player's own calendar; levels follow, with their coins and pushes.
use chrono::{Duration, Utc};
use serde_json::json;
use tokuly_core::{analytics::AnalyticsEvent, prelude::*};
use tokuly_proto::quest::{quest_event::Event as Push, *};

use crate::common::*;

/// A bump between two friends, as the social domain announces it.
fn bump(a: Uuid, b: Uuid) -> serde_json::Value {
    json!({ "bump_id": Uuid::now_v7(), "user_ids": [a, b], "friends": true, "lat": 35.68, "lng": 139.76 })
}

#[tokio::test]
async fn an_event_earns_xp_for_everyone_it_credits() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);

    ctx.feed(event("social.bump.created", Some(alice.id), bump(alice.id, bob.id))).await;
    ctx.flush(&[&alice, &bob]).await;

    for user in [&alice, &bob] {
        let progress = ctx.progress(user).await;
        let level = progress.level.as_option().unwrap();
        assert_eq!((level.level, level.total_xp, level.xp_into_level, level.xp_for_next_level), (1, 15, 15, 100));
        assert_eq!(progress.xp_today, 15);
        assert_eq!(progress.today, today().to_string());
        assert_eq!(progress.time_zone, "Asia/Tokyo");
        assert_eq!(progress.active_days, 1);
        assert!(tokuly_core::time::from_ts(progress.day_resets_at.as_option().unwrap()) > Utc::now());
        // Every source is listed, with its cap; only the one that earned has XP.
        assert_eq!(ctx.source_today(user, "friends").await, (15, false));
        assert_eq!(ctx.source_today(user, "chat").await, (0, false));
        let friends = progress.sources.iter().find(|row| row.source.as_option().unwrap().key == "friends").unwrap();
        assert_eq!(friends.source.as_option().unwrap().daily_cap, 100);
        assert_eq!(ctx.counter(user, "bumps").await, 1);
    }
    // A bump between strangers earns nothing.
    let (carol, dave) = (ctx.app.user("carol").await, ctx.app.user("dave").await);
    ctx.feed(event(
        "social.bump.created",
        Some(carol.id),
        json!({ "user_ids": [carol.id, dave.id], "friends": false }),
    ))
    .await;
    ctx.flush(&[&carol, &dave]).await;
    assert_eq!(ctx.progress(&carol).await.xp_today, 0);
}

#[tokio::test]
async fn a_redelivered_event_counts_once() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let bumped = event("social.bump.created", Some(alice.id), bump(alice.id, bob.id));

    // Delivered, redelivered before anything was written, and again after.
    ctx.feed(bumped.clone()).await;
    ctx.feed(bumped.clone()).await;
    ctx.flush(&[&alice, &bob]).await;
    ctx.feed(bumped.clone()).await;
    ctx.flush(&[&alice, &bob]).await;
    // Writing again without anything new changes nothing either.
    ctx.flush(&[&alice, &bob]).await;

    for user in [&alice, &bob] {
        assert_eq!(ctx.progress(user).await.level.as_option().unwrap().total_xp, 15);
        assert_eq!(ctx.counter(user, "bumps").await, 1);
    }
    // A different event of the same kind does count.
    ctx.feed(event("social.bump.created", Some(alice.id), bump(alice.id, bob.id))).await;
    ctx.flush(&[&alice]).await;
    assert_eq!(ctx.progress(&alice).await.level.as_option().unwrap().total_xp, 30);
    assert_eq!(ctx.counter(&alice, "bumps").await, 2);
}

#[tokio::test]
async fn a_source_stops_paying_at_its_daily_cap_but_keeps_counting() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;

    // Stamps pay 2 XP each, up to 20 a day.
    ctx.send_stamps(&alice, 9).await;
    ctx.flush(&[&alice]).await;
    assert_eq!(ctx.source_today(&alice, "stamps").await, (18, false));

    ctx.send_stamps(&alice, 21).await;
    ctx.flush(&[&alice]).await;
    assert_eq!(ctx.source_today(&alice, "stamps").await, (20, true));
    assert_eq!(ctx.progress(&alice).await.level.as_option().unwrap().total_xp, 20);
    // The counter behind missions and badges is not capped.
    assert_eq!(ctx.counter(&alice, "stamps_sent").await, 30);

    // Another source is not affected by the stamps cap.
    ctx.feed(event("notes.note.created", Some(alice.id), json!({ "note_id": Uuid::now_v7(), "author_id": alice.id, "visibility": 1, "reply_to_id": null, "quote_of_id": null }))).await;
    ctx.flush(&[&alice]).await;
    assert_eq!(ctx.source_today(&alice, "notes").await, (8, false));
    assert_eq!(ctx.progress(&alice).await.xp_today, 28);
}

#[tokio::test]
async fn levelling_up_grants_coins_once_and_tells_the_player() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let friends: Vec<_> = futures::future::join_all((0..5).map(|_| ctx.app.user("friend"))).await;
    let mut listener = ctx.app.listen_user(alice.id).await;

    // Five new friends at 20 XP each: exactly the 100 XP of level 2.
    for friend in &friends {
        ctx.friend_accepted(&alice, friend).await;
    }
    ctx.flush(&[&alice]).await;

    let progress = ctx.progress(&alice).await;
    let level = progress.level.as_option().unwrap();
    assert_eq!((level.level, level.total_xp, level.xp_into_level, level.xp_for_next_level), (2, 100, 0, 150));
    assert_eq!(ctx.source_today(&alice, "friends").await, (100, true));
    assert_eq!(ctx.coins(&alice).await, 20, "the level-up reward");

    // One coalesced XP push for the five events, then the level-up.
    let pushed = pushes(&mut listener).await;
    let gained: Vec<&XpGained> = pushed
        .iter()
        .filter_map(|push| if let Push::XpGained(gained) = push { Some(gained.as_ref()) } else { None })
        .collect();
    assert_eq!(gained.len(), 1, "one push for everything earned since the last one");
    assert_eq!((gained[0].xp, gained[0].xp_today), (100, 100));
    assert_eq!(
        gained[0].sources.iter().map(|source| (source.source.as_str(), source.xp)).collect::<Vec<_>>(),
        vec![("friends", 100)]
    );
    assert_eq!(gained[0].level.as_option().unwrap().level, 2);
    let level_ups: Vec<&LevelUp> =
        pushed.iter().filter_map(|push| if let Push::LevelUp(up) = push { Some(up.as_ref()) } else { None }).collect();
    assert_eq!(level_ups.len(), 1);
    assert_eq!((level_ups[0].level, level_ups[0].coins), (2, 20));

    // More friends on a capped source: no XP, no second reward.
    ctx.friend_accepted(&alice, &ctx.app.user("sixth").await).await;
    ctx.flush(&[&alice]).await;
    assert_eq!(ctx.progress(&alice).await.level.as_option().unwrap().total_xp, 100);
    assert_eq!(ctx.coins(&alice).await, 20);
    assert_eq!(ctx.counter(&alice, "friends_added").await, 6);
}

#[tokio::test]
async fn every_fifth_level_is_news_for_the_friends_feed() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    // 690 XP: ten short of level 5.
    sqlx::query("INSERT INTO quest_profiles (user_id, xp, level, rewarded_level) VALUES ($1, 690, 4, 4)")
        .bind(alice.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.friend_accepted(&alice, &ctx.app.user("friend").await).await;
    ctx.flush(&[&alice]).await;

    assert_eq!(ctx.progress(&alice).await.level.as_option().unwrap().level, 5);
    let levels: Vec<serde_json::Value> = ctx
        .app
        .events_by("activity.recorded", alice.id)
        .await
        .into_iter()
        .map(|event| event.data)
        .filter(|data| data["kind"] == "quest.level_up")
        .collect();
    assert_eq!(levels.len(), 1);
    assert_eq!(
        (levels[0]["data"]["level"].as_i64(), levels[0]["audience"].as_str(), levels[0]["summary"]["ja"].as_str()),
        (Some(5), Some("friends"), Some("レベル5になりました!"))
    );
    assert!(ctx.badge_keys(&alice).await.contains(&"level_bronze".to_owned()), "level 5 is also a badge");
    assert_eq!(ctx.coins(&alice).await, 20);
}

#[tokio::test]
async fn unknown_and_malformed_events_are_ignored() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    for strange in [
        event("nobody.knows.this", Some(alice.id), json!({ "user_id": alice.id })),
        event("social.bump.created", Some(alice.id), json!({ "user_ids": "not a list", "friends": true })),
        event("social.bump.created", Some(alice.id), json!("a string")),
        event("social.friend.accepted", Some(alice.id), json!({ "user_id": "not-a-uuid", "friend_id": alice.id })),
        event("notify.requested", Some(alice.id), json!({ "kind": "social.map_reaction" })),
        event(
            "notify.requested",
            Some(alice.id),
            json!({ "kind": "chat.message", "user_id": Uuid::now_v7(), "actor_id": alice.id }),
        ),
        event("activity.recorded", Some(alice.id), json!({ "kind": "wallet.bonus", "user_id": alice.id })),
        event("identity.user.deleted", None, json!({ "user": "wrong field" })),
        event(
            "chat.conversation.created",
            Some(alice.id),
            json!({ "conversation_id": Uuid::now_v7(), "kind": 1, "member_ids": [alice.id] }),
        ),
    ] {
        ctx.quest
            .handle_event(strange.clone())
            .await
            .unwrap_or_else(|e| panic!("{} was not a no-op: {e}", strange.topic));
    }
    ctx.quest.handle_analytics(AnalyticsEvent::new("video.watch").user(Some(alice.id))).await.unwrap();
    ctx.quest.handle_analytics(AnalyticsEvent::new("chat.message.send")).await.unwrap();
    ctx.flush(&[&alice]).await;
    assert_eq!(ctx.progress(&alice).await.level.as_option().unwrap().total_xp, 0);
}

#[tokio::test]
async fn events_too_old_to_be_recognised_again_are_not_counted() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.feed(event_at("social.bump.created", Some(alice.id), bump(alice.id, bob.id), Utc::now() - Duration::days(3)))
        .await;
    ctx.flush(&[&alice]).await;
    assert_eq!(ctx.progress(&alice).await.level.as_option().unwrap().total_xp, 0);
    // Yesterday's event, arriving late, still counts — for yesterday.
    ctx.feed(event_at("social.bump.created", Some(alice.id), bump(alice.id, bob.id), Utc::now() - Duration::hours(24)))
        .await;
    ctx.flush(&[&alice]).await;
    let progress = ctx.progress(&alice).await;
    assert_eq!((progress.level.as_option().unwrap().total_xp, progress.xp_today), (15, 0));
}

#[tokio::test]
async fn deleted_accounts_earn_nothing() {
    let ctx = start().await;
    let (ghost, bob) = (ctx.app.user("ghost").await, ctx.app.user("bob").await);
    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1")
        .bind(ghost.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.feed(event("social.bump.created", Some(bob.id), bump(ghost.id, bob.id))).await;
    ctx.quest.flush_user(ghost.id).await.unwrap();
    ctx.flush(&[&bob]).await;
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM quest_xp_days WHERE user_id = $1")
        .bind(ghost.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    assert_eq!(ctx.progress(&bob).await.xp_today, 15);
}

#[tokio::test]
async fn a_day_is_the_players_own_calendar_day() {
    let ctx = start().await;
    // The same instant is a different date at the two ends of the map.
    let (east, west) = (ctx.app.user("east").await, ctx.app.user("west").await);
    ctx.set_zone(&east, "Pacific/Kiritimati").await;
    ctx.set_zone(&west, "Etc/GMT+12").await;
    assert_ne!(today_in("Pacific/Kiritimati"), today_in("Etc/GMT+12"));

    ctx.feed(event("social.bump.created", Some(east.id), bump(east.id, west.id))).await;
    ctx.flush(&[&east, &west]).await;

    for (user, zone) in [(&east, "Pacific/Kiritimati"), (&west, "Etc/GMT+12")] {
        let day: chrono::NaiveDate = sqlx::query_scalar("SELECT day FROM quest_xp_days WHERE user_id = $1")
            .bind(user.id)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
        assert_eq!(day, today_in(zone), "XP is booked on the local date of {zone}");
        let progress = ctx.progress(user).await;
        assert_eq!(
            (progress.today.as_str(), progress.time_zone.as_str(), progress.xp_today),
            (day.to_string().as_str(), zone, 15)
        );
    }
}

#[tokio::test]
async fn the_history_lists_days_newest_first_with_their_sources() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.feed(event_at("social.bump.created", Some(alice.id), bump(alice.id, bob.id), Utc::now() - Duration::hours(24)))
        .await;
    ctx.feed(event("social.bump.created", Some(alice.id), bump(alice.id, bob.id))).await;
    ctx.send_stamps(&alice, 2).await;

    // The history RPC writes what is pending before it reads.
    let first: ListXpHistoryResponse = ctx
        .call(
            QUEST,
            "ListXpHistory",
            Some(&alice),
            &ListXpHistoryRequest {
                page: tokuly_proto::common::PageRequest { limit: 1, ..Default::default() }.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(first.days.len(), 1);
    let today_row = &first.days[0];
    assert_eq!((today_row.date.as_str(), today_row.xp), (today().to_string().as_str(), 19));
    assert_eq!(
        today_row.sources.iter().map(|source| (source.source.as_str(), source.xp)).collect::<Vec<_>>(),
        vec![("friends", 15), ("stamps", 4)]
    );
    let page = first.page.as_option().unwrap();
    assert!(page.has_more);

    let second: ListXpHistoryResponse = ctx
        .call(
            QUEST,
            "ListXpHistory",
            Some(&alice),
            &ListXpHistoryRequest {
                page: tokuly_proto::common::PageRequest {
                    limit: 1,
                    cursor: page.next_cursor.clone(),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(second.days.len(), 1);
    assert_eq!(
        (second.days[0].date.as_str(), second.days[0].xp),
        ((today() - Duration::days(1)).to_string().as_str(), 15)
    );
    assert!(!second.page.as_option().unwrap().has_more);

    assert_error(
        ctx.call::<_, ListXpHistoryResponse>(QUEST, "ListXpHistory", None, &ListXpHistoryRequest::default()).await,
        "unauthenticated",
        "SIGN_IN_REQUIRED",
    );
    let bad_cursor = ListXpHistoryRequest {
        page: tokuly_proto::common::PageRequest { cursor: "!!!".into(), ..Default::default() }.into(),
        ..Default::default()
    };
    assert_error(
        ctx.call::<_, ListXpHistoryResponse>(QUEST, "ListXpHistory", Some(&alice), &bad_cursor).await,
        "invalid_argument",
        "INVALID_CURSOR",
    );
    assert_error(
        ctx.call::<_, GetProgressResponse>(QUEST, "GetProgress", None, &GetProgressRequest::default()).await,
        "unauthenticated",
        "SIGN_IN_REQUIRED",
    );
}

#[tokio::test]
async fn the_rule_table_is_public() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    ctx.send_stamps(&alice, 1).await;

    let table: ListEarningRulesResponse =
        ctx.call(QUEST, "ListEarningRules", None, &ListEarningRulesRequest::default()).await.unwrap();
    let chat = table.sources.iter().find(|source| source.key == "chat").unwrap();
    assert_eq!((chat.daily_cap, chat.name.as_option().unwrap().ja.as_str()), (40, "チャット"));
    assert_eq!(table.sources.iter().find(|source| source.key == "missions").unwrap().daily_cap, 0);

    let rule = |key: &str| {
        table.rules.iter().find(|rule| rule.key == key).unwrap_or_else(|| panic!("rule {key} is not listed"))
    };
    let message = rule("chat.message");
    assert_eq!((message.source.as_str(), message.xp, message.counts_for_streak), ("chat", 2, true));
    assert_eq!(message.text.as_option().unwrap().ja, "メッセージを送る");
    assert!(message.per_unit.as_option().is_none());
    let study = rule("study.session");
    assert_eq!((study.xp, study.per_unit.as_option().unwrap().en.as_str()), (1, "minute"));
    assert!(rule("stamps.sent").live, "a stamp was just sent on this server");
    assert_eq!((rule("streaks.day").xp, rule("streaks.day").source.as_str()), (5, "streaks"));
    // Rules that only count for somebody else (likes received) are not ways to earn XP.
    assert!(!table.rules.iter().any(|rule| rule.key == "reactions.reacted"));

    let curve = table.curve.as_option().unwrap();
    assert_eq!(
        (curve.first_level_xp, curve.step_xp, curve.level_up_coins, curve.milestone_level_coins),
        (100, 50, 20, 100)
    );
    assert_eq!(&curve.thresholds[..3], &[100, 250, 450]);
}

#[tokio::test]
async fn analytics_events_reach_the_rules_through_their_stream() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    // What the stamp domain does on every send.
    for _ in 0..3 {
        ctx.app
            .core
            .analytics
            .track(
                AnalyticsEvent::new("stamp.send")
                    .user(Some(alice.id))
                    .subject("stamp", Uuid::now_v7())
                    .props(json!({ "context": "chat" })),
            )
            .await;
    }
    // Events the table does not name stay with the analytics sink.
    ctx.app.core.analytics.track(AnalyticsEvent::new("video.view").user(Some(alice.id))).await;

    // The stream is shared by the tests of this binary: wait for ours.
    let mut stamps = 0;
    for _ in 0..40 {
        ctx.quest.drain_analytics().await.unwrap();
        ctx.flush(&[&alice]).await;
        stamps = ctx.counter(&alice, "stamps_sent").await;
        if stamps >= 3 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    assert_eq!(stamps, 3);
    assert_eq!(ctx.source_today(&alice, "stamps").await, (6, false));
}

#[tokio::test]
async fn the_flush_worker_writes_days_whose_delay_is_over() {
    // No delay to speak of, so the worker's own round picks the day up.
    let ctx = start_with(tokuly_quest::QuestConfig { flush_delay_ms: 100, ..Default::default() }).await;
    let alice = ctx.app.user("alice").await;
    ctx.send_stamps(&alice, 2).await;
    let mut written = 0;
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        ctx.quest.flush_due().await.unwrap();
        written =
            sqlx::query_scalar::<_, i64>("SELECT COALESCE(sum(xp), 0)::bigint FROM quest_xp_days WHERE user_id = $1")
                .bind(alice.id)
                .fetch_one(&ctx.app.core.db)
                .await
                .unwrap();
        if written > 0 {
            break;
        }
    }
    assert_eq!(written, 4);
    // Nothing is left queued for the player.
    ctx.flush(&[&alice]).await;
    assert_eq!(ctx.progress(&alice).await.level.as_option().unwrap().total_xp, 4);
}
