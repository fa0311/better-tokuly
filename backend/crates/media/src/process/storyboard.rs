//! Seek previews: sprite sheets of small frames plus a WebVTT track that
//! maps time ranges to regions of those sheets (`sprite_001.webp#xywh=…`),
//! the format video players understand as a "thumbnails" track.

/// Longest edge of one preview frame.
const THUMB_EDGE: u32 = 160;
/// Sheets are grids of `COLUMNS`×`ROWS` frames.
pub const COLUMNS: u32 = 10;
pub const ROWS: u32 = 10;
/// Aim for at most this many frames, but never space them wider than
/// [`MAX_INTERVAL_SECONDS`] (long videos just get more sheets).
const TARGET_FRAMES: f64 = 300.0;
const MAX_INTERVAL_SECONDS: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// Seconds between two preview frames.
    pub interval: u32,
    pub thumb_width: u32,
    pub thumb_height: u32,
    /// Number of preview frames.
    pub frames: u32,
}

impl Layout {
    /// The layout for a video of `duration` seconds and the given display size.
    pub fn new(duration: f64, width: u32, height: u32) -> Self {
        let interval = ((duration / TARGET_FRAMES).ceil() as u32).clamp(1, MAX_INTERVAL_SECONDS);
        let frames = ((duration / f64::from(interval)).ceil() as u32).max(1);
        let longest = width.max(height).max(1);
        let even = |edge: u32| ((u64::from(edge) * u64::from(THUMB_EDGE) / u64::from(longest)) as u32 / 2 * 2).max(2);
        Self { interval, thumb_width: even(width), thumb_height: even(height), frames }
    }

    pub fn per_sheet(self) -> u32 {
        COLUMNS * ROWS
    }

    pub fn sheets(self) -> u32 {
        self.frames.div_ceil(self.per_sheet())
    }

    /// The ffmpeg filter that samples, scales and tiles the frames.
    pub fn filter(self) -> String {
        format!("fps=1/{},scale={}:{}:flags=bicubic,setsar=1,tile={COLUMNS}x{ROWS}", self.interval, self.thumb_width, self.thumb_height)
    }
}

/// File name of sheet `number` (1-based), as written by ffmpeg's `%03d`.
pub fn sheet_name(number: u32) -> String {
    format!("sprite_{number:03}.webp")
}

fn timestamp(seconds: f64) -> String {
    let millis = (seconds.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02}.{:03}", millis / 3_600_000, millis / 60_000 % 60, millis / 1000 % 60, millis % 1000)
}

/// The WebVTT thumbnail track. `available_sheets` is how many sheets ffmpeg
/// really produced: cues never point at a sheet that does not exist.
pub fn vtt(layout: Layout, duration: f64, available_sheets: u32) -> String {
    let mut out = String::from("WEBVTT\n");
    let frames = layout.frames.min(available_sheets * layout.per_sheet());
    for index in 0..frames {
        let start = f64::from(index * layout.interval);
        let end = if index + 1 == frames { duration.max(start + 1.0) } else { f64::from((index + 1) * layout.interval) };
        let cell = index % layout.per_sheet();
        let (x, y) = (cell % COLUMNS * layout.thumb_width, cell / COLUMNS * layout.thumb_height);
        out.push_str(&format!(
            "\n{} --> {}\n{}#xywh={x},{y},{},{}\n",
            timestamp(start),
            timestamp(end),
            sheet_name(index / layout.per_sheet() + 1),
            layout.thumb_width,
            layout.thumb_height
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_scales_with_duration() {
        let short = Layout::new(9.0, 1280, 720);
        assert_eq!(short, Layout { interval: 1, thumb_width: 160, thumb_height: 90, frames: 9 });
        assert_eq!(short.sheets(), 1);

        let long = Layout::new(2.0 * 3600.0, 1920, 1080);
        assert_eq!((long.interval, long.frames, long.sheets()), (10, 720, 8));

        let portrait = Layout::new(30.0, 1080, 1920);
        assert_eq!((portrait.thumb_width, portrait.thumb_height), (90, 160));
        assert_eq!(Layout::new(0.2, 64, 64).frames, 1);
        assert_eq!(short.filter(), "fps=1/1,scale=160:90:flags=bicubic,setsar=1,tile=10x10");
    }

    #[test]
    fn vtt_maps_time_to_sheet_regions() {
        let layout = Layout::new(125.0, 1280, 720);
        let track = vtt(layout, 125.0, layout.sheets());
        assert!(track.starts_with("WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nsprite_001.webp#xywh=0,0,160,90\n"));
        // Frame 11 starts the second row; frame 101 the second sheet.
        assert!(track.contains("\n00:00:10.000 --> 00:00:11.000\nsprite_001.webp#xywh=0,90,160,90\n"));
        assert!(track.contains("\n00:01:40.000 --> 00:01:41.000\nsprite_002.webp#xywh=0,0,160,90\n"));
        assert!(track.ends_with("\n00:02:04.000 --> 00:02:05.000\nsprite_002.webp#xywh=640,180,160,90\n"));
        assert_eq!(track.matches("-->").count(), 125);
    }

    #[test]
    fn vtt_never_references_missing_sheets() {
        let layout = Layout::new(250.0, 640, 360);
        assert_eq!(layout.sheets(), 3);
        let track = vtt(layout, 250.0, 2);
        assert_eq!(track.matches("-->").count(), 200);
        assert!(!track.contains("sprite_003"));
        assert_eq!(timestamp(3725.5), "01:02:05.500");
    }
}
