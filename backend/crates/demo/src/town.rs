//! The place the demo happens in: a circle on the map, with streets.
//!
//! The simulator knows no real street. It lays a grid of blocks over the
//! region and keeps the bots on its lines, so on the map they turn corners
//! like people instead of drifting across roofs. (When the server has a
//! routing engine, the runner replaces a leg with the engine's route: see
//! `Walker::follow`.)
use crate::{lines::SPOTS, rng::Rng};

/// A WGS84 coordinate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatLng {
    pub lat: f64,
    pub lng: f64,
}

/// A position in meters east and north of the region's center.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub east: f64,
    pub north: f64,
}

impl Point {
    pub fn distance(self, other: Point) -> f64 {
        (self.east - other.east).hypot(self.north - other.north)
    }

    /// The point `fraction` of the way to `other`.
    pub fn towards(self, other: Point, fraction: f64) -> Point {
        Point {
            east: self.east + (other.east - self.east) * fraction,
            north: self.north + (other.north - self.north) * fraction,
        }
    }

    /// Compass direction to `other`, in degrees clockwise from north.
    pub fn heading(self, other: Point) -> f64 {
        (other.east - self.east).atan2(other.north - self.north).to_degrees().rem_euclid(360.0)
    }
}

/// Where the demo happens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region {
    pub center: LatLng,
    pub radius_m: f64,
}

const METERS_PER_DEGREE: f64 = 111_320.0;

impl Region {
    pub fn locate(&self, point: Point) -> LatLng {
        LatLng {
            lat: self.center.lat + point.north / METERS_PER_DEGREE,
            lng: self.center.lng + point.east / (METERS_PER_DEGREE * self.center.lat.to_radians().cos()),
        }
    }

    pub fn place(&self, at: LatLng) -> Point {
        Point {
            east: (at.lng - self.center.lng) * METERS_PER_DEGREE * self.center.lat.to_radians().cos(),
            north: (at.lat - self.center.lat) * METERS_PER_DEGREE,
        }
    }

    pub fn contains(&self, point: Point) -> bool {
        point.east.hypot(point.north) <= self.radius_m
    }
}

/// A street corner: column and row of the grid, the center being `(0, 0)`.
pub type Corner = (i32, i32);

/// The region with its streets and the places the bots go to.
#[derive(Debug, Clone)]
pub struct Town {
    pub region: Region,
    /// Length of a block.
    block_m: f64,
    /// How far the grid is turned away from north, in radians.
    turn: f64,
    /// Corners per direction from the center.
    reach: i32,
    /// Where each of the first spots of [`SPOTS`] is.
    spots: Vec<Corner>,
}

impl Town {
    pub fn new(region: Region, rng: &Rng) -> Self {
        let mut rng = rng.fork("town");
        let block_m = (region.radius_m / 8.0).clamp(40.0, 120.0);
        // The whole grid fits in the circle however it is turned.
        let reach = ((region.radius_m / block_m / std::f64::consts::SQRT_2).floor() as i32).max(1);
        let turn = rng.between(-0.6, 0.6);
        let mut corners: Vec<Corner> =
            (-reach..=reach).flat_map(|column| (-reach..=reach).map(move |row| (column, row))).collect();
        rng.shuffle(&mut corners);
        corners.truncate(SPOTS.len());
        Self { region, block_m, turn, reach, spots: corners }
    }

    pub fn spot_count(&self) -> usize {
        self.spots.len()
    }

    pub fn spot_corner(&self, spot: usize) -> Corner {
        self.spots[spot]
    }

    pub fn point(&self, (column, row): Corner) -> Point {
        let (x, y) = (f64::from(column) * self.block_m, f64::from(row) * self.block_m);
        let (sin, cos) = self.turn.sin_cos();
        Point { east: x * cos - y * sin, north: x * sin + y * cos }
    }

    /// A way from one corner to another along the streets: at most two
    /// turns, taken at a street chosen by `rng`.
    pub fn route(&self, from: Corner, to: Corner, rng: &mut Rng) -> Vec<Point> {
        let low = from.0.min(to.0);
        let turn_at = low + rng.below((from.0.max(to.0) - low + 1) as usize) as i32;
        let mut corners = vec![from, (turn_at, from.1), (turn_at, to.1), to];
        corners.dedup();
        debug_assert!(corners.iter().all(|corner| corner.0.abs() <= self.reach && corner.1.abs() <= self.reach));
        corners.into_iter().map(|corner| self.point(corner)).collect()
    }
}

/// Length of a path in meters.
pub fn length(path: &[Point]) -> f64 {
    path.windows(2).map(|pair| pair[0].distance(pair[1])).sum()
}

/// The point `meters` along `path`, and the direction of travel there.
/// Past the end it is the end; a path of one point has no direction.
pub fn along(path: &[Point], meters: f64) -> (Point, Option<f64>) {
    let mut left = meters.max(0.0);
    let mut heading = None;
    for pair in path.windows(2) {
        let stretch = pair[0].distance(pair[1]);
        if stretch > 0.0 {
            heading = Some(pair[0].heading(pair[1]));
            if left <= stretch {
                return (pair[0].towards(pair[1], left / stretch), heading);
            }
            left -= stretch;
        }
    }
    (*path.last().expect("a path has a point"), heading)
}

/// Decodes an encoded polyline of precision 5 (what `GetDirections`
/// returns). `None` for text that is not one.
pub fn decode_polyline(encoded: &str) -> Option<Vec<LatLng>> {
    let mut bytes = encoded.bytes();
    let mut number = || -> Option<Option<i64>> {
        let (mut value, mut shift) = (0i64, 0u32);
        loop {
            let Some(byte) = bytes.next() else { return (shift == 0).then_some(None) };
            let chunk = i64::from(byte.checked_sub(63)?);
            if chunk > 63 || shift > 30 {
                return None;
            }
            value |= (chunk & 0x1f) << shift;
            shift += 5;
            if chunk < 0x20 {
                return Some(Some(if value & 1 == 1 { !(value >> 1) } else { value >> 1 }));
            }
        }
    };
    let (mut lat, mut lng) = (0i64, 0i64);
    let mut points = Vec::new();
    while let Some(delta) = number()? {
        lat += delta;
        lng += number()??;
        points.push(LatLng { lat: lat as f64 / 1e5, lng: lng as f64 / 1e5 });
    }
    Some(points)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn town(seed: u64) -> Town {
        Town::new(Region { center: LatLng { lat: 35.7, lng: 139.58 }, radius_m: 900.0 }, &Rng::new(seed))
    }

    #[test]
    fn positions_convert_both_ways() {
        let region = town(1).region;
        let point = Point { east: 310.0, north: -120.5 };
        let back = region.place(region.locate(point));
        assert!(point.distance(back) < 0.01, "{back:?}");
    }

    #[test]
    fn every_spot_is_inside_the_region_and_on_a_corner_of_its_own() {
        for seed in 0..20 {
            let town = town(seed);
            assert_eq!(town.spot_count(), SPOTS.len());
            for spot in 0..town.spot_count() {
                assert!(town.region.contains(town.point(town.spot_corner(spot))), "seed {seed} spot {spot}");
                assert!((0..spot).all(|other| town.spot_corner(other) != town.spot_corner(spot)));
            }
        }
    }

    #[test]
    fn a_small_region_still_has_streets() {
        let tiny = Town::new(Region { center: LatLng { lat: 0.0, lng: 0.0 }, radius_m: 60.0 }, &Rng::new(3));
        assert!(tiny.spot_count() >= 2);
        assert!((0..tiny.spot_count()).all(|spot| tiny.region.contains(tiny.point(tiny.spot_corner(spot)))));
    }

    #[test]
    fn a_route_follows_the_streets_from_door_to_door() {
        let town = town(5);
        let mut rng = Rng::new(9);
        for _ in 0..200 {
            let (from, to) =
                (town.spot_corner(rng.below(town.spot_count())), town.spot_corner(rng.below(town.spot_count())));
            let path = town.route(from, to, &mut rng);
            assert_eq!((path[0], *path.last().unwrap()), (town.point(from), town.point(to)));
            // Along the grid: as long as the blocks between the two corners.
            let blocks = f64::from((from.0 - to.0).abs() + (from.1 - to.1).abs());
            assert!((length(&path) - blocks * town.block_m).abs() < 0.01);
            assert!(path.iter().all(|point| town.region.contains(*point)));
        }
    }

    #[test]
    fn walking_along_a_path_turns_its_corners() {
        let path =
            [Point { east: 0.0, north: 0.0 }, Point { east: 0.0, north: 100.0 }, Point { east: 50.0, north: 100.0 }];
        assert_eq!(along(&path, 40.0), (Point { east: 0.0, north: 40.0 }, Some(0.0)));
        assert_eq!(along(&path, 120.0), (Point { east: 20.0, north: 100.0 }, Some(90.0)));
        assert_eq!(along(&path, 999.0).0, path[2]);
        assert_eq!(along(&path[..1], 5.0), (path[0], None));
    }

    #[test]
    fn polylines_decode() {
        // The example of the format's documentation.
        let points = decode_polyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@").unwrap();
        assert_eq!(
            points,
            [
                LatLng { lat: 38.5, lng: -120.2 },
                LatLng { lat: 40.7, lng: -120.95 },
                LatLng { lat: 43.252, lng: -126.453 }
            ]
        );
        assert_eq!(decode_polyline(""), Some(vec![]));
        assert_eq!(decode_polyline("_p~iF"), None);
        assert_eq!(decode_polyline("日本語"), None);
    }
}
