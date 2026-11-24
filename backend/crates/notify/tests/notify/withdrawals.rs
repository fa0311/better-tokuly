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
