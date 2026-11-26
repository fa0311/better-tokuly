//! Requests that arrive together: the limits and "one at a time" rules hold
//! (the database or a lock enforces them, not a check that two requests can
//! both pass), and no handler holds one database connection while waiting
//! for a second — the test pool has five, and each burst here is larger.
mod common;

use std::time::{Duration, Instant};

use chrono::Utc;
use common::*;
use futures::future::join_all;
use serde_json::{Value, json};
use tokuly_testkit::TestUser;

/// More calls at once than the pool has connections.
const BURST: usize = 12;

async fn call(ctx: &Ctx, user: &TestUser, path: &str, request: Value) -> Result<Value, String> {
    ctx.app
        .call::<Value, Value>(&format!("tokuly.geo.v1.{path}"), Some(user), &request)
        .await
        .map_err(|error| error.reason)
}

/// Fires the same call `BURST` times at once and returns the outcomes.
async fn burst(ctx: &Ctx, user: &TestUser, path: &str, request: Value) -> Vec<Result<Value, String>> {
    let started = Instant::now();
    let outcomes = join_all((0..BURST).map(|_| call(ctx, user, path, request.clone()))).await;
    // Far from the pool's acquire timeout: nobody waited for a connection another request of the burst was sitting on.
    assert!(started.elapsed() < Duration::from_secs(20), "{path}: {:?} for {BURST} calls", started.elapsed());
    outcomes
}

fn ok(outcomes: &[Result<Value, String>]) -> usize {
    outcomes.iter().filter(|outcome| outcome.is_ok()).count()
}

fn point_json(point: Point) -> Value {
    json!({ "lat": point.0, "lng": point.1 })
}

#[tokio::test]
async fn bursts_from_one_account_keep_the_rules_and_do_not_starve_the_pool() {
    let ctx = start().await;
    let (aoi, ren, other) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("other").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    ctx.at(&aoi, home).await;
    ctx.at(&ren, offset(home, 0.0, 2000.0)).await;

    // Live shares: ten at a time, however many are asked for at once.
    let shares = burst(&ctx, &aoi, "LocationService/CreateLiveShare", json!({ "durationSeconds": 600 })).await;
    assert_eq!(ok(&shares), 10);
    assert!(shares.iter().filter_map(|outcome| outcome.as_ref().err()).all(|reason| reason == "TOO_MANY_SHARES"));

    // One home per account.
    let homes = burst(
        &ctx,
        &aoi,
        "PlaceService/CreatePlace",
        json!({ "name": "うち", "label": "PLACE_LABEL_HOME", "center": point_json(home) }),
    )
    .await;
    assert_eq!(ok(&homes), 1);
    assert!(homes.iter().filter_map(|outcome| outcome.as_ref().err()).all(|reason| reason == "LABEL_TAKEN"));

    // One active trip: each new one replaces the one before.
    let trips = burst(
        &ctx,
        &aoi,
        "TripService/StartTrip",
        json!({ "destination": { "position": point_json(offset(home, 90.0, 3000.0)) } }),
    )
    .await;
    assert!(ok(&trips) >= 1, "{trips:?}");
    let active: i64 = sqlx::query_scalar("SELECT count(*) FROM geo_trips WHERE user_id = $1 AND status = 1")
        .bind(aoi.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(active, 1);

    // One SOS: every tap gets the same one, and the friend is alerted once.
    let alarms = burst(&ctx, &aoi, "SafetyService/TriggerSos", json!({ "message": "助けて" })).await;
    let ids: std::collections::HashSet<String> =
        alarms.iter().filter_map(|outcome| outcome.as_ref().ok()).map(|sos| sos["sos"]["id"].to_string()).collect();
    assert_eq!(ids.len(), 1, "{alarms:?}");
    // (Taps beyond the hourly limit are turned away; none creates a second SOS.)
    assert!(
        ok(&alarms) >= 10
            && alarms.iter().filter_map(|outcome| outcome.as_ref().err()).all(|reason| reason == "RATE_LIMITED")
    );
    assert_eq!(ctx.notifications_for(ren.id, "geo.sos").await.len(), 1);

    // Ghost mode set many times at once: one rule, and it is the one asked for.
    let rules = burst(&ctx, &aoi, "LocationService/SetGhostMode", json!({ "mode": "GHOST_MODE_FROZEN" })).await;
    assert_eq!(ok(&rules), BURST, "{rules:?}");
    let stored: Vec<i16> = sqlx::query_scalar("SELECT mode FROM geo_ghost_rules WHERE owner_id = $1")
        .bind(aoi.id)
        .fetch_all(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(stored, vec![3]);

    // Alerts about a friend: thirty at most, also when asked for together.
    let mut alerts = Vec::new();
    for _ in 0..3 {
        alerts.extend(
            burst(
                &ctx,
                &ren,
                "SafetyService/CreatePlaceAlert",
                json!({ "subjectUserId": aoi.id, "center": point_json(home), "trigger": "ALERT_TRIGGER_ARRIVE" }),
            )
            .await,
        );
    }
    assert_eq!(ok(&alerts), 30, "{:?}", alerts.iter().filter_map(|outcome| outcome.as_ref().err()).collect::<Vec<_>>());

    // Location uploads of one account are processed one after the other:
    // every fix of every upload is accepted exactly once.
    let now = Utc::now();
    let uploads = join_all((0..BURST as i64).map(|i| {
        let fix = sample(offset(home, 90.0, i as f64 * 20.0), now + chrono::Duration::seconds(5 + i * 2));
        call(&ctx, &aoi, "LocationService/ReportLocations", json!({ "samples": [serde_json::to_value(fix).unwrap()] }))
    }))
    .await;
    assert_eq!(ok(&uploads), BURST, "{uploads:?}");
    let accepted: i64 =
        uploads.iter().map(|outcome| outcome.as_ref().unwrap()["acceptedCount"].as_i64().unwrap_or(0)).sum();
    assert!(accepted >= 1, "the newest fix is always accepted");
    let mine = call(&ctx, &aoi, "LocationService/GetMyLocation", json!({})).await.unwrap();
    let newest = offset(home, 90.0, (BURST as f64 - 1.0) * 20.0);
    assert!((mine["location"]["position"]["lng"].as_f64().unwrap() - newest.1).abs() < 1e-9, "the newest fix wins");

    // Everybody else was served all along.
    assert!(call(&ctx, &other, "LocationService/ListFriendLocations", json!({})).await.is_ok());
}

#[tokio::test]
async fn friends_who_meet_at_the_same_instant_end_up_in_one_gathering() {
    let ctx = start().await;
    let users =
        [ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("kai").await, ctx.app.user("mio").await];
    for (index, a) in users.iter().enumerate() {
        for b in &users[index + 1..] {
            ctx.befriend(a, b).await;
        }
    }
    let spot = somewhere();
    let t = Utc::now();
    let position = |index: usize| offset(spot, index as f64 * 90.0, 6.0);
    // All four report at the same moments, each batch of uploads in parallel.
    for seconds_before in [200, 170, 40, 10] {
        let ctx = &ctx;
        let outcomes = join_all(users.iter().enumerate().map(|(index, user)| async move {
            let fix = sample(position(index), t - chrono::Duration::seconds(seconds_before));
            let report = tokuly_core::presence::LocationReport {
                user_id: user.id,
                session_id: None,
                received_at: Utc::now(),
                sample: fix,
            };
            ctx.geo.ingest(report).await
        }))
        .await;
        for outcome in outcomes {
            outcome.expect("every upload is processed");
        }
    }
    let ids: Vec<uuid::Uuid> = users.iter().map(|user| user.id).collect();
    let memberships: Vec<(uuid::Uuid, uuid::Uuid)> = sqlx::query_as(
        "SELECT gathering_id, user_id FROM geo_gathering_members WHERE user_id = ANY($1) AND left_at IS NULL",
    )
    .bind(&ids)
    .fetch_all(&ctx.app.core.db)
    .await
    .unwrap();
    let gatherings: std::collections::HashSet<uuid::Uuid> =
        memberships.iter().map(|(gathering, _)| *gathering).collect();
    assert_eq!((gatherings.len(), memberships.len()), (1, 4), "one gathering with all four: {memberships:?}");
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM geo_gatherings g WHERE ended_at IS NULL
         AND EXISTS (SELECT 1 FROM geo_gathering_members m WHERE m.gathering_id = g.id AND m.user_id = ANY($1))",
    )
    .bind(&ids)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(active, 1, "gatherings founded at the same instant were merged, none is left over");
}
