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
//! # How people watched, and why it is reported in blocks
//!
//! Counts are reported as they are. *How* people watched (the
//! watch-through ratio, loops, watch time, plays) is an average over
//! viewers, and an average over one viewer is that viewer: the creator of
//! a friends-only short with a single view would learn how long that
//! friend lasted. A minimum audience per report is not enough, because
//! two reports can be subtracted: a range with and without one day, a
//! channel with and without one short, a number read before and after a
//! friend watched.
//!
//! So these numbers are only ever sums of fixed **blocks**: one short on
//! one calendar day in the time zone of the channel owner's account (the
//! caller cannot move the day boundary), counted once the day is over, and
//! only if at least [`MIN_AUDIENCE`] accounts watched that short that day.
//! Every report (a short, a channel, any range) adds up the qualifying
//! blocks inside it, so the difference of two reports is again a sum of
//! qualifying blocks, never one person. The results are rounded. There are
//! no live figures of this kind anywhere in the API.
use chrono::{NaiveDate, TimeZone as _};
use tokuly_core::prelude::*;
use tokuly_proto::shorts::{DailyShortAnalytics, DateRange, PlaybackSourceStats, ShortAnalyticsTotals};
pub use tokuly_video::analytics::Range;
use tokuly_video::channels::ChannelRow;

use crate::shorts::ShortRow;

/// Accounts that must have watched a short on a day for that day's
/// watching of the short to be reported at all.
pub const MIN_AUDIENCE: i64 = 5;

fn unavailable(error: clickhouse::error::Error) -> AppError {
    AppError::unavailable("ANALYTICS_UNAVAILABLE", format!("analytics query failed: {error}"))
}

/// Validates a date range (`DATE_RANGE_INVALID`) and cuts its days in the
/// time zone of the channel owner's account.
pub async fn owner_range(core: &Core, owner_id: Uuid, range: Option<&DateRange>) -> AppResult<Range> {
    let time_zone: Option<String> =
        sqlx::query_scalar("SELECT time_zone FROM users WHERE id = $1").bind(owner_id).fetch_optional(&core.db).await?;
    parse_range(range, &time_zone.unwrap_or_default())
}

fn parse_range(range: Option<&DateRange>, time_zone: &str) -> AppResult<Range> {
    // An account whose stored zone is unknown to this build counts its days in UTC.
    let time_zone = if time_zone.parse::<chrono_tz::Tz>().is_ok() { time_zone } else { "" };
    let range = range.map(|range| tokuly_proto::video::DateRange {
        start_date: range.start_date.clone(),
        end_date: range.end_date.clone(),
        time_zone: time_zone.to_owned(),
        ..Default::default()
    });
    Range::parse(range.as_ref())
}

/// The first instant of the running day in the range's time zone (Unix
/// milliseconds): watching is reported up to here.
fn start_of_today_ms(range: &Range) -> i64 {
    let today = now().with_timezone(&range.time_zone).date_naive();
    let local = today.and_hms_opt(0, 0, 0).unwrap_or_default();
    range
        .time_zone
        .from_local_datetime(&local)
        .earliest()
        .map(|instant| instant.timestamp_millis())
        .unwrap_or_else(|| now().timestamp_millis())
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

const NAMES: &str = "('short.view', 'short.like', 'short.comment', 'short.save', 'short.share', 'short.follow')";

const COUNTS: &str = "toUInt64(countIf(name = 'short.view')) AS views, \
     toUInt64(uniqExactIf(JSONExtractString(props, 'viewer'), name = 'short.view')) AS viewers, \
     toInt64(sumIf(toInt64(value), name = 'short.like')) AS likes, \
     toInt64(sumIf(toInt64(value), name = 'short.comment')) AS comments, \
     toUInt64(countIf(name = 'short.share')) AS shares, \
     toInt64(sumIf(toInt64(value), name = 'short.save')) AS saves, \
     toUInt64(countIf(name = 'short.follow')) AS follows";

/// `(day, views, viewers, likes, comments, shares, saves, follows)`.
type DayCounts = (String, u64, u64, i64, i64, u64, i64, u64);

/// How one day's qualifying blocks were watched.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Watching {
    ratio_sum: f64,
    plays: u64,
    loops: i64,
    seconds: f64,
}

impl Watching {
    fn add(&mut self, other: &Watching) {
        self.ratio_sum += other.ratio_sum;
        self.plays += other.plays;
        self.loops += other.loops;
        self.seconds += other.seconds;
    }

    /// The average watch-through ratio, to two decimals.
    fn ratio(&self) -> f64 {
        if self.plays == 0 { 0.0 } else { (self.ratio_sum / self.plays as f64 * 100.0).round() / 100.0 }
    }
}

/// The counts per day of the range, up to now.
async fn counts(core: &Core, scope: Scope, range: &Range) -> AppResult<Vec<DayCounts>> {
    let sql = format!(
        "SELECT toString(toDate(at, ?)) AS day, {COUNTS} FROM events
         WHERE owner_id = toUUID(?) AND subject_type = 'short' AND name IN {NAMES}
           AND at >= fromUnixTimestamp64Milli(?) AND at < fromUnixTimestamp64Milli(?) AND {}
         GROUP BY day",
        scope.predicate()
    );
    core.analytics
        .ch
        .query(&sql)
        .bind(range.time_zone.name())
        .bind(scope.owner_id().to_string())
        .bind(range.from_ms)
        .bind(range.until_ms)
        .bind(scope.target().to_string())
        .fetch_all()
        .await
        .map_err(unavailable)
}

/// How the scope's shorts were watched, per finished day of the range:
/// the sum of the blocks (one short, one day) that at least
/// [`MIN_AUDIENCE`] accounts watched.
async fn watching(core: &Core, scope: Scope, range: &Range) -> AppResult<Vec<(String, Watching)>> {
    let until_ms = range.until_ms.min(start_of_today_ms(range));
    if until_ms <= range.from_ms {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT day, toFloat64(sum(ratio_sum)), toUInt64(sum(plays)), toInt64(sum(loops)), toFloat64(sum(seconds))
         FROM (
             SELECT toString(toDate(at, ?)) AS day, subject_id,
                    sum(JSONExtractFloat(props, 'ratio')) AS ratio_sum, count() AS plays,
                    sum(JSONExtractInt(props, 'loops')) AS loops, sum(value) AS seconds
             FROM events
             WHERE owner_id = toUUID(?) AND subject_type = 'short' AND name = 'short.watch'
               AND user_id != toUUID('00000000-0000-0000-0000-000000000000')
               AND at >= fromUnixTimestamp64Milli(?) AND at < fromUnixTimestamp64Milli(?) AND {}
             GROUP BY day, subject_id
             HAVING uniqExact(user_id) >= ?
         )
         GROUP BY day",
        scope.predicate()
    );
    let rows: Vec<(String, f64, u64, i64, f64)> = core
        .analytics
        .ch
        .query(&sql)
        .bind(range.time_zone.name())
        .bind(scope.owner_id().to_string())
        .bind(range.from_ms)
        .bind(until_ms)
        .bind(scope.target().to_string())
        .bind(MIN_AUDIENCE)
        .fetch_all()
        .await
        .map_err(unavailable)?;
    Ok(rows
        .into_iter()
        .map(|(day, ratio_sum, plays, loops, seconds)| (day, Watching { ratio_sum, plays, loops, seconds }))
        .collect())
}

/// Totals over the range and one entry per day (days without activity
/// included).
pub async fn report(
    core: &Core,
    scope: Scope,
    range: &Range,
) -> AppResult<(ShortAnalyticsTotals, Vec<DailyShortAnalytics>)> {
    let counted = counts(core, scope, range).await?;
    let watched = watching(core, scope, range).await?;
    // Distinct viewers do not add up over days: ask for the range as a whole.
    let sql = format!(
        "SELECT toUInt64(uniqExact(JSONExtractString(props, 'viewer'))) FROM events
         WHERE owner_id = toUUID(?) AND subject_type = 'short' AND name = 'short.view'
           AND at >= fromUnixTimestamp64Milli(?) AND at < fromUnixTimestamp64Milli(?) AND {}",
        scope.predicate()
    );
    let unique_viewers: u64 = core
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
    Ok(assemble(range.days(), &counted, &watched, unique_viewers as i64))
}

/// Puts the per-day rows together into the totals and the daily series.
fn assemble(
    days: impl Iterator<Item = NaiveDate>,
    counted: &[DayCounts],
    watched: &[(String, Watching)],
    unique_viewers: i64,
) -> (ShortAnalyticsTotals, Vec<DailyShortAnalytics>) {
    let mut totals = ShortAnalyticsTotals { unique_viewers, ..Default::default() };
    let mut all = Watching::default();
    let daily: Vec<DailyShortAnalytics> = days
        .map(|day| {
            let date = day.to_string();
            let mut entry = DailyShortAnalytics { date: date.clone(), ..Default::default() };
            if let Some((_, views, viewers, likes, comments, shares, saves, follows)) =
                counted.iter().find(|row| row.0 == date)
            {
                entry.views = *views as i64;
                entry.unique_viewers = *viewers as i64;
                entry.likes = *likes;
                entry.comments = *comments;
                entry.shares = *shares as i64;
                entry.saves = *saves;
                entry.followers_gained = *follows as i64;
                totals.views += entry.views;
                totals.likes += entry.likes;
                totals.comments += entry.comments;
                totals.shares += entry.shares;
                totals.saves += entry.saves;
                totals.followers_gained += entry.followers_gained;
            }
            if let Some((_, watching)) = watched.iter().find(|row| row.0 == date) {
                entry.average_watch_ratio = watching.ratio();
                entry.loops = watching.loops;
                entry.watch_time_seconds = watching.seconds.round();
                entry.plays = watching.plays as i64;
                all.add(watching);
            }
            entry
        })
        .collect();
    totals.average_watch_ratio = all.ratio();
    totals.loops = all.loops;
    totals.watch_time_seconds = all.seconds.round();
    totals.plays = all.plays as i64;
    (totals, daily)
}

/// Views by where viewers were, most views first. Only sources with at
/// least one view are listed.
pub async fn traffic_sources(core: &Core, scope: Scope, range: &Range) -> AppResult<Vec<PlaybackSourceStats>> {
    let sql = format!(
        "SELECT toInt32(JSONExtractInt(props, 'source')) AS source, toUInt64(count()) AS views
         FROM events
         WHERE owner_id = toUUID(?) AND subject_type = 'short' AND name = 'short.view'
           AND at >= fromUnixTimestamp64Milli(?) AND at < fromUnixTimestamp64Milli(?) AND {}
         GROUP BY source
         ORDER BY views DESC, source",
        scope.predicate()
    );
    let rows: Vec<(i32, u64)> = core
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
        .map(|(source, views)| PlaybackSourceStats { source: source.into(), views: views as i64, ..Default::default() })
        .collect())
}

/// The most viewed shorts of a channel over the range: `(short, views)`.
pub async fn top_shorts(core: &Core, channel: &ChannelRow, range: &Range, limit: u32) -> AppResult<Vec<(Uuid, i64)>> {
    let rows: Vec<(String, u64)> = core
        .analytics
        .ch
        .query(
            "SELECT toString(subject_id) AS short, toUInt64(count()) AS views
             FROM events
             WHERE owner_id = toUUID(?) AND subject_type = 'short' AND name = 'short.view'
               AND at >= fromUnixTimestamp64Milli(?) AND at < fromUnixTimestamp64Milli(?)
               AND JSONExtractString(props, 'channel_id') = ?
             GROUP BY subject_id
             ORDER BY views DESC, short
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
    Ok(rows.into_iter().filter_map(|(short, views)| Some((Uuid::parse_str(&short).ok()?, views as i64))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: &str, end: &str) -> Option<DateRange> {
        Some(DateRange { start_date: start.into(), end_date: end.into(), ..Default::default() })
    }

    #[test]
    fn days_are_cut_in_the_owners_time_zone() {
        let parsed = parse_range(range("2026-11-01", "2026-11-03").as_ref(), "Asia/Tokyo").unwrap();
        assert_eq!(parsed.days().count(), 3);
        // Midnight in Tokyo is 15:00 UTC the day before.
        assert_eq!(parsed.from_ms, 1_793_458_800_000);
        assert_eq!(
            parse_range(range("2026-11-01", "2026-11-03").as_ref(), "Mars/Base").unwrap().from_ms,
            1_793_491_200_000
        );
        assert_eq!(parse_range(None, "Asia/Tokyo").unwrap_err().reason, "DATE_RANGE_INVALID");
        assert_eq!(
            parse_range(range("2026-11-03", "2026-11-01").as_ref(), "").unwrap_err().reason,
            "DATE_RANGE_INVALID"
        );
    }

    #[test]
    fn reports_are_sums_of_days() {
        let parsed = parse_range(range("2026-11-01", "2026-11-03").as_ref(), "").unwrap();
        let counted: Vec<DayCounts> =
            vec![("2026-11-02".into(), 4, 3, 1, 0, 1, -1, 1), ("2026-11-03".into(), 2, 2, 0, 1, 0, 0, 0)];
        let watched = vec![
            ("2026-11-01".to_owned(), Watching { ratio_sum: 3.0, plays: 6, loops: 2, seconds: 40.4 }),
            ("2026-11-02".to_owned(), Watching { ratio_sum: 4.0, plays: 5, loops: 1, seconds: 30.3 }),
        ];
        let (totals, daily) = assemble(parsed.days(), &counted, &watched, 4);
        assert_eq!(
            daily.iter().map(|day| day.date.as_str()).collect::<Vec<_>>(),
            ["2026-11-01", "2026-11-02", "2026-11-03"]
        );
        assert_eq!((totals.views, totals.unique_viewers, totals.saves, totals.comments), (6, 4, -1, 1));
        assert_eq!((daily[0].views, daily[1].views, daily[2].views), (0, 4, 2));
        // Watching: rounded, and the total is the sum of the days it is made of.
        assert_eq!(
            (daily[0].average_watch_ratio, daily[1].average_watch_ratio, daily[2].average_watch_ratio),
            (0.5, 0.8, 0.0)
        );
        assert_eq!((totals.plays, totals.loops, totals.watch_time_seconds), (11, 3, 71.0));
        assert_eq!(totals.average_watch_ratio, 0.64);
        assert_eq!((daily[0].watch_time_seconds, daily[2].plays), (40.0, 0));
    }
}
