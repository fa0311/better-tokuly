//! Gateway counters: a cheap snapshot for health endpoints and tests, and a
//! structured log line at intervals for operators.
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use serde::Serialize;
use tokio_util::sync::CancellationToken;

/// A point-in-time view of one gateway process.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GatewayStats {
    /// Open WebTransport connections.
    pub webtransport_connections: u64,
    /// Open WebSocket connections.
    pub websocket_connections: u64,
    /// Open connections that are signed in.
    pub authenticated_connections: u64,
    /// Connections accepted since start.
    pub connections_accepted: u64,
    /// Connections refused since start (origin, path, connection limit).
    pub connections_refused: u64,
    /// Client frames received (stream/message frames and datagrams).
    pub frames_in: u64,
    /// Of `frames_in`, how many arrived as datagrams.
    pub datagrams_in: u64,
    /// Frames written to clients (including datagrams).
    pub frames_out: u64,
    /// Of `frames_out`, how many left as datagrams.
    pub datagrams_out: u64,
    /// Droppable frames discarded because a client was not reading fast enough.
    pub frames_dropped: u64,
    /// Client frames refused (undecodable, invalid, unauthorized, rate limited).
    pub frames_rejected: u64,
    /// Connections closed because their reliable backlog overflowed.
    pub slow_consumer_disconnects: u64,
    /// Location samples handed to the geo pipeline.
    pub locations_forwarded: u64,
    /// Users with at least one connection here (= user bus subscriptions).
    pub user_subscriptions: u64,
    /// Topics with at least one subscriber here (= topic bus subscriptions).
    pub topic_subscriptions: u64,
}

/// A value that only grows.
#[derive(Default)]
pub(crate) struct Counter(AtomicU64);

impl Counter {
    pub(crate) fn add(&self, n: u64) {
        self.0.fetch_add(n, Ordering::Relaxed);
    }

    pub(crate) fn incr(&self) {
        self.add(1);
    }

    fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// A value that goes up and down.
#[derive(Default)]
pub(crate) struct Gauge(AtomicU64);

impl Gauge {
    pub(crate) fn incr(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn decr(&self) {
        // Saturating, so an accounting slip can never wrap around to 2^64.
        let mut current = self.0.load(Ordering::Relaxed);
        while current > 0 {
            match self.0.compare_exchange_weak(current, current - 1, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
    }

    pub(crate) fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Default)]
pub(crate) struct Counters {
    pub webtransport_connections: Gauge,
    pub websocket_connections: Gauge,
    pub authenticated_connections: Gauge,
    pub connections_accepted: Counter,
    pub connections_refused: Counter,
    pub frames_in: Counter,
    pub datagrams_in: Counter,
    pub frames_out: Counter,
    pub datagrams_out: Counter,
    pub frames_dropped: Counter,
    pub frames_rejected: Counter,
    pub slow_consumer_disconnects: Counter,
    pub locations_forwarded: Counter,
    pub user_subscriptions: Gauge,
    pub topic_subscriptions: Gauge,
}

impl Counters {
    pub(crate) fn open_connections(&self) -> u64 {
        self.webtransport_connections.get() + self.websocket_connections.get()
    }

    pub(crate) fn snapshot(&self) -> GatewayStats {
        GatewayStats {
            webtransport_connections: self.webtransport_connections.get(),
            websocket_connections: self.websocket_connections.get(),
            authenticated_connections: self.authenticated_connections.get(),
            connections_accepted: self.connections_accepted.get(),
            connections_refused: self.connections_refused.get(),
            frames_in: self.frames_in.get(),
            datagrams_in: self.datagrams_in.get(),
            frames_out: self.frames_out.get(),
            datagrams_out: self.datagrams_out.get(),
            frames_dropped: self.frames_dropped.get(),
            frames_rejected: self.frames_rejected.get(),
            slow_consumer_disconnects: self.slow_consumer_disconnects.get(),
            locations_forwarded: self.locations_forwarded.get(),
            user_subscriptions: self.user_subscriptions.get(),
            topic_subscriptions: self.topic_subscriptions.get(),
        }
    }

    /// Logs a snapshot every `interval` until `stop` is cancelled. Idle
    /// gateways stay quiet: nothing is logged while nothing changes.
    pub(crate) async fn log_periodically(&self, instance: &str, interval: Duration, stop: CancellationToken) {
        let mut previous = self.snapshot();
        let mut tick = tokio::time::interval(interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await;
        loop {
            tokio::select! {
                _ = stop.cancelled() => break,
                _ = tick.tick() => {}
            }
            let stats = self.snapshot();
            if stats == previous {
                continue;
            }
            tracing::info!(
                instance,
                webtransport = stats.webtransport_connections,
                websocket = stats.websocket_connections,
                authenticated = stats.authenticated_connections,
                accepted = stats.connections_accepted,
                refused = stats.connections_refused,
                frames_in = stats.frames_in,
                datagrams_in = stats.datagrams_in,
                frames_out = stats.frames_out,
                datagrams_out = stats.datagrams_out,
                dropped = stats.frames_dropped,
                rejected = stats.frames_rejected,
                slow_consumers = stats.slow_consumer_disconnects,
                locations = stats.locations_forwarded,
                users = stats.user_subscriptions,
                topics = stats.topic_subscriptions,
                "realtime gateway stats"
            );
            previous = stats;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_reflects_counters_and_gauges() {
        let counters = Counters::default();
        counters.websocket_connections.incr();
        counters.websocket_connections.incr();
        counters.websocket_connections.decr();
        counters.webtransport_connections.incr();
        counters.frames_in.add(3);
        counters.frames_dropped.incr();
        let stats = counters.snapshot();
        assert_eq!((stats.websocket_connections, stats.webtransport_connections, stats.frames_in, stats.frames_dropped), (1, 1, 3, 1));
        assert_eq!(counters.open_connections(), 2);
    }

    #[test]
    fn gauges_never_wrap_below_zero() {
        let gauge = Gauge::default();
        gauge.decr();
        assert_eq!(gauge.get(), 0);
    }
}
