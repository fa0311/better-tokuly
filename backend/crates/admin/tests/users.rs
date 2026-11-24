//! Account administration: search, the staff view, sanctions with their
//! automatic expiry, roles, the verified badge and the bootstrap helper.
mod common;

use common::*;
use tokuly_admin::AdminConfig;
use tokuly_core::events::Event;
use tokuly_proto::{admin::*, common::PageRequest};
use tokuly_testkit::{TestApp, TestUser};
use uuid::Uuid;

async fn search(app: &TestApp, staff: &TestUser, request: SearchUsersRequest) -> SearchUsersResponse {
    app.call(&format!("{USERS}/SearchUsers"), Some(staff), &request).await.unwrap()
}

async fn get_user(app: &TestApp, staff: &TestUser, user_id: Uuid) -> GetUserResponse {
    app.call(&format!("{USERS}/GetUser"), Some(staff), &GetUserRequest { user_id: user_id.to_string(), ..Default::default() }).await.unwrap()
}

fn suspend_request(user: &TestUser, seconds: i64) -> SuspendUserRequest {
    SuspendUserRequest { user_id: user.id.to_string(), reason: "スパム行為".into(), suspension_seconds: seconds, ..Default::default() }
}

/// Whether the user's access token is still accepted.
async fn can_call(app: &TestApp, user: &TestUser) -> bool {
    let me: Result<tokuly_proto::admin::ListAnnouncementsResponse, _> =
        app.call(&format!("{ANNOUNCEMENTS}/ListAnnouncements"), Some(user), &ListAnnouncementsRequest::default()).await;
    me.is_ok()
}

#[tokio::test]
async fn user_administration_is_role_gated() {
    let (app, _admin) = start().await;
    let user = app.user("user").await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    for rpc in ["SearchUsers", "GetUser", "WarnUser", "SuspendUser", "UnsuspendUser", "SetUserVerified", "RevokeUserSessions"] {
        assert_role_gated(&app, &format!("{USERS}/{rpc}"), &[&user]).await;
    }
    // Granting roles is for admins only: moderators are refused too.
    assert_role_gated(&app, &format!("{USERS}/SetUserRoles"), &[&user, &moderator]).await;
}

#[tokio::test]
async fn search_matches_handle_name_email_and_id() {
    let (app, _admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let tag = tokuly_core::ids::random_base62(8).to_ascii_lowercase();
    let alice = app.user(&format!("alice{tag}")).await;
    let bob = app.user(&format!("bob{tag}")).await;
    let ids = |response: &SearchUsersResponse| response.users.iter().map(|u| u.user.id.clone()).collect::<Vec<_>>();

    // Part of a handle or display name, case-insensitively, newest account first.
    let by_tag = search(&app, &moderator, SearchUsersRequest { query: tag.to_uppercase(), ..Default::default() }).await;
    assert_eq!(ids(&by_tag), vec![bob.id.to_string(), alice.id.to_string()]);
    let by_handle = search(&app, &moderator, SearchUsersRequest { query: format!("@{}", alice.handle), ..Default::default() }).await;
    assert_eq!(ids(&by_handle), vec![alice.id.to_string()]);

    // Email: the exact address or its beginning.
    let email = format!("{}@test.tokuly.localhost", alice.handle);
    let by_email = search(&app, &moderator, SearchUsersRequest { query: email.clone(), ..Default::default() }).await;
    assert_eq!(ids(&by_email), vec![alice.id.to_string()]);
    assert_eq!(by_email.users[0].email, email);
    assert!(by_email.users[0].email_verified);
    let by_prefix = search(&app, &moderator, SearchUsersRequest { query: format!("{}@test", alice.handle), ..Default::default() }).await;
    assert_eq!(ids(&by_prefix), vec![alice.id.to_string()]);

    let by_id = search(&app, &moderator, SearchUsersRequest { query: bob.id.to_string(), ..Default::default() }).await;
    assert_eq!(ids(&by_id), vec![bob.id.to_string()]);
    assert_eq!(by_id.users[0].state, AccountState::ACCOUNT_STATE_ACTIVE);
    assert_eq!(by_id.users[0].active_session_count, 1);
    assert!(by_id.users[0].last_active_at.is_set());

    // LIKE wildcards in the query are literal.
    assert!(search(&app, &moderator, SearchUsersRequest { query: "%".into(), ..Default::default() }).await.users.is_empty());

    // An empty query lists accounts newest first, a page at a time.
    let first: SearchUsersResponse =
        search(&app, &moderator, SearchUsersRequest { page: PageRequest { limit: 2, ..Default::default() }.into(), ..Default::default() }).await;
    assert_eq!(first.users.len(), 2);
    assert!(first.page.has_more);
    let second = search(
        &app,
        &moderator,
        SearchUsersRequest { page: PageRequest { limit: 2, cursor: first.page.next_cursor.clone(), ..Default::default() }.into(), ..Default::default() },
    )
    .await;
    assert!(ids(&second).iter().all(|id| !ids(&first).contains(id)), "pages do not overlap");
    assert!(ids(&first)[1] > ids(&second)[0], "ordered by ID descending across pages");

    let too_long = app
        .call::<_, SearchUsersResponse>(&format!("{USERS}/SearchUsers"), Some(&moderator), &SearchUsersRequest { query: "x".repeat(101), ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!(too_long.reason, "QUERY_TOO_LONG");
    let bad_role = app
        .call::<_, SearchUsersResponse>(&format!("{USERS}/SearchUsers"), Some(&moderator), &SearchUsersRequest { role: "root".into(), ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!(bad_role.reason, "ROLE_INVALID");
}

#[tokio::test]
async fn search_filters_by_state_role_and_badge() {
    let (app, _admin) = start().await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let tag = tokuly_core::ids::random_base62(8).to_ascii_lowercase();
    let plain = app.user(&format!("plain{tag}")).await;
    let suspended = app.user(&format!("susp{tag}")).await;
    let leaving = app.user(&format!("leaving{tag}")).await;
    let helper = app.user_with_roles(&format!("helper{tag}"), &["staff"]).await;
    let star = app.user(&format!("star{tag}")).await;

    let _: SuspendUserResponse = app.call(&format!("{USERS}/SuspendUser"), Some(&boss), &suspend_request(&suspended, 7200)).await.unwrap();
    sqlx::query("UPDATE users SET deletion_scheduled_at = now() + interval '30 days' WHERE id = $1").bind(leaving.id).execute(&app.core.db).await.unwrap();
    let _: SetUserVerifiedResponse = app
        .call(&format!("{USERS}/SetUserVerified"), Some(&boss), &SetUserVerifiedRequest { user_id: star.id.to_string(), verified: true, ..Default::default() })
        .await
        .unwrap();

    let find = |request: SearchUsersRequest| {
        let (app, boss) = (&app, &boss);
        let request = SearchUsersRequest { query: tag.clone(), ..request };
        async move {
            let mut ids: Vec<String> = search(app, boss, request).await.users.into_iter().map(|u| u.user.id.clone()).collect();
            ids.sort();
            ids
        }
    };
    let sorted = |users: &[&TestUser]| {
        let mut ids: Vec<String> = users.iter().map(|u| u.id.to_string()).collect();
        ids.sort();
        ids
    };

    assert_eq!(find(SearchUsersRequest::default()).await.len(), 5);
    assert_eq!(find(SearchUsersRequest { state: AccountState::ACCOUNT_STATE_ACTIVE.into(), ..Default::default() }).await, sorted(&[&plain, &helper, &star]));
    assert_eq!(find(SearchUsersRequest { state: AccountState::ACCOUNT_STATE_SUSPENDED.into(), ..Default::default() }).await, sorted(&[&suspended]));
    assert_eq!(find(SearchUsersRequest { state: AccountState::ACCOUNT_STATE_DELETION_SCHEDULED.into(), ..Default::default() }).await, sorted(&[&leaving]));
    assert!(find(SearchUsersRequest { state: AccountState::ACCOUNT_STATE_DELETED.into(), ..Default::default() }).await.is_empty());
    assert_eq!(find(SearchUsersRequest { role: "staff".into(), ..Default::default() }).await, sorted(&[&helper]));
    assert_eq!(find(SearchUsersRequest { verified_only: true, ..Default::default() }).await, sorted(&[&star]));
}

#[tokio::test]
async fn staff_view_shows_account_state_and_moderation_record() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let (owner, reporter) = (app.user("owner").await, app.user("rep").await);
    let _extra_device = app.sign_in(owner.id).await;

    let fresh = get_user(&app, &moderator, owner.id).await;
    assert_eq!(fresh.user.user.handle, owner.handle);
    assert_eq!(fresh.user.state, AccountState::ACCOUNT_STATE_ACTIVE);
    assert_eq!(fresh.user.active_session_count, 2);
    assert!(fresh.user.suspension.is_unset() && fresh.user.roles.is_empty());
    assert_eq!(fresh.user.locale, "ja");
    assert_eq!(*fresh.moderation, UserModerationSummary::default());
    assert!(fresh.recent_actions.is_empty());

    // One report against them, resolved with a warning; they filed nothing themselves.
    let case = open_case(&app, &admin, &moderator, &reporter, &owner).await;
    resolve(
        &app,
        &moderator,
        ResolveCaseRequest {
            case_id: case.id.clone(),
            action: ModerationActionKind::MODERATION_ACTION_KIND_WARN.into(),
            reason: "注意".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let after = get_user(&app, &moderator, owner.id).await;
    assert_eq!((after.moderation.reports_received, after.moderation.reports_filed, after.moderation.warnings, after.moderation.open_cases), (1, 0, 1, 0));
    assert_eq!(after.recent_actions.len(), 1);
    assert_eq!(after.recent_actions[0].case_id, case.id);
    assert_eq!(get_user(&app, &moderator, reporter.id).await.moderation.reports_filed, 1);

    let missing = app
        .call::<_, GetUserResponse>(
            &format!("{USERS}/GetUser"),
            Some(&moderator),
            &GetUserRequest { user_id: Uuid::now_v7().to_string(), ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!((missing.code.as_str(), missing.reason.as_str()), ("not_found", "USER_NOT_FOUND"));
}

#[tokio::test]
async fn warning_a_user_notifies_logs_and_audits() {
    let (app, _admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let colleague = app.user_with_roles("colleague", &["moderator"]).await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let user = app.user("user").await;
    let warn = |staff: &TestUser, target: Uuid, reason: &str| {
        let app = &app;
        let staff = staff.clone();
        let request = WarnUserRequest { user_id: target.to_string(), reason: reason.into(), note: "DMでの勧誘".into(), ..Default::default() };
        async move { app.call::<_, WarnUserResponse>(&format!("{USERS}/WarnUser"), Some(&staff), &request).await }
    };

    let warned = warn(&moderator, user.id, "しつこい勧誘はやめましょう").await.unwrap();
    assert_eq!(warned.action.kind, ModerationActionKind::MODERATION_ACTION_KIND_WARN);
    assert_eq!(warned.action.target_user.id, user.id.to_string());
    assert_eq!((warned.action.reason.as_str(), warned.action.note.as_str()), ("しつこい勧誘はやめましょう", "DMでの勧誘"));
    assert!(warned.action.case_id.is_empty() && warned.action.subject.is_unset());

    let notices = notifications_for(&app, user.id).await;
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].kind, "system.warning");
    assert!(notices[0].inbox && notices[0].push && notices[0].body.ja.contains("しつこい勧誘はやめましょう"));
    assert!(!is_banned(&app, user.id).await);

    let audit = audit_by(&app, &boss, moderator.id).await;
    assert_eq!(audit.len(), 1);
    assert_eq!((audit[0].action.as_str(), audit[0].target_type.as_str()), ("user.warned", "user"));
    assert_eq!(audit[0].target_user.id, user.id.to_string());
    assert_eq!(audit[0].reason, "しつこい勧誘はやめましょう");

    assert_eq!(warn(&moderator, user.id, "  ").await.unwrap_err().reason, "REASON_REQUIRED");
    assert_eq!(warn(&moderator, user.id, &"あ".repeat(501)).await.unwrap_err().reason, "REASON_TOO_LONG");
    assert_eq!(warn(&moderator, Uuid::now_v7(), "x").await.unwrap_err().reason, "USER_NOT_FOUND");
    let own = warn(&moderator, moderator.id, "x").await.unwrap_err();
    assert_eq!((own.code.as_str(), own.reason.as_str()), ("permission_denied", "CANNOT_SANCTION_SELF"));
    let staff = warn(&moderator, colleague.id, "x").await.unwrap_err();
    assert_eq!((staff.code.as_str(), staff.reason.as_str()), ("permission_denied", "TARGET_IS_STAFF"));
    warn(&boss, colleague.id, "管理者からの注意").await.unwrap();
}

#[tokio::test]
async fn timed_suspension_locks_the_account_until_the_worker_lifts_it() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let user = app.user("user").await;

    let before = chrono::Utc::now();
    let suspended: SuspendUserResponse = app.call(&format!("{USERS}/SuspendUser"), Some(&moderator), &suspend_request(&user, 3 * 86_400)).await.unwrap();
    assert_eq!(suspended.action.kind, ModerationActionKind::MODERATION_ACTION_KIND_SUSPEND);
    let view = &suspended.user;
    assert_eq!(view.state, AccountState::ACCOUNT_STATE_SUSPENDED);
    assert_eq!(view.active_session_count, 0, "suspension signs the user out everywhere");
    let suspension = view.suspension.as_option().expect("suspension details");
    assert_eq!(suspension.reason, "スパム行為");
    assert_eq!(suspension.moderator.id, moderator.id.to_string());
    assert!(!suspension.permanent && !suspension.id.is_empty() && suspension.case_id.is_empty());
    let expires = tokuly_core::time::from_ts(suspension.expires_at.as_option().unwrap());
    assert!((expires - before - chrono::Duration::days(3)).num_seconds().abs() < 60);
    assert!(is_banned(&app, user.id).await);
    assert!(!can_call(&app, &user).await);

    // Not due yet: the worker leaves it alone.
    admin.expire_due_suspensions().await.unwrap();
    assert!(is_banned(&app, user.id).await);

    // Time passes...
    sqlx::query("UPDATE user_suspensions SET expires_at = now() - interval '1 minute' WHERE user_id = $1").bind(user.id).execute(&app.core.db).await.unwrap();
    assert!(admin.expire_due_suspensions().await.unwrap() >= 1);
    assert!(!is_banned(&app, user.id).await);
    let lifted = get_user(&app, &moderator, user.id).await;
    assert_eq!(lifted.user.state, AccountState::ACCOUNT_STATE_ACTIVE);
    assert!(lifted.user.suspension.is_unset());
    // The automatic lift is in the moderation log without a moderator, and in the audit log without an actor.
    let unsuspend = &lifted.recent_actions[0];
    assert_eq!(unsuspend.kind, ModerationActionKind::MODERATION_ACTION_KIND_UNSUSPEND);
    assert!(unsuspend.moderator.is_unset());
    let audit: ListAuditLogResponse = app
        .call(&format!("{SYSTEM}/ListAuditLog"), Some(&boss), &ListAuditLogRequest { target_user_id: user.id.to_string(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(audit.entries.iter().map(|e| e.action.as_str()).collect::<Vec<_>>(), vec!["user.suspension_expired", "user.suspended"]);
    assert!(audit.entries[0].actor.is_unset());
    assert_eq!(app.events("identity.user.unbanned").await.iter().filter(|e| e.data["user_id"] == user.id.to_string()).count(), 1);
    // They can sign in again.
    assert!(can_call(&app, &app.sign_in(user.id).await).await);

    // A second pass has nothing left to do for this account.
    admin.expire_due_suspensions().await.unwrap();
    assert_eq!(get_user(&app, &moderator, user.id).await.recent_actions.len(), 2);
}

#[tokio::test]
async fn bans_are_permanent_replace_suspensions_and_can_be_lifted() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let user = app.user("user").await;
    let suspend = |request: SuspendUserRequest| {
        let (app, moderator) = (&app, &moderator);
        async move { app.call::<_, SuspendUserResponse>(&format!("{USERS}/SuspendUser"), Some(moderator), &request).await }
    };
    let unsuspend = |user_id: Uuid| {
        let (app, moderator) = (&app, &moderator);
        let request = UnsuspendUserRequest { user_id: user_id.to_string(), note: "異議申し立てを認めました".into(), ..Default::default() };
        async move { app.call::<_, UnsuspendUserResponse>(&format!("{USERS}/UnsuspendUser"), Some(moderator), &request).await }
    };

    // Exactly one of "permanent" and a duration.
    for bad in [
        suspend_request(&user, 0),
        suspend_request(&user, 59),
        suspend_request(&user, 366 * 86_400),
        SuspendUserRequest { permanent: true, ..suspend_request(&user, 3600) },
    ] {
        assert_eq!(suspend(bad).await.unwrap_err().reason, "SUSPENSION_DURATION_INVALID");
    }
    assert_eq!(suspend(SuspendUserRequest { reason: String::new(), ..suspend_request(&user, 3600) }).await.unwrap_err().reason, "REASON_REQUIRED");
    assert_eq!(suspend(SuspendUserRequest { user_id: Uuid::now_v7().to_string(), ..suspend_request(&user, 3600) }).await.unwrap_err().reason, "USER_NOT_FOUND");
    assert_eq!(suspend(suspend_request(&moderator, 3600)).await.unwrap_err().reason, "CANNOT_SANCTION_SELF");
    assert!(!is_banned(&app, user.id).await, "rejected requests changed nothing");
    assert_eq!(unsuspend(user.id).await.unwrap_err().reason, "USER_NOT_SUSPENDED");

    // A timed suspension, then a ban on top of it.
    suspend(suspend_request(&user, 3600)).await.unwrap();
    let banned = suspend(SuspendUserRequest { permanent: true, ..suspend_request(&user, 0) }).await.unwrap();
    assert_eq!(banned.action.kind, ModerationActionKind::MODERATION_ACTION_KIND_BAN);
    let suspension = banned.user.suspension.as_option().unwrap();
    assert!(suspension.permanent && suspension.expires_at.is_unset());
    // A ban never expires.
    admin.expire_due_suspensions().await.unwrap();
    assert!(is_banned(&app, user.id).await);

    let lifted = unsuspend(user.id).await.unwrap();
    assert_eq!(lifted.user.state, AccountState::ACCOUNT_STATE_ACTIVE);
    assert!(!is_banned(&app, user.id).await);
    let causes: Vec<String> =
        sqlx::query_scalar("SELECT end_cause FROM user_suspensions WHERE user_id = $1 ORDER BY id").bind(user.id).fetch_all(&app.core.db).await.unwrap();
    assert_eq!(causes, vec!["superseded", "lifted"]);
    let history = get_user(&app, &moderator, user.id).await;
    let kinds: Vec<_> = history.recent_actions.iter().map(|a| a.kind.as_known().unwrap()).collect();
    assert_eq!(
        kinds,
        vec![
            ModerationActionKind::MODERATION_ACTION_KIND_UNSUSPEND,
            ModerationActionKind::MODERATION_ACTION_KIND_BAN,
            ModerationActionKind::MODERATION_ACTION_KIND_SUSPEND
        ]
    );
    assert_eq!(history.recent_actions[0].moderator.id, moderator.id.to_string());
    assert_eq!(history.recent_actions[0].note, "異議申し立てを認めました");
    assert_eq!(history.moderation.suspensions, 2);
    assert_eq!(unsuspend(user.id).await.unwrap_err().reason, "USER_NOT_SUSPENDED");
}

#[tokio::test]
async fn only_admins_touch_staff_accounts() {
    let (app, _admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let colleague = app.user_with_roles("colleague", &["moderator"]).await;
    let boss = app.user_with_roles("boss", &["admin"]).await;

    let denied = app.call::<_, SuspendUserResponse>(&format!("{USERS}/SuspendUser"), Some(&moderator), &suspend_request(&colleague, 3600)).await.unwrap_err();
    assert_eq!((denied.code.as_str(), denied.reason.as_str()), ("permission_denied", "TARGET_IS_STAFF"));
    let denied = app.call::<_, SuspendUserResponse>(&format!("{USERS}/SuspendUser"), Some(&moderator), &suspend_request(&boss, 3600)).await.unwrap_err();
    assert_eq!(denied.reason, "TARGET_IS_STAFF");
    let denied = app
        .call::<_, RevokeUserSessionsResponse>(
            &format!("{USERS}/RevokeUserSessions"),
            Some(&moderator),
            &RevokeUserSessionsRequest { user_id: colleague.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!(denied.reason, "TARGET_IS_STAFF");

    // The admin suspends the moderator; another moderator cannot undo that, the admin can.
    let _: SuspendUserResponse = app.call(&format!("{USERS}/SuspendUser"), Some(&boss), &suspend_request(&colleague, 3600)).await.unwrap();
    let unsuspend = UnsuspendUserRequest { user_id: colleague.id.to_string(), ..Default::default() };
    let denied = app.call::<_, UnsuspendUserResponse>(&format!("{USERS}/UnsuspendUser"), Some(&moderator), &unsuspend).await.unwrap_err();
    assert_eq!(denied.reason, "TARGET_IS_STAFF");
    let _: UnsuspendUserResponse = app.call(&format!("{USERS}/UnsuspendUser"), Some(&boss), &unsuspend).await.unwrap();
    assert!(!is_banned(&app, colleague.id).await);
}

#[tokio::test]
async fn verified_badge_is_granted_and_removed() {
    let (app, _admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let user = app.user("user").await;
    let set = |verified: bool| {
        let (app, moderator) = (&app, &moderator);
        let request = SetUserVerifiedRequest { user_id: user.id.to_string(), verified, note: "本人確認済み".into(), ..Default::default() };
        async move { app.call::<_, SetUserVerifiedResponse>(&format!("{USERS}/SetUserVerified"), Some(moderator), &request).await.unwrap() }
    };

    assert!(set(true).await.user.user.verified);
    assert!(!set(false).await.user.user.verified);
    assert!(!app.events("identity.user.updated").await.iter().filter(|e| e.data["user_id"] == user.id.to_string()).collect::<Vec<_>>().is_empty());

    let audit = audit_by(&app, &boss, moderator.id).await;
    assert_eq!(audit.len(), 2);
    assert_eq!(audit[1].action, "user.verified_changed");
    assert_eq!((audit[1].before_json.as_str(), audit[1].after_json.as_str()), (r#"{"verified":false}"#, r#"{"verified":true}"#));
    assert_eq!(audit[1].reason, "本人確認済み");

    let missing = app
        .call::<_, SetUserVerifiedResponse>(
            &format!("{USERS}/SetUserVerified"),
            Some(&moderator),
            &SetUserVerifiedRequest { user_id: Uuid::now_v7().to_string(), verified: true, ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!(missing.reason, "USER_NOT_FOUND");
}

#[tokio::test]
async fn moderators_cannot_verify_themselves_or_other_staff() {
    let (app, _admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let colleague = app.user_with_roles("colleague", &["moderator"]).await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let set = |by: &TestUser, whom: &TestUser| {
        let (app, by) = (&app, by.clone());
        let request = SetUserVerifiedRequest { user_id: whom.id.to_string(), verified: true, ..Default::default() };
        async move { app.call::<_, SetUserVerifiedResponse>(&format!("{USERS}/SetUserVerified"), Some(&by), &request).await }
    };
    for whom in [&moderator, &colleague, &boss] {
        let refused = set(&moderator, whom).await.unwrap_err();
        assert_eq!((refused.code.as_str(), refused.reason.as_str()), ("permission_denied", "TARGET_IS_STAFF"), "{}", whom.handle);
    }
    let verified: bool = sqlx::query_scalar("SELECT verified FROM users WHERE id = $1").bind(moderator.id).fetch_one(&app.core.db).await.unwrap();
    assert!(!verified);
    // An admin decides about staff badges.
    assert!(set(&boss, &moderator).await.unwrap().user.user.verified);
}

#[tokio::test]
async fn admins_grant_and_remove_roles() {
    let (app, _admin) = start().await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let user = app.user("user").await;
    let set_roles = |target: Uuid, roles: &[&str]| {
        let (app, boss) = (&app, &boss);
        let request = SetUserRolesRequest { user_id: target.to_string(), roles: roles.iter().map(|r| r.to_string()).collect(), ..Default::default() };
        async move { app.call::<_, SetUserRolesResponse>(&format!("{USERS}/SetUserRoles"), Some(boss), &request).await }
    };

    // Granting: stored sorted and de-duplicated; the user's current session keeps working.
    let granted = set_roles(user.id, &["staff", "moderator", "staff"]).await.unwrap();
    assert_eq!(granted.user.roles, vec!["moderator", "staff"]);
    assert!(can_call(&app, &user).await);
    // The new role is in tokens issued from now on.
    let as_moderator = app.sign_in(user.id).await;
    let _: ListCasesResponse = app.call(&format!("{MODERATION}/ListCases"), Some(&as_moderator), &ListCasesRequest::default()).await.unwrap();

    // Removing a role signs the account out, so the old token's role stops working at once.
    let demoted = set_roles(user.id, &["staff"]).await.unwrap();
    assert_eq!(demoted.user.roles, vec!["staff"]);
    assert_eq!(demoted.user.active_session_count, 0);
    let stale = app.call::<_, ListCasesResponse>(&format!("{MODERATION}/ListCases"), Some(&as_moderator), &ListCasesRequest::default()).await.unwrap_err();
    assert_eq!(stale.code, "unauthenticated");

    assert_eq!(set_roles(user.id, &["superuser"]).await.unwrap_err().reason, "ROLE_INVALID");
    assert_eq!(set_roles(Uuid::now_v7(), &["staff"]).await.unwrap_err().reason, "USER_NOT_FOUND");

    // An admin cannot drop their own admin role, but may change their other roles.
    let own = set_roles(boss.id, &["moderator"]).await.unwrap_err();
    assert_eq!((own.code.as_str(), own.reason.as_str()), ("permission_denied", "CANNOT_DEMOTE_SELF"));
    assert_eq!(set_roles(boss.id, &[]).await.unwrap_err().reason, "CANNOT_DEMOTE_SELF");
    let kept = set_roles(boss.id, &["admin", "staff"]).await.unwrap();
    assert_eq!(kept.user.roles, vec!["admin", "staff"]);
    set_roles(boss.id, &["admin"]).await.unwrap();
    assert!(can_call(&app, &boss).await, "changing your own roles does not sign you out");

    let audit = audit_by(&app, &boss, boss.id).await;
    let first = audit.last().unwrap();
    assert_eq!(first.action, "user.roles_changed");
    assert_eq!((first.before_json.as_str(), first.after_json.as_str()), (r#"{"roles":[]}"#, r#"{"roles":["moderator","staff"]}"#));
}

#[tokio::test]
async fn revoking_sessions_signs_the_user_out_everywhere() {
    let (app, _admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let user = app.user("user").await;
    let second_device = app.sign_in(user.id).await;

    let revoked: RevokeUserSessionsResponse = app
        .call(
            &format!("{USERS}/RevokeUserSessions"),
            Some(&moderator),
            &RevokeUserSessionsRequest { user_id: user.id.to_string(), note: "アカウント乗っ取りの報告".into(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(revoked.revoked_count, 2);
    assert!(!can_call(&app, &user).await && !can_call(&app, &second_device).await);
    assert!(!is_banned(&app, user.id).await, "the account itself stays usable");
    assert!(can_call(&app, &app.sign_in(user.id).await).await);

    let missing = app
        .call::<_, RevokeUserSessionsResponse>(
            &format!("{USERS}/RevokeUserSessions"),
            Some(&moderator),
            &RevokeUserSessionsRequest { user_id: Uuid::now_v7().to_string(), ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!(missing.reason, "USER_NOT_FOUND");
}

#[tokio::test]
async fn ensure_admin_bootstraps_the_first_admin() {
    let (app, admin) = start().await;
    let founder = app.user("founder").await;
    let helper = app.user_with_roles("helper", &["staff"]).await;

    assert!(!admin.ensure_admin("nobody-by-this-name").await.unwrap(), "unknown accounts are skipped");
    assert!(!admin.ensure_admin("  ").await.unwrap());

    assert!(admin.ensure_admin(&format!("@{}", founder.handle)).await.unwrap());
    assert!(!admin.ensure_admin(&founder.handle).await.unwrap(), "already an admin: nothing to do");
    // By email, keeping the roles the account already has.
    assert!(admin.ensure_admin(&format!("{}@test.tokuly.localhost", helper.handle)).await.unwrap());
    let roles: Vec<String> = sqlx::query_scalar("SELECT roles FROM users WHERE id = $1").bind(helper.id).fetch_one(&app.core.db).await.unwrap();
    assert_eq!(roles, vec!["staff", "admin"]);

    // The new admin can use admin RPCs after signing in, and the grant is on record without an actor.
    let founder = app.sign_in(founder.id).await;
    let audit: ListAuditLogResponse = app
        .call(&format!("{SYSTEM}/ListAuditLog"), Some(&founder), &ListAuditLogRequest { target_user_id: founder.id.to_string(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(audit.entries.len(), 1);
    assert_eq!(audit.entries[0].action, "user.roles_changed");
    assert!(audit.entries[0].actor.is_unset());
    assert_eq!(audit.entries[0].after_json, r#"{"roles":["admin"]}"#);
}

/// `identity.user.created` as identity emits it.
fn user_created(user: &TestUser) -> Event {
    Event {
        id: Uuid::now_v7(),
        topic: "identity.user.created".into(),
        actor_id: Some(user.id),
        occurred_at: chrono::Utc::now(),
        data: serde_json::json!({ "user_id": user.id, "handle": user.handle, "invite_code": "" }),
    }
}

#[tokio::test]
async fn the_configured_owner_becomes_admin_when_they_sign_up() {
    let owner_email = format!("owner-{}@deploy.test", Uuid::now_v7().simple());
    let (app, admin) = start_with(AdminConfig { bootstrap_admin: owner_email.to_uppercase(), ..Default::default() }).await;
    let roles_of = async |user_id: Uuid| -> Vec<String> {
        sqlx::query_scalar("SELECT roles FROM users WHERE id = $1").bind(user_id).fetch_one(&app.core.db).await.unwrap()
    };

    // Nobody has that address yet: startup grants nothing, and other people signing up are not promoted.
    assert!(!admin.grant_bootstrap_admin().await.unwrap());
    let stranger = app.user("stranger").await;
    admin.handle_event(user_created(&stranger)).await.unwrap();
    assert!(roles_of(stranger.id).await.is_empty());

    // The owner signs up: the worker promotes them, once.
    let owner = app.user("owner").await;
    sqlx::query("UPDATE users SET email = $2::citext WHERE id = $1").bind(owner.id).bind(&owner_email).execute(&app.core.db).await.unwrap();
    admin.handle_event(user_created(&owner)).await.unwrap();
    assert_eq!(roles_of(owner.id).await, vec!["admin"]);
    admin.handle_event(user_created(&owner)).await.unwrap();
    assert_eq!(roles_of(owner.id).await, vec!["admin"], "a redelivered event changes nothing");
    assert!(roles_of(stranger.id).await.is_empty());

    // Without the setting nothing is ever granted.
    let (_, unconfigured) = start().await;
    assert!(!unconfigured.grant_bootstrap_admin().await.unwrap());
}

#[tokio::test]
async fn racing_bootstraps_grant_and_record_once() {
    let address = format!("racer-{}@deploy.test", Uuid::now_v7().simple());
    let (app, admin) = start_with(AdminConfig { bootstrap_admin: address.clone(), ..Default::default() }).await;
    let owner = app.user("racer").await;
    sqlx::query("UPDATE users SET email = $2::citext WHERE id = $1").bind(owner.id).bind(&address).execute(&app.core.db).await.unwrap();
    let results = futures::future::join_all((0..4).map(|_| admin.grant_bootstrap_admin())).await;
    assert_eq!(results.iter().filter(|r| matches!(r, Ok(true))).count(), 1, "{results:?}");
    let (records, grants): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE action = 'admin.bootstrapped'), count(*) FILTER (WHERE action = 'user.roles_changed') FROM admin_audit_log WHERE target_user_id = $1",
    )
    .bind(owner.id)
    .fetch_one(&app.core.db)
    .await
    .unwrap();
    assert_eq!((records, grants), (1, 1), "the record and the grant come together, once");
}

/// `identity.user.email_verified` as identity emits it.
fn email_verified(user: &TestUser) -> Event {
    Event { topic: "identity.user.email_verified".into(), data: serde_json::json!({ "user_id": user.id }), ..user_created(user) }
}

#[tokio::test]
async fn the_bootstrap_admin_needs_the_verified_address_and_happens_once() {
    let address = format!("founder-{}@deploy.test", Uuid::now_v7().simple());
    let (app, admin) = start_with(AdminConfig { bootstrap_admin: address.clone(), ..Default::default() }).await;
    let roles_of = async |user_id: Uuid| -> Vec<String> {
        sqlx::query_scalar("SELECT roles FROM users WHERE id = $1").bind(user_id).fetch_one(&app.core.db).await.unwrap()
    };
    // Someone signs up with the address but cannot open its mail: no admin.
    let squatter = app.user("squatter").await;
    sqlx::query("UPDATE users SET email = $2::citext, email_verified_at = NULL WHERE id = $1").bind(squatter.id).bind(&address).execute(&app.core.db).await.unwrap();
    admin.handle_event(user_created(&squatter)).await.unwrap();
    assert!(!admin.grant_bootstrap_admin().await.unwrap());
    assert!(roles_of(squatter.id).await.is_empty());

    // The real owner gets the address and confirms it.
    sqlx::query("UPDATE users SET email = NULL WHERE id = $1").bind(squatter.id).execute(&app.core.db).await.unwrap();
    let owner = app.user("founder").await;
    sqlx::query("UPDATE users SET email = $2::citext, email_verified_at = NULL WHERE id = $1").bind(owner.id).bind(&address).execute(&app.core.db).await.unwrap();
    admin.handle_event(user_created(&owner)).await.unwrap();
    assert!(roles_of(owner.id).await.is_empty(), "not before the address is verified");
    sqlx::query("UPDATE users SET email_verified_at = now() WHERE id = $1").bind(owner.id).execute(&app.core.db).await.unwrap();
    admin.handle_event(email_verified(&owner)).await.unwrap();
    assert_eq!(roles_of(owner.id).await, vec!["admin"]);

    // Demoted, and the address passes to someone else: nobody is promoted again.
    sqlx::query("UPDATE users SET roles = '{}', email = NULL WHERE id = $1").bind(owner.id).execute(&app.core.db).await.unwrap();
    let next = app.user("next").await;
    sqlx::query("UPDATE users SET email = $2::citext WHERE id = $1").bind(next.id).bind(&address).execute(&app.core.db).await.unwrap();
    admin.handle_event(email_verified(&next)).await.unwrap();
    assert!(!admin.grant_bootstrap_admin().await.unwrap());
    assert!(roles_of(next.id).await.is_empty() && roles_of(owner.id).await.is_empty());

    // A handle is not an address anyone proved: it promotes nobody.
    let (_, by_handle) = start_with(AdminConfig { bootstrap_admin: next.handle.clone(), ..Default::default() }).await;
    assert!(!by_handle.grant_bootstrap_admin().await.unwrap());
    assert!(roles_of(next.id).await.is_empty());
}
