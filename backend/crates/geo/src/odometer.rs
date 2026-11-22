//! Distance and time travelled, accumulated per hour and activity.
//!
//! Every accepted sample passes through a user's [`Odometer`], which lives in
//! their live state. It counts distance only when the user has moved beyond
//! GPS noise, and hands out finished hour buckets ([`HourBucket`]) to be
//! added to `geo_activity_hours`. Pure: no clock, no I/O.
use serde::{Deserialize, Serialize};

use crate::math::{self, Point};

/// Movement below this many meters is treated as GPS wobble.
const MIN_STEP_M: f64 = 15.0;
/// The share of the reported accuracy that also counts as wobble, up to this many meters.
const MAX_NOISE_M: f64 = 50.0;
/// Samples further apart than this are not connected: where the user went
/// in between is unknown.
const MAX_GAP_MS: i64 = 5 * 60 * 1000;
/// Faster than any airliner: a position jump, not movement.
pub const MAX_SPEED_MPS: f64 = 350.0;

const HOUR_MS: i64 = 3_600_000;

/// Totals for one activity within the pending hour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivityTotals {
    pub activity: i32,
    pub distance_m: f64,
    pub duration_s: f64,
    pub max_speed_mps: f64,
    pub points: u32,
}

/// What was accumulated for one hour, ready to be stored.
#[derive(Debug, Clone, PartialEq)]
pub struct HourBucket {
    /// Start of the hour, Unix milliseconds.
    pub hour_ms: i64,
    pub totals: Vec<ActivityTotals>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Odometer {
    /// The point distance was last counted up to.
    pub from: Option<Point>,
    /// Time of the previous sample.
    pub last_ms: i64,
    /// The hour the pending totals belong to.
    pub hour_ms: i64,
    pub pending: Vec<ActivityTotals>,
    /// Sample time at which pending totals were last handed out.
    pub flushed_ms: i64,
}

impl Odometer {
    fn totals(&mut self, activity: i32) -> &mut ActivityTotals {
        let index = match self.pending.iter().position(|totals| totals.activity == activity) {
            Some(index) => index,
            None => {
                self.pending.push(ActivityTotals {
                    activity,
                    distance_m: 0.0,
                    duration_s: 0.0,
                    max_speed_mps: 0.0,
                    points: 0,
                });
                self.pending.len() - 1
            }
        };
        &mut self.pending[index]
    }

    /// Counts one sample. Returns the previous hour's bucket when the sample
    /// starts a new hour.
    pub fn add(
        &mut self,
        point: Point,
        at_ms: i64,
        accuracy_m: f64,
        activity: i32,
        speed_mps: f64,
    ) -> Option<HourBucket> {
        let hour_ms = at_ms.div_euclid(HOUR_MS) * HOUR_MS;
        let finished = if hour_ms != self.hour_ms { self.take() } else { None };
        self.hour_ms = hour_ms;
        if self.flushed_ms == 0 {
            self.flushed_ms = at_ms;
        }

        let elapsed_ms = at_ms - self.last_ms;
        let connected = self.from.is_some() && (0..=MAX_GAP_MS).contains(&elapsed_ms);
        match self.from {
            Some(from) if connected => {
                let step = math::distance(from, point);
                let noise = MIN_STEP_M.max(accuracy_m.clamp(0.0, MAX_NOISE_M));
                let plausible = elapsed_ms == 0 || step / (elapsed_ms as f64 / 1000.0).max(1.0) <= MAX_SPEED_MPS;
                if step >= noise && plausible {
                    self.totals(activity).distance_m += step;
                    self.from = Some(point);
                } else if !plausible {
                    self.from = Some(point);
                }
                self.totals(activity).duration_s += elapsed_ms as f64 / 1000.0;
            }
            // First sample, or a gap too long to bridge: start over from here.
            _ => self.from = Some(point),
        }
        let totals = self.totals(activity);
        totals.points += 1;
        if speed_mps.is_finite() && speed_mps <= MAX_SPEED_MPS {
            totals.max_speed_mps = totals.max_speed_mps.max(speed_mps);
        }
        self.last_ms = at_ms;
        finished
    }

    /// Hands out the pending totals (if any) and starts afresh for the same hour.
    pub fn take(&mut self) -> Option<HourBucket> {
        self.flushed_ms = self.last_ms;
        if self.pending.is_empty() {
            return None;
        }
        Some(HourBucket { hour_ms: self.hour_ms, totals: std::mem::take(&mut self.pending) })
    }

    /// Whether pending totals have waited long enough (in sample time) to be stored.
    pub fn due(&self, at_ms: i64, every_ms: i64) -> bool {
        !self.pending.is_empty() && at_ms - self.flushed_ms >= every_ms
    }

    /// Forgets the connection to earlier samples (sharing stopped, history deleted).
    pub fn disconnect(&mut self) {
        self.from = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: Point = (35.6595, 139.7005);
    const WALKING: i32 = 2;
    const STILL: i32 = 1;
    /// An instant on the hour (late 2026).
    const T0: i64 = 1_795_230_000_000;

    fn east(meters: f64) -> Point {
        math::destination(START.0, START.1, 90.0, meters)
    }

    #[test]
    fn t0_is_on_the_hour() {
        assert_eq!(T0 % HOUR_MS, 0);
    }

    #[test]
    fn walking_adds_up_distance_and_time() {
        let mut odometer = Odometer::default();
        // 1.4 m/s for ten minutes, a sample every 10 s.
        for i in 0..=60 {
            assert_eq!(odometer.add(east(i as f64 * 14.0), T0 + i * 10_000, 8.0, WALKING, 1.4), None);
        }
        let bucket = odometer.take().unwrap();
        assert_eq!(bucket.hour_ms, T0);
        assert_eq!(bucket.totals.len(), 1);
        let walked = &bucket.totals[0];
        assert_eq!(walked.activity, WALKING);
        // 14 m steps are below the 15 m threshold, so every second sample counts 28 m.
        assert!((walked.distance_m - 840.0).abs() < 2.0, "{}", walked.distance_m);
        assert!((walked.duration_s - 600.0).abs() < 0.001);
        assert_eq!(walked.points, 61);
        assert!((walked.max_speed_mps - 1.4).abs() < 1e-9);
        assert_eq!(odometer.take(), None, "nothing pending after a flush");
    }

    #[test]
    fn gps_wobble_while_still_counts_no_distance() {
        let mut odometer = Odometer::default();
        for i in 0..120 {
            let wobble = math::destination(START.0, START.1, (i * 47 % 360) as f64, (i % 5) as f64 * 2.5);
            odometer.add(wobble, T0 + i * 5_000, 12.0, STILL, 0.0);
        }
        let bucket = odometer.take().unwrap();
        assert_eq!(bucket.totals[0].distance_m, 0.0);
        assert!((bucket.totals[0].duration_s - 595.0).abs() < 0.001);
    }

    #[test]
    fn poor_accuracy_raises_the_noise_threshold() {
        let mut odometer = Odometer::default();
        odometer.add(START, T0, 40.0, WALKING, 1.0);
        odometer.add(east(30.0), T0 + 20_000, 40.0, WALKING, 1.0);
        assert_eq!(odometer.pending[0].distance_m, 0.0, "30 m is within 40 m accuracy");
        odometer.add(east(60.0), T0 + 40_000, 40.0, WALKING, 1.0);
        assert!((odometer.pending[0].distance_m - 60.0).abs() < 0.5);
    }

    #[test]
    fn long_gaps_and_jumps_are_not_bridged() {
        let mut odometer = Odometer::default();
        odometer.add(START, T0, 8.0, WALKING, 1.0);
        // Twenty minutes of silence, then 5 km away: unknown path.
        odometer.add(east(5000.0), T0 + 20 * 60_000, 8.0, WALKING, 1.0);
        assert_eq!(odometer.pending[0].distance_m, 0.0);
        assert_eq!(odometer.pending[0].duration_s, 0.0);
        // Then a normal step from the new position counts.
        odometer.add(east(5050.0), T0 + 20 * 60_000 + 30_000, 8.0, WALKING, 1.0);
        assert!((odometer.pending[0].distance_m - 50.0).abs() < 0.5);
        // A 100 km jump within ten seconds is a glitch: not counted, but it becomes the new origin.
        odometer.add(east(105_000.0), T0 + 20 * 60_000 + 40_000, 8.0, WALKING, 1.0);
        assert!((odometer.pending[0].distance_m - 50.0).abs() < 0.5);
        assert_eq!(odometer.from, Some(east(105_000.0)));
    }

    #[test]
    fn activities_are_kept_apart() {
        let mut odometer = Odometer::default();
        odometer.add(START, T0, 8.0, WALKING, 1.4);
        odometer.add(east(100.0), T0 + 60_000, 8.0, WALKING, 1.6);
        odometer.add(east(1100.0), T0 + 120_000, 8.0, 5, 16.0);
        let bucket = odometer.take().unwrap();
        let by_activity = |activity| bucket.totals.iter().find(|t| t.activity == activity).unwrap();
        assert!((by_activity(WALKING).distance_m - 100.0).abs() < 0.5);
        assert!((by_activity(5).distance_m - 1000.0).abs() < 2.0);
        assert_eq!((by_activity(WALKING).duration_s, by_activity(5).duration_s), (60.0, 60.0));
        assert_eq!(by_activity(5).max_speed_mps, 16.0);
    }

    #[test]
    fn a_new_hour_hands_out_the_previous_one() {
        let mut odometer = Odometer::default();
        odometer.add(START, T0 + 59 * 60_000, 8.0, WALKING, 1.4);
        odometer.add(east(60.0), T0 + 59 * 60_000 + 40_000, 8.0, WALKING, 1.4);
        let finished = odometer.add(east(120.0), T0 + HOUR_MS + 20_000, 8.0, WALKING, 1.4).unwrap();
        assert_eq!(finished.hour_ms, T0);
        assert!((finished.totals[0].distance_m - 60.0).abs() < 0.5);
        // The step across the hour boundary belongs to the new hour.
        let current = odometer.take().unwrap();
        assert_eq!(current.hour_ms, T0 + HOUR_MS);
        assert!((current.totals[0].distance_m - 60.0).abs() < 0.5);
    }

    #[test]
    fn pending_totals_become_due_in_sample_time() {
        let mut odometer = Odometer::default();
        assert!(!odometer.due(T0, 60_000), "nothing pending");
        odometer.add(START, T0, 8.0, WALKING, 1.0);
        assert!(!odometer.due(T0 + 30_000, 60_000));
        assert!(odometer.due(T0 + 60_000, 60_000));
        odometer.add(east(50.0), T0 + 70_000, 8.0, WALKING, 1.0);
        odometer.take();
        assert!(!odometer.due(T0 + 80_000, 60_000));
    }

    #[test]
    fn disconnecting_starts_distance_over() {
        let mut odometer = Odometer::default();
        odometer.add(START, T0, 8.0, WALKING, 1.0);
        odometer.disconnect();
        odometer.add(east(200.0), T0 + 30_000, 8.0, WALKING, 1.0);
        assert_eq!(odometer.pending[0].distance_m, 0.0);
    }
}
