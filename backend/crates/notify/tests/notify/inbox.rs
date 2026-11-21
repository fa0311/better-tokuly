//! `InboxService`: reading the inbox (list, filters, unread counts).
use std::time::Duration;

use tokuly_proto::{
    common::PageRequest,
    notify::{mark_read_request::Target, *},
    realtime::server_frame::Body,
};
use tokuly_testkit::TestUser;
use uuid::Uuid;

use crate::common::*;

const WAIT: Duration = Duration::from_secs(2);

async fn list(
    ctx: &Ctx,
    user: &TestUser,
    request: ListNotificationsRequest,
) -> ListNotificationsResponse {
    ctx.app
        .call(INBOX_LIST, Some(user), &request)
        .await
        .unwrap()
}

#[tokio::test]
async fn every_inbox_rpc_requires_sign_in() {
    let ctx = start().await;
    let app = &ctx.app;
    let reasons = [
        app.call::<_, ListNotificationsResponse>(
            INBOX_LIST,
            None,
            &ListNotificationsRequest::default(),
        )
        .await
        .unwrap_err(),
        app.call::<_, GetUnreadCountResponse>(
            INBOX_UNREAD,
            None,
            &GetUnreadCountRequest::default(),
        )
        .await
        .unwrap_err(),
        app.call::<_, MarkReadResponse>(
            INBOX_MARK_READ,
            None,
            &MarkReadRequest {
                target: Some(Target::All(true)),
                ..Default::default()
            },
        )
        .await
        .unwrap_err(),
        app.call::<_, DeleteNotificationsResponse>(
            INBOX_DELETE,
            None,
            &DeleteNotificationsRequest {
                notification_ids: vec![Uuid::now_v7().to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap_err(),
        app.call::<_, ClearNotificationsResponse>(
            INBOX_CLEAR,
            None,
            &ClearNotificationsRequest::default(),
        )
        .await
        .unwrap_err(),
    ];
    for err in reasons {
        assert_eq!((err.status, err.reason.as_str()), (401, "SIGN_IN_REQUIRED"));
    }
}

#[tokio::test]
async fn list_is_newest_first_and_paginates_with_a_cursor() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    fill(
        &ctx,
        &alice,
        &[
            ("chat.message", "1"),
            ("live.started", "2"),
            ("chat.message", "3"),
            ("social.buzz", "4"),
            ("chat.message", "5"),
        ],
    )
    .await;

    let page = |cursor: &str| PageRequest {
        limit: 2,
        cursor: cursor.to_owned(),
        ..Default::default()
    };
    let first = list(
        &ctx,
        &alice,
        ListNotificationsRequest {
            page: page("").into(),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(titles(&first), ["5", "4"]);
    let info = first.page.as_option().unwrap();
    assert!(info.has_more && !info.next_cursor.is_empty());
    assert_eq!(
        unread_of(&first),
        5,
        "the unread count is about the whole inbox, not the page"
    );

    let second = list(
        &ctx,
        &alice,
        ListNotificationsRequest {
            page: page(&info.next_cursor).into(),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(titles(&second), ["3", "2"]);
    let third = list(
        &ctx,
        &alice,
        ListNotificationsRequest {
            page: page(&second.page.as_option().unwrap().next_cursor).into(),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(titles(&third), ["1"]);
    let info = third.page.as_option().unwrap();
    assert!(!info.has_more && info.next_cursor.is_empty());

    let err = ctx
        .app
        .call::<_, ListNotificationsResponse>(
            INBOX_LIST,
            Some(&alice),
            &ListNotificationsRequest {
                page: page("garbage!").into(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.reason, "INVALID_CURSOR");
}

#[tokio::test]
async fn list_filters_by_category_and_unread() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let ids_ = fill(
        &ctx,
        &alice,
        &[
            ("chat.message", "c1"),
            ("live.started", "l1"),
            ("chat.message", "c2"),
            ("note.reply", "n1"),
        ],
    )
    .await;
    mark_read(&ctx, &alice, ids(&[&ids_[0]]), "").await;

    let chat = list(
        &ctx,
        &alice,
        ListNotificationsRequest {
            category: "chat".into(),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(titles(&chat), ["c2", "c1"]);
    let notes = list(
        &ctx,
        &alice,
        ListNotificationsRequest {
            category: "notes".into(),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(
        titles(&notes),
        ["n1"],
        "`note.*` kinds belong to the notes category"
    );
    let unread_only = list(
        &ctx,
        &alice,
        ListNotificationsRequest {
            unread_only: true,
            ..Default::default()
        },
    )
    .await;
    assert_eq!(titles(&unread_only), ["n1", "c2", "l1"]);
    let unread_chat = list(
        &ctx,
        &alice,
        ListNotificationsRequest {
            category: "chat".into(),
            unread_only: true,
            ..Default::default()
        },
    )
    .await;
    assert_eq!(titles(&unread_chat), ["c2"]);
    let read_entry = chat.notifications.iter().find(|n| n.title == "c1").unwrap();
    assert!(read_entry.read && read_entry.read_at.as_option().is_some());

    let err = ctx
        .app
        .call::<_, ListNotificationsResponse>(
            INBOX_LIST,
            Some(&alice),
            &ListNotificationsRequest {
                category: "nope".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        (err.code.as_str(), err.reason.as_str()),
        ("invalid_argument", "CATEGORY_UNKNOWN")
    );
}

#[tokio::test]
async fn unread_count_is_total_and_per_category() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    assert_eq!(unread(&ctx, &alice).await.total, 0);
    let ids_ = fill(
        &ctx,
        &alice,
        &[
            ("chat.message", "a"),
            ("chat.message", "b"),
            ("live.started", "c"),
            ("wallet.gift", "d"),
        ],
    )
    .await;

    let count = unread(&ctx, &alice).await;
    assert_eq!(count.total, 4);
    assert_eq!(
        (
            count.by_category.get("chat"),
            count.by_category.get("live"),
            count.by_category.get("wallet")
        ),
        (Some(&2), Some(&1), Some(&1))
    );
    assert_eq!(
        count.by_category.len(),
        3,
        "categories without unread entries are absent"
    );

    mark_read(&ctx, &alice, ids(&[&ids_[0], &ids_[2]]), "").await;
    let count = unread(&ctx, &alice).await;
    assert_eq!(
        (
            count.total,
            count.by_category.get("chat"),
            count.by_category.get("live")
        ),
        (2, Some(&1), None)
    );
    assert_eq!(unread(&ctx, &bob).await.total, 0, "counts are per user");
}

#[tokio::test]
async fn bulk_changes_tell_other_devices_to_refetch() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    // More entries than one realtime event lists.
    sqlx::query(
        "INSERT INTO notifications (user_id, kind, category, title_ja, title_en, body_ja, body_en, path)
         SELECT $1, 'feed.news', 'feed', 'ニュース', 'News', '', '', '/news' FROM generate_series(1, 230)",
    )
    .bind(alice.id)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
    let mut frames = ctx.app.listen_user(alice.id).await;

    let response = mark_read(&ctx, &alice, Target::All(true), "").await;
    assert_eq!(response.marked_count, 230);
    let Some(Body::Notify(event)) = frames.next(WAIT).await.unwrap().body else {
        panic!("expected a notify frame")
    };
    let Some(notify_event::Event::Read(read)) = event.event else {
        panic!("expected `read`")
    };
    assert!(read.bulk && read.notification_ids.is_empty());

    // The page size is capped.
    let big = list(
        &ctx,
        &alice,
        ListNotificationsRequest {
            page: PageRequest {
                limit: 1000,
                ..Default::default()
            }
            .into(),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(big.notifications.len(), 100);
}
