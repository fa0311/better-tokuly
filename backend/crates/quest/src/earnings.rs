//! Applying what a player earned, in PostgreSQL.
//!
//! Everything that changes a player's numbers goes through [`Quest::apply`],
//! inside the caller's transaction and with the player's profile row locked:
//! the flush of the Valkey counters, a mission claim, a completed streak
//! day. One call adds XP per source and metric counts for one local day and
//! works out what follows from them — the new level, missions that reached
//! their target, counted badges whose threshold was crossed.
//!
//! What follows is collected in [`Effects`] and announced by
//! [`Quest::announce`] once the transaction has committed: the coins for a
//! level-up, the realtime pushes, the analytics events.
use std::collections::BTreeMap;

use chrono::NaiveDate;
use sqlx::PgConnection;
use tokuly_core::{
    analytics::AnalyticsEvent,
    outbound::{self, ActivityEntry, Localized},
    prelude::*,
};
use tokuly_proto::quest::{self as pb, quest_event::Event as Push};
use tokuly_wallet::ledger;

use crate::{
    Quest,
    badges::{
        self,
        catalog::{BadgeDef, CATALOG, Criterion},
    },
    levels,
    metrics::Metric,
    missions::{self, MissionRow},
    profile::ProfileRow,
    rules::Source,
};

/// Levels that make an activity entry: every this-many-th.
const FEED_LEVEL_STEP: i32 = 5;

/// What to add to a player for one local day.
#[derive(Debug, Default, Clone)]
pub(crate) struct Earned {
    pub xp: Vec<(Source, i64)>,
    pub metrics: Vec<(Metric, i64)>,
}

/// What a player's changes set in motion.
#[derive(Debug, Default)]
pub(crate) struct Effects {
    /// XP added, per source.
    pub xp: BTreeMap<Source, i64>,
    pub level_before: i32,
    pub level_after: i32,
    pub total_xp: i64,
    /// Highest level whose coins were granted so far.
    pub rewarded_level: i32,
    /// Missions that reached their target.
    pub missions: Vec<MissionRow>,
    /// Badges earned.
    pub badges: Vec<&'static BadgeDef>,
}

impl Effects {
    /// Effects starting from a player's profile as it was locked.
    pub(crate) fn starting_from(profile: &ProfileRow) -> Self {
        Self {
            level_before: profile.level,
            level_after: profile.level,
            total_xp: profile.xp,
            rewarded_level: profile.rewarded_level,
            ..Default::default()
        }
    }

    pub(crate) fn xp_total(&self) -> i64 {
        self.xp.values().sum()
    }

    fn is_empty(&self) -> bool {
        self.xp.is_empty()
            && self.missions.is_empty()
            && self.badges.is_empty()
            && self.level_after <= self.rewarded_level
    }
}

impl Quest {
    /// Adds `earned` to the player's `day`. `profile` must be the row locked
    /// in this transaction ([`crate::profile::lock_profile`]); it is updated
    /// in place, so several calls can follow each other.
    pub(crate) async fn apply(
        &self,
        tx: &mut PgConnection,
        profile: &mut ProfileRow,
        day: NaiveDate,
        earned: &Earned,
        effects: &mut Effects,
    ) -> AppResult<()> {
        let user_id = profile.user_id;
        let xp: Vec<(Source, i64)> = earned.xp.iter().copied().filter(|(_, xp)| *xp > 0).collect();
        let metrics: Vec<(Metric, i64)> = earned.metrics.iter().copied().filter(|(_, amount)| *amount > 0).collect();
        if xp.is_empty() && metrics.is_empty() {
            return Ok(());
        }

        if !xp.is_empty() {
            let sources: Vec<&str> = xp.iter().map(|(source, _)| source.key()).collect();
            let amounts: Vec<i32> = xp.iter().map(|(_, xp)| (*xp).min(i32::MAX as i64) as i32).collect();
            sqlx::query(
                "INSERT INTO quest_xp_days (user_id, day, source, xp)
                 SELECT $1, $2, source, xp FROM UNNEST($3::text[], $4::int[]) AS added (source, xp)
                 ON CONFLICT (user_id, day, source) DO UPDATE SET xp = quest_xp_days.xp + EXCLUDED.xp",
            )
            .bind(user_id)
            .bind(day)
            .bind(&sources)
            .bind(&amounts)
            .execute(&mut *tx)
            .await?;
            for (source, amount) in &xp {
                *effects.xp.entry(*source).or_default() += amount;
            }
        }

        // Counts: the day's row (missions, weekly and seasonal boards) and
        // the lifetime total (badges, all-time boards).
        let mut totals: Vec<(String, i64)> = Vec::new();
        if !metrics.is_empty() {
            let keys: Vec<&str> = metrics.iter().map(|(metric, _)| metric.key()).collect();
            let amounts: Vec<i64> = metrics.iter().map(|(_, amount)| *amount).collect();
            sqlx::query(
                "INSERT INTO quest_metric_days (user_id, day, metric, value)
                 SELECT $1, $2, metric, value FROM UNNEST($3::text[], $4::bigint[]) AS added (metric, value)
                 ON CONFLICT (user_id, day, metric) DO UPDATE SET value = quest_metric_days.value + EXCLUDED.value",
            )
            .bind(user_id)
            .bind(day)
            .bind(&keys)
            .bind(&amounts)
            .execute(&mut *tx)
            .await?;
            totals = sqlx::query_as(
                "INSERT INTO quest_counters (user_id, metric, value)
                 SELECT $1, metric, value FROM UNNEST($2::text[], $3::bigint[]) AS added (metric, value)
                 ON CONFLICT (user_id, metric) DO UPDATE SET value = quest_counters.value + EXCLUDED.value
                 RETURNING metric, value",
            )
            .bind(user_id)
            .bind(&keys)
            .bind(&amounts)
            .fetch_all(&mut *tx)
            .await?;
        }

        // The profile: total XP, the level it amounts to, the last active day.
        let gained: i64 = xp.iter().map(|(_, xp)| xp).sum();
        let level_before = profile.level;
        profile.xp += gained;
        profile.level = levels::level_for(profile.xp).max(profile.level);
        profile.last_active_day = Some(profile.last_active_day.map_or(day, |last| last.max(day)));
        sqlx::query("UPDATE quest_profiles SET xp = $2, level = $3, last_active_day = $4 WHERE user_id = $1")
            .bind(user_id)
            .bind(profile.xp)
            .bind(profile.level)
            .bind(profile.last_active_day)
            .execute(&mut *tx)
            .await?;
        effects.total_xp = profile.xp;
        effects.level_after = profile.level;

        // Missions counting any of these metrics during this day.
        if !metrics.is_empty() {
            let keys: Vec<&str> = metrics.iter().map(|(metric, _)| metric.key()).collect();
            effects.missions.extend(missions::refresh_progress(&mut *tx, user_id, &keys, day).await?);
        }

        // Counted badges whose threshold this call crossed.
        for (metric, amount) in &metrics {
            let Some(total) = totals.iter().find(|(key, _)| key == metric.key()).map(|(_, total)| *total) else {
                continue;
            };
            let before = total - amount;
            for badge in CATALOG.iter() {
                if let Criterion::Count(counted, threshold) = badge.criterion
                    && counted == *metric
                    && before < threshold
                    && threshold <= total
                    && badges::award_tx(&mut *tx, user_id, badge, serde_json::Value::Null).await?
                {
                    effects.badges.push(badge);
                }
            }
        }
        if profile.level > level_before {
            // Every fifth level is news for the friends' feed.
            for level in (level_before + 1..=profile.level).filter(|level| level % FEED_LEVEL_STEP == 0) {
                let summary = Localized::new(format!("レベル{level}になりました!"), format!("Reached level {level}"));
                let entry = ActivityEntry::new(
                    user_id,
                    "quest.level_up",
                    badges::activity_audience(profile.show_on_profile),
                    summary,
                )
                .path("/quest")
                .data(serde_json::json!({ "level": level }));
                outbound::activity(&mut *tx, &entry).await?;
            }
            for badge in CATALOG.iter() {
                if let Criterion::Level(level) = badge.criterion
                    && level_before < level
                    && level <= profile.level
                    && badges::award_tx(&mut *tx, user_id, badge, serde_json::Value::Null).await?
                {
                    effects.badges.push(badge);
                }
            }
        }
        Ok(())
    }

    /// Announces what a committed change set in motion. Never fails the
    /// caller: the change is stored, and everything here is either
    /// repeatable (the level-up coins, retried with the player's next
    /// earnings) or best effort (pushes, analytics).
    pub(crate) async fn announce(&self, user_id: Uuid, effects: Effects) {
        if effects.is_empty() {
            return;
        }
        let coins = match self.reward_levels(user_id, effects.rewarded_level, effects.level_after).await {
            Ok(coins) => coins,
            Err(e) => {
                tracing::error!(%user_id, error = %e, "quest: level-up reward not granted; it will be retried");
                0
            }
        };
        let progress = levels::progress(effects.total_xp);
        let gained = effects.xp_total();
        if gained > 0 {
            let xp_today = self.xp_today(user_id).await.unwrap_or(0);
            let sources = effects
                .xp
                .iter()
                .map(|(source, xp)| pb::XpDaySource {
                    source: source.key().to_owned(),
                    xp: *xp as i32,
                    ..Default::default()
                })
                .collect();
            let pushed = pb::XpGained {
                xp: gained as i32,
                sources,
                level: progress.clone().into(),
                xp_today: xp_today as i32,
                ..Default::default()
            };
            self.push(user_id, Push::XpGained(Box::new(pushed))).await;
            for (source, xp) in &effects.xp {
                let event = AnalyticsEvent::new("quest.xp")
                    .user(Some(user_id))
                    .value(*xp as f64)
                    .props(serde_json::json!({ "source": source.key() }));
                self.core.analytics.track(event).await;
            }
        }
        if effects.level_after > effects.level_before {
            let pushed = pb::LevelUp {
                level: effects.level_after,
                coins: coins as i32,
                progress: progress.into(),
                ..Default::default()
            };
            self.push(user_id, Push::LevelUp(Box::new(pushed))).await;
            let event = AnalyticsEvent::new("quest.level_up")
                .user(Some(user_id))
                .value(effects.level_after as f64)
                .props(serde_json::json!({ "from": effects.level_before, "coins": coins }));
            self.core.analytics.track(event).await;
        }
        if !effects.missions.is_empty() {
            let zone = self.zone_of(user_id).await.ok().flatten().unwrap_or(crate::clock::DEFAULT_ZONE);
            let now = now();
            for mission in &effects.missions {
                self.push(user_id, Push::MissionCompleted(Box::new(mission.to_proto(zone, now)))).await;
                let event = AnalyticsEvent::new("quest.mission.complete")
                    .user(Some(user_id))
                    .subject("mission", mission.id)
                    .props(serde_json::json!({ "mission": mission.mission_key, "period": mission.period }));
                self.core.analytics.track(event).await;
            }
        }
        for badge in effects.badges {
            badges::announce_unlocked(&self.core, user_id, badge).await;
        }
    }

    /// Grants the coins of every level above `rewarded` up to `level` and
    /// remembers how far it got. Each grant is idempotent per level, so
    /// repeating this after a failure never pays twice. Returns the coins
    /// granted by this call.
    async fn reward_levels(&self, user_id: Uuid, rewarded: i32, level: i32) -> AppResult<i64> {
        let mut coins = 0;
        for reached in (rewarded + 1)..=level {
            let amount = levels::level_up_coins(reached, self.cfg.level_up_coins, self.cfg.milestone_level_coins);
            if amount > 0 {
                let key = format!("quest.level:{user_id}:{reached}");
                let (_, applied) = ledger::grant(
                    &self.core,
                    user_id,
                    amount,
                    "quest.level_up",
                    &key,
                    serde_json::json!({ "level": reached }),
                )
                .await?;
                if applied {
                    coins += amount;
                }
            }
        }
        if level > rewarded {
            sqlx::query("UPDATE quest_profiles SET rewarded_level = GREATEST(rewarded_level, $2) WHERE user_id = $1")
                .bind(user_id)
                .bind(level)
                .execute(&self.core.db)
                .await?;
        }
        Ok(coins)
    }

    /// XP the player earned on their current local day.
    pub(crate) async fn xp_today(&self, user_id: Uuid) -> AppResult<i64> {
        let zone = self.zone_of(user_id).await?.unwrap_or(crate::clock::DEFAULT_ZONE);
        let today = crate::clock::local_date(now(), zone);
        Ok(sqlx::query_scalar("SELECT COALESCE(sum(xp), 0)::bigint FROM quest_xp_days WHERE user_id = $1 AND day = $2")
            .bind(user_id)
            .bind(today)
            .fetch_one(&self.core.db)
            .await?)
    }
}
