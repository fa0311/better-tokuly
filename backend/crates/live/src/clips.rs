//! Clips: short cuts (0.1-60 s) of a stream on air or of a video.
//!
//! A clip is cut in the background (`live.clip` job): from the recording
//! segments for a stream on air, from the video's processed renditions
//! otherwise. The cut copies the stream when it starts on a keyframe and
//! re-encodes when it does not. The result is stored as a media asset of
//! the clip's creator; when the media domain has processed it
//! (`media.asset.ready`) the clip is READY.
//!
//! A clip has no visibility of its own: whoever may watch its source may
//! watch the clip. A clip of a stream on air is linked to the stream; once
//! the stream has its archive video the clip is linked to that too and
//! follows the video from then on.
//!
//! # For other domains
//!
//! The shorts domain makes shorts from clips: [`find`] resolves a clip by
//! UUID or short ID, and [`ClipRow`] names its upload (`asset_id`, a READY
//! video asset once `status` is READY). The upload belongs to the owner of
//! the channel the clip was cut from (`owner_id`), not to whoever cut it:
//! the cutter gets a clip, not a file of someone else's broadcast among
//! their own uploads to reuse as they like. Whether a short may be made
//! from a clip is for the shorts domain to decide with [`source_of`].
use std::{collections::HashMap, time::Duration};

use tokuly_core::{events as core_events, ids::Ref, outbound, prelude::*, storage::Bucket};
use tokuly_media::assets;
use tokuly_proto::{
    common::{ChannelSummary, Image, UserSummary},
    live::{Clip, ClipStatus, live_event::Event},
    media::{AssetPurpose, VideoInfo},
};
use tokuly_video::{
    access::Viewer,
    channels,
    videos::{self, VideoRow},
};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::{
    Live, access,
    events::{CLIP_DELETED, CLIP_READY, ClipEvent, ClipJob, JOB_CLIP},
    ffmpeg, lifecycle, recordings, search,
    streams::{self, AssetRemoval, StreamRow, Tx},
};

pub const MAX_TITLE: usize = 64;
pub const MIN_SECONDS: f64 = 0.1;
pub const MAX_SECONDS: f64 = 60.0;

pub const PROCESSING: i32 = ClipStatus::CLIP_STATUS_PROCESSING as i32;
pub const READY: i32 = ClipStatus::CLIP_STATUS_READY as i32;
pub const FAILED: i32 = ClipStatus::CLIP_STATUS_FAILED as i32;

/// A cut that starts within this many seconds of a keyframe is copied
/// from that keyframe instead of being re-encoded.
const KEYFRAME_TOLERANCE: f64 = 0.1;
const CUT_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ClipRow {
    pub id: Uuid,
    pub short_id: String,
    /// Who cut it (and owns its upload).
    pub creator_id: Uuid,
    /// The channel it was cut from, and that channel's owner.
    pub channel_id: Uuid,
    pub owner_id: Uuid,
    /// The source: a stream, a video, or both once a stream has its archive.
    pub stream_id: Option<Uuid>,
    pub video_id: Option<Uuid>,
    pub title: String,
    /// Start in the source (the stream's media timeline, or seconds into
    /// the video) and length.
    pub start_seconds: f64,
    pub duration_seconds: f64,
    /// `tokuly.live.v1.ClipStatus`.
    pub status: i32,
    pub error: String,
    /// The clip's upload once it is cut.
    pub asset_id: Option<Uuid>,
    /// `tokuly.media.v1.VideoInfo` / `tokuly.common.v1.Image` as proto JSON.
    pub playback: Option<serde_json::Value>,
    pub thumbnail: Option<serde_json::Value>,
    pub like_count: i64,
    pub created_at: Time,
    pub updated_at: Time,
}

macro_rules! clip_columns {
    () => {
        "c.id, c.short_id, c.creator_id, c.channel_id, c.owner_id, c.stream_id, c.video_id, c.title, c.start_seconds, \
         c.duration_seconds, c.status, c.error, c.asset_id, c.playback, c.thumbnail, c.like_count, c.created_at, c.updated_at"
    };
}
pub(crate) use clip_columns;

impl ClipRow {
    pub fn is_ready(&self) -> bool {
        self.status == READY
    }

    /// Whether the channel's owner cut it from their own channel: such a
    /// clip is the channel's, and does not name its creator to others.
    pub fn is_channel_clip(&self) -> bool {
        self.creator_id == self.owner_id
    }

    /// Path of the clip page in the web app.
    pub fn path(&self) -> String {
        format!("/clip/{}", self.short_id)
    }

    pub fn playback(&self) -> Option<VideoInfo> {
        self.playback.clone().and_then(|value| serde_json::from_value(value).ok())
    }

    pub fn thumbnail(&self) -> Option<Image> {
        assets::image_from_json(self.thumbnail.as_ref())
    }

    pub fn thumbnail_url(&self) -> Option<String> {
        self.thumbnail().map(|image| image.url).filter(|url| !url.is_empty())
    }

    fn event(&self) -> ClipEvent {
        ClipEvent {
            clip_id: self.id,
            short_id: self.short_id.clone(),
            creator_id: self.creator_id,
            channel_id: self.channel_id,
            stream_id: self.stream_id,
            video_id: self.video_id,
            asset_id: self.asset_id,
        }
    }
}

pub(crate) fn not_found() -> AppError {
    AppError::not_found("CLIP_NOT_FOUND", "clip not found")
}

/// Holder under which a clip keeps its upload alive.
pub fn holder(clip_id: Uuid) -> String {
    format!("live:clip:{clip_id}")
}

/// A clip title: 1 to 64 characters.
pub fn validate_title(input: &str) -> AppResult<String> {
    let title = input.split_whitespace().collect::<Vec<_>>().join(" ");
    ensure!(
        (1..=MAX_TITLE).contains(&title.graphemes(true).count()) && !title.chars().any(char::is_control),
        invalid,
        "CLIP_TITLE_INVALID",
        "a clip title is 1 to {MAX_TITLE} characters"
    );
    Ok(title)
}

/// A clip range: starts at or after 0 and is 0.1 to 60 seconds long.
pub fn validate_range(start_seconds: f64, duration_seconds: f64) -> AppResult<()> {
    ensure!(
        start_seconds.is_finite() && start_seconds >= 0.0 && (MIN_SECONDS..=MAX_SECONDS).contains(&duration_seconds),
        invalid,
        "CLIP_RANGE_INVALID",
        "a clip is {MIN_SECONDS} to {MAX_SECONDS} seconds long"
    );
    Ok(())
}

// ───────────────────────────── lookups ─────────────────────────────

/// A clip by UUID or short ID, whatever its state. Callers decide who may
/// see it (its source's visibility: [`source_of`]).
pub async fn find(core: &Core, reference: &Ref) -> AppResult<Option<ClipRow>> {
    Ok(match reference {
        Ref::Id(id) => {
            sqlx::query_as(concat!("SELECT ", clip_columns!(), " FROM live_clips c WHERE c.id = $1"))
                .bind(id)
                .fetch_optional(&core.db)
                .await?
        }
        Ref::Short(short_id) => {
            sqlx::query_as(concat!("SELECT ", clip_columns!(), " FROM live_clips c WHERE c.short_id = $1"))
                .bind(short_id)
                .fetch_optional(&core.db)
                .await?
        }
    })
}

pub async fn find_by_id(core: &Core, clip_id: Uuid) -> AppResult<Option<ClipRow>> {
    find(core, &Ref::Id(clip_id)).await
}

/// A clip by what clients send as `clip_id`.
pub async fn find_ref(core: &Core, reference: &str) -> AppResult<Option<ClipRow>> {
    match Ref::parse(reference.trim()) {
        Ok(reference) => find(core, &reference).await,
        Err(_) => Ok(None),
    }
}

async fn find_by_asset(core: &Core, asset_id: Uuid) -> AppResult<Option<ClipRow>> {
    Ok(sqlx::query_as(concat!("SELECT ", clip_columns!(), " FROM live_clips c WHERE c.asset_id = $1"))
        .bind(asset_id)
        .fetch_optional(&core.db)
        .await?)
}

/// What a clip was cut from, as far as it still exists.
#[derive(Debug, Clone)]
pub struct Source {
    pub title: String,
    pub visibility: i32,
    /// Short IDs for links.
    pub stream_short_id: Option<String>,
    pub video_short_id: Option<String>,
    /// Whether the viewer may watch the source (and with it the clip).
    pub watchable: bool,
    /// Whether everyone may find it (search).
    pub public: bool,
}

/// Looks up the sources of clips, remembering what it already resolved.
#[derive(Default)]
pub struct Sources {
    videos: HashMap<Uuid, Option<VideoRow>>,
    streams: HashMap<Uuid, Option<StreamRow>>,
}

impl Sources {
    async fn video(&mut self, core: &Core, video_id: Uuid) -> AppResult<Option<VideoRow>> {
        if !self.videos.contains_key(&video_id) {
            self.videos.insert(video_id, videos::find_by_id(core, video_id).await?);
        }
        Ok(self.videos.get(&video_id).cloned().flatten())
    }

    async fn stream(&mut self, core: &Core, stream_id: Uuid) -> AppResult<Option<StreamRow>> {
        if !self.streams.contains_key(&stream_id) {
            self.streams.insert(stream_id, streams::find_by_id(core, stream_id).await?);
        }
        Ok(self.streams.get(&stream_id).cloned().flatten())
    }

    /// The source of `clip` as `viewer` sees it; `None` when it is gone.
    /// The video decides once there is one, the stream before that.
    pub async fn of(&mut self, core: &Core, viewer: &Viewer, clip: &ClipRow) -> AppResult<Option<Source>> {
        let stream = match clip.stream_id {
            Some(stream_id) => self.stream(core, stream_id).await?,
            None => None,
        };
        if let Some(video_id) = clip.video_id {
            let Some(video) = self.video(core, video_id).await? else { return Ok(None) };
            return Ok(Some(Source {
                title: video.title.clone(),
                visibility: video.visibility,
                stream_short_id: stream.map(|stream| stream.short_id),
                video_short_id: Some(video.short_id.clone()),
                watchable: viewer.can_watch(&video.facts()),
                public: video.is_public(),
            }));
        }
        Ok(stream.map(|stream| Source {
            title: stream.title.clone(),
            visibility: stream.visibility,
            watchable: access::can_watch(viewer, &stream.facts()),
            public: stream.visibility == access::PUBLIC && !stream.suspended,
            stream_short_id: Some(stream.short_id),
            video_short_id: None,
        }))
    }
}

/// The source of one clip as `viewer` sees it.
pub async fn source_of(core: &Core, viewer: &Viewer, clip: &ClipRow) -> AppResult<Option<Source>> {
    Sources::default().of(core, viewer, clip).await
}

/// Whether `viewer` may see `clip` given its source: its creator and the
/// owner of the channel it was cut from always may (they manage it).
fn visible(viewer: &Viewer, clip: &ClipRow, source: &Source) -> bool {
    viewer.is(clip.creator_id) || viewer.is(clip.owner_id) || (source.watchable && !viewer.hides(clip.creator_id))
}

/// Clips as `viewer` may see them, in order: clips whose source is gone or
/// not theirs to watch are left out, and so are clips that are not READY
/// unless they are the viewer's own.
pub async fn to_protos(core: &Core, viewer: &Viewer, rows: &[ClipRow]) -> AppResult<Vec<Clip>> {
    let mut sources = Sources::default();
    let mut shown: Vec<(&ClipRow, Source)> = Vec::new();
    for row in rows {
        let Some(source) = sources.of(core, viewer, row).await? else { continue };
        if visible(viewer, row, &source) && (row.is_ready() || viewer.is(row.creator_id)) {
            shown.push((row, source));
        }
    }
    let mut creator_ids: Vec<Uuid> = shown.iter().map(|(row, _)| row.creator_id).collect();
    creator_ids.sort_unstable();
    creator_ids.dedup();
    let creators: HashMap<Uuid, UserSummary> = tokuly_identity::summaries(core, &creator_ids).await?;
    let mut channel_ids: Vec<Uuid> = shown.iter().map(|(row, _)| row.channel_id).collect();
    channel_ids.sort_unstable();
    channel_ids.dedup();
    let channels: HashMap<Uuid, ChannelSummary> = channels::summaries(core, &channel_ids).await?;
    Ok(shown
        .into_iter()
        .map(|(row, source)| Clip {
            id: row.id.to_string(),
            short_id: row.short_id.clone(),
            title: row.title.clone(),
            // A channel owner cutting their own channel speaks as the channel.
            creator: (!row.is_channel_clip() || viewer.is(row.creator_id))
                .then(|| tokuly_identity::users::pick(&creators, row.creator_id))
                .into(),
            channel: channels.get(&row.channel_id).cloned().into(),
            stream_id: source.stream_short_id.unwrap_or_default(),
            video_id: source.video_short_id.unwrap_or_default(),
            source_title: source.title,
            start_seconds: row.start_seconds,
            duration_seconds: row.duration_seconds,
            status: row.status.into(),
            playback: if row.is_ready() { row.playback() } else { None }.into(),
            thumbnail: row.thumbnail().into(),
            visibility: source.visibility.into(),
            like_count: row.like_count,
            created_at: ts(row.created_at),
            can_delete: viewer.is(row.creator_id) || viewer.is(row.owner_id),
            // What posting it as a short takes; that is the creator's alone.
            asset_id: row
                .asset_id
                .filter(|_| row.is_ready() && viewer.is(row.creator_id))
                .map(|id| id.to_string())
                .unwrap_or_default(),
            ..Default::default()
        })
        .collect())
}

/// One clip as `viewer` sees it (`CLIP_NOT_FOUND` when they may not).
pub async fn to_proto(core: &Core, viewer: &Viewer, row: &ClipRow) -> AppResult<Clip> {
    to_protos(core, viewer, std::slice::from_ref(row)).await?.pop().ok_or_else(not_found)
}

/// Which clips a listing is about.
#[derive(Debug, Clone, Copy)]
pub enum Filter {
    Stream(Uuid),
    Video(Uuid),
    Channel(Uuid),
    /// The clips someone cut. Those they cut from their own channel
    /// ([`ClipRow::is_channel_clip`]) only with `with_channel_clips` (the
    /// creator asking): to everyone else they are the channel's, and listing
    /// them by person would name the channel's owner.
    Creator {
        id: Uuid,
        with_channel_clips: bool,
    },
}

/// Clips matching `filter` before `before`, newest first.
pub async fn list(core: &Core, filter: Filter, before: Option<Uuid>, limit: i64) -> AppResult<Vec<ClipRow>> {
    let (column, id, channel_clips) = match filter {
        Filter::Stream(id) => ("c.stream_id", id, true),
        Filter::Video(id) => ("c.video_id", id, true),
        Filter::Channel(id) => ("c.channel_id", id, true),
        Filter::Creator { id, with_channel_clips } => ("c.creator_id", id, with_channel_clips),
    };
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {} FROM live_clips c WHERE {column} = $1 AND ($2::uuid IS NULL OR c.id < $2)
           AND ($4 OR c.creator_id <> c.owner_id)
         ORDER BY c.id DESC LIMIT $3",
        clip_columns!()
    )))
    .bind(id)
    .bind(before)
    .bind(limit)
    .bind(channel_clips)
    .fetch_all(&core.db)
    .await?)
}

// ───────────────────────────── create / delete ─────────────────────────────

/// What a new clip is cut from.
pub enum NewSource {
    /// A stream on air: cut from its recording.
    Stream(StreamRow),
    /// A video (optionally the archive of `stream`).
    Video { video: VideoRow, stream_id: Option<Uuid> },
}

/// Deletes a clip inside `tx`. Returns the upload to remove once the
/// transaction committed (`force`: remove it whatever else holds it).
pub async fn delete_in(
    tx: &mut Tx<'_>,
    clip: &ClipRow,
    actor: Option<Uuid>,
    force: bool,
) -> AppResult<Option<AssetRemoval>> {
    assets::detach_holder(&mut **tx, &holder(clip.id)).await?;
    sqlx::query("DELETE FROM live_clips WHERE id = $1").bind(clip.id).execute(&mut **tx).await?;
    outbound::unindex(&mut **tx, "clip", clip.id).await?;
    core_events::emit(&mut **tx, CLIP_DELETED, actor, &clip.event()).await?;
    Ok(clip
        .asset_id
        .map(|asset_id| AssetRemoval { asset_id, owner_id: if force { None } else { Some(clip.owner_id) } }))
}

/// The URL of a processed media file as this process reaches it: files in
/// the media bucket are read from the storage service directly instead of
/// through the public edge.
fn internal_media_url(core: &Core, public_url: &str) -> String {
    let base = format!("{}/", core.cfg.media_url());
    match public_url.strip_prefix(&base) {
        Some(key) => {
            format!("{}/{}/{key}", core.cfg.s3_endpoint.trim_end_matches('/'), core.storage.bucket_name(Bucket::Media))
        }
        None => public_url.to_owned(),
    }
}

/// What ffmpeg reads to cut a clip, and where the cut starts in it.
struct CutInput {
    args: Vec<String>,
    position: f64,
    /// A single file or URL that can be probed for keyframes.
    probe: Option<String>,
}

impl Live {
    /// Creates a clip (PROCESSING) and queues its cut.
    pub async fn start_clip(
        &self,
        creator_id: Uuid,
        source: NewSource,
        title: &str,
        start_seconds: f64,
        duration_seconds: f64,
    ) -> AppResult<ClipRow> {
        let core = &self.core;
        let title = validate_title(title)?;
        validate_range(start_seconds, duration_seconds)?;
        let out_of_range = || AppError::invalid("CLIP_RANGE_INVALID", "the clip lies outside what can be cut");
        // A clip is a copy that outlives the moment: only what anyone with
        // the link may watch can be clipped. Friends-only and private
        // streams and videos stay with their audience.
        let source_visibility = match &source {
            NewSource::Stream(stream) => stream.visibility,
            NewSource::Video { video, .. } => video.visibility,
        };
        ensure!(
            access::open_to_everyone(source_visibility),
            precondition,
            "CLIP_SOURCE_NOT_PUBLIC",
            "friends-only and private streams and videos cannot be clipped"
        );
        let (channel_id, owner_id, stream_id, video_id) = match &source {
            NewSource::Stream(stream) => {
                ensure!(
                    stream.is_live(),
                    precondition,
                    "CLIP_SOURCE_NOT_READY",
                    "this stream cannot be clipped right now"
                );
                ensure!(
                    stream.dvr_enabled,
                    precondition,
                    "DVR_DISABLED",
                    "the streamer turned rewinding off, so the stream cannot be clipped"
                );
                let position = lifecycle::offset_ms_now(&core.db, stream.id).await? as f64 / 1000.0;
                // Half a second of slack for a player slightly ahead of us.
                let end = start_seconds + duration_seconds;
                if end > position + 0.5 || start_seconds < position - self.cfg.dvr_window_seconds as f64 {
                    return Err(out_of_range());
                }
                (stream.channel_id, stream.owner_id, Some(stream.id), None)
            }
            NewSource::Video { video, stream_id } => {
                ensure!(video.is_ready(), precondition, "CLIP_SOURCE_NOT_READY", "this video is still being processed");
                if start_seconds + duration_seconds > video.duration_seconds + 0.05 {
                    return Err(out_of_range());
                }
                (video.channel_id, video.owner_id, *stream_id, Some(video.id))
            }
        };
        core.cache.rate_limit(&format!("live-clip:{creator_id}"), 20, Duration::from_secs(600)).await?;

        let clip_id = new_id();
        sqlx::query(
            "INSERT INTO live_clips (id, short_id, creator_id, channel_id, owner_id, stream_id, video_id, title, start_seconds, duration_seconds)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(clip_id)
        .bind(tokuly_core::ids::new_short_id())
        .bind(creator_id)
        .bind(channel_id)
        .bind(owner_id)
        .bind(stream_id)
        .bind(video_id)
        .bind(&title)
        .bind(start_seconds)
        .bind(duration_seconds)
        .execute(&core.db)
        .await?;
        core.jobs.enqueue_unique(JOB_CLIP, &clip_id.to_string(), &ClipJob { clip_id }).await?;
        find_by_id(core, clip_id).await?.ok_or_else(not_found)
    }

    /// Handles one `live.clip` job: cuts the clip and stores it as an
    /// asset. A clip that cannot be cut becomes FAILED (the job is not
    /// retried: the creator sees the failure and can cut again).
    pub async fn cut_clip(&self, job: ClipJob) -> AppResult<()> {
        let core = &self.core;
        let Some(clip) = find_by_id(core, job.clip_id).await? else { return Ok(()) };
        if clip.status != PROCESSING || clip.asset_id.is_some() {
            return Ok(());
        }
        match self.produce_clip(&clip).await {
            Ok(asset_id) => self.sync_clip_asset(asset_id).await,
            Err(error) => {
                tracing::warn!(clip_id = %clip.id, %error, "cutting a clip failed");
                self.fail_clip(&clip, &error.to_string()).await
            }
        }
    }

    async fn fail_clip(&self, clip: &ClipRow, error: &str) -> AppResult<()> {
        let message: String = error.chars().take(300).collect();
        let failed: Option<ClipRow> = sqlx::query_as(concat!(
            "UPDATE live_clips c SET status = 3, error = $2 WHERE c.id = $1 AND c.status = 1 RETURNING ",
            clip_columns!()
        ))
        .bind(clip.id)
        .bind(message)
        .fetch_optional(&self.core.db)
        .await?;
        if let Some(failed) = failed {
            self.push_clip(&failed).await;
        }
        Ok(())
    }

    /// Marks clips that have been PROCESSING for too long as FAILED.
    pub(crate) async fn fail_stuck_clips(&self, minutes: i32) -> AppResult<u64> {
        let stuck: Vec<ClipRow> = sqlx::query_as(concat!(
            "SELECT ",
            clip_columns!(),
            " FROM live_clips c WHERE c.status = 1 AND c.created_at < now() - make_interval(mins => $1)"
        ))
        .bind(minutes)
        .fetch_all(&self.core.db)
        .await?;
        for clip in &stuck {
            self.fail_clip(clip, "the clip could not be produced in time").await?;
        }
        Ok(stuck.len() as u64)
    }

    async fn push_clip(&self, clip: &ClipRow) {
        let viewer = Viewer { user_id: Some(clip.creator_id), ..Default::default() };
        match to_proto(&self.core, &viewer, clip).await {
            Ok(proto) => self.push_to_user(clip.creator_id, Event::ClipUpdated(Box::new(proto))).await,
            Err(error) => tracing::warn!(clip_id = %clip.id, %error, "could not push a clip update"),
        }
    }

    /// Where the clip's material comes from.
    async fn cut_input(&self, clip: &ClipRow, work: &std::path::Path) -> AppResult<CutInput> {
        let core = &self.core;
        let gone = || AppError::precondition("CLIP_SOURCE_NOT_READY", "the clip's source is no longer available");
        if let Some(video_id) = clip.video_id {
            let video = videos::find_by_id(core, video_id).await?.ok_or_else(gone)?;
            let asset = assets::find(core, video.asset_id).await?.ok_or_else(gone)?;
            let info = asset.video().ok_or_else(gone)?;
            let url = if info.mp4_url.is_empty() { info.hls_url } else { info.mp4_url };
            ensure!(!url.is_empty(), precondition, "CLIP_SOURCE_NOT_READY", "the video has no playable rendition");
            let url = internal_media_url(core, &url);
            return Ok(CutInput {
                args: vec!["-i".to_owned(), url.clone()],
                position: clip.start_seconds,
                probe: Some(url),
            });
        }
        let stream = match clip.stream_id {
            Some(stream_id) => streams::find_by_id(core, stream_id).await?.ok_or_else(gone)?,
            None => return Err(gone()),
        };
        let from = lifecycle::time_at_offset(core, &stream, clip.start_seconds).await?.ok_or_else(gone)?;
        // The end of the cut must have reached the disk.
        let settle = (from + chrono::Duration::milliseconds((clip.duration_seconds * 1000.0) as i64 + 2500) - now())
            .num_milliseconds();
        if settle > 0 {
            tokio::time::sleep(Duration::from_millis(settle.min(10_000) as u64)).await;
        }
        let segments = recordings::segments(&self.cfg, &stream.short_id).await;
        let index = segments.iter().rposition(|segment| segment.start <= from).ok_or_else(gone)?;
        let position = (from - segments[index].start).num_milliseconds() as f64 / 1000.0;
        match segments.get(index + 1) {
            // The cut may run into the next segment: read both as one.
            Some(next) => {
                let list = work.join("segments.txt");
                let lines = [
                    "ffconcat version 1.0".to_owned(),
                    ffmpeg::concat_entry(&segments[index].path),
                    ffmpeg::concat_entry(&next.path),
                ];
                tokio::fs::write(&list, lines.join("\n") + "\n")
                    .await
                    .map_err(|e| AppError::internal(anyhow::anyhow!("cannot write the segment list: {e}")))?;
                let args =
                    ["-f", "concat", "-safe", "0", "-i", &ffmpeg::arg(&list)].iter().map(|s| (*s).to_owned()).collect();
                Ok(CutInput { args, position, probe: None })
            }
            None => {
                let file = ffmpeg::arg(&segments[index].path);
                Ok(CutInput { args: vec!["-i".to_owned(), file.clone()], position, probe: Some(file) })
            }
        }
    }

    /// Cuts the clip and stores it as an asset of its creator. Returns the
    /// asset's ID.
    async fn produce_clip(&self, clip: &ClipRow) -> AppResult<Uuid> {
        let core = &self.core;
        let cfg = &self.cfg;
        let work = tempfile::tempdir()
            .map_err(|e| AppError::internal(anyhow::anyhow!("cannot create a work directory: {e}")))?;
        let input = self.cut_input(clip, work.path()).await?;
        let out = work.path().join("clip.mp4");
        let keyframe = match &input.probe {
            Some(probe) => {
                ffmpeg::keyframe_at(&cfg.ffprobe_path, probe, input.position, KEYFRAME_TOLERANCE).await.unwrap_or(None)
            }
            None => None,
        };
        let duration = format!("{:.3}", clip.duration_seconds);
        let mut args: Vec<String> =
            ["-y", "-hide_banner", "-loglevel", "error", "-ss"].iter().map(|s| (*s).to_owned()).collect();
        args.push(format!("{:.3}", keyframe.unwrap_or(input.position)));
        args.extend(input.args.iter().cloned());
        args.extend(["-t".to_owned(), duration]);
        let codec: &[&str] = match keyframe {
            // The cut starts on a keyframe: copy the stream as it is.
            Some(_) => &["-c", "copy", "-avoid_negative_ts", "make_zero"],
            None => &[
                "-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a",
                "160k",
            ],
        };
        args.extend(codec.iter().map(|s| (*s).to_owned()));
        args.extend(["-movflags".to_owned(), "+faststart".to_owned(), ffmpeg::arg(&out)]);
        ffmpeg::run(&cfg.ffmpeg_path, &args, CUT_TIMEOUT).await?;

        let asset = tokuly_media::create_asset_from_file(
            core,
            // The channel's owner, not the cutter: see the module documentation.
            clip.owner_id,
            AssetPurpose::ASSET_PURPOSE_SHORT,
            &out,
            &format!("clip-{}.mp4", clip.short_id),
        )
        .await?;
        let mut tx = core.db.begin().await?;
        let stored =
            sqlx::query("UPDATE live_clips SET asset_id = $2 WHERE id = $1 AND status = 1 AND asset_id IS NULL")
                .bind(clip.id)
                .bind(asset.id)
                .execute(&mut *tx)
                .await?
                .rows_affected()
                == 1;
        if stored {
            assets::attach(&mut *tx, asset.id, &holder(clip.id)).await?;
            tx.commit().await?;
        } else {
            // The clip was deleted (or failed) while it was being cut.
            drop(tx);
            assets::delete(core, asset.id, None).await?;
        }
        Ok(asset.id)
    }

    /// Brings a clip in line with its upload: READY once the upload is
    /// processed, FAILED when it could not be. Called for `media.asset.*`
    /// events and after a cut.
    pub async fn sync_clip_asset(&self, asset_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        let Some(clip) = find_by_asset(core, asset_id).await? else { return Ok(()) };
        if clip.status != PROCESSING {
            return Ok(());
        }
        let Some(asset) = assets::find(core, asset_id).await? else {
            return self.fail_clip(&clip, "the clip's upload was deleted").await;
        };
        if asset.has_status(tokuly_proto::media::AssetStatus::ASSET_STATUS_FAILED) {
            return self.fail_clip(&clip, "the clip could not be processed").await;
        }
        if !asset.is_ready() {
            return Ok(());
        }
        // Read before the transaction, which then needs no second connection.
        let public = source_of(core, &Viewer::anonymous(), &clip).await?.is_some_and(|source| source.public);
        let mut tx = core.db.begin().await?;
        // Only one caller makes a clip READY (and announces it).
        let ready: Option<ClipRow> = sqlx::query_as(concat!(
            "UPDATE live_clips c SET status = 2, error = '', playback = $2, thumbnail = $3 WHERE c.id = $1 AND c.status = 1 RETURNING ",
            clip_columns!()
        ))
        .bind(clip.id)
        .bind(&asset.video)
        .bind(&asset.image)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(ready) = ready else { return Ok(()) };
        core_events::emit(&mut *tx, CLIP_READY, Some(ready.creator_id), &ready.event()).await?;
        search::sync_clip(&mut tx, &ready, public).await?;
        tx.commit().await?;
        self.push_clip(&ready).await;
        Ok(())
    }

    /// Rewrites the search documents of the clips of a video after its
    /// visibility changed.
    pub(crate) async fn reindex_clips_of_video(&self, video_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        let clips: Vec<ClipRow> = sqlx::query_as(concat!(
            "SELECT ",
            clip_columns!(),
            " FROM live_clips c WHERE c.video_id = $1 AND c.status = 2"
        ))
        .bind(video_id)
        .fetch_all(&core.db)
        .await?;
        // The sources are read first; the transaction only writes.
        let mut sources = Sources::default();
        let mut public = Vec::with_capacity(clips.len());
        for clip in &clips {
            public.push(sources.of(core, &Viewer::anonymous(), clip).await?.is_some_and(|source| source.public));
        }
        let mut tx = core.db.begin().await?;
        for (clip, public) in clips.iter().zip(public) {
            search::sync_clip(&mut tx, clip, public).await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_titles_and_ranges_are_checked() {
        assert_eq!(validate_title("  神回   まとめ ").unwrap(), "神回 まとめ");
        assert_eq!(validate_title("").unwrap_err().reason, "CLIP_TITLE_INVALID");
        assert_eq!(validate_title(&"あ".repeat(65)).unwrap_err().reason, "CLIP_TITLE_INVALID");
        assert!(validate_title(&"あ".repeat(64)).is_ok());
        assert!(validate_range(0.0, 0.1).is_ok() && validate_range(12.5, 60.0).is_ok());
        for (start, duration) in [(0.0, 0.05), (0.0, 60.1), (-1.0, 10.0), (f64::NAN, 10.0), (0.0, f64::INFINITY)] {
            assert_eq!(validate_range(start, duration).unwrap_err().reason, "CLIP_RANGE_INVALID", "{start} {duration}");
        }
    }
}
