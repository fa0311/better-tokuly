//! The background workers as the worker process runs them: started with
//! `spawn_workers`, fed through the bus and the event stream. (What they do
//! is tested in detail in `tests/background.rs`; this checks the wiring.)
mod common;

use std::time::Duration;

use chrono::Utc;
use common::*;
use tokuly_core::{
    events,
    presence::{self, PresenceChange},
};
use tokuly_proto::social::*;

#[tokio::test]
async fn workers_fan_out_presence_and_consume_identity_events() {
    let ctx = start().await;
    let core = &ctx.app.core;
    ctx.social.spawn_workers();
    // The worker process also runs the relay that moves outbox rows to the event stream.
    tokio::spawn(events::run_outbox_relay(core.db.clone(), core.bus.clone(), core.shutdown.clone()));

    // Presence: the gateway announces a transition on the bus; the friend's devices hear of it.
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    let mut bob_devices = ctx.app.listen_user(bob.id).await;
    let mut heard = Vec::new();
    for _ in 0..40 {
        // The worker subscribes asynchronously; announce until it listens.
        presence::announce(&core.bus, &PresenceChange { user_id: alice.id, online: true, at: Utc::now() })
            .await
            .unwrap();
        heard = presence_events(&mut bob_devices, Duration::from_millis(250)).await;
        if !heard.is_empty() {
            break;
        }
    }
    assert!(heard.iter().all(|event| event.user_id == alice.id.to_string() && event.online));
    assert!(!heard.is_empty(), "the presence worker forwards announcements to friends");

    // Events: a sign-up carrying an invite code becomes an intro.
    let (inviter, newcomer) = (ctx.app.user("inviter").await, ctx.app.user("newcomer").await);
    let invite: GetMyInviteResponse =
        ctx.friends("GetMyInvite", &inviter, &GetMyInviteRequest::default()).await.unwrap();
    let code = invite.invite.into_option().unwrap().code;
    core.emit(
        "identity.user.created",
        Some(newcomer.id),
        &serde_json::json!({ "user_id": newcomer.id, "handle": newcomer.handle, "invite_code": code }),
    )
    .await
    .unwrap();
    let mut intro_count = 0;
    for _ in 0..80 {
        let invite: GetMyInviteResponse =
            ctx.friends("GetMyInvite", &inviter, &GetMyInviteRequest::default()).await.unwrap();
        intro_count = invite.invite.into_option().unwrap().intro_count;
        if intro_count > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(intro_count, 1, "the event consumer recorded the intro");

    core.shutdown.cancel();
}
