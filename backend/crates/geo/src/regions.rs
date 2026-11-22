//! Countries and first-level subdivisions: seeding the bundled boundaries
//! into PostGIS and finding the regions a footprint cell belongs to.
//!
//! The geometry stays in PostgreSQL (`geo_regions`, GiST-indexed); names,
//! areas and centers of the ~4,700 regions are also kept in memory
//! ([`RegionIndex`]) because every statistics request needs them.
use std::{collections::HashMap, io::Read as _, sync::Arc};

use parking_lot::RwLock;
use tokuly_core::prelude::*;
use tokuly_proto::geo::{Region, RegionKind};

use crate::{
    Geo,
    geocode::Gazetteer,
    math::{self, Point},
    state::RegionRef,
};

static REGIONS_GZ: &[u8] = include_bytes!("../data/regions.ndjson.gz");

/// ID of the pseudo-region covering all land.
pub const WORLD_ID: &str = "world";
pub(crate) const KIND_COUNTRY: i32 = RegionKind::REGION_KIND_COUNTRY as i32;
pub(crate) const KIND_ADMIN1: i32 = RegionKind::REGION_KIND_ADMIN1 as i32;

/// Cells whose center is just off a (simplified) coastline still belong to
/// the nearest region within this many degrees (about 3 km).
const COAST_TOLERANCE_DEG: f64 = 0.03;
/// Islands missing from the simplified boundaries are attributed through the
/// nearest gazetteer place within this distance.
const ISLAND_TOLERANCE_M: f64 = 30_000.0;
const SEED_BATCH: usize = 250;

#[derive(Debug, Clone, PartialEq)]
pub struct RegionMeta {
    pub id: String,
    /// `RegionKind` number.
    pub kind: i32,
    pub name: String,
    pub name_ja: String,
    pub country_code: String,
    pub area_km2: f64,
    pub center: Point,
}

impl RegionMeta {
    pub fn display_name(&self, ja: bool) -> &str {
        if ja && !self.name_ja.is_empty() { &self.name_ja } else { &self.name }
    }

    pub fn to_proto(&self, ja: bool) -> Region {
        Region {
            id: self.id.clone(),
            kind: self.kind.into(),
            name: self.name.clone(),
            local_name: if ja { self.name_ja.clone() } else { String::new() },
            country_code: self.country_code.clone(),
            area_km2: self.area_km2,
            center: math::lat_lng(self.center.0, self.center.1).into(),
            ..Default::default()
        }
    }
}

/// Everything about the seeded regions except their geometry.
#[derive(Debug, Default)]
pub struct RegionIndex {
    pub by_id: HashMap<String, RegionMeta>,
    /// GeoNames admin1 key (`JP.40`) → region ID.
    by_gn: HashMap<String, String>,
    /// Land area of all countries together.
    pub world_area_km2: f64,
}

impl RegionIndex {
    pub fn get(&self, id: &str) -> Option<&RegionMeta> {
        self.by_id.get(id)
    }

    /// The whole world as a region.
    pub fn world(&self, ja: bool) -> Region {
        Region {
            id: WORLD_ID.to_owned(),
            kind: RegionKind::REGION_KIND_WORLD.into(),
            name: "World".to_owned(),
            local_name: if ja { "世界".to_owned() } else { String::new() },
            area_km2: self.world_area_km2,
            ..Default::default()
        }
    }
}

pub(crate) struct Regions {
    index: RwLock<Option<Arc<RegionIndex>>>,
    /// Regions of footprint cells seen recently (cells in cities are shared by many users).
    cells: moka::sync::Cache<i64, RegionRef>,
}

impl Default for Regions {
    fn default() -> Self {
        Self { index: RwLock::new(None), cells: moka::sync::Cache::new(200_000) }
    }
}

/// Fingerprint of the bundled boundary data: reseeding happens when it changes.
fn data_version() -> String {
    let hash = REGIONS_GZ
        .iter()
        .fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| (hash ^ *byte as u64).wrapping_mul(0x0100_0000_01b3));
    format!("{}-{hash:016x}", REGIONS_GZ.len())
}

#[derive(serde::Deserialize)]
struct SeedRegion {
    id: String,
    kind: String,
    name: String,
    #[serde(default)]
    name_ja: String,
    #[serde(default)]
    cc: String,
    #[serde(default)]
    gn: String,
    geometry: serde_json::Value,
}

impl Geo {
    /// Loads the bundled country and subdivision boundaries into
    /// `geo_regions`. Idempotent and safe to call from several processes at
    /// once: it does nothing when the stored data is already the bundled
    /// version. Call it at startup (after migrations) and from tests.
    pub async fn seed_regions(&self) -> AppResult<()> {
        let version = data_version();
        let stored: Option<String> = sqlx::query_scalar("SELECT value FROM geo_meta WHERE key = 'regions_version'")
            .fetch_optional(&self.core.db)
            .await?;
        if stored.as_deref() == Some(version.as_str()) {
            return Ok(());
        }
        let started = std::time::Instant::now();
        let mut text = String::new();
        flate2::read::GzDecoder::new(REGIONS_GZ)
            .read_to_string(&mut text)
            .map_err(|e| AppError::internal(anyhow::anyhow!("bundled region data is corrupt: {e}")))?;
        let regions: Vec<SeedRegion> = text
            .lines()
            .filter(|line| !line.is_empty())
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()
            .map_err(|e| AppError::internal(anyhow::anyhow!("bundled region data is corrupt: {e}")))?;

        let mut tx = self.core.db.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('geo:seed_regions', 0))").execute(&mut *tx).await?;
        // Another process may have seeded while we waited for the lock.
        let current: Option<String> = sqlx::query_scalar("SELECT value FROM geo_meta WHERE key = 'regions_version'")
            .fetch_optional(&mut *tx)
            .await?;
        if current.as_deref() == Some(version.as_str()) {
            return Ok(());
        }
        for batch in regions.chunks(SEED_BATCH) {
            let ids: Vec<&str> = batch.iter().map(|r| r.id.as_str()).collect();
            let kinds: Vec<i16> = batch
                .iter()
                .map(|r| if r.kind == "country" { KIND_COUNTRY as i16 } else { KIND_ADMIN1 as i16 })
                .collect();
            let names: Vec<&str> = batch.iter().map(|r| r.name.as_str()).collect();
            let names_ja: Vec<&str> = batch.iter().map(|r| r.name_ja.as_str()).collect();
            let countries: Vec<&str> = batch.iter().map(|r| r.cc.as_str()).collect();
            let gn_codes: Vec<&str> = batch.iter().map(|r| r.gn.as_str()).collect();
            let geometries: Vec<String> = batch.iter().map(|r| r.geometry.to_string()).collect();
            sqlx::query(
                "INSERT INTO geo_regions (id, kind, name, name_ja, country_code, gn_code, area_km2, center_lat, center_lng, geom)
                 SELECT s.id, s.kind, s.name, s.name_ja, s.cc, s.gn, ST_Area(s.g::geography) / 1000000.0,
                        ST_Y(ST_PointOnSurface(s.g)), ST_X(ST_PointOnSurface(s.g)), s.g
                 FROM (
                     SELECT t.id, t.kind, t.name, t.name_ja, t.cc, t.gn,
                            ST_Multi(ST_CollectionExtract(ST_MakeValid(ST_SetSRID(ST_GeomFromGeoJSON(t.geojson), 4326)), 3)) AS g
                     FROM UNNEST($1::text[], $2::smallint[], $3::text[], $4::text[], $5::text[], $6::text[], $7::text[])
                          AS t(id, kind, name, name_ja, cc, gn, geojson)
                 ) s
                 WHERE NOT ST_IsEmpty(s.g)
                 ON CONFLICT (id) DO UPDATE SET
                     kind = EXCLUDED.kind, name = EXCLUDED.name, name_ja = EXCLUDED.name_ja, country_code = EXCLUDED.country_code,
                     gn_code = EXCLUDED.gn_code, area_km2 = EXCLUDED.area_km2, center_lat = EXCLUDED.center_lat,
                     center_lng = EXCLUDED.center_lng, geom = EXCLUDED.geom",
            )
            .bind(ids)
            .bind(kinds)
            .bind(names)
            .bind(names_ja)
            .bind(countries)
            .bind(gn_codes)
            .bind(geometries)
            .execute(&mut *tx)
            .await?;
        }
        // Regions that disappeared from the data (their statistics go with them).
        let keep: Vec<&str> = regions.iter().map(|r| r.id.as_str()).collect();
        sqlx::query("DELETE FROM geo_regions WHERE id <> ALL($1)").bind(keep).execute(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO geo_meta (key, value)
             SELECT 'world_area_km2', COALESCE(sum(area_km2), 0)::text FROM geo_regions WHERE kind = 2
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO geo_meta (key, value) VALUES ('regions_version', $1) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value")
            .bind(&version)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.reload_regions();
        tracing::info!(
            regions = regions.len(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "footprint regions seeded"
        );
        Ok(())
    }

    /// Forgets what is cached about regions, so the next use reads the tables
    /// again. For operators and tests that edit `geo_regions` by hand.
    pub fn reload_regions(&self) {
        *self.regions.index.write() = None;
        self.regions.cells.invalidate_all();
    }

    /// Names, kinds, areas and centers of every region.
    pub(crate) async fn region_index(&self) -> AppResult<Arc<RegionIndex>> {
        if let Some(index) = self.regions.index.read().clone() {
            return Ok(index);
        }
        let rows: Vec<(String, i16, String, String, String, String, f64, f64, f64)> = sqlx::query_as(
            "SELECT id, kind, name, name_ja, country_code, gn_code, area_km2, center_lat, center_lng FROM geo_regions",
        )
        .fetch_all(&self.core.db)
        .await?;
        let world_area_km2: f64 =
            sqlx::query_scalar::<_, String>("SELECT value FROM geo_meta WHERE key = 'world_area_km2'")
                .fetch_optional(&self.core.db)
                .await?
                .and_then(|value| value.parse().ok())
                .unwrap_or_else(|| rows.iter().filter(|row| row.1 as i32 == KIND_COUNTRY).map(|row| row.6).sum());
        let mut index = RegionIndex { world_area_km2, ..Default::default() };
        for (id, kind, name, name_ja, country_code, gn_code, area_km2, lat, lng) in rows {
            if !gn_code.is_empty() {
                index.by_gn.insert(gn_code, id.clone());
            }
            index.by_id.insert(
                id.clone(),
                RegionMeta { id, kind: kind as i32, name, name_ja, country_code, area_km2, center: (lat, lng) },
            );
        }
        let index = Arc::new(index);
        // An empty table means "not seeded yet": look again next time.
        if !index.by_id.is_empty() {
            *self.regions.index.write() = Some(index.clone());
        }
        Ok(index)
    }

    /// The country and subdivision containing each point, in the order given.
    /// `cells` are `(H3 index, lat, lng)` of cell centers; results are cached per cell.
    pub(crate) async fn regions_of(&self, cells: &[(i64, f64, f64)]) -> AppResult<Vec<RegionRef>> {
        let mut found: HashMap<i64, RegionRef> = HashMap::with_capacity(cells.len());
        let mut unknown = Vec::new();
        for (cell, lat, lng) in cells {
            match self.regions.cells.get(cell) {
                Some(region) => {
                    found.insert(*cell, region);
                }
                None => unknown.push((*cell, *lat, *lng)),
            }
        }
        if !unknown.is_empty() {
            let index = self.region_index().await?;
            let (ids, lats, lngs): (Vec<i64>, Vec<f64>, Vec<f64>) = unzip3(&unknown);
            let rows: Vec<(i64, String, String)> = sqlx::query_as(
                "SELECT t.cell,
                        COALESCE(
                            (SELECT r.id FROM geo_regions r WHERE r.kind = 2 AND ST_Covers(r.geom, t.pt) ORDER BY r.area_km2 LIMIT 1),
                            (SELECT r.id FROM geo_regions r WHERE r.kind = 2 AND ST_DWithin(r.geom, t.pt, $4)
                             ORDER BY ST_Distance(r.geom, t.pt) LIMIT 1),
                            ''),
                        COALESCE(
                            (SELECT r.id FROM geo_regions r WHERE r.kind = 3 AND ST_Covers(r.geom, t.pt) ORDER BY r.area_km2 LIMIT 1),
                            (SELECT r.id FROM geo_regions r WHERE r.kind = 3 AND ST_DWithin(r.geom, t.pt, $4)
                             ORDER BY ST_Distance(r.geom, t.pt) LIMIT 1),
                            '')
                 FROM (SELECT u.cell, ST_SetSRID(ST_MakePoint(u.lng, u.lat), 4326) AS pt
                       FROM UNNEST($1::bigint[], $2::float8[], $3::float8[]) AS u(cell, lat, lng)) t",
            )
            .bind(&ids)
            .bind(&lats)
            .bind(&lngs)
            .bind(COAST_TOLERANCE_DEG)
            .fetch_all(&self.core.db)
            .await?;
            let positions: HashMap<i64, Point> = unknown.iter().map(|(cell, lat, lng)| (*cell, (*lat, *lng))).collect();
            for (cell, country_id, admin1_id) in rows {
                let mut region = RegionRef { country_id, admin1_id };
                if region.country_id.is_empty() || region.admin1_id.is_empty() {
                    if let Some((lat, lng)) = positions.get(&cell) {
                        fill_from_gazetteer(&mut region, &index, *lat, *lng);
                    }
                }
                self.regions.cells.insert(cell, region.clone());
                found.insert(cell, region);
            }
        }
        Ok(cells.iter().map(|(cell, _, _)| found.get(cell).cloned().unwrap_or_default()).collect())
    }

    /// The regions containing one point.
    pub(crate) async fn regions_at(&self, lat: f64, lng: f64) -> AppResult<RegionRef> {
        let Some(cell) = math::cell_at(lat, lng, math::CELL_RESOLUTION) else { return Ok(RegionRef::default()) };
        let center = math::cell_center(cell);
        Ok(self.regions_of(&[(math::cell_to_db(cell), center.0, center.1)]).await?.pop().unwrap_or_default())
    }
}

fn unzip3(items: &[(i64, f64, f64)]) -> (Vec<i64>, Vec<f64>, Vec<f64>) {
    let mut out = (Vec::with_capacity(items.len()), Vec::with_capacity(items.len()), Vec::with_capacity(items.len()));
    for (a, b, c) in items {
        out.0.push(*a);
        out.1.push(*b);
        out.2.push(*c);
    }
    out
}

/// Fills in what the boundaries missed (small islands) from the nearest
/// populated place, and keeps country and subdivision consistent.
fn fill_from_gazetteer(region: &mut RegionRef, index: &RegionIndex, lat: f64, lng: f64) {
    if region.country_id.is_empty() && !region.admin1_id.is_empty() {
        if let Some(admin1) = index.get(&region.admin1_id) {
            if index.by_id.contains_key(&admin1.country_code) {
                region.country_id = admin1.country_code.clone();
            }
        }
    }
    if !region.country_id.is_empty() && !region.admin1_id.is_empty() {
        return;
    }
    let Some(place) = Gazetteer::shared().nearest(lat, lng).filter(|place| place.distance_m <= ISLAND_TOLERANCE_M)
    else {
        return;
    };
    if region.country_id.is_empty()
        && index.get(&place.country_code).is_some_and(|country| country.kind == KIND_COUNTRY)
    {
        region.country_id = place.country_code.clone();
    }
    if region.admin1_id.is_empty() && region.country_id == place.country_code {
        if let Some(id) = index.by_gn.get(&place.admin1_key) {
            region.admin1_id = id.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_regions_have_a_stable_version() {
        let version = data_version();
        assert_eq!(version, data_version());
        assert!(version.starts_with(&format!("{}-", REGIONS_GZ.len())));
    }

    #[test]
    fn the_bundled_regions_parse() {
        let mut text = String::new();
        flate2::read::GzDecoder::new(REGIONS_GZ).read_to_string(&mut text).unwrap();
        let regions: Vec<SeedRegion> = text.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        let countries = regions.iter().filter(|r| r.kind == "country").count();
        assert!((200..300).contains(&countries), "{countries}");
        assert!(regions.len() > 4000);
        let tokyo = regions.iter().find(|r| r.id == "JP-13").expect("Tokyo");
        assert_eq!(
            (tokyo.name.as_str(), tokyo.name_ja.as_str(), tokyo.cc.as_str(), tokyo.gn.as_str()),
            ("Tokyo", "東京都", "JP", "JP.40")
        );
        let japan = regions.iter().find(|r| r.id == "JP").expect("Japan");
        assert_eq!((japan.kind.as_str(), japan.name_ja.as_str()), ("country", "日本"));
        assert!(
            regions.iter().all(|r| !r.id.is_empty() && !r.name.is_empty() && r.geometry.get("coordinates").is_some())
        );
    }

    fn meta(id: &str, kind: i32, country: &str) -> RegionMeta {
        RegionMeta {
            id: id.into(),
            kind,
            name: id.into(),
            name_ja: String::new(),
            country_code: country.into(),
            area_km2: 1.0,
            center: (0.0, 0.0),
        }
    }

    #[test]
    fn islands_are_attributed_through_the_gazetteer() {
        let mut index = RegionIndex::default();
        index.by_id.insert("JP".into(), meta("JP", KIND_COUNTRY, "JP"));
        index.by_id.insert("JP-13".into(), meta("JP-13", KIND_ADMIN1, "JP"));
        index.by_gn.insert("JP.40".into(), "JP-13".into());
        // Chichijima (Ogasawara, Tokyo), 1,000 km south of the mainland.
        let mut region = RegionRef::default();
        fill_from_gazetteer(&mut region, &index, 27.094, 142.192);
        assert_eq!((region.country_id.as_str(), region.admin1_id.as_str()), ("JP", "JP-13"));
        // The open ocean stays unattributed.
        let mut ocean = RegionRef::default();
        fill_from_gazetteer(&mut ocean, &index, 0.0, -140.0);
        assert_eq!(ocean, RegionRef::default());
        // A subdivision alone implies its country.
        let mut partial = RegionRef { country_id: String::new(), admin1_id: "JP-13".into() };
        fill_from_gazetteer(&mut partial, &index, 0.0, -140.0);
        assert_eq!(partial.country_id, "JP");
    }

    #[test]
    fn region_messages_carry_the_local_name_only_when_asked() {
        let tokyo =
            RegionMeta { name: "Tokyo".into(), name_ja: "東京都".into(), ..meta("JP-13", KIND_ADMIN1, "JP") };
        assert_eq!(tokyo.to_proto(true).local_name, "東京都");
        assert_eq!(tokyo.to_proto(false).local_name, "");
        assert_eq!((tokyo.display_name(true), tokyo.display_name(false)), ("東京都", "Tokyo"));
        assert_eq!(meta("X", KIND_COUNTRY, "X").display_name(true), "X", "falls back to the English name");
        let world = RegionIndex { world_area_km2: 1.5e8, ..Default::default() }.world(true);
        assert_eq!((world.id.as_str(), world.local_name.as_str(), world.area_km2), (WORLD_ID, "世界", 1.5e8));
    }
}
