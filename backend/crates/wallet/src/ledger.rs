//! The coin ledger: the only code that changes balances.
//!
//! Every movement is one `wallet_entries` row plus balanced
//! `wallet_postings`, applied in a single transaction with the account rows
//! locked. Balances cannot go negative, and every operation is idempotent on
//! its key: repeating a call with the same key returns the original entry
//! without moving coins again. Other domains (live gifts, paid stamp packs)
//! call [`spend`], [`transfer`] and [`refund`] directly.
//!
//! Each function here is a transaction of its own. The steps a movement is
//! made of live in `postings.rs`; the wallet's own operations, which run on
//! their caller's transaction, are in [`crate::ledger_tx`].
use serde::{Deserialize, Serialize};
use tokuly_core::prelude::*;

use crate::{
    ledger_tx,
    postings::{Account, NewEntry, announce, apply, check_amount, check_key, existing_entry, lock_account, post, system},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Balance {
    /// Spendable coins.
    pub coins: i64,
    /// Coins earned from others (gifts, sales).
    pub earnings: i64,
}

/// Result of a transfer between users.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferReceipt {
    pub entry_id: Uuid,
    /// Platform fee withheld.
    pub fee: i64,
    /// What the recipient's earnings grew by (`amount - fee`).
    pub net: i64,
    /// False when this call repeated an earlier one (nothing moved now).
    pub applied: bool,
}

/// Emitted as `wallet.balance.changed` whenever a user's balance changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceChanged {
    pub user_id: Uuid,
    pub coins: i64,
    pub earnings: i64,
    pub entry_id: Uuid,
    pub reason: String,
}

/// A user's balance (zero for users who never had coins).
pub async fn balance(core: &Core, user_id: Uuid) -> AppResult<Balance> {
    let row: Option<(i64, i64)> = sqlx::query_as("SELECT coins, earnings FROM wallet_accounts WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(&core.db)
        .await?;
    Ok(row.map(|(coins, earnings)| Balance { coins, earnings }).unwrap_or(Balance { coins: 0, earnings: 0 }))
}

/// A movement as its idempotency key finds it.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct KeyedEntry {
    #[sqlx(rename = "id")]
    pub entry_id: Uuid,
    pub amount: i64,
    /// Platform fee withheld (transfers; 0 otherwise).
    pub fee: i64,
    /// The movement was reversed since.
    pub refunded: bool,
}

/// The movement made under `idempotency_key`, if one was. Lets a caller that
/// lost the answer of a [`spend`] or [`transfer`] (a timeout, a crash) tell
/// "paid" from "not paid" without making the movement.
pub async fn find_by_key(core: &Core, idempotency_key: &str) -> AppResult<Option<KeyedEntry>> {
    Ok(sqlx::query_as("SELECT id, amount, fee, refunded_at IS NOT NULL AS refunded FROM wallet_entries WHERE idempotency_key = $1")
        .bind(idempotency_key)
        .fetch_optional(&core.db)
        .await?)
}

/// Issues new coins to a user (a purchase, a bonus, an admin adjustment).
/// Returns the entry ID and whether this call applied it.
pub async fn grant(core: &Core, user_id: Uuid, amount: i64, reason: &str, idempotency_key: &str, memo: serde_json::Value) -> AppResult<(Uuid, bool)> {
    // Checked here as well so that, as with the other operations, a bad
    // request is refused before a connection is taken.
    check_amount(amount)?;
    check_key(idempotency_key)?;
    // The wallet's own flows grant inside their transactions
    // (`ledger_tx::grant`); this is that operation as a transaction of its own.
    let mut tx = core.db.begin().await?;
    let granted = ledger_tx::grant(&mut tx, user_id, amount, reason, idempotency_key, &memo).await?;
    if granted.applied {
        tx.commit().await?;
    }
    Ok((granted.entry_id, granted.applied))
}

/// Takes coins from a user for something the platform sells.
/// Fails with `INSUFFICIENT_COINS` when the balance is too low.
/// Returns the entry ID and whether this call applied it.
pub async fn spend(core: &Core, user_id: Uuid, amount: i64, reason: &str, idempotency_key: &str, memo: serde_json::Value) -> AppResult<(Uuid, bool)> {
    check_amount(amount)?;
    check_key(idempotency_key)?;
    let mut tx = core.db.begin().await?;
    let before = lock_account(&mut tx, user_id).await?;
    if let Some(existing) = existing_entry(&mut tx, idempotency_key).await? {
        return Ok((existing.retry_of("spend", Some(user_id), None, amount)?.entry_id, false));
    }
    ensure!(before.coins >= amount, precondition, "INSUFFICIENT_COINS", "not enough coins");
    let postings = [(Account::Coins.of(user_id), -amount), (system::SPENT.to_owned(), amount)];
    let entry = NewEntry { kind: "spend", reason, key: idempotency_key, from: Some(user_id), to: None, amount, fee: 0, refund_of: None, memo: &memo };
    let entry_id = post(&mut tx, entry, &postings).await?;
    let after = apply(&mut tx, user_id, -amount, 0).await?;
    announce(&mut tx, user_id, after, entry_id, reason).await?;
    tx.commit().await?;
    Ok((entry_id, true))
}

/// Moves coins from a payer to a recipient's earnings, withholding
/// `fee_percent` (0-100) for the platform. Fails with `INSUFFICIENT_COINS`
/// when the payer cannot afford it.
pub async fn transfer(
    core: &Core,
    from_user: Uuid,
    to_user: Uuid,
    amount: i64,
    fee_percent: u32,
    reason: &str,
    idempotency_key: &str,
    memo: serde_json::Value,
) -> AppResult<TransferReceipt> {
    transfer_inner(core, from_user, to_user, amount, fee_percent, reason, idempotency_key, memo, false).await
}

/// [`transfer`] to a recipient whose account the payer must not learn: a
/// gift to a channel pays the channel's owner, and a channel does not
/// reveal who is behind it. The payer's history, live updates and data
/// export show the entry without a counterpart (`counterpart_hidden`), and
/// so does the refund of such a transfer; say in `memo` what was paid (the
/// channel, the stream). The recipient sees the payer as usual.
#[allow(clippy::too_many_arguments)]
pub async fn transfer_to_hidden_recipient(
    core: &Core,
    from_user: Uuid,
    to_user: Uuid,
    amount: i64,
    fee_percent: u32,
    reason: &str,
    idempotency_key: &str,
    memo: serde_json::Value,
) -> AppResult<TransferReceipt> {
    transfer_inner(core, from_user, to_user, amount, fee_percent, reason, idempotency_key, memo, true).await
}

#[allow(clippy::too_many_arguments)]
async fn transfer_inner(
    core: &Core,
    from_user: Uuid,
    to_user: Uuid,
    amount: i64,
    fee_percent: u32,
    reason: &str,
    idempotency_key: &str,
    memo: serde_json::Value,
    hide_recipient: bool,
) -> AppResult<TransferReceipt> {
    check_amount(amount)?;
    check_key(idempotency_key)?;
    ensure!(from_user != to_user, invalid, "SELF_TRANSFER", "you cannot send coins to yourself");
    ensure!(fee_percent <= 100, invalid, "FEE_INVALID", "fee must be 0-100 percent");
    let fee = amount * fee_percent as i64 / 100;
    let net = amount - fee;
    let mut tx = core.db.begin().await?;
    // Lock both accounts in a stable order so opposite transfers cannot deadlock.
    let (first, second) = if from_user < to_user { (from_user, to_user) } else { (to_user, from_user) };
    let first_balance = lock_account(&mut tx, first).await?;
    let second_balance = lock_account(&mut tx, second).await?;
    if let Some(existing) = existing_entry(&mut tx, idempotency_key).await? {
        // What the original moved, even if the fee setting changed since.
        let existing = existing.retry_of("transfer", Some(from_user), Some(to_user), amount)?;
        return Ok(TransferReceipt { entry_id: existing.entry_id, fee: existing.fee, net: existing.amount - existing.fee, applied: false });
    }
    let payer = if first == from_user { first_balance } else { second_balance };
    ensure!(payer.coins >= amount, precondition, "INSUFFICIENT_COINS", "not enough coins");
    let mut postings = vec![(Account::Coins.of(from_user), -amount), (Account::Earnings.of(to_user), net)];
    if fee > 0 {
        postings.push((system::FEES.to_owned(), fee));
    }
    let entry = NewEntry { kind: "transfer", reason, key: idempotency_key, from: Some(from_user), to: Some(to_user), amount, fee, refund_of: None, memo: &memo };
    let entry_id = post(&mut tx, entry, &postings).await?;
    if hide_recipient {
        sqlx::query("UPDATE wallet_entries SET hidden_user_id = $2 WHERE id = $1").bind(entry_id).bind(to_user).execute(&mut *tx).await?;
    }
    let payer_after = apply(&mut tx, from_user, -amount, 0).await?;
    let recipient_after = apply(&mut tx, to_user, 0, net).await?;
    announce(&mut tx, from_user, payer_after, entry_id, reason).await?;
    announce(&mut tx, to_user, recipient_after, entry_id, reason).await?;
    tx.commit().await?;
    Ok(TransferReceipt { entry_id, fee, net, applied: true })
}

/// Reverses a spend or transfer. Idempotent: refunding twice is a no-op.
/// Fails with `REFUND_IMPOSSIBLE` when the recipient already used the
/// earnings, or when either side's account was deleted.
pub async fn refund(core: &Core, entry_id: Uuid, reason: &str) -> AppResult<()> {
    refund_inner(core, entry_id, reason).await.map_err(|e| {
        // Coins cannot go back to, or be taken back from, a deleted account.
        if e.reason == "ACCOUNT_CLOSED" { AppError::precondition("REFUND_IMPOSSIBLE", "an account of this movement was deleted") } else { e }
    })
}

async fn refund_inner(core: &Core, entry_id: Uuid, reason: &str) -> AppResult<()> {
    let mut tx = core.db.begin().await?;
    let row: Option<(String, Option<Uuid>, Option<Uuid>, i64, i64, bool)> = sqlx::query_as(
        "SELECT kind, from_user_id, to_user_id, amount, fee, refunded_at IS NOT NULL FROM wallet_entries WHERE id = $1 FOR UPDATE",
    )
    .bind(entry_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (kind, from, to, amount, fee, refunded) = row.or_not_found("ENTRY_NOT_FOUND", "ledger entry not found")?;
    if refunded {
        return Ok(());
    }
    let payer = from.or_not_found("REFUND_IMPOSSIBLE", "the payer no longer exists")?;
    let key = format!("refund:{entry_id}");
    let memo = serde_json::json!({});
    let refund_id = match kind.as_str() {
        "spend" => {
            lock_account(&mut tx, payer).await?;
            let postings = [(system::SPENT.to_owned(), -amount), (Account::Coins.of(payer), amount)];
            let entry = NewEntry { kind: "refund", reason, key: &key, from: None, to: Some(payer), amount, fee: 0, refund_of: Some(entry_id), memo: &memo };
            let id = post(&mut tx, entry, &postings).await?;
            let after = apply(&mut tx, payer, amount, 0).await?;
            announce(&mut tx, payer, after, id, reason).await?;
            id
        }
        "transfer" => {
            let recipient = to.or_not_found("REFUND_IMPOSSIBLE", "the recipient no longer exists")?;
            let net = amount - fee;
            let (first, second) = if payer < recipient { (payer, recipient) } else { (recipient, payer) };
            let first_balance = lock_account(&mut tx, first).await?;
            let second_balance = lock_account(&mut tx, second).await?;
            let recipient_balance = if first == recipient { first_balance } else { second_balance };
            ensure!(recipient_balance.earnings >= net, precondition, "REFUND_IMPOSSIBLE", "the recipient already withdrew these earnings");
            let mut postings = vec![(Account::Earnings.of(recipient), -net), (Account::Coins.of(payer), amount)];
            if fee > 0 {
                postings.push((system::FEES.to_owned(), -fee));
            }
            let entry = NewEntry { kind: "refund", reason, key: &key, from: Some(recipient), to: Some(payer), amount, fee: 0, refund_of: Some(entry_id), memo: &memo };
            let id = post(&mut tx, entry, &postings).await?;
            let payer_after = apply(&mut tx, payer, amount, 0).await?;
            let recipient_after = apply(&mut tx, recipient, 0, -net).await?;
            announce(&mut tx, payer, payer_after, id, reason).await?;
            announce(&mut tx, recipient, recipient_after, id, reason).await?;
            id
        }
        _ => bail!(precondition, "REFUND_IMPOSSIBLE", "only spends and transfers can be refunded"),
    };
    // The refund keeps the same account hidden as the entry it reverses.
    sqlx::query("UPDATE wallet_entries SET hidden_user_id = (SELECT hidden_user_id FROM wallet_entries WHERE id = $1) WHERE id = $2")
        .bind(entry_id)
        .bind(refund_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE wallet_entries SET refunded_at = now() WHERE id = $1").bind(entry_id).execute(&mut *tx).await?;
    tracing::info!(%entry_id, %refund_id, "ledger entry refunded");
    tx.commit().await?;
    Ok(())
}
