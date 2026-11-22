//! Leaderboards among friends, and seasons.
use chrono::{Datelike as _, NaiveDate};
use serde_json::json;
use tokuly_proto::quest::*;
use tokuly_testkit::{RpcError, TestUser};

use crate::common::*;

async fn board(
    ctx: &Ctx,
    viewer: &TestUser,
    key: &str,
    period: LeaderboardPeriod,
    limit: i32,
) -> Result<GetLeaderboardResponse, RpcError> {
    ctx.call(
        BOARDS,
        "GetLeaderboard",
        Some(viewer),
        &GetLeaderboardRequest { board: key.to_owned(), period: period.into(), limit, ..Default::default() },
    )
    .await
}

/// `(user, value, rank)` of every row.
fn rows(response: &GetLeaderboardResponse) -> Vec<(String, i64, i32)> {
    response.entries.iter().map(|entry| (entry.user.as_option().unwrap().id.clone(), entry.value, entry.rank)).collect()
}

async fn hide_from_leaderboards(ctx: &Ctx, user: &TestUser) {
    let _: UpdatePreferencesResponse = ctx
        .call(
            QUEST,
            "UpdatePreferences",
            Some(user),
            &UpdatePreferencesRequest { show_on_leaderboards: Some(false), ..Default::default() },
        )
        .await
        .unwrap();
}

/// XP as if earned on `day` (a day of a season long past).
async fn xp_on(ctx: &Ctx, user: &TestUser, day: NaiveDate, xp: i32) {
    sqlx::query("INSERT INTO quest_xp_days (user_id, day, source, xp) VALUES ($1, $2, 'chat', $3)")
        .bind(user.id)
        .bind(day)
        .bind(xp)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
}

async fn study(ctx: &Ctx, user: &TestUser, minutes: i64) {
    ctx.feed(event(
        "school.session.completed",
        Some(user.id),
        json!({ "user_id": user.id, "minutes": minutes, "subject": "math" }),
    ))
    .await;
}

#[tokio::test]
async fn the_xp_board_ranks_the_caller_among_their_friends() {
    let ctx = start().await;
    let (alice, bob, carol, dave, eve) = (
        ctx.app.user("alice").await,
        ctx.app.user("bob").await,
        ctx.app.user("carol").await,
        ctx.app.user("dave").await,
        ctx.app.user("eve").await,
    );
    for friend in [&bob, &carol, &dave] {
        ctx.befriend(&alice, friend).await;
    }
    hide_from_leaderboards(&ctx, &carol).await;
    // Stamps pay 2 XP each: Alice 6, Bob 10, Carol 4; Dave nothing; Eve, a stranger, 20.
    for (user, stamps) in [(&alice, 3), (&bob, 5), (&carol, 2), (&eve, 10)] {
        ctx.send_stamps(user, stamps).await;
    }
    ctx.flush(&[&alice, &bob, &carol, &eve]).await;

    let weekly = board(&ctx, &alice, "xp", LeaderboardPeriod::LEADERBOARD_PERIOD_WEEK, 0).await.unwrap();
    assert_eq!(
        rows(&weekly),
        vec![(bob.id.to_string(), 10, 1), (alice.id.to_string(), 6, 2)],
        "friends only; Carol opted out; Dave has no XP"
    );
    assert_eq!(weekly.ranked_count, 2);
    let me = weekly.me.as_option().unwrap();
    assert_eq!((me.rank, me.value, me.me, me.level), (2, 6, true, 1));
    assert!(weekly.entries[1].me && !weekly.entries[0].me);
    assert_eq!(weekly.board.as_option().unwrap().key, "xp");
    let (start, end): (NaiveDate, NaiveDate) =
        (weekly.period_start.parse().unwrap(), weekly.period_end.parse().unwrap());
    assert_eq!((start.weekday(), end.weekday()), (chrono::Weekday::Mon, chrono::Weekday::Sun));
    assert!(start <= today() && today() <= end);

    // Whoever opted out still sees their own row among the friends who show.
    let carols = board(&ctx, &carol, "xp", LeaderboardPeriod::LEADERBOARD_PERIOD_WEEK, 0).await.unwrap();
    assert_eq!(rows(&carols), vec![(alice.id.to_string(), 6, 1), (carol.id.to_string(), 4, 2)]);
    // Somebody without XP is on the board as themselves, unranked.
    let daves = board(&ctx, &dave, "xp", LeaderboardPeriod::LEADERBOARD_PERIOD_WEEK, 0).await.unwrap();
    assert_eq!(rows(&daves), vec![(alice.id.to_string(), 6, 1)]);
    let me = daves.me.as_option().unwrap();
    assert_eq!((me.rank, me.value, me.me), (0, 0, true));

    // The top can be cut short; the caller's row comes along anyway.
    let top = board(&ctx, &alice, "xp", LeaderboardPeriod::LEADERBOARD_PERIOD_ALL_TIME, 1).await.unwrap();
    assert_eq!(rows(&top), vec![(bob.id.to_string(), 10, 1)]);
    assert_eq!(top.me.as_option().unwrap().rank, 2);
    assert!(top.period_start.is_empty());
    // No period given: the board's first one.
    let default = board(&ctx, &alice, "xp", LeaderboardPeriod::LEADERBOARD_PERIOD_UNSPECIFIED, 0).await.unwrap();
    assert_eq!(default.period, LeaderboardPeriod::LEADERBOARD_PERIOD_WEEK);

    // Equal values share a rank.
    ctx.send_stamps(&alice, 2).await;
    let tied = board(&ctx, &alice, "xp", LeaderboardPeriod::LEADERBOARD_PERIOD_SEASON, 0).await.unwrap();
    assert_eq!(tied.entries.iter().map(|entry| (entry.value, entry.rank)).collect::<Vec<_>>(), vec![(10, 1), (10, 1)]);

    assert_error(
        board(&ctx, &alice, "no_such_board", LeaderboardPeriod::LEADERBOARD_PERIOD_WEEK, 0).await,
        "not_found",
        "BOARD_NOT_FOUND",
    );
    assert_error(
        board(&ctx, &alice, "streak", LeaderboardPeriod::LEADERBOARD_PERIOD_WEEK, 0).await,
        "invalid_argument",
        "PERIOD_NOT_SUPPORTED",
    );
    let request = GetLeaderboardRequest { board: "xp".into(), ..Default::default() };
    assert_error(
        ctx.call::<_, GetLeaderboardResponse>(BOARDS, "GetLeaderboard", None, &request).await,
        "unauthenticated",
        "SIGN_IN_REQUIRED",
    );
    assert_error(
        ctx.call::<_, ListLeaderboardsResponse>(BOARDS, "ListLeaderboards", None, &ListLeaderboardsRequest::default())
            .await,
        "unauthenticated",
        "SIGN_IN_REQUIRED",
    );
}

#[tokio::test]
async fn the_streak_board_ranks_by_the_longest_running_streak() {
    let ctx = start().await;
    let (alice, bob, carol, dave) = (
        ctx.app.user("alice").await,
        ctx.app.user("bob").await,
        ctx.app.user("carol").await,
        ctx.app.user("dave").await,
    );
    ctx.befriend(&alice, &bob).await;
    ctx.befriend(&alice, &carol).await;
    ctx.befriend(&bob, &dave).await;
    ctx.seed_streak(&alice, &bob, 7, 1).await;
    ctx.seed_streak(&alice, &carol, 3, 1).await;
    // Bob's longest streak is with somebody Alice does not know.
    ctx.seed_streak(&bob, &dave, 10, 1).await;

    let streaks = board(&ctx, &alice, "streak", LeaderboardPeriod::LEADERBOARD_PERIOD_ALL_TIME, 0).await.unwrap();
    assert_eq!(
        rows(&streaks),
        vec![(bob.id.to_string(), 10, 1), (alice.id.to_string(), 7, 2), (carol.id.to_string(), 3, 3)]
    );
    assert_eq!(streaks.board.as_option().unwrap().unit.as_option().unwrap().ja, "日");

    // A streak that ran out does not rank, swept or not.
    ctx.seed_streak(&alice, &bob, 7, 3).await;
    let streaks = board(&ctx, &alice, "streak", LeaderboardPeriod::LEADERBOARD_PERIOD_ALL_TIME, 0).await.unwrap();
    assert_eq!(
        rows(&streaks),
        vec![(bob.id.to_string(), 10, 1), (alice.id.to_string(), 3, 2), (carol.id.to_string(), 3, 2)]
    );
}

#[tokio::test]
async fn metric_boards_exist_once_their_events_do() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    study(&ctx, &alice, 45).await;
    study(&ctx, &bob, 90).await;
    study(&ctx, &bob, 30).await;
    ctx.flush(&[&alice, &bob]).await;

    let boards: ListLeaderboardsResponse =
        ctx.call(BOARDS, "ListLeaderboards", Some(&alice), &ListLeaderboardsRequest::default()).await.unwrap();
    let keys: Vec<&str> = boards.boards.iter().map(|board| board.key.as_str()).collect();
    assert!(keys.contains(&"xp") && keys.contains(&"streak"), "{keys:?}");
    assert!(keys.contains(&"study_time"), "study sessions were just reported on this server");
    let study_board = boards.boards.iter().find(|board| board.key == "study_time").unwrap();
    assert_eq!(study_board.unit.as_option().unwrap().en, "min");
    assert_eq!(study_board.periods.len(), 3);

    let minutes = board(&ctx, &alice, "study_time", LeaderboardPeriod::LEADERBOARD_PERIOD_WEEK, 0).await.unwrap();
    assert_eq!(rows(&minutes), vec![(bob.id.to_string(), 120, 1), (alice.id.to_string(), 45, 2)]);
    let all_time = board(&ctx, &alice, "study_time", LeaderboardPeriod::LEADERBOARD_PERIOD_ALL_TIME, 0).await.unwrap();
    assert_eq!(rows(&all_time), rows(&minutes));
    // Study XP is a minute a point, up to the day's cap; the minutes themselves are not capped.
    assert_eq!(ctx.source_today(&bob, "study").await, (60, true));
    assert_eq!(ctx.counter(&bob, "study_sessions").await, 2);
}

#[tokio::test]
async fn the_running_season_ranks_this_months_xp() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);
    ctx.befriend(&alice, &bob).await;
    ctx.befriend(&alice, &carol).await;
    ctx.send_stamps(&alice, 3).await;
    ctx.send_stamps(&bob, 5).await;
    ctx.flush(&[&bob]).await;

    let response: GetSeasonResponse =
        ctx.call(BOARDS, "GetSeason", Some(&alice), &GetSeasonRequest::default()).await.unwrap();
    let season = response.season.as_option().unwrap();
    let month = today().with_day(1).unwrap();
    assert_eq!(season.id, format!("{:04}-{:02}", month.year(), month.month()));
    assert_eq!(season.name.as_option().unwrap().ja, format!("{}年{}月シーズン", month.year(), month.month()));
    assert_eq!(season.starts_on, month.to_string());
    // Carol earned nothing this month: two take part, and the top third of two is one.
    assert_eq!((season.xp, season.rank, season.group_size, season.badge_rank), (6, 2, 2, 1));
    assert!(!season.r#final && !season.badge_won);
    assert_eq!(season.badge.as_option().unwrap().key, format!("season_{:02}", month.month()));
    assert!(tokuly_core::time::from_ts(season.ends_at.as_option().unwrap()) > chrono::Utc::now());

    // Alone, there is nobody to beat.
    let loner = ctx.app.user("loner").await;
    let response: GetSeasonResponse =
        ctx.call(BOARDS, "GetSeason", Some(&loner), &GetSeasonRequest::default()).await.unwrap();
    let season = response.season.as_option().unwrap();
    assert_eq!((season.xp, season.rank, season.group_size, season.badge_rank), (0, 0, 0, 0));
    assert_error(
        ctx.call::<_, GetSeasonResponse>(BOARDS, "GetSeason", None, &GetSeasonRequest::default()).await,
        "unauthenticated",
        "SIGN_IN_REQUIRED",
    );
}

#[tokio::test]
async fn a_finished_season_gives_the_top_third_of_each_group_its_badge() {
    let ctx = start().await;
    // A month nobody else in this test run touches.
    let march = NaiveDate::from_ymd_opt(2001, 3, 1).unwrap();
    let day = NaiveDate::from_ymd_opt(2001, 3, 14).unwrap();
    let (alice, bob, carol, dave) = (
        ctx.app.user("alice").await,
        ctx.app.user("bob").await,
        ctx.app.user("carol").await,
        ctx.app.user("dave").await,
    );
    let (hidden, loner) = (ctx.app.user("hidden").await, ctx.app.user("loner").await);
    let (gina, hal, ida) = (ctx.app.user("gina").await, ctx.app.user("hal").await, ctx.app.user("ida").await);
    for friend in [&bob, &carol, &dave, &hidden] {
        ctx.befriend(&alice, friend).await;
    }
    ctx.befriend(&gina, &hal).await;
    ctx.befriend(&gina, &ida).await;
    hide_from_leaderboards(&ctx, &hidden).await;
    for (user, xp) in [
        (&alice, 300),
        (&bob, 500),
        (&carol, 100),
        (&dave, 50),
        (&hidden, 9000),
        (&loner, 400),
        (&gina, 700),
        (&hal, 20),
        (&ida, 10),
    ] {
        xp_on(&ctx, user, day, xp).await;
    }

    ctx.quest.finalize_season(march).await.unwrap();

    let result = |user: &TestUser| {
        let (db, user_id) = (ctx.app.core.db.clone(), user.id);
        async move {
            sqlx::query_as::<_, (i64, i32, i32, bool)>(
                "SELECT xp, rank, group_size, badge_won FROM quest_season_results WHERE month = $1 AND user_id = $2",
            )
            .bind(march)
            .bind(user_id)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    // Alice among Bob, Carol and Dave (the hidden friend does not compete with them): second of four, and two win.
    assert_eq!(result(&alice).await, (300, 2, 4, true));
    // Bob knows only Alice: first of two.
    assert_eq!(result(&bob).await, (500, 1, 2, true));
    assert_eq!(result(&carol).await, (100, 2, 2, false));
    assert_eq!(result(&dave).await, (50, 2, 2, false));
    // Whoever hides from leaderboards still has their own result.
    assert_eq!(result(&hidden).await, (9000, 1, 2, true));
    // Nobody to compete with: no contest, no badge.
    assert_eq!(result(&loner).await, (400, 1, 1, false));
    assert_eq!(result(&gina).await, (700, 1, 3, true));

    // The badge of the month, the "seasons won" line, and the champion of a group of three.
    let alices = ctx.badge_keys(&alice).await;
    assert!(
        alices.contains(&"season_03".to_owned()) && alices.contains(&"season_regular_bronze".to_owned()),
        "{alices:?}"
    );
    assert!(!alices.contains(&"season_champion".to_owned()));
    assert!(!ctx.badge_keys(&bob).await.contains(&"season_champion".to_owned()), "first of two is not a champion");
    assert!(ctx.badge_keys(&gina).await.contains(&"season_champion".to_owned()));
    assert!(!ctx.badge_keys(&carol).await.contains(&"season_03".to_owned()));
    assert!(!ctx.badge_keys(&loner).await.contains(&"season_03".to_owned()));
    assert_eq!(ctx.counter(&alice, "seasons_won").await, 1);

    // Everyone who had rivals hears how it went; winners with a push.
    let won = ctx.notifications(&alice, "quest.season").await;
    assert_eq!(won.len(), 1);
    assert_eq!(
        (won[0]["push"].as_bool(), won[0]["data"]["rank"].as_i64(), won[0]["data"]["badge_won"].as_bool()),
        (Some(true), Some(2), Some(true))
    );
    assert_eq!(won[0]["title"]["ja"], "2001年3月シーズンが終わりました!");
    let lost = ctx.notifications(&carol, "quest.season").await;
    assert_eq!((lost.len(), lost[0]["push"].as_bool()), (1, Some(false)));
    assert!(ctx.notifications(&loner, "quest.season").await.is_empty());

    let finalized: (bool, i32) =
        sqlx::query_as("SELECT finalized_at IS NOT NULL, participants FROM quest_seasons WHERE month = $1")
            .bind(march)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(finalized, (true, 9));

    // Finalizing again changes nothing.
    ctx.quest.finalize_season(march).await.unwrap();
    assert_eq!(ctx.counter(&alice, "seasons_won").await, 1);
    assert_eq!(ctx.notifications(&alice, "quest.season").await.len(), 1);

    // The result is in the player's history.
    let history: ListSeasonResultsResponse =
        ctx.call(BOARDS, "ListSeasonResults", Some(&alice), &ListSeasonResultsRequest::default()).await.unwrap();
    assert_eq!(history.seasons.len(), 1);
    let season = &history.seasons[0];
    assert_eq!(
        (season.id.as_str(), season.xp, season.rank, season.group_size, season.badge_rank),
        ("2001-03", 300, 2, 4, 2)
    );
    assert!(season.r#final && season.badge_won);
    assert_eq!((season.starts_on.as_str(), season.ends_on.as_str()), ("2001-03-01", "2001-03-31"));
    let badge = season.badge.as_option().unwrap();
    assert!(badge.owned && badge.key == "season_03" && badge.times == 1);
    assert_eq!(badge.name.as_option().unwrap().ja, "弥生シーズン");

    // Winning the same month again a year later counts on the same badge.
    let next_march = NaiveDate::from_ymd_opt(2002, 3, 1).unwrap();
    xp_on(&ctx, &alice, NaiveDate::from_ymd_opt(2002, 3, 2).unwrap(), 80).await;
    xp_on(&ctx, &bob, NaiveDate::from_ymd_opt(2002, 3, 2).unwrap(), 20).await;
    ctx.quest.finalize_season(next_march).await.unwrap();
    let history: ListSeasonResultsResponse = ctx
        .call(
            BOARDS,
            "ListSeasonResults",
            Some(&alice),
            &ListSeasonResultsRequest {
                page: tokuly_proto::common::PageRequest { limit: 1, ..Default::default() }.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!((history.seasons[0].id.as_str(), history.seasons[0].rank), ("2002-03", 1), "newest first");
    assert_eq!(history.seasons[0].badge.as_option().unwrap().times, 2);
    assert!(history.page.as_option().unwrap().has_more);
    assert_eq!(ctx.counter(&alice, "seasons_won").await, 2);
    assert_error(
        ctx.call::<_, ListSeasonResultsResponse>(
            BOARDS,
            "ListSeasonResults",
            None,
            &ListSeasonResultsRequest::default(),
        )
        .await,
        "unauthenticated",
        "SIGN_IN_REQUIRED",
    );
}
