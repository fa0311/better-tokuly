//! Background work: the location stream consumer, the event consumer, the
//! history flusher, the sweeper for everything that expires, and the daily
//! retention pass.
use std::{
    collections::HashMap,
    hash::{Hash as _, Hasher as _},
    time::Duration,
};

use async_nats::jetstream::{AckKind, Message};
use chrono::Utc;
use futures::StreamExt as _;
use tokio::sync::mpsc;
use tokuly_core::{
    bus::STREAM_LOCATIONS,
    events::{self, ConsumeOptions},
    prelude::*,
    presence::LOCATION_SUBJECT_PREFIX,
};
use tokuly_identity::settings;

use crate::{Geo, consumers, pipeline::LocationReport};

/// Durable name of the location stream consumer; worker processes share it.
const LOCATION_CONSUMER: &str = "geo-pipeline";
/// How long a sample may be in processing before it is redelivered.
const LOCATION_ACK_WAIT: Duration = Duration::from_secs(60);
const LOCATION_MAX_DELIVER: i64 = 5;
/// Samples queued per lane before the consumer stops pulling.
const LANE_DEPTH: usize = 256;
const FLUSH_EVERY: Duration = Duration::from_secs(2);
const SWEEP_EVERY: Duration = Duration::from_secs(15);
const DAILY: Duration = Duration::from_secs(24 * 3600);
/// The first daily pass waits a little so that a restart loop does not hammer ClickHouse.
const DAILY_DELAY: Duration = Duration::from_secs(300);
/// Users per retention delete.
const RETENTION_BATCH: usize = 500;

/// The lane a user's samples go through: always the same one, so they are
/// processed in the order they arrived.
fn lane_of(user_id: Uuid, lanes: usize) -> usize {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    user_id.hash(&mut hasher);
    (hasher.finish() % lanes.max(1) as u64) as usize
}

impl Geo {
    /// Starts the background work of the geo domain (worker processes only).
    pub fn spawn_workers(&self) {
        let geo = self.clone();
        tokio::spawn(async move { geo.run_location_consumer().await });

        let geo = self.clone();
        tokio::spawn(async move {
            let handler = geo.clone();
            events::consume(geo.core.bus.clone(), "geo", consumers::TOPICS, ConsumeOptions::default(), geo.core.shutdown.clone(), move |event| {
                let geo = handler.clone();
                async move { geo.handle_event(&event).await }
            })
            .await;
        });

        let geo = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(FLUSH_EVERY);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = geo.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {}
                }
                if let Err(e) = geo.flush_history().await {
                    tracing::warn!(error = %e, "location history not written; will retry");
                }
            }
            // Whatever is still buffered when the process stops.
            if let Err(e) = geo.flush_history().await {
                tracing::error!(error = %e, "location history lost at shutdown");
            }
        });

        let geo = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(SWEEP_EVERY);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = geo.core.shutdown.cancelled() => break,
                    _ = tick.tick() => geo.sweep().await,
                }
            }
        });

        let geo = self.clone();
        tokio::spawn(async move {
            let mut next = DAILY_DELAY;
            loop {
                tokio::select! {
                    _ = geo.core.shutdown.cancelled() => break,
                    _ = tokio::time::sleep(next) => {}
                }
                next = DAILY;
                match geo.enforce_retention().await {
                    Ok(users) => tracing::info!(users, "location history retention applied"),
                    Err(e) => tracing::error!(error = %e, "location history retention failed"),
                }
                if let Err(e) = geo.prune().await {
                    tracing::error!(error = %e, "geo pruning failed");
                }
            }
        });
    }

    /// Ends everything whose time is up: timed ghost rules, live shares,
    /// waiting place alerts, unresolved SOS alerts, abandoned trips, and
    /// gatherings whose members went silent. Run by the sweeper every few
    /// seconds; public so tests and operators can trigger it.
    pub async fn sweep(&self) {
        let jobs: [(&str, AppResult<u64>); 6] = [
            ("ghost rules", self.expire_ghost_rules().await),
            ("live shares", self.expire_live_shares().await),
            ("place alerts", self.expire_place_alerts().await),
            ("sos", self.expire_sos().await),
            ("trips", self.expire_trips().await),
            ("gatherings", self.expire_gatherings().await),
        ];
        for (job, result) in jobs {
            match result {
                Ok(0) => {}
                Ok(ended) => tracing::debug!(job, ended, "expired"),
                Err(e) => tracing::error!(job, error = %e, "expiry failed"),
            }
        }
    }

    /// Deletes raw history older than each user's `location_history_days`
    /// (0 keeps it forever). Returns how many users have a limit.
    pub async fn enforce_retention(&self) -> AppResult<u64> {
        let now = Utc::now();
        let mut limited = 0;
        let mut after = Uuid::nil();
        loop {
            // Only accounts that ever changed their privacy settings can have a limit.
            let page: Vec<(Uuid, serde_json::Value)> =
                sqlx::query_as("SELECT id, privacy FROM users WHERE id > $1 AND deleted_at IS NULL AND privacy <> '{}'::jsonb ORDER BY id LIMIT 1000")
                    .bind(after)
                    .fetch_all(&self.core.db)
                    .await?;
            let Some((last, _)) = page.last() else { break };
            after = *last;
            let mut by_days: HashMap<i32, Vec<Uuid>> = HashMap::new();
            for (user_id, privacy) in &page {
                let days = settings::privacy_from_json(privacy).location_history_days;
                if days > 0 {
                    by_days.entry(days).or_default().push(*user_id);
                }
            }
            for (days, users) in by_days {
                limited += users.len() as u64;
                for batch in users.chunks(RETENTION_BATCH) {
                    self.delete_points_before(batch, now - chrono::Duration::days(days as i64)).await?;
                }
            }
        }
        Ok(limited)
    }

    /// Deletes rows nobody will look at again: place alerts that ended a
    /// month ago, live shares that ended a week ago, gatherings and SOS
    /// alerts over after a year.
    pub async fn prune(&self) -> AppResult<()> {
        for statement in [
            "DELETE FROM geo_place_alerts WHERE status IN (3, 4, 5) AND created_at < now() - interval '30 days'",
            "DELETE FROM geo_live_shares WHERE ended_at < now() - interval '7 days'",
            "DELETE FROM geo_gatherings WHERE ended_at < now() - interval '365 days'",
            "DELETE FROM geo_sos WHERE resolved_at < now() - interval '365 days'",
            "DELETE FROM geo_trips WHERE status <> 1 AND ended_at < now() - interval '365 days'",
        ] {
            sqlx::query(statement).execute(&self.core.db).await?;
        }
        Ok(())
    }

    /// Consumes location samples forwarded by the realtime gateway until
    /// shutdown. Samples are spread over lanes by user, so one user's samples
    /// are processed in order while different users proceed in parallel.
    async fn run_location_consumer(&self) {
        let shutdown = self.core.shutdown.clone();
        let lanes: Vec<mpsc::Sender<(Message, LocationReport)>> = (0..self.cfg.pipeline_lanes)
            .map(|_| {
                let (sender, mut receiver) = mpsc::channel::<(Message, LocationReport)>(LANE_DEPTH);
                let geo = self.clone();
                tokio::spawn(async move {
                    while let Some((message, report)) = receiver.recv().await {
                        geo.process_stream_sample(message, report).await;
                    }
                });
                sender
            })
            .collect();
        let filter = format!("{LOCATION_SUBJECT_PREFIX}.>");
        while !shutdown.is_cancelled() {
            let consumer = match self.core.bus.durable_consumer(STREAM_LOCATIONS, LOCATION_CONSUMER, vec![filter.clone()], LOCATION_ACK_WAIT, LOCATION_MAX_DELIVER).await {
                Ok(consumer) => consumer,
                Err(e) => {
                    tracing::error!(error = %e, "location consumer setup failed; retrying");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };
            let mut messages = match consumer.messages().await {
                Ok(messages) => messages,
                Err(e) => {
                    tracing::error!(error = %e, "location stream failed; retrying");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };
            tracing::info!(lanes = lanes.len(), "location pipeline started");
            loop {
                let next = tokio::select! {
                    _ = shutdown.cancelled() => return,
                    next = messages.next() => next,
                };
                let message = match next {
                    Some(Ok(message)) => message,
                    Some(Err(e)) => {
                        tracing::warn!(error = %e, "location stream error; reconnecting");
                        break;
                    }
                    None => break,
                };
                let report: LocationReport = match serde_json::from_slice(&message.payload) {
                    Ok(report) => report,
                    Err(e) => {
                        tracing::warn!(error = %e, "undecodable location sample dropped");
                        let _ = message.ack_with(AckKind::Term).await;
                        continue;
                    }
                };
                // A full lane makes us wait here, which stops pulling: backpressure onto the stream.
                if lanes[lane_of(report.user_id, lanes.len())].send((message, report)).await.is_err() {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    async fn process_stream_sample(&self, message: Message, report: LocationReport) {
        let user_id = report.user_id;
        let delivered = message.info().map(|info| info.delivered).unwrap_or(1);
        match self.ingest(report).await {
            Ok(()) => {
                if let Err(e) = message.ack().await {
                    tracing::warn!(%user_id, error = %e, "location sample ack failed");
                }
            }
            Err(e) if delivered >= LOCATION_MAX_DELIVER => {
                tracing::error!(%user_id, error = %e, "location sample failed permanently; dropped");
                let _ = message.ack_with(AckKind::Term).await;
            }
            Err(e) => {
                tracing::warn!(%user_id, delivered, error = %e, "location sample failed; will retry");
                let _ = message.ack_with(AckKind::Nak(Some(Duration::from_secs(1 << delivered.clamp(0, 5))))).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_always_gets_the_same_lane() {
        let user = Uuid::now_v7();
        let lane = lane_of(user, 16);
        assert!(lane < 16);
        assert_eq!(lane_of(user, 16), lane);
        assert_eq!(lane_of(user, 1), 0);
        assert_eq!(lane_of(user, 0), 0, "a misconfigured lane count cannot divide by zero");
        // Different users spread out.
        let lanes: std::collections::HashSet<usize> = (0..200).map(|_| lane_of(Uuid::now_v7(), 16)).collect();
        assert!(lanes.len() > 8);
    }
}
