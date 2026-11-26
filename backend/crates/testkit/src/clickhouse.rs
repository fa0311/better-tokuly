//! The run's ClickHouse database, named like its PostgreSQL database, with
//! `backend/clickhouse/*.sql` applied.
use std::path::PathBuf;

use tokuly_core::config::Config;

fn schema_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../clickhouse")
}

/// Creates a private ClickHouse database with every schema file applied.
/// Returns `None` when ClickHouse is not reachable (tests that need it will
/// then fail on their first analytics query, with a clear connection error).
pub(crate) async fn create(cfg: &Config, name: &str, now: u64) -> Option<String> {
    let server = clickhouse::Client::default()
        .with_url(&cfg.clickhouse_url)
        .with_user(&cfg.clickhouse_user)
        .with_password(&cfg.clickhouse_password);
    let existing: Vec<String> =
        server.query("SELECT name FROM system.databases WHERE name LIKE 'tokuly_test_%'").fetch_all().await.ok()?;
    for database in existing {
        let started = database.split('_').nth(2).and_then(|t| t.parse::<u64>().ok()).unwrap_or(0);
        if now.saturating_sub(started) > 20 * 60 {
            let _ = server.query(&format!("DROP DATABASE IF EXISTS `{database}`")).execute().await;
        }
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(schema_dir())
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "sql"))
        .collect();
    files.sort();
    let statements: Vec<(String, String)> = files
        .iter()
        .map(|f| {
            (
                f.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(f).expect("testkit: read ClickHouse schema"),
            )
        })
        .collect();
    let borrowed: Vec<(&str, &str)> = statements.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();
    let mut test_cfg = cfg.clone();
    test_cfg.clickhouse_database = name.to_owned();
    tokuly_core::analytics::apply_schema(&test_cfg, &borrowed)
        .await
        .unwrap_or_else(|e| panic!("testkit: ClickHouse schema failed: {e}"));
    Some(name.to_owned())
}
