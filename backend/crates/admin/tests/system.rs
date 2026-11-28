//! Server stats and the audit log.
mod common;

use common::*;
use tokuly_proto::{admin::*, common::PageRequest};
use tokuly_testkit::{TestApp, TestUser};
use uuid::Uuid;

async fn stats(app: &TestApp, boss: &TestUser) -> GetServerStatsResponse {
    app.call(&format!("{SYSTEM}/GetServerStats"), Some(boss), &GetServerStatsRequest::default()).await.unwrap()
}

async fn audit(app: &TestApp, boss: &TestUser, request: ListAuditLogRequest) -> ListAuditLogResponse {
    app.call(&format!("{SYSTEM}/ListAuditLog"), Some(boss), &request).await.unwrap()
}

#[tokio::test]
async fn system_rpcs_are_for_admins_only() {
    let (app, _admin) = start().await;
    let user = app.user("user").await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    for rpc in ["GetServerStats", "ListAuditLog"] {
        assert_role_gated(&app, &format!("{SYSTEM}/{rpc}"), &[&user, &moderator]).await;
    }
}

#[tokio::test]
async fn server_stats_describe_the_deployment() {
    let (app, admin) = start().await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let before = stats(&app, &boss).await;

    // Add what the numbers should pick up: accounts, a second device, a suspension, a badge, an open case.
    let (owner, reporter, suspended, star) = (app.user("owner").await, app.user("rep").await, app.user("susp").await, app.user("star").await);
    let _second_device = app.sign_in(owner.id).await;
    let _: SuspendUserResponse = app
        .call(
            &format!("{USERS}/SuspendUser"),
            Some(&boss),
            &SuspendUserRequest { user_id: suspended.id.to_string(), reason: "テスト".into(), permanent: true, ..Default::default() },
        )
        .await
        .unwrap();
    let _: SetUserVerifiedResponse = app
        .call(&format!("{USERS}/SetUserVerified"), Some(&boss), &SetUserVerifiedRequest { user_id: star.id.to_string(), verified: true, ..Default::default() })
        .await
        .unwrap();
    open_case(&app, &admin, &moderator, &reporter, &owner).await;

    let after = stats(&app, &boss).await;
    // Other tests share the database, so the numbers are compared as lower bounds.
    let (users_before, users) = (&before.users, &after.users);
    assert!(users.total >= users_before.total + 4);
    assert!(users.new_today >= users_before.new_today + 4 && users.new_7d >= users.new_today);
    assert!(users.new_7d <= users.total);
    // This test's own accounts hold six live sessions (the suspended one lost theirs) and all seven were active today.
    assert!(users.active_sessions >= 6);
    assert!(users.active_today >= 7 && users.active_7d >= users.active_today && users.active_30d >= users.active_7d);
    assert!(users.suspended > users_before.suspended);
    assert!(users.verified > users_before.verified);

    assert!(after.moderation.open_cases > before.moderation.open_cases);
    assert!(after.moderation.unassigned_cases >= 1 && after.moderation.reports_24h > before.moderation.reports_24h);
    assert!(after.moderation.oldest_open_case_at.is_set());

    // Content counts cover the tables that exist here (media's) and skip the ones that do not (notes').
    let content: Vec<&str> = after.content.iter().map(|c| c.name.as_str()).collect();
    assert!(content.contains(&"media_assets"), "{content:?}");
    assert!(!content.contains(&"notes") && !content.contains(&"videos"), "{content:?}");
    assert!(after.content.iter().all(|c| c.count >= 0 && !c.approximate));

    // No relay runs in tests, so everything emitted above is still in the outbox.
    assert!(after.queues.outbox_pending > before.queues.outbox_pending);
    assert!(after.queues.outbox_oldest_age_seconds >= 0);
    let streams: Vec<&str> = after.queues.streams.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(streams, vec!["EVENTS", "JOBS", "ANALYTICS", "LOCATIONS"]);

    // Every dependency is probed; the ones the test harness itself needs must be up.
    let dependency = |name: &str| after.dependencies.iter().find(|d| d.name == name).unwrap_or_else(|| panic!("{name} is probed"));
    for required in ["postgres", "valkey", "nats"] {
        assert!(dependency(required).reachable, "{required}: {}", dependency(required).detail);
    }
    for optional in ["clickhouse", "meilisearch"] {
        let status = dependency(optional);
        assert_eq!(status.reachable, status.detail.is_empty(), "{optional} reports an error exactly when it is down");
    }
    assert!(after.generated_at.is_set());
}

#[tokio::test]
async fn stream_stats_include_consumer_lag() {
    let (app, _admin) = start().await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    // A durable consumer with one job waiting for it.
    let consumer = app
        .core
        .bus
        .durable_consumer(tokuly_core::bus::STREAM_JOBS, "job-admin-stats-test", vec!["job.admin.stats_test".into()], std::time::Duration::from_secs(30), 3, tokuly_core::bus::StartAt::All)
        .await
        .unwrap();
    drop(consumer);
    app.core.jobs.enqueue("admin.stats_test", &serde_json::json!({ "n": 1 })).await.unwrap();

    let snapshot = stats(&app, &boss).await;
    let jobs = snapshot.queues.streams.iter().find(|s| s.name == "JOBS").expect("JOBS stream");
    assert!(jobs.messages >= 1 && jobs.bytes > 0);
    let lagging = jobs.consumers.iter().find(|c| c.name == "job-admin-stats-test").expect("consumer is listed");
    assert_eq!((lagging.pending, lagging.ack_pending), (1, 0));
}

#[tokio::test]
async fn audit_log_records_filters_and_pages() {
    let (app, _admin) = start().await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let user = app.user("user").await;
    let started = chrono::Utc::now() - chrono::Duration::seconds(5);

    // Three actions by two staff members.
    let flag = format!("audit.{}", tokuly_core::ids::random_base62(10).to_ascii_lowercase());
    let _: CreateFlagResponse =
        app.call(&format!("{FLAGS}/CreateFlag"), Some(&boss), &CreateFlagRequest { name: flag.clone(), ..Default::default() }).await.unwrap();
    let _: WarnUserResponse = app
        .call(&format!("{USERS}/WarnUser"), Some(&moderator), &WarnUserRequest { user_id: user.id.to_string(), reason: "注意".into(), ..Default::default() })
        .await
        .unwrap();
    let _: SuspendUserResponse = app
        .call(
            &format!("{USERS}/SuspendUser"),
            Some(&boss),
            &SuspendUserRequest {
                user_id: user.id.to_string(), reason: "停止".into(), note: "三度目".into(), suspension_seconds: 3600, ..Default::default()
            },
        )
        .await
        .unwrap();

    // By actor.
    let by_boss = audit(&app, &boss, ListAuditLogRequest { actor_id: boss.id.to_string(), ..Default::default() }).await;
    assert_eq!(by_boss.entries.iter().map(|e| e.action.as_str()).collect::<Vec<_>>(), vec!["user.suspended", "flag.created"]);
    let suspension = &by_boss.entries[0];
    assert_eq!(suspension.actor.id, boss.id.to_string());
    assert_eq!((suspension.target_type.as_str(), suspension.target_id.as_str()), ("user", user.id.to_string().as_str()));
    assert_eq!(suspension.target_user.id, user.id.to_string());
    assert_eq!(suspension.reason, "停止");
    assert!(!suspension.ip.is_empty(), "the client address is kept");
    let after: serde_json::Value = serde_json::from_str(&suspension.after_json).unwrap();
    assert_eq!((&after["permanent"], &after["note"]), (&serde_json::json!(false), &serde_json::json!("三度目")));
    let before: serde_json::Value = serde_json::from_str(&suspension.before_json).unwrap();
    assert_eq!(before["suspended"], false);

    // By affected account, by exact action and by action prefix.
    let about_user = audit(&app, &boss, ListAuditLogRequest { target_user_id: user.id.to_string(), ..Default::default() }).await;
    assert_eq!(about_user.entries.iter().map(|e| e.action.as_str()).collect::<Vec<_>>(), vec!["user.suspended", "user.warned"]);
    let warnings = audit(&app, &boss, ListAuditLogRequest { target_user_id: user.id.to_string(), action: "user.warned".into(), ..Default::default() }).await;
    assert_eq!(warnings.entries.len(), 1);
    assert_eq!(warnings.entries[0].actor.id, moderator.id.to_string());
    let user_actions = audit(&app, &boss, ListAuditLogRequest { actor_id: boss.id.to_string(), action: "user.".into(), ..Default::default() }).await;
    assert_eq!(user_actions.entries.iter().map(|e| e.action.as_str()).collect::<Vec<_>>(), vec!["user.suspended"]);
    let flags = audit(&app, &boss, ListAuditLogRequest { target_type: "flag".into(), target_id: flag.clone(), ..Default::default() }).await;
    assert_eq!(flags.entries.len(), 1);
    assert!(flags.entries[0].target_user.is_unset());

    // Time range.
    let since = audit(&app, &boss, ListAuditLogRequest { actor_id: boss.id.to_string(), since: tokuly_core::time::ts(started), ..Default::default() }).await;
    assert_eq!(since.entries.len(), 2);
    let earlier = audit(&app, &boss, ListAuditLogRequest { actor_id: boss.id.to_string(), until: tokuly_core::time::ts(started), ..Default::default() }).await;
    assert!(earlier.entries.is_empty());
    let inverted = app
        .call::<_, ListAuditLogResponse>(
            &format!("{SYSTEM}/ListAuditLog"),
            Some(&boss),
            &ListAuditLogRequest { since: tokuly_core::time::ts(chrono::Utc::now()), until: tokuly_core::time::ts(started), ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!((inverted.code.as_str(), inverted.reason.as_str()), ("invalid_argument", "TIME_RANGE_INVALID"));

    // Paging, newest first.
    let first = audit(
        &app,
        &boss,
        ListAuditLogRequest { target_user_id: user.id.to_string(), page: PageRequest { limit: 1, ..Default::default() }.into(), ..Default::default() },
    )
    .await;
    assert_eq!(first.entries[0].action, "user.suspended");
    assert!(first.page.has_more);
    let second = audit(
        &app,
        &boss,
        ListAuditLogRequest {
            target_user_id: user.id.to_string(),
            page: PageRequest { limit: 1, cursor: first.page.next_cursor.clone(), ..Default::default() }.into(),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(second.entries[0].action, "user.warned");
    assert!(!second.page.has_more);

    let bad_id = app
        .call::<_, ListAuditLogResponse>(&format!("{SYSTEM}/ListAuditLog"), Some(&boss), &ListAuditLogRequest { actor_id: "nope".into(), ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!(bad_id.reason, "INVALID_ID");
    let nobody = audit(&app, &boss, ListAuditLogRequest { actor_id: Uuid::now_v7().to_string(), ..Default::default() }).await;
    assert!(nobody.entries.is_empty());
}

#[tokio::test]
async fn moderation_data_is_part_of_the_personal_data_export() {
    let (app, _admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let user = app.user("user").await;
    let _: WarnUserResponse = app
        .call(
            &format!("{USERS}/WarnUser"),
            Some(&moderator),
            &WarnUserRequest { user_id: user.id.to_string(), reason: "言葉づかいに注意".into(), note: "社内メモ".into(), ..Default::default() },
        )
        .await
        .unwrap();

    let exporter = app.core.exporters.all().into_iter().find(|e| e.name() == "moderation").expect("admin registers an exporter");
    let section = exporter.export(user.id).await.unwrap();
    let actions = section["actions"].as_array().unwrap();
    assert_eq!(actions.len(), 1);
    assert_eq!((&actions[0]["action"], &actions[0]["reason"]), (&serde_json::json!("warning"), &serde_json::json!("言葉づかいに注意")));
    // Staff-internal details are not the user's data.
    let raw = section.to_string();
    assert!(!raw.contains("社内メモ") && !raw.contains(&moderator.id.to_string()));
    assert_eq!(exporter.export(moderator.id).await.unwrap()["actions"].as_array().unwrap().len(), 0);
}
