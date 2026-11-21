//! The video recipe: one upload becomes an adaptive HLS stream plus the
//! extras players and feeds need.
//!
//! ```text
//! video/<asset_id>/
//!   hls/master.m3u8                 adaptive playlist (the `hls_url`)
//!   hls/<rung>/index.m3u8 …         H.264 renditions, fMP4 (CMAF) segments
//!   hls/<rung>-av1/index.m3u8 …     optional AV1 rendition of the top rung
//!   hls/audio/index.m3u8 …          AAC audio shared by all renditions
//!   storyboard/storyboard.vtt …     seek-preview track and its sprite sheets
//!   preview.webp                    short muted animated preview
//!   clip.mp4                        progressive MP4 (clips of at most 60 s)
//! img/<asset_id>/…                  poster variants (processed like any image)
//! ```
//!
//! The source is decoded once into all H.264 rungs. The extras are then cut
//! from those renditions rather than from the source: they are well-formed
//! whatever the upload looked like (seekable, upright, deinterlaced, SDR),
//! and the small ones decode in no time.
use std::path::{Path, PathBuf};

use futures::{StreamExt as _, TryStreamExt as _};
use tokuly_core::storage::Bucket;
use tokuly_proto::media::{Rendition, VideoInfo};

use super::{
    ProcessError, ProcessResult, Progress, blocking,
    ffmpeg::{Args, AudioStream, Stage, VideoStream, timeout_for},
    hls::{self, Bitrates, MediaPlaylist},
    image,
    ladder::{self, Rung, SEGMENT_SECONDS},
    storyboard,
};
use crate::{Media, assets::AssetRow, lifecycle::Processed, recipe, store};

/// Clips up to this length also get a progressive MP4.
const MP4_MAX_SECONDS: f64 = 60.0;
/// Largest source frame accepted (either edge).
const MAX_SOURCE_EDGE: u32 = 8192;
const AUDIO_BITRATE_KBPS: u32 = 128;
/// Players start on the best rung of at most this short side.
const START_RUNG: u32 = 720;
/// The animated preview: a few seconds at a low frame rate.
const PREVIEW_SECONDS: f64 = 3.0;
const PREVIEW_EDGE: u32 = 320;
const PREVIEW_FPS: u32 = 10;
/// Files uploaded to the object store in parallel.
const UPLOAD_CONCURRENCY: usize = 8;

/// Progress milestones (percent) of the recipe's stages.
mod stage {
    pub const FETCHED: i32 = 5;
    pub const TRANSCODED: i32 = 80;
    pub const EXTRAS: i32 = 90;
}

const AUDIO_DIR: &str = "audio";

/// The upload as far as the encoder is concerned.
struct Source<'a> {
    path: &'a Path,
    video: &'a VideoStream,
    audio: Option<&'a AudioStream>,
    /// Convert HDR to SDR (the renditions are 8-bit BT.709).
    tonemap: bool,
}

impl Source<'_> {
    /// Filters every decoded frame passes through before scaling, each
    /// followed by a comma (empty when none apply). Rotation needs no
    /// filter: ffmpeg applies the display matrix on its own.
    fn frame_filters(&self) -> String {
        let mut filters = String::new();
        if self.video.interlaced {
            filters.push_str("yadif,");
        }
        if self.tonemap {
            filters.push_str("zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,");
        }
        filters
    }
}

fn rung_filter(rung: &Rung) -> String {
    format!("scale={}:{}:flags=lanczos,setsar=1,fps={},format=yuv420p", rung.width, rung.height, rung.frame_rate.to_arg())
}

fn hls_output(args: Args, segment_pattern: &Path, playlist: &Path) -> Args {
    args.args(["-f", "hls", "-hls_time"])
        .arg(SEGMENT_SECONDS.to_string())
        .args(["-hls_playlist_type", "vod", "-hls_segment_type", "fmp4", "-hls_flags", "independent_segments", "-hls_segment_filename"])
        .arg(segment_pattern)
        .arg(playlist)
}

/// One ffmpeg invocation that decodes the source once and encodes every
/// H.264 rung plus the audio into `hls_dir/<name>/`.
fn ladder_args(source: &Source<'_>, rungs: &[Rung], preset: &str, threads: &[String], hls_dir: &Path) -> Args {
    // [0:v] → shared frame filters → split → one scaler per rung.
    let mut graph = format!("[0:{}]{}split={}", source.video.index, source.frame_filters(), rungs.len());
    for index in 0..rungs.len() {
        graph.push_str(&format!("[s{index}]"));
    }
    for (index, rung) in rungs.iter().enumerate() {
        graph.push_str(&format!(";[s{index}]{}[v{index}]", rung_filter(rung)));
    }

    let mut args = Args::new().input(source.path).arg("-filter_complex").arg(graph);
    for index in 0..rungs.len() {
        args = args.arg("-map").arg(format!("[v{index}]"));
    }
    if let Some(audio) = source.audio {
        args = args.arg("-map").arg(format!("0:{}", audio.index));
    }

    // Keyframes at every segment boundary, in every rung, and no others the
    // encoder might add at scene cuts: renditions must be switchable.
    args = args
        .args(["-c:v", "libx264", "-preset", preset, "-profile:v", "high", "-sc_threshold", "0", "-force_key_frames"])
        .arg(format!("expr:gte(t,n_forced*{SEGMENT_SECONDS})"));
    let mut stream_map = Vec::new();
    for (index, rung) in rungs.iter().enumerate() {
        let gop = rung.frame_rate.frames_per_segment().to_string();
        args = args
            .arg(format!("-crf:v:{index}"))
            .arg(rung.crf.to_string())
            .arg(format!("-maxrate:v:{index}"))
            .arg(format!("{}k", rung.max_bitrate_kbps))
            .arg(format!("-bufsize:v:{index}"))
            .arg(format!("{}k", rung.max_bitrate_kbps * 2))
            .arg(format!("-g:v:{index}"))
            .arg(&gop)
            .arg(format!("-keyint_min:v:{index}"))
            .arg(&gop);
        stream_map.push(match source.audio {
            Some(_) => format!("v:{index},agroup:aud,name:{}", rung.name),
            None => format!("v:{index},name:{}", rung.name),
        });
    }
    if source.audio.is_some() {
        args = args.args(["-c:a", "aac", "-b:a"]).arg(format!("{AUDIO_BITRATE_KBPS}k")).args(["-ac", "2", "-ar", "48000"]);
        stream_map.push(format!("a:0,agroup:aud,name:{AUDIO_DIR},default:yes"));
    }
    args = args.args(threads).strip_metadata().args(["-max_muxing_queue_size", "4096", "-var_stream_map"]).arg(stream_map.join(" "));
    hls_output(args, &hls_dir.join("%v").join("seg_%05d.m4s"), &hls_dir.join("%v").join("index.m3u8"))
}

/// The AV1 rendition of one rung (video only; it shares the AAC audio).
fn av1_args(source: &Source<'_>, rung: &Rung, threads: u32, out_dir: &Path) -> Args {
    let gop = rung.frame_rate.frames_per_segment();
    let mut params = format!("keyint={gop}:scd=0");
    if threads > 0 {
        params.push_str(&format!(":lp={threads}"));
    }
    let args = Args::new()
        .input(source.path)
        .arg("-map")
        .arg(format!("0:{}", source.video.index))
        .args(["-an", "-vf"])
        .arg(format!("{}{}", source.frame_filters(), rung_filter(rung)))
        .args(["-c:v", "libsvtav1", "-preset", "8", "-crf", "32", "-g"])
        .arg(gop.to_string())
        .arg("-svtav1-params")
        .arg(params)
        .strip_metadata();
    hls_output(args, &out_dir.join("seg_%05d.m4s"), &out_dir.join("index.m3u8"))
}

/// A representative frame from early in the video, taken from the top rung.
/// (Seeking in arbitrary uploads is unreliable; our own renditions are not.)
fn poster_args(rendition: &Path, duration: f64, out: &Path) -> Args {
    let at = (duration * 0.1).min(10.0);
    Args::new()
        .input_playlist(rendition)
        .args(["-an", "-vf"])
        // `thumbnail` picks the most typical of the next 24 frames, which
        // avoids black frames and mid-transition blurs.
        .arg(format!("trim=start={at:.3},thumbnail=24,setsar=1"))
        .args(["-frames:v", "1"])
        .strip_metadata()
        .arg(out)
}

fn storyboard_args(rendition: &Path, layout: storyboard::Layout, out_dir: &Path) -> Args {
    Args::new()
        .input_playlist(rendition)
        .args(["-an", "-vf"])
        .arg(layout.filter())
        .args(["-c:v", "libwebp", "-quality", "60", "-start_number", "1"])
        .arg(out_dir.join("sprite_%03d.webp"))
}

fn preview_args(rendition: &Path, rung: &Rung, duration: f64, out: &Path) -> Args {
    // Skip intros on longer videos; short clips preview from the start.
    let start = if duration > 4.0 * PREVIEW_SECONDS { (duration * 0.2).min(30.0) } else { 0.0 };
    let (width, height) = image::scaled_dimensions(rung.width, rung.height, PREVIEW_EDGE);
    Args::new()
        .input_playlist(rendition)
        .args(["-an", "-vf"])
        // Cut with a filter rather than by seeking the input: exact, and it
        // works on single-segment playlists where seeking yields nothing.
        .arg(format!(
            "trim=start={start:.3}:duration={PREVIEW_SECONDS},setpts=PTS-STARTPTS,fps={PREVIEW_FPS},scale={}:{}:flags=lanczos",
            width.max(2) / 2 * 2,
            height.max(2) / 2 * 2
        ))
        .args(["-c:v", "libwebp_anim", "-quality", "55", "-loop", "0"])
        .strip_metadata()
        .arg(out)
}

/// Remuxes (no re-encode) the top rung and the audio into a fast-start MP4.
fn mp4_args(video: &Path, audio: Option<&Path>, out: &Path) -> Args {
    let mut args = Args::new().input_playlist(video);
    if let Some(audio) = audio {
        args = args.input_playlist(audio);
    }
    args = args.args(["-map", "0:v:0"]);
    if audio.is_some() {
        args = args.args(["-map", "1:a:0"]);
    }
    args.args(["-c", "copy", "-movflags", "+faststart"]).strip_metadata().arg(out)
}

fn content_type_of(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("m3u8") => "application/vnd.apple.mpegurl",
        Some("m4s") => "video/iso.segment",
        Some("mp4") => "video/mp4",
        Some("vtt") => "text/vtt",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}

/// Every file below `dir`, as paths relative to it.
fn list_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                walk(root, &path, out)?;
            } else if let Ok(relative) = path.strip_prefix(root) {
                out.push(relative.to_path_buf());
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(dir, dir, &mut files)?;
    files.sort();
    Ok(files)
}

/// One finished HLS rendition, measured from the files ffmpeg wrote.
struct Measured {
    /// Directory name below `hls/`.
    dir: String,
    width: u32,
    height: u32,
    frame_rate: f64,
    codec: &'static str,
    codecs: String,
    bitrates: Bitrates,
    short_side: u32,
}

impl Media {
    pub(crate) async fn process_video(&self, asset: &AssetRow, progress: &Progress) -> ProcessResult<Processed> {
        let core = &self.core;
        let scratch = self.scratch_dir()?;
        let work = scratch.path();
        let path = self.fetch_original(asset, work).await?;
        progress.set(stage::FETCHED).await?;

        let probe = self.ffmpeg.probe(&path).await?;
        let Some(video) = probe.video else {
            return Err(ProcessError::rejected("NOT_A_VIDEO", "the file has no video stream"));
        };
        if video.width.max(video.height) > MAX_SOURCE_EDGE {
            return Err(ProcessError::rejected("VIDEO_TOO_LARGE", format!("{}x{} frames", video.width, video.height)));
        }
        let duration = if probe.duration > 0.0 { probe.duration } else { self.ffmpeg.measure_duration(&path, timeout_for(0.0)).await? };
        if duration <= 0.0 {
            return Err(ProcessError::rejected("CORRUPT_MEDIA", "the video has no measurable duration"));
        }
        if duration > self.cfg.max_video_seconds() {
            return Err(ProcessError::rejected("MEDIA_TOO_LONG", format!("{duration:.0} s")));
        }
        let timeout = timeout_for(duration);
        let tonemap = video.hdr && self.ffmpeg.has_filter("zscale").await && self.ffmpeg.has_filter("tonemap").await;
        if video.hdr && !tonemap {
            tracing::warn!(asset_id = %asset.id, "HDR source but this ffmpeg lacks zscale/tonemap: colours will look washed out");
        }
        let source = Source { path: &path, video: &video, audio: probe.audio.as_ref(), tonemap };
        let rungs = ladder::ladder(video.width, video.height, video.frame_rate, recipe::max_rung(asset.asset_purpose()));

        // A retry starts from a clean slate, never mixing two attempts' files.
        crate::lifecycle::remove_renditions(core, asset.id).await?;
        let hls_dir = work.join("hls");
        tokio::fs::create_dir_all(&hls_dir).await?;

        // ── transcode ──
        let h264_end = if self.cfg.av1 { (stage::FETCHED + stage::TRANSCODED) / 2 } else { stage::TRANSCODED };
        let threads = self.ffmpeg.thread_args();
        let ladder = ladder_args(&source, &rungs, &self.cfg.x264_preset, &threads, &hls_dir);
        self.ffmpeg.run(ladder, Some(Stage { progress, duration, from: stage::FETCHED, to: h264_end }), timeout).await?;
        let mut renditions = Vec::with_capacity(rungs.len() + 1);
        for rung in &rungs {
            renditions.push(self.measure_rendition(&hls_dir, &rung.name, "h264").await?);
        }
        if self.cfg.av1 {
            let name = format!("{}-av1", rungs[0].name);
            let out_dir = hls_dir.join(&name);
            tokio::fs::create_dir_all(&out_dir).await?;
            let av1 = av1_args(&source, &rungs[0], self.ffmpeg.threads(), &out_dir);
            self.ffmpeg.run(av1, Some(Stage { progress, duration, from: h264_end, to: stage::TRANSCODED }), timeout).await?;
            renditions.push(self.measure_rendition(&hls_dir, &name, "av1").await?);
        }
        let audio = match source.audio {
            Some(_) => Some(self.measure_audio(&hls_dir).await?),
            None => None,
        };
        progress.set(stage::TRANSCODED).await?;

        // ── master playlist ──
        let (h264, av1): (Vec<&Measured>, Vec<&Measured>) = renditions.iter().partition(|r| r.codec == "h264");
        let ordered = hls::start_order(h264, |r| r.short_side, START_RUNG).into_iter().chain(av1);
        let variants: Vec<hls::Variant> = ordered
            .map(|r| hls::Variant {
                uri: format!("{}/index.m3u8", r.dir),
                width: r.width,
                height: r.height,
                frame_rate: r.frame_rate,
                codecs: r.codecs.clone(),
                bitrates: r.bitrates,
            })
            .collect();
        tokio::fs::write(hls_dir.join("master.m3u8"), hls::master_playlist(&variants, audio.as_ref())).await?;

        // ── extras ──
        let top = &rungs[0];
        let smallest = rungs.last().expect("a ladder has at least one rung");
        let playlist_of = |rung: &Rung| hls_dir.join(&rung.name).join("index.m3u8");

        let poster_path = work.join("poster.png");
        self.ffmpeg.run(poster_args(&playlist_of(top), duration, &poster_path), None, timeout).await?;
        let poster_bytes = tokio::fs::read(&poster_path).await?;
        let poster = self.publish_image(asset.id, blocking(move || image::decode(&poster_bytes)).await?).await?;

        let storyboard_dir = work.join("storyboard");
        tokio::fs::create_dir_all(&storyboard_dir).await?;
        let layout = storyboard::Layout::new(duration, smallest.width, smallest.height);
        self.ffmpeg.run(storyboard_args(&playlist_of(smallest), layout, &storyboard_dir), None, timeout).await?;
        let sheets = (list_files(&storyboard_dir)?.len() as u32).min(layout.sheets());
        tokio::fs::write(storyboard_dir.join("storyboard.vtt"), storyboard::vtt(layout, duration, sheets)).await?;

        self.ffmpeg.run(preview_args(&playlist_of(smallest), smallest, duration, &work.join("preview.webp")), None, timeout).await?;

        let has_mp4 = duration <= MP4_MAX_SECONDS;
        let audio_playlist = hls_dir.join(AUDIO_DIR).join("index.m3u8");
        if has_mp4 {
            let audio_input = audio.is_some().then_some(audio_playlist.as_path());
            self.ffmpeg.run(mp4_args(&playlist_of(top), audio_input, &work.join("clip.mp4")), None, timeout).await?;
        }
        let loudness_lufs = if audio.is_some() { self.measure_loudness(Args::new().input_playlist(&audio_playlist), timeout).await? } else { 0.0 };
        progress.set(stage::EXTRAS).await?;

        // ── publish ──
        // The master playlist goes last: nothing points at a rendition that
        // is not completely there.
        let prefix = store::video_prefix(asset.id);
        let mut uploads: Vec<(PathBuf, String)> = Vec::new();
        for relative in list_files(&hls_dir)?.into_iter().filter(|file| file != Path::new("master.m3u8")) {
            uploads.push((hls_dir.join(&relative), format!("{prefix}hls/{}", relative.to_string_lossy())));
        }
        for relative in list_files(&storyboard_dir)? {
            uploads.push((storyboard_dir.join(&relative), format!("{prefix}storyboard/{}", relative.to_string_lossy())));
        }
        uploads.push((work.join("preview.webp"), format!("{prefix}preview.webp")));
        if has_mp4 {
            uploads.push((work.join("clip.mp4"), format!("{prefix}clip.mp4")));
        }
        self.upload_files(uploads, progress, stage::EXTRAS, 99).await?;
        let master_key = format!("{prefix}hls/master.m3u8");
        core.storage.put_file(Bucket::Media, &master_key, &hls_dir.join("master.m3u8"), content_type_of(Path::new("master.m3u8")), Some(store::IMMUTABLE)).await?;

        // ── describe ──
        let url = |key: String| core.storage.media_url(&key);
        let mut described: Vec<Rendition> = renditions
            .iter()
            .map(|r| Rendition {
                name: r.dir.clone(),
                width: r.width as i32,
                height: r.height as i32,
                bitrate_kbps: (r.bitrates.average / 1000) as i32,
                codec: r.codec.to_owned(),
                url: url(format!("{prefix}hls/{}/index.m3u8", r.dir)),
                ..Default::default()
            })
            .collect();
        if let Some(audio) = &audio {
            described.push(Rendition {
                name: AUDIO_DIR.to_owned(),
                bitrate_kbps: (audio.bitrates.average / 1000) as i32,
                codec: "aac".to_owned(),
                url: url(format!("{prefix}hls/{AUDIO_DIR}/index.m3u8")),
                ..Default::default()
            });
        }
        let hls_url = url(master_key);
        let mp4_url = if has_mp4 { url(format!("{prefix}clip.mp4")) } else { String::new() };
        let info = VideoInfo {
            duration_seconds: duration,
            width: top.width as i32,
            height: top.height as i32,
            frame_rate: top.frame_rate.as_f64(),
            hls_url: hls_url.clone(),
            mp4_url: mp4_url.clone(),
            poster: Some(poster.clone()).into(),
            storyboard_url: url(format!("{prefix}storyboard/storyboard.vtt")),
            renditions: described,
            has_audio: audio.is_some(),
            preview_url: url(format!("{prefix}preview.webp")),
            loudness_lufs,
            ..Default::default()
        };
        Ok(Processed {
            // Short clips play anywhere from the MP4; everything else streams.
            url: if has_mp4 { mp4_url } else { hls_url },
            image: Some(poster),
            video: Some(info),
            ..Default::default()
        })
    }

    /// Reads back one rendition ffmpeg wrote: its real codec level, size and
    /// measured bitrates (the encoder only promised a ceiling).
    async fn measure_rendition(&self, hls_dir: &Path, name: &str, codec: &'static str) -> ProcessResult<Measured> {
        let dir = hls_dir.join(name);
        let playlist_path = dir.join("index.m3u8");
        let probe = self.ffmpeg.probe_output(&playlist_path).await?;
        let Some(video) = probe.video else {
            return Err(ProcessError::transient(format!("rendition {name} has no video")));
        };
        let playlist = MediaPlaylist::parse(&tokio::fs::read_to_string(&playlist_path).await?);
        let bitrates = playlist.bitrates(|uri| std::fs::metadata(dir.join(uri)).map(|m| m.len()).unwrap_or(0));
        let codecs = if codec == "av1" { hls::av1_codec_string(video.level) } else { hls::h264_codec_string(&video.profile, video.level) };
        Ok(Measured {
            dir: name.to_owned(),
            width: video.width,
            height: video.height,
            frame_rate: video.frame_rate,
            codec,
            codecs,
            bitrates,
            short_side: video.width.min(video.height),
        })
    }

    async fn measure_audio(&self, hls_dir: &Path) -> ProcessResult<hls::Audio> {
        let dir = hls_dir.join(AUDIO_DIR);
        let playlist = MediaPlaylist::parse(&tokio::fs::read_to_string(dir.join("index.m3u8")).await?);
        let bitrates = playlist.bitrates(|uri| std::fs::metadata(dir.join(uri)).map(|m| m.len()).unwrap_or(0));
        Ok(hls::Audio { uri: format!("{AUDIO_DIR}/index.m3u8"), codecs: hls::AAC_LC.to_owned(), channels: 2, bitrates })
    }

    /// Uploads `(local path, object key)` pairs to the media bucket, a few
    /// at a time, reporting progress across `from..to` percent.
    pub(crate) async fn upload_files(&self, files: Vec<(PathBuf, String)>, progress: &Progress, from: i32, to: i32) -> ProcessResult<()> {
        let total = files.len().max(1);
        let storage = &self.core.storage;
        let mut uploads = futures::stream::iter(files)
            .map(|(path, key)| async move { storage.put_file(Bucket::Media, &key, &path, content_type_of(&path), Some(store::IMMUTABLE)).await })
            .buffer_unordered(UPLOAD_CONCURRENCY);
        let mut done = 0usize;
        while uploads.try_next().await?.is_some() {
            done += 1;
            progress.within(from, to, done as f64 / total as f64).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::ladder::ladder;

    fn video(interlaced: bool) -> VideoStream {
        VideoStream { index: 0, codec: "h264".into(), width: 1280, height: 720, frame_rate: 30.0, hdr: false, interlaced, profile: "High".into(), level: 31 }
    }

    fn rendered(args: Args) -> Vec<String> {
        args.to_strings()
    }

    #[test]
    fn ladder_command_encodes_every_rung_with_aligned_keyframes() {
        let video = video(false);
        let audio = AudioStream { index: 1, codec: "aac".into(), channels: 2 };
        let source = Source { path: Path::new("/work/source"), video: &video, audio: Some(&audio), tonemap: false };
        let rungs = ladder(1280, 720, 30.0, 1080);
        let args = rendered(ladder_args(&source, &rungs, "veryfast", &[], Path::new("/work/hls")));
        let joined = args.join(" ");
        assert!(joined.contains("-protocol_whitelist file -i /work/source"), "{joined}");
        assert!(joined.contains("[0:0]split=4[s0][s1][s2][s3];[s0]scale=1280:720:flags=lanczos,setsar=1,fps=30/1,format=yuv420p[v0];"), "{joined}");
        assert!(joined.contains("-force_key_frames expr:gte(t,n_forced*4)"));
        assert!(joined.contains("-g:v:0 120 -keyint_min:v:0 120"));
        assert!(joined.contains("-maxrate:v:1 1500k -bufsize:v:1 3000k"));
        assert!(joined.contains("-map_metadata -1"), "container metadata (GPS position) must not be copied");
        assert!(joined.contains("v:0,agroup:aud,name:720p v:1,agroup:aud,name:480p v:2,agroup:aud,name:360p v:3,agroup:aud,name:240p a:0,agroup:aud,name:audio,default:yes"));
        assert!(joined.ends_with("-hls_segment_filename /work/hls/%v/seg_%05d.m4s /work/hls/%v/index.m3u8"), "{joined}");
    }

    #[test]
    fn silent_and_special_sources_change_the_command() {
        let interlaced = video(true);
        let source = Source { path: Path::new("/s"), video: &interlaced, audio: None, tonemap: true };
        let joined = rendered(ladder_args(&source, &ladder(1280, 720, 30.0, 1080)[..1], "fast", &["-threads".into(), "2".into()], Path::new("/h"))).join(" ");
        assert!(joined.contains("[0:0]yadif,zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,split=1[s0]"));
        assert!(!joined.contains("-c:a") && !joined.contains("agroup"), "no audio stream to encode: {joined}");
        assert!(joined.contains("-threads 2"));
    }

    #[test]
    fn files_get_their_streaming_content_types() {
        assert_eq!(content_type_of(Path::new("hls/720p/index.m3u8")), "application/vnd.apple.mpegurl");
        assert_eq!(content_type_of(Path::new("seg_00001.m4s")), "video/iso.segment");
        assert_eq!(content_type_of(Path::new("init_0.mp4")), "video/mp4");
        assert_eq!(content_type_of(Path::new("storyboard.vtt")), "text/vtt");
        assert_eq!(content_type_of(Path::new("sprite_001.webp")), "image/webp");
    }
}
