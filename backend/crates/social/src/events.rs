//! Domain events emitted by the social domain (see `docs/events/social.md`).
//!
//! Consumers deserialize these payloads with [`tokuly_core::events::Event::data`].
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const FRIEND_REQUESTED: &str = "social.friend.requested";
pub const FRIEND_ACCEPTED: &str = "social.friend.accepted";
pub const FRIEND_REMOVED: &str = "social.friend.removed";
pub const FRIEND_PREFS_CHANGED: &str = "social.friend.prefs_changed";
pub const USER_BLOCKED: &str = "social.user.blocked";
pub const USER_UNBLOCKED: &str = "social.user.unblocked";
pub const BUMP_CREATED: &str = "social.bump.created";
pub const INTRO_CREATED: &str = "social.intro.created";

/// `social.friend.requested`: a friend request was sent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendRequested {
    pub request_id: Uuid,
    pub from_user_id: Uuid,
    pub to_user_id: Uuid,
    /// `tokuly.social.v1.FriendSource` as its number.
    pub source: i32,
}

/// `social.friend.accepted`: two users became friends. `user_id` accepted
/// (or completed a crossed request); `friend_id` had asked first.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendAccepted {
    pub user_id: Uuid,
    pub friend_id: Uuid,
    pub source: i32,
}

/// `social.friend.removed`: a friendship ended. `user_id` ended it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendRemoved {
    pub user_id: Uuid,
    pub friend_id: Uuid,
    /// The friendship ended because `user_id` blocked `friend_id`.
    pub blocked: bool,
}

/// `social.friend.prefs_changed`: `user_id` muted or unmuted `friend_id`.
/// The notify domain keeps its mute list in sync from this.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendPrefsChanged {
    pub user_id: Uuid,
    pub friend_id: Uuid,
    pub muted: bool,
}

/// `social.user.blocked` / `social.user.unblocked`: `user_id` (un)blocked `blocked_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockChanged {
    pub user_id: Uuid,
    pub blocked_id: Uuid,
}

/// `social.bump.created`: two users bumped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BumpCreated {
    pub bump_id: Uuid,
    pub user_ids: [Uuid; 2],
    /// False when the two were not friends (a friend request may have been sent).
    pub friends: bool,
    pub lat: f64,
    pub lng: f64,
}

/// `social.intro.created`: `user_id` joined through `inviter_id`'s invite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntroCreated {
    pub inviter_id: Uuid,
    pub user_id: Uuid,
}
