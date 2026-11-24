//! A reported account's プロフ帳, read in full for an open case.
mod common;

use common::*;
use tokuly_proto::{admin::*, common::SubjectType};
use tokuly_testkit::TestUser;

async fn reported_profile(
    app: &tokuly_testkit::TestApp,
    moderator: &TestUser,
    case_id: &str,
) -> Result<GetReportedProfileResponse, tokuly_testkit::RpcError> {
    app.call(
        &format!("{MODERATION}/GetReportedProfile"),
        Some(moderator),
        &GetReportedProfileRequest { case_id: case_id.into(), ..Default::default() },
    )
    .await
}

#[tokio::test]
async fn an_open_case_about_an_account_shows_its_whole_profile_book_on_record() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let admin_user = app.user_with_roles("adm", &["admin"]).await;
    let (owner, reporter) = (app.user("owner").await, app.user("rep").await);
    // Written for nobody but the owner.
    sqlx::query("INSERT INTO profile_extras (user_id, long_bio, long_bio_visibility) VALUES ($1, 'ここだけの話', 5)")
        .bind(owner.id)
        .execute(&app.core.db)
        .await
        .unwrap();

    assert_role_gated(&app, &format!("{MODERATION}/GetReportedProfile"), &[&reporter]).await;

    admin
        .handle_event(report_event(
            reporter.id,
            (SubjectType::SUBJECT_TYPE_USER, owner.id),
            owner.id,
            "HARASSMENT",
            "プロフに悪口",
        ))
        .await
        .unwrap();
    let case = case_of_owner(&app, &moderator, owner.id).await.remove(0);
    let seen = reported_profile(&app, &moderator, &case.id).await.unwrap();
    assert_eq!(seen.extras.as_option().unwrap().long_bio, "ここだけの話");
    assert_eq!(seen.user.as_option().unwrap().id, owner.id.to_string());
    let audit = audit_by(&app, &admin_user, moderator.id).await;
    assert!(audit.iter().any(|entry| entry.action == "case.profile_viewed"), "{audit:?}");

    // Not for content cases, not for a case about oneself, not once resolved.
    let note_case = open_case(&app, &admin, &moderator, &reporter, &owner).await;
    assert_eq!(reported_profile(&app, &moderator, &note_case.id).await.unwrap_err().reason, "NOT_AN_ACCOUNT");
    admin
        .handle_event(report_event(
            reporter.id,
            (SubjectType::SUBJECT_TYPE_USER, moderator.id),
            moderator.id,
            "SPAM",
            "x",
        ))
        .await
        .unwrap();
    let own_id: uuid::Uuid = sqlx::query_scalar("SELECT id FROM moderation_cases WHERE subject_id = $1")
        .bind(moderator.id)
        .fetch_one(&app.core.db)
        .await
        .unwrap();
    assert_eq!(
        reported_profile(&app, &moderator, &own_id.to_string()).await.unwrap_err().reason,
        "CONFLICT_OF_INTEREST"
    );
    resolve(&app, &moderator, resolution(&case, ModerationActionKind::MODERATION_ACTION_KIND_DISMISS, "問題なし"))
        .await
        .unwrap();
    assert_eq!(reported_profile(&app, &moderator, &case.id).await.unwrap_err().reason, "CASE_CLOSED");
}
