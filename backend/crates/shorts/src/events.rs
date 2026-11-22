//! Events the shorts domain emits, the payloads of the ones it consumes,
//! and the payloads of its background jobs.
//!
//! Emitted (all in the transaction of the change they describe):
//!
//! | Topic | Payload | When |
//! |---|---|---|
//! | `shorts.short.created` | [`ShortEvent`] | a draft was created |
//! | `shorts.short.published` | [`ShortEvent`] | a short went live, or reached an audience for the first time |
//! | `shorts.short.deleted` | [`ShortEvent`] | a short is gone (owner, channel or account deletion, takedown, deleted upload) |
use serde::{Deserialize, Serialize};
use tokuly_core::prelude::*;

pub const SHORT_CREATED: &str = "shorts.short.created";
pub const SHORT_PUBLISHED: &str = "shorts.short.published";
pub const SHORT_DELETED: &str = "shorts.short.deleted";

// Topics of other domains this one consumes.
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
pub const CHANNEL_UNSUBSCRIBED: &str = tokuly_video::events::CHANNEL_UNSUBSCRIBED;
pub const REACTION_CHANGED: &str = "engagement.reaction.changed";
pub const COMMENT_PUBLISHED: &str = "engagement.comment.published";
pub const REPORT_CREATED: &str = "engagement.report.created";
pub const REINDEX_REQUESTED: &str = "search.reindex.requested";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortEvent {
    /// The short's UUID.
    pub id: Uuid,
    /// Its 11-character public ID.
    pub short_id: String,
    pub channel_id: Uuid,
    pub owner_id: Uuid,
    /// `tokuly.common.v1.Visibility` as its number.
    pub visibility: i32,
    /// Only on `shorts.short.published`: the first time this short reached
    /// an audience (subscribers and friends are told once).
    #[serde(default)]
    pub first_publication: bool,
    /// Only on `shorts.short.deleted`: `owner`, `channel_deleted`,
    /// `account_deleted`, `moderation` or `asset_deleted`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

/// Payload of the `identity.user.*` events consumed here.
#[derive(Debug, Clone, Deserialize)]
pub struct UserRef {
    pub user_id: Uuid,
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

/// Payload of `video.channel.deleted`.
#[derive(Debug, Clone, Deserialize)]
pub struct ChannelDeleted {
    pub channel_id: Uuid,
}

/// Payload of `video.channel.subscribed` and `video.channel.unsubscribed`.
#[derive(Debug, Clone, Deserialize)]
pub struct ChannelSubscribed {
    pub channel_id: Uuid,
    /// The channel's owner.
    pub owner_id: Uuid,
    /// The subscriber.
    pub user_id: Uuid,
}

/// Payload of `engagement.reaction.changed`, as far as it is read here.
/// `kind` and `previous_kind` are `ReactionKind` numbers (1 = like, 0 = none).
#[derive(Debug, Clone, Deserialize)]
pub struct ReactionChanged {
    pub user_id: Uuid,
    pub subject_type: i32,
    pub subject_id: Uuid,
    #[serde(default)]
    pub kind: i32,
    #[serde(default)]
    pub previous_kind: i32,
}

/// Payload of `engagement.comment.published`, as far as it is read here.
#[derive(Debug, Clone, Deserialize)]
pub struct CommentPublished {
    pub subject_type: i32,
    pub subject_id: Uuid,
    pub author_id: Uuid,
}

/// Payload of `engagement.report.created`, as far as it is read here.
#[derive(Debug, Clone, Deserialize)]
pub struct ReportCreated {
    pub subject_type: i32,
    pub subject_id: Uuid,
}

/// Payload of `search.reindex.requested`.
#[derive(Debug, Clone, Deserialize)]
pub struct ReindexRequested {
    #[serde(default)]
    pub kind: String,
}

/// `shorts.announce`: tell the next batch of a channel's subscribers about
/// a newly published short.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnounceJob {
    pub short_id: Uuid,
    /// Subscribers up to and including this user ID have been handled.
    #[serde(default)]
    pub after: Option<Uuid>,
}

/// `shorts.cover`: cut the frame at `offset_ms` out of a short's video and
/// make it the cover.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverJob {
    pub short_id: Uuid,
    pub offset_ms: i32,
}

/// `shorts.reindex`: republish the next batch of search documents of a kind
/// (`short` or `hashtag`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReindexJob {
    pub kind: String,
    /// Rows up to and including this ID have been handled.
    #[serde(default)]
    pub after: Option<Uuid>,
}

pub const JOB_ANNOUNCE: &str = "shorts.announce";
pub const JOB_COVER: &str = "shorts.cover";
pub const JOB_REINDEX: &str = "shorts.reindex";
