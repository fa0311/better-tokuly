//! The row-level steps every ledger movement is made of.
//!
//! [`crate::ledger`] (operations that are their own transaction, called by
//! other domains) and [`crate::ledger_tx`] (operations on the caller's
//! transaction, used by the wallet itself) both build their movements from
//! the functions here, always in this order:
//!
//! 1. [`check_amount`], [`check_key`] — refuse nonsense before touching a row;
//! 2. [`lock_account`] — lock every account row involved (two accounts: in
//!    ascending user ID, so opposite movements cannot deadlock);
//! 3. [`existing_entry`] — stop if the idempotency key was used before;
//! 4. [`post`] — write the entry with postings that sum to zero;
//! 5. [`apply`], [`announce`] — change the balances and emit
//!    `wallet.balance.changed`.
//!
//! Nothing here begins or commits a transaction: that is the caller's job.
use sqlx::{Postgres, Transaction};
use tokuly_core::prelude::*;

use crate::ledger::{Balance, BalanceChanged};

/// Largest single movement.
pub(crate) const MAX_AMOUNT: i64 = 100_000_000;

/// Posting accounts of the platform itself. Together with the users'
/// accounts ([`Account::of`]) these are all the accounts there are.
pub(crate) mod system {
    /// Where coins come from: grants (purchases, bonuses) are posted against it.
    pub const ISSUED: &str = "system:issued";
    /// Coins spent on things the platform sells.
    pub const SPENT: &str = "system:spent";
    /// Platform fees withheld from transfers.
    pub const FEES: &str = "system:fees";
    /// Earnings locked in open payout requests.
    pub const PAYOUT_HOLD: &str = "system:payout_hold";
    /// Earnings paid out as money.
    pub const PAID_OUT: &str = "system:paid_out";
    /// The other side of manual corrections by admins.
    pub const ADJUSTMENTS: &str = "system:adjustments";
    /// Balances left behind by deleted accounts.
    pub const FORFEITED: &str = "system:forfeited";
}

/// One of a user's two balances.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Account {
    Coins,
    Earnings,
}

impl Account {
    /// Name used in the adjustment log (`wallet_adjustments.account`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Coins => "coins",
            Self::Earnings => "earnings",
        }
    }

    /// The posting account of `user_id` for this balance.
    pub fn of(self, user_id: Uuid) -> String {
        format!("user:{user_id}:{}", self.name())
    }
}

pub(crate) fn check_amount(amount: i64) -> AppResult<()> {
    ensure!((1..=MAX_AMOUNT).contains(&amount), invalid, "AMOUNT_INVALID", "amount must be between 1 and 100,000,000 coins");
    Ok(())
}

pub(crate) fn check_key(key: &str) -> AppResult<()> {
    ensure!(!key.is_empty() && key.len() <= 200, invalid, "IDEMPOTENCY_KEY_INVALID", "an idempotency key of 1-200 characters is required");
    Ok(())
}

/// The entry an idempotency key was already used for.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ExistingEntry {
    #[sqlx(rename = "id")]
    pub entry_id: Uuid,
    pub kind: String,
    #[sqlx(rename = "from_user_id")]
    pub from: Option<Uuid>,
    #[sqlx(rename = "to_user_id")]
    pub to: Option<Uuid>,
    pub amount: i64,
    /// Platform fee the entry withheld (transfers; 0 otherwise).
    pub fee: i64,
    /// The entry was reversed since (`refunded_at`).
    pub refunded: bool,
}

impl ExistingEntry {
    /// The entry, when the call that found it asks for the same movement
    /// (a retry); `IDEMPOTENCY_KEY_REUSED` when it asks for another one. A
    /// key names one movement — kind, payer, recipient, amount — so a
    /// different movement under the same key must not be answered as if it
    /// had already happened: its caller would get what it pays for for free.
    ///
    /// A retry of a movement that was refunded since fails with
    /// `ENTRY_REFUNDED`: answering "done" would let its caller deliver what
    /// was paid for although the coins are back with the payer.
    pub fn retry_of(self, kind: &str, from: Option<Uuid>, to: Option<Uuid>, amount: i64) -> AppResult<Self> {
        let same = self.kind == kind && self.from == from && self.to == to && self.amount == amount;
        if !same {
            return Err(key_reused());
        }
        ensure!(!self.refunded, precondition, "ENTRY_REFUNDED", "this movement was refunded; it is not made again under the same key");
        Ok(self)
    }
}

pub(crate) fn key_reused() -> AppError {
    AppError::already_exists("IDEMPOTENCY_KEY_REUSED", "this idempotency key was used for a different movement")
}

/// Looks an idempotency key up. Call it after [`lock_account`]: under the
/// account lock a concurrent call with the same key has either committed
/// (and is found here) or has not posted yet. (A call with the same key for
/// other accounts is not held back by the lock; [`post`] refuses it.)
pub(crate) async fn existing_entry(tx: &mut Transaction<'_, Postgres>, key: &str) -> AppResult<Option<ExistingEntry>> {
    Ok(sqlx::query_as("SELECT id, kind, from_user_id, to_user_id, amount, fee, refunded_at IS NOT NULL AS refunded FROM wallet_entries WHERE idempotency_key = $1")
        .bind(key)
        .fetch_optional(&mut **tx)
        .await?)
}

/// Locks (creating if needed) a user's account row and returns its balance.
/// Every movement starts with this lock; callers take it themselves first
/// when they must check something of their own (an open payout request,
/// say) under the same lock that serializes the user's movements.
pub(crate) async fn lock_account(tx: &mut Transaction<'_, Postgres>, user_id: Uuid) -> AppResult<Balance> {
    sqlx::query("INSERT INTO wallet_accounts (user_id) VALUES ($1) ON CONFLICT DO NOTHING").bind(user_id).execute(&mut **tx).await?;
    let (coins, earnings): (i64, i64) =
        sqlx::query_as("SELECT coins, earnings FROM wallet_accounts WHERE user_id = $1 FOR UPDATE").bind(user_id).fetch_one(&mut **tx).await?;
    Ok(Balance { coins, earnings })
}

/// Adds the (signed) deltas to a locked account and returns the new balance.
/// The table's `CHECK` constraints are the last line of defence against a
/// negative balance; callers check affordability first to fail with a
/// meaningful reason.
pub(crate) async fn apply(tx: &mut Transaction<'_, Postgres>, user_id: Uuid, coins_delta: i64, earnings_delta: i64) -> AppResult<Balance> {
    let (coins, earnings): (i64, i64) = sqlx::query_as(
        "UPDATE wallet_accounts SET coins = coins + $2, earnings = earnings + $3, updated_at = now() WHERE user_id = $1 RETURNING coins, earnings",
    )
    .bind(user_id)
    .bind(coins_delta)
    .bind(earnings_delta)
    .fetch_one(&mut **tx)
    .await?;
    Ok(Balance { coins, earnings })
}

/// The `wallet_entries` row of a movement about to be posted.
pub(crate) struct NewEntry<'a> {
    /// 'grant' | 'spend' | 'transfer' | 'refund' | 'payout' |
    /// 'payout_settlement' | 'adjustment' | 'closure'
    pub kind: &'a str,
    /// What the movement was for, e.g. `live.gift`.
    pub reason: &'a str,
    /// Idempotency key; unique across the whole ledger.
    pub key: &'a str,
    pub from: Option<Uuid>,
    pub to: Option<Uuid>,
    pub amount: i64,
    /// Platform fee taken out of `amount` (transfers; 0 otherwise).
    pub fee: i64,
    /// For refunds: the entry being reversed.
    pub refund_of: Option<Uuid>,
    pub memo: &'a serde_json::Value,
}

/// Writes an entry and its postings, given as `(account, delta)` pairs.
/// Refuses postings that do not sum to zero: an unbalanced entry would
/// silently create or destroy coins.
///
/// Fails with `ACCOUNT_CLOSED` when a user the entry names was erased by
/// identity (`users.deleted_at`): that wallet is closed, or about to be by
/// the `identity.user.deleted` consumer, and coins moved into it now would
/// re-open it on a tombstone where nobody can use them. Checked here, after
/// the caller looked its key up, so that the retry of a movement that did
/// happen is still answered as a retry.
pub(crate) async fn post(tx: &mut Transaction<'_, Postgres>, entry: NewEntry<'_>, postings: &[(String, i64)]) -> AppResult<Uuid> {
    if postings.iter().map(|(_, delta)| delta).sum::<i64>() != 0 {
        return Err(AppError::internal(anyhow::anyhow!("unbalanced postings for a `{}` entry", entry.kind)));
    }
    let parties: Vec<Uuid> = [entry.from, entry.to].into_iter().flatten().collect();
    if !parties.is_empty() {
        let closed: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = ANY($1) AND deleted_at IS NOT NULL)").bind(&parties).fetch_one(&mut **tx).await?;
        ensure!(!closed, precondition, "ACCOUNT_CLOSED", "this account was deleted");
    }
    let inserted: Result<Uuid, sqlx::Error> = sqlx::query_scalar(
        "INSERT INTO wallet_entries (kind, reason, idempotency_key, from_user_id, to_user_id, amount, fee, refund_of, memo)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id",
    )
    .bind(entry.kind)
    .bind(entry.reason)
    .bind(entry.key)
    .bind(entry.from)
    .bind(entry.to)
    .bind(entry.amount)
    .bind(entry.fee)
    .bind(entry.refund_of)
    .bind(entry.memo)
    .fetch_one(&mut **tx)
    .await;
    let entry_id = match inserted.map_err(AppError::from) {
        Ok(entry_id) => entry_id,
        // The only unique column: the same key raced in for other accounts,
        // whose lock this call does not hold. The same movement holds the
        // same lock, so this is never a retry.
        Err(e) if e.is_already_exists() => return Err(key_reused()),
        Err(e) => return Err(e),
    };
    let (accounts, deltas): (Vec<&str>, Vec<i64>) = postings.iter().map(|(account, delta)| (account.as_str(), *delta)).unzip();
    // One statement for all postings; `position` keeps their IDs in the order given.
    sqlx::query(
        "INSERT INTO wallet_postings (entry_id, account, delta)
         SELECT $1, p.account, p.delta FROM unnest($2::text[], $3::bigint[]) WITH ORDINALITY AS p (account, delta, position) ORDER BY p.position",
    )
    .bind(entry_id)
    .bind(accounts)
    .bind(deltas)
    .execute(&mut **tx)
    .await?;
    Ok(entry_id)
}

/// Emits `wallet.balance.changed` for a user whose balance an entry changed.
pub(crate) async fn announce(tx: &mut Transaction<'_, Postgres>, user_id: Uuid, balance: Balance, entry_id: Uuid, reason: &str) -> AppResult<()> {
    tokuly_core::events::emit(
        &mut **tx,
        crate::events::BALANCE_CHANGED,
        Some(user_id),
        &BalanceChanged { user_id, coins: balance.coins, earnings: balance.earnings, entry_id, reason: reason.to_owned() },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_and_keys_are_bounded() {
        assert!(check_amount(1).is_ok() && check_amount(MAX_AMOUNT).is_ok());
        for bad in [0, -1, MAX_AMOUNT + 1, i64::MIN] {
            assert_eq!(check_amount(bad).unwrap_err().reason, "AMOUNT_INVALID");
        }
        assert!(check_key("k").is_ok() && check_key(&"k".repeat(200)).is_ok());
        for bad in [String::new(), "k".repeat(201)] {
            assert_eq!(check_key(&bad).unwrap_err().reason, "IDEMPOTENCY_KEY_INVALID");
        }
    }

    #[test]
    fn posting_accounts_are_named_after_the_user() {
        let user = new_id();
        assert_eq!(Account::Coins.of(user), format!("user:{user}:coins"));
        assert_eq!(Account::Earnings.of(user), format!("user:{user}:earnings"));
        assert_eq!((Account::Coins.name(), Account::Earnings.name()), ("coins", "earnings"));
    }
}
