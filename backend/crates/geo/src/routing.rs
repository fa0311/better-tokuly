//! Directions: a Valhalla routing engine when `TOKULY_ROUTING_URL` is set,
//! otherwise an estimate from the great-circle distance and a typical speed
//! for the travel mode.
use std::time::Duration;

use tokuly_core::prelude::*;
use tokuly_proto::geo::{DirectionStep, TravelMode};

use crate::{
    Geo,
    math::{self, Point},
};

/// Real routes are longer than the straight line by about this factor.
const DETOUR: f64 = 1.3;
/// Waiting and walking to the stop, added to transit estimates.
const TRANSIT_OVERHEAD_S: f64 = 300.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub distance_m: f64,
    pub duration_s: i32,
    /// Encoded polyline, precision 5.
    pub polyline: String,
    /// `"valhalla"` or `"estimate"`.
    pub provider: &'static str,
    pub steps: Vec<DirectionStep>,
}

/// Unspecified means walking.
pub fn travel_mode(mode: buffa::EnumValue<TravelMode>) -> TravelMode {
    match mode.as_known() {
        Some(TravelMode::TRAVEL_MODE_UNSPECIFIED) | None => TravelMode::TRAVEL_MODE_WALK,
        Some(mode) => mode,
    }
}

/// Door-to-door speed in m/s people typically manage with a mode over a
/// straight-line distance (longer trips use faster roads and lines).
pub fn typical_speed(mode: TravelMode, distance_m: f64) -> f64 {
    let (short, medium, long) = match mode {
        TravelMode::TRAVEL_MODE_BICYCLE => (4.2, 4.2, 4.2),
        TravelMode::TRAVEL_MODE_DRIVE => (8.3, 13.9, 22.2),
        TravelMode::TRAVEL_MODE_TRANSIT => (5.5, 9.7, 16.7),
        _ => (1.35, 1.35, 1.35),
    };
    match distance_m {
        d if d < 5_000.0 => short,
        d if d < 50_000.0 => medium,
        _ => long,
    }
}

/// Seconds a trip over `distance_m` (straight line) typically takes.
pub fn typical_duration(mode: TravelMode, distance_m: f64) -> f64 {
    let moving = distance_m * DETOUR / typical_speed(mode, distance_m);
    if mode == TravelMode::TRAVEL_MODE_TRANSIT && distance_m > 0.0 { moving + TRANSIT_OVERHEAD_S } else { moving }
}

/// The estimate used without a routing engine: the great circle, at a
/// typical speed for the mode.
pub fn estimate(origin: Point, destination: Point, mode: TravelMode, ja: bool) -> Route {
    let distance_m = math::distance(origin, destination);
    let duration_s = typical_duration(mode, distance_m).round() as i32;
    let line = math::great_circle_line(origin, destination, 50_000.0, 64);
    let instruction = if ja { "目的地へ向かう(直線距離からの推定)" } else { "Head to the destination (estimated from the straight-line distance)" };
    Route {
        distance_m,
        duration_s,
        polyline: math::encode_polyline(&line),
        provider: "estimate",
        steps: vec![DirectionStep { instruction: instruction.to_owned(), distance_meters: distance_m, duration_seconds: duration_s, ..Default::default() }],
    }
}

#[derive(serde::Deserialize)]
struct ValhallaResponse {
    trip: ValhallaTrip,
}

#[derive(serde::Deserialize)]
struct ValhallaTrip {
    legs: Vec<ValhallaLeg>,
    summary: ValhallaSummary,
}

#[derive(serde::Deserialize)]
struct ValhallaSummary {
    /// Kilometers (we ask for that unit).
    length: f64,
    /// Seconds.
    time: f64,
}

#[derive(serde::Deserialize)]
struct ValhallaLeg {
    /// Encoded polyline, precision 6.
    shape: String,
    #[serde(default)]
    maneuvers: Vec<ValhallaManeuver>,
}

#[derive(serde::Deserialize)]
struct ValhallaManeuver {
    #[serde(default)]
    instruction: String,
    #[serde(default)]
    length: f64,
    #[serde(default)]
    time: f64,
}

fn costing(mode: TravelMode) -> &'static str {
    match mode {
        TravelMode::TRAVEL_MODE_BICYCLE => "bicycle",
        TravelMode::TRAVEL_MODE_DRIVE => "auto",
        TravelMode::TRAVEL_MODE_TRANSIT => "multimodal",
        _ => "pedestrian",
    }
}

/// Turns a Valhalla answer into a route (legs joined, polyline re-encoded at precision 5).
fn from_valhalla(response: ValhallaResponse) -> Option<Route> {
    let mut line: Vec<Point> = Vec::new();
    let mut steps = Vec::new();
    for leg in response.trip.legs {
        let shape = math::decode_polyline(&leg.shape, 6);
        // Consecutive legs share their joint.
        let skip = usize::from(!line.is_empty() && line.last() == shape.first());
        line.extend(shape.into_iter().skip(skip));
        steps.extend(leg.maneuvers.into_iter().map(|maneuver| DirectionStep {
            instruction: maneuver.instruction,
            distance_meters: maneuver.length * 1000.0,
            duration_seconds: maneuver.time.round() as i32,
            ..Default::default()
        }));
    }
    if line.len() < 2 {
        return None;
    }
    Some(Route {
        distance_m: response.trip.summary.length * 1000.0,
        duration_s: response.trip.summary.time.round() as i32,
        polyline: math::encode_polyline(&line),
        provider: "valhalla",
        steps,
    })
}

impl Geo {
    /// A route between two points. Falls back to the estimate when no
    /// routing engine is configured, or when it fails or finds no route.
    pub(crate) async fn route(&self, origin: Point, destination: Point, mode: TravelMode, ja: bool) -> Route {
        if self.cfg.routing_url.is_empty() {
            return estimate(origin, destination, mode, ja);
        }
        match self.valhalla_route(origin, destination, mode, ja).await {
            Ok(Some(route)) => route,
            Ok(None) => estimate(origin, destination, mode, ja),
            Err(e) => {
                tracing::warn!(error = %e, "routing engine failed; using an estimate");
                estimate(origin, destination, mode, ja)
            }
        }
    }

    async fn valhalla_route(&self, origin: Point, destination: Point, mode: TravelMode, ja: bool) -> AppResult<Option<Route>> {
        let url = format!("{}/route", self.cfg.routing_url.trim_end_matches('/'));
        let body = serde_json::json!({
            "locations": [{ "lat": origin.0, "lon": origin.1 }, { "lat": destination.0, "lon": destination.1 }],
            "costing": costing(mode),
            "units": "kilometers",
            "language": if ja { "ja-JP" } else { "en-US" },
        });
        let response = self.core.http.post(url).json(&body).timeout(Duration::from_secs(8)).send().await?;
        if response.status().is_client_error() {
            // "No route found" and the like: not an outage.
            return Ok(None);
        }
        let parsed: ValhallaResponse = response.error_for_status()?.json().await?;
        Ok(from_valhalla(parsed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIBUYA: Point = (35.6595, 139.7005);
    const TOKYO_STATION: Point = (35.681236, 139.767125);

    #[test]
    fn estimates_use_the_straight_line_and_a_typical_speed() {
        let walk = estimate(SHIBUYA, TOKYO_STATION, TravelMode::TRAVEL_MODE_WALK, false);
        assert_eq!(walk.provider, "estimate");
        assert!((6300.0..6500.0).contains(&walk.distance_m), "{}", walk.distance_m);
        // 6.4 km × 1.3 at 4.9 km/h: about an hour and three quarters.
        assert!((6000..6400).contains(&walk.duration_s), "{}", walk.duration_s);
        let line = math::decode_polyline(&walk.polyline, 5);
        assert!(math::distance(line[0], SHIBUYA) < 2.0 && math::distance(*line.last().unwrap(), TOKYO_STATION) < 2.0);
        assert_eq!(walk.steps.len(), 1);
        assert_eq!(walk.steps[0].duration_seconds, walk.duration_s);

        let drive = estimate(SHIBUYA, TOKYO_STATION, TravelMode::TRAVEL_MODE_DRIVE, true);
        let bike = estimate(SHIBUYA, TOKYO_STATION, TravelMode::TRAVEL_MODE_BICYCLE, true);
        let transit = estimate(SHIBUYA, TOKYO_STATION, TravelMode::TRAVEL_MODE_TRANSIT, true);
        assert!(drive.duration_s < transit.duration_s && transit.duration_s < bike.duration_s && bike.duration_s < walk.duration_s);
        assert!(drive.steps[0].instruction.contains("目的地"));
    }

    #[test]
    fn long_trips_are_faster_per_kilometer() {
        assert!(typical_speed(TravelMode::TRAVEL_MODE_DRIVE, 300_000.0) > typical_speed(TravelMode::TRAVEL_MODE_DRIVE, 3_000.0));
        assert_eq!(typical_speed(TravelMode::TRAVEL_MODE_WALK, 300_000.0), typical_speed(TravelMode::TRAVEL_MODE_WALK, 300.0));
        assert_eq!(typical_duration(TravelMode::TRAVEL_MODE_TRANSIT, 0.0), 0.0);
        assert!(typical_duration(TravelMode::TRAVEL_MODE_TRANSIT, 1000.0) > TRANSIT_OVERHEAD_S);
    }

    #[test]
    fn an_estimate_to_the_same_point_is_empty_but_well_formed() {
        let here = estimate(SHIBUYA, SHIBUYA, TravelMode::TRAVEL_MODE_WALK, false);
        assert_eq!((here.distance_m, here.duration_s), (0.0, 0));
        assert_eq!(math::decode_polyline(&here.polyline, 5).len(), 2);
    }

    #[test]
    fn unspecified_mode_means_walking() {
        assert_eq!(travel_mode(TravelMode::TRAVEL_MODE_UNSPECIFIED.into()), TravelMode::TRAVEL_MODE_WALK);
        assert_eq!(travel_mode(buffa::EnumValue::from(99)), TravelMode::TRAVEL_MODE_WALK);
        assert_eq!(travel_mode(TravelMode::TRAVEL_MODE_DRIVE.into()), TravelMode::TRAVEL_MODE_DRIVE);
        assert_eq!((costing(TravelMode::TRAVEL_MODE_DRIVE), costing(TravelMode::TRAVEL_MODE_WALK)), ("auto", "pedestrian"));
    }

    #[test]
    fn valhalla_answers_become_routes() {
        // Two legs sharing a joint; shapes are precision-6 polylines.
        let first = polyline::encode_coordinates([geo::Coord { x: 139.70, y: 35.65 }, geo::Coord { x: 139.72, y: 35.66 }], 6).unwrap();
        let second = polyline::encode_coordinates([geo::Coord { x: 139.72, y: 35.66 }, geo::Coord { x: 139.75, y: 35.68 }], 6).unwrap();
        let body = serde_json::json!({
            "trip": {
                "legs": [
                    { "shape": first, "maneuvers": [{ "instruction": "北へ進む", "length": 2.1, "time": 300.4 }] },
                    { "shape": second, "maneuvers": [{ "instruction": "右折する", "length": 3.3, "time": 420.0 }, { "instruction": "目的地です", "length": 0.0, "time": 0.0 }] }
                ],
                "summary": { "length": 5.4, "time": 720.4 }
            }
        });
        let route = from_valhalla(serde_json::from_value(body).unwrap()).unwrap();
        assert_eq!((route.provider, route.duration_s), ("valhalla", 720));
        assert!((route.distance_m - 5400.0).abs() < 1e-6);
        assert_eq!(route.steps.len(), 3);
        assert_eq!((route.steps[0].instruction.as_str(), route.steps[0].duration_seconds), ("北へ進む", 300));
        assert!((route.steps[0].distance_meters - 2100.0).abs() < 1e-6);
        let line = math::decode_polyline(&route.polyline, 5);
        assert_eq!(line.len(), 3, "the shared joint appears once");
        assert!((line[2].0 - 35.68).abs() < 1e-5 && (line[2].1 - 139.75).abs() < 1e-5);

        let empty = serde_json::json!({ "trip": { "legs": [], "summary": { "length": 0.0, "time": 0.0 } } });
        assert!(from_valhalla(serde_json::from_value(empty).unwrap()).is_none());
    }
}
