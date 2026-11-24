//! Acting on reported content: resolving cases and taking content down.
//!
//! Both paths run in one transaction that locks the case, applies the
//! action (see [`crate::sanctions`] for account sanctions), emits
//! `admin.content.removed` for takedowns and `admin.report.resolved` for the
//! reports it settles, notifies the owner and the reporters, and writes the
//! moderation and audit logs.
use buffa::EnumValue;
use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use tokuly_core::{events as core_events, outbound, prelude::*};
use tokuly_proto::{admin::ModerationActionKind, common::SubjectType};

use crate::{
    Admin,
    actions::{ActionRow, NewAction},
    audit::Audit,
    cases::{CaseRow, STATUS_RESOLVED, case_columns},
    events::{CONTENT_REMOVED, ContentRemoved, REPORT_RESOLVED, ReportResolved},
    notices,
    sanctions::{Sanction, Target},
    staff::{Staff, note_text, reason_text, suspension_duration},
};

use ModerationActionKind::{
    MODERATION_ACTION_KIND_BAN as BAN, MODERATION_ACTION_KIND_DISMISS as DISMISS, MODERATION_ACTION_KIND_REMOVE_CONTENT as REMOVE_CONTENT,
    MODERATION_ACTION_KIND_SUSPEND as SUSPEND, MODERATION_ACTION_KIND_WARN as WARN,
};

/// A moderator's decision on a case, validated.
#[derive(Debug, Clone)]
pub(crate) struct Resolution {
    pub action: ModerationActionKind,
    /// Whether the content is taken down (always true for REMOVE_CONTENT).
    pub remove_content: bool,
    /// Shown to the owner; empty only for dismissals.
    pub reason: String,
    /// Internal.
    pub note: String,
    /// SUSPEND only.
    pub suspension: Option<chrono::Duration>,
}

impl Resolution {
    /// Validates the fields of a `ResolveCaseRequest`.
    pub fn parse(action: EnumValue<ModerationActionKind>, remove_content: bool, reason: &str, note: &str, suspension_seconds: i64) -> AppResult<Self> {
        let action = action
            .as_known()
            .filter(|a| matches!(a, DISMISS | REMOVE_CONTENT | WARN | SUSPEND | BAN))
            .ok_or_else(|| AppError::invalid("ACTION_INVALID", "action must be DISMISS, REMOVE_CONTENT, WARN, SUSPEND or BAN"))?;
        ensure!(!(action == DISMISS && remove_content), invalid, "ACTION_INVALID", "a dismissal cannot remove content");
        let reason = reason_text(reason, action != DISMISS)?;
        let note = note_text(note)?;
        let suspension = if action == SUSPEND {
            Some(suspension_duration(suspension_seconds)?)
        } else {
            ensure!(suspension_seconds == 0, invalid, "SUSPENSION_DURATION_INVALID", "only SUSPEND takes a duration");
            None
        };
        Ok(Self { action, remove_content: remove_content || action == REMOVE_CONTENT, reason, note, suspension })
    }

    /// A plain takedown, as issued by `TakedownContent`.
    pub fn takedown(reason: String, note: String) -> Self {
        Self { action: REMOVE_CONTENT, remove_content: true, reason, note, suspension: None }
    }

    fn sanctions_owner(&self) -> bool {
        matches!(self.action, WARN | SUSPEND | BAN)
    }
}

/// Outcome of resolving a case.
#[derive(Debug)]
pub(crate) struct Resolved {
    pub case: CaseRow,
    /// Moderation log entries written, in the order applied.
    pub actions: Vec<ActionRow>,
}

/// Outcome of a takedown by reference.
#[derive(Debug)]
pub(crate) struct Takedown {
    pub action: ActionRow,
    /// The open case the takedown resolved, if there was one.
    pub resolved_case: Option<CaseRow>,
}

/// The content a takedown removes.
struct Removal<'a> {
    subject_type: i32,
    subject_id: Uuid,
    owner_id: Option<Uuid>,
    /// `owner_id` is the hidden author of anonymous content.
    owner_hidden: bool,
    title: &'a str,
    reason: &'a str,
    note: &'a str,
    case_id: Option<Uuid>,
}

/// What we could find out about a subject referenced in a takedown.
struct LocatedSubject {
    id: Uuid,
    owner_id: Option<Uuid>,
    /// `owner_id` is the hidden author of anonymous content (`owners.rs`).
    owner_hidden: bool,
    title: String,
}

impl Admin {
    /// Closes an open case with the moderator's decision.
    pub(crate) async fn apply_resolution(&self, staff: &Staff, case_id: Uuid, resolution: &Resolution) -> AppResult<Resolved> {
        // Everything that can refuse the request is checked before the
        // transaction opens, so the case row is locked only for the writes.
        let owner = self.check_resolution(staff, &self.case(case_id).await?, resolution).await?;
        if resolution.action != DISMISS {
            self.limit_sanctions(staff).await?;
        }
        // Suspending the owner has identity lock the account while the case row is locked.
        let slot = match resolution.action {
            SUSPEND | BAN => Some(self.identity_slot().await?),
            _ => None,
        };

        let mut tx = self.core.db.begin().await?;
        let case = self.lock_case(&mut tx, case_id).await?;
        // Re-checked under the lock: another moderator may have been faster.
        ensure!(case.is_open(), precondition, "CASE_ALREADY_RESOLVED", "this case has already been resolved");

        let sanction = Sanction { reason: &resolution.reason, note: &resolution.note, case: Some(&case) };
        let mut actions = Vec::new();
        // `Some(expiry)` once the owner was suspended, so the mail goes out after commit.
        let mut suspended_until: Option<Option<DateTime<Utc>>> = None;
        match (resolution.action, &owner) {
            (WARN, Some(owner)) => actions.push(self.warn(&mut tx, staff, owner, sanction).await?),
            (SUSPEND | BAN, Some(owner)) => {
                let expires_at = resolution.suspension.map(|length| Utc::now() + length);
                actions.push(self.suspend(&mut tx, staff, owner, sanction, expires_at).await?);
                suspended_until = Some(expires_at);
            }
            (DISMISS, _) => {
                // Nothing happened to the owner: a hidden author is not even recorded.
                let dismissal = NewAction::new(DISMISS, Some(staff.id))
                    .target(case.subject_owner_id.filter(|_| !case.owner_hidden))
                    .hidden_target(case.owner_hidden)
                    .case(Some(case.id))
                    .subject(case.subject_type, case.subject_id, &case.subject_title)
                    .texts("", &resolution.note);
                actions.push(dismissal.insert(&mut tx).await?);
            }
            _ => {}
        }
        if resolution.remove_content {
            let removal = Removal {
                subject_type: case.subject_type,
                subject_id: case.subject_id,
                owner_id: case.subject_owner_id,
                owner_hidden: case.owner_hidden,
                title: &case.subject_title,
                reason: &resolution.reason,
                note: &resolution.note,
                case_id: Some(case.id),
            };
            actions.push(self.remove_content(&mut tx, staff, removal).await?);
        }
        self.settle_reports(&mut tx, staff, case.id, resolution.action != DISMISS).await?;

        let resolved: CaseRow = sqlx::query_as(concat!(
            "UPDATE moderation_cases
             SET status = $2, resolution = $3, content_removed = $4, resolution_reason = $5, resolved_by = $6, resolved_at = now()
             WHERE id = $1 RETURNING ",
            case_columns!()
        ))
        .bind(case.id)
        .bind(STATUS_RESOLVED)
        .bind(resolution.action as i32)
        .bind(resolution.remove_content)
        .bind(&resolution.reason)
        .bind(staff.id)
        .fetch_one(&mut *tx)
        .await?;
        Audit::by(staff, "case.resolved")
            .case(&case)
            .reason(resolution.reason.clone())
            .before(serde_json::json!({ "status": case.status, "report_count": case.report_count, "assignee_id": case.assignee_id }))
            .after(serde_json::json!({
                "status": resolved.status,
                "action": action_name(resolution.action),
                "content_removed": resolution.remove_content,
                "note": resolution.note,
            }))
            .record(&mut *tx)
            .await?;
        tx.commit().await?;
        drop(slot);

        if let (Some(expires_at), Some(owner)) = (suspended_until, &owner) {
            self.mail_suspension(owner, &resolution.reason, expires_at);
        }
        Ok(Resolved { case: resolved, actions })
    }

    /// Takes content down by reference. An open case about the same subject
    /// is resolved through the regular path so its reporters hear back.
    pub(crate) async fn takedown(&self, staff: &Staff, subject_type: EnumValue<SubjectType>, reference: &str, resolution: Resolution) -> AppResult<Takedown> {
        let subject_type = subject_type
            .as_known()
            .filter(|t| *t != SubjectType::SUBJECT_TYPE_UNSPECIFIED)
            .ok_or_else(|| AppError::invalid("SUBJECT_INVALID", "a subject type is required"))?;
        ensure!(
            subject_type != SubjectType::SUBJECT_TYPE_USER,
            invalid,
            "ACTION_NOT_APPLICABLE",
            "an account cannot be removed as content; suspend or ban it instead"
        );
        let subject = self.locate_subject(staff, subject_type, reference).await?;

        let open_case: Option<Uuid> = sqlx::query_scalar("SELECT id FROM moderation_cases WHERE subject_type = $1 AND subject_id = $2 AND status = 'open'")
            .bind(subject_type as i32)
            .bind(subject.id)
            .fetch_optional(&self.core.db)
            .await?;
        if let Some(case_id) = open_case {
            match self.apply_resolution(staff, case_id, &resolution).await {
                Ok(resolved) => {
                    let removal = resolved.actions.iter().find(|a| a.kind == REMOVE_CONTENT as i32).cloned();
                    let action = removal.ok_or_else(|| AppError::internal(anyhow::anyhow!("takedown resolution wrote no removal entry")))?;
                    return Ok(Takedown { action, resolved_case: Some(resolved.case) });
                }
                // Another moderator closed it in the meantime: take the content down directly.
                Err(e) if e.reason == "CASE_ALREADY_RESOLVED" => {}
                Err(e) => return Err(e),
            }
        }

        // Taking the post down tells its author, and the moderation log names
        // them: not something for a moderator who is involved in it.
        ensure!(
            !(subject.owner_hidden && self.is_party_to_hidden(staff.id, subject_type as i32, subject.id).await?),
            denied,
            "CONFLICT_OF_INTEREST",
            "you are involved in this post; another moderator has to handle it"
        );
        self.limit_sanctions(staff).await?;
        let mut tx = self.core.db.begin().await?;
        let removal = Removal {
            subject_type: subject_type as i32,
            subject_id: subject.id,
            owner_id: subject.owner_id,
            owner_hidden: subject.owner_hidden,
            title: &subject.title,
            reason: &resolution.reason,
            note: &resolution.note,
            case_id: None,
        };
        let action = self.remove_content(&mut tx, staff, removal).await?;
        tx.commit().await?;
        Ok(Takedown { action, resolved_case: None })
    }

    /// Whether `staff` may apply `resolution` to `case`. Returns the content
    /// owner when the resolution sanctions them.
    async fn check_resolution(&self, staff: &Staff, case: &CaseRow, resolution: &Resolution) -> AppResult<Option<Target>> {
        ensure!(case.is_open(), precondition, "CASE_ALREADY_RESOLVED", "this case has already been resolved");
        ensure!(!case.is_about(staff.id), denied, "CONFLICT_OF_INTEREST", "you cannot resolve a case about yourself or your own content");
        // An anonymous post: whoever is involved in it leaves the case to
        // another moderator, as they would inside the community.
        ensure!(
            !(case.owner_hidden && self.is_party_to_hidden(staff.id, case.subject_type, case.subject_id).await?),
            denied,
            "CONFLICT_OF_INTEREST",
            "you are involved in this post; another moderator has to handle the case"
        );
        ensure!(
            !(resolution.remove_content && case.subject_type == SubjectType::SUBJECT_TYPE_USER as i32),
            invalid,
            "ACTION_NOT_APPLICABLE",
            "an account cannot be removed as content; suspend or ban it instead"
        );
        if !resolution.sanctions_owner() {
            return Ok(None);
        }
        let gone = || AppError::precondition("ACTION_NOT_APPLICABLE", "the owner's account no longer exists");
        let owner_id = case.subject_owner_id.ok_or_else(gone)?;
        let owner = self.target(owner_id).await.map_err(|e| if e.is_not_found() { gone() } else { e })?;
        Self::check_may_sanction(staff, &owner)?;
        Ok(Some(owner))
    }

    /// Announces a takedown to the owning domain and the owner, and logs it.
    async fn remove_content(&self, conn: &mut PgConnection, staff: &Staff, removal: Removal<'_>) -> AppResult<ActionRow> {
        let event =
            ContentRemoved { subject_type: removal.subject_type, subject_id: removal.subject_id, reason: removal.reason.to_owned(), moderator_id: staff.id };
        core_events::emit(&mut *conn, CONTENT_REMOVED, Some(staff.id), &event).await?;
        if let Some(owner_id) = removal.owner_id {
            let notice = notices::content_removed(owner_id, (removal.subject_type, removal.subject_id), removal.title, removal.reason);
            outbound::notify(&mut *conn, &notice).await?;
        }
        let action = NewAction::new(REMOVE_CONTENT, Some(staff.id))
            .target(removal.owner_id)
            .hidden_target(removal.owner_hidden)
            .subject(removal.subject_type, removal.subject_id, removal.title)
            .case(removal.case_id)
            .texts(removal.reason, removal.note)
            .insert(&mut *conn)
            .await?;
        Audit::by(staff, "content.removed")
            .subject(removal.subject_type, removal.subject_id, removal.owner_id, removal.owner_hidden)
            .reason(removal.reason)
            .after(serde_json::json!({ "title": removal.title, "case_id": removal.case_id, "note": removal.note }))
            .record(&mut *conn)
            .await?;
        Ok(action)
    }

    /// Tells everyone who reported the subject that the case was handled, and
    /// the domain that took the reports which of them are settled and how.
    async fn settle_reports(&self, conn: &mut PgConnection, staff: &Staff, case_id: Uuid, action_taken: bool) -> AppResult<()> {
        let reports: Vec<(Uuid, Option<Uuid>)> =
            sqlx::query_as("SELECT id, reporter_id FROM moderation_reports WHERE case_id = $1 ORDER BY created_at, id").bind(case_id).fetch_all(&mut *conn).await?;
        if reports.is_empty() {
            return Ok(());
        }
        let mut reporters: Vec<Uuid> = reports.iter().filter_map(|(_, reporter_id)| *reporter_id).collect();
        reporters.sort_unstable();
        reporters.dedup();
        for reporter_id in reporters {
            outbound::notify(&mut *conn, &notices::report_resolved(reporter_id, case_id, action_taken)).await?;
        }
        let settled = ReportResolved { report_ids: reports.into_iter().map(|(report_id, _)| report_id).collect(), action_taken };
        core_events::emit(&mut *conn, REPORT_RESOLVED, Some(staff.id), &settled).await?;
        Ok(())
    }

    /// Finds the subject of a takedown: its UUID, and its owner and title
    /// when anything knows them.
    async fn locate_subject(&self, staff: &Staff, subject_type: SubjectType, reference: &str) -> AppResult<LocatedSubject> {
        let reference = reference.trim();
        ensure!(!reference.is_empty(), invalid, "SUBJECT_INVALID", "a subject id is required");

        // The owning domain knows best, when it registered a resolver and
        // the content is visible to this moderator.
        if self.core.subjects.is_registered(subject_type) {
            let found = match self.core.subjects.resolve(subject_type, reference, Some(staff.id)).await {
                Ok(info) => Some(info),
                // Private content is hidden from the resolver's viewer check.
                // Reported, it resolves the way its reporters see it (not for
                // its owner or reporters); otherwise by UUID below.
                Err(e) if e.is_not_found() => match Uuid::parse_str(reference) {
                    Ok(id) => self.reported_subject(staff, subject_type as i32, id).await?,
                    Err(_) => None,
                },
                Err(e) => return Err(e),
            };
            if let Some(info) = found {
                let (owner_id, owner_hidden) = match self.existing_user(info.owner_id).await? {
                    Some(owner_id) => (Some(owner_id), false),
                    None => self.existing_hidden_owner(subject_type, info.id).await?,
                };
                return Ok(LocatedSubject { id: info.id, owner_id, owner_hidden, title: info.title });
            }
        }

        let id = Uuid::parse_str(reference)
            .map_err(|_| AppError::not_found("SUBJECT_NOT_FOUND", "the item was not found; private content must be referenced by its UUID"))?;
        // Earlier reports are the only other place its owner and title are known.
        let known: Option<(Option<Uuid>, bool, String)> =
            sqlx::query_as("SELECT subject_owner_id, owner_hidden, subject_title FROM moderation_cases WHERE subject_type = $1 AND subject_id = $2 ORDER BY id DESC LIMIT 1")
                .bind(subject_type as i32)
                .bind(id)
                .fetch_optional(&self.core.db)
                .await?;
        let (owner_id, owner_hidden, title) = known.unwrap_or_default();
        let (owner_id, owner_hidden) = match owner_id {
            Some(owner_id) => (Some(owner_id), owner_hidden),
            None => self.existing_hidden_owner(subject_type, id).await?,
        };
        Ok(LocatedSubject { id, owner_id, owner_hidden, title })
    }

    /// The owner of content that names none (see `owners.rs`), if that
    /// account still exists, and whether there is such a hidden owner.
    async fn existing_hidden_owner(&self, subject_type: SubjectType, subject_id: Uuid) -> AppResult<(Option<Uuid>, bool)> {
        match self.hidden_owner(subject_type as i32, subject_id).await? {
            Some(owner_id) => Ok((self.existing_user(owner_id).await?, true)),
            None => Ok((None, false)),
        }
    }

    /// `user_id` if that account exists and is not deleted.
    pub(crate) async fn existing_user(&self, user_id: Uuid) -> AppResult<Option<Uuid>> {
        Ok(sqlx::query_scalar("SELECT id FROM users WHERE id = $1 AND deleted_at IS NULL").bind(user_id).fetch_optional(&self.core.db).await?)
    }
}

/// Stable lowercase name of an action for audit JSON.
fn action_name(action: ModerationActionKind) -> &'static str {
    match action {
        DISMISS => "dismiss",
        REMOVE_CONTENT => "remove_content",
        WARN => "warn",
        SUSPEND => "suspend",
        BAN => "ban",
        ModerationActionKind::MODERATION_ACTION_KIND_UNSUSPEND => "unsuspend",
        ModerationActionKind::MODERATION_ACTION_KIND_UNSPECIFIED => "unspecified",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(action: ModerationActionKind, remove_content: bool, reason: &str, seconds: i64) -> AppResult<Resolution> {
        Resolution::parse(action.into(), remove_content, reason, "", seconds)
    }

    #[test]
    fn dismissals_need_no_reason_and_cannot_remove_content() {
        let dismissal = parse(DISMISS, false, "", 0).unwrap();
        assert!(!dismissal.remove_content && dismissal.reason.is_empty() && !dismissal.sanctions_owner());
        assert_eq!(parse(DISMISS, true, "", 0).unwrap_err().reason, "ACTION_INVALID");
    }

    #[test]
    fn every_other_action_needs_a_reason() {
        for action in [REMOVE_CONTENT, WARN, SUSPEND, BAN] {
            assert_eq!(parse(action, false, "  ", 86_400).unwrap_err().reason, "REASON_REQUIRED", "{action:?}");
        }
    }

    #[test]
    fn remove_content_always_removes_and_sanctions_can_opt_in() {
        assert!(parse(REMOVE_CONTENT, false, "spam", 0).unwrap().remove_content);
        assert!(!parse(WARN, false, "spam", 0).unwrap().remove_content);
        let both = parse(BAN, true, "spam", 0).unwrap();
        assert!(both.remove_content && both.sanctions_owner() && both.suspension.is_none());
    }

    #[test]
    fn only_suspensions_take_a_duration() {
        assert_eq!(parse(SUSPEND, false, "spam", 86_400).unwrap().suspension, Some(chrono::Duration::days(1)));
        assert_eq!(parse(SUSPEND, false, "spam", 0).unwrap_err().reason, "SUSPENSION_DURATION_INVALID");
        assert_eq!(parse(BAN, false, "spam", 86_400).unwrap_err().reason, "SUSPENSION_DURATION_INVALID");
        assert_eq!(parse(WARN, false, "spam", 60).unwrap_err().reason, "SUSPENSION_DURATION_INVALID");
    }

    #[test]
    fn unknown_and_internal_actions_are_rejected() {
        assert_eq!(parse(ModerationActionKind::MODERATION_ACTION_KIND_UNSPECIFIED, false, "x", 0).unwrap_err().reason, "ACTION_INVALID");
        assert_eq!(parse(ModerationActionKind::MODERATION_ACTION_KIND_UNSUSPEND, false, "x", 0).unwrap_err().reason, "ACTION_INVALID");
        assert_eq!(Resolution::parse(EnumValue::from(99), false, "x", "", 0).unwrap_err().reason, "ACTION_INVALID");
    }
}
