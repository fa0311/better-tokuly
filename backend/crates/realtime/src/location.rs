//! Sanity checks on location samples before they enter the geo pipeline.
//!
//! The gateway only rejects what is plainly unusable (impossible
//! coordinates, non-numbers, absurd timestamps); judging plausibility — mock
//! locations, jumps, accuracy — is the geo domain's business.
use chrono::{DateTime, Duration, Utc};
use tokuly_core::{geoindex::valid_coordinates, time::ts};
use tokuly_proto::geo::LocationSample;

use crate::frames::{Code, Rejection};

const INVALID: Rejection = Rejection::new(Code::InvalidArgument, "LOCATION_INVALID");

/// Device clocks run ahead a little; more than this is a broken clock.
const MAX_CLOCK_SKEW: Duration = Duration::minutes(2);
/// Live samples are recent. Older backlogs go through `ReportLocations`,
/// and the bus keeps samples for a day anyway.
const MAX_AGE: Duration = Duration::hours(24);

/// Checks a sample received at `received_at` and fills in a missing
/// `recorded_at` with the time of receipt.
pub(crate) fn validate(mut sample: LocationSample, received_at: DateTime<Utc>) -> Result<LocationSample, Rejection> {
    let position = sample.position.as_option().ok_or(INVALID)?;
    if !valid_coordinates(position.lat, position.lng) {
        return Err(INVALID);
    }
    if !sample.accuracy_meters.is_finite() || sample.accuracy_meters < 0.0 {
        return Err(INVALID);
    }
    let optional_numbers = [sample.altitude_meters, sample.speed_mps, sample.heading_degrees];
    if optional_numbers.into_iter().flatten().any(|value| !value.is_finite()) {
        return Err(INVALID);
    }
    match sample.recorded_at.as_option() {
        Some(recorded_at) => {
            // Checked on the raw seconds: converting first would clamp an
            // out-of-range value and make a nonsense timestamp look fine.
            let earliest = (received_at - MAX_AGE).timestamp();
            let latest = (received_at + MAX_CLOCK_SKEW).timestamp();
            if !(earliest..=latest).contains(&recorded_at.seconds) {
                return Err(INVALID);
            }
        }
        None => sample.recorded_at = ts(received_at),
    }
    Ok(sample)
}

#[cfg(test)]
mod tests {
    use tokuly_core::time::from_ts;
    use tokuly_proto::common::LatLng;

    use super::*;

    fn sample(lat: f64, lng: f64) -> LocationSample {
        LocationSample { position: LatLng { lat, lng, ..Default::default() }.into(), accuracy_meters: 12.0, ..Default::default() }
    }

    fn rejected(sample: LocationSample) -> bool {
        validate(sample, Utc::now()) == Err(INVALID)
    }

    #[test]
    fn a_plain_sample_passes_and_gets_a_timestamp() {
        let now = Utc::now();
        let checked = validate(sample(35.6595, 139.7005), now).unwrap();
        assert_eq!(from_ts(checked.recorded_at.as_option().unwrap()).timestamp_millis(), now.timestamp_millis());
    }

    #[test]
    fn a_recent_device_timestamp_is_kept() {
        let now = Utc::now();
        let mut recent = sample(35.0, 139.0);
        recent.recorded_at = ts(now - Duration::seconds(30));
        let checked = validate(recent.clone(), now).unwrap();
        assert_eq!(checked.recorded_at, recent.recorded_at);

        let mut slightly_ahead = sample(35.0, 139.0);
        slightly_ahead.recorded_at = ts(now + Duration::seconds(60));
        assert!(validate(slightly_ahead, now).is_ok());
    }

    #[test]
    fn unusable_samples_are_rejected() {
        assert!(rejected(LocationSample::default()), "no position");
        assert!(rejected(sample(91.0, 0.0)));
        assert!(rejected(sample(0.0, 181.0)));
        assert!(rejected(sample(f64::NAN, 0.0)));
        assert!(rejected(sample(0.0, f64::INFINITY)));

        let mut bad_accuracy = sample(35.0, 139.0);
        bad_accuracy.accuracy_meters = f64::NAN;
        assert!(rejected(bad_accuracy));
        let mut negative_accuracy = sample(35.0, 139.0);
        negative_accuracy.accuracy_meters = -1.0;
        assert!(rejected(negative_accuracy));

        let mut bad_speed = sample(35.0, 139.0);
        bad_speed.speed_mps = Some(f64::INFINITY);
        assert!(rejected(bad_speed));
        let mut bad_heading = sample(35.0, 139.0);
        bad_heading.heading_degrees = Some(f64::NAN);
        assert!(rejected(bad_heading));
    }

    #[test]
    fn absurd_timestamps_are_rejected() {
        let now = Utc::now();
        let mut future = sample(35.0, 139.0);
        future.recorded_at = ts(now + Duration::minutes(10));
        assert!(rejected(future));

        let mut stale = sample(35.0, 139.0);
        stale.recorded_at = ts(now - Duration::hours(25));
        assert!(rejected(stale));

        let mut overflowing = sample(35.0, 139.0);
        overflowing.recorded_at = tokuly_proto::Timestamp { seconds: i64::MAX, nanos: 0, ..Default::default() }.into();
        assert!(rejected(overflowing));
    }
}
