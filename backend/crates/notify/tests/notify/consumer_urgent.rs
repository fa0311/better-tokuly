//! Urgent notifications (incoming calls, SOS): pushed through quiet hours,
//! the push pause and the per-user cap, but not past a category opt-out or
//! a mute.
use chrono::{Timelike as _, Utc};
use tokuly_core::outbound::{Localized, NotificationRequest};
use tokuly_notify::{
    NotifyConfig,
    consumer::{DropReason, PushDecision, PushSkip, RequestOutcome},
};
use tokuly_proto::notify::*;
use tokuly_testkit::TestUser;

use crate::common::*;

/// An incoming call from `from`: the kind of notification that cannot wait.
fn incoming_call(to: &TestUser, from: &TestUser) -> NotificationRequest {
    NotificationRequest::new(
        to.id,
        "chat.call",
        Localized::new("着信中…", "Incoming call"),
        Localized::same(from.handle.clone()),
        "/chat/42/call",
    )
    .actor(from.id)
    .urgent()
}

#[tokio::test]
async fn urgent_notifications_get_through_quiet_hours_and_the_pause() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.register_browser(&alice).await;
    // Both "not now" settings at once: quiet all day (but for one minute a
    // few minutes from now) and pushes paused until further notice.
    let local = Utc::now().with_timezone(&chrono_tz::Asia::Tokyo);
    let gap = ((local.hour() * 60 + local.minute()) as i32 + 5) % 1440;
    let all_day = QuietHours { enabled: true, start_minute: (gap + 1) % 1440, end_minute: gap, ..Default::default() };
    update_preferences(&ctx, &alice, UpdatePreferencesRequest { quiet_hours: all_day.into(), ..Default::default() })
        .await;
    let _: PausePushResponse = ctx
        .app
        .call("tokuly.notify.v1.PreferencesService/PausePush", Some(&alice), &PausePushRequest::default())
        .await
        .unwrap();

    // An ordinary message waits in the inbox...
    assert_eq!(skipped(&ctx.deliver(&chat_message(&alice, &bob)).await), Some(PushSkip::Paused));
    // ...a call rings.
    let outcome = ctx.deliver(&incoming_call(&alice, &bob)).await;
    let RequestOutcome::Handled { notification_id: Some(call_id), push: PushDecision::Queued(job) } = outcome else {
        panic!("{outcome:?}")
    };
    assert!(job.urgent, "the job carries the urgency to the push channels");
    assert_eq!(job.kind, "chat.call");

    // Clients can tell the two apart.
    let inbox = ctx.inbox(&alice).await;
    let urgent_flags: Vec<(&str, bool)> = inbox.notifications.iter().map(|n| (n.kind.as_str(), n.urgent)).collect();
    assert_eq!(urgent_flags, [("chat.call", true), ("chat.message", false)]);
    assert_eq!(inbox.notifications[0].id, call_id.to_string());
}

#[tokio::test]
async fn urgent_notifications_still_respect_category_opt_outs_and_mutes() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);
    ctx.register_browser(&alice).await;

    // Push switched off for the category: off means off.
    update_preferences(&ctx, &alice, toggle("chat", None, Some(false))).await;
    let outcome = ctx.deliver(&incoming_call(&alice, &bob)).await;
    assert!(
        matches!(
            outcome,
            RequestOutcome::Handled { notification_id: Some(_), push: PushDecision::Skipped(PushSkip::CategoryOff) }
        ),
        "{outcome:?}"
    );
    update_preferences(&ctx, &alice, toggle("chat", None, Some(true))).await;

    // A muted person stays muted, however urgent their call.
    let _: MuteUserResponse = ctx
        .app
        .call(
            "tokuly.notify.v1.PreferencesService/MuteUser",
            Some(&alice),
            &MuteUserRequest { user_id: bob.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(dropped(&ctx.deliver(&incoming_call(&alice, &bob)).await), Some(DropReason::Muted));
    // Someone else's call rings.
    assert!(matches!(
        ctx.deliver(&incoming_call(&alice, &carol)).await,
        RequestOutcome::Handled { push: PushDecision::Queued(_), .. }
    ));

    // The producer can still ask for an inbox entry only; and without a
    // device there is nothing to ring.
    let outcome = ctx.deliver(&incoming_call(&alice, &carol).inbox_only()).await;
    assert_eq!(skipped(&outcome), Some(PushSkip::NotRequested));
    let outcome = ctx.deliver(&incoming_call(&carol, &alice)).await;
    assert_eq!(skipped(&outcome), Some(PushSkip::NoDevices));
}

#[tokio::test]
async fn urgent_notifications_are_not_crowded_out_by_a_storm() {
    let ctx = start_with(NotifyConfig { push_rate_per_minute: 2, ..Default::default() }).await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.register_browser(&alice).await;
    let is_queued =
        |outcome: &RequestOutcome| matches!(outcome, RequestOutcome::Handled { push: PushDecision::Queued(_), .. });

    // Urgent pushes do not use up the budget of ordinary ones...
    for _ in 0..3 {
        assert!(is_queued(&ctx.deliver(&incoming_call(&alice, &bob)).await));
    }
    assert!(is_queued(&ctx.deliver(&chat_message(&alice, &bob)).await));
    assert!(is_queued(&ctx.deliver(&chat_message(&alice, &bob)).await));
    assert_eq!(skipped(&ctx.deliver(&chat_message(&alice, &bob)).await), Some(PushSkip::RateLimited));
    // ...and still go out once that budget is spent.
    assert!(is_queued(&ctx.deliver(&incoming_call(&alice, &bob)).await));
}
