//! Positions other domains are told an account is at: are they plausible?
//!
//! The social domain matches nearby discovery and bumps on a position the
//! client sends with the request. This module answers, for such a claim,
//! whether the account's own recent location fixes put it there — and
//! nothing else: [`tokuly_social::claims::PositionCheck`] returns a verdict
//! about the caller's own claim, never a position or a distance. Social so
//! never holds anything derived from a true position that it could pass on
//! to someone who does not see that person precisely (the rule of
//! [`crate::visibility`]); what the people it shows the claimer to get is
//! computed from the claim.
//!
//! # The real position ([`Trust`], kept in the live state)
//!
//! Only the newest sample of each upload counts (the realtime gateway sends
//! one per message): older samples of a batch are history. All times here
//! are the server's: when a sample arrived, never the device's clock, which
//! may be minutes off.
//!
//! A sample becomes the real fix when it is accurate to 100 m or better,
//! not mocked, and follows from the previous real fix (the *anchor*):
//!
//! - at most [`GROUND_MPS`] (a fast train) from it, when it arrived within
//!   [`SHORT_GAP_MS`] of it; or
//! - within [`STILL_M`] of it after a longer silence (the phone was put
//!   down, the person walked a little).
//!
//! Anything else is a jump. A jump becomes the real position only through a
//! *run*: at least [`RUN_FIXES`] fixes that arrived over at least
//! [`RUN_SPAN_MS`], each within [`RUN_GAP_MS`] of the one before and
//! reachable from it at ground speed. One sample far away — after a minute
//! of silence or after a day — changes nothing on its own.
//!
//! A mock sample takes away the real fix (nothing is plausible until a real
//! one comes) but not the anchor: the next real fix must still follow from
//! where the account really was. An account's very first fix (no anchor
//! yet, or none for the 60 days the live state is kept) is taken as it is.
//!
//! # Claims
//!
//! A claim is plausible when the real fix arrived at most 10 minutes ago (45
//! while the account is staying in one spot — still phones report rarely)
//! and the claim lies within reach of it: the two accuracies, a margin, and
//! walking speed for the time since the fix, at most [`MAX_CLAIM_M`].
//! Otherwise the verdict is "too coarse" when the latest sample was refused
//! only for its accuracy (approximate location), and "not verified" else.
//!
//! What this guarantees: the real fix moves at ground speed, or jumps only
//! after two minutes of consistent reports at the new place. The samples
//! are still the device's word: a device that fakes a ground-speed track,
//! or reports a fake place steadily for two minutes, moves its real fix
//! there.
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokuly_core::{geoindex, prelude::*};
use tokuly_social::claims::{Claim, PositionCheck, Verdict};

use crate::{
    Geo,
    math::{self, Point},
};

/// Fixes less accurate than this are not real enough to vouch for a claim.
pub(crate) const TRUSTED_ACCURACY_M: f64 = 100.0;
/// The fastest the real fix moves between two fixes close in time: a fast
/// train (planes are covered by runs).
const GROUND_MPS: f64 = 90.0;
/// Fixes arriving this close to the anchor may move from it at ground speed...
const SHORT_GAP_MS: i64 = 10 * 60 * 1000;
/// ...after a longer silence, only this far.
const STILL_M: f64 = 1_000.0;
/// A jump becomes the real position after this many consistent fixes...
const RUN_FIXES: u32 = 3;
/// ...that arrived over at least this long...
const RUN_SPAN_MS: i64 = 2 * 60 * 1000;
/// ...each at most this long after the one before.
const RUN_GAP_MS: i64 = 5 * 60 * 1000;
/// A fix vouches for claims for this long after it arrived...
const FRESH_MS: i64 = 10 * 60 * 1000;
/// ...or this long while the account is staying in one spot.
const FRESH_STAYING_MS: i64 = 45 * 60 * 1000;
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

/// One fix as far as trust is concerned.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct TrustedFix {
    pub lat: f64,
    pub lng: f64,
    pub accuracy: f64,
    /// When the sample arrived, server time, Unix milliseconds.
    pub arrived_ms: i64,
    /// The account was in a confirmed stay at that fix.
    pub staying: bool,
}

impl TrustedFix {
    fn position(&self) -> Point {
        (self.lat, self.lng)
    }
}

/// Consistent fixes away from the anchor, on their way to becoming the real position.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub first_ms: i64,
    pub last: TrustedFix,
    pub count: u32,
}

/// What the live state knows about the account's real position (see the module docs).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Trust {
    /// The real fix claims are checked against; `None` after a mock sample.
    pub fix: Option<TrustedFix>,
    /// The latest real fix, also after a mock: where the next one must follow from.
    pub anchor: Option<TrustedFix>,
    pub run: Option<Run>,
    /// The latest sample was refused only for its accuracy.
    pub coarse: bool,
}

/// A sample the pipeline accepted, as far as trust is concerned.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Taken {
    pub lat: f64,
    pub lng: f64,
    pub accuracy: f64,
    pub mocked: bool,
    pub staying: bool,
    /// The newest sample of its upload: the only one that is the present.
    pub newest: bool,
    /// Server time the upload arrived.
    pub arrived_ms: i64,
}

/// Whether `to` can be reached from `from` at ground speed.
fn at_ground_speed(from: &TrustedFix, to: &TrustedFix) -> bool {
    let seconds = ((to.arrived_ms - from.arrived_ms) as f64 / 1000.0).max(1.0);
    // Two fixes a second apart can wobble by their accuracies without moving.
    let apart = (math::distance(from.position(), to.position()) - from.accuracy - to.accuracy).max(0.0);
    apart / seconds <= GROUND_MPS
}

impl Trust {
    /// Takes one accepted sample into account.
    pub(crate) fn take(&mut self, taken: &Taken) {
        if taken.mocked {
            // The device mocks its location now: nothing it says is the real position.
            self.fix = None;
            self.run = None;
            self.coarse = false;
            return;
        }
        if !taken.newest {
            return;
        }
        if taken.accuracy > TRUSTED_ACCURACY_M {
            self.coarse = true;
            return;
        }
        self.coarse = false;
        let fix = TrustedFix {
            lat: taken.lat,
            lng: taken.lng,
            accuracy: taken.accuracy,
            arrived_ms: taken.arrived_ms,
            staying: taken.staying,
        };
        let follows = self.anchor.is_none_or(|anchor| {
            let gap = fix.arrived_ms - anchor.arrived_ms;
            if gap <= SHORT_GAP_MS {
                at_ground_speed(&anchor, &fix)
            } else {
                math::distance(anchor.position(), fix.position()) <= STILL_M
            }
        });
        if follows || self.extend_run(fix) {
            self.fix = Some(fix);
            self.anchor = Some(fix);
            self.run = None;
        }
    }

    /// Adds a jump to the run of fixes at the new place; true when the run
    /// is now long enough to be the real position.
    fn extend_run(&mut self, fix: TrustedFix) -> bool {
        let run = match self.run {
            Some(run) if fix.arrived_ms - run.last.arrived_ms <= RUN_GAP_MS && at_ground_speed(&run.last, &fix) => {
                Run { first_ms: run.first_ms, last: fix, count: run.count + 1 }
            }
            _ => Run { first_ms: fix.arrived_ms, last: fix, count: 1 },
        };
        self.run = Some(run);
        run.count >= RUN_FIXES && run.last.arrived_ms - run.first_ms >= RUN_SPAN_MS
    }

    /// The staying flag ends with the stay (sharing stopped).
    pub(crate) fn left_stay(&mut self) {
        if let Some(fix) = self.fix.as_mut() {
            fix.staying = false;
        }
    }

    /// Whether the account can plausibly be at `claim` at `now_ms`.
    pub(crate) fn verdict(&self, claim: &Claim, now_ms: i64) -> Verdict {
        if plausible(self.fix.as_ref(), claim, now_ms) {
            Verdict::Plausible
        } else if self.coarse {
            Verdict::TooCoarse
        } else {
            Verdict::NotVerified
        }
    }
}

/// Whether `claim` is within reach of `fix` at `now_ms`.
fn plausible(fix: Option<&TrustedFix>, claim: &Claim, now_ms: i64) -> bool {
    let Some(fix) = fix else { return false };
    if !geoindex::valid_coordinates(claim.lat, claim.lng) {
        return false;
    }
    let age_ms = (now_ms - fix.arrived_ms).max(0);
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
    async fn check(&self, user_id: Uuid, claim: Claim) -> AppResult<Verdict> {
        let trust = self.load_state(user_id).await?.map(|state| state.trust).unwrap_or_default();
        Ok(trust.verdict(&claim, Utc::now().timestamp_millis()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HERE: Point = (35.6595, 139.7005);
    const NOW: i64 = 1_800_000_000_000;
    const MINUTE: i64 = 60_000;

    fn north(meters: f64) -> Point {
        (HERE.0 + meters / 111_195.0, HERE.1)
    }

    fn fix_at(at: Point, arrived_ms: i64) -> TrustedFix {
        TrustedFix { lat: at.0, lng: at.1, accuracy: 10.0, arrived_ms, staying: false }
    }

    fn taken(at: Point, arrived_ms: i64) -> Taken {
        Taken { lat: at.0, lng: at.1, accuracy: 10.0, mocked: false, staying: false, newest: true, arrived_ms }
    }

    fn claim(at: Point) -> Claim {
        Claim { lat: at.0, lng: at.1, accuracy_meters: 5.0 }
    }

    /// The trust after these samples, in order.
    fn after(samples: &[Taken]) -> Trust {
        let mut trust = Trust::default();
        for sample in samples {
            trust.take(sample);
        }
        trust
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
        let staying_long = TrustedFix { arrived_ms: NOW - FRESH_STAYING_MS - 1, ..staying };
        assert!(!plausible(Some(&staying_long), &claim(HERE), NOW));
    }

    #[test]
    fn time_since_the_fix_widens_the_reach_up_to_a_cap() {
        let fix = fix_at(HERE, NOW - 9 * MINUTE);
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
    fn the_first_fix_and_fixes_at_ground_speed_are_real() {
        let trust = after(&[taken(HERE, NOW)]);
        assert_eq!(trust.fix, Some(fix_at(HERE, NOW)));
        let moved = after(&[taken(HERE, NOW), taken(north(2_000.0), NOW + MINUTE)]);
        assert_eq!(moved.fix.map(|f| f.position()), Some(north(2_000.0)), "2 km in a minute: a train");
        let home = after(&[taken(HERE, NOW), taken(north(300.0), NOW + 8 * 60 * MINUTE)]);
        assert_eq!(home.fix.map(|f| f.position()), Some(north(300.0)), "after a night, a few steps away");
    }

    #[test]
    fn one_sample_far_away_is_no_journey() {
        let far = north(20_000.0);
        // A minute of silence, then 20 km: 333 m/s.
        let trust = after(&[taken(HERE, NOW), taken(far, NOW + MINUTE)]);
        assert_eq!(trust.fix, Some(fix_at(HERE, NOW)));
        // A day of silence, then the other side of the world.
        let trust = after(&[taken(HERE, NOW), taken((40.7, -74.0), NOW + 24 * 60 * MINUTE)]);
        assert_eq!(trust.fix, Some(fix_at(HERE, NOW)));
        // A mock in between does not help: the next real fix must follow from the anchor.
        let trust =
            after(&[taken(HERE, NOW), Taken { mocked: true, ..taken(HERE, NOW + 1_000) }, taken(far, NOW + MINUTE)]);
        assert_eq!(trust.fix, None);
    }

    #[test]
    fn a_jump_becomes_real_after_consistent_fixes_over_two_minutes() {
        let far = north(20_000.0);
        let mut samples = vec![taken(HERE, NOW), taken(far, NOW + MINUTE), taken(far, NOW + 2 * MINUTE)];
        assert_eq!(after(&samples).fix, Some(fix_at(HERE, NOW)), "two fixes are not enough");
        samples.push(taken(north(20_050.0), NOW + 2 * MINUTE + 30_000));
        assert_eq!(after(&samples).fix, Some(fix_at(HERE, NOW)), "three over 90 s are not enough");
        let mut quick = vec![taken(HERE, NOW)];
        quick.extend((1..=10).map(|i| taken(far, NOW + i * 5_000)));
        assert_eq!(after(&quick).fix, Some(fix_at(HERE, NOW)), "ten fixes in 50 s are not two minutes");
        samples.push(taken(far, NOW + 3 * MINUTE + 10_000));
        assert_eq!(after(&samples).fix.map(|f| f.position()), Some(far));
        // A run that wanders off (another jump) starts over.
        let wandering = after(&[
            taken(HERE, NOW),
            taken(far, NOW + MINUTE),
            taken((40.7, -74.0), NOW + 2 * MINUTE),
            taken(far, NOW + 3 * MINUTE),
        ]);
        assert_eq!(wandering.fix, Some(fix_at(HERE, NOW)));
    }

    #[test]
    fn only_the_newest_sample_of_an_upload_counts() {
        let backlog = Taken { newest: false, ..taken(north(10.0), NOW + 1_000) };
        assert_eq!(after(&[taken(HERE, NOW), backlog]).fix, Some(fix_at(HERE, NOW)));
        // A whole fake run in one upload is one sample.
        let mut batch = vec![taken(HERE, NOW)];
        batch.extend((0..5).map(|_| Taken { newest: false, ..taken(north(20_000.0), NOW + MINUTE) }));
        batch.push(taken(north(20_000.0), NOW + MINUTE));
        assert_eq!(after(&batch).fix, Some(fix_at(HERE, NOW)));
    }

    #[test]
    fn coarse_and_mocked_samples_are_told_apart() {
        let coarse = Taken { accuracy: 500.0, ..taken(north(400.0), NOW + MINUTE) };
        let trust = after(&[taken(HERE, NOW), coarse]);
        assert_eq!(trust.fix, Some(fix_at(HERE, NOW)), "a coarse fix keeps the previous one");
        assert_eq!(trust.verdict(&claim(HERE), NOW + MINUTE), Verdict::Plausible);
        assert_eq!(trust.verdict(&claim(north(400.0)), NOW + MINUTE), Verdict::TooCoarse);
        assert_eq!(
            after(&[coarse]).verdict(&claim(north(400.0)), NOW + MINUTE),
            Verdict::TooCoarse,
            "approximate location only"
        );
        let mocked = after(&[taken(HERE, NOW), Taken { mocked: true, ..taken(HERE, NOW + 5_000) }]);
        assert_eq!(
            mocked.verdict(&claim(HERE), NOW + 5_000),
            Verdict::NotVerified,
            "a mock location forgets the real fix"
        );
        let fine_again = after(&[coarse, taken(north(400.0), NOW + 2 * MINUTE)]);
        assert_eq!(fine_again.verdict(&claim(north(1_000.0)), NOW + 2 * MINUTE), Verdict::NotVerified);
    }
}
