//! The dead letters in the console: listed without what they carried, sent
//! round again, and the redrive in the audit log.
mod common;

use common::*;
use tokuly_core::{dead_letters, events::Event};
use tokuly_proto::admin::*;
use tokuly_testkit::{TestApp, TestUser};
use uuid::Uuid;

async fn list(app: &TestApp, boss: &TestUser, consumer_prefix: &str) -> ListDeadLettersResponse {
    let request = ListDeadLettersRequest { consumer_prefix: consumer_prefix.into(), ..Default::default() };
    app.call(&format!("{SYSTEM}/ListDeadLetters"), Some(boss), &request).await.unwrap()
}

/// Buries an event that names a secret in its payload, as a consumer that gave up on it does.
async fn bury_event(app: &TestApp, consumer: &str, secret: &str) -> Event {
    let event = Event {
        id: Uuid::now_v7(),
        topic: "chat.message.sent".into(),
        actor_id: None,
        occurred_at: chrono::Utc::now(),
        data: serde_json::json!({ "text": secret }),
    };
    let payload = serde_json::to_vec(&event).unwrap();
    let subject = app.core.bus.subject("evt.chat.message.sent");
    dead_letters::bury(&app.core.bus, consumer, &subject, &payload, "the search engine is down", 12).await.unwrap();
    event
}

#[tokio::test]
async fn dead_letters_are_for_admins_only() {
    let (app, _admin) = start().await;
    let user = app.user("user").await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    for rpc in ["ListDeadLetters", "RedriveDeadLetters"] {
        assert_role_gated(&app, &format!("{SYSTEM}/{rpc}"), &[&user, &moderator]).await;
    }
}

#[tokio::test]
async fn staff_see_where_and_why_a_message_died_but_not_what_it_carried() {
    let (app, _admin) = start().await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let consumer = format!("search-{}", Uuid::now_v7().simple());
    let secret = format!("ひみつ{}", Uuid::now_v7().simple());
    let event = bury_event(&app, &consumer, &secret).await;
    let job_queue = format!("job-{consumer}");
    let job = serde_json::json!({ "key": secret });
    dead_letters::bury(&app.core.bus, &job_queue, "job.media.transcode", job.to_string().as_bytes(), "ffmpeg crashed", 3).await.unwrap();

    let listed = list(&app, &boss, &consumer).await;
    assert_eq!(listed.letters.len(), 1, "only the consumer asked for");
    let letter = &listed.letters[0];
    assert_eq!((letter.consumer.as_str(), letter.subject.as_str()), (consumer.as_str(), "evt.chat.message.sent"));
    assert_eq!((letter.error.as_str(), letter.deliveries), ("the search engine is down", 12));
    assert_eq!(letter.event_id, event.id.to_string());
    assert!(letter.occurred_at.as_option().is_some() && letter.died_at.as_option().is_some());
    assert!(listed.total >= 2);

    let jobs = list(&app, &boss, &job_queue).await;
    assert_eq!(jobs.letters.len(), 1);
    assert!(jobs.letters[0].event_id.is_empty() && jobs.letters[0].occurred_at.as_option().is_none(), "a job has no event");

    let everything: serde_json::Value =
        app.call(&format!("{SYSTEM}/ListDeadLetters"), Some(&boss), &serde_json::json!({})).await.unwrap();
    assert!(!everything.to_string().contains(&secret), "payloads stay out of the console");

    let long = "x".repeat(101);
    let refused = app
        .call::<_, serde_json::Value>(&format!("{SYSTEM}/ListDeadLetters"), Some(&boss), &serde_json::json!({ "consumerPrefix": long }))
        .await
        .unwrap_err();
    assert_eq!(refused.reason, "CONSUMER_PREFIX_TOO_LONG");
}

#[tokio::test]
async fn a_redrive_sends_the_consumers_letters_round_again_and_is_audited() {
    let (app, _admin) = start().await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let consumer = format!("notify-{}", Uuid::now_v7().simple());
    let other = format!("feed-{}", Uuid::now_v7().simple());
    bury_event(&app, &consumer, "一通目").await;
    bury_event(&app, &consumer, "二通目").await;
    bury_event(&app, &other, "べつの").await;

    let done: RedriveDeadLettersResponse = app
        .call(&format!("{SYSTEM}/RedriveDeadLetters"), Some(&boss), &RedriveDeadLettersRequest { consumer_prefix: consumer.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!((done.sent, done.failed), (2, 0));
    assert!(list(&app, &boss, &consumer).await.letters.is_empty(), "sent and removed");
    assert_eq!(list(&app, &boss, &other).await.letters.len(), 1, "another consumer's letter stays");

    let entries: ListAuditLogResponse = app
        .call(
            &format!("{SYSTEM}/ListAuditLog"),
            Some(&boss),
            &ListAuditLogRequest { action: "dead_letters.redriven".into(), target_id: consumer.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(entries.entries.len(), 1);
    let entry = &entries.entries[0];
    assert_eq!((entry.target_type.as_str(), entry.actor.as_option().map(|a| a.id.as_str())), ("dead_letters", Some(boss.id.to_string().as_str())));
    let after: serde_json::Value = serde_json::from_str(&entry.after_json).unwrap();
    assert_eq!(after, serde_json::json!({ "sent": 2, "failed": 0 }));
}
