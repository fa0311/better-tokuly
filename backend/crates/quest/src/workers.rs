//! Background work of the quest domain.
//!
//! | Worker | Schedule | What it does |
//! |---|---|---|
//! | event consumer `quest` | on events | `Quest::handle_event` for every topic of the rule table and the housekeeping topics |
//! | analytics consumer `quest` | on analytics events | `Quest::handle_analytics` for the analytics names of the rule table |
//! | flush | every 500 ms | `Quest::flush_due`: Valkey counters whose delay is over → PostgreSQL |
//! | streak sweep | every minute | `Quest::sweep_streaks`: ends overdue streaks, sends "expires soon" reminders |
//! | payouts | every minute | `Quest::pay_stragglers`, `Quest::reward_stragglers`, `Quest::settle_pending_restores`: coins a stopped process left unpaid, restores it left unfinished |
//! | seasons | every 10 minutes | `Quest::finalize_due_seasons` |
//! | rarity | every `TOKULY_QUEST_RARITY_INTERVAL_SECONDS` | `Quest::recompute_rarity` |
//! | pruning | every 6 hours | old missions, old per-day metric rows |
use std::time::Duration;

use async_nats::jetstream::AckKind;
use futures::StreamExt as _;
use tokuly_core::{
    analytics::AnalyticsEvent,
    bus::STREAM_ANALYTICS,
    events::{self, ConsumeOptions},
    prelude::*,
};

use crate::{
    Quest, ingest,
    rules::{self, Feed},
};

/// Durable name of both consumers (one per stream).
const DURABLE: &str = "quest";
/// Analytics events fetched per round, and handled at the same time.
const ANALYTICS_BATCH: usize = 200;
const ANALYTICS_CONCURRENCY: usize = 16;
/// Days of per-day metric rows kept: the longest period anything reads (a
/// season) plus the time its results stay open.
const METRIC_DAYS_KEPT: i64 = 70;

impl Quest {
    /// Starts the quest workers (worker processes only).
    pub fn spawn_workers(&self) {
        let quest = self.clone();
        tokio::spawn(async move {
            let handler = quest.clone();
            let topics = ingest::event_topics();
            // Deeds are cheap and independent: handle many at once.
            let options = ConsumeOptions { concurrency: 32, ..Default::default() };
            events::consume(
                quest.core.bus.clone(),
                DURABLE,
                &topics,
                options,
                quest.core.shutdown.clone(),
                move |event| {
                    let quest = handler.clone();
                    async move { quest.handle_event(event).await }
                },
            )
            .await;
        });

        let quest = self.clone();
        tokio::spawn(async move { quest.run_analytics_consumer().await });

        self.every("flush", Duration::from_millis(500), Duration::ZERO, |quest| async move {
            // Keep going while there is a backlog.
            while quest.flush_due().await? > 0 && !quest.core.shutdown.is_cancelled() {}
            Ok(())
        });
        // The sweep waits out the grace after a start, so events that queued
        // up while the workers were down are handled before any streak ends.
        let grace = Duration::from_secs(self.cfg.streak_grace_minutes.max(0) as u64 * 60);
        self.every("streak sweep", Duration::from_secs(60), grace, |quest| async move {
            quest.sweep_streaks().await.map(|_| ())
        });
        self.every("payouts", Duration::from_secs(60), Duration::from_secs(30), |quest| async move {
            quest.pay_stragglers().await?;
            quest.settle_pending_restores().await?;
            quest.reward_stragglers().await.map(|_| ())
        });
        self.every("seasons", Duration::from_secs(600), Duration::from_secs(20), |quest| async move {
            quest.finalize_due_seasons().await.map(|_| ())
        });
        self.every(
            "badge rarity",
            Duration::from_secs(self.cfg.rarity_interval_seconds),
            Duration::from_secs(10),
            |quest| async move { quest.recompute_rarity().await },
        );
        self.every("pruning", Duration::from_secs(6 * 3600), Duration::from_secs(120), |quest| async move {
            quest.prune().await
        });
    }

    /// Runs `job` every `period` (first after `delay`) until shutdown,
    /// logging failures.
    fn every<F, Fut>(&self, name: &'static str, period: Duration, delay: Duration, job: F)
    where
        F: Fn(Quest) -> Fut + Send + 'static,
        Fut: Future<Output = AppResult<()>> + Send,
    {
        let quest = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + delay, period);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = quest.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        if let Err(e) = job(quest.clone()).await {
                            tracing::error!(job = name, error = %e, "quest: periodic job failed");
                        }
                    }
                }
            }
        });
    }

    /// Consumes the analytics events the rule table names, until shutdown.
    /// The stream keeps them for the analytics sink anyway; this is a
    /// second, independent reader with its own position.
    async fn run_analytics_consumer(&self) {
        let filters: Vec<String> =
            rules::topics(Feed::Analytics).into_iter().map(|name| format!("an.{name}")).collect();
        let shutdown = self.core.shutdown.clone();
        while !shutdown.is_cancelled() {
            let consumer = match self
                .core
                .bus
                .durable_consumer(STREAM_ANALYTICS, DURABLE, filters.clone(), Duration::from_secs(30), 8)
                .await
            {
                Ok(consumer) => consumer,
                Err(e) => {
                    tracing::error!(error = %e, "quest: analytics consumer setup failed; retrying");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };
            tracing::info!(?filters, "quest: analytics consumer started");
            while !shutdown.is_cancelled() {
                let fetched = tokio::select! {
                    _ = shutdown.cancelled() => return,
                    fetched = consumer.fetch().max_messages(ANALYTICS_BATCH).expires(Duration::from_secs(1)).messages() => fetched,
                };
                let batch = match fetched {
                    Ok(batch) => batch,
                    Err(e) => {
                        tracing::warn!(error = %e, "quest: analytics fetch failed; reconnecting");
                        break;
                    }
                };
                batch
                    .for_each_concurrent(ANALYTICS_CONCURRENCY, |message| async move {
                        let Ok(message) = message else { return };
                        let outcome = match serde_json::from_slice::<AnalyticsEvent>(&message.payload) {
                            Ok(tracked) => self.handle_analytics(tracked).await,
                            // Not an analytics event: nothing to retry.
                            Err(_) => Ok(()),
                        };
                        let acked = match outcome {
                            Ok(()) => message.ack().await,
                            Err(e) => {
                                tracing::warn!(error = %e, "quest: analytics event failed; will retry");
                                message.ack_with(AckKind::Nak(Some(Duration::from_secs(5)))).await
                            }
                        };
                        if let Err(e) = acked {
                            tracing::warn!(error = %e, "quest: analytics ack failed");
                        }
                    })
                    .await;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Handles whatever analytics events are waiting right now and returns
    /// how many there were. The consumer loop does the same continuously;
    /// this is for tests and one-off catch-ups.
    pub async fn drain_analytics(&self) -> AppResult<usize> {
        let filters: Vec<String> =
            rules::topics(Feed::Analytics).into_iter().map(|name| format!("an.{name}")).collect();
        let consumer =
            self.core.bus.durable_consumer(STREAM_ANALYTICS, DURABLE, filters, Duration::from_secs(30), 8).await?;
        let mut handled = 0;
        loop {
            let mut batch = consumer
                .fetch()
                .max_messages(ANALYTICS_BATCH)
                .expires(Duration::from_millis(500))
                .messages()
                .await
                .map_err(|e| AppError::unavailable("BUS_UNAVAILABLE", format!("analytics fetch failed: {e}")))?;
            let mut in_batch = 0;
            while let Some(message) = batch.next().await {
                let Ok(message) = message else { break };
                if let Ok(tracked) = serde_json::from_slice::<AnalyticsEvent>(&message.payload) {
                    self.handle_analytics(tracked).await?;
                }
                let _ = message.ack().await;
                in_batch += 1;
            }
            handled += in_batch;
            if in_batch < ANALYTICS_BATCH {
                return Ok(handled);
            }
        }
    }

    /// Grants level-up coins a stopped process left ungranted. Returns how
    /// many players were caught up.
    pub async fn reward_stragglers(&self) -> AppResult<usize> {
        let behind: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM quest_profiles WHERE rewarded_level < level AND updated_at < now() - interval '1 minute' LIMIT 200").fetch_all(&self.core.db).await?;
        for user_id in &behind {
            let profile = self.profile(*user_id).await?;
            let effects = crate::earnings::Effects::starting_from(&profile);
            self.announce(*user_id, effects).await;
        }
        Ok(behind.len())
    }

    /// Deletes what nothing reads any more.
    pub async fn prune(&self) -> AppResult<()> {
        let missions = self.prune_missions().await?;
        let before = chrono::Utc::now().date_naive() - chrono::Duration::days(METRIC_DAYS_KEPT);
        let days = sqlx::query("DELETE FROM quest_metric_days WHERE day < $1")
            .bind(before)
            .execute(&self.core.db)
            .await?
            .rows_affected();
        let once = sqlx::query("DELETE FROM quest_once WHERE expires_on < $1")
            .bind(chrono::Utc::now().date_naive() - chrono::Duration::days(1))
            .execute(&self.core.db)
            .await?
            .rows_affected();
        if missions > 0 || days > 0 || once > 0 {
            tracing::info!(missions, metric_days = days, once, "quest: pruned old rows");
        }
        Ok(())
    }
}
