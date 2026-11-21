//! Manual balance corrections by admins, and their audit log.
//!
//! An adjustment is a ledger entry (`kind = 'adjustment'`) plus a row in
//! `wallet_adjustments` saying who did it and why, written in one
//! transaction: there is no adjustment without its audit record. The log is
//! append-only; a mistake is corrected with another adjustment.
use tokuly_core::prelude::*;
use tokuly_proto::{
    common::PageInfo,
    wallet::{Adjustment, LedgerAccount},
};

use crate::{
    Wallet,
    entries::NEWEST,
    ledger_tx,
    postings::{Account, MAX_AMOUNT, lock_account},
};

const MAX_REASON_CHARS: usize = 500;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct AdjustmentRow {
    pub id: Uuid,
    pub admin_id: Option<Uuid>,
    /// `None` once the adjusted account was deleted.
    pub user_id: Option<Uuid>,
    pub account: String,
    pub delta: i64,
    pub reason: String,
    pub entry_id: Uuid,
    pub created_at: Time,
}

macro_rules! adjustment_columns {
    () => {
        "id, admin_id, user_id, account, delta, reason, entry_id, created_at"
    };
}

/// An adjustment as an admin submitted it, checked and normalized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AdjustmentInput {
    pub user_id: Uuid,
    pub account: Account,
    pub delta: i64,
    pub reason: String,
    pub idempotency_key: String,
}

pub(crate) fn validate_adjustment(user_id: &str, account: Option<LedgerAccount>, delta: i64, reason: &str, idempotency_key: &str) -> AppResult<AdjustmentInput> {
    let user_id = parse_id(user_id)?;
    let account = match account {
        Some(LedgerAccount::LEDGER_ACCOUNT_COINS) => Account::Coins,
        Some(LedgerAccount::LEDGER_ACCOUNT_EARNINGS) => Account::Earnings,
        _ => bail!(invalid, "ACCOUNT_INVALID", "choose the balance to adjust: coins or earnings"),
    };
    ensure!(
        delta != 0 && (-MAX_AMOUNT..=MAX_AMOUNT).contains(&delta),
        invalid,
        "AMOUNT_INVALID",
        "an adjustment is a non-zero amount of at most 100,000,000 coins"
    );
    let reason = reason.trim();
    ensure!(!reason.is_empty(), invalid, "REASON_REQUIRED", "say why the balance is being adjusted");
    ensure!(reason.chars().count() <= MAX_REASON_CHARS, invalid, "REASON_TOO_LONG", "the reason is at most {MAX_REASON_CHARS} characters");
    ensure!(idempotency_key.len() <= 200, invalid, "IDEMPOTENCY_KEY_INVALID", "an idempotency key is at most 200 characters");
    Ok(AdjustmentInput { user_id, account, delta, reason: reason.to_owned(), idempotency_key: idempotency_key.to_owned() })
}

fn account_to_proto(account: &str) -> LedgerAccount {
    match account {
        "coins" => LedgerAccount::LEDGER_ACCOUNT_COINS,
        "earnings" => LedgerAccount::LEDGER_ACCOUNT_EARNINGS,
        _ => LedgerAccount::LEDGER_ACCOUNT_UNSPECIFIED,
    }
}

impl AdjustmentRow {
    /// Answers a request that reused this adjustment's idempotency key: the
    /// adjustment itself when the request describes the same change (a
    /// retry), `IDEMPOTENCY_KEY_REUSED` when it describes another one.
    fn answer_retry(self, input: &AdjustmentInput) -> AppResult<Self> {
        let same = self.user_id == Some(input.user_id) && self.account == input.account.name() && self.delta == input.delta;
        if same { Ok(self) } else { Err(key_reused()) }
    }
}

fn key_reused() -> AppError {
    AppError::already_exists("IDEMPOTENCY_KEY_REUSED", "this idempotency key was used for a different adjustment")
}

impl Wallet {
    /// Carries out an adjustment by `admin_id` and logs it.
    /// See `WalletAdminService.AdjustBalance`.
    pub(crate) async fn apply_adjustment(&self, admin_id: Uuid, input: &AdjustmentInput) -> AppResult<AdjustmentRow> {
        if let Some(existing) = self.adjustment_by_key(&self.core.db, admin_id, &input.idempotency_key).await? {
            return existing.answer_retry(input);
        }
        let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL)").bind(input.user_id).fetch_one(&self.core.db).await?;
        ensure!(exists, not_found, "USER_NOT_FOUND", "user not found");

        let adjustment_id = new_id();
        let mut tx = self.core.db.begin().await?;
        // Under the account lock a concurrent retry with the same key has
        // either committed (and is found here) or has not started posting.
        lock_account(&mut tx, input.user_id).await?;
        if let Some(existing) = self.adjustment_by_key(&mut *tx, admin_id, &input.idempotency_key).await? {
            return existing.answer_retry(input);
        }
        let memo = serde_json::json!({ "adjustment_id": adjustment_id });
        let posted = ledger_tx::adjust(&mut tx, input.user_id, input.account, input.delta, &format!("adjustment:{adjustment_id}"), &memo).await?;
        let inserted: Result<AdjustmentRow, sqlx::Error> = sqlx::query_as(concat!(
            "INSERT INTO wallet_adjustments (id, admin_id, user_id, account, delta, reason, idempotency_key, entry_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING ",
            adjustment_columns!()
        ))
        .bind(adjustment_id)
        .bind(admin_id)
        .bind(input.user_id)
        .bind(input.account.name())
        .bind(input.delta)
        .bind(&input.reason)
        .bind(&input.idempotency_key)
        .bind(posted.entry_id)
        .fetch_one(&mut *tx)
        .await;
        let adjustment = match inserted.map_err(AppError::from) {
            Ok(adjustment) => adjustment,
            // The same key raced in for another user's account (a different lock).
            Err(e) if e.is_already_exists() => return Err(key_reused()),
            Err(e) => return Err(e),
        };
        tx.commit().await?;
        tracing::info!(adjustment_id = %adjustment.id, %admin_id, user_id = %input.user_id, account = input.account.name(), delta = input.delta, "balance adjusted");
        Ok(adjustment)
    }

    async fn adjustment_by_key<'e>(&self, executor: impl sqlx::PgExecutor<'e>, admin_id: Uuid, idempotency_key: &str) -> AppResult<Option<AdjustmentRow>> {
        if idempotency_key.is_empty() {
            return Ok(None);
        }
        Ok(sqlx::query_as(concat!("SELECT ", adjustment_columns!(), " FROM wallet_adjustments WHERE admin_id = $1 AND idempotency_key = $2"))
            .bind(admin_id)
            .bind(idempotency_key)
            .fetch_optional(executor)
            .await?)
    }

    /// One page of the adjustment log, newest first; optionally one user's.
    pub(crate) async fn adjustments_page(&self, user_id: Option<Uuid>, page: &Page) -> AppResult<(Vec<AdjustmentRow>, PageInfo)> {
        let before = page.decode::<Uuid>()?.unwrap_or(NEWEST);
        // Two statements rather than one with an optional filter, so each
        // can walk its index (`(user_id, id)` or the primary key) in order.
        let query = match user_id {
            Some(user_id) => sqlx::query_as(concat!("SELECT ", adjustment_columns!(), " FROM wallet_adjustments WHERE user_id = $1 AND id < $2 ORDER BY id DESC LIMIT $3")).bind(user_id),
            None => sqlx::query_as(concat!("SELECT ", adjustment_columns!(), " FROM wallet_adjustments WHERE id < $1 ORDER BY id DESC LIMIT $2")),
        };
        let mut rows: Vec<AdjustmentRow> = query.bind(before).bind(page.fetch()).fetch_all(&self.core.db).await?;
        let info = page.finish(&mut rows, |row| row.id);
        Ok((rows, info))
    }

    /// Log rows as the API shows them, with both people resolved in one query.
    pub(crate) async fn adjustments_to_proto(&self, rows: &[AdjustmentRow]) -> AppResult<Vec<Adjustment>> {
        let people: Vec<Uuid> = rows.iter().flat_map(|row| [row.user_id, row.admin_id]).flatten().collect();
        let users = tokuly_identity::summaries(&self.core, &people).await?;
        Ok(rows
            .iter()
            .map(|row| Adjustment {
                id: row.id.to_string(),
                user: row.user_id.map(|id| tokuly_identity::users::pick(&users, id)).into(),
                admin: row.admin_id.map(|id| tokuly_identity::users::pick(&users, id)).into(),
                account: account_to_proto(&row.account).into(),
                delta: row.delta,
                reason: row.reason.clone(),
                entry_id: row.entry_id.to_string(),
                created_at: ts(row.created_at),
                ..Default::default()
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COINS: Option<LedgerAccount> = Some(LedgerAccount::LEDGER_ACCOUNT_COINS);

    #[test]
    fn a_valid_adjustment_is_normalized() {
        let user = new_id();
        let input = validate_adjustment(&user.to_string(), Some(LedgerAccount::LEDGER_ACCOUNT_EARNINGS), -250, "  二重付与の取り消し ", "k").unwrap();
        assert_eq!(input, AdjustmentInput { user_id: user, account: Account::Earnings, delta: -250, reason: "二重付与の取り消し".into(), idempotency_key: "k".into() });
    }

    #[test]
    fn adjustments_are_validated() {
        let user = new_id().to_string();
        let cases: Vec<(&str, AppResult<AdjustmentInput>)> = vec![
            ("INVALID_ID", validate_adjustment("nope", COINS, 5, "r", "")),
            ("ACCOUNT_INVALID", validate_adjustment(&user, None, 5, "r", "")),
            ("ACCOUNT_INVALID", validate_adjustment(&user, Some(LedgerAccount::LEDGER_ACCOUNT_UNSPECIFIED), 5, "r", "")),
            ("AMOUNT_INVALID", validate_adjustment(&user, COINS, 0, "r", "")),
            ("AMOUNT_INVALID", validate_adjustment(&user, COINS, MAX_AMOUNT + 1, "r", "")),
            ("AMOUNT_INVALID", validate_adjustment(&user, COINS, i64::MIN, "r", "")),
            ("REASON_REQUIRED", validate_adjustment(&user, COINS, 5, "   ", "")),
            ("REASON_TOO_LONG", validate_adjustment(&user, COINS, 5, &"x".repeat(MAX_REASON_CHARS + 1), "")),
            ("IDEMPOTENCY_KEY_INVALID", validate_adjustment(&user, COINS, 5, "r", &"k".repeat(201))),
        ];
        for (reason, result) in cases {
            assert_eq!(result.unwrap_err().reason, reason);
        }
    }
}
