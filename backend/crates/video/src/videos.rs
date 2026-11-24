//! Videos: rows, the publishing state machine, ratings and deletion.
//!
//! A video follows two independent states:
//!
//! ```text
//! processing:  PROCESSING ──media.asset.ready──▶ READY
//!                   └───────media.asset.failed─▶ FAILED
//! status:      DRAFT ──publish──▶ SCHEDULED ──due and READY──▶ PUBLISHED
//!                 ▲                                                │
//!                 └──────────────────unpublish────────────────────┘
//! ```
//!
//! "Publish now" on a READY video goes straight to PUBLISHED; on a video
//! that is still processing it is a schedule for "as soon as it is ready".
//! The first time a video is both PUBLISHED and visible to an audience
//! (PUBLIC or FRIENDS) it is *announced* once: an activity entry for the
//! owner's friends and `video.video.published` with `first_publication`,
//! from which the subscriber notifications fan out (see `workers`).
//!
//! # For other domains
//!
//! - [`find`] resolves a video by UUID or short ID;
//! - [`create_from_asset`] turns a media asset (e.g. a live archive) into a
//!   video on a channel.
use std::collections::HashMap;

use tokuly_core::{events as core_events, ids::Ref, outbound, prelude::*};
use tokuly_media::assets::{self, AssetRow, image_from_json};
use tokuly_proto::{
    common::{Image, SubjectType, Visibility},
    media::{AssetKind, AssetStatus, VideoInfo},
    video::{
        Chapter, PlaybackBlock, ProcessingState, Rating, Video, VideoEvent as VideoPush, VideoOrigin, VideoStatus,
        VideoStudioInfo, VideoSummary, VideoViewerState, video_event,
    },
};
use tokuly_social::graph::ContentAudience;

use crate::{
    access::{self, PUBLIC, PUBLISHED, VideoFacts, Viewer},
    channels::{self, ChannelRow},
    events::{VIDEO_CREATED, VIDEO_DELETED, VIDEO_PUBLISHED, VIDEO_UNPUBLISHED, VIDEO_VISIBILITY_CHANGED, VideoEvent},
    games, locks, party, playlists, search, subtitles, validate,
};

pub const DRAFT: i32 = VideoStatus::VIDEO_STATUS_DRAFT as i32;
pub const SCHEDULED: i32 = VideoStatus::VIDEO_STATUS_SCHEDULED as i32;
pub const PROCESSING: i32 = ProcessingState::PROCESSING_STATE_PROCESSING as i32;
pub const READY: i32 = ProcessingState::PROCESSING_STATE_READY as i32;
pub const FAILED: i32 = ProcessingState::PROCESSING_STATE_FAILED as i32;

pub const LIKE: i32 = Rating::RATING_LIKE as i32;
pub const DISLIKE: i32 = Rating::RATING_DISLIKE as i32;

/// How far ahead a video may be scheduled.
pub const MAX_SCHEDULE_DAYS: i64 = 366;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct VideoRow {
    pub id: Uuid,
    pub short_id: String,
    pub channel_id: Uuid,
    /// The channel's owner.
    pub owner_id: Uuid,
    /// The media asset being played.
    pub asset_id: Uuid,
    /// `tokuly.video.v1.VideoOrigin`, and the live stream an archive came from.
    pub origin: i32,
    pub origin_id: Option<Uuid>,
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    pub category: i32,
    pub game: String,
    /// `[tokuly.video.v1.Chapter]` as proto JSON.
    pub chapters: serde_json::Value,
    /// `tokuly.common.v1.Image` as proto JSON: custom thumbnail or poster frame.
    pub thumbnail: Option<serde_json::Value>,
    pub thumbnail_asset_id: Option<Uuid>,
    pub visibility: i32,
    /// VISIBILITY_CIRCLE only: the owner's circle.
    pub circle_id: Option<Uuid>,
    pub allow_link: bool,
    pub age_restricted: bool,
    pub comments_enabled: bool,
    pub notify_subscribers: bool,
    pub status: i32,
    pub processing: i32,
    pub processing_error: String,
    pub scheduled_at: Option<Time>,
    pub published_at: Option<Time>,
    pub announced_at: Option<Time>,
    /// `tokuly.media.v1.VideoInfo` as proto JSON, once processed.
    pub playback: Option<serde_json::Value>,
    pub duration_seconds: f64,
    pub view_count: i64,
    pub like_count: i64,
    pub dislike_count: i64,
    pub comment_count: i64,
    pub trending_score: f64,
    pub suspended: bool,
    /// The owner's account is locked, as this domain's copy knows it.
    pub owner_locked: bool,
    pub created_at: Time,
    pub updated_at: Time,
}

macro_rules! video_columns {
    () => {
        "v.id, v.short_id, v.channel_id, v.owner_id, v.asset_id, v.origin, v.origin_id, v.title, v.description, v.tags, \
         v.category, v.game, v.chapters, v.thumbnail, v.thumbnail_asset_id, v.visibility, v.circle_id, v.allow_link, v.age_restricted, \
         v.comments_enabled, v.notify_subscribers, v.status, v.processing, v.processing_error, v.scheduled_at, \
         v.published_at, v.announced_at, v.playback, v.duration_seconds, v.view_count, v.like_count, v.dislike_count, \
         v.comment_count, v.trending_score, v.suspended, v.owner_locked, v.created_at, v.updated_at"
    };
}
pub(crate) use video_columns;

impl VideoRow {
    pub fn facts(&self) -> VideoFacts {
        VideoFacts {
            owner_id: self.owner_id,
            status: self.status,
            visibility: self.visibility,
            circle_id: self.circle_id,
            allow_link: self.allow_link,
            suspended: self.suspended,
            owner_locked: self.owner_locked,
        }
    }

    /// Whether anyone with the link may watch it, were the owner not locked.
    pub fn is_open(&self) -> bool {
        access::is_open(self.visibility) || (self.visibility == access::FRIENDS && self.allow_link)
    }

    /// The owner, whether the video is open to everyone, and the copy of the
    /// owner's lock: what [`Viewer::drop_newly_locked`] needs.
    pub fn lock_facts(&self) -> (Uuid, bool, bool) {
        (self.owner_id, self.is_open(), self.owner_locked)
    }

    /// The audience the video is shared with, as the social graph names it:
    /// open videos of a locked owner reach followers (and friends) only.
    pub fn effective_audience(&self) -> ContentAudience {
        if self.is_open() {
            if self.owner_locked { ContentAudience::Followers } else { ContentAudience::Public }
        } else {
            access::audience_of(self.visibility, self.circle_id)
        }
    }

    /// The visibility in effect, as `video.video.visibility_changed` names it:
    /// a PUBLIC video (or a FRIENDS one with a link) of a locked owner reads
    /// as FOLLOWERS. NO_FEED and UNLISTED keep their number: they were never
    /// announced to friends, and a lock must not widen what the feed shows.
    pub fn effective_visibility(&self) -> i32 {
        let widest = self.visibility == PUBLIC || (self.visibility == access::FRIENDS && self.allow_link);
        if widest && self.owner_locked { access::FOLLOWERS } else { self.visibility }
    }

    pub fn is_published(&self) -> bool {
        self.status == PUBLISHED
    }

    pub fn is_ready(&self) -> bool {
        self.processing == READY
    }

    /// Published for everyone and listed everywhere (search, feeds): public,
    /// and its owner neither suspended nor locked.
    pub fn is_public(&self) -> bool {
        self.status == PUBLISHED && self.visibility == PUBLIC && !self.suspended && !self.owner_locked
    }

    /// Published to an audience that subscribers and friends are told about
    /// (once, [`after_visible_change_in`]): PUBLIC or one of the social audiences.
    pub fn is_announceable(&self) -> bool {
        self.status == PUBLISHED
            && !self.suspended
            && matches!(
                self.visibility,
                PUBLIC | access::FRIENDS | access::FOLLOWERS | access::CLOSE_FRIENDS | access::CIRCLE
            )
    }

    /// Path of the watch page in the web app.
    pub fn path(&self) -> String {
        format!("/watch/{}", self.short_id)
    }

    pub fn thumbnail(&self) -> Option<Image> {
        image_from_json(self.thumbnail.as_ref())
    }

    pub fn thumbnail_url(&self) -> Option<String> {
        self.thumbnail().map(|image| image.url).filter(|url| !url.is_empty())
    }

    pub fn playback(&self) -> Option<VideoInfo> {
        self.playback.clone().and_then(|value| serde_json::from_value(value).ok())
    }

    pub fn chapters(&self) -> Vec<Chapter> {
        serde_json::from_value(self.chapters.clone()).unwrap_or_default()
    }

    pub(crate) fn event(&self, first_publication: bool) -> VideoEvent {
        VideoEvent {
            video_id: self.id,
            short_id: self.short_id.clone(),
            channel_id: self.channel_id,
            owner_id: self.owner_id,
            visibility: self.visibility,
            origin: self.origin,
            origin_id: self.origin_id,
            first_publication,
        }
    }
}

fn not_found() -> AppError {
    AppError::not_found("VIDEO_NOT_FOUND", "video not found")
}

/// Holder under which a video keeps its upload and custom thumbnail alive.
pub fn holder(video_id: Uuid) -> String {
    format!("video:video:{video_id}")
}

// ───────────────────────────── lookups ─────────────────────────────

/// A video by UUID or short ID, whatever its state. Callers decide who may
/// see it ([`Viewer::can_watch`]).
pub async fn find(core: &Core, reference: &Ref) -> AppResult<Option<VideoRow>> {
    Ok(match reference {
        Ref::Id(id) => {
            sqlx::query_as(concat!("SELECT ", video_columns!(), " FROM videos v WHERE v.id = $1"))
                .bind(id)
                .fetch_optional(&core.db)
                .await?
        }
        Ref::Short(short_id) => {
            sqlx::query_as(concat!("SELECT ", video_columns!(), " FROM videos v WHERE v.short_id = $1"))
                .bind(short_id)
                .fetch_optional(&core.db)
                .await?
        }
    })
}

pub async fn find_by_id(core: &Core, video_id: Uuid) -> AppResult<Option<VideoRow>> {
    find(core, &Ref::Id(video_id)).await
}

/// A video by what clients send as `video_id`. A malformed reference names
/// no video.
pub async fn find_ref(core: &Core, reference: &str) -> AppResult<Option<VideoRow>> {
    match Ref::parse(reference.trim()) {
        Ok(reference) => find(core, &reference).await,
        Err(_) => Ok(None),
    }
}

/// The video a client named, if `viewer` may watch it (`VIDEO_NOT_FOUND`
/// otherwise, whatever the reason).
pub async fn find_watchable(core: &Core, viewer: &Viewer, reference: &str) -> AppResult<VideoRow> {
    let video = find_ref(core, reference).await?.ok_or_else(not_found)?;
    ensure!(viewer.can_watch_now(core, &video.facts()).await?, not_found, "VIDEO_NOT_FOUND", "video not found");
    Ok(video)
}

/// The video a client named, which must belong to `owner_id`.
pub async fn find_owned(core: &Core, owner_id: Uuid, reference: &str) -> AppResult<VideoRow> {
    find_ref(core, reference).await?.filter(|video| video.owner_id == owner_id).ok_or_else(not_found)
}

pub(crate) async fn find_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    video_id: Uuid,
) -> AppResult<Option<VideoRow>> {
    Ok(sqlx::query_as(concat!("SELECT ", video_columns!(), " FROM videos v WHERE v.id = $1 FOR UPDATE"))
        .bind(video_id)
        .fetch_optional(&mut **tx)
        .await?)
}

pub(crate) async fn of_channel_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    channel_id: Uuid,
) -> AppResult<Vec<VideoRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        video_columns!(),
        " FROM videos v WHERE v.channel_id = $1 ORDER BY v.id FOR UPDATE"
    ))
    .bind(channel_id)
    .fetch_all(&mut **tx)
    .await?)
}

// ───────────────────────────── API representations ─────────────────────────────

/// Cards for `rows`, in order, as `viewer` sees them: with the channel and,
/// for signed-in viewers, where they left off.
pub async fn summaries(core: &Core, viewer: &Viewer, rows: &[VideoRow]) -> AppResult<Vec<VideoSummary>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut channel_ids: Vec<Uuid> = rows.iter().map(|row| row.channel_id).collect();
    channel_ids.sort_unstable();
    channel_ids.dedup();
    let channels = channels::summaries(core, &channel_ids).await?;
    let progress: HashMap<Uuid, (f64, bool)> = match viewer.user_id {
        None => HashMap::new(),
        Some(user_id) => {
            let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
            let seen: Vec<(Uuid, f64, bool)> =
                sqlx::query_as("SELECT video_id, position_seconds, completed FROM watch_history WHERE user_id = $1 AND video_id = ANY($2)")
                    .bind(user_id)
                    .bind(&ids)
                    .fetch_all(&core.db)
                    .await?;
            seen.into_iter().map(|(id, position, completed)| (id, (position, completed))).collect()
        }
    };
    Ok(rows
        .iter()
        .map(|row| {
            let (position, completed) = progress.get(&row.id).copied().unwrap_or((0.0, false));
            VideoSummary {
                id: row.id.to_string(),
                short_id: row.short_id.clone(),
                title: row.title.clone(),
                thumbnail: row.thumbnail().into(),
                duration_seconds: row.duration_seconds,
                channel: channels.get(&row.channel_id).cloned().into(),
                view_count: row.view_count,
                like_count: row.like_count,
                comment_count: row.comment_count,
                published_at: opt_ts(row.published_at),
                visibility: access::shown_visibility(row.visibility, viewer.is(row.owner_id)).into(),
                status: row.status.into(),
                processing: row.processing.into(),
                preview_url: row.playback().map(|info| info.preview_url).unwrap_or_default(),
                age_restricted: row.age_restricted,
                origin: row.origin.into(),
                category: row.category.into(),
                resume_position_seconds: if completed { 0.0 } else { position },
                watched: completed,
                created_at: ts(row.created_at),
                scheduled_at: if row.status == SCHEDULED { opt_ts(row.scheduled_at) } else { opt_ts(None) },
                ..Default::default()
            }
        })
        .collect())
}

pub async fn summary(core: &Core, viewer: &Viewer, row: &VideoRow) -> AppResult<VideoSummary> {
    Ok(summaries(core, viewer, std::slice::from_ref(row)).await?.pop().unwrap_or_default())
}

/// The watch page (or studio editor) view of a video for `viewer`, who must
/// already have been allowed to see it.
pub async fn to_proto(core: &Core, viewer: &Viewer, row: &VideoRow) -> AppResult<Video> {
    let is_owner = viewer.is(row.owner_id);
    let block = if !row.is_ready() {
        Some(PlaybackBlock::PLAYBACK_BLOCK_NOT_READY)
    } else {
        access::age_block(core, viewer, row.owner_id, row.age_restricted).await?
    };
    let viewer_state = match viewer.user_id {
        None => None,
        Some(user_id) => {
            let (rating, in_watch_later, level): (Option<i32>, bool, Option<i32>) = sqlx::query_as(
                "SELECT (SELECT rating FROM video_reactions WHERE video_id = $1 AND user_id = $2),
                        EXISTS (SELECT 1 FROM playlist_items i JOIN playlists p ON p.id = i.playlist_id
                                WHERE i.video_id = $1 AND p.owner_id = $2 AND p.kind = 2),
                        (SELECT notify_level FROM channel_subscriptions WHERE channel_id = $3 AND user_id = $2)",
            )
            .bind(row.id)
            .bind(user_id)
            .bind(row.channel_id)
            .fetch_one(&core.db)
            .await?;
            Some(VideoViewerState {
                is_owner,
                rating: rating.unwrap_or(Rating::RATING_NONE as i32).into(),
                in_watch_later,
                subscribed: level.is_some(),
                notification_level: level.unwrap_or(0).into(),
                ..Default::default()
            })
        }
    };
    let studio = is_owner.then(|| VideoStudioInfo {
        asset_id: row.asset_id.to_string(),
        processing_error: row.processing_error.clone(),
        has_custom_thumbnail: row.thumbnail_asset_id.is_some(),
        notify_subscribers: row.notify_subscribers,
        dislike_count: row.dislike_count,
        origin_id: row.origin_id.map(|id| id.to_string()).unwrap_or_default(),
        circle_id: row.circle_id.map(|id| id.to_string()).unwrap_or_default(),
        ..Default::default()
    });
    Ok(Video {
        summary: summary(core, viewer, row).await?.into(),
        description: row.description.clone(),
        tags: row.tags.clone(),
        game: row.game.clone(),
        chapters: row.chapters(),
        comments_enabled: row.comments_enabled,
        playback: if block.is_none() { row.playback().into() } else { None.into() },
        playback_block: block.unwrap_or(PlaybackBlock::PLAYBACK_BLOCK_UNSPECIFIED).into(),
        subtitles: subtitles::list(core, row.id).await?,
        viewer: viewer_state.into(),
        studio: studio.into(),
        allow_link: row.allow_link,
        ..Default::default()
    })
}

/// Tells the owner's devices that one of their videos changed state.
pub async fn push_updated(core: &Core, row: &VideoRow) {
    let viewer = Viewer { user_id: Some(row.owner_id), ..Default::default() };
    match summary(core, &viewer, row).await {
        Ok(summary) => {
            let event =
                VideoPush { event: Some(video_event::Event::VideoUpdated(Box::new(summary))), ..Default::default() };
            core.rt.notify_user(row.owner_id, &event).await;
        }
        Err(error) => tracing::warn!(video_id = %row.id, %error, "could not push a video update"),
    }
}

// ───────────────────────────── creation ─────────────────────────────

/// Everything needed to create a video. Text fields are validated by
/// [`create_from_asset`]; construct with [`NewVideo::new`] and set what differs.
#[derive(Debug, Clone)]
pub struct NewVideo {
    pub channel_id: Uuid,
    /// A video asset owned by the channel's owner, READY or still processing.
    pub asset_id: Uuid,
    /// Empty: the asset's file name.
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    /// `tokuly.video.v1.VideoCategory` as its number (0 = none).
    pub category: i32,
    pub game: String,
    pub visibility: Visibility,
    /// For VISIBILITY_CIRCLE: one of the owner's circles.
    pub circle_id: Option<Uuid>,
    pub allow_link: bool,
    pub age_restricted: bool,
    pub comments_enabled: bool,
    pub notify_subscribers: bool,
    /// A READY image asset of the owner to use instead of the poster frame.
    pub thumbnail_asset_id: Option<Uuid>,
    pub chapters: Vec<Chapter>,
    pub origin: VideoOrigin,
    /// For archives: the live stream this video came from.
    pub origin_id: Option<Uuid>,
    /// Publish as soon as the asset is ready instead of leaving a draft.
    pub publish: bool,
}

impl NewVideo {
    /// A private upload-style draft with comments and notifications on.
    pub fn new(channel_id: Uuid, asset_id: Uuid, title: impl Into<String>) -> Self {
        Self {
            channel_id,
            asset_id,
            title: title.into(),
            description: String::new(),
            tags: Vec::new(),
            category: 0,
            game: String::new(),
            visibility: Visibility::VISIBILITY_PRIVATE,
            circle_id: None,
            allow_link: false,
            age_restricted: false,
            comments_enabled: true,
            notify_subscribers: true,
            thumbnail_asset_id: None,
            chapters: Vec::new(),
            origin: VideoOrigin::VIDEO_ORIGIN_UPLOAD,
            origin_id: None,
            publish: false,
        }
    }
}

/// Creates a video on a channel from a media asset and returns its ID.
///
/// This is the entry point for other domains (the live domain turns a
/// stream's archive into a video with it); `VideoService.CreateVideo` goes
/// through the same code after checking that the caller owns the channel and
/// the upload. The asset must be a video owned by the channel's owner that
/// is processing or ready, and not used by another video
/// (`ASSET_ALREADY_USED`). With `publish`, the video goes live as soon as
/// the asset is ready; otherwise it is a draft.
pub async fn create_from_asset(core: &Core, new: NewVideo) -> AppResult<Uuid> {
    let channel =
        channels::find_by_id(core, new.channel_id).await?.or_not_found("CHANNEL_NOT_FOUND", "channel not found")?;
    let asset = assets::find(core, new.asset_id).await?.or_not_found("ASSET_NOT_FOUND", "asset not found")?;
    ensure!(asset.owner_id == channel.owner_id, not_found, "ASSET_NOT_FOUND", "asset not found");
    ensure!(asset.has_kind(AssetKind::ASSET_KIND_VIDEO), invalid, "ASSET_WRONG_PURPOSE", "a video upload is required");
    ensure!(
        !asset.has_status(AssetStatus::ASSET_STATUS_FAILED),
        precondition,
        "ASSET_FAILED",
        "the upload could not be processed"
    );
    ensure!(
        !asset.has_status(AssetStatus::ASSET_STATUS_UPLOADING),
        precondition,
        "ASSET_NOT_READY",
        "finish the upload first"
    );

    let title = if new.title.trim().is_empty() {
        validate::title_from_file_name(&asset.file_name)
    } else {
        validate::title(&new.title)?
    };
    let description = validate::description(&new.description)?;
    let tags = validate::tags(&new.tags)?;
    let game = validate::game(&new.game)?;
    let category = validate::category(new.category.into())?;
    let visibility = validate::visibility(new.visibility.into(), Visibility::VISIBILITY_PRIVATE)?;
    let circle_id = access::chosen_circle(core, channel.owner_id, visibility, new.circle_id).await?;
    let owner_locked = access::is_locked_now(core, channel.owner_id).await?;
    let ready = asset.is_ready();
    let duration = asset.video().map(|info| info.duration_seconds).unwrap_or(0.0);
    let chapters = validate::chapters(&new.chapters, ready.then_some(duration))?;
    let thumbnail = match new.thumbnail_asset_id {
        Some(asset_id) => Some(
            assets::require_image_json(
                core,
                asset_id,
                channel.owner_id,
                &[tokuly_proto::media::AssetPurpose::ASSET_PURPOSE_THUMBNAIL],
            )
            .await?,
        ),
        None => None,
    };

    let video_id = new_id();
    let mut tx = core.db.begin().await?;
    sqlx::query(
        "INSERT INTO videos (id, short_id, channel_id, owner_id, asset_id, origin, origin_id, title, description, tags, category,
                             game, chapters, thumbnail, thumbnail_asset_id, visibility, allow_link, age_restricted,
                             comments_enabled, notify_subscribers, processing, playback, duration_seconds, suspended,
                             circle_id, owner_locked)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24,
                 $25, $26)",
    )
    .bind(video_id)
    .bind(tokuly_core::ids::new_short_id())
    .bind(channel.id)
    .bind(channel.owner_id)
    .bind(asset.id)
    .bind(new.origin as i32)
    .bind(new.origin_id)
    .bind(&title)
    .bind(&description)
    .bind(&tags)
    .bind(category)
    .bind(&game)
    .bind(chapters_json(&chapters)?)
    .bind(thumbnail.clone().or_else(|| if ready { asset.image.clone() } else { None }))
    .bind(new.thumbnail_asset_id)
    .bind(visibility)
    .bind(new.allow_link && visibility == access::FRIENDS)
    .bind(new.age_restricted)
    .bind(new.comments_enabled)
    .bind(new.notify_subscribers)
    .bind(if ready { READY } else { PROCESSING })
    .bind(if ready { asset.video.clone() } else { None })
    .bind(if ready { duration } else { 0.0 })
    .bind(channel.suspended)
    .bind(circle_id)
    .bind(owner_locked)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        let error = AppError::from(e);
        if error.is_already_exists() { AppError::already_exists("ASSET_ALREADY_USED", "this upload already belongs to a video") } else { error }
    })?;
    assets::attach(&mut *tx, asset.id, &holder(video_id)).await?;
    if let Some(thumbnail_asset_id) = new.thumbnail_asset_id {
        assets::attach(&mut *tx, thumbnail_asset_id, &holder(video_id)).await?;
    }
    let row = find_in(&mut tx, video_id).await?.ok_or_else(not_found)?;
    core_events::emit(&mut *tx, VIDEO_CREATED, Some(channel.owner_id), &row.event(false)).await?;
    if new.publish {
        request_publish_in(&mut tx, &row, None).await?;
    }
    tx.commit().await?;
    // The account may have been locked or unlocked since it was read, and
    // the pass that applies that to every video may have run before this
    // one existed: apply it now (see `Video::sync_owner_lock`).
    if access::is_locked_now(core, channel.owner_id).await? != owner_locked {
        locks::sync_owner(core, channel.owner_id).await?;
    }

    // The asset may have finished between our read and the insert, in which
    // case its `ready` event found no video to update: catch up now.
    if !ready && let Some(row) = find_by_id(core, video_id).await? {
        sync_processing(core, &row).await?;
    }
    Ok(video_id)
}

pub(crate) fn chapters_json(chapters: &[Chapter]) -> AppResult<serde_json::Value> {
    serde_json::to_value(chapters).map_err(|e| AppError::internal(anyhow::anyhow!("chapters encode: {e}")))
}

// ───────────────────────────── publishing ─────────────────────────────

/// Publishes `video` now, or at `publish_at`. A video that is still
/// processing is scheduled for the moment it is ready (or `publish_at`,
/// whichever is later). Publishing a video that is already live is a no-op.
pub(crate) async fn request_publish_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    video: &VideoRow,
    publish_at: Option<Time>,
) -> AppResult<()> {
    ensure!(video.processing != FAILED, precondition, "VIDEO_PROCESSING_FAILED", "the upload could not be processed");
    if video.is_published() {
        return Ok(());
    }
    let current = now();
    let at = publish_at.unwrap_or(current);
    ensure!(
        at > current - chrono::Duration::minutes(5) && at < current + chrono::Duration::days(MAX_SCHEDULE_DAYS),
        invalid,
        "SCHEDULE_INVALID",
        "the publish time must be in the future, at most a year ahead"
    );
    if at <= current && video.is_ready() {
        publish_now_in(tx, video.id).await?;
    } else {
        sqlx::query("UPDATE videos SET status = $2, scheduled_at = $3 WHERE id = $1")
            .bind(video.id)
            .bind(SCHEDULED)
            .bind(at.max(current))
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// Makes a READY video PUBLISHED and does everything that follows from it.
/// Returns the published row, or `None` when there was nothing to publish
/// (already live, not ready, gone).
pub(crate) async fn publish_now_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    video_id: Uuid,
) -> AppResult<Option<VideoRow>> {
    let published: Option<Uuid> = sqlx::query_scalar(
        "UPDATE videos SET status = 3, scheduled_at = NULL, published_at = COALESCE(published_at, now())
         WHERE id = $1 AND status <> 3 AND processing = 2
         RETURNING id",
    )
    .bind(video_id)
    .fetch_optional(&mut **tx)
    .await?;
    if published.is_none() {
        return Ok(None);
    }
    let row = find_in(tx, video_id).await?.ok_or_else(not_found)?;
    let row = after_visible_change_in(tx, false, row, true).await?;
    Ok(Some(row))
}

/// Brings everything derived from a video's audience up to date after its
/// status or visibility changed: the search document, the one-time
/// announcement, the game suggestion counter and the `published` event.
/// `was_public` is whether the video was public before the change;
/// `went_live` whether this change is a publication.
pub(crate) async fn after_visible_change_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    was_public: bool,
    mut row: VideoRow,
    went_live: bool,
) -> AppResult<VideoRow> {
    let channel =
        channels::find_in(tx, row.channel_id).await?.or_not_found("CHANNEL_NOT_FOUND", "channel not found")?;
    search::sync_video(tx, was_public, &row, &channel).await?;
    if was_public != row.is_public() {
        playlists::sync_search_of_video_in(tx, row.id).await?;
    }
    let announce = row.is_announceable() && row.announced_at.is_none();
    if announce {
        let at: Time =
            sqlx::query_scalar("UPDATE videos SET announced_at = now() WHERE id = $1 RETURNING announced_at")
                .bind(row.id)
                .fetch_one(&mut **tx)
                .await?;
        row.announced_at = Some(at);
        announce_activity_in(tx, &row, &channel).await?;
        if row.visibility == PUBLIC && !row.game.is_empty() {
            games::record_use_in(tx, &row.game).await?;
        }
    }
    if went_live || announce {
        core_events::emit(&mut **tx, VIDEO_PUBLISHED, Some(row.owner_id), &row.event(announce)).await?;
    }
    Ok(row)
}

/// The activity entry for a newly announced video. It goes to the owner's
/// friends only, also for public videos: a channel does not reveal which
/// account is behind it to strangers. Friends are inside the public,
/// followers and friends audiences; a close-friends or circle video's
/// entry is the owner's own (the feed cannot tell who is in a circle).
async fn announce_activity_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    row: &VideoRow,
    channel: &ChannelRow,
) -> AppResult<()> {
    let mut entry = outbound::ActivityEntry::new(
        row.owner_id,
        "video.published",
        if matches!(row.visibility, access::CLOSE_FRIENDS | access::CIRCLE) {
            outbound::ActivityAudience::Private
        } else {
            outbound::ActivityAudience::Friends
        },
        outbound::Localized::new(
            format!("動画「{}」を公開したよ!", row.title),
            format!("published a video: {}", row.title),
        ),
    )
    .subject(SubjectType::SUBJECT_TYPE_VIDEO, row.id)
    .path(row.path())
    .data(serde_json::json!({
        "video_id": row.id,
        "short_id": row.short_id,
        "channel_id": channel.id,
        "channel_handle": channel.handle,
        "channel_name": channel.name,
        "duration_seconds": row.duration_seconds,
    }));
    if let Some(url) = row.thumbnail_url() {
        entry = entry.image(url);
    }
    outbound::activity(&mut **tx, &entry).await
}

/// Emits `video.video.visibility_changed` when a video that was and stays
/// published got another audience (`before` and `after` the change).
/// `members`: who is in the audience the caller meant to give it, read
/// before the transaction (`None` for an audience open to everyone); when
/// the row ended up with another audience (a concurrent change), nobody is
/// named, which withdraws too much rather than too little.
pub(crate) async fn announce_visibility_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    before: &VideoRow,
    after: &VideoRow,
    planned: &PlannedAudience,
) -> AppResult<()> {
    let changed = before.visibility != after.visibility
        || before.circle_id != after.circle_id
        || before.is_open() != after.is_open();
    if !(before.is_published() && after.is_published() && changed) {
        return Ok(());
    }
    let audience = after.effective_audience();
    let visible_to = match audience {
        ContentAudience::Public => None,
        ContentAudience::Private => Some(Vec::new()),
        _ if planned.audience == audience => Some(planned.members.clone()),
        _ => Some(Vec::new()),
    };
    emit_visibility_changed_in(tx, after, visible_to).await
}

/// Emits `video.video.visibility_changed` for `row` as it is now.
pub(crate) async fn emit_visibility_changed_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    row: &VideoRow,
    visible_to: Option<Vec<Uuid>>,
) -> AppResult<()> {
    let changed_at: Time = sqlx::query_scalar("SELECT clock_timestamp()").fetch_one(&mut **tx).await?;
    let mut video = row.event(false);
    video.visibility = row.effective_visibility();
    let payload = crate::events::VideoVisibilityChanged { video, visible_to, changed_at };
    core_events::emit(&mut **tx, VIDEO_VISIBILITY_CHANGED, Some(row.owner_id), &payload).await
}

/// The audience a creator is about to give a video, and who is in it, read
/// from the social graph before a transaction opens.
#[derive(Debug, Clone)]
pub(crate) struct PlannedAudience {
    pub audience: ContentAudience,
    pub members: Vec<Uuid>,
}

impl PlannedAudience {
    /// For `video` (as read before the change) given the visibility,
    /// circle and link setting it is about to get.
    pub async fn read(
        core: &Core,
        video: &VideoRow,
        visibility: i32,
        circle_id: Option<Uuid>,
        allow_link: bool,
    ) -> AppResult<Self> {
        let planned = VideoRow {
            visibility,
            circle_id,
            allow_link: allow_link && visibility == access::FRIENDS,
            ..video.clone()
        };
        let audience = planned.effective_audience();
        let members = match audience {
            ContentAudience::Public | ContentAudience::Private => Vec::new(),
            audience => {
                let members = tokuly_social::graph::audience_member_ids(core, video.owner_id, audience).await?;
                if members.len() > core_events::MAX_VISIBLE_TO {
                    // Too many to name on the bus: only the ones who may have
                    // something about the video in their inbox.
                    let told: Vec<Uuid> = sqlx::query_scalar(
                        "SELECT user_id FROM channel_subscriptions WHERE channel_id = $1
                         UNION SELECT user_id FROM video_reactions WHERE video_id = $2",
                    )
                    .bind(video.channel_id)
                    .bind(video.id)
                    .fetch_all(&core.db)
                    .await?;
                    core_events::bounded_audience(members, Some(&told))
                } else {
                    core_events::bounded_audience(members, None)
                }
            }
        };
        Ok(Self { audience, members })
    }
}

/// Whether going from `before` to `after` may leave out someone who could
/// watch the video: it was open to everyone and is not any more, or it
/// went to a narrower audience. Watch parties end then (their members are
/// strangers to the owner as often as not).
pub(crate) fn narrowed(before: &VideoRow, after: &VideoRow) -> bool {
    let (was, now) = (before.effective_audience(), after.effective_audience());
    let contains = now == was
        || now == ContentAudience::Public
        || (now == ContentAudience::Followers && was != ContentAudience::Public);
    before.is_published() && (!after.is_published() || !contains)
}

/// Takes a video back to DRAFT (cancelling a schedule).
pub(crate) async fn unpublish_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    video: &VideoRow,
) -> AppResult<VideoRow> {
    sqlx::query("UPDATE videos SET status = $2, scheduled_at = NULL WHERE id = $1")
        .bind(video.id)
        .bind(DRAFT)
        .execute(&mut **tx)
        .await?;
    let row = find_in(tx, video.id).await?.ok_or_else(not_found)?;
    if video.is_published() {
        let channel =
            channels::find_in(tx, row.channel_id).await?.or_not_found("CHANNEL_NOT_FOUND", "channel not found")?;
        search::sync_video(tx, video.is_public(), &row, &channel).await?;
        if video.is_public() {
            playlists::sync_search_of_video_in(tx, row.id).await?;
        }
        core_events::emit(&mut **tx, VIDEO_UNPUBLISHED, Some(row.owner_id), &row.event(false)).await?;
    }
    Ok(row)
}

/// Publishes scheduled videos that are due. Returns how many went live.
pub async fn publish_due(core: &Core) -> AppResult<u64> {
    let due: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM videos WHERE status = 2 AND processing = 2 AND scheduled_at <= now() ORDER BY scheduled_at LIMIT 200",
    )
    .fetch_all(&core.db)
    .await?;
    let mut published = 0;
    for video_id in due {
        let mut tx = core.db.begin().await?;
        let row = publish_now_in(&mut tx, video_id).await?;
        tx.commit().await?;
        if let Some(row) = row {
            published += 1;
            push_updated(core, &row).await;
        }
    }
    Ok(published)
}

// ───────────────────────────── processing results ─────────────────────────────

/// Brings a video in step with the state of its media asset: READY fills in
/// the streams, duration and poster (and publishes a video that was waiting
/// for it); FAILED records why. Idempotent; called for `media.asset.ready` /
/// `failed` and by the worker that catches missed events.
pub async fn sync_processing(core: &Core, video: &VideoRow) -> AppResult<()> {
    if video.processing != PROCESSING {
        return Ok(());
    }
    let Some(asset) = assets::find(core, video.asset_id).await? else { return Ok(()) };
    if asset.is_ready() {
        apply_ready(core, video, &asset).await
    } else if asset.has_status(AssetStatus::ASSET_STATUS_FAILED) {
        apply_failed(core, video, &asset).await
    } else {
        Ok(())
    }
}

async fn apply_ready(core: &Core, video: &VideoRow, asset: &AssetRow) -> AppResult<()> {
    let duration = asset.video().map(|info| info.duration_seconds).unwrap_or(0.0);
    let mut tx = core.db.begin().await?;
    let updated: Option<Uuid> = sqlx::query_scalar(
        "UPDATE videos SET processing = 2, processing_error = '', playback = $2, duration_seconds = $3,
                thumbnail = CASE WHEN thumbnail_asset_id IS NULL THEN $4 ELSE thumbnail END
         WHERE id = $1 AND processing = 1
         RETURNING id",
    )
    .bind(video.id)
    .bind(&asset.video)
    .bind(duration)
    .bind(&asset.image)
    .fetch_optional(&mut *tx)
    .await?;
    if updated.is_none() {
        return Ok(()); // someone else got here first
    }
    // Chapters were accepted before the length was known; drop what does not fit.
    let chapters: Vec<Chapter> =
        video.chapters().into_iter().filter(|chapter| chapter.start_seconds < duration).collect();
    if chapters.len() != video.chapters().len() {
        let kept = if chapters.len() > 1 { chapters } else { Vec::new() };
        sqlx::query("UPDATE videos SET chapters = $2 WHERE id = $1")
            .bind(video.id)
            .bind(chapters_json(&kept)?)
            .execute(&mut *tx)
            .await?;
    }
    let mut row = find_in(&mut tx, video.id).await?.ok_or_else(not_found)?;
    let due = row.status == SCHEDULED && row.scheduled_at.is_none_or(|at| at <= now());
    if due && let Some(published) = publish_now_in(&mut tx, row.id).await? {
        row = published;
    }
    let (ja, en) = if row.is_published() {
        (
            format!("「{}」の処理が終わって公開されたよ!", row.title),
            format!("\"{}\" finished processing and is now live", row.title),
        )
    } else {
        (
            format!("「{}」の処理が終わったよ! 公開できるよ!", row.title),
            format!("\"{}\" finished processing and is ready to publish", row.title),
        )
    };
    let mut request = outbound::NotificationRequest::new(
        row.owner_id,
        "video.processing_done",
        outbound::Localized::new("動画の準備ができたよ!", "Your video is ready"),
        outbound::Localized::new(ja, en),
        format!("/studio/videos/{}", row.short_id),
    )
    .subject(SubjectType::SUBJECT_TYPE_VIDEO, row.id)
    .collapse(format!("video.processing:{}", row.id))
    .data(serde_json::json!({ "video_id": row.id, "short_id": row.short_id, "published": row.is_published() }));
    if let Some(url) = row.thumbnail_url() {
        request = request.image(url);
    }
    outbound::notify(&mut *tx, &request).await?;
    tx.commit().await?;
    push_updated(core, &row).await;
    Ok(())
}

async fn apply_failed(core: &Core, video: &VideoRow, asset: &AssetRow) -> AppResult<()> {
    let mut tx = core.db.begin().await?;
    let updated: Option<Uuid> = sqlx::query_scalar(
        "UPDATE videos SET processing = 3, processing_error = $2, scheduled_at = NULL,
                status = CASE WHEN status = 2 THEN 1 ELSE status END
         WHERE id = $1 AND processing = 1
         RETURNING id",
    )
    .bind(video.id)
    .bind(if asset.error.is_empty() { "PROCESSING_FAILED" } else { asset.error.as_str() })
    .fetch_optional(&mut *tx)
    .await?;
    if updated.is_none() {
        return Ok(());
    }
    let row = find_in(&mut tx, video.id).await?.ok_or_else(not_found)?;
    let request = outbound::NotificationRequest::new(
        row.owner_id,
        "video.processing_failed",
        outbound::Localized::new("動画を処理できなかったよ…", "Your video could not be processed"),
        outbound::Localized::new(
            format!("「{}」を処理できなかったよ。別のファイルで試してみてね!", row.title),
            format!("\"{}\" could not be processed. Try uploading another file.", row.title),
        ),
        format!("/studio/videos/{}", row.short_id),
    )
    .subject(SubjectType::SUBJECT_TYPE_VIDEO, row.id)
    .collapse(format!("video.processing:{}", row.id))
    .data(serde_json::json!({ "video_id": row.id, "short_id": row.short_id, "error": row.processing_error }));
    outbound::notify(&mut *tx, &request).await?;
    tx.commit().await?;
    push_updated(core, &row).await;
    Ok(())
}

/// Catches up videos whose asset finished while nobody was listening (an
/// event consumed before the video row existed, a dropped event).
pub async fn reconcile_processing(core: &Core) -> AppResult<u64> {
    let stuck: Vec<VideoRow> = sqlx::query_as(concat!(
        "SELECT ",
        video_columns!(),
        " FROM videos v WHERE v.processing = 1 AND v.created_at < now() - interval '1 minute' ORDER BY v.id LIMIT 500"
    ))
    .fetch_all(&core.db)
    .await?;
    let mut moved = 0;
    for video in stuck {
        let before = video.processing;
        sync_processing(core, &video).await?;
        if find_by_id(core, video.id).await?.is_some_and(|row| row.processing != before) {
            moved += 1;
        }
    }
    Ok(moved)
}

// ───────────────────────────── deletion ─────────────────────────────

/// What is left to do once a transaction that deleted videos has
/// committed: remove uploads and subtitle files, tell watch parties.
#[derive(Debug, Default)]
pub struct Aftermath {
    /// `(asset, owner, force)`: the uploads of deleted videos. Forced
    /// deletions (takedowns) ignore other holders.
    assets: Vec<(Uuid, Uuid, bool)>,
    /// Object key prefixes in the media bucket to remove.
    prefixes: Vec<String>,
    /// Parties that lost their video.
    parties: Vec<party::Audience>,
}

impl Aftermath {
    pub fn merge(&mut self, other: Aftermath) {
        self.assets.extend(other.assets);
        self.prefixes.extend(other.prefixes);
        self.parties.extend(other.parties);
    }

    /// Best effort: whatever fails here is collected later by the media
    /// janitor (unheld uploads) or is harmless (orphaned subtitle files).
    pub async fn run(self, core: &Core) {
        for audience in &self.parties {
            party::push_ended(core, audience, tokuly_proto::video::PartyEndReason::PARTY_END_REASON_VIDEO_UNAVAILABLE)
                .await;
        }
        for (asset_id, owner_id, force) in self.assets {
            // An upload something else still uses (a clip, a short) stays.
            match assets::delete(core, asset_id, if force { None } else { Some(owner_id) }).await {
                Ok(_) => {}
                Err(error) if error.reason == "ASSET_IN_USE" => {}
                Err(error) => tracing::warn!(%asset_id, %error, "could not delete the upload of a deleted video"),
            }
        }
        for prefix in self.prefixes {
            if let Err(error) = core.storage.delete_prefix(tokuly_core::storage::Bucket::Media, &prefix).await {
                tracing::warn!(prefix, %error, "could not delete subtitle files of a deleted video");
            }
        }
    }
}

/// Deletes a video inside `tx`: the row with everything that hangs off it
/// (subtitles, ratings, history, playlist entries, parties), its search
/// document and its holds on uploads. Run the returned [`Aftermath`] after
/// the commit. `force_assets` deletes the upload even when something else
/// still uses it (takedowns).
pub async fn delete_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    video: &VideoRow,
    actor: Option<Uuid>,
    force_assets: bool,
) -> AppResult<Aftermath> {
    let parties = party::audiences_of_video_in(tx, video.id).await?;
    let lists = playlists::public_lists_of_video_in(tx, video.id).await?;
    assets::detach_holder(&mut **tx, &holder(video.id)).await?;
    sqlx::query("DELETE FROM videos WHERE id = $1").bind(video.id).execute(&mut **tx).await?;
    playlists::sync_search_of_lists_in(tx, &lists).await?;
    // Whatever `video` says: it may have been read before the video went
    // public, and removing a document that does not exist costs nothing.
    outbound::unindex(&mut **tx, "video", video.id).await?;
    core_events::emit(&mut **tx, VIDEO_DELETED, actor, &video.event(false)).await?;
    Ok(Aftermath {
        assets: vec![(video.asset_id, video.owner_id, force_assets)],
        prefixes: vec![subtitles::object_prefix(video.id)],
        parties,
    })
}

/// Deletes a video in a transaction of its own and cleans up after it.
pub async fn delete(core: &Core, video: &VideoRow, actor: Option<Uuid>, force_assets: bool) -> AppResult<()> {
    let mut tx = core.db.begin().await?;
    let aftermath = delete_in(&mut tx, video, actor, force_assets).await?;
    tx.commit().await?;
    aftermath.run(core).await;
    Ok(())
}

// ───────────────────────────── ratings ─────────────────────────────

/// What changing a rating from `previous` to `next` does to the counters:
/// `(likes, dislikes)` deltas.
pub fn rating_deltas(previous: Option<i32>, next: Option<i32>) -> (i64, i64) {
    let count = |rating: Option<i32>, wanted: i32| i64::from(rating == Some(wanted));
    (count(next, LIKE) - count(previous, LIKE), count(next, DISLIKE) - count(previous, DISLIKE))
}

/// The outcome of [`rate`].
#[derive(Debug, Clone, Copy)]
pub struct Rated {
    pub like_count: i64,
    /// The rating before this call.
    pub previous: Option<i32>,
}

/// Sets (`Some(LIKE | DISLIKE)`) or clears (`None`) a user's rating of a
/// video. Liked videos are kept in the user's built-in "liked videos" list.
pub async fn rate(core: &Core, user_id: Uuid, video: &VideoRow, rating: Option<i32>) -> AppResult<Rated> {
    let mut tx = core.db.begin().await?;
    // One rating change per user and video at a time, so the deltas are exact.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("video:rate:{}:{}", video.id.simple(), user_id.simple()))
        .execute(&mut *tx)
        .await?;
    let previous: Option<i32> =
        sqlx::query_scalar("SELECT rating FROM video_reactions WHERE video_id = $1 AND user_id = $2")
            .bind(video.id)
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    let (likes, dislikes) = rating_deltas(previous, rating);
    match rating {
        Some(rating) => {
            sqlx::query(
                "INSERT INTO video_reactions (video_id, user_id, rating) VALUES ($1, $2, $3)
                 ON CONFLICT (video_id, user_id) DO UPDATE SET rating = EXCLUDED.rating, created_at = now()",
            )
            .bind(video.id)
            .bind(user_id)
            .bind(rating)
            .execute(&mut *tx)
            .await?;
        }
        None => {
            sqlx::query("DELETE FROM video_reactions WHERE video_id = $1 AND user_id = $2")
                .bind(video.id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        }
    }
    let like_count: i64 = sqlx::query_scalar(
        "UPDATE videos SET like_count = GREATEST(like_count + $2, 0), dislike_count = GREATEST(dislike_count + $3, 0)
         WHERE id = $1 RETURNING like_count",
    )
    .bind(video.id)
    .bind(likes)
    .bind(dislikes)
    .fetch_one(&mut *tx)
    .await?;
    if likes != 0 {
        let liked_list = playlists::builtin_in(&mut tx, user_id, playlists::KIND_LIKED).await?;
        if likes > 0 {
            playlists::add_item_in(&mut tx, liked_list, video.id).await?;
            // Likes of the owner do not make their own video trend.
            if user_id != video.owner_id {
                sqlx::query(
                    "INSERT INTO video_hourly_stats (video_id, hour, likes) VALUES ($1, date_trunc('hour', now()), 1)
                     ON CONFLICT (video_id, hour) DO UPDATE SET likes = video_hourly_stats.likes + 1",
                )
                .bind(video.id)
                .execute(&mut *tx)
                .await?;
            }
        } else {
            playlists::remove_item_in(&mut tx, liked_list, video.id).await?;
        }
    }
    tx.commit().await?;
    Ok(Rated { like_count, previous })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rating_changes_move_exactly_the_affected_counters() {
        assert_eq!(rating_deltas(None, Some(LIKE)), (1, 0));
        assert_eq!(rating_deltas(Some(LIKE), Some(LIKE)), (0, 0));
        assert_eq!(rating_deltas(Some(LIKE), Some(DISLIKE)), (-1, 1));
        assert_eq!(rating_deltas(Some(DISLIKE), None), (0, -1));
        assert_eq!(rating_deltas(None, None), (0, 0));
    }
}
