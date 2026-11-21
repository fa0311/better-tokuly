//! Buying paid packs with coins.
//!
//! The wallet ledger commits a payment in its own transaction, so a purchase
//! cannot be one atomic step. It is three, ordered so that nothing is lost
//! whichever one fails:
//!
//! 1. an *intent* row is written. From then on the creator cannot delete the
//!    pack (see `StampStudioService.DeletePack`);
//! 2. the ledger moves the coins, idempotently on the key
//!    `stamp-pack:<pack_id>:<user_id>`;
//! 3. the purchase row is written, the intent removed, the pack installed.
//!
//! If step 3 fails the user has paid without owning the pack — until they
//! try again: the ledger recognises the key, moves nothing and step 3 runs.
//! `PurchasePack` is therefore safe to retry, and never charges twice.
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
        core.cache.rate_limit(&format!("stamp-buy:{user_id}"), 20, Duration::from_secs(60)).await?;

        sqlx::query(
            "INSERT INTO stamp_purchase_intents (user_id, pack_id) VALUES ($1, $2)
             ON CONFLICT (user_id, pack_id) DO UPDATE SET created_at = now()",
        )
        .bind(user_id)
        .bind(pack.id)
        .execute(&core.db)
        .await
        .map_err(|e| match AppError::from(e) {
            // A foreign key failure: the pack was deleted a moment ago.
            e if e.is_not_found() => AppError::not_found("PACK_NOT_FOUND", "stamp pack not found"),
            e => e,
        })?;

        let payment = match self.pay(user_id, pack).await {
            Ok(payment) => payment,
            Err(e) => {
                // Nothing was charged, so the pack need not be protected any longer.
                let _ = sqlx::query("DELETE FROM stamp_purchase_intents WHERE user_id = $1 AND pack_id = $2")
                    .bind(user_id)
                    .bind(pack.id)
                    .execute(&core.db)
                    .await;
                return Err(e);
            }
        };

        if let Err(e) = self.complete_purchase(user_id, pack, &payment).await {
            // The one way to pay for a pack that no longer exists: its creator's
            // account was purged between the payment and now. Give the coins back.
            if payment.applied && packs::find_by_id(&core.db, pack.id).await?.is_none() {
                ledger::refund(core, payment.entry_id, "stamp.pack.refund").await?;
            }
            return Err(e);
        }

        let mut event = AnalyticsEvent::new("stamp.pack.purchase")
            .user(Some(user_id))
            .subject("stamp_pack", pack.id)
            .value(pack.price as f64);
        if let Some(creator_id) = pack.creator_id {
            event = event.owner(creator_id);
        }
        core.analytics.track(event).await;
        Ok(if payment.applied { pack.price } else { 0 })
    }

    /// Moves the coins: to the creator's earnings minus the platform fee,
    /// or to the platform for official packs.
    async fn pay(&self, user_id: Uuid, pack: &PackRow) -> AppResult<Payment> {
        let key = idempotency_key(pack.id, user_id);
        let memo = serde_json::json!({ "pack_id": pack.id, "pack_name": pack.name });
        let price = pack.price as i64;
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
    async fn complete_purchase(&self, user_id: Uuid, pack: &PackRow, payment: &Payment) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let recorded = sqlx::query(
            "INSERT INTO stamp_pack_purchases (user_id, pack_id, price, net, entry_id) VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT DO NOTHING",
        )
        .bind(user_id)
        .bind(pack.id)
        .bind(pack.price)
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
            self.announce_sale(&mut tx, user_id, pack, payment).await?;
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
            price: pack.price as i64,
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
