//! Positions claimed by the client.
//!
//! Nearby discovery and bumps are matched on a position the client sends
//! with the request. On its own that is the client's word: anyone could open
//! the add-friend screen, or shake, "at" any point of the map, to see who is
//! there. So the claim is checked first against where the account really
//! is, as far as the server knows it: the geo domain, which receives the
//! account's location samples, implements [`PositionCheck`] and is plugged
//! in with [`crate::Social::set_position_check`].
//!
//! The answer is a verdict about the caller's own claim (plausible, not
//! verified, or too coarse: the device only gives approximate locations) and
//! nothing else, never a position or a distance: social never learns where
//! the account really is, and so cannot pass anything derived from it on to
//! the people it shows the claimer to.
//! What those people get is computed from the claim alone (a distance band,
//! one's own shake position); the true position only decides whether the
//! claimer takes part at all.
//!
//! What a refused claim gets, per feature:
//!
//! - **Nearby discovery** refuses the session (`LOCATION_NOT_VERIFIED`). It
//!   cannot be moved to the real position instead: social does not have it,
//!   and a session placed there would show strangers a distance band to the
//!   true position, which is what ghost mode forbids for anyone who does not
//!   see the person precisely.
//!   A device that only reports approximate locations (worse than 100 m)
//!   gets `LOCATION_TOO_COARSE` instead, so it can ask for precise location.
//! - **Bumps** refuse the shake (`LOCATION_NOT_VERIFIED` /
//!   `LOCATION_TOO_COARSE`) before it is
//!   published, for the same reason: a bump moved to the real position would
//!   tell friends of both the point between the true positions. A refused
//!   shake can match nobody, so the other phone simply sees no bump.
//!
//! Without a check plugged in every claim is refused: a server that forgot
//! the wiring fails closed.
use std::sync::Arc;

use tokuly_core::prelude::*;

/// A position the client says the account is at, as sent with the request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Claim {
    pub lat: f64,
    pub lng: f64,
    /// Reported accuracy in meters (the client's word as well).
    pub accuracy_meters: f64,
}

/// What the check says about a claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Plausible,
    /// No recent real fix puts the account there (or none at all).
    NotVerified,
    /// Not plausible, and the device's latest location was refused only for
    /// being too coarse (approximate location).
    TooCoarse,
}

/// Decides whether a claimed position is plausible for an account, from that
/// account's own recent location fixes. Implemented by the geo domain.
#[async_trait::async_trait]
pub trait PositionCheck: Send + Sync + 'static {
    /// Whether `user_id` may plausibly be at `claim` right now.
    async fn check(&self, user_id: Uuid, claim: Claim) -> AppResult<Verdict>;
}

/// The check in use; `None` until one is plugged in.
pub(crate) type Slot = std::sync::OnceLock<Arc<dyn PositionCheck>>;

/// The error for a claim a device with only approximate locations made.
pub(crate) fn too_coarse() -> AppError {
    AppError::precondition("LOCATION_TOO_COARSE", "your location is too approximate; turn on precise location and try again")
}

/// The error for a claim the check refused (or could not confirm).
pub(crate) fn not_verified() -> AppError {
    AppError::precondition(
        "LOCATION_NOT_VERIFIED",
        "your current location could not be confirmed; turn on location sharing and try again where you are",
    )
}

impl crate::Social {
    /// Plugs in the check claimed positions go through. Only the first call
    /// counts (the server wires it once at startup).
    pub fn set_position_check(&self, check: Arc<dyn PositionCheck>) {
        if self.position_check.set(check).is_err() {
            tracing::warn!("a position check was already set; keeping the first one");
        }
    }

    /// Fails with `LOCATION_NOT_VERIFIED` (or `LOCATION_TOO_COARSE`) unless
    /// the claim is plausible for `user_id`.
    pub(crate) async fn require_plausible(&self, user_id: Uuid, claim: Claim) -> AppResult<()> {
        let Some(check) = self.position_check.get() else { return Err(not_verified()) };
        match check.check(user_id, claim).await? {
            Verdict::Plausible => Ok(()),
            Verdict::NotVerified => Err(not_verified()),
            Verdict::TooCoarse => Err(too_coarse()),
        }
    }
}
