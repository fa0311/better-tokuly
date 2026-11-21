//! A small vector rasterizer for the official stamp art.
//!
//! A [`Scene`] is a stack of coloured [`Shape`]s on a 100×100 design canvas
//! (x to the right, y down). Shapes are signed distance functions — negative
//! inside, positive outside — which makes anti-aliasing a one-liner and lets
//! the same scene be rendered crisply at any pixel size: the 512px stamp,
//! the picker thumbnail and the tiny image a thumbhash is computed from.

/// Side length of the design canvas.
pub const CANVAS: f32 = 100.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

pub const fn pt(x: f32, y: f32) -> Point {
    Point { x, y }
}

impl Point {
    fn minus(self, other: Point) -> Point {
        pt(self.x - other.x, self.y - other.y)
    }

    fn dot(self, other: Point) -> f32 {
        self.x * other.x + self.y * other.y
    }

    fn length(self) -> f32 {
        self.x.hypot(self.y)
    }
}

/// A colour with straight (not premultiplied) alpha; channels are 0-1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    /// An opaque colour from `0xRRGGBB`.
    pub const fn rgb(hex: u32) -> Self {
        Self {
            r: ((hex >> 16) & 0xff) as f32 / 255.0,
            g: ((hex >> 8) & 0xff) as f32 / 255.0,
            b: (hex & 0xff) as f32 / 255.0,
            a: 1.0,
        }
    }

    pub const fn alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    /// `#rrggbb`, as used for `Image.dominant_color`.
    pub fn hex(self) -> String {
        let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!("#{:02x}{:02x}{:02x}", channel(self.r), channel(self.g), channel(self.b))
    }
}

#[derive(Debug, Clone)]
pub enum Shape {
    Circle {
        center: Point,
        radius: f32,
    },
    Ellipse {
        center: Point,
        rx: f32,
        ry: f32,
    },
    /// A line with round ends, `radius` thick on each side.
    Capsule {
        from: Point,
        to: Point,
        radius: f32,
    },
    /// A stroke along part of a circle, from `start` to `end` degrees.
    /// 0° points right and angles grow clockwise on screen (90° is down).
    Arc {
        center: Point,
        radius: f32,
        start: f32,
        end: f32,
        width: f32,
    },
    /// A convex polygon; `rounding` grows it outward with round corners.
    Polygon {
        points: Vec<Point>,
        rounding: f32,
    },
    /// Everything inside any of the shapes.
    Union(Vec<Shape>),
    /// The part of the first shape that lies inside the second.
    Clip(Box<Shape>, Box<Shape>),
    /// The shape grown outward by a margin (used for outlines).
    Grow(Box<Shape>, f32),
}

/// An axis-aligned box: `(min, max)`.
type Bounds = (Point, Point);

impl Shape {
    /// Signed distance from `p` to the shape's edge, in canvas units.
    pub fn distance(&self, p: Point) -> f32 {
        match self {
            Shape::Circle { center, radius } => p.minus(*center).length() - radius,
            Shape::Ellipse { center, rx, ry } => ellipse_distance(p.minus(*center), *rx, *ry),
            Shape::Capsule { from, to, radius } => segment_distance(p, *from, *to) - radius,
            Shape::Arc { center, radius, start, end, width } => {
                arc_distance(p.minus(*center), *radius, *start, *end) - width / 2.0
            }
            Shape::Polygon { points, rounding } => polygon_distance(p, points) - rounding,
            Shape::Union(shapes) => shapes.iter().map(|s| s.distance(p)).fold(f32::INFINITY, f32::min),
            Shape::Clip(shape, mask) => shape.distance(p).max(mask.distance(p)),
            Shape::Grow(shape, margin) => shape.distance(p) - margin,
        }
    }

    /// A box that contains the shape; pixels outside it are skipped.
    fn bounds(&self) -> Bounds {
        let around =
            |center: Point, dx: f32, dy: f32| (pt(center.x - dx, center.y - dy), pt(center.x + dx, center.y + dy));
        let enclose = |points: &[Point], margin: f32| {
            let min = points.iter().fold(pt(f32::INFINITY, f32::INFINITY), |m, p| pt(m.x.min(p.x), m.y.min(p.y)));
            let max =
                points.iter().fold(pt(f32::NEG_INFINITY, f32::NEG_INFINITY), |m, p| pt(m.x.max(p.x), m.y.max(p.y)));
            (pt(min.x - margin, min.y - margin), pt(max.x + margin, max.y + margin))
        };
        match self {
            Shape::Circle { center, radius } => around(*center, *radius, *radius),
            Shape::Ellipse { center, rx, ry } => around(*center, *rx, *ry),
            Shape::Capsule { from, to, radius } => enclose(&[*from, *to], *radius),
            Shape::Arc { center, radius, width, .. } => around(*center, radius + width / 2.0, radius + width / 2.0),
            Shape::Polygon { points, rounding } => enclose(points, *rounding),
            Shape::Union(shapes) => {
                let corners: Vec<Point> = shapes.iter().flat_map(|s| <[Point; 2]>::from(s.bounds())).collect();
                enclose(&corners, 0.0)
            }
            Shape::Clip(shape, _) => shape.bounds(),
            Shape::Grow(shape, margin) => {
                let (min, max) = shape.bounds();
                (pt(min.x - margin, min.y - margin), pt(max.x + margin, max.y + margin))
            }
        }
    }
}

/// Distance to an ellipse centred on the origin. Exact on the outline and a
/// good approximation near it, which is all anti-aliasing needs.
fn ellipse_distance(p: Point, rx: f32, ry: f32) -> f32 {
    let k0 = (p.x / rx).hypot(p.y / ry);
    let k1 = (p.x / (rx * rx)).hypot(p.y / (ry * ry));
    if k1 == 0.0 { -rx.min(ry) } else { k0 * (k0 - 1.0) / k1 }
}

fn segment_distance(p: Point, from: Point, to: Point) -> f32 {
    let along = to.minus(from);
    let length_squared = along.dot(along);
    let t = if length_squared == 0.0 { 0.0 } else { (p.minus(from).dot(along) / length_squared).clamp(0.0, 1.0) };
    p.minus(pt(from.x + along.x * t, from.y + along.y * t)).length()
}

/// Distance to the centre line of an arc around the origin.
fn arc_distance(p: Point, radius: f32, start: f32, end: f32) -> f32 {
    let angle = p.y.atan2(p.x).to_degrees();
    // Bring the angle into [start, start + 360) to test it against the sweep.
    let turned = start + (angle - start).rem_euclid(360.0);
    if turned <= end {
        return (p.length() - radius).abs();
    }
    let tip = |degrees: f32| pt(radius * degrees.to_radians().cos(), radius * degrees.to_radians().sin());
    p.minus(tip(start)).length().min(p.minus(tip(end)).length())
}

/// Exact distance to a convex polygon given in either winding order.
fn polygon_distance(p: Point, points: &[Point]) -> f32 {
    let edges = || points.iter().zip(points.iter().cycle().skip(1));
    let nearest = edges().map(|(a, b)| segment_distance(p, *a, *b)).fold(f32::INFINITY, f32::min);
    // Inside a convex polygon the point is on the same side of every edge.
    let side = |a: &Point, b: &Point| (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
    let inside = edges().all(|(a, b)| side(a, b) >= 0.0) || edges().all(|(a, b)| side(a, b) <= 0.0);
    if inside { -nearest } else { nearest }
}

#[derive(Debug, Clone)]
struct Layer {
    shape: Shape,
    color: Color,
    bounds: Bounds,
}

/// A picture: shapes painted in order, later ones on top.
#[derive(Debug, Clone, Default)]
pub struct Scene {
    layers: Vec<Layer>,
}

impl Scene {
    pub fn new() -> Self {
        Self::default()
    }

    /// Paints `shape` on top of what is there.
    pub fn paint(&mut self, shape: Shape, color: Color) -> &mut Self {
        let bounds = shape.bounds();
        self.layers.push(Layer { shape, color, bounds });
        self
    }

    /// Paints `shape` with an outline of `ink` around it.
    pub fn paint_outlined(&mut self, shape: Shape, fill: Color, ink: Color, outline: f32) -> &mut Self {
        self.paint(Shape::Grow(Box::new(shape.clone()), outline), ink);
        self.paint(shape, fill)
    }

    /// Renders the scene to `size`×`size` RGBA pixels (straight alpha,
    /// transparent background).
    pub fn render(&self, size: u32) -> Vec<u8> {
        let scale = size as f32 / CANVAS;
        let mut pixels = Vec::with_capacity((size * size * 4) as usize);
        for y in 0..size {
            for x in 0..size {
                let p = pt((x as f32 + 0.5) / scale, (y as f32 + 0.5) / scale);
                pixels.extend_from_slice(&self.pixel(p, scale));
            }
        }
        pixels
    }

    /// The colour at canvas point `p`, where one pixel is `1 / scale` wide.
    fn pixel(&self, p: Point, scale: f32) -> [u8; 4] {
        let pixel_width = 1.0 / scale;
        // Accumulated colour, premultiplied by alpha.
        let (mut r, mut g, mut b, mut a) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for layer in &self.layers {
            let (min, max) = layer.bounds;
            if p.x < min.x - pixel_width
                || p.x > max.x + pixel_width
                || p.y < min.y - pixel_width
                || p.y > max.y + pixel_width
            {
                continue;
            }
            // The edge is blended across one pixel: fully inside half a pixel
            // in, fully outside half a pixel out.
            let coverage = (0.5 - layer.shape.distance(p) * scale).clamp(0.0, 1.0);
            let alpha = layer.color.a * coverage;
            if alpha <= 0.0 {
                continue;
            }
            r = layer.color.r * alpha + r * (1.0 - alpha);
            g = layer.color.g * alpha + g * (1.0 - alpha);
            b = layer.color.b * alpha + b * (1.0 - alpha);
            a = alpha + a * (1.0 - alpha);
        }
        if a <= 0.0 {
            return [0, 0, 0, 0];
        }
        let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        [channel(r / a), channel(g / a), channel(b / a), channel(a)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(pixels: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * size + x) * 4) as usize;
        [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
    }

    #[test]
    fn distances_are_negative_inside_and_positive_outside() {
        let circle = Shape::Circle { center: pt(50.0, 50.0), radius: 10.0 };
        assert_eq!(circle.distance(pt(50.0, 50.0)), -10.0);
        assert_eq!(circle.distance(pt(70.0, 50.0)), 10.0);

        let capsule = Shape::Capsule { from: pt(10.0, 10.0), to: pt(30.0, 10.0), radius: 2.0 };
        assert_eq!(capsule.distance(pt(20.0, 10.0)), -2.0);
        assert_eq!(capsule.distance(pt(35.0, 10.0)), 3.0, "past the round end");

        let ellipse = Shape::Ellipse { center: pt(50.0, 50.0), rx: 20.0, ry: 10.0 };
        assert!(ellipse.distance(pt(50.0, 50.0)) < 0.0);
        assert!(ellipse.distance(pt(70.0, 50.0)).abs() < 1e-4, "on the outline");
        assert!(ellipse.distance(pt(50.0, 65.0)) > 0.0);
    }

    #[test]
    fn polygons_work_in_both_winding_orders() {
        let square = vec![pt(10.0, 10.0), pt(30.0, 10.0), pt(30.0, 30.0), pt(10.0, 30.0)];
        let reversed: Vec<Point> = square.iter().rev().copied().collect();
        for points in [square, reversed] {
            let polygon = Shape::Polygon { points, rounding: 0.0 };
            assert_eq!(polygon.distance(pt(20.0, 20.0)), -10.0);
            assert_eq!(polygon.distance(pt(35.0, 20.0)), 5.0);
            assert_eq!(polygon.distance(pt(33.0, 34.0)), 5.0, "the corner is the nearest point");
        }
    }

    #[test]
    fn arcs_cover_only_their_sweep() {
        // The lower half of a circle: a smile.
        let smile = Shape::Arc { center: pt(50.0, 50.0), radius: 10.0, start: 0.0, end: 180.0, width: 2.0 };
        assert_eq!(smile.distance(pt(50.0, 60.0)), -1.0, "bottom of the arc, on its centre line");
        assert!(smile.distance(pt(50.0, 40.0)) > 5.0, "the top is not part of it");
        assert!((smile.distance(pt(60.0, 48.0)) - 1.0).abs() < 1e-4, "just past the end, measured from the tip");
    }

    #[test]
    fn clip_and_grow_combine_shapes() {
        let disc = Shape::Circle { center: pt(50.0, 50.0), radius: 10.0 };
        let lower_half = Shape::Polygon {
            points: vec![pt(0.0, 50.0), pt(100.0, 50.0), pt(100.0, 100.0), pt(0.0, 100.0)],
            rounding: 0.0,
        };
        let bowl = Shape::Clip(Box::new(disc.clone()), Box::new(lower_half));
        assert!(bowl.distance(pt(50.0, 55.0)) < 0.0);
        assert!(bowl.distance(pt(50.0, 45.0)) > 0.0);
        assert_eq!(Shape::Grow(Box::new(disc), 3.0).distance(pt(63.0, 50.0)), 0.0);
    }

    #[test]
    fn rendering_fills_shapes_and_leaves_the_background_transparent() {
        let mut scene = Scene::new();
        scene.paint(Shape::Circle { center: pt(50.0, 50.0), radius: 30.0 }, Color::rgb(0xff0000));
        scene.paint(Shape::Circle { center: pt(50.0, 50.0), radius: 10.0 }, Color::rgb(0x0000ff).alpha(0.5));
        let size = 64;
        let pixels = scene.render(size);
        assert_eq!(pixels.len(), (size * size * 4) as usize);
        assert_eq!(pixel(&pixels, size, 0, 0), [0, 0, 0, 0], "corner is outside the circle");
        assert_eq!(pixel(&pixels, size, 32, 14), [255, 0, 0, 255], "inside the red ring");
        let centre = pixel(&pixels, size, 32, 32);
        assert_eq!(centre[3], 255);
        assert!(centre[0] > 100 && centre[2] > 100 && centre[1] == 0, "half-transparent blue over red: {centre:?}");
    }

    #[test]
    fn edges_are_anti_aliased() {
        let mut scene = Scene::new();
        scene.paint(Shape::Circle { center: pt(50.0, 50.0), radius: 30.0 }, Color::rgb(0x000000));
        let size = 100;
        let pixels = scene.render(size);
        // Along the row through the centre the circle ends at x = 80 exactly,
        // between two pixels: one is almost full, the next almost empty.
        let alphas: Vec<u8> = (76..84).map(|x| pixel(&pixels, size, x, 50)[3]).collect();
        assert_eq!(&alphas[..3], &[255, 255, 255]);
        assert_eq!(&alphas[5..], &[0, 0, 0]);
        assert!(alphas[3] > alphas[4], "coverage falls off across the edge: {alphas:?}");
    }

    #[test]
    fn colours_format_as_hex() {
        assert_eq!(Color::rgb(0xffd23f).hex(), "#ffd23f");
        assert_eq!(Color::rgb(0x000000).alpha(0.3).hex(), "#000000");
    }
}
