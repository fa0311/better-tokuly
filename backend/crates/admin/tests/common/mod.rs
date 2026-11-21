//! Shared setup for the admin integration tests.
#![allow(dead_code)] // each test binary uses a different subset

use chrono::Utc;
use serde_json::json;
use tokuly_admin::{Admin, AdminConfig, events::ContentRemoved};
use tokuly_core::{events::Event, outbound::NotificationRequest};
use tokuly_proto::{
    admin::*,
    common::{SubjectRef, SubjectType},
};
use tokuly_testkit::{RpcError, TestApp, TestUser};
use uuid::Uuid;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0400", "1600"];

pub const MODERATION: &str = "tokuly.admin.v1.ModerationService";
pub const USERS: &str = "tokuly.admin.v1.UserAdminService";
pub const FLAGS: &str = "tokuly.admin.v1.FeatureFlagService";
pub const ANNOUNCEMENTS: &str = "tokuly.admin.v1.AnnouncementService";
pub const SYSTEM: &str = "tokuly.admin.v1.SystemService";

/// An app with the admin services registered, plus the domain handle for
/// driving event handlers and workers directly.
pub async fn start() -> (TestApp, Admin) {
    start_with(AdminConfig::default()).await
}

pub async fn start_with(cfg: AdminConfig) -> (TestApp, Admin) {
    let mut slot = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        let admin = Admin::new(core.clone(), identity.clone(), cfg).expect("admin config");
        let router = admin.register(router);
        slot = Some(admin);
        router
    })
    .await;
    (app, slot.expect("register ran"))
}

/// Asserts that an RPC refuses anonymous callers and callers without the role.
pub async fn assert_role_gated(app: &TestApp, procedure: &str, outsiders: &[&TestUser]) {
    let anonymous = app.call::<_, serde_json::Value>(procedure, None, &json!({})).await.unwrap_err();
    assert_eq!((anonymous.code.as_str(), anonymous.reason.as_str()), ("unauthenticated", "SIGN_IN_REQUIRED"), "{procedure}");
    for outsider in outsiders {
        let denied = app.call::<_, serde_json::Value>(procedure, Some(outsider), &json!({})).await.unwrap_err();
        assert_eq!((denied.code.as_str(), denied.reason.as_str()), ("permission_denied", "FORBIDDEN"), "{procedure} as {}", outsider.handle);
    }
}

/// An `engagement.report.created` event as the engagement domain emits it.
pub fn report_event(reporter: Uuid, subject: (SubjectType, Uuid), owner: Uuid, category: &str, text: &str) -> Event {
    Event {
        id: Uuid::now_v7(),
        topic: "engagement.report.created".into(),
        actor_id: Some(reporter),
        occurred_at: Utc::now(),
        data: json!({
            "report_id": Uuid::now_v7(),
            "reporter_id": reporter,
            "subject_type": subject.0 as i32,
            "subject_id": subject.1,
            "subject_owner_id": owner,
            "category": category,
            "text": text,
            "subject_title": "今日のランチ",
            "subject_path": format!("/notes/{}", subject.1),
        }),
    }
}

/// Files a report about a fresh note owned by `owner` and returns the case.
pub async fn open_case(app: &TestApp, admin: &Admin, moderator: &TestUser, reporter: &TestUser, owner: &TestUser) -> ModerationCase {
    let subject = (SubjectType::SUBJECT_TYPE_NOTE, Uuid::now_v7());
    admin.handle_event(report_event(reporter.id, subject, owner.id, "SPAM", "宣伝ばかりです")).await.unwrap();
    case_of_owner(app, moderator, owner.id).await.into_iter().find(|c| c.subject.id == subject.1.to_string()).expect("case was opened")
}

/// Open cases about content owned by `owner`.
pub async fn case_of_owner(app: &TestApp, moderator: &TestUser, owner: Uuid) -> Vec<ModerationCase> {
    let response: ListCasesResponse =
        app.call(&format!("{MODERATION}/ListCases"), Some(moderator), &ListCasesRequest { owner_id: owner.to_string(), ..Default::default() }).await.unwrap();
    response.cases
}

pub async fn resolve(app: &TestApp, moderator: &TestUser, request: ResolveCaseRequest) -> Result<ResolveCaseResponse, RpcError> {
    app.call(&format!("{MODERATION}/ResolveCase"), Some(moderator), &request).await
}

/// Notification requests currently in the outbox for `user_id`.
pub async fn notifications_for(app: &TestApp, user_id: Uuid) -> Vec<NotificationRequest> {
    app.events("notify.requested")
        .await
        .into_iter()
        .filter_map(|event| event.data::<NotificationRequest>().ok())
        .filter(|request| request.user_id == user_id)
        .collect()
}

/// Audit entries written by `actor`, newest first.
pub async fn audit_by(app: &TestApp, admin_user: &TestUser, actor: Uuid) -> Vec<AuditEntry> {
    let response: ListAuditLogResponse = app
        .call(&format!("{SYSTEM}/ListAuditLog"), Some(admin_user), &ListAuditLogRequest { actor_id: actor.to_string(), ..Default::default() })
        .await
        .unwrap();
    response.entries
}

/// Whether the account is currently locked by identity.
pub async fn is_banned(app: &TestApp, user_id: Uuid) -> bool {
    sqlx::query_scalar("SELECT banned_at IS NOT NULL FROM users WHERE id = $1").bind(user_id).fetch_one(&app.core.db).await.unwrap()
}

pub const NOTE: SubjectType = SubjectType::SUBJECT_TYPE_NOTE;

pub fn subject_ref(subject_type: SubjectType, id: impl ToString) -> SubjectRef {
    SubjectRef { r#type: subject_type.into(), id: id.to_string(), ..Default::default() }
}

/// A `ResolveCase` request with just the action and the reason.
pub fn resolution(case: &ModerationCase, action: ModerationActionKind, reason: &str) -> ResolveCaseRequest {
    ResolveCaseRequest { case_id: case.id.clone(), action: action.into(), reason: reason.into(), ..Default::default() }
}

pub async fn get_case(app: &TestApp, moderator: &TestUser, case_id: &str) -> GetCaseResponse {
    app.call(&format!("{MODERATION}/GetCase"), Some(moderator), &GetCaseRequest { case_id: case_id.into(), ..Default::default() }).await.unwrap()
}

/// `admin.content.removed` events emitted by `moderator` about `subject_id`.
pub async fn removals(app: &TestApp, moderator: &TestUser, subject_id: &str) -> Vec<ContentRemoved> {
    app.events_by("admin.content.removed", moderator.id)
        .await
        .iter()
        .map(|event| event.data::<ContentRemoved>().unwrap())
        .filter(|removed| removed.subject_id.to_string() == subject_id)
        .collect()
}
