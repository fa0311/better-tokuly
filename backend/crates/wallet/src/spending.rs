//! The monthly spending limit.
//!
//! Under 18 ([`tokuly_identity::age::is_minor`], asked on the day of each
//! charge) an account spends at most a ceiling a calendar month
//! (`TOKULY_WALLET_MINOR_MONTHLY_LIMIT_YEN`). Anyone may choose a limit of
//! their own (`wallet_accounts.monthly_limit_yen`), under 18 only up to the
//! ceiling; it stays when they turn 18. The lower of the two is in force.
//!
//! Spending, per calendar month in Japan time:
//! - coin purchases at their price ([`purchase_yen`]): in the month they
//!   were paid, and while they wait for the payment whenever they were
//!   started, so a purchase started before the month turned and paid after
//!   it uses the new month's room;
//! - `transfer` entries the account paid (gifts, stamp packs of creators),
//!   unless refunded. There is no exchange rate for coins: one coin is ¥1.
//!
//! Grants (purchased coins arriving, the daily bonus, rewards), spends on
//! what the platform sells and everything the account receives are not
//! spending.
//!
//! A purchase start and a transfer check the limit on the transaction that
//! holds the payer's account row ([`crate::postings::lock_account`]), the
//! lock every movement of that account takes first: two charges at once are
//! counted one after the other. A purchase that was paid is never refused
//! when it completes; it was counted when it started.
use chrono::{Datelike, Months, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Asia::Tokyo;
use sqlx::{PgExecutor, Postgres, Transaction};
use tokuly_core::prelude::*;
use tokuly_identity::age;
use tokuly_proto::wallet::SpendingLimit;

use crate::{Wallet, WalletConfig};

/// Largest limit anyone may choose, in yen.
const MAX_OWN_LIMIT_YEN: i64 = 100_000_000;

/// The yen a purchase counts for: its price when it is in yen, otherwise its
/// coins (not the bonus), one coin as ¥1. [`spent_since`] says the same in SQL.
pub(crate) fn purchase_yen(currency: &str, price_minor: i64, coins: i64) -> i64 {
    if currency == "JPY" { price_minor } else { coins }
}

/// The calendar month in Japan time that `at` falls in: its first instant
/// and the first instant of the next month.
pub(crate) fn month_of(at: Time) -> (Time, Time) {
    let first = at.with_timezone(&Tokyo).date_naive().with_day(1).expect("every month has a first day");
    let next = first.checked_add_months(Months::new(1)).expect("a date far from the end of time");
    (midnight_in_japan(first), midnight_in_japan(next))
}

fn midnight_in_japan(day: NaiveDate) -> Time {
    Tokyo.from_local_datetime(&day.and_time(NaiveTime::MIN)).single().expect("Japan has no daylight saving time").with_timezone(&Utc)
}

/// The smallest UUIDv7 of the millisecond `at`. Entries have UUIDv7 keys,
/// so `id >= ` this lets the per-user index scan stop at the start of the
/// month instead of reading the whole history.
fn first_id_at(at: Time) -> Uuid {
    Uuid::from_u128((at.timestamp_millis().max(0) as u128) << 80)
}

/// Yen `user_id` has spent from `since` on, counting the purchases still
/// waiting for their payment.
pub(crate) async fn spent_since(db: impl PgExecutor<'_>, user_id: Uuid, since: Time) -> AppResult<i64> {
    // Purchases are not bounded by ID: one paid now may have started long
    // before (a payment the provider reported late).
    Ok(sqlx::query_scalar(
        "SELECT ((SELECT COALESCE(sum(CASE WHEN currency = 'JPY' THEN price_minor ELSE coins END), 0) FROM wallet_purchases
                  WHERE user_id = $1 AND (status = 'pending' OR (status = 'completed' AND completed_at >= $2)))
               + (SELECT COALESCE(sum(amount), 0) FROM wallet_entries
                  WHERE from_user_id = $1 AND id >= $3 AND created_at >= $2 AND kind = 'transfer' AND refunded_at IS NULL))::bigint",
    )
    .bind(user_id)
    .bind(since)
    .bind(first_id_at(since))
    .fetch_one(db)
    .await?)
}

/// The ceiling as this server sets it. Read from the live settings, because
/// the ledger's callers hand over nothing but `Core`; the default when no
/// settings are registered (tests).
fn minor_ceiling(core: &Core) -> i64 {
    core.settings.current::<WalletConfig>("wallet").map_or_else(|| WalletConfig::default().minor_monthly_limit_yen, |cfg| cfg.minor_monthly_limit_yen)
}

/// The ceiling that applies to `user_id` today: `None` from 18 on. Asks
/// identity on the pool, so call it before taking a lock.
pub(crate) async fn ceiling_for(core: &Core, user_id: Uuid) -> AppResult<Option<i64>> {
    Ok(age::is_minor(core, user_id).await?.then(|| minor_ceiling(core)))
}

/// The limit in force: the lower of the ceiling and the account's own.
fn in_force(ceiling: Option<i64>, own: Option<i64>) -> Option<i64> {
    match (ceiling, own) {
        (Some(ceiling), Some(own)) => Some(ceiling.min(own)),
        (ceiling, own) => ceiling.or(own),
    }
}

async fn own_limit(db: impl PgExecutor<'_>, user_id: Uuid) -> AppResult<Option<i64>> {
    let own: Option<Option<i64>> = sqlx::query_scalar("SELECT monthly_limit_yen FROM wallet_accounts WHERE user_id = $1").bind(user_id).fetch_optional(db).await?;
    Ok(own.flatten())
}

/// Refuses `yen` more spending by `user_id` this month with
/// `SPENDING_LIMIT_REACHED` when it would go past the limit in force. Call
/// it on the transaction holding the account row, with [`ceiling_for`].
pub(crate) async fn check(tx: &mut Transaction<'_, Postgres>, user_id: Uuid, ceiling: Option<i64>, yen: i64) -> AppResult<()> {
    let Some(limit) = in_force(ceiling, own_limit(&mut **tx, user_id).await?) else {
        return Ok(());
    };
    let spent = spent_since(&mut **tx, user_id, month_of(now()).0).await?;
    ensure!(spent + yen <= limit, precondition, "SPENDING_LIMIT_REACHED", "this would go past your monthly spending limit");
    Ok(())
}

impl Wallet {
    /// `user_id`'s limits and this month's spending. See `WalletService.GetSpendingLimit`.
    pub(crate) async fn spending_limit(&self, user_id: Uuid) -> AppResult<SpendingLimit> {
        let ceiling = ceiling_for(&self.core, user_id).await?;
        let own = own_limit(&self.core.db, user_id).await?;
        let (month, next_month) = month_of(now());
        let spent = spent_since(&self.core.db, user_id, month).await?;
        let limit = in_force(ceiling, own);
        Ok(SpendingLimit {
            ceiling_yen: ceiling,
            own_limit_yen: own,
            limit_yen: limit,
            spent_yen: spent,
            remaining_yen: limit.map(|limit| (limit - spent).max(0)),
            resets_at: ts(next_month),
            ..Default::default()
        })
    }

    /// Sets (`Some`) or clears (`None`) `user_id`'s own limit. Under 18 it
    /// stays at or below the ceiling and cannot be cleared. See
    /// `WalletService.SetSpendingLimit`.
    pub(crate) async fn set_own_limit(&self, user_id: Uuid, yen: Option<i64>) -> AppResult<SpendingLimit> {
        ensure!(
            yen.is_none_or(|yen| (0..=MAX_OWN_LIMIT_YEN).contains(&yen)),
            invalid,
            "SPENDING_LIMIT_INVALID",
            "a spending limit is 0 to 100,000,000 yen"
        );
        if let Some(ceiling) = ceiling_for(&self.core, user_id).await? {
            ensure!(yen.is_some_and(|yen| yen <= ceiling), invalid, "SPENDING_LIMIT_RESTRICTED", "under 18, the monthly limit stays at ¥{ceiling} or below");
        }
        sqlx::query("INSERT INTO wallet_accounts (user_id, monthly_limit_yen) VALUES ($1, $2) ON CONFLICT (user_id) DO UPDATE SET monthly_limit_yen = EXCLUDED.monthly_limit_yen")
            .bind(user_id)
            .bind(yen)
            .execute(&self.core.db)
            .await?;
        self.spending_limit(user_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(rfc3339: &str) -> Time {
        chrono::DateTime::parse_from_rfc3339(rfc3339).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn months_turn_at_midnight_in_japan() {
        // 2026-11-30 23:30 JST is still November; 15:00 UTC on the 30th is midnight of December 1st there.
        assert_eq!(month_of(at("2026-11-30T14:30:00Z")), (at("2026-10-31T15:00:00Z"), at("2026-11-30T15:00:00Z")));
        assert_eq!(month_of(at("2026-11-30T15:00:00Z")), (at("2026-11-30T15:00:00Z"), at("2026-12-31T15:00:00Z")));
        assert_eq!(month_of(at("2026-12-31T16:00:00Z")).0, at("2026-12-31T15:00:00Z"), "January");
    }

    #[test]
    fn the_lower_limit_is_in_force() {
        assert_eq!(in_force(None, None), None);
        assert_eq!(in_force(Some(5000), None), Some(5000));
        assert_eq!(in_force(None, Some(8000)), Some(8000));
        assert_eq!(in_force(Some(5000), Some(1200)), Some(1200));
        assert_eq!(in_force(Some(3000), Some(4000)), Some(3000), "a ceiling lowered since wins");
    }

    #[test]
    fn a_purchase_counts_its_price_in_yen() {
        assert_eq!(purchase_yen("JPY", 1000, 1000), 1000);
        assert_eq!(purchase_yen("USD", 499, 500), 500);
    }

    #[test]
    fn ids_of_a_month_start_at_its_first_millisecond() {
        let start = at("2026-11-30T15:00:00Z");
        let ms = start.timestamp_millis() as u128;
        // A UUIDv7 of that millisecond (version 7, variant 10, random bits) and the last one before it.
        let in_month = Uuid::from_u128(ms << 80 | 0x7 << 76 | 0x8 << 60 | 0x1234_5678);
        let before = Uuid::from_u128((ms - 1) << 80 | u128::MAX >> 48);
        assert!(before < first_id_at(start) && first_id_at(start) <= in_month);
    }
}
