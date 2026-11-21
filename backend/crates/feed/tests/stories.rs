//! Map stories: reels of friends' recent located activity, the tray, and
//! the private seen state.
mod common;

use chrono::{Duration, Utc};
use common::*;
use tokuly_core::outbound::ActivityAudience;
use tokuly_feed::FeedConfig;
use tokuly_proto::feed::*;
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

async fn stories(ctx: &Ctx, user: &TestUser) -> ListMapStoriesResponse {
    ctx.call(STORIES, "ListMapStories", Some(user), &ListMapStoriesRequest::default()).await.expect("map stories")
}

async fn stories_of(ctx: &Ctx, user: &TestUser, other: Uuid) -> Result<ListMapStoriesResponse, RpcError> {
    ctx.call(
        STORIES,
        "ListMapStories",
        Some(user),
        &ListMapStoriesRequest { user_id: other.to_string(), ..Default::default() },
    )
    .await
}

async fn tray(ctx: &Ctx, user: &TestUser) -> GetMapStoryTrayResponse {
    ctx.call(STORIES, "GetMapStoryTray", Some(user), &GetMapStoryTrayRequest::default()).await.expect("tray")
}

async fn mark_seen(ctx: &Ctx, user: &TestUser, ids: &[Uuid]) -> Result<MarkMapStorySeenResponse, RpcError> {
    ctx.call(
        STORIES,
        "MarkMapStorySeen",
        Some(user),
        &MarkMapStorySeenRequest { activity_ids: ids.iter().map(Uuid::to_string).collect(), ..Default::default() },
    )
    .await
}

fn reel_user(reel: &MapStoryReel) -> Uuid {
    reel.user.as_option().expect("a reel has a user").id.parse().unwrap()
}

fn item_ids(reel: &MapStoryReel) -> Vec<String> {
    reel.items.iter().map(|item| item.activity.as_option().expect("an item has an entry").id.clone()).collect()
}

#[tokio::test]
async fn reels_group_friends_recent_located_activity() {
    let ctx = start().await;
    let (me, anna, ben, stranger) = (
        ctx.app.user("me").await,
        ctx.app.user("anna").await,
        ctx.app.user("ben").await,
        ctx.app.user("stranger").await,
    );
    ctx.befriend(&me, &anna).await;
    ctx.befriend(&me, &ben).await;
    let now = Utc::now();

    // Anna: two located entries inside the window (one with an image)...
    let anna_first = ctx
        .record_at(
            entry(&anna, "geo.visit", ActivityAudience::Friends)
                .at(35.66, 139.70)
                .image("https://cdn.example/shibuya.webp"),
            now - Duration::hours(30),
        )
        .await;
    let anna_second = ctx.record_at(bump(&anna, &ben, 35.69, 139.69), now - Duration::hours(2)).await;
    // ...and what is not a map story: too old, no location, private.
    ctx.record_at(entry(&anna, "geo.visit", ActivityAudience::Friends).at(35.0, 139.0), now - Duration::hours(49))
        .await;
    ctx.record_at(note(&anna), now - Duration::hours(1)).await;
    ctx.record_at(entry(&anna, "geo.visit", ActivityAudience::Private).at(35.1, 139.1), now - Duration::hours(1)).await;
    // Ben: one, more recent than Anna's.
    let ben_only = ctx
        .record_at(
            entry(&ben, "geo.new_place", ActivityAudience::Public).at(34.70, 135.50),
            now - Duration::minutes(10),
        )
        .await;
    // A stranger's public located entry is not a friend's story.
    ctx.record_at(
        entry(&stranger, "geo.new_place", ActivityAudience::Public).at(34.0, 135.0),
        now - Duration::minutes(5),
    )
    .await;
    // My own: shown to me separately.
    let mine = ctx
        .record_at(entry(&me, "geo.visit", ActivityAudience::Friends).at(43.06, 141.35), now - Duration::minutes(3))
        .await;
    ctx.record_at(entry(&me, "geo.visit", ActivityAudience::Private).at(43.0, 141.0), now - Duration::minutes(2)).await;

    let response = stories(&ctx, &me).await;
    assert_eq!(response.window_hours, 48);
    assert_eq!(
        response.reels.iter().map(reel_user).collect::<Vec<_>>(),
        vec![ben.id, anna.id],
        "most recent reel first"
    );
    let anna_reel = &response.reels[1];
    assert_eq!(item_ids(anna_reel), [anna_first, anna_second].map(|id| id.to_string()), "a reel plays oldest first");
    assert_eq!(anna_reel.unseen_count, 2);
    assert!(anna_reel.items.iter().all(|item| !item.seen));
    assert_eq!(
        anna_reel.latest_location.as_option().map(|l| (l.lat, l.lng)),
        Some((35.69, 139.69)),
        "the reel sits where its newest item happened"
    );
    let first_item = anna_reel.items[0].activity.as_option().unwrap();
    assert_eq!(first_item.location.as_option().map(|l| (l.lat, l.lng)), Some((35.66, 139.70)));
    assert_eq!(first_item.image_url, "https://cdn.example/shibuya.webp");
    assert_eq!(item_ids(&response.reels[0]), vec![ben_only.to_string()]);

    // My own reel is what my friends see of me: never the private entry, never "unseen".
    let my_reel = response.my_reel.as_option().expect("my reel");
    assert_eq!(item_ids(my_reel), vec![mine.to_string()]);
    assert_eq!(my_reel.unseen_count, 0);

    // One friend's reel; my own; and people who are not my friends.
    let only_anna = stories_of(&ctx, &me, anna.id).await.unwrap();
    assert_eq!(only_anna.reels.iter().map(reel_user).collect::<Vec<_>>(), vec![anna.id]);
    let only_me = stories_of(&ctx, &me, me.id).await.unwrap();
    assert!(only_me.reels.is_empty());
    assert_eq!(item_ids(only_me.my_reel.as_option().unwrap()), vec![mine.to_string()]);
    assert_eq!(stories_of(&ctx, &me, stranger.id).await.unwrap_err().reason, "USER_NOT_FOUND");
    assert_eq!(stories_of(&ctx, &me, Uuid::now_v7()).await.unwrap_err().reason, "USER_NOT_FOUND");
    let bad_id = ctx
        .call::<_, ListMapStoriesResponse>(
            STORIES,
            "ListMapStories",
            Some(&me),
            &ListMapStoriesRequest { user_id: "nope".into(), ..Default::default() },
        )
        .await;
    assert_eq!(bad_id.unwrap_err().reason, "INVALID_ID");

    // A friend without stories gives an empty list, not an error.
    let quiet = ctx.app.user("quiet").await;
    ctx.befriend(&me, &quiet).await;
    let none = stories_of(&ctx, &me, quiet.id).await.unwrap();
    assert!(none.reels.is_empty());
    // Someone with no friends and no stories.
    let lonely = stories(&ctx, &stranger).await;
    assert!(lonely.reels.is_empty());
    assert_eq!(item_ids(lonely.my_reel.as_option().unwrap()).len(), 1, "their own public located entry is their reel");

    // Signed-in users only.
    let anonymous =
        ctx.call::<_, ListMapStoriesResponse>(STORIES, "ListMapStories", None, &ListMapStoriesRequest::default()).await;
    assert_eq!(anonymous.unwrap_err().code, "unauthenticated");
    let anonymous = ctx
        .call::<_, GetMapStoryTrayResponse>(STORIES, "GetMapStoryTray", None, &GetMapStoryTrayRequest::default())
        .await;
    assert_eq!(anonymous.unwrap_err().code, "unauthenticated");
}

#[tokio::test]
async fn seen_state_is_per_viewer_and_orders_reels_and_the_tray() {
    let ctx = start().await;
    let (me, anna, ben, carol) =
        (ctx.app.user("me").await, ctx.app.user("anna").await, ctx.app.user("ben").await, ctx.app.user("carol").await);
    for friend in [&anna, &ben] {
        ctx.befriend(&me, friend).await;
    }
    ctx.befriend(&carol, &anna).await;
    let now = Utc::now();
    let anna_old = ctx
        .record_at(
            entry(&anna, "geo.visit", ActivityAudience::Friends).at(35.0, 139.0).image("https://cdn.example/a1.webp"),
            now - Duration::hours(5),
        )
        .await;
    let anna_new = ctx
        .record_at(entry(&anna, "geo.visit", ActivityAudience::Friends).at(35.2, 139.2), now - Duration::minutes(5))
        .await;
    let ben_only = ctx
        .record_at(entry(&ben, "geo.visit", ActivityAudience::Friends).at(34.0, 135.0), now - Duration::hours(1))
        .await;

    let before = tray(&ctx, &me).await;
    assert_eq!(before.unseen_reel_count, 2);
    assert_eq!(
        before.entries.iter().map(|e| e.user.as_option().unwrap().id.clone()).collect::<Vec<_>>(),
        [anna.id, ben.id].map(|id| id.to_string())
    );
    let anna_entry = &before.entries[0];
    assert_eq!((anna_entry.item_count, anna_entry.unseen_count), (2, 2));
    assert_eq!(anna_entry.preview_image_url, "https://cdn.example/a1.webp", "the newest item that has an image");
    assert_eq!(anna_entry.latest_location.as_option().map(|l| (l.lat, l.lng)), Some((35.2, 139.2)));

    // Watching both of Anna's items; marking again changes nothing.
    assert_eq!(mark_seen(&ctx, &me, &[anna_old, anna_new]).await.unwrap().marked_count, 2);
    assert_eq!(mark_seen(&ctx, &me, &[anna_old, anna_new]).await.unwrap().marked_count, 0);

    let after = stories(&ctx, &me).await;
    assert_eq!(
        after.reels.iter().map(reel_user).collect::<Vec<_>>(),
        vec![ben.id, anna.id],
        "the reel with something unseen leads, though Anna's is newer"
    );
    assert_eq!(after.reels[1].unseen_count, 0);
    assert!(after.reels[1].items.iter().all(|item| item.seen));
    assert_eq!(after.reels[0].unseen_count, 1);
    let after_tray = tray(&ctx, &me).await;
    assert_eq!(after_tray.unseen_reel_count, 1);
    assert_eq!(after_tray.entries.iter().map(|e| e.unseen_count).collect::<Vec<_>>(), vec![1, 0]);

    // Seen state is mine alone: Carol (Anna's other friend) still has everything unseen.
    assert_eq!(stories(&ctx, &carol).await.reels[0].unseen_count, 2);

    // A new item makes the reel unseen again.
    let anna_newest = ctx
        .record_at(entry(&anna, "geo.visit", ActivityAudience::Friends).at(35.3, 139.3), now - Duration::minutes(1))
        .await;
    let again = stories(&ctx, &me).await;
    assert_eq!(again.reels.iter().map(reel_user).collect::<Vec<_>>(), vec![anna.id, ben.id]);
    assert_eq!(again.reels[0].unseen_count, 1);
    assert_eq!(again.reels[0].items.last().unwrap().activity.as_option().unwrap().id, anna_newest.to_string());
    let _ = ben_only;
}

#[tokio::test]
async fn only_what_the_viewer_can_watch_is_marked_and_nobody_is_told() {
    let ctx = start().await;
    let (me, anna, stranger) = (ctx.app.user("me").await, ctx.app.user("anna").await, ctx.app.user("stranger").await);
    ctx.befriend(&me, &anna).await;
    let now = Utc::now();
    let story = ctx
        .record_at(entry(&anna, "geo.visit", ActivityAudience::Friends).at(35.0, 139.0), now - Duration::hours(1))
        .await;
    let expired = ctx
        .record_at(entry(&anna, "geo.visit", ActivityAudience::Friends).at(35.0, 139.0), now - Duration::hours(50))
        .await;
    let unlocated = ctx.record_at(note(&anna), now - Duration::hours(1)).await;
    let private = ctx
        .record_at(entry(&anna, "geo.visit", ActivityAudience::Private).at(35.0, 139.0), now - Duration::hours(1))
        .await;
    let strangers = ctx
        .record_at(entry(&stranger, "geo.visit", ActivityAudience::Public).at(35.0, 139.0), now - Duration::hours(1))
        .await;
    let mine = ctx
        .record_at(entry(&me, "geo.visit", ActivityAudience::Friends).at(35.0, 139.0), now - Duration::hours(1))
        .await;

    // Everything I caused so far, and every notification addressed to Anna.
    let caused_by_me = ctx.app.events_by("", me.id).await.len();
    let for_anna = |events: Vec<tokuly_core::events::Event>| {
        events.into_iter().filter(|e| e.data["user_id"] == serde_json::json!(anna.id)).count()
    };
    let notices_before = for_anna(ctx.app.events("notify.requested").await);
    let marked =
        mark_seen(&ctx, &me, &[story, expired, unlocated, private, strangers, mine, Uuid::now_v7()]).await.unwrap();
    assert_eq!(marked.marked_count, 1, "only the friend's current map story counts");
    let rows: Vec<(Uuid, Uuid)> =
        sqlx::query_as("SELECT activity_id, author_id FROM map_story_views WHERE viewer_id = $1")
            .bind(me.id)
            .fetch_all(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(rows, vec![(story, anna.id)]);
    // Viewing is private: no event, no notification, nothing for the author.
    assert_eq!(ctx.app.events_by("", me.id).await.len(), caused_by_me, "marking a story as seen emits nothing");
    assert_eq!(for_anna(ctx.app.events("notify.requested").await), notices_before, "and the author is not notified");

    // A stranger cannot mark (or probe) someone's stories.
    assert_eq!(mark_seen(&ctx, &stranger, &[story]).await.unwrap().marked_count, 0);

    // Validation.
    assert_eq!(mark_seen(&ctx, &me, &[]).await.unwrap_err().reason, "IDS_REQUIRED");
    let too_many: Vec<Uuid> = (0..201).map(|_| Uuid::now_v7()).collect();
    assert_eq!(mark_seen(&ctx, &me, &too_many).await.unwrap_err().reason, "TOO_MANY_IDS");
    let bad = ctx
        .call::<_, MarkMapStorySeenResponse>(
            STORIES,
            "MarkMapStorySeen",
            Some(&me),
            &MarkMapStorySeenRequest { activity_ids: vec!["nope".into()], ..Default::default() },
        )
        .await;
    assert_eq!(bad.unwrap_err().reason, "INVALID_ID");
    let anonymous = ctx
        .call::<_, MarkMapStorySeenResponse>(
            STORIES,
            "MarkMapStorySeen",
            None,
            &MarkMapStorySeenRequest { activity_ids: vec![story.to_string()], ..Default::default() },
        )
        .await;
    assert_eq!(anonymous.unwrap_err().code, "unauthenticated");
}

#[tokio::test]
async fn stories_respect_hidden_kinds_blocks_and_the_end_of_a_friendship() {
    let ctx = start().await;
    let (me, anna, ben, mallory) = (
        ctx.app.user("me").await,
        ctx.app.user("anna").await,
        ctx.app.user("ben").await,
        ctx.app.user("mallory").await,
    );
    for friend in [&anna, &ben] {
        ctx.befriend(&me, friend).await;
    }
    ctx.befriend(&anna, &mallory).await;
    let now = Utc::now();
    let anna_visit = ctx
        .record_at(entry(&anna, "geo.visit", ActivityAudience::Friends).at(35.0, 139.0), now - Duration::hours(3))
        .await;
    let anna_bump = ctx.record_at(bump(&anna, &mallory, 35.1, 139.1), now - Duration::hours(2)).await;
    let ben_visit = ctx
        .record_at(entry(&ben, "geo.visit", ActivityAudience::Friends).at(34.0, 135.0), now - Duration::hours(1))
        .await;
    assert_eq!(mark_seen(&ctx, &me, &[anna_visit, anna_bump, ben_visit]).await.unwrap().marked_count, 3);

    // Anna hides her map activity: those items leave her reel (the bump stays).
    let _: SetActivityKindVisibilityResponse = ctx
        .call(
            ACTIVITY,
            "SetActivityKindVisibility",
            Some(&anna),
            &SetActivityKindVisibilityRequest { kind: "geo".into(), visible_to_others: false, ..Default::default() },
        )
        .await
        .unwrap();
    let response = stories(&ctx, &me).await;
    let anna_reel = response.reels.iter().find(|reel| reel_user(reel) == anna.id).expect("anna still has a reel");
    assert_eq!(item_ids(anna_reel), vec![anna_bump.to_string()]);

    // I block Mallory: Anna's bump with Mallory is not shown to me anymore.
    ctx.block(&me, &mallory).await;
    let response = stories(&ctx, &me).await;
    assert_eq!(response.reels.iter().map(reel_user).collect::<Vec<_>>(), vec![ben.id]);
    assert_eq!(tray(&ctx, &me).await.entries.len(), 1);

    // Ben and I stop being friends: his reel is gone for me, and when the
    // event arrives what I had watched of his is forgotten.
    ctx.unfriend(&me, &ben).await;
    assert!(stories(&ctx, &me).await.reels.is_empty());
    assert_eq!(stories_of(&ctx, &me, ben.id).await.unwrap_err().reason, "USER_NOT_FOUND");
    ctx.feed
        .handle_event(event(
            "social.friend.removed",
            Some(me.id),
            serde_json::json!({ "user_id": me.id, "friend_id": ben.id, "blocked": false }),
        ))
        .await
        .unwrap();
    let left: Vec<Uuid> =
        sqlx::query_scalar("SELECT author_id FROM map_story_views WHERE viewer_id = $1 ORDER BY author_id")
            .bind(me.id)
            .fetch_all(&ctx.app.core.db)
            .await
            .unwrap();
    assert!(!left.contains(&ben.id), "seen marks between former friends are dropped");
    assert!(left.contains(&anna.id));
    // A block drops them as well.
    ctx.feed
        .handle_event(event(
            "social.user.blocked",
            Some(anna.id),
            serde_json::json!({ "user_id": anna.id, "blocked_id": me.id }),
        ))
        .await
        .unwrap();
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM map_story_views WHERE viewer_id = $1")
        .bind(me.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn the_story_window_is_configurable() {
    let ctx = start_with(FeedConfig { map_story_hours: 6, ..test_config() }).await;
    let (me, anna) = (ctx.app.user("me").await, ctx.app.user("anna").await);
    ctx.befriend(&me, &anna).await;
    let now = Utc::now();
    let recent = ctx
        .record_at(entry(&anna, "geo.visit", ActivityAudience::Friends).at(35.0, 139.0), now - Duration::hours(5))
        .await;
    let older = ctx
        .record_at(entry(&anna, "geo.visit", ActivityAudience::Friends).at(35.0, 139.0), now - Duration::hours(7))
        .await;
    let response = stories(&ctx, &me).await;
    assert_eq!(response.window_hours, 6);
    assert_eq!(item_ids(&response.reels[0]), vec![recent.to_string()]);
    assert_eq!(mark_seen(&ctx, &me, &[recent, older]).await.unwrap().marked_count, 1);
}
