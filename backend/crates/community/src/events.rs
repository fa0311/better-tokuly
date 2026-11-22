//! Events the community domain emits, and the parts it reads of the events
//! it consumes.
//!
//! | Topic | Payload | When |
//! |---|---|---|
//! | `community.created` | [`CommunityChanged`] | a community was created |
//! | `community.updated` | [`CommunityChanged`] | its profile, visibility or owner changed |
//! | `community.deleted` | [`CommunityChanged`] | it was closed |
//! | `community.member.joined` | [`MemberChanged`] | someone became a member |
//! | `community.member.left` | [`MemberChanged`] | someone left, was removed or banned |
//! | `community.thread.created` | [`PostCreated`] | a thread entered public view |
//! | `community.reply.created` | [`PostCreated`] | a reply entered public view |
//! | `community.thread.deleted` | [`PostDeleted`] | a thread was deleted or taken down |
//! | `community.event.created` | [`EventChanged`] | an event was put on the calendar |
//! | `community.event.attended` | [`EventAttended`] | a member checked in at an event |
//!
//! **Anonymous posts never name their author on the bus**: `author_id` is
//! `None` and the event has no actor, so no consumer (activity feeds,
//! webhooks) can unmask anyone by accident.
use serde::{Deserialize, Serialize};
use tokuly_core::prelude::*;

pub const COMMUNITY_CREATED: &str = "community.created";
pub const COMMUNITY_UPDATED: &str = "community.updated";
pub const COMMUNITY_DELETED: &str = "community.deleted";
pub const MEMBER_JOINED: &str = "community.member.joined";
pub const MEMBER_LEFT: &str = "community.member.left";
pub const THREAD_CREATED: &str = "community.thread.created";
pub const REPLY_CREATED: &str = "community.reply.created";
pub const THREAD_DELETED: &str = "community.thread.deleted";
pub const EVENT_CREATED: &str = "community.event.created";
pub const EVENT_ATTENDED: &str = "community.event.attended";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityChanged {
    pub community_id: Uuid,
    pub owner_id: Uuid,
    /// `tokuly.community.v1.CommunityVisibility` as its number.
    pub visibility: i32,
    /// `tokuly.community.v1.CommunityCategory` as its number.
    pub category: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberChanged {
    pub community_id: Uuid,
    pub user_id: Uuid,
    /// The community is public (its memberships may be talked about).
    pub public: bool,
    /// For `community.member.left`: `left`, `removed`, `banned` or
    /// `account_deleted`. Empty for joins.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cause: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostCreated {
    pub community_id: Uuid,
    pub channel_id: Uuid,
    pub thread_id: Uuid,
    /// Set for replies.
    pub reply_id: Option<Uuid>,
    /// `None` for posts in anonymous channels.
    pub author_id: Option<Uuid>,
    pub anonymous: bool,
    /// The community is public.
    pub public: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostDeleted {
    pub community_id: Uuid,
    pub thread_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventChanged {
    pub community_id: Uuid,
    pub event_id: Uuid,
    pub organizer_id: Uuid,
    pub public: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventAttended {
    pub community_id: Uuid,
    pub event_id: Uuid,
    pub user_id: Uuid,
    pub public: bool,
}

// ───────────────────────────── consumed ─────────────────────────────

/// `identity.user.deleted`.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct UserDeleted {
    pub user_id: Uuid,
}

/// `engagement.report.created`, as far as this domain reads it.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ReportCreated {
    pub report_id: Uuid,
    pub reporter_id: Uuid,
    pub subject_type: i32,
    pub subject_id: Uuid,
}

/// `admin.content.removed`.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ContentRemoved {
    pub subject_type: i32,
    pub subject_id: Uuid,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub moderator_id: Option<Uuid>,
}

/// `media.asset.deleted`.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AssetDeleted {
    pub asset_id: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anonymous_posts_carry_no_author() {
        let event = PostCreated {
            community_id: Uuid::nil(),
            channel_id: Uuid::nil(),
            thread_id: Uuid::nil(),
            reply_id: None,
            author_id: None,
            anonymous: true,
            public: false,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert!(json["author_id"].is_null());
        assert_eq!(json["anonymous"], true);
    }

    #[test]
    fn consumed_payloads_tolerate_extra_and_missing_fields() {
        let report: ReportCreated = serde_json::from_value(serde_json::json!({
            "report_id": Uuid::nil(), "reporter_id": Uuid::nil(), "subject_type": 22, "subject_id": Uuid::nil(),
            "subject_owner_id": null, "category": "SPAM", "text": "", "subject_title": "t", "subject_path": "/p",
            "something_new": 1
        }))
        .unwrap();
        assert_eq!(report.subject_type, 22);
        let removed: ContentRemoved =
            serde_json::from_value(serde_json::json!({ "subject_type": 23, "subject_id": Uuid::nil() })).unwrap();
        assert!(removed.reason.is_empty() && removed.moderator_id.is_none());
    }
}
