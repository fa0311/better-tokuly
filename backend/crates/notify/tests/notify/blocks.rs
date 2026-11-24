//! Blocks from the social domain: nothing crosses a block, and the blocker's
//! inbox forgets the blocked person.
use chrono::Utc;
use tokuly_notify::consumer::RequestOutcome;
use tokuly_testkit::TestUser;

use crate::common::*;

fn block_event(user: &TestUser, blocked: &TestUser, blocked_now: bool, ago_seconds: i64) -> tokuly_core::events::Event {
    event_at(
        if blocked_now { "social.user.blocked" } else { "social.user.unblocked" },
        serde_json::json!({ "user_id": user.id, "blocked_id": blocked.id }),
        Utc::now() - chrono::Duration::seconds(ago_seconds),
    )
}

fn is_dropped(outcome: &RequestOutcome) -> bool {
    matches!(outcome, RequestOutcome::Dropped(_))
}

#[tokio::test]
async fn nothing_crosses_a_block_and_the_blockers_inbox_forgets_the_blocked() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);
    ctx.deliver_to_inbox(&chat_message(&alice, &bob)).await;
    ctx.deliver_to_inbox(&chat_message(&alice, &carol)).await;
    ctx.deliver_to_inbox(&chat_message(&bob, &alice)).await;

    ctx.notify.handle_event(block_event(&alice, &bob, true, 0)).await.unwrap();

    let inbox = ctx.inbox(&alice).await;
    assert_eq!(inbox.notifications.len(), 1, "only carol's entry is left");
    assert_eq!(inbox.notifications[0].actor.as_option().unwrap().id, carol.id.to_string());
    // Blocks are silent: what the blocked person already has stays.
    assert_eq!(ctx.inbox(&bob).await.notifications.len(), 1);

    // Neither direction gets through any more.
    assert!(is_dropped(&ctx.deliver(&chat_message(&alice, &bob)).await));
    assert!(is_dropped(&ctx.deliver(&chat_message(&bob, &alice)).await));
    assert!(!is_dropped(&ctx.deliver(&chat_message(&alice, &carol)).await));

    ctx.notify.handle_event(block_event(&alice, &bob, false, 0)).await.unwrap();
    assert!(!is_dropped(&ctx.deliver(&chat_message(&alice, &bob)).await));
}

#[tokio::test]
async fn block_events_out_of_order_leave_the_latest_state() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    // Blocked 10 s ago, unblocked 5 s ago; the unblock is handled first.
    ctx.notify.handle_event(block_event(&alice, &bob, false, 5)).await.unwrap();
    ctx.notify.handle_event(block_event(&alice, &bob, true, 10)).await.unwrap();
    assert!(!is_dropped(&ctx.deliver(&chat_message(&alice, &bob)).await));

    // And the other way round: unblocked, then blocked again.
    ctx.notify.handle_event(block_event(&alice, &bob, true, 1)).await.unwrap();
    ctx.notify.handle_event(block_event(&alice, &bob, false, 3)).await.unwrap();
    assert!(is_dropped(&ctx.deliver(&chat_message(&alice, &bob)).await));
}

#[tokio::test]
async fn a_block_handled_while_the_blocked_persons_notifications_arrive_leaves_none() {
    let ctx = start().await;
    for _ in 0..10 {
        let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
        let requests: Vec<_> = (0..4).map(|_| request_event(&chat_message(&alice, &bob))).collect();
        let deliveries = futures::future::join_all(requests.iter().map(|request| ctx.notify.handle_request(request)));
        let (outcomes, blocked) = tokio::join!(deliveries, ctx.notify.handle_event(block_event(&alice, &bob, true, 0)));
        blocked.unwrap();
        for outcome in outcomes {
            outcome.unwrap();
        }
        assert!(ctx.inbox(&alice).await.notifications.is_empty());
    }
}

#[tokio::test]
async fn a_push_queued_before_a_block_or_mute_is_not_sent() {
    let ctx = start_insecure().await;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);
    let stub = Stub::accepting().await;
    let subscription = Subscription::new(&format!("{}/push/laptop", stub.url));
    ctx.register(&alice, &subscription.request()).await;
    let from_bob = ctx.deliver_for_push(&chat_message(&alice, &bob)).await;
    let from_carol = ctx.deliver_for_push(&chat_message(&alice, &carol)).await;

    // Alice blocks Bob and mutes Carol before the push service is reached.
    ctx.notify.handle_event(block_event(&alice, &bob, true, 0)).await.unwrap();
    let _: tokuly_proto::notify::MuteUserResponse = ctx
        .app
        .call(
            "tokuly.notify.v1.PreferencesService/MuteUser",
            Some(&alice),
            &tokuly_proto::notify::MuteUserRequest { user_id: carol.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(ctx.notify.deliver_push(&from_bob).await.unwrap().delivered, 0);
    assert_eq!(ctx.notify.deliver_push(&from_carol).await.unwrap().delivered, 0);
    assert!(stub.requests().is_empty());

    // A block in the other direction holds too.
    let dave = ctx.app.user("dave").await;
    let from_dave = ctx.deliver_for_push(&chat_message(&alice, &dave)).await;
    ctx.notify.handle_event(block_event(&dave, &alice, true, 0)).await.unwrap();
    assert_eq!(ctx.notify.deliver_push(&from_dave).await.unwrap().delivered, 0);
    assert!(stub.requests().is_empty());
}
