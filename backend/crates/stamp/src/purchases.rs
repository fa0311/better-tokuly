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
    model::{PackRow, pack_columns},
    packs, stats,
};

/// Ledger reason of pack purchases (`wallet_entries.reason`).
const LEDGER_REASON: &str = "stamp.pack";

fn idempotency_key(pack_id: Uuid, user_id: Uuid) -> String {
    format!("stamp-pack:{pack_id}:{user_id}")
}

/// Age after which the worker settles a purchase the buyer did not finish.
/// Also how long a purchase takes at most when nothing fails.
const UNFINISHED_AFTER: &str = "15 minutes";

/// The price of `user_id`'s unfinished purchase of a pack, if they started one.
async fn unfinished_price(db: &sqlx::PgPool, user_id: Uuid, pack_id: Uuid) -> AppResult<Option<i32>> {
    Ok(sqlx::query_scalar("SELECT price FROM stamp_purchase_intents WHERE user_id = $1 AND pack_id = $2")
        .bind(user_id)
        .bind(pack_id)
        .fetch_optional(db)
        .await?)
}

async fn forget_intent(db: &sqlx::PgPool, user_id: Uuid, pack_id: Uuid) -> AppResult<()> {
    sqlx::query("DELETE FROM stamp_purchase_intents WHERE user_id = $1 AND pack_id = $2")
        .bind(user_id)
        .bind(pack_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Whether the ledger turned a payment down (no coins moved), as opposed
/// to a failure that may have happened after it committed. (A key used for
/// a different amount, `IDEMPOTENCY_KEY_REUSED`, means coins did move.)
fn refused(error: &AppError) -> bool {
    use connectrpc::ErrorCode;
    matches!(error.code, ErrorCode::InvalidArgument | ErrorCode::FailedPrecondition | ErrorCode::NotFound)
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
    /// A purchase that started before (its intent is stored) is finished at
    /// the price agreed then, whether or not the pack is still sold or costs
    /// the same: the earlier attempt may have paid already.
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
        if holding.owned {
            // Buying again is a request to have the pack: make sure it is installed.
            let mut tx = core.db.begin().await?;
            installs::install(&mut tx, user_id, pack, true).await?;
            tx.commit().await?;
            return Ok(0);
        }
        core.cache.rate_limit(&format!("stamp-buy:{user_id}"), 20, Duration::from_secs(60)).await?;
        let price = match unfinished_price(&core.db, user_id, pack.id).await? {
            Some(price) => price,
            None => {
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
                // The order, at the price the buyer confirmed. A concurrent
                // first attempt that stored one already keeps its price.
                sqlx::query_scalar(
                    "INSERT INTO stamp_purchase_intents (user_id, pack_id, price) VALUES ($1, $2, $3)
                     ON CONFLICT (user_id, pack_id) DO UPDATE SET price = stamp_purchase_intents.price
                     RETURNING price",
                )
                .bind(user_id)
                .bind(pack.id)
                .bind(pack.price)
                .fetch_one(&core.db)
                .await
                .map_err(|e| match AppError::from(e) {
                    // A foreign key failure: the pack was deleted a moment ago.
                    e if e.is_not_found() => AppError::not_found("PACK_NOT_FOUND", "stamp pack not found"),
                    e => e,
                })?
            }
        };
        self.settle(user_id, pack, price).await
    }

    /// Pays for (or recognises the payment of) the purchase whose intent at
    /// `price` is stored, and records it. `pack` is the pack as it is now.
    async fn settle(&self, user_id: Uuid, pack: &PackRow, price: i32) -> AppResult<i32> {
        let core = &self.core;
        let payment = match self.pay(user_id, pack, price).await {
            Ok(payment) => payment,
            Err(e) => {
                // The ledger refused: nothing moved, and the order is over.
                // Any other failure (the database, a timeout) may have come
                // after the payment committed: the intent stays, so the pack
                // stays protected and the purchase is settled later.
                if refused(&e) {
                    forget_intent(&core.db, user_id, pack.id).await?;
                }
                return Err(e);
            }
        };
        // Charged just now for a pack that is no longer sold: the earlier
        // attempt never paid, so there is nothing to finish. Give it back.
        if payment.applied && !(pack.is_published() && pack.price > 0) {
            ledger::refund(core, payment.entry_id, "stamp.pack.refund").await?;
            forget_intent(&core.db, user_id, pack.id).await?;
            bail!(precondition, "PACK_NOT_FOR_SALE", "this pack is not sold any more");
        }

        if let Err(e) = self.complete_purchase(user_id, pack, &payment, price).await {
            // Paid for a pack that no longer exists (its intent went with it):
            // give the coins back.
            if packs::find_by_id(&core.db, pack.id).await?.is_none() {
                ledger::refund(core, payment.entry_id, "stamp.pack.refund").await?;
            }
            return Err(e);
        }

        let mut event = AnalyticsEvent::new("stamp.pack.purchase")
            .user(Some(user_id))
            .subject("stamp_pack", pack.id)
            .value(price as f64);
        if let Some(creator_id) = pack.creator_id {
            event = event.owner(creator_id);
        }
        core.analytics.track(event).await;
        Ok(if payment.applied { price } else { 0 })
    }

    /// Settles the purchases nobody came back to (worker, every few
    /// minutes): an intent older than [`UNFINISHED_AFTER`] is paid, or its
    /// earlier payment recognised, at its price and recorded. Returns how
    /// many were looked at.
    pub async fn settle_unfinished_purchases(&self) -> AppResult<usize> {
        let unfinished: Vec<(Uuid, Uuid, i32)> = sqlx::query_as(
            "SELECT user_id, pack_id, price FROM stamp_purchase_intents
             WHERE created_at < now() - $1::interval
             ORDER BY created_at LIMIT 100",
        )
        .bind(UNFINISHED_AFTER)
        .fetch_all(&self.core.db)
        .await?;
        for (user_id, pack_id, price) in &unfinished {
            let Some(pack) = packs::find_by_id(&self.core.db, *pack_id).await? else { continue };
            match self.settle(*user_id, &pack, *price).await {
                Ok(_) => {}
                Err(e) if refused(&e) => {
                    tracing::info!(%user_id, %pack_id, reason = %e.reason, "an unfinished stamp purchase was not paid")
                }
                Err(e) => tracing::warn!(%user_id, %pack_id, error = %e, "an unfinished stamp purchase is still open"),
            }
        }
        Ok(unfinished.len())
    }

    /// Moves the coins: to the creator's earnings minus the platform fee,
    /// or to the platform for official packs.
    async fn pay(&self, user_id: Uuid, pack: &PackRow, price: i32) -> AppResult<Payment> {
        let key = idempotency_key(pack.id, user_id);
        let memo = serde_json::json!({ "pack_id": pack.id, "pack_name": pack.name });
        let price = price as i64;
        match pack.creator_id {
            Some(creator_id) => {
                let receipt = ledger::transfer(
                    &self.core,
                    user_id,
                    creator_id,
                    price,
                    self.cfg.fee_percent,
                    LEDGER_REASON,
                    &key,
                    memo,
                )
                .await?;
                Ok(Payment { entry_id: receipt.entry_id, net: receipt.net, applied: receipt.applied })
            }
            None => {
                let (entry_id, applied) = ledger::spend(&self.core, user_id, price, LEDGER_REASON, &key, memo).await?;
                Ok(Payment { entry_id, net: 0, applied })
            }
        }
    }

    /// Records the purchase and everything that follows from it, atomically.
    async fn complete_purchase(&self, user_id: Uuid, pack: &PackRow, payment: &Payment, price: i32) -> AppResult<()> {
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
        sqlx::query("DELETE FROM stamp_purchase_intents WHERE user_id = $1 AND pack_id = $2")
            .bind(user_id)
            .bind(pack.id)
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
    fn idempotency_key_names_pack_and_buyer() {
        let (pack, user) = (Uuid::now_v7(), Uuid::now_v7());
        assert_eq!(idempotency_key(pack, user), format!("stamp-pack:{pack}:{user}"));
        assert!(idempotency_key(pack, user).len() <= 200, "the ledger accepts keys up to 200 characters");
    }
}
