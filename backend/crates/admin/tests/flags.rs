//! Feature flag administration against the table core evaluates.
mod common;

use common::*;
use tokuly_proto::{admin::*, common::PageRequest};
use tokuly_testkit::{TestApp, TestUser};
use uuid::Uuid;

/// A flag name no other test uses (they share the table).
fn unique_flag(prefix: &str) -> String {
    format!("{prefix}.{}", tokuly_core::ids::random_base62(10).to_ascii_lowercase())
}

async fn create(app: &TestApp, boss: &TestUser, request: CreateFlagRequest) -> FeatureFlag {
    let response: CreateFlagResponse = app.call(&format!("{FLAGS}/CreateFlag"), Some(boss), &request).await.unwrap();
    response.flag.into_option().unwrap()
}

async fn update(app: &TestApp, boss: &TestUser, request: UpdateFlagRequest) -> FeatureFlag {
    let response: UpdateFlagResponse = app.call(&format!("{FLAGS}/UpdateFlag"), Some(boss), &request).await.unwrap();
    response.flag.into_option().unwrap()
}

#[tokio::test]
async fn feature_flags_are_for_admins_only() {
    let (app, _admin) = start().await;
    let user = app.user("user").await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    for rpc in ["ListFlags", "CreateFlag", "UpdateFlag", "DeleteFlag", "EvaluateFlags"] {
        assert_role_gated(&app, &format!("{FLAGS}/{rpc}"), &[&user, &moderator]).await;
    }
}

#[tokio::test]
async fn flag_lifecycle_takes_effect_immediately() {
    let (app, _admin) = start().await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let (insider, outsider) = (app.user("insider").await, app.user("outsider").await);
    let name = unique_flag("new_map");

    // Created switched off, with one allow-listed user and the default 100 % rollout.
    let flag = create(
        &app,
        &boss,
        CreateFlagRequest {
            name: format!("  {name} "),
            description: "新しいマップ表示".into(),
            allow_user_ids: vec![insider.id.to_string()],
            ..Default::default()
        },
    )
    .await;
    assert_eq!((flag.name.as_str(), flag.description.as_str(), flag.enabled, flag.rollout_percent), (name.as_str(), "新しいマップ表示", false, 100));
    assert_eq!(flag.allow_users.iter().map(|u| u.id.as_str()).collect::<Vec<_>>(), vec![insider.id.to_string()]);
    assert!(flag.updated_at.is_set());
    // Core sees it right away: allow-listed users get a disabled flag, others do not.
    assert!(app.core.flags.enabled(&name, Some(insider.id)).await);
    assert!(!app.core.flags.enabled(&name, Some(outsider.id)).await);

    // Enable for everyone.
    let flag = update(&app, &boss, UpdateFlagRequest { name: name.clone(), enabled: Some(true), ..Default::default() }).await;
    assert!(flag.enabled);
    assert_eq!(flag.description, "新しいマップ表示", "fields that are not sent stay as they are");
    assert!(app.core.flags.enabled(&name, Some(outsider.id)).await);
    assert!(app.core.flags.enabled(&name, None).await);

    // Roll back to 0 %, swap the allow-list.
    let flag = update(
        &app,
        &boss,
        UpdateFlagRequest {
            name: name.clone(),
            rollout_percent: Some(0),
            description: Some("段階的に公開中".into()),
            add_allow_user_ids: vec![outsider.id.to_string(), outsider.id.to_string()],
            remove_allow_user_ids: vec![insider.id.to_string()],
            ..Default::default()
        },
    )
    .await;
    assert_eq!((flag.rollout_percent, flag.description.as_str()), (0, "段階的に公開中"));
    assert_eq!(flag.allow_users.iter().map(|u| u.id.as_str()).collect::<Vec<_>>(), vec![outsider.id.to_string()]);
    assert!(app.core.flags.enabled(&name, Some(outsider.id)).await);
    assert!(!app.core.flags.enabled(&name, Some(insider.id)).await);

    // Listed by name fragment.
    let listed: ListFlagsResponse =
        app.call(&format!("{FLAGS}/ListFlags"), Some(&boss), &ListFlagsRequest { query: name.to_uppercase(), ..Default::default() }).await.unwrap();
    assert_eq!(listed.flags.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), vec![name.as_str()]);

    // Deleted flags are off for everyone.
    let _: DeleteFlagResponse =
        app.call(&format!("{FLAGS}/DeleteFlag"), Some(&boss), &DeleteFlagRequest { name: name.clone(), ..Default::default() }).await.unwrap();
    assert!(!app.core.flags.enabled(&name, Some(outsider.id)).await);
    let gone = app
        .call::<_, DeleteFlagResponse>(&format!("{FLAGS}/DeleteFlag"), Some(&boss), &DeleteFlagRequest { name: name.clone(), ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!((gone.code.as_str(), gone.reason.as_str()), ("not_found", "FLAG_NOT_FOUND"));

    // Every change is in the audit log with before/after snapshots.
    let audit: ListAuditLogResponse = app
        .call(
            &format!("{SYSTEM}/ListAuditLog"),
            Some(&boss),
            &ListAuditLogRequest { target_type: "flag".into(), target_id: name.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(audit.entries.iter().map(|e| e.action.as_str()).collect::<Vec<_>>(), vec!["flag.deleted", "flag.updated", "flag.updated", "flag.created"]);
    let enabling: (serde_json::Value, serde_json::Value) =
        (serde_json::from_str(&audit.entries[2].before_json).unwrap(), serde_json::from_str(&audit.entries[2].after_json).unwrap());
    assert_eq!((&enabling.0["enabled"], &enabling.1["enabled"]), (&serde_json::json!(false), &serde_json::json!(true)));
    assert!(audit.entries[0].after_json.is_empty() && !audit.entries[0].before_json.is_empty());
}

#[tokio::test]
async fn flag_input_is_validated() {
    let (app, _admin) = start().await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let name = unique_flag("valid");
    create(&app, &boss, CreateFlagRequest { name: name.clone(), ..Default::default() }).await;

    let creations: Vec<(&str, &str, CreateFlagRequest)> = vec![
        ("already_exists", "FLAG_EXISTS", CreateFlagRequest { name: name.clone(), ..Default::default() }),
        ("invalid_argument", "FLAG_NAME_INVALID", CreateFlagRequest { name: "Bad Name".into(), ..Default::default() }),
        ("invalid_argument", "FLAG_NAME_INVALID", CreateFlagRequest::default()),
        ("invalid_argument", "FLAG_DESCRIPTION_TOO_LONG", CreateFlagRequest { name: unique_flag("d"), description: "x".repeat(501), ..Default::default() }),
        ("invalid_argument", "ROLLOUT_INVALID", CreateFlagRequest { name: unique_flag("r"), rollout_percent: Some(101), ..Default::default() }),
        ("invalid_argument", "INVALID_ID", CreateFlagRequest { name: unique_flag("i"), allow_user_ids: vec!["someone".into()], ..Default::default() }),
        ("not_found", "USER_NOT_FOUND", CreateFlagRequest { name: unique_flag("u"), allow_user_ids: vec![Uuid::now_v7().to_string()], ..Default::default() }),
    ];
    for (code, reason, request) in creations {
        let err = app.call::<_, CreateFlagResponse>(&format!("{FLAGS}/CreateFlag"), Some(&boss), &request).await.unwrap_err();
        assert_eq!((err.code.as_str(), err.reason.as_str()), (code, reason), "{err}");
    }

    let updates: Vec<(&str, UpdateFlagRequest)> = vec![
        ("FLAG_NOT_FOUND", UpdateFlagRequest { name: unique_flag("missing"), enabled: Some(true), ..Default::default() }),
        ("ROLLOUT_INVALID", UpdateFlagRequest { name: name.clone(), rollout_percent: Some(-1), ..Default::default() }),
        ("FLAG_DESCRIPTION_TOO_LONG", UpdateFlagRequest { name: name.clone(), description: Some("x".repeat(501)), ..Default::default() }),
        ("USER_NOT_FOUND", UpdateFlagRequest { name: name.clone(), add_allow_user_ids: vec![Uuid::now_v7().to_string()], ..Default::default() }),
    ];
    for (reason, request) in updates {
        let err = app.call::<_, UpdateFlagResponse>(&format!("{FLAGS}/UpdateFlag"), Some(&boss), &request).await.unwrap_err();
        assert_eq!(err.reason, reason, "{err}");
    }
    // The rejected updates left the flag untouched.
    let unchanged = update(&app, &boss, UpdateFlagRequest { name: name.clone(), ..Default::default() }).await;
    assert_eq!((unchanged.enabled, unchanged.rollout_percent, unchanged.allow_users.len()), (false, 100, 0));
}

#[tokio::test]
async fn flags_list_in_name_order_with_paging() {
    let (app, _admin) = start().await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let group = unique_flag("paging");
    for suffix in ["c", "a", "b"] {
        create(&app, &boss, CreateFlagRequest { name: format!("{group}.{suffix}"), ..Default::default() }).await;
    }
    let mut names = Vec::new();
    let mut cursor = String::new();
    loop {
        let page: ListFlagsResponse = app
            .call(
                &format!("{FLAGS}/ListFlags"),
                Some(&boss),
                &ListFlagsRequest {
                    query: group.clone(),
                    page: PageRequest { limit: 2, cursor: cursor.clone(), ..Default::default() }.into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        names.extend(page.flags.iter().map(|f| f.name.clone()));
        if !page.page.has_more {
            break;
        }
        cursor = page.page.next_cursor.clone();
    }
    assert_eq!(names, vec![format!("{group}.a"), format!("{group}.b"), format!("{group}.c")]);
}

#[tokio::test]
async fn evaluation_preview_explains_each_flag() {
    let (app, _admin) = start().await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let user = app.user("user").await;
    let group = unique_flag("eval");
    let named = |suffix: &str| format!("{group}.{suffix}");

    create(&app, &boss, CreateFlagRequest { name: named("allow"), allow_user_ids: vec![user.id.to_string()], ..Default::default() }).await;
    create(&app, &boss, CreateFlagRequest { name: named("off"), ..Default::default() }).await;
    create(&app, &boss, CreateFlagRequest { name: named("everyone"), enabled: true, ..Default::default() }).await;
    create(&app, &boss, CreateFlagRequest { name: named("nobody"), enabled: true, rollout_percent: Some(0), ..Default::default() }).await;
    create(&app, &boss, CreateFlagRequest { name: named("half"), enabled: true, rollout_percent: Some(50), ..Default::default() }).await;

    let preview: EvaluateFlagsResponse =
        app.call(&format!("{FLAGS}/EvaluateFlags"), Some(&boss), &EvaluateFlagsRequest { user_id: user.id.to_string(), ..Default::default() }).await.unwrap();
    let of = |suffix: &str| preview.evaluations.iter().find(|e| e.name == named(suffix)).unwrap_or_else(|| panic!("{suffix} is evaluated"));
    assert_eq!((of("allow").on, of("allow").decision.as_known().unwrap()), (true, FlagDecision::FLAG_DECISION_ALLOW_LISTED));
    assert_eq!((of("off").on, of("off").decision.as_known().unwrap()), (false, FlagDecision::FLAG_DECISION_DISABLED));
    assert_eq!((of("everyone").on, of("everyone").decision.as_known().unwrap()), (true, FlagDecision::FLAG_DECISION_EVERYONE));
    assert_eq!((of("nobody").on, of("nobody").decision.as_known().unwrap()), (false, FlagDecision::FLAG_DECISION_OUT_OF_ROLLOUT));
    // A partial rollout lands on one side or the other, and the preview agrees with what core serves the user.
    let half = of("half");
    let expected = if half.on { FlagDecision::FLAG_DECISION_IN_ROLLOUT } else { FlagDecision::FLAG_DECISION_OUT_OF_ROLLOUT };
    assert_eq!(half.decision, expected);
    for evaluation in preview.evaluations.iter().filter(|e| e.name.starts_with(&group)) {
        assert_eq!(evaluation.on, app.core.flags.enabled(&evaluation.name, Some(user.id)).await, "{}", evaluation.name);
    }

    let unknown = app
        .call::<_, EvaluateFlagsResponse>(
            &format!("{FLAGS}/EvaluateFlags"),
            Some(&boss),
            &EvaluateFlagsRequest { user_id: Uuid::now_v7().to_string(), ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!((unknown.code.as_str(), unknown.reason.as_str()), ("not_found", "USER_NOT_FOUND"));
}

#[tokio::test]
async fn deleted_accounts_leave_the_allow_lists() {
    let (app, admin) = start().await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let (leaving, staying) = (app.user("leaving").await, app.user("staying").await);
    let name = unique_flag("cleanup");
    create(&app, &boss, CreateFlagRequest { name: name.clone(), allow_user_ids: vec![leaving.id.to_string(), staying.id.to_string()], ..Default::default() })
        .await;
    assert!(app.core.flags.enabled(&name, Some(leaving.id)).await);

    admin
        .handle_event(tokuly_core::events::Event {
            id: Uuid::now_v7(),
            topic: "identity.user.deleted".into(),
            actor_id: Some(leaving.id),
            occurred_at: chrono::Utc::now(),
            data: serde_json::json!({ "user_id": leaving.id }),
        })
        .await
        .unwrap();
    let allow: Vec<Uuid> = sqlx::query_scalar("SELECT allow_user_ids FROM feature_flags WHERE name = $1").bind(&name).fetch_one(&app.core.db).await.unwrap();
    assert_eq!(allow, vec![staying.id]);
    assert!(!app.core.flags.enabled(&name, Some(leaving.id)).await, "the cached table was dropped too");
}
