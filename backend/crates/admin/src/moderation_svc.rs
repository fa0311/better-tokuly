//! `ModerationService`: the reports queue. Every RPC needs the `moderator` role.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::admin::*;

use crate::{
    Admin,
    actions::ActionFilter,
    audit::Audit,
    cases::{CaseFilter, CaseRow, NoteRow, case_columns, category_to_str, note_to_proto},
    first_page,
    resolution::Resolution,
    staff::{NOTE_MAX_CHARS, Staff, note_text, reason_text},
};

/// Reports included with `GetCase`; the rest is paged through `ListCaseReports`.
const CASE_PREVIEW_REPORTS: i64 = 20;
/// Moderation log entries included with `GetCase`.
const CASE_ACTIONS_LIMIT: i64 = 50;

impl Admin {
    /// Whether the account may be assigned cases: an active moderator or admin.
    async fn is_moderator(&self, user_id: Uuid) -> AppResult<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL AND banned_at IS NULL AND roles && ARRAY['admin', 'moderator'])",
        )
        .bind(user_id)
        .fetch_one(&self.core.db)
        .await?)
    }

    /// Refuses a staff member the case is about (their content or their
    /// account): they must not learn who reported them or what staff noted,
    /// nor steer the case.
    fn refuse_own_case(staff: &Staff, case: &CaseRow) -> AppResult<()> {
        ensure!(!case.is_about(staff.id), denied, "CONFLICT_OF_INTEREST", "this case is about you; another moderator has to handle it");
        Ok(())
    }
}

impl ModerationService for Admin {
    async fn list_cases(&self, ctx: RequestContext, request: ServiceRequest<'_, ListCasesRequest>) -> ServiceResult<ListCasesResponse> {
        handle(async {
            let staff = Staff::moderator(&ctx)?;
            let category = match request.category.as_known() {
                None | Some(ReportCategory::REPORT_CATEGORY_UNSPECIFIED) => None,
                Some(category) => category_to_str(category),
            };
            let filter = CaseFilter {
                open: request.status.as_known() != Some(CaseStatus::CASE_STATUS_RESOLVED),
                assignee: request.assignee.as_known().unwrap_or(AssigneeFilter::ASSIGNEE_FILTER_UNSPECIFIED),
                assignee_id: parse_opt_id(request.assignee_id)?,
                subject_type: request.subject_type.to_i32(),
                category,
                owner_id: parse_opt_id(request.owner_id)?,
                sort: request.sort.as_known().unwrap_or(CaseSort::CASE_SORT_UNSPECIFIED),
            };
            let page = Page::from_view(request.page.as_option(), 100);
            let (rows, info) = self.find_cases(staff.id, &filter, &page).await?;
            ok(ListCasesResponse { cases: self.cases_to_proto(&rows).await?, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn get_case(&self, ctx: RequestContext, request: ServiceRequest<'_, GetCaseRequest>) -> ServiceResult<GetCaseResponse> {
        handle(async {
            let staff = Staff::moderator(&ctx)?;
            let case = self.case(parse_id(request.case_id)?).await?;
            let action_filter = ActionFilter { case_id: Some(case.id), ..Default::default() };
            let (actions, _) = self.list_actions(&action_filter, &first_page(CASE_ACTIONS_LIMIT)).await?;
            if case.is_about(staff.id) {
                // A case about the caller: that it exists and what was done, not
                // who reported them, what they wrote, or what staff noted.
                let mut moderation_case = self.case_to_proto(&case).await?;
                moderation_case.conflict_of_interest = true;
                moderation_case.about_you = true;
                let actions = self.case_actions(&staff, &actions, false).await?;
                return ok(GetCaseResponse { moderation_case: moderation_case.into(), actions, ..Default::default() });
            }
            let (reports, _) = self.case_reports(case.id, &first_page(CASE_PREVIEW_REPORTS)).await?;
            // A hidden author is named here, to a moderator who is not involved, and nowhere else.
            let (mut moderation_case, owner_named) = self.opened_case(&staff, &case).await?;
            // The content as its reporters can still see it, also after its
            // owner hid it; not for a moderator involved in a hidden author's post.
            if !moderation_case.conflict_of_interest
                && let Some(subject) = self.reported_subject(&staff, case.subject_type, case.subject_id).await?
            {
                moderation_case.current_subject =
                    CaseSubject { title: subject.title, path: subject.path, ..Default::default() }.into();
            }
            // The owner's record counts their anonymous posts only where this view named them: this one.
            let revealed = case.owner_hidden.then_some((case.subject_type, case.subject_id)).filter(|_| owner_named);
            let owner_record = match case.subject_owner_id.filter(|_| owner_named) {
                Some(owner_id) => Some(self.moderation_summary(owner_id, revealed).await?),
                None => None,
            };
            let actions = self.case_actions(&staff, &actions, !case.owner_hidden || owner_named).await?;
            ok(GetCaseResponse {
                moderation_case: moderation_case.into(),
                reports,
                notes: self.case_notes(case.id).await?,
                actions,
                owner_record: owner_record.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_case_reports(&self, ctx: RequestContext, request: ServiceRequest<'_, ListCaseReportsRequest>) -> ServiceResult<ListCaseReportsResponse> {
        handle(async {
            let staff = Staff::moderator(&ctx)?;
            let case = self.case(parse_id(request.case_id)?).await?;
            Self::refuse_own_case(&staff, &case)?;
            let page = Page::from_view(request.page.as_option(), 100);
            let (reports, info) = self.case_reports(case.id, &page).await?;
            ok(ListCaseReportsResponse { reports, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn assign_case(&self, ctx: RequestContext, request: ServiceRequest<'_, AssignCaseRequest>) -> ServiceResult<AssignCaseResponse> {
        handle(async {
            let staff = Staff::moderator(&ctx)?;
            let case_id = parse_id(request.case_id)?;
            let assignee = match (request.unassign, parse_opt_id(request.assignee_id)?) {
                (true, _) => None,
                (false, None) => Some(staff.id),
                (false, Some(assignee_id)) => {
                    ensure!(
                        assignee_id == staff.id || self.is_moderator(assignee_id).await?,
                        invalid,
                        "ASSIGNEE_NOT_MODERATOR",
                        "cases can only be assigned to moderators"
                    );
                    Some(assignee_id)
                }
            };
            let mut tx = self.core.db.begin().await?;
            let case = self.lock_case(&mut tx, case_id).await?;
            Self::refuse_own_case(&staff, &case)?;
            ensure!(case.is_open(), precondition, "CASE_ALREADY_RESOLVED", "this case has already been resolved");
            let updated: CaseRow = sqlx::query_as(concat!(
                "UPDATE moderation_cases SET assignee_id = $2, assigned_at = CASE WHEN $2::uuid IS NULL THEN NULL ELSE now() END
                 WHERE id = $1 RETURNING ",
                case_columns!()
            ))
            .bind(case.id)
            .bind(assignee)
            .fetch_one(&mut *tx)
            .await?;
            Audit::by(&staff, if assignee.is_some() { "case.assigned" } else { "case.unassigned" })
                .case(&case)
                .before(serde_json::json!({ "assignee_id": case.assignee_id }))
                .after(serde_json::json!({ "assignee_id": assignee }))
                .record(&mut *tx)
                .await?;
            tx.commit().await?;
            ok(AssignCaseResponse { moderation_case: self.case_to_proto(&updated).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn add_case_note(&self, ctx: RequestContext, request: ServiceRequest<'_, AddCaseNoteRequest>) -> ServiceResult<AddCaseNoteResponse> {
        handle(async {
            let staff = Staff::moderator(&ctx)?;
            let body = note_text(request.body)?;
            ensure!(!body.is_empty(), invalid, "NOTE_INVALID", "a note needs 1-{NOTE_MAX_CHARS} characters");
            let case = self.case(parse_id(request.case_id)?).await?;
            Self::refuse_own_case(&staff, &case)?;
            let mut tx = self.core.db.begin().await?;
            let note: NoteRow =
                sqlx::query_as("INSERT INTO moderation_case_notes (case_id, author_id, body) VALUES ($1, $2, $3) RETURNING id, author_id, body, created_at")
                    .bind(case.id)
                    .bind(staff.id)
                    .bind(&body)
                    .fetch_one(&mut *tx)
                    .await?;
            Audit::by(&staff, "case.note_added").case(&case).after(serde_json::json!({ "note_id": note.id })).record(&mut *tx).await?;
            tx.commit().await?;
            let people = tokuly_identity::summaries(&self.core, &[staff.id]).await?;
            ok(AddCaseNoteResponse { note: note_to_proto(note, &people).into(), ..Default::default() })
        })
        .await
    }

    async fn resolve_case(&self, ctx: RequestContext, request: ServiceRequest<'_, ResolveCaseRequest>) -> ServiceResult<ResolveCaseResponse> {
        handle(async {
            let staff = Staff::moderator(&ctx)?;
            let case_id = parse_id(request.case_id)?;
            let resolution = Resolution::parse(request.action, request.remove_content, request.reason, request.note, request.suspension_seconds)?;
            let resolved = self.apply_resolution(&staff, case_id, &resolution).await?;
            ok(ResolveCaseResponse {
                moderation_case: self.case_to_proto(&resolved.case).await?.into(),
                actions: self.actions_to_proto(&resolved.actions).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn reopen_case(&self, ctx: RequestContext, request: ServiceRequest<'_, ReopenCaseRequest>) -> ServiceResult<ReopenCaseResponse> {
        handle(async {
            let staff = Staff::moderator(&ctx)?;
            let case_id = parse_id(request.case_id)?;
            let note = note_text(request.note)?;
            let mut tx = self.core.db.begin().await?;
            let case = self.lock_case(&mut tx, case_id).await?;
            Self::refuse_own_case(&staff, &case)?;
            ensure!(!case.is_open(), precondition, "CASE_NOT_RESOLVED", "only resolved cases can be reopened");
            // Only one case per subject may be open; reports that arrived
            // since the resolution already opened a newer one.
            let superseded: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM moderation_cases WHERE subject_type = $1 AND subject_id = $2 AND status = 'open')")
                    .bind(case.subject_type)
                    .bind(case.subject_id)
                    .fetch_one(&mut *tx)
                    .await?;
            ensure!(!superseded, precondition, "CASE_SUPERSEDED", "a newer open case exists for the same subject");
            let reopened: CaseRow = sqlx::query_as(concat!(
                "UPDATE moderation_cases
                 SET status = 'open', resolution = NULL, content_removed = false, resolution_reason = '', resolved_by = NULL, resolved_at = NULL
                 WHERE id = $1 RETURNING ",
                case_columns!()
            ))
            .bind(case.id)
            .fetch_one(&mut *tx)
            .await?;
            if !note.is_empty() {
                sqlx::query("INSERT INTO moderation_case_notes (case_id, author_id, body) VALUES ($1, $2, $3)")
                    .bind(case.id)
                    .bind(staff.id)
                    .bind(&note)
                    .execute(&mut *tx)
                    .await?;
            }
            Audit::by(&staff, "case.reopened")
                .case(&case)
                .reason(note)
                .before(serde_json::json!({
                    "status": case.status,
                    "resolution": case.resolution,
                    "content_removed": case.content_removed,
                    "resolved_by": case.resolved_by,
                    "resolved_at": case.resolved_at,
                }))
                .after(serde_json::json!({ "status": reopened.status }))
                .record(&mut *tx)
                .await?;
            tx.commit().await?;
            ok(ReopenCaseResponse { moderation_case: self.case_to_proto(&reopened).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn takedown_content(&self, ctx: RequestContext, request: ServiceRequest<'_, TakedownContentRequest>) -> ServiceResult<TakedownContentResponse> {
        handle(async {
            let staff = Staff::moderator(&ctx)?;
            let subject = request.subject.as_option().ok_or_else(|| AppError::invalid("SUBJECT_INVALID", "a subject is required"))?;
            let resolution = Resolution::takedown(reason_text(request.reason, true)?, note_text(request.note)?);
            let takedown = self.takedown(&staff, subject.r#type, subject.id, resolution).await?;
            let resolved_case = match &takedown.resolved_case {
                Some(case) => Some(self.case_to_proto(case).await?),
                None => None,
            };
            ok(TakedownContentResponse {
                action: self.action_to_proto(&takedown.action).await?.into(),
                resolved_case: resolved_case.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_moderation_actions(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListModerationActionsRequest>,
    ) -> ServiceResult<ListModerationActionsResponse> {
        handle(async {
            let staff = Staff::moderator(&ctx)?;
            let filter = ActionFilter {
                target_user_id: parse_opt_id(request.user_id)?,
                moderator_id: parse_opt_id(request.moderator_id)?,
                kind: request.kind.to_i32(),
                case_id: parse_opt_id(request.case_id)?,
            };
            let page = Page::from_view(request.page.as_option(), 100);
            let (rows, info) = self.list_actions(&filter, &page).await?;
            ok(ListModerationActionsResponse { actions: self.actions_listed(&staff, &rows, filter.target_user_id).await?, page: info.into(), ..Default::default() })
        })
        .await
    }
}
