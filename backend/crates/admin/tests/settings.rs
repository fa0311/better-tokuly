//! 「サーバー設定」: the console layer of the server settings.
//!
//! Every test uses settings of its own (they share the `server_settings`
//! table), in a `[demo]` section registered next to the real `[core]`.
mod common;

use std::{collections::HashMap, sync::Arc};

use clap::Args;
use common::*;
use tokuly_core::{
    config::Config,
    settings::{Layers, Process, Registry, SectionSpec, Settings, State},
};
use tokuly_proto::admin::*;
use tokuly_testkit::{TestApp, TestUser};

const SETTINGS: &str = "tokuly.admin.v1.ServerSettingsService";

#[derive(Args, Debug)]
struct Demo {
    /// Applies at once.
    #[arg(long = "demo-limit", env = "TOKULY_DEMO_LIMIT", default_value_t = 5)]
    limit: i64,
    /// Applies on restart.
    #[arg(long = "demo-slow", env = "TOKULY_DEMO_SLOW", default_value_t = 1)]
    slow: i64,
    /// A credential.
    #[arg(long = "demo-token", env = "TOKULY_DEMO_TOKEN", default_value = "", hide_env_values = true)]
    token: String,
    /// Set in tokuly.toml by the test.
    #[arg(long = "demo-filed", env = "TOKULY_DEMO_FILED", default_value = "a")]
    filed: String,
    /// Set in the environment by the test.
    #[arg(long = "demo-envd", env = "TOKULY_DEMO_ENVD", default_value = "a")]
    envd: String,
    /// A bootstrap setting.
    #[arg(long = "demo-listen", env = "TOKULY_DEMO_LISTEN", default_value = "0.0.0.0:1")]
    listen: String,
    /// Edited on its own screen.
    #[arg(long = "demo-ai-model", env = "TOKULY_DEMO_AI_MODEL", default_value = "m")]
    ai_model: String,
    /// Must be at least 1 (the section's check).
    #[arg(long = "demo-min", env = "TOKULY_DEMO_MIN", default_value_t = 1)]
    min: i64,
    /// Free text for the export.
    #[arg(long = "demo-note", env = "TOKULY_DEMO_NOTE", default_value = "")]
    note: String,
    /// Written by two admins at once.
    #[arg(long = "demo-race", env = "TOKULY_DEMO_RACE", default_value_t = 0)]
    race: i64,
}

fn registry() -> Arc<Registry> {
    let core = SectionSpec::new("core", Config::augment_args)
        .processes(&[Process::Api, Process::Realtime, Process::Worker])
        .live(&["rate_limits"])
        .bootstrap(&["secret", "database_url"]);
    let demo = SectionSpec::new("demo", Demo::augment_args)
        .live(&["limit"])
        .bootstrap(&["listen"])
        .managed(&[("ai_", "/admin/chat-ai")])
        .check(|m, _| {
            use clap::FromArgMatches as _;
            anyhow::ensure!(Demo::from_arg_matches(m)?.min >= 1, "TOKULY_DEMO_MIN must be at least 1");
            Ok(())
        });
    Arc::new(Registry::new(vec![core, demo]).unwrap())
}

/// An app whose settings are the registry above: `filed` set by the file,
/// `envd` by the environment.
async fn start_settings() -> (TestApp, TestUser) {
    let (app, _admin) = start().await;
    let mut layers = Layers::default();
    layers.file.insert("filed".into(), "from-file".into());
    layers.env.insert("envd".into(), "from-env".into());
    app.core.settings.install(State::new(registry(), layers, Some("/etc/tokuly/tokuly.toml".into()), false));
    app.core.reload_settings().await.unwrap();
    let boss = app.user_with_roles("boss", &["admin"]).await;
    (app, boss)
}

async fn list(app: &TestApp, boss: &TestUser) -> ListServerSettingsResponse {
    app.call(&format!("{SETTINGS}/ListServerSettings"), Some(boss), &ListServerSettingsRequest::default()).await.unwrap()
}

async fn update(app: &TestApp, boss: &TestUser, key: &str, value: &str) -> Result<ServerSetting, tokuly_testkit::RpcError> {
    let request = UpdateServerSettingRequest { section: "demo".into(), key: key.into(), value: value.into(), ..Default::default() };
    let response: UpdateServerSettingResponse = app.call(&format!("{SETTINGS}/UpdateServerSetting"), Some(boss), &request).await?;
    Ok(response.setting.into_option().unwrap())
}

async fn reset(app: &TestApp, boss: &TestUser, section: &str, key: &str) -> ServerSetting {
    let request = UpdateServerSettingRequest { section: section.into(), key: key.into(), reset: true, ..Default::default() };
    let response: UpdateServerSettingResponse = app.call(&format!("{SETTINGS}/UpdateServerSetting"), Some(boss), &request).await.unwrap();
    response.setting.into_option().unwrap()
}

async fn changes(app: &TestApp, boss: &TestUser, key: &str) -> Vec<ServerSettingChange> {
    let request = ListServerSettingChangesRequest { section: "demo".into(), key: key.into(), ..Default::default() };
    let response: ListServerSettingChangesResponse = app.call(&format!("{SETTINGS}/ListServerSettingChanges"), Some(boss), &request).await.unwrap();
    response.changes
}

fn demo(app: &TestApp) -> Arc<Demo> {
    app.core.settings.current::<Demo>("demo").unwrap()
}

#[tokio::test]
async fn server_settings_are_for_admins_only() {
    let (app, _boss) = start_settings().await;
    let user = app.user("user").await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    for rpc in ["ListServerSettings", "UpdateServerSetting", "ExportServerSettings", "ListServerSettingChanges"] {
        assert_role_gated(&app, &format!("{SETTINGS}/{rpc}"), &[&user, &moderator]).await;
    }
}

#[tokio::test]
async fn every_source_is_shown_and_the_console_sits_below_file_and_environment() {
    let (app, boss) = start_settings().await;
    let listing = list(&app, &boss).await;
    assert_eq!(listing.file_path, "/etc/tokuly/tokuly.toml");
    let by_key: HashMap<String, ServerSetting> = listing.settings.into_iter().filter(|s| s.section == "demo").map(|s| (s.key.clone(), s)).collect();
    let filed = &by_key["filed"];
    assert_eq!((filed.value.as_deref(), filed.source.as_known(), filed.lock.as_known()), (
        Some("from-file"),
        Some(ServerSettingSource::SERVER_SETTING_SOURCE_FILE),
        Some(ServerSettingLock::SERVER_SETTING_LOCK_OVERRIDDEN)
    ));
    assert_eq!(by_key["envd"].source.as_known(), Some(ServerSettingSource::SERVER_SETTING_SOURCE_ENVIRONMENT));
    assert_eq!(by_key["listen"].lock.as_known(), Some(ServerSettingLock::SERVER_SETTING_LOCK_BOOTSTRAP));
    let managed = &by_key["ai_model"];
    assert_eq!((managed.lock.as_known(), managed.managed_at.as_str()), (Some(ServerSettingLock::SERVER_SETTING_LOCK_MANAGED_ELSEWHERE), "/admin/chat-ai"));
    let slow = &by_key["slow"];
    assert_eq!((slow.default_value.as_deref(), slow.kind.as_known(), slow.live), (Some("1"), Some(ServerSettingKind::SERVER_SETTING_KIND_INTEGER), false));
    assert_eq!(slow.description, "Applies on restart.");
    assert_eq!(slow.env, "TOKULY_DEMO_SLOW");

    // What the file and the environment set cannot be changed here.
    for key in ["filed", "envd"] {
        let err = update(&app, &boss, key, "x").await.unwrap_err();
        assert_eq!(err.reason, "SETTING_LOCKED", "{key}");
    }
    assert_eq!(update(&app, &boss, "listen", "0.0.0.0:2").await.unwrap_err().reason, "SETTING_BOOTSTRAP");
    assert_eq!(update(&app, &boss, "ai_model", "other").await.unwrap_err().reason, "SETTING_MANAGED_ELSEWHERE");
    assert_eq!(update(&app, &boss, "nope", "1").await.unwrap_err().reason, "SETTING_NOT_FOUND");
    // The type, and the domain's own rules.
    let err = update(&app, &boss, "limit", "seven").await.unwrap_err();
    assert_eq!(err.reason, "SETTING_INVALID");
    let err = update(&app, &boss, "min", "0").await.unwrap_err();
    assert_eq!(err.reason, "SETTING_INVALID");
    assert!(err.message.contains("at least 1"), "{}", err.message);
    assert_eq!(update(&app, &boss, "note", &"x".repeat(17 * 1024)).await.unwrap_err().reason, "SETTING_TOO_LONG");
    let bootstrap = UpdateServerSettingRequest { section: "core".into(), key: "secret".into(), value: "y".repeat(40), ..Default::default() };
    let err = app.call::<_, UpdateServerSettingResponse>(&format!("{SETTINGS}/UpdateServerSetting"), Some(&boss), &bootstrap).await.unwrap_err();
    assert_eq!(err.reason, "SETTING_BOOTSTRAP");
}

#[tokio::test]
async fn live_settings_apply_at_once_and_others_on_restart() {
    let (app, boss) = start_settings().await;
    let limit = update(&app, &boss, "limit", "7").await.unwrap();
    assert_eq!((limit.value.as_deref(), limit.console_value.as_deref(), limit.source.as_known()), (
        Some("7"),
        Some("7"),
        Some(ServerSettingSource::SERVER_SETTING_SOURCE_CONSOLE)
    ));
    assert!(!limit.restart_pending);
    assert_eq!(demo(&app).limit, 7);

    let slow = update(&app, &boss, "slow", "3").await.unwrap();
    assert!(slow.restart_pending);
    assert_eq!(
        slow.processes.iter().filter_map(|p| p.as_known()).collect::<Vec<_>>(),
        vec![ServerProcess::SERVER_PROCESS_API, ServerProcess::SERVER_PROCESS_WORKER]
    );

    // Back to the default: nothing pending, the history has both steps.
    let limit = reset(&app, &boss, "demo", "limit").await;
    assert_eq!((limit.value.as_deref(), limit.console_value.as_deref(), limit.source.as_known()), (
        Some("5"),
        None,
        Some(ServerSettingSource::SERVER_SETTING_SOURCE_DEFAULT)
    ));
    assert_eq!(demo(&app).limit, 5);
    assert!(!reset(&app, &boss, "demo", "slow").await.restart_pending);
    let history = changes(&app, &boss, "limit").await;
    let steps: Vec<(Option<&str>, Option<&str>)> = history.iter().map(|c| (c.old_value.as_deref(), c.new_value.as_deref())).collect();
    assert_eq!(steps, vec![(Some("7"), None), (None, Some("7"))]);
    assert!(history.iter().all(|c| c.actor.as_option().is_some_and(|a| a.id == boss.id.to_string()) && c.changed_at.is_set()));

    // Audit log and event, one each per change.
    let audit: Vec<(String, serde_json::Value, serde_json::Value)> = sqlx::query_as(
        "SELECT target_id, before, after FROM admin_audit_log WHERE actor_id = $1 AND action = 'server_setting.updated' ORDER BY created_at",
    )
    .bind(boss.id)
    .fetch_all(&app.core.db)
    .await
    .unwrap();
    assert_eq!(audit.len(), 4);
    assert_eq!(audit[0], ("demo.limit".to_owned(), serde_json::json!({ "value": null }), serde_json::json!({ "value": "7" })));
    let events = app.events_by("admin.server_setting.changed", boss.id).await;
    assert_eq!(events.len(), 4);
    assert_eq!(events[0].data, serde_json::json!({ "section": "demo", "key": "limit", "secret": false, "live": true }));

    // Saving the value in force again changes nothing and records nothing.
    reset(&app, &boss, "demo", "limit").await;
    assert_eq!(changes(&app, &boss, "limit").await.len(), 2);
}

#[tokio::test]
async fn the_rate_limits_switch_is_live() {
    let (app, boss) = start_settings().await;
    let request = |value: &str| UpdateServerSettingRequest { section: "core".into(), key: "rate_limits".into(), value: value.into(), ..Default::default() };
    let _: UpdateServerSettingResponse = app.call(&format!("{SETTINGS}/UpdateServerSetting"), Some(&boss), &request("false")).await.unwrap();
    assert!(!app.core.cache.enforces_rate_limits());
    reset(&app, &boss, "core", "rate_limits").await;
    assert!(app.core.cache.enforces_rate_limits());
}

#[tokio::test]
async fn secrets_are_write_only() {
    let (app, boss) = start_settings().await;
    let plain = "supersecret-token-1234";
    let token = update(&app, &boss, "token", plain).await.unwrap();
    assert_eq!((token.value.as_deref(), token.default_value.as_deref(), token.console_value.as_deref()), (None, None, None));
    assert!(token.secret && token.secret_set && token.console_secret_set);
    assert_eq!(token.secret_last4, "1234");
    // The process has it, sealed at rest.
    assert_eq!(demo(&app).token, plain);
    let (value, sealed): (Option<String>, Vec<u8>) =
        sqlx::query_as("SELECT value, sealed FROM server_settings WHERE section = 'demo' AND key = 'token'").fetch_one(&app.core.db).await.unwrap();
    assert!(value.is_none() && !sealed.windows(plain.len()).any(|w| w == plain.as_bytes()));

    // Nothing that leaves the API carries it.
    let listing = serde_json::to_string(&list(&app, &boss).await).unwrap();
    let export: ExportServerSettingsResponse =
        app.call(&format!("{SETTINGS}/ExportServerSettings"), Some(&boss), &ExportServerSettingsRequest::default()).await.unwrap();
    let history = serde_json::to_string(&changes(&app, &boss, "token").await).unwrap();
    let audit: Vec<String> = sqlx::query_scalar("SELECT concat(before::text, after::text) FROM admin_audit_log WHERE target_id = 'demo.token'")
        .fetch_all(&app.core.db)
        .await
        .unwrap();
    let outbox: Vec<String> =
        sqlx::query_scalar("SELECT data::text FROM outbox WHERE topic = 'admin.server_setting.changed'").fetch_all(&app.core.db).await.unwrap();
    for text in [listing, export.toml, history.clone()].iter().chain(&audit).chain(&outbox) {
        assert!(!text.contains("supersecret"), "{text}");
    }
    assert!(history.contains("••••"));
    reset(&app, &boss, "demo", "token").await;
    assert_eq!(demo(&app).token, "");
}

#[tokio::test]
async fn the_export_is_a_tokuly_toml_of_what_is_in_force() {
    let (app, boss) = start_settings().await;
    update(&app, &boss, "note", "こんにちは \"quoted\"").await.unwrap();
    let export: ExportServerSettingsResponse =
        app.call(&format!("{SETTINGS}/ExportServerSettings"), Some(&boss), &ExportServerSettingsRequest::default()).await.unwrap();
    assert!(export.toml.contains("[demo]"), "{}", export.toml);
    assert!(export.toml.contains("from tokuly.toml") && export.toml.contains("from TOKULY_DEMO_ENVD"), "{}", export.toml);
    // It reads back as the same values.
    let values = tokuly_core::settings::file::parse(&registry(), &export.toml, "export").unwrap();
    assert_eq!(values["note"], "こんにちは \"quoted\"");
    assert_eq!(values["filed"], "from-file");
    assert!(!values.contains_key("min"), "defaults are left out");
    reset(&app, &boss, "demo", "note").await;
}

/// Two admins saving one setting at once: the history stays a chain.
#[tokio::test]
async fn concurrent_changes_keep_the_history_consistent() {
    let (app, boss) = start_settings().await;
    let other = app.user_with_roles("boss2", &["admin"]).await;
    let (a, b) = tokio::join!(update(&app, &boss, "race", "10"), update(&app, &other, "race", "11"));
    a.unwrap();
    b.unwrap();
    let history = changes(&app, &boss, "race").await;
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].old_value, None);
    assert_eq!(history[0].old_value, history[1].new_value);
    let saved: String = sqlx::query_scalar("SELECT value FROM server_settings WHERE section = 'demo' AND key = 'race'").fetch_one(&app.core.db).await.unwrap();
    assert_eq!(Some(saved), history[0].new_value);
}

#[tokio::test]
async fn without_a_registry_the_service_is_unavailable() {
    let (app, _admin) = start().await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    assert!(Settings::default().state().is_none());
    let err = app.call::<_, ListServerSettingsResponse>(&format!("{SETTINGS}/ListServerSettings"), Some(&boss), &ListServerSettingsRequest::default()).await.unwrap_err();
    assert_eq!(err.reason, "SETTINGS_UNAVAILABLE");
}
