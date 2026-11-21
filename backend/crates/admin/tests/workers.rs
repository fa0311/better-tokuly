//! The background workers wired end to end: events arriving on the stream,
//! the broadcast job and the maintenance timer. One test, one binary: the
//! workers share durable consumers, so they get a NATS namespace of their own.
mod common;

use std::{future::Future, time::Duration};

use common::*;
use tokuly_proto::admin::*;
use uuid::Uuid;

/// Polls `condition` until it holds; panics after 20 seconds.
async fn wait_until<F, Fut>(what: &str, condition: F)
where
    F: Fn() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while !condition().await {
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting until {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn workers_consume_events_run_jobs_and_lift_expired_suspensions() {
    let (app, admin) = start().await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let (owner, reporter, sleeper) = (app.user("owner").await, app.user("rep").await, app.user("sleeper").await);

    // Left over from before the workers started: a suspension that has run out.
    let _: SuspendUserResponse = app
        .call(
            &format!("{USERS}/SuspendUser"),
            Some(&moderator),
            &SuspendUserRequest { user_id: sleeper.id.to_string(), reason: "テスト".into(), suspension_seconds: 3600, ..Default::default() },
        )
        .await
        .unwrap();
    sqlx::query("UPDATE user_suspensions SET expires_at = now() - interval '1 minute' WHERE user_id = $1")
        .bind(sleeper.id)
        .execute(&app.core.db)
        .await
        .unwrap();

    admin.spawn_workers();

    // Timer: its first pass lifts the expired suspension without anyone asking.
    wait_until("the expired suspension is lifted", || async { !is_banned(&app, sleeper.id).await }).await;

    // Events: a report published on the event stream (as the outbox relay does) becomes a case.
    let event = report_event(reporter.id, (NOTE, Uuid::now_v7()), owner.id, "SPAM", "ストリーム経由");
    app.core.bus.publish_durable(format!("evt.{}", event.topic), serde_json::to_vec(&event).unwrap(), Some(&event.id.to_string())).await.unwrap();
    wait_until("the report is filed", || async { !case_of_owner(&app, &moderator, owner.id).await.is_empty() }).await;
    assert_eq!(case_of_owner(&app, &moderator, owner.id).await[0].report_count, 1);

    // Jobs: publishing with the notify option queues the broadcast, and the worker runs it to the end.
    let created: CreateAnnouncementResponse = app
        .call(
            &format!("{ANNOUNCEMENTS}/CreateAnnouncement"),
            Some(&boss),
            &CreateAnnouncementRequest {
                title: LocalizedText { ja: "ワーカーのテスト".into(), ..Default::default() }.into(),
                body: LocalizedText { ja: "本文".into(), ..Default::default() }.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let id = created.announcement.id.clone();
    let _: PublishAnnouncementResponse = app
        .call(
            &format!("{ANNOUNCEMENTS}/PublishAnnouncement"),
            Some(&boss),
            &PublishAnnouncementRequest { id: id.clone(), notify_users: true, ..Default::default() },
        )
        .await
        .unwrap();
    let managed = || async {
        let response: GetManagedAnnouncementResponse = app
            .call(&format!("{ANNOUNCEMENTS}/GetManagedAnnouncement"), Some(&boss), &GetManagedAnnouncementRequest { id: id.clone(), ..Default::default() })
            .await
            .unwrap();
        response.announcement.into_option().unwrap()
    };
    wait_until("the broadcast finishes", || async { managed().await.notified_at.is_set() }).await;
    // Everyone signed in was notified; the sleeper lost their sessions to the suspension.
    assert_eq!(managed().await.notified_count, 4);
    assert_eq!(notifications_for(&app, owner.id).await.iter().filter(|n| n.kind == "announcement.published").count(), 1);
    assert!(notifications_for(&app, sleeper.id).await.is_empty());

    app.core.shutdown.cancel();
}
