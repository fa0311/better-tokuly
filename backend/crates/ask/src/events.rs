//! Domain events of the ask domain (see `docs/events/ask.md`).
//!
//! Two groups:
//! - **public** events other domains may consume: an answer was published or
//!   removed, a prompt opened, a round was completed. None of them says who
//!   sent a question, a response or a vote.
//! - **delivery** events (`ask.*.stored`): this domain's own queue. Sending
//!   something only stores it and one of these; the worker then tells the
//!   recipient, or does nothing when the item was dropped or held. That keeps
//!   the request of a blocked sender identical to everyone else's. They are
//!   emitted without an actor and carry nothing but the ID. Other domains
//!   must not consume them: they fire for dropped items too.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const ANSWER_PUBLISHED: &str = "ask.answer.published";
pub const ANSWER_DELETED: &str = "ask.answer.deleted";
pub const PROMPT_OPENED: &str = "ask.prompt.opened";
pub const ROUND_COMPLETED: &str = "ask.round.completed";

pub const QUESTION_STORED: &str = "ask.question.stored";
pub const RESPONSE_STORED: &str = "ask.response.stored";
pub const FLAME_STORED: &str = "ask.flame.stored";

/// `ask.answer.published`: the owner of a box answered a question. Also
/// emitted when an edit changes the audience. Actor: the owner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnswerPublished {
    pub question_id: Uuid,
    pub owner_id: Uuid,
    /// `tokuly.ask.v1.AnswerAudience` as its number.
    pub audience: i32,
    /// True for the answer itself, false for a later change of audience.
    pub first: bool,
}

/// `ask.answer.deleted`: an answer was removed (by its owner, with the
/// question, or by moderation). Actor: the owner, or nobody.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnswerDeleted {
    pub question_id: Uuid,
    pub owner_id: Uuid,
}

/// `ask.prompt.opened`: a prompt was posted to the owner's friends. Actor:
/// the owner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptOpened {
    pub prompt_id: Uuid,
    pub owner_id: Uuid,
    pub closes_at: DateTime<Utc>,
}

/// `ask.round.completed`: a user finished a round of compliment polls.
/// Actor: the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoundCompleted {
    pub user_id: Uuid,
    pub round_id: Uuid,
    /// Polls voted on.
    pub votes: i32,
    /// Polls skipped.
    pub skipped: i32,
    /// Coins granted for the round.
    pub coins: i64,
}

/// `ask.question.stored`, `ask.response.stored`, `ask.flame.stored`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stored {
    pub id: Uuid,
}

// ───────────────────────────── consumed ─────────────────────────────

pub(crate) const USER_DELETED: &str = "identity.user.deleted";
pub(crate) const USER_BANNED: &str = "identity.user.banned";
pub(crate) const USER_UNBANNED: &str = "identity.user.unbanned";
pub(crate) const USER_BLOCKED: &str = tokuly_social::events::USER_BLOCKED;
pub(crate) const FRIEND_ACCEPTED: &str = tokuly_social::events::FRIEND_ACCEPTED;
pub(crate) const FRIEND_REMOVED: &str = tokuly_social::events::FRIEND_REMOVED;
pub(crate) const FRIEND_PREFS_CHANGED: &str = tokuly_social::events::FRIEND_PREFS_CHANGED;
pub(crate) const CONTENT_REMOVED: &str = "admin.content.removed";
pub(crate) const ASSET_DELETED: &str = "media.asset.deleted";

/// `admin.content.removed`, as far as this domain reads it.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ContentRemoved {
    #[serde(default)]
    pub subject_type: i32,
    pub subject_id: Uuid,
}

/// `media.asset.deleted`, as far as this domain reads it.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AssetDeleted {
    pub asset_id: Uuid,
}

/// The topics the `ask` consumer subscribes to.
pub(crate) const CONSUMED: &[&str] = &[
    QUESTION_STORED,
    RESPONSE_STORED,
    FLAME_STORED,
    ANSWER_PUBLISHED,
    PROMPT_OPENED,
    USER_DELETED,
    USER_BANNED,
    USER_UNBANNED,
    USER_BLOCKED,
    FRIEND_ACCEPTED,
    FRIEND_REMOVED,
    FRIEND_PREFS_CHANGED,
    CONTENT_REMOVED,
    ASSET_DELETED,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreign_payloads_are_read_tolerantly() {
        let id = Uuid::now_v7();
        let removed: ContentRemoved =
            serde_json::from_value(serde_json::json!({ "subject_type": 18, "subject_id": id, "reason": "x", "moderator_id": id, "new_field": 1 })).unwrap();
        assert_eq!((removed.subject_type, removed.subject_id), (18, id));
        let asset: AssetDeleted =
            serde_json::from_value(serde_json::json!({ "asset_id": id, "owner_id": id, "purpose": 8, "kind": 1 }))
                .unwrap();
        assert_eq!(asset.asset_id, id);
    }

    #[test]
    fn delivery_events_carry_only_an_id() {
        let stored = serde_json::to_value(Stored { id: Uuid::nil() }).unwrap();
        assert_eq!(stored.as_object().unwrap().len(), 1);
    }
}
