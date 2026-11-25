//! Uploads, assets, image processing and the video transcoding pipeline.
//!
//! How the pieces fit:
//!
//! ```text
//! client ──CreateUpload──▶ upload ──presigned URLs──▶ object store (private bucket)
//! client ──CompleteUpload─▶ upload: size, checksum, real content type
//!                              │
//!                              ▼
//!                      lifecycle (state machine, events, realtime pushes)
//!                              │ job
//!                              ▼
//!                      process::{image, animation, video, audio, file}
//!                              │ renditions
//!                              ▼
//!                      object store (media bucket, public, revalidated)
//! ```
//!
//! What other domains use:
//! - [`assets`] to resolve asset IDs received from clients ([`assets::require_ready`]…);
//! - [`link_preview`] for link metadata in messages and notes (its pictures
//!   are served by [`Media::http_routes`]);
//! - [`create_asset_from_file`] for media the server produces itself;
//! - [`events`] for the payload of `media.asset.*` events.
use std::{ops::Deref, sync::Arc, time::Duration};

use async_trait::async_trait;
use tokuly_core::{
    events::{self as core_events, ConsumeOptions},
    export::DataExporter,
    jobs::JobOptions,
    prelude::*,
};

pub mod assets;
pub mod config;
pub mod events;
mod holders;
mod ingest;
mod janitor;
mod lifecycle;
pub mod link_preview;
mod process;
mod recipe;
mod service;
mod store;
pub mod testing;
mod upload;

pub use config::MediaConfig;
pub use ingest::create_asset_from_file;
pub use janitor::Sweep;
pub use lifecycle::{JOB_PROCESS, JOB_TRANSCODE, ProcessJob};
pub use link_preview::link_preview;

use link_preview::Previews;
use tokuly_core::egress::Policy;
use process::ffmpeg::Ffmpeg;

/// How often the janitor sweeps.
const JANITOR_INTERVAL: Duration = Duration::from_secs(300);

pub struct MediaInner {
    pub core: Core,
    pub cfg: MediaConfig,
    pub(crate) originals: store::OriginalStore,
    pub(crate) ffmpeg: Ffmpeg,
    pub(crate) previews: Previews,
    /// Animations are decoded into memory frame by frame; processing one at
    /// a time keeps a burst of large GIFs from exhausting the worker.
    pub(crate) animation_slot: tokio::sync::Semaphore,
}

/// The media domain. Cheap to clone.
#[derive(Clone)]
pub struct Media(Arc<MediaInner>);

impl Deref for Media {
    type Target = MediaInner;

    fn deref(&self) -> &MediaInner {
        &self.0
    }
}

impl Media {
    pub fn new(core: Core, cfg: MediaConfig) -> anyhow::Result<Self> {
        cfg.validate()?;
        let policy = Policy { allow_loopback: cfg.link_preview_allow_loopback };
        let previews = Previews::new(policy, core.cfg.public_url())?;
        let ffmpeg = Ffmpeg::new(cfg.ffmpeg_path.clone(), cfg.ffprobe_path.clone(), cfg.ffmpeg_threads);
        core.exporters.register(Arc::new(MediaExport { core: core.clone() }));
        let originals = store::OriginalStore::new(&core.cfg);
        Ok(Self(Arc::new(MediaInner { core, cfg, originals, ffmpeg, previews, animation_slot: tokio::sync::Semaphore::new(1) })))
    }

    /// The settings in force now. The upload and transcoding limits are
    /// read through this, so a change in the staff console applies without
    /// a restart (they are the live `[media]` keys of the server's settings).
    /// Without registered settings (tests) it is the configuration given to [`Media::new`].
    pub fn live(&self) -> Arc<MediaConfig> {
        self.core.settings.current::<MediaConfig>("media").unwrap_or_else(|| Arc::new(self.cfg.clone()))
    }

    /// Registers this domain's RPC service on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::media::MediaServiceExt;
        MediaServiceExt::register(Arc::new(self.clone()), router)
    }

    /// The plain HTTP routes of this domain: the pictures of link previews
    /// (`link_preview::image_proxy`). Mount them inside the auth layer, which
    /// tells them the client's address.
    pub fn http_routes(&self) -> axum::Router {
        link_preview::image_proxy::routes(self.clone())
    }

    /// Starts the processing workers, the janitor and the account-deletion
    /// consumer. Call once, in worker processes.
    pub fn spawn_workers(&self) {
        self.spawn_job_worker(JOB_PROCESS, self.cfg.image_concurrency, false);
        self.spawn_job_worker(JOB_TRANSCODE, self.cfg.video_concurrency, true);

        let media = self.clone();
        tokio::spawn(async move {
            let handler = media.clone();
            core_events::consume(
                media.core.bus.clone(),
                "media-user-deleted",
                &["identity.user.deleted"],
                // Removing a heavy user's objects takes a while.
                ConsumeOptions { ack_wait: Duration::from_secs(600), concurrency: 2, ..Default::default() },
                media.core.shutdown.clone(),
                move |event| {
                    let media = handler.clone();
                    async move { media.on_user_deleted(event).await }
                },
            )
            .await;
        });

        let media = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(JANITOR_INTERVAL);
            loop {
                tokio::select! {
                    _ = media.core.shutdown.cancelled() => break,
                    _ = tick.tick() => match media.sweep().await {
                        Ok(sweep) if sweep == Sweep::default() => {}
                        Ok(sweep) => tracing::info!(?sweep, "media janitor"),
                        Err(e) => tracing::error!(error = %e, "media janitor failed"),
                    },
                }
            }
        });
    }

    /// Processes one asset right here instead of waiting for a worker, and
    /// returns once it is READY or FAILED. Meant for tests and tooling; a
    /// queued job for the same asset later finds nothing left to do.
    pub async fn process_now(&self, asset_id: Uuid) -> AppResult<()> {
        self.run_job(ProcessJob { asset_id }).await
    }

    /// Takes one turn on the transcoder right here, the way a
    /// `media.transcode` job does: transcodes the waiting video that is
    /// fairest to go next (round-robin across owners, within
    /// `TOKULY_MEDIA_TRANSCODES_PER_ACCOUNT`) and returns its ID, or `None`
    /// when no video may start. Meant for tests and tooling.
    pub async fn transcode_next(&self) -> AppResult<Option<Uuid>> {
        self.run_transcode_turn(ProcessJob { asset_id: Uuid::nil() }).await
    }

    fn spawn_job_worker(&self, queue: &'static str, concurrency: usize, fair_turns: bool) {
        let media = self.clone();
        tokio::spawn(async move {
            let handler = media.clone();
            let options = JobOptions {
                concurrency: concurrency.max(1),
                lease: Duration::from_secs(lifecycle::JOB_LEASE_SECONDS),
                // Attempts are counted on the asset; leave room for duplicates.
                max_deliver: i64::from(lifecycle::MAX_ATTEMPTS) + 2,
            };
            media
                .core
                .jobs
                .run(queue, options, media.core.shutdown.clone(), move |job: ProcessJob| {
                    let media = handler.clone();
                    async move {
                        if fair_turns { media.run_transcode_turn(job).await.map(|_| ()) } else { media.run_job(job).await }
                    }
                })
                .await;
        });
    }
}

/// The media section of a personal data export: what the user uploaded.
struct MediaExport {
    core: Core,
}

#[async_trait]
impl DataExporter for MediaExport {
    fn name(&self) -> &'static str {
        "media"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        let rows: Vec<(Uuid, i32, i32, i32, String, i64, String, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
            "SELECT id, purpose, kind, status, content_type, size_bytes, file_name, url, created_at
             FROM media_assets WHERE owner_id = $1 AND deleted_at IS NULL ORDER BY id",
        )
        .bind(user_id)
        .fetch_all(&self.core.db)
        .await?;
        let assets: Vec<serde_json::Value> = rows
            .into_iter()
            .map(|(id, purpose, kind, status, content_type, size_bytes, file_name, url, created_at)| {
                serde_json::json!({
                    "id": id,
                    "purpose": purpose,
                    "kind": kind,
                    "status": status,
                    "content_type": content_type,
                    "size_bytes": size_bytes,
                    "file_name": file_name,
                    "url": url,
                    "created_at": created_at.to_rfc3339(),
                })
            })
            .collect();
        Ok(serde_json::json!({ "assets": assets }))
    }
}
