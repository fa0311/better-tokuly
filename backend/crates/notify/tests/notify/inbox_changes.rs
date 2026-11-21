//! `InboxService`: changing the inbox (mark read, delete, clear).
use std::time::Duration;

use tokuly_proto::{
    notify::{mark_read_request::Target, *},
    realtime::server_frame::Body,
};
use uuid::Uuid;

use crate::common::*;

const WAIT: Duration = Duration::from_secs(2);

#[tokio::test]
async fn mark_read_by_ids_touches_only_the_callers_entries() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let alices = fill(
        &ctx,
        &alice,
        &[
            ("chat.message", "a1"),
            ("chat.message", "a2"),
            ("chat.message", "a3"),
        ],
    )
    .await;
    let bobs = fill(&ctx, &bob, &[("chat.message", "b1")]).await;
    let mut frames = ctx.app.listen_user(alice.id).await;

    // Alice names one of her own entries and one of Bob's.
    let response = mark_read(&ctx, &alice, ids(&[&alices[0], &bobs[0]]), "").await;
    assert_eq!(
        (
            response.marked_count,
            response.unread.as_option().unwrap().total
        ),
        (1, 2)
    );
    assert_eq!(
        unread(&ctx, &bob).await.total,
        1,
        "someone else's entry is out of reach"
    );

    // Her other devices hear about it.
    let Some(Body::Notify(event)) = frames.next(WAIT).await.unwrap().body else {
        panic!("expected a notify frame")
    };
    let Some(notify_event::Event::Read(read)) = event.event else {
        panic!("expected `read`")
    };
    assert_eq!(read.notification_ids, [alices[0].clone()]);
    assert!(!read.bulk);
    assert_eq!(read.unread.as_option().unwrap().total, 2);

    // Marking again changes nothing and stays quiet.
    let response = mark_read(&ctx, &alice, ids(&[&alices[0]]), "").await;
    assert_eq!(response.marked_count, 0);
    assert!(frames.next(Duration::from_millis(300)).await.is_none());
}

#[tokio::test]
async fn mark_read_up_to_an_entry_leaves_newer_ones_unread() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let entries = fill(
        &ctx,
        &alice,
        &[
            ("chat.message", "1"),
            ("live.started", "2"),
            ("chat.message", "3"),
            ("chat.message", "4"),
        ],
    )
    .await;

    // Only the chat entries up to "3".
    let response = mark_read(&ctx, &alice, Target::UpToId(entries[2].clone()), "chat").await;
    assert_eq!(response.marked_count, 2);
    let unread_titles = |response: ListNotificationsResponse| {
        response
            .notifications
            .iter()
            .filter(|n| !n.read)
            .map(|n| n.title.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(unread_titles(ctx.inbox(&alice).await), ["4", "2"]);

    // Every category up to "3": "2" goes too, "4" arrived later and stays.
    let response = mark_read(&ctx, &alice, Target::UpToId(entries[2].clone()), "").await;
    assert_eq!(
        (
            response.marked_count,
            response.unread.as_option().unwrap().total
        ),
        (1, 1)
    );
    assert_eq!(unread_titles(ctx.inbox(&alice).await), ["4"]);

    // The pivot must be in the caller's own inbox.
    let err = ctx
        .app
        .call::<_, MarkReadResponse>(
            INBOX_MARK_READ,
            Some(&bob),
            &MarkReadRequest {
                target: Some(Target::UpToId(entries[2].clone())),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        (err.code.as_str(), err.reason.as_str()),
        ("not_found", "NOTIFICATION_NOT_FOUND")
    );
    assert_eq!(unread(&ctx, &alice).await.total, 1);
}

#[tokio::test]
async fn mark_all_read_optionally_by_category() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    fill(
        &ctx,
        &alice,
        &[
            ("chat.message", "1"),
            ("live.started", "2"),
            ("chat.message", "3"),
        ],
    )
    .await;

    let response = mark_read(&ctx, &alice, Target::All(true), "chat").await;
    assert_eq!(
        (
            response.marked_count,
            response.unread.as_option().unwrap().total
        ),
        (2, 1)
    );
    let response = mark_read(&ctx, &alice, Target::All(true), "").await;
    assert_eq!(
        (
            response.marked_count,
            response.unread.as_option().unwrap().total
        ),
        (1, 0)
    );
    assert!(ctx.inbox(&alice).await.notifications.iter().all(|n| n.read));
}

#[tokio::test]
async fn mark_read_validates_its_target() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let call = |request: MarkReadRequest| {
        let (app, alice) = (&ctx.app, &alice);
        async move {
            app.call::<_, MarkReadResponse>(INBOX_MARK_READ, Some(alice), &request)
                .await
                .unwrap_err()
                .reason
        }
    };
    assert_eq!(call(MarkReadRequest::default()).await, "TARGET_REQUIRED");
    assert_eq!(
        call(MarkReadRequest {
            target: Some(Target::All(false)),
            ..Default::default()
        })
        .await,
        "TARGET_REQUIRED"
    );
    assert_eq!(
        call(MarkReadRequest {
            target: Some(Target::from(NotificationIds::default())),
            ..Default::default()
        })
        .await,
        "TARGET_REQUIRED"
    );
    let many = NotificationIds {
        ids: (0..201).map(|_| Uuid::now_v7().to_string()).collect(),
        ..Default::default()
    };
    assert_eq!(
        call(MarkReadRequest {
            target: Some(Target::from(many)),
            ..Default::default()
        })
        .await,
        "TOO_MANY_IDS"
    );
    let bad = NotificationIds {
        ids: vec!["not-an-id".into()],
        ..Default::default()
    };
    assert_eq!(
        call(MarkReadRequest {
            target: Some(Target::from(bad)),
            ..Default::default()
        })
        .await,
        "INVALID_ID"
    );
    assert_eq!(
        call(MarkReadRequest {
            target: Some(Target::UpToId("nope".into())),
            ..Default::default()
        })
        .await,
        "INVALID_ID"
    );
    assert_eq!(
        call(MarkReadRequest {
            target: Some(Target::All(true)),
            category: "nope".into(),
            ..Default::default()
        })
        .await,
        "CATEGORY_UNKNOWN"
    );
}

#[tokio::test]
async fn delete_removes_only_the_callers_entries() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let alices = fill(
        &ctx,
        &alice,
        &[("chat.message", "a1"), ("chat.message", "a2")],
    )
    .await;
    let bobs = fill(&ctx, &bob, &[("chat.message", "b1")]).await;
    let mut frames = ctx.app.listen_user(alice.id).await;

    let request = DeleteNotificationsRequest {
        notification_ids: vec![
            alices[0].clone(),
            bobs[0].clone(),
            Uuid::now_v7().to_string(),
        ],
        ..Default::default()
    };
    let response: DeleteNotificationsResponse = ctx
        .app
        .call(INBOX_DELETE, Some(&alice), &request)
        .await
        .unwrap();
    assert_eq!(
        (
            response.deleted_count,
            response.unread.as_option().unwrap().total
        ),
        (1, 1)
    );
    assert_eq!(titles(&ctx.inbox(&alice).await), ["a2"]);
    assert_eq!(
        titles(&ctx.inbox(&bob).await),
        ["b1"],
        "someone else's entry is out of reach"
    );

    let Some(Body::Notify(event)) = frames.next(WAIT).await.unwrap().body else {
        panic!("expected a notify frame")
    };
    let Some(notify_event::Event::Removed(removed)) = event.event else {
        panic!("expected `removed`")
    };
    assert_eq!(removed.notification_ids, [alices[0].clone()]);
    assert_eq!(removed.unread.as_option().unwrap().total, 1);

    let reason = |ids: Vec<String>| {
        let (app, alice) = (&ctx.app, &alice);
        async move {
            app.call::<_, DeleteNotificationsResponse>(
                INBOX_DELETE,
                Some(alice),
                &DeleteNotificationsRequest {
                    notification_ids: ids,
                    ..Default::default()
                },
            )
            .await
            .unwrap_err()
            .reason
        }
    };
    assert_eq!(reason(vec![]).await, "IDS_REQUIRED");
    assert_eq!(reason(vec!["nope".into()]).await, "INVALID_ID");
    assert_eq!(
        reason((0..201).map(|_| Uuid::now_v7().to_string()).collect()).await,
        "TOO_MANY_IDS"
    );
}

#[tokio::test]
async fn clear_empties_the_inbox_by_category_or_read_state() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let entries = fill(
        &ctx,
        &alice,
        &[
            ("chat.message", "c1"),
            ("chat.message", "c2"),
            ("live.started", "l1"),
            ("wallet.gift", "w1"),
        ],
    )
    .await;
    fill(&ctx, &bob, &[("chat.message", "b1")]).await;
    mark_read(&ctx, &alice, ids(&[&entries[0], &entries[2]]), "").await;
    let clear = |request: ClearNotificationsRequest| {
        let (app, alice) = (&ctx.app, &alice);
        async move {
            app.call::<_, ClearNotificationsResponse>(INBOX_CLEAR, Some(alice), &request)
                .await
        }
    };

    // Read chat entries only.
    let response = clear(ClearNotificationsRequest {
        category: "chat".into(),
        read_only: true,
        ..Default::default()
    })
    .await
    .unwrap();
    assert_eq!(response.deleted_count, 1);
    assert_eq!(titles(&ctx.inbox(&alice).await), ["w1", "l1", "c2"]);
    // Everything read.
    let response = clear(ClearNotificationsRequest {
        read_only: true,
        ..Default::default()
    })
    .await
    .unwrap();
    assert_eq!(response.deleted_count, 1);
    assert_eq!(titles(&ctx.inbox(&alice).await), ["w1", "c2"]);
    // One category, read or not.
    let response = clear(ClearNotificationsRequest {
        category: "wallet".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    assert_eq!(
        (
            response.deleted_count,
            response.unread.as_option().unwrap().total
        ),
        (1, 1)
    );
    // Everything.
    let mut frames = ctx.app.listen_user(alice.id).await;
    let response = clear(ClearNotificationsRequest::default()).await.unwrap();
    assert_eq!(
        (
            response.deleted_count,
            response
                .unread
                .as_option()
                .map(|u| u.total)
                .unwrap_or_default()
        ),
        (1, 0)
    );
    assert!(ctx.inbox(&alice).await.notifications.is_empty());
    let Some(Body::Notify(event)) = frames.next(WAIT).await.unwrap().body else {
        panic!("expected a notify frame")
    };
    assert!(matches!(event.event, Some(notify_event::Event::Removed(_))));
    // An empty inbox clears to nothing, quietly.
    assert_eq!(
        clear(ClearNotificationsRequest::default())
            .await
            .unwrap()
            .deleted_count,
        0
    );
    assert!(frames.next(Duration::from_millis(300)).await.is_none());

    assert_eq!(
        titles(&ctx.inbox(&bob).await),
        ["b1"],
        "other inboxes are untouched"
    );
    assert_eq!(
        clear(ClearNotificationsRequest {
            category: "nope".into(),
            ..Default::default()
        })
        .await
        .unwrap_err()
        .reason,
        "CATEGORY_UNKNOWN"
    );
}
