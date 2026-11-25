//! Turning a stored upload into something servable.
//!
//! One job per asset ([`lifecycle::ProcessJob`]) runs the recipe of the
//! asset's kind:
//!
//! | kind | module | result |
//! |---|---|---|
//! | image / animation | [`image`], [`animation`] | WebP variants, ThumbHash, dominant colour |
//! | video | [`video`] | HLS ladder, poster, storyboard, preview, MP4 |
//! | audio | [`audio`] | AAC + Opus, waveform |
//! | file | [`file`] | checksum check; dotLottie stamps are published |
//!
//! A recipe fails in one of three ways ([`ProcessError`]), and the
//! difference decides what happens next: a rejected upload becomes FAILED at
//! once, infrastructure trouble is retried, and a cancelled job (the asset
//! was deleted meanwhile) just cleans up after itself.
use std::{
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

use tokuly_core::{prelude::*, storage::Bucket};
use tokuly_proto::media::AssetKind;

use crate::{
    Media,
    assets::AssetRow,
    lifecycle::{self, MAX_ATTEMPTS, ProcessJob, Processed},
    recipe, store,
    upload::checksum_deferred,
};

pub mod animation;
pub mod audio;
pub mod ffmpeg;
pub mod file;
pub mod hls;
pub mod image;
pub mod image_job;
pub mod ladder;
pub mod storyboard;
pub mod video;

/// Why a processing recipe stopped.
#[derive(Debug)]
pub enum ProcessError {
    /// The upload itself is the problem (corrupt, wrong content, too big to
    /// decode). Retrying cannot help; `reason` is shown to the client.
    Rejected { reason: &'static str, detail: String },
    /// Something around us failed (storage, database, a missing binary).
    Transient(AppError),
    /// The asset was deleted, or another worker finished it.
    Cancelled,
}

impl ProcessError {
    pub fn rejected(reason: &'static str, detail: impl Into<String>) -> Self {
        Self::Rejected { reason, detail: detail.into() }
    }

    pub fn transient(error: impl std::fmt::Display) -> Self {
        Self::Transient(AppError::internal(anyhow::anyhow!("{error}")))
    }
}

impl From<AppError> for ProcessError {
    fn from(error: AppError) -> Self {
        Self::Transient(error)
    }
}

impl From<std::io::Error> for ProcessError {
    fn from(error: std::io::Error) -> Self {
        Self::transient(format!("media scratch i/o: {error}"))
    }
}

pub type ProcessResult<T> = Result<T, ProcessError>;

/// Runs CPU-heavy work off the async threads. A panic in the work (a decoder
/// bug on a hostile file) is reported as a rejected upload, not a crash.
pub(crate) async fn blocking<T, F>(work: F) -> ProcessResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> ProcessResult<T> + Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result,
        Err(e) if e.is_panic() => Err(ProcessError::rejected("CORRUPT_MEDIA", "the decoder panicked")),
        Err(e) => Err(ProcessError::transient(format!("blocking task: {e}"))),
    }
}

/// Reports processing progress of one asset, throttled so that a chatty
/// source (ffmpeg emits progress twice a second) does not flood the database
/// and the realtime bus.
pub struct Progress {
    core: Core,
    asset_id: Uuid,
    /// Last reported percent and when it was reported.
    last: Mutex<(i32, Instant)>,
}

const PROGRESS_MIN_INTERVAL: Duration = Duration::from_secs(1);

impl Progress {
    pub fn new(core: &Core, asset_id: Uuid) -> Self {
        Self { core: core.clone(), asset_id, last: Mutex::new((-1, Instant::now())) }
    }

    /// Records `percent`. Fails with [`ProcessError::Cancelled`] when the
    /// asset is no longer being processed, which is how a running recipe
    /// learns that its asset was deleted.
    pub async fn set(&self, percent: i32) -> ProcessResult<()> {
        let percent = percent.clamp(0, 99);
        {
            let mut last = self.last.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let (last_percent, last_at) = *last;
            if percent <= last_percent || (last_percent >= 0 && last_at.elapsed() < PROGRESS_MIN_INTERVAL) {
                return Ok(());
            }
            *last = (percent, Instant::now());
        }
        if lifecycle::report_progress(&self.core, self.asset_id, percent).await? { Ok(()) } else { Err(ProcessError::Cancelled) }
    }

    /// Records a position within a stage that spans `from..to` percent.
    pub async fn within(&self, from: i32, to: i32, fraction: f64) -> ProcessResult<()> {
        let fraction = if fraction.is_finite() { fraction.clamp(0.0, 1.0) } else { 0.0 };
        self.set(from + ((to - from) as f64 * fraction) as i32).await
    }
}

/// Renews the job's claim on its asset while it runs, so duplicates stay
/// away and the janitor can tell a slow job from a dead one. Stops when
/// dropped.
struct Heartbeat(tokio::task::JoinHandle<()>);

impl Heartbeat {
    fn start(core: &Core, asset_id: Uuid) -> Self {
        let core = core.clone();
        Self(tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(lifecycle::HEARTBEAT_SECONDS));
            tick.tick().await; // the first tick fires immediately
            loop {
                tick.tick().await;
                if let Err(e) = lifecycle::heartbeat(&core, asset_id).await {
                    tracing::warn!(%asset_id, error = %e, "asset heartbeat failed");
                }
            }
        }))
    }
}

impl Drop for Heartbeat {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Media {
    /// Handles one processing job. Returns `Err` only when the job should be
    /// retried; every final outcome (READY, FAILED, nothing to do) is `Ok`.
    pub(crate) async fn run_job(&self, job: ProcessJob) -> AppResult<()> {
        let Some(asset) = lifecycle::begin_attempt(&self.core, job.asset_id).await? else {
            return Ok(()); // finished, failed or deleted in the meantime
        };
        self.work_on(asset).await
    }

    /// Handles one `media.transcode` job: a turn on the transcoder, spent on
    /// whichever video is fairest to transcode next
    /// ([`lifecycle::claim_next_transcode`]), not necessarily the one whose
    /// arrival queued the job. Returns the asset it worked on.
    pub(crate) async fn run_transcode_turn(&self, _job: ProcessJob) -> AppResult<Option<Uuid>> {
        let core = &self.core;
        let Some(asset) = lifecycle::claim_next_transcode(core, self.live().transcodes_per_account).await? else {
            // Nothing may start now. A video held back by its owner's limit
            // gets the turn queued when one of their transcodes ends.
            return Ok(None);
        };
        let result = self.work_on(asset.clone()).await;
        // A turn that found every waiting owner at their limit was spent on
        // nothing: give the next one a turn now that a slot is free.
        if lifecycle::transcodes_waiting(core).await? {
            lifecycle::enqueue_transcode_turn(core, &asset).await?;
        }
        result.map(|()| Some(asset.id))
    }

    /// Processes an asset this worker has claimed.
    async fn work_on(&self, asset: AssetRow) -> AppResult<()> {
        let core = &self.core;
        if asset.attempts > MAX_ATTEMPTS {
            tracing::error!(asset_id = %asset.id, attempts = asset.attempts, "asset gave up after repeated processing failures");
            lifecycle::fail(core, asset.id, "PROCESSING_FAILED").await?;
            return Ok(());
        }
        let _heartbeat = Heartbeat::start(core, asset.id);
        let started = Instant::now();
        let progress = Progress::new(core, asset.id);
        match self.process(&asset, &progress).await {
            Ok(processed) => {
                if lifecycle::finish(core, asset.id, processed).await?.is_none() {
                    // Deleted while we were working: take our output with us.
                    self.discard_output(&asset).await;
                } else {
                    tracing::info!(asset_id = %asset.id, kind = asset.kind, elapsed_ms = started.elapsed().as_millis() as u64, "asset ready");
                }
                Ok(())
            }
            Err(ProcessError::Rejected { reason, detail }) => {
                tracing::info!(asset_id = %asset.id, reason, detail, "asset rejected");
                lifecycle::fail(core, asset.id, reason).await?;
                Ok(())
            }
            Err(ProcessError::Cancelled) => {
                self.discard_output(&asset).await;
                Ok(())
            }
            Err(ProcessError::Transient(error)) if asset.attempts >= MAX_ATTEMPTS => {
                tracing::error!(asset_id = %asset.id, error = %error, "asset processing failed permanently");
                lifecycle::fail(core, asset.id, "PROCESSING_FAILED").await?;
                Ok(())
            }
            Err(ProcessError::Transient(error)) => {
                // Free the claim so the redelivered job can take the asset.
                lifecycle::release(core, asset.id).await?;
                Err(error)
            }
        }
    }

    /// Removes whatever a job wrote for an asset that was deleted while it
    /// ran: the delete's own purge may have run before our last write.
    async fn discard_output(&self, asset: &AssetRow) {
        match crate::assets::load(&self.core, asset.id).await {
            Ok(None) => {
                if let Err(e) = lifecycle::remove_objects(&self.core, asset).await {
                    tracing::warn!(asset_id = %asset.id, error = %e, "could not remove the output of a cancelled job");
                }
            }
            // Still there: finished or being retried by another worker, which owns the output.
            Ok(Some(_)) => {}
            Err(e) => tracing::warn!(asset_id = %asset.id, error = %e, "could not check a cancelled job's asset"),
        }
    }

    async fn process(&self, asset: &AssetRow, progress: &Progress) -> ProcessResult<Processed> {
        match asset.asset_kind() {
            AssetKind::ASSET_KIND_IMAGE | AssetKind::ASSET_KIND_ANIMATION => self.process_image(asset, progress).await,
            AssetKind::ASSET_KIND_VIDEO => self.process_video(asset, progress).await,
            AssetKind::ASSET_KIND_AUDIO => self.process_audio(asset, progress).await,
            AssetKind::ASSET_KIND_FILE => self.process_file(asset).await,
            AssetKind::ASSET_KIND_UNSPECIFIED => Err(ProcessError::rejected("UNSUPPORTED_CONTENT", "asset has no kind")),
        }
    }

    /// Refuses a recording longer than its purpose takes
    /// ([`recipe::max_duration_seconds`]).
    pub(crate) fn check_duration(&self, asset: &AssetRow, duration: f64) -> ProcessResult<()> {
        if recipe::too_long(&self.live(), asset.asset_purpose(), duration) {
            return Err(ProcessError::rejected("MEDIA_TOO_LONG", format!("{duration:.0} s")));
        }
        Ok(())
    }

    /// Books a video's minutes on its owner's daily transcoding budget
    /// (`TOKULY_MEDIA_DAILY_TRANSCODE_MINUTES`) before the transcode starts.
    /// Media the server produces itself (live archives and clips) is bounded
    /// by the domain that makes it and is not counted.
    pub(crate) async fn reserve_transcode(&self, asset: &AssetRow, duration: f64) -> ProcessResult<()> {
        let Some(budget) = self.live().daily_transcode_seconds() else { return Ok(()) };
        if asset.is_server_made() {
            return Ok(());
        }
        if !lifecycle::book_transcode(&self.core, asset, duration, budget).await? {
            return Err(ProcessError::rejected("DAILY_TRANSCODE_LIMIT_EXCEEDED", format!("{duration:.0} s over the daily budget")));
        }
        Ok(())
    }

    /// A scratch directory for one job, removed when dropped.
    pub(crate) fn scratch_dir(&self) -> ProcessResult<tempfile::TempDir> {
        let mut builder = tempfile::Builder::new();
        builder.prefix("tokuly-media-");
        let dir = match &self.cfg.work_dir {
            Some(root) => {
                std::fs::create_dir_all(root)?;
                builder.tempdir_in(root)?
            }
            None => builder.tempdir()?,
        };
        Ok(dir)
    }

    /// Downloads the original into `dir` and, when the client supplied a
    /// checksum that was too expensive to verify at upload time, verifies it.
    pub(crate) async fn fetch_original(&self, asset: &AssetRow, dir: &std::path::Path) -> ProcessResult<PathBuf> {
        let path = dir.join("source");
        let downloaded = self.core.storage.download(Bucket::Private, &asset.original_key, &path).await?;
        if downloaded as i64 != asset.size_bytes {
            return Err(ProcessError::rejected("SIZE_MISMATCH", format!("stored {downloaded} bytes, expected {}", asset.size_bytes)));
        }
        if checksum_deferred(asset) {
            let file = path.clone();
            let actual = blocking(move || Ok(store::sha256_file(&file)?)).await?;
            if actual != asset.sha256 {
                return Err(ProcessError::rejected("CHECKSUM_MISMATCH", "sha256 differs from the one announced"));
            }
        }
        Ok(path)
    }
}
