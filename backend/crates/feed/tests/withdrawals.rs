//! Entries about content that is gone, or that fewer people may see now,
//! follow that content: deleted notes, stories, videos, shorts, streams and
//! stamp packs leave every list; a note whose audience changed is shown to
//! its new audience; an expired story is the owner's own memory again.
mod common;

use chrono::{Duration, Utc};
use common::*;
use tokuly_core::{events::Event, outbound::ActivityAudience};
use tokuly_feed::IngestOutcome;
use tokuly_proto::{common::SubjectType, feed::*};
use tokuly_testkit::TestUser;
use uuid::Uuid;

/// A public note with a place tag, as the notes domain records it.
fn public_note(user: &TestUser, note_id: Uuid) -> tokuly_core::outbound::ActivityEntry {
    entry(user, "note.posted", ActivityAudience::Public)
        .subject(SubjectType::SUBJECT_TYPE_NOTE, note_id)
        .path(format!("/notes/{note_id}"))
        .at(35.6595, 139.7005)
        .data(serde_json::json!({ "note_id": note_id, "text": "渋谷なう" }))
}

fn visibility_changed(note_id: Uuid, author: Uuid, visibility: i32) -> Event {
    event(
        "notes.note.visibility_changed",
        Some(author),
        serde_json::json!({ "note_id": note_id, "author_id": author, "visibility": visibility }),
    )
}

async fn map_story_items(ctx: &Ctx, viewer: &TestUser) -> usize {
    let response: ListMapStoriesResponse =
        ctx.call(STORIES, "ListMapStories", Some(viewer), &ListMapStoriesRequest::default()).await.unwrap();
    response.reels.iter().map(|reel| reel.items.len()).sum()
}

#[tokio::test]
async fn a_deleted_note_leaves_every_list() {
    let ctx = start().await;
    let (author, friend) = (ctx.app.user("author").await, ctx.app.user("friend").await);
    ctx.befriend(&author, &friend).await;
    let note_id = Uuid::now_v7();
    let id = ctx.record(public_note(&author, note_id), 5).await.to_string();

    assert_eq!(ids(&ctx.profile(None, author.id).await.unwrap()), vec![id.clone()]);
    assert_eq!(ctx.friends_feed(&friend).await.len(), 1);
    assert_eq!(ctx.home_feed(&friend).await.len(), 1);
    assert_eq!(map_story_items(&ctx, &friend).await, 1);

    let deleted = event(
        "notes.note.deleted",
        Some(author.id),
        serde_json::json!({ "note_id": note_id, "author_id": author.id, "reason": "author" }),
    );
    ctx.feed.handle_event(deleted.clone()).await.unwrap();
    ctx.feed.handle_event(deleted).await.unwrap(); // a redelivery changes nothing

    assert!(ctx.profile(None, author.id).await.unwrap().is_empty(), "strangers no longer see its text");
    assert!(ctx.friends_feed(&friend).await.is_empty());
    assert!(ctx.home_feed(&friend).await.is_empty());
    assert_eq!(map_story_items(&ctx, &friend).await, 0, "nor where it was posted");
    assert!(ctx.my_log(&author).await.iter().all(|activity| activity.id != id), "the author's log lets go of it too");
}

#[tokio::test]
async fn a_note_shown_to_fewer_people_is_listed_for_fewer_people() {
    let ctx = start().await;
    let (author, friend, stranger) =
        (ctx.app.user("author").await, ctx.app.user("friend").await, ctx.app.user("stranger").await);
    ctx.befriend(&author, &friend).await;
    let note_id = Uuid::now_v7();
    let id = ctx.record(public_note(&author, note_id), 5).await.to_string();
    // Somebody else's entry about the note keeps its own audience.
    let other = ctx
        .record(
            entry(&friend, "note.liked", ActivityAudience::Public).subject(SubjectType::SUBJECT_TYPE_NOTE, note_id),
            4,
        )
        .await
        .to_string();

    ctx.feed.handle_event(visibility_changed(note_id, author.id, 4)).await.unwrap();
    assert!(ctx.profile(None, author.id).await.unwrap().is_empty(), "friends only: not for visitors");
    assert!(ctx.profile(Some(&stranger), author.id).await.unwrap().is_empty());
    assert_eq!(ids(&ctx.profile(Some(&friend), author.id).await.unwrap()), vec![id.clone()]);
    assert_eq!(ids(&ctx.profile(None, friend.id).await.unwrap()), vec![other]);

    ctx.feed.handle_event(visibility_changed(note_id, author.id, 5)).await.unwrap();
    assert!(ctx.profile(Some(&friend), author.id).await.unwrap().is_empty(), "private: the author's own");
    assert!(ctx.friends_feed(&friend).await.iter().all(|card| head_of(card) != id));
    assert_eq!(map_story_items(&ctx, &friend).await, 0);
    assert_eq!(ids(&ctx.my_log(&author).await)[0], id);

    ctx.feed.handle_event(visibility_changed(note_id, author.id, 1)).await.unwrap();
    assert_eq!(ids(&ctx.profile(None, author.id).await.unwrap()), vec![id], "public again");
}

#[tokio::test]
async fn changes_that_arrive_before_the_entry_or_out_of_order_still_hold() {
    let ctx = start().await;
    let (author, friend) = (ctx.app.user("author").await, ctx.app.user("friend").await);
    ctx.befriend(&author, &friend).await;

    // Deleted before the entry was stored: it never appears.
    let gone = Uuid::now_v7();
    ctx.feed
        .handle_event(event(
            "notes.note.deleted",
            Some(author.id),
            serde_json::json!({ "note_id": gone, "author_id": author.id, "reason": "author" }),
        ))
        .await
        .unwrap();
    let outcome = ctx.deliver(&public_note(&author, gone), Utc::now() - Duration::minutes(1)).await;
    assert_eq!(outcome, IngestOutcome::Dropped);

    // Made private before the entry was stored: stored for the author only.
    let narrowed = Uuid::now_v7();
    ctx.feed.handle_event(visibility_changed(narrowed, author.id, 5)).await.unwrap();
    let id = ctx.record(public_note(&author, narrowed), 1).await.to_string();
    assert!(ctx.profile(Some(&friend), author.id).await.unwrap().is_empty());
    assert_eq!(ids(&ctx.my_log(&author).await), vec![id.clone()]);

    // An older change delivered after a newer one does not win.
    let mut older = visibility_changed(narrowed, author.id, 1);
    older.occurred_at = Utc::now() - Duration::minutes(10);
    ctx.feed.handle_event(older).await.unwrap();
    assert!(ctx.profile(None, author.id).await.unwrap().is_empty(), "still private");
}

#[tokio::test]
async fn an_expired_story_is_the_owners_memory_again() {
    let ctx = start().await;
    let (owner, friend) = (ctx.app.user("owner").await, ctx.app.user("friend").await);
    ctx.befriend(&owner, &friend).await;
    let story_id = Uuid::now_v7();
    let posted = entry(&owner, "story.posted", ActivityAudience::Friends)
        .subject(SubjectType::SUBJECT_TYPE_STORY, story_id)
        .path(format!("/stories/{story_id}"))
        .image("https://cdn.example/cover.webp")
        .data(serde_json::json!({ "story_id": story_id, "title": "夏の思い出" }));
    let id = ctx.record(posted, 5).await.to_string();
    assert_eq!(ctx.home_feed(&friend).await.len(), 1);

    ctx.feed
        .handle_event(event(
            "notes.story.expired",
            None,
            serde_json::json!({ "story_id": story_id, "owner_id": owner.id }),
        ))
        .await
        .unwrap();
    assert!(ctx.home_feed(&friend).await.is_empty(), "its title and cover are no longer shown to friends");
    assert!(ctx.profile(Some(&friend), owner.id).await.unwrap().is_empty());
    assert_eq!(ids(&ctx.my_log(&owner).await), vec![id]);
}

#[tokio::test]
async fn other_kinds_of_deleted_content_leave_the_feed_too() {
    let ctx = start().await;
    let (owner, friend) = (ctx.app.user("owner").await, ctx.app.user("friend").await);
    ctx.befriend(&owner, &friend).await;
    let cases = [
        ("story.posted", SubjectType::SUBJECT_TYPE_STORY, "notes.story.deleted", "story_id"),
        ("video.published", SubjectType::SUBJECT_TYPE_VIDEO, "video.video.deleted", "video_id"),
        ("short.published", SubjectType::SUBJECT_TYPE_SHORT, "shorts.short.deleted", "id"),
        ("live.started", SubjectType::SUBJECT_TYPE_LIVE_STREAM, "live.stream.deleted", "stream_id"),
        ("stamp.pack.published", SubjectType::SUBJECT_TYPE_STAMP_PACK, "stamp.pack.deleted", "pack_id"),
    ];
    for (minutes_ago, (kind, subject_type, topic, field)) in cases.into_iter().enumerate() {
        let subject_id = Uuid::now_v7();
        let id = ctx
            .record(
                entry(&owner, kind, ActivityAudience::Friends).subject(subject_type, subject_id),
                100 - minutes_ago as i64 * 10,
            )
            .await
            .to_string();
        assert!(ids(&ctx.profile(Some(&friend), owner.id).await.unwrap()).contains(&id), "{kind}");
        let mut data = serde_json::json!({ "owner_id": owner.id });
        data[field] = serde_json::json!(subject_id);
        ctx.feed.handle_event(event(topic, Some(owner.id), data)).await.unwrap();
        assert!(!ids(&ctx.profile(Some(&friend), owner.id).await.unwrap()).contains(&id), "{topic} withdraws {kind}");
        assert!(!ids(&ctx.my_log(&owner).await).contains(&id));
    }
}
