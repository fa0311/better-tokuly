//! Recordings: what the media server writes to disk, the archive made from
//! it, and keeping the disk tidy.
//!
//! MediaMTX records every published stream as fragmented MP4 into
//! `<TOKULY_LIVE_RECORDINGS_DIR>/live/<short_id>/<unix seconds>-<microseconds>.mp4`
//! (`recordPath: …/%path/%s-%f`): one folder per stream, one file per
//! segment, named after the moment the segment starts. The directory is
//! shared with the worker, which
//!
//! - **archives**: when a stream ends with `auto_archive`, the
//!   `live.archive` job joins the segments from the moment the stream went
//!   on air (stream copy, no re-encode), stores the result as a media asset
//!   (`ASSET_PURPOSE_LIVE_ARCHIVE`), creates the video on the channel
//!   (`tokuly_video::videos::create_from_asset`) and deletes the segments;
//! - **cuts clips** of a stream on air out of them ([`crate::clips`]);
//! - **sweeps** ([`Live::sweep_recordings`]): recordings of streams that
//!   are gone, that will not be archived or whose archive exists are
//!   deleted; a stream on air that will not be archived keeps only its DVR
//!   window; failed archives are kept for `recordings_max_age_hours` so
//!   they can be retried, then dropped. The same sweep erases deleted
//!   streams whose retention period is over and finishes gifts whose
//!   payment was interrupted.
//!
//! Working files live in `<recordings dir>/.work/`, on the same volume as
//! the recordings: an archive can be many gigabytes.
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use tokuly_core::{events as core_events, prelude::*};
use tokuly_media::assets;
use tokuly_proto::{common::Visibility, live::live_event::Event, media::AssetPurpose, video::VideoOrigin};
use tokuly_video::videos::{self, NewVideo};

use crate::{
    Live, LiveConfig,
    events::{ArchiveJob, JOB_ARCHIVE, STREAM_ARCHIVED, StreamArchived},
    ffmpeg,
    streams::{self, ARCHIVE_FAILED, ARCHIVE_NONE, ARCHIVE_PENDING, ARCHIVE_PROCESSING, ARCHIVE_READY, StreamRow},
};

/// Longest an archive may take to assemble.
const ARCHIVE_TIMEOUT: Duration = Duration::from_secs(4 * 3600);
/// The last part of a recording reaches the disk about this long after the
/// encoder stopped.
const FLUSH_SECONDS: i64 = 3;
/// An archive in PENDING or PROCESSING this long without news is queued again.
const ARCHIVE_STUCK_MINUTES: i32 = 15;
/// A clip still PROCESSING after this long has failed.
const CLIP_STUCK_MINUTES: i32 = 30;
/// Slack on top of what must be kept when trimming segments.
const TRIM_SLACK: Duration = Duration::from_secs(120);

/// One recorded segment.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub path: PathBuf,
    /// When the segment starts.
    pub start: Time,
}

/// What a sweep did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Sweep {
    pub removed_recordings: u32,
    pub trimmed_segments: u32,
    pub requeued_archives: u32,
    pub failed_clips: u32,
    /// Deleted streams whose retention period is over.
    pub erased_streams: u32,
    /// Gifts whose interrupted payment was finished.
    pub settled_gifts: u32,
}

/// The folder a stream is recorded into.
pub fn dir_of(cfg: &LiveConfig, short_id: &str) -> PathBuf {
    Path::new(&cfg.recordings_dir).join("live").join(short_id)
}

/// A folder for working files.
pub fn work_dir(cfg: &LiveConfig, name: &str) -> PathBuf {
    Path::new(&cfg.recordings_dir).join(".work").join(name)
}

/// The start time in a segment's file name (`<unix seconds>-<microseconds>.mp4`).
pub fn segment_start(file_name: &str) -> Option<Time> {
    let stem = file_name.strip_suffix(".mp4")?;
    let (seconds, micros) = stem.split_once('-')?;
    let seconds: i64 = seconds.parse().ok()?;
    let micros: u32 = micros.parse().ok()?;
    (micros < 1_000_000).then(|| chrono::DateTime::from_timestamp(seconds, micros * 1000)).flatten()
}

/// The recorded segments of a stream, oldest first (none when nothing was
/// recorded or the directory cannot be read).
pub async fn segments(cfg: &LiveConfig, short_id: &str) -> Vec<Segment> {
    let mut found = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(dir_of(cfg, short_id)).await else { return found };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if let Some(start) = entry.file_name().to_str().and_then(segment_start) {
            found.push(Segment { path: entry.path(), start });
        }
    }
    found.sort_by_key(|segment| segment.start);
    found
}

/// Deletes the recording of a stream. Missing folders are fine.
pub async fn remove(cfg: &LiveConfig, short_id: &str) {
    if short_id.is_empty() || !short_id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return;
    }
    let dir = dir_of(cfg, short_id);
    match tokio::fs::remove_dir_all(&dir).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => tracing::warn!(dir = %dir.display(), %error, "could not delete a recording"),
    }
}

async fn remove_work_dir(dir: &Path) {
    if let Err(error) = tokio::fs::remove_dir_all(dir).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(dir = %dir.display(), %error, "could not delete working files");
    }
}

fn io_error(context: &str, error: std::io::Error) -> AppError {
    AppError::internal(anyhow::anyhow!("{context}: {error}"))
}

/// The part of each segment that belongs to `[from, to]`, as the lines of
/// an ffmpeg concat list. `durations` are the segments' lengths in seconds.
/// Returns the lines and the seconds they cover.
pub fn concat_list(segments: &[Segment], durations: &[f64], from: Time, to: Time) -> (Vec<String>, f64) {
    let mut lines = vec!["ffconcat version 1.0".to_owned()];
    let mut covered = 0.0;
    for (segment, duration) in segments.iter().zip(durations) {
        let end = segment.start + chrono::Duration::milliseconds((duration * 1000.0) as i64);
        if end <= from || segment.start >= to {
            continue;
        }
        lines.push(ffmpeg::concat_entry(&segment.path));
        let skip = (from - segment.start).num_milliseconds() as f64 / 1000.0;
        if skip > 0.0 {
            lines.push(format!("inpoint {skip:.3}"));
        }
        if end > to {
            lines.push(format!("outpoint {:.3}", (to - segment.start).num_milliseconds() as f64 / 1000.0));
        }
        covered += (end.min(to) - segment.start.max(from)).num_milliseconds() as f64 / 1000.0;
    }
    (lines, covered)
}

impl Live {
    /// Handles one `live.archive` job: turns the recording of an ended
    /// stream into a video on its channel. Safe to run again: it picks up
    /// where an earlier attempt stopped and does nothing once the video
    /// exists.
    pub async fn archive(&self, job: ArchiveJob) -> AppResult<()> {
        let core = &self.core;
        let Some(stream) = streams::find_by_id(core, job.stream_id).await? else { return Ok(()) };
        let wanted = matches!(stream.archive_status, ARCHIVE_PENDING | ARCHIVE_PROCESSING | ARCHIVE_FAILED);
        if !stream.is_ended() || !wanted || stream.archive_video_id.is_some() {
            return Ok(());
        }
        // One attempt at a time per stream, even if the job is delivered twice.
        let lock = format!("live:archive:{}", stream.id);
        if !core.cache.set_nx(&lock, "1", ARCHIVE_TIMEOUT).await? {
            return Ok(());
        }
        sqlx::query(
            "UPDATE live_streams SET archive_status = $2, archive_error = '', archive_changed_at = now() WHERE id = $1",
        )
        .bind(stream.id)
        .bind(ARCHIVE_PROCESSING)
        .execute(&core.db)
        .await?;
        let result = self.run_archive(&stream).await;
        if let Err(error) = &result {
            tracing::error!(stream_id = %stream.id, %error, "archiving a stream failed");
            let message: String = error.to_string().chars().take(500).collect();
            sqlx::query("UPDATE live_streams SET archive_status = $2, archive_error = $3, archive_changed_at = now() WHERE id = $1")
                .bind(stream.id)
                .bind(ARCHIVE_FAILED)
                .bind(message)
                .execute(&core.db)
                .await?;
        }
        core.cache.del(&lock).await?;
        if let Some(updated) = streams::find_by_id(core, stream.id).await?
            && let Ok(summary) = streams::summary(core, &updated).await
        {
            self.push_to_user(updated.owner_id, Event::StreamUpdated(Box::new(summary))).await;
        }
        result
    }

    async fn run_archive(&self, stream: &StreamRow) -> AppResult<()> {
        let core = &self.core;
        // The upload of an earlier attempt is reused.
        let existing = match stream.archive_asset_id {
            Some(asset_id) => assets::find(core, asset_id).await?.map(|asset| asset.id),
            None => None,
        };
        let asset_id = match existing {
            Some(asset_id) => asset_id,
            None => {
                let work = work_dir(&self.cfg, &stream.id.to_string());
                let assembled = self.assemble_recording(stream, &work).await;
                let file = match assembled {
                    Ok(Some(file)) => file,
                    Ok(None) => {
                        // The encoder never sent anything worth keeping.
                        sqlx::query(
                            "UPDATE live_streams SET archive_status = $2, archive_error = 'nothing was recorded', archive_changed_at = now()
                             WHERE id = $1",
                        )
                        .bind(stream.id)
                        .bind(ARCHIVE_NONE)
                        .execute(&core.db)
                        .await?;
                        remove_work_dir(&work).await;
                        remove(&self.cfg, &stream.short_id).await;
                        return Ok(());
                    }
                    Err(error) => {
                        remove_work_dir(&work).await;
                        return Err(error);
                    }
                };
                let stored = tokuly_media::create_asset_from_file(
                    core,
                    stream.owner_id,
                    AssetPurpose::ASSET_PURPOSE_LIVE_ARCHIVE,
                    &file,
                    &format!("{}.mp4", stream.short_id),
                )
                .await;
                remove_work_dir(&work).await;
                let asset = stored?;
                sqlx::query("UPDATE live_streams SET archive_asset_id = $2 WHERE id = $1")
                    .bind(stream.id)
                    .bind(asset.id)
                    .execute(&core.db)
                    .await?;
                asset.id
            }
        };

        // Assembling and uploading took a while. The stream is read again:
        // one that was deleted meanwhile must not come back as a video, and
        // the video gets the stream's title and visibility as they are now.
        let Some(stream) = streams::find_by_id(core, stream.id).await? else {
            assets::delete(core, asset_id, None).await?;
            return Ok(());
        };
        let stream = &stream;

        let visibility = buffa::EnumValue::<Visibility>::from(stream.visibility)
            .as_known()
            .unwrap_or(Visibility::VISIBILITY_PRIVATE);
        // The stream's thumbnail becomes the video's, if it is still there.
        let thumbnail_asset_id = match stream.thumbnail_asset_id {
            Some(id) => assets::require_image_json(core, id, stream.owner_id, &[AssetPurpose::ASSET_PURPOSE_THUMBNAIL])
                .await
                .is_ok()
                .then_some(id),
            None => None,
        };
        let video_id = videos::create_from_asset(
            core,
            NewVideo {
                description: stream.description.clone(),
                tags: stream.tags.clone(),
                category: stream.category,
                game: stream.game.clone(),
                visibility,
                // Subscribers were told when the stream went on air.
                notify_subscribers: false,
                thumbnail_asset_id,
                origin: VideoOrigin::VIDEO_ORIGIN_LIVE_ARCHIVE,
                origin_id: Some(stream.id),
                publish: true,
                ..NewVideo::new(stream.channel_id, asset_id, stream.title.clone())
            },
        )
        .await?;

        let mut tx = core.db.begin().await?;
        let recorded = sqlx::query(
            "UPDATE live_streams SET archive_status = $2, archive_video_id = $3, archive_error = '', archive_changed_at = now()
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(stream.id)
        .bind(ARCHIVE_READY)
        .bind(video_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        if !recorded {
            // Deleted in the last moment: the video that was just made goes too.
            drop(tx);
            if let Some(video) = videos::find_by_id(core, video_id).await? {
                videos::delete(core, &video, None, true).await?;
            }
            return Ok(());
        }
        // Clips cut while the stream was on air now also belong to the video.
        sqlx::query("UPDATE live_clips SET video_id = $2 WHERE stream_id = $1 AND video_id IS NULL")
            .bind(stream.id)
            .bind(video_id)
            .execute(&mut *tx)
            .await?;
        let archived =
            StreamArchived { stream_id: stream.id, video_id, channel_id: stream.channel_id, owner_id: stream.owner_id };
        core_events::emit(&mut *tx, STREAM_ARCHIVED, None, &archived).await?;
        tx.commit().await?;
        remove(&self.cfg, &stream.short_id).await;
        Ok(())
    }

    /// Joins the recorded segments of a stream, from the moment it went on
    /// air to its end, into one MP4 in `work`. `None` when nothing (worth
    /// keeping) was recorded.
    async fn assemble_recording(&self, stream: &StreamRow, work: &Path) -> AppResult<Option<PathBuf>> {
        let (Some(from), Some(to)) = (stream.started_at, stream.ended_at) else { return Ok(None) };
        // Give the media server a moment to finish the last segment.
        let settle = (to + chrono::Duration::seconds(FLUSH_SECONDS) - now()).num_milliseconds();
        if settle > 0 {
            tokio::time::sleep(Duration::from_millis(settle as u64)).await;
        }
        let found = segments(&self.cfg, &stream.short_id).await;
        let mut usable = Vec::new();
        let mut durations = Vec::new();
        for segment in found {
            match ffmpeg::duration_of(&self.cfg.ffprobe_path, &ffmpeg::arg(&segment.path)).await {
                Ok(duration) if duration > 0.0 => {
                    usable.push(segment);
                    durations.push(duration);
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(segment = %segment.path.display(), %error, "skipping an unreadable recording segment")
                }
            }
        }
        let (lines, covered) = concat_list(&usable, &durations, from, to);
        if covered < 0.5 {
            return Ok(None);
        }
        tokio::fs::create_dir_all(work).await.map_err(|e| io_error("cannot create the archive work directory", e))?;
        let list = work.join("segments.txt");
        tokio::fs::write(&list, lines.join("\n") + "\n")
            .await
            .map_err(|e| io_error("cannot write the segment list", e))?;
        let out = work.join("archive.mp4");
        let args: Vec<String> = [
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            &ffmpeg::arg(&list),
            "-c",
            "copy",
            "-movflags",
            "+faststart",
            &ffmpeg::arg(&out),
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        ffmpeg::run(&self.cfg.ffmpeg_path, &args, ARCHIVE_TIMEOUT).await?;
        Ok(Some(out))
    }

    /// Keeps the recordings directory tidy and the archive and clip jobs
    /// honest (see the module documentation). Runs every few minutes.
    pub async fn sweep_recordings(&self) -> AppResult<Sweep> {
        let core = &self.core;
        let cfg = &self.cfg;
        let mut sweep = Sweep::default();
        let max_age = chrono::Duration::hours(cfg.recordings_max_age_hours);

        if let Ok(mut entries) = tokio::fs::read_dir(Path::new(&cfg.recordings_dir).join("live")).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let Some(short_id) = entry.file_name().to_str().map(str::to_owned) else { continue };
                if streams::short_id_of_path(&streams::media_path(&short_id)).is_none() {
                    continue;
                }
                let stream = streams::find(core, &tokuly_core::ids::Ref::Short(short_id.clone())).await?;
                let drop_all = match &stream {
                    None => true,
                    Some(stream) if stream.is_ended() => {
                        let ended = stream.ended_at.unwrap_or(stream.updated_at);
                        match stream.archive_status {
                            ARCHIVE_PENDING | ARCHIVE_PROCESSING => false,
                            // Kept for a while so the archive can be retried.
                            ARCHIVE_FAILED => ended + max_age < now(),
                            _ => ended + chrono::Duration::seconds(60) < now(),
                        }
                    }
                    Some(_) => false,
                };
                if drop_all {
                    remove(cfg, &short_id).await;
                    sweep.removed_recordings += 1;
                    continue;
                }
                // Not over: a stream that will not be archived keeps only
                // what rewinding needs; nothing is kept past the maximum age.
                let Some(stream) = stream.filter(|stream| !stream.is_ended()) else { continue };
                let keep = if stream.auto_archive {
                    Duration::from_secs(cfg.recordings_max_age_hours as u64 * 3600)
                } else if stream.dvr_enabled {
                    Duration::from_secs(cfg.dvr_window_seconds as u64) + TRIM_SLACK
                } else {
                    TRIM_SLACK
                };
                for segment in segments(cfg, &short_id).await {
                    let age = tokio::fs::metadata(&segment.path)
                        .await
                        .ok()
                        .and_then(|meta| meta.modified().ok())
                        .and_then(|modified| SystemTime::now().duration_since(modified).ok());
                    if age.is_some_and(|age| age > keep) && tokio::fs::remove_file(&segment.path).await.is_ok() {
                        sweep.trimmed_segments += 1;
                    }
                }
            }
        }

        // Working files nobody came back for.
        if let Ok(mut entries) = tokio::fs::read_dir(Path::new(&cfg.recordings_dir).join(".work")).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let stale = entry
                    .metadata()
                    .await
                    .ok()
                    .and_then(|meta| meta.modified().ok())
                    .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                    .is_some_and(|age| age > Duration::from_secs(86_400));
                if stale {
                    remove_work_dir(&entry.path()).await;
                }
            }
        }

        // Archives whose job was lost.
        let stuck: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM live_streams
             WHERE archive_status IN (2, 3) AND deleted_at IS NULL AND archive_changed_at < now() - make_interval(mins => $1)",
        )
        .bind(ARCHIVE_STUCK_MINUTES)
        .fetch_all(&core.db)
        .await?;
        for stream_id in stuck {
            sqlx::query("UPDATE live_streams SET archive_changed_at = now() WHERE id = $1")
                .bind(stream_id)
                .execute(&core.db)
                .await?;
            core.jobs.enqueue(JOB_ARCHIVE, &ArchiveJob { stream_id }).await?;
            sweep.requeued_archives += 1;
        }

        sweep.failed_clips = self.fail_stuck_clips(CLIP_STUCK_MINUTES).await? as u32;
        sweep.erased_streams = streams::purge_deleted(core, cfg.deleted_retention_days).await? as u32;
        sweep.settled_gifts = self.settle_pending_gifts(None).await as u32;
        Ok(sweep)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64) -> Time {
        chrono::DateTime::from_timestamp(1_800_000_000 + seconds, 0).unwrap()
    }

    #[test]
    fn segment_names_carry_their_start_time() {
        let start = segment_start("1795307619-871082.mp4").unwrap();
        assert_eq!(start.timestamp(), 1_795_307_619);
        assert_eq!(start.timestamp_subsec_micros(), 871_082);
        for name in ["1795307619.mp4", "x-1.mp4", "1795307619-871082.ts", "1795307619-9999999.mp4", "segments.txt", ""]
        {
            assert!(segment_start(name).is_none(), "{name}");
        }
    }

    #[test]
    fn recordings_live_under_the_configured_directory() {
        let cfg = LiveConfig { recordings_dir: "/rec".into(), ..Default::default() };
        assert_eq!(dir_of(&cfg, "abcDEF12345"), PathBuf::from("/rec/live/abcDEF12345"));
        assert_eq!(work_dir(&cfg, "x"), PathBuf::from("/rec/.work/x"));
    }

    #[test]
    fn the_archive_covers_the_time_on_air_only() {
        let segments = vec![
            Segment { path: "/rec/live/a/0.mp4".into(), start: at(0) },
            Segment { path: "/rec/live/a/1.mp4".into(), start: at(600) },
            Segment { path: "/rec/live/a/2.mp4".into(), start: at(1300) },
        ];
        let durations = [600.0, 600.0, 100.0];
        // The encoder connected at 0 (preview); on air from 30 to 1350.
        let (lines, covered) = concat_list(&segments, &durations, at(30), at(1350));
        assert_eq!(
            lines,
            vec![
                "ffconcat version 1.0",
                "file '/rec/live/a/0.mp4'",
                "inpoint 30.000",
                "file '/rec/live/a/1.mp4'",
                "file '/rec/live/a/2.mp4'",
                "outpoint 50.000",
            ]
        );
        // 570 s + 600 s + 50 s; the 100 s gap between the second and third segment is not media.
        assert!((covered - 1220.0).abs() < 0.001);
        // A stream that went on air after everything recorded has nothing to archive.
        let (lines, covered) = concat_list(&segments, &durations, at(2000), at(2100));
        assert_eq!((lines.len(), covered), (1, 0.0));
    }
}
