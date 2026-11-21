//! Geometry on the sphere, path simplification, polylines and H3 helpers.
//! Everything here is pure.
use h3o::{CellIndex, LatLng as H3LatLng, Resolution};
use tokuly_proto::common::{BoundingBox, LatLng};

pub use tokuly_core::geoindex::{distance_meters, valid_coordinates};

const EARTH_RADIUS_M: f64 = 6_371_008.8;

/// A `(latitude, longitude)` pair in degrees.
pub type Point = (f64, f64);

pub fn lat_lng(lat: f64, lng: f64) -> LatLng {
    LatLng { lat, lng, ..Default::default() }
}

/// Distance in meters between two points.
pub fn distance(a: Point, b: Point) -> f64 {
    distance_meters(a.0, a.1, b.0, b.1)
}

fn normalize_lng(lng: f64) -> f64 {
    let wrapped = (lng + 180.0).rem_euclid(360.0) - 180.0;
    // Keep +180 as it is rather than flipping it to -180.
    if wrapped == -180.0 && lng > 0.0 { 180.0 } else { wrapped }
}

/// The point `distance_m` meters from `(lat, lng)` in direction `bearing_deg`
/// (clockwise from north).
pub fn destination(lat: f64, lng: f64, bearing_deg: f64, distance_m: f64) -> Point {
    let (phi, lambda, theta) = (lat.to_radians(), lng.to_radians(), bearing_deg.to_radians());
    let delta = distance_m / EARTH_RADIUS_M;
    let phi2 = (phi.sin() * delta.cos() + phi.cos() * delta.sin() * theta.cos()).clamp(-1.0, 1.0).asin();
    let lambda2 = lambda + (theta.sin() * delta.sin() * phi.cos()).atan2(delta.cos() - phi.sin() * phi2.sin());
    (phi2.to_degrees().clamp(-90.0, 90.0), normalize_lng(lambda2.to_degrees()))
}

/// Initial bearing from `a` to `b`, degrees clockwise from north in `[0, 360)`.
pub fn bearing(a: Point, b: Point) -> f64 {
    let (phi1, phi2) = (a.0.to_radians(), b.0.to_radians());
    let dl = (b.1 - a.1).to_radians();
    let y = dl.sin() * phi2.cos();
    let x = phi1.cos() * phi2.sin() - phi1.sin() * phi2.cos() * dl.cos();
    y.atan2(x).to_degrees().rem_euclid(360.0)
}

/// The point a `fraction` (0..=1) of the way along the great circle from `a` to `b`.
pub fn intermediate(a: Point, b: Point, fraction: f64) -> Point {
    let total = distance(a, b);
    if total < 1.0 {
        return if fraction < 0.5 { a } else { b };
    }
    destination(a.0, a.1, bearing(a, b), total * fraction.clamp(0.0, 1.0))
}

/// The great circle from `a` to `b` as a line with a vertex at least every
/// `max_segment_m` meters (at most `max_vertices` in total).
pub fn great_circle_line(a: Point, b: Point, max_segment_m: f64, max_vertices: usize) -> Vec<Point> {
    let total = distance(a, b);
    let segments = ((total / max_segment_m.max(1.0)).ceil() as usize).clamp(1, max_vertices.max(2) - 1);
    let mut line = Vec::with_capacity(segments + 1);
    line.push(a);
    for step in 1..segments {
        line.push(intermediate(a, b, step as f64 / segments as f64));
    }
    line.push(b);
    line
}

/// Distance in meters from `p` to the segment `a`–`b` (planar approximation
/// around `a`, fine for the short segments of a recorded path).
fn distance_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let scale = a.0.to_radians().cos();
    let to_xy = |q: Point| ((q.1 - a.1).to_radians() * scale * EARTH_RADIUS_M, (q.0 - a.0).to_radians() * EARTH_RADIUS_M);
    let (px, py) = to_xy(p);
    let (bx, by) = to_xy(b);
    let length_sq = bx * bx + by * by;
    if length_sq < 1e-9 {
        return (px * px + py * py).sqrt();
    }
    let t = ((px * bx + py * by) / length_sq).clamp(0.0, 1.0);
    let (dx, dy) = (px - t * bx, py - t * by);
    (dx * dx + dy * dy).sqrt()
}

/// Douglas–Peucker simplification: drops vertices that deviate less than
/// `tolerance_m` meters from the simplified line. Endpoints are kept.
pub fn simplify(points: &[Point], tolerance_m: f64) -> Vec<Point> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0usize, points.len() - 1)];
    while let Some((first, last)) = stack.pop() {
        if last <= first + 1 {
            continue;
        }
        let (mut farthest, mut worst) = (first, 0.0);
        for index in first + 1..last {
            let deviation = distance_to_segment(points[index], points[first], points[last]);
            if deviation > worst {
                (farthest, worst) = (index, deviation);
            }
        }
        if worst > tolerance_m {
            keep[farthest] = true;
            stack.push((first, farthest));
            stack.push((farthest, last));
        }
    }
    points.iter().zip(keep).filter(|(_, keep)| *keep).map(|(point, _)| *point).collect()
}

/// Picks at most `max` evenly spread indexes out of `len`, always including
/// the first and the last.
pub fn downsample_indexes(len: usize, max: usize) -> Vec<usize> {
    let max = max.max(2);
    if len <= max {
        return (0..len).collect();
    }
    let step = (len - 1) as f64 / (max - 1) as f64;
    let mut indexes: Vec<usize> = (0..max).map(|i| ((i as f64 * step).round() as usize).min(len - 1)).collect();
    indexes.dedup();
    indexes
}

/// Google encoded polyline (precision 5) of a path.
pub fn encode_polyline(points: &[Point]) -> String {
    let coordinates = points.iter().map(|(lat, lng)| geo::Coord { x: *lng, y: *lat });
    polyline::encode_coordinates(coordinates, 5).unwrap_or_default()
}

/// Decodes an encoded polyline of the given precision (Valhalla uses 6).
pub fn decode_polyline(encoded: &str, precision: u32) -> Vec<Point> {
    polyline::decode_polyline(encoded, precision).map(|line| line.coords().map(|c| (c.y, c.x)).collect()).unwrap_or_default()
}

/// The smallest box containing every point (`None` for no points). Paths
/// crossing the antimeridian get a box spanning the whole longitude range,
/// which clients handle better than an inverted one.
pub fn bounding_box(points: impl IntoIterator<Item = Point>) -> Option<BoundingBox> {
    let mut bounds: Option<(f64, f64, f64, f64)> = None;
    for (lat, lng) in points {
        bounds = Some(match bounds {
            None => (lat, lng, lat, lng),
            Some((south, west, north, east)) => (south.min(lat), west.min(lng), north.max(lat), east.max(lng)),
        });
    }
    bounds.map(|(south, west, north, east)| BoundingBox {
        south_west: lat_lng(south, west).into(),
        north_east: lat_lng(north, east).into(),
        ..Default::default()
    })
}

/// Whether `(lat, lng)` lies inside a box given by its corners. A box whose
/// west edge is east of its east edge crosses the antimeridian.
pub fn in_bounds(lat: f64, lng: f64, south_west: Point, north_east: Point) -> bool {
    if lat < south_west.0 || lat > north_east.0 {
        return false;
    }
    if south_west.1 <= north_east.1 { lng >= south_west.1 && lng <= north_east.1 } else { lng >= south_west.1 || lng <= north_east.1 }
}

// ── H3 ──────────────────────────────────────────────────────────────────────

/// Resolution of footprint cells (hexagons of about 0.1 km²).
pub const CELL_RESOLUTION: Resolution = Resolution::Nine;
/// Coarser resolutions materialized for low zoom levels.
pub const PARENT_RESOLUTIONS: [Resolution; 2] = [Resolution::Seven, Resolution::Five];

/// The H3 cell containing a point.
pub fn cell_at(lat: f64, lng: f64, resolution: Resolution) -> Option<CellIndex> {
    H3LatLng::new(lat, lng).ok().map(|position| position.to_cell(resolution))
}

pub fn cell_center(cell: CellIndex) -> Point {
    let center = H3LatLng::from(cell);
    (center.lat(), center.lng())
}

/// H3 indexes are stored as `bigint` (they never use the sign bit).
pub fn cell_to_db(cell: CellIndex) -> i64 {
    u64::from(cell) as i64
}

pub fn cell_from_db(value: i64) -> Option<CellIndex> {
    CellIndex::try_from(value as u64).ok()
}

/// The 15-character hex form clients use.
pub fn cell_hex(value: i64) -> String {
    format!("{:x}", value as u64)
}

/// The cells on the line from `from` to `to`, excluding `from` and including
/// `to`. Falls back to just `to` when the two are too far apart to bridge
/// (`max_cells`) or H3 cannot connect them (pentagon distortion).
pub fn cells_between(from: CellIndex, to: CellIndex, max_cells: usize) -> Vec<CellIndex> {
    if from == to {
        return Vec::new();
    }
    let bridged = match from.grid_path_cells_size(to) {
        Ok(size) if size as usize <= max_cells + 1 => {
            from.grid_path_cells(to).ok().and_then(|path| path.collect::<Result<Vec<_>, _>>().ok())
        }
        _ => None,
    };
    match bridged {
        Some(path) => path.into_iter().filter(|cell| *cell != from).collect(),
        None => vec![to],
    }
}

/// H3 resolution to draw footprints with at a map zoom level.
pub fn resolution_for_zoom(zoom: f64) -> Resolution {
    if !zoom.is_finite() || zoom >= 11.5 {
        Resolution::Nine
    } else if zoom >= 7.5 {
        Resolution::Seven
    } else {
        Resolution::Five
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIBUYA: Point = (35.6595, 139.7005);
    const TOKYO_STATION: Point = (35.681236, 139.767125);

    #[test]
    fn destination_and_bearing_are_inverse() {
        for (bearing_deg, meters) in [(0.0, 500.0), (90.0, 1200.0), (217.0, 300.0), (359.0, 80_000.0)] {
            let target = destination(SHIBUYA.0, SHIBUYA.1, bearing_deg, meters);
            assert!((distance(SHIBUYA, target) - meters).abs() < meters * 0.001 + 0.5, "{bearing_deg} {meters}");
            let back = bearing(SHIBUYA, target);
            let diff = (back - bearing_deg).abs().min(360.0 - (back - bearing_deg).abs());
            assert!(diff < 0.5, "bearing {back} vs {bearing_deg}");
        }
    }

    #[test]
    fn destination_wraps_the_antimeridian() {
        let (_, lng) = destination(0.0, 179.9, 90.0, 50_000.0);
        assert!((-180.0..-179.0).contains(&lng), "{lng}");
    }

    #[test]
    fn great_circle_lines_have_evenly_spaced_vertices() {
        let line = great_circle_line(SHIBUYA, TOKYO_STATION, 1000.0, 64);
        assert_eq!((line[0], *line.last().unwrap()), (SHIBUYA, TOKYO_STATION));
        assert_eq!(line.len(), 8); // 6.4 km in 1 km steps
        for pair in line.windows(2) {
            assert!(distance(pair[0], pair[1]) < 1001.0);
        }
        assert_eq!(great_circle_line(SHIBUYA, SHIBUYA, 1000.0, 64).len(), 2);
        // Capped for very long lines.
        assert_eq!(great_circle_line(SHIBUYA, (40.7128, -74.0060), 1000.0, 32).len(), 32);
    }

    #[test]
    fn simplify_drops_points_on_a_straight_line_and_keeps_corners() {
        // East for 1 km in 10 steps, then north for 1 km in 10 steps.
        let mut path = Vec::new();
        for step in 0..=10 {
            path.push(destination(35.0, 139.0, 90.0, step as f64 * 100.0));
        }
        let corner = *path.last().unwrap();
        for step in 1..=10 {
            path.push(destination(corner.0, corner.1, 0.0, step as f64 * 100.0));
        }
        let simple = simplify(&path, 10.0);
        assert_eq!(simple.len(), 3, "{simple:?}");
        assert_eq!(simple[0], path[0]);
        assert!(distance(simple[1], corner) < 1.0);
        assert_eq!(simple[2], *path.last().unwrap());
        // A zig-zag larger than the tolerance survives.
        let zigzag = vec![(35.0, 139.0), destination(35.0, 139.0, 45.0, 200.0), destination(35.0, 139.0, 90.0, 280.0)];
        assert_eq!(simplify(&zigzag, 10.0).len(), 3);
        assert_eq!(simplify(&zigzag, 500.0).len(), 2);
        assert_eq!(simplify(&path[..1], 10.0).len(), 1);
    }

    #[test]
    fn downsampling_keeps_both_ends() {
        assert_eq!(downsample_indexes(5, 10), vec![0, 1, 2, 3, 4]);
        let picked = downsample_indexes(1000, 10);
        assert_eq!(picked.len(), 10);
        assert_eq!((picked[0], picked[9]), (0, 999));
        assert_eq!(downsample_indexes(3, 2), vec![0, 2]);
        assert!(downsample_indexes(0, 10).is_empty());
    }

    #[test]
    fn polylines_roundtrip() {
        let path = vec![(38.5, -120.2), (40.7, -120.95), (43.252, -126.453)];
        let encoded = encode_polyline(&path);
        assert_eq!(encoded, "_p~iF~ps|U_ulLnnqC_mqNvxq`@");
        let decoded = decode_polyline(&encoded, 5);
        for (a, b) in path.iter().zip(&decoded) {
            assert!((a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5);
        }
        assert!(decode_polyline("not a polyline \u{1}", 5).is_empty());
    }

    #[test]
    fn bounds_cover_all_points() {
        let bounds = bounding_box([SHIBUYA, TOKYO_STATION]).unwrap();
        let (sw, ne) = (bounds.south_west.as_option().unwrap(), bounds.north_east.as_option().unwrap());
        assert_eq!((sw.lat, sw.lng, ne.lat, ne.lng), (SHIBUYA.0, SHIBUYA.1, TOKYO_STATION.0, TOKYO_STATION.1));
        assert!(bounding_box(Vec::<Point>::new()).is_none());
        assert!(in_bounds(35.67, 139.73, SHIBUYA, TOKYO_STATION));
        assert!(!in_bounds(35.70, 139.73, SHIBUYA, TOKYO_STATION));
        // A box across the antimeridian: 170°E .. 170°W.
        assert!(in_bounds(0.0, 179.0, (-10.0, 170.0), (10.0, -170.0)));
        assert!(in_bounds(0.0, -175.0, (-10.0, 170.0), (10.0, -170.0)));
        assert!(!in_bounds(0.0, 0.0, (-10.0, 170.0), (10.0, -170.0)));
    }

    #[test]
    fn cells_convert_to_and_from_storage() {
        let cell = cell_at(SHIBUYA.0, SHIBUYA.1, CELL_RESOLUTION).unwrap();
        let stored = cell_to_db(cell);
        assert!(stored > 0);
        assert_eq!(cell_from_db(stored), Some(cell));
        assert_eq!(cell_hex(stored).len(), 15);
        assert_eq!(cell_hex(stored), cell.to_string());
        assert!(cell_from_db(42).is_none());
        let center = cell_center(cell);
        assert!(distance(center, SHIBUYA) < 250.0);
        assert!(cell_at(f64::NAN, 0.0, CELL_RESOLUTION).is_none());
    }

    #[test]
    fn cell_paths_bridge_gaps_but_not_oceans() {
        let from = cell_at(SHIBUYA.0, SHIBUYA.1, CELL_RESOLUTION).unwrap();
        let near = destination(SHIBUYA.0, SHIBUYA.1, 90.0, 2000.0);
        let to = cell_at(near.0, near.1, CELL_RESOLUTION).unwrap();
        let path = cells_between(from, to, 64);
        assert!(path.len() >= 5 && path.len() <= 12, "{}", path.len());
        assert!(!path.contains(&from));
        assert_eq!(*path.last().unwrap(), to);
        assert!(cells_between(from, from, 64).is_empty());
        let far = cell_at(TOKYO_STATION.0, TOKYO_STATION.1, CELL_RESOLUTION).unwrap();
        assert_eq!(cells_between(from, far, 8), vec![far]);
    }

    #[test]
    fn zoom_picks_the_resolution() {
        assert_eq!(resolution_for_zoom(15.0), Resolution::Nine);
        assert_eq!(resolution_for_zoom(9.0), Resolution::Seven);
        assert_eq!(resolution_for_zoom(4.0), Resolution::Five);
        assert_eq!(resolution_for_zoom(f64::NAN), Resolution::Nine);
    }
}
