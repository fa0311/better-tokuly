//! Blocks, mirrored from the social domain.
//!
//! Producers are expected to leave blocked people out, but a block is the
//! one setting nobody may get around, so this domain enforces it as well:
//! nothing is delivered across a block in either direction, and blocking
//! someone removes what they caused from the blocker's inbox. The blocked
//! person's inbox is left as it is: blocks are silent.
//!
//! The mirror keeps one row per pair with the time of the event that set it,
//! so events handled out of order (or redelivered) end in the latest state.
use sqlx::PgExecutor;
use tokuly_core::{events::Event, prelude::*};

use crate::{Notify, events::UserBlocked, locks, withdraw::UNWITHDRAWABLE_CATEGORY};

/// Whether the two users are in a block, whoever blocked whom.
pub(crate) async fn either_blocked<'e, E: PgExecutor<'e>>(
    executor: E,
    a: Uuid,
    b: Uuid,
) -> AppResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM notify_blocks
             WHERE blocked AND ((user_id = $1 AND blocked_id = $2) OR (user_id = $2 AND blocked_id = $1))
         )",
    )
    .bind(a)
    .bind(b)
    .fetch_one(executor)
    .await?)
}

impl Notify {
    /// Handles `social.user.blocked` (`blocked = true`) and
    /// `social.user.unblocked`.
    pub async fn handle_block(&self, event: &Event, blocked: bool) -> AppResult<()> {
        let change: UserBlocked = event.data()?;
        if change.user_id == change.blocked_id {
            return Ok(());
        }
        let mut tx = self.core.db.begin().await?;
        locks::lock(&mut tx, locks::pair_key(change.user_id, change.blocked_id)).await?;
        // The join skips accounts that no longer exist (events outlive them).
        let applied: Option<bool> = sqlx::query_scalar(
            "INSERT INTO notify_blocks (user_id, blocked_id, blocked, changed_at)
             SELECT a.id, b.id, $3, $4 FROM users a, users b WHERE a.id = $1 AND b.id = $2
             ON CONFLICT (user_id, blocked_id) DO UPDATE SET blocked = EXCLUDED.blocked, changed_at = EXCLUDED.changed_at
             WHERE notify_blocks.changed_at <= EXCLUDED.changed_at
             RETURNING blocked",
        )
        .bind(change.user_id)
        .bind(change.blocked_id)
        .bind(blocked)
        .bind(event.occurred_at)
        .fetch_optional(&mut *tx)
        .await?;
        let removed: Vec<(Uuid, Uuid)> = if applied == Some(true) {
            sqlx::query_as(
                "DELETE FROM notifications WHERE user_id = $1 AND actor_id = $2 AND category <> $3
                 RETURNING user_id, id",
            )
            .bind(change.user_id)
            .bind(change.blocked_id)
            .bind(UNWITHDRAWABLE_CATEGORY)
            .fetch_all(&mut *tx)
            .await?
        } else {
            Vec::new()
        };
        tx.commit().await?;
        self.announce_removed(removed).await
    }
}
