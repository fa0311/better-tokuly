//! Buying paid packs with coins.
//!
//! The wallet ledger commits a payment in its own transaction, so a purchase
//! cannot be one atomic step. It is three, ordered so that nothing is lost
//! whichever one fails:
//!
//! 1. an *intent* row is written with the price the buyer confirmed. From
//!    then on the creator cannot delete the pack (see
//!    `StampStudioService.DeletePack`);
//! 2. the ledger moves that price, idempotently on the key
//!    `stamp-pack:<pack_id>:<user_id>`;
//! 3. the purchase row is written, the intent removed, the pack installed.
//!
//! If step 3 fails (or step 2 fails in a way that leaves open whether it
//! committed) the intent stays, and the purchase is settled at its stored
//! price by the buyer's next `PurchasePack` or by the worker after 15
//! minutes: the ledger recognises the key, moves nothing and step 3 runs —
//! also when the pack's price changed or it left the store meanwhile. Only
//! a refused payment removes the intent. `PurchasePack` is therefore safe
//! to retry, and never charges twice.
use std::time::Duration;

use sqlx::{Postgres, Transaction};
use tokuly_core::{
    analytics::AnalyticsEvent,
    events,
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::common::{PageInfo, SubjectType};
use tokuly_wallet::ledger;

use crate::{
    Stamp,
    access::{Holding, Viewer},
    events::{PACK_PURCHASED, PackPurchase},
    installs,
    model::{PackRow, pack_columns, status},
    packs, stats,
};

/// Ledger reason of pack purchases (`wallet_entries.reason`).
const LEDGER_REASON: &str = "stamp.pack";

/// A fresh ledger key for one order of `pack_id` by `user_id`.
fn order_key(pack_id: Uuid, user_id: Uuid) -> String {
    format!("stamp-pack:{pack_id}:{user_id}:{}", Uuid::now_v7().simple())
}

/// Age after which the worker settles an order the buyer did not finish.
const UNFINISHED_AFTER: &str = "15 minutes";
/// How long a settler holds an order; also the worker's back-off after an
/// attempt it could not finish.
const LEASE: &str = "5 minutes";
/// Orders one worker pass settles.
const SETTLE_BATCH: i64 = 100;
/// How long a request waits for another settler of the same order (a
/// second tap on "buy") before answering `PURCHASE_IN_PROGRESS`.
const BUSY_WAIT: Duration = Duration::from_millis(50);
const BUSY_TRIES: u32 = 100;

/// An open order (`stamp_purchase_intents`).
#[derive(Debug, Clone, sqlx::FromRow)]
struct Order {
    user_id: Uuid,
    pack_id: Uuid,
    price: i32,
    payment_key: String,
}

async fn find_order(db: &sqlx::PgPool, user_id: Uuid, pack_id: Uuid) -> AppResult<Option<Order>> {
    Ok(sqlx::query_as(
        "SELECT user_id, pack_id, price, payment_key FROM stamp_purchase_intents WHERE user_id = $1 AND pack_id = $2",
    )
    .bind(user_id)
    .bind(pack_id)
    .fetch_optional(db)
    .await?)
}

/// Takes the order for this settler; `false` while another one holds it.
/// A lease, not a row lock: settling calls the ledger, which needs a
/// connection of its own.
async fn claim(db: &sqlx::PgPool, order: &Order) -> AppResult<bool> {
    let claimed = sqlx::query(
        "UPDATE stamp_purchase_intents SET leased_until = now() + $2::interval
         WHERE payment_key = $1 AND (leased_until IS NULL OR leased_until < now())",
    )
    .bind(&order.payment_key)
    .bind(LEASE)
    .execute(db)
    .await?;
    Ok(claimed.rows_affected() == 1)
}

async fn release(db: &sqlx::PgPool, order: &Order) -> AppResult<()> {
    sqlx::query("UPDATE stamp_purchase_intents SET leased_until = NULL WHERE payment_key = $1")
        .bind(&order.payment_key)
        .execute(db)
        .await?;
    Ok(())
}

/// Whether the ledger turned a payment down: nothing moved under the key.
fn refused(error: &AppError) -> bool {
    use connectrpc::ErrorCode;
    matches!(error.code, ErrorCode::InvalidArgument | ErrorCode::FailedPrecondition | ErrorCode::NotFound)
        // Keys are per order, so these cannot happen; if they did, the key
        // could never pay this order.
        || matches!(error.reason.as_ref(), "IDEMPOTENCY_KEY_REUSED" | "ENTRY_REFUNDED")
}

fn not_for_sale() -> AppError {
    AppError::precondition("PACK_NOT_FOR_SALE", "this pack is not sold any more")
}

/// What settling an order came to.
enum Settled {
    /// Recorded; the coins this call charged (0 when paid earlier).
    Bought(i32),
    /// Over without a purchase: never paid, or paid and refunded.
    Ended,
}

/// A payment as recorded by the ledger.
struct Payment {
    entry_id: Uuid,
    /// What the creator received; 0 for official packs.
    net: i64,
    /// False when the ledger had already seen this purchase (a retry).
    applied: bool,
}

impl Stamp {
    /// Buys `pack` for `user_id` and installs it. Returns the coins charged
    /// by this call (0 when the pack was already owned or already paid for).
    ///
    /// An earlier order of the buyer is settled first: if its payment went
    /// through, it is recorded at the price agreed then, whatever the pack
    /// costs now or whether it is still sold; if not, it ends and this call
    /// is a new purchase at today's price.
    pub(crate) async fn purchase(
        &self,
        viewer: &Viewer,
        user_id: Uuid,
        pack: &PackRow,
        holding: Holding,
        expected_price: i32,
    ) -> AppResult<i32> {
        let core = &self.core;
        ensure!(!viewer.manages(&pack.facts()), precondition, "PACK_NOT_FOR_SALE", "this is your own pack");
        // Checked before any coins move: a purchase ends with the pack in the library.
        installs::ensure_room(&core.db, user_id, pack.id).await?;
        if !holding.owned {
            core.cache.rate_limit(&format!("stamp-buy:{user_id}"), 20, Duration::from_secs(60)).await?;
        }
        let mut holding = holding;
        let mut tries = 0;
        loop {
            if holding.owned {
                // Buying again is a request to have the pack: make sure it is installed.
                let mut tx = core.db.begin().await?;
                installs::install(&mut tx, user_id, pack, true).await?;
                tx.commit().await?;
                return Ok(0);
            }
            let (order, placed_now) = match find_order(&core.db, user_id, pack.id).await? {
                Some(order) => (order, false),
                None => (self.place_order(user_id, pack, expected_price).await?, true),
            };
            if claim(&core.db, &order).await? {
                // An order placed by this call is paid; an earlier one only
                // finishes if it was paid already.
                match self.settle_claimed(&order, pack, placed_now).await {
                    Ok(Settled::Bought(charged)) => return Ok(charged),
                    Ok(Settled::Ended) if !placed_now => continue,
                    Ok(Settled::Ended) => return Err(not_for_sale()),
                    Err(e) => {
                        release(&core.db, &order).await?;
                        return Err(e);
                    }
                }
            }
            // Someone else (a second tap) is settling it: wait for them.
            tries += 1;
            ensure!(tries < BUSY_TRIES, unavailable, "PURCHASE_IN_PROGRESS", "the purchase is still being processed");
            tokio::time::sleep(BUSY_WAIT).await;
            holding = packs::holding(core, Some(user_id), pack.id).await?;
        }
    }

    /// Writes a new order at today's price, after the checks a new purchase
    /// needs. When a concurrent call placed one first, that one is returned.
    async fn place_order(&self, user_id: Uuid, pack: &PackRow, expected_price: i32) -> AppResult<Order> {
        ensure!(
            pack.is_published() && pack.price > 0,
            precondition,
            "PACK_NOT_FOR_SALE",
            "this pack is not sold for coins"
        );
        ensure!(
            expected_price == pack.price,
            precondition,
            "PRICE_CHANGED",
            "the price changed to {} coins; confirm the new price",
            pack.price
        );
        let gone = || AppError::not_found("PACK_NOT_FOUND", "stamp pack not found");
        sqlx::query(
            "INSERT INTO stamp_purchase_intents (user_id, pack_id, price, payment_key) VALUES ($1, $2, $3, $4)
             ON CONFLICT (user_id, pack_id) DO NOTHING",
        )
        .bind(user_id)
        .bind(pack.id)
        .bind(pack.price)
        .bind(order_key(pack.id, user_id))
        .execute(&self.core.db)
        .await
        .map_err(|e| match AppError::from(e) {
            // A foreign key failure: the pack was deleted a moment ago.
            e if e.is_not_found() => gone(),
            e => e,
        })?;
        find_order(&self.core.db, user_id, pack.id).await?.ok_or_else(gone)
    }

    /// Settles an order this caller holds. `pack` is the pack as it is now.
    ///
    /// The ledger says whether the order was paid. Paid: the purchase is
    /// recorded at the agreed price — also for a pack that left the store or
    /// became free since — unless moderators took the pack down, which
    /// refunds it. Unpaid: with `pay`, it is paid now; otherwise it ends.
    async fn settle_claimed(&self, order: &Order, pack: &PackRow, pay: bool) -> AppResult<Settled> {
        let core = &self.core;
        let payment = match ledger::find_by_key(core, &order.payment_key).await? {
            Some(entry) if entry.refunded => {
                self.end_order(order).await?;
                return Ok(Settled::Ended);
            }
            Some(entry) => Payment { entry_id: entry.entry_id, net: entry.amount - entry.fee, applied: false },
            None if !pay => {
                self.end_order(order).await?;
                return Ok(Settled::Ended);
            }
            None => match self.pay(order, pack).await {
                Ok(payment) => payment,
                Err(e) if refused(&e) => {
                    self.end_order(order).await?;
                    // The creator's account was erased a moment ago: their
                    // packs leave the store as soon as the stamp domain hears of it.
                    return Err(if e.reason == "ACCOUNT_CLOSED" { not_for_sale() } else { e });
                }
                // Perhaps after the payment committed: the order stays.
                Err(e) => return Err(e),
            },
        };
        if pack.status == status::REMOVED {
            ledger::refund(core, payment.entry_id, "stamp.pack.refund").await?;
            self.end_order(order).await?;
            return Ok(Settled::Ended);
        }
        self.complete_purchase(order, pack, &payment).await?;
        let mut event = AnalyticsEvent::new("stamp.pack.purchase")
            .user(Some(order.user_id))
            .subject("stamp_pack", pack.id)
            .value(order.price as f64);
        if let Some(creator_id) = pack.creator_id {
            event = event.owner(creator_id);
        }
        core.analytics.track(event).await;
        Ok(Settled::Bought(if payment.applied { order.price } else { 0 }))
    }

    /// Removes an order that ends without a purchase. A pack kept only for
    /// open orders after its creator's account was erased goes with the last.
    async fn end_order(&self, order: &Order) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        sqlx::query("DELETE FROM stamp_purchase_intents WHERE payment_key = $1")
            .bind(&order.payment_key)
            .execute(&mut *tx)
            .await?;
        self.erase_if_creator_gone(&mut tx, order.pack_id).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Settles the orders nobody came back to (worker, every few minutes):
    /// an order older than [`UNFINISHED_AFTER`] is recorded if it was paid
    /// and ended if not. Orders another settler holds are skipped, and one
    /// that could not be finished waits out its lease, so a pass always
    /// reaches the next ones. Returns how many were looked at.
    pub async fn settle_unfinished_purchases(&self) -> AppResult<usize> {
        let orders: Vec<Order> = sqlx::query_as(
            "UPDATE stamp_purchase_intents o SET leased_until = now() + $2::interval
             FROM (SELECT payment_key FROM stamp_purchase_intents
                   WHERE created_at < now() - $1::interval AND (leased_until IS NULL OR leased_until < now())
                   ORDER BY leased_until NULLS FIRST, created_at
                   LIMIT $3
                   FOR UPDATE SKIP LOCKED) due
             WHERE o.payment_key = due.payment_key
             RETURNING o.user_id, o.pack_id, o.price, o.payment_key",
        )
        .bind(UNFINISHED_AFTER)
        .bind(LEASE)
        .bind(SETTLE_BATCH)
        .fetch_all(&self.core.db)
        .await?;
        for order in &orders {
            let Some(pack) = packs::find_by_id(&self.core.db, order.pack_id).await? else { continue };
            let (user_id, pack_id) = (order.user_id, order.pack_id);
            match self.settle_claimed(order, &pack, false).await {
                Ok(Settled::Bought(_)) => {
                    tracing::info!(%user_id, %pack_id, "an unfinished stamp purchase was recorded")
                }
                Ok(Settled::Ended) => {}
                // The lease stays as the back-off.
                Err(e) => tracing::warn!(%user_id, %pack_id, error = %e, "an unfinished stamp purchase is still open"),
            }
        }
        Ok(orders.len())
    }

    /// Moves the coins: to the creator's earnings minus the platform fee,
    /// or to the platform for official packs.
    async fn pay(&self, order: &Order, pack: &PackRow) -> AppResult<Payment> {
        let (user_id, key) = (order.user_id, order.payment_key.as_str());
        let memo = serde_json::json!({ "pack_id": pack.id, "pack_name": pack.name });
        let price = order.price as i64;
        match pack.creator_id {
            Some(creator_id) => {
                let receipt = ledger::transfer(
                    &self.core,
                    user_id,
                    creator_id,
                    price,
                    self.cfg.fee_percent,
                    LEDGER_REASON,
                    key,
                    memo,
                )
                .await?;
                Ok(Payment { entry_id: receipt.entry_id, net: receipt.net, applied: receipt.applied })
            }
            None => {
                let (entry_id, applied) = ledger::spend(&self.core, user_id, price, LEDGER_REASON, key, memo).await?;
                Ok(Payment { entry_id, net: 0, applied })
            }
        }
    }

    /// Records the purchase and everything that follows from it, atomically.
    async fn complete_purchase(&self, order: &Order, pack: &PackRow, payment: &Payment) -> AppResult<()> {
        let (user_id, price) = (order.user_id, order.price);
        let mut tx = self.core.db.begin().await?;
        let recorded = sqlx::query(
            "INSERT INTO stamp_pack_purchases (user_id, pack_id, price, net, entry_id) VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT DO NOTHING",
        )
        .bind(user_id)
        .bind(pack.id)
        .bind(price)
        .bind(payment.net as i32)
        .bind(payment.entry_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        sqlx::query("DELETE FROM stamp_purchase_intents WHERE payment_key = $1")
            .bind(&order.payment_key)
            .execute(&mut *tx)
            .await?;
        // A concurrent retry may have recorded it already; count the sale once.
        if recorded {
            self.announce_sale(&mut tx, user_id, pack, payment, price).await?;
        }
        installs::install(&mut tx, user_id, pack, true).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn announce_sale(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        buyer_id: Uuid,
        pack: &PackRow,
        payment: &Payment,
        price: i32,
    ) -> AppResult<()> {
        sqlx::query("UPDATE stamp_packs SET purchase_count = purchase_count + 1 WHERE id = $1")
            .bind(pack.id)
            .execute(&mut **tx)
            .await?;
        stats::bump_daily(tx, pack.id, 0, 1, payment.net).await?;
        let purchase = PackPurchase {
            pack_id: pack.id,
            buyer_id,
            creator_id: pack.creator_id,
            price: price as i64,
            net: payment.net,
            entry_id: payment.entry_id,
        };
        events::emit(&mut **tx, PACK_PURCHASED, Some(buyer_id), &purchase).await?;
        let Some(creator_id) = pack.creator_id else { return Ok(()) };
        let notice = NotificationRequest::new(
            creator_id,
            "stamp.pack.sold",
            Localized::new("スタンプが売れたよ!", "You sold a stamp pack!"),
            Localized::new(
                format!("「{}」が購入されたよ! +{}コイン", pack.name, payment.net),
                format!("Someone bought “{}”. +{} coins", pack.name, payment.net),
            ),
            pack.studio_path(),
        )
        .actor(buyer_id)
        .subject(SubjectType::SUBJECT_TYPE_STAMP_PACK, pack.id)
        // One entry per pack: a popular pack must not flood the inbox.
        .collapse(format!("stamp.pack.sold:{}", pack.id));
        outbound::notify(&mut **tx, &notice).await
    }
}

/// One page of the packs a user bought, most recent first. Packs that were
/// taken down are left out.
pub async fn list(core: &Core, user_id: Uuid, page: &Page) -> AppResult<(Vec<PackRow>, PageInfo)> {
    #[derive(sqlx::FromRow)]
    struct PurchasedRow {
        #[sqlx(flatten)]
        pack: PackRow,
        purchased_at: Time,
    }
    let cursor: Option<(Time, Uuid)> = page.decode()?;
    let (before_at, before_id) = cursor.unzip();
    let mut rows: Vec<PurchasedRow> = sqlx::query_as(concat!(
        "SELECT ",
        pack_columns!(),
        ", o.purchased_at
         FROM stamp_pack_purchases o JOIN stamp_packs p ON p.id = o.pack_id
         WHERE o.user_id = $1 AND p.status <> 5
           AND ($2::timestamptz IS NULL OR (o.purchased_at, o.pack_id) < ($2, $3))
         ORDER BY o.purchased_at DESC, o.pack_id DESC
         LIMIT $4"
    ))
    .bind(user_id)
    .bind(before_at)
    .bind(before_id)
    .bind(page.fetch())
    .fetch_all(&core.db)
    .await?;
    let info = page.finish(&mut rows, |row| (row.purchased_at, row.pack.id));
    Ok((rows.into_iter().map(|row| row.pack).collect(), info))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_order_pays_under_a_key_of_its_own() {
        let (pack, user) = (Uuid::now_v7(), Uuid::now_v7());
        let (first, second) = (order_key(pack, user), order_key(pack, user));
        assert_ne!(first, second);
        assert!(first.starts_with(&format!("stamp-pack:{pack}:{user}:")));
        assert!(first.len() <= 200, "the ledger accepts keys up to 200 characters");
    }
}
