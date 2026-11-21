//! Stories (記憶): posting, who sees them and for how long, the tray,
//! viewers and reactions, the archive and its calendar.
mod common;

use chrono::{DateTime, Utc};
use common::*;
use tokuly_notes::NotesConfig;
use tokuly_proto::{
    media::{AssetKind, AssetPurpose},
    notes::{notes_event::Event as Push, *},
};
use tokuly_testkit::TestUser;
use uuid::Uuid;

fn story_ref(story_id: &str) -> GetStoryRequest {
    GetStoryRequest { story_id: story_id.to_owned(), ..Default::default() }
}

fn item(asset_id: String) -> StoryItemInput {
    StoryItemInput { asset_id, ..Default::default() }
}

async fn react(ctx: &Ctx, user: &TestUser, story: &Story, emoji: &str) {
    let request = ReactToStoryRequest { story_id: story.id.clone(), emoji: emoji.to_owned(), ..Default::default() };
    let _: ReactToStoryResponse = ctx
        .story("ReactToStory", Some(user), &request)
        .await
        .unwrap_or_else(|e| panic!("reacting with {emoji} failed: {e}"));
}

async fn tray(ctx: &Ctx, user: &TestUser) -> Vec<StoryReel> {
    let response: GetStoryTrayResponse =
        ctx.story("GetStoryTray", Some(user), &GetStoryTrayRequest::default()).await.unwrap();
    response.reels
}

/// Moves a story to a moment in the past, as if it had been posted then.
async fn posted_at(ctx: &Ctx, story: &Story, at: &str) {
    let at: DateTime<Utc> = at.parse().unwrap();
    sqlx::query("UPDATE stories SET created_at = $2, expires_at = $2 + interval '24 hours' WHERE id = $1")
        .bind(id(&story.id))
        .bind(at)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_story_is_posted_to_the_owners_friends() {
    let ctx = start().await;
    let (alice, friend, stranger) =
        (ctx.app.user("alice").await, ctx.app.user("friend").await, ctx.app.user("stranger").await);
    ctx.befriend(&alice, &friend).await;
    let mut friend_feed = ctx.app.listen_user(friend.id).await;
    let mut stranger_feed = ctx.app.listen_user(stranger.id).await;

    // Stories need no agreement to the note rules.
    let story = ctx.story_with(&alice, "  海に行った!\n最高  ", 3, |_| {}).await;
    assert_eq!(story.title, "海に行った! 最高");
    assert_eq!(story.owner.id, alice.id.to_string());
    assert_eq!(
        story.items.iter().map(|item| item.caption.as_str()).collect::<Vec<_>>(),
        vec!["photo 1", "photo 2", "photo 3"]
    );
    assert!(story.items.iter().all(|item| item.media.image.as_option().is_some_and(|image| !image.url.is_empty())));
    assert_eq!(story.audience, StoryAudience::STORY_AUDIENCE_FRIENDS);
    assert_eq!(story.expires_at.seconds - story.created_at.seconds, 24 * 3600);
    assert!(!story.expired && story.viewed && story.my_reaction.is_empty() && !story.highlighted);
    assert_eq!((story.view_count, story.reaction_count), (0, 0));
    for item in &story.items {
        assert!(
            tokuly_media::assets::is_attached(&ctx.app.core, id(&item.media.asset_id)).await.unwrap(),
            "a story holds its uploads"
        );
    }

    let story_id = id(&story.id);
    let created = ctx.app.events_by("notes.story.created", alice.id).await;
    assert_eq!(created.len(), 1);
    assert_eq!(
        (created[0].data["story_id"].clone(), created[0].data["audience"].clone()),
        (serde_json::json!(story_id), serde_json::json!(1))
    );
    let activity = ctx.activities_of(alice.id, "story.posted").await;
    assert_eq!(activity.len(), 1);
    assert_eq!(activity[0].data["audience"], "friends");
    assert_eq!(activity[0].data["subject"], serde_json::json!([4, story_id]));
    assert_eq!(activity[0].data["path"], format!("/stories/{story_id}"));
    assert!(activity[0].data["image_url"].is_string());

    match next_push(&mut friend_feed).await {
        Some(Push::StoryPosted(pushed)) => {
            assert_eq!(pushed.id, story.id);
            assert!(!pushed.viewed && pushed.view_count == 0, "friends get the story as they will see it");
        }
        other => panic!("expected the new story, got {other:?}"),
    }
    assert!(pushes(&mut stranger_feed, QUIET).await.is_empty());

    let seen = ctx.get_story(&friend, &story.id).await.unwrap();
    assert!(!seen.viewed && seen.audience_user_ids.is_empty());
    assert_eq!(ctx.get_story(&stranger, &story.id).await.unwrap_err().reason, "STORY_NOT_FOUND");
    assert_eq!(ctx.story_reason("GetStory", None, &story_ref(&story.id)).await, "SIGN_IN_REQUIRED");
    assert_eq!(
        ctx.story_reason("GetStory", Some(&friend), &story_ref(&Uuid::now_v7().to_string())).await,
        "STORY_NOT_FOUND"
    );
    assert_eq!(ctx.story_reason("GetStory", Some(&friend), &story_ref("nope")).await, "INVALID_ID");

    // After a day the story is the owner's alone.
    ctx.age_story(&story, 25).await;
    assert_eq!(ctx.get_story(&friend, &story.id).await.unwrap_err().reason, "STORY_NOT_FOUND");
    assert!(ctx.get_story(&alice, &story.id).await.unwrap().expired);
}

#[tokio::test]
async fn posting_a_story_is_validated() {
    let ctx = start_with(NotesConfig { stories_per_hour: 4, ..Default::default() }).await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let photo = ctx.story_image(&alice).await;
    let request = |fill: &dyn Fn(&mut CreateStoryRequest)| {
        let mut request =
            CreateStoryRequest { title: "t".into(), items: vec![item(photo.clone())], ..Default::default() };
        fill(&mut request);
        request
    };
    let reason = |request: CreateStoryRequest| {
        let ctx = &ctx;
        let alice = alice.clone();
        async move { ctx.story_reason("CreateStory", Some(&alice), &request).await }
    };

    assert_eq!(ctx.story_reason("CreateStory", None, &request(&|_| {})).await, "SIGN_IN_REQUIRED");
    assert_eq!(reason(request(&|r| r.items.clear())).await, "STORY_EMPTY");
    assert_eq!(
        reason(request(&|r| r.items = (0..11).map(|_| item(Uuid::now_v7().to_string())).collect())).await,
        "TOO_MANY_ITEMS"
    );
    assert_eq!(reason(request(&|r| r.title = "あ".repeat(141))).await, "TITLE_TOO_LONG");
    assert_eq!(reason(request(&|r| r.items[0].caption = "あ".repeat(201))).await, "CAPTION_TOO_LONG");
    assert_eq!(reason(request(&|r| r.items[0].asset_id = "nope".into())).await, "INVALID_ID");
    assert_eq!(reason(request(&|r| r.items[0].asset_id = Uuid::now_v7().to_string())).await, "ASSET_NOT_FOUND");
    let someone_elses = ctx.story_image(&bob).await;
    assert_eq!(reason(request(&|r| r.items[0].asset_id = someone_elses.clone())).await, "ASSET_NOT_FOUND");
    let for_notes = ctx.note_image(&alice).await;
    assert_eq!(reason(request(&|r| r.items[0].asset_id = for_notes.clone())).await, "ASSET_WRONG_PURPOSE");
    let voice = ctx.upload(&alice, AssetPurpose::ASSET_PURPOSE_STORY, AssetKind::ASSET_KIND_AUDIO).await;
    assert_eq!(reason(request(&|r| r.items[0].asset_id = voice.clone())).await, "ATTACHMENT_UNSUPPORTED");
    assert_eq!(reason(request(&|r| r.idempotency_key = "k".repeat(65))).await, "INVALID_IDEMPOTENCY_KEY");

    let close = StoryAudience::STORY_AUDIENCE_CLOSE_FRIENDS;
    assert_eq!(reason(request(&|r| r.audience = close.into())).await, "AUDIENCE_EMPTY");
    assert_eq!(
        reason(request(&|r| {
            r.audience = close.into();
            r.audience_user_ids = (0..151).map(|_| Uuid::now_v7().to_string()).collect();
        }))
        .await,
        "AUDIENCE_TOO_LARGE"
    );
    assert_eq!(
        reason(request(&|r| {
            r.audience = close.into();
            r.audience_user_ids = vec![bob.id.to_string()];
        }))
        .await,
        "AUDIENCE_NOT_FRIENDS"
    );
    // Chosen friends are ignored for a story that goes to every friend.
    let open: CreateStoryResponse = ctx
        .story("CreateStory", Some(&alice), &request(&|r| r.audience_user_ids = vec![bob.id.to_string()]))
        .await
        .unwrap();
    assert_eq!(open.story.audience, StoryAudience::STORY_AUDIENCE_FRIENDS);

    // A video is a story item too; a story can have no title; a retried request posts once.
    let video = ctx.upload(&alice, AssetPurpose::ASSET_PURPOSE_STORY, AssetKind::ASSET_KIND_VIDEO).await;
    let key = unique("key");
    let untitled = request(&|r| {
        r.title.clear();
        r.items = vec![item(video.clone())];
        r.idempotency_key = key.clone();
    });
    let first: CreateStoryResponse = ctx.story("CreateStory", Some(&alice), &untitled).await.unwrap();
    let again: CreateStoryResponse = ctx.story("CreateStory", Some(&alice), &untitled).await.unwrap();
    assert_eq!(first.story.id, again.story.id);
    assert!(first.story.title.is_empty() && first.story.items[0].media.video.as_option().is_some());

    // Two stories were posted; the limit of this app is four an hour.
    for _ in 0..2 {
        let _: CreateStoryResponse = ctx.story("CreateStory", Some(&alice), &request(&|_| {})).await.unwrap();
    }
    assert_eq!(reason(request(&|_| {})).await, "RATE_LIMITED");
}

#[tokio::test]
async fn a_close_friends_story_is_only_for_the_chosen() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);
    ctx.befriend(&alice, &bob).await;
    ctx.befriend(&alice, &carol).await;
    let mut bob_feed = ctx.app.listen_user(bob.id).await;
    let mut carol_feed = ctx.app.listen_user(carol.id).await;

    let story = ctx
        .story_with(&alice, "ないしょ", 1, |r| {
            r.audience = StoryAudience::STORY_AUDIENCE_CLOSE_FRIENDS.into();
            r.audience_user_ids = vec![bob.id.to_string(), bob.id.to_string()];
        })
        .await;
    assert_eq!(story.audience, StoryAudience::STORY_AUDIENCE_CLOSE_FRIENDS);
    assert_eq!(story.audience_user_ids, vec![bob.id.to_string()], "the owner sees who was chosen");

    assert!(matches!(next_push(&mut bob_feed).await, Some(Push::StoryPosted(_))));
    assert!(pushes(&mut carol_feed, QUIET).await.is_empty());
    let for_bob = ctx.get_story(&bob, &story.id).await.unwrap();
    assert_eq!(for_bob.audience, StoryAudience::STORY_AUDIENCE_CLOSE_FRIENDS);
    assert!(for_bob.audience_user_ids.is_empty(), "the chosen do not learn who else was");
    assert_eq!(ctx.get_story(&carol, &story.id).await.unwrap_err().reason, "STORY_NOT_FOUND");
    assert_eq!(tray(&ctx, &bob).await.len(), 1);
    assert!(tray(&ctx, &carol).await.is_empty());
    // The friends feed cannot tell who was chosen, so the activity stays in the owner's own log.
    assert_eq!(ctx.activities_of(alice.id, "story.posted").await[0].data["audience"], "private");

    // Someone who stops being a friend stops seeing the story at once.
    ctx.block(&bob, &alice).await;
    assert_eq!(ctx.get_story(&bob, &story.id).await.unwrap_err().reason, "STORY_NOT_FOUND");
}

#[tokio::test]
async fn the_tray_orders_reels_by_what_is_new_to_the_viewer() {
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
    ctx.post_story(&bob, "b1").await;
    ctx.post_story(&bob, "b2").await;
    let carols = ctx.post_story(&carol, "c1").await;
    let daves = ctx.post_story(&dave, "d1").await;
    ctx.post_story(&eve, "e1").await;
    ctx.post_story(&alice, "a1").await;
    ctx.watch(&alice, &carols).await;

    let reels = tray(&ctx, &alice).await;
    let owners: Vec<&str> = reels.iter().map(|reel| reel.user.id.as_str()).collect();
    // Own reel, then friends with something unseen (newest first), then the rest; never strangers.
    assert_eq!(owners, vec![alice.id.to_string(), dave.id.to_string(), bob.id.to_string(), carol.id.to_string()]);
    assert_eq!(reels.iter().map(|reel| reel.has_unseen).collect::<Vec<_>>(), vec![false, true, true, false]);
    let bobs = &reels[2];
    assert_eq!(
        bobs.stories.iter().map(|story| story.title.as_str()).collect::<Vec<_>>(),
        vec!["b1", "b2"],
        "a reel plays oldest first"
    );
    assert_eq!(bobs.latest_at, bobs.stories[1].created_at);
    assert!(reels[3].stories[0].viewed);

    // Watching everything of a friend moves their reel behind the unseen ones.
    for story in &reels[1].stories {
        ctx.watch(&alice, story).await;
    }
    let owners: Vec<String> = tray(&ctx, &alice).await.iter().map(|reel| reel.user.id.clone()).collect();
    assert_eq!(owners, vec![alice.id.to_string(), bob.id.to_string(), dave.id.to_string(), carol.id.to_string()]);
    // Expired stories leave the tray.
    ctx.age_story(&daves, 25).await;
    assert_eq!(tray(&ctx, &alice).await.len(), 3);
    assert_eq!(ctx.story_reason("GetStoryTray", None, &GetStoryTrayRequest::default()).await, "SIGN_IN_REQUIRED");
    assert_eq!(tray(&ctx, &eve).await.len(), 1, "someone without friends has their own reel");

    // The live stories of one person, as on their profile.
    let of = |user_id: String| ListUserStoriesRequest { user_id, ..Default::default() };
    let bobs: ListUserStoriesResponse =
        ctx.story("ListUserStories", Some(&alice), &of(bob.id.to_string())).await.unwrap();
    assert_eq!(bobs.stories.iter().map(|story| story.title.as_str()).collect::<Vec<_>>(), vec!["b1", "b2"]);
    let expired: ListUserStoriesResponse =
        ctx.story("ListUserStories", Some(&alice), &of(dave.id.to_string())).await.unwrap();
    assert!(expired.stories.is_empty());
    let not_a_friend: ListUserStoriesResponse =
        ctx.story("ListUserStories", Some(&eve), &of(bob.id.to_string())).await.unwrap();
    assert!(not_a_friend.stories.is_empty());
    let nobody: ListUserStoriesResponse =
        ctx.story("ListUserStories", Some(&alice), &of(Uuid::now_v7().to_string())).await.unwrap();
    assert!(nobody.stories.is_empty());
    assert_eq!(ctx.story_reason("ListUserStories", None, &of(bob.id.to_string())).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn viewers_and_reactions_are_counted_once_and_shown_to_the_owner() {
    let ctx = start().await;
    let (alice, bob, carol, dave, stranger) = (
        ctx.app.user("alice").await,
        ctx.app.user("bob").await,
        ctx.app.user("carol").await,
        ctx.app.user("dave").await,
        ctx.app.user("stranger").await,
    );
    for friend in [&bob, &carol, &dave] {
        ctx.befriend(&alice, friend).await;
    }
    let story = ctx.post_story(&alice, "見て").await;
    let mut owner_feed = ctx.app.listen_user(alice.id).await;
    let viewed = |story_id: &str| MarkStoryViewedRequest { story_id: story_id.to_owned(), ..Default::default() };
    let reaction =
        |emoji: &str| ReactToStoryRequest { story_id: story.id.clone(), emoji: emoji.to_owned(), ..Default::default() };

    ctx.watch(&bob, &story).await;
    assert_eq!(
        next_push(&mut owner_feed).await,
        Some(Push::StoryViewed(Box::new(StoryViewCount {
            story_id: story.id.clone(),
            view_count: 1,
            ..Default::default()
        })))
    );
    ctx.watch(&bob, &story).await;
    ctx.watch(&alice, &story).await;
    assert_eq!(
        ctx.get_story(&alice, &story.id).await.unwrap().view_count,
        1,
        "repeat views and the owner's own do not count"
    );
    assert_eq!(ctx.story_reason("MarkStoryViewed", Some(&stranger), &viewed(&story.id)).await, "STORY_NOT_FOUND");
    assert_eq!(ctx.story_reason("MarkStoryViewed", None, &viewed(&story.id)).await, "SIGN_IN_REQUIRED");

    // Reacting counts as watching.
    react(&ctx, &carol, &story, "❤️").await;
    let pushed = pushes(&mut owner_feed, QUIET).await;
    assert!(pushed.iter().any(|push| matches!(push, Push::StoryViewed(count) if count.view_count == 2)));
    assert!(pushed.iter().any(|push| matches!(push, Push::StoryReaction(reaction) if reaction.emoji == "❤️" && reaction.user.id == carol.id.to_string())));
    react(&ctx, &carol, &story, "😂").await;
    react(&ctx, &carol, &story, "😂").await;
    react(&ctx, &bob, &story, "👍").await;
    let own = ctx.get_story(&alice, &story.id).await.unwrap();
    assert_eq!((own.view_count, own.reaction_count), (2, 2), "a changed emoji is still one reaction");
    let carols = ctx.get_story(&carol, &story.id).await.unwrap();
    assert!(carols.viewed && carols.my_reaction == "😂" && carols.view_count == 0, "the numbers are the owner's");

    let notified = ctx.notifications_for(alice.id, "notes.story_reaction").await;
    assert_eq!(notified.len(), 3, "the same emoji again is not announced");
    assert_eq!(notified[0].data["collapse_key"], format!("notes.story_reaction:{}:{}", story.id, carol.id));
    assert_eq!(
        notified[0].data["collapse_key"], notified[1].data["collapse_key"],
        "a changed emoji replaces the notification"
    );
    assert_eq!(notified[1].data["data"]["emoji"], "😂");
    assert_eq!(notified[0].data["path"], format!("/stories/{}", story.id));

    assert_eq!(ctx.story_reason("ReactToStory", Some(&dave), &reaction("ab")).await, "INVALID_REACTION");
    assert_eq!(ctx.story_reason("ReactToStory", Some(&dave), &reaction("😂😂")).await, "INVALID_REACTION");
    assert_eq!(ctx.story_reason("ReactToStory", Some(&dave), &reaction("")).await, "INVALID_REACTION");
    assert_eq!(ctx.story_reason("ReactToStory", Some(&alice), &reaction("😂")).await, "CANNOT_REACT_TO_OWN_STORY");
    assert_eq!(ctx.story_reason("ReactToStory", Some(&stranger), &reaction("😂")).await, "STORY_NOT_FOUND");
    assert_eq!(ctx.story_reason("ReactToStory", None, &reaction("😂")).await, "SIGN_IN_REQUIRED");

    // The viewer list is the owner's, most recent first, with reactions.
    let viewers = |user: TestUser, cursor: String| {
        let request =
            ListStoryViewersRequest { story_id: story.id.clone(), page: page(1, &cursor), ..Default::default() };
        let ctx = &ctx;
        async move { ctx.story::<_, ListStoryViewersResponse>("ListStoryViewers", Some(&user), &request).await }
    };
    let first = viewers(alice.clone(), String::new()).await.unwrap();
    assert_eq!((first.viewers[0].user.id.clone(), first.viewers[0].reaction.as_str()), (carol.id.to_string(), "😂"));
    assert!(first.viewers[0].viewed_at.as_option().is_some() && first.page.has_more);
    assert_eq!((first.view_count, first.reaction_count), (2, 2));
    let second = viewers(alice.clone(), first.page.next_cursor.clone()).await.unwrap();
    assert_eq!((second.viewers[0].user.id.clone(), second.viewers[0].reaction.as_str()), (bob.id.to_string(), "👍"));
    assert!(!second.page.has_more);
    assert_eq!(
        viewers(bob.clone(), String::new()).await.unwrap_err().reason,
        "STORY_NOT_FOUND",
        "only the owner sees who watched"
    );

    let remove = RemoveStoryReactionRequest { story_id: story.id.clone(), ..Default::default() };
    let _: RemoveStoryReactionResponse = ctx.story("RemoveStoryReaction", Some(&bob), &remove).await.unwrap();
    let _: RemoveStoryReactionResponse = ctx.story("RemoveStoryReaction", Some(&bob), &remove).await.unwrap();
    assert!(ctx.get_story(&bob, &story.id).await.unwrap().my_reaction.is_empty());
    assert_eq!(ctx.get_story(&alice, &story.id).await.unwrap().reaction_count, 1);
    assert_eq!(ctx.story_reason("RemoveStoryReaction", Some(&stranger), &remove).await, "STORY_NOT_FOUND");
    assert_eq!(ctx.story_reason("RemoveStoryReaction", None, &remove).await, "SIGN_IN_REQUIRED");

    // Someone in a block with the owner drops out of the list.
    ctx.block(&carol, &alice).await;
    let all: ListStoryViewersResponse = ctx
        .story(
            "ListStoryViewers",
            Some(&alice),
            &ListStoryViewersRequest { story_id: story.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(all.viewers.iter().map(|viewer| viewer.user.id.clone()).collect::<Vec<_>>(), vec![bob.id.to_string()]);

    // An expired story takes no more views, but a reaction can still be withdrawn.
    react(&ctx, &dave, &story, "🎉").await;
    ctx.age_story(&story, 25).await;
    assert_eq!(ctx.story_reason("MarkStoryViewed", Some(&bob), &viewed(&story.id)).await, "STORY_NOT_FOUND");
    let _: RemoveStoryReactionResponse = ctx.story("RemoveStoryReaction", Some(&dave), &remove).await.unwrap();
    assert_eq!(ctx.story_reason("RemoveStoryReaction", Some(&dave), &remove).await, "STORY_NOT_FOUND");
}

#[tokio::test]
async fn the_archive_is_browsed_by_day_in_the_owners_time_zone() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    let s1 = ctx.post_story(&alice, "s1").await;
    let s2 = ctx.post_story(&alice, "s2").await;
    let s3 = ctx.post_story(&alice, "s3").await;
    let s4 = ctx.post_story(&alice, "s4").await;
    // Test users live in Asia/Tokyo (UTC+9).
    posted_at(&ctx, &s1, "2026-03-14T14:59:00Z").await; // 14th, 23:59
    posted_at(&ctx, &s2, "2026-03-14T15:00:00Z").await; // 15th, 00:00
    posted_at(&ctx, &s3, "2026-03-15T03:00:00Z").await; // 15th, noon
    posted_at(&ctx, &s4, "2026-03-31T16:00:00Z").await; // April 1st, 01:00

    let calendar = |user: TestUser, year: i32, month: i32| {
        let ctx = &ctx;
        async move {
            ctx.story::<_, GetStoryCalendarResponse>(
                "GetStoryCalendar",
                Some(&user),
                &GetStoryCalendarRequest { year, month, ..Default::default() },
            )
            .await
        }
    };
    let march = calendar(alice.clone(), 2026, 3).await.unwrap();
    assert_eq!(
        march.days.iter().map(|day| (day.date.as_str(), day.story_count)).collect::<Vec<_>>(),
        vec![("2026-03-14", 1), ("2026-03-15", 2)]
    );
    assert_eq!(
        march.days[1].cover.asset_id, s2.items[0].media.asset_id,
        "the picture of the day is the first one posted"
    );
    let april = calendar(alice.clone(), 2026, 4).await.unwrap();
    assert_eq!(
        april.days.iter().map(|day| (day.date.as_str(), day.story_count)).collect::<Vec<_>>(),
        vec![("2026-04-01", 1)]
    );
    assert!(calendar(alice.clone(), 2026, 5).await.unwrap().days.is_empty());
    assert!(calendar(bob.clone(), 2026, 3).await.unwrap().days.is_empty(), "the calendar is of one's own stories");
    for (year, month) in [(2026, 0), (2026, 13), (1999, 3)] {
        assert_eq!(calendar(alice.clone(), year, month).await.unwrap_err().reason, "INVALID_MONTH");
    }
    assert_eq!(
        ctx.story_reason(
            "GetStoryCalendar",
            None,
            &GetStoryCalendarRequest { year: 2026, month: 3, ..Default::default() }
        )
        .await,
        "SIGN_IN_REQUIRED"
    );

    let archive = |user: TestUser, date: &str, limit: i32, cursor: String| {
        let request =
            ListArchivedStoriesRequest { date: date.to_owned(), page: page(limit, &cursor), ..Default::default() };
        let ctx = &ctx;
        async move { ctx.story::<_, ListArchivedStoriesResponse>("ListArchivedStories", Some(&user), &request).await }
    };
    let titles = |response: &ListArchivedStoriesResponse| {
        response.stories.iter().map(|story| story.title.clone()).collect::<Vec<_>>()
    };
    let first = archive(alice.clone(), "", 3, String::new()).await.unwrap();
    assert_eq!(titles(&first), vec!["s4", "s3", "s2"]);
    assert!(first.page.has_more && first.stories.iter().all(|story| story.expired));
    let rest = archive(alice.clone(), "", 3, first.page.next_cursor.clone()).await.unwrap();
    assert_eq!(titles(&rest), vec!["s1"]);
    assert!(!rest.page.has_more);
    assert_eq!(titles(&archive(alice.clone(), "2026-03-15", 10, String::new()).await.unwrap()), vec!["s3", "s2"]);
    assert_eq!(titles(&archive(alice.clone(), "2026-03-14", 10, String::new()).await.unwrap()), vec!["s1"]);
    assert!(archive(alice.clone(), "2026-03-16", 10, String::new()).await.unwrap().stories.is_empty());
    assert!(
        archive(bob.clone(), "", 10, String::new()).await.unwrap().stories.is_empty(),
        "nobody browses someone else's archive"
    );
    assert_eq!(archive(alice.clone(), "03/15/2026", 10, String::new()).await.unwrap_err().reason, "INVALID_DATE");
    assert_eq!(
        ctx.story_reason("ListArchivedStories", None, &ListArchivedStoriesRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );

    // The same stories fall on other days for someone living on UTC.
    sqlx::query("UPDATE users SET time_zone = 'UTC' WHERE id = $1")
        .bind(alice.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(titles(&archive(alice.clone(), "2026-03-14", 10, String::new()).await.unwrap()), vec!["s2", "s1"]);
    let march = calendar(alice.clone(), 2026, 3).await.unwrap();
    assert_eq!(
        march.days.iter().map(|day| (day.date.as_str(), day.story_count)).collect::<Vec<_>>(),
        vec![("2026-03-14", 2), ("2026-03-15", 1), ("2026-03-31", 1)]
    );
}

#[tokio::test]
async fn expiry_is_announced_once_and_deleting_removes_a_story_everywhere() {
    let ctx = start().await;
    let (alice, friend) = (ctx.app.user("alice").await, ctx.app.user("friend").await);
    ctx.befriend(&alice, &friend).await;
    let old = ctx.post_story(&alice, "old").await;
    let live = ctx.story_with(&alice, "live", 2, |_| {}).await;
    ctx.age_story(&old, 25).await;
    let expired_events = |story_id: Uuid| {
        let ctx = &ctx;
        async move {
            ctx.app
                .events("notes.story.expired")
                .await
                .into_iter()
                .filter(|event| event.data["story_id"] == serde_json::json!(story_id))
                .count()
        }
    };

    assert!(ctx.notes.expire_stories().await.unwrap() >= 1);
    ctx.notes.expire_stories().await.unwrap();
    assert_eq!(expired_events(id(&old.id)).await, 1, "each story's expiry is announced exactly once");
    assert_eq!(expired_events(id(&live.id)).await, 0);
    let archived: Option<DateTime<Utc>> = sqlx::query_scalar("SELECT archived_at FROM stories WHERE id = $1")
        .bind(id(&old.id))
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(archived.is_some());

    // Deleting: only the owner can, and the story's uploads are released.
    let delete = |story: &Story| DeleteStoryRequest { story_id: story.id.clone(), ..Default::default() };
    assert_eq!(ctx.story_reason("DeleteStory", Some(&friend), &delete(&live)).await, "STORY_NOT_FOUND");
    assert_eq!(ctx.story_reason("DeleteStory", None, &delete(&live)).await, "SIGN_IN_REQUIRED");
    ctx.watch(&friend, &live).await;
    let mut friend_feed = ctx.app.listen_user(friend.id).await;
    let _: DeleteStoryResponse = ctx.story("DeleteStory", Some(&alice), &delete(&live)).await.unwrap();
    let gone = StoryRef { story_id: live.id.clone(), owner_id: alice.id.to_string(), ..Default::default() };
    assert_eq!(next_push(&mut friend_feed).await, Some(Push::StoryDeleted(Box::new(gone))));
    assert_eq!(ctx.get_story(&alice, &live.id).await.unwrap_err().reason, "STORY_NOT_FOUND");
    assert_eq!(ctx.story_reason("DeleteStory", Some(&alice), &delete(&live)).await, "STORY_NOT_FOUND");
    for item in &live.items {
        assert!(!tokuly_media::assets::is_attached(&ctx.app.core, id(&item.media.asset_id)).await.unwrap());
    }
    let views: i64 = sqlx::query_scalar("SELECT count(*) FROM story_views WHERE story_id = $1")
        .bind(id(&live.id))
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(views, 0);
    let deleted = ctx.app.events_by("notes.story.deleted", alice.id).await;
    assert_eq!(deleted.len(), 1);
    assert_eq!(
        (deleted[0].data["story_id"].clone(), deleted[0].data["reason"].clone()),
        (serde_json::json!(id(&live.id)), serde_json::json!("author"))
    );

    // An archived story is deleted quietly: no friend has it on screen.
    let _: DeleteStoryResponse = ctx.story("DeleteStory", Some(&alice), &delete(&old)).await.unwrap();
    assert!(pushes(&mut friend_feed, QUIET).await.is_empty());
}

#[tokio::test]
async fn story_counters_stay_exact_under_concurrent_taps() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let story = ctx.post_story(&alice, "tap tap").await;
    let mut friends = Vec::new();
    for n in 0..6 {
        let friend = ctx.app.user(&format!("friend{n}")).await;
        ctx.befriend(&alice, &friend).await;
        friends.push(friend);
    }
    // Everyone watches and reacts at once, double-tapping.
    let taps = friends.iter().flat_map(|friend| [(friend, "❤️"), (friend, "🔥")]).map(|(friend, emoji)| async {
        ctx.watch(friend, &story).await;
        react(&ctx, friend, &story, emoji).await;
    });
    futures::future::join_all(taps).await;
    let own = ctx.get_story(&alice, &story.id).await.unwrap();
    assert_eq!((own.view_count, own.reaction_count), (6, 6));
    let stored: (i64, i64) =
        sqlx::query_as("SELECT count(*), count(*) FILTER (WHERE reaction <> '') FROM story_views WHERE story_id = $1")
            .bind(id(&story.id))
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(stored, (6, 6));
}
