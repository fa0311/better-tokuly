//! How the notes domain fits into the rest of the backend: subject
//! resolvers, the events it consumes, the personal data export, profile
//! counters, link previews and the Rust API for the demo simulator.
mod common;

use common::*;
use tokuly_core::events::Event;
use tokuly_notes::NotesConfig;
use tokuly_proto::{
    common::{SubjectType, Visibility},
    identity::{GetProfileRequest, GetProfileResponse, get_profile_request::By},
    notes::{notes_event::Event as Push, *},
};
use tokuly_testkit::TestUser;
use uuid::Uuid;

/// An event as the relay would deliver it.
fn event(topic: &str, data: serde_json::Value) -> Event {
    Event { id: Uuid::now_v7(), topic: topic.to_owned(), actor_id: None, occurred_at: chrono::Utc::now(), data }
}

async fn count(ctx: &Ctx, sql: &str, about: Uuid) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned())).bind(about).fetch_one(&ctx.app.core.db).await.unwrap()
}

#[tokio::test]
async fn notes_and_stories_resolve_as_subjects_for_those_who_may_see_them() {
    let ctx = start().await;
    let (alice, friend, stranger) =
        (ctx.member("alice").await, ctx.member("friend").await, ctx.member("stranger").await);
    ctx.befriend(&alice, &friend).await;
    let subjects = &ctx.app.core.subjects;
    assert!(
        subjects.is_registered(SubjectType::SUBJECT_TYPE_NOTE)
            && subjects.is_registered(SubjectType::SUBJECT_TYPE_STORY)
    );
    let note_type = SubjectType::SUBJECT_TYPE_NOTE;

    let image = ctx.note_image(&alice).await;
    let public = ctx.post_with(&alice, "見て見て!\nこれすごい", |r| r.asset_ids = vec![image]).await;
    let info = subjects.resolve(note_type, &public.id, None).await.unwrap();
    assert_eq!((info.id, info.owner_id), (id(&public.id), alice.id));
    assert_eq!(info.title, "見て見て! これすごい");
    assert_eq!(info.path, format!("/notes/{}", public.id));
    assert!(info.image_url.is_some() && info.interactions_enabled);

    let for_friends = ctx.post_visible_to(&alice, "friends only", Visibility::VISIBILITY_FRIENDS).await;
    assert!(subjects.resolve(note_type, &for_friends.id, Some(friend.id)).await.is_ok());
    assert!(subjects.resolve(note_type, &for_friends.id, Some(alice.id)).await.is_ok());
    for viewer in [Some(stranger.id), None] {
        assert_eq!(subjects.resolve(note_type, &for_friends.id, viewer).await.unwrap_err().reason, "SUBJECT_NOT_FOUND");
    }
    assert_eq!(subjects.resolve(note_type, "not-an-id", None).await.unwrap_err().reason, "SUBJECT_NOT_FOUND");
    assert_eq!(
        subjects.resolve(note_type, &Uuid::now_v7().to_string(), None).await.unwrap_err().reason,
        "SUBJECT_NOT_FOUND"
    );
    // A sensitive note offers no picture; a note without text still has a title.
    let image = ctx.note_image(&alice).await;
    let sensitive = ctx
        .post_with(&alice, "", |r| {
            r.asset_ids = vec![image];
            r.content_warning = true;
        })
        .await;
    let info = subjects.resolve(note_type, &sensitive.id, None).await.unwrap();
    assert!(info.image_url.is_none() && !info.title.is_empty());

    let story = ctx.post_story(&alice, "海").await;
    let story_type = SubjectType::SUBJECT_TYPE_STORY;
    let info = subjects.resolve(story_type, &story.id, Some(friend.id)).await.unwrap();
    assert_eq!(
        (info.owner_id, info.title.as_str(), info.path.clone()),
        (alice.id, "海", format!("/stories/{}", story.id))
    );
    assert!(
        info.image_url.is_some() && !info.interactions_enabled,
        "stories have their own reactions; replies are DMs"
    );
    for viewer in [Some(stranger.id), None] {
        assert_eq!(subjects.resolve(story_type, &story.id, viewer).await.unwrap_err().reason, "SUBJECT_NOT_FOUND");
    }
    assert_eq!(
        subjects.resolve(story_type, "not-an-id", Some(friend.id)).await.unwrap_err().reason,
        "SUBJECT_NOT_FOUND"
    );
}

#[tokio::test]
async fn what_reached_someone_stays_reportable_after_it_was_hidden_from_them() {
    let ctx = start().await;
    let (mallory, victim, bystander, friend) = (
        ctx.member("mallory").await,
        ctx.member("victim").await,
        ctx.member("bystander").await,
        ctx.member("friend").await,
    );
    let subjects = &ctx.app.core.subjects;
    let note_type = SubjectType::SUBJECT_TYPE_NOTE;
    let reportable = |kind: SubjectType, id: String, viewer: Uuid| async move {
        subjects.resolve_for_report(kind, &id, Some(viewer)).await.map(|info| info.owner_id)
    };

    // A mention, then the note is made private: the person named in it
    // received it and can still report it; nobody else can.
    let abuse = ctx.post(&mallory, &format!("@{} ひどいこと", victim.handle)).await;
    let _: SetNoteVisibilityResponse = ctx
        .note(
            "SetNoteVisibility",
            Some(&mallory),
            &SetNoteVisibilityRequest {
                note_id: abuse.id.clone(),
                visibility: Visibility::VISIBILITY_PRIVATE.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(subjects.resolve(note_type, &abuse.id, Some(victim.id)).await.is_err(), "hidden from the victim");
    assert_eq!(reportable(note_type, abuse.id.clone(), victim.id).await.unwrap(), mallory.id);
    assert_eq!(
        reportable(note_type, abuse.id.clone(), bystander.id).await.unwrap_err().reason,
        "SUBJECT_NOT_FOUND",
        "a private note never reached the bystander"
    );

    // A public note stays reportable after the reader blocked its author.
    let public = ctx.post(&mallory, "public words").await;
    ctx.block(&victim, &mallory).await;
    assert!(subjects.resolve(note_type, &public.id, Some(victim.id)).await.is_err());
    assert_eq!(reportable(note_type, public.id.clone(), victim.id).await.unwrap(), mallory.id);

    // Not once the author's account is suspended: their notes are shown to nobody.
    sqlx::query("UPDATE users SET banned_at = now() WHERE id = $1")
        .bind(mallory.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(
        reportable(note_type, public.id.clone(), victim.id).await.unwrap_err().reason,
        "SUBJECT_NOT_FOUND",
        "a suspended account's public note"
    );
    sqlx::query("UPDATE users SET banned_at = NULL WHERE id = $1")
        .bind(mallory.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();

    // A story that was watched stays reportable after its day is over.
    ctx.befriend(&mallory, &friend).await;
    ctx.befriend(&mallory, &bystander).await;
    let story = ctx.post_story(&mallory, "ひどい記憶").await;
    ctx.watch(&friend, &story).await;
    ctx.age_story(&story, 48).await;
    let story_type = SubjectType::SUBJECT_TYPE_STORY;
    assert!(subjects.resolve(story_type, &story.id, Some(friend.id)).await.is_err(), "expired");
    assert_eq!(reportable(story_type, story.id.clone(), friend.id).await.unwrap(), mallory.id);
    assert_eq!(
        reportable(story_type, story.id.clone(), bystander.id).await.unwrap_err().reason,
        "SUBJECT_NOT_FOUND",
        "a friend who never watched it did not receive it"
    );
}

#[tokio::test]
async fn an_erased_account_leaves_nothing_behind() {
    let ctx = start().await;
    let (alice, bob) = (ctx.member("alice").await, ctx.member("bob").await);
    ctx.befriend(&alice, &bob).await;
    let tag = unique("erase");

    // What alice made…
    let image = ctx.note_image(&alice).await;
    let mine = ctx.post_with(&alice, &format!("alice's note #{tag}"), |r| r.asset_ids = vec![image.clone()]).await;
    let _: PinNoteResponse = ctx
        .note("PinNote", Some(&alice), &PinNoteRequest { note_id: mine.id.clone(), ..Default::default() })
        .await
        .unwrap();
    let bobs_reply = ctx.reply(&bob, &mine, "bob answers alice").await;
    ctx.like(&bob, &mine).await;
    let my_story = ctx.post_story(&alice, "alice's story").await;
    ctx.watch(&bob, &my_story).await;
    let _: CreateHighlightResponse = ctx
        .story(
            "CreateHighlight",
            Some(&alice),
            &CreateHighlightRequest { title: "h".into(), story_ids: vec![my_story.id.clone()], ..Default::default() },
        )
        .await
        .unwrap();
    // …and what she did to bob's things.
    let bobs = ctx.post(&bob, &format!("bob's note for @{}", alice.handle)).await;
    ctx.like(&alice, &bobs).await;
    ctx.renote(&alice, &bobs).await;
    ctx.reply(&alice, &bobs, "alice answers bob").await;
    ctx.quote(&alice, &bobs, "alice quotes bob").await;
    let bobs_story = ctx
        .story_with(&bob, "for alice", 1, |r| {
            r.audience = StoryAudience::STORY_AUDIENCE_CLOSE_FRIENDS.into();
            r.audience_user_ids = vec![alice.id.to_string()];
        })
        .await;
    let reaction = ReactToStoryRequest { story_id: bobs_story.id.clone(), emoji: "🎉".into(), ..Default::default() };
    let _: ReactToStoryResponse = ctx.story("ReactToStory", Some(&alice), &reaction).await.unwrap();

    let deleted = event("identity.user.deleted", serde_json::json!({ "user_id": alice.id }));
    ctx.notes.handle_event(deleted.clone()).await.unwrap();
    ctx.notes.handle_event(deleted).await.unwrap(); // delivered twice: nothing more to do

    for (what, sql) in [
        ("notes", "SELECT count(*) FROM notes WHERE author_id = $1"),
        ("likes", "SELECT count(*) FROM note_likes WHERE user_id = $1"),
        ("renotes", "SELECT count(*) FROM note_renotes WHERE user_id = $1"),
        ("mentions", "SELECT count(*) FROM note_mentions WHERE user_id = $1"),
        ("deliveries", "SELECT count(*) FROM note_recipients WHERE user_id = $1"),
        ("stories", "SELECT count(*) FROM stories WHERE owner_id = $1"),
        ("story views", "SELECT count(*) FROM story_views WHERE viewer_id = $1"),
        ("audiences", "SELECT count(*) FROM story_audience WHERE user_id = $1"),
        ("highlights", "SELECT count(*) FROM story_highlights WHERE owner_id = $1"),
        ("rules", "SELECT count(*) FROM note_rule_agreements WHERE user_id = $1"),
    ] {
        assert_eq!(count(&ctx, sql, alice.id).await, 0, "{what}");
    }
    assert!(!tokuly_media::assets::is_attached(&ctx.app.core, id(&image)).await.unwrap());
    assert!(!ctx.is_indexed("note", id(&mine.id)).await);
    let tag_count: i64 = sqlx::query_scalar("SELECT note_count FROM hashtags WHERE tag = $1")
        .bind(&tag)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(tag_count, 0);
    let reasons: Vec<String> = ctx
        .app
        .events_by("notes.note.deleted", alice.id)
        .await
        .iter()
        .map(|event| event.data["reason"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(reasons, vec!["account_deleted"; 3]);

    // Bob's note is back to what bob and others made of it.
    let bobs = ctx.get_note(Some(&bob), &bobs.id).await.unwrap();
    assert_eq!(
        (bobs.counters.like_count, bobs.counters.renote_count, bobs.counters.reply_count, bobs.counters.quote_count),
        (0, 0, 0, 0)
    );
    assert!(bobs.mentions.is_empty(), "the mention is plain text again");
    let bobs_story = ctx.get_story(&bob, &bobs_story.id).await.unwrap();
    assert_eq!((bobs_story.view_count, bobs_story.reaction_count), (0, 0));
    // Bob's reply to the erased note stays, pointing at something unavailable.
    assert!(ctx.get_note(Some(&bob), &bobs_reply.id).await.unwrap().reply_to.unavailable);
}

#[tokio::test]
async fn moderation_takedowns_delete_notes_and_stories() {
    let ctx = start().await;
    let (alice, friend) = (ctx.member("alice").await, ctx.member("friend").await);
    ctx.befriend(&alice, &friend).await;
    let moderator = Uuid::now_v7();
    let removal = |subject_type: SubjectType, subject_id: &str| {
        event(
            "admin.content.removed",
            serde_json::json!({ "subject_type": subject_type as i32, "subject_id": subject_id, "reason": "spam", "moderator_id": moderator }),
        )
    };

    let note = ctx.post(&alice, "buy my stuff").await;
    let reply = ctx.reply(&friend, &note, "no thanks").await;
    let mut watching = ctx.app.listen_topic(&format!("subject:3:{}", note.id)).await;
    // Notifications to the two people of this test (the outbox is shared with other tests).
    let notifications = || {
        let ctx = &ctx;
        let people = [serde_json::json!(alice.id), serde_json::json!(friend.id)];
        async move {
            ctx.app
                .events("notify.requested")
                .await
                .into_iter()
                .filter(|event| people.contains(&event.data["user_id"]))
                .count()
        }
    };
    let notifications_before = notifications().await;
    ctx.notes.handle_event(removal(SubjectType::SUBJECT_TYPE_NOTE, &note.id)).await.unwrap();
    assert_eq!(ctx.get_note(Some(&alice), &note.id).await.unwrap_err().reason, "NOTE_NOT_FOUND");
    assert_eq!(next_push(&mut watching).await, Some(Push::NoteDeletedId(note.id.clone())));
    assert!(ctx.get_note(Some(&friend), &reply.id).await.unwrap().reply_to.unavailable);
    let deleted: Vec<Event> = ctx
        .app
        .events("notes.note.deleted")
        .await
        .into_iter()
        .filter(|event| event.data["note_id"] == serde_json::json!(id(&note.id)))
        .collect();
    assert_eq!(deleted.len(), 1);
    assert_eq!(
        (deleted[0].data["reason"].clone(), deleted[0].actor_id),
        (serde_json::json!("moderation"), Some(moderator))
    );
    assert_eq!(notifications().await, notifications_before, "the admin domain tells the author; this one does not");

    let story = ctx.post_story(&alice, "also spam").await;
    ctx.notes.handle_event(removal(SubjectType::SUBJECT_TYPE_STORY, &story.id)).await.unwrap();
    assert_eq!(ctx.get_story(&alice, &story.id).await.unwrap_err().reason, "STORY_NOT_FOUND");
    let deleted: Vec<Event> = ctx
        .app
        .events("notes.story.deleted")
        .await
        .into_iter()
        .filter(|event| event.data["story_id"] == serde_json::json!(id(&story.id)))
        .collect();
    assert_eq!(deleted[0].data["reason"], "moderation");

    // Already removed, unknown, or somebody else's kind of content: done.
    ctx.notes.handle_event(removal(SubjectType::SUBJECT_TYPE_NOTE, &note.id)).await.unwrap();
    ctx.notes.handle_event(removal(SubjectType::SUBJECT_TYPE_STORY, &Uuid::now_v7().to_string())).await.unwrap();
    let kept = ctx.post(&alice, "fine").await;
    ctx.notes.handle_event(removal(SubjectType::SUBJECT_TYPE_VIDEO, &kept.id)).await.unwrap();
    assert!(ctx.get_note(None, &kept.id).await.is_ok());
}

#[tokio::test]
async fn a_forcibly_deleted_upload_is_taken_out_of_what_shows_it() {
    let ctx = start().await;
    let alice = ctx.member("alice").await;
    let (first, second, only, with_text) = (
        ctx.note_image(&alice).await,
        ctx.note_image(&alice).await,
        ctx.note_image(&alice).await,
        ctx.note_image(&alice).await,
    );
    let gallery = ctx.post_with(&alice, "", |r| r.asset_ids = vec![first.clone(), second.clone()]).await;
    let picture = ctx.post_with(&alice, "", |r| r.asset_ids = vec![only.clone()]).await;
    let captioned = ctx.post_with(&alice, "with words", |r| r.asset_ids = vec![with_text.clone()]).await;
    let asset_deleted = |asset_id: &str| {
        event(
            "media.asset.deleted",
            serde_json::json!({ "asset_id": asset_id, "owner_id": alice.id, "purpose": 8, "kind": 1 }),
        )
    };

    ctx.notes.handle_event(asset_deleted(&first)).await.unwrap();
    let gallery = ctx.get_note(None, &gallery.id).await.unwrap();
    assert_eq!(gallery.attachments.iter().map(|a| a.asset_id.clone()).collect::<Vec<_>>(), vec![second.clone()]);
    // A note that was nothing but the picture goes with it; one with words keeps them.
    ctx.notes.handle_event(asset_deleted(&only)).await.unwrap();
    assert_eq!(ctx.get_note(Some(&alice), &picture.id).await.unwrap_err().reason, "NOTE_NOT_FOUND");
    let reasons: Vec<Event> = ctx
        .app
        .events("notes.note.deleted")
        .await
        .into_iter()
        .filter(|event| event.data["note_id"] == serde_json::json!(id(&picture.id)))
        .collect();
    assert_eq!(reasons[0].data["reason"], "media_deleted");
    ctx.notes.handle_event(asset_deleted(&with_text)).await.unwrap();
    let captioned = ctx.get_note(None, &captioned.id).await.unwrap();
    assert!(captioned.attachments.is_empty() && captioned.text == "with words");
    assert_eq!(ctx.search_changes("note", id(&captioned.id)).await.len(), 2, "the search document is refreshed");

    let story = ctx.story_with(&alice, "two photos", 2, |_| {}).await;
    let single = ctx.post_story(&alice, "one photo").await;
    ctx.notes.handle_event(asset_deleted(&story.items[0].media.asset_id)).await.unwrap();
    let story = ctx.get_story(&alice, &story.id).await.unwrap();
    assert_eq!(story.items.iter().map(|item| item.caption.as_str()).collect::<Vec<_>>(), vec!["photo 2"]);
    ctx.notes.handle_event(asset_deleted(&single.items[0].media.asset_id)).await.unwrap();
    assert_eq!(ctx.get_story(&alice, &single.id).await.unwrap_err().reason, "STORY_NOT_FOUND");
    // An upload nothing here shows is none of this domain's business.
    ctx.notes.handle_event(asset_deleted(&Uuid::now_v7().to_string())).await.unwrap();
}

#[tokio::test]
async fn a_block_ends_standing_grants_between_the_two() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    let story = ctx
        .story_with(&alice, "for bob", 1, |r| {
            r.audience = StoryAudience::STORY_AUDIENCE_CLOSE_FRIENDS.into();
            r.audience_user_ids = vec![bob.id.to_string()];
        })
        .await;
    ctx.block(&bob, &alice).await;
    // The block alone already hides the story (every read asks who is blocked)…
    assert_eq!(ctx.get_story(&bob, &story.id).await.unwrap_err().reason, "STORY_NOT_FOUND");
    // …and the event removes bob from the chosen, whichever of the two blocked.
    assert_eq!(count(&ctx, "SELECT count(*) FROM story_audience WHERE story_id = $1", id(&story.id)).await, 1);
    ctx.notes
        .handle_event(event("social.user.blocked", serde_json::json!({ "user_id": bob.id, "blocked_id": alice.id })))
        .await
        .unwrap();
    assert_eq!(count(&ctx, "SELECT count(*) FROM story_audience WHERE story_id = $1", id(&story.id)).await, 0);
    assert!(ctx.get_story(&alice, &story.id).await.unwrap().audience_user_ids.is_empty());
    // Events of no interest are acknowledged.
    ctx.notes
        .handle_event(event("social.user.unblocked", serde_json::json!({ "user_id": bob.id, "blocked_id": alice.id })))
        .await
        .unwrap();
}

#[tokio::test]
async fn the_data_export_has_a_notes_section() {
    let ctx = start().await;
    let (alice, bob) = (ctx.member("alice").await, ctx.member("bob").await);
    ctx.befriend(&alice, &bob).await;
    let note = ctx.post(&alice, "exported #tag").await;
    let bobs = ctx.post(&bob, "bob's").await;
    let mention = ctx.post(&bob, &format!("hi @{}", alice.handle)).await;
    ctx.like(&alice, &bobs).await;
    ctx.renote(&alice, &bobs).await;
    let story = ctx.post_story(&alice, "exported story").await;
    let bobs_story = ctx.post_story(&bob, "bob's story").await;
    ctx.watch(&alice, &bobs_story).await;
    let _: CreateHighlightResponse = ctx
        .story(
            "CreateHighlight",
            Some(&alice),
            &CreateHighlightRequest { title: "best".into(), story_ids: vec![story.id.clone()], ..Default::default() },
        )
        .await
        .unwrap();

    let exporter = ctx
        .app
        .core
        .exporters
        .all()
        .into_iter()
        .find(|exporter| exporter.name() == "notes")
        .expect("the notes exporter is registered");
    let export = exporter.export(alice.id).await.unwrap();
    assert_eq!(export["rules_agreement"]["version"], tokuly_notes::rules::RULES_VERSION);
    assert_eq!(export["notes"].as_array().unwrap().len(), 1);
    assert_eq!(
        (export["notes"][0]["id"].clone(), export["notes"][0]["text"].clone()),
        (serde_json::json!(note.id), serde_json::json!("exported #tag"))
    );
    assert_eq!(export["notes"][0]["hashtags"], serde_json::json!(["tag"]));
    assert_eq!(export["likes"][0]["note_id"], serde_json::json!(bobs.id));
    assert_eq!(export["renotes"][0]["note_id"], serde_json::json!(bobs.id));
    assert_eq!(
        (export["stories"][0]["id"].clone(), export["stories"][0]["title"].clone()),
        (serde_json::json!(story.id), serde_json::json!("exported story"))
    );
    assert_eq!(export["story_views"][0]["story_id"], serde_json::json!(bobs_story.id));
    assert_eq!(export["received_notes"], serde_json::json!([{ "note_id": mention.id }]), "notes announced to her");
    assert_eq!(export["highlights"][0]["story_ids"], serde_json::json!([story.id]));
    // Someone who never used notes exports empty lists.
    let nobody = exporter.export(Uuid::now_v7()).await.unwrap();
    assert!(nobody["rules_agreement"].is_null() && nobody["notes"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn profiles_count_the_notes_their_viewer_may_see() {
    let ctx = start().await;
    let (alice, friend) = (ctx.member("alice").await, ctx.member("friend").await);
    ctx.befriend(&alice, &friend).await;
    let first = ctx.post(&alice, "one").await;
    ctx.post(&alice, "two").await;
    ctx.post_visible_to(&alice, "three", Visibility::VISIBILITY_FRIENDS).await;
    ctx.post_visible_to(&alice, "four", Visibility::VISIBILITY_PRIVATE).await;
    ctx.reply(&alice, &first, "a reply is not counted").await;

    let note_count = |viewer: Option<TestUser>| {
        let request = GetProfileRequest { by: Some(By::UserId(alice.id.to_string())), ..Default::default() };
        let ctx = &ctx;
        async move {
            let response: GetProfileResponse =
                ctx.app.call("tokuly.identity.v1.ProfileService/GetProfile", viewer.as_ref(), &request).await.unwrap();
            response.profile.stats.note_count
        }
    };
    assert_eq!(note_count(None).await, 2);
    assert_eq!(note_count(Some(friend.clone())).await, 3);
    assert_eq!(note_count(Some(alice.clone())).await, 4);
}

#[tokio::test]
async fn link_previews_are_resolved_in_the_background_and_pushed() {
    let ctx = start().await;
    let alice = ctx.member("alice").await;
    let other = ctx.post(&alice, "the linked note").await;
    // A link to this instance previews without any request leaving the server.
    let link = format!("http://localhost:8080/notes/{}", other.id);

    let note = ctx.post(&alice, &format!("見て {link} すごい")).await;
    assert!(note.link_preview.as_option().is_none(), "the preview arrives later");
    let mut author_feed = ctx.app.listen_user(alice.id).await;
    let mut watching = ctx.app.listen_topic(&format!("subject:3:{}", note.id)).await;
    ctx.notes.resolve_link_preview(id(&note.id)).await.unwrap();
    let preview =
        ctx.get_note(None, &note.id).await.unwrap().link_preview.as_option().cloned().expect("the preview is stored");
    assert_eq!(
        (preview.url.as_str(), preview.embed_provider.as_str(), preview.embed_id.clone()),
        (link.as_str(), "tokuly", format!("note/{}", other.id))
    );
    for feed in [&mut author_feed, &mut watching] {
        match next_push(feed).await {
            Some(Push::LinkPreviewResolved(resolved)) => assert_eq!(
                (resolved.note_id.as_str(), resolved.link_preview.embed_provider.as_str()),
                (note.id.as_str(), "tokuly")
            ),
            other => panic!("expected the resolved preview, got {other:?}"),
        }
    }
    // The job running twice changes and announces nothing.
    ctx.notes.resolve_link_preview(id(&note.id)).await.unwrap();
    assert!(pushes(&mut watching, QUIET).await.is_empty());

    // The preview of a note that is not public only goes to its author.
    let quiet = ctx.post_visible_to(&alice, &format!("ひみつ {link}"), Visibility::VISIBILITY_FRIENDS).await;
    let mut watching = ctx.app.listen_topic(&format!("subject:3:{}", quiet.id)).await;
    ctx.notes.resolve_link_preview(id(&quiet.id)).await.unwrap();
    assert!(matches!(next_push(&mut author_feed).await, Some(Push::LinkPreviewResolved(_))));
    assert!(pushes(&mut watching, QUIET).await.is_empty());

    // A link that may not be fetched, a note without a link, a note that is gone: nothing to do.
    let unreachable = ctx.post(&alice, "http://127.0.0.1:9/internal").await;
    ctx.notes.resolve_link_preview(id(&unreachable.id)).await.unwrap();
    assert!(ctx.get_note(None, &unreachable.id).await.unwrap().link_preview.as_option().is_none());
    ctx.notes.resolve_link_preview(id(&other.id)).await.unwrap();
    ctx.notes.resolve_link_preview(Uuid::now_v7()).await.unwrap();
}

#[tokio::test]
async fn the_simulator_posts_notes_without_the_api() {
    let ctx = start_with(NotesConfig { posts_per_minute: 2, ..Default::default() }).await;
    let (seeded, mentioned) = (ctx.app.user("seeded").await, ctx.app.user("mentioned").await);
    let tag = unique("demo");

    // No rules agreement, more notes than the API's rate limit allows.
    let mut posted = Vec::new();
    for n in 0..4 {
        posted.push(ctx.notes.post_note(seeded.id, &format!("demo {n} #{tag} @{}", mentioned.handle)).await.unwrap());
    }
    let note = ctx.get_note(None, &posted[0].to_string()).await.unwrap();
    assert_eq!(note.visibility, Visibility::VISIBILITY_PUBLIC);
    assert_eq!(
        (note.hashtags.clone(), note.mentions[0].user_id.clone()),
        (vec![tag.clone()], mentioned.id.to_string())
    );
    assert_eq!(ctx.notifications_for(mentioned.id, "notes.mention").await.len(), 4);
    assert_eq!(ctx.activities_of(seeded.id, "note.posted").await.len(), 4);
    // The agreement is on record, so the user can go on posting through the API.
    let rules: GetNoteRulesResponse =
        ctx.note("GetNoteRules", Some(&seeded), &GetNoteRulesRequest::default()).await.unwrap();
    assert!(rules.agreed);
    // Content rules still apply.
    assert_eq!(ctx.notes.post_note(seeded.id, "   ").await.unwrap_err().reason, "NOTE_EMPTY");
    assert_eq!(ctx.notes.post_note(seeded.id, &"あ".repeat(3001)).await.unwrap_err().reason, "NOTE_TOO_LONG");
}
