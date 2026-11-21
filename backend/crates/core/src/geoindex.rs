//! The shared index of users' latest positions (Valkey GEO).
//!
//! The geo domain writes it on every accepted location sample; any domain
//! may read it for distance checks ("are these two within 200 m?") and
//! proximity searches. Positions here are the *true* latest positions: they
//! are server-side facts for rule checks, never something to send to another
//! user (ghost mode is applied by the geo domain when sharing locations).
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{cache::Cache, error::AppResult};

const POSITIONS: &str = "geo:pos";
const TIMESTAMPS: &str = "geo:pos:ts";

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
/// known and no older than `max_age` (stale positions prove nothing).
pub async fn distance_between(cache: &Cache, a: Uuid, b: Uuid, max_age: std::time::Duration) -> AppResult<Option<f64>> {
    let (Some(pa), Some(pb)) = (get(cache, a).await?, get(cache, b).await?) else { return Ok(None) };
    let oldest = Utc::now() - chrono::Duration::from_std(max_age).unwrap_or(chrono::Duration::MAX);
    if pa.recorded_at < oldest || pb.recorded_at < oldest {
        return Ok(None);
    }
    Ok(Some(distance_meters(pa.lat, pa.lng, pb.lat, pb.lng)))
}

/// Users whose latest position is within `radius_meters` of a point, nearest
/// first, as `(user_id, distance in meters)`. Positions may be stale; check
/// [`get`] when freshness matters.
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
    fn coordinate_validation() {
        assert!(valid_coordinates(35.68, 139.76));
        assert!(!valid_coordinates(91.0, 0.0));
        assert!(!valid_coordinates(0.0, 181.0));
        assert!(!valid_coordinates(f64::NAN, 0.0));
    }
}
