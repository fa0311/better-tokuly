//! Ledger operations that run inside the caller's transaction.
//!
//! [`crate::ledger`] is the ledger other domains call; each function there is
//! its own transaction. The wallet's own flows need more than that: a purchase
//! must flip to "completed" in the same transaction that grants its coins, a
//! payout request must exist if and only if its earnings are locked, and a
//! manual adjustment must never exist without its audit row. The functions
//! here post to the same tables under the same rules — account rows locked,
//! postings that sum to zero, one idempotency key per movement, balances that
//! cannot go negative — but on a transaction the caller owns and commits.
//!
//! They also cover the movements the base ledger has no operation for:
//!
//! | operation | postings |
//! |---|---|
//! | [`hold_payout`] | `user:earnings` → `system:payout_hold` |
//! | [`release_payout`] | `system:payout_hold` → `user:earnings` (a refund of the hold) |
//! | [`settle_payout`] | `system:payout_hold` → `system:paid_out` |
//! | [`adjust`] | `system:adjustments` ↔ `user:coins` / `user:earnings` |
//! | [`close_account`] | `user:coins` + `user:earnings` → `system:forfeited` |
//! | [`unlink_user`] | none: removes a deleted user's ID from entries |
//!
//! Lock order, everywhere in this crate: ledger entry rows (when existing
//! entries are reversed, settled or unlinked) before account rows. The base
//! ledger's `refund` follows the same order.
//!
//! The row-level steps (locking, posting, applying) are the ones the base
//! ledger uses, from `postings.rs`.
//!
//! The module is public so the integration tests can drive these operations
//! directly. Other domains have no business here: they use [`crate::ledger`].
use sqlx::{Postgres, Transaction};
use tokuly_core::prelude::*;

pub use crate::postings::Account;
use crate::postings::{NewEntry, announce, apply, check_amount, check_key, existing_entry, lock_account, post, system};

/// Outcome of a posting attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Posted {
    pub entry_id: Uuid,
    /// False when the idempotency key was used before: nothing moved now.
    pub applied: bool,
}

/// What to answer when `key` was used before: the original entry, not applied again.
async fn replay_of(tx: &mut Transaction<'_, Postgres>, key: &str) -> AppResult<Option<Posted>> {
    Ok(existing_entry(tx, key).await?.map(|existing| Posted { entry_id: existing.entry_id, applied: false }))
}

/// Issues new coins to a user as part of the caller's transaction.
/// [`crate::ledger::grant`] is this same operation committed on its own.
pub async fn grant(tx: &mut Transaction<'_, Postgres>, user_id: Uuid, amount: i64, reason: &str, key: &str, memo: &serde_json::Value) -> AppResult<Posted> {
    check_amount(amount)?;
    check_key(key)?;
    lock_account(tx, user_id).await?;
    if let Some(replay) = replay_of(tx, key).await? {
        return Ok(replay);
    }
    let postings = [(system::ISSUED.to_owned(), -amount), (Account::Coins.of(user_id), amount)];
    let entry = NewEntry { kind: "grant", reason, key, from: None, to: Some(user_id), amount, fee: 0, refund_of: None, memo };
    let entry_id = post(tx, entry, &postings).await?;
    let after = apply(tx, user_id, amount, 0).await?;
    announce(tx, user_id, after, entry_id, reason).await?;
    Ok(Posted { entry_id, applied: true })
}

/// Locks `amount` of a user's earnings for a payout request: the coins leave
/// the earnings balance and wait in `system:payout_hold` until the request is
/// settled ([`settle_payout`]) or returned ([`release_payout`]).
/// Fails with `INSUFFICIENT_EARNINGS` when the user has not earned that much.
pub async fn hold_payout(tx: &mut Transaction<'_, Postgres>, user_id: Uuid, amount: i64, key: &str, memo: &serde_json::Value) -> AppResult<Posted> {
    check_amount(amount)?;
    check_key(key)?;
    let before = lock_account(tx, user_id).await?;
    if let Some(replay) = replay_of(tx, key).await? {
        return Ok(replay);
    }
    ensure!(before.earnings >= amount, precondition, "INSUFFICIENT_EARNINGS", "not enough earnings");
    let postings = [(Account::Earnings.of(user_id), -amount), (system::PAYOUT_HOLD.to_owned(), amount)];
    let entry = NewEntry { kind: "payout", reason: "payout", key, from: Some(user_id), to: None, amount, fee: 0, refund_of: None, memo };
    let entry_id = post(tx, entry, &postings).await?;
    let after = apply(tx, user_id, 0, -amount).await?;
    announce(tx, user_id, after, entry_id, "payout").await?;
    Ok(Posted { entry_id, applied: true })
}

/// A payout hold, locked for the rest of the transaction.
struct Hold {
    user_id: Option<Uuid>,
    amount: i64,
    returned: bool,
}

async fn lock_hold(tx: &mut Transaction<'_, Postgres>, hold_entry_id: Uuid) -> AppResult<Hold> {
    let row: Option<(String, Option<Uuid>, i64, bool)> =
        sqlx::query_as("SELECT kind, from_user_id, amount, refunded_at IS NOT NULL FROM wallet_entries WHERE id = $1 FOR UPDATE")
            .bind(hold_entry_id)
            .fetch_optional(&mut **tx)
            .await?;
    let (kind, user_id, amount, returned) = row.or_not_found("ENTRY_NOT_FOUND", "ledger entry not found")?;
    ensure!(kind == "payout", precondition, "NOT_A_PAYOUT_HOLD", "this ledger entry is not a payout hold");
    Ok(Hold { user_id, amount, returned })
}

fn settlement_key(hold_entry_id: Uuid) -> String {
    format!("payout.settled:{hold_entry_id}")
}

/// Returns a payout hold to the user's earnings (the request was rejected or
/// cancelled). `reason` names why, e.g. `payout.rejected`. Idempotent:
/// returning twice is a no-op (`None`). Fails with `PAYOUT_ALREADY_SETTLED`
/// once the payout was paid.
///
/// When the hold was unlinked from its user ([`unlink_user`]: the account is
/// being deleted) the coins cannot go back to anyone and are forfeited
/// instead, so the hold account still empties.
pub async fn release_payout(tx: &mut Transaction<'_, Postgres>, hold_entry_id: Uuid, reason: &str) -> AppResult<Option<Uuid>> {
    let hold = lock_hold(tx, hold_entry_id).await?;
    if hold.returned {
        return Ok(None);
    }
    let settled = existing_entry(tx, &settlement_key(hold_entry_id)).await?.is_some();
    ensure!(!settled, precondition, "PAYOUT_ALREADY_SETTLED", "this payout was already paid");
    let key = format!("refund:{hold_entry_id}");
    let memo = serde_json::json!({});
    let entry = NewEntry { kind: "refund", reason, key: &key, from: None, to: hold.user_id, amount: hold.amount, fee: 0, refund_of: Some(hold_entry_id), memo: &memo };
    let refund_id = match hold.user_id {
        Some(user_id) => {
            lock_account(tx, user_id).await?;
            let postings = [(system::PAYOUT_HOLD.to_owned(), -hold.amount), (Account::Earnings.of(user_id), hold.amount)];
            let refund_id = post(tx, entry, &postings).await?;
            let after = apply(tx, user_id, 0, hold.amount).await?;
            announce(tx, user_id, after, refund_id, reason).await?;
            refund_id
        }
        None => {
            let postings = [(system::PAYOUT_HOLD.to_owned(), -hold.amount), (system::FORFEITED.to_owned(), hold.amount)];
            post(tx, entry, &postings).await?
        }
    };
    sqlx::query("UPDATE wallet_entries SET refunded_at = now() WHERE id = $1").bind(hold_entry_id).execute(&mut **tx).await?;
    Ok(Some(refund_id))
}

/// Records that a payout hold was paid out as money: the coins move from
/// `system:payout_hold` to `system:paid_out` and are gone for good. No user
/// balance changes (the hold already took the coins), so the settlement entry
/// names no user and stays out of ledger histories. Idempotent per hold.
/// Fails with `PAYOUT_ALREADY_RETURNED` when the hold went back to the user.
pub async fn settle_payout(tx: &mut Transaction<'_, Postgres>, hold_entry_id: Uuid, memo: &serde_json::Value) -> AppResult<Posted> {
    let hold = lock_hold(tx, hold_entry_id).await?;
    ensure!(!hold.returned, precondition, "PAYOUT_ALREADY_RETURNED", "this payout hold was already returned to the user");
    let key = settlement_key(hold_entry_id);
    if let Some(replay) = replay_of(tx, &key).await? {
        return Ok(replay);
    }
    let postings = [(system::PAYOUT_HOLD.to_owned(), -hold.amount), (system::PAID_OUT.to_owned(), hold.amount)];
    let entry = NewEntry { kind: "payout_settlement", reason: "payout.paid", key: &key, from: None, to: None, amount: hold.amount, fee: 0, refund_of: None, memo };
    let entry_id = post(tx, entry, &postings).await?;
    Ok(Posted { entry_id, applied: true })
}

/// A manual correction by an admin: adds (`delta > 0`) or removes
/// (`delta < 0`) coins on one of a user's balances, against
/// `system:adjustments`. Fails with `INSUFFICIENT_COINS` /
/// `INSUFFICIENT_EARNINGS` when a removal would push the balance below zero.
pub async fn adjust(tx: &mut Transaction<'_, Postgres>, user_id: Uuid, account: Account, delta: i64, key: &str, memo: &serde_json::Value) -> AppResult<Posted> {
    ensure!(delta != 0 && delta != i64::MIN, invalid, "AMOUNT_INVALID", "an adjustment cannot be zero");
    let amount = delta.abs();
    check_amount(amount)?;
    check_key(key)?;
    let before = lock_account(tx, user_id).await?;
    if let Some(replay) = replay_of(tx, key).await? {
        return Ok(replay);
    }
    let user_account = account.of(user_id);
    let (from, to, postings) = if delta > 0 {
        (None, Some(user_id), [(system::ADJUSTMENTS.to_owned(), -amount), (user_account, amount)])
    } else {
        match account {
            Account::Coins => ensure!(before.coins >= amount, precondition, "INSUFFICIENT_COINS", "not enough coins"),
            Account::Earnings => ensure!(before.earnings >= amount, precondition, "INSUFFICIENT_EARNINGS", "not enough earnings"),
        }
        (Some(user_id), None, [(user_account, -amount), (system::ADJUSTMENTS.to_owned(), amount)])
    };
    let entry = NewEntry { kind: "adjustment", reason: "adjustment", key, from, to, amount, fee: 0, refund_of: None, memo };
    let entry_id = post(tx, entry, &postings).await?;
    let after = match account {
        Account::Coins => apply(tx, user_id, delta, 0).await?,
        Account::Earnings => apply(tx, user_id, 0, delta).await?,
    };
    announce(tx, user_id, after, entry_id, "adjustment").await?;
    Ok(Posted { entry_id, applied: true })
}

/// Unlinks a deleted user from every ledger entry, doing by hand what the
/// schema's `ON DELETE SET NULL` would do if the user row were removed. The
/// entries stay for accounting but no longer say whose they were;
/// counterparts still see their side of a transfer, without a name.
///
/// Takes entry-row locks, so it must run before anything in the same
/// transaction locks an account row (see the lock order in the module docs).
pub async fn unlink_user(tx: &mut Transaction<'_, Postgres>, user_id: Uuid) -> AppResult<()> {
    sqlx::query("UPDATE wallet_entries SET from_user_id = NULL WHERE from_user_id = $1").bind(user_id).execute(&mut **tx).await?;
    sqlx::query("UPDATE wallet_entries SET to_user_id = NULL WHERE to_user_id = $1").bind(user_id).execute(&mut **tx).await?;
    Ok(())
}

/// Closes the account of a deleted user: whatever is left on both balances
/// is forfeited to `system:forfeited` with a closing entry (so the postings
/// of the account sum to zero) and the account row is deleted, as the
/// schema's `ON DELETE CASCADE` would. The closing entry is written already
/// unlinked from the user.
///
/// Returns the closing entry, if one was needed. Safe to repeat: a second
/// call finds no account to close.
pub async fn close_account(tx: &mut Transaction<'_, Postgres>, user_id: Uuid) -> AppResult<Option<Uuid>> {
    let row: Option<(i64, i64)> = sqlx::query_as("SELECT coins, earnings FROM wallet_accounts WHERE user_id = $1 FOR UPDATE").bind(user_id).fetch_optional(&mut **tx).await?;
    let Some((coins, earnings)) = row else { return Ok(None) };
    let mut closing_entry = None;
    let total = coins + earnings;
    if total > 0 {
        let mut postings = vec![(system::FORFEITED.to_owned(), total)];
        if coins > 0 {
            postings.push((Account::Coins.of(user_id), -coins));
        }
        if earnings > 0 {
            postings.push((Account::Earnings.of(user_id), -earnings));
        }
        // The account row is deleted below, and that is what makes a repeat
        // harmless; the key only has to be unique.
        let key = format!("closure:{}", new_id());
        let memo = serde_json::json!({ "coins": coins, "earnings": earnings });
        let entry = NewEntry { kind: "closure", reason: "account.deleted", key: &key, from: None, to: None, amount: total, fee: 0, refund_of: None, memo: &memo };
        closing_entry = Some(post(tx, entry, &postings).await?);
    }
    sqlx::query("DELETE FROM wallet_accounts WHERE user_id = $1").bind(user_id).execute(&mut **tx).await?;
    Ok(closing_entry)
}
