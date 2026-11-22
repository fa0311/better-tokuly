//! Events the live domain emits and consumes, and its jobs.
//!
//! | Topic | Payload | When |
//! |---|---|---|
//! | `live.stream.started` | [`StreamEvent`] | a stream went on air |
//! | `live.stream.ended` | [`StreamEvent`] | a stream is over (`end_reason` says why) |
//! | `live.stream.deleted` | [`StreamEvent`] | a stream is gone |
//! | `live.stream.archived` | [`StreamArchived`] | the recording became a video |
//! | `live.gift.sent` | [`GiftSent`] | coins moved from a viewer to a streamer |
//! | `live.clip.ready` | [`ClipEvent`] | a clip finished processing |
//! | `live.clip.deleted` | [`ClipEvent`] | a clip is gone |
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const STREAM_STARTED: &str = "live.stream.started";
pub const STREAM_ENDED: &str = "live.stream.ended";
pub const STREAM_DELETED: &str = "live.stream.deleted";
pub const STREAM_ARCHIVED: &str = "live.stream.archived";
pub const GIFT_SENT: &str = "live.gift.sent";
pub const CLIP_READY: &str = "live.clip.ready";
pub const CLIP_DELETED: &str = "live.clip.deleted";

// Consumed from other domains.
pub const USER_DELETED: &str = "identity.user.deleted";
pub const USER_BANNED: &str = "identity.user.banned";
pub const USER_UNBANNED: &str = "identity.user.unbanned";
pub const USER_BLOCKED: &str = "social.user.blocked";
pub const CONTENT_REMOVED: &str = "admin.content.removed";
pub const ASSET_READY: &str = tokuly_media::events::ASSET_READY;
pub const ASSET_FAILED: &str = tokuly_media::events::ASSET_FAILED;
pub const ASSET_DELETED: &str = tokuly_media::events::ASSET_DELETED;
pub const CHANNEL_DELETED: &str = tokuly_video::events::CHANNEL_DELETED;
pub const CHANNEL_SUBSCRIBED: &str = tokuly_video::events::CHANNEL_SUBSCRIBED;
pub const VIDEO_PUBLISHED: &str = tokuly_video::events::VIDEO_PUBLISHED;
pub const VIDEO_UNPUBLISHED: &str = tokuly_video::events::VIDEO_UNPUBLISHED;
pub const VIDEO_DELETED: &str = tokuly_video::events::VIDEO_DELETED;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEvent {
    pub stream_id: Uuid,
    pub short_id: String,
    pub channel_id: Uuid,
    pub owner_id: Uuid,
    /// `tokuly.common.v1.Visibility` as its number.
    pub visibility: i32,
    /// `tokuly.live.v1.EndReason` as its number; 0 unless ended.
    #[serde(default)]
    pub end_reason: i32,
    /// Whether the stream was ever on air (an ended stream that was not is
    /// a cancelled one).
    #[serde(default)]
    pub went_live: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamArchived {
    pub stream_id: Uuid,
    pub video_id: Uuid,
    pub channel_id: Uuid,
    pub owner_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GiftSent {
    pub gift_id: Uuid,
    /// `None` when the stream was gone by the time the gift settled.
    #[serde(default)]
    pub stream_id: Option<Uuid>,
    pub channel_id: Uuid,
    pub sender_id: Uuid,
    pub recipient_id: Uuid,
    pub coins: i64,
    /// What reached the recipient's earnings.
    pub net_coins: i64,
    pub tier: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipEvent {
    pub clip_id: Uuid,
    pub short_id: String,
    pub creator_id: Uuid,
    pub channel_id: Uuid,
    #[serde(default)]
    pub stream_id: Option<Uuid>,
    #[serde(default)]
    pub video_id: Option<Uuid>,
    #[serde(default)]
    pub asset_id: Option<Uuid>,
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

// ───────────────────────────── jobs ─────────────────────────────

/// Tells the next batch of people that a stream went on air.
pub const JOB_ANNOUNCE: &str = "live.announce";
/// Turns the recording of an ended stream into a video.
pub const JOB_ARCHIVE: &str = "live.archive";
/// Cuts a clip.
pub const JOB_CLIP: &str = "live.clip";

/// Whom a `live.announce` job is telling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnouncePhase {
    /// People who asked for a reminder (`live.reminder`).
    Reminders,
    /// Subscribers of the channel (`live.started`).
    Subscribers,
    /// Friends of the owner, for friends-only streams (`live.started`).
    Friends,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnounceJob {
    pub stream_id: Uuid,
    pub phase: AnnouncePhase,
    /// Recipients up to and including this user ID have been handled.
    #[serde(default)]
    pub after: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveJob {
    pub stream_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipJob {
    pub clip_id: Uuid,
}
