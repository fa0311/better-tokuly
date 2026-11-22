//! `HistoryService`: the caller's own location log — visits, the calendar,
//! day timelines, route replay, day memos, nights, statistics and deletion.
//!
//! Days are cut in the time zone the caller asks for (their account's by
//! default); the stored data is zone-free (instants and UTC hour buckets).
use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Datelike as _, Duration, NaiveDate, TimeZone as _, Utc};
use chrono_tz::Tz;
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_media::assets::{self, image_from_json};
use tokuly_proto::{common::Image, geo::*, media::AssetPurpose};
use tokuly_social::graph;

use crate::{
    Geo,
    history::PointRow,
    math::{self, Point},
    odometer::MAX_SPEED_MPS,
    places::{PLACE_COLUMNS, PlaceRow},
    visits::{VISIT_COLUMNS, VisitRow},
};

const MAX_MEMO_CHARS: usize = 2000;
const MAX_MEMO_PHOTOS: usize = 10;
/// Points read for a day's path before simplifying.
const DAY_POINTS: usize = 20_000;
/// The day path keeps vertices that deviate more than this from a straight line.
const PATH_TOLERANCE_M: f64 = 12.0;
/// Fixes worse than this do not belong on a drawn path.
const PATH_ACCURACY_M: f32 = 100.0;
const DEFAULT_ROUTE_POINTS: i32 = 2000;
const MAX_ROUTE_POINTS: i32 = 10_000;
const MAX_ROUTE_DAYS: i64 = 31;

fn memo_holder(user_id: Uuid, day: NaiveDate) -> String {
    format!("geo:day-memo:{user_id}:{day}")
}

fn parse_date(input: &str) -> AppResult<NaiveDate> {
    NaiveDate::parse_from_str(input, "%Y-%m-%d")
        .map_err(|_| AppError::invalid("INVALID_DATE", "dates look like 2026-11-20"))
}

/// The instant a local calendar day begins (the first valid instant when
/// midnight is skipped by a daylight-saving change).
fn day_start(zone: Tz, day: NaiveDate) -> DateTime<Utc> {
    let midnight = day.and_hms_opt(0, 0, 0).expect("midnight exists");
    (0..=3)
        .find_map(|hours| zone.from_local_datetime(&(midnight + Duration::hours(hours))).earliest())
        .map(|local| local.with_timezone(&Utc))
        .unwrap_or_else(|| midnight.and_utc())
}

/// An optional `[from, to)` range from a request; when both ends are given they must be in order.
fn optional_range(
    from: Option<&tokuly_proto::Timestamp>,
    to: Option<&tokuly_proto::Timestamp>,
) -> AppResult<(Option<DateTime<Utc>>, Option<DateTime<Utc>>)> {
    let (from, to) = (from.map(from_ts), to.map(from_ts));
    if let (Some(from), Some(to)) = (from, to) {
        ensure!(from < to, invalid, "INVALID_RANGE", "the range must end after it starts");
    }
    Ok((from, to))
}

#[derive(sqlx::FromRow)]
struct MemoRow {
    text: String,
    photos: serde_json::Value,
    updated_at: DateTime<Utc>,
}

impl MemoRow {
    fn images(&self) -> Vec<Image> {
        self.photos.as_array().into_iter().flatten().filter_map(|photo| image_from_json(Some(photo))).collect()
    }

    fn to_proto(&self) -> DayMemo {
        DayMemo {
            text: self.text.clone(),
            photos: self.images(),
            updated_at: ts(self.updated_at),
            ..Default::default()
        }
    }
}

#[derive(sqlx::FromRow)]
struct NightRow {
    night: NaiveDate,
    place_id: Option<Uuid>,
    place_name: String,
    lat: f64,
    lng: f64,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    with_user_ids: Vec<Uuid>,
    at_home: bool,
    locality: String,
    locality_ja: String,
}

impl Geo {
    /// The time zone days are cut in: the one asked for, else the account's.
    async fn day_zone(&self, user_id: Uuid, asked: &str) -> AppResult<Tz> {
        let name = match asked.trim() {
            "" => self.ctx_of(user_id).await?.time_zone,
            asked => asked.to_owned(),
        };
        if name.is_empty() {
            return Ok(chrono_tz::Asia::Tokyo);
        }
        name.parse().map_err(|_| AppError::invalid("INVALID_TIME_ZONE", "unknown time zone"))
    }

    /// Summaries of the days in `[first, end)` (local dates in `zone`) on
    /// which something was recorded, keyed by date.
    async fn day_summaries(
        &self,
        user_id: Uuid,
        zone: Tz,
        first: NaiveDate,
        end: NaiveDate,
        ja: bool,
    ) -> AppResult<BTreeMap<NaiveDate, DaySummary>> {
        let db = &self.core.db;
        let (from, to) = (day_start(zone, first), day_start(zone, end));
        let zone_name = zone.name();
        let mut days: BTreeMap<NaiveDate, DaySummary> = BTreeMap::new();
        /// The summary of one date, created on first use.
        fn day(days: &mut BTreeMap<NaiveDate, DaySummary>, date: NaiveDate) -> &mut DaySummary {
            days.entry(date).or_insert_with(|| DaySummary { date: date.to_string(), ..Default::default() })
        }

        let moved: Vec<(NaiveDate, f64, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT (hour AT TIME ZONE $4)::date AS day, sum(distance_m)::float8,
                    mode() WITHIN GROUP (ORDER BY locality) FILTER (WHERE locality <> ''),
                    mode() WITHIN GROUP (ORDER BY locality_ja) FILTER (WHERE locality_ja <> '')
             FROM geo_activity_hours WHERE user_id = $1 AND hour >= date_trunc('hour', $2::timestamptz) AND hour < $3
             GROUP BY 1",
        )
        .bind(user_id)
        .bind(from)
        .bind(to)
        .bind(zone_name)
        .fetch_all(db)
        .await?;
        for (date, distance, locality, locality_ja) in moved {
            if date < first || date >= end {
                continue;
            }
            let summary = day(&mut days, date);
            summary.distance_meters = distance;
            summary.top_locality = if ja { locality_ja.or(locality) } else { locality }.unwrap_or_default();
        }
        let visited: Vec<(NaiveDate, i64)> =
            sqlx::query_as("SELECT (arrived_at AT TIME ZONE $4)::date AS day, count(*) FROM geo_visits WHERE user_id = $1 AND arrived_at >= $2 AND arrived_at < $3 GROUP BY 1")
                .bind(user_id)
                .bind(from)
                .bind(to)
                .bind(zone_name)
                .fetch_all(db)
                .await?;
        for (date, count) in visited {
            day(&mut days, date).visit_count = count as i32;
        }
        let uncovered: Vec<(NaiveDate, i64)> = sqlx::query_as(
            "SELECT (first_at AT TIME ZONE $4)::date AS day, count(*) FROM geo_cells WHERE user_id = $1 AND res = 9 AND first_at >= $2 AND first_at < $3 GROUP BY 1",
        )
        .bind(user_id)
        .bind(from)
        .bind(to)
        .bind(zone_name)
        .fetch_all(db)
        .await?;
        for (date, count) in uncovered {
            day(&mut days, date).new_cell_count = count as i32;
        }
        // Memos belong to calendar dates as their author wrote them.
        let memos: Vec<(NaiveDate, serde_json::Value)> =
            sqlx::query_as("SELECT day, photos FROM geo_day_memos WHERE user_id = $1 AND day >= $2 AND day < $3")
                .bind(user_id)
                .bind(first)
                .bind(end)
                .fetch_all(db)
                .await?;
        for (date, photos) in memos {
            let summary = day(&mut days, date);
            summary.has_memo = true;
            summary.thumbnail = photos.get(0).and_then(|photo| image_from_json(Some(photo))).into();
        }
        Ok(days)
    }

    /// Night rows as messages, like [`Geo::visits_for`] does for visits.
    async fn nights_for(&self, user_id: Uuid, rows: &[NightRow], ja: bool) -> AppResult<Vec<Night>> {
        let place_ids: Vec<Uuid> =
            rows.iter().filter_map(|row| row.place_id).collect::<HashSet<_>>().into_iter().collect();
        let place_rows: Vec<PlaceRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {PLACE_COLUMNS} FROM geo_places WHERE id = ANY($1)")))
                .bind(&place_ids)
                .fetch_all(&self.core.db)
                .await?;
        let mut visible = Vec::new();
        for place in place_rows {
            if self.can_see_place(user_id, &place).await? {
                visible.push(place);
            }
        }
        let places: std::collections::HashMap<Uuid, Place> =
            visible.iter().map(|row| row.id).zip(self.places_for(user_id, &visible).await?).collect();
        let friends: HashSet<Uuid> = graph::friend_ids(&self.core, user_id).await?.into_iter().collect();
        let companions: Vec<Uuid> = rows
            .iter()
            .flat_map(|row| &row.with_user_ids)
            .filter(|id| friends.contains(id))
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let users = tokuly_identity::summaries(&self.core, &companions).await?;
        Ok(rows
            .iter()
            .map(|row| {
                let locality = if ja && !row.locality_ja.is_empty() { &row.locality_ja } else { &row.locality };
                Night {
                    date: row.night.to_string(),
                    place: row.place_id.and_then(|id| places.get(&id).cloned()).into(),
                    name: if row.place_name.is_empty() { locality.clone() } else { row.place_name.clone() },
                    center: math::lat_lng(row.lat, row.lng).into(),
                    from: ts(row.started_at),
                    to: ts(row.ended_at),
                    with_friends: row
                        .with_user_ids
                        .iter()
                        .filter_map(|id| users.get(id))
                        .filter(|user| !user.deleted)
                        .cloned()
                        .collect(),
                    at_home: row.at_home,
                    ..Default::default()
                }
            })
            .collect())
    }
}

impl HistoryService for Geo {
    async fn list_visits(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListVisitsRequest>,
    ) -> ServiceResult<ListVisitsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let (from, to) = optional_range(req.from.as_option(), req.to.as_option())?;
            let place_id = parse_opt_id(&req.place_id)?;
            let page = Page::from_request(req.page.as_option(), 100);
            let cursor: Option<(DateTime<Utc>, Uuid)> = page.decode()?;
            let mut rows: Vec<VisitRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {VISIT_COLUMNS} FROM geo_visits
                 WHERE user_id = $1
                   AND ($2::timestamptz IS NULL OR arrived_at >= $2) AND ($3::timestamptz IS NULL OR arrived_at < $3)
                   AND ($4::uuid IS NULL OR place_id = $4)
                   AND ($5::timestamptz IS NULL OR (arrived_at, id) < ($5, $6))
                 ORDER BY arrived_at DESC, id DESC LIMIT $7"
            )))
            .bind(user_id)
            .bind(from)
            .bind(to)
            .bind(place_id)
            .bind(cursor.map(|c| c.0))
            .bind(cursor.map(|c| c.1))
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let info = page.finish(&mut rows, |row| (row.arrived_at, row.id));
            let ja = self.speaks_japanese(user_id).await?;
            ok(ListVisitsResponse {
                visits: self.visits_for(user_id, &rows, ja).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_active_days(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListActiveDaysRequest>,
    ) -> ServiceResult<ListActiveDaysResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let first = NaiveDate::from_ymd_opt(request.year, request.month.max(0) as u32, 1)
                .filter(|date| (2000..=2100).contains(&date.year()))
                .ok_or_else(|| AppError::invalid("INVALID_DATE", "year and month (1-12) are required"))?;
            let next = if first.month() == 12 {
                NaiveDate::from_ymd_opt(first.year() + 1, 1, 1)
            } else {
                NaiveDate::from_ymd_opt(first.year(), first.month() + 1, 1)
            }
            .expect("the first of a month exists");
            let zone = self.day_zone(user_id, request.time_zone).await?;
            self.settle_odometer(user_id).await?;
            let ja = self.speaks_japanese(user_id).await?;
            let days = self.day_summaries(user_id, zone, first, next, ja).await?;
            ok(ListActiveDaysResponse { days: days.into_values().collect(), ..Default::default() })
        })
        .await
    }

    async fn get_day(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetDayRequest>,
    ) -> ServiceResult<GetDayResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let date = parse_date(request.date)?;
            let next = date.succ_opt().ok_or_else(|| AppError::invalid("INVALID_DATE", "that date is out of range"))?;
            let zone = self.day_zone(user_id, request.time_zone).await?;
            let (from, to) = (day_start(zone, date), day_start(zone, next));
            self.settle_odometer(user_id).await?;
            let ja = self.speaks_japanese(user_id).await?;
            let db = &self.core.db;

            let summary = self.day_summaries(user_id, zone, date, next, ja).await?.remove(&date).unwrap_or_else(|| DaySummary { date: date.to_string(), ..Default::default() });
            let raw: AppResult<Vec<PointRow>> = async {
                // Points still buffered here would be missing from today's path otherwise.
                self.flush_history().await?;
                self.points(user_id, from, to, DAY_POINTS).await
            }
            .await;
            // The raw history holds the path only: without it the day still has its visits, totals and memo.
            let points = raw.unwrap_or_else(|e| {
                tracing::warn!(%user_id, error = %e, "raw history unavailable; the day comes without its path");
                Vec::new()
            });
            let path: Vec<Point> = points.iter().filter(|point| point.accuracy <= PATH_ACCURACY_M).map(|point| (point.lat, point.lng)).collect();
            let max_speed_mps = points.iter().map(|point| point.speed as f64).filter(|speed| speed.is_finite() && *speed <= MAX_SPEED_MPS).fold(0.0, f64::max);
            let simplified = math::simplify(&path, PATH_TOLERANCE_M);

            let visits: Vec<VisitRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {VISIT_COLUMNS} FROM geo_visits
                 WHERE user_id = $1 AND arrived_at < $3 AND COALESCE(departed_at, now()) >= $2 ORDER BY arrived_at, id LIMIT 500"
            )))
            .bind(user_id)
            .bind(from)
            .bind(to)
            .fetch_all(db)
            .await?;
            // People met: named on a visit, or in the same gathering at the same time.
            let together: Vec<Uuid> = sqlx::query_scalar(
                "SELECT DISTINCT other.user_id
                 FROM geo_gathering_members mine
                 JOIN geo_gathering_members other ON other.gathering_id = mine.gathering_id AND other.user_id <> mine.user_id
                 WHERE mine.user_id = $1 AND mine.arrived_at < $3 AND COALESCE(mine.left_at, now()) >= $2
                   AND other.arrived_at < LEAST(COALESCE(mine.left_at, now()), $3) AND COALESCE(other.left_at, now()) > GREATEST(mine.arrived_at, $2)",
            )
            .bind(user_id)
            .bind(from)
            .bind(to)
            .fetch_all(db)
            .await?;
            let friends: HashSet<Uuid> = graph::friend_ids(&self.core, user_id).await?.into_iter().collect();
            let mut met: Vec<Uuid> =
                visits.iter().flat_map(|visit| visit.with_user_ids.iter().copied()).chain(together).filter(|id| friends.contains(id)).collect::<HashSet<_>>().into_iter().collect();
            met.sort();
            let friends_met = tokuly_identity::summaries_ordered(&self.core, &met).await?.into_iter().filter(|user| !user.deleted).collect();

            let memo: Option<MemoRow> =
                sqlx::query_as("SELECT text, photos, updated_at FROM geo_day_memos WHERE user_id = $1 AND day = $2").bind(user_id).bind(date).fetch_optional(db).await?;
            ok(GetDayResponse {
                summary: summary.into(),
                path_polyline: math::encode_polyline(&simplified),
                visits: self.visits_for(user_id, &visits, ja).await?,
                friends_met,
                memo: memo.map(|memo| memo.to_proto()).into(),
                bounds: math::bounding_box(path.iter().copied().chain(visits.iter().map(|visit| (visit.lat, visit.lng)))).into(),
                max_speed_mps,
                ..Default::default()
            })
        })
        .await
    }

    async fn get_route(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetRouteRequest>,
    ) -> ServiceResult<GetRouteResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let (Some(from), Some(to)) = (req.from.as_option().map(from_ts), req.to.as_option().map(from_ts)) else {
                bail!(invalid, "INVALID_RANGE", "both ends of the range are required");
            };
            ensure!(
                from < to && to - from <= Duration::days(MAX_ROUTE_DAYS),
                invalid,
                "INVALID_RANGE",
                "a route covers at most {MAX_ROUTE_DAYS} days"
            );
            let max_points =
                if req.max_points <= 0 { DEFAULT_ROUTE_POINTS } else { req.max_points.clamp(10, MAX_ROUTE_POINTS) };
            self.flush_history().await?;
            let points = self.points(user_id, from, to, max_points as usize).await?;
            ok(GetRouteResponse {
                points: points
                    .into_iter()
                    .map(|point| RoutePoint {
                        position: math::lat_lng(point.lat, point.lng).into(),
                        recorded_at: ts(point.recorded_at),
                        speed_mps: point.speed as f64,
                        activity: (point.activity as i32).into(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })
        })
        .await
    }

    async fn set_day_memo(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetDayMemoRequest>,
    ) -> ServiceResult<SetDayMemoResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let day = parse_date(&req.date)?;
            // "Today" somewhere on Earth can be a day ahead of UTC, never two.
            ensure!(day <= (Utc::now() + Duration::days(1)).date_naive() && day.year() >= 2000, invalid, "INVALID_DATE", "a memo is for a day that has begun");
            let text = req.text.trim();
            ensure!(text.chars().count() <= MAX_MEMO_CHARS, invalid, "TEXT_TOO_LONG", "a memo has at most {MAX_MEMO_CHARS} characters");
            ensure!(req.photo_asset_ids.len() <= MAX_MEMO_PHOTOS, invalid, "TOO_MANY_PHOTOS", "a memo has at most {MAX_MEMO_PHOTOS} photos");
            let mut asset_ids = Vec::new();
            let mut photos = Vec::new();
            for asset_id in parse_ids(&req.photo_asset_ids)? {
                if asset_ids.contains(&asset_id) {
                    continue;
                }
                photos.push(assets::require_image_json(&self.core, asset_id, user_id, &[AssetPurpose::ASSET_PURPOSE_DAY_MEMO]).await?);
                asset_ids.push(asset_id);
            }
            let holder = memo_holder(user_id, day);
            let mut tx = self.core.db.begin().await?;
            let before: Vec<Uuid> = sqlx::query_scalar("SELECT photo_asset_ids FROM geo_day_memos WHERE user_id = $1 AND day = $2 FOR UPDATE")
                .bind(user_id)
                .bind(day)
                .fetch_optional(&mut *tx)
                .await?
                .unwrap_or_default();
            if text.is_empty() && asset_ids.is_empty() {
                // An empty memo is no memo.
                sqlx::query("DELETE FROM geo_day_memos WHERE user_id = $1 AND day = $2").bind(user_id).bind(day).execute(&mut *tx).await?;
                assets::detach_holder(&mut *tx, &holder).await?;
                tx.commit().await?;
                return ok(SetDayMemoResponse::default());
            }
            let memo: MemoRow = sqlx::query_as(
                "INSERT INTO geo_day_memos (user_id, day, text, photos, photo_asset_ids) VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (user_id, day) DO UPDATE SET text = EXCLUDED.text, photos = EXCLUDED.photos, photo_asset_ids = EXCLUDED.photo_asset_ids
                 RETURNING text, photos, updated_at",
            )
            .bind(user_id)
            .bind(day)
            .bind(text)
            .bind(serde_json::Value::Array(photos))
            .bind(&asset_ids)
            .fetch_one(&mut *tx)
            .await?;
            for removed in before.iter().filter(|id| !asset_ids.contains(id)) {
                assets::detach(&mut *tx, *removed, &holder).await?;
            }
            for added in asset_ids.iter().filter(|id| !before.contains(id)) {
                assets::attach(&mut *tx, *added, &holder).await?;
            }
            tx.commit().await?;
            ok(SetDayMemoResponse { memo: memo.to_proto().into(), ..Default::default() })
        })
        .await
    }

    async fn list_nights(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListNightsRequest>,
    ) -> ServiceResult<ListNightsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let page = Page::from_view(request.page.as_option(), 100);
            let cursor: Option<NaiveDate> = page.decode()?;
            let mut rows: Vec<NightRow> = sqlx::query_as(
                "SELECT night, place_id, place_name, lat, lng, started_at, ended_at, with_user_ids, at_home, locality, locality_ja
                 FROM geo_nights WHERE user_id = $1 AND ($2::date IS NULL OR night < $2) ORDER BY night DESC LIMIT $3",
            )
            .bind(user_id)
            .bind(cursor)
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let info = page.finish(&mut rows, |row| row.night);
            let ja = self.speaks_japanese(user_id).await?;
            ok(ListNightsResponse { nights: self.nights_for(user_id, &rows, ja).await?, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn get_stats(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetStatsRequest>,
    ) -> ServiceResult<GetStatsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let (from, to) = optional_range(req.from.as_option(), req.to.as_option())?;
            let all_time = from.is_none() && to.is_none();
            self.settle_odometer(user_id).await?;
            let db = &self.core.db;

            let moved: Vec<(i16, f64, i64, f32)> = sqlx::query_as(
                "SELECT activity, sum(distance_m)::float8, sum(duration_s)::bigint, max(max_speed_mps)
                 FROM geo_activity_hours
                 WHERE user_id = $1 AND ($2::timestamptz IS NULL OR hour >= date_trunc('hour', $2)) AND ($3::timestamptz IS NULL OR hour < $3)
                 GROUP BY activity ORDER BY activity",
            )
            .bind(user_id)
            .bind(from)
            .bind(to)
            .fetch_all(db)
            .await?;
            let (visit_count, place_count): (i64, i64) = sqlx::query_as(
                "SELECT count(*), count(DISTINCT place_id) FROM geo_visits
                 WHERE user_id = $1 AND ($2::timestamptz IS NULL OR arrived_at >= $2) AND ($3::timestamptz IS NULL OR arrived_at < $3)",
            )
            .bind(user_id)
            .bind(from)
            .bind(to)
            .fetch_one(db)
            .await?;
            // Where: all-time from the footprint totals; for a range, from where the user moved in it.
            let (mut country_codes, region_count, cell_count): (Vec<String>, i64, i64) = if all_time {
                let index = self.region_index().await?;
                let regions: Vec<String> = sqlx::query_scalar("SELECT region_id FROM geo_region_stats WHERE user_id = $1").bind(user_id).fetch_all(db).await?;
                let countries: Vec<String> = regions.iter().filter(|id| index.get(id).is_some_and(|r| r.kind == crate::regions::KIND_COUNTRY)).cloned().collect();
                let cells: i64 = sqlx::query_scalar("SELECT cell_count FROM geo_users WHERE user_id = $1").bind(user_id).fetch_optional(db).await?.unwrap_or(0);
                let subdivisions = (regions.len() - countries.len()) as i64;
                (countries, subdivisions, cells)
            } else {
                let countries: Vec<String> = sqlx::query_scalar(
                    "SELECT DISTINCT country_code FROM geo_activity_hours
                     WHERE user_id = $1 AND country_code <> '' AND ($2::timestamptz IS NULL OR hour >= date_trunc('hour', $2)) AND ($3::timestamptz IS NULL OR hour < $3)",
                )
                .bind(user_id)
                .bind(from)
                .bind(to)
                .fetch_all(db)
                .await?;
                let regions: i64 = sqlx::query_scalar(
                    "SELECT count(DISTINCT admin1_id) FROM geo_activity_hours
                     WHERE user_id = $1 AND admin1_id <> '' AND ($2::timestamptz IS NULL OR hour >= date_trunc('hour', $2)) AND ($3::timestamptz IS NULL OR hour < $3)",
                )
                .bind(user_id)
                .bind(from)
                .bind(to)
                .fetch_one(db)
                .await?;
                let cells: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM geo_cells
                     WHERE user_id = $1 AND res = 9 AND ($2::timestamptz IS NULL OR first_at >= $2) AND ($3::timestamptz IS NULL OR first_at < $3)",
                )
                .bind(user_id)
                .bind(from)
                .bind(to)
                .fetch_one(db)
                .await?;
                (countries, regions, cells)
            };
            country_codes.sort();
            let bump_count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM geo_bumps
                 WHERE (user_a = $1 OR user_b = $1) AND ($2::timestamptz IS NULL OR created_at >= $2) AND ($3::timestamptz IS NULL OR created_at < $3)",
            )
            .bind(user_id)
            .bind(from)
            .bind(to)
            .fetch_one(db)
            .await?;
            let nights_away: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM geo_nights
                 WHERE user_id = $1 AND NOT at_home AND ($2::timestamptz IS NULL OR started_at >= $2) AND ($3::timestamptz IS NULL OR started_at < $3)",
            )
            .bind(user_id)
            .bind(from)
            .bind(to)
            .fetch_one(db)
            .await?;
            ok(GetStatsResponse {
                distance_meters: moved.iter().map(|row| row.1).sum(),
                visit_count: visit_count as i32,
                place_count: place_count as i32,
                country_count: country_codes.len() as i32,
                region_count: region_count as i32,
                cell_count,
                bump_count: bump_count as i32,
                nights_away: nights_away as i32,
                max_speed_mps: moved.iter().map(|row| row.3 as f64).fold(0.0, f64::max),
                by_activity: moved
                    .iter()
                    .filter(|row| row.1 > 0.0 || row.2 > 0)
                    .map(|(activity, distance, duration, _)| ActivityDistance { activity: (*activity as i32).into(), distance_meters: *distance, duration_seconds: *duration, ..Default::default() })
                    .collect(),
                country_codes,
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_history(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteHistoryRequest>,
    ) -> ServiceResult<DeleteHistoryResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let (from, to) = optional_range(req.from.as_option(), req.to.as_option())?;
            let from = from.unwrap_or(DateTime::UNIX_EPOCH);
            // An open end reaches past "now" so that samples still on their way are covered.
            let to = to.unwrap_or_else(|| Utc::now() + Duration::days(1));
            let include_footprints = req.include_footprints;
            self.core.cache.rate_limit(&format!("geo-delete-history:{user_id}"), 20, std::time::Duration::from_secs(3600)).await?;

            // Under the pipeline lock: a sample being processed must not write
            // back what is being deleted.
            self.with_state(user_id, async |state| {
                self.delete_points(user_id, from, to).await?;
                let mut tx = self.core.db.begin().await?;
                sqlx::query(
                    "WITH gone AS (
                         DELETE FROM geo_visits WHERE user_id = $1 AND arrived_at >= $2 AND arrived_at < $3 RETURNING place_id
                     )
                     UPDATE geo_places p SET visit_count = GREATEST(p.visit_count - g.n, 0)
                     FROM (SELECT place_id, count(*)::int AS n FROM gone WHERE place_id IS NOT NULL GROUP BY place_id) g
                     WHERE p.id = g.place_id",
                )
                .bind(user_id)
                .bind(from)
                .bind(to)
                .execute(&mut *tx)
                .await?;
                sqlx::query("DELETE FROM geo_nights WHERE user_id = $1 AND started_at >= $2 AND started_at < $3").bind(user_id).bind(from).bind(to).execute(&mut *tx).await?;
                sqlx::query("DELETE FROM geo_activity_hours WHERE user_id = $1 AND hour >= date_trunc('hour', $2::timestamptz) AND hour < $3")
                    .bind(user_id)
                    .bind(from)
                    .bind(to)
                    .execute(&mut *tx)
                    .await?;
                if include_footprints {
                    let removed = sqlx::query("DELETE FROM geo_cells WHERE user_id = $1 AND res = 9 AND first_at >= $2 AND first_at < $3")
                        .bind(user_id)
                        .bind(from)
                        .bind(to)
                        .execute(&mut *tx)
                        .await?
                        .rows_affected();
                    if removed > 0 {
                        self.rebuild_footprints(&mut tx, user_id).await?;
                    }
                }
                let visit_gone = match state.visit_id {
                    Some(visit_id) => !sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM geo_visits WHERE id = $1)").bind(visit_id).fetch_one(&mut *tx).await?,
                    None => false,
                };
                tx.commit().await?;

                // The live state must not bring deleted history back.
                if visit_gone {
                    state.visit_id = None;
                    state.visit_with.clear();
                    state.night = None;
                }
                state.odometer.take();
                state.odometer.disconnect();
                if include_footprints {
                    state.recent_cells.clear();
                }
                Ok(())
            })
            .await?;
            ok(DeleteHistoryResponse::default())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_iso_calendar_dates() {
        assert_eq!(parse_date("2026-11-20").unwrap(), NaiveDate::from_ymd_opt(2026, 11, 20).unwrap());
        for bad in ["", "2026-13-01", "20261120", "2026-02-30", "yesterday"] {
            assert_eq!(parse_date(bad).unwrap_err().reason, "INVALID_DATE", "{bad}");
        }
    }

    #[test]
    fn days_start_at_local_midnight() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
        assert_eq!(day_start(chrono_tz::Asia::Tokyo, day), Utc.with_ymd_and_hms(2026, 11, 19, 15, 0, 0).unwrap());
        assert_eq!(day_start(chrono_tz::America::New_York, day), Utc.with_ymd_and_hms(2026, 11, 20, 5, 0, 0).unwrap());
        assert_eq!(day_start(chrono_tz::UTC, day), Utc.with_ymd_and_hms(2026, 11, 20, 0, 0, 0).unwrap());
        // Midnight does not exist on the day Santiago's clocks jump forward at 00:00: the day starts at 01:00.
        let jump = NaiveDate::from_ymd_opt(2026, 9, 6).unwrap();
        let start = day_start(chrono_tz::America::Santiago, jump);
        assert_eq!(start.with_timezone(&chrono_tz::America::Santiago).date_naive(), jump);
        assert!(start > day_start(chrono_tz::UTC, jump));
    }

    #[test]
    fn ranges_must_be_in_order_when_both_ends_are_given() {
        let stamp = |seconds: i64| tokuly_proto::Timestamp { seconds, ..Default::default() };
        assert!(optional_range(None, None).unwrap() == (None, None));
        assert!(optional_range(Some(&stamp(10)), None).is_ok());
        assert!(optional_range(Some(&stamp(10)), Some(&stamp(20))).is_ok());
        assert_eq!(optional_range(Some(&stamp(20)), Some(&stamp(10))).unwrap_err().reason, "INVALID_RANGE");
        assert_eq!(optional_range(Some(&stamp(10)), Some(&stamp(10))).unwrap_err().reason, "INVALID_RANGE");
    }

    #[test]
    fn memo_holders_are_per_user_and_day() {
        let user = Uuid::nil();
        assert_eq!(
            memo_holder(user, NaiveDate::from_ymd_opt(2026, 11, 20).unwrap()),
            "geo:day-memo:00000000-0000-0000-0000-000000000000:2026-11-20"
        );
    }
}
