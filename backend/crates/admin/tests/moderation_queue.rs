//! The reports queue: intake from engagement events, listing, assignment,
//! notes, the moderation log, and the scrubbing of deleted accounts.
mod common;

use common::*;
use tokuly_core::events::Event;
use tokuly_proto::{
    admin::*,
    common::{PageRequest, SubjectType},
};
use uuid::Uuid;

use ModerationActionKind::{
    MODERATION_ACTION_KIND_DISMISS as DISMISS, MODERATION_ACTION_KIND_REMOVE_CONTENT as REMOVE_CONTENT, MODERATION_ACTION_KIND_WARN as WARN,
};

#[tokio::test]
async fn every_moderation_rpc_needs_the_moderator_role() {
    let (app, _admin) = start().await;
    let user = app.user("user").await;
    let staff_only = app.user_with_roles("staffer", &["staff"]).await;
    for rpc in ["ListCases", "GetCase", "ListCaseReports", "AssignCase", "AddCaseNote", "ResolveCase", "ReopenCase", "TakedownContent", "ListModerationActions"]
    {
        assert_role_gated(&app, &format!("{MODERATION}/{rpc}"), &[&user, &staff_only]).await;
    }
    // Moderators and admins both get in.
    for role in ["moderator", "admin"] {
        let staff = app.user_with_roles(role, &[role]).await;
        let _: ListCasesResponse = app.call(&format!("{MODERATION}/ListCases"), Some(&staff), &ListCasesRequest::default()).await.unwrap();
    }
}

#[tokio::test]
async fn reports_about_one_subject_share_a_case_and_redelivery_is_ignored() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let (owner, alice, bob) = (app.user("owner").await, app.user("alice").await, app.user("bob").await);
    let subject = (NOTE, Uuid::now_v7());

    let first = report_event(alice.id, subject, owner.id, "SPAM", "宣伝です");
    admin.handle_event(first.clone()).await.unwrap();
    admin.handle_event(report_event(bob.id, subject, owner.id, "hate", "ひどい内容")).await.unwrap();
    admin.handle_event(report_event(bob.id, subject, owner.id, "SPAM", "")).await.unwrap();
    // At-least-once delivery: the same event again must not count twice.
    admin.handle_event(first).await.unwrap();

    let cases = case_of_owner(&app, &moderator, owner.id).await;
    assert_eq!(cases.len(), 1, "one case per subject");
    let case = &cases[0];
    assert_eq!(case.report_count, 3);
    assert_eq!(case.status, CaseStatus::CASE_STATUS_OPEN);
    assert_eq!(case.categories, vec![ReportCategory::REPORT_CATEGORY_SPAM, ReportCategory::REPORT_CATEGORY_HATE]);
    assert_eq!(case.subject.id, subject.1.to_string());
    assert_eq!(case.subject.r#type, NOTE);
    assert_eq!(case.subject_owner.id, owner.id.to_string());
    assert_eq!((case.subject_title.as_str(), case.subject_path.as_str()), ("今日のランチ", format!("/notes/{}", subject.1).as_str()));

    let detail = get_case(&app, &moderator, &case.id).await;
    assert_eq!(detail.reports.len(), 3);
    assert!(detail.reports.iter().any(|r| r.reporter.id == alice.id.to_string() && r.text == "宣伝です"));
    assert_eq!(detail.owner_record.reports_received, 3);
    assert_eq!(detail.owner_record.open_cases, 1);
    assert!(detail.notes.is_empty() && detail.actions.is_empty());

    // Reports page newest first.
    let page_one: ListCaseReportsResponse = app
        .call(
            &format!("{MODERATION}/ListCaseReports"),
            Some(&moderator),
            &ListCaseReportsRequest { case_id: case.id.clone(), page: PageRequest { limit: 2, ..Default::default() }.into(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(page_one.reports.len(), 2);
    assert!(page_one.page.has_more);
    let page_two: ListCaseReportsResponse = app
        .call(
            &format!("{MODERATION}/ListCaseReports"),
            Some(&moderator),
            &ListCaseReportsRequest {
                case_id: case.id.clone(),
                page: PageRequest { limit: 2, cursor: page_one.page.next_cursor.clone(), ..Default::default() }.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(page_two.reports.len(), 1);
    assert!(!page_two.page.has_more);
    let mut seen: Vec<&str> = page_one.reports.iter().chain(&page_two.reports).map(|r| r.id.as_str()).collect();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 3, "pages do not overlap");

    // Unknown cases are reported as such by both readers.
    let unknown = Uuid::now_v7().to_string();
    let missing = app
        .call::<_, GetCaseResponse>(&format!("{MODERATION}/GetCase"), Some(&moderator), &GetCaseRequest { case_id: unknown.clone(), ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!((missing.code.as_str(), missing.reason.as_str()), ("not_found", "CASE_NOT_FOUND"));
    let missing = app
        .call::<_, ListCaseReportsResponse>(
            &format!("{MODERATION}/ListCaseReports"),
            Some(&moderator),
            &ListCaseReportsRequest { case_id: unknown, ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!(missing.reason, "CASE_NOT_FOUND");
}

#[tokio::test]
async fn reports_from_unknown_accounts_and_categories_still_file() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let owner = app.user("owner").await;
    let subject = (SubjectType::SUBJECT_TYPE_VIDEO, Uuid::now_v7());
    // The reporter's account no longer exists and the category is one we do not know.
    admin.handle_event(report_event(Uuid::now_v7(), subject, owner.id, "BRAND_NEW_CATEGORY", "x")).await.unwrap();
    let case = case_of_owner(&app, &moderator, owner.id).await.remove(0);
    assert_eq!(case.categories, vec![ReportCategory::REPORT_CATEGORY_OTHER]);
    let detail = get_case(&app, &moderator, &case.id).await;
    assert!(detail.reports[0].reporter.is_unset());
}

#[tokio::test]
async fn queue_filters_sorts_and_pages() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let other_moderator = app.user_with_roles("mod2", &["moderator"]).await;
    let (owner, reporter, second_reporter) = (app.user("owner").await, app.user("rep").await, app.user("rep2").await);

    // Three cases about the same owner: `busy` has two reports, `video` is a different subject type.
    let quiet = (NOTE, Uuid::now_v7());
    let busy = (NOTE, Uuid::now_v7());
    let video = (SubjectType::SUBJECT_TYPE_VIDEO, Uuid::now_v7());
    admin.handle_event(report_event(reporter.id, quiet, owner.id, "SPAM", "")).await.unwrap();
    admin.handle_event(report_event(reporter.id, busy, owner.id, "HARASSMENT", "")).await.unwrap();
    admin.handle_event(report_event(second_reporter.id, busy, owner.id, "HARASSMENT", "")).await.unwrap();
    admin.handle_event(report_event(reporter.id, video, owner.id, "COPYRIGHT", "")).await.unwrap();

    // Every listing is narrowed to this test's owner: the database is shared with other tests.
    let list = |request: ListCasesRequest| {
        let (app, moderator) = (&app, &moderator);
        let request = ListCasesRequest { owner_id: owner.id.to_string(), ..request };
        async move {
            let response: ListCasesResponse = app.call(&format!("{MODERATION}/ListCases"), Some(moderator), &request).await.unwrap();
            response
        }
    };
    let subjects = |response: &ListCasesResponse| response.cases.iter().map(|c| c.subject.id.clone()).collect::<Vec<_>>();

    // Default order: most recently reported first.
    let recent = list(ListCasesRequest::default()).await;
    assert_eq!(subjects(&recent), vec![video.1.to_string(), busy.1.to_string(), quiet.1.to_string()]);
    let oldest = list(ListCasesRequest { sort: CaseSort::CASE_SORT_OLDEST.into(), ..Default::default() }).await;
    assert_eq!(subjects(&oldest), vec![quiet.1.to_string(), busy.1.to_string(), video.1.to_string()]);
    let most_reported = list(ListCasesRequest { sort: CaseSort::CASE_SORT_MOST_REPORTED.into(), ..Default::default() }).await;
    assert_eq!(most_reported.cases[0].subject.id, busy.1.to_string());

    // Filters.
    let videos = list(ListCasesRequest { subject_type: SubjectType::SUBJECT_TYPE_VIDEO.into(), ..Default::default() }).await;
    assert_eq!(subjects(&videos), vec![video.1.to_string()]);
    let harassment = list(ListCasesRequest { category: ReportCategory::REPORT_CATEGORY_HARASSMENT.into(), ..Default::default() }).await;
    assert_eq!(subjects(&harassment), vec![busy.1.to_string()]);

    // Paging through every sort order visits each case exactly once.
    for sort in [CaseSort::CASE_SORT_UNSPECIFIED, CaseSort::CASE_SORT_OLDEST, CaseSort::CASE_SORT_MOST_REPORTED] {
        let mut cursor = String::new();
        let mut visited = Vec::new();
        loop {
            let page = list(ListCasesRequest {
                sort: sort.into(),
                page: PageRequest { limit: 2, cursor: cursor.clone(), ..Default::default() }.into(),
                ..Default::default()
            })
            .await;
            visited.extend(subjects(&page));
            if !page.page.has_more {
                break;
            }
            cursor = page.page.next_cursor.clone();
        }
        visited.sort();
        visited.dedup();
        assert_eq!(visited.len(), 3, "{sort:?}");
    }
    // A cursor from one order is not valid in another.
    let first_page = list(ListCasesRequest { page: PageRequest { limit: 1, ..Default::default() }.into(), ..Default::default() }).await;
    let mixed = app
        .call::<_, ListCasesResponse>(
            &format!("{MODERATION}/ListCases"),
            Some(&moderator),
            &ListCasesRequest {
                sort: CaseSort::CASE_SORT_MOST_REPORTED.into(),
                page: PageRequest { limit: 1, cursor: first_page.page.next_cursor.clone(), ..Default::default() }.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(mixed.reason, "INVALID_CURSOR");

    // Assignment filters: take `busy`, hand `video` to a colleague.
    let busy_case = recent.cases.iter().find(|c| c.subject.id == busy.1.to_string()).unwrap();
    let video_case = recent.cases.iter().find(|c| c.subject.id == video.1.to_string()).unwrap();
    let _: AssignCaseResponse = app
        .call(&format!("{MODERATION}/AssignCase"), Some(&moderator), &AssignCaseRequest { case_id: busy_case.id.clone(), ..Default::default() })
        .await
        .unwrap();
    let _: AssignCaseResponse = app
        .call(
            &format!("{MODERATION}/AssignCase"),
            Some(&moderator),
            &AssignCaseRequest { case_id: video_case.id.clone(), assignee_id: other_moderator.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    let mine = list(ListCasesRequest { assignee: AssigneeFilter::ASSIGNEE_FILTER_MINE.into(), ..Default::default() }).await;
    assert_eq!(subjects(&mine), vec![busy.1.to_string()]);
    let unassigned = list(ListCasesRequest { assignee: AssigneeFilter::ASSIGNEE_FILTER_UNASSIGNED.into(), ..Default::default() }).await;
    assert_eq!(subjects(&unassigned), vec![quiet.1.to_string()]);
    let colleagues = list(ListCasesRequest { assignee_id: other_moderator.id.to_string(), ..Default::default() }).await;
    assert_eq!(subjects(&colleagues), vec![video.1.to_string()]);

    // Resolved cases leave the default (open) list and show under RESOLVED.
    resolve(&app, &moderator, resolution(busy_case, DISMISS, "")).await.unwrap();
    assert_eq!(list(ListCasesRequest::default()).await.cases.len(), 2);
    let resolved = list(ListCasesRequest { status: CaseStatus::CASE_STATUS_RESOLVED.into(), ..Default::default() }).await;
    assert_eq!(subjects(&resolved), vec![busy.1.to_string()]);
}

#[tokio::test]
async fn cases_are_assigned_unassigned_and_audited() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let colleague = app.user_with_roles("colleague", &["moderator"]).await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let (owner, reporter) = (app.user("owner").await, app.user("rep").await);
    let case = open_case(&app, &admin, &moderator, &reporter, &owner).await;
    let assign = |request: AssignCaseRequest| {
        let (app, moderator) = (&app, &moderator);
        async move { app.call::<_, AssignCaseResponse>(&format!("{MODERATION}/AssignCase"), Some(moderator), &request).await }
    };

    let taken = assign(AssignCaseRequest { case_id: case.id.clone(), ..Default::default() }).await.unwrap();
    assert_eq!(taken.moderation_case.assignee.id, moderator.id.to_string());
    assert!(taken.moderation_case.assigned_at.is_set());

    let handed_over = assign(AssignCaseRequest { case_id: case.id.clone(), assignee_id: colleague.id.to_string(), ..Default::default() }).await.unwrap();
    assert_eq!(handed_over.moderation_case.assignee.id, colleague.id.to_string());

    let not_staff = assign(AssignCaseRequest { case_id: case.id.clone(), assignee_id: owner.id.to_string(), ..Default::default() }).await.unwrap_err();
    assert_eq!((not_staff.code.as_str(), not_staff.reason.as_str()), ("invalid_argument", "ASSIGNEE_NOT_MODERATOR"));

    let cleared = assign(AssignCaseRequest { case_id: case.id.clone(), unassign: true, ..Default::default() }).await.unwrap();
    assert!(cleared.moderation_case.assignee.is_unset() && cleared.moderation_case.assigned_at.is_unset());

    let missing = assign(AssignCaseRequest { case_id: Uuid::now_v7().to_string(), ..Default::default() }).await.unwrap_err();
    assert_eq!((missing.code.as_str(), missing.reason.as_str()), ("not_found", "CASE_NOT_FOUND"));
    let malformed = assign(AssignCaseRequest { case_id: "nope".into(), ..Default::default() }).await.unwrap_err();
    assert_eq!(malformed.reason, "INVALID_ID");

    resolve(&app, &moderator, resolution(&case, DISMISS, "")).await.unwrap();
    let closed = assign(AssignCaseRequest { case_id: case.id.clone(), ..Default::default() }).await.unwrap_err();
    assert_eq!((closed.code.as_str(), closed.reason.as_str()), ("failed_precondition", "CASE_ALREADY_RESOLVED"));

    let actions: Vec<String> = audit_by(&app, &boss, moderator.id).await.into_iter().map(|e| e.action).collect();
    assert_eq!(actions, vec!["case.resolved", "case.unassigned", "case.assigned", "case.assigned"]);
}

#[tokio::test]
async fn notes_are_internal_remarks_on_a_case() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let (owner, reporter) = (app.user("owner").await, app.user("rep").await);
    let case = open_case(&app, &admin, &moderator, &reporter, &owner).await;
    let add = |case_id: String, body: &str| {
        let (app, moderator) = (&app, &moderator);
        let request = AddCaseNoteRequest { case_id, body: body.into(), ..Default::default() };
        async move { app.call::<_, AddCaseNoteResponse>(&format!("{MODERATION}/AddCaseNote"), Some(moderator), &request).await }
    };

    let added = add(case.id.clone(), "  前にも同じ投稿で警告済み  ").await.unwrap();
    assert_eq!(added.note.body, "前にも同じ投稿で警告済み");
    assert_eq!(added.note.author.id, moderator.id.to_string());
    add(case.id.clone(), "second").await.unwrap();

    let detail = get_case(&app, &moderator, &case.id).await;
    assert_eq!(detail.notes.iter().map(|n| n.body.as_str()).collect::<Vec<_>>(), vec!["前にも同じ投稿で警告済み", "second"]);

    assert_eq!(add(case.id.clone(), "   ").await.unwrap_err().reason, "NOTE_INVALID");
    assert_eq!(add(case.id.clone(), &"x".repeat(2001)).await.unwrap_err().reason, "NOTE_INVALID");
    assert_eq!(add(Uuid::now_v7().to_string(), "hello").await.unwrap_err().reason, "CASE_NOT_FOUND");
    // Nothing about notes reaches the people involved.
    assert!(notifications_for(&app, owner.id).await.is_empty());
    assert!(notifications_for(&app, reporter.id).await.is_empty());
}

#[tokio::test]
async fn moderation_log_is_filterable_history() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let other_moderator = app.user_with_roles("mod2", &["moderator"]).await;
    let (owner, reporter) = (app.user("owner").await, app.user("rep").await);

    let first = open_case(&app, &admin, &moderator, &reporter, &owner).await;
    resolve(&app, &moderator, resolution(&first, DISMISS, "")).await.unwrap();
    let second = open_case(&app, &admin, &moderator, &reporter, &owner).await;
    resolve(&app, &other_moderator, ResolveCaseRequest { remove_content: true, ..resolution(&second, WARN, "注意") }).await.unwrap();

    let list = |request: ListModerationActionsRequest| {
        let (app, moderator) = (&app, &moderator);
        async move {
            let response: ListModerationActionsResponse = app.call(&format!("{MODERATION}/ListModerationActions"), Some(moderator), &request).await.unwrap();
            response
        }
    };
    let kinds = |response: &ListModerationActionsResponse| response.actions.iter().map(|a| a.kind.as_known().unwrap()).collect::<Vec<_>>();

    // The account's history, newest first.
    let history = list(ListModerationActionsRequest { user_id: owner.id.to_string(), ..Default::default() }).await;
    assert_eq!(kinds(&history), vec![REMOVE_CONTENT, WARN, DISMISS]);
    let warnings = list(ListModerationActionsRequest { user_id: owner.id.to_string(), kind: WARN.into(), ..Default::default() }).await;
    assert_eq!(kinds(&warnings), vec![WARN]);
    let by_colleague = list(ListModerationActionsRequest { moderator_id: other_moderator.id.to_string(), ..Default::default() }).await;
    assert_eq!(kinds(&by_colleague), vec![REMOVE_CONTENT, WARN]);
    let of_case = list(ListModerationActionsRequest { case_id: first.id.clone(), ..Default::default() }).await;
    assert_eq!(kinds(&of_case), vec![DISMISS]);

    let page_one =
        list(ListModerationActionsRequest { user_id: owner.id.to_string(), page: PageRequest { limit: 2, ..Default::default() }.into(), ..Default::default() })
            .await;
    assert_eq!(kinds(&page_one), vec![REMOVE_CONTENT, WARN]);
    assert!(page_one.page.has_more);
    let page_two = list(ListModerationActionsRequest {
        user_id: owner.id.to_string(),
        page: PageRequest { limit: 2, cursor: page_one.page.next_cursor.clone(), ..Default::default() }.into(),
        ..Default::default()
    })
    .await;
    assert_eq!(kinds(&page_two), vec![DISMISS]);
    assert!(!page_two.page.has_more);
}

#[tokio::test]
async fn deleted_accounts_are_scrubbed_from_the_moderation_record() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let (owner, reporter) = (app.user("owner").await, app.user("rep").await);
    let deleted = |user_id: Uuid| Event {
        id: Uuid::now_v7(),
        topic: "identity.user.deleted".into(),
        actor_id: Some(user_id),
        occurred_at: chrono::Utc::now(),
        data: serde_json::json!({ "user_id": user_id }),
    };

    let handled = open_case(&app, &admin, &moderator, &reporter, &owner).await;
    resolve(&app, &moderator, resolution(&handled, WARN, "注意")).await.unwrap();
    let pending = open_case(&app, &admin, &moderator, &reporter, &owner).await;

    // The reporter leaves: their reports stay as numbers, without name or text.
    admin.handle_event(deleted(reporter.id)).await.unwrap();
    let detail = get_case(&app, &moderator, &pending.id).await;
    assert_eq!(detail.moderation_case.report_count, 1);
    assert!(detail.reports[0].reporter.is_unset() && detail.reports[0].text.is_empty());

    // The owner leaves: open cases close, references and titles disappear, the logs remain.
    admin.handle_event(deleted(owner.id)).await.unwrap();
    admin.handle_event(deleted(owner.id)).await.unwrap(); // redelivery is harmless
    let closed = get_case(&app, &moderator, &pending.id).await.moderation_case;
    assert_eq!(closed.status, CaseStatus::CASE_STATUS_RESOLVED);
    assert_eq!(closed.resolution_reason, "account deleted");
    assert!(closed.subject_owner.is_unset() && closed.subject_title.is_empty() && closed.subject_path.is_empty());
    let earlier = get_case(&app, &moderator, &handled.id).await;
    assert_eq!(earlier.moderation_case.resolution, WARN, "an earlier resolution is kept as it was");
    assert_eq!(earlier.actions.len(), 1);
    assert!(earlier.actions[0].target_user.is_unset() && earlier.actions[0].subject_title.is_empty());
    assert_eq!(earlier.actions[0].moderator.id, moderator.id.to_string());

    let audit = audit_by(&app, &boss, moderator.id).await;
    assert!(!audit.is_empty(), "audit rows survive");
    assert!(audit.iter().all(|entry| entry.target_user.is_unset()));

    // A departing moderator is nulled out of the log too, and the rows stay.
    admin.handle_event(deleted(moderator.id)).await.unwrap();
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM moderation_actions WHERE case_id = $1 AND moderator_id IS NULL")
        .bind(handled.id.parse::<Uuid>().unwrap())
        .fetch_one(&app.core.db)
        .await
        .unwrap();
    assert_eq!(rows, 1);
    assert!(audit_by(&app, &boss, moderator.id).await.is_empty(), "no audit row names the deleted moderator any more");
}

/// Records what it was asked to hold and release, on the caller's transaction.
#[derive(Default)]
struct FakeEvidence {
    calls: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl tokuly_admin::ReportEvidence for FakeEvidence {
    async fn hold(&self, conn: &mut sqlx::PgConnection, _subject_type: SubjectType, subject_id: Uuid, holder: &str) -> tokuly_core::AppResult<()> {
        // On the transaction filing the report: the case it names already exists there.
        let case_id = holder.strip_prefix("admin:case:").unwrap().parse::<Uuid>().unwrap();
        let subject: Uuid = sqlx::query_scalar("SELECT subject_id FROM moderation_cases WHERE id = $1").bind(case_id).fetch_one(&mut *conn).await?;
        assert_eq!(subject, subject_id);
        self.calls.lock().unwrap().push(format!("hold {holder}"));
        Ok(())
    }

    async fn release(&self, _conn: &mut sqlx::PgConnection, holder: &str) -> tokuly_core::AppResult<()> {
        self.calls.lock().unwrap().push(format!("release {holder}"));
        Ok(())
    }
}

#[tokio::test]
async fn reported_uploads_are_held_until_long_after_the_case_is_resolved() {
    let (app, admin) = start().await;
    let evidence = std::sync::Arc::new(FakeEvidence::default());
    admin.set_report_evidence(evidence.clone());
    let (moderator, owner) = (app.user_with_roles("mod", &["moderator"]).await, app.user("owner").await);
    let (first, second) = (app.user("rep1").await, app.user("rep2").await);
    let message = (SubjectType::SUBJECT_TYPE_CHAT_MESSAGE, Uuid::now_v7());
    let report = report_event(first.id, message, owner.id, "HARASSMENT", "写真で脅されています");
    admin.handle_event(report.clone()).await.unwrap();
    admin.handle_event(report).await.unwrap();
    admin.handle_event(report_event(second.id, message, owner.id, "HARASSMENT", "")).await.unwrap();
    let case = case_of_owner(&app, &moderator, owner.id).await.remove(0);
    let holder = format!("admin:case:{}", case.id);
    // Held by every report as it is filed (a redelivery files nothing).
    assert_eq!(*evidence.calls.lock().unwrap(), vec![format!("hold {holder}"), format!("hold {holder}")]);

    // Resolving does not let go: the evidence stays for the retention period.
    resolve(&app, &moderator, resolution(&case, WARN, "脅迫")).await.unwrap();
    admin.release_due_evidence().await.unwrap();
    assert_eq!(evidence.calls.lock().unwrap().len(), 2);

    sqlx::query("UPDATE moderation_cases SET resolved_at = now() - interval '91 days' WHERE id = $1::uuid")
        .bind(&case.id)
        .execute(&app.core.db)
        .await
        .unwrap();
    let (a, b) = tokio::join!(admin.release_due_evidence(), admin.release_due_evidence());
    assert!(a.unwrap() + b.unwrap() >= 1);
    admin.release_due_evidence().await.unwrap();
    let released: Vec<String> = evidence.calls.lock().unwrap().iter().filter(|call| call.starts_with("release")).cloned().collect();
    assert_eq!(released, vec![format!("release {holder}")], "released once");
}
