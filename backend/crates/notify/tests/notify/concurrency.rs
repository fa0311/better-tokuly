//! Bursts from one account against every path that holds a transaction or
//! an advisory lock. The test pool has 5 connections and waits 10 s for one:
//! a locked section that asks the pool for a second connection would stall
//! the burst until that timeout.
use std::time::{Duration, Instant};

use futures::future::join_all;
use tokuly_proto::{common::SubjectType, notify::*};
use uuid::Uuid;

use crate::common::*;

#[tokio::test]
async fn a_burst_from_one_account_does_not_starve_the_pool() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.register_browser(&alice).await;
    let message_id = Uuid::now_v7();
    let requests: Vec<_> = (0..10)
        .map(|_| request_event(&chat_message(&alice, &bob).subject(SubjectType::SUBJECT_TYPE_CHAT_MESSAGE, message_id)))
        .collect();
    let started = Instant::now();

    let consumer = join_all(requests.iter().map(|r| ctx.notify.handle_request(r)));
    let changes = join_all((0..5).map(|i| {
        let event = if i % 2 == 0 {
            event("chat.message.deleted", serde_json::json!({ "message_id": message_id }))
        } else {
            event("social.user.blocked", serde_json::json!({ "user_id": alice.id, "blocked_id": bob.id }))
        };
        ctx.notify.handle_event(event)
    }));
    let mute = MuteUserRequest { user_id: bob.id.to_string(), ..Default::default() };
    let mutes = join_all((0..5).map(|_| {
        ctx.app.call::<_, MuteUserResponse>("tokuly.notify.v1.PreferencesService/MuteUser", Some(&alice), &mute)
    }));
    let push_off = toggle("chat", None, Some(false));
    let preferences = join_all((0..5).map(|_| {
        ctx.app.call::<_, UpdatePreferencesResponse>(
            "tokuly.notify.v1.PreferencesService/UpdatePreferences",
            Some(&alice),
            &push_off,
        )
    }));
    let devices = join_all((0..5).map(|_| async {
        let subscription =
            Subscription::new(&format!("https://fcm.googleapis.com/fcm/send/{}", Uuid::now_v7().simple()));
        ctx.app
            .call::<_, RegisterDeviceResponse>(
                "tokuly.notify.v1.PushService/RegisterDevice",
                Some(&alice),
                &subscription.request(),
            )
            .await
    }));
    let (consumer, changes, mutes, preferences, devices) = tokio::join!(consumer, changes, mutes, preferences, devices);

    for outcome in consumer {
        outcome.unwrap();
    }
    for change in changes {
        change.unwrap();
    }
    for response in mutes {
        response.unwrap();
    }
    for response in preferences {
        response.unwrap();
    }
    // Registrations of one session replace each other, in turn.
    for response in devices {
        response.unwrap();
    }
    assert!(started.elapsed() < Duration::from_secs(8), "the burst took {:?}", started.elapsed());
    assert!(ctx.inbox(&alice).await.notifications.is_empty());
    assert_eq!(ctx.devices(&alice).await.len(), 1);
}
