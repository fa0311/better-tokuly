//! Shared setup and helpers for the geo integration tests.
#![allow(dead_code)] // each test binary uses its own subset

use std::time::Duration;

use chrono::{DateTime, Utc};
use rand::RngExt as _;
use tokuly_core::events::Event;
use tokuly_geo::{Geo, GeoConfig, math};
use tokuly_identity::settings;
use tokuly_proto::{
    common::LatLng,
    geo::{geo_event::Event as Push, *},
    identity::{PrivacySettings, UpdateSettingsRequest, UpdateSettingsResponse},
    social::{
        FriendSource, RemoveFriendRequest, RemoveFriendResponse, RespondFriendRequestRequest,
        RespondFriendRequestResponse, SendFriendRequestRequest, SendFriendRequestResponse, send_friend_request_request,
    },
};
use tokuly_social::{Social, SocialConfig};
use tokuly_testkit::{FrameListener, RpcError, TestApp, TestUser};
use uuid::Uuid;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0200", "0300", "0400"];

/// How long to wait for a realtime frame that should arrive (generous: the
/// machine may be busy compiling something else).
pub const SOON: Duration = Duration::from_secs(5);
/// How long to listen when asserting that nothing arrives.
pub const QUIET: Duration = Duration::from_millis(400);

pub type Point = (f64, f64);

/// A running test app plus the domain under test (for its worker entry points).
pub struct Ctx {
    pub app: TestApp,
    pub geo: Geo,
    pub social: Social,
}

pub async fn start() -> Ctx {
    start_with(GeoConfig::default()).await
}

pub async fn start_with(cfg: GeoConfig) -> Ctx {
    let mut domains = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        let social = Social::new(core.clone(), identity.clone(), SocialConfig::default()).expect("social config");
        let geo = Geo::new(core.clone(), identity.clone(), cfg, &social).expect("geo config");
        let router = geo.register(social.register(identity.register(router)));
        domains = Some((geo, social));
        router
    })
    .await;
    let (geo, social) = domains.expect("register ran");
    Ctx { app, geo, social }
}

impl Ctx {
    /// `user` opens the app: online, with a connection that receives pushes.
    /// Pins are pushed only to friends with the app open.
    pub async fn open_app(&self, user: &TestUser) -> FrameListener {
        tokuly_core::presence::mark_online(&self.app.core.cache, user.id).await.unwrap();
        self.app.listen_user(user.id).await
    }

    /// Calls an RPC of `tokuly.geo.v1.<service>`.
    pub async fn rpc<Req: serde::Serialize, Resp: serde::de::DeserializeOwned>(
        &self,
        service: &str,
        method: &str,
        user: &TestUser,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.app.call(&format!("tokuly.geo.v1.{service}/{method}"), Some(user), request).await
    }

    pub async fn location<Req: serde::Serialize, Resp: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        user: &TestUser,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.rpc("LocationService", method, user, request).await
    }

    pub async fn places<Req: serde::Serialize, Resp: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        user: &TestUser,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.rpc("PlaceService", method, user, request).await
    }

    pub async fn history<Req: serde::Serialize, Resp: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        user: &TestUser,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.rpc("HistoryService", method, user, request).await
    }

    pub async fn footprints<Req: serde::Serialize, Resp: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        user: &TestUser,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.rpc("FootprintService", method, user, request).await
    }

    pub async fn trips<Req: serde::Serialize, Resp: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        user: &TestUser,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.rpc("TripService", method, user, request).await
    }

    pub async fn safety<Req: serde::Serialize, Resp: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        user: &TestUser,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.rpc("SafetyService", method, user, request).await
    }

    /// Makes two users friends through the social API (`a` asks, `b` accepts).
    pub async fn befriend(&self, a: &TestUser, b: &TestUser) {
        let request = SendFriendRequestRequest {
            target: Some(send_friend_request_request::Target::UserId(b.id.to_string())),
            source: FriendSource::FRIEND_SOURCE_PROFILE.into(),
            ..Default::default()
        };
        let sent: SendFriendRequestResponse = self
            .app
            .call("tokuly.social.v1.FriendService/SendFriendRequest", Some(a), &request)
            .await
            .expect("send friend request");
        let Some(request) = sent.request.as_option() else { return };
        let _: RespondFriendRequestResponse = self
            .app
            .call(
                "tokuly.social.v1.FriendService/RespondFriendRequest",
                Some(b),
                &RespondFriendRequestRequest { request_id: request.id.clone(), accept: true, ..Default::default() },
            )
            .await
            .expect("accept");
    }

    /// Ends a friendship through the social API and delivers the event the geo domain would consume.
    pub async fn unfriend(&self, a: &TestUser, b: &TestUser) {
        let _: RemoveFriendResponse = self
            .app
            .call(
                "tokuly.social.v1.FriendService/RemoveFriend",
                Some(a),
                &RemoveFriendRequest { user_id: b.id.to_string(), ..Default::default() },
            )
            .await
            .expect("remove friend");
        self.deliver(
            "social.friend.removed",
            a.id,
            serde_json::json!({ "user_id": a.id, "friend_id": b.id, "blocked": false }),
        )
        .await;
    }

    /// Delivers an event to the geo domain's consumer (no relay runs in tests).
    pub async fn deliver(&self, topic: &str, actor: Uuid, data: serde_json::Value) {
        let event =
            Event { id: Uuid::now_v7(), topic: topic.to_owned(), actor_id: Some(actor), occurred_at: Utc::now(), data };
        self.geo.handle_event(&event).await.unwrap_or_else(|e| panic!("handling {topic}: {e}"));
    }

    /// Changes a user's privacy settings through the identity API and delivers the resulting event.
    pub async fn set_privacy(&self, user: &TestUser, change: impl FnOnce(&mut PrivacySettings)) {
        let mut privacy = settings::privacy_of(&self.app.core, user.id).await.expect("privacy");
        change(&mut privacy);
        let _: UpdateSettingsResponse = self
            .app
            .call(
                "tokuly.identity.v1.AccountService/UpdateSettings",
                Some(user),
                &UpdateSettingsRequest { privacy: privacy.into(), ..Default::default() },
            )
            .await
            .expect("update settings");
        self.deliver("identity.user.privacy_changed", user.id, serde_json::json!({ "user_id": user.id })).await;
    }

    /// Uploads samples through `ReportLocations`.
    pub async fn report(&self, user: &TestUser, samples: Vec<LocationSample>) -> ReportLocationsResponse {
        self.location("ReportLocations", user, &ReportLocationsRequest { samples, ..Default::default() })
            .await
            .expect("report locations")
    }

    /// One good fix at `point`, right now.
    pub async fn at(&self, user: &TestUser, point: Point) {
        let response = self.report(user, vec![sample(point, next_fix_time(user.id))]).await;
        assert_eq!(response.accepted_count, 1, "the sample was accepted");
    }

    /// A fix at `point` every 30 seconds from `from` to `to` (inclusive).
    pub async fn stay(&self, user: &TestUser, point: Point, from: DateTime<Utc>, to: DateTime<Utc>) {
        let mut samples = Vec::new();
        let mut at = from;
        while at <= to {
            samples.push(sample(point, at));
            at += chrono::Duration::seconds(30);
        }
        // (Fixes less than a second apart are dropped by the pipeline.)
        let last = samples.last().and_then(|s| s.recorded_at.as_option().map(tokuly_core::time::from_ts));
        if last.is_none_or(|last| to - last >= chrono::Duration::seconds(1)) {
            samples.push(sample(point, to));
        }
        let sent = samples.len() as i32;
        assert_eq!(self.report(user, samples).await.accepted_count, sent, "every sample of the stay was accepted");
    }

    pub async fn friend_locations(&self, viewer: &TestUser) -> Vec<FriendLocation> {
        let response: ListFriendLocationsResponse = self
            .location("ListFriendLocations", viewer, &ListFriendLocationsRequest::default())
            .await
            .expect("list friend locations");
        response.locations
    }

    /// How `viewer` sees `owner` on the friend map, if at all.
    pub async fn seen_by(&self, viewer: &TestUser, owner: &TestUser) -> Option<FriendLocation> {
        self.friend_locations(viewer).await.into_iter().find(|location| location.user_id == owner.id.to_string())
    }

    pub async fn my_location(&self, user: &TestUser) -> GetMyLocationResponse {
        self.location("GetMyLocation", user, &GetMyLocationRequest::default()).await.expect("get my location")
    }

    pub async fn ghost(
        &self,
        user: &TestUser,
        viewers: &[&TestUser],
        mode: GhostMode,
        duration_seconds: i32,
    ) -> Result<GetGhostSettingsResponse, RpcError> {
        let request = SetGhostModeRequest {
            user_ids: viewers.iter().map(|viewer| viewer.id.to_string()).collect(),
            mode: mode.into(),
            duration_seconds,
            ..Default::default()
        };
        let response: SetGhostModeResponse = self.location("SetGhostMode", user, &request).await?;
        Ok(response.settings.into_option().expect("settings"))
    }

    /// Notification requests (`notify.requested`) addressed to `user`, by kind.
    pub async fn notifications_for(&self, user: Uuid, kind: &str) -> Vec<Event> {
        self.app
            .events("notify.requested")
            .await
            .into_iter()
            .filter(|e| e.data["user_id"] == serde_json::json!(user) && e.data["kind"] == serde_json::json!(kind))
            .collect()
    }

    /// Activity entries (`activity.recorded`) of `user`, by kind.
    pub async fn activities_of(&self, user: Uuid, kind: &str) -> Vec<Event> {
        self.app
            .events_by("activity.recorded", user)
            .await
            .into_iter()
            .filter(|e| e.data["kind"] == serde_json::json!(kind))
            .collect()
    }
}

/// "Now" for a fix of `user`: the pipeline drops fixes less than a second
/// after the previous one, and tests report faster than that, so each call
/// is at least a second after the last one for the same user.
pub fn next_fix_time(user: Uuid) -> DateTime<Utc> {
    static LAST: std::sync::Mutex<Option<std::collections::HashMap<Uuid, i64>>> = std::sync::Mutex::new(None);
    let mut last = LAST.lock().unwrap();
    let last = last.get_or_insert_with(Default::default);
    let now = Utc::now().timestamp_millis();
    let at = last.get(&user).map_or(now, |previous| now.max(previous + 1000));
    last.insert(user, at);
    DateTime::from_timestamp_millis(at).unwrap()
}

/// A device reading with good accuracy.
pub fn sample(point: Point, at: DateTime<Utc>) -> LocationSample {
    LocationSample {
        position: lat_lng(point).into(),
        accuracy_meters: 8.0,
        recorded_at: tokuly_core::time::ts(at),
        foreground: true,
        ..Default::default()
    }
}

pub fn lat_lng((lat, lng): Point) -> LatLng {
    LatLng { lat, lng, ..Default::default() }
}

/// The coordinates of a position field (`message.position.as_option()`).
pub fn point(position: Option<&LatLng>) -> Point {
    let position = position.expect("a position");
    (position.lat, position.lng)
}

/// Where a pin is shown.
pub fn pin_point(location: &FriendLocation) -> Point {
    point(location.position.as_option())
}

/// A random point on land-ish latitudes, in the middle of its footprint cell.
/// Valkey is shared with other test runs, so every test that uses positions
/// picks its own corner of the world.
pub fn somewhere() -> Point {
    let mut rng = rand::rng();
    let point = (rng.random_range(-55.0..55.0), rng.random_range(-170.0..170.0));
    math::cell_center(math::cell_at(point.0, point.1, math::CELL_RESOLUTION).expect("valid coordinates"))
}

/// The point `meters` away from `origin` in direction `bearing` (degrees clockwise from north).
pub fn offset(origin: Point, bearing: f64, meters: f64) -> Point {
    math::destination(origin.0, origin.1, bearing, meters)
}

pub fn distance(a: Point, b: Point) -> f64 {
    math::distance(a, b)
}

/// Whether two points are the same spot. Coordinates travel through JSON,
/// which may change the last bit of a float: equality is "within a centimeter".
pub fn at(a: Point, b: Point) -> bool {
    math::distance(a, b) < 0.01
}

/// Asserts that a position is where it should be (see [`at`]).
#[allow(unused_macros)] // not every test binary compares positions
macro_rules! assert_at {
    ($actual:expr, $expected:expr $(,)?) => {{
        let (actual, expected) = ($actual, $expected);
        assert!($crate::common::at(actual, expected), "expected {expected:?}, got {actual:?}");
    }};
    ($actual:expr, $expected:expr, $($message:tt)+) => {{
        let (actual, expected) = ($actual, $expected);
        assert!($crate::common::at(actual, expected), "expected {expected:?}, got {actual:?}: {}", format!($($message)+));
    }};
}
#[allow(unused_imports)] // not every test binary compares positions
pub(crate) use assert_at;

pub fn minutes_ago(minutes: i64) -> DateTime<Utc> {
    Utc::now() - chrono::Duration::minutes(minutes)
}

pub fn seconds_ago(seconds: i64) -> DateTime<Utc> {
    Utc::now() - chrono::Duration::seconds(seconds)
}

/// The next geo event pushed to a listener (other frames are skipped).
pub async fn next_push(listener: &mut FrameListener) -> Option<Push> {
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let left = deadline.checked_duration_since(tokio::time::Instant::now())?;
        match listener.next(left).await?.event::<GeoEvent>() {
            Some(event) => return event.event,
            _ => continue,
        }
    }
}

/// Every geo event pushed to a listener within `window`.
pub async fn pushes(listener: &mut FrameListener, window: Duration) -> Vec<Push> {
    listener
        .drain(window)
        .await
        .into_iter()
        .filter_map(|frame| frame.event::<GeoEvent>().and_then(|event| event.event))
        .collect()
}

/// The next pin pushed to a listener (other geo events are skipped).
pub async fn next_pin(listener: &mut FrameListener) -> Option<FriendLocation> {
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let left = deadline.checked_duration_since(tokio::time::Instant::now())?;
        if let Some(Push::Location(location)) =
            listener.next(left).await?.event::<GeoEvent>().and_then(|event| event.event)
        {
            return Some(*location);
        }
    }
}

/// The pins pushed to a listener within `window`.
pub async fn pins(listener: &mut FrameListener, window: Duration) -> Vec<FriendLocation> {
    pushes(listener, window)
        .await
        .into_iter()
        .filter_map(|push| match push {
            Push::Location(location) => Some(*location),
            _ => None,
        })
        .collect()
}

/// Samples of `user` at `point` at the given numbers of seconds before `t`.
pub async fn seen_at(ctx: &Ctx, user: &TestUser, point: Point, t: DateTime<Utc>, seconds_before: &[i64]) {
    let samples: Vec<LocationSample> =
        seconds_before.iter().map(|seconds| sample(point, t - chrono::Duration::seconds(*seconds))).collect();
    let sent = samples.len() as i32;
    assert_eq!(ctx.report(user, samples).await.accepted_count, sent);
}

/// Two friends standing a few meters apart for long enough to count as
/// together. The gathering forms with `b`'s last sample, 110 s before `t`.
pub async fn meet(ctx: &Ctx, a: &TestUser, b: &TestUser, spot: Point, t: DateTime<Utc>) {
    let beside = offset(spot, 90.0, 8.0);
    seen_at(ctx, a, spot, t, &[300, 270, 240]).await;
    seen_at(ctx, b, beside, t, &[300, 270, 240]).await; // b notices a nearby
    seen_at(ctx, a, spot, t, &[200]).await; // a notices b nearby
    seen_at(ctx, b, beside, t, &[110]).await; // two minutes later for b: together
    seen_at(ctx, a, spot, t, &[60]).await; // and for a
}
