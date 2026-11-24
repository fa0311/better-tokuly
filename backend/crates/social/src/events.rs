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

pub const FOLLOW_CREATED: &str = "social.follow.created";
pub const FOLLOW_REMOVED: &str = "social.follow.removed";
pub const FOLLOW_REQUESTED: &str = "social.follow.requested";
pub const FOLLOW_REQUEST_WITHDRAWN: &str = "social.follow.request_withdrawn";
pub const FOLLOW_PREFS_CHANGED: &str = "social.follow.prefs_changed";
pub const FOLLOW_SETTINGS_CHANGED: &str = "social.follow.settings_changed";
pub const CIRCLE_CREATED: &str = "social.circle.created";
pub const CIRCLE_UPDATED: &str = "social.circle.updated";
pub const CIRCLE_DELETED: &str = "social.circle.deleted";
pub const CIRCLE_MEMBERS_CHANGED: &str = "social.circle.members_changed";

/// `social.follow.created`: `follower_id` follows `followee_id` now.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowCreated {
    pub follower_id: Uuid,
    pub followee_id: Uuid,
    /// The followee approved a request (or unlocked their account).
    pub approved: bool,
}

/// Why a follow ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FollowEndReason {
    /// The follower unfollowed.
    Unfollowed,
    /// The followee removed the follower.
    Removed,
    /// One of the two blocked the other.
    Blocked,
}

/// `social.follow.removed`: `follower_id` no longer follows `followee_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowRemoved {
    pub follower_id: Uuid,
    pub followee_id: Uuid,
    pub reason: FollowEndReason,
}

/// `social.follow.requested`: `follower_id` asked to follow the locked
/// account `followee_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowRequested {
    pub request_id: Uuid,
    pub follower_id: Uuid,
    pub followee_id: Uuid,
}

/// `social.follow.request_withdrawn`: a request the followee could still
/// answer is gone without an answer (withdrawn by its sender, or dropped by
/// a block). The notification about it can go too.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowRequestWithdrawn {
    pub request_id: Uuid,
    pub follower_id: Uuid,
    pub followee_id: Uuid,
}

/// `social.follow.prefs_changed`: the follower changed their mute or
/// notification level for `followee_id`. Private to the follower.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowPrefsChanged {
    pub follower_id: Uuid,
    pub followee_id: Uuid,
    pub muted: bool,
    /// When a timed mute ends (`None`: no end, or not muted).
    pub muted_until: Option<chrono::DateTime<chrono::Utc>>,
    /// `tokuly.social.v1.FollowNotifyLevel` as its number.
    pub notify_level: i32,
}

/// `social.follow.settings_changed`: `user_id` locked or unlocked their account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowSettingsChanged {
    pub user_id: Uuid,
    pub locked: bool,
}

/// `social.circle.created` / `social.circle.updated`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircleChanged {
    pub circle_id: Uuid,
    pub owner_id: Uuid,
    /// `tokuly.social.v1.CircleKind` as its number.
    pub kind: i32,
}

/// `social.circle.deleted`: content shared with the circle is the owner's
/// alone from now on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircleDeleted {
    pub circle_id: Uuid,
    pub owner_id: Uuid,
    /// Who was in it.
    #[serde(default)]
    pub member_ids: Vec<Uuid>,
}

/// `social.circle.members_changed`: people were added to or taken out of a
/// circle (by its owner, or because their tie to the owner ended). Never
/// shown to the members.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircleMembersChanged {
    pub circle_id: Uuid,
    pub owner_id: Uuid,
    #[serde(default)]
    pub added: Vec<Uuid>,
    #[serde(default)]
    pub removed: Vec<Uuid>,
}
