//! Who is online: the gateway's half of `tokuly_core::presence`.
//!
//! A user is *online* while at least one of their connections, on any
//! gateway, has the app in the foreground. Each gateway knows only its own
//! connections, so the cross-gateway truth lives in Valkey as *leases*:
//!
//! - `rt:fg:u:<user>` — sorted set of gateway instances holding a foreground
//!   connection of the user, scored by when their lease expires;
//! - `rt:fg:leases` — sorted set of users, scored by their latest lease
//!   expiry, so expired leases can be found without scanning.
//!
//! Gateways renew their leases on a heartbeat. When a gateway dies its leases
//! run out and any surviving gateway *reaps* them, announcing the users
//! offline. All transitions are decided atomically in Valkey (Lua), using its
//! clock, so two gateways can never both (or neither) announce a change.
use std::{collections::HashMap, time::Duration};

use chrono::Utc;
use futures::{StreamExt as _, stream};
use parking_lot::Mutex;
use redis::Script;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tokuly_core::{
    AppResult, Core,
    presence::{self, PresenceChange},
};
use uuid::Uuid;

use crate::keys;

/// Users handled per Valkey round trip when renewing or reaping leases.
const BATCH: usize = 256;
/// Parallel Valkey/bus operations while applying a batch.
const CONCURRENCY: usize = 32;

/// `KEYS[1]` lease index. `ARGV`: per-user key prefix, gateway instance,
/// lease in ms, then user IDs. Takes or renews this gateway's lease for each
/// user and returns how many live leases each user had *before* (0 = the
/// user just came online).
const RENEW: &str = r"
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local expires = now + tonumber(ARGV[3])
local before = {}
for i = 4, #ARGV do
  local key = ARGV[1] .. ARGV[i]
  redis.call('ZREMRANGEBYSCORE', key, '-inf', now)
  before[#before + 1] = redis.call('ZCARD', key)
  redis.call('ZADD', key, expires, ARGV[2])
  redis.call('PEXPIRE', key, ARGV[3])
  redis.call('ZADD', KEYS[1], expires, ARGV[i])
end
return before
";

/// `KEYS[1]` lease index. `ARGV`: per-user key prefix, gateway instance,
/// user ID. Gives up this gateway's lease and returns `{released, remaining}`:
/// whether a lease was actually held and how many live leases are left.
const RELEASE: &str = r"
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local key = ARGV[1] .. ARGV[3]
local released = redis.call('ZREM', key, ARGV[2])
redis.call('ZREMRANGEBYSCORE', key, '-inf', now)
local remaining = redis.call('ZCARD', key)
if remaining == 0 then
  redis.call('ZREM', KEYS[1], ARGV[3])
end
return {released, remaining}
";

/// `KEYS[1]` lease index. `ARGV`: per-user key prefix, batch size. Claims
/// users whose every lease ran out and returns their IDs; each user is
/// returned to exactly one caller.
const REAP: &str = r"
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local due = redis.call('ZRANGEBYSCORE', KEYS[1], '-inf', now, 'LIMIT', 0, tonumber(ARGV[2]))
local gone = {}
for _, user in ipairs(due) do
  local key = ARGV[1] .. user
  redis.call('ZREMRANGEBYSCORE', key, '-inf', now)
  local latest = redis.call('ZRANGE', key, -1, -1, 'WITHSCORES')
  if #latest == 0 then
    redis.call('ZREM', KEYS[1], user)
    gone[#gone + 1] = user
  else
    redis.call('ZADD', KEYS[1], latest[2], user)
  end
end
return gone
";

/// A user's first foreground connection here appeared, or the last one went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transition {
    Online(Uuid),
    Offline(Uuid),
}

pub(crate) struct Presence {
    core: Core,
    instance: String,
    lease: Duration,
    heartbeat: Duration,
    /// Foreground connections per user on this gateway.
    local: Mutex<HashMap<Uuid, usize>>,
    /// Transitions in the order they happened, for the worker to apply.
    queue: mpsc::UnboundedSender<Transition>,
    pending: Mutex<Option<mpsc::UnboundedReceiver<Transition>>>,
    renew: Script,
    release: Script,
    reap: Script,
}

impl Presence {
    pub(crate) fn new(core: Core, instance: String, lease: Duration, heartbeat: Duration) -> Self {
        let (queue, pending) = mpsc::unbounded_channel();
        Self {
            core,
            instance,
            lease,
            heartbeat,
            local: Mutex::new(HashMap::new()),
            queue,
            pending: Mutex::new(Some(pending)),
            renew: Script::new(RENEW),
            release: Script::new(RELEASE),
            reap: Script::new(REAP),
        }
    }

    /// A signed-in connection brought the app to the foreground.
    pub(crate) fn foreground_started(&self, user_id: Uuid) {
        let mut local = self.local.lock();
        let count = local.entry(user_id).or_insert(0);
        *count += 1;
        if *count == 1 {
            // Sent under the lock so the queue order matches the order of changes.
            let _ = self.queue.send(Transition::Online(user_id));
        }
    }

    /// A foreground connection closed or moved to the background.
    pub(crate) fn foreground_ended(&self, user_id: Uuid) {
        let mut local = self.local.lock();
        match local.get_mut(&user_id) {
            Some(count) if *count > 1 => *count -= 1,
            Some(_) => {
                local.remove(&user_id);
                let _ = self.queue.send(Transition::Offline(user_id));
            }
            None => {}
        }
    }

    /// Applies transitions, renews leases and reaps expired ones until
    /// `stop` is cancelled; then releases whatever this gateway still holds.
    /// Stop it only after the connections are gone.
    pub(crate) async fn run(&self, stop: CancellationToken) {
        let Some(mut pending) = self.pending.lock().take() else { return };
        let mut heartbeat = tokio::time::interval(self.heartbeat);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = stop.cancelled() => break,
                first = pending.recv() => {
                    let Some(first) = first else { break };
                    self.apply(Self::batch(first, &mut pending)).await;
                }
                _ = heartbeat.tick() => {
                    self.renew_leases().await;
                    self.reap_expired().await;
                }
            }
        }
        if let Ok(first) = pending.try_recv() {
            self.apply(Self::batch(first, &mut pending)).await;
        }
        // Normally empty by now; anything left would otherwise stay online
        // until its lease expires.
        let leftover: Vec<Uuid> = self.local.lock().drain().map(|(user_id, _)| user_id).collect();
        self.apply(leftover.into_iter().map(Transition::Offline).collect()).await;
    }

    fn batch(first: Transition, pending: &mut mpsc::UnboundedReceiver<Transition>) -> Vec<Transition> {
        let mut batch = vec![first];
        while let Ok(next) = pending.try_recv() {
            batch.push(next);
        }
        batch
    }

    /// Applies a batch of transitions. Users are independent of each other,
    /// so they run in parallel; the order per user is kept by [`net_changes`]
    /// and by finishing a batch before the next one starts.
    async fn apply(&self, batch: Vec<Transition>) {
        stream::iter(net_changes(batch))
            .for_each_concurrent(CONCURRENCY, |transition| async move {
                let (user_id, result) = match transition {
                    Transition::Online(user_id) => (user_id, self.came_online(user_id).await),
                    Transition::Offline(user_id) => (user_id, self.went_offline(user_id).await),
                };
                if let Err(error) = result {
                    // The next heartbeat (or lease expiry) repairs the state.
                    tracing::warn!(%user_id, %error, "presence update failed");
                }
            })
            .await;
    }

    async fn came_online(&self, user_id: Uuid) -> AppResult<()> {
        let leases_before = self.renew(&[user_id]).await?;
        presence::mark_online(&self.core.cache, user_id).await?;
        if leases_before.first() == Some(&0) {
            self.announce(user_id, true).await?;
        }
        Ok(())
    }

    async fn went_offline(&self, user_id: Uuid) -> AppResult<()> {
        let (released, remaining): (i64, i64) = self
            .release
            .key(keys::FOREGROUND_LEASES)
            .arg(keys::FOREGROUND_PREFIX)
            .arg(&self.instance)
            .arg(user_id.simple().to_string())
            .invoke_async(&mut self.core.cache.conn())
            .await?;
        // Without a released lease the user was already reaped (and announced).
        if released == 1 && remaining == 0 {
            presence::mark_offline(&self.core.cache, user_id).await?;
            self.announce(user_id, false).await?;
        }
        Ok(())
    }

    /// Takes or renews this gateway's leases; returns each user's live lease
    /// count from before the call.
    async fn renew(&self, user_ids: &[Uuid]) -> AppResult<Vec<i64>> {
        let mut invocation = self.renew.key(keys::FOREGROUND_LEASES);
        invocation.arg(keys::FOREGROUND_PREFIX).arg(&self.instance).arg(self.lease.as_millis() as u64);
        for user_id in user_ids {
            invocation.arg(user_id.simple().to_string());
        }
        Ok(invocation.invoke_async(&mut self.core.cache.conn()).await?)
    }

    /// Heartbeat: keeps every locally online user online.
    async fn renew_leases(&self) {
        let online: Vec<Uuid> = self.local.lock().keys().copied().collect();
        for chunk in online.chunks(BATCH) {
            let leases_before = match self.renew(chunk).await {
                Ok(leases_before) => leases_before,
                Err(error) => {
                    tracing::warn!(%error, "presence lease renewal failed");
                    continue;
                }
            };
            stream::iter(chunk.iter().zip(leases_before))
                .for_each_concurrent(CONCURRENCY, |(user_id, leases_before)| async move {
                    let mut result = presence::mark_online(&self.core.cache, *user_id).await;
                    // Our lease had run out (this process stalled) and the
                    // user was reaped as offline: put that right.
                    if result.is_ok() && leases_before == 0 {
                        result = self.announce(*user_id, true).await;
                    }
                    if let Err(error) = result {
                        tracing::warn!(%user_id, %error, "presence heartbeat failed");
                    }
                })
                .await;
        }
    }

    /// Announces users offline whose gateways stopped renewing their leases.
    async fn reap_expired(&self) {
        loop {
            let claimed: Vec<String> = match self
                .reap
                .key(keys::FOREGROUND_LEASES)
                .arg(keys::FOREGROUND_PREFIX)
                .arg(BATCH)
                .invoke_async(&mut self.core.cache.conn())
                .await
            {
                Ok(claimed) => claimed,
                Err(error) => {
                    tracing::warn!(%error, "presence reaping failed");
                    return;
                }
            };
            let claimed_count = claimed.len();
            stream::iter(claimed.iter().filter_map(|id| Uuid::parse_str(id).ok()))
                .for_each_concurrent(CONCURRENCY, |user_id| async move {
                    let result = async {
                        presence::mark_offline(&self.core.cache, user_id).await?;
                        self.announce(user_id, false).await
                    };
                    if let Err(error) = result.await {
                        tracing::warn!(%user_id, %error, "announcing a reaped user offline failed");
                    }
                })
                .await;
            if claimed_count < BATCH {
                return;
            }
        }
    }

    async fn announce(&self, user_id: Uuid, online: bool) -> AppResult<()> {
        presence::announce(&self.core.bus, &PresenceChange { user_id, online, at: Utc::now() }).await
    }
}

/// Reduces a batch to at most one transition per user. A user's transitions
/// alternate, so an even number of them cancels out and an odd number leaves
/// the last one.
fn net_changes(batch: Vec<Transition>) -> Vec<Transition> {
    let mut net: HashMap<Uuid, Option<Transition>> = HashMap::new();
    for transition in batch {
        let (Transition::Online(user_id) | Transition::Offline(user_id)) = transition;
        let slot = net.entry(user_id).or_default();
        *slot = match slot {
            Some(_) => None,
            None => Some(transition),
        };
    }
    net.into_values().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opposite_transitions_of_one_user_cancel_out() {
        let (a, b, c) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let mut net = net_changes(vec![
            Transition::Online(a),
            Transition::Online(b),
            Transition::Offline(a),
            Transition::Offline(c),
            Transition::Online(c),
            Transition::Offline(c),
        ]);
        net.sort_by_key(|transition| match transition {
            Transition::Online(id) | Transition::Offline(id) => *id,
        });
        let mut expected = vec![(b, Transition::Online(b)), (c, Transition::Offline(c))];
        expected.sort_by_key(|(id, _)| *id);
        assert_eq!(net, expected.into_iter().map(|(_, transition)| transition).collect::<Vec<_>>());
    }
}
