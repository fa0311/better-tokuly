//! `VideoFeedService`: home, subscriptions, trending, related videos and
//! "not interested". The lists are global, so every test looks only at the
//! videos it made itself.
mod common;

use common::*;
use tokuly_proto::{common::Visibility, video::*};
use tokuly_testkit::TestUser;

/// The titles of `videos` that are among `mine`, in feed order.
fn only<'a>(videos: &'a [VideoSummary], mine: &[&Video]) -> Vec<&'a str> {
    videos
        .iter()
        .filter(|video| mine.iter().any(|own| card(own).id == video.id))
        .map(|video| video.title.as_str())
        .collect()
}

async fn home(ctx: &Ctx, viewer: Option<&TestUser>) -> Vec<VideoSummary> {
    let response: GetHomeFeedResponse = ctx
        .call(FEEDS, "GetHomeFeed", viewer, &GetHomeFeedRequest { page: page(50, ""), ..Default::default() })
        .await
        .expect("home feed");
    response.videos
}

async fn trending(ctx: &Ctx, viewer: Option<&TestUser>, category: VideoCategory) -> Vec<VideoSummary> {
    let request = ListTrendingRequest { category: category.into(), page: page(50, ""), ..Default::default() };
    let response: ListTrendingResponse = ctx.call(FEEDS, "ListTrending", viewer, &request).await.expect("trending");
    response.videos
}

fn not_interested_in_video(video: &Video) -> MarkNotInterestedRequest {
    MarkNotInterestedRequest {
        target: Some(mark_not_interested_request::Target::VideoId(card(video).id.clone())),
        ..Default::default()
    }
}

#[tokio::test]
async fn the_subscriptions_feed_shows_new_videos_of_followed_channels() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let carol = ctx.app.user("carol").await;
    let viewer = ctx.app.user("viewer").await;
    let friend_viewer = ctx.app.user("friend").await;
    ctx.befriend(&alice, &friend_viewer).await;
    let followed = ctx.channel(&alice, "followed").await;
    let other = ctx.channel(&carol, "other").await;
    for user in [&viewer, &friend_viewer] {
        ctx.subscribe(user, &followed.summary.id, NotificationLevel::NOTIFICATION_LEVEL_UNSPECIFIED).await;
    }

    ctx.public_video(&alice, &followed, "one").await;
    ctx.published(&alice, &followed, "no feed", Visibility::VISIBILITY_NO_FEED).await;
    ctx.published(&alice, &followed, "unlisted", Visibility::VISIBILITY_UNLISTED).await;
    ctx.published(&alice, &followed, "friends", Visibility::VISIBILITY_FRIENDS).await;
    ctx.published(&alice, &followed, "private", Visibility::VISIBILITY_PRIVATE).await;
    ctx.draft_with(&alice, &followed, "draft", |r| r.visibility = Visibility::VISIBILITY_PUBLIC.into()).await;
    ctx.public_video(&alice, &followed, "two").await;
    ctx.public_video(&carol, &other, "not followed").await;
    ctx.public_video(&alice, &followed, "three").await;

    let feed =
        |cursor: &str, limit: i32| GetSubscriptionsFeedRequest { page: page(limit, cursor), ..Default::default() };
    let response: GetSubscriptionsFeedResponse =
        ctx.call(FEEDS, "GetSubscriptionsFeed", Some(&viewer), &feed("", 0)).await.unwrap();
    assert_eq!(titles(&response.videos), vec!["three", "two", "one"], "newest first; only what is public and in feeds");
    let response: GetSubscriptionsFeedResponse =
        ctx.call(FEEDS, "GetSubscriptionsFeed", Some(&friend_viewer), &feed("", 0)).await.unwrap();
    assert_eq!(
        titles(&response.videos),
        vec!["three", "two", "friends", "one"],
        "friends also get friends-only videos"
    );

    let first: GetSubscriptionsFeedResponse =
        ctx.call(FEEDS, "GetSubscriptionsFeed", Some(&viewer), &feed("", 2)).await.unwrap();
    let info = first.page.as_option().unwrap();
    assert!(info.has_more);
    let rest: GetSubscriptionsFeedResponse =
        ctx.call(FEEDS, "GetSubscriptionsFeed", Some(&viewer), &feed(&info.next_cursor, 2)).await.unwrap();
    assert_eq!(titles(&rest.videos), vec!["one"]);
    assert!(!rest.page.as_option().unwrap().has_more);

    assert_eq!(ctx.reason(FEEDS, "GetSubscriptionsFeed", None, &feed("", 0)).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason(FEEDS, "GetSubscriptionsFeed", Some(&viewer), &feed("garbage", 0)).await, "INVALID_CURSOR");
    // A block empties the feed at once (the subscription itself is removed by the event consumer).
    ctx.block(&viewer, &alice).await;
    let response: GetSubscriptionsFeedResponse =
        ctx.call(FEEDS, "GetSubscriptionsFeed", Some(&viewer), &feed("", 0)).await.unwrap();
    assert!(response.videos.is_empty());
}

#[tokio::test]
async fn the_home_feed_mixes_subscriptions_friends_and_popular_videos() {
    let ctx = start().await;
    let viewer = ctx.app.user("viewer").await;
    let creator = ctx.app.user("creator").await;
    let friend = ctx.app.user("friend").await;
    let stranger = ctx.app.user("stranger").await;
    let hidden_owner = ctx.app.user("hidden").await;
    ctx.befriend(&viewer, &friend).await;
    ctx.block(&viewer, &hidden_owner).await;

    let subscribed_channel = ctx.channel(&creator, "subscribed").await;
    ctx.subscribe(&viewer, &subscribed_channel.summary.id, NotificationLevel::NOTIFICATION_LEVEL_UNSPECIFIED).await;
    let friends_channel = ctx.channel(&friend, "friend's").await;
    let strangers_channel = ctx.channel(&stranger, "stranger's").await;
    let own_channel = ctx.channel(&viewer, "own").await;

    let subscribed = ctx.public_video(&creator, &subscribed_channel, "from a subscription").await;
    let from_friend = ctx.public_video(&friend, &friends_channel, "friend's upload").await;
    let friends_only =
        ctx.published(&friend, &friends_channel, "friend's friends-only", Visibility::VISIBILITY_FRIENDS).await;
    let liked_by_friend = ctx.public_video(&stranger, &strangers_channel, "liked by a friend").await;
    ctx.rate(&friend, &card(&liked_by_friend).id, Rating::RATING_LIKE).await;
    let plain = ctx.public_video(&stranger, &strangers_channel, "just new").await;
    let finished = ctx.public_video(&stranger, &strangers_channel, "already watched").await;
    ctx.watch(Some(&viewer), &card(&finished).id, 12.0, TrafficSource::TRAFFIC_SOURCE_HOME).await;
    let own = ctx.public_video(&viewer, &own_channel, "my own").await;
    let blocked =
        ctx.public_video(&hidden_owner, &ctx.channel(&hidden_owner, "hidden").await, "from a blocked user").await;
    let unlisted = ctx.published(&stranger, &strangers_channel, "unlisted", Visibility::VISIBILITY_UNLISTED).await;
    let no_feed = ctx.published(&stranger, &strangers_channel, "no feed", Visibility::VISIBILITY_NO_FEED).await;
    let mine = [
        &subscribed,
        &from_friend,
        &friends_only,
        &liked_by_friend,
        &plain,
        &finished,
        &own,
        &blocked,
        &unlisted,
        &no_feed,
    ];

    let feed = home(&ctx, Some(&viewer)).await;
    let shown = only(&feed, &mine);
    let mut sorted = shown.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        vec!["friend's friends-only", "friend's upload", "from a subscription", "just new", "liked by a friend"]
    );
    // Where a video comes from decides the order among equally fresh ones.
    let position = |title: &str| shown.iter().position(|shown| *shown == title).unwrap();
    assert!(position("from a subscription") < position("friend's upload"));
    assert!(position("friend's upload") < position("liked by a friend"));
    assert!(position("liked by a friend") < position("just new"));
    // Cards carry the viewer's state.
    assert!(feed.iter().all(|video| !video.watched));

    // Anonymous visitors get what is public: no friends-only videos, nothing personal.
    let anonymous = home(&ctx, None).await;
    let mut shown = only(&anonymous, &mine);
    shown.sort_unstable();
    assert_eq!(
        shown,
        vec![
            "already watched",
            "friend's upload",
            "from a blocked user",
            "from a subscription",
            "just new",
            "liked by a friend",
            "my own"
        ]
    );
}

#[tokio::test]
async fn the_home_feed_pages_without_repeats_and_honours_not_interested() {
    let ctx = start().await;
    let viewer = ctx.app.user("viewer").await;
    let creator = ctx.app.user("creator").await;
    let other = ctx.app.user("other").await;
    let channel = ctx.channel(&creator, "ch").await;
    let other_channel = ctx.channel(&other, "other").await;
    let mut videos = Vec::new();
    for index in 0..7 {
        videos.push(ctx.public_video(&creator, &channel, &format!("video {index}")).await);
    }
    let from_other = ctx.public_video(&other, &other_channel, "other's video").await;
    videos.push(from_other.clone());
    let mine: Vec<&Video> = videos.iter().collect();

    // Scroll through the whole feed two at a time.
    let mut paged: Vec<VideoSummary> = Vec::new();
    let mut cursor = String::new();
    loop {
        let response: GetHomeFeedResponse = ctx
            .call(
                FEEDS,
                "GetHomeFeed",
                Some(&viewer),
                &GetHomeFeedRequest { page: page(2, &cursor), ..Default::default() },
            )
            .await
            .unwrap();
        assert!(response.videos.len() <= 2);
        paged.extend(response.videos);
        let info = response.page.as_option().unwrap();
        if !info.has_more {
            break;
        }
        cursor = info.next_cursor.clone();
    }
    let seen = only(&paged, &mine);
    assert_eq!(seen.len(), 8, "every video exactly once: {seen:?}");
    let mut unique_ids: Vec<&str> = paged.iter().map(|video| video.id.as_str()).collect();
    unique_ids.sort_unstable();
    unique_ids.dedup();
    assert_eq!(unique_ids.len(), paged.len(), "no video twice while scrolling");
    // One channel does not fill the screen: the other channel's video is moved up.
    assert!(seen.iter().position(|title| *title == "other's video").unwrap() <= 2, "{seen:?}");
    // A cursor that points nowhere starts over instead of failing.
    let stale = tokuly_core::page::encode_cursor(&serde_json::json!({ "token": "expired", "offset": 4 }));
    let response: GetHomeFeedResponse = ctx
        .call(FEEDS, "GetHomeFeed", Some(&viewer), &GetHomeFeedRequest { page: page(50, &stale), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(only(&response.videos, &mine).len(), 8);

    // "Not interested" in one video, then in a whole channel.
    let _: MarkNotInterestedResponse =
        ctx.call(FEEDS, "MarkNotInterested", Some(&viewer), &not_interested_in_video(&videos[3])).await.unwrap();
    let _: MarkNotInterestedResponse =
        ctx.call(FEEDS, "MarkNotInterested", Some(&viewer), &not_interested_in_video(&videos[3])).await.unwrap();
    let feed = home(&ctx, Some(&viewer)).await;
    assert!(!only(&feed, &mine).contains(&"video 3") && only(&feed, &mine).len() == 7);
    let channel_target = |reference: &str| MarkNotInterestedRequest {
        target: Some(mark_not_interested_request::Target::ChannelId(reference.to_owned())),
        ..Default::default()
    };
    let _: MarkNotInterestedResponse =
        ctx.call(FEEDS, "MarkNotInterested", Some(&viewer), &channel_target(&channel.summary.handle)).await.unwrap();
    assert_eq!(only(&home(&ctx, Some(&viewer)).await, &mine), vec!["other's video"]);
    // Other people's feeds are untouched.
    assert_eq!(only(&home(&ctx, Some(&other)).await, &mine).len(), 7, "everything but their own video");

    // Taking it back.
    let undo = UndoNotInterestedRequest {
        target: Some(undo_not_interested_request::Target::ChannelId(channel.summary.id.clone())),
        ..Default::default()
    };
    let _: UndoNotInterestedResponse = ctx.call(FEEDS, "UndoNotInterested", Some(&viewer), &undo).await.unwrap();
    assert_eq!(only(&home(&ctx, Some(&viewer)).await, &mine).len(), 7);
    let undo = UndoNotInterestedRequest {
        target: Some(undo_not_interested_request::Target::VideoId(card(&videos[3]).short_id.clone())),
        ..Default::default()
    };
    let _: UndoNotInterestedResponse = ctx.call(FEEDS, "UndoNotInterested", Some(&viewer), &undo).await.unwrap();
    assert_eq!(only(&home(&ctx, Some(&viewer)).await, &mine).len(), 8);

    let private = ctx.published(&creator, &channel, "private", Visibility::VISIBILITY_PRIVATE).await;
    assert_eq!(
        ctx.reason(FEEDS, "MarkNotInterested", Some(&viewer), &not_interested_in_video(&private)).await,
        "VIDEO_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason(FEEDS, "MarkNotInterested", Some(&viewer), &channel_target(&unique("nochannel"))).await,
        "CHANNEL_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason(FEEDS, "MarkNotInterested", Some(&viewer), &MarkNotInterestedRequest::default()).await,
        "TARGET_REQUIRED"
    );
    assert_eq!(
        ctx.reason(FEEDS, "UndoNotInterested", Some(&viewer), &UndoNotInterestedRequest::default()).await,
        "TARGET_REQUIRED"
    );
    assert_eq!(
        ctx.reason(FEEDS, "MarkNotInterested", None, &not_interested_in_video(&videos[0])).await,
        "SIGN_IN_REQUIRED"
    );
    assert_eq!(ctx.reason(FEEDS, "UndoNotInterested", None, &undo).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn trending_ranks_by_recent_views_and_likes_with_decay() {
    let ctx = start().await;
    let db = &ctx.app.core.db;
    let creator = ctx.app.user("creator").await;
    let fans = [ctx.app.user("fan1").await, ctx.app.user("fan2").await, ctx.app.user("fan3").await];
    let channel = ctx.channel(&creator, "ch").await;
    let gaming = |r: &mut CreateVideoRequest| r.category = VideoCategory::VIDEO_CATEGORY_GAMING.into();
    let hot = ctx.published_with(&creator, &channel, "hot", Visibility::VISIBILITY_PUBLIC, gaming).await;
    let liked = ctx
        .published_with(&creator, &channel, "liked", Visibility::VISIBILITY_PUBLIC, |r| {
            r.category = VideoCategory::VIDEO_CATEGORY_MUSIC.into()
        })
        .await;
    let old_hit = ctx.public_video(&creator, &channel, "old hit").await;
    let unwatched = ctx.public_video(&creator, &channel, "unwatched").await;
    let unlisted = ctx.published(&creator, &channel, "unlisted", Visibility::VISIBILITY_UNLISTED).await;
    let mine = [&hot, &liked, &old_hit, &unwatched, &unlisted];

    // hot: three views now. liked: one view and one like now. old hit: ten views three days ago.
    for fan in &fans {
        ctx.watch(Some(fan), &card(&hot).id, 2.0, TrafficSource::TRAFFIC_SOURCE_TRENDING).await;
    }
    ctx.watch(Some(&fans[0]), &card(&liked).id, 2.0, TrafficSource::TRAFFIC_SOURCE_TRENDING).await;
    ctx.rate(&fans[0], &card(&liked).id, Rating::RATING_LIKE).await;
    ctx.watch(Some(&fans[0]), &card(&unlisted).id, 2.0, TrafficSource::TRAFFIC_SOURCE_DIRECT).await;
    sqlx::query("INSERT INTO video_hourly_stats (video_id, hour, views) VALUES ($1, date_trunc('hour', now() - interval '72 hours'), 10)")
        .bind(id(&card(&old_hit).id))
        .execute(db)
        .await
        .unwrap();
    assert!(
        only(&trending(&ctx, None, VideoCategory::VIDEO_CATEGORY_UNSPECIFIED).await, &mine).is_empty(),
        "scores are the worker's job"
    );

    tokuly_video::feeds::refresh_trending(&ctx.app.core, 24.0).await.unwrap();
    let listed = trending(&ctx, None, VideoCategory::VIDEO_CATEGORY_UNSPECIFIED).await;
    // liked: 1 view + 4 for the like = 5; hot: 3; old hit: 10 views at an eighth of their weight.
    assert_eq!(only(&listed, &mine), vec!["liked", "hot", "old hit"], "unwatched and unlisted videos do not trend");
    assert_eq!(only(&trending(&ctx, None, VideoCategory::VIDEO_CATEGORY_GAMING).await, &mine), vec!["hot"]);

    // Activity that has aged out of the window stops counting, and the row is pruned.
    sqlx::query("UPDATE video_hourly_stats SET hour = hour - interval '9 days' WHERE video_id = $1")
        .bind(id(&card(&old_hit).id))
        .execute(db)
        .await
        .unwrap();
    tokuly_video::feeds::refresh_trending(&ctx.app.core, 24.0).await.unwrap();
    assert_eq!(
        only(&trending(&ctx, None, VideoCategory::VIDEO_CATEGORY_UNSPECIFIED).await, &mine),
        vec!["liked", "hot"]
    );
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM video_hourly_stats WHERE video_id = $1")
        .bind(id(&card(&old_hit).id))
        .fetch_one(db)
        .await
        .unwrap();
    assert_eq!(left, 0);

    // Paging by offset.
    let first: ListTrendingResponse = ctx
        .call(FEEDS, "ListTrending", None, &ListTrendingRequest { page: page(1, ""), ..Default::default() })
        .await
        .unwrap();
    let info = first.page.as_option().unwrap();
    assert!(info.has_more && first.videos.len() == 1);
    let second: ListTrendingResponse = ctx
        .call(
            FEEDS,
            "ListTrending",
            None,
            &ListTrendingRequest { page: page(1, &info.next_cursor), ..Default::default() },
        )
        .await
        .unwrap();
    assert_ne!(second.videos[0].id, first.videos[0].id);

    // Blocks hide trending videos, too.
    let blocker = ctx.app.user("blocker").await;
    ctx.block(&blocker, &creator).await;
    assert!(only(&trending(&ctx, Some(&blocker), VideoCategory::VIDEO_CATEGORY_UNSPECIFIED).await, &mine).is_empty());
    let odd = ListTrendingRequest { category: buffa::EnumValue::from(99), ..Default::default() };
    assert_eq!(ctx.reason(FEEDS, "ListTrending", None, &odd).await, "CATEGORY_INVALID");
}

#[tokio::test]
async fn related_videos_share_the_channel_tags_game_or_category() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let elsewhere = ctx.channel(&bob, "elsewhere").await;
    let (tag, game) = (unique("tag"), unique("game"));
    let describe = |tags: Vec<String>, game: &str, category: VideoCategory| {
        let game = game.to_owned();
        move |r: &mut CreateVideoRequest| {
            r.tags = tags;
            r.game = game;
            r.category = category.into();
        }
    };
    let source = ctx
        .published_with(
            &alice,
            &channel,
            "source",
            Visibility::VISIBILITY_PUBLIC,
            describe(vec![tag.clone(), "other".into()], &game, VideoCategory::VIDEO_CATEGORY_GAMING),
        )
        .await;
    let same_channel_and_tag = ctx
        .published_with(
            &alice,
            &channel,
            "same channel + tag",
            Visibility::VISIBILITY_PUBLIC,
            describe(vec![tag.clone()], "", VideoCategory::VIDEO_CATEGORY_UNSPECIFIED),
        )
        .await;
    let same_channel = ctx.public_video(&alice, &channel, "same channel").await;
    let same_tag_and_game = ctx
        .published_with(
            &bob,
            &elsewhere,
            "same tag + game",
            Visibility::VISIBILITY_PUBLIC,
            describe(vec![tag.clone()], &game, VideoCategory::VIDEO_CATEGORY_UNSPECIFIED),
        )
        .await;
    let same_category = ctx
        .published_with(
            &bob,
            &elsewhere,
            "same category",
            Visibility::VISIBILITY_PUBLIC,
            describe(vec![], "", VideoCategory::VIDEO_CATEGORY_GAMING),
        )
        .await;
    let unrelated = ctx.public_video(&bob, &elsewhere, "unrelated").await;
    let private = ctx
        .published_with(
            &alice,
            &channel,
            "private sibling",
            Visibility::VISIBILITY_PRIVATE,
            describe(vec![tag.clone()], &game, VideoCategory::VIDEO_CATEGORY_GAMING),
        )
        .await;
    let mine =
        [&source, &same_channel_and_tag, &same_channel, &same_tag_and_game, &same_category, &unrelated, &private];

    let request =
        |limit: i32| ListRelatedVideosRequest { video_id: card(&source).short_id.clone(), limit, ..Default::default() };
    let response: ListRelatedVideosResponse = ctx.call(FEEDS, "ListRelatedVideos", None, &request(50)).await.unwrap();
    let related = only(&response.videos, &mine);
    assert_eq!(
        &related[..4],
        ["same channel + tag", "same tag + game", "same channel", "same category"],
        "the more in common, the higher"
    );
    assert!(related.contains(&"unrelated"), "the list is topped up with other public videos");
    assert!(!related.contains(&"source") && !related.contains(&"private sibling"));
    let response: ListRelatedVideosResponse = ctx.call(FEEDS, "ListRelatedVideos", None, &request(2)).await.unwrap();
    assert_eq!(titles(&response.videos), vec!["same channel + tag", "same tag + game"]);

    // "Not interested" and blocks apply here as well.
    let viewer = ctx.app.user("viewer").await;
    let _: MarkNotInterestedResponse =
        ctx.call(FEEDS, "MarkNotInterested", Some(&viewer), &not_interested_in_video(&same_channel)).await.unwrap();
    ctx.block(&viewer, &bob).await;
    let response: ListRelatedVideosResponse =
        ctx.call(FEEDS, "ListRelatedVideos", Some(&viewer), &request(50)).await.unwrap();
    assert_eq!(only(&response.videos, &mine), vec!["same channel + tag"]);

    let hidden = ListRelatedVideosRequest { video_id: card(&private).id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(FEEDS, "ListRelatedVideos", Some(&bob), &hidden).await, "VIDEO_NOT_FOUND");
    let own: ListRelatedVideosResponse = ctx.call(FEEDS, "ListRelatedVideos", Some(&alice), &hidden).await.unwrap();
    assert!(only(&own.videos, &mine).contains(&"source"), "the owner gets suggestions for their private video");
}
