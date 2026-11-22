//! The for-you ranking on a fixed set of shorts.
//!
//! This file is a test binary of its own, and so has a database of its
//! own: the for-you feed is global, and here every short in it is known.
//! The one test walks through the life of a feed: scores, the cold feed,
//! a personal feed, "not interested", seen-tracking, stable paging and
//! what happens when everything has been seen.
mod common;

use common::*;
use tokuly_proto::{common::Visibility, shorts::*, video::NotificationLevel};
use tokuly_shorts::ranking;
use tokuly_testkit::TestUser;

/// Gives a short engagement in the hourly bucket `hours_ago` hours back.
async fn engagement(ctx: &Ctx, short_id: &str, hours_ago: i32, views: i64, likes: i64, shares: i64) {
    sqlx::query(
        "INSERT INTO short_hourly_stats (short_id, hour, views, likes, shares)
         VALUES ($1, date_trunc('hour', now()) - make_interval(hours => $2), $3, $4, $5)",
    )
    .bind(id(short_id))
    .bind(hours_ago)
    .bind(views)
    .bind(likes)
    .bind(shares)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
}

/// Says how much of a short its viewers watched: `samples` plays at `ratio`.
async fn watched(ctx: &Ctx, short_id: &str, ratio: f64, samples: i64) {
    sqlx::query("UPDATE shorts SET watch_ratio_sum = $2, watch_samples = $3 WHERE id = $1")
        .bind(id(short_id))
        .bind(ratio * samples as f64)
        .bind(samples)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
}

/// Makes a short `hours` old.
async fn aged(ctx: &Ctx, short_id: &str, hours: i32) {
    sqlx::query("UPDATE shorts SET published_at = now() - make_interval(hours => $2) WHERE id = $1")
        .bind(id(short_id))
        .bind(hours)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
}

/// The score the documented formula gives a short, from what is stored
/// about it, as of the moment the worker scored it.
async fn expected_score(ctx: &Ctx, short_id: &str, half_life: f64) -> f64 {
    let db = &ctx.app.core.db;
    let (scored_at, published_at, ratio_sum, samples): (
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
        f64,
        i64,
    ) = sqlx::query_as("SELECT scored_at, published_at, watch_ratio_sum, watch_samples FROM shorts WHERE id = $1")
        .bind(id(short_id))
        .fetch_one(db)
        .await
        .unwrap();
    let buckets: Vec<(chrono::DateTime<chrono::Utc>, i64, i64, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT hour, views, loops, likes, comments, shares, saves FROM short_hourly_stats WHERE short_id = $1",
    )
    .bind(id(short_id))
    .fetch_all(db)
    .await
    .unwrap();
    let hours = |from: chrono::DateTime<chrono::Utc>| (scored_at - from).num_milliseconds() as f64 / 3_600_000.0;
    let engagement: f64 = buckets
        .iter()
        .map(|(hour, views, loops, likes, comments, shares, saves)| {
            ranking::decayed_engagement(*views, *loops, *likes, *comments, *shares, *saves, hours(*hour), half_life)
        })
        .sum();
    ranking::base_score(
        engagement,
        ranking::quality(ratio_sum, samples),
        ranking::freshness(hours(published_at), half_life),
    )
}

fn names(items: &[FeedItem]) -> Vec<&str> {
    items.iter().map(|item| item_short(item).caption.split_whitespace().next().unwrap_or_default()).collect()
}

fn reasons(items: &[FeedItem]) -> Vec<(&str, FeedReason)> {
    items
        .iter()
        .map(|item| {
            let name = item_short(item).caption.split_whitespace().next().unwrap_or_default();
            (name, item.reason.as_known().expect("a known reason"))
        })
        .collect()
}

async fn feed(ctx: &Ctx, viewer: Option<&TestUser>) -> Vec<FeedItem> {
    ctx.for_you_all(viewer, 5).await
}

#[tokio::test]
async fn the_for_you_feed_ranks_a_known_set_of_shorts() {
    let ctx = start().await;
    let half_life = ctx.shorts.cfg.ranking_half_life_hours;
    let vera = ctx.app.user("vera").await;
    let creator = |name: &'static str| {
        let ctx = &ctx;
        async move {
            let user = ctx.app.user(name).await;
            let channel = ctx.channel(&user, name).await;
            (user, channel)
        }
    };
    let (viral_user, viral_channel) = creator("viral").await;
    let (good_user, good_channel) = creator("good").await;
    let (boring_user, boring_channel) = creator("boring").await;
    let (old_user, old_channel) = creator("old").await;
    let (prank_user, prank_channel) = creator("prank").await;
    let (liked_user, liked_channel) = creator("liked").await;
    let (friend_user, friend_channel) = creator("friend").await;
    let (followed_user, followed_channel) = creator("followed").await;
    let (tag_user, tag_channel) = creator("tag").await;
    let (new_user, new_channel) = creator("new").await;
    let (blocked_user, blocked_channel) = creator("blocked").await;
    let veras_channel = ctx.channel(&vera, "vera").await;
    ctx.befriend(&vera, &friend_user).await;
    ctx.block(&vera, &blocked_user).await;
    ctx.subscribe(&vera, &followed_channel, NotificationLevel::NOTIFICATION_LEVEL_PERSONALIZED).await;

    // The first word of each caption is the name the assertions use.
    let (liked_tag, prank_tag) = (unique("liked"), unique("prank"));
    let viral = ctx.short(&viral_user, &viral_channel, "viral たくさん見られて、最後まで見られる").await;
    engagement(&ctx, &viral.id, 0, 1000, 100, 20).await;
    watched(&ctx, &viral.id, 0.8, 500).await;
    let good = ctx.short(&good_user, &good_channel, &format!("good 少ないけど最後まで見られる #{prank_tag}")).await;
    engagement(&ctx, &good.id, 0, 100, 10, 0).await;
    watched(&ctx, &good.id, 0.9, 100).await;
    let good2 = ctx.short(&good_user, &good_channel, "good2 同じチャンネルの二本目").await;
    engagement(&ctx, &good2.id, 0, 200, 0, 0).await;
    let boring = ctx.short(&boring_user, &boring_channel, "boring 見られるけどすぐ飛ばされる").await;
    engagement(&ctx, &boring.id, 0, 1000, 0, 0).await;
    watched(&ctx, &boring.id, 0.1, 1000).await;
    let old = ctx.short(&old_user, &old_channel, "old 五日前に見られた十日前の投稿").await;
    engagement(&ctx, &old.id, 120, 530, 0, 0).await;
    aged(&ctx, &old.id, 240).await;
    let prank2 =
        ctx.short(&prank_user, &prank_channel, &format!("prank2 同じハッシュタグの別チャンネル #{prank_tag}")).await;
    engagement(&ctx, &prank2.id, 0, 300, 0, 0).await;
    let liked = ctx.short(&liked_user, &liked_channel, &format!("liked veraがいいねした #{liked_tag}")).await;
    let friends = ctx
        .short_with(&friend_user, &friend_channel, "friend フレンド限定", |request| {
            request.visibility = Visibility::VISIBILITY_FRIENDS.into()
        })
        .await;
    let followed = ctx.short(&followed_user, &followed_channel, "followed 登録チャンネルの新作").await;
    let tagged = ctx.short(&tag_user, &tag_channel, &format!("tag いいねしたのと同じタグ #{liked_tag}")).await;
    let fresh = ctx.short(&new_user, &new_channel, "new 誰もまだ見ていない").await;
    let blocked = ctx.short(&blocked_user, &blocked_channel, "blocked ブロックした人の大人気動画").await;
    engagement(&ctx, &blocked.id, 0, 100_000, 0, 0).await;
    let own = ctx.short(&vera, &veras_channel, "own 自分の動画").await;
    engagement(&ctx, &own.id, 0, 50_000, 0, 0).await;
    ctx.like(&vera, &liked.id).await;

    // ── 1. The worker's scores are the documented formula ──
    ctx.shorts.run_ranking().await.unwrap();
    let all =
        [&viral, &good, &good2, &boring, &old, &prank2, &liked, &friends, &followed, &tagged, &fresh, &blocked, &own];
    for short in all {
        let (stored, expected) = (ctx.row(&short.id).await.score, expected_score(&ctx, &short.id, half_life).await);
        assert!(
            (stored - expected).abs() < 1e-6 * expected.max(1.0),
            "{}: stored {stored}, formula {expected}",
            short.caption
        );
    }
    let score = |short: &Short| {
        let ctx = &ctx;
        let short_id = short.id.clone();
        async move { ctx.row(&short_id).await.score }
    };
    // Engagement lifts, watch-through lifts more, age and skipping cost.
    assert!(score(&viral).await > score(&good).await && score(&good).await > score(&boring).await);
    assert!(score(&boring).await > score(&old).await && score(&old).await > score(&fresh).await);
    assert!((score(&fresh).await - 1.0).abs() < 1e-3, "an unseen short starts at one");
    assert!((score(&viral).await - 11.3).abs() < 0.2 && (score(&old).await - 1.25).abs() < 0.05);
    assert!(ctx.row(&viral.id).await.trending > 1000.0 && ctx.row(&fresh.id).await.trending == 0.0);

    // ── 2. Cold start: no account, or an account that knows nobody ──
    let cold =
        ["blocked", "own", "viral", "good", "prank2", "boring", "good2", "liked", "old", "new", "tag", "followed"];
    let anonymous = feed(&ctx, None).await;
    // (`good2` waits a place: its channel was shown two shorts earlier.)
    assert_eq!(names(&anonymous), cold);
    assert!(anonymous.iter().all(|item| !item.repeat && item.friend.as_option().is_none()));
    assert!(anonymous.iter().all(|item| item_short(item).playback.as_option().is_some()), "the feed is ready to play");
    let newbie = ctx.app.user("newbie").await;
    let first_visit = feed(&ctx, Some(&newbie)).await;
    assert_eq!(names(&first_visit), cold, "a new account gets a full feed");
    assert_eq!(first_visit[0].reason, FeedReason::FEED_REASON_TRENDING);
    assert_eq!(first_visit.last().unwrap().reason, FeedReason::FEED_REASON_NEW);

    // ── 3. Vera's feed: her graph and her likes move things ──
    let personal = feed(&ctx, Some(&vera)).await;
    use FeedReason::*;
    assert_eq!(
        reasons(&personal),
        [
            ("viral", FEED_REASON_TRENDING),
            ("good", FEED_REASON_TRENDING),
            ("prank2", FEED_REASON_TRENDING),
            ("boring", FEED_REASON_TRENDING),
            ("good2", FEED_REASON_TRENDING),
            ("liked", FEED_REASON_HASHTAG),
            ("friend", FEED_REASON_FRIEND),
            ("followed", FEED_REASON_FOLLOWING),
            ("old", FEED_REASON_TRENDING),
            ("tag", FEED_REASON_HASHTAG),
            ("new", FEED_REASON_NEW),
        ],
        "no blocked creator, none of her own, friends-only from her friend"
    );
    let by_name = |items: &[FeedItem], name: &str| {
        items.iter().find(|item| item_short(item).caption.starts_with(name)).cloned().unwrap()
    };
    assert_eq!(by_name(&personal, "tag").hashtag, liked_tag);
    assert_eq!(
        by_name(&personal, "friend").friend.as_option().expect("the friend is named").id,
        friend_user.id.to_string()
    );
    assert!(by_name(&personal, "followed").friend.as_option().is_none());
    assert!(viewer_state(item_short(&by_name(&personal, "followed"))).subscribed);
    assert!(viewer_state(item_short(&by_name(&personal, "liked"))).liked);
    // The same question gets the same answer.
    assert_eq!(names(&feed(&ctx, Some(&vera)).await), names(&personal));

    // ── 4. Paging is stable while shorts arrive ──
    let first = ctx.for_you(Some(&vera), 3, "").await;
    assert_eq!(names(&first.items), ["viral", "good", "prank2"]);
    let cursor = first.page.as_option().unwrap().next_cursor.clone();
    assert!(first.page.as_option().unwrap().has_more);
    let late = ctx.short(&new_user, &new_channel, "late 途中で投稿された大人気動画").await;
    engagement(&ctx, &late.id, 0, 80_000, 0, 0).await;
    ctx.shorts.run_ranking().await.unwrap();
    let second = ctx.for_you(Some(&vera), 3, &cursor).await;
    assert_eq!(names(&second.items), ["boring", "good2", "liked"], "the batch being read does not move");
    assert_eq!(names(&ctx.for_you(Some(&vera), 3, &cursor).await.items), ["boring", "good2", "liked"]);
    assert_eq!(names(&ctx.for_you(Some(&vera), 2, "").await.items), ["late", "viral"], "a new batch has it");
    let bad = GetForYouFeedRequest { page: page(3, "garbage"), ..Default::default() };
    assert_eq!(ctx.reason(FEEDS, "GetForYouFeed", Some(&vera), &bad).await, "INVALID_CURSOR");

    // ── 5. "Not interested" hides the short and pushes its kind down ──
    ctx.mark_not_interested(&vera, &good.short_id).await;
    let after = feed(&ctx, Some(&vera)).await;
    assert_eq!(
        names(&after),
        ["late", "viral", "prank2", "boring", "liked", "good2", "friend", "followed", "old", "tag", "new"],
        "`good` is gone; its channel's `good2` and its hashtag's `prank2` fell"
    );
    assert!(viewer_state(&ctx.get(Some(&vera), &good.id).await.unwrap()).not_interested, "it still opens by link");
    assert_eq!(
        names(&feed(&ctx, Some(&newbie)).await)[..5],
        ["blocked", "late", "own", "viral", "good"],
        "only her feed changed"
    );
    // Taking it back restores the feed.
    let undo = UndoNotInterestedRequest { short_id: good.id.clone(), ..Default::default() };
    let _: UndoNotInterestedResponse = ctx.call(FEEDS, "UndoNotInterested", Some(&vera), &undo).await.unwrap();
    assert_eq!(names(&feed(&ctx, Some(&vera)).await)[..6], ["late", "viral", "good", "prank2", "boring", "good2"]);

    // ── 6. Seen shorts do not come back ──
    let _lock = playback_lock().await;
    let seen = vec![
        stay(&late.id, PlaybackSource::PLAYBACK_SOURCE_FOR_YOU, 12_000, 1),
        stay(&viral.short_id, PlaybackSource::PLAYBACK_SOURCE_FOR_YOU, 3_000, 0),
        // Swiped away at once: no view, but she has seen it.
        stay(&good.id, PlaybackSource::PLAYBACK_SOURCE_FOR_YOU, 200, 0),
    ];
    let recorded = ctx.report(Some(&vera), seen).await;
    assert_eq!((recorded.accepted, recorded.views_counted), (3, 2));
    let unseen = feed(&ctx, Some(&vera)).await;
    assert_eq!(names(&unseen), ["prank2", "good2", "boring", "liked", "friend", "followed", "old", "tag", "new"]);
    assert!(unseen.iter().all(|item| !item.repeat && !viewer_state(item_short(item)).seen));
    assert!(viewer_state(&ctx.get(Some(&vera), &viral.id).await.unwrap()).seen);
    assert!(names(&feed(&ctx, Some(&newbie)).await).contains(&"viral"), "what vera saw is still new to others");

    // ── 7. When everything has been seen, the feed starts over, flagged ──
    let rest: Vec<PlaybackReport> = unseen
        .iter()
        .map(|item| stay(&item_short(item).id, PlaybackSource::PLAYBACK_SOURCE_FOR_YOU, 5_000, 0))
        .collect();
    ctx.report(Some(&vera), rest).await;
    let again = feed(&ctx, Some(&vera)).await;
    assert_eq!(names(&again)[..3], ["late", "viral", "good"]);
    assert_eq!(again.len(), 12);
    assert!(again.iter().all(|item| item.repeat && viewer_state(item_short(item)).seen));
    let page_info = ctx.for_you(Some(&vera), 30, "").await.page.as_option().cloned().unwrap();
    assert!(
        page_info.has_more && !page_info.next_cursor.is_empty(),
        "the feed never ends while there is something to show"
    );
    ctx.flush().await;
    drop(_lock);

    // The unused creators exist so that every short has a channel of its own.
    let _ = (boring_user, old_user, prank_user, liked_user, tag_user, viral_user, good_user);
}
