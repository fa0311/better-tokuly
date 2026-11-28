//! Bot accounts: the account-level truth.
//!
//! The developer platform (`tokuly-dev`) makes bots: accounts nobody signs in
//! to, acting through API tokens, owned by a person. Which accounts are bots
//! and whose they are is stored here, on the account (`users.bot`,
//! `users.bot_owner_id`), so every domain can ask without depending on the
//! developer platform.
//!
//! Bots never count as people. Wherever a number or a list stands for people
//! — friend suggestions, "people you may know", leaderboards and rankings,
//! nearby discovery, intro counts, the minimum size of an anonymous group,
//! "N人が集合中" — call [`humans_only`] (or join `users` and require
//! `NOT u.bot`) instead of deciding per domain what a bot is.
use std::collections::{HashMap, HashSet};

use sqlx::PgConnection;
use tokuly_core::prelude::*;

/// Creates the account of a bot owned by `owner`: no email, no password, no
/// birthday, in the owner's language and time zone. The handle must already
/// have passed [`crate::users::validate_unheld_handle`]; `HANDLE_TAKEN` when
/// someone has it. Runs on the caller's transaction, so the developer
/// platform records the bot in the same commit. Emits nothing: a bot is not
/// a person signing up (`identity.user.created` would hand it welcome
/// rewards and invite credit).
pub async fn create_bot_account(
    tx: &mut PgConnection,
    owner: Uuid,
    handle: &str,
    display_name: &str,
    bio: &str,
) -> AppResult<Uuid> {
    let taken: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE handle = $1::citext)")
        .bind(handle)
        .fetch_one(&mut *tx)
        .await?;
    ensure!(!taken, already_exists, "HANDLE_TAKEN", "this handle is already in use");
    sqlx::query_scalar(
        "INSERT INTO users (handle, display_name, bio, locale, time_zone, bot, bot_owner_id)
         SELECT $2::citext, $3, $4, o.locale, o.time_zone, true, o.id FROM users o
         WHERE o.id = $1 AND o.deleted_at IS NULL AND NOT o.bot
         RETURNING id",
    )
    .bind(owner)
    .bind(handle)
    .bind(display_name)
    .bind(bio)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| {
        let e = AppError::from(e);
        if e.is_already_exists() {
            AppError::already_exists("HANDLE_TAKEN", "this handle is already in use")
        } else {
            e
        }
    })?
    .or_not_found("USER_NOT_FOUND", "the owner's account does not exist")
}

/// The account of a bot the server runs itself (Tokuly Bot): no owner, no
/// email, no password, made the first time it is asked for. The handle must
/// be one nobody can take ([`crate::users::claimable_handle`] refuses it),
/// so the account cannot be impersonated; concurrent first calls end with
/// the same account. `HANDLE_TAKEN` when a person or an owned bot holds it.
/// Emits nothing, like [`create_bot_account`].
pub async fn service_bot(core: &Core, handle: &str, display_name: &str, bio: &str) -> AppResult<Uuid> {
    sqlx::query(
        "INSERT INTO users (handle, display_name, bio, bot) VALUES ($1::citext, $2, $3, true)
         ON CONFLICT (handle) DO NOTHING",
    )
    .bind(handle)
    .bind(display_name)
    .bind(bio)
    .execute(&core.db)
    .await?;
    let found: Option<(Uuid, bool, Option<Uuid>)> =
        sqlx::query_as("SELECT id, bot, bot_owner_id FROM users WHERE handle = $1::citext")
            .bind(handle)
            .fetch_optional(&core.db)
            .await?;
    match found {
        Some((id, true, None)) => Ok(id),
        _ => bail!(already_exists, "HANDLE_TAKEN", "the handle of a service bot belongs to another account"),
    }
}

/// Hands a bot to `owner` (an accepted transfer), on the caller's
/// transaction. `BOT_NOT_FOUND` when `bot_id` is not a live bot; a bot
/// cannot own a bot (`OWNER_INVALID`).
pub async fn set_bot_owner(tx: &mut PgConnection, bot_id: Uuid, owner: Uuid) -> AppResult<()> {
    let owner_ok: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL AND NOT bot)")
            .bind(owner)
            .fetch_one(&mut *tx)
            .await?;
    ensure!(owner_ok, invalid, "OWNER_INVALID", "a bot belongs to a person");
    let done = sqlx::query("UPDATE users SET bot_owner_id = $2 WHERE id = $1 AND bot AND deleted_at IS NULL")
        .bind(bot_id)
        .bind(owner)
        .execute(&mut *tx)
        .await?;
    ensure!(done.rows_affected() == 1, not_found, "BOT_NOT_FOUND", "bot not found");
    Ok(())
}

/// Whether `user_id` is a bot.
pub async fn is_bot(core: &Core, user_id: Uuid) -> AppResult<bool> {
    Ok(bots_among(core, &[user_id]).await?.contains(&user_id))
}

/// The bots among `user_ids`, in one query.
pub async fn bots_among(core: &Core, user_ids: &[Uuid]) -> AppResult<HashSet<Uuid>> {
    if user_ids.is_empty() {
        return Ok(HashSet::new());
    }
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE id = ANY($1) AND bot")
        .bind(user_ids)
        .fetch_all(&core.db)
        .await?;
    Ok(ids.into_iter().collect())
}

/// The owners of the bots among `user_ids` (bot → owner). A bot whose owner
/// is gone is absent.
pub async fn bot_owners(core: &Core, user_ids: &[Uuid]) -> AppResult<HashMap<Uuid, Uuid>> {
    if user_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(Uuid, Uuid)> =
        sqlx::query_as("SELECT id, bot_owner_id FROM users WHERE id = ANY($1) AND bot AND bot_owner_id IS NOT NULL")
            .bind(user_ids)
            .fetch_all(&core.db)
            .await?;
    Ok(rows.into_iter().collect())
}

/// `user_ids` without the bots, in the same order: the one rule for every
/// place where only people count (see the module documentation).
pub async fn humans_only(core: &Core, user_ids: &[Uuid]) -> AppResult<Vec<Uuid>> {
    let bots = bots_among(core, user_ids).await?;
    Ok(user_ids.iter().copied().filter(|id| !bots.contains(id)).collect())
}
