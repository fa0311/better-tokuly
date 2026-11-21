//! Creator analytics, computed from the ClickHouse `events` table
//! (`tokuly_core::analytics`).
//!
//! Events this domain tracks, all with `owner_id` = the channel's owner:
//!
//! | Name | Subject | Value | Props |
//! |---|---|---|---|
//! | `video.play` | video | — | `channel_id`, `session`, `viewer`, `source` |
//! | `video.watch` | video | seconds watched | the same, plus `from`/`to` (retention buckets 0-99) |
//! | `video.view` | video | 1 | `channel_id`, `session`, `viewer`, `source` |
//! | `video.like` / `video.dislike` | video | — | `channel_id` |
//! | `channel.subscribe` / `channel.unsubscribe` | channel | — | `video_id` (where the viewer subscribed from; may be empty) |
//!
//! `viewer` is the account ID, or a keyed hash of the network address for
//! anonymous viewers; `source` is `TrafficSource` as its number.
use chrono::{Days, NaiveDate, TimeZone as _};
use tokuly_core::{analytics::AnalyticsEvent, prelude::*};
use tokuly_proto::video::{AnalyticsTotals, DailyAnalytics, DateRange, TrafficSourceStats};

use crate::{channels::ChannelRow, playback::RETENTION_BUCKETS, videos::VideoRow};

/// Longest range one request may cover, in days.
pub const MAX_RANGE_DAYS: i64 = 366;

fn unavailable(error: clickhouse::error::Error) -> AppError {
    AppError::unavailable("ANALYTICS_UNAVAILABLE", format!("analytics query failed: {error}"))
}

/// A validated date range: the days it covers and the instants that bound it.
#[derive(Debug, Clone)]
pub struct Range {
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub time_zone: chrono_tz::Tz,
    /// First instant of `start` and first instant after `end`, Unix milliseconds.
    pub from_ms: i64,
    pub until_ms: i64,
}

impl Range {
    pub fn parse(range: Option<&DateRange>) -> AppResult<Self> {
        let bad = |message: &str| AppError::invalid("DATE_RANGE_INVALID", message.to_owned());
        let range = range.ok_or_else(|| bad("a date range is required"))?;
        let time_zone: chrono_tz::Tz = if range.time_zone.trim().is_empty() {
            chrono_tz::UTC
        } else {
            range.time_zone.trim().parse().map_err(|_| AppError::invalid("TIME_ZONE_INVALID", "unknown time zone"))?
        };
        let date = |text: &str| {
            NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d").map_err(|_| bad("dates must be YYYY-MM-DD"))
        };
        let (start, end) = (date(&range.start_date)?, date(&range.end_date)?);
        if end < start {
            return Err(bad("the range ends before it starts"));
        }
        if (end - start).num_days() > MAX_RANGE_DAYS {
            return Err(bad("the range is longer than 366 days"));
        }
        let midnight = |day: NaiveDate| -> AppResult<i64> {
            let local = day.and_hms_opt(0, 0, 0).ok_or_else(|| bad("date out of range"))?;
            // A midnight skipped by a clock change starts at the first valid instant after it.
            let instant = time_zone
                .from_local_datetime(&local)
                .earliest()
                .or_else(|| time_zone.from_local_datetime(&(local + chrono::Duration::hours(1))).earliest())
                .ok_or_else(|| bad("date out of range"))?;
            Ok(instant.timestamp_millis())
        };
        let after_end = end.checked_add_days(Days::new(1)).ok_or_else(|| bad("date out of range"))?;
        Ok(Self { start, end, time_zone, from_ms: midnight(start)?, until_ms: midnight(after_end)? })
    }

    /// Every day of the range, in order.
    pub fn days(&self) -> impl Iterator<Item = NaiveDate> {
        self.start.iter_days().take_while(move |day| *day <= self.end)
    }
}

/// What a report is about.
#[derive(Debug, Clone, Copy)]
pub enum Scope {
    Channel { owner_id: Uuid, channel_id: Uuid },
    Video { owner_id: Uuid, video_id: Uuid },
}

impl Scope {
    pub fn channel(channel: &ChannelRow) -> Self {
        Self::Channel { owner_id: channel.owner_id, channel_id: channel.id }
    }

    pub fn video(video: &VideoRow) -> Self {
        Self::Video { owner_id: video.owner_id, video_id: video.id }
    }

    fn owner_id(&self) -> Uuid {
        match self {
            Self::Channel { owner_id, .. } | Self::Video { owner_id, .. } => *owner_id,
        }
    }

    fn target(&self) -> Uuid {
        match self {
            Self::Channel { channel_id, .. } => *channel_id,
            Self::Video { video_id, .. } => *video_id,
        }
    }

    /// SQL predicate selecting this scope's events; binds the target ID twice.
    fn predicate(&self) -> &'static str {
        match self {
            // Video events carry their channel; subscription events are about the channel itself.
            Self::Channel { .. } => {
                "((subject_type = 'video' AND JSONExtractString(props, 'channel_id') = ?) OR (subject_type = 'channel' AND subject_id = toUUID(?)))"
            }
            // Subscriptions made from a video's page are credited to that video.
            Self::Video { .. } => {
                "((subject_type = 'video' AND subject_id = toUUID(?)) OR (subject_type = 'channel' AND JSONExtractString(props, 'video_id') = ?))"
            }
        }
    }
}

const NAMES: &str =
    "('video.view', 'video.watch', 'video.like', 'video.dislike', 'channel.subscribe', 'channel.unsubscribe')";

const AGGREGATES: &str = "toUInt64(countIf(name = 'video.view')), \
     toUInt64(uniqExactIf(JSONExtractString(props, 'viewer'), name = 'video.view')), \
     toFloat64(sumIf(value, name = 'video.watch')), \
     toUInt64(countIf(name = 'channel.subscribe')), \
     toUInt64(countIf(name = 'channel.unsubscribe')), \
     toUInt64(countIf(name = 'video.like')), \
     toUInt64(countIf(name = 'video.dislike'))";

type Aggregates = (u64, u64, f64, u64, u64, u64, u64);

/// Totals over the range.
pub async fn totals(core: &Core, scope: Scope, range: &Range) -> AppResult<AnalyticsTotals> {
    let sql = format!(
        "SELECT {AGGREGATES} FROM events
         WHERE owner_id = toUUID(?) AND name IN {NAMES}
           AND at >= fromUnixTimestamp64Milli(?) AND at < fromUnixTimestamp64Milli(?) AND {}",
        scope.predicate()
    );
    let target = scope.target().to_string();
    let (views, unique_viewers, watch_seconds, gained, lost, likes, dislikes): Aggregates = core
        .analytics
        .ch
        .query(&sql)
        .bind(scope.owner_id().to_string())
        .bind(range.from_ms)
        .bind(range.until_ms)
        .bind(&target)
        .bind(&target)
        .fetch_one()
        .await
        .map_err(unavailable)?;
    Ok(AnalyticsTotals {
        views: views as i64,
        unique_viewers: unique_viewers as i64,
        watch_time_seconds: watch_seconds,
        average_view_duration_seconds: if views > 0 { watch_seconds / views as f64 } else { 0.0 },
        subscribers_gained: gained as i64,
        subscribers_lost: lost as i64,
        likes: likes as i64,
        dislikes: dislikes as i64,
        ..Default::default()
    })
}

/// One entry per day of the range (days without activity included), cut in
/// the range's time zone.
pub async fn daily(core: &Core, scope: Scope, range: &Range) -> AppResult<Vec<DailyAnalytics>> {
    let sql = format!(
        "SELECT toString(toDate(at, ?)) AS day, {AGGREGATES} FROM events
         WHERE owner_id = toUUID(?) AND name IN {NAMES}
           AND at >= fromUnixTimestamp64Milli(?) AND at < fromUnixTimestamp64Milli(?) AND {}
         GROUP BY day",
        scope.predicate()
    );
    let target = scope.target().to_string();
    let rows: Vec<(String, u64, u64, f64, u64, u64, u64, u64)> = core
        .analytics
        .ch
        .query(&sql)
        .bind(range.time_zone.name())
        .bind(scope.owner_id().to_string())
        .bind(range.from_ms)
        .bind(range.until_ms)
        .bind(&target)
        .bind(&target)
        .fetch_all()
        .await
        .map_err(unavailable)?;
    Ok(range
        .days()
        .map(|day| {
            let date = day.to_string();
            let row = rows.iter().find(|row| row.0 == date);
            DailyAnalytics {
                views: row.map(|r| r.1 as i64).unwrap_or(0),
                unique_viewers: row.map(|r| r.2 as i64).unwrap_or(0),
                watch_time_seconds: row.map(|r| r.3).unwrap_or(0.0),
                subscribers_gained: row.map(|r| r.4 as i64).unwrap_or(0),
                subscribers_lost: row.map(|r| r.5 as i64).unwrap_or(0),
                date,
                ..Default::default()
            }
        })
        .collect())
}

/// Views and watch time by where viewers came from, most views first.
pub async fn traffic_sources(core: &Core, scope: Scope, range: &Range) -> AppResult<Vec<TrafficSourceStats>> {
    let sql = format!(
        "SELECT toInt32(JSONExtractInt(props, 'source')) AS source,
                toUInt64(countIf(name = 'video.view')) AS views,
                toFloat64(sumIf(value, name = 'video.watch')) AS watch_seconds
         FROM events
         WHERE owner_id = toUUID(?) AND name IN ('video.view', 'video.watch')
           AND at >= fromUnixTimestamp64Milli(?) AND at < fromUnixTimestamp64Milli(?) AND {}
         GROUP BY source
         ORDER BY views DESC, watch_seconds DESC, source",
        scope.predicate()
    );
    let target = scope.target().to_string();
    let rows: Vec<(i32, u64, f64)> = core
        .analytics
        .ch
        .query(&sql)
        .bind(scope.owner_id().to_string())
        .bind(range.from_ms)
        .bind(range.until_ms)
        .bind(&target)
        .bind(&target)
        .fetch_all()
        .await
        .map_err(unavailable)?;
    Ok(rows
        .into_iter()
        .map(|(source, views, watch_seconds)| TrafficSourceStats {
            source: source.into(),
            views: views as i64,
            watch_time_seconds: watch_seconds,
            ..Default::default()
        })
        .collect())
}

/// The most viewed videos of a channel between two instants:
/// `(video, views, watch seconds)`.
pub async fn top_videos(
    core: &Core,
    channel: &ChannelRow,
    from_ms: i64,
    until_ms: i64,
    limit: u32,
) -> AppResult<Vec<(Uuid, i64, f64)>> {
    let rows: Vec<(String, u64, f64)> = core
        .analytics
        .ch
        .query(
            "SELECT toString(subject_id) AS video,
                    toUInt64(countIf(name = 'video.view')) AS views,
                    toFloat64(sumIf(value, name = 'video.watch')) AS watch_seconds
             FROM events
             WHERE owner_id = toUUID(?) AND name IN ('video.view', 'video.watch') AND subject_type = 'video'
               AND at >= fromUnixTimestamp64Milli(?) AND at < fromUnixTimestamp64Milli(?)
               AND JSONExtractString(props, 'channel_id') = ?
             GROUP BY subject_id
             HAVING views > 0
             ORDER BY views DESC, watch_seconds DESC, video
             LIMIT ?",
        )
        .bind(channel.owner_id.to_string())
        .bind(from_ms)
        .bind(until_ms)
        .bind(channel.id.to_string())
        .bind(limit)
        .fetch_all()
        .await
        .map_err(unavailable)?;
    Ok(rows
        .into_iter()
        .filter_map(|(video, views, watch)| Some((Uuid::parse_str(&video).ok()?, views as i64, watch)))
        .collect())
}

/// Turns per-bucket session counts into the retention curve: for each
/// hundredth of the video, the share of sessions that watched it.
pub fn retention_curve(sessions: u64, buckets: &[(u8, u64)]) -> Vec<f64> {
    if sessions == 0 {
        return Vec::new();
    }
    let mut curve = vec![0.0; RETENTION_BUCKETS];
    for (bucket, count) in buckets {
        if let Some(slot) = curve.get_mut(*bucket as usize) {
            *slot = (*count as f64 / sessions as f64).min(1.0);
        }
    }
    curve
}

/// The audience retention curve of a video over the range: 100 values, or
/// none when nobody watched.
pub async fn retention(core: &Core, video: &VideoRow, range: &Range) -> AppResult<Vec<f64>> {
    let owner = video.owner_id.to_string();
    let target = video.id.to_string();
    let sessions: u64 = core
        .analytics
        .ch
        .query(
            "SELECT toUInt64(uniqExact(JSONExtractString(props, 'session'))) FROM events
             WHERE owner_id = toUUID(?) AND name = 'video.play' AND subject_id = toUUID(?)
               AND at >= fromUnixTimestamp64Milli(?) AND at < fromUnixTimestamp64Milli(?)",
        )
        .bind(&owner)
        .bind(&target)
        .bind(range.from_ms)
        .bind(range.until_ms)
        .fetch_one()
        .await
        .map_err(unavailable)?;
    if sessions == 0 {
        return Ok(Vec::new());
    }
    let buckets: Vec<(u8, u64)> = core
        .analytics
        .ch
        .query(
            "SELECT toUInt8(bucket) AS bucket, toUInt64(uniqExact(session)) AS sessions FROM (
                 SELECT JSONExtractString(props, 'session') AS session,
                        arrayJoin(range(toUInt32(JSONExtractUInt(props, 'from')), toUInt32(least(JSONExtractUInt(props, 'to'), 99)) + 1)) AS bucket
                 FROM events
                 WHERE owner_id = toUUID(?) AND name = 'video.watch' AND subject_id = toUUID(?)
                   AND at >= fromUnixTimestamp64Milli(?) AND at < fromUnixTimestamp64Milli(?)
                   AND JSONHas(props, 'from')
             )
             GROUP BY bucket",
        )
        .bind(&owner)
        .bind(&target)
        .bind(range.from_ms)
        .bind(range.until_ms)
        .fetch_all()
        .await
        .map_err(unavailable)?;
    Ok(retention_curve(sessions, &buckets))
}

/// Views of the recent past: per hour for the last 48 hours and per minute
/// for the last 60 minutes, oldest first (the last entry is the current
/// hour / minute).
///
/// Slots are cut on the Unix clock (`timestamp / slot length`), not with
/// `dateDiff`, which compares wall-clock fields and so depends on the time
/// zones of its arguments and of the ClickHouse server.
pub async fn realtime(core: &Core, scope: Scope) -> AppResult<(Vec<i64>, Vec<i64>)> {
    let target = scope.target().to_string();
    let series = |slot_seconds: i64, slots: usize| {
        let sql = format!(
            "SELECT toInt64(intDiv(toInt64(toUnixTimestamp(now())), ?) - intDiv(toInt64(toUnixTimestamp(at)), ?)) AS ago,
                    toUInt64(count()) AS views
             FROM events
             WHERE owner_id = toUUID(?) AND name = 'video.view' AND at > now() - toIntervalSecond(?) AND {}
             GROUP BY ago",
            scope.predicate()
        );
        let query = core
            .analytics
            .ch
            .query(&sql)
            .bind(slot_seconds)
            .bind(slot_seconds)
            .bind(scope.owner_id().to_string())
            .bind(slot_seconds * slots as i64)
            .bind(&target)
            .bind(&target);
        async move {
            let rows: Vec<(i64, u64)> = query.fetch_all().await.map_err(unavailable)?;
            let mut out = vec![0i64; slots];
            for (ago, views) in rows {
                if (0..slots as i64).contains(&ago) {
                    out[slots - 1 - ago as usize] += views as i64;
                }
            }
            AppResult::Ok(out)
        }
    };
    let (hours, minutes) = tokio::try_join!(series(3600, 48), series(60, 60))?;
    Ok((hours, minutes))
}

// ───────────────────────────── tracking ─────────────────────────────

/// Records that `user_id` subscribed to (or left) a channel, crediting the
/// video they were watching when they did.
pub async fn track_subscription(
    core: &Core,
    channel: &ChannelRow,
    user_id: Uuid,
    subscribed: bool,
    source_video: Option<Uuid>,
) {
    let name = if subscribed { "channel.subscribe" } else { "channel.unsubscribe" };
    let event = AnalyticsEvent::new(name)
        .user(Some(user_id))
        .subject("channel", channel.id)
        .owner(channel.owner_id)
        .props(serde_json::json!({ "video_id": source_video.map(|id| id.to_string()).unwrap_or_default() }));
    core.analytics.track(event).await;
}

/// Records a new like or dislike of a video.
pub async fn track_rating(core: &Core, video: &VideoRow, user_id: Uuid, like: bool) {
    let event = AnalyticsEvent::new(if like { "video.like" } else { "video.dislike" })
        .user(Some(user_id))
        .subject("video", video.id)
        .owner(video.owner_id)
        .props(serde_json::json!({ "channel_id": video.channel_id }));
    core.analytics.track(event).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: &str, end: &str, zone: &str) -> AppResult<Range> {
        Range::parse(Some(&DateRange {
            start_date: start.into(),
            end_date: end.into(),
            time_zone: zone.into(),
            ..Default::default()
        }))
    }

    #[test]
    fn ranges_are_cut_at_local_midnights() {
        let tokyo = range("2026-11-01", "2026-11-03", "Asia/Tokyo").unwrap();
        // Midnight in Tokyo is 15:00 UTC the day before.
        assert_eq!(
            tokyo.from_ms,
            chrono::DateTime::parse_from_rfc3339("2026-10-31T15:00:00Z").unwrap().timestamp_millis()
        );
        assert_eq!(
            tokyo.until_ms,
            chrono::DateTime::parse_from_rfc3339("2026-11-03T15:00:00Z").unwrap().timestamp_millis()
        );
        assert_eq!(
            tokyo.days().map(|d| d.to_string()).collect::<Vec<_>>(),
            vec!["2026-11-01", "2026-11-02", "2026-11-03"]
        );
        let utc = range("2026-11-21", "2026-11-21", "").unwrap();
        assert_eq!(utc.until_ms - utc.from_ms, 86_400_000);
        // A day with a clock change is 23 or 25 hours long.
        let spring = range("2026-03-08", "2026-03-08", "America/New_York").unwrap();
        assert_eq!(spring.until_ms - spring.from_ms, 23 * 3_600_000);
    }

    #[test]
    fn bad_ranges_have_reasons() {
        assert_eq!(Range::parse(None).unwrap_err().reason, "DATE_RANGE_INVALID");
        assert_eq!(range("2026-11-03", "2026-11-01", "").unwrap_err().reason, "DATE_RANGE_INVALID");
        assert_eq!(range("2026-11-03", "soon", "").unwrap_err().reason, "DATE_RANGE_INVALID");
        assert_eq!(range("2025-01-01", "2026-06-01", "").unwrap_err().reason, "DATE_RANGE_INVALID");
        assert_eq!(range("2026-11-01", "2026-11-02", "Mars/Olympus").unwrap_err().reason, "TIME_ZONE_INVALID");
        assert!(range("2025-11-21", "2026-11-21", "").is_ok(), "a full year is allowed");
    }

    #[test]
    fn retention_is_a_share_of_sessions_per_bucket() {
        assert!(retention_curve(0, &[(0, 3)]).is_empty());
        let curve = retention_curve(4, &[(0, 4), (1, 3), (99, 1), (200, 9)]);
        assert_eq!(curve.len(), 100);
        assert_eq!((curve[0], curve[1], curve[2], curve[99]), (1.0, 0.75, 0.0, 0.25));
        // More sessions watched a bucket than started in the range (they began before it): capped.
        assert_eq!(retention_curve(2, &[(5, 3)])[5], 1.0);
    }
}
