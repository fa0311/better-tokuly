//! Edited comments: inbox entries and queued pushes quote the comment as it
//! reads now, and a mention the edit took out is withdrawn.
use chrono::{DateTime, Utc};
use tokuly_core::outbound::{Localized, NOTIFY_TOPIC, NotificationRequest};
use tokuly_notify::consumer::{DropReason, RequestOutcome};
use tokuly_proto::common::SubjectType;
use tokuly_testkit::TestUser;
use uuid::Uuid;

use crate::common::*;

/// A notification about a comment as engagement writes it, quoting the
/// version `edited_at` (`None`: never edited).
fn about_comment(
    kind: &str,
    to: &TestUser,
    from: &TestUser,
    comment: Uuid,
    text: &str,
    edited_at: Option<DateTime<Utc>>,
) -> NotificationRequest {
    NotificationRequest::new(to.id, kind, Localized::same("コメント"), Localized::same(text), "/notes/1")
        .actor(from.id)
        .subject(SubjectType::SUBJECT_TYPE_NOTE, Uuid::now_v7())
        .data(serde_json::json!({ "comment_id": comment, "parent_id": null, "edited_at": edited_at }))
}

fn edited(
    comment: Uuid,
    author: &TestUser,
    text: &str,
    mentioned: &[Uuid],
    edited_at: DateTime<Utc>,
) -> tokuly_core::events::Event {
    event(
        "engagement.comment.edited",
        serde_json::json!({
            "comment_id": comment,
            "subject_type": SubjectType::SUBJECT_TYPE_NOTE as i32,
            "subject_id": Uuid::now_v7(),
            "subject_owner_id": Uuid::now_v7(),
            "author_id": author.id,
            "channel_id": null,
            "parent_id": null,
            "body": { "ja": text, "en": text },
            "mentioned": mentioned,
            "edited_at": edited_at,
        }),
    )
}

async fn bodies(ctx: &Ctx, user: &TestUser) -> Vec<(String, String)> {
    ctx.inbox(user).await.notifications.into_iter().map(|n| (n.kind, n.body)).collect()
}

#[tokio::test]
async fn an_edit_rewrites_the_entries_and_withdraws_mentions_it_dropped() {
    let ctx = start().await;
    let (owner, carol, dave, bob) = (
        ctx.app.user("owner").await,
        ctx.app.user("carol").await,
        ctx.app.user("dave").await,
        ctx.app.user("bob").await,
    );
    let (comment, other) = (Uuid::now_v7(), Uuid::now_v7());
    ctx.deliver_to_inbox(&about_comment("engagement.comment", &owner, &bob, comment, "ばか @carol @dave", None)).await;
    ctx.deliver_to_inbox(&about_comment("engagement.mention", &carol, &bob, comment, "ばか @carol @dave", None)).await;
    ctx.deliver_to_inbox(&about_comment("engagement.mention", &dave, &bob, comment, "ばか @carol @dave", None)).await;
    ctx.deliver_to_inbox(&about_comment("engagement.comment", &owner, &bob, other, "べつのコメント", None)).await;

    // Bob takes Carol out of the comment and the insult with her.
    let first = Utc::now() - chrono::Duration::seconds(10);
    let edit = edited(comment, &bob, "ごめんね @dave", &[dave.id], first);
    ctx.notify.handle_event(edit.clone()).await.unwrap();
    ctx.notify.handle_event(edit).await.unwrap(); // at-least-once

    let mut owners = bodies(&ctx, &owner).await;
    owners.sort();
    assert_eq!(
        owners,
        [
            ("engagement.comment".to_owned(), "ごめんね @dave".to_owned()),
            ("engagement.comment".into(), "べつのコメント".into())
        ]
    );
    assert!(bodies(&ctx, &carol).await.is_empty(), "the mention is gone with the edit");
    assert_eq!(bodies(&ctx, &dave).await, [("engagement.mention".to_owned(), "ごめんね @dave".to_owned())]);

    // An older edit handled late changes nothing.
    let older = edited(comment, &bob, "ばか @carol", &[carol.id], first - chrono::Duration::seconds(5));
    ctx.notify.handle_event(older).await.unwrap();
    assert_eq!(bodies(&ctx, &dave).await, [("engagement.mention".to_owned(), "ごめんね @dave".to_owned())]);
    assert_eq!(bodies(&ctx, &owner).await.len(), 2);

    // A newer one wins, also over entries that already quote the one before.
    ctx.notify.handle_event(edited(comment, &bob, "またね", &[], first + chrono::Duration::seconds(5))).await.unwrap();
    assert!(bodies(&ctx, &dave).await.is_empty());
    assert!(bodies(&ctx, &owner).await.iter().any(|(_, body)| body == "またね"));
}

#[tokio::test]
async fn a_late_request_quotes_the_comment_as_it_reads_now() {
    let ctx = start().await;
    let (owner, carol, bob) = (ctx.app.user("owner").await, ctx.app.user("carol").await, ctx.app.user("bob").await);
    let comment = Uuid::now_v7();
    let edited_at = Utc::now() - chrono::Duration::seconds(5);
    ctx.notify.handle_event(edited(comment, &bob, "ごめんね", &[], edited_at)).await.unwrap();

    // Asked for before the edit, handled after it.
    let late = event_at(
        NOTIFY_TOPIC,
        about_comment("engagement.comment", &owner, &bob, comment, "ばか @carol", None),
        edited_at - chrono::Duration::seconds(1),
    );
    ctx.notify.handle_request(&late).await.unwrap();
    assert_eq!(bodies(&ctx, &owner).await, [("engagement.comment".to_owned(), "ごめんね".to_owned())]);
    let mention = request_event(&about_comment("engagement.mention", &carol, &bob, comment, "ばか @carol", None));
    assert!(matches!(
        ctx.notify.handle_request(&mention).await.unwrap(),
        RequestOutcome::Dropped(DropReason::Withdrawn)
    ));
    assert!(bodies(&ctx, &carol).await.is_empty());

    // A request about that version or a later one is left as it is.
    let current = about_comment("engagement.reply", &carol, &bob, comment, "ごめんね!", Some(edited_at));
    ctx.deliver_to_inbox(&current).await;
    assert_eq!(bodies(&ctx, &carol).await, [("engagement.reply".to_owned(), "ごめんね!".to_owned())]);
}

#[tokio::test]
async fn a_queued_push_sends_the_edited_text() {
    let ctx = start_insecure().await;
    let (owner, carol, bob) = (ctx.app.user("owner").await, ctx.app.user("carol").await, ctx.app.user("bob").await);
    let stub = Stub::accepting().await;
    let (owner_device, carol_device) = (
        Subscription::new(&format!("{}/push/owner", stub.url)),
        Subscription::new(&format!("{}/push/carol", stub.url)),
    );
    ctx.register(&owner, &owner_device.request()).await;
    ctx.register(&carol, &carol_device.request()).await;
    let comment = Uuid::now_v7();
    let to_owner =
        ctx.deliver_for_push(&about_comment("engagement.comment", &owner, &bob, comment, "ばか @carol", None)).await;
    let to_carol =
        ctx.deliver_for_push(&about_comment("engagement.mention", &carol, &bob, comment, "ばか @carol", None)).await;

    // Edited before the push service could be reached.
    ctx.notify.handle_event(edited(comment, &bob, "ごめんね", &[], Utc::now())).await.unwrap();
    assert_eq!(ctx.notify.deliver_push(&to_owner).await.unwrap().delivered, 1);
    assert_eq!(ctx.notify.deliver_push(&to_carol).await.unwrap().delivered, 0, "the mention is gone");
    let sent = stub.requests_to("/push/owner");
    assert_eq!(sent.len(), 1);
    assert_eq!(owner_device.decrypt(&sent[0].body)["notification"]["body"], "ごめんね");
    assert!(stub.requests_to("/push/carol").is_empty());
}
