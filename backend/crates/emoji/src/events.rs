//! Event topics and payloads of the emoji domain (see `docs/events/emoji.md`).
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── emitted ──

/// An emoji entered the review queue (a new submission, or a rejected one
/// submitted again). Payload: [`EmojiRef`].
pub const EMOJI_SUBMITTED: &str = "emoji.emoji.submitted";
/// A moderator declined an emoji. Payload: [`EmojiDecision`].
pub const EMOJI_REJECTED: &str = "emoji.emoji.rejected";
/// An emoji joined the catalogue. Payload: [`EmojiAdded`].
pub const EMOJI_ADDED: &str = "emoji.emoji.added";
/// An emoji in the catalogue changed. Payload: [`EmojiUpdated`].
pub const EMOJI_UPDATED: &str = "emoji.emoji.updated";
/// An emoji left the catalogue. Payload: [`EmojiRemoved`].
pub const EMOJI_REMOVED: &str = "emoji.emoji.removed";
/// A moderator changed a submitter's trusted flag. Payload: [`SubmitterTrust`].
pub const SUBMITTER_TRUST_CHANGED: &str = "emoji.submitter.trust_changed";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmojiRef {
    pub emoji_id: Uuid,
    pub name: String,
    /// The credited user; `None` for imports and uncredited additions.
    pub author_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmojiDecision {
    pub emoji_id: Uuid,
    pub name: String,
    pub author_id: Option<Uuid>,
    pub moderator_id: Uuid,
    pub reason: String,
}

/// How an emoji got into the catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddedVia {
    /// A moderator approved a submission.
    Review,
    /// Submitted by a trusted submitter or a moderator: no review.
    Trusted,
    /// A moderator added it directly.
    Staff,
    /// It came in with an imported pack.
    Import,
    /// A moderator enabled a disabled emoji again.
    Enabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmojiAdded {
    pub emoji_id: Uuid,
    pub name: String,
    pub author_id: Option<Uuid>,
    pub via: AddedVia,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmojiUpdated {
    pub emoji_id: Uuid,
    pub name: String,
    /// The name it had before, when this change renamed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_name: Option<String>,
}

/// Why an emoji left the catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemovedBecause {
    /// A moderator disabled it; it may come back.
    Disabled,
    /// A report against it was upheld (`admin.content.removed`); disabled.
    TakenDown,
    /// A moderator deleted it.
    Deleted,
    /// Its upload was deleted by force; the emoji is gone with it.
    AssetDeleted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmojiRemoved {
    pub emoji_id: Uuid,
    pub name: String,
    pub author_id: Option<Uuid>,
    pub because: RemovedBecause,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitterTrust {
    pub user_id: Uuid,
    pub trusted: bool,
    pub moderator_id: Uuid,
}

// ── consumed ──

pub const USER_DELETED: &str = "identity.user.deleted";
pub const ASSET_READY: &str = "media.asset.ready";
pub const ASSET_FAILED: &str = "media.asset.failed";
pub const ASSET_DELETED: &str = "media.asset.deleted";
pub const CONTENT_REMOVED: &str = "admin.content.removed";

/// `media.asset.*`: the fields the emoji domain reads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetEvent {
    pub asset_id: Uuid,
    /// `tokuly.media.v1.AssetPurpose`; 0 when the producer did not say.
    #[serde(default)]
    pub purpose: i32,
}

/// `admin.content.removed`: a moderator took content down. The emoji domain
/// acts on custom emoji and ignores everything else.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentRemoved {
    /// `tokuly.common.v1.SubjectType`.
    pub subject_type: i32,
    pub subject_id: Uuid,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub moderator_id: Option<Uuid>,
}

// ── jobs ──

/// Reads an uploaded pack and starts importing its entries. Payload: [`ImportJob`].
pub const JOB_IMPORT: &str = "emoji.import";
/// Writes the catalogue to a ZIP. Payload: [`ExportJob`].
pub const JOB_EXPORT: &str = "emoji.export";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportJob {
    pub import_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportJob {
    pub export_id: Uuid,
}
