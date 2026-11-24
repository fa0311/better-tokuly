//! Entries about content that is gone, or that fewer people may see now,
//! follow that content: deleted notes, stories, videos, shorts, streams and
//! stamp packs leave every list; a note whose audience changed is shown to
//! its new audience; an expired story is the owner's own memory again; a
//! video that went back to draft or private is the owner's own until it is
//! published to an audience again; a deleted answer's "answered" entries go.
//!
//! Every payload is built from the producing domain's own event struct, so
//! a renamed field fails here instead of silently withdrawing nothing.
mod common;

use chrono::{DateTime, Duration, Utc};
use common::*;
use serde::Serialize;
use tokuly_core::{
    events::Event,
    outbound::{ActivityAudience, ActivityEntry},
};
use tokuly_feed::IngestOutcome;
use tokuly_notes::events::{NoteDeleted, NoteVisibilityChanged, RemovalReason, StoryDeleted, StoryExpired};
use tokuly_proto::{common::SubjectType, feed::*};
use tokuly_testkit::TestUser;
use uuid::Uuid;

/// The event the relay would deliver for a producer's payload.
fn produced(topic: &str, actor: Option<Uuid>, payload: &impl Serialize) -> Event {
    event(topic, actor, serde_json::to_value(payload).expect("payload serializes"))
}

/// A public note with a place tag, as the notes domain records it.
fn public_note(user: &TestUser, note_id: Uuid) -> ActivityEntry {
    entry(user, "note.posted", ActivityAudience::Public)
        .subject(SubjectType::SUBJECT_TYPE_NOTE, note_id)
        .path(format!("/notes/{note_id}"))
        .at(35.6595, 139.7005)
        .data(serde_json::json!({ "note_id": note_id, "text": "渋谷なう" }))
}

fn note_deleted(note_id: Uuid, author: Uuid) -> Event {
    produced(
        "notes.note.deleted",
        Some(author),
        &NoteDeleted { note_id, author_id: author, reason: RemovalReason::Author },
    )
}

fn visibility_changed_at(note_id: Uuid, author: Uuid, visibility: i32, changed_at: DateTime<Utc>) -> Event {
    produced(
        "notes.note.visibility_changed",
        Some(author),
        &NoteVisibilityChanged { note_id, author_id: author, visibility, visible_to: None, changed_at },
    )
}

fn visibility_changed(note_id: Uuid, author: Uuid, visibility: i32) -> Event {
    visibility_changed_at(note_id, author, visibility, Utc::now())
}

fn video_event(topic: &str, video_id: Uuid, owner: Uuid, visibility: i32) -> Event {
    produced(
        topic,
        Some(owner),
        &tokuly_video::events::VideoEvent {
            video_id,
            short_id: "abcdefghijk".into(),
            channel_id: Uuid::now_v7(),
            owner_id: owner,
            visibility,
            origin: 1,
            origin_id: None,
            first_publication: false,
        },
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

    let deleted = note_deleted(note_id, author.id);
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
    ctx.feed.handle_event(note_deleted(gone, author.id)).await.unwrap();
    let outcome = ctx.deliver(&public_note(&author, gone), Utc::now() - Duration::minutes(1)).await;
    assert_eq!(outcome, IngestOutcome::Dropped);

    // Made private before the entry was stored: stored for the author only.
    let narrowed = Uuid::now_v7();
    ctx.feed.handle_event(visibility_changed(narrowed, author.id, 5)).await.unwrap();
    let id = ctx.record(public_note(&author, narrowed), 1).await.to_string();
    assert!(ctx.profile(Some(&friend), author.id).await.unwrap().is_empty());
    assert_eq!(ids(&ctx.my_log(&author).await), vec![id.clone()]);

    // An older change delivered after a newer one does not win, even when
    // its event is the younger one: the note's own `changed_at` decides.
    let older = visibility_changed_at(narrowed, author.id, 1, Utc::now() - Duration::minutes(10));
    assert!(older.occurred_at > Utc::now() - Duration::minutes(1));
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
        .handle_event(produced("notes.story.expired", None, &StoryExpired { story_id, owner_id: owner.id }))
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
    let owner_id = owner.id;
    type Payload = fn(Uuid, Uuid) -> serde_json::Value;
    let cases: [(&str, SubjectType, &str, Payload); 5] = [
        ("story.posted", SubjectType::SUBJECT_TYPE_STORY, "notes.story.deleted", |id, owner| {
            serde_json::to_value(StoryDeleted { story_id: id, owner_id: owner, reason: RemovalReason::Author }).unwrap()
        }),
        ("video.published", SubjectType::SUBJECT_TYPE_VIDEO, "video.video.deleted", |id, owner| {
            video_event("video.video.deleted", id, owner, 1).data
        }),
        ("short.published", SubjectType::SUBJECT_TYPE_SHORT, "shorts.short.deleted", |id, owner| {
            serde_json::to_value(tokuly_shorts::events::ShortEvent {
                id,
                short_id: "abcdefghijk".into(),
                channel_id: Uuid::now_v7(),
                owner_id: owner,
                visibility: 1,
                first_publication: false,
                reason: "owner".into(),
            })
            .unwrap()
        }),
        ("live.started", SubjectType::SUBJECT_TYPE_LIVE_STREAM, "live.stream.deleted", |id, owner| {
            serde_json::to_value(tokuly_live::events::StreamEvent {
                stream_id: id,
                short_id: "abcdefghijk".into(),
                channel_id: Uuid::now_v7(),
                owner_id: owner,
                visibility: 1,
                end_reason: 0,
                went_live: true,
            })
            .unwrap()
        }),
        ("stamp.pack.published", SubjectType::SUBJECT_TYPE_STAMP_PACK, "stamp.pack.deleted", |id, owner| {
            serde_json::to_value(tokuly_stamp::events::PackEvent {
                pack_id: id,
                creator_id: Some(owner),
                official: false,
            })
            .unwrap()
        }),
    ];
    for (minutes_ago, (kind, subject_type, topic, payload)) in cases.into_iter().enumerate() {
        let subject_id = Uuid::now_v7();
        let id = ctx
            .record(
                entry(&owner, kind, ActivityAudience::Friends).subject(subject_type, subject_id),
                100 - minutes_ago as i64 * 10,
            )
            .await
            .to_string();
        assert!(ids(&ctx.profile(Some(&friend), owner_id).await.unwrap()).contains(&id), "{kind}");
        ctx.feed.handle_event(event(topic, Some(owner_id), payload(subject_id, owner_id))).await.unwrap();
        assert!(!ids(&ctx.profile(Some(&friend), owner_id).await.unwrap()).contains(&id), "{topic} withdraws {kind}");
        assert!(!ids(&ctx.my_log(&owner).await).contains(&id));
    }
}

#[tokio::test]
async fn a_video_back_in_draft_or_private_is_the_owners_until_published_again() {
    let ctx = start().await;
    let (owner, friend) = (ctx.app.user("owner").await, ctx.app.user("friend").await);
    ctx.befriend(&owner, &friend).await;
    let video_id = Uuid::now_v7();
    let id = ctx
        .record(
            entry(&owner, "video.published", ActivityAudience::Friends)
                .subject(SubjectType::SUBJECT_TYPE_VIDEO, video_id),
            30,
        )
        .await
        .to_string();

    ctx.feed.handle_event(video_event("video.video.unpublished", video_id, owner.id, 1)).await.unwrap();
    assert!(!seen(&ctx, &friend, owner.id, &id).await, "back to draft");
    assert_eq!(ids(&ctx.my_log(&owner).await), vec![id.clone()], "the owner keeps it");

    ctx.feed.handle_event(video_event("video.video.published", video_id, owner.id, 1)).await.unwrap();
    assert!(seen(&ctx, &friend, owner.id, &id).await, "published again");

    ctx.feed.handle_event(video_event("video.video.visibility_changed", video_id, owner.id, 5)).await.unwrap();
    assert!(!seen(&ctx, &friend, owner.id, &id).await, "made private");
    ctx.feed.handle_event(video_event("video.video.visibility_changed", video_id, owner.id, 4)).await.unwrap();
    assert!(seen(&ctx, &friend, owner.id, &id).await, "for friends again");
}

#[tokio::test]
async fn a_deleted_answer_withdraws_its_entries_but_the_question_can_be_answered_again() {
    let ctx = start().await;
    let (owner, friend) = (ctx.app.user("owner").await, ctx.app.user("friend").await);
    ctx.befriend(&owner, &friend).await;
    let question_id = Uuid::now_v7();
    let answered = || {
        entry(&owner, "ask.answered", ActivityAudience::Friends)
            .subject(SubjectType::SUBJECT_TYPE_QUESTION, question_id)
    };
    let first = ctx.record(answered(), 20).await.to_string();
    let deleted = produced(
        "ask.answer.deleted",
        Some(owner.id),
        &tokuly_ask::events::AnswerDeleted { question_id, owner_id: owner.id },
    );
    ctx.feed.handle_event(deleted).await.unwrap();
    assert!(!ids(&ctx.my_log(&owner).await).contains(&first));

    // Answered again under the same question: shown as usual.
    let again = ctx.record(answered(), 1).await.to_string();
    assert!(ids(&ctx.profile(Some(&friend), owner.id).await.unwrap()).contains(&again));
}

/// Whether `viewer` finds entry `id` on `owner`'s profile.
async fn seen(ctx: &Ctx, viewer: &TestUser, owner: Uuid, id: &str) -> bool {
    ids(&ctx.profile(Some(viewer), owner).await.unwrap()).iter().any(|shown| shown == id)
}
