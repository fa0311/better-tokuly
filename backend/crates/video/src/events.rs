//! Events the video domain emits, and the payloads of the ones it consumes.
//!
//! Emitted (all in the transaction of the change they describe):
//!
//! | Topic | Payload | When |
//! |---|---|---|
//! | `video.channel.created` / `.updated` / `.deleted` | [`ChannelEvent`] | a channel was created, edited (name, handle, art) or deleted with its content |
//! | `video.channel.subscribed` / `.unsubscribed` | [`SubscriptionEvent`] | a subscription began or ended |
//! | `video.video.created` | [`VideoEvent`] | a draft was created |
//! | `video.video.published` | [`VideoEvent`] | a video went live (first time or again) |
//! | `video.video.unpublished` | [`VideoEvent`] | a published video went back to draft |
//! | `video.video.visibility_changed` | [`VideoVisibilityChanged`] | a published video got another audience (it stays published), or its owner locked or unlocked their account |
//! | `video.video.deleted` | [`VideoEvent`] | a video is gone (owner, channel deletion, takedown, account deletion) |
//! | `video.playlist.deleted` | [`PlaylistEvent`] | a playlist is gone |
use serde::{Deserialize, Serialize};
use tokuly_core::prelude::*;

pub const CHANNEL_CREATED: &str = "video.channel.created";
pub const CHANNEL_UPDATED: &str = "video.channel.updated";
pub const CHANNEL_DELETED: &str = "video.channel.deleted";
pub const CHANNEL_SUBSCRIBED: &str = "video.channel.subscribed";
pub const CHANNEL_UNSUBSCRIBED: &str = "video.channel.unsubscribed";
pub const VIDEO_CREATED: &str = "video.video.created";
pub const VIDEO_PUBLISHED: &str = "video.video.published";
pub const VIDEO_UNPUBLISHED: &str = "video.video.unpublished";
pub const VIDEO_VISIBILITY_CHANGED: &str = "video.video.visibility_changed";
pub const VIDEO_DELETED: &str = "video.video.deleted";
pub const PLAYLIST_DELETED: &str = "video.playlist.deleted";
/// Internal: the search pictures of some public playlists may show a video
/// that changed (see [`PlaylistsStale`]). Consumed by this domain only.
pub const PLAYLISTS_STALE: &str = "video.playlists.stale";

// Topics of other domains this one consumes.
pub const USER_DELETED: &str = "identity.user.deleted";
pub const USER_BANNED: &str = "identity.user.banned";
pub const USER_UNBANNED: &str = "identity.user.unbanned";
pub const USER_BLOCKED: &str = "social.user.blocked";
pub const FOLLOW_SETTINGS_CHANGED: &str = "social.follow.settings_changed";
pub const CONTENT_REMOVED: &str = "admin.content.removed";
pub const ASSET_READY: &str = tokuly_media::events::ASSET_READY;
pub const ASSET_FAILED: &str = tokuly_media::events::ASSET_FAILED;
pub const ASSET_DELETED: &str = tokuly_media::events::ASSET_DELETED;

/// Payload of `social.follow.settings_changed` (only a hint: the social
/// domain is asked again).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowSettingsChanged {
    pub user_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelEvent {
    pub channel_id: Uuid,
    pub owner_id: Uuid,
    pub handle: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionEvent {
    pub channel_id: Uuid,
    /// The channel's owner.
    pub owner_id: Uuid,
    /// The subscriber.
    pub user_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoEvent {
    pub video_id: Uuid,
    pub short_id: String,
    pub channel_id: Uuid,
    pub owner_id: Uuid,
    /// `tokuly.common.v1.Visibility` as its number.
    pub visibility: i32,
    /// `tokuly.video.v1.VideoOrigin` as its number, and the live stream an
    /// archive came from.
    pub origin: i32,
    #[serde(default)]
    pub origin_id: Option<Uuid>,
    /// Only on `video.video.published`: the first time this video reached an
    /// audience (subscribers are told once).
    #[serde(default)]
    pub first_publication: bool,
}

/// Payload of `video.video.visibility_changed`: the video, with `visibility`
/// the audience now in effect (an open video of a locked owner reads as
/// FOLLOWERS), and who besides the owner may still watch it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoVisibilityChanged {
    #[serde(flatten)]
    pub video: VideoEvent,
    /// `None` (JSON `null`): everyone may. Otherwise the people in the
    /// audience as the social graph gives them (for a lock: only the ones
    /// this domain may have told about the video). **Never forward to
    /// outside parties: it is the owner's friend and follower list.**
    pub visible_to: Option<Vec<Uuid>>,
    /// When the change was made, taken while the video row was locked:
    /// consumers order two changes of one video by it.
    pub changed_at: Time,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistEvent {
    pub playlist_id: Uuid,
    pub owner_id: Uuid,
}

/// Payload of `social.user.blocked`.
#[derive(Debug, Clone, Deserialize)]
pub struct UserBlocked {
    pub user_id: Uuid,
    pub blocked_id: Uuid,
}

/// Payload of `admin.content.removed`.
#[derive(Debug, Clone, Deserialize)]
pub struct ContentRemoved {
    pub subject_type: i32,
    pub subject_id: Uuid,
}

/// Payload of the `identity.user.*` events consumed here.
#[derive(Debug, Clone, Deserialize)]
pub struct UserRef {
    pub user_id: Uuid,
}

/// Payload of the `video.announce` job: tell the next batch of a channel's
/// subscribers about a newly published video.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnounceJob {
    pub video_id: Uuid,
    /// Subscribers up to and including this user ID have been handled.
    #[serde(default)]
    pub after: Option<Uuid>,
}

pub const JOB_ANNOUNCE: &str = "video.announce";

/// Payload of `video.playlists.stale`, emitted in the transaction of the
/// change: the public playlists listing `video_id`, or the `playlist_ids`
/// given (for a deleted video, whose list entries went with it), get their
/// search document again.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlaylistsStale {
    #[serde(default)]
    pub video_id: Option<Uuid>,
    #[serde(default)]
    pub playlist_ids: Vec<Uuid>,
}

/// Payload of the `video.playlists.search` job: one batch of
/// [`PlaylistsStale`]. With `video_id`, the public playlists listing it
/// after `after` (by ID); `playlist_ids` are refreshed as they are.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlaylistSearchJob {
    #[serde(default)]
    pub video_id: Option<Uuid>,
    #[serde(default)]
    pub playlist_ids: Vec<Uuid>,
    #[serde(default)]
    pub after: Option<Uuid>,
}

pub const JOB_PLAYLIST_SEARCH: &str = "video.playlists.search";
