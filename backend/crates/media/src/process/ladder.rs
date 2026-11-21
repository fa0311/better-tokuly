//! Choosing the HLS rendition ladder for a source video.
//!
//! Rungs are named after their short side (`1080p`, `720p`…), which is the
//! height of a landscape video and the *width* of a portrait one, so a phone
//! clip stays portrait at every quality. Nothing is ever upscaled: the top
//! rung is the source's own size (capped), and the standard rungs below it
//! follow.

/// Length of an HLS segment; every rung has a keyframe at each multiple.
pub const SEGMENT_SECONDS: u32 = 4;

/// Short sides of the standard rungs, tallest first.
const STANDARD_RUNGS: [u32; 5] = [1080, 720, 480, 360, 240];

/// A standard rung is only added below another when it is at least this
/// much smaller (in percent of the short side); closer ones waste storage.
const MIN_STEP_PERCENT: u32 = 80;

/// An exact frame rate, kept rational so NTSC rates stay precise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRate {
    pub numerator: u32,
    pub denominator: u32,
}

impl FrameRate {
    const COMMON: [FrameRate; 9] = [
        FrameRate { numerator: 24_000, denominator: 1001 },
        FrameRate { numerator: 24, denominator: 1 },
        FrameRate { numerator: 25, denominator: 1 },
        FrameRate { numerator: 30_000, denominator: 1001 },
        FrameRate { numerator: 30, denominator: 1 },
        FrameRate { numerator: 48, denominator: 1 },
        FrameRate { numerator: 50, denominator: 1 },
        FrameRate { numerator: 60_000, denominator: 1001 },
        FrameRate { numerator: 60, denominator: 1 },
    ];

    /// The constant output frame rate for a source: snapped to a standard
    /// rate when within 1% (phones record "30 fps" as 29.8–30.1), halved
    /// until it is at most 60.
    pub fn for_source(mut source: f64) -> Self {
        if !source.is_finite() || source < 1.0 {
            source = 30.0;
        }
        while source > 60.5 {
            source /= 2.0;
        }
        let nearest =
            Self::COMMON.iter().copied().min_by(|a, b| (a.as_f64() - source).abs().total_cmp(&(b.as_f64() - source).abs())).expect("list is not empty");
        if (nearest.as_f64() - source).abs() <= source * 0.01 {
            nearest
        } else {
            Self { numerator: (source * 1000.0).round() as u32, denominator: 1000 }
        }
    }

    pub fn as_f64(self) -> f64 {
        f64::from(self.numerator) / f64::from(self.denominator)
    }

    /// Half the rate: every other frame, so keyframes still line up.
    pub fn halved(self) -> Self {
        if self.numerator.is_multiple_of(2) {
            Self { numerator: self.numerator / 2, denominator: self.denominator }
        } else {
            Self { numerator: self.numerator, denominator: self.denominator * 2 }
        }
    }

    /// The form ffmpeg takes (`30000/1001`).
    pub fn to_arg(self) -> String {
        format!("{}/{}", self.numerator, self.denominator)
    }

    /// Frames in one HLS segment.
    pub fn frames_per_segment(self) -> u32 {
        (self.as_f64() * f64::from(SEGMENT_SECONDS)).round().max(1.0) as u32
    }
}

/// One video rendition of the ladder.
#[derive(Debug, Clone, PartialEq)]
pub struct Rung {
    /// `1080p`, `720p`… (short side); also the directory name of the rendition.
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    /// Ceiling for the encoder's rate control.
    pub max_bitrate_kbps: u32,
    /// x264 constant rate factor: quality target below the ceiling.
    pub crf: u32,
}

fn even(value: f64) -> u32 {
    ((value / 2.0).round() as u32 * 2).max(2)
}

/// Bitrate ceiling for H.264 at a given frame size: 5 Mbit/s at 1080p,
/// scaling sub-linearly with the pixel count (2.7 Mbit/s at 720p, 1.5 at
/// 480p, 0.5 at 240p), and 40% more for high frame rates.
fn max_bitrate_kbps(width: u32, height: u32, frame_rate: FrameRate) -> u32 {
    let relative_pixels = f64::from(width) * f64::from(height) / (1920.0 * 1080.0);
    let base = 5000.0 * relative_pixels.powf(0.75);
    let boost = if frame_rate.as_f64() > 40.0 { 1.4 } else { 1.0 };
    ((base * boost / 50.0).round() * 50.0).max(250.0) as u32
}

/// The rungs for a source of `width`×`height` (display size), tallest first.
/// `max_short_side` caps the top rung (purposes with small players get a
/// shorter ladder).
pub fn ladder(width: u32, height: u32, source_rate: f64, max_short_side: u32) -> Vec<Rung> {
    let (short, long) = (width.min(height).max(2), width.max(height).max(2));
    let portrait = height > width;
    let frame_rate = FrameRate::for_source(source_rate);

    let top = short.min(max_short_side);
    let mut short_sides = vec![top];
    for standard in STANDARD_RUNGS {
        let previous = *short_sides.last().expect("starts non-empty");
        if standard < top && standard * 100 <= previous * MIN_STEP_PERCENT {
            short_sides.push(standard);
        }
    }

    short_sides
        .into_iter()
        .map(|side| {
            let side = even(f64::from(side));
            let other = even(f64::from(long) * f64::from(side) / f64::from(short));
            let (width, height) = if portrait { (side, other) } else { (other, side) };
            // Small renditions exist for bad connections: 60 fps is wasted on them.
            let frame_rate = if side < 720 && frame_rate.as_f64() > 30.5 { frame_rate.halved() } else { frame_rate };
            Rung {
                name: format!("{side}p"),
                width,
                height,
                frame_rate,
                max_bitrate_kbps: max_bitrate_kbps(width, height, frame_rate),
                crf: if side >= 720 { 22 } else { 23 },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(rungs: &[Rung]) -> Vec<(String, u32, u32)> {
        rungs.iter().map(|r| (r.name.clone(), r.width, r.height)).collect()
    }

    fn names(rungs: &[Rung]) -> Vec<&str> {
        rungs.iter().map(|r| r.name.as_str()).collect()
    }

    #[test]
    fn full_hd_gets_the_whole_ladder() {
        let rungs = ladder(1920, 1080, 30.0, 1080);
        assert_eq!(
            summary(&rungs),
            vec![
                ("1080p".to_owned(), 1920, 1080),
                ("720p".to_owned(), 1280, 720),
                ("480p".to_owned(), 854, 480),
                ("360p".to_owned(), 640, 360),
                ("240p".to_owned(), 426, 240),
            ]
        );
        assert_eq!(rungs.iter().map(|r| r.max_bitrate_kbps).collect::<Vec<_>>(), vec![5000, 2700, 1500, 950, 500]);
    }

    #[test]
    fn portrait_stays_portrait() {
        let rungs = ladder(1080, 1920, 30.0, 1080);
        assert_eq!(summary(&rungs)[..2], [("1080p".to_owned(), 1080, 1920), ("720p".to_owned(), 720, 1280)]);
        assert!(rungs.iter().all(|r| r.height > r.width));
    }

    #[test]
    fn nothing_is_upscaled() {
        assert_eq!(names(&ladder(3840, 2160, 30.0, 1080)), ["1080p", "720p", "480p", "360p", "240p"], "4K is capped at the top rung");
        assert_eq!(summary(&ladder(640, 360, 30.0, 1080)), vec![("360p".to_owned(), 640, 360), ("240p".to_owned(), 426, 240)]);
        assert_eq!(summary(&ladder(320, 240, 30.0, 1080)), vec![("240p".to_owned(), 320, 240)]);
        assert_eq!(summary(&ladder(200, 100, 30.0, 1080)), vec![("100p".to_owned(), 200, 100)]);
        // 800p source: 720p is too close to be worth a rung of its own.
        assert_eq!(names(&ladder(1280, 800, 30.0, 1080)), ["800p", "480p", "360p", "240p"]);
        assert_eq!(names(&ladder(1600, 900, 30.0, 1080)), ["900p", "720p", "480p", "360p", "240p"]);
    }

    #[test]
    fn purposes_can_cap_the_ladder() {
        assert_eq!(names(&ladder(1920, 1080, 30.0, 720)), ["720p", "480p", "360p", "240p"]);
        assert_eq!(names(&ladder(1080, 1920, 30.0, 480)), ["480p", "360p", "240p"]);
    }

    #[test]
    fn odd_sizes_become_even() {
        let rungs = ladder(853, 479, 30.0, 1080);
        assert!(rungs.iter().all(|r| r.width % 2 == 0 && r.height % 2 == 0), "{rungs:?}");
        assert_eq!((rungs[0].width, rungs[0].height), (854, 480));
    }

    #[test]
    fn frame_rates_snap_to_standards() {
        assert_eq!(FrameRate::for_source(29.97).to_arg(), "30000/1001");
        assert_eq!(FrameRate::for_source(30.02).to_arg(), "30/1");
        assert_eq!(FrameRate::for_source(59.94).to_arg(), "60000/1001");
        assert_eq!(FrameRate::for_source(120.0).to_arg(), "60/1");
        assert_eq!(FrameRate::for_source(15.0).to_arg(), "15000/1000");
        assert_eq!(FrameRate::for_source(0.0).to_arg(), "30/1");
        assert_eq!(FrameRate::for_source(f64::NAN).to_arg(), "30/1");
        assert_eq!(FrameRate::for_source(60.0).halved().to_arg(), "30/1");
        assert_eq!(FrameRate::for_source(59.94).halved().to_arg(), "30000/1001");
        assert_eq!(FrameRate::for_source(29.97).frames_per_segment(), 120);
    }

    #[test]
    fn high_frame_rates_are_kept_only_on_large_rungs() {
        let rungs = ladder(1920, 1080, 60.0, 1080);
        let rates: Vec<f64> = rungs.iter().map(|r| r.frame_rate.as_f64()).collect();
        assert_eq!(rates, vec![60.0, 60.0, 30.0, 30.0, 30.0]);
        assert_eq!(rungs[0].max_bitrate_kbps, 7000);
    }
}
