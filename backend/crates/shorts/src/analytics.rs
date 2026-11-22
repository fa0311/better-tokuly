//! Creator analytics, computed from the ClickHouse `events` table
//! (`tokuly_core::analytics`).
//!
//! Events this domain tracks, all with subject `("short", id)`, `owner_id`
//! = the channel's owner and `channel_id` in the props:
//!
//! | Name | Value | Props | When |
//! |---|---|---|---|
//! | `short.watch` | seconds played | `viewer`, `source`, `ratio`, `loops`, `completed` | every reported stay |
//! | `short.view` | 1 | `viewer`, `source` | a stay counted as a view |
//! | `short.like` | +n / −n | — | the like count moved (engagement) |
//! | `short.comment` | +n / −n | — | the comment count moved (engagement) |
//! | `short.save` | +1 / −1 | — | saved / removed from a saved list |
//! | `short.share` | 1 | `target` | shared |
//! | `short.follow` | 1 | — | someone subscribed to the channel within an hour of watching the short |
//! | `short.not_interested` | 1 | — | marked "not interested" (not shown to creators) |
//!
//! `viewer` is the account ID, or a keyed hash of the network address for
//! anonymous viewers; `source` is `PlaybackSource` as its number. Stays of
//! the owner on their own shorts are not tracked.
//!
//! # Small audiences
//!
//! Counts are reported as they are. *How* people watched (the
//! watch-through ratio, loops, watch time, plays) is an average over
//! viewers, and an average over one viewer is that viewer: the creator of
//! a friends-only short with a single view would learn how long that
//! friend lasted. Such numbers are reported only where at least
//! [`MIN_AUDIENCE`] distinct viewers stand behind them (in the range, on
//! the day, from the source, for the short) and are zero below that.
use chrono::NaiveDate;
use serde::Deserialize;
use tokuly_core::prelude::*;
use tokuly_proto::shorts::{DailyShortAnalytics, DateRange, PlaybackSourceStats, ShortAnalyticsTotals};
pub use tokuly_video::analytics::Range;
use tokuly_video::channels::ChannelRow;

use crate::shorts::ShortRow;

/// Distinct viewers that must stand behind a number describing how people
/// watched before it is shown to the creator.
pub const MIN_AUDIENCE: i64 = 5;

/// `value` if at least [`MIN_AUDIENCE`] viewers stand behind it.
fn for_an_audience<T: Default>(viewers: u64, value: T) -> T {
    if viewers as i64 >= MIN_AUDIENCE { value } else { T::default() }
}

fn unavailable(error: clickhouse::error::Error) -> AppError {
    AppError::unavailable("ANALYTICS_UNAVAILABLE", format!("analytics query failed: {error}"))
}

/// Validates a date range (`DATE_RANGE_INVALID`, `TIME_ZONE_INVALID`): the
/// same rules as the video domain's analytics.
pub fn parse_range(range: Option<&DateRange>) -> AppResult<Range> {
    let range = range.map(|range| tokuly_proto::video::DateRange {
        start_date: range.start_date.clone(),
        end_date: range.end_date.clone(),
        time_zone: range.time_zone.clone(),
        ..Default::default()
    });
    Range::parse(range.as_ref())
}

/// What a report is about.
#[derive(Debug, Clone, Copy)]
pub enum Scope {
    Short { owner_id: Uuid, short_id: Uuid },
    Channel { owner_id: Uuid, channel_id: Uuid },
}

impl Scope {
    pub fn short(short: &ShortRow) -> Self {
        Self::Short { owner_id: short.owner_id, short_id: short.id }
    }

    pub fn channel(channel: &ChannelRow) -> Self {
        Self::Channel { owner_id: channel.owner_id, channel_id: channel.id }
    }

    fn owner_id(&self) -> Uuid {
        match self {
            Self::Short { owner_id, .. } | Self::Channel { owner_id, .. } => *owner_id,
        }
    }

    fn target(&self) -> Uuid {
        match self {
            Self::Short { short_id, .. } => *short_id,
            Self::Channel { channel_id, .. } => *channel_id,
        }
    }

    /// SQL predicate selecting this scope's events; binds the target ID once.
    fn predicate(&self) -> &'static str {
        match self {
            Self::Short { .. } => "subject_id = toUUID(?)",
            Self::Channel { .. } => "JSONExtractString(props, 'channel_id') = ?",
        }
    }
}

const NAMES: &str =
    "('short.view', 'short.watch', 'short.like', 'short.comment', 'short.save', 'short.share', 'short.follow')";

/// The aggregates of [`Counts`], by column name.
const AGGREGATES: &str = "toUInt64(countIf(name = 'short.view')) AS views, \
     toUInt64(uniqExactIf(JSONExtractString(props, 'viewer'), name = 'short.view')) AS viewers, \
     toFloat64(ifNotFinite(avgIf(JSONExtractFloat(props, 'ratio'), name = 'short.watch'), 0)) AS ratio, \
     toInt64(sumIf(JSONExtractInt(props, 'loops'), name = 'short.watch')) AS loops, \
     toInt64(sumIf(toInt64(value), name = 'short.like')) AS likes, \
     toInt64(sumIf(toInt64(value), name = 'short.comment')) AS comments, \
     toUInt64(countIf(name = 'short.share')) AS shares, \
     toInt64(sumIf(toInt64(value), name = 'short.save')) AS saves, \
     toUInt64(countIf(name = 'short.follow')) AS follows, \
     toFloat64(sumIf(value, name = 'short.watch')) AS watch_seconds, \
     toUInt64(countIf(name = 'short.watch')) AS plays, \
     toUInt64(uniqExactIf(JSONExtractString(props, 'viewer'), name = 'short.watch')) AS audience";

#[derive(Debug, Clone, Default, clickhouse::Row, Deserialize)]
struct Counts {
    views: u64,
    viewers: u64,
    ratio: f64,
    loops: i64,
    likes: i64,
    comments: i64,
    shares: u64,
    saves: i64,
    follows: u64,
    watch_seconds: f64,
    plays: u64,
    /// Distinct viewers whose way of watching was recorded.
    audience: u64,
}

#[derive(Debug, Clone, clickhouse::Row, Deserialize)]
struct DayCounts {
    day: String,
    views: u64,
    viewers: u64,
    ratio: f64,
    loops: i64,
    likes: i64,
    comments: i64,
    shares: u64,
    saves: i64,
    follows: u64,
    watch_seconds: f64,
    plays: u64,
    /// Distinct viewers whose way of watching was recorded.
    audience: u64,
}

/// Totals over the range.
pub async fn totals(core: &Core, scope: Scope, range: &Range) -> AppResult<ShortAnalyticsTotals> {
    let sql = format!(
        "SELECT {AGGREGATES} FROM events
         WHERE owner_id = toUUID(?) AND subject_type = 'short' AND name IN {NAMES}
           AND at >= fromUnixTimestamp64Milli(?) AND at < fromUnixTimestamp64Milli(?) AND {}",
        scope.predicate()
    );
    let counts: Counts = core
        .analytics
        .ch
        .query(&sql)
        .bind(scope.owner_id().to_string())
        .bind(range.from_ms)
        .bind(range.until_ms)
        .bind(scope.target().to_string())
        .fetch_one()
        .await
        .map_err(unavailable)?;
    Ok(ShortAnalyticsTotals {
        views: counts.views as i64,
        unique_viewers: counts.viewers as i64,
        average_watch_ratio: for_an_audience(counts.audience, counts.ratio),
        loops: for_an_audience(counts.audience, counts.loops),
        likes: counts.likes,
        comments: counts.comments,
        shares: counts.shares as i64,
        saves: counts.saves,
        followers_gained: counts.follows as i64,
        watch_time_seconds: for_an_audience(counts.audience, counts.watch_seconds),
        plays: for_an_audience(counts.audience, counts.plays as i64),
        ..Default::default()
    })
}

/// One entry per day of the range (days without activity included), cut in
/// the range's time zone.
pub async fn daily(core: &Core, scope: Scope, range: &Range) -> AppResult<Vec<DailyShortAnalytics>> {
    let sql = format!(
        "SELECT toString(toDate(at, ?)) AS day, {AGGREGATES} FROM events
         WHERE owner_id = toUUID(?) AND subject_type = 'short' AND name IN {NAMES}
           AND at >= fromUnixTimestamp64Milli(?) AND at < fromUnixTimestamp64Milli(?) AND {}
         GROUP BY day",
        scope.predicate()
    );
    let rows: Vec<DayCounts> = core
        .analytics
        .ch
        .query(&sql)
        .bind(range.time_zone.name())
        .bind(scope.owner_id().to_string())
        .bind(range.from_ms)
        .bind(range.until_ms)
        .bind(scope.target().to_string())
        .fetch_all()
        .await
        .map_err(unavailable)?;
    Ok(fill_days(range.days(), &rows))
}

/// The daily series of a range: one entry per day, zeros where nothing happened.
fn fill_days(days: impl Iterator<Item = NaiveDate>, rows: &[DayCounts]) -> Vec<DailyShortAnalytics> {
    days.map(|day| {
        let date = day.to_string();
        match rows.iter().find(|row| row.day == date) {
            Some(row) => DailyShortAnalytics {
                date,
                views: row.views as i64,
                unique_viewers: row.viewers as i64,
                average_watch_ratio: for_an_audience(row.audience, row.ratio),
                loops: for_an_audience(row.audience, row.loops),
                likes: row.likes,
                comments: row.comments,
                shares: row.shares as i64,
                saves: row.saves,
                followers_gained: row.follows as i64,
                watch_time_seconds: for_an_audience(row.audience, row.watch_seconds),
                plays: for_an_audience(row.audience, row.plays as i64),
                ..Default::default()
            },
            None => DailyShortAnalytics { date, ..Default::default() },
        }
    })
    .collect()
}

/// Views and watch-through by where viewers were, most views first. Only
/// sources with at least one view are listed.
pub async fn traffic_sources(core: &Core, scope: Scope, range: &Range) -> AppResult<Vec<PlaybackSourceStats>> {
    let sql = format!(
        "SELECT toInt32(JSONExtractInt(props, 'source')) AS source,
                toUInt64(countIf(name = 'short.view')) AS views,
                toFloat64(ifNotFinite(avgIf(JSONExtractFloat(props, 'ratio'), name = 'short.watch'), 0)) AS ratio,
                toUInt64(uniqExactIf(JSONExtractString(props, 'viewer'), name = 'short.watch')) AS viewers
         FROM events
         WHERE owner_id = toUUID(?) AND subject_type = 'short' AND name IN ('short.view', 'short.watch')
           AND at >= fromUnixTimestamp64Milli(?) AND at < fromUnixTimestamp64Milli(?) AND {}
         GROUP BY source
         HAVING views > 0
         ORDER BY views DESC, source",
        scope.predicate()
    );
    let rows: Vec<(i32, u64, f64, u64)> = core
        .analytics
        .ch
        .query(&sql)
        .bind(scope.owner_id().to_string())
        .bind(range.from_ms)
        .bind(range.until_ms)
        .bind(scope.target().to_string())
        .fetch_all()
        .await
        .map_err(unavailable)?;
    Ok(rows
        .into_iter()
        .map(|(source, views, ratio, viewers)| PlaybackSourceStats {
            source: source.into(),
            views: views as i64,
            average_watch_ratio: for_an_audience(viewers, ratio),
            ..Default::default()
        })
        .collect())
}

/// The most viewed shorts of a channel over the range:
/// `(short, views, average watch ratio)`.
pub async fn top_shorts(
    core: &Core,
    channel: &ChannelRow,
    range: &Range,
    limit: u32,
) -> AppResult<Vec<(Uuid, i64, f64)>> {
    let rows: Vec<(String, u64, f64, u64)> = core
        .analytics
        .ch
        .query(
            "SELECT toString(subject_id) AS short,
                    toUInt64(countIf(name = 'short.view')) AS views,
                    toFloat64(ifNotFinite(avgIf(JSONExtractFloat(props, 'ratio'), name = 'short.watch'), 0)) AS ratio,
                    toUInt64(uniqExactIf(JSONExtractString(props, 'viewer'), name = 'short.watch')) AS viewers
             FROM events
             WHERE owner_id = toUUID(?) AND subject_type = 'short' AND name IN ('short.view', 'short.watch')
               AND at >= fromUnixTimestamp64Milli(?) AND at < fromUnixTimestamp64Milli(?)
               AND JSONExtractString(props, 'channel_id') = ?
             GROUP BY subject_id
             HAVING views > 0
             ORDER BY views DESC, ratio DESC, short
             LIMIT ?",
        )
        .bind(channel.owner_id.to_string())
        .bind(range.from_ms)
        .bind(range.until_ms)
        .bind(channel.id.to_string())
        .bind(limit)
        .fetch_all()
        .await
        .map_err(unavailable)?;
    Ok(rows
        .into_iter()
        .filter_map(|(short, views, ratio, viewers)| {
            Some((Uuid::parse_str(&short).ok()?, views as i64, for_an_audience(viewers, ratio)))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: &str, end: &str, zone: &str) -> Option<DateRange> {
        Some(DateRange { start_date: start.into(), end_date: end.into(), time_zone: zone.into(), ..Default::default() })
    }

    #[test]
    fn ranges_are_validated_like_the_video_domain_does() {
        let parsed = parse_range(range("2026-11-01", "2026-11-03", "Asia/Tokyo").as_ref()).unwrap();
        assert_eq!(parsed.days().count(), 3);
        // Midnight in Tokyo is 15:00 UTC the day before.
        assert_eq!(parsed.from_ms, 1_793_458_800_000);
        assert_eq!(parse_range(None).unwrap_err().reason, "DATE_RANGE_INVALID");
        assert_eq!(
            parse_range(range("2026-11-03", "2026-11-01", "").as_ref()).unwrap_err().reason,
            "DATE_RANGE_INVALID"
        );
        assert_eq!(
            parse_range(range("2026-11-01", "2026-11-02", "Mars/Base").as_ref()).unwrap_err().reason,
            "TIME_ZONE_INVALID"
        );
    }

    #[test]
    fn days_without_activity_are_zeros() {
        let parsed = parse_range(range("2026-11-01", "2026-11-03", "").as_ref()).unwrap();
        let rows = vec![DayCounts {
            day: "2026-11-02".into(),
            views: 4,
            viewers: 3,
            ratio: 0.5,
            loops: 2,
            likes: 1,
            comments: 0,
            shares: 1,
            saves: -1,
            follows: 1,
            watch_seconds: 30.0,
            plays: 5,
            audience: 3,
        }];
        let series = fill_days(parsed.days(), &rows);
        assert_eq!(
            series.iter().map(|day| day.date.as_str()).collect::<Vec<_>>(),
            ["2026-11-01", "2026-11-02", "2026-11-03"]
        );
        assert_eq!((series[0].views, series[1].views, series[1].saves, series[2].views), (0, 4, -1, 0));
        // Three viewers are too few to say how they watched.
        assert_eq!((series[1].average_watch_ratio, series[1].loops, series[1].plays), (0.0, 0, 0));
        assert_eq!((for_an_audience(4, 0.7), for_an_audience(5, 0.7), for_an_audience(5, 9i64)), (0.0, 0.7, 9));
    }
}
