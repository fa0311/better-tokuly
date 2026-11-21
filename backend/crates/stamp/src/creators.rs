//! Creator records: who makes packs, and who is trusted to publish without review.
use sqlx::{PgExecutor, Postgres, Transaction};
use tokuly_core::prelude::*;

/// Whether the user's packs are published without review.
pub async fn is_trusted<'e>(executor: impl PgExecutor<'e>, user_id: Uuid) -> AppResult<bool> {
    let trusted: Option<bool> = sqlx::query_scalar("SELECT trusted FROM stamp_creators WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(executor)
        .await?;
    Ok(trusted.unwrap_or(false))
}

/// Locks the user's creator record for the rest of the transaction,
/// creating it for a first-time creator. Pack creation takes this lock so
/// that the per-creator pack limit cannot be raced past.
pub async fn lock(tx: &mut Transaction<'_, Postgres>, user_id: Uuid) -> AppResult<()> {
    sqlx::query("INSERT INTO stamp_creators (user_id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("SELECT 1 FROM stamp_creators WHERE user_id = $1 FOR UPDATE").bind(user_id).execute(&mut **tx).await?;
    Ok(())
}

/// Sets the trusted flag, creating the record when a moderator flags someone
/// who has not made a pack yet. Returns whether the flag changed.
pub async fn set_trusted(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    trusted: bool,
    moderator_id: Uuid,
) -> AppResult<bool> {
    let before: Option<bool> = sqlx::query_scalar("SELECT trusted FROM stamp_creators WHERE user_id = $1 FOR UPDATE")
        .bind(user_id)
        .fetch_optional(&mut **tx)
        .await?;
    if before.unwrap_or(false) == trusted {
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO stamp_creators (user_id, trusted, trusted_by, trusted_at)
         VALUES ($1, $2, $3, CASE WHEN $2 THEN now() END)
         ON CONFLICT (user_id) DO UPDATE SET trusted = EXCLUDED.trusted, trusted_by = EXCLUDED.trusted_by, trusted_at = EXCLUDED.trusted_at",
    )
    .bind(user_id)
    .bind(trusted)
    .bind(moderator_id)
    .execute(&mut **tx)
    .await?;
    Ok(true)
}
