//! Topic audiences: how many connections follow a topic, across gateways.
//!
//! Each gateway reports its own subscriber count per topic through
//! `tokuly_core::presence::set_topic_audience`; the sum over gateways is the
//! audience (a live stream's viewer count, for instance). Whenever the local
//! count of a topic changes, the gateway rewrites its report and publishes a
//! `TopicStats` frame *on the topic itself*, so subscribers on every gateway
//! receive it through the normal fan-out. Pushes are throttled per topic.
//!
//! Reports go stale after `AUDIENCE_TTL_SECONDS` unless rewritten, which is
//! how the counts of a gateway that died disappear. So every gateway
//! rewrites its reports on a timer, and uses the occasion to push totals
//! that changed without anyone announcing it (such as a report going stale).
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use futures::{StreamExt as _, stream};
use parking_lot::Mutex;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tokuly_core::{AppResult, Core, presence, realtime::topic_subject};

use crate::{
    frames,
    hub::{Hub, RouteKey},
};

/// Parallel Valkey/bus operations while flushing or refreshing.
const CONCURRENCY: usize = 16;

#[derive(Default)]
struct Known {
    last_flush: Option<Instant>,
    /// The audience last pushed to subscribers.
    last_total: Option<i64>,
}

#[derive(Default)]
struct State {
    /// Topics this gateway has (or just had) subscribers for.
    known: HashMap<String, Known>,
    /// Topics whose local count changed, with the earliest time the change
    /// may be written and pushed.
    due: HashMap<String, Instant>,
}

pub(crate) struct Audience {
    core: Core,
    instance: String,
    hub: Arc<Hub>,
    /// Minimum time between two pushes for one topic.
    throttle: Duration,
    refresh: Duration,
    state: Mutex<State>,
    wake: Notify,
}

impl Audience {
    pub(crate) fn new(core: Core, instance: String, hub: Arc<Hub>, throttle: Duration, refresh: Duration) -> Self {
        Self { core, instance, hub, throttle, refresh, state: Mutex::new(State::default()), wake: Notify::new() }
    }

    /// Notes that the local subscriber count of `topic` changed. The count
    /// itself is read from the hub when the change is flushed, so bursts of
    /// joins and leaves collapse into one write.
    pub(crate) fn touch(&self, topic: &str) {
        let mut state = self.state.lock();
        if state.due.contains_key(topic) {
            return;
        }
        let now = Instant::now();
        let earliest = state.known.entry(topic.to_owned()).or_default().last_flush.map(|at| at + self.throttle);
        state.due.insert(topic.to_owned(), earliest.map_or(now, |at| at.max(now)));
        drop(state);
        self.wake.notify_one();
    }

    /// Flushes changes as they become due and refreshes everything
    /// periodically until `stop` is cancelled; then withdraws this gateway's
    /// counts. Stop it only after the connections are gone.
    pub(crate) async fn run(&self, stop: CancellationToken) {
        let mut refresh = tokio::time::interval(self.refresh);
        refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        refresh.tick().await;
        loop {
            let next_due = self.state.lock().due.values().min().copied();
            let until_due = async {
                match next_due {
                    Some(at) => tokio::time::sleep_until(at.into()).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                _ = stop.cancelled() => break,
                // A topic became due earlier than what we were waiting for.
                _ = self.wake.notified() => {}
                _ = until_due => self.flush_due().await,
                _ = refresh.tick() => self.refresh_all().await,
            }
        }
        self.withdraw().await;
    }

    async fn flush_due(&self) {
        let now = Instant::now();
        let due: Vec<String> = {
            let mut state = self.state.lock();
            let due: Vec<String> = state.due.iter().filter(|(_, at)| **at <= now).map(|(topic, _)| topic.clone()).collect();
            for topic in &due {
                state.due.remove(topic);
                state.known.entry(topic.clone()).or_default().last_flush = Some(now);
            }
            due
        };
        stream::iter(due)
            .for_each_concurrent(CONCURRENCY, |topic| async move {
                if let Err(error) = self.flush(&topic).await {
                    tracing::warn!(topic, %error, "topic audience update failed");
                }
            })
            .await;
    }

    /// Writes this gateway's report for a topic and pushes the total to the
    /// topic's subscribers everywhere. Always pushes, even if the total is
    /// what it was: whoever just subscribed has not seen it yet.
    async fn flush(&self, topic: &str) -> AppResult<()> {
        let total = self.write_count(topic).await?;
        if let Some(known) = self.state.lock().known.get_mut(topic) {
            known.last_total = Some(total);
        }
        if total > 0 {
            self.core.bus.publish(topic_subject(topic), frames::topic_stats(topic, total)).await?;
        }
        Ok(())
    }

    /// Reports this gateway's current count for a topic and returns the
    /// audience across all gateways.
    async fn write_count(&self, topic: &str) -> AppResult<i64> {
        let local = self.hub.member_count(&RouteKey::Topic(topic.to_owned())) as i64;
        presence::set_topic_audience(&self.core.cache, topic, &self.instance, local).await?;
        let total = presence::topic_audience(&self.core.cache, topic).await?;
        if local == 0 {
            // Nobody here follows the topic anymore: stop tracking it,
            // unless someone joined again in the meantime.
            let mut state = self.state.lock();
            if !state.due.contains_key(topic) {
                state.known.remove(topic);
            }
        }
        Ok(total)
    }

    /// Rewrites every report before it goes stale and schedules a push for
    /// totals that changed behind our back.
    async fn refresh_all(&self) {
        let topics: Vec<String> = self.state.lock().known.keys().cloned().collect();
        stream::iter(topics)
            .for_each_concurrent(CONCURRENCY, |topic| async move {
                if let Err(error) = self.refresh(&topic).await {
                    tracing::warn!(topic, %error, "topic audience refresh failed");
                }
            })
            .await;
    }

    async fn refresh(&self, topic: &str) -> AppResult<()> {
        let total = self.write_count(topic).await?;
        let outdated = self.state.lock().known.get(topic).is_some_and(|known| known.last_total != Some(total));
        // Through the throttle like any other change, so a refresh never
        // adds a push right next to a scheduled one.
        if outdated {
            self.touch(topic);
        }
        Ok(())
    }

    /// Shutdown: this gateway no longer contributes to any audience.
    async fn withdraw(&self) {
        let topics: Vec<String> = {
            let mut state = self.state.lock();
            state.due.clear();
            state.known.drain().map(|(topic, _)| topic).collect()
        };
        stream::iter(topics)
            .for_each_concurrent(CONCURRENCY, |topic| async move {
                if let Err(error) = presence::set_topic_audience(&self.core.cache, &topic, &self.instance, 0).await {
                    tracing::warn!(topic, %error, "topic audience report could not be withdrawn");
                }
            })
            .await;
    }
}
