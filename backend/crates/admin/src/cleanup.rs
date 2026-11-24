//! Personal data: erasing it when an account is deleted, and exporting it.
//!
//! When identity erases an account (`identity.user.deleted`) the moderation
//! record and the audit trail are kept — they document what staff did — but
//! every reference to the person is cut and the text they wrote is removed,
//! except the actor of audit entries: a staff member's actions stay theirs.
use async_trait::async_trait;
use tokuly_core::{events::Event, export::DataExporter, prelude::*};
use tokuly_proto::common::SubjectType;

use crate::{Admin, events::UserRef};

impl Admin {
    /// Removes a deleted account's personal data from the admin tables.
    /// Idempotent: handling the event twice changes nothing more.
    pub(crate) async fn on_user_deleted(&self, event: &Event) -> AppResult<()> {
        let UserRef { user_id } = event.data()?;
        let mut tx = self.core.db.begin().await?;

        // Reports the user filed: keep the count, drop who and what they wrote.
        sqlx::query("UPDATE moderation_reports SET reporter_id = NULL, text = '' WHERE reporter_id = $1").bind(user_id).execute(&mut *tx).await?;

        // Cases about the user's content or the account itself. The owning
        // domains delete the content, so open cases have nothing left to act on.
        sqlx::query(
            "UPDATE moderation_cases
             SET subject_owner_id = NULL, subject_title = '', subject_path = '',
                 status = 'resolved',
                 resolved_at = COALESCE(resolved_at, now()),
                 resolution_reason = CASE WHEN status = 'open' THEN 'account deleted' ELSE resolution_reason END
             WHERE subject_owner_id = $1 OR (subject_type = $2 AND subject_id = $1)",
        )
        .bind(user_id)
        .bind(SubjectType::SUBJECT_TYPE_USER as i32)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE moderation_cases SET assignee_id = NULL, assigned_at = NULL WHERE assignee_id = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE moderation_cases SET resolved_by = NULL WHERE resolved_by = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE moderation_case_notes SET author_id = NULL WHERE author_id = $1").bind(user_id).execute(&mut *tx).await?;

        // The moderation log keeps what was done, not to whom or by whom.
        sqlx::query("UPDATE moderation_actions SET target_user_id = NULL, subject_title = '' WHERE target_user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE moderation_actions SET moderator_id = NULL WHERE moderator_id = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM user_suspensions WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE user_suspensions SET moderator_id = NULL WHERE moderator_id = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE user_suspensions SET ended_by = NULL WHERE ended_by = $1").bind(user_id).execute(&mut *tx).await?;

        sqlx::query("UPDATE announcements SET author_id = NULL WHERE author_id = $1").bind(user_id).execute(&mut *tx).await?;

        // Audit rows stay. What staff did stays theirs: the account is anonymized,
        // not removed, so the actor still resolves to the (erased) account and
        // the trail keeps saying who acted. As the affected account, the person
        // is nulled out.
        sqlx::query(
            "UPDATE admin_audit_log
             SET target_user_id = NULL, target_id = CASE WHEN target_type = 'user' THEN '' ELSE target_id END
             WHERE target_user_id = $1",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;

        let allow_listed = sqlx::query("UPDATE feature_flags SET allow_user_ids = array_remove(allow_user_ids, $1) WHERE $1 = ANY(allow_user_ids)")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        if allow_listed.rows_affected() > 0 {
            self.core.flags.invalidate().await;
        }
        Ok(())
    }
}

/// The admin domain's section of a personal data export: the sanctions
/// taken against the account. Internal notes and the names of the
/// moderators involved are not the user's data and stay out.
pub(crate) struct ModerationExporter {
    pub core: Core,
}

#[async_trait]
impl DataExporter for ModerationExporter {
    fn name(&self) -> &'static str {
        "moderation"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        let actions: Vec<(i32, String, String, Option<chrono::DateTime<chrono::Utc>>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
            "SELECT kind, reason, subject_title, expires_at, created_at FROM moderation_actions
             WHERE target_user_id = $1 AND kind <> 1 ORDER BY id",
        )
        .bind(user_id)
        .fetch_all(&self.core.db)
        .await?;
        let actions: Vec<serde_json::Value> = actions
            .into_iter()
            .map(|(kind, reason, subject_title, expires_at, created_at)| {
                serde_json::json!({
                    "action": export_action_name(kind),
                    "reason": reason,
                    "content": subject_title,
                    "expires_at": expires_at,
                    "at": created_at,
                })
            })
            .collect();
        Ok(serde_json::json!({ "actions": actions }))
    }
}

/// `ModerationActionKind` numbers as words. Dismissed reports (1) are not
/// exported: nothing happened to the account.
fn export_action_name(kind: i32) -> &'static str {
    match kind {
        2 => "content_removed",
        3 => "warning",
        4 => "suspension",
        5 => "ban",
        6 => "suspension_lifted",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use tokuly_proto::admin::ModerationActionKind;

    use super::*;

    #[test]
    fn export_names_follow_the_proto_enum() {
        let name = |kind: ModerationActionKind| export_action_name(kind as i32);
        assert_eq!(ModerationActionKind::MODERATION_ACTION_KIND_DISMISS as i32, 1, "the export query skips kind 1");
        assert_eq!(name(ModerationActionKind::MODERATION_ACTION_KIND_REMOVE_CONTENT), "content_removed");
        assert_eq!(name(ModerationActionKind::MODERATION_ACTION_KIND_WARN), "warning");
        assert_eq!(name(ModerationActionKind::MODERATION_ACTION_KIND_SUSPEND), "suspension");
        assert_eq!(name(ModerationActionKind::MODERATION_ACTION_KIND_BAN), "ban");
        assert_eq!(name(ModerationActionKind::MODERATION_ACTION_KIND_UNSUSPEND), "suspension_lifted");
    }
}
