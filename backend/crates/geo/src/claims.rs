//! Positions other domains are told an account is at: are they plausible?
//!
//! The social domain matches nearby discovery and bumps on a position the
//! client sends with the request. This module answers, for such a claim,
//! whether the account's own recent location fixes put it there — and
//! nothing else: [`tokuly_social::claims::PositionCheck`] returns a yes or a
//! no, never a position or a distance. Social so never holds anything
//! derived from a true position that it could pass on to someone who does
//! not see that person precisely (the rule of [`crate::visibility`]); what
//! the people it shows the claimer to get is computed from the claim.
//!
//! What counts as a real fix ([`TrustedFix`], kept in the live state): a
//! sample the pipeline took as the position, accurate to 100 m or better,
//! not from a mock location, received while it was live (not a backlog
//! upload), and reachable from the previous real fix at a speed something
//! can travel at. A mock sample forgets the real fix: until the device
//! reports a real one again, nothing it claims is plausible.
//!
//! A claim is plausible when the latest real fix is recent (10 minutes, 45
//! while the account is staying in one spot — still phones report rarely)
//! and the claim lies within reach of it: the two accuracies, a margin, and
//! walking speed for the time since the fix, at most [`MAX_CLAIM_M`].
//!
//! Known limit: the samples themselves come from the device. This stops
//! claims that jump to wherever the client likes; a device that fakes a
//! continuous track (and does not flag it as mocked) moves its real fix
//! along with it, no faster than [`MAX_SPEED_MPS`].
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokuly_core::{geoindex, prelude::*};
use tokuly_social::claims::{Claim, PositionCheck};

use crate::{
    Geo,
    math::{self, Point},
    odometer::MAX_SPEED_MPS,
};

/// Fixes less accurate than this are not real enough to vouch for a claim.
const TRUSTED_ACCURACY_M: f64 = 100.0;
/// A fix vouches for claims for this long after it was taken...
const FRESH_MS: i64 = 10 * 60 * 1000;
/// ...or this long while the account is staying in one spot.
const FRESH_STAYING_MS: i64 = 45 * 60 * 1000;
/// A fix received later than this after it was taken is history: it does
/// not become the real position.
const LIVE_MS: i64 = 5 * 60 * 1000;
/// Distance always allowed between the fix and the claim, on top of their
/// accuracies (GPS wanders, phones of one person differ).
const MARGIN_M: f64 = 50.0;
/// The claim's own accuracy counts up to this much.
const MAX_CLAIM_ACCURACY_M: f64 = 50.0;
/// How fast someone with the app open is assumed to have moved since the fix.
const WALKING_MPS: f64 = 1.5;
/// However old the fix and poor the accuracies, a claim farther than this
/// from the real fix is not plausible.
pub(crate) const MAX_CLAIM_M: f64 = 300.0;

/// The latest fix that is the account's real position (see the module docs).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct TrustedFix {
    pub lat: f64,
    pub lng: f64,
    pub accuracy: f64,
    /// Device time of the sample, Unix milliseconds.
    pub recorded_ms: i64,
    /// The account was in a confirmed stay at that fix.
    pub staying: bool,
}

impl TrustedFix {
    fn position(&self) -> Point {
        (self.lat, self.lng)
    }
}

/// A sample the pipeline took as the position, as far as trust is concerned.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Taken {
    pub lat: f64,
    pub lng: f64,
    pub accuracy: f64,
    pub recorded_ms: i64,
    pub mocked: bool,
    pub staying: bool,
}

/// The real fix after `taken`, received at `received_ms`, given the one before.
pub(crate) fn next_trusted(previous: Option<TrustedFix>, taken: &Taken, received_ms: i64) -> Option<TrustedFix> {
    if taken.mocked {
        return None;
    }
    let fix = TrustedFix {
        lat: taken.lat,
        lng: taken.lng,
        accuracy: taken.accuracy,
        recorded_ms: taken.recorded_ms,
        staying: taken.staying,
    };
    let real = taken.accuracy <= TRUSTED_ACCURACY_M
        && received_ms - taken.recorded_ms <= LIVE_MS
        && previous.is_none_or(|before| reachable(&before, &fix));
    if real { Some(fix) } else { previous }
}

/// Whether `to` can be reached from `from` in the time between them.
fn reachable(from: &TrustedFix, to: &TrustedFix) -> bool {
    let seconds = ((to.recorded_ms - from.recorded_ms) as f64 / 1000.0).max(1.0);
    // Two fixes a second apart can wobble by their accuracies without moving.
    let apart = (math::distance(from.position(), to.position()) - from.accuracy - to.accuracy).max(0.0);
    apart / seconds <= MAX_SPEED_MPS
}

/// Whether the account can plausibly be at `claim` at `now_ms`, given its
/// latest real fix.
pub(crate) fn plausible(fix: Option<&TrustedFix>, claim: &Claim, now_ms: i64) -> bool {
    let Some(fix) = fix else { return false };
    if !geoindex::valid_coordinates(claim.lat, claim.lng) {
        return false;
    }
    let age_ms = (now_ms - fix.recorded_ms).max(0);
    if age_ms > if fix.staying { FRESH_STAYING_MS } else { FRESH_MS } {
        return false;
    }
    let claim_accuracy =
        if claim.accuracy_meters.is_finite() { claim.accuracy_meters.clamp(0.0, MAX_CLAIM_ACCURACY_M) } else { 0.0 };
    let walked = WALKING_MPS * age_ms as f64 / 1000.0;
    let reach = (MARGIN_M + fix.accuracy + claim_accuracy + walked).min(MAX_CLAIM_M);
    math::distance(fix.position(), (claim.lat, claim.lng)) <= reach
}

#[async_trait::async_trait]
impl PositionCheck for Geo {
    async fn plausible(&self, user_id: Uuid, claim: Claim) -> AppResult<bool> {
        let trusted = self.load_state(user_id).await?.and_then(|state| state.trusted);
        Ok(plausible(trusted.as_ref(), &claim, Utc::now().timestamp_millis()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HERE: Point = (35.6595, 139.7005);
    const NOW: i64 = 1_800_000_000_000;

    fn north(meters: f64) -> Point {
        (HERE.0 + meters / 111_195.0, HERE.1)
    }

    fn fix_at(at: Point, recorded_ms: i64) -> TrustedFix {
        TrustedFix { lat: at.0, lng: at.1, accuracy: 10.0, recorded_ms, staying: false }
    }

    fn taken(at: Point, recorded_ms: i64) -> Taken {
        Taken { lat: at.0, lng: at.1, accuracy: 10.0, recorded_ms, mocked: false, staying: false }
    }

    fn claim(at: Point) -> Claim {
        Claim { lat: at.0, lng: at.1, accuracy_meters: 5.0 }
    }

    #[test]
    fn a_claim_near_a_fresh_fix_is_plausible() {
        let fix = fix_at(HERE, NOW - 5_000);
        assert!(plausible(Some(&fix), &claim(HERE), NOW));
        assert!(plausible(Some(&fix), &claim(north(60.0)), NOW));
        assert!(!plausible(Some(&fix), &claim(north(120.0)), NOW), "50 m margin + 10 m + 5 m + 7.5 m walked");
    }

    #[test]
    fn no_fix_or_a_stale_one_vouches_for_nothing() {
        assert!(!plausible(None, &claim(HERE), NOW));
        let old = fix_at(HERE, NOW - FRESH_MS - 1);
        assert!(!plausible(Some(&old), &claim(HERE), NOW));
        let staying = TrustedFix { staying: true, ..old };
        assert!(plausible(Some(&staying), &claim(HERE), NOW), "still phones report rarely");
        let staying_long = TrustedFix { recorded_ms: NOW - FRESH_STAYING_MS - 1, ..staying };
        assert!(!plausible(Some(&staying_long), &claim(HERE), NOW));
    }

    #[test]
    fn time_since_the_fix_widens_the_reach_up_to_a_cap() {
        let fix = fix_at(HERE, NOW - 9 * 60 * 1000);
        assert!(plausible(Some(&fix), &claim(north(290.0)), NOW));
        assert!(!plausible(Some(&fix), &claim(north(310.0)), NOW), "never more than {MAX_CLAIM_M} m");
        let vague = Claim { accuracy_meters: f64::INFINITY, ..claim(north(80.0)) };
        assert!(
            !plausible(Some(&fix_at(HERE, NOW)), &vague, NOW),
            "a claim cannot widen its reach with a bad accuracy"
        );
        let invalid = Claim { lat: 91.0, ..claim(HERE) };
        assert!(!plausible(Some(&fix_at(HERE, NOW)), &invalid, NOW));
    }

    #[test]
    fn only_accurate_live_real_reachable_fixes_become_the_real_position() {
        let first = next_trusted(None, &taken(HERE, NOW), NOW);
        assert_eq!(first, Some(fix_at(HERE, NOW)));

        let coarse = Taken { accuracy: 500.0, ..taken(north(400.0), NOW + 60_000) };
        assert_eq!(next_trusted(first, &coarse, NOW + 60_000), first, "a coarse fix keeps the previous one");

        let backlog = taken(north(10.0), NOW + 1_000);
        assert_eq!(next_trusted(first, &backlog, NOW + LIVE_MS + 2_000), first, "a late upload is history");

        let jump = taken((HERE.0 + 1.0, HERE.1), NOW + 10_000);
        assert_eq!(next_trusted(first, &jump, NOW + 10_000), first, "111 km in 10 s is not travel");
        let flight = taken((HERE.0 + 1.0, HERE.1), NOW + 3_600_000);
        assert_eq!(
            next_trusted(first, &flight, NOW + 3_600_000),
            Some(fix_at((HERE.0 + 1.0, HERE.1), NOW + 3_600_000))
        );

        let mocked = Taken { mocked: true, ..taken(HERE, NOW + 5_000) };
        assert_eq!(next_trusted(first, &mocked, NOW + 5_000), None, "a mock location forgets the real fix");
    }
}
