//! Offline reverse geocoding, place-name search and time zones.
//!
//! The gazetteer bundled in `data/` (GeoNames, see `data/README.md`) is loaded
//! once per process into a grid index. Lookups are in-memory and take
//! microseconds, so the pipeline can name the area of every pin, and a blurred
//! pin can be named after where it is shown rather than where its owner is.
//! Time zones come from `tzf-rs`.
use std::{collections::HashMap, io::Read as _, sync::OnceLock};

use tokuly_proto::geo::GeocodeResult;

use crate::{
    math::{self, Point},
    state::Area,
};

static GAZETTEER_GZ: &[u8] = include_bytes!("../data/gazetteer.tsv.gz");
static ADMIN1_GZ: &[u8] = include_bytes!("../data/admin1.tsv.gz");
static COUNTRIES_TSV: &str = include_str!("../data/countries.tsv");

/// Grid cells are a quarter of a degree (about 28 km north–south).
const GRID: f64 = 4.0;
/// Search radii tried in turn: the neighbourhood, the surroundings, the region.
const SEARCH_KM: [f64; 3] = [6.0, 30.0, 100.0];

/// A populated place. Strings are indexes into the gazetteer's string table.
#[derive(Debug, Clone, Copy)]
struct Place {
    lat: f32,
    lng: f32,
    country: [u8; 2],
    /// Index into `admin1_keys` (`u32::MAX` = none).
    admin1: u32,
    name: u32,
    name_ja: u32,
    /// Municipality in English and Japanese (empty string = not known).
    area: u32,
    area_ja: u32,
    population: u32,
}

pub struct Gazetteer {
    places: Vec<Place>,
    /// Places per grid cell, as index ranges into `places` (sorted by cell).
    grid: HashMap<(i16, i16), (u32, u32)>,
    strings: Vec<Box<str>>,
    /// GeoNames admin1 keys (`JP.40`) and their names.
    admin1_keys: Vec<Box<str>>,
    admin1_names: Vec<(Box<str>, Box<str>)>,
    countries: HashMap<[u8; 2], (Box<str>, Box<str>)>,
    time_zones: tzf_rs::DefaultFinder,
}

/// A search hit or reverse-geocoding match.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub position: Point,
    pub name: String,
    pub name_ja: String,
    /// Municipality (Japan) in English and Japanese; empty elsewhere.
    pub area: String,
    pub area_ja: String,
    pub region: String,
    pub region_ja: String,
    pub country: String,
    pub country_ja: String,
    pub country_code: String,
    /// GeoNames admin1 key, e.g. `JP.40` (empty if unknown).
    pub admin1_key: String,
    pub population: u32,
    /// Distance from the query point, for reverse lookups.
    pub distance_m: f64,
}

impl Found {
    /// The locality: the municipality where known, else the place itself.
    pub fn locality(&self, ja: bool) -> &str {
        match (ja, self.area.is_empty()) {
            (true, false) if !self.area_ja.is_empty() => &self.area_ja,
            (_, false) => &self.area,
            (true, true) if !self.name_ja.is_empty() => &self.name_ja,
            _ => &self.name,
        }
    }

    pub fn region(&self, ja: bool) -> &str {
        if ja && !self.region_ja.is_empty() { &self.region_ja } else { &self.region }
    }

    pub fn country(&self, ja: bool) -> &str {
        if ja && !self.country_ja.is_empty() { &self.country_ja } else { &self.country }
    }

    /// "Shibuya, Tokyo, Japan" / 「東京都渋谷区」 style one-liner.
    pub fn address(&self, ja: bool) -> String {
        let (locality, region, country) = (self.locality(ja), self.region(ja), self.country(ja));
        if ja {
            let mut out = String::new();
            if self.country_code != "JP" {
                out.push_str(country);
                if !country.is_empty() && !region.is_empty() {
                    out.push(' ');
                }
            }
            out.push_str(region);
            if locality != region {
                out.push_str(locality);
            }
            out
        } else {
            let mut parts: Vec<&str> = Vec::new();
            for part in [locality, region, country] {
                // "Kita, Osaka" already names its region.
                let repeated = parts.iter().any(|seen| *seen == part || seen.ends_with(&format!(", {part}")));
                if !part.is_empty() && !repeated {
                    parts.push(part);
                }
            }
            parts.join(", ")
        }
    }
}

fn gunzip(bytes: &[u8]) -> String {
    let mut text = String::new();
    flate2::read::GzDecoder::new(bytes).read_to_string(&mut text).expect("bundled gazetteer data is valid gzip");
    text
}

fn cell_of(lat: f64, lng: f64) -> (i16, i16) {
    ((lat * GRID).floor() as i16, (lng * GRID).floor() as i16)
}

/// Case-insensitive (ASCII) substring search; `needle` must be lowercase.
fn find_ignore_case(haystack: &str, needle: &str) -> Option<usize> {
    let (haystack, needle) = (haystack.as_bytes(), needle.as_bytes());
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&start| haystack[start..start + needle.len()].iter().zip(needle).all(|(h, n)| h.to_ascii_lowercase() == *n))
}

/// Latin letters without their accents, lowercased ("Kyōto" → "kyoto"), so
/// that plain-ASCII queries find accented names.
fn fold(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            'À'..='Å' | 'à'..='å' | 'Ā' | 'ā' | 'Ă' | 'ă' | 'Ą' | 'ą' => 'a',
            'È'..='Ë' | 'è'..='ë' | 'Ē' | 'ē' | 'Ė' | 'ė' | 'Ę' | 'ę' | 'Ě' | 'ě' => 'e',
            'Ì'..='Ï' | 'ì'..='ï' | 'Ī' | 'ī' | 'İ' | 'ı' => 'i',
            'Ò'..='Ö' | 'ò'..='ö' | 'Ø' | 'ø' | 'Ō' | 'ō' | 'Ő' | 'ő' => 'o',
            'Ù'..='Ü' | 'ù'..='ü' | 'Ū' | 'ū' | 'Ů' | 'ů' | 'Ű' | 'ű' => 'u',
            'Ç' | 'ç' | 'Ć' | 'ć' | 'Č' | 'č' => 'c',
            'Ñ' | 'ñ' | 'Ń' | 'ń' | 'Ň' | 'ň' => 'n',
            'Ś' | 'ś' | 'Š' | 'š' | 'Ş' | 'ş' => 's',
            'Ž' | 'ž' | 'Ź' | 'ź' | 'Ż' | 'ż' => 'z',
            'Ý' | 'ý' | 'ÿ' => 'y',
            'Ł' | 'ł' => 'l',
            'Ğ' | 'ğ' => 'g',
            '’' | '\'' | '-' => ' ',
            other => other.to_ascii_lowercase(),
        })
        .collect()
}

impl Gazetteer {
    /// The process-wide gazetteer, loaded on first use (a few hundred milliseconds).
    pub fn shared() -> &'static Gazetteer {
        static SHARED: OnceLock<Gazetteer> = OnceLock::new();
        SHARED.get_or_init(Gazetteer::load)
    }

    fn load() -> Self {
        let started = std::time::Instant::now();
        let mut strings: Vec<Box<str>> = vec!["".into()];
        let mut interned: HashMap<String, u32> = HashMap::from([(String::new(), 0)]);
        let mut intern = |text: &str| -> u32 {
            if let Some(index) = interned.get(text) {
                return *index;
            }
            let index = strings.len() as u32;
            strings.push(text.into());
            interned.insert(text.to_owned(), index);
            index
        };

        let mut admin1_keys: Vec<Box<str>> = Vec::new();
        let mut admin1_names = Vec::new();
        let mut admin1_index: HashMap<String, u32> = HashMap::new();
        for line in gunzip(ADMIN1_GZ).lines() {
            let mut columns = line.split('\t');
            let (Some(key), Some(name)) = (columns.next(), columns.next()) else { continue };
            admin1_index.insert(key.to_owned(), admin1_keys.len() as u32);
            admin1_keys.push(key.into());
            admin1_names.push((name.into(), columns.next().unwrap_or_default().into()));
        }

        let mut countries = HashMap::new();
        for line in COUNTRIES_TSV.lines() {
            let mut columns = line.split('\t');
            let (Some(code), Some(name)) = (columns.next(), columns.next()) else { continue };
            if let [a, b] = code.as_bytes() {
                countries.insert([*a, *b], (name.into(), columns.next().unwrap_or_default().into()));
            }
        }

        let mut places = Vec::with_capacity(240_000);
        for line in gunzip(GAZETTEER_GZ).lines() {
            let columns: Vec<&str> = line.split('\t').collect();
            let [lat, lng, country, admin1, name, name_ja, population, area, area_ja] = columns[..] else { continue };
            let (Ok(lat), Ok(lng), [a, b]) = (lat.parse::<f32>(), lng.parse::<f32>(), country.as_bytes()) else { continue };
            places.push(Place {
                lat,
                lng,
                country: [*a, *b],
                admin1: admin1_index.get(&format!("{country}.{admin1}")).copied().unwrap_or(u32::MAX),
                name: intern(name),
                name_ja: intern(name_ja),
                area: intern(area),
                area_ja: intern(area_ja),
                population: population.parse().unwrap_or(0),
            });
        }
        places.sort_by_key(|place| cell_of(place.lat as f64, place.lng as f64));
        let mut grid: HashMap<(i16, i16), (u32, u32)> = HashMap::new();
        for (index, place) in places.iter().enumerate() {
            let range = grid.entry(cell_of(place.lat as f64, place.lng as f64)).or_insert((index as u32, index as u32));
            range.1 = index as u32 + 1;
        }
        let time_zones = tzf_rs::DefaultFinder::new();
        tracing::info!(places = places.len(), elapsed_ms = started.elapsed().as_millis() as u64, "gazetteer loaded");
        Self { places, grid, strings, admin1_keys, admin1_names, countries, time_zones }
    }

    pub fn len(&self) -> usize {
        self.places.len()
    }

    pub fn is_empty(&self) -> bool {
        self.places.is_empty()
    }

    fn text(&self, index: u32) -> &str {
        &self.strings[index as usize]
    }

    fn found(&self, place: &Place, distance_m: f64) -> Found {
        let (region, region_ja, admin1_key) = match self.admin1_names.get(place.admin1 as usize) {
            Some((name, name_ja)) => (name.to_string(), name_ja.to_string(), self.admin1_keys[place.admin1 as usize].to_string()),
            None => Default::default(),
        };
        let (country, country_ja) = self.countries.get(&place.country).map(|(en, ja)| (en.to_string(), ja.to_string())).unwrap_or_default();
        Found {
            position: (place.lat as f64, place.lng as f64),
            name: self.text(place.name).to_owned(),
            name_ja: self.text(place.name_ja).to_owned(),
            area: self.text(place.area).to_owned(),
            area_ja: self.text(place.area_ja).to_owned(),
            region,
            region_ja,
            country,
            country_ja,
            country_code: String::from_utf8_lossy(&place.country).into_owned(),
            admin1_key,
            population: place.population,
            distance_m,
        }
    }

    /// Places within `radius_km` of a point, with their distance in meters.
    fn within(&self, lat: f64, lng: f64, radius_km: f64) -> Vec<(&Place, f64)> {
        let lat_span = radius_km / 111.2;
        let lng_span = (radius_km / (111.2 * lat.to_radians().cos().max(0.05))).min(180.0);
        let (south, north) = (((lat - lat_span) * GRID).floor() as i32, ((lat + lat_span) * GRID).floor() as i32);
        let (west, east) = (((lng - lng_span) * GRID).floor() as i32, ((lng + lng_span) * GRID).floor() as i32);
        let columns = (360.0 * GRID) as i32;
        let mut hits = Vec::new();
        for row in south..=north {
            for column in west..=east {
                // Columns wrap around the antimeridian.
                let wrapped = (column + columns / 2).rem_euclid(columns) - columns / 2;
                let Some((start, end)) = self.grid.get(&(row as i16, wrapped as i16)) else { continue };
                for place in &self.places[*start as usize..*end as usize] {
                    let distance = math::distance_meters(lat, lng, place.lat as f64, place.lng as f64);
                    if distance <= radius_km * 1000.0 {
                        hits.push((place, distance));
                    }
                }
            }
        }
        hits
    }

    /// The populated place that best describes a point, if one is within 100 km.
    ///
    /// In Japan that is simply the nearest place (the gazetteer is dense there
    /// and every place knows its municipality). Elsewhere a nearby city wins
    /// over a slightly nearer village: places are ranked by distance scaled
    /// down by the fourth root of their population.
    pub fn nearest(&self, lat: f64, lng: f64) -> Option<Found> {
        if !math::valid_coordinates(lat, lng) {
            return None;
        }
        for radius_km in SEARCH_KM {
            let hits = self.within(lat, lng, radius_km);
            let Some(closest) = hits.iter().min_by(|a, b| a.1.total_cmp(&b.1)) else { continue };
            let dense = closest.0.area != 0;
            let best = if dense {
                closest
            } else {
                let score = |(place, distance): &(&Place, f64)| (distance / 1000.0 + 0.3) / (place.population.max(500) as f64).powf(0.25);
                hits.iter().min_by(|a, b| score(a).total_cmp(&score(b))).unwrap_or(closest)
            };
            return Some(self.found(best.0, best.1));
        }
        None
    }

    /// IANA time zone at a point (`Etc/GMT±N` at sea).
    pub fn time_zone(&self, lat: f64, lng: f64) -> String {
        if !math::valid_coordinates(lat, lng) {
            return String::new();
        }
        self.time_zones.get_tz_name(lng, lat).to_owned()
    }

    /// The area around a point in both languages, for a pin.
    pub fn area(&self, lat: f64, lng: f64) -> Area {
        let time_zone = self.time_zone(lat, lng);
        match self.nearest(lat, lng) {
            Some(found) => Area {
                locality: found.locality(false).to_owned(),
                locality_ja: found.locality(true).to_owned(),
                region: found.region,
                region_ja: found.region_ja,
                country_code: found.country_code,
                time_zone,
            },
            None => Area { time_zone, ..Default::default() },
        }
    }

    /// Reverse geocoding for the API, in one language.
    pub fn reverse(&self, lat: f64, lng: f64, ja: bool) -> GeocodeResult {
        let time_zone = self.time_zone(lat, lng);
        let Some(found) = self.nearest(lat, lng) else {
            return GeocodeResult { time_zone, ..Default::default() };
        };
        let locality = found.locality(ja).to_owned();
        GeocodeResult {
            name: locality.clone(),
            locality,
            region: found.region(ja).to_owned(),
            country: found.country(ja).to_owned(),
            country_code: found.country_code,
            time_zone,
            ..Default::default()
        }
    }

    pub fn country_name(&self, code: &str, ja: bool) -> String {
        let Some((en, name_ja)) = code.as_bytes().try_into().ok().and_then(|key: [u8; 2]| self.countries.get(&key)) else {
            return String::new();
        };
        if ja && !name_ja.is_empty() { name_ja.to_string() } else { en.to_string() }
    }

    /// Finds places by name (English, accent-insensitive, or Japanese).
    /// Exact matches come first, then prefixes, then anything containing the
    /// query; bigger places first within each group, places close to `near`
    /// preferred.
    pub fn search(&self, query: &str, near: Option<Point>, limit: usize) -> Vec<Found> {
        let query = query.trim();
        if query.is_empty() || limit == 0 {
            return Vec::new();
        }
        let folded = fold(query);
        let ascii = folded.is_ascii();
        let mut scored: Vec<(f64, &Place)> = Vec::new();
        for place in &self.places {
            // 0 = exact, 1 = prefix, 2 = contains.
            let rank = if ascii {
                let name = self.text(place.name);
                let hit = if name.is_ascii() { find_ignore_case(name, &folded) } else { fold(name).find(&folded) };
                let area = self.text(place.area);
                match hit {
                    Some(0) if name.chars().count() == folded.chars().count() => 0,
                    Some(0) => 1,
                    Some(_) => 2,
                    None if find_ignore_case(area, &folded) == Some(0) => 2,
                    None => continue,
                }
            } else {
                let (name_ja, area_ja) = (self.text(place.name_ja), self.text(place.area_ja));
                if name_ja == query {
                    0
                } else if name_ja.starts_with(query) {
                    1
                } else if name_ja.contains(query) || area_ja.contains(query) {
                    2
                } else {
                    continue;
                }
            };
            // Lower is better: match quality first, then size, then proximity.
            let size = (place.population.max(1) as f64).log10();
            let mut score = rank as f64 * 10.0 - size;
            if let Some((lat, lng)) = near {
                let km = math::distance_meters(lat, lng, place.lat as f64, place.lng as f64) / 1000.0;
                score += (km / 100.0).min(4.0);
            }
            scored.push((score, place));
        }
        scored.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut out: Vec<Found> = Vec::new();
        for (_, place) in scored {
            let distance = near.map_or(0.0, |(lat, lng)| math::distance_meters(lat, lng, place.lat as f64, place.lng as f64));
            let found = self.found(place, distance);
            // The same town often appears as several nearby entries.
            let duplicate = out.iter().any(|seen| seen.name == found.name && seen.admin1_key == found.admin1_key && seen.area == found.area);
            if !duplicate {
                out.push(found);
                if out.len() >= limit {
                    break;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_gazetteer_loads() {
        let gazetteer = Gazetteer::shared();
        assert!(gazetteer.len() > 200_000, "{}", gazetteer.len());
        assert!(!gazetteer.is_empty());
        assert_eq!(gazetteer.country_name("JP", true), "日本");
        assert_eq!(gazetteer.country_name("JP", false), "Japan");
        assert_eq!(gazetteer.country_name("??", false), "");
    }

    #[test]
    fn shibuya_crossing_is_in_shibuya_ward() {
        let gazetteer = Gazetteer::shared();
        let ja = gazetteer.reverse(35.6595, 139.7005, true);
        assert_eq!(ja.name, "渋谷区");
        assert_eq!((ja.locality.as_str(), ja.region.as_str(), ja.country.as_str()), ("渋谷区", "東京都", "日本"));
        assert_eq!((ja.country_code.as_str(), ja.time_zone.as_str()), ("JP", "Asia/Tokyo"));
        let en = gazetteer.reverse(35.6595, 139.7005, false);
        assert_eq!((en.name.as_str(), en.region.as_str(), en.country.as_str()), ("Shibuya", "Tokyo", "Japan"));
        let area = gazetteer.area(35.6595, 139.7005);
        assert_eq!((area.label(true), area.label(false)), ("渋谷区".to_owned(), "Shibuya, Tokyo".to_owned()));
    }

    #[test]
    fn osaka_resolves_to_the_city_and_its_ward() {
        let gazetteer = Gazetteer::shared();
        // Umeda, Kita ward.
        let ja = gazetteer.reverse(34.7025, 135.4959, true);
        assert_eq!(ja.name, "大阪市北区");
        assert_eq!((ja.region.as_str(), ja.country_code.as_str(), ja.time_zone.as_str()), ("大阪府", "JP", "Asia/Tokyo"));
        let en = gazetteer.reverse(34.7025, 135.4959, false);
        assert_eq!((en.name.as_str(), en.region.as_str()), ("Kita, Osaka", "Osaka"));
        assert_eq!(gazetteer.area(34.7025, 135.4959).label(false), "Kita, Osaka");
    }

    #[test]
    fn towns_in_districts_resolve_to_the_town() {
        // Okutama, in Nishitama district: the town, not the district.
        let ja = Gazetteer::shared().reverse(35.8090, 139.0960, true);
        assert_eq!(ja.name, "奥多摩町");
    }

    #[test]
    fn new_york_is_new_york_not_a_village_across_the_river() {
        let gazetteer = Gazetteer::shared();
        let city_hall = gazetteer.reverse(40.7128, -74.0060, false);
        assert_eq!(city_hall.name, "New York City");
        assert_eq!((city_hall.region.as_str(), city_hall.country.as_str(), city_hall.country_code.as_str()), ("New York", "United States", "US"));
        assert_eq!(city_hall.time_zone, "America/New_York");
        assert_eq!(gazetteer.reverse(40.7128, -74.0060, true).name, "ニューヨーク");
        // Times Square: Manhattan or New York City, never New Jersey.
        let times_square = gazetteer.reverse(40.7580, -73.9855, false);
        assert_eq!(times_square.region, "New York");
        assert!(["Manhattan", "New York City"].contains(&times_square.name.as_str()), "{}", times_square.name);
    }

    #[test]
    fn mid_ocean_has_a_time_zone_and_nothing_else() {
        let gazetteer = Gazetteer::shared();
        let pacific = gazetteer.reverse(0.0, -140.0, true);
        assert_eq!((pacific.name.as_str(), pacific.country_code.as_str()), ("", ""));
        assert!(pacific.time_zone.starts_with("Etc/GMT"), "{}", pacific.time_zone);
        assert!(gazetteer.nearest(0.0, -140.0).is_none());
        assert_eq!(gazetteer.area(0.0, -140.0).label(true), "");
        // Nonsense coordinates do not panic.
        assert!(gazetteer.nearest(f64::NAN, 0.0).is_none());
        assert_eq!(gazetteer.time_zone(200.0, 0.0), "");
    }

    #[test]
    fn lookups_work_across_the_antimeridian_and_near_the_poles() {
        let gazetteer = Gazetteer::shared();
        // Taveuni, Fiji, sits on the 180th meridian.
        let fiji = gazetteer.nearest(-16.8, 179.99).expect("a place in Fiji");
        assert_eq!(fiji.country_code, "FJ");
        // Longyearbyen, Svalbard (78°N).
        assert_eq!(gazetteer.nearest(78.22, 15.65).unwrap().country_code, "SJ");
    }

    #[test]
    fn search_finds_places_by_name_in_both_languages() {
        let gazetteer = Gazetteer::shared();
        let hits = gazetteer.search("shibuya", None, 5);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].country_code, "JP");
        assert!(hits[0].name.eq_ignore_ascii_case("shibuya"), "{}", hits[0].name);

        let hits = gazetteer.search("渋谷", None, 5);
        assert!(hits.iter().any(|hit| hit.name_ja.starts_with("渋谷") && hit.area_ja == "渋谷区"), "{hits:?}");

        // Accents are optional, big cities first.
        let hits = gazetteer.search("kyoto", None, 3);
        assert_eq!(hits[0].region, "Kyoto");
        let hits = gazetteer.search("paris", None, 3);
        assert_eq!((hits[0].name.as_str(), hits[0].country_code.as_str()), ("Paris", "FR"));
        assert_eq!(hits[0].address(false), "Paris, Île-de-France, France");
    }

    #[test]
    fn search_prefers_places_near_the_bias_point() {
        let gazetteer = Gazetteer::shared();
        // Springfield, Illinois vs. Springfield, Massachusetts.
        let near_boston = gazetteer.search("Springfield", Some((42.36, -71.06)), 1);
        assert_eq!(near_boston[0].region, "Massachusetts");
        let near_chicago = gazetteer.search("Springfield", Some((41.88, -87.63)), 1);
        assert_eq!(near_chicago[0].region, "Illinois");
        assert!(near_boston[0].distance_m > 50_000.0 && near_boston[0].distance_m < 200_000.0);
    }

    #[test]
    fn empty_and_unknown_queries_find_nothing() {
        let gazetteer = Gazetteer::shared();
        assert!(gazetteer.search("  ", None, 5).is_empty());
        assert!(gazetteer.search("zzzzqqqqxxxx", None, 5).is_empty());
        assert!(gazetteer.search("tokyo", None, 0).is_empty());
    }

    #[test]
    fn addresses_read_naturally_in_each_language() {
        let gazetteer = Gazetteer::shared();
        let shibuya = gazetteer.nearest(35.6595, 139.7005).unwrap();
        assert_eq!(shibuya.address(true), "東京都渋谷区");
        assert_eq!(shibuya.address(false), "Shibuya, Tokyo, Japan");
        let new_york = gazetteer.nearest(40.7128, -74.0060).unwrap();
        assert_eq!(new_york.address(false), "New York City, New York, United States");
    }

    #[test]
    fn folding_strips_accents() {
        assert_eq!(fold("Kyōto"), "kyoto");
        assert_eq!(fold("Île-de-France"), "ile de france");
        assert_eq!(fold("渋谷"), "渋谷");
        assert_eq!(find_ignore_case("New York City", "york"), Some(4));
        assert_eq!(find_ignore_case("York", "new york"), None);
    }
}
