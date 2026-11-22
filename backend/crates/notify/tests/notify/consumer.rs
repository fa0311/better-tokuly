//! The event consumer, driven with constructed events (no relay runs in tests):
//! how a request becomes an inbox entry, a realtime event and a push job.
use std::time::Duration;

use chrono::Utc;
use tokuly_core::outbound::{Localized, NOTIFY_TOPIC, NotificationRequest};
use tokuly_notify::consumer::{DropReason, PushDecision, PushSkip, RequestOutcome};
use tokuly_proto::{common::SubjectType, notify::*};
use tokuly_testkit::TestUser;
use uuid::Uuid;

use crate::common::*;

const WAIT: Duration = Duration::from_secs(2);
const QUIET: Duration = Duration::from_millis(300);

#[tokio::test]
async fn a_request_becomes_an_inbox_entry_and_a_realtime_event() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let mut frames = ctx.app.listen_user(alice.id).await;
    let subject_id = Uuid::now_v7();
    let request = chat_message(&alice, &bob)
        .subject(SubjectType::SUBJECT_TYPE_CHAT_MESSAGE, subject_id)
        .image("https://cdn.example/preview.webp")
        .data(serde_json::json!({ "conversation_id": "42" }));

    let id = ctx.deliver_to_inbox(&request).await;

    let inbox = ctx.inbox(&alice).await;
    assert_eq!(inbox.notifications.len(), 1);
    let entry = &inbox.notifications[0];
    assert_eq!(
        (
            entry.id.as_str(),
            entry.kind.as_str(),
            entry.category.as_str()
        ),
        (id.as_str(), "chat.message", "chat")
    );
    assert_eq!(
        (entry.title.as_str(), entry.body.as_str()),
        ("新着メッセージ", "やっほー！"),
        "accounts read Japanese by default"
    );
    assert_eq!(entry.actor.as_option().unwrap().id, bob.id.to_string());
    assert_eq!(entry.actor.as_option().unwrap().display_name, "bob");
    assert_eq!(
        (entry.path.as_str(), entry.image_url.as_str()),
        ("/chat/42", "https://cdn.example/preview.webp")
    );
    let subject = entry.subject.as_option().unwrap();
    assert_eq!(
        (subject.r#type.as_known(), subject.id.as_str()),
        (
            Some(SubjectType::SUBJECT_TYPE_CHAT_MESSAGE),
            subject_id.to_string().as_str()
        )
    );
    assert_eq!(entry.data_json, r#"{"conversation_id":"42"}"#);
    assert!(!entry.read && entry.group_count == 1 && entry.occurred_at.as_option().is_some());
    let unread = inbox.unread.as_option().unwrap();
    assert_eq!(
        (unread.total, unread.by_category.get("chat").copied()),
        (1, Some(1))
    );

    let frame = frames.next(WAIT).await.expect("a realtime frame");
    let Some(event) = frame.event::<NotifyEvent>() else {
        panic!("expected a notify frame")
    };
    let Some(notify_event::Event::Received(received)) = event.event else {
        panic!("expected `received`")
    };
    assert_eq!(received.notification.as_option().unwrap().id, id);
    assert_eq!(
        received
            .notification
            .as_option()
            .unwrap()
            .actor
            .as_option()
            .unwrap()
            .id,
        bob.id.to_string()
    );
    assert_eq!(received.unread.as_option().unwrap().total, 1);

    // The other user's inbox is untouched.
    assert!(ctx.inbox(&bob).await.notifications.is_empty());
}

#[tokio::test]
async fn a_redelivered_event_changes_nothing() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.register_browser(&alice).await;
    let event = request_event(&chat_message(&alice, &bob).collapse("chat:42"));

    let first = ctx.notify.handle_request(&event).await.unwrap();
    assert!(
        matches!(
            first,
            RequestOutcome::Handled {
                notification_id: Some(_),
                push: PushDecision::Queued(_)
            }
        ),
        "{first:?}"
    );

    let mut frames = ctx.app.listen_user(alice.id).await;
    for _ in 0..3 {
        let again = ctx.notify.handle_request(&event).await.unwrap();
        assert!(matches!(again, RequestOutcome::Duplicate), "{again:?}");
    }
    // Through the consumer's entry point too.
    ctx.notify.handle_event(event).await.unwrap();

    let inbox = ctx.inbox(&alice).await;
    assert_eq!((inbox.notifications.len(), unread_of(&inbox)), (1, 1));
    assert_eq!(
        inbox.notifications[0].group_count, 1,
        "the duplicate was not folded into the entry"
    );
    assert!(
        frames.next(QUIET).await.is_none(),
        "no second realtime event"
    );
}

#[tokio::test]
async fn text_follows_the_recipients_language() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    sqlx::query("UPDATE users SET locale = 'en-US' WHERE id = $1")
        .bind(alice.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let mut frames = ctx.app.listen_user(alice.id).await;

    ctx.deliver_to_inbox(&chat_message(&alice, &bob)).await;

    let Some(event) = frames.next(WAIT).await.unwrap().event::<NotifyEvent>() else {
        panic!("expected a notify frame")
    };
    let Some(notify_event::Event::Received(received)) = event.event else {
        panic!("expected `received`")
    };
    assert_eq!(
        received.notification.as_option().unwrap().title,
        "New message"
    );
    let entry = &ctx.inbox(&alice).await.notifications[0];
    assert_eq!(
        (entry.title.as_str(), entry.body.as_str()),
        ("New message", "Hi there!")
    );

    // Texts are stored in both languages: switching back re-reads the same entry in Japanese.
    sqlx::query("UPDATE users SET locale = 'ja' WHERE id = $1")
        .bind(alice.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(
        ctx.inbox(&alice).await.notifications[0].title,
        "新着メッセージ"
    );
}

#[tokio::test]
async fn a_collapse_key_groups_events_into_one_entry() {
    let ctx = start().await;
    let (alice, bob, carol) = (
        ctx.app.user("alice").await,
        ctx.app.user("bob").await,
        ctx.app.user("carol").await,
    );
    let message = |from: &TestUser, text: &str| {
        NotificationRequest::new(
            alice.id,
            "chat.message",
            Localized::same(from.handle.clone()),
            Localized::same(text),
            "/chat/42",
        )
        .actor(from.id)
        .collapse("chat:42")
    };

    let first = ctx.deliver_to_inbox(&message(&bob, "one")).await;
    let second = ctx.deliver_to_inbox(&message(&bob, "two")).await;
    let third = ctx.deliver_to_inbox(&message(&carol, "three")).await;
    assert!(first == second && second == third, "the entry keeps its ID");
    // A different key and no key at all are separate entries.
    ctx.deliver_to_inbox(&message(&bob, "elsewhere").collapse("chat:99"))
        .await;
    let mut loose = message(&bob, "loose");
    loose.collapse_key = None;
    ctx.deliver_to_inbox(&loose).await;

    let inbox = ctx.inbox(&alice).await;
    assert_eq!(
        (inbox.notifications.len(), unread_of(&inbox)),
        (3, 3),
        "a grouped entry counts once"
    );
    let grouped = inbox.notifications.iter().find(|n| n.id == first).unwrap();
    assert_eq!(
        (
            grouped.body.as_str(),
            grouped.group_count,
            grouped.collapse_key.as_str()
        ),
        ("three", 3, "chat:42")
    );
    assert_eq!(
        grouped.actor.as_option().unwrap().id,
        carol.id.to_string(),
        "the latest event wins"
    );

    // Once read, the next event makes it unread again and restarts the count.
    let _: MarkReadResponse = ctx
        .app
        .call(
            "tokuly.notify.v1.InboxService/MarkRead",
            Some(&alice),
            &MarkReadRequest {
                target: Some(mark_read_request::Target::All(true)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    ctx.deliver_to_inbox(&message(&bob, "four")).await;
    let inbox = ctx.inbox(&alice).await;
    let grouped = &inbox.notifications[0];
    assert_eq!(grouped.id, first, "the bumped entry is back on top");
    assert_eq!(
        (grouped.body.as_str(), grouped.group_count, grouped.read),
        ("four", 1, false)
    );
    assert_eq!(unread_of(&inbox), 1);
}

#[tokio::test]
async fn an_older_event_does_not_overwrite_a_newer_one() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.register_browser(&alice).await;
    let message = |text: &str| {
        NotificationRequest::new(
            alice.id,
            "chat.message",
            Localized::same("bob"),
            Localized::same(text),
            "/chat/7",
        )
        .actor(bob.id)
        .collapse("chat:7")
    };
    let now = Utc::now();

    // Events of one conversation handled out of order: the later one first.
    let newer = event_at(NOTIFY_TOPIC, message("newer"), now);
    let older = event_at(
        NOTIFY_TOPIC,
        message("older"),
        now - chrono::Duration::seconds(5),
    );
    assert!(matches!(
        ctx.notify.handle_request(&newer).await.unwrap(),
        RequestOutcome::Handled { .. }
    ));
    let outcome = ctx.notify.handle_request(&older).await.unwrap();
    assert_eq!(
        dropped(&outcome),
        Some(DropReason::Superseded),
        "{outcome:?}"
    );

    let inbox = ctx.inbox(&alice).await;
    assert_eq!(
        (
            inbox.notifications.len(),
            inbox.notifications[0].body.as_str()
        ),
        (1, "newer")
    );
    // The stale event is still remembered as handled.
    assert!(matches!(
        ctx.notify.handle_request(&older).await.unwrap(),
        RequestOutcome::Duplicate
    ));
}

#[tokio::test]
async fn the_producer_chooses_inbox_only_or_push_only() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.register_browser(&alice).await;
    ctx.deliver_to_inbox(&chat_message(&alice, &bob)).await;

    // Push only (a buzz): no inbox entry, badge = what is already unread.
    let buzz = NotificationRequest::new(
        alice.id,
        "social.buzz",
        Localized::new("ブルブル！", "Buzz!"),
        Localized::same("bob"),
        "/map",
    )
    .actor(bob.id)
    .push_only()
    .collapse("buzz:bob");
    let outcome = ctx.deliver(&buzz).await;
    let RequestOutcome::Handled {
        notification_id: None,
        push: PushDecision::Queued(job),
    } = outcome
    else {
        panic!("{outcome:?}")
    };
    assert_eq!(
        (
            job.user_id,
            job.kind.as_str(),
            job.category.as_str(),
            job.badge
        ),
        (alice.id, "social.buzz", "social", 1)
    );
    assert_eq!(
        (
            job.title.ja.as_str(),
            job.title.en.as_str(),
            job.path.as_str()
        ),
        ("ブルブル！", "Buzz!", "/map")
    );
    assert_eq!(job.collapse_key.as_deref(), Some("buzz:bob"));
    assert_eq!(ctx.inbox(&alice).await.notifications.len(), 1);

    // Inbox only: recorded, never pushed.
    let low = NotificationRequest::new(
        alice.id,
        "feed.digest",
        Localized::same("今週のまとめ"),
        Localized::same("…"),
        "/feed",
    )
    .inbox_only();
    let outcome = ctx.deliver(&low).await;
    assert!(
        matches!(
            outcome,
            RequestOutcome::Handled {
                notification_id: Some(_),
                push: PushDecision::Skipped(PushSkip::NotRequested)
            }
        ),
        "{outcome:?}"
    );

    // A push for a recorded notification carries its ID and counts it in the badge.
    let job = ctx.deliver_for_push(&chat_message(&alice, &bob)).await;
    assert_eq!(job.badge, 3);
    assert!(
        ctx.inbox(&alice)
            .await
            .notifications
            .iter()
            .any(|n| Some(n.id.clone()) == job.notification_id.map(|id| id.to_string()))
    );
}

#[tokio::test]
async fn users_without_devices_get_no_push_job() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let outcome = ctx.deliver(&chat_message(&alice, &bob)).await;
    assert!(
        matches!(
            outcome,
            RequestOutcome::Handled {
                notification_id: Some(_),
                push: PushDecision::Skipped(PushSkip::NoDevices)
            }
        ),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn requests_that_cannot_be_delivered_are_dropped_quietly() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);

    // About the recipient's own action.
    let own = chat_message(&alice, &alice);
    assert_eq!(
        dropped(&ctx.deliver(&own).await),
        Some(DropReason::OwnAction)
    );
    // To an account that does not exist, or was deleted.
    let mut ghost = chat_message(&alice, &bob);
    ghost.user_id = Uuid::now_v7();
    assert_eq!(
        dropped(&ctx.deliver(&ghost).await),
        Some(DropReason::RecipientGone)
    );
    let gone = ctx.app.user("gone").await;
    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1")
        .bind(gone.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(
        dropped(&ctx.deliver(&chat_message(&gone, &bob)).await),
        Some(DropReason::RecipientGone)
    );
    // Malformed.
    let mut bad_kind = chat_message(&alice, &bob);
    bad_kind.kind = "Not A Kind".into();
    assert_eq!(
        dropped(&ctx.deliver(&bad_kind).await),
        Some(DropReason::Invalid)
    );
    let mut untitled = chat_message(&alice, &bob);
    untitled.title = Localized::same("");
    assert_eq!(
        dropped(&ctx.deliver(&untitled).await),
        Some(DropReason::Invalid)
    );
    assert!(ctx.inbox(&alice).await.notifications.is_empty());

    // A payload that is not a notification request fails loudly, so the
    // event is retried and eventually parked instead of vanishing.
    assert!(
        ctx.notify
            .handle_event(event(NOTIFY_TOPIC, serde_json::json!({ "nonsense": true })))
            .await
            .is_err()
    );
    // Topics this consumer does not know are ignored.
    ctx.notify
        .handle_event(event("chat.message.sent", serde_json::json!({})))
        .await
        .unwrap();
}

#[tokio::test]
async fn unsafe_parts_of_a_request_are_neutralized() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let mut request = chat_message(&alice, &bob);
    request.path = "https://evil.example/phish".into();
    request.image_url = Some("javascript:alert(1)".into());
    request.title = Localized::new("", "Only English");
    request.kind = "mystery.thing".into();
    ctx.deliver_to_inbox(&request).await;

    let entry = &ctx.inbox(&alice).await.notifications[0];
    assert_eq!(entry.path, "/", "only in-app paths are kept");
    assert!(entry.image_url.is_empty());
    assert_eq!(
        entry.title, "Only English",
        "a missing language falls back to the other"
    );
    assert_eq!(
        entry.category, "system",
        "unknown kind prefixes land in the system category"
    );
}
