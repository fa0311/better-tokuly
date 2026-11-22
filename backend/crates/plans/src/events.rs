//! Domain events emitted by the plans domain (see `docs/events/plans.md`).
//!
//! Consumers deserialize these payloads with [`tokuly_core::events::Event::data`].
//! None of them carries user-written text or a position: a note, an area or
//! a plan title stays inside the domain, visible to the people it was meant for.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const FREE_SET: &str = "plans.free.set";
pub const PLAN_CREATED: &str = "plans.plan.created";
pub const PLAN_CANCELLED: &str = "plans.plan.cancelled";
pub const PLAN_RSVP_CHANGED: &str = "plans.plan.rsvp_changed";
pub const PLAN_ATTENDED: &str = "plans.plan.attended";
pub const POLL_CREATED: &str = "plans.poll.created";
pub const POLL_FINALIZED: &str = "plans.poll.finalized";
pub const ANNIVERSARY_STARTED: &str = "plans.anniversary.started";

/// `plans.free.set`: `user_id` said "ひま" for a window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FreeSet {
    pub user_id: Uuid,
    pub status_id: Uuid,
    /// `tokuly.plans.v1.FreeWindow` as its number.
    pub window: i32,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    /// `tokuly.plans.v1.FreeAudience` as its number.
    pub audience: i32,
}

/// `plans.plan.created`: `host_id` created a plan and invited people.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanCreated {
    pub plan_id: Uuid,
    pub host_id: Uuid,
    pub invitee_ids: Vec<Uuid>,
    /// When it starts; `None` while the time is to be decided.
    pub starts_at: Option<DateTime<Utc>>,
    /// The date poll it came out of, if any.
    pub poll_id: Option<Uuid>,
}

/// `plans.plan.cancelled`: the host called a plan off.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanCancelled {
    pub plan_id: Uuid,
    pub host_id: Uuid,
}

/// `plans.plan.rsvp_changed`: `user_id` answered an invitation (or changed
/// their answer, or moved up from the waitlist).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanRsvpChanged {
    pub plan_id: Uuid,
    pub user_id: Uuid,
    /// `tokuly.plans.v1.Rsvp` as its number.
    pub rsvp: i32,
    pub previous: i32,
}

/// `plans.plan.attended`: the plan's time has passed and `user_id` was
/// going. Emitted once per person who was going, only for plans that existed
/// before they started.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanAttended {
    pub plan_id: Uuid,
    pub user_id: Uuid,
    pub host_id: Uuid,
    /// Everyone who was going, `user_id` included.
    pub attendee_ids: Vec<Uuid>,
}

/// `plans.poll.created`: `creator_id` started a date poll.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollCreated {
    pub poll_id: Uuid,
    pub creator_id: Uuid,
    pub invitee_ids: Vec<Uuid>,
    pub slot_count: i32,
}

/// `plans.poll.finalized`: a date poll turned into a plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollFinalized {
    pub poll_id: Uuid,
    pub creator_id: Uuid,
    pub plan_id: Uuid,
    /// How many people answered the poll.
    pub answered_count: i32,
}

/// `plans.anniversary.started`: a "days since" counter came into being — a
/// friendship counter was started, or a couple invitation was accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnniversaryStarted {
    pub anniversary_id: Uuid,
    /// `tokuly.plans.v1.AnniversaryKind` as its number.
    pub kind: i32,
    pub user_ids: [Uuid; 2],
}
