//! The detectors that turn a stream of location samples into meaning: stays,
//! proximity to friends (the basis of gatherings), movement activity, sleep
//! and nights.
//!
//! All of them are pure state machines: they take the previous state and one
//! observation and return what changed. Time is always passed in (Unix
//! milliseconds or `DateTime`), never read from the clock, so synthetic tracks
//! can be replayed through them in tests and backlog uploads behave like live
//! ones.
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, TimeZone as _, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use tokuly_proto::geo::Activity;
use uuid::Uuid;

use crate::math::{self, Point};

// ── Stays ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct StayRules {
    /// How far samples may scatter around the spot.
    pub radius_m: f64,
    /// How long the user must remain before the stay counts.
    pub min_duration_ms: i64,
}

/// GPS error we are willing to add to the radius, in meters.
const ACCURACY_ALLOWANCE_M: f64 = 30.0;

/// A cluster of consecutive samples around one spot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    /// Center of the cluster (fixed once the stay is confirmed).
    pub lat: f64,
    pub lng: f64,
    /// First and latest sample of the cluster.
    pub since_ms: i64,
    pub last_ms: i64,
    pub count: u32,
    /// The cluster has lasted long enough to be a stay.
    pub confirmed: bool,
}

impl Anchor {
    fn at(lat: f64, lng: f64, at_ms: i64) -> Self {
        Self { lat, lng, since_ms: at_ms, last_ms: at_ms, count: 1, confirmed: false }
    }

    pub fn center(&self) -> Point {
        (self.lat, self.lng)
    }
}

/// Where the user seems to be heading after a first sample outside a stay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Departure {
    pub lat: f64,
    pub lng: f64,
    pub at_ms: i64,
}

/// Per-user stay state. `anchor` is the current cluster; `leaving` is set
/// after one sample outside a confirmed stay — a single GPS outlier must not
/// end a stay, so it takes a second one.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StayTracker {
    pub anchor: Option<Anchor>,
    #[serde(default)]
    pub leaving: Option<Departure>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StayEvent {
    /// The user has been at `(lat, lng)` since `since_ms` (in the past by the minimum duration).
    Started { lat: f64, lng: f64, since_ms: i64 },
    /// The stay that began at `since_ms` ended; the user was last seen there at `until_ms`.
    Ended { lat: f64, lng: f64, since_ms: i64, until_ms: i64 },
}

impl StayTracker {
    /// The confirmed stay, if the user is in one.
    pub fn current(&self) -> Option<&Anchor> {
        self.anchor.as_ref().filter(|anchor| anchor.confirmed)
    }

    pub fn stationary_since_ms(&self) -> Option<i64> {
        self.current().map(|anchor| anchor.since_ms)
    }

    /// Feeds one sample (with usable accuracy) and returns what changed, in order.
    pub fn update(&mut self, rules: StayRules, lat: f64, lng: f64, accuracy_m: f64, at_ms: i64) -> Vec<StayEvent> {
        let mut events = Vec::new();
        let Some(anchor) = self.anchor.as_mut() else {
            self.anchor = Some(Anchor::at(lat, lng, at_ms));
            return events;
        };
        let reach = rules.radius_m + accuracy_m.clamp(0.0, ACCURACY_ALLOWANCE_M);
        let inside = math::distance((lat, lng), anchor.center()) <= reach;
        if inside {
            self.leaving = None;
            if !anchor.confirmed {
                // The center settles on the mean of the samples seen so far.
                let weight = (anchor.count + 1) as f64;
                anchor.lat += (lat - anchor.lat) / weight;
                anchor.lng += (lng - anchor.lng) / weight;
            }
            anchor.count = anchor.count.saturating_add(1);
            anchor.last_ms = anchor.last_ms.max(at_ms);
            if !anchor.confirmed && at_ms - anchor.since_ms >= rules.min_duration_ms {
                anchor.confirmed = true;
                events.push(StayEvent::Started { lat: anchor.lat, lng: anchor.lng, since_ms: anchor.since_ms });
            }
            return events;
        }
        if !anchor.confirmed {
            // Still on the move: follow the newest sample.
            self.anchor = Some(Anchor::at(lat, lng, at_ms));
            return events;
        }
        match self.leaving.take() {
            None => {
                // First sample outside: remember it, decide on the next one.
                self.leaving = Some(Departure { lat, lng, at_ms });
            }
            Some(first_outside) => {
                events.push(StayEvent::Ended {
                    lat: anchor.lat,
                    lng: anchor.lng,
                    since_ms: anchor.since_ms,
                    until_ms: anchor.last_ms,
                });
                // The new cluster starts where the user went, counting the first outside sample if it fits.
                let mut next = Anchor::at(first_outside.lat, first_outside.lng, first_outside.at_ms);
                if math::distance((lat, lng), next.center()) <= reach {
                    next.lat = (next.lat + lat) / 2.0;
                    next.lng = (next.lng + lng) / 2.0;
                    next.count = 2;
                    next.last_ms = at_ms;
                } else {
                    next = Anchor::at(lat, lng, at_ms);
                }
                self.anchor = Some(next);
            }
        }
        events
    }

    /// Ends tracking (sharing stopped): closes a confirmed stay and forgets the cluster.
    pub fn stop(&mut self) -> Option<StayEvent> {
        self.leaving = None;
        let anchor = self.anchor.take()?;
        anchor.confirmed.then_some(StayEvent::Ended {
            lat: anchor.lat,
            lng: anchor.lng,
            since_ms: anchor.since_ms,
            until_ms: anchor.last_ms,
        })
    }
}

// ── Proximity (gatherings) ──────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct ProximityRules {
    /// How long two friends must stay close before they count as together.
    pub min_duration_ms: i64,
    /// How long a friend may go unseen nearby before the two count as apart
    /// (covers sparse reporting and GPS wobble at the edge of the radius).
    pub grace_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Near {
    pub user_id: Uuid,
    /// When the friend was first and most recently seen nearby in this run.
    pub since_ms: i64,
    pub last_ms: i64,
}

/// Which friends have been near a user, and for how long.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProximityTracker {
    pub near: Vec<Near>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProximityUpdate {
    /// Friends who have been near for the minimum duration (and still are, within the grace period).
    pub together: Vec<Uuid>,
    /// Friends who were together with the user and are not near any more.
    pub parted: Vec<Uuid>,
}

impl ProximityTracker {
    /// Records which friends are near the user at `at_ms`.
    pub fn update(&mut self, rules: ProximityRules, at_ms: i64, near_now: &[Uuid]) -> ProximityUpdate {
        let mut update = ProximityUpdate::default();
        for user_id in near_now {
            match self.near.iter_mut().find(|near| near.user_id == *user_id) {
                Some(near) => near.last_ms = near.last_ms.max(at_ms),
                None => self.near.push(Near { user_id: *user_id, since_ms: at_ms, last_ms: at_ms }),
            }
        }
        self.near.retain(|near| {
            let fresh = at_ms - near.last_ms <= rules.grace_ms;
            if !fresh && near.last_ms - near.since_ms >= rules.min_duration_ms {
                update.parted.push(near.user_id);
            }
            fresh
        });
        update.together = self
            .near
            .iter()
            .filter(|near| at_ms - near.since_ms >= rules.min_duration_ms)
            .map(|near| near.user_id)
            .collect();
        update
    }

    /// Friends currently counted as together with the user (no new observation).
    pub fn together(&self, rules: ProximityRules, at_ms: i64) -> Vec<Uuid> {
        self.near
            .iter()
            .filter(|near| at_ms - near.last_ms <= rules.grace_ms && at_ms - near.since_ms >= rules.min_duration_ms)
            .map(|near| near.user_id)
            .collect()
    }

    /// Forgets a friend at once (unfriended, blocked).
    pub fn forget(&mut self, user_id: Uuid) {
        self.near.retain(|near| near.user_id != user_id);
    }
}

// ── Activity ────────────────────────────────────────────────────────────────

/// Guesses the movement activity from ground speed (m/s) when the device
/// reports none. Boats cannot be told from cars by speed and are never guessed.
pub fn activity_for_speed(speed_mps: f64) -> Activity {
    match speed_mps {
        s if !s.is_finite() || s < 0.5 => Activity::ACTIVITY_STILL,
        s if s < 2.3 => Activity::ACTIVITY_WALKING,
        s if s < 4.5 => Activity::ACTIVITY_RUNNING,
        s if s < 9.0 => Activity::ACTIVITY_CYCLING,
        // Up to 150 km/h.
        s if s < 42.0 => Activity::ACTIVITY_DRIVING,
        // High-speed rail tops out around 320 km/h.
        s if s < 95.0 => Activity::ACTIVITY_TRAIN,
        _ => Activity::ACTIVITY_FLYING,
    }
}

/// Ground speed between two consecutive samples, when the gap is short
/// enough for it to mean something.
pub fn speed_between(from: Point, from_ms: i64, to: Point, to_ms: i64) -> Option<f64> {
    let elapsed_ms = to_ms - from_ms;
    if !(1_000..=180_000).contains(&elapsed_ms) {
        return None;
    }
    Some(math::distance(from, to) / (elapsed_ms as f64 / 1000.0))
}

/// The activity and speed to record for a sample: the device's own activity
/// wins; otherwise a confirmed stay means still, and anything else is guessed
/// from the device speed or, failing that, the speed since the last sample.
pub fn resolve_activity(
    reported: Activity,
    device_speed: Option<f64>,
    derived_speed: Option<f64>,
    staying: bool,
) -> (Activity, f64) {
    let speed = device_speed.filter(|s| s.is_finite() && *s >= 0.0).or(derived_speed).unwrap_or(0.0);
    if reported != Activity::ACTIVITY_UNSPECIFIED {
        return (reported, speed);
    }
    if staying {
        return (Activity::ACTIVITY_STILL, speed);
    }
    (activity_for_speed(speed), speed)
}

// ── Sleep ───────────────────────────────────────────────────────────────────

/// Local time at which people may be assumed to go to bed, and get up.
const BEDTIME: NaiveTime = NaiveTime::from_hms_opt(21, 0, 0).unwrap();
const WAKE_TIME: NaiveTime = NaiveTime::from_hms_opt(10, 0, 0).unwrap();
/// How long someone must be still and away from the app before we call it sleep.
const SLEEP_AFTER: Duration = Duration::minutes(45);
/// A pin older than this says nothing about whether its owner sleeps.
const SLEEP_MAX_SILENCE: Duration = Duration::hours(14);

#[derive(Debug, Clone, Copy)]
pub struct SleepInput {
    /// Start of the confirmed stay, if there is one.
    pub stationary_since: Option<DateTime<Utc>>,
    /// When the app was last seen in the foreground.
    pub last_active: Option<DateTime<Utc>>,
    /// When the device last reported a position.
    pub last_sample: DateTime<Utc>,
    pub online: bool,
    pub now: DateTime<Utc>,
}

/// The local instant `time` on the latest day on which it is not after `now`.
fn latest_local(now: DateTime<Utc>, zone: Tz, time: NaiveTime) -> DateTime<Utc> {
    let local = now.with_timezone(&zone);
    let mut day = local.date_naive();
    if local.time() < time {
        day = day.pred_opt().unwrap_or(day);
    }
    local_instant(zone, day, time)
}

/// A local date and time as an instant. Times skipped or repeated by a
/// daylight-saving change resolve to the earliest valid reading.
fn local_instant(zone: Tz, day: NaiveDate, time: NaiveTime) -> DateTime<Utc> {
    let naive = day.and_time(time);
    match zone.from_local_datetime(&naive).earliest() {
        Some(instant) => instant.with_timezone(&Utc),
        // Skipped by a DST jump: an hour later exists.
        None => zone
            .from_local_datetime(&(naive + Duration::hours(1)))
            .earliest()
            .map(|i| i.with_timezone(&Utc))
            .unwrap_or_else(|| naive.and_utc()),
    }
}

/// Since when the user has probably been asleep: at night (local time at
/// their position), in a stay, with the app closed, for a while.
pub fn sleeping_since(input: SleepInput, zone: Tz) -> Option<DateTime<Utc>> {
    if input.online {
        return None;
    }
    let stationary_since = input.stationary_since?;
    if input.now - input.last_sample > SLEEP_MAX_SILENCE {
        return None;
    }
    let local_time = input.now.with_timezone(&zone).time();
    if local_time >= WAKE_TIME && local_time < BEDTIME {
        return None;
    }
    let bedtime = latest_local(input.now, zone, BEDTIME);
    let still_since = input.last_active.map_or(stationary_since, |active| active.max(stationary_since));
    let since = still_since.max(bedtime);
    (input.now - since >= SLEEP_AFTER).then_some(since)
}

// ── Nights ──────────────────────────────────────────────────────────────────

/// A night counts when a stay covers this much of 22:00–07:00.
const NIGHT_MIN_OVERLAP: Duration = Duration::hours(3);
const NIGHT_START: NaiveTime = NaiveTime::from_hms_opt(22, 0, 0).unwrap();
const NIGHT_LENGTH: Duration = Duration::hours(9);
/// A night's reported span is clipped to 18:00–12:00 around it.
const EVENING_BEFORE: Duration = Duration::hours(4);
const MORNING_AFTER: Duration = Duration::hours(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NightSpan {
    /// The evening's date.
    pub date: NaiveDate,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

/// The most recent night (local time `zone`) that a stay from `since` to
/// `until` covers for at least three hours, if any.
pub fn night_of(since: DateTime<Utc>, until: DateTime<Utc>, zone: Tz) -> Option<NightSpan> {
    if until <= since {
        return None;
    }
    let window_start = latest_local(until, zone, NIGHT_START);
    let window_end = window_start + NIGHT_LENGTH;
    let overlap = until.min(window_end) - since.max(window_start);
    if overlap < NIGHT_MIN_OVERLAP {
        return None;
    }
    Some(NightSpan {
        date: window_start.with_timezone(&zone).date_naive(),
        from: since.max(window_start - EVENING_BEFORE),
        to: until.min(window_end + MORNING_AFTER),
    })
}

/// Whether a stay looks like time at work or school: a weekday, mostly
/// between 08:00 and 19:00 local time, three hours or more.
pub fn is_workday_stay(since: DateTime<Utc>, until: DateTime<Utc>, zone: Tz) -> bool {
    use chrono::{Datelike as _, Timelike as _, Weekday};
    if until - since < Duration::hours(3) {
        return false;
    }
    let (start, end) = (since.with_timezone(&zone), until.with_timezone(&zone));
    let weekday = !matches!(start.weekday(), Weekday::Sat | Weekday::Sun);
    weekday
        && start.date_naive() == end.date_naive()
        && start.hour() >= 6
        && end.hour() <= 20
        && (8..=14).contains(&start.hour())
}

#[cfg(test)]
mod tests {
    use chrono_tz::Asia::Tokyo;

    use super::*;

    const RULES: StayRules = StayRules { radius_m: 80.0, min_duration_ms: 300_000 };
    const HOME: Point = (35.6595, 139.7005);

    /// A point `meters` away from `origin` in direction `bearing`.
    fn offset(origin: Point, bearing: f64, meters: f64) -> Point {
        math::destination(origin.0, origin.1, bearing, meters)
    }

    /// Feeds a track of `(point, seconds since start)` and collects the events.
    fn replay(tracker: &mut StayTracker, track: &[(Point, i64)]) -> Vec<StayEvent> {
        track.iter().flat_map(|((lat, lng), seconds)| tracker.update(RULES, *lat, *lng, 10.0, seconds * 1000)).collect()
    }

    #[test]
    fn standing_still_becomes_a_stay_after_five_minutes() {
        let mut tracker = StayTracker::default();
        // GPS wobble of a few meters around home, one sample every 30 s.
        let track: Vec<(Point, i64)> =
            (0..=12).map(|i| (offset(HOME, (i * 67) as f64, (i % 4) as f64 * 6.0), i * 30)).collect();
        let events = replay(&mut tracker, &track[..10]);
        assert!(events.is_empty(), "4.5 minutes is not a stay yet");
        assert_eq!(tracker.stationary_since_ms(), None);
        let events = replay(&mut tracker, &track[10..]);
        assert_eq!(events.len(), 1);
        let StayEvent::Started { lat, lng, since_ms } = events[0] else { panic!("{events:?}") };
        assert_eq!(since_ms, 0, "the stay began with the first sample, not when it was confirmed");
        assert!(math::distance((lat, lng), HOME) < 15.0);
        assert_eq!(tracker.stationary_since_ms(), Some(0));
    }

    #[test]
    fn walking_never_becomes_a_stay() {
        let mut tracker = StayTracker::default();
        // 1.4 m/s due east for 20 minutes.
        let track: Vec<(Point, i64)> = (0..=120).map(|i| (offset(HOME, 90.0, i as f64 * 14.0), i * 10)).collect();
        assert!(replay(&mut tracker, &track).is_empty());
        assert!(tracker.current().is_none());
    }

    #[test]
    fn leaving_ends_the_stay_at_the_last_sample_inside() {
        let mut tracker = StayTracker::default();
        let mut track: Vec<(Point, i64)> = (0..=20).map(|i| (HOME, i * 30)).collect(); // 10 minutes at home
        track.extend((1..=6).map(|i| (offset(HOME, 0.0, 100.0 + i as f64 * 50.0), 600 + i * 10))); // walks away
        let events = replay(&mut tracker, &track);
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(matches!(events[0], StayEvent::Started { since_ms: 0, .. }));
        let StayEvent::Ended { since_ms, until_ms, lat, lng } = events[1] else { panic!("{events:?}") };
        assert_eq!((since_ms, until_ms), (0, 600_000));
        assert!(math::distance((lat, lng), HOME) < 1.0);
        assert!(tracker.current().is_none());
        assert_eq!(tracker.leaving, None);
    }

    #[test]
    fn a_single_outlier_does_not_end_a_stay() {
        let mut tracker = StayTracker::default();
        let mut track: Vec<(Point, i64)> = (0..=12).map(|i| (HOME, i * 30)).collect();
        track.push((offset(HOME, 45.0, 400.0), 390)); // GPS glitch
        track.extend((14..=20).map(|i| (HOME, i * 30)));
        let events = replay(&mut tracker, &track);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(tracker.stationary_since_ms(), Some(0));
        assert_eq!(tracker.current().unwrap().last_ms, 600_000);
    }

    #[test]
    fn a_new_stay_starts_where_the_user_went() {
        let mut tracker = StayTracker::default();
        let cafe = offset(HOME, 120.0, 900.0);
        let mut track: Vec<(Point, i64)> = (0..=12).map(|i| (HOME, i * 30)).collect(); // home until 6:00
        track.extend((1..=9).map(|i| (offset(HOME, 120.0, i as f64 * 100.0), 360 + i * 60))); // walk 9 minutes
        track.extend((0..=12).map(|i| (cafe, 900 + i * 30))); // café from 15:00
        let events = replay(&mut tracker, &track);
        assert_eq!(events.len(), 3, "{events:?}");
        assert!(matches!(events[1], StayEvent::Ended { until_ms: 360_000, .. }));
        let StayEvent::Started { lat, lng, since_ms } = events[2] else { panic!("{events:?}") };
        assert!(math::distance((lat, lng), cafe) < 5.0);
        assert_eq!(since_ms, 900_000);
    }

    #[test]
    fn poor_accuracy_widens_the_radius_only_a_little() {
        let mut tracker = StayTracker::default();
        for i in 0..=12 {
            tracker.update(RULES, HOME.0, HOME.1, 10.0, i * 30_000);
        }
        assert!(tracker.current().is_some());
        // 100 m away with 25 m accuracy: within 80 + 25.
        let near = offset(HOME, 10.0, 100.0);
        assert!(tracker.update(RULES, near.0, near.1, 25.0, 400_000).is_empty());
        assert_eq!(tracker.leaving, None);
        // 150 m away with awful accuracy: the allowance is capped at 30 m.
        let far = offset(HOME, 10.0, 150.0);
        tracker.update(RULES, far.0, far.1, 500.0, 410_000);
        assert!(tracker.leaving.is_some());
    }

    #[test]
    fn a_long_silence_at_the_same_spot_continues_the_stay() {
        let mut tracker = StayTracker::default();
        for i in 0..=12 {
            tracker.update(RULES, HOME.0, HOME.1, 10.0, i * 30_000);
        }
        // Phone off for eight hours, then the same spot.
        assert!(tracker.update(RULES, HOME.0, HOME.1, 10.0, 8 * 3_600_000).is_empty());
        assert_eq!(tracker.stationary_since_ms(), Some(0));
    }

    #[test]
    fn stopping_closes_a_confirmed_stay_only() {
        let mut tracker = StayTracker::default();
        tracker.update(RULES, HOME.0, HOME.1, 10.0, 0);
        assert_eq!(tracker.stop(), None);
        for i in 0..=12 {
            tracker.update(RULES, HOME.0, HOME.1, 10.0, i * 30_000);
        }
        assert!(matches!(tracker.stop(), Some(StayEvent::Ended { since_ms: 0, until_ms: 360_000, .. })));
        assert_eq!(tracker, StayTracker::default());
    }

    const TOGETHER: ProximityRules = ProximityRules { min_duration_ms: 120_000, grace_ms: 90_000 };

    #[test]
    fn friends_are_together_after_two_minutes_nearby() {
        let (aoi, ren) = (Uuid::now_v7(), Uuid::now_v7());
        let mut tracker = ProximityTracker::default();
        assert!(tracker.update(TOGETHER, 0, &[aoi]).together.is_empty());
        assert!(tracker.update(TOGETHER, 60_000, &[aoi, ren]).together.is_empty());
        assert_eq!(tracker.update(TOGETHER, 120_000, &[aoi, ren]).together, vec![aoi]);
        let update = tracker.update(TOGETHER, 180_000, &[aoi, ren]);
        assert_eq!(update.together, vec![aoi, ren]);
        assert!(update.parted.is_empty());
        assert_eq!(tracker.together(TOGETHER, 200_000), vec![aoi, ren]);
    }

    #[test]
    fn passing_by_is_not_a_gathering() {
        let aoi = Uuid::now_v7();
        let mut tracker = ProximityTracker::default();
        tracker.update(TOGETHER, 0, &[aoi]);
        tracker.update(TOGETHER, 30_000, &[aoi]);
        // Gone for longer than the grace period, then near again: the clock restarts.
        let update = tracker.update(TOGETHER, 200_000, &[]);
        assert!(update.together.is_empty() && update.parted.is_empty());
        assert!(tracker.near.is_empty());
        assert!(tracker.update(TOGETHER, 210_000, &[aoi]).together.is_empty());
        assert!(tracker.update(TOGETHER, 300_000, &[aoi]).together.is_empty());
        assert_eq!(tracker.update(TOGETHER, 330_000, &[aoi]).together, vec![aoi]);
    }

    #[test]
    fn a_short_gap_keeps_friends_together_and_a_long_one_parts_them() {
        let aoi = Uuid::now_v7();
        let mut tracker = ProximityTracker::default();
        tracker.update(TOGETHER, 0, &[aoi]);
        tracker.update(TOGETHER, 150_000, &[aoi]);
        // Not seen for a minute: still together.
        let update = tracker.update(TOGETHER, 210_000, &[]);
        assert_eq!(update.together, vec![aoi]);
        assert!(update.parted.is_empty());
        // Not seen for more than the grace period: parted.
        let update = tracker.update(TOGETHER, 250_000, &[]);
        assert!(update.together.is_empty());
        assert_eq!(update.parted, vec![aoi]);
        assert!(tracker.together(TOGETHER, 250_000).is_empty());
    }

    #[test]
    fn forgetting_a_friend_removes_them_at_once() {
        let aoi = Uuid::now_v7();
        let mut tracker = ProximityTracker::default();
        tracker.update(TOGETHER, 0, &[aoi]);
        tracker.update(TOGETHER, 150_000, &[aoi]);
        tracker.forget(aoi);
        assert!(tracker.together(TOGETHER, 150_000).is_empty());
    }

    #[test]
    fn activity_follows_speed() {
        for (speed, expected) in [
            (0.0, Activity::ACTIVITY_STILL),
            (1.4, Activity::ACTIVITY_WALKING),
            (3.2, Activity::ACTIVITY_RUNNING),
            (6.0, Activity::ACTIVITY_CYCLING),
            (16.0, Activity::ACTIVITY_DRIVING),
            (75.0, Activity::ACTIVITY_TRAIN),
            (240.0, Activity::ACTIVITY_FLYING),
            (f64::NAN, Activity::ACTIVITY_STILL),
        ] {
            assert_eq!(activity_for_speed(speed), expected, "{speed} m/s");
        }
    }

    #[test]
    fn speed_is_derived_only_from_plausible_gaps() {
        let there = offset(HOME, 90.0, 140.0);
        let speed = speed_between(HOME, 0, there, 100_000).unwrap();
        assert!((speed - 1.4).abs() < 0.01, "{speed}");
        assert_eq!(speed_between(HOME, 0, there, 500), None, "too close together");
        assert_eq!(speed_between(HOME, 0, there, 600_000), None, "too far apart");
        assert_eq!(speed_between(HOME, 10_000, there, 0), None, "backwards");
    }

    #[test]
    fn the_device_activity_wins_over_inference() {
        assert_eq!(resolve_activity(Activity::ACTIVITY_CYCLING, Some(1.0), None, false).0, Activity::ACTIVITY_CYCLING);
        assert_eq!(
            resolve_activity(Activity::ACTIVITY_UNSPECIFIED, Some(1.2), Some(30.0), false),
            (Activity::ACTIVITY_WALKING, 1.2)
        );
        assert_eq!(
            resolve_activity(Activity::ACTIVITY_UNSPECIFIED, None, Some(20.0), false),
            (Activity::ACTIVITY_DRIVING, 20.0)
        );
        // A negative device speed means "unknown" on iOS.
        assert_eq!(
            resolve_activity(Activity::ACTIVITY_UNSPECIFIED, Some(-1.0), Some(3.0), false).0,
            Activity::ACTIVITY_RUNNING
        );
        // GPS wobble inside a stay is not walking.
        assert_eq!(resolve_activity(Activity::ACTIVITY_UNSPECIFIED, None, Some(1.0), true).0, Activity::ACTIVITY_STILL);
        assert_eq!(
            resolve_activity(Activity::ACTIVITY_UNSPECIFIED, None, None, false),
            (Activity::ACTIVITY_STILL, 0.0)
        );
    }

    fn jst(day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Tokyo.with_ymd_and_hms(2026, 11, day, hour, minute, 0).unwrap().with_timezone(&Utc)
    }

    fn asleep(
        stationary: DateTime<Utc>,
        active: Option<DateTime<Utc>>,
        online: bool,
        now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        sleeping_since(
            SleepInput {
                stationary_since: Some(stationary),
                last_active: active,
                last_sample: now - Duration::minutes(10),
                online,
                now,
            },
            Tokyo,
        )
    }

    #[test]
    fn stillness_at_night_with_the_app_closed_is_sleep() {
        // Home since 19:00, last used the app at 23:10.
        let (home_since, last_used) = (jst(20, 19, 0), jst(20, 23, 10));
        assert_eq!(asleep(home_since, Some(last_used), false, jst(20, 23, 40)), None, "only 30 minutes of quiet");
        assert_eq!(asleep(home_since, Some(last_used), false, jst(21, 0, 0)), Some(last_used));
        assert_eq!(asleep(home_since, Some(last_used), false, jst(21, 6, 30)), Some(last_used));
        assert_eq!(asleep(home_since, Some(last_used), true, jst(21, 3, 0)), None, "online means awake");
        assert_eq!(asleep(home_since, Some(last_used), false, jst(21, 11, 0)), None, "late morning is not night");
    }

    #[test]
    fn sleep_starts_no_earlier_than_bedtime() {
        // Still and offline since the afternoon: asleep since 21:00, not since 15:00.
        assert_eq!(asleep(jst(20, 15, 0), None, false, jst(20, 20, 30)), None);
        assert_eq!(asleep(jst(20, 15, 0), None, false, jst(20, 22, 0)), Some(jst(20, 21, 0)));
        assert_eq!(asleep(jst(20, 15, 0), None, false, jst(21, 2, 0)), Some(jst(20, 21, 0)));
    }

    #[test]
    fn no_stay_or_a_silent_phone_means_no_sleep() {
        let now = jst(21, 2, 0);
        let moving = SleepInput { stationary_since: None, last_active: None, last_sample: now, online: false, now };
        assert_eq!(sleeping_since(moving, Tokyo), None);
        let silent = SleepInput {
            stationary_since: Some(jst(19, 20, 0)),
            last_active: None,
            last_sample: jst(20, 8, 0),
            online: false,
            now,
        };
        assert_eq!(sleeping_since(silent, Tokyo), None, "nothing heard for 18 hours");
    }

    #[test]
    fn night_is_judged_in_the_local_time_zone() {
        // 02:00 in Tokyo is 12:00 the day before in New York: asleep here, not there.
        let now = jst(21, 2, 0);
        let input = SleepInput {
            stationary_since: Some(jst(20, 18, 0)),
            last_active: None,
            last_sample: now,
            online: false,
            now,
        };
        assert!(sleeping_since(input, Tokyo).is_some());
        assert!(sleeping_since(input, chrono_tz::America::New_York).is_none());
    }

    #[test]
    fn a_stay_through_the_night_is_a_night() {
        // Home from 19:30 to 08:15.
        let night = night_of(jst(20, 19, 30), jst(21, 8, 15), Tokyo).unwrap();
        assert_eq!(night.date, NaiveDate::from_ymd_opt(2026, 11, 20).unwrap());
        assert_eq!((night.from, night.to), (jst(20, 19, 30), jst(21, 8, 15)));
        // Still there at 01:00: three hours of the night have passed.
        assert_eq!(
            night_of(jst(20, 19, 30), jst(21, 1, 0), Tokyo).unwrap().date,
            NaiveDate::from_ymd_opt(2026, 11, 20).unwrap()
        );
        assert_eq!(night_of(jst(20, 19, 30), jst(21, 0, 30), Tokyo), None, "two and a half hours so far");
    }

    #[test]
    fn evenings_out_and_daytime_stays_are_not_nights() {
        assert_eq!(night_of(jst(20, 19, 0), jst(20, 23, 30), Tokyo), None, "left before 01:00");
        assert_eq!(night_of(jst(20, 9, 0), jst(20, 18, 0), Tokyo), None, "a day at the office");
        assert_eq!(night_of(jst(21, 5, 0), jst(21, 12, 0), Tokyo), None, "arrived at dawn");
        assert_eq!(night_of(jst(21, 5, 0), jst(21, 4, 0), Tokyo), None, "inverted range");
    }

    #[test]
    fn a_weekend_at_home_reports_the_latest_night_clipped() {
        // Home from Friday 18:00, asked on Sunday 09:00: Saturday night.
        let night = night_of(jst(20, 18, 0), jst(22, 9, 0), Tokyo).unwrap();
        assert_eq!(night.date, NaiveDate::from_ymd_opt(2026, 11, 21).unwrap());
        assert_eq!((night.from, night.to), (jst(21, 18, 0), jst(22, 9, 0)));
    }

    #[test]
    fn workday_stays_are_weekday_daytime_stays() {
        // 2026-11-20 is a Friday, 2026-11-21 a Saturday.
        assert!(is_workday_stay(jst(20, 9, 0), jst(20, 18, 0), Tokyo));
        assert!(!is_workday_stay(jst(21, 9, 0), jst(21, 18, 0), Tokyo), "weekend");
        assert!(!is_workday_stay(jst(20, 9, 0), jst(20, 11, 0), Tokyo), "too short");
        assert!(!is_workday_stay(jst(20, 19, 0), jst(21, 8, 0), Tokyo), "overnight");
    }
}
