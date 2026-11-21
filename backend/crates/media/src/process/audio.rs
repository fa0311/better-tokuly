//! The audio recipe (voice messages): AAC for everything that plays MP4,
//! Opus for the web, and a waveform to draw while it plays.
//!
//! ```text
//! audio/<asset_id>/voice.m4a     AAC-LC, fast start (the asset's `url`)
//! audio/<asset_id>/voice.webm    Opus
//! ```
use std::time::Duration;

use tokuly_core::storage::Bucket;
use tokuly_proto::media::{Rendition, VideoInfo};

use super::{
    ProcessError, ProcessResult, Progress,
    ffmpeg::{Args, Stage, timeout_for},
};
use crate::{Media, assets::AssetRow, lifecycle::Processed, store};

/// Bars in a voice message's waveform.
pub const WAVEFORM_BARS: usize = 64;
/// Waveforms are computed from mono PCM at this rate, in windows of 10 ms.
const WAVEFORM_SAMPLE_RATE: u32 = 8000;
const WINDOW_SAMPLES: usize = 80;
/// A recording that never rises above this (of 32 768) is drawn flat.
const SILENCE_FLOOR: u16 = 64;

const AAC_BITRATE_KBPS: u32 = 96;
const OPUS_BITRATE_KBPS: u32 = 48;

/// Reduces a stream of 16-bit little-endian mono PCM to one peak per window
/// without keeping the samples.
#[derive(Debug, Default)]
pub struct PeakCollector {
    peaks: Vec<u16>,
    window_peak: u16,
    window_fill: usize,
    /// First byte of a sample split across two chunks.
    carry: Option<u8>,
    samples: u64,
}

impl PeakCollector {
    pub fn feed(&mut self, mut bytes: &[u8]) {
        if let (Some(low), Some((&high, rest))) = (self.carry, bytes.split_first()) {
            self.carry = None;
            self.sample(i16::from_le_bytes([low, high]));
            bytes = rest;
        }
        let (pairs, remainder) = bytes.as_chunks::<2>();
        for pair in pairs {
            self.sample(i16::from_le_bytes(*pair));
        }
        if let [low] = remainder {
            self.carry = Some(*low);
        }
    }

    fn sample(&mut self, value: i16) {
        self.samples += 1;
        self.window_peak = self.window_peak.max(value.unsigned_abs());
        self.window_fill += 1;
        if self.window_fill == WINDOW_SAMPLES {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if self.window_fill > 0 {
            self.peaks.push(self.window_peak);
            self.window_peak = 0;
            self.window_fill = 0;
        }
    }

    /// The window peaks and the number of samples seen.
    pub fn finish(mut self) -> (Vec<u16>, u64) {
        self.flush();
        (self.peaks, self.samples)
    }
}

/// Condenses window peaks into `bars` values of 0–255, scaled so the loudest
/// bar is 255: a quiet recording still shows its shape. Silence is all zeros.
pub fn waveform(peaks: &[u16], bars: usize) -> Vec<u8> {
    if peaks.is_empty() || bars == 0 {
        return Vec::new();
    }
    let bar_peaks: Vec<u16> = (0..bars)
        .map(|bar| {
            let start = bar * peaks.len() / bars;
            let end = ((bar + 1) * peaks.len() / bars).max(start + 1).min(peaks.len());
            peaks[start.min(peaks.len() - 1)..end].iter().copied().max().unwrap_or(0)
        })
        .collect();
    let loudest = bar_peaks.iter().copied().max().unwrap_or(0);
    if loudest < SILENCE_FLOOR {
        return vec![0; bars];
    }
    bar_peaks.into_iter().map(|peak| ((u32::from(peak) * 255 + u32::from(loudest) / 2) / u32::from(loudest)) as u8).collect()
}

/// Reads the last integrated-loudness value from the output of ffmpeg's
/// `ebur128` + `ametadata=print` filters.
fn parse_loudness(output: &str) -> Option<f64> {
    let value = output.rsplit("lavfi.r128.I=").next()?.lines().next()?.trim();
    value.parse::<f64>().ok().filter(|lufs| lufs.is_finite())
}

impl Media {
    pub(crate) async fn process_audio(&self, asset: &AssetRow, progress: &Progress) -> ProcessResult<Processed> {
        let core = &self.core;
        let scratch = self.scratch_dir()?;
        let work = scratch.path();
        let path = self.fetch_original(asset, work).await?;
        progress.set(10).await?;

        let probe = self.ffmpeg.probe(&path).await?;
        let Some(audio) = probe.audio else {
            return Err(ProcessError::rejected("NO_AUDIO", "the file has no audio stream"));
        };
        let stream = format!("0:{}", audio.index);

        // Decoding for the waveform also tells the real duration, which
        // browser recordings do not store in the file.
        let mut collector = PeakCollector::default();
        let pcm = Args::new()
            .input(&path)
            .args(["-map", &stream, "-vn", "-ac", "1", "-ar"])
            .arg(WAVEFORM_SAMPLE_RATE.to_string())
            .args(["-f", "s16le", "-"]);
        self.ffmpeg.capture(pcm, timeout_for(probe.duration), |chunk| collector.feed(chunk)).await?;
        let (peaks, samples) = collector.finish();
        let decoded_seconds = samples as f64 / f64::from(WAVEFORM_SAMPLE_RATE);
        let duration = if probe.duration > 0.0 { probe.duration } else { decoded_seconds };
        if samples == 0 || duration <= 0.0 {
            return Err(ProcessError::rejected("CORRUPT_MEDIA", "the audio cannot be decoded"));
        }
        if duration > self.cfg.max_video_seconds() {
            return Err(ProcessError::rejected("MEDIA_TOO_LONG", format!("{duration:.0} s")));
        }
        let timeout = timeout_for(duration);
        progress.set(30).await?;

        let (m4a, webm) = (work.join("voice.m4a"), work.join("voice.webm"));
        let channels = audio.channels.clamp(1, 2).to_string();
        let encode = Args::new()
            .input(&path)
            .args(["-map", &stream, "-vn", "-c:a", "aac", "-b:a"])
            .arg(format!("{AAC_BITRATE_KBPS}k"))
            .args(["-ac", &channels, "-ar", "48000", "-movflags", "+faststart"])
            .strip_metadata()
            .arg(&m4a)
            .args(["-map", &stream, "-vn", "-c:a", "libopus", "-b:a"])
            .arg(format!("{OPUS_BITRATE_KBPS}k"))
            .args(["-ac", &channels])
            .strip_metadata()
            .arg(&webm);
        self.ffmpeg.run(encode, Some(Stage { progress, duration, from: 30, to: 80 }), timeout).await?;
        let loudness_lufs = self.measure_loudness(Args::new().input(&m4a), timeout).await?;

        crate::lifecycle::remove_renditions(core, asset.id).await?;
        let prefix = store::audio_prefix(asset.id);
        let (m4a_key, webm_key) = (format!("{prefix}voice.m4a"), format!("{prefix}voice.webm"));
        let storage = &core.storage;
        storage.put_file(Bucket::Media, &m4a_key, &m4a, "audio/mp4", Some(store::IMMUTABLE)).await?;
        storage.put_file(Bucket::Media, &webm_key, &webm, "audio/webm", Some(store::IMMUTABLE)).await?;

        let m4a_url = storage.media_url(&m4a_key);
        let rendition = |name: &str, kbps: u32, url: String| Rendition {
            name: name.to_owned(),
            codec: name.to_owned(),
            bitrate_kbps: kbps as i32,
            url,
            ..Default::default()
        };
        let info = VideoInfo {
            duration_seconds: duration,
            has_audio: true,
            mp4_url: m4a_url.clone(),
            renditions: vec![rendition("aac", AAC_BITRATE_KBPS, m4a_url.clone()), rendition("opus", OPUS_BITRATE_KBPS, storage.media_url(&webm_key))],
            loudness_lufs,
            ..Default::default()
        };
        Ok(Processed { url: m4a_url, video: Some(info), waveform: Some(waveform(&peaks, WAVEFORM_BARS)), ..Default::default() })
    }

    /// Integrated loudness (LUFS, EBU R 128) of the first audio stream of
    /// `input`. 0 when it cannot be measured.
    pub(crate) async fn measure_loudness(&self, input: Args, timeout: Duration) -> ProcessResult<f64> {
        let args = input.args(["-map", "0:a:0", "-vn", "-af", "ebur128=metadata=1,ametadata=mode=print:key=lavfi.r128.I:file=-", "-f", "null", "-"]);
        // The filter prints a value for every 100 ms; only the last one
        // (the whole programme) matters, so keep just the tail.
        let mut tail: Vec<u8> = Vec::new();
        self.ffmpeg
            .capture(args, timeout, |chunk| {
                tail.extend_from_slice(chunk);
                if tail.len() > 1024 {
                    tail.drain(..tail.len() - 512);
                }
            })
            .await?;
        Ok(parse_loudness(&String::from_utf8_lossy(&tail)).unwrap_or(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm(samples: &[i16]) -> Vec<u8> {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    #[test]
    fn peaks_are_collected_across_chunk_boundaries() {
        let mut samples = vec![0i16; 200];
        samples[10] = -20_000;
        samples[95] = 12_345;
        let bytes = pcm(&samples);
        let whole = {
            let mut collector = PeakCollector::default();
            collector.feed(&bytes);
            collector.finish()
        };
        // Odd-sized chunks split samples in half.
        let pieces = {
            let mut collector = PeakCollector::default();
            for chunk in bytes.chunks(7) {
                collector.feed(chunk);
            }
            collector.finish()
        };
        assert_eq!(whole, (vec![20_000, 12_345, 0], 200));
        assert_eq!(pieces, whole);
    }

    #[test]
    fn waveform_is_normalised_to_the_loudest_bar() {
        let peaks: Vec<u16> = (0..640).map(|i| if i < 320 { 1000 } else { 4000 }).collect();
        let bars = waveform(&peaks, 64);
        assert_eq!(bars.len(), 64);
        assert!(bars[..32].iter().all(|b| *b == 64), "{bars:?}");
        assert!(bars[32..].iter().all(|b| *b == 255));
    }

    #[test]
    fn waveform_handles_silence_and_tiny_inputs() {
        assert_eq!(waveform(&[0; 100], 64), vec![0; 64]);
        assert_eq!(waveform(&[10, 20, 30], 64), vec![0; 64], "below the noise floor");
        assert!(waveform(&[], 64).is_empty());
        let short = waveform(&[100, 200], 4);
        assert_eq!(short, vec![128, 128, 255, 255], "fewer windows than bars: bars repeat");
    }

    #[test]
    fn loudness_is_read_from_the_last_report() {
        let output = "frame:88   pts:422400  pts_time:8.8\nlavfi.r128.I=-23.1\nframe:89   pts:427200  pts_time:8.9\nlavfi.r128.I=-21.787\n";
        assert_eq!(parse_loudness(output), Some(-21.787));
        assert_eq!(parse_loudness("…28.I=-30.5\nframe:9 pts:1 pts_time:1\nlavfi.r128.I=-18.25"), Some(-18.25));
        assert_eq!(parse_loudness("nothing"), None);
        assert_eq!(parse_loudness("lavfi.r128.I=nan"), None);
    }
}
