//! The ingest pipeline: every location sample, whether it arrives over the
//! realtime gateway (`LOCATIONS` stream) or through `ReportLocations`, goes
//! through [`Geo::ingest_batch`].
//!
//! Per user, under their lock and in order:
//! 1. validate; drop duplicates, samples older than the newest one known and
//!    samples recorded before the latest stop;
//! 2. update the live state — stay detection, activity, area name, distance,
//!    footprints, raw history, nights (history only if the user records it);
//! 3. for a live sample: place alerts, the active trip, proximity to friends
//!    (gatherings);
//! 4. publish: the shared position index, the pin to each friend through
//!    their ghost rule, live shares, visit and footprint events.
//!
//! Steps 1–2 treat a backlog upload exactly like live samples (sample time
//! drives every detector), so history comes out the same either way. Step 3
//! only looks at the present.
use std::hash::{Hash as _, Hasher as _};

use chrono::{DateTime, Duration, Utc};
use tokuly_core::{geoindex, prelude::*, presence::LocationReport};
use tokuly_proto::{
    Timestamp,
    geo::{Activity, FootprintProgress, LocationSample, geo_event::Event},
};
use tokuly_social::graph;

use crate::{
    Geo, claims,
    ctx::OwnerCtx,
    detect::{ProximityRules, StayEvent, StayRules, resolve_activity, speed_between},
    fanout::{Recipients, VisitNews},
    footprints::Uncovered,
    gatherings::Changes,
    geocode::Gazetteer,
    history::PointRow,
    math::{self, Point},
    odometer::MAX_SPEED_MPS,
    state::{LiveState, Pushed, UserLock},
    visibility,
};

/// Device clocks run ahead a little; more than this is a broken clock.
const MAX_CLOCK_SKEW: Duration = Duration::minutes(2);
/// Samples closer together than this (device time) are dropped: nothing
/// moves fast enough to need them, and it bounds what one account can make
/// the pipeline do — a week of history is 600,000 samples at most.
const MIN_SPACING_MS: i64 = 1000;
/// The user's lock is renewed after this many samples of a batch.
const RENEW_EVERY: usize = 50;
/// Positions less certain than this are not positions.
const MAX_ACCURACY_M: f64 = 10_000.0;
/// Only fixes at least this good feed stays, distance and footprints.
const FINE_ACCURACY_M: f64 = 100.0;
/// A coarse fix (cell tower, Wi-Fi) within this long of a good one is ignored
/// as a position: it would only make the pin jump.
const COARSE_AFTER_MS: i64 = 3 * 60 * 1000;
/// The area name is looked up again after moving this far.
const AREA_REFRESH_M: f64 = 250.0;
/// Pending distance is stored after this much sample time.
const ODOMETER_EVERY_MS: i64 = 60_000;
/// Footprints between two samples are filled in when the samples are at most this far apart.
const BRIDGE_MS: i64 = 5 * 60 * 1000;
const BRIDGE_M: f64 = 5_000.0;
const BRIDGE_CELLS: usize = 64;
/// Samples older than this are history, not the present: no alerts, trips or gatherings.
const LIVE_WINDOW_MS: i64 = 5 * 60 * 1000;
/// Proximity to friends is checked at most this often (sample time).
const PROXIMITY_EVERY_MS: i64 = 15_000;
/// A friend's position counts for proximity if it is this fresh — or this
/// fresh while they are staying somewhere (still devices report rarely).
const NEAR_FRESH_MS: i64 = 10 * 60 * 1000;
const NEAR_FRESH_STAYING_MS: i64 = 45 * 60 * 1000;
/// Friends at the edge of the radius may wobble out for this long.
const PROXIMITY_GRACE_MS: i64 = 90_000;
/// GPS error allowed on top of the gathering radius.
const NEAR_ALLOWANCE_M: f64 = 20.0;
/// Gathering membership is confirmed in the database this often.
const GATHERING_HEARTBEAT_MS: i64 = 60_000;
/// Friends get a pin at least this often while samples keep coming...
const PUSH_EVERY_MS: i64 = 30_000;
/// ...and at once when it moved this far or anything else about it changed.
const PUSH_MOVED_M: f64 = 5.0;

/// A sample that passed validation.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub lat: f64,
    pub lng: f64,
    pub accuracy: f64,
    pub altitude: Option<f64>,
    pub speed: Option<f64>,
    pub heading: Option<f64>,
    pub recorded_ms: i64,
    pub battery: Option<i32>,
    pub charging: Option<bool>,
    pub activity: Activity,
    pub mocked: bool,
    pub foreground: bool,
}

/// Why a sample is unusable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    NoPosition,
    BadAccuracy,
    FromTheFuture,
    TooOld,
}

/// Checks a device reading received at `received_at`. Unusable optional
/// values (a NaN speed, a battery of 150%) are dropped rather than failing
/// the sample; a missing `recorded_at` means "now".
pub fn validate(sample: &LocationSample, received_at: DateTime<Utc>, max_age: Duration) -> Result<Sample, Rejection> {
    let position = sample.position.as_option().ok_or(Rejection::NoPosition)?;
    if !math::valid_coordinates(position.lat, position.lng) {
        return Err(Rejection::NoPosition);
    }
    if !sample.accuracy_meters.is_finite() || !(0.0..=MAX_ACCURACY_M).contains(&sample.accuracy_meters) {
        return Err(Rejection::BadAccuracy);
    }
    let recorded_ms = match sample.recorded_at.as_option() {
        None => received_at.timestamp_millis(),
        Some(recorded_at) => {
            // Compared on the raw seconds: converting first would clamp nonsense into range.
            if recorded_at.seconds > (received_at + MAX_CLOCK_SKEW).timestamp() {
                return Err(Rejection::FromTheFuture);
            }
            if recorded_at.seconds < (received_at - max_age).timestamp() {
                return Err(Rejection::TooOld);
            }
            from_ts(recorded_at).timestamp_millis()
        }
    };
    let finite = |value: Option<f64>| value.filter(|v| v.is_finite());
    Ok(Sample {
        lat: position.lat,
        lng: position.lng,
        accuracy: sample.accuracy_meters,
        altitude: finite(sample.altitude_meters).filter(|m| (-1_000.0..=100_000.0).contains(m)),
        speed: finite(sample.speed_mps).filter(|s| (0.0..=MAX_SPEED_MPS).contains(s)),
        heading: finite(sample.heading_degrees).filter(|h| (0.0..=360.0).contains(h)).map(|h| h % 360.0),
        recorded_ms,
        battery: sample.battery_percent.filter(|b| (0..=100).contains(b)),
        charging: sample.charging,
        activity: sample.activity.as_known().unwrap_or(Activity::ACTIVITY_UNSPECIFIED),
        mocked: sample.mocked,
        foreground: sample.foreground,
    })
}

/// When a device stopped sharing, by its own clock (the one its samples'
/// times come from): the `stopped_at` it sent, or the server's time when it
/// sent none. A time further ahead than any sample may be is capped there.
pub fn stop_time(stopped_at: Option<&Timestamp>, received_at: DateTime<Utc>) -> i64 {
    let latest = received_at + MAX_CLOCK_SKEW;
    match stopped_at {
        None => received_at.timestamp_millis(),
        // Compared on the raw seconds: converting first would clamp nonsense into range.
        Some(at) if at.seconds > latest.timestamp() => latest.timestamp_millis(),
        Some(at) => from_ts(at).timestamp_millis().min(latest.timestamp_millis()),
    }
}

/// Everything about a pin a friend could notice besides its position.
fn fingerprint(state: &LiveState) -> u64 {
    let pin = &state.pin;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (pin.battery, pin.charging, pin.activity, pin.sharing, pin.stationary_since_ms).hash(&mut hasher);
    pin.place.as_ref().map(|place| place.id).hash(&mut hasher);
    pin.area.locality.hash(&mut hasher);
    hasher.finish()
}

/// Whether friends should be shown the pin again.
fn worth_pushing(state: &LiveState) -> bool {
    match &state.pushed {
        None => true,
        Some(pushed) => {
            state.pin.recorded_ms - pushed.at_ms >= PUSH_EVERY_MS
                || math::distance((pushed.lat, pushed.lng), state.pin.position()) >= PUSH_MOVED_M
                || pushed.fingerprint != fingerprint(state)
        }
    }
}

fn from_millis(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap_or_default()
}

/// What processing a batch produced besides the new state.
#[derive(Default)]
struct Effects {
    visits: Vec<VisitNews>,
    footprints: Vec<Uncovered>,
    gatherings: Option<Changes>,
}

impl Geo {
    /// Processes one location report directly, as if it had arrived from the
    /// realtime gateway. For simulators and tools; the gateway path and
    /// `ReportLocations` end up in the same place. Raw history is buffered:
    /// a process that does not run [`Geo::spawn_workers`] calls
    /// [`Geo::flush_history`] when it is done.
    pub async fn ingest(&self, report: LocationReport<LocationSample>) -> AppResult<()> {
        self.ingest_batch(report.user_id, report.received_at, std::slice::from_ref(&report.sample)).await.map(|_| ())
    }

    /// Processes samples of one user, oldest first, and returns how many
    /// were accepted.
    pub(crate) async fn ingest_batch(
        &self,
        user_id: Uuid,
        received_at: DateTime<Utc>,
        samples: &[LocationSample],
    ) -> AppResult<usize> {
        let max_age = Duration::hours(self.cfg.max_sample_age_hours);
        let mut valid: Vec<Sample> =
            samples.iter().filter_map(|sample| validate(sample, received_at, max_age).ok()).collect();
        if valid.is_empty() {
            return Ok(0);
        }
        valid.sort_by_key(|sample| sample.recorded_ms);
        Gazetteer::ready().await;
        let lock = self.lock_user(user_id).await?;
        let result = self.process(user_id, valid, &lock).await;
        self.unlock_user(lock).await;
        result
    }

    async fn process(&self, user_id: Uuid, samples: Vec<Sample>, lock: &UserLock) -> AppResult<usize> {
        let ctx = self.ctx_of(user_id).await?;
        if !ctx.active {
            return Ok(0);
        }
        let mut state = self.load_state(user_id).await?.unwrap_or_default();
        let mut effects = Effects::default();
        let mut accepted = 0;
        // When this upload arrived, by the server's clock: device clocks may be off.
        let arrived_ms = Utc::now().timestamp_millis();
        for (index, sample) in samples.iter().enumerate() {
            // Duplicates, stragglers and floods: the state only moves forward, a second at a time.
            if state.located() && sample.recorded_ms < state.pin.recorded_ms + MIN_SPACING_MS {
                continue;
            }
            // Recorded before the device stopped, and only now arriving: it must not resume sharing.
            if sample.recorded_ms <= state.stopped_ms {
                continue;
            }
            self.apply(user_id, &ctx, &mut state, sample, &mut effects).await?;
            // Only the newest sample of an upload is the present (see `claims`).
            state.trust.take(&claims::Taken {
                lat: sample.lat,
                lng: sample.lng,
                accuracy: sample.accuracy,
                mocked: sample.mocked,
                staying: state.stay.current().is_some(),
                newest: index + 1 == samples.len(),
                arrived_ms,
            });
            accepted += 1;
            // A long batch outlives the lock's own lifetime unless it says it is still at it.
            if accepted % RENEW_EVERY == 0 {
                self.renew_lock(lock).await?;
            }
        }
        if accepted == 0 {
            return Ok(0);
        }

        let pin_at = from_millis(state.pin.recorded_ms);
        // The true position goes into the shared index together with who
        // may be told anything about it: other domains ask the index.
        geoindex::set_sight(&self.core.cache, user_id, &ctx.sight()).await?;
        geoindex::set(&self.core.cache, user_id, state.pin.lat, state.pin.lng, pin_at).await?;
        let live = Utc::now().timestamp_millis() - state.pin.recorded_ms <= LIVE_WINDOW_MS;
        if live && state.last_fine_ms == state.pin.recorded_ms {
            self.evaluate_alerts(user_id, &ctx, &state.pin).await?;
            self.advance_trip(user_id, &ctx, &mut state).await?;
            effects.gatherings = self.check_proximity(user_id, &ctx, &mut state).await?;
        }
        let push = worth_pushing(&state);
        if push {
            state.pushed = Some(Pushed {
                at_ms: state.pin.recorded_ms,
                lat: state.pin.lat,
                lng: state.pin.lng,
                fingerprint: fingerprint(&state),
            });
        }
        self.save_state_locked(user_id, &state, lock).await?;

        // The state is stored: what follows only tells people about it.
        for uncovered in &effects.footprints {
            self.push_footprint_progress(user_id, uncovered, ctx.ja).await;
        }
        // (Arrivals and departures of a backlog are history, not news.)
        if live {
            self.push_visit_news(user_id, &ctx, &effects.visits).await?;
        }
        if push {
            self.fan_out(user_id, Some(&state.pin), &ctx, Recipients::Friends, false).await?;
            self.publish_shares(user_id, &state.pin, &ctx).await;
        }
        if let Some(changes) = effects.gatherings {
            self.publish_gathering_changes(changes).await?;
        }
        Ok(accepted)
    }

    /// Applies one accepted sample to the live state.
    async fn apply(
        &self,
        user_id: Uuid,
        ctx: &OwnerCtx,
        state: &mut LiveState,
        sample: &Sample,
        effects: &mut Effects,
    ) -> AppResult<()> {
        let at_ms = sample.recorded_ms;
        let position = (sample.lat, sample.lng);
        let fine = sample.accuracy <= FINE_ACCURACY_M;
        let record = ctx.privacy.record_history;
        // The previous good fix, for speed and for bridging footprints.
        let previous: Option<(Point, i64)> = (state.located() && state.last_fine_ms == state.pin.recorded_ms)
            .then(|| (state.pin.position(), state.pin.recorded_ms));
        let keep_position = !fine && state.located() && at_ms - state.last_fine_ms < COARSE_AFTER_MS;

        if !keep_position {
            let derived_speed =
                previous.filter(|_| fine).and_then(|(from, from_ms)| speed_between(from, from_ms, position, at_ms));
            if fine {
                let rules =
                    StayRules { radius_m: self.cfg.stay_radius_meters, min_duration_ms: self.cfg.stay_seconds * 1000 };
                for event in state.stay.update(rules, sample.lat, sample.lng, sample.accuracy, at_ms) {
                    let news = match event {
                        StayEvent::Started { lat, lng, since_ms } => {
                            let live = Utc::now().timestamp_millis() - at_ms <= LIVE_WINDOW_MS;
                            self.begin_stay(user_id, ctx, state, (lat, lng), since_ms, live).await?
                        }
                        StayEvent::Ended { until_ms, .. } => self.finish_stay(user_id, ctx, state, until_ms).await?,
                    };
                    effects.visits.push(news);
                }
            }
            let (activity, speed) =
                resolve_activity(sample.activity, sample.speed, derived_speed, state.stay.current().is_some());
            state.pin.lat = sample.lat;
            state.pin.lng = sample.lng;
            state.pin.accuracy = sample.accuracy;
            state.pin.speed = speed;
            state.pin.heading = sample.heading;
            state.pin.activity = activity as i32;
            state.altitude = sample.altitude;
            if state.area_at.is_none_or(|looked_up| math::distance(looked_up, position) >= AREA_REFRESH_M) {
                state.pin.area = Gazetteer::shared().area(sample.lat, sample.lng);
                state.area_at = Some(position);
            }
            if fine {
                state.last_fine_ms = at_ms;
                if record {
                    if let Some(bucket) = state.odometer.add(position, at_ms, sample.accuracy, activity as i32, speed) {
                        self.store_hours(user_id, &bucket, &state.pin.area, &state.region).await?;
                    }
                    if state.odometer.due(at_ms, ODOMETER_EVERY_MS) {
                        if let Some(bucket) = state.odometer.take() {
                            self.store_hours(user_id, &bucket, &state.pin.area, &state.region).await?;
                        }
                    }
                    if !sample.mocked {
                        self.leave_footprint(user_id, ctx, state, position, at_ms, previous, effects).await?;
                    }
                    self.record_night(user_id, ctx, state, from_millis(at_ms), false).await?;
                }
            }
        }

        state.pin.recorded_ms = at_ms;
        state.pin.sharing = true;
        state.mocked = sample.mocked;
        if let Some(battery) = sample.battery {
            state.pin.battery = Some(battery);
        }
        if let Some(charging) = sample.charging {
            state.pin.charging = charging;
        }
        if sample.foreground {
            state.pin.last_foreground_ms = at_ms;
        }
        if record {
            self.record_point(PointRow {
                user_id,
                recorded_at: from_millis(at_ms),
                lat: sample.lat,
                lng: sample.lng,
                accuracy: sample.accuracy as f32,
                altitude: sample.altitude.map_or(f32::NAN, |m| m as f32),
                speed: state.pin.speed as f32,
                heading: sample.heading.map_or(-1.0, |h| h as f32),
                activity: state.pin.activity as u8,
                battery: sample.battery.map_or(-1, |b| b as i8),
                flags: PointRow::flags(sample.mocked, sample.foreground, sample.charging.unwrap_or(false)),
            })
            .await;
        }
        Ok(())
    }

    /// Uncovers the footprint cell of a good fix (and the cells between it
    /// and the previous fix, when the two are close enough to connect).
    async fn leave_footprint(
        &self,
        user_id: Uuid,
        ctx: &OwnerCtx,
        state: &mut LiveState,
        position: Point,
        at_ms: i64,
        previous: Option<(Point, i64)>,
        effects: &mut Effects,
    ) -> AppResult<()> {
        let Some(cell) = math::cell_at(position.0, position.1, math::CELL_RESOLUTION) else { return Ok(()) };
        // The fix this one continues from: the previous good one, if it is
        // recent and itself left a footprint. After a rejected jump it did
        // not, so the next fix starts over instead of drawing a line of
        // cells from wherever the last footprint was.
        let connected = previous.filter(|(_, from_ms)| at_ms - from_ms <= BRIDGE_MS && *from_ms == state.footprint_ms);
        let last = state.recent_cells.last().copied();
        if last == Some(math::cell_to_db(cell)) {
            state.footprint_ms = at_ms;
            return Ok(());
        }
        // A position jump (GPS glitch, spoofing) leaves no footprint.
        if let Some((from, from_ms)) = previous.filter(|(_, from_ms)| at_ms - from_ms <= BRIDGE_MS) {
            let seconds = ((at_ms - from_ms) as f64 / 1000.0).max(1.0);
            if math::distance(from, position) / seconds > MAX_SPEED_MPS {
                return Ok(());
            }
        }
        state.footprint_ms = at_ms;
        state.region = self.regions_at(position.0, position.1).await?;
        let mut path = match (last.and_then(math::cell_from_db), connected) {
            (Some(from_cell), Some((from, _))) if math::distance(from, position) <= BRIDGE_M => {
                math::cells_between(from_cell, cell, BRIDGE_CELLS)
            }
            _ => vec![cell],
        };
        // Cells seen moments ago are known: no need to ask the database.
        path.retain(|cell| state.remember_cell(math::cell_to_db(*cell)));
        if path.is_empty() {
            return Ok(());
        }
        let uncovered = self.uncover(user_id, &path, from_millis(at_ms), ctx).await?;
        if !uncovered.new_cells.is_empty() {
            effects.footprints.push(uncovered);
        }
        Ok(())
    }

    /// Tells the user about newly uncovered cells and regions.
    async fn push_footprint_progress(&self, user_id: Uuid, uncovered: &Uncovered, ja: bool) {
        let mut regions = uncovered.new_regions.iter().map(|(region, _)| region.to_proto(ja));
        let first = FootprintProgress {
            new_cells: uncovered.new_cells.iter().map(|cell| math::cell_hex(*cell)).collect(),
            total_cell_count: uncovered.total,
            new_region: regions.next().into(),
            ..Default::default()
        };
        self.push(user_id, Event::FootprintProgress(Box::new(first))).await;
        // A country and its first prefecture are entered at the same time: one event each.
        for region in regions {
            let more = FootprintProgress {
                total_cell_count: uncovered.total,
                new_region: Some(region).into(),
                ..Default::default()
            };
            self.push(user_id, Event::FootprintProgress(Box::new(more))).await;
        }
    }

    /// Finds the friends within the gathering radius right now, feeds the
    /// proximity tracker, and syncs gatherings when who the user is together
    /// with changed.
    async fn check_proximity(
        &self,
        user_id: Uuid,
        ctx: &OwnerCtx,
        state: &mut LiveState,
    ) -> AppResult<Option<Changes>> {
        let at_ms = state.pin.recorded_ms;
        if at_ms - state.proximity_checked_ms < PROXIMITY_EVERY_MS {
            return Ok(None);
        }
        state.proximity_checked_ms = at_ms;
        let position = state.pin.position();
        let friends = graph::friend_ids(&self.core, user_id).await?;
        let mut near_now = Vec::new();
        if !friends.is_empty() {
            let radius = self.cfg.gathering_radius_meters;
            let hits =
                geoindex::nearby(&self.core.cache, position.0, position.1, radius + NEAR_ALLOWANCE_M, 64).await?;
            let nearby: Vec<Uuid> =
                hits.into_iter().map(|(id, _)| id).filter(|id| *id != user_id && friends.contains(id)).collect();
            // "Near" is a fact about two true positions. It may exist only
            // between two people each of whom sees the other's precise pin:
            // anything else would tell one of them where the other is.
            let candidates = self.mutually_precise(user_id, ctx, &nearby).await?;
            for (friend, theirs) in candidates.iter().zip(self.load_states(&candidates).await?) {
                let Some(theirs) = theirs.filter(|theirs| theirs.located() && theirs.pin.sharing) else { continue };
                let age = (at_ms - theirs.pin.recorded_ms).abs();
                let fresh =
                    age <= NEAR_FRESH_MS || (theirs.pin.stationary_since_ms.is_some() && age <= NEAR_FRESH_STAYING_MS);
                let allowance = (state.pin.accuracy + theirs.pin.accuracy).min(NEAR_ALLOWANCE_M);
                if fresh && math::distance(position, theirs.pin.position()) <= radius + allowance {
                    near_now.push(*friend);
                }
            }
        }
        let rules = ProximityRules { min_duration_ms: self.cfg.gathering_seconds * 1000, grace_ms: PROXIMITY_GRACE_MS };
        let mut together = state.near.update(rules, at_ms, &near_now).together;
        together.sort();
        self.add_companions(state, &together).await?;

        if together != state.gathering_with {
            let changes = self.sync_gathering(user_id, &together, position, from_millis(at_ms)).await?;
            state.gathering_with = together;
            state.gathering_seen_ms = at_ms;
            return Ok((!changes.is_empty()).then_some(changes));
        }
        if !together.is_empty() && at_ms - state.gathering_seen_ms >= GATHERING_HEARTBEAT_MS {
            self.gathering_heartbeat(user_id, position, from_millis(at_ms)).await?;
            state.gathering_seen_ms = at_ms;
        }
        Ok(None)
    }

    /// The friends among `others` who see `user_id` precisely and whom
    /// `user_id` sees precisely, right now.
    pub(crate) async fn mutually_precise(
        &self,
        user_id: Uuid,
        ctx: &OwnerCtx,
        others: &[Uuid],
    ) -> AppResult<Vec<Uuid>> {
        if others.is_empty() {
            return Ok(Vec::new());
        }
        let now = Utc::now();
        let contexts = self.ctx_many(others).await?;
        Ok(others
            .iter()
            .copied()
            .filter(|other| {
                visibility::sees_precisely(ctx, *other, now)
                    && contexts.get(other).is_some_and(|theirs| visibility::sees_precisely(theirs, user_id, now))
            })
            .collect())
    }

    /// Who sees `owner` precisely has changed (a ghost rule or an SOS began
    /// or ended). Whatever `owner` shares with someone by being near them
    /// ends at once if the two no longer see each other precisely: each
    /// drops out of the other's proximity tracker and the gatherings follow.
    /// Nothing here looks at where anyone is.
    pub(crate) async fn sight_changed(&self, owner: Uuid) -> AppResult<()> {
        // Without the lock first: most people are not near anyone.
        let Some(state) = self.load_state(owner).await? else { return Ok(()) };
        let mut linked: Vec<Uuid> = state.near.near.iter().map(|near| near.user_id).collect();
        for friend in &state.gathering_with {
            if !linked.contains(friend) {
                linked.push(*friend);
            }
        }
        if linked.is_empty() {
            return Ok(());
        }
        let ctx = self.ctx_of(owner).await?;
        let kept = self.mutually_precise(owner, &ctx, &linked).await?;
        let cut: Vec<Uuid> = linked.into_iter().filter(|id| !kept.contains(id)).collect();
        if cut.is_empty() {
            // Nobody leaves; people watching may still see fewer members now.
            return self.refresh_gathering_of(owner).await;
        }
        let mut all = vec![self.with_state(owner, async |state| self.cut_links(owner, state, &cut).await).await?];
        for other in &cut {
            all.push(self.with_state(*other, async |state| self.cut_links(*other, state, &[owner]).await).await?);
        }
        for changes in all.into_iter().flatten() {
            self.publish_gathering_changes(changes).await?;
        }
        self.refresh_gathering_of(owner).await
    }

    /// Takes `others` out of the proximity tracker of `user_id` and brings
    /// their gathering in line.
    async fn cut_links(&self, user_id: Uuid, state: &mut LiveState, others: &[Uuid]) -> AppResult<Option<Changes>> {
        for other in others {
            state.near.forget(*other);
        }
        if !state.gathering_with.iter().any(|id| others.contains(id)) {
            return Ok(None);
        }
        state.gathering_with.retain(|id| !others.contains(id));
        let changes = self.sync_gathering(user_id, &state.gathering_with, state.pin.position(), Utc::now()).await?;
        Ok((!changes.is_empty()).then_some(changes))
    }

    /// The device stopped sharing at `stopped_ms` (device time, see
    /// [`stop_time`]): the pin stays where it is and is marked paused, the
    /// stay and gathering end, and the true-position index forgets the user.
    /// Samples recorded until then are dropped from now on.
    pub(crate) async fn stop_sharing(&self, user_id: Uuid, stopped_ms: i64) -> AppResult<()> {
        let ctx = self.ctx_of(user_id).await?;
        let (pin, news) = self
            .with_state(user_id, async |state| {
                state.stopped_ms = state.stopped_ms.max(stopped_ms);
                // Never shared, or already paused: nothing else changes and nobody needs telling again.
                if !state.located() || !state.pin.sharing {
                    return Ok((None, None));
                }
                let mut news = None;
                if let Some(StayEvent::Ended { until_ms, .. }) = state.stay.stop() {
                    news = Some(self.finish_stay(user_id, &ctx, state, until_ms).await?);
                }
                if let Some(bucket) = state.odometer.take() {
                    self.store_hours(user_id, &bucket, &state.pin.area, &state.region).await?;
                }
                state.odometer.disconnect();
                state.pin.sharing = false;
                state.pin.speed = 0.0;
                // The stay ended: the real fix is no longer "staying" either.
                state.trust.left_stay();
                state.near = Default::default();
                state.gathering_with.clear();
                state.pushed = None;
                Ok((Some(state.pin.clone()), news))
            })
            .await?;
        geoindex::remove(&self.core.cache, user_id).await?;
        self.leave_gathering(user_id).await?;
        self.flush_history().await?;
        if let Some(news) = news {
            self.push_visit_news(user_id, &ctx, &[news]).await?;
        }
        if let Some(pin) = pin {
            self.fan_out(user_id, Some(&pin), &ctx, Recipients::Friends, false).await?;
            self.publish_shares(user_id, &pin, &ctx).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tokuly_proto::common::LatLng;

    use super::*;
    use crate::state::{Pin, StayPlace};

    fn reading(lat: f64, lng: f64) -> LocationSample {
        LocationSample {
            position: LatLng { lat, lng, ..Default::default() }.into(),
            accuracy_meters: 12.0,
            ..Default::default()
        }
    }

    fn week() -> Duration {
        Duration::days(7)
    }

    #[test]
    fn a_plain_reading_is_accepted_and_stamped_with_the_time_of_receipt() {
        let now = Utc::now();
        let sample = validate(&reading(35.6595, 139.7005), now, week()).unwrap();
        assert_eq!((sample.lat, sample.lng, sample.accuracy), (35.6595, 139.7005, 12.0));
        assert_eq!(sample.recorded_ms, now.timestamp_millis());
        assert_eq!((sample.speed, sample.heading, sample.battery, sample.charging), (None, None, None, None));
        assert_eq!(sample.activity, Activity::ACTIVITY_UNSPECIFIED);
    }

    #[test]
    fn unusable_positions_and_accuracies_are_rejected() {
        let now = Utc::now();
        assert_eq!(validate(&LocationSample::default(), now, week()), Err(Rejection::NoPosition));
        assert_eq!(validate(&reading(91.0, 0.0), now, week()), Err(Rejection::NoPosition));
        assert_eq!(validate(&reading(f64::NAN, 0.0), now, week()), Err(Rejection::NoPosition));
        for accuracy in [-1.0, f64::NAN, f64::INFINITY, 20_000.0] {
            let bad = LocationSample { accuracy_meters: accuracy, ..reading(35.0, 139.0) };
            assert_eq!(validate(&bad, now, week()), Err(Rejection::BadAccuracy), "{accuracy}");
        }
    }

    #[test]
    fn a_stop_is_timed_by_the_device_clock_and_capped_like_samples() {
        let now = Utc::now();
        let stamp = |time: DateTime<Utc>| Timestamp {
            seconds: time.timestamp(),
            nanos: time.timestamp_subsec_nanos() as i32,
            ..Default::default()
        };
        let behind = now - Duration::minutes(5);
        assert_eq!(stop_time(Some(&stamp(behind)), now), behind.timestamp_millis(), "a slow clock is kept");
        assert_eq!(stop_time(None, now), now.timestamp_millis(), "nothing said: the server's time");
        let cap = (now + MAX_CLOCK_SKEW).timestamp_millis();
        assert_eq!(stop_time(Some(&stamp(now + Duration::days(400))), now), cap);
        let absurd = Timestamp { seconds: i64::MAX, ..Default::default() };
        assert_eq!(stop_time(Some(&absurd), now), cap);
    }

    #[test]
    fn timestamps_must_be_recent_and_not_from_the_future() {
        let now = Utc::now();
        let at = |time: DateTime<Utc>| LocationSample { recorded_at: ts(time), ..reading(35.0, 139.0) };
        assert_eq!(
            validate(&at(now - Duration::hours(3)), now, week()).unwrap().recorded_ms,
            (now - Duration::hours(3)).timestamp_millis()
        );
        assert!(validate(&at(now + Duration::seconds(90)), now, week()).is_ok(), "a little clock skew is fine");
        assert_eq!(validate(&at(now + Duration::minutes(5)), now, week()), Err(Rejection::FromTheFuture));
        assert_eq!(validate(&at(now - Duration::days(8)), now, week()), Err(Rejection::TooOld));
        assert!(validate(&at(now - Duration::days(6)), now, week()).is_ok());
        // Nonsense seconds do not wrap around into something acceptable.
        let absurd = LocationSample {
            recorded_at: Some(tokuly_proto::Timestamp { seconds: i64::MAX, ..Default::default() }).into(),
            ..reading(35.0, 139.0)
        };
        assert_eq!(validate(&absurd, now, week()), Err(Rejection::FromTheFuture));
    }

    #[test]
    fn unusable_optional_values_are_dropped_not_fatal() {
        let now = Utc::now();
        let messy = LocationSample {
            speed_mps: Some(-1.0),
            heading_degrees: Some(f64::NAN),
            altitude_meters: Some(1e9),
            battery_percent: Some(150),
            charging: Some(true),
            activity: buffa::EnumValue::from(42),
            ..reading(35.0, 139.0)
        };
        let sample = validate(&messy, now, week()).unwrap();
        assert_eq!((sample.speed, sample.heading, sample.altitude, sample.battery), (None, None, None, None));
        assert_eq!((sample.charging, sample.activity), (Some(true), Activity::ACTIVITY_UNSPECIFIED));
        let tidy = LocationSample {
            speed_mps: Some(1.5),
            heading_degrees: Some(360.0),
            battery_percent: Some(0),
            ..reading(35.0, 139.0)
        };
        let sample = validate(&tidy, now, week()).unwrap();
        assert_eq!((sample.speed, sample.heading, sample.battery), (Some(1.5), Some(0.0), Some(0)));
    }

    fn state_at(position: Point, recorded_ms: i64) -> LiveState {
        LiveState {
            pin: Pin {
                lat: position.0,
                lng: position.1,
                recorded_ms,
                sharing: true,
                battery: Some(80),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn pushed(state: &mut LiveState) {
        state.pushed = Some(Pushed {
            at_ms: state.pin.recorded_ms,
            lat: state.pin.lat,
            lng: state.pin.lng,
            fingerprint: fingerprint(state),
        });
    }

    #[test]
    fn the_first_pin_and_real_changes_are_pushed() {
        let home = (35.6595, 139.7005);
        let mut state = state_at(home, 1_000_000);
        assert!(worth_pushing(&state), "never pushed");
        pushed(&mut state);
        assert!(!worth_pushing(&state));

        // A still device reporting again a few seconds later: nothing to show.
        state.pin.recorded_ms += 10_000;
        assert!(!worth_pushing(&state));
        // ...but friends hear from it at least every half minute.
        state.pin.recorded_ms += 20_000;
        assert!(worth_pushing(&state));
        pushed(&mut state);

        // Movement.
        let moved = math::destination(home.0, home.1, 0.0, 8.0);
        (state.pin.lat, state.pin.lng) = moved;
        assert!(worth_pushing(&state));
        pushed(&mut state);
        let wobble = math::destination(moved.0, moved.1, 90.0, 2.0);
        (state.pin.lat, state.pin.lng) = wobble;
        assert!(!worth_pushing(&state), "two meters is GPS wobble");
    }

    #[test]
    fn anything_a_friend_could_notice_triggers_a_push() {
        let mut state = state_at((35.0, 139.0), 1_000_000);
        pushed(&mut state);
        let changes: [fn(&mut LiveState); 6] = [
            |s| s.pin.battery = Some(79),
            |s| s.pin.charging = true,
            |s| s.pin.activity = 2,
            |s| s.pin.stationary_since_ms = Some(5),
            |s| {
                s.pin.place = Some(StayPlace {
                    id: Uuid::now_v7(),
                    owner_id: Uuid::now_v7(),
                    label: 4,
                    name: "x".into(),
                    emoji: String::new(),
                    visibility: 1,
                })
            },
            |s| s.pin.sharing = false,
        ];
        for change in changes {
            let mut changed = state.clone();
            change(&mut changed);
            assert!(worth_pushing(&changed));
        }
    }
}
