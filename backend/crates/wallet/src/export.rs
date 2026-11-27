//! The wallet's section of a personal data export.
use async_trait::async_trait;
use chrono::NaiveDate;
use sqlx::PgPool;
use tokuly_core::{export::DataExporter, keys::Keys, prelude::*};

use crate::{
    entries::{Scope, fetch_entries},
    payouts::PayoutRow,
    purchases::PurchaseRow,
};

/// Most ledger entries one export contains (the newest ones).
const MAX_EXPORTED_ENTRIES: i64 = 50_000;

/// Exports a user's balances, ledger history, purchases, payout requests
/// and daily-bonus state.
///
/// Holds a database handle and the keys rather than the wallet itself: the
/// exporter is stored inside `Core`, and pointing back at it would form a
/// reference cycle.
pub(crate) struct WalletExporter {
    db: PgPool,
    keys: Keys,
}

impl WalletExporter {
    pub fn new(core: &Core) -> Self {
        Self { db: core.db.clone(), keys: core.keys.clone() }
    }
}

#[async_trait]
impl DataExporter for WalletExporter {
    fn name(&self) -> &'static str {
        "wallet"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        let account: Option<(i64, i64, Option<i64>)> =
            sqlx::query_as("SELECT coins, earnings, monthly_limit_yen FROM wallet_accounts WHERE user_id = $1").bind(user_id).fetch_optional(&self.db).await?;
        let (coins, earnings, monthly_limit_yen) = account.unwrap_or((0, 0, None));

        let entries = fetch_entries(&self.db, user_id, Scope::ALL, None, MAX_EXPORTED_ENTRIES + 1, None).await?;
        let truncated = entries.len() as i64 > MAX_EXPORTED_ENTRIES;
        let ledger: Vec<serde_json::Value> = entries
            .iter()
            .take(MAX_EXPORTED_ENTRIES as usize)
            .map(|entry| {
                // An account the entry keeps from this user stays out of their export too.
                let (from_user_id, to_user_id) = entry.parties_for(user_id);
                serde_json::json!({
                    "id": entry.id,
                    "kind": entry.kind,
                    "reason": entry.reason,
                    "coins_delta": entry.coins_delta,
                    "earnings_delta": entry.earnings_delta,
                    "amount": entry.amount,
                    "fee": entry.fee,
                    "from_user_id": from_user_id,
                    "to_user_id": to_user_id,
                    "refunded": entry.refunded,
                    "refund_of": entry.refund_of,
                    "memo": entry.memo,
                    "created_at": entry.created_at.to_rfc3339(),
                })
            })
            .collect();

        let purchases: Vec<PurchaseRow> = sqlx::query_as(concat!("SELECT ", crate::purchases::purchase_columns!(), " FROM wallet_purchases WHERE user_id = $1 ORDER BY id")).bind(user_id).fetch_all(&self.db).await?;
        let purchases: Vec<serde_json::Value> = purchases
            .iter()
            .map(|purchase| {
                serde_json::json!({
                    "id": purchase.id,
                    "status": purchase.status,
                    "sku": purchase.sku,
                    "coins": purchase.coins,
                    "bonus_coins": purchase.bonus_coins,
                    "price_minor": purchase.price_minor,
                    "currency": purchase.currency,
                    "provider": purchase.provider,
                    "created_at": purchase.created_at.to_rfc3339(),
                    "completed_at": purchase.completed_at.map(|at| at.to_rfc3339()),
                })
            })
            .collect();

        let payouts: Vec<PayoutRow> = sqlx::query_as(concat!("SELECT ", crate::payouts::payout_columns!(), " FROM wallet_payout_requests WHERE user_id = $1 ORDER BY id")).bind(user_id).fetch_all(&self.db).await?;
        let payouts: Vec<serde_json::Value> = payouts
            .iter()
            .map(|payout| {
                serde_json::json!({
                    "id": payout.id,
                    "status": payout.status,
                    "coins": payout.coins,
                    "payout_minor": payout.payout_minor,
                    "currency": payout.currency,
                    "destination": payout.destination(&self.keys),
                    "note": payout.note,
                    "review_note": payout.review_note,
                    "created_at": payout.created_at.to_rfc3339(),
                    "reviewed_at": payout.reviewed_at.map(|at| at.to_rfc3339()),
                    "paid_at": payout.paid_at.map(|at| at.to_rfc3339()),
                })
            })
            .collect();

        let bonus: Option<(Option<NaiveDate>, i32, i32, i32, i64)> =
            sqlx::query_as("SELECT last_claimed_on, streak, longest_streak, total_claims, total_coins FROM wallet_daily_bonuses WHERE user_id = $1").bind(user_id).fetch_optional(&self.db).await?;
        let daily_bonus = bonus.map(|(last_claimed_on, streak, longest_streak, total_claims, total_coins)| {
            serde_json::json!({
                "last_claimed_on": last_claimed_on.map(|day| day.to_string()),
                "streak": streak,
                "longest_streak": longest_streak,
                "total_claims": total_claims,
                "total_coins": total_coins,
            })
        });

        Ok(serde_json::json!({
            "balance": { "coins": coins, "earnings": earnings },
            "monthly_spending_limit_yen": monthly_limit_yen,
            "ledger": ledger,
            "ledger_truncated": truncated,
            "purchases": purchases,
            "payout_requests": payouts,
            "daily_bonus": daily_bonus,
        }))
    }
}
