//! Running `ffmpeg` and `ffprobe` for archives and clips.
use std::{path::Path, process::Stdio, time::Duration};

use tokio::process::Command;
use tokuly_core::prelude::*;

/// How much of a failed command's output is kept for the log.
const ERROR_TAIL: usize = 800;

fn failed(program: &str, detail: impl std::fmt::Display) -> AppError {
    AppError::internal(anyhow::anyhow!("{program} failed: {detail}"))
}

/// Runs a command to completion and returns its standard output. The
/// process is killed when `timeout` passes or the caller goes away.
pub async fn run(program: &str, args: &[String], timeout: Duration) -> AppResult<String> {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| failed(program, format!("cannot start ({e})")))?;
    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(output) => output.map_err(|e| failed(program, e))?,
        Err(_) => return Err(failed(program, format!("no result after {} s", timeout.as_secs()))),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: String = stderr.chars().rev().take(ERROR_TAIL).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(failed(program, format!("{}: {}", output.status, tail.trim())));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Length of a media file or URL in seconds.
pub async fn duration_of(ffprobe: &str, input: &str) -> AppResult<f64> {
    let args: Vec<String> =
        ["-v", "error", "-show_entries", "format=duration", "-of", "default=noprint_wrappers=1:nokey=1", input]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
    let out = run(ffprobe, &args, Duration::from_secs(60)).await?;
    out.trim().parse::<f64>().map_err(|_| failed(ffprobe, format!("no duration for {input}")))
}

/// The first video keyframe at or right after `at` (within `window`
/// seconds), if the input has one there.
pub async fn keyframe_at(ffprobe: &str, input: &str, at: f64, window: f64) -> AppResult<Option<f64>> {
    let interval = format!("{:.3}%+{:.3}", (at - 0.05).max(0.0), window + 0.05);
    let args: Vec<String> = [
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-skip_frame",
        "nokey",
        "-show_entries",
        "frame=pts_time",
        "-of",
        "csv=p=0",
        "-read_intervals",
        &interval,
        input,
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    let out = run(ffprobe, &args, Duration::from_secs(120)).await?;
    Ok(out
        .lines()
        .filter_map(|line| line.trim().trim_end_matches(',').parse::<f64>().ok())
        .find(|time| *time >= at - 0.05 && *time <= at + window))
}

/// A path as an ffmpeg argument.
pub fn arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// One `file` line of a concat list (single quotes escaped the way the
/// concat demuxer wants them).
pub fn concat_entry(path: &Path) -> String {
    format!("file '{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concat_entries_quote_paths() {
        assert_eq!(concat_entry(Path::new("/recordings/live/abc/1-2.mp4")), "file '/recordings/live/abc/1-2.mp4'");
        assert_eq!(concat_entry(Path::new("/tmp/it's.mp4")), "file '/tmp/it'\\''s.mp4'");
    }

    #[tokio::test]
    async fn a_missing_program_is_an_error_not_a_panic() {
        let error = run("/nonexistent/ffmpeg-for-test", &[], Duration::from_secs(5)).await.unwrap_err();
        assert_eq!(error.reason, "INTERNAL");
    }
}
