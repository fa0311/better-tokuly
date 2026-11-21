//! Event payloads of the search domain (see `docs/events/search.md`).
//!
//! Consumed:
//! - `search.changed` — `tokuly_core::outbound::SearchChange`
//! - `identity.user.created` / `updated` / `privacy_changed` / `banned` /
//!   `unbanned` / `deleted` — [`UserRef`]
//! - `admin.content.removed` — [`ContentRemoved`]
//!
//! Emitted:
//! - `search.reindex.requested` — [`ReindexRequested`]
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const USER_CREATED_TOPIC: &str = "identity.user.created";
pub const USER_UPDATED_TOPIC: &str = "identity.user.updated";
pub const USER_PRIVACY_CHANGED_TOPIC: &str = "identity.user.privacy_changed";
pub const USER_BANNED_TOPIC: &str = "identity.user.banned";
pub const USER_UNBANNED_TOPIC: &str = "identity.user.unbanned";
pub const USER_DELETED_TOPIC: &str = "identity.user.deleted";
pub const CONTENT_REMOVED_TOPIC: &str = "admin.content.removed";
pub const REINDEX_REQUESTED_TOPIC: &str = "search.reindex.requested";

/// Our view of identity's user events: only the user matters, the document
/// is always rebuilt from the account as it is now.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRef {
    pub user_id: Uuid,
}

/// Our view of the admin domain's takedown event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentRemoved {
    /// `tokuly.common.v1.SubjectType` as its number.
    pub subject_type: i32,
    pub subject_id: Uuid,
}

/// An administrator asked for one kind to be rebuilt. The domain that owns
/// the kind answers by publishing every public document of that kind again
/// with `tokuly_core::outbound::index` (unchanged documents cost nothing:
/// the registry notices and does not write them to the engine again).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReindexRequested {
    /// `SearchDoc::kind`, e.g. `video`.
    pub kind: String,
    /// The administrator who asked.
    pub requested_by: Option<Uuid>,
}
