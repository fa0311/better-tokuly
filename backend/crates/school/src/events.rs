//! Domain events emitted by the school domain (see `docs/events/school.md`).
//!
//! Consumers deserialize these payloads with [`tokuly_core::events::Event::data`].
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const SESSION_ENDED: &str = "school.study.session_ended";
pub const GOAL_REACHED: &str = "school.study.goal_reached";
pub const ROOM_OPENED: &str = "school.room.opened";
pub const ROOM_CLOSED: &str = "school.room.closed";

/// `school.study.session_ended`: a study session of at least one minute was
/// logged — the timer was stopped (by the user or the server), a stay in a
/// study room ended, or time was logged by hand.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEnded {
    pub user_id: Uuid,
    pub session_id: Uuid,
    /// Whole minutes of counted study time (pauses and breaks excluded).
    pub minutes: i32,
    pub seconds: i32,
    /// Name of the subject; empty when the session has none.
    pub subject: String,
    pub subject_id: Option<Uuid>,
    /// `tokuly.school.v1.SessionSource` as its number.
    pub source: i32,
    /// The study room the session took place in, if any.
    pub room_id: Option<Uuid>,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    /// Logged after the fact rather than timed by the server.
    pub manual: bool,
    /// Ended by the server (heartbeats stopped, room closed, limits).
    pub auto_stopped: bool,
}

/// `school.study.goal_reached`: a session took the user's study time of the
/// day (their time zone) past their daily goal. At most once per day.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalReached {
    pub user_id: Uuid,
    /// The local day, `YYYY-MM-DD`.
    pub date: String,
    pub minutes: i32,
    pub goal_minutes: i32,
    /// Consecutive days the goal has been reached, this one included.
    pub streak_days: i32,
}

/// `school.room.opened`: a study room was created.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomOpened {
    pub room_id: Uuid,
    pub host_id: Uuid,
    /// `tokuly.school.v1.RoomAudience` as its number.
    pub audience: i32,
    pub title: String,
}

/// `school.room.closed`: a study room closed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomClosed {
    pub room_id: Uuid,
    pub host_id: Uuid,
    /// `tokuly.school.v1.RoomCloseReason` as its number.
    pub reason: i32,
}

// ── Events of other domains this one reacts to ──

pub(crate) const USER_DELETED: &str = "identity.user.deleted";
pub(crate) const USER_BANNED: &str = "identity.user.banned";
pub(crate) const FRIEND_REMOVED: &str = tokuly_social::events::FRIEND_REMOVED;
pub(crate) const USER_BLOCKED: &str = tokuly_social::events::USER_BLOCKED;
pub(crate) const CONTENT_REMOVED: &str = "admin.content.removed";

/// `admin.content.removed`, as far as this domain reads it.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ContentRemoved {
    #[serde(default)]
    pub subject_type: i32,
    pub subject_id: Uuid,
}
