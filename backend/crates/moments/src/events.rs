//! Domain events of the moments domain (see `docs/events/moments.md`).
//!
//! Consumers deserialize the emitted payloads with
//! [`tokuly_core::events::Event::data`]. Every event is written in the
//! transaction that makes the change it describes. None carries a picture,
//! a caption or a location.
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── emitted ──

pub const PROMPT_SENT: &str = "moments.prompt.sent";
pub const MOMENT_POSTED: &str = "moments.moment.posted";
pub const MOMENT_DELETED: &str = "moments.moment.deleted";

/// `moments.prompt.sent`: a region's prompt went out; its day began. Once
/// per region and date. No actor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptSent {
    pub day_id: Uuid,
    /// The IANA time zone.
    pub region: String,
    pub date: NaiveDate,
    pub prompt_at: DateTime<Utc>,
}

/// `moments.moment.posted`: someone posted their moment of a day. Actor:
/// the owner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MomentPosted {
    pub moment_id: Uuid,
    pub owner_id: Uuid,
    pub day_id: Uuid,
    pub date: NaiveDate,
    /// 0 on time.
    pub late_seconds: i64,
    /// `tokuly.moments.v1.MomentAudience` as its number.
    pub audience: i32,
}

/// `moments.moment.deleted`: a moment was deleted (`reason`: `owner`,
/// `moderation`, `media_deleted`, `account_deleted`). Actor: whoever did it
/// (none for the last two).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MomentDeleted {
    pub moment_id: Uuid,
    pub owner_id: Uuid,
    pub reason: String,
}

// ── consumed ──

pub const USER_CREATED: &str = "identity.user.created";
pub const BIRTHDATE_GIVEN: &str = "identity.user.birthdate_given";
pub const USER_DELETED: &str = "identity.user.deleted";
pub const CONTENT_REMOVED: &str = "admin.content.removed";
pub const ASSET_DELETED: &str = "media.asset.deleted";

/// `identity.user.created` / `identity.user.birthdate_given`: the age group
/// the account's defaults are decided by (`None`: an old event; counted as
/// a minor).
#[derive(Debug, Clone, Deserialize)]
pub struct AgeGroupKnown {
    pub user_id: Uuid,
    #[serde(default)]
    pub minor: Option<bool>,
}

/// `identity.user.deleted`.
#[derive(Debug, Clone, Deserialize)]
pub struct UserDeleted {
    pub user_id: Uuid,
}

/// `admin.content.removed`: a moderator took something down.
#[derive(Debug, Clone, Deserialize)]
pub struct ContentRemoved {
    /// `tokuly.common.v1.SubjectType` as its number.
    pub subject_type: i32,
    pub subject_id: Uuid,
    #[serde(default)]
    pub moderator_id: Option<Uuid>,
}

/// `media.asset.deleted`: an upload is gone, whoever held it.
#[derive(Debug, Clone, Deserialize)]
pub struct AssetDeleted {
    pub asset_id: Uuid,
}
