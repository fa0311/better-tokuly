//! Domain events of the chat domain (see `docs/events/chat.md`).
//!
//! Emitted events are written in the same transaction as the change they
//! describe; consumers deserialize the payloads with
//! [`tokuly_core::events::Event::data`]. Sending a message emits no domain
//! event (it happens far too often): messages are counted through analytics
//! (`chat.message.send`).
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── emitted ──

pub const CONVERSATION_CREATED: &str = "chat.conversation.created";
pub const MEMBER_JOINED: &str = "chat.member.joined";
pub const MEMBER_LEFT: &str = "chat.member.left";
pub const MESSAGE_DELETED: &str = "chat.message.deleted";
pub const CALL_STARTED: &str = "chat.call.started";
pub const CALL_ENDED: &str = "chat.call.ended";

/// `chat.conversation.created`: a direct conversation was opened or a group
/// was created. The actor is the user who did it (nobody for conversations
/// opened by the server).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationCreated {
    pub conversation_id: Uuid,
    /// `tokuly.chat.v1.ConversationKind` as its number.
    pub kind: i32,
    pub member_ids: Vec<Uuid>,
}

/// `chat.member.joined`: a user was added to a group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberJoined {
    pub conversation_id: Uuid,
    pub user_id: Uuid,
    pub added_by: Uuid,
}

/// `chat.member.left`: a user left a group (`removed_by` unset), was removed
/// from it, or lost the membership with their account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberLeft {
    pub conversation_id: Uuid,
    pub user_id: Uuid,
    pub removed_by: Option<Uuid>,
}

/// `chat.message.deleted`: a message was deleted for everyone, by its sender,
/// a group admin (`deleted_by`) or moderation (`moderated`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageDeleted {
    pub message_id: Uuid,
    pub conversation_id: Uuid,
    pub sender_id: Option<Uuid>,
    pub deleted_by: Option<Uuid>,
    pub moderated: bool,
}

/// `chat.call.started`: a call began ringing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallStarted {
    pub call_id: Uuid,
    pub conversation_id: Uuid,
    pub started_by: Uuid,
    pub video: bool,
}

/// `chat.call.ended`: a call ended.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallEnded {
    pub call_id: Uuid,
    pub conversation_id: Uuid,
    /// `tokuly.chat.v1.CallEndReason` as its number.
    pub end_reason: i32,
    pub duration_seconds: i32,
    /// Everyone who was connected at some point.
    pub participant_ids: Vec<Uuid>,
}

/// `chat.assist.provider_updated`: an admin saved an AI provider's
/// operator settings (the actor). The record of who changed what; the key
/// itself is never in it.
pub const ASSIST_PROVIDER_UPDATED: &str = "chat.assist.provider_updated";
/// `chat.assist.default_changed`: an admin chose the default provider.
pub const ASSIST_DEFAULT_CHANGED: &str = "chat.assist.default_changed";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistProviderUpdated {
    /// `tokuly.chat.v1.AiProvider` as its number.
    pub provider: i32,
    pub enabled: bool,
    pub model: String,
    pub base_url: String,
    /// `kept`, `set` or `cleared`.
    pub api_key: String,
    /// The outbound guard's exception for the base URL's host.
    pub allow_private_host: bool,
    /// The host the exception covers, when it is on.
    pub private_host: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistDefaultChanged {
    /// `tokuly.chat.v1.AiProvider` as its number; 0: the first enabled one.
    pub provider: i32,
}

// ── consumed ──

pub const USER_CREATED: &str = "identity.user.created";
pub const BIRTHDATE_GIVEN: &str = "identity.user.birthdate_given";
pub const USER_DELETED: &str = "identity.user.deleted";
pub const USER_BLOCKED: &str = "social.user.blocked";
pub const USER_UNBLOCKED: &str = "social.user.unblocked";
pub const FRIEND_ACCEPTED: &str = "social.friend.accepted";
pub const ASSET_READY: &str = "media.asset.ready";
pub const ASSET_FAILED: &str = "media.asset.failed";
pub const ASSET_DELETED: &str = "media.asset.deleted";
pub const CONTENT_REMOVED: &str = "admin.content.removed";

/// `media.asset.*`: only the field the chat domain needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetRef {
    pub asset_id: Uuid,
}

/// `admin.content.removed`: a moderator took content down. The chat domain
/// acts on chat messages and ignores everything else.
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
