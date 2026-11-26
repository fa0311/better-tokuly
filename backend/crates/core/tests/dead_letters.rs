//! Events and jobs their handlers gave up on, against a real NATS server
//! (`TOKULY_NATS_URL`, as exported by `tools/dev/infra.sh env`).
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio_util::sync::CancellationToken;
use tokuly_core::{
    AppError,
    bus::{Bus, STREAM_ANALYTICS, STREAM_DEAD, STREAM_EVENTS, STREAM_JOBS, STREAM_LOCATIONS},
    dead_letters,
    events::{self, ConsumeOptions, Event},
    jobs::{JobOptions, Jobs},
};
use uuid::Uuid;

/// A bus in a namespace of its own (see `bus.rs`).
async fn isolated_bus() -> Bus {
    let url = std::env::var("TOKULY_NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_owned());
    let bus = Bus::connect(&url, &format!("deadtest_{}", Uuid::now_v7().simple())).await.expect("NATS is reachable");
    bus.ensure_streams().await.expect("streams are created");
    bus
}

async fn remove_streams(bus: &Bus) {
    for name in [STREAM_EVENTS, STREAM_JOBS, STREAM_ANALYTICS, STREAM_LOCATIONS, STREAM_DEAD] {
        bus.js.delete_stream(bus.stream_name(name)).await.expect("stream is deleted");
    }
}

async fn until(what: &str, mut done: impl AsyncFnMut() -> bool) {
    let started = std::time::Instant::now();
    while !done().await {
        assert!(started.elapsed() < Duration::from_secs(10), "timed out waiting until {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn an_event_its_consumer_gives_up_on_is_buried_and_can_be_sent_round_again() {
    let bus = isolated_bus().await;
    let stop = CancellationToken::new();
    let (broken, handled) = (Arc::new(AtomicBool::new(true)), Arc::new(AtomicUsize::new(0)));
    let consumer = {
        let (bus, stop, broken, handled) = (bus.clone(), stop.clone(), broken.clone(), handled.clone());
        tokio::spawn(async move {
            // One delivery, so the test does not wait out the retries.
            let options = ConsumeOptions { max_deliver: 1, ..Default::default() };
            events::consume(bus, "purger", &["identity.user.deleted"], options, stop, move |_event: Event| {
                let (broken, handled) = (broken.clone(), handled.clone());
                async move {
                    if broken.load(Ordering::SeqCst) {
                        return Err(AppError::unavailable("DB_DOWN", "the database is restarting"));
                    }
                    handled.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .await;
        })
    };
    // Another consumer of the topic, which handles the event the first time.
    let bystander_saw = Arc::new(AtomicUsize::new(0));
    let bystander = {
        let (bus, stop, saw) = (bus.clone(), stop.clone(), bystander_saw.clone());
        tokio::spawn(async move {
            events::consume(bus, "bystander", &["identity.user.deleted"], ConsumeOptions::default(), stop, move |_event: Event| {
                let saw = saw.clone();
                async move {
                    saw.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .await;
        })
    };
    let event = Event {
        id: Uuid::now_v7(),
        topic: "identity.user.deleted".into(),
        actor_id: None,
        occurred_at: chrono::Utc::now(),
        data: serde_json::json!({ "user_id": Uuid::now_v7() }),
    };
    bus.publish_durable("evt.identity.user.deleted", serde_json::to_vec(&event).unwrap(), Some(&event.id.to_string())).await.unwrap();

    // The handler fails; the event is kept with where and why it died.
    until("the event is buried", async || !dead_letters::list(&bus, None, 10).await.unwrap().is_empty()).await;
    let buried = dead_letters::list(&bus, None, 10).await.unwrap();
    assert_eq!(buried.len(), 1);
    let letter = &buried[0].letter;
    assert_eq!((letter.consumer.as_str(), letter.subject.as_str(), letter.deliveries), ("purger", "evt.identity.user.deleted", 1));
    assert!(letter.error.contains("the database is restarting"), "{}", letter.error);
    assert_eq!(serde_json::from_value::<Event>(letter.payload.clone()).unwrap().id, event.id);
    assert!(dead_letters::list(&bus, Some("notify"), 10).await.unwrap().is_empty(), "another consumer's letters only");

    until("the bystander handled it", async || bystander_saw.load(Ordering::SeqCst) == 1).await;

    // The cause is fixed; the operator sends it round again.
    broken.store(false, Ordering::SeqCst);
    let redriven = dead_letters::redrive(&bus, Some("purger"), 10).await.unwrap();
    assert_eq!((redriven.sent, redriven.failed), (1, 0));
    until("the event is handled", async || handled.load(Ordering::SeqCst) == 1).await;
    assert!(dead_letters::list(&bus, None, 10).await.unwrap().is_empty(), "sent letters leave the stream");
    // Only the consumer it failed in hears of it again: the other one handled
    // it the first time and is not made to (it would notify people twice).
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(bystander_saw.load(Ordering::SeqCst), 1, "the bystander is not sent the redrive");

    stop.cancel();
    let _ = consumer.await;
    let _ = bystander.await;
    remove_streams(&bus).await;
}

#[tokio::test]
async fn a_job_that_failed_its_last_attempt_and_one_that_cannot_be_read_are_buried() {
    let bus = isolated_bus().await;
    let jobs = Jobs::new(bus.clone());
    let stop = CancellationToken::new();
    let worker = {
        let (jobs, stop) = (jobs.clone(), stop.clone());
        tokio::spawn(async move {
            let options = JobOptions { concurrency: 1, lease: Duration::from_secs(30), max_deliver: 1 };
            jobs.run("test.count", options, stop, |_: Vec<u32>| async { Err(AppError::internal(anyhow::anyhow!("no luck"))) }).await;
        })
    };
    jobs.enqueue("test.count", &vec![1, 2, 3]).await.unwrap();
    bus.publish_durable("job.test.count", "not json", None).await.unwrap();

    until("both jobs are buried", async || dead_letters::list(&bus, Some("job-test-count"), 10).await.unwrap().len() == 2).await;
    let buried = dead_letters::list(&bus, None, 10).await.unwrap();
    let failed = buried.iter().find(|b| b.letter.raw.is_none()).expect("the job that failed");
    assert_eq!((failed.letter.subject.as_str(), &failed.letter.payload), ("job.test.count", &serde_json::json!([1, 2, 3])));
    let unreadable = buried.iter().find(|b| b.letter.raw.is_some()).expect("the job that could not be read");
    assert_eq!(unreadable.letter.raw.as_deref(), Some("not json"));
    assert!(unreadable.letter.error.starts_with("undecodable job"), "{}", unreadable.letter.error);

    stop.cancel();
    let _ = worker.await;
    remove_streams(&bus).await;
}

#[tokio::test]
async fn erasing_an_account_removes_the_letters_that_mention_it_but_not_its_erasure() {
    let bus = isolated_bus().await;
    let (erased, other) = (Uuid::now_v7(), Uuid::now_v7());
    let event = |topic: &str, user: Uuid| {
        let event = Event {
            id: Uuid::now_v7(),
            topic: topic.into(),
            actor_id: Some(user),
            occurred_at: chrono::Utc::now(),
            data: serde_json::json!({ "user_id": user }),
        };
        serde_json::to_vec(&event).unwrap()
    };
    let bury = async |consumer: &str, subject: &str, payload: Vec<u8>| {
        dead_letters::bury(&bus, consumer, subject, &payload, "no luck", 12).await.unwrap();
    };
    bury("notify-events", "evt.notes.note.created", event("notes.note.created", erased)).await;
    bury("job-media-transcode", "job.media.transcode", format!("not json, owner {}", erased.simple()).into_bytes()).await;
    bury("search-indexer", "evt.identity.user.deleted", event("identity.user.deleted", erased)).await;
    bury("notify-events", "evt.notes.note.created", event("notes.note.created", other)).await;

    assert_eq!(dead_letters::erase_users(&bus, &[erased]).await.unwrap(), 2);
    let left: Vec<(String, String)> = dead_letters::list(&bus, None, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|b| (b.letter.subject, b.letter.payload["data"]["user_id"].as_str().unwrap_or_default().to_owned()))
        .collect();
    assert_eq!(
        left,
        [
            ("evt.identity.user.deleted".to_owned(), erased.to_string()),
            ("evt.notes.note.created".to_owned(), other.to_string()),
        ],
        "the erasure still to redrive and someone else's letter stay"
    );
    assert_eq!(dead_letters::erase_users(&bus, &[erased]).await.unwrap(), 0, "a second erasure finds nothing");
    remove_streams(&bus).await;
}

#[tokio::test]
async fn buried_messages_expire_after_the_configured_retention() {
    let url = std::env::var("TOKULY_NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_owned());
    let prefix = format!("deadtest_{}", Uuid::now_v7().simple());
    let max_age = async |bus: &Bus| bus.js.get_stream(bus.stream_name(STREAM_DEAD)).await.unwrap().info().await.unwrap().config.max_age;

    let bus = Bus::connect(&url, &prefix).await.expect("NATS is reachable");
    bus.ensure_streams().await.unwrap();
    assert_eq!(max_age(&bus).await, dead_letters::DEFAULT_RETENTION);

    // A deployment that shortens the setting: the existing stream follows.
    let bus = bus.with_dead_letter_retention(Duration::from_secs(1));
    bus.ensure_streams().await.unwrap();
    assert_eq!(max_age(&bus).await, Duration::from_secs(1));
    dead_letters::bury(&bus, "purger", "evt.notes.note.created", b"{}", "no luck", 12).await.unwrap();
    assert_eq!(dead_letters::list(&bus, None, 10).await.unwrap().len(), 1);
    until("the letter expires", async || dead_letters::list(&bus, None, 10).await.unwrap().is_empty()).await;
    remove_streams(&bus).await;
}
