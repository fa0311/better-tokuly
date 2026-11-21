//! Footprints (the scratch map): the H3 cells a user has been in, explored
//! share per region, leaderboards and the recap. Also `FootprintService`.
//!
//! Resolution-9 cells are the truth. Their parents at resolutions 7 and 5
//! are materialized so a viewport at low zoom reads a few hundred rows
//! instead of tens of thousands, and every cell is attributed once to the
//! country and subdivision containing its center, which keeps per-region
//! totals (`geo_region_stats`) a matter of adding up.
use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use h3o::CellIndex;
use tokuly_core::{
    outbound::{self, ActivityAudience, ActivityEntry, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::{
    geo::*,
    identity::Audience,
};
use tokuly_social::graph;

use crate::{
    Geo,
    ctx::Privacy,
    math,
    regions::{KIND_ADMIN1, KIND_COUNTRY, RegionIndex, RegionMeta, WORLD_ID},
    state::RegionRef,
};

/// Cells returned by a recap at most.
const RECAP_CELLS: i64 = 5000;
const NOBODY: i32 = Audience::AUDIENCE_NOBODY as i32;
const EVERYONE: i32 = Audience::AUDIENCE_EVERYONE as i32;

/// What uncovering cells changed.
#[derive(Debug, Default)]
pub(crate) struct Uncovered {
    /// Cells that were new (H3 indexes).
    pub new_cells: Vec<i64>,
    /// The user's cell total afterwards.
    pub total: i64,
    /// Regions entered for the first time, and whether that is news (the
    /// very first country and subdivision of an account are not).
    pub new_regions: Vec<(RegionMeta, bool)>,
    /// Regions of the last cell given.
    pub region: RegionRef,
}

pub(crate) fn audience_of(number: i32) -> Audience {
    buffa::EnumValue::<Audience>::from(number).as_known().filter(|a| *a != Audience::AUDIENCE_UNSPECIFIED).unwrap_or(Audience::AUDIENCE_FRIENDS)
}

fn percent(explored_km2: f64, area_km2: f64) -> f64 {
    if area_km2 > 0.0 { (explored_km2 / area_km2 * 100.0).clamp(0.0, 100.0) } else { 0.0 }
}

fn region_stat(meta: &RegionMeta, cell_count: i64, explored_km2: f64, friend_rank: i32, ja: bool) -> RegionStat {
    RegionStat {
        region: meta.to_proto(ja).into(),
        percent: percent(explored_km2, meta.area_km2),
        cell_count,
        explored_km2,
        friend_rank,
        ..Default::default()
    }
}

fn new_region_copy(region: &RegionMeta) -> (Localized, Localized) {
    let (ja, en) = (region.display_name(true), region.display_name(false));
    (Localized::new(format!("{ja}に初上陸!🎉"), format!("First time in {en}! 🎉")), Localized::new("足あとマップを見てみよう!", "Take a look at your footprint map!"))
}

impl Geo {
    /// Marks cells as visited by a user at `at`. Cells already known change
    /// nothing. New regions are announced (activity entry and notification)
    /// in the same transaction.
    pub(crate) async fn uncover(&self, user_id: Uuid, cells: &[CellIndex], at: DateTime<Utc>, privacy: &Privacy) -> AppResult<Uncovered> {
        let mut out = Uncovered::default();
        if cells.is_empty() {
            return Ok(out);
        }
        let centers: Vec<(i64, f64, f64)> = cells
            .iter()
            .map(|cell| {
                let (lat, lng) = math::cell_center(*cell);
                (math::cell_to_db(*cell), lat, lng)
            })
            .collect();
        let regions = self.regions_of(&centers).await?;
        out.region = regions.last().cloned().unwrap_or_default();

        let ids: Vec<i64> = centers.iter().map(|c| c.0).collect();
        let lats: Vec<f64> = centers.iter().map(|c| c.1).collect();
        let lngs: Vec<f64> = centers.iter().map(|c| c.2).collect();
        let countries: Vec<&str> = regions.iter().map(|r| r.country_id.as_str()).collect();
        let admin1s: Vec<&str> = regions.iter().map(|r| r.admin1_id.as_str()).collect();

        let mut tx = self.core.db.begin().await?;
        out.new_cells = sqlx::query_scalar(
            "INSERT INTO geo_cells (user_id, res, cell, lat, lng, first_at, country_id, admin1_id)
             SELECT $1, 9, t.cell, t.lat, t.lng, $2, t.country_id, t.admin1_id
             FROM UNNEST($3::bigint[], $4::float8[], $5::float8[], $6::text[], $7::text[]) AS t(cell, lat, lng, country_id, admin1_id)
             ON CONFLICT DO NOTHING
             RETURNING cell",
        )
        .bind(user_id)
        .bind(at)
        .bind(&ids)
        .bind(&lats)
        .bind(&lngs)
        .bind(&countries)
        .bind(&admin1s)
        .fetch_all(&mut *tx)
        .await?;
        if out.new_cells.is_empty() {
            out.total = sqlx::query_scalar("SELECT cell_count FROM geo_users WHERE user_id = $1").bind(user_id).fetch_optional(&mut *tx).await?.unwrap_or(0);
            return Ok(out);
        }

        // Parents for low zoom levels, and the area per region.
        let fresh: HashSet<i64> = out.new_cells.iter().copied().collect();
        let mut parents: BTreeMap<(i16, i64), (f64, f64)> = BTreeMap::new();
        let mut per_region: BTreeMap<&str, (i64, f64)> = BTreeMap::new();
        let mut added_km2 = 0.0;
        for (cell, region) in cells.iter().zip(&regions) {
            if !fresh.contains(&math::cell_to_db(*cell)) {
                continue;
            }
            let km2 = cell.area_km2();
            added_km2 += km2;
            for id in [region.country_id.as_str(), region.admin1_id.as_str()] {
                if !id.is_empty() {
                    let entry = per_region.entry(id).or_default();
                    entry.0 += 1;
                    entry.1 += km2;
                }
            }
            for resolution in math::PARENT_RESOLUTIONS {
                if let Some(parent) = cell.parent(resolution) {
                    parents.entry((u8::from(resolution) as i16, math::cell_to_db(parent))).or_insert_with(|| math::cell_center(parent));
                }
            }
        }
        // (BTreeMaps: rows are always written in the same order, so concurrent writers cannot deadlock.)
        let resolutions: Vec<i16> = parents.keys().map(|(res, _)| *res).collect();
        let parent_ids: Vec<i64> = parents.keys().map(|(_, cell)| *cell).collect();
        let parent_lats: Vec<f64> = parents.values().map(|center| center.0).collect();
        let parent_lngs: Vec<f64> = parents.values().map(|center| center.1).collect();
        sqlx::query(
            "INSERT INTO geo_cells (user_id, res, cell, lat, lng, first_at)
             SELECT $1, t.res, t.cell, t.lat, t.lng, $2
             FROM UNNEST($3::smallint[], $4::bigint[], $5::float8[], $6::float8[]) AS t(res, cell, lat, lng)
             ON CONFLICT DO NOTHING",
        )
        .bind(user_id)
        .bind(at)
        .bind(resolutions)
        .bind(parent_ids)
        .bind(parent_lats)
        .bind(parent_lngs)
        .execute(&mut *tx)
        .await?;

        out.total = sqlx::query_scalar(
            "INSERT INTO geo_users (user_id, footprints_audience, cell_count, explored_km2) VALUES ($1, $2, $3, $4)
             ON CONFLICT (user_id) DO UPDATE SET
                 cell_count = geo_users.cell_count + EXCLUDED.cell_count,
                 explored_km2 = geo_users.explored_km2 + EXCLUDED.explored_km2,
                 footprints_audience = EXCLUDED.footprints_audience
             RETURNING cell_count",
        )
        .bind(user_id)
        .bind(privacy.footprints_audience as i16)
        .bind(out.new_cells.len() as i64)
        .bind(added_km2)
        .fetch_one(&mut *tx)
        .await?;

        if !per_region.is_empty() {
            let region_ids: Vec<&str> = per_region.keys().copied().collect();
            let counts: Vec<i64> = per_region.values().map(|v| v.0).collect();
            let areas: Vec<f64> = per_region.values().map(|v| v.1).collect();
            let touched: Vec<(String, bool)> = sqlx::query_as(
                "INSERT INTO geo_region_stats (user_id, region_id, cell_count, explored_km2, first_at)
                 SELECT $1, t.region_id, t.cells, t.km2, $2 FROM UNNEST($3::text[], $4::bigint[], $5::float8[]) AS t(region_id, cells, km2)
                 ON CONFLICT (user_id, region_id) DO UPDATE SET
                     cell_count = geo_region_stats.cell_count + EXCLUDED.cell_count,
                     explored_km2 = geo_region_stats.explored_km2 + EXCLUDED.explored_km2
                 RETURNING region_id, (xmax = 0) AS first_time",
            )
            .bind(user_id)
            .bind(at)
            .bind(region_ids)
            .bind(counts)
            .bind(areas)
            .fetch_all(&mut *tx)
            .await?;
            let first_time: Vec<String> = touched.into_iter().filter(|(_, first_time)| *first_time).map(|(id, _)| id).collect();
            if !first_time.is_empty() {
                let index = self.region_index().await?;
                let (country_count, admin1_count): (i64, i64) = sqlx::query_as(
                    "SELECT count(*) FILTER (WHERE r.kind = 2), count(*) FILTER (WHERE r.kind = 3)
                     FROM geo_region_stats s JOIN geo_regions r ON r.id = s.region_id WHERE s.user_id = $1",
                )
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await?;
                let new_countries = first_time.iter().filter(|id| index.get(id).is_some_and(|r| r.kind == KIND_COUNTRY)).count() as i64;
                let new_admin1s = first_time.len() as i64 - new_countries;
                for id in &first_time {
                    let Some(region) = index.get(id) else { continue };
                    // The regions an account starts in are not an achievement.
                    let had_before = if region.kind == KIND_COUNTRY { country_count - new_countries } else { admin1_count - new_admin1s };
                    let news = had_before > 0;
                    if news {
                        let (title, body) = new_region_copy(region);
                        let audience = if privacy.footprints_audience == NOBODY { ActivityAudience::Private } else { ActivityAudience::Friends };
                        let entry = ActivityEntry::new(user_id, "geo.new_region", audience, title.clone())
                            .path("/footprints")
                            .at(region.center.0, region.center.1)
                            .data(serde_json::json!({ "region_id": region.id, "kind": region.kind, "country_code": region.country_code }));
                        outbound::activity(&mut *tx, &entry).await?;
                        let notification = NotificationRequest::new(user_id, "geo.new_region", title, body, "/footprints")
                            .collapse(format!("geo.new_region:{}", region.id))
                            .data(serde_json::json!({ "region_id": region.id }));
                        outbound::notify(&mut *tx, &notification).await?;
                    }
                    out.new_regions.push((region.clone(), news));
                }
            }
        }
        tx.commit().await?;
        Ok(out)
    }

    /// Recomputes the parent cells and every total of a user from their
    /// resolution-9 cells (after cells were deleted).
    pub(crate) async fn rebuild_footprints(&self, tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, user_id: Uuid) -> AppResult<()> {
        let cells: Vec<(i64, DateTime<Utc>, String, String)> =
            sqlx::query_as("SELECT cell, first_at, country_id, admin1_id FROM geo_cells WHERE user_id = $1 AND res = 9").bind(user_id).fetch_all(&mut **tx).await?;
        let mut parents: BTreeMap<(i16, i64), ((f64, f64), DateTime<Utc>)> = BTreeMap::new();
        let mut per_region: BTreeMap<String, (i64, f64, DateTime<Utc>)> = BTreeMap::new();
        let mut total_km2 = 0.0;
        for (cell, first_at, country_id, admin1_id) in &cells {
            let Some(cell) = math::cell_from_db(*cell) else { continue };
            let km2 = cell.area_km2();
            total_km2 += km2;
            for id in [country_id, admin1_id] {
                if !id.is_empty() {
                    let entry = per_region.entry(id.clone()).or_insert((0, 0.0, *first_at));
                    entry.0 += 1;
                    entry.1 += km2;
                    entry.2 = entry.2.min(*first_at);
                }
            }
            for resolution in math::PARENT_RESOLUTIONS {
                if let Some(parent) = cell.parent(resolution) {
                    let entry = parents.entry((u8::from(resolution) as i16, math::cell_to_db(parent))).or_insert_with(|| (math::cell_center(parent), *first_at));
                    entry.1 = entry.1.min(*first_at);
                }
            }
        }
        sqlx::query("DELETE FROM geo_cells WHERE user_id = $1 AND res <> 9").bind(user_id).execute(&mut **tx).await?;
        sqlx::query(
            "INSERT INTO geo_cells (user_id, res, cell, lat, lng, first_at)
             SELECT $1, t.res, t.cell, t.lat, t.lng, t.first_at
             FROM UNNEST($2::smallint[], $3::bigint[], $4::float8[], $5::float8[], $6::timestamptz[]) AS t(res, cell, lat, lng, first_at)",
        )
        .bind(user_id)
        .bind(parents.keys().map(|k| k.0).collect::<Vec<_>>())
        .bind(parents.keys().map(|k| k.1).collect::<Vec<_>>())
        .bind(parents.values().map(|v| v.0.0).collect::<Vec<_>>())
        .bind(parents.values().map(|v| v.0.1).collect::<Vec<_>>())
        .bind(parents.values().map(|v| v.1).collect::<Vec<_>>())
        .execute(&mut **tx)
        .await?;
        sqlx::query("DELETE FROM geo_region_stats WHERE user_id = $1").bind(user_id).execute(&mut **tx).await?;
        sqlx::query(
            "INSERT INTO geo_region_stats (user_id, region_id, cell_count, explored_km2, first_at)
             SELECT $1, t.region_id, t.cells, t.km2, t.first_at
             FROM UNNEST($2::text[], $3::bigint[], $4::float8[], $5::timestamptz[]) AS t(region_id, cells, km2, first_at)
             WHERE EXISTS (SELECT 1 FROM geo_regions r WHERE r.id = t.region_id)",
        )
        .bind(user_id)
        .bind(per_region.keys().cloned().collect::<Vec<_>>())
        .bind(per_region.values().map(|v| v.0).collect::<Vec<_>>())
        .bind(per_region.values().map(|v| v.1).collect::<Vec<_>>())
        .bind(per_region.values().map(|v| v.2).collect::<Vec<_>>())
        .execute(&mut **tx)
        .await?;
        sqlx::query("UPDATE geo_users SET cell_count = $2, explored_km2 = $3 WHERE user_id = $1")
            .bind(user_id)
            .bind(cells.len() as i64)
            .bind(total_km2)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    /// Resolves whose footprints a request is about and whether the caller
    /// may see them in full (cells) or only as statistics.
    async fn footprint_owner(&self, viewer: Uuid, user_id: &str) -> AppResult<(Uuid, bool)> {
        let owner = parse_opt_id(user_id)?.unwrap_or(viewer);
        if owner == viewer {
            return Ok((owner, true));
        }
        let ctx = self.ctx_of(owner).await?;
        ensure!(ctx.active && !graph::is_blocked_either(&self.core, owner, viewer).await?, not_found, "USER_NOT_FOUND", "user not found");
        let full = graph::allows(&self.core, owner, Some(viewer), audience_of(ctx.privacy.footprints_audience)).await?;
        Ok((owner, full))
    }

    /// The friends of `user_id` who show them their footprints.
    async fn footprint_friends(&self, user_id: Uuid) -> AppResult<Vec<Uuid>> {
        let friends = graph::friend_ids(&self.core, user_id).await?;
        if friends.is_empty() {
            return Ok(friends);
        }
        Ok(sqlx::query_scalar("SELECT user_id FROM geo_users WHERE user_id = ANY($1) AND footprints_audience <> $2")
            .bind(&friends)
            .bind(NOBODY as i16)
            .fetch_all(&self.core.db)
            .await?)
    }

    /// The share of the user's home country they have explored, and how many
    /// countries they have been to (for profiles). The home country is that
    /// of their home place, or else the country they have explored most.
    pub(crate) async fn explored_summary(&self, user_id: Uuid) -> AppResult<(f64, i32)> {
        let index = self.region_index().await?;
        let rows: Vec<(String, i64, f64)> =
            sqlx::query_as("SELECT region_id, cell_count, explored_km2 FROM geo_region_stats WHERE user_id = $1").bind(user_id).fetch_all(&self.core.db).await?;
        let countries: Vec<(&RegionMeta, i64, f64)> =
            rows.iter().filter_map(|(id, cells, km2)| index.get(id).filter(|r| r.kind == KIND_COUNTRY).map(|r| (r, *cells, *km2))).collect();
        if countries.is_empty() {
            return Ok((0.0, 0));
        }
        let home: Option<(f64, f64)> = sqlx::query_as("SELECT lat, lng FROM geo_places WHERE owner_id = $1 AND label = 1").bind(user_id).fetch_optional(&self.core.db).await?;
        let home_country = match home {
            Some((lat, lng)) => self.regions_at(lat, lng).await?.country_id,
            None => String::new(),
        };
        let best = countries.iter().find(|(region, _, _)| region.id == home_country).or_else(|| countries.iter().max_by_key(|(_, cells, _)| *cells));
        Ok((best.map_or(0.0, |(region, _, km2)| percent(*km2, region.area_km2)), countries.len() as i32))
    }
}

/// The margin (in degrees) by which a viewport is widened so that cells
/// overlapping its edge are included.
fn margin_degrees(resolution: h3o::Resolution) -> f64 {
    match resolution {
        h3o::Resolution::Nine => 0.003,
        h3o::Resolution::Seven => 0.02,
        _ => 0.15,
    }
}

/// Who a leaderboard ranks.
enum Rivals {
    /// The caller and the friends who show them their footprints.
    Friends(Vec<Uuid>),
    /// Everyone whose footprints are public (and the caller), minus people
    /// in a block with the caller.
    Everyone { hidden: Vec<Uuid> },
}

/// Starts a query over the ranked users of a region (`None` = the world,
/// ranked by the totals in `geo_users`). Callers append ordering or more
/// conditions.
fn ranking_query<'a>(select: &str, region_id: Option<&'a str>, viewer: Uuid, rivals: &'a Rivals) -> sqlx::QueryBuilder<'a, sqlx::Postgres> {
    let mut query = sqlx::QueryBuilder::new(select);
    match region_id {
        Some(id) => {
            query.push(" FROM geo_region_stats s JOIN geo_users g ON g.user_id = s.user_id JOIN users u ON u.id = s.user_id WHERE s.region_id = ").push_bind(id);
        }
        None => {
            query.push(" FROM geo_users s JOIN geo_users g ON g.user_id = s.user_id JOIN users u ON u.id = s.user_id WHERE true");
        }
    }
    query.push(" AND s.cell_count > 0 AND u.deleted_at IS NULL AND u.banned_at IS NULL");
    match rivals {
        Rivals::Friends(ids) => {
            query.push(" AND s.user_id = ANY(").push_bind(ids).push(")");
        }
        Rivals::Everyone { hidden } => {
            query.push(" AND (s.user_id = ").push_bind(viewer);
            query.push(" OR (g.footprints_audience = ").push_bind(EVERYONE as i16);
            query.push(" AND NOT (s.user_id = ANY(").push_bind(hidden).push("))))");
        }
    }
    query
}

fn world_stat(index: &RegionIndex, cell_count: i64, explored_km2: f64, friend_rank: i32, ja: bool) -> RegionStat {
    RegionStat {
        region: index.world(ja).into(),
        percent: percent(explored_km2, index.world_area_km2),
        cell_count,
        explored_km2,
        friend_rank,
        ..Default::default()
    }
}

impl FootprintService for Geo {
    async fn get_cells(&self, ctx: RequestContext, request: ServiceRequest<'_, GetCellsRequest>) -> ServiceResult<GetCellsResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let req = request.to_owned_message();
            let bounds = req.bounds.as_option().ok_or_else(|| AppError::invalid("INVALID_BOUNDS", "a viewport is required"))?;
            let (Some(sw), Some(ne)) = (bounds.south_west.as_option(), bounds.north_east.as_option()) else {
                bail!(invalid, "INVALID_BOUNDS", "a viewport is required");
            };
            ensure!(
                math::valid_coordinates(sw.lat, sw.lng) && math::valid_coordinates(ne.lat, ne.lng) && sw.lat <= ne.lat,
                invalid,
                "INVALID_BOUNDS",
                "the viewport is not a valid box"
            );
            let (owner, full) = self.footprint_owner(viewer, &req.user_id).await?;
            ensure!(full, denied, "FOOTPRINTS_PRIVATE", "this scratch map is not shown to you");

            let resolution = math::resolution_for_zoom(req.zoom);
            let margin = margin_degrees(resolution);
            let (south, north) = (sw.lat - margin, ne.lat + margin);
            let (west, east) = (sw.lng - margin, ne.lng + margin);
            // West of east: the viewport crosses the antimeridian.
            let crossing = sw.lng > ne.lng;
            let limit = self.cfg.max_cells;
            let mut cells: Vec<i64> = sqlx::query_scalar(
                "SELECT cell FROM geo_cells
                 WHERE user_id = $1 AND res = $2 AND lat BETWEEN $3 AND $4
                   AND CASE WHEN $7 THEN (lng >= $5 OR lng <= $6) ELSE lng BETWEEN $5 AND $6 END
                 ORDER BY cell LIMIT $8",
            )
            .bind(owner)
            .bind(u8::from(resolution) as i16)
            .bind(south)
            .bind(north)
            .bind(west)
            .bind(east)
            .bind(crossing)
            .bind(limit + 1)
            .fetch_all(&self.core.db)
            .await?;
            let truncated = cells.len() as i64 > limit;
            cells.truncate(limit as usize);
            ok(GetCellsResponse {
                cells: cells.into_iter().map(math::cell_hex).collect(),
                resolution: u8::from(resolution) as i32,
                truncated,
                ..Default::default()
            })
        })
        .await
    }

    async fn get_stats(&self, ctx: RequestContext, request: ServiceRequest<'_, GetFootprintStatsRequest>) -> ServiceResult<GetFootprintStatsResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let req = request.to_owned_message();
            let (owner, _) = self.footprint_owner(viewer, &req.user_id).await?;
            let ja = self.speaks_japanese(viewer).await?;
            let index = self.region_index().await?;

            let (total_cells, total_km2): (i64, f64) =
                sqlx::query_as("SELECT cell_count, explored_km2 FROM geo_users WHERE user_id = $1").bind(owner).fetch_optional(&self.core.db).await?.unwrap_or((0, 0.0));
            let rows: Vec<(String, i64, f64)> =
                sqlx::query_as("SELECT region_id, cell_count, explored_km2 FROM geo_region_stats WHERE user_id = $1").bind(owner).fetch_all(&self.core.db).await?;
            let country_count = rows.iter().filter(|(id, _, _)| index.get(id).is_some_and(|r| r.kind == KIND_COUNTRY)).count() as i32;

            // Ranks among friends are the caller's own business only.
            let mut ranks: HashMap<String, i32> = HashMap::new();
            let mut world_rank = 0;
            if owner == viewer {
                let friends = self.footprint_friends(viewer).await?;
                let ranked: Vec<(String, i64)> = sqlx::query_as(
                    "SELECT mine.region_id, count(theirs.user_id) + 1
                     FROM geo_region_stats mine
                     LEFT JOIN geo_region_stats theirs
                            ON theirs.region_id = mine.region_id AND theirs.user_id = ANY($2) AND theirs.cell_count > mine.cell_count
                     WHERE mine.user_id = $1
                     GROUP BY mine.region_id",
                )
                .bind(viewer)
                .bind(&friends)
                .fetch_all(&self.core.db)
                .await?;
                ranks = ranked.into_iter().map(|(id, rank)| (id, rank as i32)).collect();
                if total_cells > 0 {
                    let ahead: i64 = sqlx::query_scalar("SELECT count(*) FROM geo_users WHERE user_id = ANY($1) AND cell_count > $2")
                        .bind(&friends)
                        .bind(total_cells)
                        .fetch_one(&self.core.db)
                        .await?;
                    world_rank = ahead as i32 + 1;
                }
            }

            let kind = req.kind.as_known().unwrap_or(RegionKind::REGION_KIND_UNSPECIFIED);
            let containing = match req.at.as_option() {
                Some(at) => {
                    ensure!(math::valid_coordinates(at.lat, at.lng), invalid, "INVALID_LOCATION", "a valid location is required");
                    Some(self.regions_at(at.lat, at.lng).await?)
                }
                None => None,
            };
            let mut regions: Vec<RegionStat> = rows
                .iter()
                .filter_map(|(id, cells, km2)| index.get(id).map(|meta| (meta, *cells, *km2)))
                .filter(|(meta, _, _)| match kind {
                    RegionKind::REGION_KIND_UNSPECIFIED => true,
                    RegionKind::REGION_KIND_COUNTRY => meta.kind == KIND_COUNTRY,
                    RegionKind::REGION_KIND_ADMIN1 => meta.kind == KIND_ADMIN1,
                    // The world is reported separately; city boundaries are not bundled.
                    RegionKind::REGION_KIND_WORLD | RegionKind::REGION_KIND_CITY => false,
                })
                .filter(|(meta, _, _)| containing.as_ref().is_none_or(|here| meta.id == here.country_id || meta.id == here.admin1_id))
                .map(|(meta, cells, km2)| region_stat(meta, cells, km2, ranks.get(&meta.id).copied().unwrap_or(0), ja))
                .collect();
            regions.sort_by(|a, b| b.percent.total_cmp(&a.percent).then_with(|| b.cell_count.cmp(&a.cell_count)));
            ok(GetFootprintStatsResponse {
                world: world_stat(&index, total_cells, total_km2, world_rank, ja).into(),
                regions,
                total_cell_count: total_cells,
                country_count,
                ..Default::default()
            })
        })
        .await
    }

    async fn get_leaderboard(&self, ctx: RequestContext, request: ServiceRequest<'_, GetLeaderboardRequest>) -> ServiceResult<GetLeaderboardResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let limit = if request.limit <= 0 { 20 } else { request.limit.min(100) } as i64;
            let global = request.scope.as_known() == Some(LeaderboardScope::LEADERBOARD_SCOPE_GLOBAL);
            let ja = self.speaks_japanese(viewer).await?;
            let index = self.region_index().await?;
            let region = match request.region_id {
                "" | WORLD_ID => None,
                id => Some(index.get(id).or_not_found("REGION_NOT_FOUND", "region not found")?),
            };
            let (region_message, area_km2) = match region {
                Some(meta) => (meta.to_proto(ja), meta.area_km2),
                None => (index.world(ja), index.world_area_km2),
            };

            // Who competes: the caller plus either the friends who show them
            // their footprints, or everyone who shows theirs to everyone.
            let scope = if global {
                Rivals::Everyone { hidden: graph::blocked_ids_either(&self.core, viewer).await? }
            } else {
                let mut friends = self.footprint_friends(viewer).await?;
                friends.push(viewer);
                Rivals::Friends(friends)
            };
            let region_id = region.map(|meta| meta.id.as_str());
            let mut query = ranking_query("SELECT s.user_id, s.cell_count, s.explored_km2", region_id, viewer, &scope);
            query.push(" ORDER BY s.cell_count DESC, s.user_id LIMIT ").push_bind(limit);
            let top: Vec<(Uuid, i64, f64)> = query.build_query_as().fetch_all(&self.core.db).await?;
            let mine: Option<(i64, f64)> = match region_id {
                Some(id) => {
                    sqlx::query_as("SELECT cell_count, explored_km2 FROM geo_region_stats WHERE user_id = $1 AND region_id = $2 AND cell_count > 0")
                        .bind(viewer)
                        .bind(id)
                        .fetch_optional(&self.core.db)
                        .await?
                }
                None => {
                    sqlx::query_as("SELECT cell_count, explored_km2 FROM geo_users WHERE user_id = $1 AND cell_count > 0")
                        .bind(viewer)
                        .fetch_optional(&self.core.db)
                        .await?
                }
            };

            let ids: Vec<Uuid> = top.iter().map(|row| row.0).chain([viewer]).collect();
            let summaries = tokuly_identity::summaries(&self.core, &ids).await?;
            let entry = |rank: i32, user_id: Uuid, cells: i64, km2: f64| LeaderboardEntry {
                rank,
                user: tokuly_identity::users::pick(&summaries, user_id).into(),
                percent: percent(km2, area_km2),
                cell_count: cells,
                ..Default::default()
            };
            // Equal cell counts share a rank.
            let mut entries = Vec::with_capacity(top.len());
            let (mut rank, mut previous) = (0, i64::MAX);
            for (position, (user_id, cells, km2)) in top.iter().enumerate() {
                if *cells < previous {
                    rank = position as i32 + 1;
                    previous = *cells;
                }
                entries.push(entry(rank, *user_id, *cells, *km2));
            }
            let me = match mine {
                Some((cells, km2)) => {
                    let mut query = ranking_query("SELECT count(*)", region_id, viewer, &scope);
                    query.push(" AND s.cell_count > ").push_bind(cells);
                    let ahead: i64 = query.build_query_scalar().fetch_one(&self.core.db).await?;
                    Some(entry(ahead as i32 + 1, viewer, cells, km2))
                }
                None => None,
            };
            ok(GetLeaderboardResponse { region: region_message.into(), entries, me: me.into(), ..Default::default() })
        })
        .await
    }

    async fn get_recap(&self, ctx: RequestContext, request: ServiceRequest<'_, GetRecapRequest>) -> ServiceResult<GetRecapResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let db = &self.core.db;
            self.settle_odometer(user_id).await?;
            let ja = self.speaks_japanese(user_id).await?;
            let index = self.region_index().await?;
            let now = Utc::now();
            let marker: Option<(Option<DateTime<Utc>>, DateTime<Utc>)> =
                sqlx::query_as("SELECT recap_seen_at, created_at FROM geo_users WHERE user_id = $1").bind(user_id).fetch_optional(db).await?;
            // Never looked before: everything since the first footprint.
            let since = marker.map_or(now, |(seen, created)| seen.unwrap_or(created - chrono::Duration::seconds(1)));

            let new_cells: Vec<i64> = sqlx::query_scalar("SELECT cell FROM geo_cells WHERE user_id = $1 AND res = 9 AND first_at > $2 ORDER BY first_at DESC, cell LIMIT $3")
                .bind(user_id)
                .bind(since)
                .bind(RECAP_CELLS)
                .fetch_all(db)
                .await?;
            let (new_cell_count, south, west, north, east): (i64, Option<f64>, Option<f64>, Option<f64>, Option<f64>) =
                sqlx::query_as("SELECT count(*), min(lat), min(lng), max(lat), max(lng) FROM geo_cells WHERE user_id = $1 AND res = 9 AND first_at > $2")
                    .bind(user_id)
                    .bind(since)
                    .fetch_one(db)
                    .await?;
            // Regions that gained cells, with their totals as they stand now.
            let regions: Vec<(String, i64, f64)> = sqlx::query_as(
                "SELECT s.region_id, s.cell_count, s.explored_km2 FROM geo_region_stats s
                 WHERE s.user_id = $1 AND s.region_id IN (
                     SELECT country_id FROM geo_cells WHERE user_id = $1 AND res = 9 AND first_at > $2
                     UNION SELECT admin1_id FROM geo_cells WHERE user_id = $1 AND res = 9 AND first_at > $2)",
            )
            .bind(user_id)
            .bind(since)
            .fetch_all(db)
            .await?;
            let mut regions: Vec<RegionStat> =
                regions.iter().filter_map(|(id, cells, km2)| index.get(id).map(|meta| region_stat(meta, *cells, *km2, 0, ja))).collect();
            regions.sort_by(|a, b| b.percent.total_cmp(&a.percent));
            let new_regions: Vec<String> = sqlx::query_scalar("SELECT region_id FROM geo_region_stats WHERE user_id = $1 AND first_at > $2 ORDER BY first_at")
                .bind(user_id)
                .bind(since)
                .fetch_all(db)
                .await?;
            let distance_meters: f64 = sqlx::query_scalar("SELECT COALESCE(sum(distance_m), 0)::float8 FROM geo_activity_hours WHERE user_id = $1 AND hour >= date_trunc('hour', $2::timestamptz)")
                .bind(user_id)
                .bind(since)
                .fetch_one(db)
                .await?;
            if request.mark_seen {
                sqlx::query(
                    "INSERT INTO geo_users (user_id, recap_seen_at) VALUES ($1, $2)
                     ON CONFLICT (user_id) DO UPDATE SET recap_seen_at = EXCLUDED.recap_seen_at",
                )
                .bind(user_id)
                .bind(now)
                .execute(db)
                .await?;
            }
            let bounds = match (south, west, north, east) {
                (Some(south), Some(west), Some(north), Some(east)) => Some(tokuly_proto::common::BoundingBox {
                    south_west: math::lat_lng(south, west).into(),
                    north_east: math::lat_lng(north, east).into(),
                    ..Default::default()
                }),
                _ => None,
            };
            ok(GetRecapResponse {
                since: ts(since),
                new_cells: new_cells.into_iter().map(math::cell_hex).collect(),
                new_cell_count,
                regions,
                new_regions: new_regions.iter().filter_map(|id| index.get(id)).map(|meta| meta.to_proto(ja)).collect(),
                distance_meters,
                bounds: bounds.into(),
                ..Default::default()
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentages_are_bounded() {
        assert_eq!(percent(1.0, 4.0), 25.0);
        assert_eq!(percent(10.0, 4.0), 100.0);
        assert_eq!(percent(1.0, 0.0), 0.0);
    }

    #[test]
    fn unknown_audiences_fall_back_to_friends() {
        assert_eq!(audience_of(1), Audience::AUDIENCE_EVERYONE);
        assert_eq!(audience_of(4), Audience::AUDIENCE_NOBODY);
        assert_eq!(audience_of(0), Audience::AUDIENCE_FRIENDS);
        assert_eq!(audience_of(99), Audience::AUDIENCE_FRIENDS);
    }

    #[test]
    fn new_regions_are_announced_in_both_languages() {
        let tokyo = RegionMeta { id: "JP-13".into(), kind: KIND_ADMIN1, name: "Tokyo".into(), name_ja: "東京都".into(), country_code: "JP".into(), area_km2: 2194.0, center: (35.7, 139.7) };
        let (title, _) = new_region_copy(&tokyo);
        assert_eq!(title.ja, "東京都に初上陸!🎉");
        assert_eq!(title.en, "First time in Tokyo! 🎉");
    }
}
