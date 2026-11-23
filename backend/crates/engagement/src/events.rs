//! Event topics and payloads of the engagement domain (see
//! `docs/events/engagement.md`).
//!
//! Everything emitted here is written in the transaction of the change it
//! describes. `subject_type` is always `tokuly.common.v1.SubjectType` as its
//! number and `subject_id` the subject's UUID.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── emitted ──

/// A user reported a subject. Payload: [`ReportCreated`]. Consumed by the
/// admin domain; the shape is a contract with it.
pub const REPORT_CREATED: &str = "engagement.report.created";
/// A comment entered public view: written, or approved. Payload: [`CommentEvent`].
pub const COMMENT_PUBLISHED: &str = "engagement.comment.published";
/// A comment was deleted (by its author, the subject owner or moderation).
/// Payload: [`CommentEvent`]. Not emitted for the replies deleted with it,
/// nor for comments purged with their subject or their author's account.
pub const COMMENT_DELETED: &str = "engagement.comment.deleted";
/// A user set, changed or removed their reaction. Payload: [`ReactionChanged`].
pub const REACTION_CHANGED: &str = "engagement.reaction.changed";
/// A user saved a subject or removed it from everything they saved.
/// Payload: [`BookmarkChanged`].
pub const BOOKMARK_CHANGED: &str = "engagement.bookmark.changed";
/// A subject was shared. Payload: [`ShareRecorded`].
pub const SHARE_RECORDED: &str = "engagement.share.recorded";

/// The report categories as they travel in [`ReportCreated::category`].
pub const REPORT_CATEGORIES: [&str; 10] =
    ["SPAM", "HARASSMENT", "HATE", "SEXUAL", "VIOLENCE", "SELF_HARM", "ILLEGAL", "IMPERSONATION", "COPYRIGHT", "OTHER"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportCreated {
    pub report_id: Uuid,
    pub reporter_id: Uuid,
    pub subject_type: i32,
    pub subject_id: Uuid,
    /// `null` for content that belongs to nobody (official content, places).
    pub subject_owner_id: Option<Uuid>,
    /// One of [`REPORT_CATEGORIES`].
    pub category: String,
    pub text: String,
    pub subject_title: String,
    pub subject_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentEvent {
    pub comment_id: Uuid,
    pub subject_type: i32,
    pub subject_id: Uuid,
    pub subject_owner_id: Uuid,
    /// The account that wrote it, also for comments written as a channel.
    pub author_id: Uuid,
    pub channel_id: Option<Uuid>,
    /// The top-level comment, for replies.
    pub parent_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactionChanged {
    pub user_id: Uuid,
    pub subject_type: i32,
    pub subject_id: Uuid,
    /// `None` when the subject could no longer be resolved (a removal by UUID).
    pub subject_owner_id: Option<Uuid>,
    /// `tokuly.engagement.v1.ReactionKind` as its number; 0 = no reaction.
    pub kind: i32,
    pub emoji: String,
    /// What the user's reaction was before; 0 = none.
    pub previous_kind: i32,
    pub previous_emoji: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkChanged {
    pub user_id: Uuid,
    pub subject_type: i32,
    pub subject_id: Uuid,
    /// Whether the user has the subject saved now.
    pub saved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareRecorded {
    /// `None` for guests.
    pub user_id: Option<Uuid>,
    pub subject_type: i32,
    pub subject_id: Uuid,
    pub subject_owner_id: Uuid,
    /// `tokuly.engagement.v1.ShareTarget` as its number.
    pub target: i32,
}

// ── consumed ──

pub const USER_DELETED: &str = "identity.user.deleted";
pub const USER_BLOCKED: &str = "social.user.blocked";
pub const FRIEND_REMOVED: &str = "social.friend.removed";
pub const ASSET_DELETED: &str = "media.asset.deleted";
pub const CONTENT_REMOVED: &str = "admin.content.removed";
/// A moderator resolved the case the reports were filed into. Payload:
/// [`ReportResolved`].
pub const REPORT_RESOLVED: &str = "admin.report.resolved";

/// `admin.content.removed`: a moderator took content down.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentRemoved {
    pub subject_type: i32,
    pub subject_id: Uuid,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub moderator_id: Option<Uuid>,
}

/// `media.asset.deleted`: an upload and its files are gone. Only the field
/// this domain needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetDeleted {
    pub asset_id: Uuid,
}

/// `admin.report.resolved`: the moderation team decided about reports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportResolved {
    /// `engagement.report.created` report IDs covered by the decision.
    pub report_ids: Vec<Uuid>,
    /// Whether action was taken (content removed, account warned or locked).
    pub action_taken: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_payload_has_the_shape_the_admin_domain_reads() {
        let (report, reporter, subject, owner) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let event = ReportCreated {
            report_id: report,
            reporter_id: reporter,
            subject_type: 3,
            subject_id: subject,
            subject_owner_id: Some(owner),
            category: "SPAM".into(),
            text: "宣伝です".into(),
            subject_title: "今日のランチ".into(),
            subject_path: "/notes/abc".into(),
        };
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            serde_json::json!({
                "report_id": report,
                "reporter_id": reporter,
                "subject_type": 3,
                "subject_id": subject,
                "subject_owner_id": owner,
                "category": "SPAM",
                "text": "宣伝です",
                "subject_title": "今日のランチ",
                "subject_path": "/notes/abc",
            })
        );
        let ownerless = ReportCreated { subject_owner_id: None, ..event };
        assert!(serde_json::to_value(&ownerless).unwrap()["subject_owner_id"].is_null());
    }

    #[test]
    fn content_removed_reads_the_admin_payload() {
        let (subject, moderator) = (Uuid::now_v7(), Uuid::now_v7());
        let removed: ContentRemoved = serde_json::from_value(
            serde_json::json!({ "subject_type": 5, "subject_id": subject, "reason": "規約違反", "moderator_id": moderator }),
        )
        .unwrap();
        assert_eq!((removed.subject_type, removed.subject_id, removed.moderator_id), (5, subject, Some(moderator)));
    }
}
