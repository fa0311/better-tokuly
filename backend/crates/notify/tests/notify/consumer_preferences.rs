//! The event consumer and the recipient's preferences: category toggles,
//! quiet hours, the push pause, mutes and the per-user push cap.
use std::time::Duration;

use chrono::{Timelike as _, Utc};
use tokuly_core::outbound::{Localized, NotificationRequest};
use tokuly_notify::{
    NotifyConfig,
    consumer::{DropReason, PushDecision, PushSkip, RequestOutcome},
    events::{FRIEND_PREFS_CHANGED_TOPIC, FriendPrefsChanged},
};
use tokuly_proto::notify::*;
use uuid::Uuid;

use crate::common::*;

const QUIET: Duration = Duration::from_millis(300);

#[tokio::test]
async fn category_preferences_gate_inbox_and_push_separately() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.register_browser(&alice).await;
    let chat = chat_message(&alice, &bob);
    let live = NotificationRequest::new(
        alice.id,
        "live.started",
        Localized::same("配信開始！"),
        Localized::same("bob"),
        "/live/1",
    )
    .actor(bob.id);

    // Defaults: everything on.
    let outcome = ctx.deliver(&chat).await;
    assert!(
        matches!(
            outcome,
            RequestOutcome::Handled {
                notification_id: Some(_),
                push: PushDecision::Queued(_)
            }
        ),
        "{outcome:?}"
    );

    // Push off for chat: recorded, not pushed. Other categories are unaffected.
    update_preferences(&ctx, &alice, toggle("chat", None, Some(false))).await;
    let outcome = ctx.deliver(&chat).await;
    assert!(
        matches!(
            outcome,
            RequestOutcome::Handled {
                notification_id: Some(_),
                push: PushDecision::Skipped(PushSkip::CategoryOff)
            }
        ),
        "{outcome:?}"
    );
    assert!(matches!(
        ctx.deliver(&live).await,
        RequestOutcome::Handled {
            notification_id: Some(_),
            push: PushDecision::Queued(_)
        }
    ));

    // Inbox off, push back on: pushed, not recorded.
    update_preferences(&ctx, &alice, toggle("chat", Some(false), Some(true))).await;
    let before = ctx.inbox(&alice).await.notifications.len();
    let mut frames = ctx.app.listen_user(alice.id).await;
    let outcome = ctx.deliver(&chat).await;
    let RequestOutcome::Handled {
        notification_id: None,
        push: PushDecision::Queued(job),
    } = outcome
    else {
        panic!("{outcome:?}")
    };
    assert_eq!(job.notification_id, None);
    assert_eq!(ctx.inbox(&alice).await.notifications.len(), before);
    assert!(
        frames.next(QUIET).await.is_none(),
        "no realtime event without an inbox entry"
    );

    // Both off: nothing at all.
    update_preferences(&ctx, &alice, toggle("chat", Some(false), Some(false))).await;
    let outcome = ctx.deliver(&chat).await;
    assert!(
        matches!(
            outcome,
            RequestOutcome::Handled {
                notification_id: None,
                push: PushDecision::Skipped(PushSkip::CategoryOff)
            }
        ),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn quiet_hours_and_the_pause_silence_pushes_but_not_the_inbox() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.register_browser(&alice).await;
    let chat = chat_message(&alice, &bob);
    sqlx::query("UPDATE users SET time_zone = 'Europe/Paris' WHERE id = $1")
        .bind(alice.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();

    // A window around the current time in the user's zone, and one that is exactly opposite.
    let local = Utc::now().with_timezone(&chrono_tz::Europe::Paris);
    let minute = (local.hour() * 60 + local.minute()) as i32;
    let around_now = QuietHours {
        enabled: true,
        start_minute: (minute + 1440 - 120) % 1440,
        end_minute: (minute + 120) % 1440,
        ..Default::default()
    };
    let half_a_day_away = QuietHours {
        enabled: true,
        start_minute: (minute + 600) % 1440,
        end_minute: (minute + 840) % 1440,
        ..Default::default()
    };

    update_preferences(
        &ctx,
        &alice,
        UpdatePreferencesRequest {
            quiet_hours: around_now.clone().into(),
            ..Default::default()
        },
    )
    .await;
    let outcome = ctx.deliver(&chat).await;
    assert!(
        matches!(
            outcome,
            RequestOutcome::Handled {
                notification_id: Some(_),
                push: PushDecision::Skipped(PushSkip::QuietHours)
            }
        ),
        "{outcome:?}"
    );

    update_preferences(
        &ctx,
        &alice,
        UpdatePreferencesRequest {
            quiet_hours: half_a_day_away.into(),
            ..Default::default()
        },
    )
    .await;
    assert!(matches!(
        ctx.deliver(&chat).await,
        RequestOutcome::Handled {
            push: PushDecision::Queued(_),
            ..
        }
    ));

    // The same window switched off does nothing.
    update_preferences(
        &ctx,
        &alice,
        UpdatePreferencesRequest {
            quiet_hours: QuietHours {
                enabled: false,
                ..around_now
            }
            .into(),
            ..Default::default()
        },
    )
    .await;
    assert!(matches!(
        ctx.deliver(&chat).await,
        RequestOutcome::Handled {
            push: PushDecision::Queued(_),
            ..
        }
    ));

    // Global pause.
    let _: PausePushResponse = ctx
        .app
        .call(
            "tokuly.notify.v1.PreferencesService/PausePush",
            Some(&alice),
            &PausePushRequest {
                duration_seconds: 3600,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let outcome = ctx.deliver(&chat).await;
    assert!(
        matches!(
            outcome,
            RequestOutcome::Handled {
                notification_id: Some(_),
                push: PushDecision::Skipped(PushSkip::Paused)
            }
        ),
        "{outcome:?}"
    );
    // A pause that ran out no longer applies, even before anyone resumes.
    sqlx::query("UPDATE notify_settings SET push_paused_until = now() - interval '1 minute' WHERE user_id = $1").bind(alice.id).execute(&ctx.app.core.db).await.unwrap();
    assert!(matches!(
        ctx.deliver(&chat).await,
        RequestOutcome::Handled {
            push: PushDecision::Queued(_),
            ..
        }
    ));
    let _: PausePushResponse = ctx
        .app
        .call(
            "tokuly.notify.v1.PreferencesService/PausePush",
            Some(&alice),
            &PausePushRequest::default(),
        )
        .await
        .unwrap();
    assert_eq!(skipped(&ctx.deliver(&chat).await), Some(PushSkip::Paused));
    let _: ResumePushResponse = ctx
        .app
        .call(
            "tokuly.notify.v1.PreferencesService/ResumePush",
            Some(&alice),
            &ResumePushRequest::default(),
        )
        .await
        .unwrap();
    assert!(matches!(
        ctx.deliver(&chat).await,
        RequestOutcome::Handled {
            push: PushDecision::Queued(_),
            ..
        }
    ));

    // Every one of those was recorded.
    assert_eq!(ctx.inbox(&alice).await.notifications.len(), 7);
}

#[tokio::test]
async fn a_muted_person_causes_no_notifications() {
    let ctx = start().await;
    let (alice, bob, carol) = (
        ctx.app.user("alice").await,
        ctx.app.user("bob").await,
        ctx.app.user("carol").await,
    );
    ctx.register_browser(&alice).await;
    let _: MuteUserResponse = ctx
        .app
        .call(
            "tokuly.notify.v1.PreferencesService/MuteUser",
            Some(&alice),
            &MuteUserRequest {
                user_id: bob.id.to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let mut frames = ctx.app.listen_user(alice.id).await;
    let outcome = ctx.deliver(&chat_message(&alice, &bob)).await;
    assert_eq!(dropped(&outcome), Some(DropReason::Muted), "{outcome:?}");
    assert!(ctx.inbox(&alice).await.notifications.is_empty());
    assert!(frames.next(QUIET).await.is_none());

    // Other people and system notices still get through.
    assert!(matches!(
        ctx.deliver(&chat_message(&alice, &carol)).await,
        RequestOutcome::Handled {
            notification_id: Some(_),
            push: PushDecision::Queued(_)
        }
    ));
    let system = NotificationRequest::new(
        alice.id,
        "system.announcement",
        Localized::same("お知らせ"),
        Localized::same("…"),
        "/news",
    );
    assert!(matches!(
        ctx.deliver(&system).await,
        RequestOutcome::Handled {
            notification_id: Some(_),
            ..
        }
    ));
    // Muting is one-directional.
    assert!(matches!(
        ctx.deliver(&chat_message(&bob, &alice)).await,
        RequestOutcome::Handled {
            notification_id: Some(_),
            ..
        }
    ));

    let _: UnmuteUserResponse = ctx
        .app
        .call(
            "tokuly.notify.v1.PreferencesService/UnmuteUser",
            Some(&alice),
            &UnmuteUserRequest {
                user_id: bob.id.to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        ctx.deliver(&chat_message(&alice, &bob)).await,
        RequestOutcome::Handled {
            notification_id: Some(_),
            ..
        }
    ));
}

#[tokio::test]
async fn friend_mutes_from_the_social_domain_are_mirrored() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let muted_users = || async {
        let response: ListMutedUsersResponse = ctx
            .app
            .call(
                "tokuly.notify.v1.PreferencesService/ListMutedUsers",
                Some(&alice),
                &ListMutedUsersRequest::default(),
            )
            .await
            .unwrap();
        response.users
    };
    let prefs_changed = |muted: bool| {
        event(
            FRIEND_PREFS_CHANGED_TOPIC,
            FriendPrefsChanged {
                user_id: alice.id,
                friend_id: bob.id,
                muted,
            },
        )
    };

    ctx.notify.handle_event(prefs_changed(true)).await.unwrap();
    ctx.notify.handle_event(prefs_changed(true)).await.unwrap(); // at-least-once delivery
    let muted = muted_users().await;
    assert_eq!(muted.len(), 1);
    assert_eq!(muted[0].user.as_option().unwrap().id, bob.id.to_string());
    assert_eq!(
        dropped(&ctx.deliver(&chat_message(&alice, &bob)).await),
        Some(DropReason::Muted)
    );

    ctx.notify.handle_event(prefs_changed(false)).await.unwrap();
    ctx.notify.handle_event(prefs_changed(false)).await.unwrap();
    assert!(muted_users().await.is_empty());
    assert!(matches!(
        ctx.deliver(&chat_message(&alice, &bob)).await,
        RequestOutcome::Handled {
            notification_id: Some(_),
            ..
        }
    ));

    // Mirroring does not announce itself back (that would loop with social).
    assert!(ctx.app.events_by("notify.mute.", alice.id).await.is_empty());
    // An event about an account that no longer exists is not an error.
    ctx.notify
        .handle_event(event(
            FRIEND_PREFS_CHANGED_TOPIC,
            FriendPrefsChanged {
                user_id: alice.id,
                friend_id: Uuid::now_v7(),
                muted: true,
            },
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_storm_of_notifications_is_capped_for_push_only() {
    let ctx = start_with(NotifyConfig {
        push_rate_per_minute: 3,
        ..Default::default()
    })
    .await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.register_browser(&alice).await;
    let mut queued = 0;
    let mut capped = 0;
    for _ in 0..8 {
        match ctx.deliver(&chat_message(&alice, &bob)).await {
            RequestOutcome::Handled {
                notification_id: Some(_),
                push: PushDecision::Queued(_),
            } => queued += 1,
            RequestOutcome::Handled {
                notification_id: Some(_),
                push: PushDecision::Skipped(PushSkip::RateLimited),
            } => capped += 1,
            other => panic!("{other:?}"),
        }
    }
    assert_eq!((queued, capped), (3, 5));
    assert_eq!(
        ctx.inbox(&alice).await.notifications.len(),
        8,
        "the inbox records everything"
    );
    // The cap is per recipient.
    ctx.register_browser(&bob).await;
    assert!(matches!(
        ctx.deliver(&chat_message(&bob, &alice)).await,
        RequestOutcome::Handled {
            push: PushDecision::Queued(_),
            ..
        }
    ));
}
