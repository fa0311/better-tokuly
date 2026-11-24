//! A Valkey GEO set whose members vanish unless they keep checking in.
//!
//! GEO members cannot expire individually, so each set has a companion sorted
//! set holding every member's last check-in time. Searches ignore members
//! whose check-in is older than the set's time-to-live, and [`LiveGeoSet::prune`]
//! removes them for good. Used for nearby discovery (people with the
//! add-friend screen open) and for bump attempts (people who just shook).
//!
//! Unlike `tokuly_core::geoindex` these positions are volunteered for one
//! purpose and are gone seconds later.
use std::time::Duration;

use chrono::Utc;
use tokuly_core::{cache::Cache, prelude::*};

pub(crate) struct LiveGeoSet {
    geo_key: &'static str,
    seen_key: &'static str,
    /// How long a member stays visible after its last check-in.
    ttl: Duration,
}

/// Removes `ARGV[1]` from the GEO set `KEYS[1]` and its check-in times
/// `KEYS[2]`, and the owner `KEYS[3]`, unless the owner is someone other
/// than `ARGV[2]`.
const LEAVE_UNLESS_TAKEN_OVER: &str = r"
local owner = redis.call('GET', KEYS[3])
if owner and owner ~= ARGV[2] then
    return 0
end
redis.call('DEL', KEYS[3])
redis.call('ZREM', KEYS[1], ARGV[1])
redis.call('ZREM', KEYS[2], ARGV[1])
return 1";

/// Stale members removed per prune call.
const PRUNE_BATCH: usize = 1000;

fn member(user_id: Uuid) -> String {
    user_id.simple().to_string()
}

impl LiveGeoSet {
    pub(crate) const fn new(geo_key: &'static str, seen_key: &'static str, ttl: Duration) -> Self {
        Self { geo_key, seen_key, ttl }
    }

    fn oldest_live_millis(&self) -> i64 {
        Utc::now().timestamp_millis() - self.ttl.as_millis() as i64
    }

    /// Puts the user on the map at this position, or refreshes their presence.
    pub(crate) async fn check_in(&self, cache: &Cache, user_id: Uuid, lat: f64, lng: f64) -> AppResult<()> {
        let member = member(user_id);
        redis::pipe()
            .cmd("GEOADD").arg(self.geo_key).arg(lng).arg(lat).arg(&member).ignore()
            .cmd("ZADD").arg(self.seen_key).arg(Utc::now().timestamp_millis()).arg(&member).ignore()
            .query_async::<()>(&mut cache.conn())
            .await?;
        Ok(())
    }

    /// Takes the user off the map right away.
    pub(crate) async fn leave(&self, cache: &Cache, user_id: Uuid) -> AppResult<()> {
        self.remove(cache, &[member(user_id)]).await
    }

    /// Takes the user off the map unless `owner_key` names someone other
    /// than `owner` (a newer session of theirs that now keeps them there).
    pub(crate) async fn leave_unless_taken_over(&self, cache: &Cache, user_id: Uuid, owner_key: &str, owner: &str) -> AppResult<()> {
        redis::Script::new(LEAVE_UNLESS_TAKEN_OVER)
            .key(self.geo_key)
            .key(self.seen_key)
            .key(owner_key)
            .arg(member(user_id))
            .arg(owner)
            .invoke_async::<i64>(&mut cache.conn())
            .await?;
        Ok(())
    }

    async fn remove(&self, cache: &Cache, members: &[String]) -> AppResult<()> {
        if members.is_empty() {
            return Ok(());
        }
        redis::pipe()
            .cmd("ZREM").arg(self.geo_key).arg(members).ignore()
            .cmd("ZREM").arg(self.seen_key).arg(members).ignore()
            .query_async::<()>(&mut cache.conn())
            .await?;
        Ok(())
    }

    /// Live members within `radius_meters` of a point, nearest first, as
    /// `(user, distance in meters)`.
    pub(crate) async fn within(&self, cache: &Cache, lat: f64, lng: f64, radius_meters: f64, limit: usize) -> AppResult<Vec<(Uuid, f64)>> {
        let mut conn = cache.conn();
        let hits: Vec<(String, f64)> = redis::cmd("GEOSEARCH")
            .arg(self.geo_key)
            .arg("FROMLONLAT").arg(lng).arg(lat)
            .arg("BYRADIUS").arg(radius_meters).arg("m")
            .arg("ASC")
            .arg("COUNT").arg(limit.max(1))
            .arg("WITHDIST")
            .query_async(&mut conn)
            .await?;
        if hits.is_empty() {
            return Ok(Vec::new());
        }
        let members: Vec<&String> = hits.iter().map(|(member, _)| member).collect();
        let seen: Vec<Option<f64>> = redis::cmd("ZMSCORE").arg(self.seen_key).arg(&members).query_async(&mut conn).await?;
        let oldest_live = self.oldest_live_millis() as f64;
        Ok(hits
            .iter()
            .zip(seen)
            .filter(|(_, seen)| seen.is_some_and(|at| at >= oldest_live))
            .filter_map(|((member, distance), _)| Uuid::parse_str(member).ok().map(|id| (id, *distance)))
            .collect())
    }

    /// Removes members that stopped checking in. Call periodically.
    pub(crate) async fn prune(&self, cache: &Cache) -> AppResult<()> {
        let stale: Vec<String> = redis::cmd("ZRANGEBYSCORE")
            .arg(self.seen_key)
            .arg("-inf")
            .arg(self.oldest_live_millis())
            .arg("LIMIT").arg(0).arg(PRUNE_BATCH)
            .query_async(&mut cache.conn())
            .await?;
        self.remove(cache, &stale).await
    }
}
