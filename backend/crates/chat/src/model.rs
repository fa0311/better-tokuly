//! Database rows of the chat domain and the numbers stored in them.
//!
//! Every query in the crate uses the same table aliases — `c` for
//! `chat_conversations`, `m` for `chat_members`, `x` for `chat_messages` —
//! so the column lists below can be dropped into any of them.
use serde::{Deserialize, Serialize};
use tokuly_core::prelude::*;
use tokuly_proto::chat as pb;

/// `tokuly.chat.v1.ConversationKind` as stored.
pub(crate) mod kind {
    pub const DIRECT: i16 = 1;
    pub const GROUP: i16 = 2;
}

/// `tokuly.chat.v1.MemberRole` as stored.
pub(crate) mod role {
    pub const MEMBER: i16 = 1;
    pub const ADMIN: i16 = 2;
    pub const OWNER: i16 = 3;
}

/// `tokuly.chat.v1.RequestState` as stored.
pub(crate) mod request {
    pub const ACCEPTED: i16 = 1;
    pub const PENDING: i16 = 2;
    pub const DECLINED: i16 = 3;
}

/// `tokuly.chat.v1.MessageKind` as stored.
pub(crate) mod msg {
    pub const TEXT: i16 = 1;
    pub const STAMP: i16 = 2;
    pub const MEDIA: i16 = 3;
    pub const LOCATION: i16 = 4;
    pub const HELLO: i16 = 5;
    pub const SYSTEM: i16 = 6;
    pub const CALL: i16 = 7;
    pub const POV_REQUEST: i16 = 8;
}

/// `tokuly.chat.v1.PovState` as stored.
pub(crate) mod pov {
    pub const OPEN: i16 = 1;
    pub const ANSWERED: i16 = 2;
    pub const DECLINED: i16 = 3;
    pub const CANCELLED: i16 = 4;
    pub const EXPIRED: i16 = 5;
}

/// `tokuly.chat.v1.CallState` as stored.
pub(crate) mod call {
    pub const RINGING: i16 = 1;
    pub const ACTIVE: i16 = 2;
    pub const ENDED: i16 = 3;
}

/// `tokuly.chat.v1.CallEndReason` as stored.
pub(crate) mod call_end {
    pub const COMPLETED: i16 = 1;
    pub const MISSED: i16 = 2;
    pub const DECLINED: i16 = 3;
    pub const CANCELLED: i16 = 4;
    pub const FAILED: i16 = 5;
}

/// `tokuly.chat.v1.CallParticipantState` as stored.
pub(crate) mod party {
    pub const RINGING: i16 = 1;
    pub const JOINED: i16 = 2;
    pub const DECLINED: i16 = 3;
    pub const LEFT: i16 = 4;
    pub const MISSED: i16 = 5;
}

pub(crate) const CONV: &str = "c.id, c.kind, c.user_lo, c.user_hi, c.name, c.icon, c.icon_asset_id, \
     c.member_count, c.last_seq, c.last_message_id, c.last_activity_at, c.read_only, c.created_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ConversationRow {
    pub id: Uuid,
    pub kind: i16,
    pub user_lo: Option<Uuid>,
    pub user_hi: Option<Uuid>,
    pub name: String,
    pub icon: Option<serde_json::Value>,
    pub icon_asset_id: Option<Uuid>,
    pub member_count: i32,
    pub last_seq: i64,
    pub last_message_id: Option<Uuid>,
    pub last_activity_at: Time,
    pub read_only: bool,
    pub created_at: Time,
}

impl ConversationRow {
    pub fn is_group(&self) -> bool {
        self.kind == kind::GROUP
    }

    pub fn is_direct(&self) -> bool {
        self.kind == kind::DIRECT
    }

    /// Direct conversations: the two users, whether or not both are still members.
    pub fn pair(&self) -> Option<(Uuid, Uuid)> {
        Some((self.user_lo?, self.user_hi?))
    }

    /// Direct conversations: the user on the other side of `user_id`.
    pub fn other_of(&self, user_id: Uuid) -> Option<Uuid> {
        let (lo, hi) = self.pair()?;
        if user_id == lo {
            Some(hi)
        } else if user_id == hi {
            Some(lo)
        } else {
            None
        }
    }
}

pub(crate) const MEMBER: &str = "m.conversation_id, m.user_id, m.role, m.request_state, m.request_messages, m.read_seq, \
     m.read_at, m.unread_count, m.unread_mention_count, m.history_from_seq, m.pinned_at, m.muted, m.muted_until, \
     m.archived_at, m.hidden_at, m.added_by, m.joined_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct MemberRow {
    pub conversation_id: Uuid,
    pub user_id: Uuid,
    pub role: i16,
    pub request_state: i16,
    pub request_messages: i32,
    pub read_seq: i64,
    pub read_at: Option<Time>,
    pub unread_count: i32,
    pub unread_mention_count: i32,
    pub history_from_seq: i64,
    pub pinned_at: Option<Time>,
    pub muted: bool,
    pub muted_until: Option<Time>,
    pub archived_at: Option<Time>,
    pub hidden_at: Option<Time>,
    pub added_by: Option<Uuid>,
    pub joined_at: Time,
}

impl MemberRow {
    pub fn is_accepted(&self) -> bool {
        self.request_state == request::ACCEPTED
    }

    /// Whether the mute is in force right now (a timed mute runs out by itself).
    pub fn muted_now(&self) -> bool {
        muted_now(self.muted, self.muted_until)
    }

    /// Owner or admin.
    pub fn manages(&self) -> bool {
        self.role >= role::ADMIN
    }
}

pub(crate) fn muted_now(muted: bool, muted_until: Option<Time>) -> bool {
    muted && muted_until.is_none_or(|until| until > now())
}

pub(crate) const MSG: &str = "x.id, x.conversation_id, x.seq, x.sender_id, x.kind, x.client_id, x.text, x.mentions, \
     x.mention_all, x.stamp_id, x.attachments, x.location, x.link_url, x.link_preview, x.link_checked_at, x.reply_to_id, \
     x.forwarded, x.system, x.call_id, x.pov_request_id, x.edited_at, x.deleted_at, x.created_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct MessageRow {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub seq: i64,
    pub sender_id: Option<Uuid>,
    pub kind: i16,
    pub client_id: Option<String>,
    pub text: String,
    pub mentions: serde_json::Value,
    pub mention_all: bool,
    pub stamp_id: Option<Uuid>,
    pub attachments: serde_json::Value,
    pub location: Option<serde_json::Value>,
    pub link_url: Option<String>,
    pub link_preview: Option<serde_json::Value>,
    pub link_checked_at: Option<Time>,
    pub reply_to_id: Option<Uuid>,
    pub forwarded: bool,
    pub system: Option<serde_json::Value>,
    pub call_id: Option<Uuid>,
    pub pov_request_id: Option<Uuid>,
    pub edited_at: Option<Time>,
    pub deleted_at: Option<Time>,
    pub created_at: Time,
}

impl MessageRow {
    pub fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }

    /// Whether the message counts towards other members' unread totals.
    pub fn counts_as_unread(&self) -> bool {
        self.sender_id.is_some() && self.kind != msg::SYSTEM && !self.is_deleted()
    }

    /// The media message that answers a POV request.
    pub fn is_pov_reply(&self) -> bool {
        self.kind == msg::MEDIA && self.pov_request_id.is_some()
    }

    pub fn mentions(&self) -> Vec<MentionRef> {
        serde_json::from_value(self.mentions.clone()).unwrap_or_default()
    }

    pub fn attachments(&self) -> Vec<tokuly_proto::media::Attachment> {
        serde_json::from_value(self.attachments.clone()).unwrap_or_default()
    }

    pub fn system(&self) -> Option<SystemPayload> {
        self.system.clone().and_then(|value| serde_json::from_value(value).ok())
    }
}

/// One `@handle` mention as stored in `chat_messages.mentions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MentionRef {
    pub user_id: Uuid,
    pub handle: String,
}

/// Content of a system message as stored in `chat_messages.system`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SystemPayload {
    /// `tokuly.chat.v1.SystemEventType`.
    pub r#type: i32,
    #[serde(default)]
    pub actor_id: Option<Uuid>,
    #[serde(default)]
    pub target_ids: Vec<Uuid>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub message_id: Option<Uuid>,
}

impl SystemPayload {
    pub fn new(event: pb::SystemEventType, actor_id: Option<Uuid>) -> Self {
        Self { r#type: event as i32, actor_id, ..Default::default() }
    }

    pub fn targets(mut self, target_ids: impl IntoIterator<Item = Uuid>) -> Self {
        self.target_ids = target_ids.into_iter().collect();
        self
    }

    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    pub fn message(mut self, message_id: Uuid) -> Self {
        self.message_id = Some(message_id);
        self
    }
}

pub(crate) const POV: &str = "p.id, p.conversation_id, p.requester_id, p.target_id, p.note, p.state, p.request_message_id, \
     p.reply_message_id, p.expires_at, p.answered_at, p.reply_expires_at, p.saved_at, p.reply_expired_at, p.created_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct PovRow {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub requester_id: Uuid,
    pub target_id: Uuid,
    pub note: String,
    pub state: i16,
    pub request_message_id: Option<Uuid>,
    pub reply_message_id: Option<Uuid>,
    pub expires_at: Time,
    pub answered_at: Option<Time>,
    pub reply_expires_at: Option<Time>,
    pub saved_at: Option<Time>,
    pub reply_expired_at: Option<Time>,
    pub created_at: Time,
}

impl PovRow {
    pub fn to_proto(&self) -> pb::PovRequest {
        pb::PovRequest {
            id: self.id.to_string(),
            conversation_id: self.conversation_id.to_string(),
            requester_id: self.requester_id.to_string(),
            target_id: self.target_id.to_string(),
            note: self.note.clone(),
            state: i32::from(self.state).into(),
            expires_at: ts(self.expires_at),
            request_message_id: self.request_message_id.map(|id| id.to_string()).unwrap_or_default(),
            reply_message_id: self.reply_message_id.map(|id| id.to_string()).unwrap_or_default(),
            created_at: ts(self.created_at),
            answered_at: opt_ts(self.answered_at),
            saved: self.saved_at.is_some(),
            reply_expires_at: opt_ts(self.reply_expires_at.filter(|_| self.saved_at.is_none())),
            reply_expired: self.reply_expired_at.is_some(),
            ..Default::default()
        }
    }

    /// How the answering media message describes its link to this request.
    pub fn reply_proto(&self) -> pb::PovReply {
        pb::PovReply {
            request_id: self.id.to_string(),
            expires_at: opt_ts(self.reply_expires_at.filter(|_| self.saved_at.is_none() && self.reply_expired_at.is_none())),
            saved: self.saved_at.is_some(),
            expired: self.reply_expired_at.is_some(),
            ..Default::default()
        }
    }
}

pub(crate) const CALL: &str = "k.id, k.conversation_id, k.started_by, k.video, k.state, k.end_reason, k.room, k.message_id, \
     k.ring_expires_at, k.answered_at, k.ended_at, k.created_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct CallRow {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub started_by: Option<Uuid>,
    pub video: bool,
    pub state: i16,
    pub end_reason: i16,
    pub room: String,
    pub message_id: Option<Uuid>,
    pub ring_expires_at: Time,
    pub answered_at: Option<Time>,
    pub ended_at: Option<Time>,
    pub created_at: Time,
}

impl CallRow {
    pub fn is_live(&self) -> bool {
        self.state != call::ENDED
    }

    /// Seconds people talked: from the first answer to the end.
    pub fn duration_seconds(&self) -> i32 {
        match (self.answered_at, self.ended_at) {
            (Some(answered), Some(ended)) => (ended - answered).num_seconds().clamp(0, i64::from(i32::MAX)) as i32,
            _ => 0,
        }
    }

    /// The call as shown in the conversation timeline.
    pub fn summary(&self) -> pb::CallSummary {
        pb::CallSummary {
            call_id: self.id.to_string(),
            video: self.video,
            state: i32::from(self.state).into(),
            end_reason: i32::from(self.end_reason).into(),
            duration_seconds: self.duration_seconds(),
            started_by_id: self.started_by.map(|id| id.to_string()).unwrap_or_default(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ParticipantRow {
    pub call_id: Uuid,
    pub user_id: Uuid,
    pub state: i16,
    pub joined_at: Option<Time>,
    pub left_at: Option<Time>,
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    #[test]
    fn timed_mutes_run_out() {
        assert!(!muted_now(false, None));
        assert!(muted_now(true, None));
        assert!(muted_now(true, Some(now() + Duration::minutes(5))));
        assert!(!muted_now(true, Some(now() - Duration::seconds(1))));
    }

    #[test]
    fn call_duration_runs_from_answer_to_end() {
        let started = now();
        let mut call = CallRow {
            id: new_id(),
            conversation_id: new_id(),
            started_by: Some(new_id()),
            video: false,
            state: call::ENDED,
            end_reason: call_end::MISSED,
            room: "room".into(),
            message_id: None,
            ring_expires_at: started,
            answered_at: None,
            ended_at: Some(started + Duration::seconds(40)),
            created_at: started,
        };
        assert_eq!(call.duration_seconds(), 0, "a call nobody answered has no duration");
        call.answered_at = Some(started + Duration::seconds(10));
        assert_eq!(call.duration_seconds(), 30);
        assert_eq!(call.summary().duration_seconds, 30);
    }

    #[test]
    fn system_payloads_roundtrip_through_json() {
        let actor = new_id();
        let target = new_id();
        let payload = SystemPayload::new(pb::SystemEventType::SYSTEM_EVENT_TYPE_MEMBERS_ADDED, Some(actor)).targets([target]);
        let json = serde_json::to_value(&payload).unwrap();
        let back: SystemPayload = serde_json::from_value(json).unwrap();
        assert_eq!(back.r#type, pb::SystemEventType::SYSTEM_EVENT_TYPE_MEMBERS_ADDED as i32);
        assert_eq!(back.actor_id, Some(actor));
        assert_eq!(back.target_ids, vec![target]);
    }
}
