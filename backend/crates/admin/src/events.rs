//! Event topics and payloads of the admin domain (see `docs/events/admin.md`).
//!
//! Emitted:
//! - [`CONTENT_REMOVED`] — [`ContentRemoved`]: the domain owning the subject
//!   deletes or hides it.
//! - [`REPORT_RESOLVED`] — [`ReportResolved`]: the reports of a case were
//!   decided, so the domain that took them can show their outcome.
//! - [`SERVER_SETTING_CHANGED`] — [`ServerSettingChanged`]: an admin saved
//!   or removed a server setting in the console (never with its value).
//!
//! Consumed:
//! - [`REPORT_CREATED`] — [`ReportCreated`], from the engagement domain.
//! - [`USER_CREATED`] and [`USER_EMAIL_VERIFIED`], from identity: the
//!   first-deployment admin may have just signed up or confirmed the
//!   address (the payload is not needed).
//! - [`USER_DELETED`] — [`UserRef`], from identity.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const CONTENT_REMOVED: &str = "admin.content.removed";
pub const REPORT_RESOLVED: &str = "admin.report.resolved";
pub const SERVER_SETTING_CHANGED: &str = "admin.server_setting.changed";
pub const REPORT_CREATED: &str = "engagement.report.created";
pub const USER_CREATED: &str = "identity.user.created";
pub const USER_EMAIL_VERIFIED: &str = "identity.user.email_verified";
pub const USER_DELETED: &str = "identity.user.deleted";

/// A moderator took a piece of content down.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentRemoved {
    /// `tokuly.common.v1.SubjectType`.
    pub subject_type: i32,
    pub subject_id: Uuid,
    /// The reason shown to the owner.
    pub reason: String,
    pub moderator_id: Uuid,
}

/// A moderator decided a case: every report filed into it is settled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportResolved {
    /// The `report_id`s of the `engagement.report.created` events in the case.
    pub report_ids: Vec<Uuid>,
    /// False for a dismissal; true when content was removed or the owner was
    /// warned, suspended or banned.
    pub action_taken: bool,
}

/// An admin changed a server setting in the console (「サーバー設定」). The
/// value is not in the event: secrets must not reach the event stream, and
/// whoever needs a value reads it from `tokuly_core::settings`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerSettingChanged {
    /// `[section]` and key in `tokuly.toml`.
    pub section: String,
    pub key: String,
    pub secret: bool,
    /// The change applies without a restart.
    pub live: bool,
}

/// A user reported a subject (our view of engagement's payload).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportCreated {
    pub report_id: Uuid,
    pub reporter_id: Uuid,
    /// `tokuly.common.v1.SubjectType`.
    pub subject_type: i32,
    pub subject_id: Uuid,
    /// Engagement always sends an ID; a missing or null owner (content that
    /// belongs to nobody, such as a place) is accepted as "unknown" rather
    /// than losing the report.
    #[serde(default)]
    pub subject_owner_id: Option<Uuid>,
    /// SPAM | HARASSMENT | HATE | SEXUAL | VIOLENCE | SELF_HARM | ILLEGAL |
    /// IMPERSONATION | COPYRIGHT | OTHER
    pub category: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub subject_title: String,
    #[serde(default)]
    pub subject_path: String,
    /// What the reporter received, as the owning domain kept it for
    /// moderation: for content edited or withdrawn since, every version.
    /// `subject_title` is what the reporter could still see.
    #[serde(default)]
    pub reported_content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRef {
    pub user_id: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_payload_matches_the_engagement_contract() {
        let (report, reporter, subject, owner) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let full: ReportCreated = serde_json::from_value(serde_json::json!({
            "report_id": report,
            "reporter_id": reporter,
            "subject_type": 3,
            "subject_id": subject,
            "subject_owner_id": owner,
            "category": "SPAM",
            "text": "宣伝です",
            "subject_title": "今日のランチ",
            "subject_path": "/notes/abc",
            "reported_content": "[2版] 今日のランチ → 昨日のランチ",
        }))
        .unwrap();
        assert_eq!(full.reported_content, "[2版] 今日のランチ → 昨日のランチ");
        assert_eq!((full.report_id, full.reporter_id, full.subject_type, full.subject_id), (report, reporter, 3, subject));
        assert_eq!(full.subject_owner_id, Some(owner));
        assert_eq!(
            (full.category.as_str(), full.text.as_str(), full.subject_title.as_str(), full.subject_path.as_str()),
            ("SPAM", "宣伝です", "今日のランチ", "/notes/abc")
        );

        // Optional texts and an unknown owner do not make a report undeliverable; extra fields are ignored.
        let sparse: ReportCreated = serde_json::from_value(serde_json::json!({
            "report_id": report,
            "reporter_id": reporter,
            "subject_type": 15,
            "subject_id": subject,
            "subject_owner_id": null,
            "category": "OTHER",
            "added_later": true,
        }))
        .unwrap();
        assert!(sparse.subject_owner_id.is_none() && sparse.text.is_empty() && sparse.subject_title.is_empty());
        assert!(sparse.reported_content.is_empty());
    }

    #[test]
    fn content_removed_serializes_to_the_agreed_shape() {
        let (subject, moderator) = (Uuid::now_v7(), Uuid::now_v7());
        let event = ContentRemoved { subject_type: 5, subject_id: subject, reason: "規約違反".into(), moderator_id: moderator };
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            serde_json::json!({ "subject_type": 5, "subject_id": subject, "reason": "規約違反", "moderator_id": moderator })
        );
    }
}
