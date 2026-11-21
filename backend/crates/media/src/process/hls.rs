//! HLS playlists: reading the media playlists ffmpeg wrote and writing the
//! master playlist that ties the renditions together.
//!
//! ffmpeg can write a master playlist itself, but only for the streams of
//! one invocation and without frame rates or AV1 codec strings. We measure
//! the segments ourselves and write one master for everything.

/// What a media playlist (one rendition) contains.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaPlaylist {
    /// The `EXT-X-MAP` initialization segment.
    pub init: Option<String>,
    /// `(uri, seconds)` of every media segment, in order.
    pub segments: Vec<(String, f64)>,
}

impl MediaPlaylist {
    pub fn parse(text: &str) -> Self {
        let mut playlist = Self { init: None, segments: Vec::new() };
        let mut pending_duration: Option<f64> = None;
        for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
            if let Some(attributes) = line.strip_prefix("#EXT-X-MAP:") {
                playlist.init = attribute(attributes, "URI");
            } else if let Some(info) = line.strip_prefix("#EXTINF:") {
                pending_duration = info.split(',').next().and_then(|d| d.trim().parse().ok());
            } else if !line.starts_with('#')
                && let Some(duration) = pending_duration.take()
            {
                playlist.segments.push((line.to_owned(), duration));
            }
        }
        playlist
    }

    pub fn duration(&self) -> f64 {
        self.segments.iter().map(|(_, seconds)| seconds).sum()
    }

    /// Peak and average bitrate in bits per second, given each segment's size
    /// in bytes. HLS defines `BANDWIDTH` as the peak *segment* bitrate.
    pub fn bitrates(&self, size_of: impl Fn(&str) -> u64) -> Bitrates {
        let mut peak = 0f64;
        let mut total_bits = 0f64;
        for (uri, seconds) in &self.segments {
            let bits = size_of(uri) as f64 * 8.0;
            total_bits += bits;
            if *seconds > 0.0 {
                // Very short tail segments would exaggerate the peak.
                peak = peak.max(bits / seconds.max(1.0));
            }
        }
        let duration = self.duration();
        let average = if duration > 0.0 { total_bits / duration } else { 0.0 };
        Bitrates { peak: peak.max(average).round() as u64, average: average.round() as u64 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Bitrates {
    pub peak: u64,
    pub average: u64,
}

/// Value of `NAME` in an HLS attribute list (`A=1,NAME="x,y",B=2`).
fn attribute(attributes: &str, name: &str) -> Option<String> {
    let mut rest = attributes;
    while !rest.is_empty() {
        let (key, after_key) = rest.split_once('=')?;
        let (value, remainder) = match after_key.strip_prefix('"') {
            Some(quoted) => {
                let (value, after) = quoted.split_once('"')?;
                (value, after.strip_prefix(',').unwrap_or(after))
            }
            None => after_key.split_once(',').unwrap_or((after_key, "")),
        };
        if key.trim() == name {
            return Some(value.to_owned());
        }
        rest = remainder;
    }
    None
}

/// A video rendition as listed in the master playlist.
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    /// Media playlist, relative to the master.
    pub uri: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: f64,
    /// RFC 6381 codec string of the video (`avc1.640028`).
    pub codecs: String,
    pub bitrates: Bitrates,
}

/// The shared audio rendition.
#[derive(Debug, Clone, PartialEq)]
pub struct Audio {
    pub uri: String,
    pub codecs: String,
    pub channels: u32,
    pub bitrates: Bitrates,
}

pub const AAC_LC: &str = "mp4a.40.2";
const AUDIO_GROUP: &str = "audio";

/// RFC 6381 string for H.264: profile, constraint flags, level (`avc1.640028`
/// is High profile, level 4.0).
pub fn h264_codec_string(profile: &str, level: i32) -> String {
    let profile_and_flags = match profile {
        "Constrained Baseline" => "42e0",
        "Baseline" => "4200",
        "Main" => "4d40",
        _ => "6400", // High: what the ladder is encoded with
    };
    format!("avc1.{profile_and_flags}{:02x}", level.clamp(10, 62))
}

/// RFC 6381 string for 8-bit Main-profile AV1 (`av01.0.08M.08`); `level` is
/// the sequence level index ffprobe reports.
pub fn av1_codec_string(level: i32) -> String {
    format!("av01.0.{:02}M.08", level.clamp(0, 31))
}

/// The master playlist. `variants` are written in the given order; players
/// start with the first one.
pub fn master_playlist(variants: &[Variant], audio: Option<&Audio>) -> String {
    let mut out = String::from("#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-INDEPENDENT-SEGMENTS\n");
    if let Some(audio) = audio {
        out.push_str(&format!(
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"{AUDIO_GROUP}\",NAME=\"Audio\",DEFAULT=YES,AUTOSELECT=YES,CHANNELS=\"{}\",URI=\"{}\"\n",
            audio.channels.max(1),
            audio.uri
        ));
    }
    for variant in variants {
        // A variant's bandwidth covers everything played with it.
        let (peak, average) = match audio {
            Some(audio) => (variant.bitrates.peak + audio.bitrates.peak, variant.bitrates.average + audio.bitrates.average),
            None => (variant.bitrates.peak, variant.bitrates.average),
        };
        out.push_str(&format!(
            "#EXT-X-STREAM-INF:BANDWIDTH={peak},AVERAGE-BANDWIDTH={average},RESOLUTION={}x{},FRAME-RATE={:.3},CODECS=\"{}",
            variant.width, variant.height, variant.frame_rate, variant.codecs
        ));
        match audio {
            Some(audio) => out.push_str(&format!(",{}\",AUDIO=\"{AUDIO_GROUP}\"\n", audio.codecs)),
            None => out.push_str("\"\n"),
        }
        out.push_str(&variant.uri);
        out.push('\n');
    }
    out
}

/// Orders rungs for the master playlist: players begin with the first
/// entry, so that is the best rung of at most `start_short_side` (a safe
/// start on mobile data); the others follow from best to worst.
pub fn start_order<T>(mut rungs: Vec<T>, short_side: impl Fn(&T) -> u32, start_short_side: u32) -> Vec<T> {
    rungs.sort_by_key(|rung| std::cmp::Reverse(short_side(rung)));
    if let Some(index) = rungs.iter().position(|rung| short_side(rung) <= start_short_side) {
        let start = rungs.remove(index);
        rungs.insert(0, start);
    }
    rungs
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYLIST: &str = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n\
        #EXT-X-INDEPENDENT-SEGMENTS\n#EXT-X-MAP:URI=\"init_0.mp4\"\n#EXTINF:4.000000,\nseg_00000.m4s\n#EXTINF:4.000000,\nseg_00001.m4s\n\
        #EXTINF:1.000000,\nseg_00002.m4s\n#EXT-X-ENDLIST\n";

    #[test]
    fn media_playlists_are_parsed() {
        let playlist = MediaPlaylist::parse(PLAYLIST);
        assert_eq!(playlist.init.as_deref(), Some("init_0.mp4"));
        assert_eq!(
            playlist.segments,
            vec![("seg_00000.m4s".to_owned(), 4.0), ("seg_00001.m4s".to_owned(), 4.0), ("seg_00002.m4s".to_owned(), 1.0)]
        );
        assert_eq!(playlist.duration(), 9.0);
        assert_eq!(MediaPlaylist::parse("garbage\n#EXTINF:nope,\n"), MediaPlaylist { init: None, segments: vec![] });
    }

    #[test]
    fn bandwidth_is_the_peak_segment_bitrate() {
        let playlist = MediaPlaylist::parse(PLAYLIST);
        let bitrates = playlist.bitrates(|uri| match uri {
            "seg_00000.m4s" => 1_000_000, // 2 Mbit/s
            "seg_00001.m4s" => 500_000,   // 1 Mbit/s
            _ => 125_000,                 // 1 Mbit/s
        });
        assert_eq!(bitrates, Bitrates { peak: 2_000_000, average: 1_444_444 });
    }

    #[test]
    fn attributes_handle_quotes_and_commas() {
        let list = "BANDWIDTH=2993728,CODECS=\"avc1.64001f,mp4a.40.2\",AUDIO=\"group_aud\"";
        assert_eq!(attribute(list, "CODECS").as_deref(), Some("avc1.64001f,mp4a.40.2"));
        assert_eq!(attribute(list, "BANDWIDTH").as_deref(), Some("2993728"));
        assert_eq!(attribute(list, "AUDIO").as_deref(), Some("group_aud"));
        assert_eq!(attribute(list, "MISSING"), None);
    }

    #[test]
    fn codec_strings_follow_rfc_6381() {
        assert_eq!(h264_codec_string("High", 40), "avc1.640028");
        assert_eq!(h264_codec_string("High", 31), "avc1.64001f");
        assert_eq!(h264_codec_string("Main", 30), "avc1.4d401e");
        assert_eq!(av1_codec_string(8), "av01.0.08M.08");
    }

    #[test]
    fn master_playlist_lists_variants_with_shared_audio() {
        let variant = |uri: &str, width, height, peak| Variant {
            uri: uri.to_owned(),
            width,
            height,
            frame_rate: 30.0,
            codecs: "avc1.64001f".to_owned(),
            bitrates: Bitrates { peak, average: peak / 2 },
        };
        let audio = Audio { uri: "audio/index.m3u8".into(), codecs: AAC_LC.into(), channels: 2, bitrates: Bitrates { peak: 130_000, average: 128_000 } };
        let master = master_playlist(&[variant("720p/index.m3u8", 1280, 720, 2_000_000), variant("360p/index.m3u8", 640, 360, 800_000)], Some(&audio));
        assert_eq!(
            master,
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-INDEPENDENT-SEGMENTS\n\
             #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"Audio\",DEFAULT=YES,AUTOSELECT=YES,CHANNELS=\"2\",URI=\"audio/index.m3u8\"\n\
             #EXT-X-STREAM-INF:BANDWIDTH=2130000,AVERAGE-BANDWIDTH=1128000,RESOLUTION=1280x720,FRAME-RATE=30.000,CODECS=\"avc1.64001f,mp4a.40.2\",AUDIO=\"audio\"\n\
             720p/index.m3u8\n\
             #EXT-X-STREAM-INF:BANDWIDTH=930000,AVERAGE-BANDWIDTH=528000,RESOLUTION=640x360,FRAME-RATE=30.000,CODECS=\"avc1.64001f,mp4a.40.2\",AUDIO=\"audio\"\n\
             360p/index.m3u8\n"
        );

        let silent = master_playlist(&[variant("240p/index.m3u8", 426, 240, 400_000)], None);
        assert!(silent.contains("CODECS=\"avc1.64001f\"\n240p/index.m3u8"));
        assert!(!silent.contains("EXT-X-MEDIA"));
    }

    #[test]
    fn playback_starts_on_a_moderate_rung() {
        let order = |sides: Vec<u32>| start_order(sides, |side| *side, 720);
        assert_eq!(order(vec![240, 1080, 480, 720, 360]), vec![720, 1080, 480, 360, 240]);
        assert_eq!(order(vec![1080]), vec![1080]);
        assert_eq!(order(vec![360, 240]), vec![360, 240]);
    }
}
