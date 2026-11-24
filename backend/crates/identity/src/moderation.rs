//! Account-level moderation actions, called by the admin domain.
use sqlx::PgConnection;
use tokuly_core::{events, prelude::*};

use crate::{Identity, maintenance::UserRef};

impl Identity {
    /// Suspends an account (`reason` set) or lifts the suspension (`None`).
    /// A suspended user is signed out everywhere and cannot sign in.
    pub async fn set_banned(&self, user_id: Uuid, reason: Option<&str>) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        set_banned_on(&mut tx, user_id, reason).await?;
        tx.commit().await?;
        if reason.is_some() {
            self.revoke_all_sessions(user_id, None).await?;
        }
        Ok(())
    }

    /// Replaces a user's roles (`admin`, `moderator`, `staff`). Takes effect
    /// when their access token is next refreshed.
    pub async fn set_roles(&self, user_id: Uuid, roles: &[String]) -> AppResult<()> {
        set_roles_on(&mut *self.core.db.acquire().await?, user_id, roles).await
    }

    /// Grants or removes the verified badge.
    pub async fn set_verified(&self, user_id: Uuid, verified: bool) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let done = sqlx::query("UPDATE users SET verified = $2 WHERE id = $1 AND deleted_at IS NULL")
            .bind(user_id)
            .bind(verified)
            .execute(&mut *tx)
            .await?;
        ensure!(done.rows_affected() == 1, not_found, "USER_NOT_FOUND", "user not found");
        events::emit(&mut *tx, "identity.user.updated", None, &UserRef { user_id }).await?;
        tx.commit().await?;
        Ok(())
    }
}

/// [`Identity::set_banned`] on the caller's transaction, for a domain that
/// records the sanction in the same transaction (the admin domain): the
/// account is locked if and only if the caller commits, and no second
/// connection is needed while the caller holds its locks. A suspension
/// (`reason` set) does not sign the account out here: once the transaction
/// has committed, call [`Identity::revoke_all_sessions`].
pub async fn set_banned_on(conn: &mut PgConnection, user_id: Uuid, reason: Option<&str>) -> AppResult<()> {
    let done = match reason {
        Some(reason) => sqlx::query("UPDATE users SET banned_at = COALESCE(banned_at, now()), ban_reason = $2 WHERE id = $1 AND deleted_at IS NULL")
            .bind(user_id)
            .bind(reason)
            .execute(&mut *conn)
            .await?,
        None => sqlx::query("UPDATE users SET banned_at = NULL, ban_reason = '' WHERE id = $1 AND deleted_at IS NULL")
            .bind(user_id)
            .execute(&mut *conn)
            .await?,
    };
    ensure!(done.rows_affected() == 1, not_found, "USER_NOT_FOUND", "user not found");
    let topic = if reason.is_some() { "identity.user.banned" } else { "identity.user.unbanned" };
    events::emit(&mut *conn, topic, None, &UserRef { user_id }).await?;
    Ok(())
}

/// [`Identity::set_roles`] on the caller's transaction.
pub async fn set_roles_on(conn: &mut PgConnection, user_id: Uuid, roles: &[String]) -> AppResult<()> {
    for role in roles {
        ensure!(
            matches!(role.as_str(), "admin" | "moderator" | "staff"),
            invalid,
            "ROLE_INVALID",
            "unknown role {role}"
        );
    }
    let done = sqlx::query("UPDATE users SET roles = $2 WHERE id = $1 AND deleted_at IS NULL")
        .bind(user_id)
        .bind(roles)
        .execute(&mut *conn)
        .await?;
    ensure!(done.rows_affected() == 1, not_found, "USER_NOT_FOUND", "user not found");
    Ok(())
}
