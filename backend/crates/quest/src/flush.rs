//! Writing the Valkey counters to PostgreSQL.
//!
//! A day with changes waits in the `dirty` queue for
//! `TOKULY_QUEST_FLUSH_DELAY_MS` and is then written in one transaction per
//! player: whatever the day's counters hold beyond what PostgreSQL already
//! has is applied (`earnings.rs`), which is also where levels, mission
//! progress and badges move. So a burst of events costs one transaction,
//! and the player gets one "XP gained" push for it.
//!
//! Writing is safe to repeat and safe to interrupt: the counters are
//! absolute, only the difference is applied, and a day leaves the queue
//! only once it was written and has not changed since.
use std::collections::BTreeMap;

use chrono::NaiveDate;
use futures::StreamExt as _;
use tokuly_core::prelude::*;

use crate::{
    Quest, badges, clock,
    earnings::{Earned, Effects},
    metrics::Metric,
    profile::lock_profile,
    rules::Source,
    store::{DayCounters, DirtyDay},
};

/// Days taken off the queue per round.
const BATCH: isize = 400;
/// Players written at the same time by one process.
const CONCURRENCY: usize = 4;
/// How long [`Quest::flush_user`] waits for another process that is writing the same player.
const LOCK_WAIT_ROUNDS: u32 = 100;
const LOCK_WAIT_STEP: std::time::Duration = std::time::Duration::from_millis(50);

impl Quest {
    /// Writes the days whose delay is over. Returns how many players were
    /// written. Several processes may run this; a player is written by one
    /// of them at a time.
    pub async fn flush_due(&self) -> AppResult<usize> {
        let due = self.due_days(false, BATCH).await?;
        let mut by_player: BTreeMap<Uuid, Vec<DirtyDay>> = BTreeMap::new();
        for entry in due {
            by_player.entry(entry.user).or_default().push(entry);
        }
        let written = futures::stream::iter(by_player)
            .map(|(user, days)| async move {
                let Some(lock) = self.try_lock_flush(user).await? else { return Ok::<bool, AppError>(false) };
                let result = self.write_days(user, &days).await;
                self.unlock_flush(lock).await;
                result.map(|()| true)
            })
            .buffer_unordered(CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        let mut count = 0;
        for result in written {
            match result {
                Ok(true) => count += 1,
                Ok(false) => {}
                // The day stays queued and is tried again next round.
                Err(e) => tracing::error!(error = %e, "quest: writing a player's earnings failed; will retry"),
            }
        }
        Ok(count)
    }

    /// Writes everything collected for one player right now, without
    /// waiting for the delay: before an RPC reads the player's numbers, and
    /// in tests after feeding events.
    pub async fn flush_user(&self, user_id: Uuid) -> AppResult<()> {
        let days = self.dirty_days_of(user_id).await?;
        if days.is_empty() {
            return Ok(());
        }
        for _ in 0..LOCK_WAIT_ROUNDS {
            if let Some(lock) = self.try_lock_flush(user_id).await? {
                // Whoever held the lock may have written them already.
                let days = self.dirty_days_of(user_id).await?;
                let result = if days.is_empty() { Ok(()) } else { self.write_days(user_id, &days).await };
                self.unlock_flush(lock).await;
                return result;
            }
            tokio::time::sleep(LOCK_WAIT_STEP).await;
        }
        Err(AppError::unavailable("QUEST_BUSY", "your progress is being updated; try again in a moment"))
    }

    /// Writes the given days of one player. The caller holds the flush lock.
    async fn write_days(&self, user: Uuid, days: &[DirtyDay]) -> AppResult<()> {
        let Some(zone) = self.zone_of(user).await? else {
            // The account is gone: there is nobody to credit.
            for entry in days {
                self.discard_day(*entry).await?;
            }
            return Ok(());
        };
        let today = clock::local_date(now(), zone);
        // Today's missions exist before today's progress is counted, so a
        // completion can be pushed to a player who has not opened them yet.
        let mut completed_on_draw = Vec::new();
        if days.iter().any(|entry| entry.day == today) && self.first_sight_of_day(user, today).await? {
            match self.ensure_missions(user, today).await {
                Ok(completed) => completed_on_draw = completed,
                Err(e) => {
                    tracing::warn!(%user, error = %e, "quest: missions not drawn; they will be when the player opens them")
                }
            }
        }
        let birthdate: Option<NaiveDate> = sqlx::query_scalar("SELECT birthdate FROM users WHERE id = $1")
            .bind(user)
            .fetch_optional(&self.core.db)
            .await?
            .flatten();
        let mut read = Vec::with_capacity(days.len());
        for entry in days {
            read.push((*entry, self.read_day(*entry).await?));
        }

        let mut tx = self.core.db.begin().await?;
        let mut profile = lock_profile(&mut tx, user).await?;
        let mut effects = Effects::starting_from(&profile);
        effects.missions = completed_on_draw;
        for (entry, counters) in &read {
            let earned = self.unwritten(&mut tx, *entry, counters).await?;
            // The last active day before this one, for the comeback badge.
            let previous_active_day = profile.last_active_day.filter(|previous| *previous < entry.day);
            self.apply(&mut tx, &mut profile, entry.day, &earned, &mut effects).await?;
            for badge in badges::day_badges(entry.day, counters, birthdate, previous_active_day) {
                if badges::award_tx(&mut tx, user, badge, serde_json::Value::Null).await? {
                    effects.badges.push(badge);
                }
            }
        }
        tx.commit().await?;
        for (entry, counters) in &read {
            self.mark_written(*entry, &counters.version).await?;
        }
        self.announce(user, effects).await;
        Ok(())
    }

    /// What a day's counters hold beyond what PostgreSQL has for that day.
    async fn unwritten(
        &self,
        tx: &mut sqlx::PgConnection,
        entry: DirtyDay,
        counters: &DayCounters,
    ) -> AppResult<Earned> {
        let stored: Vec<(String, i64)> = sqlx::query_as(
            "SELECT 'x:' || source, xp::bigint FROM quest_xp_days WHERE user_id = $1 AND day = $2
             UNION ALL
             SELECT 'm:' || metric, value FROM quest_metric_days WHERE user_id = $1 AND day = $2",
        )
        .bind(entry.user)
        .bind(entry.day)
        .fetch_all(&mut *tx)
        .await?;
        let stored =
            DayCounters::from_fields(stored.into_iter().map(|(field, value)| (field, value.to_string())).collect());
        Ok(difference(counters, &stored))
    }
}

/// What `counters` hold beyond `stored`, field by field. A field that is
/// behind `stored` (the counters were lost and reloaded) contributes nothing.
fn difference(counters: &DayCounters, stored: &DayCounters) -> Earned {
    let xp: Vec<(Source, i64)> = counters
        .xp
        .iter()
        .map(|(source, value)| (*source, value - stored.xp.get(source).copied().unwrap_or(0)))
        .filter(|(_, delta)| *delta > 0)
        .collect();
    let metrics: Vec<(Metric, i64)> = counters
        .metrics
        .iter()
        .map(|(metric, value)| (*metric, value - stored.metrics.get(metric).copied().unwrap_or(0)))
        .filter(|(_, delta)| *delta > 0)
        .collect();
    Earned { xp, metrics }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counters(xp: &[(Source, i64)], metrics: &[(Metric, i64)]) -> DayCounters {
        DayCounters {
            xp: xp.iter().copied().collect(),
            metrics: metrics.iter().copied().collect(),
            ..Default::default()
        }
    }

    #[test]
    fn only_what_is_new_is_written() {
        let now = counters(&[(Source::Chat, 12), (Source::Map, 10)], &[(Metric::Messages, 6), (Metric::Xp, 22)]);
        let stored = counters(&[(Source::Chat, 8)], &[(Metric::Messages, 4), (Metric::Xp, 8)]);
        let earned = difference(&now, &stored);
        assert_eq!(earned.xp, vec![(Source::Chat, 4), (Source::Map, 10)]);
        assert_eq!(earned.metrics, vec![(Metric::Messages, 2), (Metric::Xp, 14)]);
    }

    #[test]
    fn writing_the_same_counters_twice_adds_nothing() {
        let now = counters(&[(Source::Chat, 12)], &[(Metric::Messages, 6)]);
        let earned = difference(&now, &now);
        assert!(earned.xp.is_empty() && earned.metrics.is_empty());
    }

    #[test]
    fn counters_behind_the_database_never_take_anything_away() {
        // Valkey lost the day and was reloaded from an older state.
        let now = counters(&[(Source::Chat, 2)], &[(Metric::Messages, 1)]);
        let stored = counters(
            &[(Source::Chat, 40), (Source::Missions, 30)],
            &[(Metric::Messages, 20), (Metric::MissionsClaimed, 2)],
        );
        let earned = difference(&now, &stored);
        assert!(earned.xp.is_empty() && earned.metrics.is_empty());
    }
}
