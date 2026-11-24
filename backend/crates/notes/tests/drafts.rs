//! Changing notes after posting (edits with history, delete and redraft)
//! and server drafts with scheduled posting.
mod common;

use common::*;
use tokuly_proto::{common::Visibility, notes::*};
use tokuly_testkit::TestUser;
use uuid::Uuid;

async fn edit(ctx: &Ctx, user: &TestUser, note: &Note, text: &str) -> Result<Note, String> {
    let request = EditNoteRequest { note_id: note.id.clone(), text: text.into(), ..Default::default() };
    ctx.note::<_, EditNoteResponse>("EditNote", Some(user), &request)
        .await
        .map(|response| response.note.as_option().cloned().unwrap())
        .map_err(|e| e.reason)
}

async fn draft_call<Req: serde::Serialize, Resp: serde::de::DeserializeOwned>(
    ctx: &Ctx,
    method: &str,
    user: &TestUser,
    request: &Req,
) -> Result<Resp, String> {
    ctx.app.call(&format!("tokuly.notes.v1.NoteDraftService/{method}"), Some(user), request).await.map_err(|e| e.reason)
}

#[tokio::test]
async fn notes_are_edited_within_the_hour_and_keep_their_history() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.member("alice").await, ctx.member("bob").await, ctx.member("carol").await);
    let (old_tag, new_tag) = (unique("old"), unique("new"));
    let note = ctx.post(&alice, &format!("first try #{old_tag}")).await;
    assert!(note.viewer.can_edit);
    assert!(!ctx.get_note(Some(&bob), &note.id).await.unwrap().viewer.can_edit);

    let mut listener = ctx.app.listen_topic(&tokuly_notes::model::note_topic(id(&note.id))).await;
    let edited = edit(&ctx, &alice, &note, &format!("second try #{new_tag} @{}", bob.handle)).await.unwrap();
    assert_eq!((edited.edit_count, edited.hashtags.clone()), (1, vec![new_tag.clone()]));
    assert!(edited.edited_at.as_option().is_some());
    assert!(matches!(next_push(&mut listener).await, Some(notes_event::Event::NoteEdited(_))));
    assert_eq!(ctx.notifications_for(bob.id, "notes.mention").await.len(), 1, "newly mentioned people are told");
    edit(&ctx, &alice, &edited, &format!("third @{}", bob.handle)).await.unwrap();
    assert_eq!(ctx.notifications_for(bob.id, "notes.mention").await.len(), 1, "and only once");
    // The hashtag timeline follows the text.
    let tag_count = |tag: String| {
        let ctx = &ctx;
        async move {
            sqlx::query_scalar::<_, i64>("SELECT note_count FROM hashtags WHERE tag = $1")
                .bind(tag)
                .fetch_one(&ctx.app.core.db)
                .await
                .unwrap()
        }
    };
    assert_eq!(tag_count(old_tag.clone()).await, 0);

    // Everyone who may see the note may read its history.
    let history: ListNoteEditsResponse = ctx
        .note("ListNoteEdits", Some(&carol), &ListNoteEditsRequest { note_id: note.id.clone(), ..Default::default() })
        .await
        .unwrap();
    let versions: Vec<(i32, &str)> = history.edits.iter().map(|e| (e.version, e.text.as_str())).collect();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[1], (1, format!("first try #{old_tag}").as_str()));

    assert_eq!(edit(&ctx, &bob, &note, "hijack").await.unwrap_err(), "NOT_NOTE_AUTHOR");
    assert_eq!(edit(&ctx, &alice, &note, &"あ".repeat(3001)).await.unwrap_err(), "NOTE_TOO_LONG");
    assert_eq!(edit(&ctx, &alice, &note, "  ").await.unwrap_err(), "NOTE_EMPTY");
    sqlx::query("UPDATE notes SET created_at = created_at - interval '61 minutes' WHERE id = $1")
        .bind(id(&note.id))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(edit(&ctx, &alice, &note, "too late").await.unwrap_err(), "EDIT_WINDOW_CLOSED");
    let fresh = ctx.post(&alice, "v0").await;
    sqlx::query("UPDATE notes SET edit_count = 10 WHERE id = $1")
        .bind(id(&fresh.id))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(edit(&ctx, &alice, &fresh, "v11").await.unwrap_err(), "TOO_MANY_EDITS");

    // A friends-only note's history is as hidden as the note.
    let private = ctx.post_visible_to(&alice, "secret", Visibility::VISIBILITY_FRIENDS).await;
    let reason = ctx
        .note_reason(
            "ListNoteEdits",
            Some(&carol),
            &ListNoteEditsRequest { note_id: private.id.clone(), ..Default::default() },
        )
        .await;
    assert_eq!(reason, "NOTE_NOT_FOUND");
}

#[tokio::test]
async fn delete_and_redraft_keeps_the_content_and_its_uploads() {
    let ctx = start().await;
    let alice = ctx.member("alice").await;
    let image = ctx.note_image(&alice).await;
    let note = ctx
        .post_with(&alice, "書き直したい", |request| {
            request.asset_ids = vec![image.clone()];
            request.content_warning_text = "注意".into();
            request.visibility = Visibility::VISIBILITY_FOLLOWERS.into();
        })
        .await;
    let response: RedraftNoteResponse = ctx
        .note("RedraftNote", Some(&alice), &RedraftNoteRequest { note_id: note.id.clone(), ..Default::default() })
        .await
        .unwrap();
    let draft = response.draft.as_option().unwrap();
    assert_eq!(draft.content.text, "書き直したい");
    assert_eq!(draft.content.asset_ids, vec![image.clone()]);
    assert_eq!(draft.content.content_warning_text, "注意");
    assert_eq!(draft.content.visibility, buffa::EnumValue::from(Visibility::VISIBILITY_FOLLOWERS));
    assert_eq!(draft.attachments.len(), 1);
    assert_eq!(ctx.get_note(Some(&alice), &note.id).await.unwrap_err().reason, "NOTE_NOT_FOUND");
    let holders: i64 = sqlx::query_scalar("SELECT count(*) FROM media_asset_holders WHERE asset_id = $1")
        .bind(id(&image))
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(holders, 1, "the draft holds the upload now");
    // Posting the draft posts the same content again.
    let published: PublishDraftResponse = draft_call(
        &ctx,
        "PublishDraft",
        &alice,
        &PublishDraftRequest { draft_id: draft.id.clone(), ..Default::default() },
    )
    .await
    .unwrap();
    assert_eq!(published.note.text, "書き直したい");
    assert_eq!(published.note.attachments.len(), 1);
    let gone = draft_call::<_, GetDraftResponse>(
        &ctx,
        "GetDraft",
        &alice,
        &GetDraftRequest { draft_id: draft.id.clone(), ..Default::default() },
    )
    .await;
    assert_eq!(gone.unwrap_err(), "DRAFT_NOT_FOUND");
}

#[tokio::test]
async fn drafts_are_saved_listed_scheduled_and_posted_once() {
    let ctx = start().await;
    let (alice, bob) = (ctx.member("alice").await, ctx.member("bob").await);
    let content = |text: &str| DraftContent { text: text.into(), ..Default::default() };
    let saved: SaveDraftResponse = draft_call(
        &ctx,
        "SaveDraft",
        &alice,
        &SaveDraftRequest { content: content("下書き").into(), ..Default::default() },
    )
    .await
    .unwrap();
    let draft_id = saved.draft.id.clone();
    let updated: SaveDraftResponse = draft_call(
        &ctx,
        "SaveDraft",
        &alice,
        &SaveDraftRequest {
            draft_id: draft_id.clone(), content: content("書き足した").into(), ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!((updated.draft.id.clone(), updated.draft.content.text.clone()), (draft_id.clone(), "書き足した".into()));
    // Someone else's draft does not exist for them.
    let theirs = draft_call::<_, SaveDraftResponse>(
        &ctx,
        "SaveDraft",
        &bob,
        &SaveDraftRequest { draft_id: draft_id.clone(), content: content("x").into(), ..Default::default() },
    )
    .await;
    assert_eq!(theirs.unwrap_err(), "DRAFT_NOT_FOUND");
    let someone_elses_upload = ctx.note_image(&bob).await;
    let reason = draft_call::<_, SaveDraftResponse>(
        &ctx,
        "SaveDraft",
        &alice,
        &SaveDraftRequest {
            content: DraftContent { asset_ids: vec![someone_elses_upload], ..Default::default() }.into(),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(reason.unwrap_err(), "ASSET_NOT_FOUND");

    // Scheduling: within limits only.
    let at = |seconds: i64| tokuly_core::time::ts(chrono::Utc::now() + chrono::Duration::seconds(seconds));
    let too_soon = draft_call::<_, SaveDraftResponse>(
        &ctx,
        "SaveDraft",
        &alice,
        &SaveDraftRequest { content: content("soon").into(), scheduled_at: at(60), ..Default::default() },
    )
    .await;
    assert_eq!(too_soon.unwrap_err(), "INVALID_SCHEDULE");
    let scheduled: SaveDraftResponse = draft_call(
        &ctx,
        "SaveDraft",
        &alice,
        &SaveDraftRequest {
            content: content("予約したノート").into(), scheduled_at: at(600), ..Default::default()
        },
    )
    .await
    .unwrap();
    let list: ListDraftsResponse =
        draft_call(&ctx, "ListDrafts", &alice, &ListDraftsRequest { scheduled_only: true, ..Default::default() })
            .await
            .unwrap();
    assert_eq!(list.drafts.len(), 1);
    let all: ListDraftsResponse = draft_call(&ctx, "ListDrafts", &alice, &ListDraftsRequest::default()).await.unwrap();
    assert_eq!(all.drafts.len(), 2);
    assert_eq!(all.drafts[0].id, scheduled.draft.id, "most recently changed first");

    // Its time comes: posted once, however often the scheduler runs.
    sqlx::query("UPDATE note_drafts SET scheduled_at = now() - interval '1 second' WHERE id = $1")
        .bind(id(&scheduled.draft.id))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.post(&alice, "posted before the schedule came").await;
    // (Scheduler passes of other tests in this binary may take it too: what
    // counts is that it is posted, once.)
    let (first, second) = tokio::join!(ctx.notes.publish_due_drafts(), ctx.notes.publish_due_drafts());
    first.unwrap();
    second.unwrap();
    for _ in 0..50 {
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM note_drafts WHERE id = $1")
            .bind(id(&scheduled.draft.id))
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
        if left == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(ctx.get_note(Some(&alice), &scheduled.draft.id).await.is_err(), "the note has an ID of its own");
    // It is new: on top of the profile, above what was posted while it waited.
    let profile: GetUserTimelineResponse = ctx
        .note(
            "GetUserTimeline",
            Some(&alice),
            &GetUserTimelineRequest { user_id: alice.id.to_string(), page: page(5, ""), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(texts(&profile.entries)[0], "予約したノート");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM notes WHERE author_id = $1 AND text = '予約したノート'")
        .bind(alice.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(count, 1);

    // A scheduled draft that cannot be posted stays, with the reason.
    let broken: SaveDraftResponse = draft_call(
        &ctx,
        "SaveDraft",
        &alice,
        &SaveDraftRequest {
            content: DraftContent {
                text: "reply".into(),
                reply_to_id: Uuid::now_v7().to_string(),
                ..Default::default()
            }
            .into(),
            scheduled_at: at(600),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    sqlx::query("UPDATE note_drafts SET scheduled_at = now() WHERE id = $1")
        .bind(id(&broken.draft.id))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.notes.publish_due_drafts().await.unwrap();
    // Other tests run the scheduler on the same database: a pass of theirs may have claimed this
    // draft (it reads INTERRUPTED meanwhile) and finish it a moment later.
    let mut kept: GetDraftResponse;
    let mut tries = 0;
    loop {
        kept = draft_call(
            &ctx,
            "GetDraft",
            &alice,
            &GetDraftRequest { draft_id: broken.draft.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
        if kept.draft.failure_reason != "INTERRUPTED" || tries >= 50 {
            break;
        }
        tries += 1;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(kept.draft.failure_reason, "NOTE_NOT_FOUND");
    assert!(kept.draft.scheduled_at.as_option().is_none());
    assert_eq!(ctx.notifications_for(alice.id, "notes.schedule_failed").await.len(), 1);

    let _: DeleteDraftResponse = draft_call(
        &ctx,
        "DeleteDraft",
        &alice,
        &DeleteDraftRequest { draft_id: draft_id.clone(), ..Default::default() },
    )
    .await
    .unwrap();
    let again = draft_call::<_, DeleteDraftResponse>(
        &ctx,
        "DeleteDraft",
        &alice,
        &DeleteDraftRequest { draft_id, ..Default::default() },
    )
    .await;
    assert_eq!(again.unwrap_err(), "DRAFT_NOT_FOUND");
}

#[tokio::test]
async fn the_draft_limit_holds_under_concurrent_saves() {
    let ctx = start().await;
    let alice = ctx.member("alice").await;
    for index in 0..98 {
        sqlx::query("INSERT INTO note_drafts (author_id, content) VALUES ($1, $2)")
            .bind(alice.id)
            .bind(serde_json::json!({ "text": format!("{index}") }))
            .execute(&ctx.app.core.db)
            .await
            .unwrap();
    }
    let request = SaveDraftRequest {
        content: DraftContent { text: "x".into(), ..Default::default() }.into(),
        ..Default::default()
    };
    let results = futures::future::join_all(
        (0..6).map(|_| draft_call::<_, SaveDraftResponse>(&ctx, "SaveDraft", &alice, &request)),
    )
    .await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 2);
    assert!(results.iter().filter_map(|result| result.as_ref().err()).all(|reason| reason == "TOO_MANY_DRAFTS"));
}

#[tokio::test]
async fn publishing_a_draft_while_the_scheduler_posts_it_posts_it_once() {
    let ctx = start().await;
    let alice = ctx.member("alice").await;
    for round in 0..3 {
        let text = format!("race {round}");
        let saved: SaveDraftResponse = draft_call(
            &ctx,
            "SaveDraft",
            &alice,
            &SaveDraftRequest {
                content: DraftContent { text: text.clone(), ..Default::default() }.into(),
                scheduled_at: tokuly_core::time::ts(chrono::Utc::now() + chrono::Duration::seconds(600)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        sqlx::query("UPDATE note_drafts SET scheduled_at = now() WHERE id = $1")
            .bind(id(&saved.draft.id))
            .execute(&ctx.app.core.db)
            .await
            .unwrap();
        let request = PublishDraftRequest { draft_id: saved.draft.id.clone(), ..Default::default() };
        let (published, scheduled) = tokio::join!(
            draft_call::<_, PublishDraftResponse>(&ctx, "PublishDraft", &alice, &request),
            ctx.notes.publish_due_drafts(),
        );
        let published = published.expect("the person pressing the button gets the note");
        scheduled.unwrap();
        assert_eq!(published.note.text, text);
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM notes WHERE author_id = $1 AND text = $2")
            .bind(alice.id)
            .bind(&text)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
        assert_eq!(count, 1, "posted once");
        let again: PublishDraftResponse = draft_call(&ctx, "PublishDraft", &alice, &request).await.unwrap();
        assert_eq!(again.note.id, published.note.id, "a retry gets the same note");
    }
    assert!(ctx.notifications_for(alice.id, "notes.schedule_failed").await.is_empty(), "no false failure notice");
}
