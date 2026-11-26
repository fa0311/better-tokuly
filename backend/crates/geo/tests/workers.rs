//! The background workers as the worker process runs them: started with
//! `spawn_workers`, fed through the location stream and the event stream.
//! (What they do is tested in detail in the other test files; this checks
//! the wiring.)
mod common;

use std::time::Duration;

use chrono::Utc;
use common::*;
use tokuly_core::{
    events,
    presence::{self, LocationReport},
};
use tokuly_proto::geo::{geo_event::Event as Push, *};
use uuid::Uuid;

/// Waits until `check` holds. The loops run on their own clocks and the
/// machine may be busy, so the limit is generous.
async fn eventually<F: AsyncFnMut() -> bool>(what: &str, mut check: F) {
    for _ in 0..240 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("{what}: not within a minute");
}

#[tokio::test]
async fn workers_run_the_pipeline_the_event_consumer_and_the_housekeeping() {
    let ctx = start().await;
    let core = &ctx.app.core;
    ctx.geo.spawn_workers();
    // The worker process also runs the relay that moves outbox rows to the event stream.
    tokio::spawn(events::run_outbox_relay(core.db.clone(), core.bus.clone(), core.shutdown.clone()));
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let mut rens = ctx.app.listen_user(ren.id).await;

    // The location stream: the gateway submits samples, the friend gets the pins in order.
    let home = somewhere();
    let track = [home, offset(home, 90.0, 60.0), offset(home, 90.0, 120.0)];
    for (point, age) in track.iter().zip([30, 20, 10]) {
        let report = LocationReport {
            user_id: aoi.id,
            session_id: None,
            received_at: Utc::now(),
            sample: sample(*point, seconds_ago(age)),
        };
        presence::submit_location(&core.bus, &report).await.unwrap();
    }
    let mut seen: Vec<Point> = Vec::new();
    for _ in 0..12 {
        if seen.last().is_some_and(|last| at(*last, track[2])) {
            break;
        }
        if let Some(pin) = next_pin(&mut rens).await {
            assert_eq!(pin.user_id, aoi.id.to_string());
            seen.push(pin_point(&pin));
        }
    }
    // (The new friendship may show the same pin a second time.)
    seen.dedup_by(|a, b| at(*a, *b));
    assert_eq!(seen.len(), track.len(), "{seen:?}");
    assert!(seen.iter().zip(track).all(|(seen, sent)| at(*seen, sent)), "{seen:?}");

    // The history flusher: the samples reach ClickHouse without anyone asking for them.
    eventually("the history flusher stores the samples", async || {
        let stored: u64 = core
            .analytics
            .ch
            .query("SELECT count() FROM geo_points FINAL WHERE user_id = toUUID(?)")
            .bind(aoi.id.to_string())
            .fetch_one()
            .await
            .unwrap_or(0);
        stored == track.len() as u64
    })
    .await;

    // The sweeper: a live share whose time is up ends by itself.
    let share: CreateLiveShareResponse = ctx
        .location("CreateLiveShare", &aoi, &CreateLiveShareRequest { duration_seconds: 600, ..Default::default() })
        .await
        .unwrap();
    let share_id = Uuid::parse_str(&share.share.into_option().expect("share").id).unwrap();
    sqlx::query("UPDATE geo_live_shares SET expires_at = now() - interval '1 second' WHERE id = $1")
        .bind(share_id)
        .execute(&core.db)
        .await
        .unwrap();
    eventually("the sweeper ends the share", async || {
        sqlx::query_scalar::<_, bool>("SELECT ended_at IS NOT NULL FROM geo_live_shares WHERE id = $1")
            .bind(share_id)
            .fetch_one(&core.db)
            .await
            .unwrap()
    })
    .await;

    // The event stream: the social domain says the friendship ended, and the pin goes away.
    ctx.unfriend(&aoi, &ren).await;
    let mut removed = false;
    for _ in 0..12 {
        if matches!(next_push(&mut rens).await, Some(Push::LocationRemovedUserId(id)) if id == aoi.id.to_string()) {
            removed = true;
            break;
        }
    }
    assert!(removed, "the event consumer took the pin off the former friend's map");
    assert!(ctx.seen_by(&ren, &aoi).await.is_none());

    core.shutdown.cancel();
}
