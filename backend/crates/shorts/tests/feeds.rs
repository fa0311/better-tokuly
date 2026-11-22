//! The following and friends feeds, and what the for-you feed never shows.
//! (The ranking itself has a test binary, and a database, of its own:
//! `ranking.rs`.)
mod common;

use common::*;
use tokuly_proto::{
    common::Visibility,
    shorts::*,
    video::{NotificationLevel, SubscribeRequest, SubscribeResponse, UnsubscribeRequest, UnsubscribeResponse},
};
use tokuly_testkit::TestUser;

fn names(items: &[FeedItem]) -> Vec<&str> {
    items.iter().map(|item| item_short(item).caption.as_str()).collect()
}

async fn following(
    ctx: &Ctx,
    viewer: &TestUser,
    limit: i32,
    cursor: &str,
    skip_seen: bool,
) -> GetFollowingFeedResponse {
    let request = GetFollowingFeedRequest { page: page(limit, cursor), skip_seen, ..Default::default() };
    ctx.call(FEEDS, "GetFollowingFeed", Some(viewer), &request).await.expect("following feed")
}

async fn friends(ctx: &Ctx, viewer: &TestUser, limit: i32, cursor: &str, skip_seen: bool) -> GetFriendsFeedResponse {
    let request = GetFriendsFeedRequest { page: page(limit, cursor), skip_seen, ..Default::default() };
    ctx.call(FEEDS, "GetFriendsFeed", Some(viewer), &request).await.expect("friends feed")
}

#[tokio::test]
async fn the_following_feed_lists_followed_channels_newest_first() {
    let ctx = start().await;
    let vera = ctx.app.user("vera").await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let a = ctx.channel(&alice, "a").await;
    let b = ctx.channel(&bob, "b").await;
    let c = ctx.channel(&carol, "c").await;
    ctx.subscribe(&vera, &a, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    ctx.subscribe(&vera, &b, NotificationLevel::NOTIFICATION_LEVEL_NONE).await;

    ctx.short(&alice, &a, "a1").await;
    let b1 = ctx.short(&bob, &b, "b1").await;
    let a2 = ctx.short(&alice, &a, "a2").await;
    ctx.short_with(&alice, &a, "a3 フレンド限定", |request| {
        request.visibility = Visibility::VISIBILITY_FRIENDS.into()
    })
    .await;
    ctx.short_with(&alice, &a, "a4 フィード非表示", |request| {
        request.visibility = Visibility::VISIBILITY_NO_FEED.into()
    })
    .await;
    ctx.short(&carol, &c, "c1 登録していないチャンネル").await;

    let feed = following(&ctx, &vera, 0, "", false).await;
    assert_eq!(names(&feed.items), ["a2", "b1", "a1"]);
    assert!(feed.items.iter().all(|item| viewer_state(item_short(item)).subscribed));
    assert!(
        feed.items
            .iter()
            .all(|item| item.friend.as_option().is_none() && item.reason == FeedReason::FEED_REASON_UNSPECIFIED)
    );
    assert!(!feed.page.as_option().unwrap().has_more);

    // A short posted between two pages does not shift the second one.
    let first = following(&ctx, &vera, 2, "", false).await;
    assert_eq!(names(&first.items), ["a2", "b1"]);
    let cursor = first.page.as_option().unwrap().next_cursor.clone();
    ctx.short(&bob, &b, "b2").await;
    assert_eq!(names(&following(&ctx, &vera, 2, &cursor, false).await.items), ["a1"]);
    assert_eq!(names(&following(&ctx, &vera, 0, "", false).await.items), ["b2", "a2", "b1", "a1"]);

    // Friends-only shorts of a followed channel are for friends of its owner.
    ctx.befriend(&vera, &alice).await;
    assert_eq!(names(&following(&ctx, &vera, 0, "", false).await.items), ["b2", "a3 フレンド限定", "a2", "b1", "a1"]);

    // Seen shorts are flagged, and left out on request.
    {
        let _lock = playback_lock().await;
        ctx.watch(&vera, &a2.id).await;
        ctx.flush().await;
    }
    let flagged = following(&ctx, &vera, 0, "", false).await;
    let seen: Vec<bool> = flagged.items.iter().map(|item| viewer_state(item_short(item)).seen).collect();
    assert_eq!(seen, [false, false, true, false, false]);
    assert_eq!(names(&following(&ctx, &vera, 0, "", true).await.items), ["b2", "a3 フレンド限定", "b1", "a1"]);

    // "Not interested" takes a short out of every feed.
    ctx.mark_not_interested(&vera, &b1.id).await;
    assert_eq!(names(&following(&ctx, &vera, 0, "", false).await.items), ["b2", "a3 フレンド限定", "a2", "a1"]);

    // Unsubscribing ends it.
    let leave = UnsubscribeRequest { channel_id: b.summary.id.clone(), ..Default::default() };
    let _: UnsubscribeResponse = ctx.call(CHANNELS, "Unsubscribe", Some(&vera), &leave).await.unwrap();
    assert_eq!(ctx.deliver("video.channel.unsubscribed", vera.id).await, 1);
    assert_eq!(names(&following(&ctx, &vera, 0, "", false).await.items), ["a3 フレンド限定", "a2", "a1"]);
    // A block hides the channel's shorts at once, before any event arrives.
    ctx.block(&alice, &vera).await;
    assert!(following(&ctx, &vera, 0, "", false).await.items.is_empty());

    let request = GetFollowingFeedRequest::default();
    assert_eq!(ctx.reason(FEEDS, "GetFollowingFeed", None, &request).await, "SIGN_IN_REQUIRED");
    let bad = GetFollowingFeedRequest { page: page(5, "garbage"), ..Default::default() };
    assert_eq!(ctx.reason(FEEDS, "GetFollowingFeed", Some(&vera), &bad).await, "INVALID_CURSOR");
}

#[tokio::test]
async fn subscriptions_made_before_a_channel_had_shorts_count() {
    let ctx = start().await;
    let vera = ctx.app.user("vera").await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "a").await;
    // Subscribed long ago: the shorts domain never saw the event.
    let request = SubscribeRequest { channel_id: channel.summary.id.clone(), ..Default::default() };
    let _: SubscribeResponse = ctx.call(CHANNELS, "Subscribe", Some(&vera), &request).await.unwrap();
    assert!(following(&ctx, &vera, 0, "", false).await.items.is_empty());

    ctx.short(&alice, &channel, "はじめてのショート").await;
    let feed = following(&ctx, &vera, 0, "", false).await;
    assert_eq!(names(&feed.items), ["はじめてのショート"]);
    assert!(viewer_state(item_short(&feed.items[0])).subscribed);
}

#[tokio::test]
async fn the_friends_feed_names_the_friend_and_includes_friends_only_shorts() {
    let ctx = start().await;
    let vera = ctx.app.user("vera").await;
    let fumi = ctx.app.user("fumi").await;
    let goro = ctx.app.user("goro").await;
    let stranger = ctx.app.user("stranger").await;
    let f = ctx.channel(&fumi, "ふみのチャンネル").await;
    let g = ctx.channel(&goro, "ごろうのチャンネル").await;
    let s = ctx.channel(&stranger, "知らない人").await;
    assert!(friends(&ctx, &vera, 0, "", false).await.items.is_empty(), "nobody to hear from yet");
    ctx.befriend(&vera, &fumi).await;
    ctx.befriend(&goro, &vera).await;

    ctx.short(&fumi, &f, "f1 公開").await;
    let f2 = ctx
        .short_with(&fumi, &f, "f2 フレンド限定", |request| {
            request.visibility = Visibility::VISIBILITY_FRIENDS.into()
        })
        .await;
    ctx.short_with(&goro, &g, "g1 非公開", |request| request.visibility = Visibility::VISIBILITY_PRIVATE.into()).await;
    ctx.short_with(&goro, &g, "g2 限定公開", |request| request.visibility = Visibility::VISIBILITY_UNLISTED.into())
        .await;
    ctx.short(&goro, &g, "g3 公開").await;
    ctx.short(&stranger, &s, "s1").await;

    let feed = friends(&ctx, &vera, 0, "", false).await;
    assert_eq!(names(&feed.items), ["g3 公開", "f2 フレンド限定", "f1 公開"]);
    let named: Vec<&str> =
        feed.items.iter().map(|item| item.friend.as_option().expect("the friend is named").id.as_str()).collect();
    assert_eq!(named, [goro.id.to_string().as_str(), fumi.id.to_string().as_str(), fumi.id.to_string().as_str()]);
    // The friend is the account; the short still shows the channel.
    assert_eq!(item_short(&feed.items[1]).channel.as_option().unwrap().name, "ふみのチャンネル");
    // She follows neither channel: friendship is what puts them here.
    assert!(!viewer_state(item_short(&feed.items[0])).subscribed);

    // Paging holds while friends keep posting.
    let first = friends(&ctx, &vera, 1, "", false).await;
    let cursor = first.page.as_option().unwrap().next_cursor.clone();
    ctx.short(&goro, &g, "g4 あとから").await;
    assert_eq!(names(&friends(&ctx, &vera, 5, &cursor, false).await.items), ["f2 フレンド限定", "f1 公開"]);

    // Seen and "not interested" work as in the other feeds.
    {
        let _lock = playback_lock().await;
        ctx.watch(&vera, &f2.id).await;
        ctx.flush().await;
    }
    assert_eq!(names(&friends(&ctx, &vera, 0, "", true).await.items), ["g4 あとから", "g3 公開", "f1 公開"]);
    ctx.mark_not_interested(&vera, &f2.id).await;
    assert_eq!(names(&friends(&ctx, &vera, 0, "", false).await.items), ["g4 あとから", "g3 公開", "f1 公開"]);

    // The stranger's own friends feed is empty; a block empties vera's of that friend.
    assert!(friends(&ctx, &stranger, 0, "", false).await.items.is_empty());
    ctx.block(&vera, &goro).await;
    assert_eq!(names(&friends(&ctx, &vera, 0, "", false).await.items), ["f1 公開"]);
    assert_eq!(ctx.reason(FEEDS, "GetFriendsFeed", None, &GetFriendsFeedRequest::default()).await, "SIGN_IN_REQUIRED");
    let bad = GetFriendsFeedRequest { page: page(5, "garbage"), ..Default::default() };
    assert_eq!(ctx.reason(FEEDS, "GetFriendsFeed", Some(&vera), &bad).await, "INVALID_CURSOR");
}

#[tokio::test]
async fn the_for_you_feed_shows_only_what_the_viewer_may_see_in_feeds() {
    let ctx = start().await;
    let vera = ctx.app.user("vera").await;
    let alice = ctx.app.user("alice").await;
    let foe = ctx.app.user("foe").await;
    let a = ctx.channel(&alice, "a").await;
    let x = ctx.channel(&foe, "x").await;
    let v = ctx.channel(&vera, "v").await;
    ctx.block(&foe, &vera).await;
    let tag = unique("foryou");
    let caption = |name: &str| format!("{name} #{tag}");
    let public = ctx.short(&alice, &a, &caption("public")).await;
    ctx.short_with(&alice, &a, &caption("friends"), |request| {
        request.visibility = Visibility::VISIBILITY_FRIENDS.into()
    })
    .await;
    ctx.short_with(&alice, &a, &caption("unlisted"), |request| {
        request.visibility = Visibility::VISIBILITY_UNLISTED.into()
    })
    .await;
    ctx.short_with(&alice, &a, &caption("nofeed"), |request| {
        request.visibility = Visibility::VISIBILITY_NO_FEED.into()
    })
    .await;
    ctx.short(&foe, &x, &caption("blocked")).await;
    ctx.short(&vera, &v, &caption("own")).await;

    // Other tests share this database: look at this test's shorts only.
    let mine = |items: Vec<FeedItem>| -> Vec<String> {
        let mut found: Vec<String> = items
            .iter()
            .map(|item| item_short(item).caption.clone())
            .filter(|caption| caption.ends_with(&format!("#{tag}")))
            .map(|caption| caption.split_whitespace().next().unwrap().to_owned())
            .collect();
        found.sort();
        found
    };
    assert_eq!(mine(ctx.for_you_all(Some(&vera), 10).await), ["public"]);
    // Without an account the block does not apply, and nothing is "own".
    assert_eq!(mine(ctx.for_you_all(None, 10).await), ["blocked", "own", "public"]);
    // A friend gets the friends-only short too.
    ctx.befriend(&vera, &alice).await;
    assert_eq!(mine(ctx.for_you_all(Some(&vera), 10).await), ["friends", "public"]);

    // A short deleted while its batch is being read drops out of the pages
    // still to come.
    let friends_only =
        ctx.for_you_all(Some(&vera), 10).await.into_iter().find(|item| item_short(item).caption.starts_with("friends"));
    let friends_only = item_short(&friends_only.expect("the friends-only short")).id.clone();
    let batch = ctx.for_you(Some(&vera), 1, "").await;
    let already_read = item_short(&batch.items[0]).id.clone();
    let victim = if already_read == public.id { friends_only } else { public.id.clone() };
    ctx.delete(&alice, &victim).await;
    let mut cursor = batch.page.as_option().unwrap().next_cursor.clone();
    let mut rest = Vec::new();
    for _ in 0..10 {
        let next = ctx.for_you(Some(&vera), 30, &cursor).await;
        let got = next.items.len();
        rest.extend(next.items);
        cursor = next.page.as_option().unwrap().next_cursor.clone();
        if got < 30 {
            break;
        }
    }
    assert!(rest.iter().all(|item| item_short(item).id != victim), "the stored batch still names it; the pages do not");
}

#[tokio::test]
async fn playback_reports_are_bounded_and_ignore_what_cannot_be_watched() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let hidden = ctx
        .short_with(&alice, &channel, "非公開", |request| request.visibility = Visibility::VISIBILITY_PRIVATE.into())
        .await;
    let _lock = playback_lock().await;

    let too_many = ReportPlaybackRequest {
        reports: vec![stay(&hidden.id, PlaybackSource::PLAYBACK_SOURCE_FOR_YOU, 5000, 0); 51],
        ..Default::default()
    };
    assert_eq!(ctx.reason(FEEDS, "ReportPlayback", Some(&bob), &too_many).await, "TOO_MANY_REPORTS");
    // Nothing to learn from reporting on shorts one cannot see, or that do not exist.
    let ignored = ctx
        .report(
            Some(&bob),
            vec![
                stay(&hidden.id, PlaybackSource::PLAYBACK_SOURCE_PROFILE, 5000, 0),
                stay(&uuid::Uuid::now_v7().to_string(), PlaybackSource::PLAYBACK_SOURCE_PROFILE, 5000, 0),
                stay("no such short", PlaybackSource::PLAYBACK_SOURCE_PROFILE, 5000, 0),
            ],
        )
        .await;
    assert_eq!((ignored.accepted, ignored.views_counted), (0, 0));
    assert_eq!(ctx.report(None, Vec::new()).await.accepted, 0);
    ctx.flush().await;
    assert_eq!(ctx.row(&hidden.id).await.view_count, 0);

    // "Not interested" needs a short the caller can see; taking it back does not.
    let mark = MarkNotInterestedRequest { short_id: hidden.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(FEEDS, "MarkNotInterested", Some(&bob), &mark).await, "SHORT_NOT_FOUND");
    assert_eq!(ctx.reason(FEEDS, "MarkNotInterested", None, &mark).await, "SIGN_IN_REQUIRED");
    let undo = UndoNotInterestedRequest { short_id: hidden.id.clone(), ..Default::default() };
    let _: UndoNotInterestedResponse = ctx.call(FEEDS, "UndoNotInterested", Some(&bob), &undo).await.unwrap();
    let nonsense = UndoNotInterestedRequest { short_id: "!!".into(), ..Default::default() };
    assert_eq!(ctx.reason(FEEDS, "UndoNotInterested", Some(&bob), &nonsense).await, "INVALID_ID");
}
