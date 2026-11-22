//! ClickHouse holds the raw history and nothing else: when it cannot be
//! written or read, location sharing and the PostgreSQL side of the timeline
//! carry on. (A test binary of its own: it breaks the binary's private
//! ClickHouse database.)
mod common;

use chrono::{Duration, Timelike as _, Utc};
use common::*;
use tokuly_core::time::ts;
use tokuly_proto::geo::*;

/// A time zone in which it is around noon right now, and today's date there.
fn midday_zone() -> (String, String) {
    let now = Utc::now();
    let shift = 12 - now.hour() as i64; // hours to add to UTC
    let zone = match shift {
        0 => "Etc/GMT".to_owned(),
        s if s > 0 => format!("Etc/GMT-{s}"),
        s => format!("Etc/GMT+{}", -s),
    };
    (zone, (now + Duration::hours(shift)).date_naive().to_string())
}

#[tokio::test]
async fn without_the_raw_history_store_sharing_and_the_timeline_carry_on() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    ctx.app.core.analytics.ch.query("DROP TABLE geo_points").execute().await.expect("drop the raw history table");

    // Ten minutes of walking, then eight minutes in one spot: accepted as ever, and friends see the pin.
    let from = somewhere();
    let start = minutes_ago(25);
    let end = offset(from, 90.0, 840.0);
    let mut samples: Vec<LocationSample> =
        (0..=60).map(|i| sample(offset(from, 90.0, i as f64 * 14.0), start + Duration::seconds(i * 10))).collect();
    samples.extend((1..=16).map(|i| sample(end, start + Duration::seconds(600 + i * 30))));
    let sent = samples.len() as i32;
    assert_eq!(ctx.report(&aoi, samples).await.accepted_count, sent);
    let pin = ctx.seen_by(&ren, &aoi).await.expect("a pin");
    assert_at!(pin_point(&pin), end);
    assert!(pin.stationary_since.as_option().is_some(), "stay detection does not need the raw history");

    // The day comes without its path; everything PostgreSQL holds is there.
    let (zone, date) = midday_zone();
    let day: GetDayResponse =
        ctx.history("GetDay", &aoi, &GetDayRequest { date, time_zone: zone, ..Default::default() }).await.unwrap();
    let summary = day.summary.as_option().expect("a summary");
    assert!((summary.distance_meters - 840.0).abs() < 60.0, "walked {} m", summary.distance_meters);
    assert_eq!((summary.visit_count, day.visits.len()), (1, 1));
    assert!(day.path_polyline.is_empty());
    assert!(day.bounds.as_option().is_some(), "the map still frames the visit");

    // A route replay is raw history and nothing else, and deleting history must not half-succeed.
    let now = Utc::now();
    let range = GetRouteRequest { from: ts(now - Duration::hours(1)), to: ts(now), ..Default::default() };
    let error = ctx.history::<_, GetRouteResponse>("GetRoute", &aoi, &range).await.unwrap_err();
    assert_eq!((error.code.as_str(), error.reason.as_str()), ("unavailable", "HISTORY_UNAVAILABLE"));
    let error = ctx
        .history::<_, DeleteHistoryResponse>("DeleteHistory", &aoi, &DeleteHistoryRequest::default())
        .await
        .unwrap_err();
    assert_eq!((error.code.as_str(), error.reason.as_str()), ("unavailable", "HISTORY_UNAVAILABLE"));
    let visits: ListVisitsResponse = ctx.history("ListVisits", &aoi, &ListVisitsRequest::default()).await.unwrap();
    assert_eq!(visits.visits.len(), 1, "nothing was deleted");
}
