//! Raw location history in ClickHouse (`geo_points`) and the hourly movement
//! totals in PostgreSQL (`geo_activity_hours`).
//!
//! The pipeline does not insert rows one by one: accepted samples wait in an
//! in-process buffer that is written in batches — by the flusher in worker
//! processes, at the end of every `ReportLocations` call, and inline whenever
//! the buffer has grown or aged enough. ClickHouse being down never fails
//! location sharing: rows stay buffered (up to a limit) and are retried.
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokuly_core::prelude::*;

use crate::{
    Geo,
    odometer::HourBucket,
    state::{Area, RegionRef},
};

/// Schema of the ClickHouse table this module uses, for the server's setup
/// step (`tokuly_core::analytics::apply_schema`).
pub const CLICKHOUSE_SCHEMA: &[(&str, &str)] = &[("0300_geo", include_str!("../../../clickhouse/0300_geo.sql"))];

const FLAG_MOCKED: u8 = 1;
const FLAG_FOREGROUND: u8 = 2;
const FLAG_CHARGING: u8 = 4;

/// Rows are written once this many wait...
const FLUSH_ROWS: usize = 2000;
/// ...or once the oldest has waited this long.
const FLUSH_AFTER: Duration = Duration::from_secs(2);
/// With ClickHouse unreachable, the buffer stops growing here (oldest rows go first).
const MAX_BUFFERED: usize = 200_000;

/// One `geo_points` row.
#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct PointRow {
    #[serde(with = "clickhouse::serde::uuid")]
    pub user_id: Uuid,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    pub recorded_at: DateTime<Utc>,
    pub lat: f64,
    pub lng: f64,
    pub accuracy: f32,
    /// NaN when unknown.
    pub altitude: f32,
    pub speed: f32,
    /// -1 when unknown.
    pub heading: f32,
    pub activity: u8,
    /// -1 when unknown.
    pub battery: i8,
    pub flags: u8,
}

impl PointRow {
    pub fn flags(mocked: bool, foreground: bool, charging: bool) -> u8 {
        (if mocked { FLAG_MOCKED } else { 0 }) | (if foreground { FLAG_FOREGROUND } else { 0 }) | (if charging { FLAG_CHARGING } else { 0 })
    }

    pub fn mocked(&self) -> bool {
        self.flags & FLAG_MOCKED != 0
    }
}

/// Samples waiting to be written.
#[derive(Default)]
pub(crate) struct Buffer {
    inner: Mutex<Pending>,
}

#[derive(Default)]
struct Pending {
    rows: Vec<PointRow>,
    oldest: Option<Instant>,
}

impl Buffer {
    /// Queues a row. Returns whether the buffer should be written now.
    fn push(&self, row: PointRow) -> bool {
        let mut pending = self.inner.lock();
        pending.rows.push(row);
        let oldest = *pending.oldest.get_or_insert_with(Instant::now);
        pending.rows.len() >= FLUSH_ROWS || oldest.elapsed() >= FLUSH_AFTER
    }

    fn take(&self) -> Vec<PointRow> {
        let mut pending = self.inner.lock();
        pending.oldest = None;
        std::mem::take(&mut pending.rows)
    }

    /// Puts rows back after a failed write, dropping the oldest beyond the limit.
    fn restore(&self, mut rows: Vec<PointRow>) {
        let mut pending = self.inner.lock();
        rows.append(&mut pending.rows);
        if rows.len() > MAX_BUFFERED {
            let excess = rows.len() - MAX_BUFFERED;
            rows.drain(..excess);
        }
        pending.rows = rows;
        pending.oldest.get_or_insert_with(Instant::now);
    }
}

fn unavailable(context: &str, error: impl std::fmt::Display) -> AppError {
    AppError::unavailable("HISTORY_UNAVAILABLE", format!("{context}: {error}"))
}

impl Geo {
    /// Queues a point for the raw history. Never fails the caller.
    pub(crate) async fn record_point(&self, row: PointRow) {
        if self.history.push(row) {
            if let Err(e) = self.flush_history().await {
                tracing::warn!(error = %e, "location history not written; will retry");
            }
        }
    }

    /// Writes every buffered point to ClickHouse. Called by the flusher, at
    /// the end of `ReportLocations`, and by tests before reading history.
    pub async fn flush_history(&self) -> AppResult<()> {
        let rows = self.history.take();
        if rows.is_empty() {
            return Ok(());
        }
        let written: Result<(), clickhouse::error::Error> = async {
            let mut insert = self.core.analytics.ch.insert::<PointRow>("geo_points").await?;
            for row in &rows {
                insert.write(row).await?;
            }
            insert.end().await
        }
        .await;
        written.map_err(|e| {
            self.history.restore(rows);
            unavailable("history insert", e)
        })
    }

    /// A user's points in `[from, to)`, oldest first, thinned out evenly to at
    /// most `max_points`.
    pub(crate) async fn points(&self, user_id: Uuid, from: DateTime<Utc>, to: DateTime<Utc>, max_points: usize) -> AppResult<Vec<PointRow>> {
        let ch = &self.core.analytics.ch;
        let (user, from, to) = (user_id.to_string(), from.timestamp_millis(), to.timestamp_millis());
        const RANGE: &str = "user_id = toUUID(?) AND recorded_at >= fromUnixTimestamp64Milli(?) AND recorded_at < fromUnixTimestamp64Milli(?)";
        let total: u64 = ch
            .query(&format!("SELECT count() FROM geo_points WHERE {RANGE}"))
            .bind(&user)
            .bind(from)
            .bind(to)
            .fetch_one()
            .await
            .map_err(|e| unavailable("history count", e))?;
        let step = (total as usize).div_ceil(max_points.max(1)).max(1);
        // A re-sent sample exists twice until ClickHouse merges the parts: one row per instant.
        let mut rows: Vec<PointRow> = ch
            .query(&format!(
                "SELECT ?fields FROM (
                     SELECT *, row_number() OVER (ORDER BY recorded_at) AS n FROM (
                         SELECT * FROM geo_points WHERE {RANGE} ORDER BY recorded_at LIMIT 1 BY recorded_at
                     )
                 ) WHERE (n - 1) % ? = 0 ORDER BY recorded_at"
            ))
            .bind(&user)
            .bind(from)
            .bind(to)
            .bind(step as u64)
            .fetch_all()
            .await
            .map_err(|e| unavailable("history query", e))?;
        rows.truncate(max_points);
        Ok(rows)
    }

    /// How many points a user has in `[from, to)`.
    pub(crate) async fn point_count(&self, user_id: Uuid, from: DateTime<Utc>, to: DateTime<Utc>) -> AppResult<u64> {
        self.core
            .analytics
            .ch
            .query(
                "SELECT count() FROM geo_points FINAL
                 WHERE user_id = toUUID(?) AND recorded_at >= fromUnixTimestamp64Milli(?) AND recorded_at < fromUnixTimestamp64Milli(?)",
            )
            .bind(user_id.to_string())
            .bind(from.timestamp_millis())
            .bind(to.timestamp_millis())
            .fetch_one()
            .await
            .map_err(|e| unavailable("history count", e))
    }

    /// Deletes a user's points in `[from, to)` (also those still buffered here).
    pub(crate) async fn delete_points(&self, user_id: Uuid, from: DateTime<Utc>, to: DateTime<Utc>) -> AppResult<()> {
        {
            let mut pending = self.history.inner.lock();
            pending.rows.retain(|row| !(row.user_id == user_id && row.recorded_at >= from && row.recorded_at < to));
        }
        self.core
            .analytics
            .ch
            .query(
                "DELETE FROM geo_points
                 WHERE user_id = toUUID(?) AND recorded_at >= fromUnixTimestamp64Milli(?) AND recorded_at < fromUnixTimestamp64Milli(?)",
            )
            .bind(user_id.to_string())
            .bind(from.timestamp_millis())
            .bind(to.timestamp_millis())
            .execute()
            .await
            .map_err(|e| unavailable("history delete", e))
    }

    /// Deletes the points of several users recorded before `before` (retention).
    pub(crate) async fn delete_points_before(&self, user_ids: &[Uuid], before: DateTime<Utc>) -> AppResult<()> {
        if user_ids.is_empty() {
            return Ok(());
        }
        let users: Vec<String> = user_ids.iter().map(Uuid::to_string).collect();
        self.core
            .analytics
            .ch
            .query("DELETE FROM geo_points WHERE user_id IN (SELECT toUUID(arrayJoin(?))) AND recorded_at < fromUnixTimestamp64Milli(?)")
            .bind(users)
            .bind(before.timestamp_millis())
            .execute()
            .await
            .map_err(|e| unavailable("history retention", e))
    }

    /// Adds one hour's movement totals to `geo_activity_hours`.
    pub(crate) async fn store_hours(&self, user_id: Uuid, bucket: &HourBucket, area: &Area, region: &RegionRef) -> AppResult<()> {
        let Some(hour) = DateTime::from_timestamp_millis(bucket.hour_ms) else { return Ok(()) };
        let activities: Vec<i16> = bucket.totals.iter().map(|t| t.activity as i16).collect();
        let distances: Vec<f64> = bucket.totals.iter().map(|t| t.distance_m).collect();
        let durations: Vec<i32> = bucket.totals.iter().map(|t| t.duration_s.round() as i32).collect();
        let speeds: Vec<f32> = bucket.totals.iter().map(|t| t.max_speed_mps as f32).collect();
        let points: Vec<i32> = bucket.totals.iter().map(|t| t.points as i32).collect();
        sqlx::query(
            "INSERT INTO geo_activity_hours
                 (user_id, hour, activity, distance_m, duration_s, max_speed_mps, point_count, locality, locality_ja, country_code, admin1_id)
             SELECT $1, $2, t.activity, t.distance_m, t.duration_s, t.max_speed, t.points, $8, $9, $10, $11
             FROM UNNEST($3::smallint[], $4::float8[], $5::int[], $6::real[], $7::int[]) AS t(activity, distance_m, duration_s, max_speed, points)
             ON CONFLICT (user_id, hour, activity) DO UPDATE SET
                 distance_m = geo_activity_hours.distance_m + EXCLUDED.distance_m,
                 duration_s = geo_activity_hours.duration_s + EXCLUDED.duration_s,
                 max_speed_mps = GREATEST(geo_activity_hours.max_speed_mps, EXCLUDED.max_speed_mps),
                 point_count = geo_activity_hours.point_count + EXCLUDED.point_count,
                 locality = EXCLUDED.locality, locality_ja = EXCLUDED.locality_ja,
                 country_code = EXCLUDED.country_code, admin1_id = EXCLUDED.admin1_id",
        )
        .bind(user_id)
        .bind(hour)
        .bind(activities)
        .bind(distances)
        .bind(durations)
        .bind(speeds)
        .bind(points)
        .bind(&area.locality)
        .bind(&area.locality_ja)
        .bind(&area.country_code)
        .bind(&region.admin1_id)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    /// Stores whatever distance the user's live state still holds, so that
    /// statistics read right afterwards are complete.
    pub(crate) async fn settle_odometer(&self, user_id: Uuid) -> AppResult<()> {
        // Cheap check without the lock first: most reads find nothing pending.
        match self.load_state(user_id).await? {
            Some(state) if !state.odometer.pending.is_empty() => {}
            _ => return Ok(()),
        }
        self.with_state(user_id, async |state| {
            if let Some(bucket) = state.odometer.take() {
                self.store_hours(user_id, &bucket, &state.pin.area, &state.region).await?;
            }
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(seconds: i64) -> PointRow {
        PointRow {
            user_id: Uuid::nil(),
            recorded_at: DateTime::from_timestamp(seconds, 0).unwrap(),
            lat: 35.0,
            lng: 139.0,
            accuracy: 10.0,
            altitude: f32::NAN,
            speed: 0.0,
            heading: -1.0,
            activity: 1,
            battery: -1,
            flags: 0,
        }
    }

    #[test]
    fn flags_pack_into_one_byte() {
        assert_eq!(PointRow::flags(false, false, false), 0);
        assert_eq!(PointRow::flags(true, true, true), 7);
        let mocked = PointRow { flags: PointRow::flags(true, false, true), ..row(0) };
        assert!(mocked.mocked());
        assert!(!row(0).mocked());
    }

    #[test]
    fn the_buffer_asks_for_a_flush_when_full() {
        let buffer = Buffer::default();
        for i in 0..FLUSH_ROWS as i64 - 1 {
            assert!(!buffer.push(row(i)), "row {i}");
        }
        assert!(buffer.push(row(-1)));
        assert_eq!(buffer.take().len(), FLUSH_ROWS);
        assert!(buffer.take().is_empty());
        assert!(!buffer.push(row(0)), "a fresh buffer starts over");
    }

    #[test]
    fn restored_rows_keep_their_order_and_the_limit() {
        let buffer = Buffer::default();
        buffer.push(row(3));
        buffer.restore(vec![row(1), row(2)]);
        let rows = buffer.take();
        assert_eq!(rows.iter().map(|r| r.recorded_at.timestamp()).collect::<Vec<_>>(), vec![1, 2, 3]);

        buffer.restore((0..MAX_BUFFERED as i64 + 10).map(row).collect());
        let rows = buffer.take();
        assert_eq!(rows.len(), MAX_BUFFERED);
        assert_eq!(rows[0].recorded_at.timestamp(), 10, "the oldest rows are dropped first");
    }
}
