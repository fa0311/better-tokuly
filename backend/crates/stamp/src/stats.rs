//! Numbers: the per-day pack statistics, what creators see of them, and the
//! popularity ranking computed from them.
use chrono::NaiveDate;
use sqlx::{Postgres, Transaction};
use tokuly_core::prelude::*;
use tokuly_proto::stamp as pb;

use crate::{
    access::Viewer,
    model::{PackRow, StampView, stamp_view_select},
    usage::STATS_TIME_ZONE,
};

/// Adds to today's numbers of a pack. (Sends are counted by
/// [`record_send`](crate::usage::record_send) in its own statement.)
pub async fn bump_daily(
    tx: &mut Transaction<'_, Postgres>,
    pack_id: Uuid,
    installs: i64,
    purchases: i64,
    earnings: i64,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO stamp_pack_daily (pack_id, day, installs, purchases, earnings)
         VALUES ($1, (now() AT TIME ZONE $5)::date, $2, $3, $4)
         ON CONFLICT (pack_id, day) DO UPDATE SET
             installs  = stamp_pack_daily.installs + EXCLUDED.installs,
             purchases = stamp_pack_daily.purchases + EXCLUDED.purchases,
             earnings  = stamp_pack_daily.earnings + EXCLUDED.earnings",
    )
    .bind(pack_id)
    .bind(installs)
    .bind(purchases)
    .bind(earnings)
    .bind(STATS_TIME_ZONE)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Takes one install back out of today's numbers of a pack (never below zero).
pub async fn retract_daily_install(tx: &mut Transaction<'_, Postgres>, pack_id: Uuid) -> AppResult<()> {
    sqlx::query("UPDATE stamp_pack_daily SET installs = GREATEST(installs - 1, 0) WHERE pack_id = $1 AND day = (now() AT TIME ZONE $2)::date")
        .bind(pack_id)
        .bind(STATS_TIME_ZONE)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct DailyRow {
    day: NaiveDate,
    sends: i64,
    installs: i64,
    purchases: i64,
    earnings: i64,
}

#[derive(sqlx::FromRow)]
struct TotalsRow {
    sends: i64,
    earnings: i64,
    chat_sends: i64,
    live_chat_sends: i64,
    comment_sends: i64,
    note_sends: i64,
    map_sends: i64,
}

/// Statistics of one pack for its creator: all-time totals, the last `days`
/// days one by one (days without activity included) and the most sent stamps.
pub async fn pack_stats(
    core: &Core,
    viewer: &Viewer,
    pack: &PackRow,
    days: i32,
) -> AppResult<pb::GetPackStatsResponse> {
    let totals: TotalsRow = sqlx::query_as(
        "SELECT COALESCE(sum(sends), 0)::bigint AS sends, COALESCE(sum(earnings), 0)::bigint AS earnings,
                COALESCE(sum(chat_sends), 0)::bigint AS chat_sends, COALESCE(sum(live_chat_sends), 0)::bigint AS live_chat_sends,
                COALESCE(sum(comment_sends), 0)::bigint AS comment_sends, COALESCE(sum(note_sends), 0)::bigint AS note_sends,
                COALESCE(sum(map_sends), 0)::bigint AS map_sends
         FROM stamp_pack_daily WHERE pack_id = $1",
    )
    .bind(pack.id)
    .fetch_one(&core.db)
    .await?;

    let daily: Vec<DailyRow> = sqlx::query_as(
        "SELECT calendar.day,
                COALESCE(d.sends, 0) AS sends, COALESCE(d.installs, 0) AS installs,
                COALESCE(d.purchases, 0) AS purchases, COALESCE(d.earnings, 0) AS earnings
         FROM (SELECT (now() AT TIME ZONE $3)::date - days_ago AS day FROM generate_series(0, $2::int - 1) AS days_ago) calendar
         LEFT JOIN stamp_pack_daily d ON d.pack_id = $1 AND d.day = calendar.day
         ORDER BY calendar.day",
    )
    .bind(pack.id)
    .bind(days)
    .bind(STATS_TIME_ZONE)
    .fetch_all(&core.db)
    .await?;

    let top: Vec<StampView> = sqlx::query_as(concat!(
        stamp_view_select!(),
        "WHERE s.pack_id = $2 AND s.send_count > 0 ORDER BY s.send_count DESC, s.position, s.id LIMIT 10"
    ))
    .bind(viewer.user_id)
    .bind(pack.id)
    .fetch_all(&core.db)
    .await?;

    let contexts = [
        (pb::UsageContext::USAGE_CONTEXT_CHAT, totals.chat_sends),
        (pb::UsageContext::USAGE_CONTEXT_LIVE_CHAT, totals.live_chat_sends),
        (pb::UsageContext::USAGE_CONTEXT_COMMENT, totals.comment_sends),
        (pb::UsageContext::USAGE_CONTEXT_NOTE, totals.note_sends),
        (pb::UsageContext::USAGE_CONTEXT_MAP, totals.map_sends),
    ];
    Ok(pb::GetPackStatsResponse {
        install_count: pack.install_count,
        send_count: totals.sends,
        sale_count: pack.purchase_count,
        earnings: totals.earnings,
        like_count: pack.like_count,
        daily: daily
            .into_iter()
            .map(|d| pb::DailyPackStats {
                date: d.day.to_string(),
                sends: d.sends,
                installs: d.installs,
                sales: d.purchases,
                earnings: d.earnings,
                ..Default::default()
            })
            .collect(),
        contexts: contexts
            .into_iter()
            .map(|(context, sends)| pb::ContextSends { context: context.into(), sends, ..Default::default() })
            .collect(),
        top_stamps: top
            .iter()
            .map(|view| pb::StampSends {
                stamp: view.to_proto(viewer).into(),
                sends: view.send_count,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    })
}

/// Totals over all packs of one creator.
#[derive(Debug, Default, sqlx::FromRow)]
pub struct CreatorTotals {
    pub pack_count: i64,
    pub published_pack_count: i64,
    pub install_count: i64,
    pub sale_count: i64,
    pub send_count: i64,
    pub earnings: i64,
}

pub async fn creator_totals(core: &Core, creator_id: Uuid) -> AppResult<CreatorTotals> {
    Ok(sqlx::query_as(
        "SELECT (SELECT count(*) FROM stamp_packs WHERE creator_id = $1) AS pack_count,
                (SELECT count(*) FROM stamp_packs WHERE creator_id = $1 AND status = 3) AS published_pack_count,
                (SELECT COALESCE(sum(install_count), 0)::bigint FROM stamp_packs WHERE creator_id = $1) AS install_count,
                (SELECT COALESCE(sum(purchase_count), 0)::bigint FROM stamp_packs WHERE creator_id = $1) AS sale_count,
                COALESCE(sum(d.sends), 0)::bigint AS send_count,
                COALESCE(sum(d.earnings), 0)::bigint AS earnings
         FROM stamp_pack_daily d
         WHERE d.pack_id IN (SELECT id FROM stamp_packs WHERE creator_id = $1)",
    )
    .bind(creator_id)
    .fetch_one(&core.db)
    .await?)
}

/// Recomputes the popularity score and the send counter of every pack with
/// recent activity. Returns how many packs changed.
///
/// The score looks at the last 7 days: each send counts 1, each install 5
/// and each sale 20, and a day's contribution fades by 20% per day of age,
/// so a pack has to stay in use to stay on top.
pub async fn refresh_rankings(core: &Core) -> AppResult<u64> {
    let refreshed = sqlx::query(
        "WITH today AS (
             SELECT (now() AT TIME ZONE $1)::date AS day
         ), active AS (
             -- Packs with activity in the scoring window, plus the ones that just left it (their score drops to zero).
             SELECT DISTINCT d.pack_id FROM stamp_pack_daily d, today WHERE d.day >= today.day - 8
         ), fresh AS (
             SELECT d.pack_id,
                    sum(d.sends)::bigint AS sends,
                    COALESCE(sum((d.sends + 5 * d.installs + 20 * d.purchases) * power(0.8::float8, (today.day - d.day)::float8))
                             FILTER (WHERE d.day > today.day - 7), 0)::float8 AS score
             FROM stamp_pack_daily d, today
             WHERE d.pack_id IN (SELECT pack_id FROM active)
             GROUP BY d.pack_id
         )
         UPDATE stamp_packs p SET send_count = fresh.sends, popular_score = fresh.score
         FROM fresh
         WHERE p.id = fresh.pack_id AND (p.send_count <> fresh.sends OR p.popular_score <> fresh.score)",
    )
    .bind(STATS_TIME_ZONE)
    .execute(&core.db)
    .await?
    .rows_affected();
    // Packs that left the window while the worker was not running keep a
    // stale score otherwise.
    let faded = sqlx::query(
        "UPDATE stamp_packs p SET popular_score = 0
         WHERE p.popular_score > 0
           AND NOT EXISTS (SELECT 1 FROM stamp_pack_daily d WHERE d.pack_id = p.id AND d.day > (now() AT TIME ZONE $1)::date - 7)",
    )
    .bind(STATS_TIME_ZONE)
    .execute(&core.db)
    .await?
    .rows_affected();
    Ok(refreshed + faded)
}

/// Deletes usage log rows older than the retention period. Returns the
/// number of rows deleted. (Purchase intents are settled, never swept:
/// `Stamp::settle_unfinished_purchases`.)
pub async fn prune(core: &Core, usage_retention_days: i64) -> AppResult<u64> {
    let pruned = sqlx::query("DELETE FROM stamp_usage WHERE created_at < now() - make_interval(days => $1::int)")
        .bind(usage_retention_days)
        .execute(&core.db)
        .await?
        .rows_affected();
    Ok(pruned)
}
