//! The shared index of users' latest positions (Valkey GEO).
//!
//! The geo domain writes it on every accepted location sample; any domain
//! may read it for distance checks ("are these two within 200 m?") and
//! proximity searches. Positions here are the *true* latest positions: they
//! are server-side facts for rule checks, never something to send to another
//! user (ghost mode is applied by the geo domain when sharing locations).
//!
//! A rule check is an answer too: "you are near her" tells the asker where
//! she is. So the index also knows who sees whom precisely ([`Sight`],
//! written by the geo domain next to every position), and what it says
//! about a *pair* ([`distance_between`]) it says only when each of the two
//! sees the other's precise location. Anything else a caller derives from
//! true positions for one user about another must apply the same rule.
use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{cache::Cache, error::AppResult};

const POSITIONS: &str = "geo:pos";
const TIMESTAMPS: &str = "geo:pos:ts";

fn sight_key(user_id: Uuid) -> String {
    format!("geo:pos:sight:{}", user_id.simple())
}

/// One ghost rule as far as the index cares: does the viewer see the precise
/// location, and until when does the rule hold (Unix milliseconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SightRule {
    pub precise: bool,
    pub until_ms: Option<i64>,
}

impl SightRule {
    fn active(&self, now_ms: i64) -> bool {
        self.until_ms.is_none_or(|until| until > now_ms)
    }
}

/// Who sees a user's precise location: the rule for everyone, exceptions
/// per friend, and the people an SOS alerted (who see it whatever the rules
/// say). Without rules everyone the user shares with sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sight {
    pub everyone: Option<SightRule>,
    #[serde(default)]
    pub overrides: HashMap<Uuid, SightRule>,
    #[serde(default)]
    pub alerted: Vec<Uuid>,
}

impl Sight {
    /// Whether `viewer` sees the precise location at `now_ms`.
    pub fn precise_for(&self, viewer: Uuid, now_ms: i64) -> bool {
        if self.alerted.contains(&viewer) {
            return true;
        }
        let default = self.everyone.filter(|rule| rule.active(now_ms));
        match self.overrides.get(&viewer) {
            Some(rule) if rule.active(now_ms) => rule.precise,
            // An exception that ran out: back under the rule for everyone.
            _ => default.is_none_or(|rule| rule.precise),
        }
    }
}

/// Records who sees a user's precise location. The geo domain calls this
/// with every position and whenever the user's rules change.
pub async fn set_sight(cache: &Cache, user_id: Uuid, sight: &Sight) -> AppResult<()> {
    cache.set_json(&sight_key(user_id), sight, None).await
}

/// Whether each of two users sees the other's precise location right now.
/// A user the index holds no rules for hides from nobody.
pub async fn mutually_precise(cache: &Cache, a: Uuid, b: Uuid) -> AppResult<bool> {
    let now_ms = Utc::now().timestamp_millis();
    let of_a: Option<Sight> = cache.get_json(&sight_key(a)).await?;
    let of_b: Option<Sight> = cache.get_json(&sight_key(b)).await?;
    Ok(of_a.is_none_or(|sight| sight.precise_for(b, now_ms)) && of_b.is_none_or(|sight| sight.precise_for(a, now_ms)))
}

/// Whether `viewer` sees `owner`'s precise location right now: the owner
/// shares a position and no rule of theirs hides it from the viewer. For
/// what a domain shows one user of where another was (a photo's place):
/// the same rule as for the pin.
pub async fn sees_precisely(cache: &Cache, owner: Uuid, viewer: Uuid) -> AppResult<bool> {
    Ok(seen_precisely(cache, &[owner], viewer).await?.contains(&owner))
}

/// The owners among `owners` whose precise location `viewer` sees right now
/// ([`sees_precisely`] for each), in one round trip.
pub async fn seen_precisely(cache: &Cache, owners: &[Uuid], viewer: Uuid) -> AppResult<HashSet<Uuid>> {
    if owners.is_empty() {
        return Ok(HashSet::new());
    }
    let members: Vec<String> = owners.iter().map(|owner| owner.simple().to_string()).collect();
    let sights: Vec<String> = owners.iter().map(|owner| sight_key(*owner)).collect();
    let mut conn = cache.conn();
    let (positions, rules): (Vec<Option<(f64, f64)>>, Vec<Option<Vec<u8>>>) = redis::pipe()
        .cmd("GEOPOS").arg(POSITIONS).arg(&members)
        .cmd("MGET").arg(&sights)
        .query_async(&mut conn)
        .await?;
    let now_ms = Utc::now().timestamp_millis();
    Ok(owners
        .iter()
        .zip(positions.iter().zip(&rules))
        .filter(|(_, (position, rule))| {
            let sight = rule.as_deref().and_then(|bytes| serde_json::from_slice::<Sight>(bytes).ok());
            position.is_some() && sight.is_none_or(|sight| sight.precise_for(viewer, now_ms))
        })
        .map(|(owner, _)| *owner)
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub lat: f64,
    pub lng: f64,
    pub recorded_at: DateTime<Utc>,
}

/// Great-circle distance in meters (haversine).
pub fn distance_meters(lat1: f64, lng1: f64, lat2: f64, lng2: f64) -> f64 {
    const EARTH_RADIUS_M: f64 = 6_371_008.8;
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let (dp, dl) = ((lat2 - lat1).to_radians(), (lng2 - lng1).to_radians());
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * a.sqrt().min(1.0).asin()
}

pub fn valid_coordinates(lat: f64, lng: f64) -> bool {
    lat.is_finite() && lng.is_finite() && (-85.05112878..=85.05112878).contains(&lat) && (-180.0..=180.0).contains(&lng)
}

/// Records a user's latest position.
pub async fn set(cache: &Cache, user_id: Uuid, lat: f64, lng: f64, recorded_at: DateTime<Utc>) -> AppResult<()> {
    let member = user_id.simple().to_string();
    let mut conn = cache.conn();
    redis::pipe()
        .cmd("GEOADD").arg(POSITIONS).arg(lng).arg(lat).arg(&member).ignore()
        .cmd("HSET").arg(TIMESTAMPS).arg(&member).arg(recorded_at.timestamp_millis()).ignore()
        .query_async::<()>(&mut conn)
        .await?;
    Ok(())
}

/// Forgets a user's position (sharing stopped, account deleted).
pub async fn remove(cache: &Cache, user_id: Uuid) -> AppResult<()> {
    let member = user_id.simple().to_string();
    let mut conn = cache.conn();
    redis::pipe()
        .cmd("ZREM").arg(POSITIONS).arg(&member).ignore()
        .cmd("HDEL").arg(TIMESTAMPS).arg(&member).ignore()
        .cmd("DEL").arg(sight_key(user_id)).ignore()
        .query_async::<()>(&mut conn)
        .await?;
    Ok(())
}

/// A user's latest position, if one is known.
pub async fn get(cache: &Cache, user_id: Uuid) -> AppResult<Option<Position>> {
    let member = user_id.simple().to_string();
    let mut conn = cache.conn();
    let (positions, millis): (Vec<Option<(f64, f64)>>, Option<i64>) = redis::pipe()
        .cmd("GEOPOS").arg(POSITIONS).arg(&member)
        .cmd("HGET").arg(TIMESTAMPS).arg(&member)
        .query_async(&mut conn)
        .await?;
    let Some(Some((lng, lat))) = positions.into_iter().next() else { return Ok(None) };
    let recorded_at = millis.and_then(DateTime::from_timestamp_millis).unwrap_or_default();
    Ok(Some(Position { lat, lng, recorded_at }))
}

/// Distance in meters between two users' latest positions, when both are
/// known and no older than `max_age` (stale positions prove nothing) — and
/// when each of the two sees the other's precise location: for a pair one of
/// whom hides from the other there is no distance, wherever they are.
pub async fn distance_between(cache: &Cache, a: Uuid, b: Uuid, max_age: std::time::Duration) -> AppResult<Option<f64>> {
    if !mutually_precise(cache, a, b).await? {
        return Ok(None);
    }
    let (Some(pa), Some(pb)) = (get(cache, a).await?, get(cache, b).await?) else { return Ok(None) };
    let oldest = Utc::now() - chrono::Duration::from_std(max_age).unwrap_or(chrono::Duration::MAX);
    if pa.recorded_at < oldest || pb.recorded_at < oldest {
        return Ok(None);
    }
    Ok(Some(distance_meters(pa.lat, pa.lng, pb.lat, pb.lng)))
}

/// Users whose latest position is within `radius_meters` of a point, nearest
/// first, as `(user_id, distance in meters)`. Positions may be stale; check
/// [`get`] when freshness matters. These are true positions of whoever is
/// there: before telling one user anything about another found here, check
/// [`mutually_precise`].
pub async fn nearby(cache: &Cache, lat: f64, lng: f64, radius_meters: f64, limit: usize) -> AppResult<Vec<(Uuid, f64)>> {
    let mut conn = cache.conn();
    let hits: Vec<(String, f64)> = redis::cmd("GEOSEARCH")
        .arg(POSITIONS)
        .arg("FROMLONLAT").arg(lng).arg(lat)
        .arg("BYRADIUS").arg(radius_meters).arg("m")
        .arg("ASC")
        .arg("COUNT").arg(limit.max(1))
        .arg("WITHDIST")
        .query_async(&mut conn)
        .await?;
    Ok(hits.into_iter().filter_map(|(member, dist)| Uuid::parse_str(&member).ok().map(|id| (id, dist))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haversine_matches_known_distances() {
        // Tokyo Station → Shibuya Station ≈ 6.4 km.
        let d = distance_meters(35.681236, 139.767125, 35.658034, 139.701636);
        assert!((6200.0..6600.0).contains(&d), "{d}");
        assert!(distance_meters(35.0, 139.0, 35.0, 139.0) < 0.001);
        // One degree of latitude ≈ 111.2 km.
        let d = distance_meters(0.0, 0.0, 1.0, 0.0);
        assert!((111_000.0..111_400.0).contains(&d), "{d}");
    }

    #[test]
    fn sight_follows_the_rules_and_their_end() {
        let (ren, kai, mio) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let hidden = SightRule { precise: false, until_ms: None };
        assert!(Sight::default().precise_for(ren, 0));
        // Hidden from everyone, with an exception for Kai that runs out.
        let mut sight = Sight { everyone: Some(hidden), ..Default::default() };
        sight.overrides.insert(kai, SightRule { precise: true, until_ms: Some(1_000) });
        assert!(!sight.precise_for(ren, 500) && sight.precise_for(kai, 500));
        assert!(!sight.precise_for(kai, 1_000), "back under the rule for everyone");
        // Hidden from Ren only, for a while.
        let mut sight = Sight::default();
        sight.overrides.insert(ren, SightRule { precise: false, until_ms: Some(1_000) });
        assert!(!sight.precise_for(ren, 999) && sight.precise_for(ren, 1_000) && sight.precise_for(mio, 0));
        // A rule for everyone that ended hides nobody; an SOS shows the people alerted everything.
        let ended = Sight { everyone: Some(SightRule { precise: false, until_ms: Some(10) }), ..Default::default() };
        assert!(ended.precise_for(ren, 10));
        let alarm = Sight { everyone: Some(hidden), alerted: vec![ren], ..Default::default() };
        assert!(alarm.precise_for(ren, 0) && !alarm.precise_for(kai, 0));
    }

    #[test]
    fn coordinate_validation() {
        assert!(valid_coordinates(35.68, 139.76));
        assert!(!valid_coordinates(91.0, 0.0));
        assert!(!valid_coordinates(0.0, 181.0));
        assert!(!valid_coordinates(f64::NAN, 0.0));
    }
}
