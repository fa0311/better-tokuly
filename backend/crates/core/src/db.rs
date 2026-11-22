//! PostgreSQL pool and migrations.
use std::time::Duration;

use sqlx::{PgPool, postgres::PgPoolOptions};

pub async fn connect(url: &str, max_connections: u32) -> anyhow::Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .idle_timeout(Duration::from_secs(300))
        .connect(url)
        .await
        .map_err(|e| anyhow::anyhow!("cannot connect to PostgreSQL: {e}"))
}

/// Applies pending migrations from `migrator` (embedded by the server binary). Safe to run from several processes at once
/// (sqlx takes an advisory lock).
pub async fn migrate(pool: &PgPool, migrator: &sqlx::migrate::Migrator) -> anyhow::Result<()> {
    migrator
        .run(pool)
        .await
        .map_err(|e| anyhow::anyhow!("database migration failed: {e}"))?;
    Ok(())
}

/// Cheap liveness check for readiness probes.
pub async fn ping(pool: &PgPool) -> bool {
    sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(pool).await.is_ok()
}
