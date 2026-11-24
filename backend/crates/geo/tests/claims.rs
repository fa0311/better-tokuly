//! Positions claimed for the social domain's nearby discovery and bumps are
//! checked against the account's own fixes (`tokuly_geo::claims`).
//!
//! The first tests check the check: no fix, a mock fix, a jump, a late
//! upload. The property at the end is the ghost-mode rule applied to it:
//! **what the social domain tells someone else about a claimer does not
//! depend on where the claimer really is**, beyond whether the claim is
//! accepted at all. Two worlds are run side by side; in both, Aoi claims the
//! same spot and Ren (a stranger, or a friend Aoi is frozen for) does the
//! same things next to it. Only Aoi's true position differs: in one world
//! she is at the spot, in the other a few dozen meters off it (still
//! plausible) — or, for the refused pair, far away in both, by different
//! distances. Aoi probes with a series of claims around the spot (the
//! add-friend screen and a shake at each). Everything Ren can observe (his
//! nearby stream and shake at every probe, his bumps, pushes,
//! notifications, Aoi's profile) must be the same in both.
//! Ren stands 8 m from the claim, "within 10 m"; Aoi's true position is
//! at least 27 m from him in the off-spot world, so an answer computed
//! from her true position would show.
mod common;

use std::collections::HashMap;

use common::*;
use rand::{RngExt as _, SeedableRng as _, rngs::StdRng};
use serde_json::{Value, json};
use tokuly_proto::{
    geo::{GhostMode, LocationSample},
    social::{BumpRequest, BumpResponse, DiscoverNearbyRequest, DiscoverNearbyResponse, SocialEvent},
};
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

const NEARBY: &str = "tokuly.social.v1.FriendService/DiscoverNearby";
const BUMP: &str = "tokuly.social.v1.InteractionService/Bump";
/// Where Aoi claims to be, one probe after the other: (bearing, meters) from the spot.
const PROBES: [(f64, f64); 3] = [(0.0, 0.0), (90.0, 6.0), (200.0, 9.0)];

fn nearby_at(spot: Point) -> DiscoverNearbyRequest {
    DiscoverNearbyRequest { location: lat_lng(spot).into(), accuracy_meters: 5.0, ..Default::default() }
}

async fn open_nearby(ctx: &Ctx, user: &TestUser, spot: Point) -> Result<Vec<DiscoverNearbyResponse>, RpcError> {
    ctx.app
        .stream::<_, DiscoverNearbyResponse>(NEARBY, Some(user), &nearby_at(spot), 1, std::time::Duration::from_secs(3))
        .await
}

async fn shake(ctx: &Ctx, user: &TestUser, spot: Point) -> Result<BumpResponse, RpcError> {
    let request =
        BumpRequest { location: lat_lng(spot).into(), accuracy_meters: 5.0, intensity: 2.5, ..Default::default() };
    ctx.app.call(BUMP, Some(user), &request).await
}

/// Both claims at once: the error reason of each, `None` when it was let through.
async fn claim_reasons(ctx: &Ctx, user: &TestUser, spot: Point) -> (Option<String>, Option<String>) {
    let nearby = open_nearby(ctx, user, spot).await.err().map(|e| e.reason);
    let bump = shake(ctx, user, spot).await.err().map(|e| e.reason);
    (nearby, bump)
}

/// Both claims at once, as whether each was let through.
async fn claims_pass(ctx: &Ctx, user: &TestUser, spot: Point) -> (bool, bool) {
    let verdict = |reason: Option<String>| match reason.as_deref() {
        None => true,
        Some("LOCATION_NOT_VERIFIED") => false,
        Some(other) => panic!("unexpected error {other}"),
    };
    let (nearby, bump) = claim_reasons(ctx, user, spot).await;
    (verdict(nearby), verdict(bump))
}

fn mocked(sample: LocationSample) -> LocationSample {
    LocationSample { mocked: true, ..sample }
}

/// Walks `user` from `from` to `to` in fixes about 60 m apart, one upload each.
async fn walk(ctx: &Ctx, user: &TestUser, from: Point, to: Point) {
    let steps = (distance(from, to) / 60.0).ceil().max(1.0) as usize;
    for step in 1..=steps {
        let f = step as f64 / steps as f64;
        ctx.at(user, (from.0 + (to.0 - from.0) * f, from.1 + (to.1 - from.1) * f)).await;
    }
}

#[tokio::test]
async fn claims_follow_the_accounts_own_real_fixes() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    let spot = somewhere();

    assert_eq!(claims_pass(&ctx, &aoi, spot).await, (false, false), "no fix at all");

    ctx.at(&aoi, spot).await;
    assert_eq!(claims_pass(&ctx, &aoi, spot).await, (true, true));
    assert_eq!(claims_pass(&ctx, &aoi, offset(spot, 90.0, 40.0)).await, (true, true), "GPS wanders a little");
    assert_eq!(claims_pass(&ctx, &aoi, offset(spot, 90.0, 1000.0)).await, (false, false), "a kilometer off");

    // A fix 20 km away moments later is no journey, nor are several in one upload: the real fix stays.
    let far = offset(spot, 0.0, 20_000.0);
    ctx.report(&aoi, vec![sample(far, next_fix_time(aoi.id))]).await;
    ctx.report(&aoi, (0..5).map(|_| sample(far, next_fix_time(aoi.id))).collect()).await;
    assert_eq!(claims_pass(&ctx, &aoi, far).await, (false, false), "a jump does not move the real position");
    assert_eq!(claims_pass(&ctx, &aoi, spot).await, (true, true));

    // A mock location: nothing is plausible until a real fix comes, and that must follow from the real one.
    ctx.report(&aoi, vec![mocked(sample(spot, next_fix_time(aoi.id)))]).await;
    assert_eq!(claims_pass(&ctx, &aoi, spot).await, (false, false), "the device mocks its location");
    ctx.at(&aoi, far).await;
    assert_eq!(claims_pass(&ctx, &aoi, far).await, (false, false), "no jump after a mock either");
    ctx.at(&aoi, spot).await;
    assert_eq!(claims_pass(&ctx, &aoi, spot).await, (true, true));

    // Walking at a walking pace moves the real fix along.
    let there = offset(spot, 45.0, 400.0);
    walk(&ctx, &aoi, spot, there).await;
    assert_eq!(claims_pass(&ctx, &aoi, there).await, (true, true));
    assert_eq!(claims_pass(&ctx, &aoi, spot).await, (false, false), "400 m back is too far");

    // Approximate location only: told apart, so the app can ask for precise location.
    let ren = ctx.app.user("ren").await;
    let elsewhere = somewhere();
    let coarse = LocationSample { accuracy_meters: 800.0, ..sample(elsewhere, next_fix_time(ren.id)) };
    ctx.report(&ren, vec![coarse]).await;
    let too_coarse = Some("LOCATION_TOO_COARSE".to_owned());
    assert_eq!(claim_reasons(&ctx, &ren, elsewhere).await, (too_coarse.clone(), too_coarse), "accuracy 800 m");
    ctx.at(&ren, elsewhere).await;
    assert_eq!(claims_pass(&ctx, &ren, elsewhere).await, (true, true), "precise again");

    // In a backlog upload only the newest sample is the present.
    let kai = ctx.app.user("kai").await;
    let (then, now) = (somewhere(), somewhere());
    ctx.report(&kai, vec![sample(then, minutes_ago(30)), sample(now, seconds_ago(1))]).await;
    assert_eq!(claims_pass(&ctx, &kai, then).await, (false, false), "a fix from half an hour ago");
    assert_eq!(claims_pass(&ctx, &kai, now).await, (true, true));
}

#[tokio::test]
async fn a_slow_device_clock_does_not_lock_anyone_out() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    let spot = somewhere();
    // The phone's clock is six minutes behind: every fix it sends looks six minutes old.
    ctx.report(&aoi, vec![sample(spot, minutes_ago(6))]).await;
    assert_eq!(claims_pass(&ctx, &aoi, spot).await, (true, true));
    let step = offset(spot, 0.0, 50.0);
    ctx.report(&aoi, vec![sample(step, minutes_ago(6) + chrono::Duration::seconds(5))]).await;
    assert_eq!(claims_pass(&ctx, &aoi, offset(step, 0.0, 40.0)).await, (true, true), "the fix moved along");
}

#[tokio::test]
async fn claims_are_checked_against_the_latest_fix_not_the_pin_shown_to_friends() {
    // A frozen pin is what Ren sees; Aoi's claims are checked against where she is.
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let frozen_at = somewhere();
    let now_at = offset(frozen_at, 90.0, 400.0);
    ctx.at(&aoi, frozen_at).await;
    ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_FROZEN, 0).await.expect("freeze");
    walk(&ctx, &aoi, frozen_at, now_at).await;
    assert_eq!(claims_pass(&ctx, &aoi, now_at).await, (true, true));
    assert_eq!(claims_pass(&ctx, &aoi, frozen_at).await, (false, false));
}

// ─────────────────────────── the property ───────────────────────────

/// How the two worlds of a pair differ, and what they share.
#[derive(Clone, Copy)]
struct Plan {
    /// Aoi and Ren are friends and Aoi is frozen for Ren (otherwise strangers).
    friends: bool,
    /// Aoi's claim is plausible in both worlds (otherwise refused in both).
    plausible: bool,
    /// Where Ren stands, relative to the claim.
    ren_bearing: f64,
    /// Aoi's true position in each world, relative to the claim.
    aoi_truth: [(f64, f64); 2],
}

impl Plan {
    fn draw(seed: u64, friends: bool, plausible: bool) -> Plan {
        let mut rng = StdRng::seed_from_u64(seed);
        let aoi_truth = if plausible {
            // At the spot, or 35 to 50 m off it: farther from Ren than his band.
            [(0.0, 0.0), (rng.random_range(0.0..360.0), rng.random_range(35.0..50.0))]
        } else {
            [
                (rng.random_range(0.0..360.0), rng.random_range(500.0..3_000.0)),
                (rng.random_range(0.0..360.0), rng.random_range(50_000.0..400_000.0)),
            ]
        };
        Plan { friends, plausible, ren_bearing: rng.random_range(0.0..360.0), aoi_truth }
    }
}

struct World {
    aoi: TestUser,
    ren: TestUser,
    /// Where Aoi claims to be; everything Ren sees is compared relative to it.
    claim: Point,
}

impl World {
    async fn run(ctx: &Ctx, plan: Plan, world: usize) -> (World, Vec<(String, Value)>) {
        let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
        let claim = somewhere();
        let ren_at = offset(claim, plan.ren_bearing, 8.0);
        let (bearing, meters) = plan.aoi_truth[world];
        let truth = offset(claim, bearing, meters);
        let me = World { aoi, ren, claim };
        let (aoi, ren) = (&me.aoi, &me.ren);

        if plan.friends {
            ctx.befriend(aoi, ren).await;
            // Frozen for Ren before she ever reported anything: there is no pin he could compare.
            ctx.ghost(aoi, &[ren], GhostMode::GHOST_MODE_FROZEN, 0).await.expect("freeze");
        }
        let mut rens_devices = ctx.app.listen_user(ren.id).await;
        ctx.at(ren, ren_at).await;
        ctx.at(aoi, truth).await;

        // Aoi probes: a series of claims around the spot, each with the
        // add-friend screen and a shake, while Ren looks and shakes too.
        let mut seen = Vec::new();
        for (index, (bearing, meters)) in PROBES.into_iter().enumerate() {
            let probe = offset(claim, bearing, meters);
            let rens_request = nearby_at(ren_at);
            let rens_stream =
                ctx.app.stream::<_, Value>(NEARBY, Some(ren), &rens_request, 2, std::time::Duration::from_secs(4));
            let aoi_joins = async {
                tokio::time::sleep(std::time::Duration::from_millis(700)).await;
                ctx.app
                    .stream::<_, Value>(NEARBY, Some(aoi), &nearby_at(probe), 2, std::time::Duration::from_secs(2))
                    .await
            };
            let (rens_snapshots, aoi_joined) = tokio::join!(rens_stream, aoi_joins);
            assert_eq!(aoi_joined.is_ok(), plan.plausible, "Aoi's session {index}: {:?}", aoi_joined.err());
            seen.push((format!("nearby#{index}"), Value::Array(rens_snapshots.expect("Ren's session"))));

            // Both shake: Aoi first, Ren right after.
            let aoi_shook = shake(ctx, aoi, probe).await;
            assert_eq!(aoi_shook.is_ok(), plan.plausible, "Aoi's shake {index}: {:?}", aoi_shook.err());
            let request =
                json!({ "location": { "lat": ren_at.0, "lng": ren_at.1 }, "accuracyMeters": 5.0, "intensity": 2.5 });
            let rens_shake = match ctx.app.call::<_, Value>(BUMP, Some(ren), &request).await {
                Ok(response) => response,
                Err(error) => json!({ "error": error.reason }),
            };
            seen.push((format!("shake#{index}"), rens_shake));
        }
        for (path, request) in [
            ("tokuly.social.v1.InteractionService/ListBumps", json!({})),
            (
                "tokuly.social.v1.FriendService/ListFriendRequests",
                json!({ "direction": "FRIEND_REQUEST_DIRECTION_INCOMING" }),
            ),
            ("tokuly.identity.v1.ProfileService/GetProfile", json!({ "userId": aoi.id })),
        ] {
            let answer = match ctx.app.call::<Value, Value>(path, Some(ren), &request).await {
                Ok(response) => response,
                Err(error) => json!({ "error": error.reason }),
            };
            seen.push((path.to_owned(), answer));
        }
        let pushes: Vec<Value> = rens_devices
            .events::<SocialEvent>(std::time::Duration::from_millis(500))
            .await
            .into_iter()
            .map(|event| serde_json::to_value(event).expect("a push as JSON"))
            .collect();
        seen.push(("pushes".into(), Value::Array(pushes)));
        let notifications: Vec<Value> = ctx
            .app
            .events("notify.requested")
            .await
            .into_iter()
            .filter(|event| event.data["user_id"] == json!(ren.id))
            .map(|event| event.data)
            .collect();
        seen.push(("notifications".into(), Value::Array(notifications)));
        (me, seen)
    }

    /// Replaces what differs between the worlds by construction: the two
    /// accounts by their roles, other IDs by order of appearance, server
    /// times, and positions by where they are relative to the claim.
    fn normalize(&self, value: &Value, ids: &mut HashMap<String, String>) -> Value {
        const SERVER_CLOCK: &[&str] = &[
            "createdAt",
            "updatedAt",
            "created_at",
            "friendsSince",
            "lastSeenAt",
            "recordedAt",
            "joinedAt",
            "serverTime",
        ];
        match value {
            Value::Object(fields) if fields.contains_key("lat") && fields.contains_key("lng") => {
                let (lat, lng) =
                    (fields["lat"].as_f64().unwrap_or_default(), fields["lng"].as_f64().unwrap_or_default());
                let north = (lat - self.claim.0) * 111_195.0;
                let east = (lng - self.claim.1) * 111_195.0 * self.claim.0.to_radians().cos();
                json!({ "north_dm": (north * 10.0).round(), "east_dm": (east * 10.0).round() })
            }
            Value::Object(fields) => Value::Object(
                fields
                    .iter()
                    .filter(|(key, _)| !SERVER_CLOCK.contains(&key.as_str()))
                    .map(|(key, value)| (key.clone(), self.normalize(value, ids)))
                    .collect(),
            ),
            Value::Array(items) => Value::Array(items.iter().map(|item| self.normalize(item, ids)).collect()),
            Value::String(text) => Value::String(self.normalize_text(text, ids)),
            // Distances computed at different places of the globe differ in the last bits.
            Value::Number(number) if number.is_f64() => {
                json!((number.as_f64().unwrap_or_default() * 10.0).round() / 10.0)
            }
            other => other.clone(),
        }
    }

    fn normalize_text(&self, text: &str, ids: &mut HashMap<String, String>) -> String {
        let mut out = text.to_owned();
        for (user, role) in [(&self.aoi, "<aoi>"), (&self.ren, "<ren>")] {
            out = out
                .replace(&user.id.to_string(), role)
                .replace(&user.id.simple().to_string(), role)
                .replace(&user.handle, role);
        }
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

async fn others_learn_nothing_from_the_true_position(seed: u64, friends: bool, plausible: bool) {
    let ctx = start().await;
    let plan = Plan::draw(seed, friends, plausible);
    let ((at_spot, seen_a), (off_spot, seen_b)) = tokio::join!(World::run(&ctx, plan, 0), World::run(&ctx, plan, 1));
    let (mut ids_a, mut ids_b) = (HashMap::new(), HashMap::new());
    for ((what, a), (_, b)) in seen_a.iter().zip(&seen_b) {
        let (a, b) = (at_spot.normalize(a, &mut ids_a), off_spot.normalize(b, &mut ids_b));
        assert_eq!(
            a, b,
            "seed {seed}, friends {friends}, plausible {plausible}: Ren's {what} differs between the worlds"
        );
    }
    // The worlds did what they should have: Ren met Aoi exactly when her claim passed.
    let nearby = &seen_a[0].1;
    let met = nearby
        .as_array()
        .is_some_and(|snapshots| snapshots.iter().any(|s| s["users"].as_array().is_some_and(|u| !u.is_empty())));
    assert_eq!(met, plausible, "{nearby}");
    if plausible {
        let band = nearby[1]["users"][0]["distanceMeters"].as_f64();
        assert_eq!(band, Some(10.0), "the band is from the claim, 8 m away");
        assert_eq!(seen_a[1].1["bumps"].as_array().map(Vec::len), Some(1), "{}", seen_a[1].1);
    }
}

#[tokio::test]
async fn strangers_learn_nothing_from_where_a_plausible_claimer_really_is() {
    for seed in [1, 2] {
        others_learn_nothing_from_the_true_position(seed, false, true).await;
    }
}

#[tokio::test]
async fn a_frozen_out_friend_learns_nothing_from_where_a_plausible_claimer_really_is() {
    for seed in [3, 4] {
        others_learn_nothing_from_the_true_position(seed, true, true).await;
    }
}

#[tokio::test]
async fn nobody_learns_how_far_off_a_refused_claim_was() {
    for (seed, friends) in [(5, false), (6, true)] {
        others_learn_nothing_from_the_true_position(seed, friends, false).await;
    }
}
