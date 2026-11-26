//! The run's PostgreSQL database, `tokuly_test_<started>_<id>`, with only
//! the migrations the test binary asked for.
use std::path::PathBuf;

use sqlx::{Connection as _, PgConnection};
use tokuly_core::config::Config;

use crate::run::{self, Run};

const HEAD: &str = "tokuly_test_";

pub(crate) fn database(run: &Run) -> String {
    run.name(HEAD, '_')
}

/// Replaces the database name in a PostgreSQL URL.
pub(crate) fn with_database(url: &str, database: &str) -> String {
    let (base, query) = url.split_once('?').map(|(b, q)| (b, Some(q))).unwrap_or((url, None));
    let prefix = base.rsplit_once('/').map(|(p, _)| p).unwrap_or(base);
    match query {
        Some(q) => format!("{prefix}/{database}?{q}"),
        None => format!("{prefix}/{database}"),
    }
}

fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

async fn admin(cfg: &Config) -> sqlx::Result<PgConnection> {
    PgConnection::connect(&cfg.database_url).await
}

/// Creates the database with `migrations` applied and returns its URL. Each
/// entry names a domain by its number (`"0600"`) and brings in every file of
/// its range (`0600_chat.sql`, then `0601_chat_….sql` added after a release).
pub(crate) async fn create(cfg: &Config, run: &Run, migrations: &[String]) -> String {
    let name = database(run);
    let mut admin = admin(cfg).await.unwrap_or_else(|e| {
        panic!("testkit: cannot connect to PostgreSQL at TOKULY_DATABASE_URL ({e}). Start the dev infrastructure: tools/dev/infra.sh up && eval \"$(tools/dev/infra.sh env)\"")
    });
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE \"{name}\"")))
        .execute(&mut admin)
        .await
        .expect("testkit: CREATE DATABASE failed");
    drop(admin);

    let database_url = with_database(&cfg.database_url, &name);
    let mut conn = PgConnection::connect(&database_url).await.expect("testkit: connect to the test database");
    let mut files: Vec<PathBuf> = std::fs::read_dir(migrations_dir())
        .expect("testkit: backend/migrations is readable")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "sql"))
        .collect();
    files.sort();
    for wanted in migrations {
        assert!(
            files.iter().any(|f| f.file_name().unwrap().to_string_lossy().starts_with(&format!("{wanted}_"))),
            "testkit: no migration file starts with `{wanted}_`"
        );
    }
    let range = |number: &str| number.parse::<u32>().ok().map(|n| n / 100);
    let wanted: Vec<_> = migrations.iter().map(|m| range(m)).collect();
    for file in files {
        let file_name = file.file_name().unwrap().to_string_lossy().into_owned();
        let number = file_name.split('_').next().unwrap_or_default();
        if !wanted.contains(&range(number)) {
            continue;
        }
        let sql = std::fs::read_to_string(&file).expect("testkit: read migration");
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("testkit: migration {file_name} failed: {e}"));
    }
    database_url
}

async fn drop_database(admin: &mut PgConnection, name: &str) -> sqlx::Result<()> {
    // FORCE: the pools of finished tests may still hold connections.
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"))).execute(admin).await?;
    Ok(())
}

pub(crate) async fn remove(cfg: &Config, run: &Run) -> anyhow::Result<()> {
    drop_database(&mut admin(cfg).await?, &database(run)).await?;
    Ok(())
}

/// Drops the databases of stale runs; returns how many.
pub(crate) async fn sweep(cfg: &Config, now: u64) -> anyhow::Result<usize> {
    let mut admin = admin(cfg).await?;
    let names: Vec<String> = sqlx::query_scalar("SELECT datname FROM pg_database WHERE starts_with(datname, $1)")
        .bind(HEAD)
        .fetch_all(&mut admin)
        .await?;
    let mut dropped = 0;
    for name in names.iter().filter(|name| run::is_stale(name, HEAD, '_', now)) {
        // Another binary starting at the same moment may drop it first.
        if drop_database(&mut admin, name).await.is_ok() {
            dropped += 1;
        }
    }
    Ok(dropped)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{env::base_config, run::STALE_AFTER};

    #[test]
    fn database_name_is_swapped_in_urls() {
        assert_eq!(with_database("postgres://u:p@h:5432/tokuly", "t1"), "postgres://u:p@h:5432/t1");
        assert_eq!(with_database("postgres://u@h/tokuly?sslmode=disable", "t1"), "postgres://u@h/t1?sslmode=disable");
    }

    pub(crate) async fn exists(cfg: &Config, name: &str) -> bool {
        let mut admin = admin(cfg).await.expect("PostgreSQL");
        sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(name)
            .fetch_one(&mut admin)
            .await
            .expect("query pg_database")
    }

    async fn create_empty(cfg: &Config, name: &str) {
        let mut admin = admin(cfg).await.expect("PostgreSQL");
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE \"{name}\"")))
            .execute(&mut admin)
            .await
            .expect("create");
    }

    #[tokio::test]
    async fn a_run_removes_its_database() {
        let cfg = base_config();
        let run = Run::new();
        create(&cfg, &run, &["0001".to_owned()]).await;
        assert!(exists(&cfg, &database(&run)).await);
        remove(&cfg, &run).await.unwrap();
        assert!(!exists(&cfg, &database(&run)).await);
        // Removing twice (a failed setup, then exit) is fine.
        remove(&cfg, &run).await.unwrap();
    }

    #[tokio::test]
    async fn the_sweep_takes_only_stale_test_databases() {
        let cfg = base_config();
        let now = run::unix_now();
        let stale = database(&Run::started_at(now - STALE_AFTER.as_secs() - 60));
        let live = database(&Run::started_at(now - 60));
        let foreign = format!("tokuly_sweep_{}", Run::new().name("", '_'));
        for name in [&stale, &live, &foreign] {
            create_empty(&cfg, name).await;
        }
        sweep(&cfg, now).await.unwrap();
        assert!(!exists(&cfg, &stale).await);
        assert!(exists(&cfg, &live).await);
        assert!(exists(&cfg, &foreign).await);
        let mut admin = admin(&cfg).await.unwrap();
        for name in [&live, &foreign] {
            drop_database(&mut admin, name).await.unwrap();
        }
    }
}
