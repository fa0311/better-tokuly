//! Missions: a player's daily and weekly set, progress, claims and rerolls.
//!
//! A mission counts one metric over its period: `progress` is what the
//! player's `quest_metric_days` add up to for that metric from the first
//! day of the period to the last, minus the mission's `baseline`. A drawn
//! mission has baseline 0 — the whole day (or week) counts, whenever the
//! set was drawn — and a rerolled one starts from the count at the moment
//! of the reroll, that is from zero.
//!
//! Sets are drawn on demand: when the player opens their missions, and when
//! the first earnings of a day are written (so "mission completed" can be
//! pushed to somebody who has not looked yet).
//!
//! A claim grants XP, the counter and the badge in its transaction and
//! marks the mission claimed; the coins follow through the wallet right
//! after and are recorded with `paid_at`. Should the process stop in
//! between, [`Quest::pay_stragglers`] pays what was left behind (the wallet
//! grant is idempotent per mission).
use std::collections::HashSet;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use sqlx::PgConnection;
use tokuly_core::{analytics::AnalyticsEvent, prelude::*};
use tokuly_proto::quest::{self as pb, MissionDifficulty, MissionPeriod, MissionState};
use tokuly_social::graph;
use tokuly_wallet::ledger;

use crate::{
    Quest,
    badges::{self, catalog},
    clock,
    earnings::{Earned, Effects},
    metrics::Metric,
    profile::lock_profile,
    rules::{self, Source},
    text::localized_text,
};

pub mod draw;
pub mod pool;
mod svc;

use pool::{DAILY_BONUS, DAILY_MISSIONS, Payout};

const DAILY: i16 = MissionPeriod::MISSION_PERIOD_DAILY as i16;
const WEEKLY: i16 = MissionPeriod::MISSION_PERIOD_WEEKLY as i16;

/// Days a finished mission can still be claimed after its period ended.
pub(crate) const CLAIM_DAYS: i64 = 7;
/// Rerolls per local day.
const DAILY_REROLLS: i16 = 1;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct MissionRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub period: i16,
    pub period_start: NaiveDate,
    pub slot: i16,
    pub mission_key: String,
    pub metric: String,
    pub target: i64,
    pub progress: i64,
    pub reward_coins: i32,
    pub reward_xp: i32,
    pub reward_badge: String,
    pub rerolled: bool,
    pub completed_at: Option<Time>,
    pub claimed_at: Option<Time>,
    pub paid_at: Option<Time>,
}

const MISSION_COLUMNS: &str = "id, user_id, period, period_start, slot, mission_key, metric, target, progress, reward_coins, reward_xp, reward_badge, rerolled, completed_at, claimed_at, paid_at";

/// The last day of a period starting on `start`.
fn period_end(period: i16, start: NaiveDate) -> NaiveDate {
    if period == WEEKLY { start + Duration::days(6) } else { start }
}

impl MissionRow {
    fn last_day(&self) -> NaiveDate {
        period_end(self.period, self.period_start)
    }

    /// When the mission's period is over for a player living in `zone`.
    fn ends_at(&self, zone: Tz) -> DateTime<Utc> {
        clock::end_of_day(self.last_day(), zone)
    }

    fn state(&self, zone: Tz, now: DateTime<Utc>) -> MissionState {
        if self.claimed_at.is_some() {
            MissionState::MISSION_STATE_CLAIMED
        } else if self.completed_at.is_some() {
            MissionState::MISSION_STATE_COMPLETED
        } else if now >= self.ends_at(zone) {
            MissionState::MISSION_STATE_EXPIRED
        } else {
            MissionState::MISSION_STATE_ACTIVE
        }
    }

    /// Whether the reward can (still) be collected at `now`.
    fn claimable(&self, zone: Tz, now: DateTime<Utc>) -> bool {
        self.completed_at.is_some() && now < self.ends_at(zone) + Duration::days(CLAIM_DAYS)
    }

    pub(crate) fn reward(&self) -> pb::Reward {
        let badge = catalog::find(&self.reward_badge).map(|badge| badges::to_proto(badge, None, None, None));
        pb::Reward { coins: self.reward_coins, xp: self.reward_xp, badge: badge.into(), ..Default::default() }
    }

    /// The mission as the API shows it to a player living in `zone`.
    pub(crate) fn to_proto(&self, zone: Tz, now: DateTime<Utc>) -> pb::Mission {
        // A mission that left the pool keeps working; only its text is generic.
        let def = pool::find(&self.mission_key);
        let (title, description, emoji, path, difficulty) = match def {
            Some(def) => (def.title.proto(), def.description.proto(), def.emoji, def.path, def.difficulty),
            None => (
                localized_text("ミッション", "Mission"),
                localized_text("", ""),
                "🎯",
                "",
                MissionDifficulty::MISSION_DIFFICULTY_UNSPECIFIED,
            ),
        };
        pb::Mission {
            id: self.id.to_string(),
            key: self.mission_key.clone(),
            period: (self.period as i32).into(),
            difficulty: difficulty.into(),
            title: title.into(),
            description: description.into(),
            emoji: emoji.to_owned(),
            progress: self.progress.clamp(0, self.target),
            target: self.target,
            state: self.state(zone, now).into(),
            reward: self.reward().into(),
            period_start: clock::date_text(self.period_start),
            ends_at: ts(self.ends_at(zone)),
            completed_at: opt_ts(self.completed_at),
            claimed_at: opt_ts(self.claimed_at),
            rerolled: self.rerolled,
            path: path.to_owned(),
            ..Default::default()
        }
    }
}

/// Brings the progress of the player's open missions that count one of
/// `metrics` and whose period contains `day` up to date. Returns the
/// missions that reached their target with this call.
pub(crate) async fn refresh_progress(
    tx: &mut PgConnection,
    user_id: Uuid,
    metrics: &[&str],
    day: NaiveDate,
) -> AppResult<Vec<MissionRow>> {
    let rows: Vec<MissionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "WITH counted AS (
             SELECT m.id, GREATEST(COALESCE((
                        SELECT sum(d.value) FROM quest_metric_days d
                        WHERE d.user_id = m.user_id AND d.metric = m.metric AND d.day >= m.period_start
                          AND d.day <= m.period_start + CASE m.period WHEN 2 THEN 6 ELSE 0 END
                    ), 0)::bigint - m.baseline, 0) AS done
             FROM quest_missions m
             WHERE m.user_id = $1 AND m.completed_at IS NULL AND m.metric = ANY($2)
               AND m.period_start <= $3 AND $3 <= m.period_start + CASE m.period WHEN 2 THEN 6 ELSE 0 END
         )
         UPDATE quest_missions m
         SET progress = LEAST(c.done, m.target),
             completed_at = CASE WHEN c.done >= m.target THEN now() END
         FROM counted c
         WHERE m.id = c.id AND (m.progress <> LEAST(c.done, m.target) OR c.done >= m.target)
         RETURNING {}",
        MISSION_COLUMNS.split(", ").map(|column| format!("m.{column}")).collect::<Vec<_>>().join(", ")
    )))
    .bind(user_id)
    .bind(metrics)
    .bind(day)
    .fetch_all(&mut *tx)
    .await?;
    Ok(rows.into_iter().filter(|row| row.completed_at.is_some()).collect())
}

/// What the player's `metric` adds up to from `start` to `end`.
async fn period_count(
    tx: &mut PgConnection,
    user_id: Uuid,
    metric: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> AppResult<i64> {
    Ok(sqlx::query_scalar("SELECT COALESCE(sum(value), 0)::bigint FROM quest_metric_days WHERE user_id = $1 AND metric = $2 AND day >= $3 AND day <= $4")
        .bind(user_id)
        .bind(metric)
        .bind(start)
        .bind(end)
        .fetch_one(&mut *tx)
        .await?)
}

/// A player's missions as `ListMissions` reports them.
pub(crate) struct MissionBoard {
    pub daily: Vec<MissionRow>,
    pub weekly: Option<MissionRow>,
    pub unclaimed: Vec<MissionRow>,
    pub rerolls_used: i16,
    pub bonus_granted: bool,
}

impl MissionBoard {
    pub(crate) fn daily_bonus(&self) -> pb::DailyBonus {
        daily_bonus(&self.daily, self.bonus_granted)
    }

    pub(crate) fn rerolls_left(&self) -> i32 {
        (DAILY_REROLLS - self.rerolls_used).max(0) as i32
    }
}

fn daily_bonus(daily: &[MissionRow], granted: bool) -> pb::DailyBonus {
    pb::DailyBonus {
        reward: pb::Reward { coins: DAILY_BONUS.coins, xp: DAILY_BONUS.xp, ..Default::default() }.into(),
        claimed: daily.iter().filter(|mission| mission.claimed_at.is_some()).count() as i32,
        required: daily.len() as i32,
        granted,
        ..Default::default()
    }
}

/// The outcome of a claim.
pub(crate) struct Claim {
    pub mission: MissionRow,
    /// False when the mission had been claimed before.
    pub granted: bool,
    /// Set when this claim completed the day.
    pub bonus: Option<Payout>,
    pub daily_bonus: pb::DailyBonus,
}

impl Quest {
    /// Keys of the rules that fired recently for anyone on this server.
    pub(crate) async fn live_rule_keys(&self) -> AppResult<std::sync::Arc<HashSet<&'static str>>> {
        if let Some(live) = self.live_rules.get(&()).await {
            return Ok(live);
        }
        let seen: Vec<String> = sqlx::query_scalar(
            "SELECT rule FROM quest_rule_activity WHERE last_seen_at >= now() - make_interval(days => $1)",
        )
        .bind(self.cfg.live_rule_days as i32)
        .fetch_all(&self.core.db)
        .await?;
        let live: HashSet<&'static str> =
            rules::RULES.iter().map(|rule| rule.key).filter(|key| seen.iter().any(|seen| seen == key)).collect();
        let live = std::sync::Arc::new(live);
        self.live_rules.insert((), live.clone()).await;
        Ok(live)
    }

    /// What the draw needs to know about a player.
    async fn player_for_draw(&self, user_id: Uuid) -> AppResult<draw::Player> {
        let friends = graph::friend_ids(&self.core, user_id).await?.len() as i32;
        let done: Vec<String> =
            sqlx::query_scalar("SELECT metric FROM quest_counters WHERE user_id = $1 AND value > 0")
                .bind(user_id)
                .fetch_all(&self.core.db)
                .await?;
        Ok(draw::Player {
            friends,
            done: done.iter().filter_map(|key| Metric::from_key(key)).collect(),
            live_rules: self.live_rule_keys().await?.as_ref().clone(),
        })
    }

    /// Makes sure the player has their missions for `today` and its week,
    /// drawing what is missing. Returns missions that are complete the
    /// moment they are drawn (the day's earlier activity counts).
    pub(crate) async fn ensure_missions(&self, user_id: Uuid, today: NaiveDate) -> AppResult<Vec<MissionRow>> {
        let week = clock::week_start(today);
        let have: Vec<i16> = sqlx::query_scalar("SELECT DISTINCT period FROM quest_missions WHERE user_id = $1 AND ((period = 1 AND period_start = $2) OR (period = 2 AND period_start = $3))")
            .bind(user_id)
            .bind(today)
            .bind(week)
            .fetch_all(&self.core.db)
            .await?;
        if have.contains(&DAILY) && have.contains(&WEEKLY) {
            return Ok(Vec::new());
        }
        // Everything the draw reads is fetched before the transaction opens.
        let player = self.player_for_draw(user_id).await?;
        let mut tx = self.core.db.begin().await?;
        // The profile lock makes two concurrent draws one.
        lock_profile(&mut tx, user_id).await?;
        let have: Vec<i16> = sqlx::query_scalar("SELECT DISTINCT period FROM quest_missions WHERE user_id = $1 AND ((period = 1 AND period_start = $2) OR (period = 2 AND period_start = $3))")
            .bind(user_id)
            .bind(today)
            .bind(week)
            .fetch_all(&mut *tx)
            .await?;
        let mut metrics: Vec<&'static str> = Vec::new();
        for (period, start, count) in [
            (MissionPeriod::MISSION_PERIOD_DAILY, today, DAILY_MISSIONS),
            (MissionPeriod::MISSION_PERIOD_WEEKLY, week, 1),
        ] {
            if have.contains(&(period as i16)) {
                continue;
            }
            let drawn = draw::draw(period, count, &player, draw::seed(user_id, start, period, 0));
            for (slot, mission) in drawn.iter().enumerate() {
                let payout = mission.payout();
                sqlx::query(
                    "INSERT INTO quest_missions (user_id, period, period_start, slot, mission_key, metric, target, reward_coins, reward_xp, reward_badge)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                     ON CONFLICT (user_id, period, period_start, slot) DO NOTHING",
                )
                .bind(user_id)
                .bind(period as i16)
                .bind(start)
                .bind(slot as i16)
                .bind(mission.key)
                .bind(mission.metric.key())
                .bind(mission.target)
                .bind(payout.coins)
                .bind(payout.xp)
                .bind(mission.badge)
                .execute(&mut *tx)
                .await?;
                metrics.push(mission.metric.key());
            }
        }
        sqlx::query(
            "INSERT INTO quest_mission_days (user_id, day) VALUES ($1, $2) ON CONFLICT (user_id, day) DO NOTHING",
        )
        .bind(user_id)
        .bind(today)
        .execute(&mut *tx)
        .await?;
        // What the player already did in the period counts from the start.
        let completed = refresh_progress(&mut tx, user_id, &metrics, today).await?;
        tx.commit().await?;
        Ok(completed)
    }

    /// The player's missions for `today`, drawn if needed.
    pub(crate) async fn mission_board(&self, user_id: Uuid, zone: Tz, now: DateTime<Utc>) -> AppResult<MissionBoard> {
        let today = clock::local_date(now, zone);
        let week = clock::week_start(today);
        let completed = self.ensure_missions(user_id, today).await?;
        if !completed.is_empty() {
            self.announce(user_id, Effects { missions: completed, ..Default::default() }).await;
        }
        let rows: Vec<MissionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {MISSION_COLUMNS} FROM quest_missions
             WHERE user_id = $1 AND ((period = 1 AND period_start = $2) OR (period = 2 AND period_start = $3)
                    OR (completed_at IS NOT NULL AND claimed_at IS NULL AND period_start >= $4))
             ORDER BY period, period_start DESC, slot"
        )))
        .bind(user_id)
        .bind(today)
        .bind(week)
        .bind(today - Duration::days(CLAIM_DAYS + 7))
        .fetch_all(&self.core.db)
        .await?;
        let day: Option<(i16, Option<Time>)> = sqlx::query_as(
            "SELECT rerolls_used, bonus_granted_at FROM quest_mission_days WHERE user_id = $1 AND day = $2",
        )
        .bind(user_id)
        .bind(today)
        .fetch_optional(&self.core.db)
        .await?;
        let (rerolls_used, bonus_granted_at) = day.unwrap_or((0, None));
        let mut board = MissionBoard {
            daily: Vec::new(),
            weekly: None,
            unclaimed: Vec::new(),
            rerolls_used,
            bonus_granted: bonus_granted_at.is_some(),
        };
        for row in rows {
            if row.period == DAILY && row.period_start == today {
                board.daily.push(row);
            } else if row.period == WEEKLY && row.period_start == week {
                board.weekly = Some(row);
            } else if row.claimable(zone, now) {
                board.unclaimed.push(row);
            }
        }
        Ok(board)
    }

    /// Collects the reward of a completed mission. Idempotent.
    pub(crate) async fn collect_mission(
        &self,
        user_id: Uuid,
        mission_id: Uuid,
        zone: Tz,
        now: DateTime<Utc>,
    ) -> AppResult<Claim> {
        let today = clock::local_date(now, zone);
        let mut tx = self.core.db.begin().await?;
        let mut profile = lock_profile(&mut tx, user_id).await?;
        let mission: Option<MissionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {MISSION_COLUMNS} FROM quest_missions WHERE id = $1 AND user_id = $2 FOR UPDATE"
        )))
        .bind(mission_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
        let mut mission = mission.or_not_found("MISSION_NOT_FOUND", "mission not found")?;
        let mut effects = Effects::starting_from(&profile);
        let mut bonus = None;
        let granted = mission.claimed_at.is_none();
        if granted {
            ensure!(
                mission.completed_at.is_some(),
                precondition,
                "MISSION_NOT_COMPLETED",
                "the mission is not completed yet"
            );
            ensure!(mission.claimable(zone, now), not_found, "MISSION_NOT_FOUND", "mission not found");
            mission.claimed_at = Some(now);
            sqlx::query("UPDATE quest_missions SET claimed_at = $2 WHERE id = $1")
                .bind(mission.id)
                .bind(now)
                .execute(&mut *tx)
                .await?;
            let earned = Earned {
                xp: vec![(Source::Missions, mission.reward_xp as i64)],
                metrics: vec![(Metric::MissionsClaimed, 1)],
            };
            self.apply(&mut tx, &mut profile, today, &earned, &mut effects).await?;
            if let Some(badge) = catalog::find(&mission.reward_badge)
                && badges::award_tx(&mut tx, user_id, badge, serde_json::json!({ "mission": mission.mission_key }))
                    .await?
            {
                effects.badges.push(badge);
            }
        }
        // The day's set, to see whether it is complete now.
        let daily: Vec<MissionRow> = if mission.period == DAILY {
            sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {MISSION_COLUMNS} FROM quest_missions WHERE user_id = $1 AND period = 1 AND period_start = $2 ORDER BY slot")))
                .bind(user_id)
                .bind(mission.period_start)
                .fetch_all(&mut *tx)
                .await?
        } else {
            Vec::new()
        };
        let mut bonus_granted = false;
        if mission.period == DAILY {
            let all_claimed = !daily.is_empty() && daily.iter().all(|mission| mission.claimed_at.is_some());
            if granted && all_claimed {
                let newly = sqlx::query(
                    "INSERT INTO quest_mission_days (user_id, day, bonus_granted_at, bonus_coins) VALUES ($1, $2, $3, $4)
                     ON CONFLICT (user_id, day) DO UPDATE SET bonus_granted_at = EXCLUDED.bonus_granted_at, bonus_coins = EXCLUDED.bonus_coins
                     WHERE quest_mission_days.bonus_granted_at IS NULL",
                )
                .bind(user_id)
                .bind(mission.period_start)
                .bind(now)
                .bind(DAILY_BONUS.coins)
                .execute(&mut *tx)
                .await?
                .rows_affected()
                    > 0;
                if newly {
                    let earned = Earned {
                        xp: vec![(Source::Missions, DAILY_BONUS.xp as i64)],
                        metrics: vec![(Metric::PerfectDays, 1)],
                    };
                    self.apply(&mut tx, &mut profile, today, &earned, &mut effects).await?;
                    bonus = Some(DAILY_BONUS);
                }
            }
            bonus_granted = sqlx::query_scalar(
                "SELECT bonus_granted_at IS NOT NULL FROM quest_mission_days WHERE user_id = $1 AND day = $2",
            )
            .bind(user_id)
            .bind(mission.period_start)
            .fetch_optional(&mut *tx)
            .await?
            .unwrap_or(false);
        }
        tx.commit().await?;

        // The coins, now that the claim is recorded. A failure here is not
        // the player's problem: the worker pays what stays unpaid.
        if mission.paid_at.is_none() {
            if let Err(e) = self.pay_mission(&mission).await {
                tracing::error!(%user_id, mission = %mission.id, error = %e, "quest: mission coins not paid yet; the worker will retry");
            }
        }
        if bonus_granted {
            if let Err(e) = self.pay_daily_bonus(user_id, mission.period_start).await {
                tracing::error!(%user_id, day = %mission.period_start, error = %e, "quest: daily bonus coins not paid yet; the worker will retry");
            }
        }
        if granted {
            let event = AnalyticsEvent::new("quest.mission.claim")
                .user(Some(user_id))
                .subject("mission", mission.id)
                .value(mission.reward_coins as f64)
                .props(serde_json::json!({ "mission": mission.mission_key, "period": mission.period, "xp": mission.reward_xp, "badge": mission.reward_badge }));
            self.core.analytics.track(event).await;
            if bonus.is_some() {
                let event = AnalyticsEvent::new("quest.mission.daily_bonus")
                    .user(Some(user_id))
                    .value(DAILY_BONUS.coins as f64)
                    .props(serde_json::json!({ "xp": DAILY_BONUS.xp }));
                self.core.analytics.track(event).await;
            }
        }
        self.announce(user_id, effects).await;
        // The set as it stands after this claim.
        let daily: Vec<MissionRow> =
            daily.into_iter().map(|row| if row.id == mission.id { mission.clone() } else { row }).collect();
        Ok(Claim { daily_bonus: daily_bonus(&daily, bonus_granted), mission, granted, bonus })
    }

    /// Pays a claimed mission's coins (idempotent per mission).
    async fn pay_mission(&self, mission: &MissionRow) -> AppResult<()> {
        if mission.reward_coins > 0 {
            let key = format!("quest.mission:{}", mission.id);
            ledger::grant(
                &self.core,
                mission.user_id,
                mission.reward_coins as i64,
                "quest.mission",
                &key,
                serde_json::json!({ "mission": mission.mission_key }),
            )
            .await?;
        }
        sqlx::query("UPDATE quest_missions SET paid_at = now() WHERE id = $1 AND paid_at IS NULL")
            .bind(mission.id)
            .execute(&self.core.db)
            .await?;
        Ok(())
    }

    /// Pays the bonus coins of a completed day: what the bonus was worth
    /// when it was granted (idempotent per player and day).
    async fn pay_daily_bonus(&self, user_id: Uuid, day: NaiveDate) -> AppResult<()> {
        let unpaid: Option<i32> = sqlx::query_scalar("SELECT bonus_coins FROM quest_mission_days WHERE user_id = $1 AND day = $2 AND bonus_granted_at IS NOT NULL AND bonus_paid_at IS NULL")
            .bind(user_id)
            .bind(day)
            .fetch_optional(&self.core.db)
            .await?;
        let Some(coins) = unpaid else { return Ok(()) };
        if coins > 0 {
            let key = format!("quest.daily_bonus:{user_id}:{}", clock::date_text(day));
            ledger::grant(
                &self.core,
                user_id,
                coins as i64,
                "quest.daily_bonus",
                &key,
                serde_json::json!({ "day": clock::date_text(day) }),
            )
            .await?;
        }
        sqlx::query("UPDATE quest_mission_days SET bonus_paid_at = now() WHERE user_id = $1 AND day = $2 AND bonus_paid_at IS NULL").bind(user_id).bind(day).execute(&self.core.db).await?;
        Ok(())
    }

    /// Pays what one player is still owed for their claims. Coins follow a
    /// claim right away; this is for the case that they did not (a process
    /// stopped in between), from wherever the player shows up next.
    pub(crate) async fn pay_unpaid_of(&self, user_id: Uuid) {
        let owed: AppResult<(Vec<MissionRow>, Vec<NaiveDate>)> = async {
            let missions = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {MISSION_COLUMNS} FROM quest_missions WHERE user_id = $1 AND claimed_at IS NOT NULL AND paid_at IS NULL")))
                .bind(user_id)
                .fetch_all(&self.core.db)
                .await?;
            let days = sqlx::query_scalar("SELECT day FROM quest_mission_days WHERE user_id = $1 AND bonus_granted_at IS NOT NULL AND bonus_paid_at IS NULL").bind(user_id).fetch_all(&self.core.db).await?;
            Ok((missions, days))
        }
        .await;
        let (missions, days) = match owed {
            Ok(owed) => owed,
            Err(e) => return tracing::warn!(%user_id, error = %e, "quest: unpaid claims not checked"),
        };
        for mission in &missions {
            if let Err(e) = self.pay_mission(mission).await {
                tracing::warn!(%user_id, mission = %mission.id, error = %e, "quest: mission coins still unpaid");
            }
        }
        for day in days {
            if let Err(e) = self.pay_daily_bonus(user_id, day).await {
                tracing::warn!(%user_id, %day, error = %e, "quest: daily bonus coins still unpaid");
            }
        }
    }

    /// Pays the coins of claims whose payment did not happen (a process
    /// stopped between the claim and the wallet). Returns how many were paid.
    pub async fn pay_stragglers(&self) -> AppResult<usize> {
        let missions: Vec<MissionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {MISSION_COLUMNS} FROM quest_missions WHERE claimed_at IS NOT NULL AND paid_at IS NULL AND claimed_at < now() - interval '1 minute' ORDER BY claimed_at LIMIT 200"
        )))
        .fetch_all(&self.core.db)
        .await?;
        let days: Vec<(Uuid, NaiveDate)> = sqlx::query_as(
            "SELECT user_id, day FROM quest_mission_days
             WHERE bonus_granted_at IS NOT NULL AND bonus_paid_at IS NULL AND bonus_granted_at < now() - interval '1 minute' ORDER BY bonus_granted_at LIMIT 200",
        )
        .fetch_all(&self.core.db)
        .await?;
        let mut paid = 0;
        for mission in &missions {
            match self.pay_mission(mission).await {
                Ok(()) => paid += 1,
                Err(e) => tracing::warn!(mission = %mission.id, error = %e, "quest: mission coins still unpaid"),
            }
        }
        for (user_id, day) in days {
            match self.pay_daily_bonus(user_id, day).await {
                Ok(()) => paid += 1,
                Err(e) => tracing::warn!(%user_id, %day, error = %e, "quest: daily bonus coins still unpaid"),
            }
        }
        Ok(paid)
    }

    /// Swaps an open mission of the current day or week for another one.
    /// Returns the new mission and the rerolls left today.
    pub(crate) async fn swap_mission(
        &self,
        user_id: Uuid,
        mission_id: Uuid,
        zone: Tz,
        now: DateTime<Utc>,
    ) -> AppResult<(MissionRow, i32)> {
        let today = clock::local_date(now, zone);
        let player = self.player_for_draw(user_id).await?;
        let mut tx = self.core.db.begin().await?;
        lock_profile(&mut tx, user_id).await?;
        let mission: Option<MissionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {MISSION_COLUMNS} FROM quest_missions WHERE id = $1 AND user_id = $2 FOR UPDATE"
        )))
        .bind(mission_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
        let mission = mission.or_not_found("MISSION_NOT_FOUND", "mission not found")?;
        let current = mission.period_start <= today && today <= mission.last_day();
        ensure!(
            current && mission.completed_at.is_none() && mission.claimed_at.is_none(),
            precondition,
            "MISSION_NOT_ACTIVE",
            "only an open mission of the current day or week can be rerolled"
        );

        sqlx::query(
            "INSERT INTO quest_mission_days (user_id, day) VALUES ($1, $2) ON CONFLICT (user_id, day) DO NOTHING",
        )
        .bind(user_id)
        .bind(today)
        .execute(&mut *tx)
        .await?;
        let used: i16 = sqlx::query_scalar(
            "SELECT rerolls_used FROM quest_mission_days WHERE user_id = $1 AND day = $2 FOR UPDATE",
        )
        .bind(user_id)
        .bind(today)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(
            used < DAILY_REROLLS,
            precondition,
            "NO_REROLL_LEFT",
            "today's reroll is used; a new one comes tomorrow"
        );

        let others: Vec<String> = sqlx::query_scalar(
            "SELECT metric FROM quest_missions WHERE user_id = $1 AND period = $2 AND period_start = $3 AND id <> $4",
        )
        .bind(user_id)
        .bind(mission.period)
        .bind(mission.period_start)
        .bind(mission.id)
        .fetch_all(&mut *tx)
        .await?;
        let taken: Vec<Metric> = others.iter().filter_map(|key| Metric::from_key(key)).collect();
        let period = if mission.period == WEEKLY {
            MissionPeriod::MISSION_PERIOD_WEEKLY
        } else {
            MissionPeriod::MISSION_PERIOD_DAILY
        };
        let difficulty = pool::find(&mission.mission_key)
            .map(|def| def.difficulty)
            .unwrap_or(MissionDifficulty::MISSION_DIFFICULTY_NORMAL);
        // A metric that left the code cannot be drawn again anyway.
        let replaced_metric = Metric::from_key(&mission.metric);
        let seed = draw::seed(user_id, mission.period_start, period, 1 + mission.slot as u32);
        let replacement = match replaced_metric {
            Some(metric) => draw::replacement(period, difficulty, &mission.mission_key, metric, &taken, &player, seed),
            None => draw::draw(period, 1, &player, seed).into_iter().find(|def| !taken.contains(&def.metric)),
        };
        let Some(new) = replacement else {
            bail!(precondition, "NO_OTHER_MISSION", "no other mission suits you right now");
        };
        // The new mission starts from zero: what was counted so far is its baseline.
        let baseline =
            period_count(&mut tx, user_id, new.metric.key(), mission.period_start, mission.last_day()).await?;
        let payout = new.payout();
        let row: MissionRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE quest_missions SET mission_key = $2, metric = $3, target = $4, baseline = $5, progress = 0,
                    reward_coins = $6, reward_xp = $7, reward_badge = $8, rerolled = true
             WHERE id = $1 RETURNING {MISSION_COLUMNS}"
        )))
        .bind(mission.id)
        .bind(new.key)
        .bind(new.metric.key())
        .bind(new.target)
        .bind(baseline)
        .bind(payout.coins)
        .bind(payout.xp)
        .bind(new.badge)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("UPDATE quest_mission_days SET rerolls_used = rerolls_used + 1 WHERE user_id = $1 AND day = $2")
            .bind(user_id)
            .bind(today)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        let event = AnalyticsEvent::new("quest.mission.reroll")
            .user(Some(user_id))
            .subject("mission", row.id)
            .props(serde_json::json!({ "from": mission.mission_key, "to": row.mission_key }));
        self.core.analytics.track(event).await;
        Ok((row, (DAILY_REROLLS - used - 1).max(0) as i32))
    }

    /// Deletes missions and per-day rows that can no longer be claimed or shown.
    pub(crate) async fn prune_missions(&self) -> AppResult<u64> {
        let before = Utc::now().date_naive() - Duration::days(CLAIM_DAYS + 21);
        let missions = sqlx::query(
            "DELETE FROM quest_missions WHERE period_start < $1 AND (claimed_at IS NULL OR paid_at IS NOT NULL)",
        )
        .bind(before)
        .execute(&self.core.db)
        .await?
        .rows_affected();
        sqlx::query(
            "DELETE FROM quest_mission_days WHERE day < $1 AND (bonus_granted_at IS NULL OR bonus_paid_at IS NOT NULL)",
        )
        .bind(before)
        .execute(&self.core.db)
        .await?;
        Ok(missions)
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    const TOKYO: Tz = chrono_tz::Asia::Tokyo;

    fn row(period: i16, start: NaiveDate) -> MissionRow {
        MissionRow {
            id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
            period,
            period_start: start,
            slot: 0,
            mission_key: "stamps_5".into(),
            metric: "stamps_sent".into(),
            target: 5,
            progress: 2,
            reward_coins: 10,
            reward_xp: 20,
            reward_badge: String::new(),
            rerolled: false,
            completed_at: None,
            claimed_at: None,
            paid_at: None,
        }
    }

    fn tokyo(d: u32, h: u32) -> DateTime<Utc> {
        TOKYO.with_ymd_and_hms(2026, 11, d, h, 0, 0).unwrap().with_timezone(&Utc)
    }

    fn date(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }

    #[test]
    fn a_daily_mission_ends_at_local_midnight_and_a_weekly_one_on_sunday_night() {
        let daily = row(DAILY, date(18));
        assert_eq!(daily.ends_at(TOKYO), tokyo(19, 0));
        // The week of Monday the 16th ends when Sunday the 22nd does.
        let weekly = row(WEEKLY, date(16));
        assert_eq!(weekly.ends_at(TOKYO), tokyo(23, 0));
        assert_eq!(weekly.last_day(), date(22));
    }

    #[test]
    fn states_follow_progress_and_the_clock() {
        let mut mission = row(DAILY, date(18));
        assert_eq!(mission.state(TOKYO, tokyo(18, 12)), MissionState::MISSION_STATE_ACTIVE);
        assert_eq!(mission.state(TOKYO, tokyo(19, 0)), MissionState::MISSION_STATE_EXPIRED);
        mission.completed_at = Some(tokyo(18, 20));
        assert_eq!(mission.state(TOKYO, tokyo(19, 9)), MissionState::MISSION_STATE_COMPLETED);
        // Claimable for a week after the day ended, not longer.
        assert!(mission.claimable(TOKYO, tokyo(25, 23)));
        assert!(!mission.claimable(TOKYO, tokyo(26, 0)));
        mission.claimed_at = Some(tokyo(19, 9));
        assert_eq!(mission.state(TOKYO, tokyo(19, 10)), MissionState::MISSION_STATE_CLAIMED);
    }

    #[test]
    fn the_api_message_carries_text_reward_and_clamped_progress() {
        let mut mission = row(DAILY, date(18));
        mission.progress = 9;
        mission.reward_badge = "mission_social".into();
        let proto = mission.to_proto(TOKYO, tokyo(18, 12));
        assert_eq!((proto.progress, proto.target), (5, 5));
        assert_eq!(proto.title.as_option().unwrap().ja, "スタンプを5回送ろう");
        assert_eq!(proto.difficulty, MissionDifficulty::MISSION_DIFFICULTY_NORMAL);
        assert_eq!(proto.period, MissionPeriod::MISSION_PERIOD_DAILY);
        assert_eq!(proto.period_start, "2026-11-18");
        let reward = proto.reward.as_option().unwrap();
        assert_eq!((reward.coins, reward.xp), (10, 20));
        assert_eq!(reward.badge.as_option().unwrap().key, "mission_social");
        // A mission whose definition left the pool still renders.
        mission.mission_key = "retired_mission".into();
        let proto = mission.to_proto(TOKYO, tokyo(18, 12));
        assert_eq!(proto.title.as_option().unwrap().en, "Mission");
        assert_eq!(proto.key, "retired_mission");
    }

    #[test]
    fn the_bonus_needs_every_daily_mission_claimed() {
        let mut set = vec![row(DAILY, date(18)), row(DAILY, date(18)), row(DAILY, date(18))];
        set[0].claimed_at = Some(tokyo(18, 10));
        let bonus = daily_bonus(&set, false);
        assert_eq!((bonus.claimed, bonus.required, bonus.granted), (1, 3, false));
        assert_eq!(bonus.reward.as_option().unwrap().coins, DAILY_BONUS.coins);
    }
}
