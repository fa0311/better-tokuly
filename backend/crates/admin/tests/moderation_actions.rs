//! Acting on reports: every resolution with its side effects (takedown
//! event, notifications, account locks), who may do what, reopening, and
//! takedowns by reference.
mod common;

use std::sync::Arc;

use async_trait::async_trait;
use common::*;
use tokuly_admin::HiddenOwners;
use tokuly_core::{
    AppResult,
    subject::{SubjectInfo, SubjectResolver},
};
use tokuly_proto::{
    admin::*,
    common::{SubjectRef, SubjectType},
};
use uuid::Uuid;

use ModerationActionKind::{
    MODERATION_ACTION_KIND_BAN as BAN, MODERATION_ACTION_KIND_DISMISS as DISMISS, MODERATION_ACTION_KIND_REMOVE_CONTENT as REMOVE_CONTENT,
    MODERATION_ACTION_KIND_SUSPEND as SUSPEND, MODERATION_ACTION_KIND_WARN as WARN,
};

#[tokio::test]
async fn dismissing_closes_the_case_and_only_tells_the_reporters() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let (owner, alice, bob) = (app.user("owner").await, app.user("alice").await, app.user("bob").await);
    let subject = (NOTE, Uuid::now_v7());
    admin.handle_event(report_event(alice.id, subject, owner.id, "SPAM", "")).await.unwrap();
    admin.handle_event(report_event(bob.id, subject, owner.id, "SPAM", "")).await.unwrap();
    admin.handle_event(report_event(bob.id, subject, owner.id, "OTHER", "")).await.unwrap();
    let case = case_of_owner(&app, &moderator, owner.id).await.remove(0);

    let resolved = resolve(&app, &moderator, ResolveCaseRequest { note: "問題なし".into(), ..resolution(&case, DISMISS, "") }).await.unwrap();
    let closed = &resolved.moderation_case;
    assert_eq!(closed.status, CaseStatus::CASE_STATUS_RESOLVED);
    assert_eq!(closed.resolution, DISMISS);
    assert!(!closed.content_removed && closed.resolved_at.is_set());
    assert_eq!(closed.resolved_by.id, moderator.id.to_string());
    assert_eq!(resolved.actions.len(), 1);
    assert_eq!(resolved.actions[0].kind, DISMISS);
    assert_eq!(resolved.actions[0].note, "問題なし");
    assert_eq!(resolved.actions[0].case_id, case.id);

    // Each reporter hears back once (bob reported twice), in the inbox only.
    for reporter in [&alice, &bob] {
        let notices = notifications_for(&app, reporter.id).await;
        assert_eq!(notices.len(), 1, "{}", reporter.handle);
        assert_eq!(notices[0].kind, "system.report_resolved");
        assert!(notices[0].inbox && !notices[0].push);
        assert_eq!(notices[0].data["action_taken"], false);
    }
    // Every report of the case is settled in one event, without action.
    let settled = app.events_by("admin.report.resolved", moderator.id).await;
    assert_eq!(settled.len(), 1);
    assert_eq!(settled[0].data["action_taken"], false);
    assert_eq!(settled[0].data["report_ids"].as_array().map(Vec::len), Some(3));
    // The owner is left alone and nothing is taken down.
    assert!(notifications_for(&app, owner.id).await.is_empty());
    assert!(removals(&app, &moderator, &subject.1.to_string()).await.is_empty());
    assert!(!is_banned(&app, owner.id).await);

    let again = resolve(&app, &moderator, resolution(&case, DISMISS, "")).await.unwrap_err();
    assert_eq!((again.code.as_str(), again.reason.as_str()), ("failed_precondition", "CASE_ALREADY_RESOLVED"));
}

#[tokio::test]
async fn removing_content_emits_the_takedown_event_and_notifies_owner_and_reporter() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let (owner, reporter) = (app.user("owner").await, app.user("rep").await);
    let case = open_case(&app, &admin, &moderator, &reporter, &owner).await;

    let resolved = resolve(&app, &moderator, resolution(&case, REMOVE_CONTENT, "スパム投稿のため")).await.unwrap();
    assert!(resolved.moderation_case.content_removed);
    assert_eq!(resolved.moderation_case.resolution, REMOVE_CONTENT);
    assert_eq!(resolved.moderation_case.resolution_reason, "スパム投稿のため");
    assert_eq!(resolved.actions.len(), 1);
    let action = &resolved.actions[0];
    assert_eq!(action.kind, REMOVE_CONTENT);
    assert_eq!(action.target_user.id, owner.id.to_string());
    assert_eq!(action.moderator.id, moderator.id.to_string());
    assert_eq!(action.subject.id, case.subject.id);

    // The cross-domain contract: exactly this payload on `admin.content.removed`.
    let events = app.events_by("admin.content.removed", moderator.id).await;
    let event = events.iter().find(|e| e.data["subject_id"] == case.subject.id.as_str()).expect("takedown event");
    assert_eq!(
        event.data,
        serde_json::json!({
            "subject_type": NOTE as i32,
            "subject_id": case.subject.id,
            "reason": "スパム投稿のため",
            "moderator_id": moderator.id,
        })
    );

    // The reports of the case are settled, with action taken.
    let settled = app.events_by("admin.report.resolved", moderator.id).await;
    let settled = settled.last().expect("report resolution event");
    assert_eq!(settled.data["action_taken"], true);
    assert_eq!(settled.data["report_ids"].as_array().map(Vec::len), Some(1));

    let owner_notices = notifications_for(&app, owner.id).await;
    assert_eq!(owner_notices.len(), 1);
    assert_eq!(owner_notices[0].kind, "system.content_removed");
    assert!(owner_notices[0].inbox && owner_notices[0].push);
    assert!(owner_notices[0].body.ja.contains("今日のランチ") && owner_notices[0].body.ja.contains("スパム投稿のため"));
    assert_eq!(owner_notices[0].subject, Some((NOTE as i32, case.subject.id.parse().unwrap())));

    let reporter_notices = notifications_for(&app, reporter.id).await;
    assert_eq!(reporter_notices.len(), 1);
    assert_eq!(reporter_notices[0].data["action_taken"], true);

    let detail = get_case(&app, &moderator, &case.id).await;
    assert_eq!(detail.actions.len(), 1);
    assert_eq!(detail.owner_record.content_removals, 1);
    assert_eq!(detail.owner_record.open_cases, 0);
}

#[tokio::test]
async fn warning_can_also_remove_the_content() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let (owner, reporter) = (app.user("owner").await, app.user("rep").await);
    let case = open_case(&app, &admin, &moderator, &reporter, &owner).await;

    let resolved = resolve(&app, &moderator, ResolveCaseRequest { remove_content: true, ..resolution(&case, WARN, "誹謗中傷はやめましょう") }).await.unwrap();
    assert_eq!(resolved.moderation_case.resolution, WARN);
    assert!(resolved.moderation_case.content_removed);
    assert_eq!(resolved.actions.iter().map(|a| a.kind.as_known().unwrap()).collect::<Vec<_>>(), vec![WARN, REMOVE_CONTENT]);

    let mut kinds: Vec<String> = notifications_for(&app, owner.id).await.into_iter().map(|n| n.kind).collect();
    kinds.sort();
    assert_eq!(kinds, vec!["system.content_removed", "system.warning"]);
    assert_eq!(removals(&app, &moderator, &case.subject.id).await.len(), 1);
    assert!(!is_banned(&app, owner.id).await, "a warning does not lock the account");

    let detail = get_case(&app, &moderator, &case.id).await;
    assert_eq!((detail.owner_record.warnings, detail.owner_record.content_removals, detail.owner_record.suspensions), (1, 1, 0));
}

#[tokio::test]
async fn suspending_and_banning_lock_the_owner_out() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let (owner, reporter) = (app.user("owner").await, app.user("rep").await);

    // A one-day suspension.
    let case = open_case(&app, &admin, &moderator, &reporter, &owner).await;
    let before = chrono::Utc::now();
    let resolved =
        resolve(&app, &moderator, ResolveCaseRequest { suspension_seconds: 86_400, ..resolution(&case, SUSPEND, "繰り返しのスパム") }).await.unwrap();
    assert_eq!(resolved.moderation_case.resolution, SUSPEND);
    assert!(!resolved.moderation_case.content_removed);
    let action = &resolved.actions[0];
    assert_eq!(action.kind, SUSPEND);
    let expires = tokuly_core::time::from_ts(action.expires_at.as_option().unwrap());
    assert!((expires - before - chrono::Duration::days(1)).num_seconds().abs() < 60, "expires about a day from now: {expires}");

    assert!(is_banned(&app, owner.id).await);
    assert_eq!(app.events("identity.user.banned").await.iter().filter(|e| e.data["user_id"] == owner.id.to_string()).count(), 1);
    // Their existing session is gone: any authenticated call is refused.
    let refused = app.call::<_, ListCasesResponse>(&format!("{MODERATION}/ListCases"), Some(&owner), &ListCasesRequest::default()).await.unwrap_err();
    assert_eq!(refused.code, "unauthenticated");
    let (reason, expires_at, case_id): (String, Option<chrono::DateTime<chrono::Utc>>, Option<Uuid>) =
        sqlx::query_as("SELECT reason, expires_at, case_id FROM user_suspensions WHERE user_id = $1 AND ended_at IS NULL")
            .bind(owner.id)
            .fetch_one(&app.core.db)
            .await
            .unwrap();
    assert_eq!(reason, "繰り返しのスパム");
    assert!(expires_at.is_some());
    assert_eq!(case_id.unwrap().to_string(), case.id);

    // A later case escalates to a permanent ban, replacing the suspension.
    let second = open_case(&app, &admin, &moderator, &reporter, &owner).await;
    let banned = resolve(&app, &moderator, ResolveCaseRequest { remove_content: true, ..resolution(&second, BAN, "規約違反の継続") }).await.unwrap();
    assert_eq!(banned.actions.iter().map(|a| a.kind.as_known().unwrap()).collect::<Vec<_>>(), vec![BAN, REMOVE_CONTENT]);
    assert!(banned.actions[0].expires_at.is_unset());
    let active: Vec<(Option<chrono::DateTime<chrono::Utc>>,)> =
        sqlx::query_as("SELECT expires_at FROM user_suspensions WHERE user_id = $1 AND ended_at IS NULL").bind(owner.id).fetch_all(&app.core.db).await.unwrap();
    assert_eq!(active, vec![(None,)], "exactly one active suspension, now permanent");
    let superseded: i64 = sqlx::query_scalar("SELECT count(*) FROM user_suspensions WHERE user_id = $1 AND end_cause = 'superseded'")
        .bind(owner.id)
        .fetch_one(&app.core.db)
        .await
        .unwrap();
    assert_eq!(superseded, 1);

    let detail = get_case(&app, &moderator, &second.id).await;
    assert_eq!(detail.owner_record.suspensions, 2);
}

#[tokio::test]
async fn resolutions_are_validated() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let (owner, reporter) = (app.user("owner").await, app.user("rep").await);
    let case = open_case(&app, &admin, &moderator, &reporter, &owner).await;

    let cases: Vec<(&str, ResolveCaseRequest)> = vec![
        ("ACTION_INVALID", ResolveCaseRequest { case_id: case.id.clone(), reason: "x".into(), ..Default::default() }),
        ("ACTION_INVALID", resolution(&case, ModerationActionKind::MODERATION_ACTION_KIND_UNSUSPEND, "x")),
        ("ACTION_INVALID", ResolveCaseRequest { remove_content: true, ..resolution(&case, DISMISS, "") }),
        ("REASON_REQUIRED", resolution(&case, REMOVE_CONTENT, "  ")),
        ("REASON_REQUIRED", resolution(&case, WARN, "")),
        ("REASON_TOO_LONG", resolution(&case, WARN, &"あ".repeat(501))),
        ("NOTE_INVALID", ResolveCaseRequest { note: "x".repeat(2001), ..resolution(&case, WARN, "x") }),
        ("SUSPENSION_DURATION_INVALID", resolution(&case, SUSPEND, "x")),
        ("SUSPENSION_DURATION_INVALID", ResolveCaseRequest { suspension_seconds: 60, ..resolution(&case, SUSPEND, "x") }),
        ("SUSPENSION_DURATION_INVALID", ResolveCaseRequest { suspension_seconds: 400 * 86_400, ..resolution(&case, SUSPEND, "x") }),
        ("SUSPENSION_DURATION_INVALID", ResolveCaseRequest { suspension_seconds: 86_400, ..resolution(&case, BAN, "x") }),
        ("INVALID_ID", ResolveCaseRequest { case_id: "not-a-uuid".into(), ..resolution(&case, DISMISS, "") }),
        ("CASE_NOT_FOUND", ResolveCaseRequest { case_id: Uuid::now_v7().to_string(), ..resolution(&case, DISMISS, "") }),
    ];
    for (reason, request) in cases {
        let err = resolve(&app, &moderator, request).await.unwrap_err();
        assert_eq!(err.reason, reason, "{err}");
    }
    // None of the rejected requests touched the case.
    assert_eq!(case_of_owner(&app, &moderator, owner.id).await.len(), 1);
    assert!(notifications_for(&app, owner.id).await.is_empty());
}

#[tokio::test]
async fn two_moderators_resolving_at_once_cannot_both_win() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let colleague = app.user_with_roles("colleague", &["moderator"]).await;
    let (owner, reporter) = (app.user("owner").await, app.user("rep").await);
    let case = open_case(&app, &admin, &moderator, &reporter, &owner).await;

    let (first, second) =
        tokio::join!(resolve(&app, &moderator, resolution(&case, REMOVE_CONTENT, "スパム")), resolve(&app, &colleague, resolution(&case, WARN, "注意")));
    let (winners, losers): (Vec<_>, Vec<_>) = [first, second].into_iter().partition(|outcome| outcome.is_ok());
    assert_eq!((winners.len(), losers.len()), (1, 1), "exactly one resolution is applied");
    assert_eq!(losers[0].as_ref().unwrap_err().reason, "CASE_ALREADY_RESOLVED");

    // The loser left no trace: one log entry, one notice to the owner, one to the reporter.
    let detail = get_case(&app, &moderator, &case.id).await;
    assert_eq!(detail.actions.len(), 1);
    assert_eq!(detail.actions[0].kind, detail.moderation_case.resolution.as_known().unwrap());
    assert_eq!(notifications_for(&app, owner.id).await.len(), 1);
    assert_eq!(notifications_for(&app, reporter.id).await.len(), 1);
}

#[tokio::test]
async fn moderators_cannot_judge_themselves_or_sanction_staff() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let colleague = app.user_with_roles("colleague", &["moderator"]).await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let reporter = app.user("rep").await;

    // A case about the moderator's own note: someone else has to handle it.
    let own = open_case(&app, &admin, &moderator, &reporter, &moderator).await;
    let conflict = resolve(&app, &moderator, resolution(&own, DISMISS, "")).await.unwrap_err();
    assert_eq!((conflict.code.as_str(), conflict.reason.as_str()), ("permission_denied", "CONFLICT_OF_INTEREST"));

    // A colleague may dismiss or take the content down, but not sanction a fellow moderator...
    for action in [WARN, SUSPEND, BAN] {
        let seconds = if action == SUSPEND { 3600 } else { 0 };
        let denied = resolve(&app, &colleague, ResolveCaseRequest { suspension_seconds: seconds, ..resolution(&own, action, "x") }).await.unwrap_err();
        assert_eq!((denied.code.as_str(), denied.reason.as_str()), ("permission_denied", "TARGET_IS_STAFF"), "{action:?}");
    }
    assert!(!is_banned(&app, moderator.id).await);
    // ...an admin can.
    let warned = resolve(&app, &boss, resolution(&own, WARN, "モデレーターも例外ではありません")).await.unwrap();
    assert_eq!(warned.actions[0].kind, WARN);
}

#[tokio::test]
async fn a_moderator_is_never_shown_who_reported_them() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let colleague = app.user_with_roles("colleague", &["moderator"]).await;
    let reporter = app.user("rep").await;
    let conflict = |result: Result<serde_json::Value, tokuly_testkit::RpcError>, what: &str| {
        let error = result.expect_err(what);
        assert_eq!((error.code.as_str(), error.reason.as_str()), ("permission_denied", "CONFLICT_OF_INTEREST"), "{what}");
    };

    // A report about the moderator's note, and one about the account itself.
    let about_note = open_case(&app, &admin, &colleague, &reporter, &moderator).await;
    let account = (SubjectType::SUBJECT_TYPE_USER, moderator.id);
    admin.handle_event(report_event(reporter.id, account, moderator.id, "HARASSMENT", "しつこく絡まれています")).await.unwrap();
    let about_account =
        case_of_owner(&app, &colleague, moderator.id).await.into_iter().find(|c| c.subject.id == moderator.id.to_string()).expect("account case");
    for case in [&about_note, &about_account] {
        let _: AddCaseNoteResponse = app
            .call(&format!("{MODERATION}/AddCaseNote"), Some(&colleague), &AddCaseNoteRequest { case_id: case.id.clone(), body: "要確認".into(), ..Default::default() })
            .await
            .unwrap();

        // The colleague sees the reports and notes...
        let seen = get_case(&app, &colleague, &case.id).await;
        assert!(!seen.moderation_case.conflict_of_interest);
        assert_eq!((seen.reports.len(), seen.notes.len()), (1, 1));
        assert_eq!(seen.reports[0].reporter.id, reporter.id.to_string());

        // ...the moderator the case is about sees that it exists, not who reported it or what staff noted.
        let own = get_case(&app, &moderator, &case.id).await;
        assert!(own.moderation_case.conflict_of_interest);
        assert!(own.reports.is_empty() && own.notes.is_empty() && own.owner_record.is_unset());
        let case_id = case.id.clone();
        conflict(
            app.call(&format!("{MODERATION}/ListCaseReports"), Some(&moderator), &ListCaseReportsRequest { case_id: case_id.clone(), ..Default::default() }).await,
            "ListCaseReports",
        );
        conflict(
            app.call(&format!("{MODERATION}/AddCaseNote"), Some(&moderator), &AddCaseNoteRequest { case_id: case_id.clone(), body: "誤解です".into(), ..Default::default() })
                .await,
            "AddCaseNote",
        );
        conflict(
            app.call(&format!("{MODERATION}/AssignCase"), Some(&moderator), &AssignCaseRequest { case_id: case_id.clone(), ..Default::default() }).await,
            "AssignCase",
        );

        // Nor can they reopen it once someone else decided.
        resolve(&app, &colleague, resolution(case, DISMISS, "")).await.unwrap();
        conflict(
            app.call(&format!("{MODERATION}/ReopenCase"), Some(&moderator), &ReopenCaseRequest { case_id, ..Default::default() }).await,
            "ReopenCase",
        );
    }
}

#[tokio::test]
async fn account_cases_cannot_remove_content_and_ownerless_cases_cannot_sanction() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let (owner, reporter) = (app.user("owner").await, app.user("rep").await);

    // A reported profile: the subject is the account itself.
    let profile = (SubjectType::SUBJECT_TYPE_USER, owner.id);
    admin.handle_event(report_event(reporter.id, profile, owner.id, "IMPERSONATION", "なりすまし")).await.unwrap();
    let case = case_of_owner(&app, &moderator, owner.id).await.remove(0);
    for request in [resolution(&case, REMOVE_CONTENT, "x"), ResolveCaseRequest { remove_content: true, ..resolution(&case, WARN, "x") }] {
        let err = resolve(&app, &moderator, request).await.unwrap_err();
        assert_eq!((err.code.as_str(), err.reason.as_str()), ("invalid_argument", "ACTION_NOT_APPLICABLE"));
    }
    // Banning the account is the applicable action.
    resolve(&app, &moderator, resolution(&case, BAN, "なりすまし")).await.unwrap();
    assert!(is_banned(&app, owner.id).await);

    // A case whose owner is unknown (account already erased) can be closed or taken down, not sanctioned.
    let orphan = (NOTE, Uuid::now_v7());
    admin.handle_event(report_event(reporter.id, orphan, Uuid::now_v7(), "SPAM", "")).await.unwrap();
    let orphan_id: Uuid = sqlx::query_scalar("SELECT id FROM moderation_cases WHERE subject_id = $1").bind(orphan.1).fetch_one(&app.core.db).await.unwrap();
    let orphan_case = get_case(&app, &moderator, &orphan_id.to_string()).await.moderation_case.into_option().unwrap();
    assert!(orphan_case.subject_owner.is_unset());
    let err = resolve(&app, &moderator, resolution(&orphan_case, WARN, "x")).await.unwrap_err();
    assert_eq!((err.code.as_str(), err.reason.as_str()), ("failed_precondition", "ACTION_NOT_APPLICABLE"));
    let removed = resolve(&app, &moderator, resolution(&orphan_case, REMOVE_CONTENT, "スパム")).await.unwrap();
    assert!(removed.actions[0].target_user.is_unset());
}

#[tokio::test]
async fn resolved_cases_can_be_reopened_unless_superseded() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let (owner, reporter) = (app.user("owner").await, app.user("rep").await);
    let case = open_case(&app, &admin, &moderator, &reporter, &owner).await;
    let subject = (NOTE, case.subject.id.parse::<Uuid>().unwrap());
    let reopen = |case_id: String| {
        let (app, moderator) = (&app, &moderator);
        let request = ReopenCaseRequest { case_id, note: "判断を見直します".into(), ..Default::default() };
        async move { app.call::<_, ReopenCaseResponse>(&format!("{MODERATION}/ReopenCase"), Some(moderator), &request).await }
    };

    let still_open = reopen(case.id.clone()).await.unwrap_err();
    assert_eq!((still_open.code.as_str(), still_open.reason.as_str()), ("failed_precondition", "CASE_NOT_RESOLVED"));
    assert_eq!(reopen(Uuid::now_v7().to_string()).await.unwrap_err().reason, "CASE_NOT_FOUND");

    resolve(&app, &moderator, resolution(&case, DISMISS, "")).await.unwrap();
    let reopened = reopen(case.id.clone()).await.unwrap().moderation_case;
    assert_eq!(reopened.status, CaseStatus::CASE_STATUS_OPEN);
    assert_eq!(reopened.resolution, ModerationActionKind::MODERATION_ACTION_KIND_UNSPECIFIED);
    assert!(reopened.resolved_by.is_unset() && reopened.resolved_at.is_unset());
    let detail = get_case(&app, &moderator, &case.id).await;
    assert_eq!(detail.notes.last().unwrap().body, "判断を見直します");
    assert_eq!(detail.actions.len(), 1, "the earlier dismissal stays in the log");

    // Resolve again; a new report then opens a second case, which blocks reopening the first.
    resolve(&app, &moderator, resolution(&case, DISMISS, "")).await.unwrap();
    admin.handle_event(report_event(reporter.id, subject, owner.id, "SPAM", "まだ続いています")).await.unwrap();
    let open_now = case_of_owner(&app, &moderator, owner.id).await;
    assert_eq!(open_now.len(), 1);
    assert_ne!(open_now[0].id, case.id, "a report after resolution opens a fresh case");
    assert_eq!(open_now[0].report_count, 1);
    let superseded = reopen(case.id.clone()).await.unwrap_err();
    assert_eq!((superseded.code.as_str(), superseded.reason.as_str()), ("failed_precondition", "CASE_SUPERSEDED"));
}

/// Stands in for the community domain: knows who wrote one anonymous
/// thread, and who else took part in it.
struct FakeAnonymousPosts {
    thread_id: Uuid,
    author_id: Uuid,
    /// People the author mentioned or answered, or who wrote in the thread.
    involved: Vec<Uuid>,
}

#[async_trait]
impl HiddenOwners for FakeAnonymousPosts {
    async fn owner_for_moderation(&self, subject_type: SubjectType, subject_id: Uuid) -> AppResult<Option<Uuid>> {
        Ok((subject_type == SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD && subject_id == self.thread_id).then_some(self.author_id))
    }

    async fn is_party(&self, _subject_type: SubjectType, subject_id: Uuid, user_id: Uuid) -> AppResult<bool> {
        Ok(subject_id == self.thread_id && (user_id == self.author_id || self.involved.contains(&user_id)))
    }
}

const THREAD: SubjectType = SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD;

/// The audit entries about `case_id` named `action`, as `(actor, target user)`.
async fn audit_of_case(app: &tokuly_testkit::TestApp, case_id: &str, action: &str) -> Vec<(Uuid, Option<Uuid>)> {
    sqlx::query_as("SELECT actor_id, target_user_id FROM admin_audit_log WHERE target_type = 'case' AND target_id = $1 AND action = $2 ORDER BY id")
        .bind(case_id)
        .bind(action)
        .fetch_all(&app.core.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn content_reported_without_an_owner_gets_one_from_the_domain_that_keeps_it() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    // The author of the anonymous post happens to be a moderator too.
    let author = app.user_with_roles("author", &["moderator"]).await;
    let reporter = app.user("rep").await;
    let (thread, other_thread) = (Uuid::now_v7(), Uuid::now_v7());
    admin.set_hidden_owners(Arc::new(FakeAnonymousPosts { thread_id: thread, author_id: author.id, involved: vec![] }));

    // Engagement names nobody for an anonymous post: the nil ID.
    admin.handle_event(report_event(reporter.id, (THREAD, thread), Uuid::nil(), "HARASSMENT", "ひどい")).await.unwrap();
    let case_id: Uuid = sqlx::query_scalar("SELECT id FROM moderation_cases WHERE subject_id = $1").bind(thread).fetch_one(&app.core.db).await.unwrap();
    let opened = get_case(&app, &moderator, &case_id.to_string()).await;
    let case = opened.moderation_case.into_option().unwrap();
    assert_eq!(case.subject.id, thread.to_string());
    assert_eq!(case.subject_owner.id, author.id.to_string());
    assert!(!case.owner_withheld && !case.conflict_of_interest);
    assert!(opened.owner_record.is_set(), "the author's moderation record comes with the name");

    // The author cannot judge their own anonymous post; someone else's warning reaches them.
    let own = resolve(&app, &author, resolution(&case, DISMISS, "問題なし")).await.unwrap_err();
    assert_eq!((own.code.as_str(), own.reason.as_str()), ("permission_denied", "CONFLICT_OF_INTEREST"));
    let admin_user = app.user_with_roles("boss", &["admin"]).await;
    let resolved = resolve(&app, &admin_user, ResolveCaseRequest { remove_content: true, ..resolution(&case, WARN, "嫌がらせ") }).await.unwrap();
    let kinds: Vec<String> = notifications_for(&app, author.id).await.into_iter().map(|n| n.kind).collect();
    assert!(kinds.contains(&"system.warning".to_owned()) && kinds.contains(&"system.content_removed".to_owned()), "{kinds:?}");
    // The answer to the resolution does not name the author; the audit log does.
    assert!(resolved.moderation_case.owner_withheld && resolved.moderation_case.subject_owner.is_unset());
    assert!(resolved.actions.iter().all(|action| action.target_user.is_unset() && action.target_withheld));
    assert_eq!(audit_of_case(&app, &case.id, "case.resolved").await, vec![(admin_user.id, Some(author.id))]);

    // What the lookup does not know stays without an owner, as before.
    admin.handle_event(report_event(reporter.id, (THREAD, other_thread), Uuid::nil(), "SPAM", "")).await.unwrap();
    let orphan_id: Uuid =
        sqlx::query_scalar("SELECT id FROM moderation_cases WHERE subject_id = $1").bind(other_thread).fetch_one(&app.core.db).await.unwrap();
    let orphan = get_case(&app, &moderator, &orphan_id.to_string()).await.moderation_case.into_option().unwrap();
    assert!(orphan.subject_owner.is_unset() && !orphan.owner_withheld);

    // A takedown by reference finds the author the same way, with no report at all.
    let (app2, admin2) = start().await;
    let (moderator, writer) = (app2.user_with_roles("mod", &["moderator"]).await, app2.user("writer").await);
    let involved = app2.user_with_roles("involved", &["moderator"]).await;
    let post = Uuid::now_v7();
    admin2.set_hidden_owners(Arc::new(FakeAnonymousPosts { thread_id: post, author_id: writer.id, involved: vec![involved.id] }));
    let request = TakedownContentRequest { subject: subject_ref(THREAD, post).into(), reason: "違法な内容".into(), ..Default::default() };
    // Not by a moderator who took part in the thread.
    let refused = app2.call::<_, TakedownContentResponse>(&format!("{MODERATION}/TakedownContent"), Some(&involved), &request).await.unwrap_err();
    assert_eq!((refused.code.as_str(), refused.reason.as_str()), ("permission_denied", "CONFLICT_OF_INTEREST"));
    assert!(notifications_for(&app2, writer.id).await.is_empty());
    let done: TakedownContentResponse = app2.call(&format!("{MODERATION}/TakedownContent"), Some(&moderator), &request).await.unwrap();
    assert!(done.action.target_user.is_unset() && done.action.target_withheld, "the answer does not name the author");
    assert_eq!(notifications_for(&app2, writer.id).await.len(), 1, "the author is told");
    let logged: Option<Uuid> = sqlx::query_scalar("SELECT target_user_id FROM admin_audit_log WHERE action = 'content.removed' AND actor_id = $1")
        .bind(moderator.id)
        .fetch_one(&app2.core.db)
        .await
        .unwrap();
    assert_eq!(logged, Some(writer.id), "the audit log knows whose post was removed");
}

#[tokio::test]
async fn the_author_of_an_anonymous_post_is_withheld_from_moderators_who_are_involved() {
    let (app, admin) = start().await;
    let author = app.user("author").await;
    // Three moderators: one reported the post, one wrote in the thread, one has nothing to do with it.
    let reporting = app.user_with_roles("reporting", &["moderator"]).await;
    let answered = app.user_with_roles("answered", &["moderator"]).await;
    let neutral = app.user_with_roles("neutral", &["moderator"]).await;
    let thread = Uuid::now_v7();
    admin.set_hidden_owners(Arc::new(FakeAnonymousPosts { thread_id: thread, author_id: author.id, involved: vec![answered.id] }));
    admin.handle_event(report_event(reporting.id, (THREAD, thread), Uuid::nil(), "HARASSMENT", "ひどい")).await.unwrap();
    let case_id: Uuid = sqlx::query_scalar("SELECT id FROM moderation_cases WHERE subject_id = $1").bind(thread).fetch_one(&app.core.db).await.unwrap();
    let case_id = case_id.to_string();

    // No list names the author, whoever asks, and a list by owner does not find the case.
    for staff in [&reporting, &answered, &neutral] {
        let listed: ListCasesResponse = app.call(&format!("{MODERATION}/ListCases"), Some(staff), &ListCasesRequest::default()).await.unwrap();
        let case = listed.cases.iter().find(|case| case.id == case_id).expect("the case is in the queue");
        assert!(case.owner_withheld && case.subject_owner.is_unset() && !case.conflict_of_interest, "{}", staff.handle);
        assert!(case_of_owner(&app, staff, author.id).await.is_empty(), "{}", staff.handle);
    }

    // Whoever reported the post or took part in the thread is told to leave it to someone else.
    for staff in [&reporting, &answered] {
        let opened = get_case(&app, staff, &case_id).await;
        let case = opened.moderation_case.as_option().unwrap();
        assert!(case.owner_withheld && case.conflict_of_interest && case.subject_owner.is_unset(), "{}", staff.handle);
        assert!(opened.owner_record.is_unset(), "{}", staff.handle);
        for request in [resolution(case, DISMISS, ""), resolution(case, WARN, "x"), resolution(case, REMOVE_CONTENT, "x")] {
            let refused = resolve(&app, staff, request).await.unwrap_err();
            assert_eq!((refused.code.as_str(), refused.reason.as_str()), ("permission_denied", "CONFLICT_OF_INTEREST"), "{}", staff.handle);
        }
        // Assigning it to themselves or noting something does not unmask anyone either.
        let assigned: AssignCaseResponse =
            app.call(&format!("{MODERATION}/AssignCase"), Some(staff), &AssignCaseRequest { case_id: case_id.clone(), ..Default::default() }).await.unwrap();
        assert!(assigned.moderation_case.owner_withheld && assigned.moderation_case.subject_owner.is_unset());
    }
    assert!(audit_of_case(&app, &case_id, "case.author_revealed").await.is_empty(), "nobody has been shown the author yet");

    // The moderator who is not involved sees the author, and every look is on record.
    let opened = get_case(&app, &neutral, &case_id).await;
    let case = opened.moderation_case.as_option().unwrap();
    assert_eq!(case.subject_owner.id, author.id.to_string());
    assert!(!case.owner_withheld && !case.conflict_of_interest);
    assert!(opened.owner_record.is_set());
    get_case(&app, &neutral, &case_id).await;
    assert_eq!(audit_of_case(&app, &case_id, "case.author_revealed").await, vec![(neutral.id, Some(author.id)); 2]);
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let entries = audit_by(&app, &boss, neutral.id).await;
    let entry = entries.iter().find(|entry| entry.action == "case.author_revealed").expect("the reveal is in the audit log");
    assert_eq!((entry.target_id.as_str(), entry.created_at.is_set()), (case_id.as_str(), true));

    // And they handle it; once it is resolved the involved ones still learn nothing from it.
    resolve(&app, &neutral, ResolveCaseRequest { remove_content: true, ..resolution(case, WARN, "嫌がらせ") }).await.unwrap();
    let log = |staff: &tokuly_testkit::TestUser, request: ListModerationActionsRequest| {
        let (app, staff) = (&app, staff.clone());
        async move {
            let listed: ListModerationActionsResponse = app.call(&format!("{MODERATION}/ListModerationActions"), Some(&staff), &request).await.unwrap();
            listed.actions
        }
    };
    let of_case = ListModerationActionsRequest { case_id: case_id.clone(), ..Default::default() };
    let of_author = ListModerationActionsRequest { user_id: author.id.to_string(), ..Default::default() };
    let staff_view = |staff: &tokuly_testkit::TestUser| {
        let (app, staff, user_id) = (&app, staff.clone(), author.id.to_string());
        async move {
            let view: GetUserResponse = app.call(&format!("{USERS}/GetUser"), Some(&staff), &GetUserRequest { user_id, ..Default::default() }).await.unwrap();
            view.recent_actions
        }
    };
    for staff in [&reporting, &answered] {
        let closed = get_case(&app, staff, &case_id).await;
        assert!(closed.moderation_case.subject_owner.is_unset() && closed.moderation_case.conflict_of_interest);
        assert_eq!(closed.actions.len(), 2, "{}", staff.handle);
        assert!(closed.actions.iter().all(|action| action.target_user.is_unset() && action.target_withheld), "{}", staff.handle);
        // The moderation log has the warning and the takedown, without whom they hit...
        let entries = log(staff, of_case.clone()).await;
        assert_eq!(entries.len(), 2, "{}", staff.handle);
        assert!(entries.iter().all(|action| action.target_user.is_unset() && action.target_withheld), "{}", staff.handle);
        assert!(log(staff, ListModerationActionsRequest::default()).await.iter().all(|action| action.target_user.is_unset() || action.case_id != case_id));
        // ...and the author's own history does not have them: there they would say whose post it was.
        assert!(log(staff, of_author.clone()).await.is_empty(), "{}", staff.handle);
        assert!(staff_view(staff).await.is_empty(), "{}", staff.handle);
    }
    // The moderator who is not involved reads the log as it is.
    for entries in [log(&neutral, of_case.clone()).await, log(&neutral, of_author.clone()).await, staff_view(&neutral).await] {
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|action| action.target_user.id == author.id.to_string() && !action.target_withheld));
    }

    // A sanction that has nothing to do with the anonymous post is in everyone's view of the account.
    let _: WarnUserResponse = app
        .call(&format!("{USERS}/WarnUser"), Some(&neutral), &WarnUserRequest { user_id: author.id.to_string(), reason: "別件".into(), ..Default::default() })
        .await
        .unwrap();
    for staff in [&reporting, &answered] {
        let seen = staff_view(staff).await;
        assert_eq!(seen.len(), 1, "{}", staff.handle);
        assert_eq!((seen[0].reason.as_str(), seen[0].target_user.id.as_str()), ("別件", author.id.to_string().as_str()));
    }
    assert_eq!(staff_view(&neutral).await.len(), 3);
}

#[tokio::test]
async fn without_a_lookup_nobody_is_shown_a_hidden_author() {
    // A case that was filed with a hidden owner, on a server that no longer has the lookup.
    let (app, admin) = start().await;
    let (moderator, author, reporter) = (app.user_with_roles("mod", &["moderator"]).await, app.user("author").await, app.user("rep").await);
    let thread = Uuid::now_v7();
    admin.handle_event(report_event(reporter.id, (THREAD, thread), Uuid::nil(), "SPAM", "")).await.unwrap();
    sqlx::query("UPDATE moderation_cases SET subject_owner_id = $2, owner_hidden = true WHERE subject_id = $1")
        .bind(thread)
        .bind(author.id)
        .execute(&app.core.db)
        .await
        .unwrap();
    let case_id: Uuid = sqlx::query_scalar("SELECT id FROM moderation_cases WHERE subject_id = $1").bind(thread).fetch_one(&app.core.db).await.unwrap();
    let opened = get_case(&app, &moderator, &case_id.to_string()).await;
    assert!(opened.moderation_case.owner_withheld && opened.moderation_case.conflict_of_interest);
    assert!(opened.moderation_case.subject_owner.is_unset());
}

/// Stands in for a content domain: knows one video by UUID and short ID.
struct FakeVideos {
    id: Uuid,
    owner_id: Uuid,
}

#[async_trait]
impl SubjectResolver for FakeVideos {
    async fn resolve(&self, id: &str, _viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        Ok((id == "dQw4w9WgXcQ" || id == self.id.to_string()).then(|| SubjectInfo {
            id: self.id,
            owner_id: self.owner_id,
            title: "ゲーム実況 #12".into(),
            path: "/watch/dQw4w9WgXcQ".into(),
            image_url: None,
            interactions_enabled: true,
        }))
    }
}

#[tokio::test]
async fn takedown_by_reference_without_a_report() {
    let (app, _admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let owner = app.user("owner").await;
    let video_id = Uuid::now_v7();
    app.core.subjects.register(SubjectType::SUBJECT_TYPE_VIDEO, Arc::new(FakeVideos { id: video_id, owner_id: owner.id }));
    let takedown = |subject: SubjectRef, reason: &str| {
        let (app, moderator) = (&app, &moderator);
        let request = TakedownContentRequest { subject: subject.into(), reason: reason.into(), note: "DMCA #4411".into(), ..Default::default() };
        async move { app.call::<_, TakedownContentResponse>(&format!("{MODERATION}/TakedownContent"), Some(moderator), &request).await }
    };

    // By short ID: the owning domain's resolver supplies the UUID, owner and title.
    let done = takedown(subject_ref(SubjectType::SUBJECT_TYPE_VIDEO, "dQw4w9WgXcQ"), "著作権侵害の申し立て").await.unwrap();
    assert!(done.resolved_case.is_unset());
    assert_eq!(done.action.kind, REMOVE_CONTENT);
    assert_eq!(done.action.subject.id, video_id.to_string());
    assert_eq!(done.action.target_user.id, owner.id.to_string());
    assert_eq!((done.action.subject_title.as_str(), done.action.note.as_str()), ("ゲーム実況 #12", "DMCA #4411"));
    assert!(done.action.case_id.is_empty());
    let events = removals(&app, &moderator, &video_id.to_string()).await;
    assert_eq!(events.len(), 1);
    assert_eq!(
        (events[0].subject_type, events[0].reason.as_str(), events[0].moderator_id),
        (SubjectType::SUBJECT_TYPE_VIDEO as i32, "著作権侵害の申し立て", moderator.id)
    );
    let notices = notifications_for(&app, owner.id).await;
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].kind, "system.content_removed");
    assert!(notices[0].body.ja.contains("ゲーム実況 #12"));

    // By UUID with no resolver for the type and no earlier report: the event still goes out, nobody to notify.
    let unknown_note = Uuid::now_v7();
    let blind = takedown(subject_ref(NOTE, unknown_note), "違法な内容").await.unwrap();
    assert!(blind.action.target_user.is_unset());
    assert_eq!(removals(&app, &moderator, &unknown_note.to_string()).await.len(), 1);

    // Validation.
    let cases: Vec<(&str, &str, Option<SubjectRef>, &str)> = vec![
        ("invalid_argument", "SUBJECT_INVALID", None, "x"),
        ("invalid_argument", "SUBJECT_INVALID", Some(SubjectRef { id: unknown_note.to_string(), ..Default::default() }), "x"),
        ("invalid_argument", "SUBJECT_INVALID", Some(subject_ref(NOTE, "")), "x"),
        ("invalid_argument", "ACTION_NOT_APPLICABLE", Some(subject_ref(SubjectType::SUBJECT_TYPE_USER, owner.id)), "x"),
        ("not_found", "SUBJECT_NOT_FOUND", Some(subject_ref(NOTE, "shortid1234")), "x"),
        ("not_found", "SUBJECT_NOT_FOUND", Some(subject_ref(SubjectType::SUBJECT_TYPE_VIDEO, "unknownShort")), "x"),
        ("invalid_argument", "REASON_REQUIRED", Some(subject_ref(NOTE, unknown_note)), " "),
    ];
    for (code, reason, subject, text) in cases {
        let request = TakedownContentRequest { subject: subject.into(), reason: text.into(), ..Default::default() };
        let err = app.call::<_, TakedownContentResponse>(&format!("{MODERATION}/TakedownContent"), Some(&moderator), &request).await.unwrap_err();
        assert_eq!((err.code.as_str(), err.reason.as_str()), (code, reason), "{err}");
    }
}

#[tokio::test]
async fn takedown_resolves_the_open_case_about_the_subject() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let (owner, reporter) = (app.user("owner").await, app.user("rep").await);
    let case = open_case(&app, &admin, &moderator, &reporter, &owner).await;

    let done: TakedownContentResponse = app
        .call(
            &format!("{MODERATION}/TakedownContent"),
            Some(&moderator),
            &TakedownContentRequest { subject: case.subject.clone(), reason: "規約違反".into(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(done.resolved_case.id, case.id);
    assert_eq!(done.resolved_case.status, CaseStatus::CASE_STATUS_RESOLVED);
    assert_eq!(done.resolved_case.resolution, REMOVE_CONTENT);
    assert_eq!(done.action.case_id, case.id);
    // Owner and title were taken from the case, and the reporter heard back.
    assert_eq!(done.action.target_user.id, owner.id.to_string());
    assert_eq!(notifications_for(&app, owner.id).await.len(), 1);
    assert_eq!(notifications_for(&app, reporter.id).await.len(), 1);
    assert_eq!(removals(&app, &moderator, &case.subject.id).await.len(), 1);

    // A second takedown of the same subject finds no open case; it knows the owner from the old one.
    let again: TakedownContentResponse = app
        .call(
            &format!("{MODERATION}/TakedownContent"),
            Some(&moderator),
            &TakedownContentRequest { subject: case.subject.clone(), reason: "再投稿".into(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(again.resolved_case.is_unset());
    assert_eq!(again.action.target_user.id, owner.id.to_string());
}

/// An anonymous thread by `author` in which `involved` took part, reported by
/// someone else; returns the case's ID.
async fn anonymous_case(app: &tokuly_testkit::TestApp, admin: &tokuly_admin::Admin, author: Uuid, involved: Vec<Uuid>) -> String {
    let thread = Uuid::now_v7();
    admin.set_hidden_owners(Arc::new(FakeAnonymousPosts { thread_id: thread, author_id: author, involved }));
    let reporter = app.user("rep").await;
    admin.handle_event(report_event(reporter.id, (THREAD, thread), Uuid::nil(), "HARASSMENT", "ひどい")).await.unwrap();
    let case_id: Uuid = sqlx::query_scalar("SELECT id FROM moderation_cases WHERE subject_id = $1").bind(thread).fetch_one(&app.core.db).await.unwrap();
    case_id.to_string()
}

async fn staff_view_of(app: &tokuly_testkit::TestApp, staff: &tokuly_testkit::TestUser, user_id: Uuid) -> GetUserResponse {
    app.call(&format!("{USERS}/GetUser"), Some(staff), &GetUserRequest { user_id: user_id.to_string(), ..Default::default() }).await.unwrap()
}

#[tokio::test]
async fn an_involved_moderator_cannot_read_a_hidden_author_off_their_moderation_record() {
    let (app, admin) = start().await;
    let author = app.user("author").await;
    let involved = app.user_with_roles("involved", &["moderator"]).await;
    let neutral = app.user_with_roles("neutral", &["moderator"]).await;
    let reporter = app.user("rep").await;
    // An ordinary case about the author, so their record is on screen anyway.
    let ordinary = open_case(&app, &admin, &neutral, &reporter, &author).await;
    let record = |view: GetUserResponse| view.moderation.into_option().unwrap();
    let before = record(staff_view_of(&app, &involved, author.id).await);
    assert_eq!((before.reports_received, before.open_cases, before.warnings), (1, 1, 0));

    // The anonymous post is reported, then the author is warned and the post removed for it.
    let case_id = anonymous_case(&app, &admin, author.id, vec![involved.id]).await;
    let case = get_case(&app, &neutral, &case_id).await.moderation_case.into_option().unwrap();
    resolve(&app, &neutral, ResolveCaseRequest { remove_content: true, ..resolution(&case, WARN, "嫌がらせ") }).await.unwrap();

    // To the moderator involved in the post nothing about the author moved: not in the
    // staff view of the account, not in the owner record of the ordinary case.
    let after = record(staff_view_of(&app, &involved, author.id).await);
    assert_eq!(after, before);
    assert_eq!(get_case(&app, &involved, &ordinary.id).await.owner_record.into_option().unwrap(), before);
    // A moderator who is not involved reads the record as it is.
    let full = record(staff_view_of(&app, &neutral, author.id).await);
    assert_eq!((full.reports_received, full.open_cases, full.warnings, full.content_removals), (2, 1, 1, 1));
}

#[tokio::test]
async fn an_involved_moderator_is_not_told_which_case_suspended_a_hidden_author() {
    let (app, admin) = start().await;
    let author = app.user("author").await;
    let involved = app.user_with_roles("involved", &["moderator"]).await;
    let neutral = app.user_with_roles("neutral", &["moderator"]).await;
    let case_id = anonymous_case(&app, &admin, author.id, vec![involved.id]).await;
    let case = get_case(&app, &neutral, &case_id).await.moderation_case.into_option().unwrap();
    resolve(&app, &neutral, resolution(&case, BAN, "匿名スレッドでの嫌がらせ")).await.unwrap();

    let suspension = |user: Option<&AdminUser>| user.unwrap().suspension.clone().into_option().expect("suspended");
    let seen = suspension(staff_view_of(&app, &neutral, author.id).await.user.as_option());
    assert_eq!((seen.case_id.as_str(), seen.reason.as_str()), (case_id.as_str(), "匿名スレッドでの嫌がらせ"));

    // The account is suspended for everyone to see, but not for which post, nor why.
    let withheld = suspension(staff_view_of(&app, &involved, author.id).await.user.as_option());
    assert!(withheld.case_id.is_empty() && withheld.reason.is_empty(), "{withheld:?}");
    let searched: SearchUsersResponse =
        app.call(&format!("{USERS}/SearchUsers"), Some(&involved), &SearchUsersRequest { query: author.id.to_string(), ..Default::default() }).await.unwrap();
    let withheld = suspension(searched.users.first());
    assert!(withheld.case_id.is_empty() && withheld.reason.is_empty(), "{withheld:?}");
}

#[tokio::test]
async fn the_audit_log_does_not_name_a_hidden_author_to_an_involved_admin() {
    let (app, admin) = start().await;
    let author = app.user("author").await;
    let involved = app.user_with_roles("involved", &["admin"]).await;
    let neutral = app.user_with_roles("neutral", &["admin"]).await;
    let case_id = anonymous_case(&app, &admin, author.id, vec![involved.id]).await;
    let case = get_case(&app, &neutral, &case_id).await.moderation_case.into_option().unwrap();
    resolve(&app, &neutral, ResolveCaseRequest { remove_content: true, suspension_seconds: 86_400, ..resolution(&case, SUSPEND, "嫌がらせ") }).await.unwrap();

    let audit = |staff: &tokuly_testkit::TestUser, request: ListAuditLogRequest| {
        let (app, staff) = (&app, staff.clone());
        async move {
            let listed: ListAuditLogResponse = app.call(&format!("{SYSTEM}/ListAuditLog"), Some(&staff), &request).await.unwrap();
            listed.entries
        }
    };
    let by_neutral = ListAuditLogRequest { actor_id: neutral.id.to_string(), ..Default::default() };
    // The reveal, the suspension, the takedown and the resolution, each naming the author.
    let full = audit(&neutral, by_neutral.clone()).await;
    let actions: Vec<&str> = full.iter().map(|entry| entry.action.as_str()).collect();
    for action in ["case.author_revealed", "user.suspended", "content.removed", "case.resolved"] {
        assert!(actions.contains(&action), "{actions:?}");
    }
    assert!(full.iter().all(|entry| entry.target_user.id == author.id.to_string() && !entry.target_withheld));

    // The admin who took part in the thread reads the same entries without the author.
    let withheld = audit(&involved, by_neutral).await;
    assert_eq!(withheld.len(), full.len());
    for entry in &withheld {
        assert!(entry.target_user.is_unset() && entry.target_withheld, "{entry:?}");
        assert!(!entry.target_id.contains(&author.id.to_string()), "{entry:?}");
    }
    // And the author's own history has none of them.
    for request in [
        ListAuditLogRequest { target_user_id: author.id.to_string(), ..Default::default() },
        ListAuditLogRequest { target_type: "user".into(), target_id: author.id.to_string(), ..Default::default() },
    ] {
        assert!(audit(&involved, request.clone()).await.is_empty(), "{request:?}");
        assert!(!audit(&neutral, request).await.is_empty());
    }
}

/// Every community thread is an anonymous post by `author`, and nobody is involved.
struct EveryThreadBy {
    author: Uuid,
}

#[async_trait]
impl HiddenOwners for EveryThreadBy {
    async fn owner_for_moderation(&self, subject_type: SubjectType, _subject_id: Uuid) -> AppResult<Option<Uuid>> {
        Ok((subject_type == THREAD).then_some(self.author))
    }

    async fn is_party(&self, _subject_type: SubjectType, _subject_id: Uuid, _user_id: Uuid) -> AppResult<bool> {
        Ok(false)
    }
}

#[tokio::test]
async fn a_moderator_can_unmask_only_so_many_anonymous_authors_an_hour() {
    let (app, admin) = start_with(tokuly_admin::AdminConfig { moderator_reveals_per_hour: 2, ..Default::default() }).await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let (author, reporter) = (app.user("author").await, app.user("rep").await);
    admin.set_hidden_owners(Arc::new(EveryThreadBy { author: author.id }));
    let mut cases = Vec::new();
    for _ in 0..3 {
        let thread = Uuid::now_v7();
        admin.handle_event(report_event(reporter.id, (THREAD, thread), Uuid::nil(), "SPAM", "")).await.unwrap();
        let case_id: Uuid = sqlx::query_scalar("SELECT id FROM moderation_cases WHERE subject_id = $1").bind(thread).fetch_one(&app.core.db).await.unwrap();
        cases.push(case_id.to_string());
    }
    let open = |case_id: &str| {
        let (app, moderator, request) = (&app, moderator.clone(), GetCaseRequest { case_id: case_id.to_owned(), ..Default::default() });
        async move { app.call::<_, GetCaseResponse>(&format!("{MODERATION}/GetCase"), Some(&moderator), &request).await }
    };

    // Opening the same case again does not count twice.
    for case_id in [&cases[0], &cases[0], &cases[1]] {
        let opened = open(case_id).await.unwrap();
        assert_eq!(opened.moderation_case.subject_owner.id, author.id.to_string());
    }
    let refused = open(&cases[2]).await.unwrap_err();
    assert_eq!((refused.code.as_str(), refused.reason.as_str()), ("resource_exhausted", "RATE_LIMITED"));
    assert!(audit_of_case(&app, &cases[2], "case.author_revealed").await.is_empty());
    // Cases already opened, and cases that hide nobody, still open.
    assert!(open(&cases[1]).await.is_ok());
    let ordinary = open_case(&app, &admin, &moderator, &reporter, &author).await;
    assert!(open(&ordinary.id).await.is_ok());
}
