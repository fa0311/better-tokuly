//! Buying coins: purchases and their life cycle.
//!
//! ```text
//! start ──► pending ──► completed        (provider says "paid": coins granted)
//!              ├──────► failed           (provider says it will never be paid)
//!              ├──────► cancelled        (buyer gave up)
//!              └──────► expired          (still unpaid at `expires_at`)
//! ```
//!
//! "Paid" wins over everything except `completed` itself: if a provider
//! reports a payment for a purchase we already failed, cancelled or expired,
//! the buyer still gets the coins, because the money was taken. The grant uses
//! the idempotency key `purchase:<id>` and happens in the transaction that
//! marks the purchase completed, so no path can grant twice or complete
//! without granting.
//!
//! The monthly spending limit (`spending.rs`) is checked when a purchase
//! starts, never when it completes: by then the money was taken.
use std::sync::Arc;

use sqlx::{PgExecutor, Postgres, Transaction};
use tokuly_core::{
    analytics::AnalyticsEvent,
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::{
    common::PageInfo,
    wallet::{Purchase, PurchaseStatus, wallet_event::Event},
};

use crate::{
    Wallet,
    entries::NEWEST,
    ledger_tx,
    money::format_coins,
    packages::PackageRow,
    postings::lock_account,
    providers::{PaymentProvider, PaymentState, PurchaseOrder},
    spending,
};

/// How long after `expires_at` an unpaid purchase is expired even though its
/// provider could not be asked one last time (it has been down for a day).
const UNREACHABLE_PROVIDER_GRACE: chrono::Duration = chrono::Duration::hours(24);

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct PurchaseRow {
    pub id: Uuid,
    /// `None` once the buyer's account was deleted.
    pub user_id: Option<Uuid>,
    pub package_id: Uuid,
    pub sku: String,
    pub coins: i64,
    pub bonus_coins: i64,
    pub price_minor: i64,
    pub currency: String,
    pub provider: String,
    pub provider_ref: String,
    pub checkout_url: String,
    pub status: String,
    pub failure_reason: String,
    pub entry_id: Option<Uuid>,
    pub created_at: Time,
    pub completed_at: Option<Time>,
    pub expires_at: Time,
}

macro_rules! purchase_columns {
    () => {
        "id, user_id, package_id, sku, coins, bonus_coins, price_minor, currency, provider, provider_ref, checkout_url, \
         status, failure_reason, entry_id, created_at, completed_at, expires_at"
    };
}
pub(crate) use purchase_columns;

fn status_to_proto(status: &str) -> PurchaseStatus {
    match status {
        "pending" => PurchaseStatus::PURCHASE_STATUS_PENDING,
        "completed" => PurchaseStatus::PURCHASE_STATUS_COMPLETED,
        "failed" => PurchaseStatus::PURCHASE_STATUS_FAILED,
        "cancelled" => PurchaseStatus::PURCHASE_STATUS_CANCELLED,
        "expired" => PurchaseStatus::PURCHASE_STATUS_EXPIRED,
        _ => PurchaseStatus::PURCHASE_STATUS_UNSPECIFIED,
    }
}

impl PurchaseRow {
    pub fn to_proto(&self) -> Purchase {
        Purchase {
            id: self.id.to_string(),
            status: status_to_proto(&self.status).into(),
            package_id: self.package_id.to_string(),
            sku: self.sku.clone(),
            coins: self.coins,
            bonus_coins: self.bonus_coins,
            price_minor: self.price_minor,
            currency: self.currency.clone(),
            provider: self.provider.clone(),
            checkout_url: self.checkout_url.clone(),
            failure_reason: self.failure_reason.clone(),
            entry_id: self.entry_id.map(|id| id.to_string()).unwrap_or_default(),
            created_at: ts(self.created_at),
            completed_at: opt_ts(self.completed_at),
            expires_at: ts(self.expires_at),
            ..Default::default()
        }
    }

    /// Coins the buyer receives, bonus included.
    fn total_coins(&self) -> i64 {
        self.coins + self.bonus_coins
    }

    fn is_pending(&self) -> bool {
        self.status == "pending"
    }

    /// The purchase as a provider needs to see it; `None` when the buyer's
    /// account is gone.
    fn order(&self, core: &Core) -> Option<PurchaseOrder> {
        Some(PurchaseOrder {
            purchase_id: self.id,
            user_id: self.user_id?,
            sku: self.sku.clone(),
            coins: self.total_coins(),
            price_minor: self.price_minor,
            currency: self.currency.clone(),
            return_url: core.web_url(&format!("/wallet/purchases/{}", self.id)),
        })
    }
}

/// Who learned that a purchase was paid. Decides how loudly the buyer is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trigger {
    /// The buyer's own request: the result is on their screen already.
    Buyer,
    /// A provider webhook or the expiry check: the buyer may be elsewhere.
    Background,
}

fn provider_error(context: &'static str, error: AppError) -> AppError {
    tracing::error!(error = %error, "payment provider failed to {context}");
    AppError::unavailable("PAYMENT_PROVIDER_ERROR", "the payment provider is not responding, try again shortly")
}

impl Wallet {
    /// Starts buying `package_id` through the named provider (empty = the
    /// default one). See `WalletService.StartPurchase`.
    pub(crate) async fn open_purchase(&self, user_id: Uuid, package_id: Uuid, provider_name: &str, idempotency_key: &str) -> AppResult<PurchaseRow> {
        ensure!(idempotency_key.len() <= 200, invalid, "IDEMPOTENCY_KEY_INVALID", "an idempotency key is at most 200 characters");
        let provider = self.resolve_provider(provider_name)?;
        if let Some(existing) = purchase_by_key(&self.core.db, user_id, idempotency_key).await? {
            return self.resume_purchase(existing, package_id, provider).await;
        }
        self.core.cache.rate_limit(&format!("wallet:purchase:{user_id}"), 30, std::time::Duration::from_secs(3600)).await?;
        let package = self.package_on_sale(package_id).await?;
        let ceiling = spending::ceiling_for(&self.core, user_id).await?;
        // Under the account lock every charge of the user takes, so that two
        // starts at once are counted against the spending limit one after
        // the other, and an identical start that won the race is found.
        let mut tx = self.core.db.begin().await?;
        lock_account(&mut tx, user_id).await?;
        if let Some(existing) = purchase_by_key(&mut *tx, user_id, idempotency_key).await? {
            drop(tx);
            return self.resume_purchase(existing, package_id, provider).await;
        }
        spending::check(&mut tx, user_id, ceiling, spending::purchase_yen(&package.currency, package.price_minor, package.coins)).await?;
        let purchase = self.insert_purchase(&mut tx, user_id, &package, provider.name(), idempotency_key).await?;
        tx.commit().await?;
        self.open_checkout(purchase, provider).await
    }

    /// A start repeated with the key of an earlier one. Normally a retry has
    /// nothing left to do; only a start that died before its checkout
    /// existed is picked up where it stopped.
    async fn resume_purchase(&self, existing: PurchaseRow, package_id: Uuid, provider: Arc<dyn PaymentProvider>) -> AppResult<PurchaseRow> {
        ensure!(
            existing.package_id == package_id && existing.provider == provider.name(),
            already_exists,
            "IDEMPOTENCY_KEY_REUSED",
            "this idempotency key was used for a different purchase"
        );
        if existing.is_pending() && existing.provider_ref.is_empty() {
            return self.open_checkout(existing, provider).await;
        }
        Ok(existing)
    }

    async fn insert_purchase(&self, tx: &mut Transaction<'_, Postgres>, user_id: Uuid, package: &PackageRow, provider: &str, idempotency_key: &str) -> AppResult<PurchaseRow> {
        Ok(sqlx::query_as(concat!(
            "INSERT INTO wallet_purchases (user_id, package_id, sku, coins, bonus_coins, price_minor, currency, provider, idempotency_key, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now() + make_interval(mins => $10))
             RETURNING ",
            purchase_columns!()
        ))
        .bind(user_id)
        .bind(package.id)
        .bind(&package.sku)
        .bind(package.coins)
        .bind(package.bonus_coins)
        .bind(package.price_minor)
        .bind(&package.currency)
        .bind(provider)
        .bind(idempotency_key)
        .bind(self.cfg.purchase_ttl_minutes.clamp(1, i64::from(i32::MAX)) as i32)
        .fetch_one(&mut **tx)
        .await?)
    }

    /// Creates the provider's checkout for a fresh purchase and acts on the
    /// state the provider reports for it (instant providers report "paid").
    async fn open_checkout(&self, purchase: PurchaseRow, provider: Arc<dyn PaymentProvider>) -> AppResult<PurchaseRow> {
        let order = purchase.order(&self.core).or_not_found("PURCHASE_NOT_FOUND", "purchase not found")?;
        let checkout = match provider.create_checkout(&order).await {
            Ok(checkout) if !checkout.reference.is_empty() => checkout,
            failure => {
                self.close_unpaid(purchase.id, "failed", "checkout_error").await?;
                let error = failure.err().unwrap_or_else(|| AppError::internal(anyhow::anyhow!("provider returned an empty checkout reference")));
                return Err(provider_error("create a checkout", error));
            }
        };
        // Only the first checkout is kept: a retry that ran while another
        // call was still opening one must not replace it, or a payment into
        // the replaced checkout would never find its purchase.
        let saved: Result<Option<PurchaseRow>, sqlx::Error> = sqlx::query_as(concat!(
            "UPDATE wallet_purchases SET provider_ref = $2, checkout_url = $3 WHERE id = $1 AND provider_ref = '' RETURNING ",
            purchase_columns!()
        ))
        .bind(purchase.id)
        .bind(&checkout.reference)
        .bind(checkout.checkout_url.as_deref().unwrap_or_default())
        .fetch_optional(&self.core.db)
        .await;
        let purchase = match saved.map_err(AppError::from) {
            Ok(Some(purchase)) => purchase,
            Ok(None) => {
                // Another call saved its checkout first: void ours and answer with that one.
                if let Err(e) = provider.cancel_checkout(&order, &checkout.reference).await {
                    tracing::warn!(purchase_id = %purchase.id, error = %e, "provider could not void a duplicate checkout");
                }
                let current: PurchaseRow = sqlx::query_as(concat!("SELECT ", purchase_columns!(), " FROM wallet_purchases WHERE id = $1"))
                    .bind(purchase.id)
                    .fetch_one(&self.core.db)
                    .await?;
                return Ok(current);
            }
            // The unique index on (provider, provider_ref): the provider reused a reference.
            Err(e) if e.is_already_exists() => return Err(provider_error("return a unique checkout reference", e)),
            Err(e) => return Err(e),
        };
        self.apply_payment_state(purchase, checkout.state, Trigger::Buyer).await
    }

    /// Asks the provider about a purchase of `user_id` and acts on the answer.
    /// See `WalletService.ConfirmPurchase`.
    pub(crate) async fn confirm_payment(&self, user_id: Uuid, purchase_id: Uuid, client_proof: &str) -> AppResult<PurchaseRow> {
        let purchase = self.find_purchase(user_id, purchase_id).await?;
        // Only completed is final. A failed, cancelled or expired purchase is
        // asked about again: the buyer may have retried the payment or paid at
        // the last moment, and money that was taken must become coins.
        if purchase.status == "completed" || purchase.provider_ref.is_empty() {
            return Ok(purchase);
        }
        let (Some(provider), Some(order)) = (self.provider(&purchase.provider), purchase.order(&self.core)) else {
            return Ok(purchase);
        };
        let state = provider.confirm(&order, &purchase.provider_ref, client_proof).await.map_err(|e| provider_error("confirm a payment", e))?;
        self.apply_payment_state(purchase, state, Trigger::Buyer).await
    }

    /// Records what a provider learned about a checkout on its own (a
    /// webhook). Safe to call any number of times for the same payment.
    /// Fails with `PURCHASE_NOT_FOUND` for a reference this wallet never issued.
    pub async fn settle_purchase(&self, provider: &str, reference: &str, state: PaymentState) -> AppResult<()> {
        let purchase: Option<PurchaseRow> = sqlx::query_as(concat!("SELECT ", purchase_columns!(), " FROM wallet_purchases WHERE provider = $1 AND provider_ref = $2 AND provider_ref <> ''"))
            .bind(provider)
            .bind(reference)
            .fetch_optional(&self.core.db)
            .await?;
        let purchase = purchase.or_not_found("PURCHASE_NOT_FOUND", "no purchase has this checkout reference")?;
        self.apply_payment_state(purchase, state, Trigger::Background).await?;
        Ok(())
    }

    async fn apply_payment_state(&self, purchase: PurchaseRow, state: PaymentState, trigger: Trigger) -> AppResult<PurchaseRow> {
        match state {
            PaymentState::Paid => self.complete_purchase(purchase.id, trigger).await,
            // Only a purchase still waiting can fail; closed ones stay as they are.
            PaymentState::Failed { reason } => Ok(self.close_unpaid(purchase.id, "failed", &reason).await?.unwrap_or(purchase)),
            PaymentState::Pending => Ok(purchase),
        }
    }

    /// Grants the coins of a paid purchase and marks it completed, in one
    /// transaction. Repeating it changes nothing.
    async fn complete_purchase(&self, purchase_id: Uuid, trigger: Trigger) -> AppResult<PurchaseRow> {
        let mut tx = self.core.db.begin().await?;
        let purchase: PurchaseRow = sqlx::query_as(concat!("SELECT ", purchase_columns!(), " FROM wallet_purchases WHERE id = $1 FOR UPDATE"))
            .bind(purchase_id)
            .fetch_optional(&mut *tx)
            .await?
            .or_not_found("PURCHASE_NOT_FOUND", "purchase not found")?;
        if purchase.status == "completed" {
            return Ok(purchase);
        }
        let user_id = purchase.user_id.ok_or_else(|| AppError::precondition("PURCHASE_ORPHANED", "the buyer's account no longer exists"))?;
        let total = purchase.total_coins();
        let memo = serde_json::json!({ "purchase_id": purchase.id, "sku": purchase.sku, "provider": purchase.provider, "bonus_coins": purchase.bonus_coins });
        let granted = ledger_tx::grant(&mut tx, user_id, total, "purchase", &format!("purchase:{}", purchase.id), &memo).await?;
        let purchase: PurchaseRow = sqlx::query_as(concat!(
            "UPDATE wallet_purchases SET status = 'completed', completed_at = now(), entry_id = $2, failure_reason = '' WHERE id = $1 RETURNING ",
            purchase_columns!()
        ))
        .bind(purchase.id)
        .bind(granted.entry_id)
        .fetch_one(&mut *tx)
        .await?;
        let mut notice = NotificationRequest::new(
            user_id,
            "wallet.purchase",
            Localized::new("コインを購入しました!", "Coins purchased!"),
            Localized::new(format!("{}コインがウォレットに追加されたよ!", format_coins(total)), format!("{} coins were added to your wallet.", format_coins(total))),
            "/wallet",
        )
        .collapse(format!("wallet.purchase:{}", purchase.id))
        .data(serde_json::json!({ "purchase_id": purchase.id, "coins": total }));
        if trigger == Trigger::Buyer {
            // They are watching the purchase finish; the inbox entry is the receipt.
            notice = notice.inbox_only();
        }
        outbound::notify(&mut *tx, &notice).await?;
        tx.commit().await?;
        tracing::info!(purchase_id = %purchase.id, %user_id, coins = total, provider = %purchase.provider, "purchase completed");
        let sale = serde_json::json!({ "sku": purchase.sku, "provider": purchase.provider, "price_minor": purchase.price_minor, "currency": purchase.currency });
        self.core.analytics.track(AnalyticsEvent::new("wallet.purchase").user(Some(user_id)).subject("purchase", purchase.id).value(total as f64).props(sale)).await;
        self.push(user_id, Event::PurchaseUpdated(Box::new(purchase.to_proto()))).await;
        Ok(purchase)
    }

    /// Moves a still-pending purchase to a closed, unpaid status
    /// (`failed`, `cancelled` or `expired`) and voids its checkout. `None`
    /// when it was not pending.
    async fn close_unpaid(&self, purchase_id: Uuid, status: &'static str, failure_reason: &str) -> AppResult<Option<PurchaseRow>> {
        let failure_reason: String = failure_reason.chars().take(200).collect();
        let closed: Option<PurchaseRow> = sqlx::query_as(concat!(
            "UPDATE wallet_purchases SET status = $2, failure_reason = $3 WHERE id = $1 AND status = 'pending' RETURNING ",
            purchase_columns!()
        ))
        .bind(purchase_id)
        .bind(status)
        .bind(failure_reason)
        .fetch_optional(&self.core.db)
        .await?;
        if let Some(purchase) = &closed
            && let Some(user_id) = purchase.user_id
        {
            self.push(user_id, Event::PurchaseUpdated(Box::new(purchase.to_proto()))).await;
        }
        if let Some(purchase) = &closed {
            self.void_checkout(purchase).await;
        }
        Ok(closed)
    }

    /// Has the provider make the checkout of a closed, unpaid purchase
    /// impossible to pay, and records that it did. Until then the purchase
    /// may still be paid (a declined card tried again, a payment at the last
    /// moment), so it counts against the spending limit (`spending.rs`).
    /// `false` when the provider could not void it; the maintenance run
    /// tries again ([`Self::void_closed_checkouts`]).
    async fn void_checkout(&self, purchase: &PurchaseRow) -> bool {
        if purchase.provider_ref.is_empty() {
            return true;
        }
        let voided = match (self.provider(&purchase.provider), purchase.order(&self.core)) {
            (Some(provider), Some(order)) => provider.cancel_checkout(&order, &purchase.provider_ref).await,
            // Nobody can pay into it through this wallet (provider removed, buyer deleted).
            _ => Ok(()),
        };
        if let Err(e) = voided {
            tracing::warn!(purchase_id = %purchase.id, error = %e, "provider could not void the checkout of an unpaid purchase; will retry");
            return false;
        }
        let recorded = sqlx::query("UPDATE wallet_purchases SET checkout_voided_at = now() WHERE id = $1 AND checkout_voided_at IS NULL")
            .bind(purchase.id)
            .execute(&self.core.db)
            .await;
        if let Err(e) = recorded {
            tracing::warn!(purchase_id = %purchase.id, error = %e, "could not record a voided checkout; will retry");
            return false;
        }
        true
    }

    /// Voids the checkouts that could not be voided when their purchases
    /// closed. Returns how many it voided.
    pub(crate) async fn void_closed_checkouts(&self) -> AppResult<u64> {
        let due: Vec<PurchaseRow> = sqlx::query_as(concat!(
            "SELECT ",
            purchase_columns!(),
            " FROM wallet_purchases
              WHERE status IN ('failed', 'cancelled', 'expired') AND provider_ref <> '' AND checkout_voided_at IS NULL
              ORDER BY id LIMIT 200"
        ))
        .fetch_all(&self.core.db)
        .await?;
        let mut voided = 0;
        for purchase in due {
            voided += u64::from(self.void_checkout(&purchase).await);
        }
        Ok(voided)
    }

    /// The buyer abandons an unpaid purchase. See `WalletService.CancelPurchase`.
    pub(crate) async fn abandon_purchase(&self, user_id: Uuid, purchase_id: Uuid) -> AppResult<PurchaseRow> {
        let purchase = self.find_purchase(user_id, purchase_id).await?;
        ensure!(purchase.is_pending(), precondition, "PURCHASE_NOT_PENDING", "only a pending purchase can be cancelled");
        let cancelled = self.close_unpaid(purchase.id, "cancelled", "").await?;
        // Its checkout was voided on the way (if the provider could not, it
        // still completes when paid, and counts as spending until voided).
        cancelled.ok_or_else(|| AppError::precondition("PURCHASE_NOT_PENDING", "only a pending purchase can be cancelled"))
    }

    /// One of `user_id`'s purchases, or `PURCHASE_NOT_FOUND`.
    pub(crate) async fn find_purchase(&self, user_id: Uuid, purchase_id: Uuid) -> AppResult<PurchaseRow> {
        let purchase: Option<PurchaseRow> = sqlx::query_as(concat!("SELECT ", purchase_columns!(), " FROM wallet_purchases WHERE id = $1 AND user_id = $2"))
            .bind(purchase_id)
            .bind(user_id)
            .fetch_optional(&self.core.db)
            .await?;
        purchase.or_not_found("PURCHASE_NOT_FOUND", "purchase not found")
    }

    /// One page of `user_id`'s purchases, newest first.
    pub(crate) async fn purchases_page(&self, user_id: Uuid, page: &Page) -> AppResult<(Vec<PurchaseRow>, PageInfo)> {
        let before: Option<Uuid> = page.decode()?;
        let mut rows: Vec<PurchaseRow> = sqlx::query_as(concat!(
            "SELECT ",
            purchase_columns!(),
            " FROM wallet_purchases WHERE user_id = $1 AND id < $2 ORDER BY id DESC LIMIT $3"
        ))
        .bind(user_id)
        .bind(before.unwrap_or(NEWEST))
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| row.id);
        Ok((rows, info))
    }

    /// Closes purchases that ran out of time. Each one gets a last look at
    /// its provider first, so a payment that arrived without the app hearing
    /// about it still turns into coins. Returns how many purchases changed.
    pub(crate) async fn expire_due_purchases(&self) -> AppResult<u64> {
        let due: Vec<PurchaseRow> = sqlx::query_as(concat!(
            "SELECT ",
            purchase_columns!(),
            " FROM wallet_purchases WHERE status = 'pending' AND expires_at <= now() ORDER BY expires_at LIMIT 200"
        ))
        .fetch_all(&self.core.db)
        .await?;
        let mut changed = 0;
        for purchase in due {
            let purchase_id = purchase.id;
            match self.expire_purchase(purchase).await {
                Ok(true) => changed += 1,
                Ok(false) => {}
                Err(e) => tracing::warn!(%purchase_id, error = %e, "could not close an overdue purchase; will retry"),
            }
        }
        Ok(changed)
    }

    async fn expire_purchase(&self, purchase: PurchaseRow) -> AppResult<bool> {
        let last_look = match (self.provider(&purchase.provider), purchase.order(&self.core)) {
            (Some(provider), Some(order)) if !purchase.provider_ref.is_empty() => provider.confirm(&order, &purchase.provider_ref, "").await,
            // Nobody left to ask (provider removed, checkout never created, buyer deleted).
            _ => Ok(PaymentState::Pending),
        };
        match last_look {
            Ok(PaymentState::Paid) => self.complete_purchase(purchase.id, Trigger::Background).await.map(|_| true),
            Ok(PaymentState::Failed { reason }) => Ok(self.close_unpaid(purchase.id, "failed", &reason).await?.is_some()),
            Ok(PaymentState::Pending) => Ok(self.close_unpaid(purchase.id, "expired", "").await?.is_some()),
            // A provider that cannot be reached gets another chance next round,
            // but not forever.
            Err(_) if now() - purchase.expires_at > UNREACHABLE_PROVIDER_GRACE => Ok(self.close_unpaid(purchase.id, "expired", "provider_unreachable").await?.is_some()),
            Err(e) => Err(e),
        }
    }
}

/// `user_id`'s purchase started with `idempotency_key`; `None` for an empty key.
async fn purchase_by_key(db: impl PgExecutor<'_>, user_id: Uuid, idempotency_key: &str) -> AppResult<Option<PurchaseRow>> {
    if idempotency_key.is_empty() {
        return Ok(None);
    }
    Ok(sqlx::query_as(concat!("SELECT ", purchase_columns!(), " FROM wallet_purchases WHERE user_id = $1 AND idempotency_key = $2"))
        .bind(user_id)
        .bind(idempotency_key)
        .fetch_optional(db)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_map_to_the_api_enum() {
        for (stored, api) in [
            ("pending", PurchaseStatus::PURCHASE_STATUS_PENDING),
            ("completed", PurchaseStatus::PURCHASE_STATUS_COMPLETED),
            ("failed", PurchaseStatus::PURCHASE_STATUS_FAILED),
            ("cancelled", PurchaseStatus::PURCHASE_STATUS_CANCELLED),
            ("expired", PurchaseStatus::PURCHASE_STATUS_EXPIRED),
            ("???", PurchaseStatus::PURCHASE_STATUS_UNSPECIFIED),
        ] {
            assert_eq!(status_to_proto(stored), api);
        }
    }
}
