//! The official stamp packs, drawn in code.
//!
//! Two characters — まる, a round yellow face, and にゃんこ, a cream cat —
//! each with eight moods. A stamp is a character plus a `Look`: which
//! eyes, brows, mouth and extras to draw. All artwork here is original and
//! made of the plain shapes in [`canvas`](super::canvas).
use super::canvas::{Color, Point, Scene, Shape, pt};

/// One official pack.
pub struct PackDesign {
    /// Stable identifier (`stamp_packs.official_key`).
    pub key: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub tags: &'static [&'static str],
    pub stamps: Vec<StampDesign>,
}

/// One stamp of an official pack.
pub struct StampDesign {
    /// Stable identifier, used in the storage key.
    pub key: &'static str,
    pub name: &'static str,
    pub emoji: &'static str,
    pub keywords: &'static [&'static str],
    pub scene: Scene,
    /// The character's main colour, reported as the image's dominant colour.
    pub color: Color,
}

const INK: Color = Color::rgb(0x4a3326);
const WHITE: Color = Color::rgb(0xffffff);
const BLUSH: Color = Color::rgb(0xff8fa3).alpha(0.6);
const HEART: Color = Color::rgb(0xff4d6d);
const TEAR: Color = Color::rgb(0x6ec6ff);
const MOUTH_INSIDE: Color = Color::rgb(0x8a3324);
const TONGUE: Color = Color::rgb(0xff8a80);
const SPARKLE: Color = Color::rgb(0xffe066);
const SLEEP: Color = Color::rgb(0x7c8ce0);
const OUTLINE: f32 = 2.4;

#[derive(Clone, Copy, PartialEq)]
enum Eyes {
    /// Plain round eyes.
    Dots,
    /// Closed and curved upward: smiling eyes.
    Happy,
    /// Closed and curved downward: asleep.
    Asleep,
    Hearts,
    /// Wide open with small pupils.
    Wide,
    /// Squeezed shut: `> <`.
    Squeezed,
    /// Left eye closed, right eye open.
    Wink,
}

#[derive(Clone, Copy, PartialEq)]
enum Brows {
    Angry,
    Sad,
}

#[derive(Clone, Copy, PartialEq)]
enum Mouth {
    Smile,
    /// Open, laughing, with a tongue.
    Laugh,
    Frown,
    /// A small round "o".
    Gasp,
    /// The cat's `ω`.
    Kitty,
}

#[derive(Clone, Copy, PartialEq)]
enum Extra {
    Blush,
    Tear,
    Zzz,
    Sparkles,
    FloatingHearts,
}

/// The mood of a stamp.
struct Look {
    eyes: Eyes,
    brows: Option<Brows>,
    mouth: Mouth,
    extras: &'static [Extra],
}

/// Where a character's features go.
struct Face {
    /// Centre of the left eye; the right eye mirrors it around x = 50.
    eye: Point,
    /// Centre of the mouth.
    mouth: Point,
    /// Where blush goes on the left cheek.
    cheek: Point,
}

fn mirrored(p: Point) -> Point {
    pt(100.0 - p.x, p.y)
}

/// Both of a left/right pair of features.
fn pair(left: Point) -> [Point; 2] {
    [left, mirrored(left)]
}

fn stroke(from: Point, to: Point, radius: f32) -> Shape {
    Shape::Capsule { from, to, radius }
}

fn triangle(a: Point, b: Point, c: Point, rounding: f32) -> Shape {
    Shape::Polygon { points: vec![a, b, c], rounding }
}

/// A heart centred on `center`, reaching about 1.2 × `size` to each side.
fn heart(center: Point, size: f32) -> Shape {
    // Two lobes on a diamond: the diamond's upper edges are the lobes' diameters.
    // The lobes rise a little higher than the tip hangs low, hence the shift.
    let lobe = size * 0.5;
    let (cx, cy) = (center.x, center.y + size * 0.1);
    Shape::Union(vec![
        Shape::Circle { center: pt(cx - lobe, cy - lobe), radius: lobe * 1.414 },
        Shape::Circle { center: pt(cx + lobe, cy - lobe), radius: lobe * 1.414 },
        Shape::Polygon {
            points: vec![pt(cx - size, cy), pt(cx, cy - size), pt(cx + size, cy), pt(cx, cy + size)],
            rounding: 0.0,
        },
    ])
}

/// A four-pointed sparkle.
fn sparkle(center: Point, size: f32) -> Shape {
    let (cx, cy, thin) = (center.x, center.y, size * 0.28);
    Shape::Union(vec![
        Shape::Polygon {
            points: vec![pt(cx - size, cy), pt(cx, cy - thin), pt(cx + size, cy), pt(cx, cy + thin)],
            rounding: 0.2,
        },
        Shape::Polygon {
            points: vec![pt(cx - thin, cy), pt(cx, cy - size), pt(cx + thin, cy), pt(cx, cy + size)],
            rounding: 0.2,
        },
    ])
}

/// The letter Z, `size` wide, with its top-left corner at `corner`.
fn letter_z(corner: Point, size: f32) -> Shape {
    let (x, y, weight) = (corner.x, corner.y, size * 0.13);
    Shape::Union(vec![
        stroke(pt(x, y), pt(x + size, y), weight),
        stroke(pt(x + size, y), pt(x, y + size), weight),
        stroke(pt(x, y + size), pt(x + size, y + size), weight),
    ])
}

fn paint_eyes(scene: &mut Scene, face: &Face, eyes: Eyes) {
    let [left, right] = pair(face.eye);
    let dot = |scene: &mut Scene, at: Point| {
        scene.paint(Shape::Ellipse { center: at, rx: 3.6, ry: 4.8 }, INK);
        scene.paint(Shape::Circle { center: pt(at.x - 1.1, at.y - 1.8), radius: 1.2 }, WHITE);
    };
    let closed_up = |scene: &mut Scene, at: Point| {
        scene
            .paint(Shape::Arc { center: pt(at.x, at.y + 3.0), radius: 5.2, start: 205.0, end: 335.0, width: 2.6 }, INK);
    };
    let closed_down = |scene: &mut Scene, at: Point| {
        scene.paint(Shape::Arc { center: pt(at.x, at.y - 2.5), radius: 5.2, start: 25.0, end: 155.0, width: 2.6 }, INK);
    };
    match eyes {
        Eyes::Dots => [left, right].into_iter().for_each(|at| dot(scene, at)),
        Eyes::Happy => [left, right].into_iter().for_each(|at| closed_up(scene, at)),
        Eyes::Asleep => [left, right].into_iter().for_each(|at| closed_down(scene, at)),
        Eyes::Hearts => [left, right].into_iter().for_each(|at| {
            scene.paint(heart(at, 5.6), HEART);
        }),
        Eyes::Wide => [left, right].into_iter().for_each(|at| {
            scene.paint_outlined(Shape::Circle { center: at, radius: 5.4 }, WHITE, INK, 1.3);
            scene.paint(Shape::Circle { center: at, radius: 2.3 }, INK);
        }),
        Eyes::Squeezed => {
            // Each eye is an arrowhead pointing at the nose.
            for (at, inward) in [(left, 1.0), (right, -1.0)] {
                let tip = pt(at.x + 3.6 * inward, at.y);
                scene.paint(stroke(pt(at.x - 3.6 * inward, at.y - 4.0), tip, 1.3), INK);
                scene.paint(stroke(pt(at.x - 3.6 * inward, at.y + 4.0), tip, 1.3), INK);
            }
        }
        Eyes::Wink => {
            closed_up(scene, left);
            dot(scene, right);
        }
    }
}

fn paint_brows(scene: &mut Scene, face: &Face, brows: Brows) {
    // Angry brows slope down toward the nose, sad ones up toward it.
    let (outer_rise, inner_rise) = match brows {
        Brows::Angry => (-10.5, -6.5),
        Brows::Sad => (-7.0, -10.5),
    };
    for (eye, outward) in [(face.eye, -1.0), (mirrored(face.eye), 1.0)] {
        let outer = pt(eye.x + 6.5 * outward, eye.y + outer_rise);
        let inner = pt(eye.x - 5.5 * outward, eye.y + inner_rise);
        scene.paint(stroke(outer, inner, 1.4), INK);
    }
}

fn paint_mouth(scene: &mut Scene, face: &Face, mouth: Mouth) {
    let at = face.mouth;
    match mouth {
        Mouth::Smile => {
            scene.paint(
                Shape::Arc { center: pt(at.x, at.y - 6.0), radius: 11.0, start: 30.0, end: 150.0, width: 2.8 },
                INK,
            );
        }
        Mouth::Laugh => {
            // The lower half of a disc, with the tongue clipped to it.
            let top = at.y - 2.0;
            let below = Shape::Polygon {
                points: vec![pt(0.0, top), pt(100.0, top), pt(100.0, 100.0), pt(0.0, 100.0)],
                rounding: 0.0,
            };
            let opening = Shape::Clip(Box::new(Shape::Circle { center: pt(at.x, top), radius: 12.0 }), Box::new(below));
            scene.paint_outlined(opening.clone(), MOUTH_INSIDE, INK, 1.2);
            scene.paint(
                Shape::Clip(
                    Box::new(Shape::Ellipse { center: pt(at.x, top + 11.5), rx: 7.0, ry: 5.0 }),
                    Box::new(opening),
                ),
                TONGUE,
            );
        }
        Mouth::Frown => {
            scene.paint(
                Shape::Arc { center: pt(at.x, at.y + 10.0), radius: 9.5, start: 215.0, end: 325.0, width: 2.8 },
                INK,
            );
        }
        Mouth::Gasp => {
            scene.paint_outlined(
                Shape::Ellipse { center: pt(at.x, at.y + 4.0), rx: 4.2, ry: 5.0 },
                MOUTH_INSIDE,
                INK,
                1.2,
            );
        }
        Mouth::Kitty => {
            for side in [-1.0, 1.0] {
                scene.paint(
                    Shape::Arc {
                        center: pt(at.x + 4.0 * side, at.y - 1.0),
                        radius: 4.0,
                        start: 10.0,
                        end: 170.0,
                        width: 2.2,
                    },
                    INK,
                );
            }
        }
    }
}

fn paint_extra(scene: &mut Scene, face: &Face, extra: Extra) {
    match extra {
        Extra::Blush => {
            for at in pair(face.cheek) {
                scene.paint(Shape::Ellipse { center: at, rx: 6.0, ry: 3.6 }, BLUSH);
            }
        }
        Extra::Tear => {
            let top = pt(face.eye.x, face.eye.y + 7.0);
            let drop = Shape::Union(vec![
                Shape::Circle { center: pt(top.x, top.y + 8.0), radius: 3.6 },
                triangle(top, pt(top.x + 3.3, top.y + 6.6), pt(top.x - 3.3, top.y + 6.6), 0.2),
            ]);
            scene.paint_outlined(drop, TEAR, INK, 0.9);
        }
        Extra::Zzz => {
            for (corner, size) in [(pt(66.0, 22.0), 5.0), (pt(74.0, 12.0), 7.0), (pt(84.0, 1.5), 9.0)] {
                scene.paint(letter_z(corner, size), SLEEP);
            }
        }
        Extra::Sparkles => {
            for (at, size) in [(pt(11.0, 20.0), 8.0), (pt(90.0, 26.0), 6.0), (pt(84.0, 8.0), 4.0)] {
                scene.paint_outlined(sparkle(at, size), SPARKLE, INK, 0.9);
            }
        }
        Extra::FloatingHearts => {
            for (at, size) in [(pt(12.0, 17.0), 5.0), (pt(88.0, 12.0), 6.5)] {
                scene.paint_outlined(heart(at, size), HEART, INK, 0.9);
            }
        }
    }
}

/// Draws the features of `look` on a character's `face`.
fn paint_look(scene: &mut Scene, face: &Face, look: &Look) {
    if look.extras.contains(&Extra::Blush) {
        paint_extra(scene, face, Extra::Blush);
    }
    paint_eyes(scene, face, look.eyes);
    if let Some(brows) = look.brows {
        paint_brows(scene, face, brows);
    }
    paint_mouth(scene, face, look.mouth);
    for extra in look.extras.iter().filter(|extra| **extra != Extra::Blush) {
        paint_extra(scene, face, *extra);
    }
}

const MARU_YELLOW: Color = Color::rgb(0xffd23f);
const MARU_RED: Color = Color::rgb(0xff8a5c);

/// まる: a round face. Angry まる turns red.
fn maru(look: &Look) -> (Scene, Color) {
    let color = if look.brows == Some(Brows::Angry) { MARU_RED } else { MARU_YELLOW };
    let face = Face { eye: pt(36.0, 46.0), mouth: pt(50.0, 66.0), cheek: pt(26.0, 58.0) };
    let mut scene = Scene::new();
    scene.paint_outlined(Shape::Circle { center: pt(50.0, 53.0), radius: 37.0 }, color, INK, OUTLINE);
    scene.paint(Shape::Ellipse { center: pt(37.0, 29.0), rx: 11.0, ry: 5.5 }, WHITE.alpha(0.35));
    paint_look(&mut scene, &face, look);
    (scene, color)
}

const CAT_CREAM: Color = Color::rgb(0xfff1d6);
const CAT_PINK: Color = Color::rgb(0xffa9b8);

/// にゃんこ: a cat's head with ears, a nose and whiskers.
fn nyanko(look: &Look) -> (Scene, Color) {
    let face = Face { eye: pt(35.0, 54.0), mouth: pt(50.0, 68.5), cheek: pt(25.0, 64.0) };
    let ear = triangle(pt(19.0, 40.0), pt(22.0, 13.0), pt(44.0, 29.0), 2.5);
    let inner_ear = triangle(pt(24.0, 34.0), pt(25.5, 21.0), pt(36.0, 28.5), 1.0);
    let flipped = |shape: &Shape| match shape {
        Shape::Polygon { points, rounding } => {
            Shape::Polygon { points: points.iter().map(|p| mirrored(*p)).collect(), rounding: *rounding }
        }
        other => other.clone(),
    };
    let mut scene = Scene::new();
    // Ears and head share one outline, so the ears grow out of the head.
    let silhouette =
        Shape::Union(vec![ear.clone(), flipped(&ear), Shape::Ellipse { center: pt(50.0, 58.0), rx: 38.0, ry: 32.0 }]);
    scene.paint_outlined(silhouette, CAT_CREAM, INK, OUTLINE);
    scene.paint(inner_ear.clone(), CAT_PINK);
    scene.paint(flipped(&inner_ear), CAT_PINK);
    for (from, to) in [(pt(5.0, 58.0), pt(19.0, 60.5)), (pt(6.0, 68.0), pt(19.0, 66.0))] {
        scene.paint(stroke(from, to, 0.9), INK);
        scene.paint(stroke(mirrored(from), mirrored(to), 0.9), INK);
    }
    scene.paint(triangle(pt(47.0, 61.0), pt(53.0, 61.0), pt(50.0, 64.2), 0.9), CAT_PINK);
    paint_look(&mut scene, &face, look);
    (scene, CAT_CREAM)
}

/// A mood both characters come in.
struct Mood {
    key: &'static str,
    emoji: &'static str,
    keywords: &'static [&'static str],
    look: Look,
}

/// The eight moods. `resting_mouth` is what the character's mouth does when
/// the mood does not say otherwise (a smile for まる, `ω` for にゃんこ).
fn moods(resting_mouth: Mouth) -> Vec<Mood> {
    use Extra::*;
    let look = |eyes, brows, mouth, extras| Look { eyes, brows, mouth, extras };
    let mood = |key, emoji, keywords, look| Mood { key, emoji, keywords, look };
    vec![
        mood(
            "smile",
            "😊",
            &["にこにこ", "うれしい", "smile", "happy"],
            look(Eyes::Happy, None, resting_mouth, &[Blush]),
        ),
        mood("laugh", "😆", &["わらい", "わーい", "laugh", "lol"], look(Eyes::Squeezed, None, Mouth::Laugh, &[])),
        mood(
            "love",
            "😍",
            &["すき", "だいすき", "love", "heart"],
            look(Eyes::Hearts, None, resting_mouth, &[Blush, FloatingHearts]),
        ),
        mood(
            "cry",
            "😢",
            &["かなしい", "えーん", "sad", "cry"],
            look(Eyes::Dots, Some(Brows::Sad), Mouth::Frown, &[Tear]),
        ),
        mood(
            "angry",
            "😠",
            &["おこ", "ぷんぷん", "angry", "mad"],
            look(Eyes::Dots, Some(Brows::Angry), Mouth::Frown, &[]),
        ),
        mood("surprised", "😲", &["びっくり", "えっ", "surprised", "wow"], look(Eyes::Wide, None, Mouth::Gasp, &[])),
        mood(
            "sleepy",
            "😴",
            &["ねむい", "おやすみ", "sleepy", "goodnight"],
            look(Eyes::Asleep, None, Mouth::Gasp, &[Zzz]),
        ),
        mood(
            "ok",
            "😉",
            &["おっけー", "りょうかい", "ok", "wink"],
            look(Eyes::Wink, None, Mouth::Smile, &[Blush, Sparkles]),
        ),
    ]
}

/// Display names of the moods, per character.
const MARU_NAMES: [&str; 8] =
    ["にこにこ", "わーい!", "だいすき", "えーん", "ぷんぷん", "びっくり!", "おやすみ", "おっけー!"];
const NYANKO_NAMES: [&str; 8] =
    ["にゃー", "にゃはは", "すきにゃ", "しょんぼり", "ぷんすか", "にゃっ!?", "ねむねむ", "おっけーにゃ"];

fn stamps(resting_mouth: Mouth, names: [&'static str; 8], draw: fn(&Look) -> (Scene, Color)) -> Vec<StampDesign> {
    moods(resting_mouth)
        .into_iter()
        .zip(names)
        .map(|(mood, name)| {
            let (scene, color) = draw(&mood.look);
            StampDesign { key: mood.key, name, emoji: mood.emoji, keywords: mood.keywords, scene, color }
        })
        .collect()
}

/// Every official pack, in the order they should appear in the store.
pub fn packs() -> Vec<PackDesign> {
    vec![
        PackDesign {
            key: "maru-moods",
            name: "まるのきもち",
            description: "まるい顔の「まる」が、きょうの気分を伝えるよ!毎日の会話にどうぞ。",
            tags: &["公式", "かわいい", "きもち", "まる"],
            stamps: stamps(Mouth::Smile, MARU_NAMES, maru),
        },
        PackDesign {
            key: "nyanko-moods",
            name: "にゃんこのきもち",
            description: "のんびり屋の「にゃんこ」のスタンプ。うれしい時も、ねむい時も、にゃー!",
            tags: &["公式", "かわいい", "ねこ", "きもち"],
            stamps: stamps(Mouth::Kitty, NYANKO_NAMES, nyanko),
        },
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn packs_are_small_complete_and_uniquely_keyed() {
        let packs = packs();
        assert_eq!(packs.len(), 2);
        let pack_keys: HashSet<&str> = packs.iter().map(|p| p.key).collect();
        assert_eq!(pack_keys.len(), packs.len());
        for pack in &packs {
            assert_eq!(pack.stamps.len(), 8, "{}", pack.key);
            let stamp_keys: HashSet<&str> = pack.stamps.iter().map(|s| s.key).collect();
            assert_eq!(stamp_keys.len(), 8, "stamp keys of {} are unique", pack.key);
            // The texts go through the same validation as a creator's.
            assert!(crate::validate::pack_name(pack.name).is_ok());
            assert!(crate::validate::description(pack.description).is_ok());
            assert_eq!(crate::validate::tags(pack.tags).unwrap().len(), pack.tags.len());
            for stamp in &pack.stamps {
                let text = crate::validate::stamp_text(stamp.name, stamp.emoji, stamp.keywords).unwrap();
                assert_eq!(text.keywords.len(), stamp.keywords.len(), "{} keywords are already normalized", stamp.key);
            }
        }
    }

    #[test]
    fn every_stamp_draws_a_character_on_a_transparent_background() {
        let size = 48;
        for pack in packs() {
            let mut renders = HashSet::new();
            for stamp in &pack.stamps {
                let pixels = stamp.scene.render(size);
                let alpha = |x: u32, y: u32| pixels[((y * size + x) * 4 + 3) as usize];
                assert_eq!(alpha(0, size - 1), 0, "{}/{}: the corner is empty", pack.key, stamp.key);
                assert_eq!(alpha(size / 2, size / 2 + 4), 255, "{}/{}: the face is solid", pack.key, stamp.key);
                renders.insert(pixels);
            }
            assert_eq!(renders.len(), pack.stamps.len(), "every mood of {} looks different", pack.key);
        }
    }

    #[test]
    fn hearts_and_sparkles_are_where_they_are_drawn() {
        let heart = heart(pt(50.0, 50.0), 10.0);
        assert!(heart.distance(pt(50.0, 50.0)) < 0.0);
        assert!(heart.distance(pt(50.0, 59.0)) < 0.0, "the tip points down");
        assert!(heart.distance(pt(50.0, 35.0)) > 0.0, "the notch between the lobes");
        let sparkle = sparkle(pt(50.0, 50.0), 10.0);
        assert!(sparkle.distance(pt(58.0, 50.0)) < 0.0 && sparkle.distance(pt(50.0, 42.0)) < 0.0);
        assert!(sparkle.distance(pt(57.0, 57.0)) > 0.0, "nothing on the diagonals");
    }
}
