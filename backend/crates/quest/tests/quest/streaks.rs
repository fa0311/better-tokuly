//! Friend streaks: a day needs both friends, each on their own calendar;
//! expiry, reminders, the paid restore, and the end of a friendship.
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, NaiveDate, Utc};
use serde_json::json;
use tokuly_core::{analytics::AnalyticsEvent, prelude::*};
use tokuly_proto::{
    identity::{
        GetProfileRequest as IdentityProfileRequest, GetProfileResponse as IdentityProfileResponse, get_profile_request,
    },
    quest::{quest_event::Event as Push, *},
};
use tokuly_quest::DirectChats;
use tokuly_testkit::{FrameListener, TestUser};

use crate::common::*;

async fn list(ctx: &Ctx, user: &TestUser, limit: i32, cursor: &str) -> ListStreaksResponse {
    let page = tokuly_proto::common::PageRequest { limit, cursor: cursor.to_owned(), ..Default::default() };
    ctx.call(STREAKS, "ListStreaks", Some(user), &ListStreaksRequest { page: page.into(), ..Default::default() })
        .await
        .unwrap()
}

/// The streak changes a listener received.
async fn changes(listener: &mut FrameListener) -> Vec<(Option<StreakChange>, i32)> {
    pushes(listener)
        .await
        .into_iter()
        .filter_map(|push| {
            if let Push::StreakChanged(changed) = push {
                Some((changed.change.as_known(), changed.streak.as_option().map_or(0, |streak| streak.days)))
            } else {
                None
            }
        })
        .collect()
}

#[tokio::test]
async fn a_streak_day_needs_something_from_both_friends() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    let (mut to_alice, mut to_bob) = (ctx.app.listen_user(alice.id).await, ctx.app.listen_user(bob.id).await);

    // Alice reacts to Bob, more than once: still only her part.
    ctx.map_reaction(&alice, &bob).await;
    ctx.map_reaction(&alice, &bob).await;
    let waiting = ctx.streak(&alice, &bob).await;
    assert_eq!((waiting.days, waiting.state.as_known()), (0, Some(StreakState::STREAK_STATE_NONE)));
    assert_eq!(waiting.friend.as_option().unwrap().id, bob.id.to_string());

    // Bob answers: the day is complete for both.
    ctx.map_reaction(&bob, &alice).await;
    for (viewer, friend) in [(&alice, &bob), (&bob, &alice)] {
        let streak = ctx.streak(viewer, friend).await;
        assert_eq!((streak.days, streak.best_days, streak.next_milestone), (1, 1, 3));
        assert_eq!(streak.state, StreakState::STREAK_STATE_ACTIVE);
        // Today is done; tomorrow is both friends' turn again, until tomorrow's midnight.
        assert_eq!(streak.waiting_on, StreakTurn::STREAK_TURN_BOTH);
        let expires_in = tokuly_core::time::from_ts(streak.expires_at.as_option().unwrap()) - Utc::now();
        assert!(
            expires_in > Duration::hours(24) && expires_in <= Duration::hours(48),
            "ends at the midnight after tomorrow: {expires_in}"
        );
        assert_eq!(streak.started_on, today().to_string());
    }
    // More of the same today changes nothing.
    ctx.map_reaction(&alice, &bob).await;
    ctx.map_reaction(&bob, &alice).await;
    assert_eq!(ctx.streak(&alice, &bob).await.days, 1);

    ctx.flush(&[&alice, &bob]).await;
    // Reacting to the same friend counts once a day (3 XP), however often; the streak day gave each 5 XP.
    assert_eq!(ctx.source_today(&alice, "friends").await, (3, false));
    assert_eq!(ctx.source_today(&bob, "friends").await, (3, false));
    for user in [&alice, &bob] {
        assert_eq!(ctx.source_today(user, "streaks").await, (5, false));
        assert_eq!(ctx.counter(user, "streak_days").await, 1);
        assert_eq!(ctx.counter(user, "friends_reached").await, 1, "one friend reached today, however many reactions");
    }
    for listener in [&mut to_alice, &mut to_bob] {
        assert_eq!(changes(listener).await, vec![(Some(StreakChange::STREAK_CHANGE_EXTENDED), 1)]);
    }
}

#[tokio::test]
async fn only_friends_keep_streaks() {
    let ctx = start().await;
    let (alice, stranger) = (ctx.app.user("alice").await, ctx.app.user("stranger").await);
    ctx.map_reaction(&alice, &stranger).await;
    ctx.map_reaction(&stranger, &alice).await;
    ctx.flush(&[&alice, &stranger]).await;
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM quest_streaks WHERE user_lo IN ($1, $2) OR user_hi IN ($1, $2)")
            .bind(alice.id)
            .bind(stranger.id)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(rows, 0);
    assert_eq!(ctx.counter(&alice, "friends_reached").await, 0);
    // The reaction itself still counts as a reaction.
    assert_eq!(ctx.counter(&alice, "map_reactions").await, 1);

    let request = GetStreakRequest { user_id: stranger.id.to_string(), ..Default::default() };
    assert_error(
        ctx.call::<_, GetStreakResponse>(STREAKS, "GetStreak", Some(&alice), &request).await,
        "failed_precondition",
        "NOT_FRIENDS",
    );
    let own = GetStreakRequest { user_id: alice.id.to_string(), ..Default::default() };
    assert_error(
        ctx.call::<_, GetStreakResponse>(STREAKS, "GetStreak", Some(&alice), &own).await,
        "failed_precondition",
        "NOT_FRIENDS",
    );
    assert_error(
        ctx.call::<_, GetStreakResponse>(STREAKS, "GetStreak", None, &request).await,
        "unauthenticated",
        "SIGN_IN_REQUIRED",
    );
    assert_error(ctx.restore(&alice, &stranger).await, "failed_precondition", "NOT_FRIENDS");
    assert_error(
        ctx.call::<_, ListStreaksResponse>(STREAKS, "ListStreaks", None, &ListStreaksRequest::default()).await,
        "unauthenticated",
        "SIGN_IN_REQUIRED",
    );
    assert_error(
        ctx.call::<_, RestoreStreakResponse>(
            STREAKS,
            "RestoreStreak",
            None,
            &RestoreStreakRequest { user_id: stranger.id.to_string(), ..Default::default() },
        )
        .await,
        "unauthenticated",
        "SIGN_IN_REQUIRED",
    );
}

#[tokio::test]
async fn a_streak_grows_day_by_day_and_milestones_come_with_a_badge() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    // Two days together already, up to yesterday.
    ctx.seed_streak(&alice, &bob, 2, 1).await;
    let mut to_alice = ctx.app.listen_user(alice.id).await;

    // Bob has done today's part; it is Alice's turn.
    ctx.map_reaction(&bob, &alice).await;
    let waiting = ctx.streak(&alice, &bob).await;
    assert_eq!((waiting.days, waiting.waiting_on.as_known()), (2, Some(StreakTurn::STREAK_TURN_MINE)));
    assert_eq!(ctx.streak(&bob, &alice).await.waiting_on, StreakTurn::STREAK_TURN_THEIRS);

    // Hers makes it three: the first milestone.
    ctx.map_reaction(&alice, &bob).await;
    let streak = ctx.streak(&alice, &bob).await;
    assert_eq!((streak.days, streak.best_days, streak.next_milestone), (3, 3, 7));
    for user in [&alice, &bob] {
        assert!(ctx.badge_keys(user).await.contains(&"streak_3".to_owned()), "the three-day badge goes to both");
    }
    assert!(changes(&mut to_alice).await.contains(&(Some(StreakChange::STREAK_CHANGE_MILESTONE), 3)));
    assert_eq!(
        ctx.notifications(&alice, "quest.badge")
            .await
            .iter()
            .filter(|notice| notice["data"]["badge_key"] == "streak_3")
            .count(),
        1
    );
}

#[tokio::test]
async fn what_arrives_late_counts_for_the_day_it_happened() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    let mut to_bob = ctx.app.listen_user(bob.id).await;

    // Yesterday's reactions reach the domain only now.
    let yesterday = Utc::now() - Duration::hours(24);
    ctx.map_reaction_at(&alice, &bob, yesterday).await;
    ctx.map_reaction_at(&bob, &alice, yesterday).await;
    let streak = ctx.streak(&alice, &bob).await;
    assert_eq!((streak.days, streak.started_on.as_str()), (1, (today() - Duration::days(1)).to_string().as_str()));
    // Today's follow: two days in a row.
    ctx.map_reaction(&alice, &bob).await;
    ctx.map_reaction(&bob, &alice).await;
    assert_eq!(ctx.streak(&alice, &bob).await.days, 2);
    assert_eq!(
        changes(&mut to_bob).await,
        vec![(Some(StreakChange::STREAK_CHANGE_EXTENDED), 1), (Some(StreakChange::STREAK_CHANGE_EXTENDED), 2)]
    );

    // The streak XP of each day is booked on that day.
    ctx.flush(&[&alice]).await;
    let days: Vec<(NaiveDate, i32)> =
        sqlx::query_as("SELECT day, xp FROM quest_xp_days WHERE user_id = $1 AND source = 'streaks' ORDER BY day")
            .bind(alice.id)
            .fetch_all(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(days, vec![(today() - Duration::days(1), 5), (today(), 5)]);
}

#[tokio::test]
async fn each_friend_does_their_part_on_their_own_calendar_day() {
    let ctx = start().await;
    // Tokyo is 19 hours ahead of Honolulu.
    let (east, west) = (ctx.app.user("east").await, ctx.app.user("west").await);
    ctx.set_zone(&west, "Pacific/Honolulu").await;
    ctx.befriend(&east, &west).await;

    // 19 hours ago it was, in Tokyo, the date it is in Honolulu now.
    let then = Utc::now() - Duration::hours(19);
    assert_eq!(then.with_timezone(&chrono_tz::Asia::Tokyo).date_naive(), today_in("Pacific/Honolulu"));
    ctx.map_reaction_at(&east, &west, then).await;
    assert_eq!(ctx.streak(&east, &west).await.days, 0);
    // West answers most of a day later by the world's clock — on the same date of their own calendar.
    ctx.map_reaction(&west, &east).await;
    let streak = ctx.streak(&west, &east).await;
    assert_eq!((streak.days, streak.started_on.as_str()), (1, today_in("Pacific/Honolulu").to_string().as_str()));

    // The other way round: at the two ends of the map, two reactions at the
    // same instant are parts of two different dates.
    let (north, south) = (ctx.app.user("north").await, ctx.app.user("south").await);
    ctx.set_zone(&north, "Pacific/Kiritimati").await;
    ctx.set_zone(&south, "Etc/GMT+12").await;
    ctx.befriend(&north, &south).await;
    ctx.map_reaction(&north, &south).await;
    ctx.map_reaction(&south, &north).await;
    assert_eq!(ctx.streak(&north, &south).await.days, 0);
    let (lo, hi) = if north.id < south.id { (&north, &south) } else { (&south, &north) };
    let parts: (Vec<NaiveDate>, Vec<NaiveDate>) =
        sqlx::query_as("SELECT lo_days, hi_days FROM quest_streaks WHERE user_lo = $1 AND user_hi = $2")
            .bind(lo.id)
            .bind(hi.id)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    let zone_of = |user: &TestUser| if user.id == north.id { "Pacific/Kiritimati" } else { "Etc/GMT+12" };
    // The one who is ahead has their date on record, waiting for the other
    // to get there; the other's date can no longer be matched and is settled.
    assert!(
        parts.0.iter().all(|day| *day == today_in(zone_of(lo)))
            && parts.1.iter().all(|day| *day == today_in(zone_of(hi)))
    );
    assert_eq!(parts.0.len() + parts.1.len(), 1);
}

#[tokio::test]
async fn a_missed_day_ends_the_streak() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    let mut to_alice = ctx.app.listen_user(alice.id).await;
    // Five days, the last one three days ago: the day before yesterday was missed.
    ctx.seed_streak(&alice, &bob, 5, 3).await;

    // Read at any time, the streak is over (too long ago to restore).
    let seen = ctx.streak(&alice, &bob).await;
    assert_eq!((seen.days, seen.best_days, seen.lost_days), (0, 5, 0));
    assert_eq!(seen.state, StreakState::STREAK_STATE_NONE);

    ctx.quest.sweep_streaks().await.unwrap();
    let (count, lost_count): (i32, i32) =
        sqlx::query_as("SELECT count, lost_count FROM quest_streaks WHERE user_lo IN ($1, $2) AND user_hi IN ($1, $2)")
            .bind(alice.id)
            .bind(bob.id)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!((count, lost_count), (0, 5));
    assert!(changes(&mut to_alice).await.contains(&(Some(StreakChange::STREAK_CHANGE_LOST), 0)));
    assert!(list(&ctx, &alice, 0, "").await.streaks.is_empty());

    // Both come back: a new streak starts at one, the best stays.
    ctx.map_reaction(&alice, &bob).await;
    ctx.map_reaction(&bob, &alice).await;
    let streak = ctx.streak(&alice, &bob).await;
    assert_eq!((streak.days, streak.best_days), (1, 5));
}

#[tokio::test]
async fn a_lost_streak_can_be_restored_once_a_month_for_coins() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);
    ctx.befriend(&alice, &bob).await;
    ctx.befriend(&alice, &carol).await;
    // Both streaks missed yesterday: they ended at midnight, less than a day ago.
    ctx.seed_streak(&alice, &bob, 5, 2).await;
    ctx.seed_streak(&alice, &carol, 9, 2).await;
    let (mut to_alice, mut to_bob) = (ctx.app.listen_user(alice.id).await, ctx.app.listen_user(bob.id).await);

    let lost = ctx.streak(&alice, &bob).await;
    assert_eq!((lost.days, lost.lost_days, lost.state.as_known()), (0, 5, Some(StreakState::STREAK_STATE_LOST)));
    let (lost_at, until) = (
        tokuly_core::time::from_ts(lost.lost_at.as_option().unwrap()),
        tokuly_core::time::from_ts(lost.restorable_until.as_option().unwrap()),
    );
    assert!(lost_at <= Utc::now() && until > Utc::now() && until - lost_at == Duration::hours(24));
    let offer: GetStreakResponse = ctx
        .call(
            STREAKS,
            "GetStreak",
            Some(&alice),
            &GetStreakRequest { user_id: bob.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    let offer = offer.restore.as_option().unwrap();
    assert_eq!((offer.cost_coins, offer.available, offer.available_again_on.as_str()), (100, true, ""));

    // It costs coins, and Alice has none.
    assert_error(ctx.restore(&alice, &bob).await, "failed_precondition", "INSUFFICIENT_COINS");
    assert_eq!(ctx.streak(&alice, &bob).await.state, StreakState::STREAK_STATE_LOST);

    ctx.fund(&alice, 150).await;
    let restored = ctx.restore(&alice, &bob).await.unwrap();
    assert_eq!(restored.coins_spent, 100);
    let streak = restored.streak.as_option().unwrap();
    assert_eq!(
        (streak.days, streak.lost_days, streak.waiting_on.as_known()),
        (5, 0, Some(StreakTurn::STREAK_TURN_BOTH))
    );
    assert!(matches!(
        streak.state.as_known(),
        Some(StreakState::STREAK_STATE_ACTIVE | StreakState::STREAK_STATE_EXPIRING)
    ));
    assert!(tokuly_core::time::from_ts(streak.expires_at.as_option().unwrap()) > Utc::now(), "both have until tonight");
    assert_eq!(ctx.coins(&alice).await, 50);
    // Bob sees it too, and is told who brought it back.
    assert_eq!(ctx.streak(&bob, &alice).await.days, 5);
    let notices = ctx.notifications(&bob, "quest.streak_restored").await;
    assert_eq!(notices.len(), 1);
    assert_eq!(
        (notices[0]["actor_id"].as_str(), notices[0]["data"]["days"].as_i64()),
        (Some(alice.id.to_string().as_str()), Some(5))
    );
    assert!(changes(&mut to_alice).await.contains(&(Some(StreakChange::STREAK_CHANGE_RESTORED), 5)));
    assert!(changes(&mut to_bob).await.contains(&(Some(StreakChange::STREAK_CHANGE_RESTORED), 5)));
    assert!(ctx.badge_keys(&alice).await.contains(&"phoenix".to_owned()), "the secret badge for restoring");

    // Asking again restores nothing more and costs nothing.
    let repeated = ctx.restore(&alice, &bob).await.unwrap();
    assert_eq!((repeated.coins_spent, repeated.streak.as_option().unwrap().days), (0, 5));
    assert_eq!(ctx.coins(&alice).await, 50);
    // The streak is running again: Bob has nothing to restore.
    ctx.fund(&bob, 200).await;
    assert_error(ctx.restore(&bob, &alice).await, "failed_precondition", "STREAK_NOT_LOST");
    assert_eq!(ctx.coins(&bob).await, 200);

    // One restore a month: the streak with Carol stays lost for Alice...
    ctx.fund(&alice, 200).await;
    assert_error(ctx.restore(&alice, &carol).await, "failed_precondition", "RESTORE_ALREADY_USED");
    assert_eq!(ctx.coins(&alice).await, 250);
    let listed = list(&ctx, &alice, 0, "").await;
    let offer = listed.restore.as_option().unwrap();
    assert!(!offer.available);
    let next_month: NaiveDate = offer.available_again_on.parse().unwrap();
    assert!(next_month > today() && chrono::Datelike::day(&next_month) == 1);
    // ...but Carol has her own allowance.
    ctx.fund(&carol, 100).await;
    assert_eq!(ctx.restore(&carol, &alice).await.unwrap().streak.as_option().unwrap().days, 9);
    assert_eq!(ctx.coins(&carol).await, 0);
}

#[tokio::test]
async fn a_streak_lost_long_ago_cannot_be_restored() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    ctx.seed_streak(&alice, &bob, 12, 4).await;
    ctx.fund(&alice, 500).await;
    assert_error(ctx.restore(&alice, &bob).await, "failed_precondition", "RESTORE_WINDOW_OVER");
    assert_eq!(ctx.coins(&alice).await, 500);
    // And a pair that never had a streak has nothing to restore.
    let carol = ctx.app.user("carol").await;
    ctx.befriend(&alice, &carol).await;
    assert_error(ctx.restore(&alice, &carol).await, "failed_precondition", "STREAK_NOT_LOST");
}

#[tokio::test]
async fn friends_are_reminded_once_before_their_own_midnight() {
    let ctx = start().await;
    let zone = evening_zone();
    // Alice and Bob have both not done today's part; Carol has, Dave has not;
    // Erin switched reminders off; Fay and Gus have only just started.
    let (alice, bob, carol, dave) = (
        ctx.app.user("alice").await,
        ctx.app.user("bob").await,
        ctx.app.user("carol").await,
        ctx.app.user("dave").await,
    );
    let (erin, frank, fay, gus) =
        (ctx.app.user("erin").await, ctx.app.user("frank").await, ctx.app.user("fay").await, ctx.app.user("gus").await);
    let _: UpdatePreferencesResponse = ctx
        .call(
            QUEST,
            "UpdatePreferences",
            Some(&erin),
            &UpdatePreferencesRequest { streak_reminders: Some(false), ..Default::default() },
        )
        .await
        .unwrap();
    for (a, b) in [(&alice, &bob), (&carol, &dave), (&erin, &frank), (&fay, &gus)] {
        ctx.befriend(a, b).await;
    }
    let mut to_carol = ctx.app.listen_user(carol.id).await;
    ctx.running_today(&alice, &bob, &zone, 5, None).await;
    ctx.running_today(&carol, &dave, &zone, 8, Some(&carol)).await;
    ctx.running_today(&erin, &frank, &zone, 6, None).await;
    ctx.running_today(&fay, &gus, &zone, 2, None).await;

    // Less than four hours to go: the hourglass.
    let streak = ctx.streak(&alice, &bob).await;
    assert_eq!(
        (streak.days, streak.state.as_known(), streak.waiting_on.as_known()),
        (5, Some(StreakState::STREAK_STATE_EXPIRING), Some(StreakTurn::STREAK_TURN_BOTH))
    );
    assert_eq!(ctx.streak(&carol, &dave).await.waiting_on, StreakTurn::STREAK_TURN_THEIRS);

    ctx.quest.sweep_streaks().await.unwrap();
    ctx.quest.sweep_streaks().await.unwrap();

    let reminders = |user: &TestUser| {
        let user = user.clone();
        let ctx = &ctx;
        async move { ctx.notifications(&user, "quest.streak_expiring").await }
    };
    for (user, friend) in [(&alice, &bob), (&bob, &alice)] {
        let notices = reminders(user).await;
        assert_eq!(notices.len(), 1, "one reminder, however often the sweep runs");
        assert_eq!(
            (notices[0]["actor_id"].as_str(), notices[0]["data"]["days"].as_i64()),
            (Some(friend.id.to_string().as_str()), Some(5))
        );
        assert_eq!(notices[0]["urgent"], false, "a reminder waits for quiet hours to end");
        assert!(notices[0]["title"]["ja"].as_str().unwrap().contains("もうすぐ途切れちゃう"));
    }
    // Carol did her part: only Dave has something to do.
    assert!(reminders(&carol).await.is_empty());
    assert_eq!(reminders(&dave).await.len(), 1);
    // Carol's devices still learn that the hourglass is running.
    assert!(changes(&mut to_carol).await.contains(&(Some(StreakChange::STREAK_CHANGE_EXPIRING), 8)));
    // Erin does not want reminders; Frank gets his.
    assert!(reminders(&erin).await.is_empty());
    assert_eq!(reminders(&frank).await.len(), 1);
    // Two days are not yet a streak worth a notification.
    assert!(reminders(&fay).await.is_empty() && reminders(&gus).await.is_empty());

    // Dave makes it in time: the day completes and nobody is reminded again.
    ctx.map_reaction(&dave, &carol).await;
    assert_eq!(ctx.streak(&carol, &dave).await.days, 9);
    ctx.quest.sweep_streaks().await.unwrap();
    assert_eq!(reminders(&dave).await.len(), 1);
}

#[tokio::test]
async fn ending_a_friendship_ends_the_streak() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    ctx.seed_streak(&alice, &bob, 4, 1).await;
    assert_eq!(ctx.streak(&alice, &bob).await.days, 4);
    let mut to_bob = ctx.app.listen_user(bob.id).await;

    ctx.unfriend(&alice, &bob).await;
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM quest_streaks WHERE user_lo IN ($1, $2) AND user_hi IN ($1, $2)")
            .bind(alice.id)
            .bind(bob.id)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(rows, 0);
    assert!(changes(&mut to_bob).await.contains(&(Some(StreakChange::STREAK_CHANGE_ENDED), 0)));
    assert_error(
        ctx.call::<_, GetStreakResponse>(
            STREAKS,
            "GetStreak",
            Some(&alice),
            &GetStreakRequest { user_id: bob.id.to_string(), ..Default::default() },
        )
        .await,
        "failed_precondition",
        "NOT_FRIENDS",
    );

    // Friends again: they start from nothing, with no memory of the old one.
    ctx.befriend(&alice, &bob).await;
    let fresh = ctx.streak(&alice, &bob).await;
    assert_eq!((fresh.days, fresh.best_days), (0, 0));
    ctx.map_reaction(&alice, &bob).await;
    ctx.map_reaction(&bob, &alice).await;
    assert_eq!(ctx.streak(&alice, &bob).await.days, 1);
}

#[tokio::test]
async fn streaks_are_listed_longest_first_with_the_restorable_ones_last() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let friends: Vec<TestUser> =
        futures::future::join_all(["long", "short", "lost", "gone", "none"].map(|name| ctx.app.user(name))).await;
    for friend in &friends {
        ctx.befriend(&alice, friend).await;
    }
    ctx.seed_streak(&alice, &friends[0], 7, 1).await;
    ctx.seed_streak(&alice, &friends[1], 3, 1).await;
    ctx.seed_streak(&alice, &friends[2], 5, 2).await; // ended last midnight
    ctx.seed_streak(&alice, &friends[3], 20, 5).await; // ended days ago

    let all = list(&ctx, &alice, 0, "").await;
    let rows: Vec<(String, i32, i32, Option<StreakState>)> = all
        .streaks
        .iter()
        .map(|streak| {
            (streak.friend.as_option().unwrap().id.clone(), streak.days, streak.lost_days, streak.state.as_known())
        })
        .collect();
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert_eq!((rows[0].0.as_str(), rows[0].1), (friends[0].id.to_string().as_str(), 7));
    assert_eq!((rows[1].0.as_str(), rows[1].1), (friends[1].id.to_string().as_str(), 3));
    assert_eq!(
        (rows[2].0.as_str(), rows[2].1, rows[2].2, rows[2].3),
        (friends[2].id.to_string().as_str(), 0, 5, Some(StreakState::STREAK_STATE_LOST))
    );
    assert!(!all.page.as_option().unwrap().has_more);
    assert_eq!(all.streaks[0].friend.as_option().unwrap().handle, friends[0].handle);

    let first = list(&ctx, &alice, 2, "").await;
    assert_eq!(first.streaks.iter().map(|streak| streak.days).collect::<Vec<_>>(), vec![7, 3]);
    let page = first.page.as_option().unwrap();
    assert!(page.has_more);
    let second = list(&ctx, &alice, 2, &page.next_cursor).await;
    assert_eq!(second.streaks.iter().map(|streak| streak.lost_days).collect::<Vec<_>>(), vec![5]);
    assert!(!second.page.as_option().unwrap().has_more);
    let bad = ListStreaksRequest {
        page: tokuly_proto::common::PageRequest { cursor: "!!!".into(), ..Default::default() }.into(),
        ..Default::default()
    };
    assert_error(
        ctx.call::<_, ListStreaksResponse>(STREAKS, "ListStreaks", Some(&alice), &bad).await,
        "invalid_argument",
        "INVALID_CURSOR",
    );
}

#[tokio::test]
async fn a_profile_shows_the_streak_to_the_friend_it_is_with() {
    let ctx = start().await;
    let (alice, bob, stranger) =
        (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("stranger").await);
    ctx.befriend(&alice, &bob).await;
    ctx.seed_streak(&alice, &bob, 12, 1).await;

    let cards_for = |viewer: Option<TestUser>| {
        let ctx = &ctx;
        let owner = alice.id.to_string();
        async move {
            let profile: IdentityProfileResponse = ctx
                .app
                .call(
                    "tokuly.identity.v1.ProfileService/GetProfile",
                    viewer.as_ref(),
                    &IdentityProfileRequest { by: Some(get_profile_request::By::UserId(owner)), ..Default::default() },
                )
                .await
                .unwrap();
            profile.profile.into_option().unwrap().cards
        }
    };
    let seen_by_bob = cards_for(Some(bob.clone())).await;
    let card =
        seen_by_bob.iter().find(|card| card.kind == "quest.streak").expect("Bob sees the streak he has with Alice");
    assert_eq!(card.title.as_option().unwrap().ja, "12日連続");
    assert_eq!((card.data["days"].as_str(), card.data["next_milestone"].as_str()), ("12", "30"));
    assert!(card.emoji == "🔥" || card.emoji == "⌛");
    assert!(seen_by_bob.iter().any(|card| card.kind == "quest.level"));
    // Nobody else does: not a stranger, not a guest, not Alice on her own profile.
    for viewer in [Some(stranger.clone()), None, Some(alice.clone())] {
        assert!(!cards_for(viewer).await.iter().any(|card| card.kind == "quest.streak"));
    }

    let quest_card = |viewer: Option<TestUser>| {
        let ctx = &ctx;
        let owner = alice.id.to_string();
        async move {
            let response: GetProfileResponse = ctx
                .call(QUEST, "GetProfile", viewer.as_ref(), &GetProfileRequest { user_id: owner, ..Default::default() })
                .await
                .unwrap();
            response.profile.into_option().unwrap()
        }
    };
    assert_eq!(quest_card(Some(bob.clone())).await.streak.as_option().unwrap().days, 12);
    assert!(quest_card(Some(stranger.clone())).await.streak.as_option().is_none());
    assert!(quest_card(None).await.streak.as_option().is_none());
}

/// A direct conversation's members, as the chat domain would know them.
struct KnownChat {
    conversation: Uuid,
    members: (Uuid, Uuid),
}

#[async_trait]
impl DirectChats for KnownChat {
    async fn members(&self, conversation_id: Uuid) -> AppResult<Option<(Uuid, Uuid)>> {
        Ok((conversation_id == self.conversation).then_some(self.members))
    }
}

fn message(sender: &TestUser, conversation: Uuid, group: bool) -> AnalyticsEvent {
    AnalyticsEvent::new("chat.message.send")
        .user(Some(sender.id))
        .subject("conversation", conversation)
        .props(json!({ "kind": 1, "group": group, "forwarded": false }))
}

#[tokio::test]
async fn direct_messages_keep_a_streak_and_group_messages_do_not() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;

    // A group both are in: messages earn XP but are aimed at nobody.
    let group = Uuid::now_v7();
    ctx.feed(event(
        "chat.conversation.created",
        Some(alice.id),
        json!({ "conversation_id": group, "kind": 2, "member_ids": [alice.id, bob.id] }),
    ))
    .await;
    ctx.track(message(&alice, group, true)).await;
    ctx.track(message(&bob, group, true)).await;
    assert_eq!(ctx.streak(&alice, &bob).await.days, 0);

    // Their direct conversation, announced when it was opened.
    let direct = Uuid::now_v7();
    ctx.feed(event(
        "chat.conversation.created",
        Some(alice.id),
        json!({ "conversation_id": direct, "kind": 1, "member_ids": [alice.id, bob.id] }),
    ))
    .await;
    ctx.track(message(&alice, direct, false)).await;
    ctx.track(message(&alice, direct, false)).await;
    assert_eq!(ctx.streak(&alice, &bob).await.days, 0, "only Alice wrote so far");
    ctx.track(message(&bob, direct, false)).await;
    assert_eq!(ctx.streak(&alice, &bob).await.days, 1);

    ctx.flush(&[&alice, &bob]).await;
    assert_eq!(ctx.source_today(&alice, "chat").await, (6, false));
    assert_eq!(ctx.counter(&alice, "messages").await, 3);
    assert_eq!(ctx.counter(&alice, "friends_reached").await, 1);
    assert_eq!(ctx.source_today(&bob, "streaks").await, (5, false));
}

#[tokio::test]
async fn conversations_opened_before_the_domain_listened_are_looked_up_through_the_bridge() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    let old = Uuid::now_v7();

    // Nobody announced this conversation: its messages count, but for no streak.
    ctx.track(message(&alice, old, false)).await;
    ctx.track(message(&bob, old, false)).await;
    assert_eq!(ctx.streak(&alice, &bob).await.days, 0);

    // With the chat domain connected, the same conversation resolves.
    let ctx = start().await;
    ctx.quest.set_direct_chats(Arc::new(KnownChat { conversation: old, members: (alice.id, bob.id) }));
    ctx.track(message(&alice, old, false)).await;
    ctx.track(message(&bob, old, false)).await;
    assert_eq!(ctx.streak(&alice, &bob).await.days, 1);
    // A conversation the chat domain does not know either stays unknown.
    ctx.track(message(&alice, Uuid::now_v7(), false)).await;
}
