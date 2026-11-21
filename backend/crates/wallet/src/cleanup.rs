//! Erasing a deleted user's wallet (`identity.user.deleted`).
//!
//! A wallet is half personal data, half bookkeeping, and the two are treated
//! differently:
//!
//! - **Bookkeeping stays, unlinked.** Ledger entries, purchases, payout
//!   requests and adjustment records are what the platform's accounts are
//!   made of; deleting them would make the books stop adding up. They keep
//!   their amounts and dates but lose the pointer to the person, exactly as
//!   the schema's `ON DELETE SET NULL` rules would do if the user row were
//!   removed (identity keeps that row as an anonymous tombstone, so the
//!   rules never fire on their own).
//! - **Personal data goes.** The account row with its balances, the payout
//!   destination (bank details) and note, the daily-bonus streak.
//!
//! Whatever was still on the balances is forfeited through a closing ledger
//! entry, so the ledger keeps summing to zero. Open payout requests are
//! cancelled and their held coins forfeited too: there is nobody left to pay.
//!
//! Postings keep their account names (`user:<id>:coins`), so staff can still
//! audit the closed account by its ID. That ID now belongs to an anonymous
//! tombstone and says nothing about who the person was.
use tokuly_core::prelude::*;

use crate::{Wallet, ledger_tx};

impl Wallet {
    /// Removes or unlinks everything the wallet holds about `user_id`.
    /// One transaction; repeating it finds nothing left to do.
    pub(crate) async fn on_user_deleted(&self, user_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;

        // Lock order matters here (see `ledger_tx`): payout rows, then ledger
        // entry rows, and account rows last.
        let open_payouts: Vec<(Uuid, Uuid)> =
            sqlx::query_as("SELECT id, hold_entry_id FROM wallet_payout_requests WHERE user_id = $1 AND status IN ('pending', 'approved') ORDER BY id FOR UPDATE")
                .bind(user_id)
                .fetch_all(&mut *tx)
                .await?;
        ledger_tx::unlink_user(&mut tx, user_id).await?;
        // With the holds unlinked, releasing them forfeits their coins
        // instead of crediting an account that is about to disappear.
        for (payout_id, hold_entry_id) in open_payouts {
            ledger_tx::release_payout(&mut tx, hold_entry_id, "payout.cancelled").await?;
            sqlx::query("UPDATE wallet_payout_requests SET status = 'cancelled', review_note = 'account deleted' WHERE id = $1").bind(payout_id).execute(&mut *tx).await?;
        }
        sqlx::query("UPDATE wallet_payout_requests SET user_id = NULL, destination = NULL, note = '' WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;

        // An unpaid checkout of a deleted account must not complete later.
        sqlx::query("UPDATE wallet_purchases SET status = 'cancelled' WHERE user_id = $1 AND status = 'pending'").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE wallet_purchases SET user_id = NULL WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE wallet_adjustments SET user_id = NULL WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;

        sqlx::query("DELETE FROM wallet_daily_bonuses WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM wallet_entry_notices WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;

        let closing_entry = ledger_tx::close_account(&mut tx, user_id).await?;
        tx.commit().await?;
        tracing::info!(%user_id, forfeited = closing_entry.is_some(), "wallet of a deleted user closed");
        Ok(())
    }
}
