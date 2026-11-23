//! Domain events of the spaces domain (see `docs/events/spaces.md`).
//!
//! Consumers deserialize the emitted payloads with
//! [`tokuly_core::events::Event::data`]. Every event is written in the
//! transaction that makes the change it describes.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── emitted ──

pub const ROOM_STARTED: &str = "spaces.room.started";
pub const ROOM_ENDED: &str = "spaces.room.ended";
pub const ROOM_JOINED: &str = "spaces.room.joined";
pub const HANGOUT_JOINED: &str = "spaces.hangout.joined";

/// `spaces.room.started`: a room went live. Actor: the host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomStarted {
    pub space_id: Uuid,
    pub host_id: Uuid,
    pub title: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub language: String,
    /// `tokuly.spaces.v1.SpaceAudience` as its number (1 public, 2 friends,
    /// 3 invited only).
    pub audience: i32,
    /// The room was announced in advance (rather than started on the spot).
    #[serde(default)]
    pub scheduled: bool,
}

/// `spaces.room.ended`: a live room is over. Actor: the host (the last one,
/// if the room was handed over).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomEnded {
    pub space_id: Uuid,
    pub host_id: Uuid,
    /// `tokuly.spaces.v1.SpaceAudience` as its number.
    pub audience: i32,
    /// `tokuly.spaces.v1.SpaceEndReason` as its number.
    pub reason: i32,
    /// Whole minutes the room was live, rounded up.
    pub minutes: i64,
    /// The most people that were inside at once, the stage included.
    pub peak_listeners: i32,
    /// Different people who entered at some point.
    pub total_listeners: i32,
}

/// `spaces.room.joined`: someone entered a room for the first time (coming
/// back after leaving does not repeat it; the host starting their own room
/// is `spaces.room.started`). Actor: the person.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomJoined {
    pub space_id: Uuid,
    pub user_id: Uuid,
    pub host_id: Uuid,
    /// `tokuly.spaces.v1.SpaceAudience` as its number.
    pub audience: i32,
}

/// `spaces.hangout.joined`: someone entered a hangout's voice room. Actor:
/// the person.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HangoutJoined {
    pub hangout_id: Uuid,
    pub user_id: Uuid,
    /// Nobody else was inside: they opened the hangout.
    pub first: bool,
    /// People inside after they entered.
    pub inside_count: i32,
}

// ── consumed ──

pub const USER_DELETED: &str = "identity.user.deleted";
pub const USER_BANNED: &str = "identity.user.banned";
pub const USER_BLOCKED: &str = "social.user.blocked";
pub const FRIEND_REMOVED: &str = "social.friend.removed";
pub const CONTENT_REMOVED: &str = "admin.content.removed";

/// `identity.user.deleted` / `identity.user.banned`.
#[derive(Debug, Clone, Deserialize)]
pub struct UserGone {
    pub user_id: Uuid,
}

/// `social.user.blocked`: `user_id` blocked `blocked_id`.
#[derive(Debug, Clone, Deserialize)]
pub struct UserBlocked {
    pub user_id: Uuid,
    pub blocked_id: Uuid,
}

/// `social.friend.removed`: a friendship ended.
#[derive(Debug, Clone, Deserialize)]
pub struct FriendRemoved {
    pub user_id: Uuid,
    pub friend_id: Uuid,
}

/// `admin.content.removed`: a moderator took something down.
#[derive(Debug, Clone, Deserialize)]
pub struct ContentRemoved {
    /// `tokuly.common.v1.SubjectType` as its number.
    pub subject_type: i32,
    pub subject_id: Uuid,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub moderator_id: Option<Uuid>,
}
