//! Withdrawals: when what a notification is about is deleted or taken down,
//! its inbox entries go too, and late requests and pending pushes about it
//! stay silent.
use std::time::Duration;

use chrono::Utc;
use tokuly_core::outbound::{Localized, NOTIFY_TOPIC, NotificationRequest};
use tokuly_notify::consumer::RequestOutcome;
use tokuly_proto::{common::SubjectType, notify::*};
use tokuly_testkit::TestUser;
use uuid::Uuid;

use crate::common::*;

const WAIT: Duration = Duration::from_secs(2);

/// "`from` commented on your note: <text>", as engagement writes it.
fn comment_on(to: &TestUser, from: &TestUser, note_id: Uuid, comment_id: Uuid) -> NotificationRequest {
    NotificationRequest::new(
        to.id,
        "engagement.comment",
        Localized::new("コメントが来たよ!", "New comment"),
        Localized::same("ひどいこと書いてやる"),
        format!("/notes/{note_id}?comment={comment_id}"),
    )
    .actor(from.id)
    .subject(SubjectType::SUBJECT_TYPE_NOTE, note_id)
    .data(serde_json::json!({ "comment_id": comment_id, "parent_id": null }))
}

async fn kinds(ctx: &Ctx, user: &TestUser) -> Vec<String> {
    ctx.inbox(user).await.notifications.into_iter().map(|n| n.kind).collect()
}

#[tokio::test]
async fn a_taken_down_subject_leaves_every_inbox_but_the_owners_notice() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);
    let note_id = Uuid::now_v7();
    let other_note = Uuid::now_v7();
    ctx.deliver_to_inbox(&comment_on(&alice, &bob, note_id, Uuid::now_v7())).await;
    ctx.deliver_to_inbox(&comment_on(&carol, &bob, note_id, Uuid::now_v7())).await;
    ctx.deliver_to_inbox(&comment_on(&carol, &bob, other_note, Uuid::now_v7())).await;
    // What the admin domain tells the owner about the takedown names the subject too.
    let notice = NotificationRequest::new(
        bob.id,
        "system.content_removed",
        Localized::new("投稿が削除されたよ", "Your post was removed"),
        Localized::same("…"),
        "/notifications",
    )
    .subject(SubjectType::SUBJECT_TYPE_NOTE, note_id);
    ctx.deliver_to_inbox(&notice).await;

    let mut frames = ctx.app.listen_user(alice.id).await;
    let takedown = event(
        "admin.content.removed",
        serde_json::json!({
            "subject_type": SubjectType::SUBJECT_TYPE_NOTE as i32,
            "subject_id": note_id,
            "reason": "spam",
            "moderator_id": Uuid::now_v7(),
        }),
    );
    ctx.notify.handle_event(takedown.clone()).await.unwrap();
    ctx.notify.handle_event(takedown).await.unwrap(); // at-least-once

    assert!(kinds(&ctx, &alice).await.is_empty());
    assert_eq!(unread_of(&ctx.inbox(&alice).await), 0);
    assert_eq!(ctx.inbox(&carol).await.notifications.len(), 1, "the comment on the other note stays");
    assert_eq!(kinds(&ctx, &bob).await, ["system.content_removed"]);

    // Alice's open apps take the entry away.
    let removed = loop {
        let frame = frames.next(WAIT).await.expect("a realtime frame");
        if let Some(NotifyEvent { event: Some(notify_event::Event::Removed(removed)), .. }) =
            frame.event::<NotifyEvent>()
        {
            break removed;
        }
    };
    assert_eq!(removed.notification_ids.len(), 1);
    assert_eq!(removed.unread.as_option().unwrap().total, 0);

    // A request about it that was emitted before the takedown but arrives
    // after it (events are not ordered) leaves nothing either.
    let late = event_at(
        NOTIFY_TOPIC,
        comment_on(&alice, &bob, note_id, Uuid::now_v7()),
        Utc::now() - chrono::Duration::seconds(5),
    );
    assert!(matches!(ctx.notify.handle_request(&late).await.unwrap(), RequestOutcome::Dropped(_)));
    assert!(kinds(&ctx, &alice).await.is_empty());
}

#[tokio::test]
async fn deleted_messages_and_comments_are_withdrawn() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let note_id = Uuid::now_v7();
    let (deleted_comment, kept_comment) = (Uuid::now_v7(), Uuid::now_v7());
    ctx.deliver_to_inbox(&comment_on(&alice, &bob, note_id, deleted_comment)).await;
    ctx.deliver_to_inbox(&comment_on(&alice, &bob, note_id, kept_comment)).await;
    let message_id = Uuid::now_v7();
    ctx.deliver_to_inbox(
        &chat_message(&alice, &bob)
            .subject(SubjectType::SUBJECT_TYPE_CHAT_MESSAGE, message_id)
            .data(serde_json::json!({ "conversation_id": Uuid::now_v7(), "message_id": message_id })),
    )
    .await;
    assert_eq!(ctx.inbox(&alice).await.notifications.len(), 3);

    ctx.notify
        .handle_event(event(
            "engagement.comment.deleted",
            serde_json::json!({
                "comment_id": deleted_comment,
                "subject_type": SubjectType::SUBJECT_TYPE_NOTE as i32,
                "subject_id": note_id,
                "subject_owner_id": alice.id,
                "author_id": bob.id,
                "channel_id": null,
                "parent_id": null,
            }),
        ))
        .await
        .unwrap();
    ctx.notify
        .handle_event(event(
            "chat.message.deleted",
            serde_json::json!({
                "message_id": message_id,
                "conversation_id": Uuid::now_v7(),
                "sender_id": bob.id,
                "deleted_by": bob.id,
                "moderated": false,
            }),
        ))
        .await
        .unwrap();

    let inbox = ctx.inbox(&alice).await;
    assert_eq!(inbox.notifications.len(), 1, "{:?}", kinds(&ctx, &alice).await);
    assert!(inbox.notifications[0].data_json.contains(&kept_comment.to_string()));
}

#[tokio::test]
async fn a_push_still_queued_for_withdrawn_content_is_not_sent() {
    let ctx = start_insecure().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let stub = Stub::accepting().await;
    let subscription = Subscription::new(&format!("{}/push/laptop", stub.url));
    ctx.register(&alice, &subscription.request()).await;
    let message_id = Uuid::now_v7();
    // Chat messages are push-only: there is no inbox entry to delete.
    let job = ctx
        .deliver_for_push(
            &chat_message(&alice, &bob).push_only().subject(SubjectType::SUBJECT_TYPE_CHAT_MESSAGE, message_id),
        )
        .await;

    // Unsent before the push service could be reached.
    ctx.notify
        .handle_event(event(
            "chat.message.deleted",
            serde_json::json!({
                "message_id": message_id,
                "conversation_id": Uuid::now_v7(),
                "sender_id": bob.id,
                "deleted_by": bob.id,
                "moderated": false,
            }),
        ))
        .await
        .unwrap();
    let report = ctx.notify.deliver_push(&job).await.unwrap();
    assert_eq!(report.delivered, 0);
    assert!(stub.requests().is_empty(), "the unsent text never reaches the lock screen");
}

#[tokio::test]
async fn an_unsend_handled_at_the_same_time_as_the_message_leaves_nothing() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    for _ in 0..30 {
        let message_id = Uuid::now_v7();
        let request =
            request_event(&chat_message(&alice, &bob).subject(SubjectType::SUBJECT_TYPE_CHAT_MESSAGE, message_id));
        let unsent = event("chat.message.deleted", serde_json::json!({ "message_id": message_id }));
        let (stored, withdrawn) = tokio::join!(ctx.notify.handle_request(&request), ctx.notify.handle_event(unsent));
        stored.unwrap();
        withdrawn.unwrap();
    }
    assert!(ctx.inbox(&alice).await.notifications.is_empty());
}

/// A notification about `subject` as its producer writes it.
fn about(to: &TestUser, kind: &str, subject: (SubjectType, Uuid), data: serde_json::Value) -> NotificationRequest {
    NotificationRequest::new(to.id, kind, Localized::same(kind), Localized::same("…"), "/somewhere")
        .subject(subject.0, subject.1)
        .data(data)
}

async fn withdraw_event(ctx: &Ctx, topic: &str, data: serde_json::Value) {
    ctx.notify.handle_event(event(topic, data)).await.unwrap();
}

#[tokio::test]
async fn deleted_videos_shorts_streams_and_answers_are_withdrawn() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let (video, short, stream, question) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    // As video, shorts and live write them.
    ctx.deliver_to_inbox(&about(
        &alice,
        "video.published",
        (SubjectType::SUBJECT_TYPE_VIDEO, video),
        serde_json::json!({ "video_id": video, "short_id": "abc", "channel_id": Uuid::now_v7() }),
    ))
    .await;
    ctx.deliver_to_inbox(&about(
        &alice,
        "shorts.published",
        (SubjectType::SUBJECT_TYPE_SHORT, short),
        serde_json::json!({ "id": short, "short_id": "def" }),
    ))
    .await;
    ctx.deliver_to_inbox(&about(
        &alice,
        "live.started",
        (SubjectType::SUBJECT_TYPE_LIVE_STREAM, stream),
        serde_json::json!({ "channel_id": Uuid::now_v7() }),
    ))
    .await;
    // Ask: the asker hears about the answer; the box owner had heard about the question.
    let answered = NotificationRequest::new(
        alice.id,
        "ask.answered",
        Localized::same("bobさんが質問に答えたよ!"),
        Localized::same("the answer text"),
        format!("/ask/q/{question}"),
    )
    .actor(bob.id)
    .collapse(format!("ask.answered:{question}"))
    .data(serde_json::json!({ "question_id": question }));
    ctx.deliver_to_inbox(&answered).await;
    let asked = NotificationRequest::new(
        bob.id,
        "ask.question",
        Localized::same("質問が届いたよ!"),
        Localized::same("?"),
        "/ask",
    )
    .data(serde_json::json!({ "question_id": question }));
    ctx.deliver_to_inbox(&asked).await;
    assert_eq!(ctx.inbox(&alice).await.notifications.len(), 4);

    withdraw_event(&ctx, "video.video.deleted", serde_json::json!({ "video_id": video, "owner_id": bob.id })).await;
    assert_eq!(ctx.inbox(&alice).await.notifications.len(), 3);
    withdraw_event(
        &ctx,
        "shorts.short.deleted",
        serde_json::json!({ "id": short, "short_id": "def", "owner_id": bob.id }),
    )
    .await;
    assert_eq!(ctx.inbox(&alice).await.notifications.len(), 2);
    withdraw_event(&ctx, "live.stream.deleted", serde_json::json!({ "stream_id": stream, "owner_id": bob.id })).await;
    assert_eq!(kinds(&ctx, &alice).await, ["ask.answered"]);
    withdraw_event(&ctx, "ask.answer.deleted", serde_json::json!({ "question_id": question, "owner_id": bob.id }))
        .await;
    assert!(kinds(&ctx, &alice).await.is_empty());
    assert_eq!(kinds(&ctx, &bob).await, ["ask.question"], "the question itself is still there");
    // The question can be answered again, and that is announced.
    ctx.deliver_to_inbox(&answered).await;
}

#[tokio::test]
async fn hidden_content_leaves_everyone_but_its_owner() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let (video, note) = (Uuid::now_v7(), Uuid::now_v7());
    ctx.deliver_to_inbox(&about(
        &alice,
        "video.published",
        (SubjectType::SUBJECT_TYPE_VIDEO, video),
        serde_json::json!({ "video_id": video }),
    ))
    .await;
    // The owner's own notice about the video.
    ctx.deliver_to_inbox(&about(
        &bob,
        "video.processing_done",
        (SubjectType::SUBJECT_TYPE_VIDEO, video),
        serde_json::json!({ "video_id": video, "published": true }),
    ))
    .await;
    ctx.deliver_to_inbox(&about(&alice, "notes.like", (SubjectType::SUBJECT_TYPE_NOTE, note), serde_json::json!({})))
        .await;
    ctx.deliver_to_inbox(&about(&bob, "notes.like", (SubjectType::SUBJECT_TYPE_NOTE, note), serde_json::json!({})))
        .await;

    withdraw_event(&ctx, "video.video.unpublished", serde_json::json!({ "video_id": video, "owner_id": bob.id })).await;
    assert_eq!(kinds(&ctx, &alice).await, ["notes.like"]);
    // Narrowed to friends: who still sees it is not known here.
    withdraw_event(
        &ctx,
        "notes.note.visibility_changed",
        serde_json::json!({ "note_id": note, "author_id": bob.id, "visibility": 4 }),
    )
    .await;
    assert_eq!(kinds(&ctx, &alice).await, ["notes.like"]);
    withdraw_event(
        &ctx,
        "notes.note.visibility_changed",
        serde_json::json!({ "note_id": note, "author_id": bob.id, "visibility": 5 }),
    )
    .await;
    assert!(kinds(&ctx, &alice).await.is_empty());
    assert_eq!(ctx.inbox(&bob).await.notifications.len(), 2, "the owner keeps theirs");
    // Hidden is not gone: published again, it is announced again.
    ctx.deliver_to_inbox(&about(
        &alice,
        "video.published",
        (SubjectType::SUBJECT_TYPE_VIDEO, video),
        serde_json::json!({ "video_id": video }),
    ))
    .await;
}

#[tokio::test]
async fn a_grouped_entry_loses_only_the_deleted_comment() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let note_id = Uuid::now_v7();
    let comments = [Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7()];
    let start = Utc::now() - chrono::Duration::seconds(10);
    for (i, comment) in comments.iter().enumerate() {
        let request = comment_on(&alice, &bob, note_id, *comment).collapse(format!("comments:{note_id}"));
        let event = event_at(NOTIFY_TOPIC, request, start + chrono::Duration::seconds(i as i64));
        ctx.notify.handle_request(&event).await.unwrap();
    }
    let entry = |inbox: ListNotificationsResponse| inbox.notifications.into_iter().next();
    assert_eq!(entry(ctx.inbox(&alice).await).unwrap().group_count, 3);

    // The latest comment, the one the entry quotes, is deleted.
    withdraw_event(&ctx, "engagement.comment.deleted", serde_json::json!({ "comment_id": comments[2] })).await;
    let shrunk = entry(ctx.inbox(&alice).await).expect("the group stays");
    assert_eq!(shrunk.group_count, 2);
    assert!(shrunk.body.is_empty(), "the deleted text is gone: {:?}", shrunk.body);
    // An older one: the entry keeps what it shows.
    withdraw_event(&ctx, "engagement.comment.deleted", serde_json::json!({ "comment_id": comments[0] })).await;
    assert_eq!(entry(ctx.inbox(&alice).await).unwrap().group_count, 1);
    // The last one takes the entry with it.
    withdraw_event(&ctx, "engagement.comment.deleted", serde_json::json!({ "comment_id": comments[1] })).await;
    assert!(entry(ctx.inbox(&alice).await).is_none());
}

#[tokio::test]
async fn content_with_thousands_of_recipients_is_withdrawn_in_batches() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let video = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO notifications (user_id, kind, category, title_ja, title_en, body_ja, body_en, path,
                                    subject_type, subject_id, refs)
         SELECT $1, 'video.published', 'video', 't', 't', 'b', 'b', '/', 5, $2, ARRAY[$2]::uuid[]
         FROM generate_series(1, 2500)",
    )
    .bind(alice.id)
    .bind(video)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
    let mut frames = ctx.app.listen_user(alice.id).await;
    withdraw_event(&ctx, "video.video.deleted", serde_json::json!({ "video_id": video })).await;
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE user_id = $1")
        .bind(alice.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(left, 0);
    // One `removed` per batch, each with the count after it.
    let mut batches = 0;
    while let Some(frame) = frames.next(Duration::from_millis(500)).await {
        if let Some(NotifyEvent { event: Some(notify_event::Event::Removed(removed)), .. }) =
            frame.event::<NotifyEvent>()
        {
            batches += 1;
            assert!(removed.bulk, "more than 200 per batch");
        }
    }
    assert_eq!(batches, 3);
}

fn comment_hidden(
    comment: Uuid,
    author: &TestUser,
    visible_to: &[Uuid],
    changed_at: chrono::DateTime<Utc>,
) -> tokuly_core::events::Event {
    event(
        "engagement.comment.hidden",
        serde_json::json!({
            "comment_id": comment,
            "subject_type": SubjectType::SUBJECT_TYPE_NOTE as i32,
            "subject_id": Uuid::now_v7(),
            "subject_owner_id": visible_to.first(),
            "author_id": author.id,
            "channel_id": null,
            "parent_id": null,
            "status": 3,
            "hold_reason": 4,
            "visible_to": visible_to,
            "changed_at": changed_at,
        }),
    )
}

#[tokio::test]
async fn a_hidden_comment_leaves_everyone_who_can_no_longer_see_it() {
    let ctx = start().await;
    let (owner, alice, carol, dave) = (
        ctx.app.user("owner").await,
        ctx.app.user("alice").await,
        ctx.app.user("carol").await,
        ctx.app.user("dave").await,
    );
    let note_id = Uuid::now_v7();
    let (comment, kept) = (Uuid::now_v7(), Uuid::now_v7());
    let before = Utc::now() - chrono::Duration::seconds(30);
    let at = |seconds: i64| before + chrono::Duration::seconds(seconds);
    // Alice wrote the comment; the owner of the note, Carol (answered by a
    // reply in the thread) and Alice herself (a heart) heard about it.
    for (to, from) in [(&owner, &alice), (&carol, &dave), (&alice, &owner)] {
        let request = comment_on(to, from, note_id, comment);
        ctx.notify.handle_request(&event_at(NOTIFY_TOPIC, request, at(0))).await.unwrap();
    }
    ctx.notify.handle_request(&event_at(NOTIFY_TOPIC, comment_on(&carol, &dave, note_id, kept), at(0))).await.unwrap();
    // A reply below it names it as its thread.
    let reply = comment_on(&dave, &carol, note_id, Uuid::now_v7())
        .data(serde_json::json!({ "comment_id": Uuid::now_v7(), "parent_id": comment }));
    ctx.notify.handle_request(&event_at(NOTIFY_TOPIC, reply, at(0))).await.unwrap();

    // The owner hides it: only the owner and its author still see it.
    let hidden = comment_hidden(comment, &alice, &[owner.id], at(10));
    ctx.notify.handle_event(hidden.clone()).await.unwrap();
    ctx.notify.handle_event(hidden).await.unwrap(); // at-least-once
    assert_eq!(ctx.inbox(&owner).await.notifications.len(), 1, "the owner still sees it");
    assert_eq!(ctx.inbox(&alice).await.notifications.len(), 1, "so does its author");
    assert_eq!(ctx.inbox(&carol).await.notifications.len(), 1, "only the entry about the other comment is left");
    assert!(ctx.inbox(&carol).await.notifications[0].data_json.contains(&kept.to_string()));
    assert!(ctx.inbox(&dave).await.notifications.is_empty(), "the reply went with its thread");

    // A request from before the hiding, handled after it, reaches nobody
    // who lost the comment; one made after it (approved again) does.
    let late = event_at(NOTIFY_TOPIC, comment_on(&carol, &dave, note_id, comment), at(5));
    assert!(matches!(ctx.notify.handle_request(&late).await.unwrap(), RequestOutcome::Dropped(_)));
    let late_to_owner = event_at(NOTIFY_TOPIC, comment_on(&owner, &dave, note_id, comment), at(5));
    assert!(matches!(ctx.notify.handle_request(&late_to_owner).await.unwrap(), RequestOutcome::Handled { .. }));
    let after = event_at(NOTIFY_TOPIC, comment_on(&dave, &carol, note_id, comment), at(20));
    assert!(matches!(ctx.notify.handle_request(&after).await.unwrap(), RequestOutcome::Handled { .. }));

    // An older change handled late takes nothing that came after.
    ctx.notify.handle_event(comment_hidden(comment, &alice, &[], at(8))).await.unwrap();
    assert_eq!(ctx.inbox(&dave).await.notifications.len(), 1);
    assert_eq!(ctx.inbox(&owner).await.notifications.len(), 2);
    // A newer one (a block) takes what came before it, but not the author's.
    ctx.notify.handle_event(comment_hidden(comment, &alice, &[], at(25))).await.unwrap();
    assert!(ctx.inbox(&dave).await.notifications.is_empty());
    assert!(ctx.inbox(&owner).await.notifications.is_empty());
    assert_eq!(ctx.inbox(&alice).await.notifications.len(), 1);
}

#[tokio::test]
async fn a_push_still_queued_for_a_hidden_comment_reaches_only_who_still_sees_it() {
    let ctx = start_insecure().await;
    let (owner, alice, carol) = (ctx.app.user("owner").await, ctx.app.user("alice").await, ctx.app.user("carol").await);
    let stub = Stub::accepting().await;
    ctx.register(&owner, &Subscription::new(&format!("{}/push/owner", stub.url)).request()).await;
    ctx.register(&carol, &Subscription::new(&format!("{}/push/carol", stub.url)).request()).await;
    let (note_id, comment) = (Uuid::now_v7(), Uuid::now_v7());
    let to_owner = ctx.deliver_for_push(&comment_on(&owner, &alice, note_id, comment)).await;
    let to_carol = ctx.deliver_for_push(&comment_on(&carol, &alice, note_id, comment)).await;
    ctx.notify.handle_event(comment_hidden(comment, &alice, &[owner.id], Utc::now())).await.unwrap();
    assert_eq!(ctx.notify.deliver_push(&to_carol).await.unwrap().delivered, 0);
    assert_eq!(ctx.notify.deliver_push(&to_owner).await.unwrap().delivered, 1);
    assert!(stub.requests_to("/push/carol").is_empty());
}

fn visibility_changed(
    note: Uuid,
    author: &TestUser,
    visibility: i32,
    visible_to: Option<&[Uuid]>,
    changed_at: chrono::DateTime<Utc>,
) -> tokuly_core::events::Event {
    event(
        "notes.note.visibility_changed",
        serde_json::json!({
            "note_id": note,
            "author_id": author.id,
            "visibility": visibility,
            "visible_to": visible_to,
            "changed_at": changed_at,
        }),
    )
}

#[tokio::test]
async fn a_note_narrowed_to_friends_leaves_everyone_else_in_the_order_it_changed() {
    let ctx = start().await;
    let (bob, alice, carol, dave) = (
        ctx.app.user("bob").await,
        ctx.app.user("alice").await,
        ctx.app.user("carol").await,
        ctx.app.user("dave").await,
    );
    let note = Uuid::now_v7();
    let before = Utc::now() - chrono::Duration::seconds(60);
    let at = |seconds: i64| before + chrono::Duration::seconds(seconds);
    let like = |to: &TestUser| about(to, "notes.like", (SubjectType::SUBJECT_TYPE_NOTE, note), serde_json::json!({}));
    // Bob's public note: Alice, Carol and Dave heard about it (a mention, a
    // comment on it...), and so did Bob.
    for user in [&alice, &carol, &dave, &bob] {
        ctx.notify.handle_request(&event_at(NOTIFY_TOPIC, like(user), at(0))).await.unwrap();
    }
    ctx.notify
        .handle_request(&event_at(NOTIFY_TOPIC, comment_on(&dave, &alice, note, Uuid::now_v7()), at(0)))
        .await
        .unwrap();

    // Narrowed to friends: Carol is one.
    ctx.notify.handle_event(visibility_changed(note, &bob, 4, Some(&[carol.id]), at(10))).await.unwrap();
    assert!(kinds(&ctx, &alice).await.is_empty());
    assert!(kinds(&ctx, &dave).await.is_empty(), "comments on the note go with it");
    assert_eq!(kinds(&ctx, &carol).await, ["notes.like"]);
    assert_eq!(kinds(&ctx, &bob).await, ["notes.like"], "the author keeps theirs");

    // A request from before the change, handled after it, reaches only who
    // still sees the note.
    let late = event_at(NOTIFY_TOPIC, like(&alice), at(5));
    assert!(matches!(ctx.notify.handle_request(&late).await.unwrap(), RequestOutcome::Dropped(_)));
    ctx.notify.handle_request(&event_at(NOTIFY_TOPIC, like(&carol), at(5))).await.unwrap();
    assert_eq!(ctx.inbox(&carol).await.notifications.len(), 2);

    // Public again, and Alice hears about it again; then the narrowing is
    // delivered once more, late: it takes nothing that came after it, and
    // nothing at all, since a newer change is known.
    ctx.notify.handle_event(visibility_changed(note, &bob, 1, None, at(20))).await.unwrap();
    ctx.notify.handle_request(&event_at(NOTIFY_TOPIC, like(&alice), at(25))).await.unwrap();
    ctx.notify.handle_event(visibility_changed(note, &bob, 4, Some(&[carol.id]), at(10))).await.unwrap();
    ctx.notify.handle_event(visibility_changed(note, &bob, 5, Some(&[]), at(15))).await.unwrap();
    assert_eq!(kinds(&ctx, &alice).await, ["notes.like"]);
    // And a request from the friends-only time is no longer held back.
    ctx.notify.handle_request(&event_at(NOTIFY_TOPIC, like(&dave), at(12))).await.unwrap();
    assert_eq!(kinds(&ctx, &dave).await, ["notes.like"]);

    // Made private later on: only the author is left, also for what came after the earlier changes.
    ctx.notify.handle_event(visibility_changed(note, &bob, 5, Some(&[]), at(30))).await.unwrap();
    for user in [&alice, &carol, &dave] {
        assert!(kinds(&ctx, user).await.is_empty());
    }
    assert_eq!(kinds(&ctx, &bob).await, ["notes.like"]);
}
