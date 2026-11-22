//! Friend streaks: one row per pair of friends, driven by the pure state
//! machine in [`days`].
//!
//! - [`Quest::record_part`] is called when a player did something towards a
//!   friend (once per friend and local day is enough; the ingest sees to
//!   that). It settles the pair's dates, and when a day completes, both get
//!   their streak XP and the badges of the length they reached — all in one
//!   transaction with the streak itself.
//! - [`Quest::sweep_streaks`] runs every minute: it ends the streaks whose
//!   deadline passed (after the grace for late events) and reminds players
//!   whose own deadline is less than four hours away.
//! - [`Quest::bring_back_streak`] brings a streak back within 24 hours of its
//!   end, for coins, once per player and calendar month. The restore is an
//!   order that is placed before the wallet is charged and finished (or
//!   refunded) from wherever it is seen next.
//!
//! A streak is always read through [`days::Days::settle`] at the time of
//! reading, so a streak that ran out a minute ago already reads as lost,
//! whether or not the sweep has been there.
use chrono::{DateTime, Duration, NaiveDate, Utc};
use sqlx::PgConnection;
use tokuly_core::{
    analytics::AnalyticsEvent,
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::quest::{self as pb, StreakChange, StreakState, StreakTurn, quest_event::Event as Push};
use tokuly_social::graph;
use tokuly_wallet::ledger;

use crate::{
    Quest,
    badges::{
        self,
        catalog::{self, CATALOG, Criterion},
    },
    clock,
    earnings::{Earned, Effects},
    metrics::Metric,
    profile::lock_profile,
    rules::{STREAK_DAY_XP, Source},
};

pub mod days;
mod svc;

use days::{Days, Limits, Side, Step, Zones};

/// A streak shows the hourglass when it ends within this long.
const HOURGLASS: Duration = Duration::hours(4);
/// How long a streak that ended can be restored.
const RESTORE_WINDOW: Duration = Duration::hours(24);

#[derive(Debug, Clone, sqlx::FromRow)]
struct StreakRow {
    user_lo: Uuid,
    user_hi: Uuid,
    count: i32,
    best: i32,
    settled_day: Option<NaiveDate>,
    started_on: Option<NaiveDate>,
    lo_days: Vec<NaiveDate>,
    hi_days: Vec<NaiveDate>,
    lo_warned: Option<NaiveDate>,
    hi_warned: Option<NaiveDate>,
    lost_count: i32,
    lost_at: Option<Time>,
}

const STREAK_COLUMNS: &str = "user_lo, user_hi, count, best, settled_day, started_on, lo_days, hi_days, lo_warned, hi_warned, lost_count, lost_at";

impl StreakRow {
    fn days(&self) -> Days {
        Days {
            count: self.count,
            best: self.best,
            settled: self.settled_day,
            started_on: self.started_on,
            lo_days: self.lo_days.clone(),
            hi_days: self.hi_days.clone(),
            lost_count: self.lost_count,
            lost_at: self.lost_at,
        }
    }

    fn blank(a: Uuid, b: Uuid) -> Self {
        let (user_lo, user_hi) = ordered(a, b);
        Self {
            user_lo,
            user_hi,
            count: 0,
            best: 0,
            settled_day: None,
            started_on: None,
            lo_days: Vec::new(),
            hi_days: Vec::new(),
            lo_warned: None,
            hi_warned: None,
            lost_count: 0,
            lost_at: None,
        }
    }

    fn side_of(&self, user: Uuid) -> Side {
        if user == self.user_lo { Side::Lo } else { Side::Hi }
    }

    fn other(&self, user: Uuid) -> Uuid {
        if user == self.user_lo { self.user_hi } else { self.user_lo }
    }
}

/// A streak in a list, with its length as of the moment of the query.
#[derive(Debug, Clone, sqlx::FromRow)]
struct ListedStreak {
    #[sqlx(flatten)]
    row: StreakRow,
    running: i32,
}

/// Two users in the order the table keys them. (`Uuid` and PostgreSQL's
/// `uuid` both compare byte-wise, so Rust and SQL agree.)
fn ordered(a: Uuid, b: Uuid) -> (Uuid, Uuid) {
    if a < b { (a, b) } else { (b, a) }
}

/// A streak restore a player ordered (`quest_streak_restores`).
#[derive(Debug, Clone, sqlx::FromRow)]
struct RestoreOrder {
    id: Uuid,
    user_id: Uuid,
    friend_id: Uuid,
    cost: i64,
    /// The loss the order is for: the streak's `lost_at` when it was placed.
    lost_at: Time,
    applied_at: Option<Time>,
}

const ORDER_COLUMNS: &str = "id, user_id, friend_id, cost, lost_at, applied_at";

/// How settling a restore order ended.
enum Settled {
    /// The streak is back. `paid` is what this call took from the wallet
    /// (nothing when another call had charged the order already).
    Restored { paid: i64 },
    /// There was nothing left to restore: the friend was faster, or the
    /// two are no longer friends. The coins went back and the order is gone.
    Undeliverable,
}

/// What a change to a streak means for one of the two friends.
struct Outcome {
    user: Uuid,
    effects: Effects,
}

/// A streak as one of the two sees it at `now`.
fn view(
    days: &Days,
    me: Side,
    friend: tokuly_proto::common::UserSummary,
    zones: Zones,
    limits: Limits,
    now: DateTime<Utc>,
) -> pb::Streak {
    // Read as of now, without the grace: the deadline has passed or it has not.
    let mut days = days.clone();
    days.settle(now, zones, Limits { grace: Duration::zero(), ..limits });
    let deadline = days.deadline(zones);
    let restorable = days.restorable(now, limits);
    let state = match deadline {
        Some(deadline) if deadline - now <= HOURGLASS => StreakState::STREAK_STATE_EXPIRING,
        Some(_) => StreakState::STREAK_STATE_ACTIVE,
        None if restorable => StreakState::STREAK_STATE_LOST,
        None => StreakState::STREAK_STATE_NONE,
    };
    let missing = days.waiting_on();
    let (mine, theirs) = match me {
        Side::Lo => (missing.lo, missing.hi),
        Side::Hi => (missing.hi, missing.lo),
    };
    let waiting_on = match (days.count > 0, mine, theirs) {
        (false, _, _) => StreakTurn::STREAK_TURN_UNSPECIFIED,
        (true, true, true) => StreakTurn::STREAK_TURN_BOTH,
        (true, true, false) => StreakTurn::STREAK_TURN_MINE,
        (true, false, true) => StreakTurn::STREAK_TURN_THEIRS,
        (true, false, false) => StreakTurn::STREAK_TURN_NONE,
    };
    pb::Streak {
        friend: friend.into(),
        days: days.count,
        best_days: days.best,
        state: state.into(),
        waiting_on: waiting_on.into(),
        expires_at: opt_ts(deadline),
        next_milestone: days::next_milestone(days.count),
        started_on: days.started_on.filter(|_| days.count > 0).map(clock::date_text).unwrap_or_default(),
        lost_days: if restorable { days.lost_count } else { 0 },
        lost_at: opt_ts(days.lost_at.filter(|_| restorable)),
        restorable_until: opt_ts(days.lost_at.filter(|_| restorable).map(|at| at + limits.restore_window)),
        ..Default::default()
    }
}

/// Stores the machine's state on the pair's row.
async fn save(tx: &mut PgConnection, row: &StreakRow, days: &Days, zones: Zones) -> AppResult<()> {
    sqlx::query(
        "UPDATE quest_streaks SET count = $3, best = $4, settled_day = $5, started_on = $6, lo_days = $7, hi_days = $8,
                expires_at = $9, lost_count = $10, lost_at = $11
         WHERE user_lo = $1 AND user_hi = $2",
    )
    .bind(row.user_lo)
    .bind(row.user_hi)
    .bind(days.count)
    .bind(days.best)
    .bind(days.settled)
    .bind(days.started_on)
    .bind(&days.lo_days)
    .bind(&days.hi_days)
    .bind(days.deadline(zones))
    .bind(days.lost_count)
    .bind(days.lost_at)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn lock_streak(tx: &mut PgConnection, lo: Uuid, hi: Uuid) -> AppResult<Option<StreakRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {STREAK_COLUMNS} FROM quest_streaks WHERE user_lo = $1 AND user_hi = $2 FOR UPDATE"
    )))
    .bind(lo)
    .bind(hi)
    .fetch_optional(&mut *tx)
    .await?)
}

impl Quest {
    pub(crate) fn streak_limits(&self) -> Limits {
        Limits { grace: Duration::minutes(self.cfg.streak_grace_minutes), restore_window: RESTORE_WINDOW }
    }

    /// The time zones of a pair; `None` when one of the accounts is gone.
    async fn zones_of(&self, lo: Uuid, hi: Uuid) -> AppResult<Option<Zones>> {
        match (self.zone_of(lo).await?, self.zone_of(hi).await?) {
            (Some(lo), Some(hi)) => Ok(Some(Zones { lo, hi })),
            _ => Ok(None),
        }
    }

    /// Records that `user` did something towards their friend `friend` at
    /// `at`. The caller has checked that the two are friends. Safe to
    /// repeat: a part that is already recorded changes nothing.
    pub(crate) async fn record_part(&self, user: Uuid, friend: Uuid, at: DateTime<Utc>) -> AppResult<()> {
        let (lo, hi) = ordered(user, friend);
        let Some(zones) = self.zones_of(lo, hi).await? else { return Ok(()) };
        let limits = self.streak_limits();
        let mut tx = self.core.db.begin().await?;
        sqlx::query(
            "INSERT INTO quest_streaks (user_lo, user_hi) VALUES ($1, $2) ON CONFLICT (user_lo, user_hi) DO NOTHING",
        )
        .bind(lo)
        .bind(hi)
        .execute(&mut *tx)
        .await?;
        let Some(row) = lock_streak(&mut tx, lo, hi).await? else { return Ok(()) };
        let mut days = row.days();
        let steps = days.act(row.side_of(user), at, zones, limits);
        if days == row.days() {
            return Ok(());
        }
        save(&mut tx, &row, &days, zones).await?;
        let outcomes = self.reward_completed_days(&mut tx, &row, &steps, zones, at).await?;
        tx.commit().await?;
        self.announce_streak(&row, &days, &steps, outcomes, zones).await;
        Ok(())
    }

    /// For every day the steps completed: streak XP and the day counter for
    /// both friends, and the badges of the length reached.
    async fn reward_completed_days(
        &self,
        tx: &mut PgConnection,
        row: &StreakRow,
        steps: &[Step],
        zones: Zones,
        at: DateTime<Utc>,
    ) -> AppResult<Vec<Outcome>> {
        let completed: Vec<i32> = steps
            .iter()
            .filter_map(|step| if let Step::Completed { days, .. } = step { Some(*days) } else { None })
            .collect();
        if completed.is_empty() {
            return Ok(Vec::new());
        }
        let cap = Source::Streaks.def().daily_cap as i64;
        let mut outcomes = Vec::with_capacity(2);
        // Always the lower user first: two pairs sharing a player lock in the same order.
        for (user, zone) in [(row.user_lo, zones.lo), (row.user_hi, zones.hi)] {
            let mut profile = lock_profile(tx, user).await?;
            let mut effects = Effects::starting_from(&profile);
            let day = clock::local_date(at, zone);
            for length in &completed {
                let earned_today: i64 = sqlx::query_scalar("SELECT COALESCE(sum(xp), 0)::bigint FROM quest_xp_days WHERE user_id = $1 AND day = $2 AND source = $3")
                    .bind(user)
                    .bind(day)
                    .bind(Source::Streaks.key())
                    .fetch_one(&mut *tx)
                    .await?;
                let xp = (STREAK_DAY_XP as i64).min((cap - earned_today).max(0));
                let earned = Earned { xp: vec![(Source::Streaks, xp)], metrics: vec![(Metric::StreakDays, 1)] };
                self.apply(tx, &mut profile, day, &earned, &mut effects).await?;
                for badge in CATALOG.iter() {
                    let earned = match badge.criterion {
                        Criterion::StreakDays(days) => days == *length,
                        // Reaching the length that counts may complete a "this many at once" badge.
                        Criterion::Streaks { days, count } if days == *length => {
                            let running: i64 = sqlx::query_scalar("SELECT count(*) FROM quest_streaks WHERE (user_lo = $1 OR user_hi = $1) AND count >= $2").bind(user).bind(days).fetch_one(&mut *tx).await?;
                            running >= count as i64
                        }
                        _ => false,
                    };
                    if earned
                        && badges::award_tx(tx, user, badge, serde_json::json!({ "with": row.other(user) })).await?
                    {
                        effects.badges.push(badge);
                    }
                }
            }
            outcomes.push(Outcome { user, effects });
        }
        Ok(outcomes)
    }

    /// Tells both friends what happened to their streak (after the commit).
    async fn announce_streak(
        &self,
        row: &StreakRow,
        days: &Days,
        steps: &[Step],
        outcomes: Vec<Outcome>,
        zones: Zones,
    ) {
        for outcome in outcomes {
            self.announce(outcome.user, outcome.effects).await;
        }
        let completed: Vec<i32> = steps
            .iter()
            .filter_map(|step| if let Step::Completed { days, .. } = step { Some(*days) } else { None })
            .collect();
        let lost = steps.iter().any(|step| matches!(step, Step::Lost { .. }));
        let change = if completed.iter().any(|length| days::is_milestone(*length)) {
            StreakChange::STREAK_CHANGE_MILESTONE
        } else if !completed.is_empty() {
            StreakChange::STREAK_CHANGE_EXTENDED
        } else if lost {
            StreakChange::STREAK_CHANGE_LOST
        } else {
            return;
        };
        self.push_streak(row, days, zones, change).await;
        for length in completed {
            if days::is_milestone(length) {
                let event = AnalyticsEvent::new("quest.streak.milestone")
                    .value(length as f64)
                    .props(serde_json::json!({ "users": [row.user_lo, row.user_hi] }));
                self.core.analytics.track(event).await;
            }
        }
    }

    /// Pushes `StreakChanged` to both friends, each with their own view.
    async fn push_streak(&self, row: &StreakRow, days: &Days, zones: Zones, change: StreakChange) {
        let Ok(summaries) = tokuly_identity::summaries(&self.core, &[row.user_lo, row.user_hi]).await else { return };
        let now = now();
        for user in [row.user_lo, row.user_hi] {
            let friend = tokuly_identity::users::pick(&summaries, row.other(user));
            let streak = view(days, row.side_of(user), friend, zones, self.streak_limits(), now);
            self.push(
                user,
                Push::StreakChanged(Box::new(pb::StreakChanged {
                    streak: streak.into(),
                    change: change.into(),
                    ..Default::default()
                })),
            )
            .await;
        }
    }

    /// The streak between `viewer` and `friend` as the viewer sees it.
    pub(crate) async fn streak_between(&self, viewer: Uuid, friend: Uuid) -> AppResult<pb::Streak> {
        let (lo, hi) = ordered(viewer, friend);
        let row: Option<StreakRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {STREAK_COLUMNS} FROM quest_streaks WHERE user_lo = $1 AND user_hi = $2"
        )))
        .bind(lo)
        .bind(hi)
        .fetch_optional(&self.core.db)
        .await?;
        let row = row.unwrap_or_else(|| StreakRow::blank(viewer, friend));
        let summary = tokuly_identity::summary(&self.core, friend).await?;
        let zones = self.zones_of(lo, hi).await?.unwrap_or(Zones { lo: clock::DEFAULT_ZONE, hi: clock::DEFAULT_ZONE });
        Ok(view(&row.days(), row.side_of(viewer), summary, zones, self.streak_limits(), now()))
    }

    /// A page of the player's streaks: running ones longest first, then the
    /// ones that ended within the restore window.
    pub(crate) async fn streaks_page(
        &self,
        user_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<pb::Streak>, tokuly_proto::common::PageInfo)> {
        let cursor: Option<(i32, Uuid, Uuid)> = page.decode()?;
        let (after_running, after_lo, after_hi) =
            cursor.map_or((None, None, None), |(running, lo, hi)| (Some(running), Some(lo), Some(hi)));
        // `running` is the length as of now: a streak past its deadline sorts
        // with the ended ones, whether or not the sweep has been there.
        let mut rows: Vec<ListedStreak> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {STREAK_COLUMNS}, running FROM (
                 SELECT *, CASE WHEN count > 0 AND expires_at > now() THEN count ELSE 0 END AS running
                 FROM quest_streaks
                 WHERE (user_lo = $1 OR user_hi = $1) AND (count > 0 OR (lost_count >= $2 AND lost_at > now() - make_interval(hours => $3)))
             ) listed
             WHERE $4::int IS NULL OR (running, user_lo, user_hi) < ($4, $5, $6)
             ORDER BY running DESC, user_lo DESC, user_hi DESC
             LIMIT $7"
        )))
        .bind(user_id)
        .bind(days::MIN_RESTORABLE_DAYS)
        .bind(RESTORE_WINDOW.num_hours() as i32)
        .bind(after_running)
        .bind(after_lo)
        .bind(after_hi)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |listed| (listed.running, listed.row.user_lo, listed.row.user_hi));
        let friends: Vec<Uuid> = rows.iter().map(|listed| listed.row.other(user_id)).collect();
        let summaries = tokuly_identity::summaries(&self.core, &friends).await?;
        let now = now();
        let mut streaks = Vec::with_capacity(rows.len());
        for ListedStreak { row, .. } in &rows {
            let Some(zones) = self.zones_of(row.user_lo, row.user_hi).await? else { continue };
            let streak = view(
                &row.days(),
                row.side_of(user_id),
                tokuly_identity::users::pick(&summaries, row.other(user_id)),
                zones,
                self.streak_limits(),
                now,
            );
            // A streak that ended too long ago to be restored is not worth a row.
            if streak.state != StreakState::STREAK_STATE_NONE {
                streaks.push(streak);
            }
        }
        Ok((streaks, info))
    }

    /// What restoring a streak costs `user_id` and whether they still may this month.
    pub(crate) async fn restore_offer(&self, user_id: Uuid, today: NaiveDate) -> AppResult<pb::RestoreOffer> {
        let month = clock::month_start(today);
        let used: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM quest_streak_restores WHERE user_id = $1 AND month = $2)")
                .bind(user_id)
                .bind(month)
                .fetch_one(&self.core.db)
                .await?;
        Ok(pb::RestoreOffer {
            cost_coins: self.cfg.streak_restore_coins as i32,
            available: !used,
            available_again_on: if used { clock::date_text(clock::next_month_start(today)) } else { String::new() },
            ..Default::default()
        })
    }

    /// Brings back the streak `user_id` lost with `friend`, for coins.
    /// Returns the streak and the coins this call took.
    ///
    /// A restore is an order (`quest_streak_restores`): it is placed, with
    /// the loss it is for, before any coins move, and then settled
    /// ([`Self::settle_restore`]). Whatever interrupts it, the order is
    /// finished from the next place it is seen — so a player is never
    /// charged for a streak they did not get back.
    pub(crate) async fn bring_back_streak(
        &self,
        user_id: Uuid,
        friend: Uuid,
        today: NaiveDate,
    ) -> AppResult<(pb::Streak, i64)> {
        ensure!(
            user_id != friend && graph::are_friends(&self.core, user_id, friend).await?,
            precondition,
            "NOT_FRIENDS",
            "you are not friends with this user"
        );
        // An order left unfinished comes first: it may be this very restore.
        let mut paid = self.settle_restores_of(user_id).await?;
        let (lo, hi) = ordered(user_id, friend);
        let zones = self
            .zones_of(lo, hi)
            .await?
            .ok_or_else(|| AppError::precondition("NOT_FRIENDS", "you are not friends with this user"))?;
        // At the moment of a restore the deadline has passed or it has not: no grace.
        let limits = Limits { grace: Duration::zero(), ..self.streak_limits() };
        let month = clock::month_start(today);
        let now = now();

        // Place the order, under the lock of the streak it is for.
        let mut tx = self.core.db.begin().await?;
        let row = lock_streak(&mut tx, lo, hi).await?;
        let mut days = row.as_ref().map(StreakRow::days).unwrap_or_default();
        days.settle(now, zones, limits);
        let existing: Option<RestoreOrder> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {ORDER_COLUMNS} FROM quest_streak_restores WHERE user_id = $1 AND month = $2 FOR UPDATE"
        )))
        .bind(user_id)
        .bind(month)
        .fetch_optional(&mut *tx)
        .await?;
        let order = match existing {
            // Another request of this player is settling the same order: join it.
            Some(order) if order.friend_id == friend && order.applied_at.is_none() => Some(order),
            // Repeating a restore that already happened is not an error.
            Some(order) if order.friend_id == friend && days.count > 0 && !days.restorable(now, limits) => None,
            Some(_) => bail!(precondition, "RESTORE_ALREADY_USED", "you already restored a streak this month"),
            None => {
                let lost_at = match days.lost_at.filter(|_| days.restorable(now, limits)) {
                    Some(lost_at) => lost_at,
                    None => {
                        let ended_long_ago = days.lost_count >= days::MIN_RESTORABLE_DAYS && days.lost_at.is_some();
                        ensure!(
                            !ended_long_ago,
                            precondition,
                            "RESTORE_WINDOW_OVER",
                            "the streak ended more than 24 hours ago"
                        );
                        bail!(precondition, "STREAK_NOT_LOST", "there is no lost streak to restore");
                    }
                };
                // The loss is stored with the streak, so the order names something that is on record.
                if let Some(row) = &row {
                    save(&mut tx, row, &days, zones).await?;
                }
                let placed: Option<RestoreOrder> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                    "INSERT INTO quest_streak_restores (user_id, month, friend_id, cost, lost_at) VALUES ($1, $2, $3, $4, $5)
                     ON CONFLICT (user_id, month) DO NOTHING RETURNING {ORDER_COLUMNS}"
                )))
                .bind(user_id)
                .bind(month)
                .bind(friend)
                .bind(self.cfg.streak_restore_coins)
                .bind(lost_at)
                .fetch_optional(&mut *tx)
                .await?;
                // Lost the race for the month's restore to a request for another friend.
                Some(placed.ok_or_else(|| {
                    AppError::precondition("RESTORE_ALREADY_USED", "you already restored a streak this month")
                })?)
            }
        };
        tx.commit().await?;

        if let Some(order) = order {
            match self.settle_restore(&order).await? {
                Settled::Restored { paid: now_paid } => paid += now_paid,
                Settled::Undeliverable => {
                    // Somebody was faster (the friend restored it): nothing was charged.
                    let streak = self.streak_between(user_id, friend).await?;
                    ensure!(streak.days > 0, precondition, "STREAK_NOT_LOST", "there is no lost streak to restore");
                    return Ok((streak, paid));
                }
            }
        }
        Ok((self.streak_between(user_id, friend).await?, paid))
    }

    /// Finishes a placed order: charges the wallet (once per order,
    /// whoever asks) and restores the streak — or, when there is nothing
    /// left to restore, gives the coins back and withdraws the order. Safe
    /// to call any number of times, from anywhere, at any later time.
    async fn settle_restore(&self, order: &RestoreOrder) -> AppResult<Settled> {
        let key = format!("quest.streak_restore:{}", order.id);
        let memo = serde_json::json!({ "friend_id": order.friend_id });
        let (entry_id, paid) =
            match ledger::spend(&self.core, order.user_id, order.cost, "quest.streak_restore", &key, memo).await {
                Ok(spent) => spent,
                Err(e) => {
                    if e.reason == "INSUFFICIENT_COINS" {
                        // It cannot be paid for: the order is withdrawn, and the month's restore is free again.
                        sqlx::query("DELETE FROM quest_streak_restores WHERE id = $1 AND applied_at IS NULL")
                            .bind(order.id)
                            .execute(&self.core.db)
                            .await?;
                    }
                    return Err(e);
                }
            };
        let paid = if paid { order.cost } else { 0 };
        let (lo, hi) = ordered(order.user_id, order.friend_id);
        // Everything the transaction needs to know is read before it opens.
        let zones = self.zones_of(lo, hi).await?;
        let name = tokuly_identity::summary(&self.core, order.user_id).await?.display_name;
        let limits = Limits { grace: Duration::zero(), ..self.streak_limits() };
        let now = now();

        let mut tx = self.core.db.begin().await?;
        // The streak first, then the order: the same order of locks as placing one.
        let row = lock_streak(&mut tx, lo, hi).await?;
        let current: Option<RestoreOrder> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {ORDER_COLUMNS} FROM quest_streak_restores WHERE id = $1 FOR UPDATE"
        )))
        .bind(order.id)
        .fetch_optional(&mut *tx)
        .await?;
        match &current {
            Some(current) if current.applied_at.is_some() => return Ok(Settled::Restored { paid }),
            Some(_) => {}
            // Withdrawn by a request that found the wallet empty a moment ago.
            None => {
                drop(tx);
                ledger::refund(&self.core, entry_id, "quest.streak_restore.refund").await?;
                return Ok(Settled::Undeliverable);
            }
        }
        // Still the loss the order was placed for? (The friend may have restored
        // it, the two may have started over, the friendship may have ended.)
        let lost = match (&row, zones) {
            (Some(row), Some(zones)) => {
                let mut days = row.days();
                days.settle(now, zones, limits);
                (days.lost_count > 0 && days.lost_at == Some(order.lost_at)).then_some((row, days, zones))
            }
            _ => None,
        };
        let Some((row, mut days, zones)) = lost else {
            drop(tx);
            // Refund, then withdraw: stopping in between leaves an order that
            // is settled the same way next time (the refund is idempotent).
            ledger::refund(&self.core, entry_id, "quest.streak_restore.refund").await?;
            sqlx::query("DELETE FROM quest_streak_restores WHERE id = $1 AND applied_at IS NULL")
                .bind(order.id)
                .execute(&self.core.db)
                .await?;
            return Ok(Settled::Undeliverable);
        };
        let restored = days.restore(now, zones, limits);
        save(&mut tx, row, &days, zones).await?;
        sqlx::query(
            "UPDATE quest_streak_restores SET applied_at = now(), restored_count = $2, entry_id = $3 WHERE id = $1",
        )
        .bind(order.id)
        .bind(restored)
        .bind(entry_id)
        .execute(&mut *tx)
        .await?;
        let mut unlocked = None;
        if let Some(badge) = catalog::find(catalog::PHOENIX)
            && badges::award_tx(&mut tx, order.user_id, badge, serde_json::json!({ "with": order.friend_id })).await?
        {
            unlocked = Some(badge);
        }
        let notice = NotificationRequest::new(
            order.friend_id,
            "quest.streak_restored",
            Localized::new(
                format!("{name}さんがストリークを復活させました!"),
                format!("{name} brought your streak back!"),
            ),
            Localized::new(
                format!("🔥 {restored}日連続に戻りました。今日もつなごう!"),
                format!("🔥 Back to {restored} days. Keep it going today!"),
            ),
            "/quest/streaks",
        )
        .actor(order.user_id)
        .collapse(format!("quest.streak:{lo}:{hi}"))
        .data(serde_json::json!({ "friend_id": order.user_id, "days": restored }));
        outbound::notify(&mut *tx, &notice).await?;
        tx.commit().await?;

        if let Some(badge) = unlocked {
            badges::announce_unlocked(&self.core, order.user_id, badge).await;
        }
        self.push_streak(row, &days, zones, StreakChange::STREAK_CHANGE_RESTORED).await;
        let event = AnalyticsEvent::new("quest.streak.restore")
            .user(Some(order.user_id))
            .value(order.cost as f64)
            .props(serde_json::json!({ "friend_id": order.friend_id, "days": restored }));
        self.core.analytics.track(event).await;
        Ok(Settled::Restored { paid })
    }

    /// Finishes the orders of one player that were placed but not settled
    /// (a process stopped in between). Called wherever the player looks at
    /// their streaks. Returns the coins this call took.
    pub(crate) async fn settle_restores_of(&self, user_id: Uuid) -> AppResult<i64> {
        let pending: Vec<RestoreOrder> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {ORDER_COLUMNS} FROM quest_streak_restores WHERE user_id = $1 AND applied_at IS NULL"
        )))
        .bind(user_id)
        .fetch_all(&self.core.db)
        .await?;
        let mut paid = 0;
        for order in pending {
            match self.settle_restore(&order).await {
                Ok(Settled::Restored { paid: now_paid }) => paid += now_paid,
                Ok(Settled::Undeliverable) => {}
                // Withdrawn: there is nothing to finish.
                Err(e) if e.reason == "INSUFFICIENT_COINS" => {}
                Err(e) => return Err(e),
            }
        }
        Ok(paid)
    }

    /// [`Self::settle_restores_of`] on the way to showing a player their
    /// streaks: a failure is logged and waits for the next time.
    pub(crate) async fn finish_restores(&self, user_id: Uuid) {
        if let Err(e) = self.settle_restores_of(user_id).await {
            tracing::warn!(%user_id, error = %e, "quest: a streak restore is still unsettled");
        }
    }

    /// Finishes every order that has been waiting for more than a minute.
    /// Returns how many there were.
    pub async fn settle_pending_restores(&self) -> AppResult<usize> {
        let pending: Vec<RestoreOrder> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {ORDER_COLUMNS} FROM quest_streak_restores WHERE applied_at IS NULL AND created_at < now() - interval '1 minute' ORDER BY created_at LIMIT 200"
        )))
        .fetch_all(&self.core.db)
        .await?;
        for order in &pending {
            match self.settle_restore(order).await {
                Ok(_) => {}
                Err(e) if e.reason == "INSUFFICIENT_COINS" => {}
                Err(e) => tracing::warn!(order = %order.id, error = %e, "quest: streak restore still unsettled"),
            }
        }
        Ok(pending.len())
    }

    /// Ends the streaks whose deadline passed and reminds players whose own
    /// deadline is near. Returns `(ended, reminded)`.
    pub async fn sweep_streaks(&self) -> AppResult<(usize, usize)> {
        Ok((self.end_overdue_streaks().await?, self.remind_expiring_streaks().await?))
    }

    async fn end_overdue_streaks(&self) -> AppResult<usize> {
        let limits = self.streak_limits();
        let overdue: Vec<(Uuid, Uuid)> = sqlx::query_as("SELECT user_lo, user_hi FROM quest_streaks WHERE count > 0 AND expires_at <= now() - make_interval(mins => $1) ORDER BY expires_at LIMIT 500")
            .bind(self.cfg.streak_grace_minutes as i32)
            .fetch_all(&self.core.db)
            .await?;
        let mut ended = 0;
        for (lo, hi) in overdue {
            // Zones are read fresh here: a player who travelled gets the deadline of where they are.
            self.forget_account(lo).await;
            self.forget_account(hi).await;
            let Some(zones) = self.zones_of(lo, hi).await? else {
                // One of the two is gone; the streak ends without ceremony.
                sqlx::query("UPDATE quest_streaks SET count = 0, expires_at = NULL, lost_count = 0, lost_at = NULL WHERE user_lo = $1 AND user_hi = $2").bind(lo).bind(hi).execute(&self.core.db).await?;
                continue;
            };
            let now = now();
            let mut tx = self.core.db.begin().await?;
            let Some(row) = lock_streak(&mut tx, lo, hi).await? else { continue };
            let mut days = row.days();
            let steps = days.settle(now, zones, limits);
            // Also when nothing was settled: the stored deadline is brought in
            // line with the zones as they are now.
            save(&mut tx, &row, &days, zones).await?;
            let outcomes = self.reward_completed_days(&mut tx, &row, &steps, zones, now).await?;
            tx.commit().await?;
            if steps.iter().any(|step| matches!(step, Step::Lost { .. })) {
                ended += 1;
            }
            self.announce_streak(&row, &days, &steps, outcomes, zones).await;
        }
        Ok(ended)
    }

    async fn remind_expiring_streaks(&self) -> AppResult<usize> {
        let rows: Vec<StreakRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {STREAK_COLUMNS} FROM quest_streaks
             WHERE count > 0 AND expires_at > now() AND expires_at <= now() + make_interval(hours => $1)
               AND (lo_warned IS DISTINCT FROM settled_day + 1 OR hi_warned IS DISTINCT FROM settled_day + 1)
             ORDER BY expires_at LIMIT 500"
        )))
        .bind(HOURGLASS.num_hours() as i32)
        .fetch_all(&self.core.db)
        .await?;
        let mut reminded = 0;
        for row in rows {
            let Some(zones) = self.zones_of(row.user_lo, row.user_hi).await? else { continue };
            let days = row.days();
            let Some(date) = days.open_streak_date() else { continue };
            let now = now();
            let mut told_someone = false;
            for (side, user, warned) in [(Side::Lo, row.user_lo, row.lo_warned), (Side::Hi, row.user_hi, row.hi_warned)]
            {
                if warned == Some(date) {
                    continue;
                }
                let column = if side == Side::Lo { "lo_warned" } else { "hi_warned" };
                let due = match days.deadline_of(side, zones) {
                    // Their part is done: nothing to remind them of, today.
                    None => true,
                    Some(deadline) => deadline > now && deadline - now <= HOURGLASS,
                };
                if !due {
                    continue;
                }
                // Read before the transaction opens: it should hold one connection, not two.
                let deadline = days.deadline_of(side, zones);
                let wanted = deadline.is_some()
                    && days.count >= self.cfg.streak_reminder_min_days
                    && self.profile(user).await?.streak_reminders;
                let friend = row.other(user);
                let name = if wanted {
                    tokuly_identity::summary(&self.core, friend).await?.display_name
                } else {
                    String::new()
                };
                let mut tx = self.core.db.begin().await?;
                // The update decides who sends: only one sweep gets the row.
                let first = sqlx::query(sqlx::AssertSqlSafe(format!(
                    "UPDATE quest_streaks SET {column} = $3 WHERE user_lo = $1 AND user_hi = $2 AND count > 0 AND settled_day = $4 AND {column} IS DISTINCT FROM $3"
                )))
                .bind(row.user_lo)
                .bind(row.user_hi)
                .bind(date)
                .bind(days.settled)
                .execute(&mut *tx)
                .await?
                .rows_affected()
                    > 0;
                if let (true, true, Some(deadline)) = (first, wanted, deadline) {
                    let notice = NotificationRequest::new(
                        user,
                        "quest.streak_expiring",
                        Localized::new(format!("⌛ {name}さんとのストリークがもうすぐ途切れちゃう!"), format!("⌛ Your streak with {name} is about to end!")),
                        Localized::new(format!("🔥 {}日連続。今日のうちにメッセージやスタンプを送ろう!", days.count), format!("🔥 {} days in a row. Send a message or a stamp before the day is over!", days.count)),
                        "/quest/streaks",
                    )
                    .actor(friend)
                    .collapse(format!("quest.streak:{}:{}", row.user_lo, row.user_hi))
                    .data(serde_json::json!({ "friend_id": friend, "days": days.count, "expires_at": deadline.to_rfc3339() }));
                    outbound::notify(&mut *tx, &notice).await?;
                    reminded += 1;
                    told_someone = true;
                }
                tx.commit().await?;
            }
            if told_someone {
                self.push_streak(&row, &days, zones, StreakChange::STREAK_CHANGE_EXPIRING).await;
            }
        }
        Ok(reminded)
    }

    /// The friendship of two players ended at `ended_at` (unfriend or
    /// block): so does their streak, for good.
    ///
    /// Only a streak that existed then is ended. The event can arrive again,
    /// or late — after the two became friends once more and started over —
    /// and must not take the new streak with it.
    pub(crate) async fn end_streak(&self, a: Uuid, b: Uuid, ended_at: DateTime<Utc>) -> AppResult<()> {
        let (lo, hi) = ordered(a, b);
        let row: Option<StreakRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!("DELETE FROM quest_streaks WHERE user_lo = $1 AND user_hi = $2 AND created_at <= $3 RETURNING {STREAK_COLUMNS}")))
            .bind(lo)
            .bind(hi)
            .bind(ended_at)
            .fetch_optional(&self.core.db)
            .await?;
        let Some(row) = row else { return Ok(()) };
        self.forget_towards(a, b).await?;
        if row.count > 0
            && let Some(zones) = self.zones_of(lo, hi).await?
        {
            self.push_streak(
                &row,
                &Days { best: row.best, ..Default::default() },
                zones,
                StreakChange::STREAK_CHANGE_ENDED,
            )
            .await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    const TOKYO: chrono_tz::Tz = chrono_tz::Asia::Tokyo;

    fn zones() -> Zones {
        Zones { lo: TOKYO, hi: TOKYO }
    }

    fn limits() -> Limits {
        Limits { grace: Duration::minutes(10), restore_window: RESTORE_WINDOW }
    }

    fn tokyo(d: u32, h: u32, m: u32) -> DateTime<Utc> {
        TOKYO.with_ymd_and_hms(2026, 11, d, h, m, 0).unwrap().with_timezone(&Utc)
    }

    fn friend() -> tokuly_proto::common::UserSummary {
        tokuly_proto::common::UserSummary {
            id: Uuid::now_v7().to_string(),
            handle: "friend".into(),
            ..Default::default()
        }
    }

    /// A streak of `length` days up to and including the 10th.
    fn running(length: i32) -> Days {
        let mut days = Days::default();
        for d in (11 - length as u32)..=10 {
            days.act(Side::Lo, tokyo(d, 9, 0), zones(), limits());
            days.act(Side::Hi, tokyo(d, 10, 0), zones(), limits());
        }
        assert_eq!(days.count, length);
        days
    }

    #[test]
    fn a_running_streak_shows_whose_turn_it_is() {
        let mut days = running(5);
        let seen = view(&days, Side::Lo, friend(), zones(), limits(), tokyo(11, 9, 0));
        assert_eq!((seen.days, seen.best_days, seen.next_milestone), (5, 5, 7));
        assert_eq!(seen.state, StreakState::STREAK_STATE_ACTIVE);
        assert_eq!(seen.waiting_on, StreakTurn::STREAK_TURN_BOTH);
        assert_eq!(from_ts(seen.expires_at.as_option().unwrap()), tokyo(12, 0, 0));
        assert_eq!(seen.started_on, "2026-11-06");
        // Lo does their part: from Lo's side it is the friend's turn, from Hi's their own.
        days.act(Side::Lo, tokyo(11, 10, 0), zones(), limits());
        assert_eq!(
            view(&days, Side::Lo, friend(), zones(), limits(), tokyo(11, 11, 0)).waiting_on,
            StreakTurn::STREAK_TURN_THEIRS
        );
        assert_eq!(
            view(&days, Side::Hi, friend(), zones(), limits(), tokyo(11, 11, 0)).waiting_on,
            StreakTurn::STREAK_TURN_MINE
        );
        // Both done: nobody's turn until tomorrow.
        days.act(Side::Hi, tokyo(11, 12, 0), zones(), limits());
        let seen = view(&days, Side::Hi, friend(), zones(), limits(), tokyo(11, 13, 0));
        assert_eq!((seen.days, seen.waiting_on.as_known()), (6, Some(StreakTurn::STREAK_TURN_BOTH)));
        assert_eq!(from_ts(seen.expires_at.as_option().unwrap()), tokyo(13, 0, 0));
    }

    #[test]
    fn the_hourglass_shows_in_the_last_four_hours() {
        let days = running(5);
        assert_eq!(
            view(&days, Side::Lo, friend(), zones(), limits(), tokyo(11, 19, 59)).state,
            StreakState::STREAK_STATE_ACTIVE
        );
        assert_eq!(
            view(&days, Side::Lo, friend(), zones(), limits(), tokyo(11, 20, 0)).state,
            StreakState::STREAK_STATE_EXPIRING
        );
        assert_eq!(
            view(&days, Side::Lo, friend(), zones(), limits(), tokyo(11, 23, 59)).state,
            StreakState::STREAK_STATE_EXPIRING
        );
    }

    #[test]
    fn past_the_deadline_a_streak_reads_as_lost_even_before_the_sweep() {
        let days = running(5);
        // One minute past midnight: stored as running (the sweep waits for the
        // grace), shown as lost and restorable.
        let seen = view(&days, Side::Lo, friend(), zones(), limits(), tokyo(12, 0, 1));
        assert_eq!((seen.days, seen.state.as_known(), seen.lost_days), (0, Some(StreakState::STREAK_STATE_LOST), 5));
        assert_eq!(from_ts(seen.lost_at.as_option().unwrap()), tokyo(12, 0, 0));
        assert_eq!(from_ts(seen.restorable_until.as_option().unwrap()), tokyo(13, 0, 0));
        assert!(seen.expires_at.as_option().is_none() && seen.started_on.is_empty());
        // A day later it is simply gone.
        let seen = view(&days, Side::Lo, friend(), zones(), limits(), tokyo(13, 0, 0));
        assert_eq!(
            (seen.state.as_known(), seen.lost_days, seen.best_days),
            (Some(StreakState::STREAK_STATE_NONE), 0, 5)
        );
    }

    #[test]
    fn a_single_lost_day_is_not_offered_for_restore() {
        let days = running(1);
        let seen = view(&days, Side::Lo, friend(), zones(), limits(), tokyo(12, 0, 1));
        assert_eq!((seen.state.as_known(), seen.lost_days), (Some(StreakState::STREAK_STATE_NONE), 0));
    }

    #[test]
    fn pairs_are_keyed_in_one_order() {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        assert_eq!(ordered(a, b), ordered(b, a));
        let row = StreakRow::blank(b, a);
        assert!(row.user_lo < row.user_hi);
        assert_eq!(row.other(a), b);
        assert_eq!(row.side_of(row.user_lo), Side::Lo);
        assert_eq!(row.side_of(row.user_hi), Side::Hi);
    }
}
