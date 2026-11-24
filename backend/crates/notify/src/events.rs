//! Event payloads of the notify domain (see `docs/events/notify.md`).
//!
//! Consumed:
//! - `notify.requested` — `tokuly_core::outbound::NotificationRequest`
//! - `social.friend.prefs_changed` — [`FriendPrefsChanged`]
//! - `identity.user.deleted` — [`UserDeleted`]
//! - `social.user.blocked`, `social.user.unblocked` — [`UserBlocked`]
//! - deletion events of other domains (see `withdraw.rs`)
//!
//! Emitted:
//! - `notify.mute.changed` — [`MuteChanged`]
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const FRIEND_PREFS_CHANGED_TOPIC: &str = "social.friend.prefs_changed";
pub const USER_DELETED_TOPIC: &str = "identity.user.deleted";
pub const USER_BLOCKED_TOPIC: &str = "social.user.blocked";
pub const USER_UNBLOCKED_TOPIC: &str = "social.user.unblocked";
pub const MUTE_CHANGED_TOPIC: &str = "notify.mute.changed";

/// Our view of the social domain's event: `user_id` changed their
/// preferences about `friend_id`. Only the mute flag matters here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendPrefsChanged {
    pub user_id: Uuid,
    pub friend_id: Uuid,
    pub muted: bool,
}

/// Our view of the social domain's block events: `user_id` blocked (or
/// unblocked) `blocked_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserBlocked {
    pub user_id: Uuid,
    pub blocked_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserDeleted {
    pub user_id: Uuid,
}

/// `user_id` muted or unmuted notifications caused by `muted_user_id` through
/// the notify RPCs. Not emitted for changes that arrived from the social
/// domain, so the two cannot echo each other.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MuteChanged {
    pub user_id: Uuid,
    pub muted_user_id: Uuid,
    pub muted: bool,
}
