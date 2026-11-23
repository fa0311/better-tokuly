//! Who is watching: the audience of a stream, kept in Valkey.
//!
//! Three sources say how many people watch a stream, and none of them sees
//! everyone:
//!
//! - **media readers**: connections reading the stream's path on the media
//!   server (every HLS session, WebRTC session or RTMP/SRT reader). Counts
//!   players outside the app (VLC, OBS), misses a paused player;
//! - **the realtime topic**: connections subscribed to `live:<stream>`.
//!   Counts everyone with the stream page open;
//! - **heartbeats**: players calling `ReportWatching`, kept here as a sorted
//!   set per stream (member = viewer, score = last report). Also the only
//!   source that knows *who* watches (the streamer's viewer list).
//!
//! One person usually shows up in all three, so the viewer count is the
//! largest of them, not their sum ([`viewers`]).
//!
//! | Key | Type | Purpose |
//! |---|---|---|
//! | `live:watching:<stream>` | sorted set, TTL 10 min | viewers by last heartbeat |
//! | `live:dashboard:<stream>` | marker, TTL 3 heartbeats | the streamer has their dashboard open |
use std::time::Duration;

use tokuly_core::{prelude::*, presence};

use crate::streams::topic;

/// A heartbeat counts for this many report intervals.
pub const MISSED_REPORTS: u32 = 3;

fn watching_key(stream_id: Uuid) -> String {
    format!("live:watching:{stream_id}")
}

fn dashboard_key(stream_id: Uuid) -> String {
    format!("live:dashboard:{stream_id}")
}

/// A viewer as the audience set names them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewerKey {
    User(Uuid),
    /// An anonymous client: a keyed hash of its client ID and address.
    Guest(String),
}

impl ViewerKey {
    /// A guest key for `client_id` at `ip`.
    pub fn guest(core: &Core, client_id: &str, ip: &str) -> Self {
        let mac = core.keys.mac("live/viewer", format!("{client_id}\n{ip}").as_bytes());
        Self::Guest(mac[..12].iter().map(|byte| format!("{byte:02x}")).collect())
    }

    pub fn member(&self) -> String {
        match self {
            Self::User(id) => format!("u:{id}"),
            Self::Guest(hash) => format!("g:{hash}"),
        }
    }

    /// The name analytics events carry (`video.watch` uses the same shape).
    pub fn analytics_name(&self) -> String {
        match self {
            Self::User(id) => id.to_string(),
            Self::Guest(hash) => format!("guest:{hash}"),
        }
    }
}

/// How long a heartbeat keeps a viewer in the audience.
pub fn ttl_seconds(report_interval_seconds: u32) -> i64 {
    i64::from(report_interval_seconds * MISSED_REPORTS)
}

/// Records a heartbeat. Returns the seconds since the viewer's previous
/// one, or `None` when they were not in the audience (a new viewer).
pub async fn heartbeat(
    core: &Core,
    stream_id: Uuid,
    viewer: &ViewerKey,
    report_interval_seconds: u32,
) -> AppResult<Option<i64>> {
    let key = watching_key(stream_id);
    let member = viewer.member();
    let now = now().timestamp();
    let mut conn = core.cache.conn();
    let previous: Option<f64> = redis::cmd("ZSCORE").arg(&key).arg(&member).query_async(&mut conn).await?;
    redis::pipe()
        .cmd("ZADD")
        .arg(&key)
        .arg(now)
        .arg(&member)
        .ignore()
        .cmd("EXPIRE")
        .arg(&key)
        .arg(600)
        .ignore()
        .query_async::<()>(&mut conn)
        .await?;
    let ttl = ttl_seconds(report_interval_seconds);
    Ok(previous.map(|at| now - at as i64).filter(|elapsed| (0..=ttl).contains(elapsed)))
}

/// Anonymous viewers one address may have in one audience at a time.
/// Guests are whatever client ID a request names, so without a cap one
/// machine could be a crowd. (People behind one shared address beyond
/// this number are not counted unless they sign in.)
pub const MAX_GUESTS_PER_ADDRESS: i64 = 5;

fn guests_key(stream_id: Uuid, group: &str) -> String {
    format!("live:guests:{stream_id}:{group}")
}

/// The group an address is capped with: the address itself for IPv4, its
/// /64 network for IPv6 (one subscriber usually has a whole /64).
pub fn address_group(ip: &str) -> String {
    match ip.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V6(v6)) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let s = v6.segments();
                format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
            }
        },
        Ok(std::net::IpAddr::V4(v4)) => v4.to_string(),
        Err(_) => "unknown".to_owned(),
    }
}

/// Decides in one step whether a guest may be (or stay) in the audience:
/// already counted for its address group, or the group has room.
const ADMIT_GUEST: &str = r#"
redis.call('ZREMRANGEBYSCORE', KEYS[1], '-inf', '(' .. ARGV[1])
if redis.call('ZSCORE', KEYS[1], ARGV[3]) or redis.call('ZCARD', KEYS[1]) < tonumber(ARGV[4]) then
  redis.call('ZADD', KEYS[1], ARGV[2], ARGV[3])
  redis.call('EXPIRE', KEYS[1], 600)
  return 1
end
return 0
"#;

/// Whether the guest `viewer` at `ip` counts towards the audience: at most
/// [`MAX_GUESTS_PER_ADDRESS`] guests per address group and stream do.
pub async fn admit_guest(
    core: &Core,
    stream_id: Uuid,
    ip: &str,
    viewer: &ViewerKey,
    report_interval_seconds: u32,
) -> AppResult<bool> {
    let now = now().timestamp();
    let admitted: i64 = redis::Script::new(ADMIT_GUEST)
        .key(guests_key(stream_id, &address_group(ip)))
        .arg(now - ttl_seconds(report_interval_seconds))
        .arg(now)
        .arg(viewer.member())
        .arg(MAX_GUESTS_PER_ADDRESS)
        .invoke_async(&mut core.cache.conn())
        .await?;
    Ok(admitted == 1)
}

/// Takes a viewer out of the audience again.
pub async fn forget(core: &Core, stream_id: Uuid, viewer: &ViewerKey) -> AppResult<()> {
    redis::cmd("ZREM")
        .arg(watching_key(stream_id))
        .arg(viewer.member())
        .query_async::<()>(&mut core.cache.conn())
        .await?;
    Ok(())
}

/// Viewers whose last heartbeat is recent enough.
pub async fn heartbeat_count(core: &Core, stream_id: Uuid, report_interval_seconds: u32) -> AppResult<i64> {
    let key = watching_key(stream_id);
    let cutoff = now().timestamp() - ttl_seconds(report_interval_seconds);
    let mut conn = core.cache.conn();
    redis::cmd("ZREMRANGEBYSCORE").arg(&key).arg("-inf").arg(format!("({cutoff}")).query_async::<()>(&mut conn).await?;
    Ok(redis::cmd("ZCARD").arg(&key).query_async(&mut conn).await?)
}

/// Signed-in viewers in the audience with the time of their last
/// heartbeat, most recent first.
pub async fn watching_users(
    core: &Core,
    stream_id: Uuid,
    report_interval_seconds: u32,
) -> AppResult<Vec<(Uuid, Time)>> {
    let cutoff = now().timestamp() - ttl_seconds(report_interval_seconds);
    let members: Vec<(String, f64)> = redis::cmd("ZRANGE")
        .arg(watching_key(stream_id))
        .arg("+inf")
        .arg(cutoff)
        .arg("BYSCORE")
        .arg("REV")
        .arg("WITHSCORES")
        .query_async(&mut core.cache.conn())
        .await?;
    Ok(members
        .into_iter()
        .filter_map(|(member, at)| {
            let id = member.strip_prefix("u:")?.parse().ok()?;
            Some((id, chrono::DateTime::from_timestamp(at as i64, 0)?))
        })
        .collect())
}

/// The viewer count: the largest of the three sources.
pub async fn viewers(core: &Core, stream_id: Uuid, media_readers: i64, report_interval_seconds: u32) -> AppResult<i64> {
    let subscribed = presence::topic_audience(&core.cache, &topic(stream_id)).await?;
    let reporting = heartbeat_count(core, stream_id, report_interval_seconds).await?;
    Ok(media_readers.max(subscribed).max(reporting).max(0))
}

/// Forgets the audience of a stream that is over.
pub async fn clear(core: &Core, stream_id: Uuid) -> AppResult<()> {
    core.cache.del(&watching_key(stream_id)).await?;
    core.cache.del(&dashboard_key(stream_id)).await
}

/// Notes that the streamer has the dashboard of their stream open.
pub async fn mark_dashboard_open(core: &Core, stream_id: Uuid, report_interval_seconds: u32) -> AppResult<()> {
    let ttl = Duration::from_secs(ttl_seconds(report_interval_seconds) as u64);
    core.cache.set_string(&dashboard_key(stream_id), "1", Some(ttl)).await
}

/// Whether the streamer is looking at the dashboard of their stream.
pub async fn dashboard_open(core: &Core, stream_id: Uuid) -> AppResult<bool> {
    core.cache.exists(&dashboard_key(stream_id)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewer_keys_name_users_and_guests_apart() {
        let id = Uuid::now_v7();
        assert_eq!(ViewerKey::User(id).member(), format!("u:{id}"));
        assert_eq!(ViewerKey::User(id).analytics_name(), id.to_string());
        let guest = ViewerKey::Guest("abc123".into());
        assert_eq!(guest.member(), "g:abc123");
        assert_eq!(guest.analytics_name(), "guest:abc123");
        assert_eq!(ttl_seconds(15), 45);
    }

    #[test]
    fn addresses_are_capped_per_host_or_per_ipv6_network() {
        assert_eq!(address_group("203.0.113.7"), "203.0.113.7");
        assert_eq!(address_group("2001:db8:1:2:aaaa:bbbb:cccc:dddd"), "2001:db8:1:2::/64");
        assert_eq!(address_group("2001:db8:1:2::1"), address_group("2001:db8:1:2:ffff::9"));
        assert_ne!(address_group("2001:db8:1:3::1"), address_group("2001:db8:1:2::1"));
        assert_eq!(address_group("::ffff:203.0.113.7"), "203.0.113.7");
        assert_eq!(address_group(""), "unknown");
    }
}
