//! The whole path with the real workers: another domain's transaction →
//! outbox → event stream → notify consumer → push job → push service.
//!
//! Its own test binary (so its own NATS namespace): the workers started here
//! must not pick up jobs that other tests only queue.
use std::time::{Duration, Instant};

use tokuly_core::outbound;
use tokuly_notify::events::{FRIEND_PREFS_CHANGED_TOPIC, FriendPrefsChanged};
use tokuly_proto::notify::*;

#[allow(dead_code)]
#[path = "notify/common.rs"]
mod common;
use common::*;

/// Polls until `check` holds, failing the test after 30 seconds.
async fn eventually(what: &str, mut check: impl AsyncFnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !check().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_notification_requested_by_another_domain_reaches_the_inbox_and_the_browser() {
    let ctx = start_insecure().await;
    let core = &ctx.app.core;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);
    let push_service = Stub::accepting().await;
    let subscription = Subscription::new(&format!("{}/push/alice", push_service.url));
    ctx.register(&alice, &subscription.request()).await;
    let mut frames = ctx.app.listen_user(alice.id).await;

    // What a worker process runs.
    ctx.notify.spawn_workers();
    tokio::spawn(tokuly_core::events::run_outbox_relay(core.db.clone(), core.bus.clone(), core.shutdown.clone()));

    // Another domain, inside its own transaction.
    let mut tx = core.db.begin().await.unwrap();
    outbound::notify(&mut *tx, &chat_message(&alice, &bob).collapse("chat:42")).await.unwrap();
    tx.commit().await.unwrap();

    eventually("the push to reach the push service", async || !push_service.requests_to("/push/alice").is_empty())
        .await;
    let message = subscription.decrypt(&push_service.requests_to("/push/alice")[0].body);
    assert_eq!(message["notification"]["title"], "新着メッセージ");
    assert_eq!(message["app_badge"], 1);

    let inbox = ctx.inbox(&alice).await;
    assert_eq!((inbox.notifications.len(), unread_of(&inbox)), (1, 1));
    assert_eq!(message["notification"]["data"]["id"], inbox.notifications[0].id);
    let Some(event) = frames.next(Duration::from_secs(5)).await.expect("a realtime frame").event::<NotifyEvent>()
    else {
        panic!("expected a notify frame")
    };
    assert!(matches!(event.event, Some(notify_event::Event::Received(_))));

    // The consumer also follows the social domain's friend preferences.
    core.emit(
        FRIEND_PREFS_CHANGED_TOPIC,
        Some(alice.id),
        &FriendPrefsChanged { user_id: alice.id, friend_id: carol.id, muted: true },
    )
    .await
    .unwrap();
    eventually("the friend mute to be mirrored", async || {
        let muted: ListMutedUsersResponse = ctx
            .app
            .call("tokuly.notify.v1.PreferencesService/ListMutedUsers", Some(&alice), &ListMutedUsersRequest::default())
            .await
            .unwrap();
        muted.users.len() == 1
    })
    .await;

    // Nothing was pushed twice along the way.
    assert_eq!(push_service.requests().len(), 1);
    core.shutdown.cancel();
}
