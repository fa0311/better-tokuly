//! Running `ffmpeg` and `ffprobe`.
//!
//! Both are external binaries found on `PATH` (or at the configured paths).
//! Every invocation that reads an upload restricts ffmpeg to the `file`
//! protocol: a crafted container may reference other URLs, and the
//! transcoder must never fetch them.
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use serde::Deserialize;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncReadExt as _, BufReader},
    process::Command,
};

use super::{ProcessError, ProcessResult, Progress};

/// Bytes of ffmpeg's stderr kept for the log when it fails.
const STDERR_TAIL: usize = 4096;

#[derive(Debug)]
pub struct Ffmpeg {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    /// Threads per process; 0 lets ffmpeg decide.
    threads: u32,
    /// Names of the filters this ffmpeg build has; asked once.
    filters: tokio::sync::OnceCell<String>,
}

/// Progress reporting for one ffmpeg run: the output position, relative to
/// `duration` seconds, is mapped onto `from..to` percent of the asset.
pub struct Stage<'a> {
    pub progress: &'a Progress,
    pub duration: f64,
    pub from: i32,
    pub to: i32,
}

/// An ordered list of ffmpeg arguments, built fluently.
#[derive(Debug, Default, Clone)]
pub struct Args(Vec<OsString>);

impl Args {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn arg(mut self, value: impl Into<OsString>) -> Self {
        self.0.push(value.into());
        self
    }

    pub fn args<I, S>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.0.extend(values.into_iter().map(Into::into));
        self
    }

    /// `-i <path>`, reading only local files.
    pub fn input(self, path: &Path) -> Self {
        self.args(["-protocol_whitelist", "file"]).arg("-i").arg(path)
    }

    /// [`Args::input`] that reads at most `seconds` of the file (`-t` on the
    /// input), whatever its header claims: what a purpose's length limit
    /// allows is all that is ever decoded or written.
    pub fn input_up_to(self, path: &Path, seconds: Option<f64>) -> Self {
        match seconds {
            Some(seconds) => self.arg("-t").arg(format!("{seconds:.3}")).input(path),
            None => self.input(path),
        }
    }

    /// `-i <playlist>` for an HLS playlist we wrote ourselves (its segments
    /// are local files; the HLS demuxer needs `crypto` allowed too).
    pub fn input_playlist(self, path: &Path) -> Self {
        self.args(["-protocol_whitelist", "file,crypto,data", "-allowed_extensions", "ALL"]).arg("-i").arg(path)
    }

    /// The arguments as text.
    #[cfg(test)]
    pub fn to_strings(&self) -> Vec<String> {
        self.0.iter().map(|arg| arg.to_string_lossy().into_owned()).collect()
    }

    /// Drops container metadata (titles, GPS position, device names) and chapters.
    pub fn strip_metadata(self) -> Self {
        self.args(["-map_metadata", "-1", "-map_chapters", "-1"])
    }
}

/// What `ffprobe` found in a file.
#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    /// Seconds; 0 when the container does not say (e.g. browser-recorded WebM).
    pub duration: f64,
    pub video: Option<VideoStream>,
    pub audio: Option<AudioStream>,
    /// An MP4/QuickTime family container (`mov,mp4,m4a,3gp…`), whose edit
    /// lists can trim what plays.
    pub mov: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoStream {
    /// Stream index within the file (for `-map 0:<index>`).
    pub index: u32,
    pub codec: String,
    /// Display size: after rotation and with square pixels.
    pub width: u32,
    pub height: u32,
    pub frame_rate: f64,
    /// HDR transfer (PQ or HLG); must be tone-mapped for SDR renditions.
    pub hdr: bool,
    pub interlaced: bool,
    /// Codec profile and level as ffprobe reports them (e.g. "High", 40).
    pub profile: String,
    pub level: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioStream {
    pub index: u32,
    pub codec: String,
    pub channels: u32,
}

#[derive(Deserialize)]
struct RawProbe {
    #[serde(default)]
    streams: Vec<RawStream>,
    #[serde(default)]
    format: RawFormat,
}

#[derive(Deserialize, Default)]
struct RawFormat {
    duration: Option<String>,
    #[serde(default)]
    format_name: String,
}

#[derive(Deserialize, Default)]
struct RawStream {
    #[serde(default)]
    index: u32,
    #[serde(default)]
    codec_type: String,
    #[serde(default)]
    codec_name: String,
    #[serde(default)]
    profile: String,
    level: Option<i32>,
    width: Option<u32>,
    height: Option<u32>,
    sample_aspect_ratio: Option<String>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    duration: Option<String>,
    color_transfer: Option<String>,
    color_primaries: Option<String>,
    color_space: Option<String>,
    field_order: Option<String>,
    channels: Option<u32>,
    #[serde(default)]
    disposition: RawDisposition,
    #[serde(default)]
    tags: RawTags,
    #[serde(default)]
    side_data_list: Vec<RawSideData>,
}

#[derive(Deserialize, Default)]
struct RawDisposition {
    #[serde(default)]
    attached_pic: i32,
}

#[derive(Deserialize, Default)]
struct RawTags {
    rotate: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawSideData {
    rotation: Option<f64>,
}

/// Parses "30000/1001" style rationals.
fn parse_ratio(value: &str) -> Option<f64> {
    let (numerator, denominator) = value.split_once(['/', ':'])?;
    let (numerator, denominator): (f64, f64) = (numerator.parse().ok()?, denominator.parse().ok()?);
    (denominator != 0.0 && numerator.is_finite()).then(|| numerator / denominator)
}

fn positive(value: Option<f64>) -> Option<f64> {
    value.filter(|v| v.is_finite() && *v > 0.0)
}

impl RawStream {
    fn rotation(&self) -> i64 {
        let degrees =
            self.side_data_list.iter().find_map(|side| side.rotation).or_else(|| self.tags.rotate.as_deref().and_then(|r| r.parse().ok())).unwrap_or(0.0);
        (degrees.round() as i64).rem_euclid(360)
    }

    /// PQ or HLG in BT.2020. Only fully tagged streams count: the tone-mapping
    /// filters cannot work from guesses, and a mis-tagged SDR file must not
    /// be "tone-mapped" into mud.
    fn is_hdr(&self) -> bool {
        matches!(self.color_transfer.as_deref(), Some("smpte2084" | "arib-std-b67"))
            && self.color_primaries.as_deref() == Some("bt2020")
            && matches!(self.color_space.as_deref(), Some("bt2020nc" | "bt2020c"))
    }

    fn to_video(&self) -> Option<VideoStream> {
        let (mut width, mut height) = (self.width?, self.height?);
        if width == 0 || height == 0 {
            return None;
        }
        // Anamorphic sources: stretch to square pixels.
        if let Some(sar) = positive(self.sample_aspect_ratio.as_deref().and_then(parse_ratio)) {
            width = (f64::from(width) * sar).round().max(1.0) as u32;
        }
        if matches!(self.rotation(), 90 | 270) {
            std::mem::swap(&mut width, &mut height);
        }
        let frame_rate = positive(self.avg_frame_rate.as_deref().and_then(parse_ratio))
            .or_else(|| positive(self.r_frame_rate.as_deref().and_then(parse_ratio)))
            .unwrap_or(30.0);
        Some(VideoStream {
            index: self.index,
            codec: self.codec_name.clone(),
            width,
            height,
            frame_rate,
            hdr: self.is_hdr(),
            interlaced: matches!(self.field_order.as_deref(), Some("tt" | "bb" | "tb" | "bt")),
            profile: self.profile.clone(),
            level: self.level.unwrap_or(0),
        })
    }
}

/// Interprets ffprobe's JSON output.
pub fn parse_probe(json: &[u8]) -> Result<Probe, String> {
    let raw: RawProbe = serde_json::from_slice(json).map_err(|e| format!("unreadable ffprobe output: {e}"))?;
    // Cover art is a "video" stream too; it is not what plays.
    let video = raw.streams.iter().filter(|s| s.codec_type == "video" && s.disposition.attached_pic == 0).find_map(RawStream::to_video);
    let audio = raw
        .streams
        .iter()
        .find(|s| s.codec_type == "audio")
        .map(|s| AudioStream { index: s.index, codec: s.codec_name.clone(), channels: s.channels.unwrap_or(0) });
    let seconds = |value: &Option<String>| positive(value.as_deref().and_then(|d| d.parse::<f64>().ok()));
    let duration = seconds(&raw.format.duration)
        .or_else(|| raw.streams.iter().filter(|s| matches!(s.codec_type.as_str(), "video" | "audio")).filter_map(|s| seconds(&s.duration)).reduce(f64::max))
        .unwrap_or(0.0);
    let mov = raw.format.format_name.split(',').any(|name| name == "mov");
    Ok(Probe { duration, video, audio, mov })
}

/// Reads `out_time_us=…` (microseconds of output written) from a line of
/// ffmpeg's `-progress` stream.
fn parse_out_time(line: &str) -> Option<f64> {
    let micros: i64 = line.strip_prefix("out_time_us=")?.trim().parse().ok()?;
    (micros >= 0).then(|| micros as f64 / 1_000_000.0)
}

fn tail(mut bytes: Vec<u8>) -> String {
    if bytes.len() > STDERR_TAIL {
        bytes.drain(..bytes.len() - STDERR_TAIL);
    }
    String::from_utf8_lossy(&bytes).trim().to_owned()
}

impl Ffmpeg {
    pub fn new(ffmpeg: PathBuf, ffprobe: PathBuf, threads: u32) -> Self {
        Self { ffmpeg, ffprobe, threads, filters: tokio::sync::OnceCell::new() }
    }

    pub fn threads(&self) -> u32 {
        self.threads
    }

    /// Whether this ffmpeg build includes the filter `name` (optional
    /// libraries such as zimg are not in every build).
    pub async fn has_filter(&self, name: &str) -> bool {
        let listing = self
            .filters
            .get_or_init(|| async {
                match Command::new(&self.ffmpeg).args(["-hide_banner", "-filters"]).stdin(Stdio::null()).output().await {
                    Ok(output) => String::from_utf8_lossy(&output.stdout).into_owned(),
                    Err(_) => String::new(),
                }
            })
            .await;
        // Lines look like " .S zscale            V->V       Apply resizing…".
        listing.lines().any(|line| line.split_whitespace().nth(1) == Some(name))
    }

    /// `-threads N` for encoders and filters, when a limit is configured.
    pub fn thread_args(&self) -> Vec<String> {
        if self.threads == 0 {
            Vec::new()
        } else {
            vec!["-threads".into(), self.threads.to_string(), "-filter_threads".into(), self.threads.to_string()]
        }
    }

    fn spawn_error(binary: &Path, error: std::io::Error) -> ProcessError {
        ProcessError::transient(format!("cannot run {} (is ffmpeg installed and on PATH?): {error}", binary.display()))
    }

    /// Inspects a media file. A file ffprobe cannot read is a rejected upload.
    pub async fn probe(&self, path: &Path) -> ProcessResult<Probe> {
        let output = Command::new(&self.ffprobe)
            .args(["-v", "error", "-protocol_whitelist", "file", "-print_format", "json", "-show_format", "-show_streams"])
            .arg(path)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| Self::spawn_error(&self.ffprobe, e))?;
        if !output.status.success() {
            return Err(ProcessError::rejected("CORRUPT_MEDIA", tail(output.stderr)));
        }
        parse_probe(&output.stdout).map_err(|detail| ProcessError::rejected("CORRUPT_MEDIA", detail))
    }

    /// Like [`Self::probe`] for files we produced ourselves (HLS playlists):
    /// a failure there is our problem, not the upload's.
    pub async fn probe_output(&self, path: &Path) -> ProcessResult<Probe> {
        let output = Command::new(&self.ffprobe)
            .args(["-v", "error", "-protocol_whitelist", "file,crypto,data", "-allowed_extensions", "ALL"])
            .args(["-print_format", "json", "-show_format", "-show_streams"])
            .arg(path)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| Self::spawn_error(&self.ffprobe, e))?;
        if !output.status.success() {
            return Err(ProcessError::transient(format!("ffprobe failed on {}: {}", path.display(), tail(output.stderr))));
        }
        parse_probe(&output.stdout).map_err(ProcessError::transient)
    }

    fn command(&self, args: Args) -> Command {
        let mut command = Command::new(&self.ffmpeg);
        command
            .args(["-hide_banner", "-nostdin", "-y", "-loglevel", "error", "-nostats"])
            .args(args.0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // A job that is dropped (cancelled, timed out) takes its ffmpeg with it.
            .kill_on_drop(true);
        command
    }

    /// Runs ffmpeg to completion. With a [`Stage`], progress is reported as
    /// the output advances, and the run stops early if the asset is deleted.
    /// Returns the seconds of output written (0 without a stage).
    ///
    /// A non-zero exit is reported as transient: it is as likely to be a full
    /// disk or a killed process as a bad input, and inputs were probed first.
    pub async fn run(&self, args: Args, stage: Option<Stage<'_>>, timeout: Duration) -> ProcessResult<f64> {
        let args = if stage.is_some() { Args::new().args(["-progress", "pipe:1"]).args(args.0) } else { args };
        let mut child = self.command(args).spawn().map_err(|e| Self::spawn_error(&self.ffmpeg, e))?;
        let stdout = child.stdout.take().expect("stdout is piped");
        let mut stderr = child.stderr.take().expect("stderr is piped");
        // Drain stderr concurrently so a talkative ffmpeg never blocks on a full pipe.
        let stderr_task = tokio::spawn(async move {
            let mut buffer = Vec::new();
            let _ = stderr.read_to_end(&mut buffer).await;
            buffer
        });

        let run = async {
            let mut written = 0.0;
            let mut lines = BufReader::new(stdout).lines();
            while let Some(line) = lines.next_line().await? {
                let Some(seconds) = parse_out_time(&line) else { continue };
                written = seconds;
                if let Some(stage) = &stage {
                    let fraction = if stage.duration > 0.0 { seconds / stage.duration } else { 0.0 };
                    stage.progress.within(stage.from, stage.to, fraction).await?;
                }
            }
            let status = child.wait().await?;
            Ok::<_, ProcessError>((status, written))
        };
        let outcome = tokio::time::timeout(timeout, run).await;
        let (status, written) = match outcome {
            Ok(Ok(done)) => done,
            // Dropping `child` on these paths kills the process.
            Ok(Err(error)) => return Err(error),
            Err(_) => return Err(ProcessError::transient(format!("ffmpeg did not finish within {timeout:?}"))),
        };
        if !status.success() {
            let stderr = stderr_task.await.unwrap_or_default();
            return Err(ProcessError::transient(format!("ffmpeg exited with {status}: {}", tail(stderr))));
        }
        Ok(written)
    }

    /// Runs ffmpeg and hands its stdout to `sink` chunk by chunk (raw PCM
    /// for waveforms). Fails like [`Self::run`].
    pub async fn capture(&self, args: Args, timeout: Duration, mut sink: impl FnMut(&[u8])) -> ProcessResult<()> {
        let mut child = self.command(args).spawn().map_err(|e| Self::spawn_error(&self.ffmpeg, e))?;
        let mut stdout = child.stdout.take().expect("stdout is piped");
        let mut stderr = child.stderr.take().expect("stderr is piped");
        let stderr_task = tokio::spawn(async move {
            let mut buffer = Vec::new();
            let _ = stderr.read_to_end(&mut buffer).await;
            buffer
        });
        let run = async {
            let mut buffer = vec![0u8; 64 * 1024];
            loop {
                let read = stdout.read(&mut buffer).await?;
                if read == 0 {
                    break;
                }
                sink(&buffer[..read]);
            }
            child.wait().await
        };
        let status = match tokio::time::timeout(timeout, run).await {
            Ok(status) => status?,
            Err(_) => return Err(ProcessError::transient(format!("ffmpeg did not finish within {timeout:?}"))),
        };
        if !status.success() {
            let stderr = stderr_task.await.unwrap_or_default();
            return Err(ProcessError::transient(format!("ffmpeg exited with {status}: {}", tail(stderr))));
        }
        Ok(())
    }

    /// How long a file really plays, measured by remuxing it to nowhere:
    /// every packet is read, nothing is decoded. Neither the header nor the
    /// edit list is believed alone: an MP4 edit list can trim playback
    /// (hiding packets) or repeat a span thousands of times (a 2 s file that
    /// plays for hours), so MP4/QuickTime files (`mov`) are measured both
    /// with it, as the encoders will read them, and without it, and the
    /// longer counts. Reading stops at `up_to` seconds when given: past a
    /// purpose's limit the exact length no longer matters.
    pub async fn measure_duration(&self, path: &Path, mov: bool, up_to: Option<f64>, timeout: Duration) -> ProcessResult<f64> {
        let played = self.remux_seconds(path, false, up_to, timeout).await?;
        if !mov {
            return Ok(played);
        }
        Ok(played.max(self.remux_seconds(path, true, up_to, timeout).await?))
    }

    /// One measuring remux; `ignore_editlist` is only known to the MP4 demuxer.
    async fn remux_seconds(&self, path: &Path, ignore_editlist: bool, up_to: Option<f64>, timeout: Duration) -> ProcessResult<f64> {
        let mut args = Args::new().args(["-progress", "pipe:1"]);
        if ignore_editlist {
            args = args.args(["-ignore_editlist", "1"]);
        }
        let args = args.input_up_to(path, up_to).args(["-map", "0", "-c", "copy", "-f", "null", "-"]);
        let mut child = self.command(args).spawn().map_err(|e| Self::spawn_error(&self.ffmpeg, e))?;
        let stdout = child.stdout.take().expect("stdout is piped");
        let run = async {
            let mut seconds = 0.0f64;
            let mut lines = BufReader::new(stdout).lines();
            while let Some(line) = lines.next_line().await? {
                if let Some(position) = parse_out_time(&line) {
                    seconds = seconds.max(position);
                }
            }
            child.wait().await.map(|status| (status, seconds))
        };
        match tokio::time::timeout(timeout, run).await {
            Ok(Ok((status, seconds))) if status.success() => Ok(seconds),
            Ok(Ok(_)) => Err(ProcessError::rejected("CORRUPT_MEDIA", "the media stream cannot be read to its end")),
            Ok(Err(error)) => Err(error.into()),
            Err(_) => Err(ProcessError::transient(format!("ffmpeg did not finish within {timeout:?}"))),
        }
    }
}

/// Generous wall-clock limit for work on `duration` seconds of media: a
/// stuck ffmpeg must not hold a worker slot forever.
pub fn timeout_for(duration: f64) -> Duration {
    Duration::from_secs_f64((duration * 30.0).clamp(600.0, 48.0 * 3600.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORTRAIT_PHONE_CLIP: &str = r#"{
        "streams": [
            {"index": 0, "codec_name": "mjpeg", "codec_type": "video", "width": 600, "height": 600, "disposition": {"attached_pic": 1}},
            {"index": 1, "codec_name": "hevc", "profile": "Main 10", "codec_type": "video", "width": 1920, "height": 1080, "level": 120,
             "sample_aspect_ratio": "1:1", "r_frame_rate": "30/1", "avg_frame_rate": "30000/1001", "color_transfer": "arib-std-b67",
             "color_primaries": "bt2020", "color_space": "bt2020nc",
             "field_order": "progressive", "duration": "12.5", "disposition": {"attached_pic": 0},
             "side_data_list": [{"side_data_type": "Display Matrix", "rotation": -90}]},
            {"index": 2, "codec_name": "aac", "codec_type": "audio", "channels": 2, "avg_frame_rate": "0/0"}
        ],
        "format": {"duration": "12.533000"}
    }"#;

    #[test]
    fn probe_reports_display_geometry() {
        let probe = parse_probe(PORTRAIT_PHONE_CLIP.as_bytes()).unwrap();
        let video = probe.video.unwrap();
        assert_eq!((video.width, video.height), (1080, 1920), "rotated 90°: portrait on screen");
        assert!((video.frame_rate - 29.97).abs() < 0.01);
        assert!(video.hdr && !video.interlaced);
        assert_eq!((video.index, video.codec.as_str()), (1, "hevc"), "the cover art stream is skipped");
        assert_eq!(probe.audio.unwrap(), AudioStream { index: 2, codec: "aac".into(), channels: 2 });
        assert!((probe.duration - 12.533).abs() < 1e-9);
    }

    #[test]
    fn probe_handles_odd_files() {
        let anamorphic = br#"{"streams":[{"codec_type":"video","codec_name":"mpeg2video","width":720,"height":576,
            "sample_aspect_ratio":"16:15","avg_frame_rate":"0/0","r_frame_rate":"25/1","field_order":"tt","tags":{"rotate":"180"}}],"format":{}}"#;
        let probe = parse_probe(anamorphic).unwrap();
        let video = probe.video.unwrap();
        assert_eq!((video.width, video.height), (768, 576));
        assert_eq!(video.frame_rate, 25.0);
        assert!(video.interlaced);
        assert_eq!(probe.duration, 0.0, "no duration recorded");
        assert!(probe.audio.is_none());

        let audio_only = br#"{"streams":[{"codec_type":"audio","codec_name":"opus","channels":1,"duration":"3.2"}],"format":{"duration":"N/A"}}"#;
        let probe = parse_probe(audio_only).unwrap();
        assert!(probe.video.is_none());
        assert_eq!(probe.duration, 3.2);

        // A transfer tag alone is not enough to treat a stream as HDR.
        let half_tagged = br#"{"streams":[{"codec_type":"video","width":64,"height":64,"color_transfer":"smpte2084"}]}"#;
        assert!(!parse_probe(half_tagged).unwrap().video.unwrap().hdr);

        assert!(parse_probe(b"not json").is_err());
        assert_eq!(parse_probe(b"{}").unwrap(), Probe { duration: 0.0, video: None, audio: None, mov: false });
    }

    #[test]
    fn progress_lines_are_parsed() {
        assert_eq!(parse_out_time("out_time_us=8933333"), Some(8.933333));
        assert_eq!(parse_out_time("out_time_us=N/A"), None);
        assert_eq!(parse_out_time("out_time_us=-9223372036854775807"), None);
        assert_eq!(parse_out_time("frame=270"), None);
        assert_eq!(parse_ratio("30000/1001").map(|f| (f * 100.0).round()), Some(2997.0));
        assert_eq!(parse_ratio("0/0"), None);
    }

    #[test]
    fn timeouts_scale_with_duration() {
        assert_eq!(timeout_for(5.0), Duration::from_secs(600));
        assert_eq!(timeout_for(3600.0), Duration::from_secs(108_000));
        assert_eq!(timeout_for(1e9), Duration::from_secs(48 * 3600));
    }
}
