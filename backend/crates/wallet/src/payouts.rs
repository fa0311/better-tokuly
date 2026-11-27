//! Payout requests: creators turning earned coins into money.
//!
//! ```text
//! request ──► pending ──► approved ──► paid
//!                │            │
//!                │            └──────► rejected   (coins return to earnings)
//!                ├───────────────────► rejected   (coins return to earnings)
//!                └───────────────────► cancelled  (by the requester; coins return)
//! ```
//!
//! The request row and the ledger always move together, in one transaction:
//! requesting takes a payout hold on the earnings, rejecting or cancelling
//! releases it, paying settles it (see [`crate::ledger_tx`]). The status
//! column is therefore always the truth about where the coins are.
use tokuly_core::{
    keys::Keys,
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::{
    common::PageInfo,
    wallet::{PayoutMethod, PayoutRequest, PayoutStatus, wallet_event::Event},
};

use crate::{
    Wallet,
    entries::NEWEST,
    ledger_tx,
    money::{format_coins, format_money, payout_minor},
    postings,
};

/// Label under which payout destinations are encrypted (`Keys::seal`).
const DESTINATION_SEAL: &str = "wallet/payout-destination";

pub(crate) const SEALED: tokuly_core::rekey::Sealed = tokuly_core::rekey::Sealed::bytes("wallet_payout_requests", "destination", DESTINATION_SEAL);
const MAX_TEXT_CHARS: usize = 500;
const MAX_REFERENCE_CHARS: usize = 200;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct PayoutRow {
    pub id: Uuid,
    /// `None` once the requester's account was deleted.
    pub user_id: Option<Uuid>,
    pub coins: i64,
    pub payout_minor: i64,
    pub currency: String,
    pub method: i32,
    /// Encrypted; `None` once erased.
    pub destination: Option<Vec<u8>>,
    pub note: String,
    pub status: String,
    pub hold_entry_id: Uuid,
    pub reviewed_by: Option<Uuid>,
    pub reviewed_at: Option<Time>,
    pub review_note: String,
    pub paid_at: Option<Time>,
    pub payment_reference: String,
    pub created_at: Time,
}

macro_rules! payout_columns {
    () => {
        "id, user_id, coins, payout_minor, currency, method, destination, note, status, hold_entry_id, \
         reviewed_by, reviewed_at, review_note, paid_at, payment_reference, created_at"
    };
}
pub(crate) use payout_columns;

fn status_to_proto(status: &str) -> PayoutStatus {
    match status {
        "pending" => PayoutStatus::PAYOUT_STATUS_PENDING,
        "approved" => PayoutStatus::PAYOUT_STATUS_APPROVED,
        "rejected" => PayoutStatus::PAYOUT_STATUS_REJECTED,
        "paid" => PayoutStatus::PAYOUT_STATUS_PAID,
        "cancelled" => PayoutStatus::PAYOUT_STATUS_CANCELLED,
        _ => PayoutStatus::PAYOUT_STATUS_UNSPECIFIED,
    }
}

/// The stored status a queue filter asks for; unspecified means the review backlog.
pub(crate) fn status_filter(status: Option<PayoutStatus>) -> &'static str {
    match status {
        Some(PayoutStatus::PAYOUT_STATUS_APPROVED) => "approved",
        Some(PayoutStatus::PAYOUT_STATUS_REJECTED) => "rejected",
        Some(PayoutStatus::PAYOUT_STATUS_PAID) => "paid",
        Some(PayoutStatus::PAYOUT_STATUS_CANCELLED) => "cancelled",
        _ => "pending",
    }
}

impl PayoutRow {
    /// The request as its owner and reviewers see it (`user` and `reviewer`
    /// are filled in separately for staff views).
    pub fn to_proto(&self, core: &Core) -> PayoutRequest {
        PayoutRequest {
            id: self.id.to_string(),
            status: status_to_proto(&self.status).into(),
            coins: self.coins,
            payout_minor: self.payout_minor,
            currency: self.currency.clone(),
            method: self.method.into(),
            destination: self.destination(&core.keys),
            note: self.note.clone(),
            review_note: self.review_note.clone(),
            payment_reference: self.payment_reference.clone(),
            created_at: ts(self.created_at),
            reviewed_at: opt_ts(self.reviewed_at),
            paid_at: opt_ts(self.paid_at),
            ..Default::default()
        }
    }

    /// The decrypted destination; empty once it was erased.
    pub fn destination(&self, keys: &Keys) -> String {
        let opened = self.destination.as_deref().and_then(|sealed| keys.open(DESTINATION_SEAL, sealed));
        opened.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()).unwrap_or_default()
    }

    fn is_open(&self) -> bool {
        matches!(self.status.as_str(), "pending" | "approved")
    }
}

/// A payout request as submitted, checked and normalized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PayoutInput {
    pub coins: i64,
    pub method: PayoutMethod,
    pub destination: String,
    pub note: String,
    pub idempotency_key: String,
}

/// Checks a submitted payout request against the server's terms.
pub(crate) fn validate_request(coins: i64, method: Option<PayoutMethod>, destination: &str, note: &str, idempotency_key: &str, min_coins: i64) -> AppResult<PayoutInput> {
    ensure!((1..=postings::MAX_AMOUNT).contains(&coins), invalid, "AMOUNT_INVALID", "coins must be between 1 and 100,000,000");
    ensure!(coins >= min_coins, invalid, "PAYOUT_BELOW_MINIMUM", "a payout needs at least {min_coins} earned coins");
    let method = method.filter(|m| *m != PayoutMethod::PAYOUT_METHOD_UNSPECIFIED).ok_or_else(|| AppError::invalid("PAYOUT_METHOD_INVALID", "choose how you want to be paid"))?;
    let destination = destination.trim();
    ensure!(
        !destination.is_empty() && destination.chars().count() <= MAX_TEXT_CHARS,
        invalid,
        "PAYOUT_DESTINATION_INVALID",
        "say where to send the money, in at most {MAX_TEXT_CHARS} characters"
    );
    let note = note.trim();
    ensure!(note.chars().count() <= MAX_TEXT_CHARS, invalid, "PAYOUT_NOTE_TOO_LONG", "the note is at most {MAX_TEXT_CHARS} characters");
    ensure!(idempotency_key.len() <= 200, invalid, "IDEMPOTENCY_KEY_INVALID", "an idempotency key is at most 200 characters");
    Ok(PayoutInput { coins, method, destination: destination.to_owned(), note: note.to_owned(), idempotency_key: idempotency_key.to_owned() })
}

/// What staff decided about a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decision {
    Approve,
    Reject,
}

fn review_note(note: &str, required: bool) -> AppResult<String> {
    let note = note.trim();
    ensure!(!required || !note.is_empty(), invalid, "REVIEW_NOTE_REQUIRED", "tell the requester why the payout was rejected");
    ensure!(note.chars().count() <= MAX_TEXT_CHARS, invalid, "REVIEW_NOTE_TOO_LONG", "the note is at most {MAX_TEXT_CHARS} characters");
    Ok(note.to_owned())
}

impl Wallet {
    /// Opens a payout request for `user_id` and locks the coins.
    /// See `EarningsService.RequestPayout`.
    pub(crate) async fn open_payout(&self, user_id: Uuid, input: &PayoutInput) -> AppResult<PayoutRow> {
        if let Some(existing) = self.payout_by_key(&self.core.db, user_id, &input.idempotency_key).await? {
            return self.retry_of(existing, input);
        }
        self.core.cache.rate_limit(&format!("wallet:payout-request:{user_id}"), 10, std::time::Duration::from_secs(24 * 3600)).await?;

        let payout_id = new_id();
        let mut tx = self.core.db.begin().await?;
        // The account lock serializes this user's requests, so the two checks
        // below cannot be raced by a second request.
        postings::lock_account(&mut tx, user_id).await?;
        if let Some(existing) = self.payout_by_key(&mut *tx, user_id, &input.idempotency_key).await? {
            return self.retry_of(existing, input);
        }
        let has_open: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM wallet_payout_requests WHERE user_id = $1 AND status IN ('pending', 'approved'))")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(!has_open, precondition, "PAYOUT_ALREADY_OPEN", "you already have a payout request in progress");

        let memo = serde_json::json!({ "payout_id": payout_id });
        let hold = ledger_tx::hold_payout(&mut tx, user_id, input.coins, &format!("payout:{payout_id}"), &memo).await?;
        let payout: PayoutRow = sqlx::query_as(concat!(
            "INSERT INTO wallet_payout_requests (id, user_id, coins, payout_minor, currency, method, destination, note, idempotency_key, hold_entry_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING ",
            payout_columns!()
        ))
        .bind(payout_id)
        .bind(user_id)
        .bind(input.coins)
        .bind(payout_minor(input.coins, self.cfg.payout_minor_per_100_coins))
        .bind(&self.cfg.payout_currency)
        .bind(input.method as i32)
        .bind(self.core.keys.seal(DESTINATION_SEAL, input.destination.as_bytes()))
        .bind(&input.note)
        .bind(&input.idempotency_key)
        .bind(hold.entry_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        tracing::info!(payout_id = %payout.id, %user_id, coins = payout.coins, "payout requested");
        Ok(payout)
    }

    /// Answers a request that reused an earlier request's idempotency key:
    /// that request when this is a retry of it, `IDEMPOTENCY_KEY_REUSED` when
    /// it asks for something else (another amount, method or destination).
    fn retry_of(&self, existing: PayoutRow, input: &PayoutInput) -> AppResult<PayoutRow> {
        // A destination erased after the request closed cannot be compared; the amount and method still are.
        let destination = existing.destination(&self.core.keys);
        let same = existing.coins == input.coins && existing.method == input.method as i32 && (destination.is_empty() || destination == input.destination);
        ensure!(same, already_exists, "IDEMPOTENCY_KEY_REUSED", "this idempotency key was used for a different payout request");
        Ok(existing)
    }

    /// The requester withdraws a request that nobody reviewed yet.
    /// See `EarningsService.CancelPayoutRequest`.
    pub(crate) async fn cancel_payout(&self, user_id: Uuid, payout_id: Uuid) -> AppResult<PayoutRow> {
        let mut tx = self.core.db.begin().await?;
        let payout = self.lock_payout(&mut tx, payout_id).await?.filter(|p| p.user_id == Some(user_id)).or_not_found("PAYOUT_NOT_FOUND", "payout request not found")?;
        ensure!(payout.status == "pending", precondition, "PAYOUT_NOT_PENDING", "only a request that is still waiting for review can be cancelled");
        ledger_tx::release_payout(&mut tx, payout.hold_entry_id, "payout.cancelled").await?;
        let payout: PayoutRow = sqlx::query_as(concat!("UPDATE wallet_payout_requests SET status = 'cancelled' WHERE id = $1 RETURNING ", payout_columns!()))
            .bind(payout.id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        self.push(user_id, Event::PayoutUpdated(Box::new(payout.to_proto(&self.core)))).await;
        Ok(payout)
    }

    /// Staff approves or rejects a request. Rejecting returns the coins.
    pub(crate) async fn review_payout(&self, reviewer_id: Uuid, payout_id: Uuid, decision: Decision, note: &str) -> AppResult<PayoutRow> {
        let note = review_note(note, decision == Decision::Reject)?;
        let mut tx = self.core.db.begin().await?;
        let payout = self.lock_payout(&mut tx, payout_id).await?.or_not_found("PAYOUT_NOT_FOUND", "payout request not found")?;
        ensure!(payout.user_id != Some(reviewer_id), denied, "SELF_REVIEW", "you cannot review your own payout request");
        let new_status = match decision {
            Decision::Approve => {
                ensure!(payout.status == "pending", precondition, "PAYOUT_NOT_PENDING", "only a pending request can be approved");
                self.ensure_not_suspended(&mut tx, &payout).await?;
                "approved"
            }
            Decision::Reject => {
                ensure!(payout.is_open(), precondition, "PAYOUT_NOT_OPEN", "this request is already closed");
                ledger_tx::release_payout(&mut tx, payout.hold_entry_id, "payout.rejected").await?;
                "rejected"
            }
        };
        let payout: PayoutRow = sqlx::query_as(concat!(
            "UPDATE wallet_payout_requests SET status = $2, reviewed_by = $3, reviewed_at = now(), review_note = $4 WHERE id = $1 RETURNING ",
            payout_columns!()
        ))
        .bind(payout.id)
        .bind(new_status)
        .bind(reviewer_id)
        .bind(&note)
        .fetch_one(&mut *tx)
        .await?;
        self.notify_payout(&mut tx, &payout).await?;
        tx.commit().await?;
        tracing::info!(payout_id = %payout.id, %reviewer_id, status = new_status, "payout reviewed");
        self.push_payout(&payout).await;
        Ok(payout)
    }

    /// Staff records that the money of an approved request was sent.
    pub(crate) async fn record_payout_paid(&self, staff_id: Uuid, payout_id: Uuid, payment_reference: &str) -> AppResult<PayoutRow> {
        let payment_reference = payment_reference.trim();
        ensure!(payment_reference.chars().count() <= MAX_REFERENCE_CHARS, invalid, "PAYMENT_REFERENCE_TOO_LONG", "the payment reference is at most {MAX_REFERENCE_CHARS} characters");
        let mut tx = self.core.db.begin().await?;
        let payout = self.lock_payout(&mut tx, payout_id).await?.or_not_found("PAYOUT_NOT_FOUND", "payout request not found")?;
        ensure!(payout.user_id != Some(staff_id), denied, "SELF_REVIEW", "you cannot pay out your own request");
        ensure!(payout.status == "approved", precondition, "PAYOUT_NOT_APPROVED", "only an approved request can be marked as paid");
        self.ensure_not_suspended(&mut tx, &payout).await?;
        let memo = serde_json::json!({ "payout_id": payout.id, "paid_by": staff_id });
        ledger_tx::settle_payout(&mut tx, payout.hold_entry_id, &memo).await?;
        let payout: PayoutRow = sqlx::query_as(concat!(
            "UPDATE wallet_payout_requests SET status = 'paid', paid_by = $2, paid_at = now(), payment_reference = $3 WHERE id = $1 RETURNING ",
            payout_columns!()
        ))
        .bind(payout.id)
        .bind(staff_id)
        .bind(payment_reference)
        .fetch_one(&mut *tx)
        .await?;
        self.notify_payout(&mut tx, &payout).await?;
        tx.commit().await?;
        tracing::info!(payout_id = %payout.id, %staff_id, "payout paid");
        self.push_payout(&payout).await;
        Ok(payout)
    }

    /// No money goes to a suspended account: `ACCOUNT_SUSPENDED` while the
    /// requester is suspended (`users.banned_at`). Rejecting stays possible.
    async fn ensure_not_suspended(&self, tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, payout: &PayoutRow) -> AppResult<()> {
        let Some(user_id) = payout.user_id else { return Ok(()) };
        let suspended: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND banned_at IS NOT NULL)").bind(user_id).fetch_one(&mut **tx).await?;
        ensure!(!suspended, precondition, "ACCOUNT_SUSPENDED", "the requester's account is suspended; reject the request or wait until the suspension is lifted");
        Ok(())
    }

    /// Reads a request and locks its row until the transaction ends.
    async fn lock_payout(&self, tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, payout_id: Uuid) -> AppResult<Option<PayoutRow>> {
        Ok(sqlx::query_as(concat!("SELECT ", payout_columns!(), " FROM wallet_payout_requests WHERE id = $1 FOR UPDATE"))
            .bind(payout_id)
            .fetch_optional(&mut **tx)
            .await?)
    }

    async fn payout_by_key<'e>(&self, executor: impl sqlx::PgExecutor<'e>, user_id: Uuid, idempotency_key: &str) -> AppResult<Option<PayoutRow>> {
        if idempotency_key.is_empty() {
            return Ok(None);
        }
        Ok(sqlx::query_as(concat!("SELECT ", payout_columns!(), " FROM wallet_payout_requests WHERE user_id = $1 AND idempotency_key = $2"))
            .bind(user_id)
            .bind(idempotency_key)
            .fetch_optional(executor)
            .await?)
    }

    /// Any request by ID (staff view), or `PAYOUT_NOT_FOUND`.
    pub(crate) async fn find_payout(&self, payout_id: Uuid) -> AppResult<PayoutRow> {
        let payout: Option<PayoutRow> = sqlx::query_as(concat!("SELECT ", payout_columns!(), " FROM wallet_payout_requests WHERE id = $1"))
            .bind(payout_id)
            .fetch_optional(&self.core.db)
            .await?;
        payout.or_not_found("PAYOUT_NOT_FOUND", "payout request not found")
    }

    /// One page of `user_id`'s own requests, newest first.
    pub(crate) async fn list_payouts(&self, user_id: Uuid, page: &Page) -> AppResult<(Vec<PayoutRow>, PageInfo)> {
        let before: Option<Uuid> = page.decode()?;
        let mut rows: Vec<PayoutRow> = sqlx::query_as(concat!(
            "SELECT ",
            payout_columns!(),
            " FROM wallet_payout_requests WHERE user_id = $1 AND id < $2 ORDER BY id DESC LIMIT $3"
        ))
        .bind(user_id)
        .bind(before.unwrap_or(NEWEST))
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| row.id);
        Ok((rows, info))
    }

    /// One page of every user's requests in `status`, oldest first: the
    /// review queue is worked from the front.
    pub(crate) async fn payout_queue_page(&self, status: &str, page: &Page) -> AppResult<(Vec<PayoutRow>, PageInfo)> {
        let after: Option<Uuid> = page.decode()?;
        let mut rows: Vec<PayoutRow> = sqlx::query_as(concat!(
            "SELECT ",
            payout_columns!(),
            " FROM wallet_payout_requests WHERE status = $1 AND id > $2 ORDER BY id LIMIT $3"
        ))
        .bind(status)
        // Without a cursor, start before every real ID.
        .bind(after.unwrap_or(Uuid::nil()))
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| row.id);
        Ok((rows, info))
    }

    /// Requests as staff sees them: with the requester (and whether they are
    /// suspended) and the reviewer.
    pub(crate) async fn payouts_for_staff(&self, rows: &[PayoutRow]) -> AppResult<Vec<PayoutRequest>> {
        let people: Vec<Uuid> = rows.iter().flat_map(|row| [row.user_id, row.reviewed_by]).flatten().collect();
        let users = tokuly_identity::summaries(&self.core, &people).await?;
        let requesters: Vec<Uuid> = rows.iter().filter_map(|row| row.user_id).collect();
        let suspended: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE id = ANY($1) AND banned_at IS NOT NULL").bind(&requesters).fetch_all(&self.core.db).await?;
        Ok(rows
            .iter()
            .map(|row| PayoutRequest {
                user: row.user_id.map(|id| tokuly_identity::users::pick(&users, id)).into(),
                reviewer: row.reviewed_by.map(|id| tokuly_identity::users::pick(&users, id)).into(),
                user_suspended: row.user_id.is_some_and(|id| suspended.contains(&id)),
                ..row.to_proto(&self.core)
            })
            .collect())
    }

    /// Earned coins of `user_id` locked in an open request, and paid out so far.
    pub(crate) async fn payout_totals(&self, user_id: Uuid) -> AppResult<(i64, i64)> {
        Ok(sqlx::query_as(
            "SELECT COALESCE(sum(coins) FILTER (WHERE status IN ('pending', 'approved')), 0)::bigint,
                    COALESCE(sum(coins) FILTER (WHERE status = 'paid'), 0)::bigint
             FROM wallet_payout_requests WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_one(&self.core.db)
        .await?)
    }

    /// Tells the requester their request was approved, rejected or paid.
    /// Part of the transaction that changes the status.
    async fn notify_payout(&self, tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, payout: &PayoutRow) -> AppResult<()> {
        let Some(user_id) = payout.user_id else { return Ok(()) };
        let coins = format_coins(payout.coins);
        let money = format_money(payout.payout_minor, &payout.currency);
        let (title, body) = match payout.status.as_str() {
            "approved" => (
                Localized::new("出金申請が承認されました!", "Payout request approved!"),
                Localized::new(format!("{money}（{coins}コイン）の送金を準備しているよ。"), format!("We are preparing your payout of {money} ({coins} coins).")),
            ),
            "rejected" => (
                Localized::new("出金申請が却下されました", "Payout request rejected"),
                Localized::new(
                    format!("{coins}コインは売上に戻りました。理由: {}", payout.review_note),
                    format!("{coins} coins are back in your earnings. Reason: {}", payout.review_note),
                ),
            ),
            "paid" => (
                Localized::new("出金が完了しました!", "Payout sent!"),
                Localized::new(format!("{money}を送金したよ!"), format!("{money} is on its way to you.")),
            ),
            _ => return Ok(()),
        };
        let notice = NotificationRequest::new(user_id, "wallet.payout", title, body, format!("/wallet/payouts/{}", payout.id))
            // One notification per request: a later status replaces the earlier one.
            .collapse(format!("wallet.payout:{}", payout.id))
            .data(serde_json::json!({ "payout_id": payout.id, "status": payout.status }));
        outbound::notify(&mut **tx, &notice).await
    }

    async fn push_payout(&self, payout: &PayoutRow) {
        if let Some(user_id) = payout.user_id {
            self.push(user_id, Event::PayoutUpdated(Box::new(payout.to_proto(&self.core)))).await;
        }
    }

    /// Erases the destination of requests closed more than `days` ago: bank
    /// details are kept only as long as someone may still need to look at them.
    pub(crate) async fn erase_old_destinations(&self, days: i32) -> AppResult<u64> {
        let erased = sqlx::query(
            "UPDATE wallet_payout_requests SET destination = NULL
             WHERE destination IS NOT NULL AND status IN ('rejected', 'paid', 'cancelled') AND updated_at < now() - make_interval(days => $1)",
        )
        .bind(days)
        .execute(&self.core.db)
        .await?;
        Ok(erased.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BANK: Option<PayoutMethod> = Some(PayoutMethod::PAYOUT_METHOD_BANK_TRANSFER);

    #[test]
    fn a_valid_request_is_normalized() {
        let input = validate_request(1500, BANK, "  みずほ銀行 渋谷支店 普通 1234567  ", " よろしく! ", "k1", 1000).unwrap();
        assert_eq!(input.destination, "みずほ銀行 渋谷支店 普通 1234567");
        assert_eq!(input.note, "よろしく!");
        assert_eq!((input.coins, input.method), (1500, PayoutMethod::PAYOUT_METHOD_BANK_TRANSFER));
    }

    #[test]
    fn requests_are_checked_against_the_terms() {
        let long = "あ".repeat(MAX_TEXT_CHARS + 1);
        let cases: Vec<(&str, AppResult<PayoutInput>)> = vec![
            ("AMOUNT_INVALID", validate_request(0, BANK, "x", "", "", 1000)),
            ("AMOUNT_INVALID", validate_request(postings::MAX_AMOUNT + 1, BANK, "x", "", "", 1000)),
            ("PAYOUT_BELOW_MINIMUM", validate_request(999, BANK, "x", "", "", 1000)),
            ("PAYOUT_METHOD_INVALID", validate_request(1000, None, "x", "", "", 1000)),
            ("PAYOUT_METHOD_INVALID", validate_request(1000, Some(PayoutMethod::PAYOUT_METHOD_UNSPECIFIED), "x", "", "", 1000)),
            ("PAYOUT_DESTINATION_INVALID", validate_request(1000, BANK, "   ", "", "", 1000)),
            ("PAYOUT_DESTINATION_INVALID", validate_request(1000, BANK, &long, "", "", 1000)),
            ("PAYOUT_NOTE_TOO_LONG", validate_request(1000, BANK, "x", &long, "", 1000)),
            ("IDEMPOTENCY_KEY_INVALID", validate_request(1000, BANK, "x", "", &"k".repeat(201), 1000)),
        ];
        for (reason, result) in cases {
            assert_eq!(result.unwrap_err().reason, reason);
        }
        assert!(validate_request(1000, BANK, &"あ".repeat(MAX_TEXT_CHARS), "", "", 1000).is_ok(), "limits count characters, not bytes");
    }

    #[test]
    fn rejections_need_a_reason() {
        assert_eq!(review_note("  ", true).unwrap_err().reason, "REVIEW_NOTE_REQUIRED");
        assert_eq!(review_note("", false).unwrap(), "");
        assert_eq!(review_note(" 口座名義が違います ", true).unwrap(), "口座名義が違います");
        assert_eq!(review_note(&"x".repeat(MAX_TEXT_CHARS + 1), false).unwrap_err().reason, "REVIEW_NOTE_TOO_LONG");
    }

    #[test]
    fn statuses_round_trip_between_storage_and_api() {
        for stored in ["pending", "approved", "rejected", "paid", "cancelled"] {
            assert_eq!(status_filter(Some(status_to_proto(stored))), stored);
        }
        assert_eq!(status_filter(None), "pending");
        assert_eq!(status_filter(Some(PayoutStatus::PAYOUT_STATUS_UNSPECIFIED)), "pending");
    }
}
