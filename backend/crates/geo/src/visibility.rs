//! The one place that decides what a viewer sees of someone's location.
//!
//! `ListFriendLocations`, `WatchFriendLocations`, every realtime push, trip
//! destinations that follow a friend, profile distances, gatherings and place
//! alerts all go through the functions here. They are pure: everything they
//! need is passed in, including the clock, so the whole ghost-mode matrix is
//! unit-tested below.
//!
//! The rules:
//! - **PRECISE** — the true position, minus what the owner's privacy settings
//!   withhold (battery, speed/activity, online halo, place badges).
//! - **BLURRED** — a point 300 m–1.2 km from the true position. The offset
//!   depends only on (owner, viewer, the surrounding ~5 km² cell), so the pin
//!   holds still during a stay, comes back to the same spot on every visit
//!   (averaging many sightings reveals nothing), and differs per viewer.
//!   Speed, heading, activity, time on the spot, place details and trips are
//!   withheld; the area name and time zone are those of the blurred point.
//! - **FROZEN** — the pin as the viewer saw it when freezing began, original
//!   `recorded_at` included. Only what does not come from location samples
//!   stays live (online halo, status), exactly like a phone that stopped
//!   reporting, so nothing tells a frozen pin from a stale one.
//! - An active **SOS** shows the people it alerted the precise, live pin with
//!   nothing withheld, whatever the ghost rules say.
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use h3o::Resolution;
use tokuly_core::prelude::*;
use tokuly_proto::{
    common::Visibility,
    geo::{Activity, FriendLocation, GhostMode, PlaceLabel},
};

use crate::{
    ctx::{FrozenPin, OwnerCtx},
    detect::{SleepInput, sleeping_since},
    math::{self, Point},
    state::{Area, Pin, StayPlace},
};

const PRECISE: i32 = GhostMode::GHOST_MODE_PRECISE as i32;
const BLURRED: i32 = GhostMode::GHOST_MODE_BLURRED as i32;
const FROZEN: i32 = GhostMode::GHOST_MODE_FROZEN as i32;

/// Distance of a blurred point from the center of the true position's
/// resolution-9 cell: 660–840 m. With the cell's own extent (about 200 m at
/// most) and the spread of a stay (up to 110 m) the offset from the true
/// position stays well within 300 m–1.2 km.
const BLUR_MIN_M: f64 = 660.0;
const BLUR_SPAN_M: f64 = 180.0;
/// Accuracy radius reported for a blurred pin.
pub const BLUR_ACCURACY_M: f64 = 1200.0;

/// Keyed hash used to derive blur offsets (HMAC with a server secret, so
/// offsets cannot be computed, and thus undone, by clients).
pub type Mac<'a> = &'a (dyn Fn(&[u8]) -> [u8; 32] + Send + Sync);
/// Reverse geocoder: the area around a point.
pub type AreaLookup<'a> = &'a (dyn Fn(f64, f64) -> Area + Send + Sync);

/// Current weather at a position.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Weather {
    pub temperature_c: f64,
    pub icon: String,
}

/// What the views need from their surroundings.
pub struct ViewEnv<'a> {
    pub now: DateTime<Utc>,
    pub mac: Mac<'a>,
    pub area_at: AreaLookup<'a>,
}

/// The person being looked at.
pub struct Subject<'a> {
    pub id: Uuid,
    /// Their live pin, if they ever reported a position.
    pub pin: Option<&'a Pin>,
    pub ctx: &'a OwnerCtx,
    /// The app is open right now.
    pub online: bool,
    /// Weather at their true position, when configured and cached.
    pub weather: Option<&'a Weather>,
}

/// The person looking.
#[derive(Debug, Clone, Copy)]
pub struct Viewer {
    pub id: Uuid,
    /// Wants Japanese place names.
    pub ja: bool,
}

/// How a viewer sees a subject right now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sight<'a> {
    /// Live and exact. `sos` = because of an SOS, with nothing withheld.
    Precise {
        sos: bool,
    },
    Blurred,
    /// The copy made when freezing began (`None`: nothing was known then).
    Frozen(Option<&'a FrozenPin>),
}

/// Resolves the ghost rules (and an SOS) for one viewer.
pub fn sight<'a>(ctx: &'a OwnerCtx, viewer: Uuid, now: DateTime<Utc>) -> Sight<'a> {
    if ctx.sos_for(viewer) {
        return Sight::Precise { sos: true };
    }
    match ctx.ghost.rule_for(viewer, now.timestamp_millis()) {
        Some(rule) if rule.mode == BLURRED => Sight::Blurred,
        Some(rule) if rule.mode == FROZEN => Sight::Frozen(rule.frozen.as_ref()),
        _ => Sight::Precise { sos: false },
    }
}

/// The `GhostMode` number a viewer is subject to (ignoring SOS): what a new
/// FROZEN rule must preserve.
pub fn mode_for(ctx: &OwnerCtx, viewer: Uuid, now: DateTime<Utc>) -> i32 {
    ctx.ghost.rule_for(viewer, now.timestamp_millis()).map_or(PRECISE, |rule| rule.mode)
}

/// The blurred stand-in for `base`, as `viewer` sees `owner`.
pub fn blur(mac: Mac<'_>, owner: Uuid, viewer: Uuid, base: Point) -> Point {
    let Some(cell) = math::cell_at(base.0, base.1, math::CELL_RESOLUTION) else { return base };
    let center = math::cell_center(cell);
    // The offset is constant across a neighbourhood so the pin moves smoothly with its owner.
    let neighbourhood = cell.parent(Resolution::Seven).map_or(0, u64::from);
    let mut seed = Vec::with_capacity(4 + 16 + 16 + 8);
    seed.extend_from_slice(b"blur");
    seed.extend_from_slice(owner.as_bytes());
    seed.extend_from_slice(viewer.as_bytes());
    seed.extend_from_slice(&neighbourhood.to_be_bytes());
    let hash = mac(&seed);
    let unit =
        |bytes: &[u8]| u64::from_be_bytes(bytes.try_into().expect("eight bytes")) as f64 / (u64::MAX as f64 + 1.0);
    let bearing = unit(&hash[0..8]) * 360.0;
    let distance = BLUR_MIN_M + unit(&hash[8..16]) * BLUR_SPAN_M;
    math::destination(center.0, center.1, bearing, distance)
}

fn time_zone_of(pin: &Pin, ctx: &OwnerCtx) -> Tz {
    pin.area.time_zone.parse().or_else(|_| ctx.time_zone.parse()).unwrap_or(chrono_tz::UTC)
}

fn from_millis(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap_or_default()
}

/// Since when the pin's owner is probably asleep.
fn asleep_since(pin: &Pin, ctx: &OwnerCtx, online: bool, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let input = SleepInput {
        stationary_since: pin.stationary_since_ms.map(from_millis),
        last_active: (pin.last_foreground_ms > 0).then(|| from_millis(pin.last_foreground_ms)),
        last_sample: from_millis(pin.recorded_ms),
        online,
        now,
    };
    sleeping_since(input, time_zone_of(pin, ctx))
}

/// What a viewer learns about the place someone is at.
#[derive(Debug, Clone, PartialEq)]
pub struct ShownPlace {
    pub label: PlaceLabel,
    /// Empty for home, work and school: those are a badge only.
    pub name: String,
    pub emoji: String,
}

/// What `viewer` may know about the place `owner` is staying at, given that
/// they see the precise pin:
/// - the owner's home, work or school is a badge, never a name or address,
///   and only if the owner shares place labels;
/// - any other place is named when the viewer could look it up themselves:
///   a public place, the owner's own place shared with friends, or a place
///   the viewer owns. `friend` is false for viewers who are not friends
///   (public share links): they only get public places.
pub fn shown_place(
    owner: Uuid,
    place: &StayPlace,
    share_labels: bool,
    viewer: Option<Uuid>,
    friend: bool,
) -> Option<ShownPlace> {
    let own = place.owner_id == owner;
    if own && matches!(place.label, 1..=3) {
        let label = buffa::EnumValue::<PlaceLabel>::from(place.label).as_known()?;
        return (friend && share_labels).then(|| ShownPlace { label, name: String::new(), emoji: String::new() });
    }
    let named = viewer == Some(owner)
        || place.visibility == Visibility::VISIBILITY_PUBLIC as i32
        || (friend
            && place.visibility == Visibility::VISIBILITY_FRIENDS as i32
            && (own || viewer == Some(place.owner_id)));
    named.then(|| ShownPlace {
        label: PlaceLabel::PLACE_LABEL_CUSTOM,
        name: place.name.clone(),
        emoji: place.emoji.clone(),
    })
}

/// How much of a pin a view includes.
#[derive(Clone, Copy)]
struct Detail {
    /// Ignore the owner's privacy switches (their own view, SOS).
    everything: bool,
    /// The viewer, when there is one (place names depend on who looks).
    viewer: Option<Viewer>,
    /// The halo, status and trips are for friends only (not for public share links).
    social: bool,
}

/// The exact pin with the owner's privacy settings applied.
fn precise_view(env: &ViewEnv<'_>, subject: &Subject<'_>, pin: &Pin, detail: Detail) -> FriendLocation {
    let (ctx, privacy) = (subject.ctx, &subject.ctx.privacy);
    let ja = detail.viewer.is_some_and(|viewer| viewer.ja);
    let show_activity = detail.everything || privacy.share_activity;
    let show_battery = detail.everything || privacy.share_battery;
    let online = detail.social && subject.online && (detail.everything || privacy.share_online_status);
    let mut view = FriendLocation {
        user_id: subject.id.to_string(),
        position: math::lat_lng(pin.lat, pin.lng).into(),
        accuracy_meters: pin.accuracy,
        recorded_at: ts(from_millis(pin.recorded_ms)),
        precision: GhostMode::GHOST_MODE_PRECISE.into(),
        stationary_since: opt_ts(pin.stationary_since_ms.map(from_millis)),
        locality: pin.area.label(ja),
        country_code: pin.area.country_code.clone(),
        time_zone: pin.area.time_zone.clone(),
        online,
        sharing_paused: !pin.sharing,
        ..Default::default()
    };
    if show_activity {
        view.speed_mps = pin.speed;
        view.heading_degrees = pin.heading.unwrap_or(0.0);
        view.activity = pin.activity.into();
        if detail.social {
            let asleep = asleep_since(pin, ctx, subject.online, env.now);
            view.sleeping = asleep.is_some();
            view.sleeping_since = opt_ts(asleep);
        }
    }
    if show_battery {
        view.battery_percent = pin.battery;
        view.charging = pin.charging;
    }
    if let Some(place) = &pin.place {
        let share_labels = detail.everything || privacy.share_place_labels;
        if let Some(shown) =
            shown_place(subject.id, place, share_labels, detail.viewer.map(|viewer| viewer.id), detail.social)
        {
            view.place_label = shown.label.into();
            view.place_name = shown.name;
            view.place_emoji = shown.emoji;
        }
    }
    if detail.social {
        view.status = ctx.status.as_ref().and_then(|status| status.current(env.now)).into();
        if let (Some(trip), Some(viewer)) = (&ctx.trip, detail.viewer) {
            if viewer.id == subject.id || trip.shared_with(viewer.id) {
                view.trip_id = trip.id.to_string();
            }
        }
    }
    if let Some(weather) = subject.weather {
        view.temperature_celsius = Some(weather.temperature_c);
        view.weather_icon = weather.icon.clone();
    }
    view
}

/// The approximate pin: a stand-in position and nothing that would narrow it down.
fn blurred_view(env: &ViewEnv<'_>, subject: &Subject<'_>, pin: &Pin, viewer: Viewer) -> FriendLocation {
    let (ctx, privacy) = (subject.ctx, &subject.ctx.privacy);
    let (lat, lng) = blur(env.mac, subject.id, viewer.id, pin.stay_center.unwrap_or(pin.position()));
    let area = (env.area_at)(lat, lng);
    let mut view = FriendLocation {
        user_id: subject.id.to_string(),
        position: math::lat_lng(lat, lng).into(),
        accuracy_meters: BLUR_ACCURACY_M,
        recorded_at: ts(from_millis(pin.recorded_ms)),
        precision: GhostMode::GHOST_MODE_BLURRED.into(),
        activity: Activity::ACTIVITY_UNSPECIFIED.into(),
        locality: area.label(viewer.ja),
        country_code: area.country_code,
        time_zone: area.time_zone,
        online: subject.online && privacy.share_online_status,
        status: ctx.status.as_ref().and_then(|status| status.current(env.now)).into(),
        sharing_paused: !pin.sharing,
        ..Default::default()
    };
    if privacy.share_battery {
        view.battery_percent = pin.battery;
        view.charging = pin.charging;
    }
    if privacy.share_activity {
        let asleep = asleep_since(pin, ctx, subject.online, env.now);
        view.sleeping = asleep.is_some();
        view.sleeping_since = opt_ts(asleep);
    }
    if let Some(weather) = subject.weather {
        view.temperature_celsius = Some(weather.temperature_c);
        view.weather_icon = weather.icon.clone();
    }
    view
}

/// What `viewer` (a friend) sees of `subject`: `None` when they see nothing
/// at all. Callers check the friendship; this function applies ghost mode,
/// SOS and the privacy settings.
pub fn friend_view(env: &ViewEnv<'_>, subject: &Subject<'_>, viewer: Viewer) -> Option<FriendLocation> {
    if !subject.ctx.active {
        return None;
    }
    let friend = Detail { everything: false, viewer: Some(viewer), social: true };
    match sight(subject.ctx, viewer.id, env.now) {
        Sight::Precise { sos } => Some(precise_view(env, subject, subject.pin?, Detail { everything: sos, ..friend })),
        Sight::Blurred => Some(blurred_view(env, subject, subject.pin?, viewer)),
        Sight::Frozen(frozen) => {
            let frozen = frozen?;
            // Rendered exactly as it was shown before freezing. No weather and
            // no trip: both would be news about the present.
            let stale = Subject {
                id: subject.id,
                pin: Some(&frozen.pin),
                ctx: subject.ctx,
                online: subject.online,
                weather: None,
            };
            let mut view = if frozen.base == BLURRED {
                blurred_view(env, &stale, &frozen.pin, viewer)
            } else {
                precise_view(env, &stale, &frozen.pin, friend)
            };
            view.trip_id.clear();
            Some(view)
        }
    }
}

/// The owner's own pin, with nothing withheld.
pub fn own_view(env: &ViewEnv<'_>, subject: &Subject<'_>) -> Option<FriendLocation> {
    let viewer = Viewer { id: subject.id, ja: subject.ctx.ja };
    Some(precise_view(env, subject, subject.pin?, Detail { everything: true, viewer: Some(viewer), social: true }))
}

/// The pin behind a public live share: the exact position and movement, but
/// no badges, halo, status or anything else meant for friends.
pub fn share_view(env: &ViewEnv<'_>, subject: &Subject<'_>, ja: bool) -> Option<FriendLocation> {
    if !subject.ctx.active {
        return None;
    }
    let mut view = precise_view(env, subject, subject.pin?, Detail { everything: true, viewer: None, social: false });
    view.locality = subject.pin?.area.label(ja);
    view.stationary_since = Default::default();
    Some(view)
}

/// Where `viewer` believes `subject` is, for anything computed from a
/// friend's position on the viewer's behalf (distance on a profile, a trip
/// towards a friend): the true position if they see it precisely, the blurred
/// point if blurred, nothing if frozen or unknown — a frozen pin must not be
/// refreshed through a side channel, and its stale position proves nothing.
pub fn seen_position(env: &ViewEnv<'_>, subject: &Subject<'_>, viewer: Uuid) -> Option<(Point, bool)> {
    if !subject.ctx.active {
        return None;
    }
    let pin = subject.pin?;
    match sight(subject.ctx, viewer, env.now) {
        Sight::Precise { .. } => Some((pin.position(), true)),
        Sight::Blurred => Some((blur(env.mac, subject.id, viewer, pin.stay_center.unwrap_or(pin.position())), false)),
        Sight::Frozen(_) => None,
    }
}

/// Whether `viewer` sees the subject's true, live position.
pub fn sees_precisely(ctx: &OwnerCtx, viewer: Uuid, now: DateTime<Utc>) -> bool {
    ctx.active && matches!(sight(ctx, viewer, now), Sight::Precise { .. })
}

#[cfg(test)]
pub(crate) mod tests {
    use chrono::{Duration, TimeZone as _};

    use super::*;
    use crate::{
        ctx::{GhostConfig, GhostRule, Privacy, SosCtx, StatusLite, TripCtx},
        state::StayPlace,
    };

    /// A stand-in for the keyed hash: deterministic and well mixed.
    pub(crate) fn test_mac(data: &[u8]) -> [u8; 32] {
        let mut out = [0u8; 32];
        let mut state = 0xcbf2_9ce4_8422_2325u64;
        for round in 0..4u64 {
            for byte in data.iter().chain(&round.to_le_bytes()) {
                state = (state ^ *byte as u64).wrapping_mul(0x0100_0000_01b3);
                state ^= state >> 29;
            }
            out[round as usize * 8..round as usize * 8 + 8].copy_from_slice(&state.to_be_bytes());
        }
        out
    }

    fn test_area(lat: f64, _lng: f64) -> Area {
        // Two "wards" split along a parallel, so blurred pins can land in the other one.
        let (en, ja) = if lat >= 35.66 { ("Shibuya", "渋谷区") } else { ("Meguro", "目黒区") };
        Area {
            locality: en.into(),
            locality_ja: ja.into(),
            region: "Tokyo".into(),
            region_ja: "東京都".into(),
            country_code: "JP".into(),
            time_zone: "Asia/Tokyo".into(),
        }
    }

    const HOME: Point = (35.6605, 139.7005);

    fn noon() -> DateTime<Utc> {
        chrono_tz::Asia::Tokyo.with_ymd_and_hms(2026, 11, 20, 12, 0, 0).unwrap().with_timezone(&Utc)
    }

    fn env(now: DateTime<Utc>) -> ViewEnv<'static> {
        ViewEnv { now, mac: &test_mac, area_at: &test_area }
    }

    fn pin_at(position: Point, recorded: DateTime<Utc>) -> Pin {
        Pin {
            lat: position.0,
            lng: position.1,
            accuracy: 12.0,
            speed: 1.3,
            heading: Some(87.0),
            recorded_ms: recorded.timestamp_millis(),
            battery: Some(64),
            charging: true,
            activity: Activity::ACTIVITY_WALKING as i32,
            sharing: true,
            stationary_since_ms: Some((recorded - Duration::minutes(40)).timestamp_millis()),
            stay_center: Some(position),
            place: None,
            area: test_area(position.0, position.1),
            last_foreground_ms: (recorded - Duration::minutes(5)).timestamp_millis(),
        }
    }

    fn active_ctx() -> OwnerCtx {
        OwnerCtx {
            active: true,
            privacy: Privacy::default(),
            ja: true,
            time_zone: "Asia/Tokyo".into(),
            ..Default::default()
        }
    }

    fn rule(mode: GhostMode, frozen: Option<FrozenPin>) -> GhostRule {
        GhostRule { mode: mode as i32, since_ms: 0, until_ms: None, frozen }
    }

    fn position_of(view: &FriendLocation) -> Point {
        let position = view.position.as_option().expect("position");
        (position.lat, position.lng)
    }

    struct World {
        owner: Uuid,
        viewer: Viewer,
        pin: Pin,
        ctx: OwnerCtx,
        online: bool,
    }

    impl World {
        fn new() -> Self {
            Self {
                owner: Uuid::now_v7(),
                viewer: Viewer { id: Uuid::now_v7(), ja: true },
                pin: pin_at(HOME, noon()),
                ctx: active_ctx(),
                online: true,
            }
        }

        fn subject(&self) -> Subject<'_> {
            Subject { id: self.owner, pin: Some(&self.pin), ctx: &self.ctx, online: self.online, weather: None }
        }

        fn seen_at(&self, now: DateTime<Utc>) -> Option<FriendLocation> {
            friend_view(&env(now), &self.subject(), self.viewer)
        }

        fn seen(&self) -> Option<FriendLocation> {
            self.seen_at(noon())
        }

        fn ghost_everyone(&mut self, mode: GhostMode) {
            let frozen =
                (mode == GhostMode::GHOST_MODE_FROZEN).then(|| FrozenPin { pin: self.pin.clone(), base: PRECISE });
            self.ctx.ghost.everyone = Some(rule(mode, frozen));
        }
    }

    #[test]
    fn precise_shows_the_true_pin() {
        let world = World::new();
        let view = world.seen().unwrap();
        assert_eq!(view.user_id, world.owner.to_string());
        assert_eq!(position_of(&view), HOME);
        assert_eq!(view.precision, GhostMode::GHOST_MODE_PRECISE);
        assert_eq!((view.accuracy_meters, view.speed_mps, view.heading_degrees), (12.0, 1.3, 87.0));
        assert_eq!(view.activity, Activity::ACTIVITY_WALKING);
        assert_eq!((view.battery_percent, view.charging), (Some(64), true));
        assert_eq!(from_ts(view.recorded_at.as_option().unwrap()), noon());
        assert_eq!(from_ts(view.stationary_since.as_option().unwrap()), noon() - Duration::minutes(40));
        assert_eq!(
            (view.locality.as_str(), view.country_code.as_str(), view.time_zone.as_str()),
            ("渋谷区", "JP", "Asia/Tokyo")
        );
        assert!(view.online && !view.sleeping && !view.sharing_paused);
        assert!(view.trip_id.is_empty() && view.place_name.is_empty());
    }

    #[test]
    fn the_viewers_language_picks_the_area_name() {
        let mut world = World::new();
        world.viewer.ja = false;
        assert_eq!(world.seen().unwrap().locality, "Shibuya, Tokyo");
    }

    #[test]
    fn privacy_switches_withhold_their_fields() {
        let mut world = World::new();
        world.pin.place = Some(StayPlace {
            id: Uuid::now_v7(),
            owner_id: world.owner,
            label: 1,
            name: "うち".into(),
            emoji: "🏠".into(),
            visibility: 5,
        });
        assert_eq!(world.seen().unwrap().place_label, PlaceLabel::PLACE_LABEL_HOME);

        world.ctx.privacy = Privacy {
            share_battery: false,
            share_activity: false,
            share_online_status: false,
            share_place_labels: false,
            ..Privacy::default()
        };
        let view = world.seen().unwrap();
        assert_eq!((view.battery_percent, view.charging), (None, false));
        assert_eq!((view.speed_mps, view.heading_degrees), (0.0, 0.0));
        assert_eq!(view.activity, Activity::ACTIVITY_UNSPECIFIED);
        assert!(!view.online);
        assert_eq!(view.place_label, PlaceLabel::PLACE_LABEL_UNSPECIFIED);
        // The position itself is not a privacy switch: that is what ghost mode is for.
        assert_eq!(position_of(&view), HOME);
    }

    #[test]
    fn home_is_a_badge_and_never_a_name() {
        let mut world = World::new();
        world.pin.place = Some(StayPlace {
            id: Uuid::now_v7(),
            owner_id: world.owner,
            label: 1,
            name: "世田谷の実家".into(),
            emoji: "🏠".into(),
            visibility: 5,
        });
        let view = world.seen().unwrap();
        assert_eq!(view.place_label, PlaceLabel::PLACE_LABEL_HOME);
        assert!(view.place_name.is_empty() && view.place_emoji.is_empty());
    }

    #[test]
    fn place_names_follow_the_places_visibility() {
        let mut world = World::new();
        let stranger = Uuid::now_v7();
        let place = |owner: Uuid, visibility: Visibility| {
            Some(StayPlace {
                id: Uuid::now_v7(),
                owner_id: owner,
                label: 4,
                name: "カフェ モカ".into(),
                emoji: "☕".into(),
                visibility: visibility as i32,
            })
        };
        // The owner's own custom place: shown when shared with friends, hidden when private.
        world.pin.place = place(world.owner, Visibility::VISIBILITY_FRIENDS);
        let view = world.seen().unwrap();
        assert_eq!((view.place_name.as_str(), view.place_emoji.as_str()), ("カフェ モカ", "☕"));
        assert_eq!(view.place_label, PlaceLabel::PLACE_LABEL_CUSTOM);
        world.pin.place = place(world.owner, Visibility::VISIBILITY_PRIVATE);
        assert!(world.seen().unwrap().place_name.is_empty());
        // Anyone's public place is named.
        world.pin.place = place(stranger, Visibility::VISIBILITY_PUBLIC);
        assert_eq!(world.seen().unwrap().place_name, "カフェ モカ");
        // A third person's friends-only place is named only to that person.
        world.pin.place = place(stranger, Visibility::VISIBILITY_FRIENDS);
        assert!(world.seen().unwrap().place_name.is_empty());
        world.pin.place = place(world.viewer.id, Visibility::VISIBILITY_FRIENDS);
        assert_eq!(world.seen().unwrap().place_name, "カフェ モカ");
    }

    #[test]
    fn blurred_moves_the_pin_and_withholds_details() {
        let mut world = World::new();
        world.pin.place = Some(StayPlace {
            id: Uuid::now_v7(),
            owner_id: world.owner,
            label: 1,
            name: "うち".into(),
            emoji: "🏠".into(),
            visibility: 5,
        });
        world.ctx.trip = Some(TripCtx {
            id: Uuid::now_v7(),
            dest_user_id: None,
            dest_lat: 35.7,
            dest_lng: 139.8,
            arrive_radius_m: 60.0,
            mode: 1,
            started_ms: 0,
            plan_distance_m: 1000.0,
            plan_duration_s: 600,
            shared_with: vec![],
            notify_on_arrival: false,
        });
        world.ghost_everyone(GhostMode::GHOST_MODE_BLURRED);
        let view = world.seen().unwrap();
        assert_eq!(view.precision, GhostMode::GHOST_MODE_BLURRED);
        let offset = math::distance(position_of(&view), HOME);
        assert!((300.0..=1200.0).contains(&offset), "offset {offset}");
        assert_eq!(view.accuracy_meters, BLUR_ACCURACY_M);
        assert_eq!((view.speed_mps, view.heading_degrees), (0.0, 0.0));
        assert_eq!(view.activity, Activity::ACTIVITY_UNSPECIFIED);
        assert!(view.stationary_since.as_option().is_none());
        assert_eq!(view.place_label, PlaceLabel::PLACE_LABEL_UNSPECIFIED);
        assert!(view.place_name.is_empty() && view.trip_id.is_empty());
        // Still live: the time, the battery and the halo.
        assert_eq!(from_ts(view.recorded_at.as_option().unwrap()), noon());
        assert_eq!(view.battery_percent, Some(64));
        assert!(view.online);
    }

    #[test]
    fn blurred_area_names_describe_the_blurred_point() {
        let mut world = World::new();
        world.ghost_everyone(GhostMode::GHOST_MODE_BLURRED);
        let view = world.seen().unwrap();
        let expected = test_area(position_of(&view).0, 0.0).locality_ja;
        assert_eq!(view.locality, expected);
    }

    #[test]
    fn the_blur_offset_is_always_within_bounds() {
        // Many owners, viewers and positions (including ones off the cell
        // center and off the stay center by a full stay radius).
        for i in 0..400u32 {
            let (owner, viewer) = (Uuid::now_v7(), Uuid::now_v7());
            let center = (-60.0 + (i % 40) as f64 * 3.1, -170.0 + (i * 7 % 340) as f64);
            let truth = math::destination(center.0, center.1, (i * 37 % 360) as f64, (i % 11) as f64 * 11.0);
            let blurred = blur(&test_mac, owner, viewer, center);
            let offset = math::distance(blurred, truth);
            assert!((300.0..=1200.0).contains(&offset), "offset {offset} at {center:?}");
        }
    }

    #[test]
    fn the_blurred_pin_does_not_jitter_during_a_stay() {
        let mut world = World::new();
        world.ghost_everyone(GhostMode::GHOST_MODE_BLURRED);
        let first = position_of(&world.seen().unwrap());
        // GPS wobble around the stay center, minutes apart.
        for i in 1..20 {
            let wobble = math::destination(HOME.0, HOME.1, (i * 53 % 360) as f64, 25.0);
            world.pin.lat = wobble.0;
            world.pin.lng = wobble.1;
            world.pin.recorded_ms += 60_000;
            assert_eq!(position_of(&world.seen().unwrap()), first);
        }
        // The same spot on another day gives the same stand-in.
        let later = pin_at(HOME, noon() + Duration::days(3));
        world.pin = later;
        assert_eq!(position_of(&world.seen_at(noon() + Duration::days(3)).unwrap()), first);
    }

    #[test]
    fn each_viewer_gets_their_own_blurred_point_and_cannot_compare_notes() {
        let mut world = World::new();
        world.ghost_everyone(GhostMode::GHOST_MODE_BLURRED);
        let first = position_of(&world.seen().unwrap());
        world.viewer.id = Uuid::now_v7();
        let second = position_of(&world.seen().unwrap());
        assert_ne!(first, second);
        // And a different owner at the same spot is blurred differently for the same viewer.
        world.owner = Uuid::now_v7();
        assert_ne!(position_of(&world.seen().unwrap()), second);
    }

    #[test]
    fn the_blurred_pin_follows_real_movement() {
        let mut world = World::new();
        world.ghost_everyone(GhostMode::GHOST_MODE_BLURRED);
        world.pin.stay_center = None;
        world.pin.stationary_since_ms = None;
        let start = position_of(&world.seen().unwrap());
        let far = math::destination(HOME.0, HOME.1, 45.0, 8000.0);
        world.pin.lat = far.0;
        world.pin.lng = far.1;
        let moved = position_of(&world.seen().unwrap());
        assert!(math::distance(start, moved) > 5000.0);
        let offset = math::distance(moved, far);
        assert!((300.0..=1200.0).contains(&offset), "offset {offset}");
    }

    #[test]
    fn frozen_keeps_the_pin_from_the_moment_of_freezing() {
        let mut world = World::new();
        world.ghost_everyone(GhostMode::GHOST_MODE_FROZEN);
        let at_freeze = world.seen().unwrap();
        // Two hours later the owner is across town with a different battery.
        let later = noon() + Duration::hours(2);
        world.pin = pin_at(math::destination(HOME.0, HOME.1, 90.0, 9000.0), later);
        world.pin.battery = Some(12);
        world.pin.stationary_since_ms = None;
        let view = world.seen_at(later).unwrap();
        assert_eq!(position_of(&view), HOME);
        assert_eq!(
            from_ts(view.recorded_at.as_option().unwrap()),
            noon(),
            "the original recorded_at: the pin just looks stale"
        );
        assert_eq!(view.battery_percent, Some(64));
        assert_eq!(view.stationary_since, at_freeze.stationary_since);
        assert_eq!(view.locality, at_freeze.locality);
    }

    #[test]
    fn nothing_reveals_that_a_pin_is_frozen() {
        // A frozen pin and the pin of a phone that died at the same moment
        // must be the same message, field for field.
        let mut frozen = World::new();
        frozen.ghost_everyone(GhostMode::GHOST_MODE_FROZEN);
        let mut dead_phone = World::new();
        dead_phone.owner = frozen.owner;
        dead_phone.viewer = frozen.viewer;
        dead_phone.pin = frozen.pin.clone();

        let later = noon() + Duration::hours(5);
        frozen.pin = pin_at(math::destination(HOME.0, HOME.1, 10.0, 20_000.0), later);
        for online in [true, false] {
            frozen.online = online;
            dead_phone.online = online;
            let (a, b) = (frozen.seen_at(later).unwrap(), dead_phone.seen_at(later).unwrap());
            assert_eq!(a, b);
            assert_eq!(a.precision, GhostMode::GHOST_MODE_PRECISE, "FROZEN is never revealed");
        }
    }

    #[test]
    fn frozen_keeps_live_what_does_not_come_from_the_location() {
        let mut world = World::new();
        world.ghost_everyone(GhostMode::GHOST_MODE_FROZEN);
        world.ctx.status =
            Some(StatusLite { emoji: "🍜".into(), text: String::new(), set_ms: None, expires_ms: None });
        world.online = false;
        let view = world.seen().unwrap();
        assert!(!view.online);
        assert_eq!(view.status.as_option().unwrap().emoji, "🍜");
        world.online = true;
        assert!(world.seen().unwrap().online);
    }

    #[test]
    fn frozen_never_shows_a_trip_or_weather() {
        let mut world = World::new();
        world.ghost_everyone(GhostMode::GHOST_MODE_FROZEN);
        world.ctx.trip = Some(TripCtx {
            id: Uuid::now_v7(),
            dest_user_id: None,
            dest_lat: 35.7,
            dest_lng: 139.8,
            arrive_radius_m: 60.0,
            mode: 1,
            started_ms: 0,
            plan_distance_m: 1000.0,
            plan_duration_s: 600,
            shared_with: vec![],
            notify_on_arrival: false,
        });
        let weather = Weather { temperature_c: 18.0, icon: "clear-day".into() };
        let subject = Subject { weather: Some(&weather), ..world.subject() };
        let view = friend_view(&env(noon()), &subject, world.viewer).unwrap();
        assert!(view.trip_id.is_empty());
        assert_eq!(view.temperature_celsius, None);
    }

    #[test]
    fn freezing_a_blurred_viewer_keeps_their_blurred_point() {
        let mut world = World::new();
        world.ghost_everyone(GhostMode::GHOST_MODE_BLURRED);
        let before = world.seen().unwrap();
        world.ctx.ghost.everyone =
            Some(rule(GhostMode::GHOST_MODE_FROZEN, Some(FrozenPin { pin: world.pin.clone(), base: BLURRED })));
        let later = noon() + Duration::hours(1);
        world.pin = pin_at(math::destination(HOME.0, HOME.1, 200.0, 15_000.0), later);
        let view = world.seen_at(later).unwrap();
        assert_eq!(position_of(&view), position_of(&before));
        assert_eq!(view.precision, GhostMode::GHOST_MODE_BLURRED);
        assert_eq!(view.recorded_at, before.recorded_at);
    }

    #[test]
    fn frozen_without_a_known_position_shows_nothing() {
        let mut world = World::new();
        world.ctx.ghost.everyone = Some(rule(GhostMode::GHOST_MODE_FROZEN, None));
        assert!(world.seen().is_none());
    }

    #[test]
    fn overrides_apply_to_one_friend_only() {
        let mut world = World::new();
        let other = Viewer { id: Uuid::now_v7(), ja: true };
        // Everyone is frozen, except one friend who stays precise.
        world.ghost_everyone(GhostMode::GHOST_MODE_FROZEN);
        world.ctx.ghost.overrides.insert(world.viewer.id, rule(GhostMode::GHOST_MODE_PRECISE, None));
        let later = noon() + Duration::hours(1);
        let moved = math::destination(HOME.0, HOME.1, 0.0, 3000.0);
        world.pin = pin_at(moved, later);
        assert_eq!(position_of(&world.seen_at(later).unwrap()), moved);
        let frozen = friend_view(&env(later), &world.subject(), other).unwrap();
        assert_eq!(position_of(&frozen), HOME);

        // The reverse: everyone precise, one friend blurred.
        world.ctx.ghost = GhostConfig::default();
        world.ctx.ghost.overrides.insert(world.viewer.id, rule(GhostMode::GHOST_MODE_BLURRED, None));
        assert_eq!(world.seen_at(later).unwrap().precision, GhostMode::GHOST_MODE_BLURRED);
        assert_eq!(position_of(&friend_view(&env(later), &world.subject(), other).unwrap()), moved);
    }

    #[test]
    fn timed_rules_stop_applying_when_they_end() {
        let mut world = World::new();
        let until = noon() + Duration::hours(1);
        world.ctx.ghost.everyone = Some(GhostRule {
            mode: FROZEN,
            since_ms: 0,
            until_ms: Some(until.timestamp_millis()),
            frozen: Some(FrozenPin { pin: world.pin.clone(), base: PRECISE }),
        });
        let moved = math::destination(HOME.0, HOME.1, 0.0, 3000.0);
        world.pin = pin_at(moved, noon() + Duration::minutes(59));
        assert_eq!(position_of(&world.seen_at(until - Duration::seconds(1)).unwrap()), HOME);
        assert_eq!(position_of(&world.seen_at(until).unwrap()), moved);
    }

    #[test]
    fn sos_overrides_every_ghost_mode_for_the_people_alerted() {
        for mode in [GhostMode::GHOST_MODE_BLURRED, GhostMode::GHOST_MODE_FROZEN] {
            let mut world = World::new();
            let bystander = Viewer { id: Uuid::now_v7(), ja: true };
            world.ghost_everyone(mode);
            world.ctx.privacy.share_battery = false;
            let later = noon() + Duration::minutes(30);
            let here = math::destination(HOME.0, HOME.1, 135.0, 4000.0);
            world.pin = pin_at(here, later);
            world.ctx.sos = Some(SosCtx {
                id: Uuid::now_v7(),
                message: "助けて".into(),
                notify: vec![world.viewer.id],
                started_ms: 0,
            });

            let view = world.seen_at(later).unwrap();
            assert_eq!(position_of(&view), here, "{mode:?}");
            assert_eq!(view.precision, GhostMode::GHOST_MODE_PRECISE);
            assert_eq!(view.battery_percent, Some(64), "nothing is withheld in an emergency");
            // Friends who were not alerted stay under ghost mode.
            let other = friend_view(&env(later), &world.subject(), bystander).unwrap();
            assert_ne!(position_of(&other), here);
            assert_eq!(other.battery_percent, None);

            // Resolved: ghost mode applies again.
            world.ctx.sos = None;
            assert_ne!(position_of(&world.seen_at(later).unwrap()), here);
        }
    }

    #[test]
    fn inactive_accounts_and_unknown_positions_show_nothing() {
        let mut world = World::new();
        world.ctx.active = false;
        assert!(world.seen().is_none());
        assert!(seen_position(&env(noon()), &world.subject(), world.viewer.id).is_none());
        world.ctx.active = true;
        let nowhere = Subject { pin: None, ..world.subject() };
        assert!(friend_view(&env(noon()), &nowhere, world.viewer).is_none());
        assert!(own_view(&env(noon()), &nowhere).is_none());
    }

    #[test]
    fn a_paused_pin_says_so() {
        let mut world = World::new();
        world.pin.sharing = false;
        assert!(world.seen().unwrap().sharing_paused);
        world.ghost_everyone(GhostMode::GHOST_MODE_BLURRED);
        assert!(world.seen().unwrap().sharing_paused);
    }

    #[test]
    fn sleeping_is_inferred_at_view_time() {
        let mut world = World::new();
        world.online = false;
        let evening = chrono_tz::Asia::Tokyo.with_ymd_and_hms(2026, 11, 20, 23, 0, 0).unwrap().with_timezone(&Utc);
        world.pin = pin_at(HOME, evening);
        world.pin.stationary_since_ms = Some((evening - Duration::hours(3)).timestamp_millis());
        world.pin.last_foreground_ms = evening.timestamp_millis();
        assert!(!world.seen_at(evening + Duration::minutes(20)).unwrap().sleeping);
        let asleep = world.seen_at(evening + Duration::hours(3)).unwrap();
        assert!(asleep.sleeping);
        assert_eq!(from_ts(asleep.sleeping_since.as_option().unwrap()), evening);
        world.online = true;
        assert!(!world.seen_at(evening + Duration::hours(3)).unwrap().sleeping);
        // Sharing activity off hides it.
        world.online = false;
        world.ctx.privacy.share_activity = false;
        assert!(!world.seen_at(evening + Duration::hours(3)).unwrap().sleeping);
    }

    #[test]
    fn trips_show_only_to_the_people_they_are_shared_with() {
        let mut world = World::new();
        let trip_id = Uuid::now_v7();
        let trip = |shared_with: Vec<Uuid>| {
            Some(TripCtx {
                id: trip_id,
                dest_user_id: None,
                dest_lat: 35.7,
                dest_lng: 139.8,
                arrive_radius_m: 60.0,
                mode: 1,
                started_ms: 0,
                plan_distance_m: 1000.0,
                plan_duration_s: 600,
                shared_with,
                notify_on_arrival: false,
            })
        };
        world.ctx.trip = trip(vec![]);
        assert_eq!(world.seen().unwrap().trip_id, trip_id.to_string(), "empty = all friends");
        world.ctx.trip = trip(vec![Uuid::now_v7()]);
        assert!(world.seen().unwrap().trip_id.is_empty());
        world.ctx.trip = trip(vec![world.viewer.id]);
        assert_eq!(world.seen().unwrap().trip_id, trip_id.to_string());
    }

    #[test]
    fn expired_statuses_disappear_from_the_pin() {
        let mut world = World::new();
        world.ctx.status = Some(StatusLite {
            emoji: "🎮".into(),
            text: "ゲーム中".into(),
            set_ms: None,
            expires_ms: Some((noon() + Duration::hours(1)).timestamp_millis()),
        });
        assert_eq!(world.seen().unwrap().status.as_option().unwrap().text, "ゲーム中");
        assert!(world.seen_at(noon() + Duration::hours(2)).unwrap().status.as_option().is_none());
    }

    #[test]
    fn the_owner_sees_everything_about_themselves() {
        let mut world = World::new();
        world.ctx.privacy = Privacy {
            share_battery: false,
            share_activity: false,
            share_online_status: false,
            share_place_labels: false,
            ..Privacy::default()
        };
        world.ghost_everyone(GhostMode::GHOST_MODE_FROZEN);
        world.pin = pin_at(math::destination(HOME.0, HOME.1, 0.0, 500.0), noon());
        world.pin.place = Some(StayPlace {
            id: Uuid::now_v7(),
            owner_id: world.owner,
            label: 4,
            name: "ひみつ基地".into(),
            emoji: "🏕".into(),
            visibility: 5,
        });
        let view = own_view(&env(noon()), &world.subject()).unwrap();
        assert_eq!(position_of(&view), world.pin.position());
        assert_eq!(view.battery_percent, Some(64));
        assert_eq!(view.activity, Activity::ACTIVITY_WALKING);
        assert!(view.online);
        assert_eq!(view.place_name, "ひみつ基地", "one's own private place is named to oneself");
    }

    #[test]
    fn share_links_show_the_position_and_nothing_personal() {
        let mut world = World::new();
        world.ghost_everyone(GhostMode::GHOST_MODE_FROZEN);
        world.ctx.status =
            Some(StatusLite { emoji: "🍜".into(), text: String::new(), set_ms: None, expires_ms: None });
        world.pin.place = Some(StayPlace {
            id: Uuid::now_v7(),
            owner_id: world.owner,
            label: 1,
            name: "うち".into(),
            emoji: "🏠".into(),
            visibility: 5,
        });
        let view = share_view(&env(noon()), &world.subject(), false).unwrap();
        // A share link is an explicit grant: ghost mode does not apply to it.
        assert_eq!(position_of(&view), HOME);
        assert_eq!((view.speed_mps, view.battery_percent), (1.3, Some(64)));
        assert_eq!(view.locality, "Shibuya, Tokyo");
        assert_eq!(view.place_label, PlaceLabel::PLACE_LABEL_UNSPECIFIED);
        assert!(
            view.place_name.is_empty()
                && view.status.as_option().is_none()
                && view.stationary_since.as_option().is_none()
        );
        assert!(!view.online && !view.sleeping);
    }

    #[test]
    fn seen_positions_respect_ghost_mode() {
        let mut world = World::new();
        let e = env(noon());
        assert_eq!(seen_position(&e, &world.subject(), world.viewer.id), Some((HOME, true)));
        world.ghost_everyone(GhostMode::GHOST_MODE_BLURRED);
        let (blurred, precise) = seen_position(&e, &world.subject(), world.viewer.id).unwrap();
        assert!(!precise);
        assert_eq!(blurred, position_of(&world.seen().unwrap()), "the same point the map shows");
        world.ghost_everyone(GhostMode::GHOST_MODE_FROZEN);
        assert_eq!(seen_position(&e, &world.subject(), world.viewer.id), None);
        assert!(!sees_precisely(&world.ctx, world.viewer.id, noon()));
        world.ctx.sos =
            Some(SosCtx { id: Uuid::now_v7(), message: String::new(), notify: vec![world.viewer.id], started_ms: 0 });
        assert!(sees_precisely(&world.ctx, world.viewer.id, noon()));
        assert_eq!(mode_for(&world.ctx, world.viewer.id, noon()), FROZEN, "the rule itself is unchanged by an SOS");
    }
}
