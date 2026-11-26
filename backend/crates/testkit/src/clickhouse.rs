//! The run's ClickHouse database, named like its PostgreSQL database, with
//! `backend/clickhouse/*.sql` applied.
use std::path::PathBuf;

use tokuly_core::config::Config;

use crate::{
    postgres,
    run::{self, Run},
};

const HEAD: &str = "tokuly_test_";

fn schema_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../clickhouse")
}

fn server(cfg: &Config) -> clickhouse::Client {
    clickhouse::Client::default()
        .with_url(&cfg.clickhouse_url)
        .with_user(&cfg.clickhouse_user)
        .with_password(&cfg.clickhouse_password)
}

/// Creates the run's database with every schema file applied and returns
/// its name; `None` when ClickHouse is not reachable (tests that need it
/// will then fail on their first analytics query, with a clear connection
/// error).
pub(crate) async fn create(cfg: &Config, run: &Run) -> Option<String> {
    let name = postgres::database(run);
    server(cfg).query("SELECT 1").execute().await.ok()?;
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
    test_cfg.clickhouse_database = name.clone();
    tokuly_core::analytics::apply_schema(&test_cfg, &borrowed)
        .await
        .unwrap_or_else(|e| panic!("testkit: ClickHouse schema failed: {e}"));
    Some(name)
}

async fn drop_database(server: &clickhouse::Client, name: &str) -> clickhouse::error::Result<()> {
    server.query(&format!("DROP DATABASE IF EXISTS `{name}`")).execute().await
}

pub(crate) async fn remove(cfg: &Config, run: &Run) -> anyhow::Result<()> {
    drop_database(&server(cfg), &postgres::database(run)).await?;
    Ok(())
}

/// Drops the databases of stale runs; returns how many.
pub(crate) async fn sweep(cfg: &Config, now: u64) -> anyhow::Result<usize> {
    let server = server(cfg);
    let names: Vec<String> =
        server.query("SELECT name FROM system.databases WHERE startsWith(name, ?)").bind(HEAD).fetch_all().await?;
    let mut dropped = 0;
    for name in names.iter().filter(|name| run::is_stale(name, HEAD, '_', now)) {
        if drop_database(&server, name).await.is_ok() {
            dropped += 1;
        }
    }
    Ok(dropped)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{env::base_config, run::STALE_AFTER};

    pub(crate) async fn exists(cfg: &Config, name: &str) -> bool {
        server(cfg)
            .query("SELECT count() FROM system.databases WHERE name = ?")
            .bind(name)
            .fetch_one::<u64>()
            .await
            .expect("query system.databases")
            > 0
    }

    async fn create_empty(cfg: &Config, name: &str) {
        server(cfg).query(&format!("CREATE DATABASE `{name}`")).execute().await.expect("create");
    }

    #[tokio::test]
    async fn a_run_removes_its_database() {
        let cfg = base_config();
        let run = Run::new();
        let name = create(&cfg, &run).await.expect("ClickHouse is reachable");
        assert!(exists(&cfg, &name).await);
        remove(&cfg, &run).await.unwrap();
        assert!(!exists(&cfg, &name).await);
    }

    #[tokio::test]
    async fn the_sweep_takes_only_stale_test_databases() {
        let cfg = base_config();
        let now = run::unix_now();
        let stale = postgres::database(&Run::started_at(now - STALE_AFTER.as_secs() - 60));
        let live = postgres::database(&Run::started_at(now - 60));
        let foreign = format!("tokuly_sweep_{}", Run::new().name("", '_'));
        for name in [&stale, &live, &foreign] {
            create_empty(&cfg, name).await;
        }
        sweep(&cfg, now).await.unwrap();
        assert!(!exists(&cfg, &stale).await);
        assert!(exists(&cfg, &live).await);
        assert!(exists(&cfg, &foreign).await);
        for name in [&live, &foreign] {
            drop_database(&server(&cfg), name).await.unwrap();
        }
    }
}
