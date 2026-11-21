//! The daily login bonus.
//!
//! One claim per calendar day *in the user's own time zone*; claiming on
//! consecutive days grows a streak, and the reward follows a ladder indexed
//! by the streak (configured with `TOKULY_WALLET_DAILY_BONUS_COINS`).
//!
//! The stored `last_claimed_on` date only ever moves forward. That single
//! rule is what makes time zone changes harmless: after claiming for a day,
//! no zone you switch to can make that day (or an earlier one) claimable
//! again.
use chrono::{NaiveDate, NaiveTime, TimeZone as _, Utc};
use chrono_tz::Tz;
use tokuly_core::{analytics::AnalyticsEvent, prelude::*};
use tokuly_proto::wallet::DailyBonus;

use crate::{Wallet, ledger_tx};

/// A user's streak state (`wallet_daily_bonuses`); the default is "never claimed".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, sqlx::FromRow)]
struct BonusRow {
    last_claimed_on: Option<NaiveDate>,
    /// Consecutive days claimed up to `last_claimed_on`.
    streak: i32,
    longest_streak: i32,
    total_claims: i32,
}

const BONUS_COLUMNS: &str = "last_claimed_on, streak, longest_streak, total_claims";

/// The calendar day it is for someone living in `tz`.
fn local_date(at: Time, tz: Tz) -> NaiveDate {
    at.with_timezone(&tz).date_naive()
}

/// The first instant of `date` in `tz`. Where clocks jump forward over
/// midnight the day starts at the first wall-clock time that exists.
fn start_of_day(date: NaiveDate, tz: Tz) -> Time {
    let midnight = date.and_time(NaiveTime::MIN);
    (0..24 * 4)
        .map(|quarter| midnight + chrono::Duration::minutes(15 * quarter))
        .find_map(|wall_clock| tz.from_local_datetime(&wall_clock).earliest())
        .map(|start| start.with_timezone(&Utc))
        .unwrap_or_else(|| Utc.from_utc_datetime(&midnight))
}

/// Whether a claim is allowed on `today`: only for a day after the last claim.
fn can_claim(last_claimed_on: Option<NaiveDate>, today: NaiveDate) -> bool {
    last_claimed_on.is_none_or(|last| today > last)
}

/// The streak a claim on `today` reaches: one more when yesterday was
/// claimed, otherwise it starts over at 1.
fn streak_after_claim(last_claimed_on: Option<NaiveDate>, streak: i32, today: NaiveDate) -> i32 {
    match last_claimed_on {
        Some(last) if last.succ_opt() == Some(today) => streak.saturating_add(1),
        _ => 1,
    }
}

/// The streak that is still alive on `today`: unbroken as long as the last
/// claim was today or yesterday, otherwise gone.
fn live_streak(last_claimed_on: Option<NaiveDate>, streak: i32, today: NaiveDate) -> i32 {
    match last_claimed_on {
        Some(last) if last >= today || last.succ_opt() == Some(today) => streak,
        _ => 0,
    }
}

/// The step of the reward ladder for the `streak`-th consecutive day, as
/// (1-based position, coins). The ladder starts over after its last step.
/// `None` when the bonus is disabled (empty ladder).
fn reward(cycle: &[i64], streak: i32) -> Option<(i32, i64)> {
    if cycle.is_empty() {
        return None;
    }
    let index = (streak.max(1) - 1) as usize % cycle.len();
    Some((index as i32 + 1, cycle[index]))
}

/// The bonus as the API reports it.
fn status(row: &BonusRow, today: NaiveDate, tz: Tz, cycle: &[i64]) -> DailyBonus {
    let enabled = !cycle.is_empty();
    let claimable = enabled && can_claim(row.last_claimed_on, today);
    let streak = live_streak(row.last_claimed_on, row.streak, today);
    // What the next claim reaches: today's if it is still open, otherwise the
    // one that unlocks the day after the last claim.
    let next_streak = if claimable { streak_after_claim(row.last_claimed_on, row.streak, today) } else { streak.saturating_add(1) };
    let (next_cycle_day, next_coins) = reward(cycle, next_streak).unwrap_or((0, 0));
    let next_claim_at = match row.last_claimed_on {
        Some(last) if enabled && !claimable => last.succ_opt().map(|next_day| start_of_day(next_day, tz)),
        _ => None,
    };
    DailyBonus {
        claimable,
        streak,
        next_coins,
        next_cycle_day,
        next_claim_at: opt_ts(next_claim_at),
        cycle: cycle.to_vec(),
        longest_streak: row.longest_streak,
        total_claims: row.total_claims,
        ..Default::default()
    }
}

impl Wallet {
    /// Where `user_id` stands with the daily bonus right now.
    pub(crate) async fn daily_bonus(&self, user_id: Uuid) -> AppResult<DailyBonus> {
        let tz = self.user_prefs(user_id).await?.time_zone;
        let row: Option<BonusRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {BONUS_COLUMNS} FROM wallet_daily_bonuses WHERE user_id = $1")))
            .bind(user_id)
            .fetch_optional(&self.core.db)
            .await?;
        Ok(status(&row.unwrap_or_default(), local_date(now(), tz), tz, &self.daily_cycle))
    }

    /// Claims today's bonus: grants the coins and advances the streak in one
    /// transaction. Returns the coins granted and the state afterwards.
    pub(crate) async fn claim_bonus(&self, user_id: Uuid) -> AppResult<(i64, DailyBonus)> {
        ensure!(!self.daily_cycle.is_empty(), precondition, "DAILY_BONUS_DISABLED", "the daily bonus is not available on this server");
        let tz = self.user_prefs(user_id).await?.time_zone;
        let mut tx = self.core.db.begin().await?;
        sqlx::query("INSERT INTO wallet_daily_bonuses (user_id) VALUES ($1) ON CONFLICT DO NOTHING").bind(user_id).execute(&mut *tx).await?;
        let row: BonusRow = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {BONUS_COLUMNS} FROM wallet_daily_bonuses WHERE user_id = $1 FOR UPDATE")))
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
        // "Today" is read after the row lock is ours, so of two racing claims
        // the second one sees the first one's date.
        let today = local_date(now(), tz);
        ensure!(can_claim(row.last_claimed_on, today), precondition, "DAILY_BONUS_ALREADY_CLAIMED", "today's bonus was already claimed");
        let streak = streak_after_claim(row.last_claimed_on, row.streak, today);
        let (_, coins) = reward(&self.daily_cycle, streak).ok_or_else(|| AppError::precondition("DAILY_BONUS_DISABLED", "the daily bonus is not available on this server"))?;
        let memo = serde_json::json!({ "date": today.to_string(), "streak": streak });
        let granted = ledger_tx::grant(&mut tx, user_id, coins, "daily_bonus", &format!("daily_bonus:{user_id}:{today}"), &memo).await?;
        // The key names the day, so a grant that already exists means the day was paid.
        ensure!(granted.applied, precondition, "DAILY_BONUS_ALREADY_CLAIMED", "today's bonus was already claimed");
        let row: BonusRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE wallet_daily_bonuses
             SET last_claimed_on = $2, streak = $3, longest_streak = GREATEST(longest_streak, $3),
                 total_claims = total_claims + 1, total_coins = total_coins + $4, updated_at = now()
             WHERE user_id = $1 RETURNING {BONUS_COLUMNS}"
        )))
        .bind(user_id)
        .bind(today)
        .bind(streak)
        .bind(coins)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        self.core.analytics.track(AnalyticsEvent::new("wallet.daily_bonus").user(Some(user_id)).value(coins as f64).props(serde_json::json!({ "streak": streak }))).await;
        Ok((coins, status(&row, today, tz, &self.daily_cycle)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    fn at(text: &str) -> Time {
        text.parse().unwrap()
    }

    const LADDER: &[i64] = &[10, 10, 15, 50];

    #[test]
    fn the_day_follows_the_users_time_zone() {
        let instant = at("2026-03-10T15:30:00Z");
        assert_eq!(local_date(instant, chrono_tz::Asia::Tokyo), day("2026-03-11"), "already tomorrow in Tokyo");
        assert_eq!(local_date(instant, chrono_tz::America::Los_Angeles), day("2026-03-10"));
        assert_eq!(local_date(instant, chrono_tz::UTC), day("2026-03-10"));
    }

    #[test]
    fn days_start_at_local_midnight() {
        assert_eq!(start_of_day(day("2026-03-11"), chrono_tz::Asia::Tokyo), at("2026-03-10T15:00:00Z"));
        assert_eq!(start_of_day(day("2026-07-01"), chrono_tz::Europe::Paris), at("2026-06-30T22:00:00Z"));
        // Clocks in São Paulo jumped from 00:00 to 01:00 on 2018-11-04: that day began at 01:00 local.
        assert_eq!(start_of_day(day("2018-11-04"), chrono_tz::America::Sao_Paulo), at("2018-11-04T03:00:00Z"));
    }

    #[test]
    fn one_claim_per_day_and_never_for_a_past_day() {
        let today = day("2026-03-11");
        assert!(can_claim(None, today));
        assert!(can_claim(Some(day("2026-03-10")), today));
        assert!(!can_claim(Some(today), today), "already claimed today");
        assert!(!can_claim(Some(day("2026-03-12")), today), "moving to an earlier time zone does not reopen a day");
    }

    #[test]
    fn streak_grows_on_consecutive_days_and_resets_after_a_gap() {
        let today = day("2026-03-11");
        assert_eq!(streak_after_claim(None, 0, today), 1);
        assert_eq!(streak_after_claim(Some(day("2026-03-10")), 4, today), 5);
        assert_eq!(streak_after_claim(Some(day("2026-03-09")), 4, today), 1, "a missed day starts over");
        assert_eq!(streak_after_claim(Some(day("2026-02-28")), 2, day("2026-03-01")), 3, "month boundaries are consecutive");
    }

    #[test]
    fn a_streak_stays_alive_until_a_day_is_missed() {
        let today = day("2026-03-11");
        assert_eq!(live_streak(Some(today), 5, today), 5);
        assert_eq!(live_streak(Some(day("2026-03-10")), 5, today), 5, "yesterday's claim: still time to continue");
        assert_eq!(live_streak(Some(day("2026-03-09")), 5, today), 0);
        assert_eq!(live_streak(None, 0, today), 0);
    }

    #[test]
    fn the_reward_ladder_starts_over() {
        assert_eq!(reward(LADDER, 1), Some((1, 10)));
        assert_eq!(reward(LADDER, 4), Some((4, 50)));
        assert_eq!(reward(LADDER, 5), Some((1, 10)), "day 5 is day 1 of the next round");
        assert_eq!(reward(LADDER, 8), Some((4, 50)));
        assert_eq!(reward(LADDER, 0), Some((1, 10)));
        assert_eq!(reward(&[], 3), None);
    }

    #[test]
    fn status_before_the_first_claim() {
        let bonus = status(&BonusRow::default(), day("2026-03-11"), chrono_tz::Asia::Tokyo, LADDER);
        assert!(bonus.claimable);
        assert_eq!((bonus.streak, bonus.next_coins, bonus.next_cycle_day), (0, 10, 1));
        assert!(bonus.next_claim_at.as_option().is_none());
        assert_eq!(bonus.cycle, LADDER);
    }

    #[test]
    fn status_after_claiming_today_points_at_tomorrow() {
        let row = BonusRow { last_claimed_on: Some(day("2026-03-11")), streak: 2, longest_streak: 6, total_claims: 9 };
        let bonus = status(&row, day("2026-03-11"), chrono_tz::Asia::Tokyo, LADDER);
        assert!(!bonus.claimable);
        assert_eq!((bonus.streak, bonus.next_coins, bonus.next_cycle_day), (2, 15, 3));
        assert_eq!(from_ts(bonus.next_claim_at.as_option().unwrap()), at("2026-03-11T15:00:00Z"), "midnight of the 12th in Tokyo");
        assert_eq!((bonus.longest_streak, bonus.total_claims), (6, 9));
    }

    #[test]
    fn status_with_a_live_and_a_broken_streak() {
        let row = BonusRow { last_claimed_on: Some(day("2026-03-10")), streak: 3, longest_streak: 3, total_claims: 3 };
        let alive = status(&row, day("2026-03-11"), chrono_tz::UTC, LADDER);
        assert!(alive.claimable);
        assert_eq!((alive.streak, alive.next_coins, alive.next_cycle_day), (3, 50, 4));
        let broken = status(&row, day("2026-03-13"), chrono_tz::UTC, LADDER);
        assert!(broken.claimable);
        assert_eq!((broken.streak, broken.next_coins, broken.next_cycle_day), (0, 10, 1));
    }

    #[test]
    fn a_disabled_bonus_is_never_claimable() {
        let bonus = status(&BonusRow::default(), day("2026-03-11"), chrono_tz::UTC, &[]);
        assert!(!bonus.claimable && bonus.cycle.is_empty());
        assert_eq!(bonus.next_coins, 0);
        assert!(bonus.next_claim_at.as_option().is_none());
    }
}
