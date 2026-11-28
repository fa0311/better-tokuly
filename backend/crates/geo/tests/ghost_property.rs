//! The rule behind ghost mode, as a property: **a viewer who is frozen out
//! of a person learns nothing that changes with that person's true
//! position.**
//!
//! Two worlds are run side by side. In both, Aoi freezes her pin for Ren and
//! Ren does exactly the same things (including standing where Aoi might be).
//! In one world Aoi then stays where her frozen pin is; in the other she
//! goes somewhere chosen at random and does things there: stays, checks in
//! and tags Ren, starts and ends trips, enters a new country, stands next to
//! Ren or next to their common friend Kai. After every step, everything Ren
//! can observe is collected — the answer of every RPC that could say
//! something about someone else, every realtime push to his devices, every
//! notification addressed to him, every activity entry of Aoi's that
//! friends may see — and must be the same in both worlds.
mod common;

use std::collections::HashMap;

use chrono::{DateTime, Duration, Timelike as _, Utc};
use common::*;
use rand::{RngExt as _, SeedableRng as _, rngs::StdRng};
use serde_json::{Value, json};
use tokuly_proto::geo::{GhostMode, LocationSample};
use tokuly_testkit::{FrameListener, TestUser};
use uuid::Uuid;

const STEPS: i64 = 11;
const STEP_SECONDS: i64 = 35;

/// The places things happen at, the same in both worlds of a pair.
#[derive(Clone)]
struct Spots {
    /// Where Aoi is when she freezes.
    aoi: Point,
    ren: Point,
    kai: Point,
    /// The middle of a large place Ren owns.
    trap: Point,
    /// In another (made-up) country.
    abroad: Point,
}

impl Spots {
    /// Where someone might go: by index, the same list for Aoi and for Ren.
    fn candidate(&self, index: usize) -> Point {
        [self.trap, self.kai, self.ren, self.abroad, offset(self.aoi, 0.0, 700.0)][index % 5]
    }
}

struct World {
    aoi: TestUser,
    ren: TestUser,
    kai: TestUser,
    rens_devices: FrameListener,
    trap_id: String,
    /// Aoi roams (true) or stays where her frozen pin is.
    roams: bool,
    seen_notifications: usize,
}

/// What Aoi does in the roaming world, drawn from the seed.
struct Plan {
    /// Where Aoi goes (candidate index), from step 1 on.
    aoi_goes: usize,
    /// Where Ren goes, from step 2 on, in both worlds.
    ren_goes: usize,
    /// Steps at which Aoi checks in (tagging Ren), starts a trip, ends it.
    check_in_at: i64,
    trip_at: i64,
    /// 0: to a point, 1: to Kai, 2: to Ren.
    trip_to: u8,
    trip_ends_at: i64,
}

async fn rpc(ctx: &Ctx, user: &TestUser, path: &str, request: Value) -> Value {
    match ctx.app.call::<Value, Value>(&format!("tokuly.{path}"), Some(user), &request).await {
        Ok(response) => response,
        Err(error) => json!({ "error": error.reason }),
    }
}

fn point_json(point: Point) -> Value {
    json!({ "lat": point.0, "lng": point.1 })
}

fn midday() -> (String, String) {
    let now = Utc::now();
    let shift = 12 - now.hour() as i64;
    let zone = match shift {
        0 => "Etc/GMT".to_owned(),
        s if s > 0 => format!("Etc/GMT-{s}"),
        s => format!("Etc/GMT+{}", -s),
    };
    (zone, (now + Duration::hours(shift)).date_naive().to_string())
}

async fn make_region(ctx: &Ctx, center: Point, half_deg: f64) {
    let id = format!("P{}", tokuly_core::ids::random_base62(6).to_ascii_uppercase());
    sqlx::query(
        "INSERT INTO geo_regions (id, kind, name, country_code, area_km2, center_lat, center_lng, geom)
         SELECT $1, 2, $1, $1, ST_Area(g::geography) / 1000000.0, $3, $2, g
         FROM (SELECT ST_Multi(ST_MakeEnvelope($2 - $4, $3 - $4, $2 + $4, $3 + $4, 4326)) AS g) s",
    )
    .bind(&id)
    .bind(center.1)
    .bind(center.0)
    .bind(half_deg)
    .execute(&ctx.app.core.db)
    .await
    .expect("insert region");
    ctx.geo.reload_regions();
}

impl World {
    /// Three friends, located; Ren's place and alerts; then Aoi freezes for
    /// Ren — by an override, or by a default with an exception for Kai.
    async fn start(ctx: &Ctx, spots: &Spots, roams: bool, by_default: bool, t0: DateTime<Utc>) -> World {
        let (aoi, ren, kai) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("kai").await);
        for (a, b) in [(&aoi, &ren), (&aoi, &kai), (&ren, &kai)] {
            ctx.befriend(a, b).await;
        }
        let place = rpc(
            ctx,
            &ren,
            "geo.v1.PlaceService/CreatePlace",
            json!({ "name": "見張り", "center": point_json(spots.trap), "radiusMeters": 1000.0, "visibility": "VISIBILITY_FRIENDS" }),
        )
        .await;
        let trap_id = place["place"]["id"].as_str().expect("the place").to_owned();
        let before = t0 - Duration::seconds(60);
        for (user, spot) in [(&aoi, spots.aoi), (&ren, spots.ren), (&kai, spots.kai)] {
            ctx.report(user, vec![sample(spot, before)]).await;
        }
        // Ren wants to hear when Aoi is at his place, and follows her.
        let alert = rpc(
            ctx,
            &ren,
            "geo.v1.SafetyService/CreatePlaceAlert",
            json!({ "subjectUserId": aoi.id, "center": point_json(spots.trap), "radiusMeters": 800.0, "trigger": "ALERT_TRIGGER_ARRIVE" }),
        )
        .await;
        assert!(alert["error"].is_null(), "{alert}");
        if by_default {
            ctx.ghost(&aoi, &[], GhostMode::GHOST_MODE_FROZEN, 0).await.unwrap();
            ctx.ghost(&aoi, &[&kai], GhostMode::GHOST_MODE_PRECISE, 0).await.unwrap();
        } else {
            ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_FROZEN, 0).await.unwrap();
        }
        let rens_devices = ctx.open_app(&ren).await;
        World { aoi, ren, kai, rens_devices, trap_id, roams, seen_notifications: 0 }
    }

    /// One step of the script at sample time `at`.
    async fn step(&mut self, ctx: &Ctx, spots: &Spots, plan: &Plan, step: i64, at: DateTime<Utc>) {
        let fix = |point: Point| vec![LocationSample { ..sample(point, at) }];
        // Aoi: on the spot, or on her way and then wherever she went.
        let aois = if self.roams && step >= 1 { offset(spots.candidate(plan.aoi_goes), 200.0, 6.0) } else { spots.aoi };
        ctx.report(&self.aoi, fix(aois)).await;
        // Ren and Kai do the same in both worlds.
        let rens = if step >= 2 { spots.candidate(plan.ren_goes) } else { spots.ren };
        ctx.report(&self.ren, fix(rens)).await;
        ctx.report(&self.kai, fix(spots.kai)).await;
        if step == 3 {
            // Ren sets out towards Aoi: towards where he has her.
            let trip = rpc(
                ctx,
                &self.ren,
                "geo.v1.TripService/StartTrip",
                json!({ "destination": { "userId": self.aoi.id } }),
            )
            .await;
            assert!(trip["error"].is_null(), "{trip}");
        }
        if !self.roams {
            return;
        }
        if step == plan.check_in_at {
            let checked = rpc(
                ctx,
                &self.aoi,
                "geo.v1.PlaceService/CheckIn",
                json!({ "position": point_json(aois), "withUserIds": [self.ren.id], "message": "いまここ!" }),
            )
            .await;
            assert!(checked["error"].is_null(), "{checked}");
        }
        if step == plan.trip_at {
            let destination = match plan.trip_to {
                0 => json!({ "position": point_json(offset(aois, 90.0, 2500.0)) }),
                1 => json!({ "userId": self.kai.id }),
                _ => json!({ "userId": self.ren.id }),
            };
            let trip = rpc(
                ctx,
                &self.aoi,
                "geo.v1.TripService/StartTrip",
                json!({ "destination": destination, "shareWithUserIds": [self.ren.id, self.kai.id], "notifyOnArrival": true }),
            )
            .await;
            assert!(trip["error"].is_null(), "{trip}");
        }
        if step == plan.trip_ends_at {
            let active = rpc(ctx, &self.aoi, "geo.v1.TripService/ListActiveTrips", json!({})).await;
            let own = active["trips"].as_array().and_then(|trips| {
                trips.iter().find(|trip| trip["user"]["id"] == json!(self.aoi.id)).map(|trip| trip["id"].clone())
            });
            if let Some(trip_id) = own {
                rpc(ctx, &self.aoi, "geo.v1.TripService/EndTrip", json!({ "tripId": trip_id })).await;
            }
        }
    }

    /// Everything Ren can observe right now, by name.
    async fn observe(&mut self, ctx: &Ctx, spots: &Spots) -> Vec<(String, Value)> {
        let ren = self.ren.clone();
        let (zone, date) = midday();
        let around = |center: Point| json!({ "southWest": point_json((center.0 - 0.3, center.1 - 0.3)), "northEast": point_json((center.0 + 0.3, center.1 + 0.3)) });
        let aoi_id = json!(self.aoi.id);
        let calls: Vec<(&str, Value)> = vec![
            ("geo.v1.LocationService/ListFriendLocations", json!({})),
            ("geo.v1.LocationService/ListGatherings", json!({})),
            ("geo.v1.LocationService/GetMyLocation", json!({})),
            ("geo.v1.PlaceService/ListMyPlaces", json!({})),
            ("geo.v1.PlaceService/GetPlace", json!({ "placeId": self.trap_id })),
            ("geo.v1.PlaceService/SearchPlaces", json!({ "near": point_json(spots.trap) })),
            ("geo.v1.PlaceService/ListPlaceSuggestions", json!({})),
            ("geo.v1.HistoryService/ListVisits", json!({})),
            ("geo.v1.HistoryService/GetDay", json!({ "date": date, "timeZone": zone })),
            ("geo.v1.HistoryService/ListNights", json!({})),
            (
                "geo.v1.FootprintService/GetCells",
                json!({ "userId": aoi_id, "bounds": around(spots.aoi), "zoom": 15.0 }),
            ),
            (
                "geo.v1.FootprintService/GetCells",
                json!({ "userId": aoi_id, "bounds": around(spots.abroad), "zoom": 15.0 }),
            ),
            ("geo.v1.FootprintService/GetCells", json!({ "userId": aoi_id, "bounds": around(spots.aoi), "zoom": 4.0 })),
            ("geo.v1.FootprintService/GetStats", json!({ "userId": aoi_id })),
            ("geo.v1.FootprintService/GetStats", json!({})),
            ("geo.v1.FootprintService/GetLeaderboard", json!({ "scope": "LEADERBOARD_SCOPE_FRIENDS" })),
            ("geo.v1.FootprintService/GetLeaderboard", json!({ "scope": "LEADERBOARD_SCOPE_GLOBAL" })),
            ("geo.v1.TripService/ListActiveTrips", json!({})),
            ("geo.v1.TripService/GetDirections", json!({ "destination": { "userId": aoi_id } })),
            ("geo.v1.SafetyService/ListPlaceAlerts", json!({})),
            ("geo.v1.SafetyService/GetActiveSos", json!({})),
            ("identity.v1.ProfileService/GetProfile", json!({ "userId": aoi_id })),
        ];
        let mut seen = Vec::new();
        for (index, (path, request)) in calls.into_iter().enumerate() {
            seen.push((format!("{path}#{index}"), rpc(ctx, &ren, path, request).await));
        }
        // Realtime: whatever reached his devices since the last look.
        let frames: Vec<Value> = pushes(&mut self.rens_devices, std::time::Duration::from_millis(250))
            .await
            .into_iter()
            .map(|push| serde_json::to_value(push).expect("a push as JSON"))
            .collect();
        seen.push(("pushes".into(), Value::Array(frames)));
        // Notifications addressed to him.
        let all = ctx.app.events("notify.requested").await;
        let mine: Vec<Value> =
            all.iter().filter(|event| event.data["user_id"] == json!(ren.id)).map(|event| event.data.clone()).collect();
        seen.push(("notifications".into(), Value::Array(mine[self.seen_notifications.min(mine.len())..].to_vec())));
        self.seen_notifications = mine.len();
        // Aoi's activity entries that friends may see.
        let stories: Vec<Value> = ctx
            .app
            .events_by("activity.recorded", self.aoi.id)
            .await
            .into_iter()
            .filter(|event| event.data["audience"] != "private")
            .map(|event| event.data)
            .collect();
        seen.push(("activity".into(), Value::Array(stories)));
        seen
    }

    /// Replaces what differs between the worlds by construction: the three
    /// accounts by their roles, other IDs by the order they appear in, and
    /// times the server stamped with its own clock.
    fn normalize(&self, value: &Value, ids: &mut HashMap<String, String>) -> Value {
        const SERVER_CLOCK: &[&str] = &[
            "serverTime",
            "friendsSince",
            "createdAt",
            "updatedAt",
            "expiresAt",
            "respondedAt",
            "eta",
            "setAt",
            "durationSeconds",
            "created_at",
            "joinedAt",
            "lastSeenAt",
        ];
        match value {
            Value::Object(fields) => {
                // A trip carries the moment it was started and ended (server time).
                let trip = fields.contains_key("distanceRemainingMeters");
                Value::Object(
                    fields
                        .iter()
                        .filter(|(key, _)| !SERVER_CLOCK.contains(&key.as_str()))
                        .filter(|(key, _)| !(trip && matches!(key.as_str(), "startedAt" | "endedAt")))
                        .map(|(key, value)| (key.clone(), self.normalize(value, ids)))
                        .collect(),
                )
            }
            Value::Array(items) => {
                let mut items: Vec<Value> = items.iter().map(|item| self.normalize(item, ids)).collect();
                // Lists of people (the friends' pins) come in the order of the friend list,
                // which the server does not define (an unordered query and a cached set), so
                // the two worlds may list the same pins in a different order. That order says
                // nothing about anyone's position; compare the lists by person instead.
                if items.iter().all(|item| item.get("userId").is_some_and(Value::is_string)) {
                    items.sort_by(|a, b| a["userId"].as_str().cmp(&b["userId"].as_str()));
                }
                Value::Array(items)
            }
            Value::String(text) => Value::String(self.normalize_text(text, ids)),
            other => other.clone(),
        }
    }

    fn normalize_text(&self, text: &str, ids: &mut HashMap<String, String>) -> String {
        let mut out = text.to_owned();
        for (user, role) in [(&self.aoi, "<aoi>"), (&self.ren, "<ren>"), (&self.kai, "<kai>")] {
            out = out.replace(&user.id.to_string(), role).replace(&user.handle, role);
        }
        // Any other UUID: numbered by first appearance.
        let mut at = 0;
        while at + 36 <= out.len() {
            if out.is_char_boundary(at) && out.is_char_boundary(at + 36) && Uuid::parse_str(&out[at..at + 36]).is_ok() {
                let next = format!("<id {}>", ids.len() + 1);
                let token = ids.entry(out[at..at + 36].to_owned()).or_insert(next).clone();
                out.replace_range(at..at + 36, &token);
                at += token.len();
            } else {
                at += 1;
            }
        }
        out
    }
}

async fn frozen_out_viewer_learns_nothing(seed: u64, by_default: bool) {
    let ctx = start().await;
    let mut rng = StdRng::seed_from_u64(seed * 2 + u64::from(by_default));
    let home = (rng.random_range(-50.0..-30.0), rng.random_range(-130.0..-100.0));
    let spots = Spots {
        aoi: home,
        ren: offset(home, 90.0, 3000.0),
        kai: offset(home, 180.0, 3500.0),
        trap: offset(home, 0.0, 5000.0),
        abroad: (home.0 + 0.9, home.1),
    };
    make_region(&ctx, home, 0.4).await;
    make_region(&ctx, spots.abroad, 0.3).await;
    let plan = Plan {
        // Seed 0 puts Ren exactly where Aoi goes: the prober guessed right.
        aoi_goes: if seed == 0 { 0 } else { rng.random_range(0..5) },
        ren_goes: if seed == 0 { 0 } else { rng.random_range(0..5) },
        check_in_at: rng.random_range(2..6),
        trip_at: rng.random_range(2..6),
        trip_to: rng.random_range(0..3),
        trip_ends_at: rng.random_range(7..10),
    };
    // The whole script lies within the minutes that count as "now".
    let t0 = Utc::now() - Duration::seconds(240);
    let mut still = World::start(&ctx, &spots, false, by_default, t0).await;
    let mut roaming = World::start(&ctx, &spots, true, by_default, t0).await;
    let (mut still_ids, mut roaming_ids) = (HashMap::new(), HashMap::new());
    for step in 0..STEPS {
        let at = t0 + Duration::seconds(step * STEP_SECONDS);
        still.step(&ctx, &spots, &plan, step, at).await;
        roaming.step(&ctx, &spots, &plan, step, at).await;
        let (a, b) = (still.observe(&ctx, &spots).await, roaming.observe(&ctx, &spots).await);
        for ((name, seen_still), (_, seen_roaming)) in a.iter().zip(&b) {
            let (left, right) =
                (still.normalize(seen_still, &mut still_ids), roaming.normalize(seen_roaming, &mut roaming_ids));
            assert!(
                left == right,
                "seed {seed}, step {step}, {name}: what Ren observes depends on where Aoi really is\n(Aoi went to spot {}, Ren to spot {})\nwith Aoi still:   {left}\nwith Aoi roaming: {right}",
                plan.aoi_goes,
                plan.ren_goes
            );
        }
    }
    // The roaming Aoi really did all that: her own view is not the frozen one.
    let own = rpc(&ctx, &roaming.aoi, "geo.v1.LocationService/GetMyLocation", json!({})).await;
    let truth =
        (own["location"]["position"]["lat"].as_f64().unwrap(), own["location"]["position"]["lng"].as_f64().unwrap());
    assert!(distance(truth, spots.aoi) > 500.0);
    // ...and Kai, who sees her precisely, saw it.
    let kais = rpc(&ctx, &roaming.kai, "geo.v1.LocationService/ListFriendLocations", json!({})).await;
    let seen_by_kai =
        kais["locations"].as_array().unwrap().iter().find(|pin| pin["userId"] == json!(roaming.aoi.id)).unwrap();
    assert!((seen_by_kai["position"]["lat"].as_f64().unwrap() - truth.0).abs() < 1e-9);
}

#[tokio::test]
async fn frozen_by_an_override_the_prober_stands_where_she_is() {
    frozen_out_viewer_learns_nothing(0, false).await;
}

#[tokio::test]
async fn frozen_by_an_override_random_walks() {
    for seed in [1, 2, 3] {
        frozen_out_viewer_learns_nothing(seed, false).await;
    }
}

#[tokio::test]
async fn frozen_by_default_the_prober_stands_where_she_is() {
    frozen_out_viewer_learns_nothing(0, true).await;
}

#[tokio::test]
async fn frozen_by_default_random_walks() {
    for seed in [4, 5, 6] {
        frozen_out_viewer_learns_nothing(seed, true).await;
    }
}
