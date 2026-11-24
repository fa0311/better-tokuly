//! Domain events of the notes domain (see `docs/events/notes.md`).
//!
//! Consumers deserialize the emitted payloads with
//! [`tokuly_core::events::Event::data`]. Every event is written in the
//! transaction that makes the change it describes.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── emitted ──

pub const NOTE_CREATED: &str = "notes.note.created";
pub const NOTE_DELETED: &str = "notes.note.deleted";
pub const NOTE_VISIBILITY_CHANGED: &str = "notes.note.visibility_changed";
pub const STORY_CREATED: &str = "notes.story.created";
pub const STORY_DELETED: &str = "notes.story.deleted";
pub const STORY_EXPIRED: &str = "notes.story.expired";

/// `notes.note.created`: a note, reply or quote note was posted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteCreated {
    pub note_id: Uuid,
    pub author_id: Uuid,
    /// `tokuly.common.v1.Visibility` as its number (1 public, 4 friends, 5
    /// private or direct, 6 followers, 7 close friends, 8 circle).
    pub visibility: i32,
    pub reply_to_id: Option<Uuid>,
    pub quote_of_id: Option<Uuid>,
}

/// Why a note or a story went away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemovalReason {
    /// Its author deleted it.
    Author,
    /// A moderator took it down (`admin.content.removed`).
    Moderation,
    /// The author's account was erased.
    AccountDeleted,
    /// Its only content was an upload that was deleted by force.
    MediaDeleted,
}

/// `notes.note.deleted`: a note is gone for good.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteDeleted {
    pub note_id: Uuid,
    pub author_id: Uuid,
    pub reason: RemovalReason,
}

/// `notes.note.visibility_changed`: the author changed who may see a note.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteVisibilityChanged {
    pub note_id: Uuid,
    pub author_id: Uuid,
    /// The `tokuly.common.v1.Visibility` now in effect: the note's own, except
    /// that a public note of a locked account reads 6 (followers).
    pub visibility: i32,
    /// Who besides the author may still see the note: `None` when everyone
    /// may (public, author not locked); otherwise every one of them, as the
    /// social graph's audience gives them (followers and friends, friends,
    /// close friends, circle members, the people a direct note names), plus
    /// for a reply the author of the note answered, minus anyone in a block
    /// with the author; empty for a private note. When the change comes
    /// from the author locking their account, only the people this domain
    /// told about the note (or who reacted to it) are considered, since a
    /// follower list per note would be unbounded. Whoever was shown the note
    /// before (notifications, inboxes) and is not listed lost access with
    /// this change.
    pub visible_to: Option<Vec<Uuid>>,
    /// When the change was made, taken while the note was locked: two
    /// changes of one note are ordered by this (the event's own time is when
    /// its transaction began, which can put racing changes the wrong way round).
    pub changed_at: DateTime<Utc>,
}

/// `notes.story.created`: a story was posted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryCreated {
    pub story_id: Uuid,
    pub owner_id: Uuid,
    /// `tokuly.notes.v1.StoryAudience` as its number (1 friends, 2 close friends).
    pub audience: i32,
    pub expires_at: DateTime<Utc>,
}

/// `notes.story.deleted`: a story is gone for good (not merely expired).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryDeleted {
    pub story_id: Uuid,
    pub owner_id: Uuid,
    pub reason: RemovalReason,
}

/// `notes.story.expired`: a story left the tray and is archive-only now.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryExpired {
    pub story_id: Uuid,
    pub owner_id: Uuid,
}

// ── consumed ──

pub const USER_DELETED: &str = "identity.user.deleted";
pub const USER_BLOCKED: &str = "social.user.blocked";
pub const FOLLOW_SETTINGS_CHANGED: &str = "social.follow.settings_changed";
pub const ASSET_DELETED: &str = "media.asset.deleted";
pub const CONTENT_REMOVED: &str = "admin.content.removed";

/// `media.asset.deleted`: an upload and its files are gone. Only the field
/// this domain needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetDeleted {
    pub asset_id: Uuid,
}

/// `admin.content.removed`: a moderator took content down. This domain acts
/// on notes and stories and ignores everything else.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentRemoved {
    /// `tokuly.common.v1.SubjectType` as its number.
    pub subject_type: i32,
    pub subject_id: Uuid,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub moderator_id: Option<Uuid>,
}
