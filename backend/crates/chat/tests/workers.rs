//! The background workers end to end: signals arriving over the bus, the
//! link preview job, and the durable consumer of other domains' events.
//!
//! This file holds a single test on purpose: it starts the periodic sweeps,
//! which would do the work other tests set up for themselves.
mod common;

use std::time::Duration;

use common::*;
use tokuly_core::{
    events::Event,
    presence::{CONVERSATION_PRESENCE_SUBJECT, ConversationPresenceSignal, TYPING_SUBJECT, TypingSignal},
};
use tokuly_proto::chat::{chat_event::Event as Push, *};
use tokuly_testkit::FrameListener;
use uuid::Uuid;

/// Waits for a chat event matching `wanted` (the workers run on their own
/// time), failing after ten seconds.
async fn wait_for(listener: &mut FrameListener, what: &str, wanted: impl Fn(&Push) -> bool) -> Push {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        let Some(frame) = listener.next(Duration::from_millis(250)).await else { continue };
        if let Some(event) = chat_events(vec![frame]).into_iter().find(|event| wanted(event)) {
            return event;
        }
    }
    panic!("no {what} arrived within ten seconds");
}

#[tokio::test]
async fn the_workers_pick_up_signals_jobs_and_events() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let direct = ctx.direct(&taro, &hanako).await;
    let conversation_id: Uuid = direct.id.parse().unwrap();
    ctx.chat.spawn_workers();
    // Subscriptions and consumers need a moment to exist.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let bus = &ctx.app.core.bus;
    let mut hanako_events = ctx.listen(&hanako).await;

    // What the realtime gateway publishes when a client types or opens a conversation.
    let typing = TypingSignal { user_id: taro.id, conversation_id, typing: true };
    bus.publish(TYPING_SUBJECT, serde_json::to_vec(&typing).unwrap()).await.unwrap();
    wait_for(&mut hanako_events, "typing indicator", |event| matches!(event, Push::Typing(typing) if typing.user_id == taro.id.to_string())).await;
    let present = ConversationPresenceSignal { user_id: taro.id, conversation_id, present: true };
    bus.publish(CONVERSATION_PRESENCE_SUBJECT, serde_json::to_vec(&present).unwrap()).await.unwrap();
    wait_for(&mut hanako_events, "presence", |event| matches!(event, Push::Presence(presence) if presence.present_user_ids == vec![taro.id.to_string()])).await;

    // Sending a message with a link queues the preview job.
    let message = ctx.text(&taro, &direct.id, "http://localhost:8080/watch/dQw4w9WgXcQ").await;
    let updated = wait_for(&mut hanako_events, "link preview", |event| {
        matches!(event, Push::MessageUpdated(updated) if updated.reason == MessageUpdateReason::MESSAGE_UPDATE_REASON_LINK_PREVIEW)
    })
    .await;
    let Push::MessageUpdated(updated) = updated else { unreachable!() };
    assert_eq!(updated.message.id, message.id);
    assert_eq!(updated.message.link_preview.embed_id, "video/dQw4w9WgXcQ");

    // An event of another domain, as the outbox relay publishes it.
    ctx.block(&hanako, &taro).await;
    let blocked = Event {
        id: Uuid::now_v7(),
        topic: "social.user.blocked".into(),
        actor_id: Some(hanako.id),
        occurred_at: chrono::Utc::now(),
        data: serde_json::json!({ "user_id": hanako.id, "blocked_id": taro.id }),
    };
    bus.publish_durable("evt.social.user.blocked", serde_json::to_vec(&blocked).unwrap(), Some(&blocked.id.to_string())).await.unwrap();
    wait_for(&mut hanako_events, "read-only conversation", |event| matches!(event, Push::ConversationUpdated(updated) if updated.conversation.read_only)).await;
    assert!(ctx.conversation(&taro, &direct.id).await.conversation.read_only);
}
