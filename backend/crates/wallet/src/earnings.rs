//! Creator earnings: what a user received from others, by reason and over time.
//!
//! Earnings come from transfers (`kind = 'transfer'`, the user on the
//! receiving side). A transfer that was refunded later is left out of every
//! number here, so the sums are what the creator actually kept.
use chrono::Duration;
use chrono_tz::Tz;
use tokuly_core::prelude::*;
use tokuly_proto::wallet::{EarningsBucket, EarningsByReason, EarningsPoint, EarningsTotals, GetEarningsSummaryResponse, PayoutTerms};

use crate::{Wallet, describe};

/// Most buckets a time series may have; keeps one request from asking for
/// ten years of daily points.
const MAX_BUCKETS: i64 = 400;
const DEFAULT_PERIOD: Duration = Duration::days(30);

/// Width of a time-series bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bucket {
    Day,
    Week,
    Month,
}

impl Bucket {
    fn from_proto(bucket: Option<EarningsBucket>) -> Self {
        match bucket {
            Some(EarningsBucket::EARNINGS_BUCKET_WEEK) => Self::Week,
            Some(EarningsBucket::EARNINGS_BUCKET_MONTH) => Self::Month,
            _ => Self::Day,
        }
    }

    /// The `date_trunc` field for this width.
    fn unit(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
        }
    }

    /// Shortest length a bucket of this width can have, in days.
    fn min_days(self) -> i64 {
        match self {
            Self::Day => 1,
            Self::Week => 7,
            Self::Month => 28,
        }
    }
}

/// A validated reporting period: `[from, to)` cut into buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Period {
    pub from: Time,
    pub to: Time,
    pub bucket: Bucket,
}

/// Fills in the defaults (the last 30 days, daily) and rejects periods that
/// are empty or would produce too many buckets.
pub(crate) fn resolve_period(from: Option<Time>, to: Option<Time>, bucket: Option<EarningsBucket>, now: Time) -> AppResult<Period> {
    let bucket = Bucket::from_proto(bucket);
    let to = to.unwrap_or(now);
    let from = from.unwrap_or(to - DEFAULT_PERIOD);
    ensure!(to > from, invalid, "PERIOD_INVALID", "the period must end after it starts");
    // +1: a period that starts mid-bucket touches one more bucket than its length suggests.
    let buckets = (to - from).num_days() / bucket.min_days() + 1;
    ensure!(buckets <= MAX_BUCKETS, invalid, "PERIOD_TOO_LONG", "choose a shorter period or wider buckets (at most {MAX_BUCKETS} points)");
    Ok(Period { from, to, bucket })
}

fn totals(gross: i64, fee: i64, count: i64) -> EarningsTotals {
    EarningsTotals { gross, fee, net: gross - fee, count: i32::try_from(count).unwrap_or(i32::MAX), ..Default::default() }
}

impl Wallet {
    /// The payout rules of this server, as clients show them.
    pub(crate) fn payout_terms(&self) -> PayoutTerms {
        PayoutTerms {
            min_coins: self.cfg.payout_min_coins,
            currency: self.cfg.payout_currency.clone(),
            minor_per_100_coins: self.cfg.payout_minor_per_100_coins,
            platform_fee_percent: self.cfg.platform_fee_percent as i32,
            ..Default::default()
        }
    }

    /// Net earnings `user_id` ever received (refunded transfers excluded).
    pub(crate) async fn lifetime_earned(&self, user_id: Uuid) -> AppResult<i64> {
        Ok(sqlx::query_scalar("SELECT COALESCE(sum(amount - fee), 0)::bigint FROM wallet_entries WHERE to_user_id = $1 AND kind = 'transfer' AND refunded_at IS NULL")
            .bind(user_id)
            .fetch_one(&self.core.db)
            .await?)
    }

    /// Everything `EarningsService.GetEarningsSummary` returns.
    pub(crate) async fn earnings_summary(&self, user_id: Uuid, period: Period, time_zone: Tz, locale: &str) -> AppResult<GetEarningsSummaryResponse> {
        let balance = crate::ledger::balance(&self.core, user_id).await?;
        let (pending_payout, lifetime_paid_out) = self.payout_totals(user_id).await?;
        let by_reason = self.earnings_by_reason(user_id, period, locale).await?;
        let period_totals = by_reason.iter().filter_map(|r| r.totals.as_option()).fold((0, 0, 0), |(gross, fee, count), t| (gross + t.gross, fee + t.fee, count + i64::from(t.count)));
        Ok(GetEarningsSummaryResponse {
            available: balance.earnings,
            pending_payout,
            lifetime_earned: self.lifetime_earned(user_id).await?,
            lifetime_paid_out,
            period: totals(period_totals.0, period_totals.1, period_totals.2).into(),
            by_reason,
            series: self.earnings_series(user_id, period, time_zone).await?,
            payout_terms: self.payout_terms().into(),
            ..Default::default()
        })
    }

    /// The period split by ledger reason, biggest earner first.
    async fn earnings_by_reason(&self, user_id: Uuid, period: Period, locale: &str) -> AppResult<Vec<EarningsByReason>> {
        let rows: Vec<(String, i64, i64, i64)> = sqlx::query_as(
            "SELECT reason, sum(amount)::bigint, sum(fee)::bigint, count(*)
             FROM wallet_entries
             WHERE to_user_id = $1 AND kind = 'transfer' AND refunded_at IS NULL AND created_at >= $2 AND created_at < $3
             GROUP BY reason
             ORDER BY sum(amount - fee) DESC, reason",
        )
        .bind(user_id)
        .bind(period.from)
        .bind(period.to)
        .fetch_all(&self.core.db)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(reason, gross, fee, count)| EarningsByReason {
                label: describe::earnings_label(&reason).pick(locale).to_owned(),
                reason,
                totals: totals(gross, fee, count).into(),
                ..Default::default()
            })
            .collect())
    }

    /// The period as a time series. Buckets are calendar days, weeks or
    /// months in the creator's time zone; empty buckets are returned as
    /// zeros so charts need no gap filling.
    async fn earnings_series(&self, user_id: Uuid, period: Period, time_zone: Tz) -> AppResult<Vec<EarningsPoint>> {
        // Sum per bucket first, then lay the sums over the full run of
        // buckets: joining raw entries to buckets would sort every entry.
        let rows: Vec<(Time, i64, i64, i64)> = sqlx::query_as(
            "WITH earned AS (
                 SELECT date_trunc($4::text, created_at, $5::text) AS start_at, sum(amount) AS gross, sum(fee) AS fee, count(*) AS transfers
                 FROM wallet_entries
                 WHERE to_user_id = $1 AND kind = 'transfer' AND refunded_at IS NULL AND created_at >= $2 AND created_at < $3
                 GROUP BY 1
             )
             SELECT b.start_at, COALESCE(earned.gross, 0)::bigint, COALESCE(earned.fee, 0)::bigint, COALESCE(earned.transfers, 0)
             FROM generate_series(date_trunc($4::text, $2::timestamptz, $5::text), $3::timestamptz - interval '1 microsecond',
                                  ('1 ' || $4::text)::interval, $5::text) AS b (start_at)
             LEFT JOIN earned ON earned.start_at = b.start_at
             ORDER BY b.start_at",
        )
        .bind(user_id)
        .bind(period.from)
        .bind(period.to)
        .bind(period.bucket.unit())
        .bind(time_zone.name())
        .fetch_all(&self.core.db)
        .await?;
        Ok(rows.into_iter().map(|(start_at, gross, fee, count)| EarningsPoint { period_start: ts(start_at), totals: totals(gross, fee, count).into(), ..Default::default() }).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Time {
        text.parse().unwrap()
    }

    #[test]
    fn the_default_period_is_the_last_thirty_days_by_day() {
        let now = at("2026-03-31T12:00:00Z");
        let period = resolve_period(None, None, None, now).unwrap();
        assert_eq!((period.from, period.to, period.bucket), (at("2026-03-01T12:00:00Z"), now, Bucket::Day));
        let explicit_end = resolve_period(None, Some(at("2026-02-01T00:00:00Z")), Some(EarningsBucket::EARNINGS_BUCKET_WEEK), now).unwrap();
        assert_eq!((explicit_end.from, explicit_end.bucket), (at("2026-01-02T00:00:00Z"), Bucket::Week));
    }

    #[test]
    fn empty_and_oversized_periods_are_rejected() {
        let now = at("2026-03-31T12:00:00Z");
        let start = at("2026-03-01T00:00:00Z");
        assert_eq!(resolve_period(Some(now), Some(start), None, now).unwrap_err().reason, "PERIOD_INVALID");
        assert_eq!(resolve_period(Some(now), Some(now), None, now).unwrap_err().reason, "PERIOD_INVALID");
        let two_years_ago = at("2024-03-31T12:00:00Z");
        assert_eq!(resolve_period(Some(two_years_ago), Some(now), Some(EarningsBucket::EARNINGS_BUCKET_DAY), now).unwrap_err().reason, "PERIOD_TOO_LONG");
        assert!(resolve_period(Some(two_years_ago), Some(now), Some(EarningsBucket::EARNINGS_BUCKET_WEEK), now).is_ok(), "wider buckets fit");
        assert!(resolve_period(Some(at("2000-01-01T00:00:00Z")), Some(now), Some(EarningsBucket::EARNINGS_BUCKET_MONTH), now).is_ok());
    }

    #[test]
    fn totals_derive_net_from_gross_and_fee() {
        let t = totals(1000, 300, 4);
        assert_eq!((t.gross, t.fee, t.net, t.count), (1000, 300, 700, 4));
    }
}
