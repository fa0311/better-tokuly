//! Shorts: rows, creation, editing, deletion, the processing state machine
//! and pins.
//!
//! A short follows the upload it plays:
//!
//! ```text
//! PROCESSING ──media.asset.ready, ≤ 180 s, not wider than 16:9──▶ PUBLISHED
//!      └───────media.asset.failed, or too long / too wide───────▶ FAILED
//! ```
//!
//! A short made from an asset that is already READY is published by the
//! call that creates it. Everything that depends on a short being visible
//! (hashtag pages, search, announcements, mention notifications) is brought
//! up to date by [`listing::after_change_in`], which every change ends with.
//!
//! Deleting is final, with one exception: a short that has been reported
//! is *withdrawn* when its owner deletes it. It is gone for everyone, its
//! owner included, but the row and the media stay for a retention period
//! counted from the withdrawal, so that deleting a short does not delete
//! the evidence a moderator needs ([`withdraw`], [`purge_withdrawn`]).
//!
//! # Transactions never wait for the pool
//!
//! A function here that holds a transaction (or the pin lock) runs every
//! query on that transaction. What it needs from elsewhere (the channel,
//! the owner's friends, uploads) is read *before* the transaction begins
//! ([`listing::Context`]). Otherwise a handful of concurrent requests could
//! each hold one connection while waiting for a second, and stall the pool.
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokuly_core::{events as core_events, ids::Ref, outbound, prelude::*};
use tokuly_media::assets::{self, AssetRow, image_from_json};
use tokuly_proto::{
    common::{Image, SubjectType},
    media::{AssetKind, AssetPurpose, AssetStatus, VideoInfo},
    shorts::{CoverKind, RemixKind},
};
use tokuly_video::channels::{ChannelRow, NotifyLevel};

use crate::{
    Shorts,
    access::{self, PROCESSING, PUBLISHED, ShortFacts, Viewer, WITHDRAWN},
    config::ShortsConfig,
    events::{CoverJob, JOB_COVER, SHORT_CREATED, SHORT_DELETED, ShortEvent},
    listing,
    sounds::{self, SoundChoice},
    text,
};

pub(crate) type Tx<'a> = sqlx::Transaction<'a, sqlx::Postgres>;

pub const COVER_POSTER: i32 = CoverKind::COVER_KIND_POSTER as i32;
pub const COVER_FRAME: i32 = CoverKind::COVER_KIND_FRAME as i32;
pub const COVER_IMAGE: i32 = CoverKind::COVER_KIND_IMAGE as i32;

/// Pins a channel's grid may carry.
pub const MAX_PINS: i64 = 3;
/// Slack on the length limit: containers round durations, and a 180.2 s
/// upload is what someone who trimmed to "3 minutes" produces.
const DURATION_TOLERANCE_SECONDS: f64 = 0.5;
/// Slack on the aspect limit, for sizes rounded to even pixels.
const ASPECT_TOLERANCE: f64 = 0.01;
/// Accounts one creator may newly mention per hour, over all their shorts.
const MENTIONS_PER_HOUR: u32 = 100;

/// A mention as stored with the short.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredMention {
    pub user_id: Uuid,
    pub handle: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ShortRow {
    pub id: Uuid,
    pub short_id: String,
    pub channel_id: Uuid,
    /// The channel's owner.
    pub owner_id: Uuid,
    /// The media asset being played.
    pub asset_id: Uuid,
    /// The asset is the short's own upload (deleted with it), not a clip's media.
    pub owns_asset: bool,
    pub caption: String,
    pub hashtags: Vec<String>,
    /// `[StoredMention]` as JSON.
    pub mentions: serde_json::Value,
    /// Accounts that have been told they are mentioned here (it never shrinks).
    pub mentions_told: Vec<Uuid>,
    /// `tokuly.common.v1.Image` as proto JSON.
    pub cover: Option<serde_json::Value>,
    pub cover_kind: i32,
    pub cover_asset_id: Option<Uuid>,
    pub cover_offset_ms: Option<i32>,
    pub cover_pending_asset_id: Option<Uuid>,
    pub visibility: i32,
    pub comments_enabled: bool,
    pub allow_remix: bool,
    pub allow_duet: bool,
    pub notify_subscribers: bool,
    pub sound_id: Option<Uuid>,
    pub remix_of_id: Option<Uuid>,
    pub remix_kind: i32,
    pub source_type: i32,
    pub source_id: Option<Uuid>,
    pub status: i32,
    /// When it was last reported, and when its owner deleted it after that.
    pub reported_at: Option<Time>,
    pub withdrawn_at: Option<Time>,
    pub processing_error: String,
    /// `tokuly.media.v1.VideoInfo` as proto JSON, once processed.
    pub playback: Option<serde_json::Value>,
    pub duration_seconds: f64,
    pub width: i32,
    pub height: i32,
    pub view_count: i64,
    pub like_count: i64,
    pub comment_count: i64,
    pub share_count: i64,
    pub save_count: i64,
    pub loop_count: i64,
    pub remix_count: i64,
    pub watch_ratio_sum: f64,
    pub watch_samples: i64,
    pub score: f64,
    pub trending: f64,
    pub milestone: i64,
    pub pinned_at: Option<Time>,
    pub suspended: bool,
    /// Counted as a public short right now (see [`listing`]).
    pub listed: bool,
    pub announced_at: Option<Time>,
    /// How far the subscriber notifications got (see `workers`).
    pub announce_cursor: Option<Uuid>,
    pub published_at: Option<Time>,
    pub created_at: Time,
    pub updated_at: Time,
}

macro_rules! short_columns {
    () => {
        "s.id, s.short_id, s.channel_id, s.owner_id, s.asset_id, s.owns_asset, s.caption, s.hashtags, s.mentions, \
         s.mentions_told, s.cover, s.cover_kind, s.cover_asset_id, s.cover_offset_ms, s.cover_pending_asset_id, \
         s.visibility, s.comments_enabled, s.allow_remix, s.allow_duet, s.notify_subscribers, s.sound_id, s.remix_of_id, \
         s.remix_kind, s.source_type, s.source_id, s.status, s.reported_at, s.withdrawn_at, s.processing_error, \
         s.playback, s.duration_seconds, s.width, s.height, s.view_count, s.like_count, s.comment_count, s.share_count, \
         s.save_count, s.loop_count, s.remix_count, s.watch_ratio_sum, s.watch_samples, s.score, s.trending, \
         s.milestone, s.pinned_at, s.suspended, s.listed, s.announced_at, s.announce_cursor, s.published_at, \
         s.created_at, s.updated_at"
    };
}
pub(crate) use short_columns;

impl ShortRow {
    pub fn facts(&self) -> ShortFacts {
        ShortFacts {
            owner_id: self.owner_id,
            status: self.status,
            visibility: self.visibility,
            suspended: self.suspended,
        }
    }

    pub fn is_published(&self) -> bool {
        self.status == PUBLISHED
    }

    /// Published for everyone and listed everywhere (search, feeds, pages).
    pub fn is_public(&self) -> bool {
        self.facts().is_public()
    }

    /// Path of the short in the web app.
    pub fn path(&self) -> String {
        format!("/shorts/{}", self.short_id)
    }

    /// Path of the short in the owner's studio.
    pub fn studio_path(&self) -> String {
        format!("/studio/shorts/{}", self.short_id)
    }

    pub fn cover(&self) -> Option<Image> {
        image_from_json(self.cover.as_ref())
    }

    pub fn cover_url(&self) -> Option<String> {
        self.cover().map(|image| image.url).filter(|url| !url.is_empty())
    }

    pub fn playback(&self) -> Option<VideoInfo> {
        self.playback.clone().and_then(|value| serde_json::from_value(value).ok())
    }

    pub fn mentions(&self) -> Vec<StoredMention> {
        serde_json::from_value(self.mentions.clone()).unwrap_or_default()
    }

    /// Deleted by its owner after a report: gone for everyone but moderators.
    pub fn is_withdrawn(&self) -> bool {
        self.status == WITHDRAWN
    }

    /// Average share of the short that was watched per play; 0 without plays.
    pub fn average_watch_ratio(&self) -> f64 {
        if self.watch_samples > 0 { (self.watch_ratio_sum / self.watch_samples as f64).clamp(0.0, 1.0) } else { 0.0 }
    }

    /// One line that stands for the short where a title is needed
    /// (notifications, reports, search): the start of its caption.
    pub fn title(&self) -> String {
        let line = text::excerpt(&self.caption, 60);
        if line.is_empty() { "ショート".to_owned() } else { line }
    }

    pub(crate) fn event(&self, first_publication: bool, reason: &str) -> ShortEvent {
        ShortEvent {
            id: self.id,
            short_id: self.short_id.clone(),
            channel_id: self.channel_id,
            owner_id: self.owner_id,
            visibility: self.visibility,
            first_publication,
            reason: reason.to_owned(),
        }
    }
}

pub(crate) fn not_found() -> AppError {
    AppError::not_found("SHORT_NOT_FOUND", "short not found")
}

/// Holder under which a short keeps its video and its cover image alive.
pub fn holder(short_id: Uuid) -> String {
    format!("shorts:short:{short_id}")
}

pub(crate) fn mentions_json(mentions: &[StoredMention]) -> AppResult<serde_json::Value> {
    serde_json::to_value(mentions).map_err(|e| AppError::internal(anyhow::anyhow!("mentions encode: {e}")))
}

// ───────────────────────────── lookups ─────────────────────────────

/// A short by UUID or short ID, whatever its state. Callers decide who may
/// see it ([`access::can_watch`]).
pub async fn find(core: &Core, reference: &Ref) -> AppResult<Option<ShortRow>> {
    Ok(match reference {
        Ref::Id(id) => {
            sqlx::query_as(concat!("SELECT ", short_columns!(), " FROM shorts s WHERE s.id = $1"))
                .bind(id)
                .fetch_optional(&core.db)
                .await?
        }
        Ref::Short(short_id) => {
            sqlx::query_as(concat!("SELECT ", short_columns!(), " FROM shorts s WHERE s.short_id = $1"))
                .bind(short_id)
                .fetch_optional(&core.db)
                .await?
        }
    })
}

pub async fn find_by_id(core: &Core, short_id: Uuid) -> AppResult<Option<ShortRow>> {
    find(core, &Ref::Id(short_id)).await
}

/// A short by what clients send as `short_id`. A malformed reference names
/// no short.
pub async fn find_ref(core: &Core, reference: &str) -> AppResult<Option<ShortRow>> {
    match Ref::parse(reference.trim()) {
        Ok(reference) => find(core, &reference).await,
        Err(_) => Ok(None),
    }
}

/// The short a client named, if `viewer` may watch it (`SHORT_NOT_FOUND`
/// otherwise, whatever the reason).
pub async fn find_watchable(core: &Core, viewer: &Viewer, reference: &str) -> AppResult<ShortRow> {
    find_ref(core, reference).await?.filter(|short| access::can_watch(viewer, &short.facts())).ok_or_else(not_found)
}

/// The short a client named, which must belong to `owner_id` (and not have
/// been deleted by them already).
pub async fn find_owned(core: &Core, owner_id: Uuid, reference: &str) -> AppResult<ShortRow> {
    find_ref(core, reference)
        .await?
        .filter(|short| short.owner_id == owner_id && !short.is_withdrawn())
        .ok_or_else(not_found)
}

/// The row, locked for the rest of the transaction.
pub(crate) async fn find_in(tx: &mut Tx<'_>, short_id: Uuid) -> AppResult<Option<ShortRow>> {
    Ok(sqlx::query_as(concat!("SELECT ", short_columns!(), " FROM shorts s WHERE s.id = $1 FOR UPDATE"))
        .bind(short_id)
        .fetch_optional(&mut **tx)
        .await?)
}

/// The rows with the given IDs, in the order of `ids`; unknown IDs are skipped.
pub async fn find_many(core: &Core, ids: &[Uuid]) -> AppResult<Vec<ShortRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut rows: Vec<ShortRow> =
        sqlx::query_as(concat!("SELECT ", short_columns!(), " FROM shorts s WHERE s.id = ANY($1)"))
            .bind(ids)
            .fetch_all(&core.db)
            .await?;
    let position: std::collections::HashMap<Uuid, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    rows.sort_by_key(|row| position.get(&row.id).copied().unwrap_or(usize::MAX));
    Ok(rows)
}

// ───────────────────────────── validation ─────────────────────────────

/// A caption as stored (`CAPTION_TOO_LONG` when it does not fit).
pub fn validate_caption(input: &str) -> AppResult<String> {
    let caption = text::clean(input);
    ensure!(
        text::fits(&caption, text::MAX_CAPTION_CHARS),
        invalid,
        "CAPTION_TOO_LONG",
        "a caption has at most {} characters",
        text::MAX_CAPTION_CHARS
    );
    Ok(caption)
}

/// A visibility named by a client; `default` when it named none.
pub fn validate_visibility(value: buffa::EnumValue<tokuly_proto::common::Visibility>, default: i32) -> AppResult<i32> {
    use tokuly_proto::common::Visibility;
    match value.as_known() {
        Some(Visibility::VISIBILITY_UNSPECIFIED) => Ok(default),
        Some(known) => Ok(known as i32),
        None => Err(AppError::invalid("VISIBILITY_INVALID", "unknown visibility")),
    }
}

/// Why a processed video cannot be a short, if it cannot: the reason that
/// is sent to clients and stored as the processing error.
pub fn format_problem(cfg: &ShortsConfig, info: &VideoInfo) -> Option<&'static str> {
    if info.duration_seconds > cfg.max_duration_seconds + DURATION_TOLERANCE_SECONDS {
        return Some("SHORT_TOO_LONG");
    }
    if info.width > 0
        && info.height > 0
        && f64::from(info.width) / f64::from(info.height) > cfg.max_aspect_ratio + ASPECT_TOLERANCE
    {
        return Some("SHORT_NOT_VERTICAL");
    }
    None
}

fn format_error(problem: &'static str, cfg: &ShortsConfig) -> AppError {
    match problem {
        "SHORT_TOO_LONG" => {
            AppError::invalid(problem, format!("a short is at most {} seconds long", cfg.max_duration_seconds))
        }
        _ => AppError::invalid(problem, "a short is vertical, square, or landscape no wider than 16:9"),
    }
}

/// The users behind the `@handles` of a caption, in order of appearance.
/// Handles that name nobody, a suspended account, or someone in a block
/// with the owner stay plain text.
async fn resolve_mentions(core: &Core, owner: &Viewer, handles: &[String]) -> AppResult<Vec<StoredMention>> {
    if handles.is_empty() {
        return Ok(Vec::new());
    }
    let found: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, handle::text FROM users WHERE handle = ANY($1::citext[]) AND deleted_at IS NULL AND banned_at IS NULL",
    )
    .bind(handles)
    .fetch_all(&core.db)
    .await?;
    Ok(handles
        .iter()
        .filter_map(|handle| {
            let (user_id, _) = found.iter().find(|(_, stored)| stored.eq_ignore_ascii_case(handle))?;
            (!owner.hides(*user_id)).then(|| StoredMention { user_id: *user_id, handle: handle.clone() })
        })
        .collect())
}

// ───────────────────────────── creation ─────────────────────────────

/// Which picture stands for a short.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverChoice {
    /// The media pipeline's poster frame.
    Poster,
    /// The frame at this offset, in milliseconds.
    Frame(i32),
    /// An image upload of the owner.
    Image(Uuid),
}

/// A cover choice checked against the owner's uploads.
enum CoverPlan {
    Poster,
    Frame(i32),
    Image { asset_id: Uuid, image: serde_json::Value },
}

/// Everything needed to create a short. The channel must already have been
/// checked to belong to the caller.
#[derive(Debug, Clone)]
pub struct NewShort {
    pub channel: ChannelRow,
    pub asset_id: Uuid,
    pub caption: String,
    /// `tokuly.common.v1.Visibility` as its number.
    pub visibility: i32,
    pub comments_enabled: bool,
    pub allow_remix: bool,
    pub allow_duet: bool,
    pub notify_subscribers: bool,
    pub cover: CoverChoice,
    pub sound: SoundChoice,
    /// What the short was made from, as clients name it.
    pub source: Option<(SubjectType, String)>,
    /// The short it is a remix or duet of, and the `RemixKind` number.
    pub remix: Option<(String, i32)>,
}

/// The asset a new short will play.
struct Media {
    asset: AssetRow,
    owns_asset: bool,
    source: Option<(i32, Uuid)>,
}

impl Shorts {
    /// Creates a short and returns its ID. A short whose asset is already
    /// READY is published before this returns.
    pub async fn create(&self, new: NewShort) -> AppResult<Uuid> {
        let core = &self.core;
        let owner_id = new.channel.owner_id;
        let owner = Viewer::load(core, Some(owner_id)).await?;
        let caption = validate_caption(&new.caption)?;
        let parsed = text::parse(&caption);
        let mentions = resolve_mentions(core, &owner, &parsed.handles).await?;
        let media = self.resolve_media(owner_id, new.asset_id, new.source.as_ref()).await?;
        if let Some(problem) =
            media.asset.video().filter(|_| media.asset.is_ready()).and_then(|info| format_problem(&self.cfg, &info))
        {
            return Err(format_error(problem, &self.cfg));
        }
        let cover = self.plan_cover(owner_id, &new.cover).await?;
        let remix = match &new.remix {
            Some((reference, kind)) => Some(resolve_remix(core, &owner, reference, *kind).await?),
            None => None,
        };
        let sound = sounds::plan(core, &owner, &new.sound).await?;
        self.charge_mentions(owner_id, &mentions, &[]).await?;

        let short_id = new_id();
        let (cover_kind, cover_image, cover_asset_id, cover_offset_ms) = match &cover {
            CoverPlan::Poster => (COVER_POSTER, None, None, None),
            CoverPlan::Frame(offset_ms) => (COVER_FRAME, None, None, Some(*offset_ms)),
            CoverPlan::Image { asset_id, image } => (COVER_IMAGE, Some(image), Some(*asset_id), None),
        };
        let mut tx = core.db.begin().await?;
        sqlx::query(
            "INSERT INTO shorts (id, short_id, channel_id, owner_id, asset_id, owns_asset, caption, hashtags, mentions,
                                 cover, cover_kind, cover_asset_id, cover_offset_ms, visibility, comments_enabled,
                                 allow_remix, allow_duet, notify_subscribers, remix_of_id, remix_kind, source_type,
                                 source_id, suspended)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23)",
        )
        .bind(short_id)
        .bind(tokuly_core::ids::new_short_id())
        .bind(new.channel.id)
        .bind(owner_id)
        .bind(media.asset.id)
        .bind(media.owns_asset)
        .bind(&caption)
        .bind(&parsed.hashtags)
        .bind(mentions_json(&mentions)?)
        .bind(cover_image)
        .bind(cover_kind)
        .bind(cover_asset_id)
        .bind(cover_offset_ms)
        .bind(new.visibility)
        .bind(new.comments_enabled)
        .bind(new.allow_remix)
        .bind(new.allow_duet)
        .bind(new.notify_subscribers)
        .bind(remix.map(|(id, _)| id))
        .bind(remix.map(|(_, kind)| kind).unwrap_or(0))
        .bind(media.source.map(|(subject_type, _)| subject_type).unwrap_or(0))
        .bind(media.source.map(|(_, id)| id))
        .bind(new.channel.suspended)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            let error = AppError::from(e);
            if error.is_already_exists() {
                AppError::already_exists("ASSET_ALREADY_USED", "this upload already belongs to a short")
            } else {
                error
            }
        })?;
        sounds::materialize_in(&mut tx, &sound, short_id).await?;
        assets::attach(&mut *tx, media.asset.id, &holder(short_id)).await?;
        if let Some(asset_id) = cover_asset_id {
            assets::attach(&mut *tx, asset_id, &holder(short_id)).await?;
        }
        let row = find_in(&mut tx, short_id).await?.ok_or_else(not_found)?;
        core_events::emit(&mut *tx, SHORT_CREATED, Some(owner_id), &row.event(false, "")).await?;
        let first_on_channel: bool =
            sqlx::query_scalar("SELECT NOT EXISTS (SELECT 1 FROM shorts WHERE channel_id = $1 AND id <> $2)")
                .bind(new.channel.id)
                .bind(short_id)
                .fetch_one(&mut *tx)
                .await?;
        tx.commit().await?;

        if first_on_channel {
            self.adopt_subscribers(new.channel.id).await?;
        }
        // A READY asset publishes the short right away; one that finished
        // between our read and the insert is caught up here too (its `ready`
        // event found no short to update).
        self.sync_processing(&row).await?;
        Ok(short_id)
    }

    /// Counts the accounts a caption mentions that have not been told about
    /// this short yet against the creator's hourly allowance
    /// (`RATE_LIMITED` when it is used up): mentions notify people, and
    /// nobody gets to notify a crowd.
    async fn charge_mentions(&self, owner_id: Uuid, mentions: &[StoredMention], told: &[Uuid]) -> AppResult<()> {
        let fresh = mentions.iter().filter(|mention| mention.user_id != owner_id && !told.contains(&mention.user_id));
        for _ in fresh {
            let key = format!("shorts-mention:{owner_id}");
            self.core.cache.rate_limit(&key, MENTIONS_PER_HOUR, Duration::from_secs(3600)).await?;
        }
        Ok(())
    }

    /// Copies a channel's subscribers into `short_follows`. Subscriptions
    /// are followed through events from then on; this catches the ones made
    /// before the channel had shorts (or before this domain was deployed).
    async fn adopt_subscribers(&self, channel_id: Uuid) -> AppResult<()> {
        let subscribers = tokuly_video::channels::subscriber_ids(&self.core, channel_id, NotifyLevel::None).await?;
        if subscribers.is_empty() {
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO short_follows (user_id, channel_id) SELECT user_id, $2 FROM unnest($1::uuid[]) AS user_id
             ON CONFLICT DO NOTHING",
        )
        .bind(&subscribers)
        .bind(channel_id)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    /// Checks the asset a client wants to turn into a short.
    ///
    /// An upload of the caller made for shorts may still be processing. Any
    /// other asset only qualifies as the media of a clip the caller created,
    /// named by `source`; it must be READY. Whether a clip is the caller's
    /// and plays this asset is the live domain's knowledge
    /// ([`crate::ClipDirectory`]); without it, only an asset the caller owns
    /// is trusted, on the word of the clip's subject resolver.
    async fn resolve_media(
        &self,
        user_id: Uuid,
        asset_id: Uuid,
        source: Option<&(SubjectType, String)>,
    ) -> AppResult<Media> {
        let core = &self.core;
        let asset_not_found = || AppError::not_found("ASSET_NOT_FOUND", "asset not found");
        let asset = assets::find(core, asset_id).await?.ok_or_else(asset_not_found)?;
        let own = asset.owner_id == user_id;
        let unusable = || {
            // Never confirm that someone else's asset exists.
            if own {
                AppError::invalid("ASSET_WRONG_PURPOSE", "this upload cannot be used here")
            } else {
                asset_not_found()
            }
        };
        let state = |asset: &AssetRow, pending: &'static str| -> AppResult<()> {
            ensure!(asset.has_kind(AssetKind::ASSET_KIND_VIDEO), invalid, "ASSET_WRONG_PURPOSE", "a video is required");
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
                "{pending}"
            );
            Ok(())
        };

        if own && asset.purpose == AssetPurpose::ASSET_PURPOSE_SHORT as i32 {
            state(&asset, "finish the upload first")?;
            let source = match source {
                Some((subject_type, id)) => Some(self.resolve_source(user_id, *subject_type, id).await?),
                None => None,
            };
            return Ok(Media { asset, owns_asset: true, source });
        }

        let Some((SubjectType::SUBJECT_TYPE_CLIP, clip_ref)) = source else { return Err(unusable()) };
        let source_not_found =
            || AppError::not_found("SOURCE_NOT_FOUND", "the clip does not exist, is not yours, or plays other media");
        let clip_id = match self.clip_directory() {
            Some(clips) => match clips.clip_of(user_id, clip_ref).await? {
                Some(clip) if clip.asset_id == asset.id => clip.clip_id,
                _ => return Err(if own { source_not_found() } else { asset_not_found() }),
            },
            None => {
                if !own {
                    return Err(asset_not_found());
                }
                // The caller's own media, held by something, and a clip of
                // theirs to go with it: the most that can be checked here.
                match self.visible_subject(user_id, SubjectType::SUBJECT_TYPE_CLIP, clip_ref).await? {
                    Some(clip) if clip.owner_id == user_id && asset.is_attached() => clip.id,
                    _ => return Err(source_not_found()),
                }
            }
        };
        state(&asset, "the clip is still being processed")?;
        ensure!(asset.is_ready(), precondition, "ASSET_NOT_READY", "the clip is still being processed");
        Ok(Media { asset, owns_asset: false, source: Some((SubjectType::SUBJECT_TYPE_CLIP as i32, clip_id)) })
    }

    /// A subject as `user_id` sees it, through the registry of subject
    /// resolvers. `None` whatever the reason it cannot be pointed at: no
    /// such thing, not visible, no domain serving that type.
    async fn visible_subject(
        &self,
        user_id: Uuid,
        subject_type: SubjectType,
        id: &str,
    ) -> AppResult<Option<tokuly_core::subject::SubjectInfo>> {
        match self.core.subjects.resolve(subject_type, id.trim(), Some(user_id)).await {
            Ok(info) => Ok(Some(info)),
            Err(error) if error.code == connectrpc::ErrorCode::Internal => Err(error),
            Err(_) => Ok(None),
        }
    }

    /// What a short says it was made from: a clip, video or live stream the
    /// caller can see. Returns the subject type number and the subject's UUID.
    async fn resolve_source(&self, user_id: Uuid, subject_type: SubjectType, id: &str) -> AppResult<(i32, Uuid)> {
        ensure!(
            matches!(
                subject_type,
                SubjectType::SUBJECT_TYPE_CLIP
                    | SubjectType::SUBJECT_TYPE_VIDEO
                    | SubjectType::SUBJECT_TYPE_LIVE_STREAM
            ),
            invalid,
            "SOURCE_INVALID",
            "a short can be made from a clip, a video or a live stream"
        );
        match self.visible_subject(user_id, subject_type, id).await? {
            Some(info) => Ok((subject_type as i32, info.id)),
            None => Err(AppError::not_found("SOURCE_NOT_FOUND", "the source does not exist or is not visible to you")),
        }
    }

    async fn plan_cover(&self, owner_id: Uuid, choice: &CoverChoice) -> AppResult<CoverPlan> {
        match choice {
            CoverChoice::Poster => Ok(CoverPlan::Poster),
            CoverChoice::Frame(offset_ms) => {
                let longest = (self.cfg.max_duration_seconds + DURATION_TOLERANCE_SECONDS) * 1000.0;
                ensure!(
                    *offset_ms >= 0 && f64::from(*offset_ms) <= longest,
                    invalid,
                    "COVER_OFFSET_INVALID",
                    "the cover offset must lie within the short"
                );
                Ok(CoverPlan::Frame(*offset_ms))
            }
            CoverChoice::Image(asset_id) => {
                let image = assets::require_image_json(
                    &self.core,
                    *asset_id,
                    owner_id,
                    &[AssetPurpose::ASSET_PURPOSE_THUMBNAIL],
                )
                .await?;
                Ok(CoverPlan::Image { asset_id: *asset_id, image })
            }
        }
    }
}

/// The short a new one builds on: it must be live, visible to the creator
/// and allow that kind of reuse (its own owner always may).
async fn resolve_remix(core: &Core, creator: &Viewer, reference: &str, kind: i32) -> AppResult<(Uuid, i32)> {
    let duet = RemixKind::REMIX_KIND_DUET as i32;
    ensure!(
        kind == RemixKind::REMIX_KIND_REMIX as i32 || kind == duet,
        invalid,
        "REMIX_KIND_INVALID",
        "say whether this is a remix or a duet"
    );
    let original = find_ref(core, reference)
        .await?
        .filter(|short| short.is_published() && access::can_watch(creator, &short.facts()))
        .ok_or_else(|| AppError::not_found("REMIX_NOT_FOUND", "the short to remix does not exist"))?;
    let allowed =
        creator.is(original.owner_id) || if kind == duet { original.allow_duet } else { original.allow_remix };
    ensure!(allowed, denied, "REMIX_NOT_ALLOWED", "the creator of that short does not allow this");
    Ok((original.id, kind))
}

// ───────────────────────────── processing results ─────────────────────────────

impl Shorts {
    /// Brings a draft in step with the state of its media asset: READY
    /// publishes it (or fails it, when the video is too long or too wide);
    /// FAILED records why. Idempotent; called after creation, for
    /// `media.asset.ready` / `failed`, and by the worker that catches missed
    /// events.
    pub async fn sync_processing(&self, short: &ShortRow) -> AppResult<()> {
        if short.status != PROCESSING {
            return Ok(());
        }
        let Some(asset) = assets::find(&self.core, short.asset_id).await? else { return Ok(()) };
        if asset.is_ready() {
            match asset.video() {
                Some(info) => match format_problem(&self.cfg, &info) {
                    None => self.apply_ready(short, &asset, &info).await,
                    Some(problem) => self.apply_failed(short, problem).await,
                },
                None => self.apply_failed(short, "PROCESSING_FAILED").await,
            }
        } else if asset.has_status(AssetStatus::ASSET_STATUS_FAILED) {
            self.apply_failed(short, if asset.error.is_empty() { "PROCESSING_FAILED" } else { asset.error.as_str() })
                .await
        } else {
            Ok(())
        }
    }

    async fn apply_ready(&self, short: &ShortRow, asset: &AssetRow, info: &VideoInfo) -> AppResult<()> {
        let core = &self.core;
        let context = listing::Context::load(core, short).await?;
        let mut tx = core.db.begin().await?;
        // Until a cover of the creator's choosing exists, the poster stands in.
        let updated: Option<Uuid> = sqlx::query_scalar(
            "UPDATE shorts SET status = 2, processing_error = '', playback = $2, duration_seconds = $3, width = $4,
                    height = $5, published_at = now(),
                    cover = CASE WHEN cover_asset_id IS NULL THEN $6 ELSE cover END
             WHERE id = $1 AND status = 1
             RETURNING id",
        )
        .bind(short.id)
        .bind(&asset.video)
        .bind(info.duration_seconds)
        .bind(info.width)
        .bind(info.height)
        .bind(&asset.image)
        .fetch_optional(&mut *tx)
        .await?;
        if updated.is_none() {
            return Ok(()); // someone else got here first
        }
        if !info.has_audio {
            // A silent video has no sound of its own to offer.
            sounds::drop_own_original_in(&mut tx, short.id).await?;
        }
        let row = listing::after_change_in(&mut tx, short.id, true, &context).await?;
        tx.commit().await?;
        if let (COVER_FRAME, Some(offset_ms)) = (row.cover_kind, row.cover_offset_ms) {
            self.queue_cover(row.id, offset_ms).await?;
        }
        Ok(())
    }

    async fn apply_failed(&self, short: &ShortRow, error: &str) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let updated: Option<Uuid> = sqlx::query_scalar(
            "UPDATE shorts SET status = 3, processing_error = $2 WHERE id = $1 AND status = 1 RETURNING id",
        )
        .bind(short.id)
        .bind(error)
        .fetch_optional(&mut *tx)
        .await?;
        if updated.is_none() {
            return Ok(());
        }
        let (ja, en) = match error {
            "SHORT_TOO_LONG" => (
                format!("ショートは{}秒までだよ。短くしてもう一度投稿してね!", self.cfg.max_duration_seconds),
                format!("Shorts can be up to {} seconds long. Trim it and post again.", self.cfg.max_duration_seconds),
            ),
            "SHORT_NOT_VERTICAL" => (
                "横長すぎる動画はショートにできないよ。縦長の動画で試してみてね!".to_owned(),
                "This video is too wide for a short. Try a vertical one.".to_owned(),
            ),
            _ => (
                "ショートを処理できなかったよ。別のファイルで試してみてね!".to_owned(),
                "Your short could not be processed. Try uploading another file.".to_owned(),
            ),
        };
        let request = outbound::NotificationRequest::new(
            short.owner_id,
            "shorts.processing_failed",
            outbound::Localized::new("ショートを投稿できなかったよ…", "Your short could not be posted"),
            outbound::Localized::new(ja, en),
            short.studio_path(),
        )
        .subject(SubjectType::SUBJECT_TYPE_SHORT, short.id)
        .collapse(format!("shorts.processing:{}", short.id))
        .data(serde_json::json!({ "id": short.id, "short_id": short.short_id, "error": error }));
        outbound::notify(&mut *tx, &request).await?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn queue_cover(&self, short_id: Uuid, offset_ms: i32) -> AppResult<()> {
        let job = CoverJob { short_id, offset_ms };
        self.core.jobs.enqueue_unique(JOB_COVER, &format!("{short_id}:{offset_ms}"), &job).await
    }

    /// Catches up what an event should have moved but did not (an event
    /// consumed before the row existed, a dropped event, a lost job):
    /// drafts whose asset finished, cover frames that were processed, and
    /// cover frames that were never cut. Returns how many shorts it touched.
    pub async fn reconcile(&self) -> AppResult<u64> {
        let core = &self.core;
        let mut touched = 0;
        let drafts: Vec<ShortRow> = sqlx::query_as(concat!(
            "SELECT ",
            short_columns!(),
            " FROM shorts s WHERE s.status = 1 AND s.created_at < now() - interval '1 minute' ORDER BY s.id LIMIT 500"
        ))
        .fetch_all(&core.db)
        .await?;
        for draft in drafts {
            self.sync_processing(&draft).await?;
            if find_by_id(core, draft.id).await?.is_some_and(|row| row.status != PROCESSING) {
                touched += 1;
            }
        }
        let pending: Vec<Uuid> = sqlx::query_scalar(
            "SELECT cover_pending_asset_id FROM shorts
             WHERE cover_pending_asset_id IS NOT NULL AND updated_at < now() - interval '1 minute' LIMIT 500",
        )
        .fetch_all(&core.db)
        .await?;
        for asset_id in pending {
            if self.sync_cover(asset_id).await? {
                touched += 1;
            }
        }
        let uncut: Vec<(Uuid, i32)> = sqlx::query_as(
            "SELECT id, cover_offset_ms FROM shorts
             WHERE status = 2 AND cover_kind = 2 AND cover_asset_id IS NULL AND cover_pending_asset_id IS NULL
               AND cover_offset_ms IS NOT NULL AND updated_at < now() - interval '10 minutes'
             LIMIT 200",
        )
        .fetch_all(&core.db)
        .await?;
        for (short_id, offset_ms) in uncut {
            self.queue_cover(short_id, offset_ms).await?;
            touched += 1;
        }
        Ok(touched)
    }
}

// ───────────────────────────── editing ─────────────────────────────

/// What a creator wants a short to look like. `None` = leave as it is.
#[derive(Debug, Default)]
pub struct ShortEdit {
    pub caption: Option<String>,
    pub visibility: Option<i32>,
    pub comments_enabled: Option<bool>,
    pub allow_remix: Option<bool>,
    pub allow_duet: Option<bool>,
    pub cover: Option<CoverChoice>,
}

impl Shorts {
    /// Applies `edit` to a short of its owner and returns the new row.
    pub async fn edit(&self, short: &ShortRow, edit: ShortEdit) -> AppResult<ShortRow> {
        let core = &self.core;
        let caption = edit.caption.as_deref().map(validate_caption).transpose()?;
        let rewritten = match &caption {
            Some(caption) => {
                let owner = Viewer::load(core, Some(short.owner_id)).await?;
                let parsed = text::parse(caption);
                let mentions = resolve_mentions(core, &owner, &parsed.handles).await?;
                self.charge_mentions(short.owner_id, &mentions, &short.mentions_told).await?;
                Some((parsed.hashtags, mentions))
            }
            None => None,
        };
        let cover = match &edit.cover {
            Some(choice) => Some(self.plan_cover(short.owner_id, choice).await?),
            None => None,
        };

        let context = listing::Context::load(core, short).await?;
        let mut aftermath = Aftermath::default();
        let mut tx = core.db.begin().await?;
        let current =
            find_in(&mut tx, short.id).await?.filter(|current| !current.is_withdrawn()).ok_or_else(not_found)?;
        sqlx::query(
            "UPDATE shorts SET caption = COALESCE($2, caption), hashtags = COALESCE($3, hashtags),
                    mentions = COALESCE($4, mentions), visibility = COALESCE($5, visibility),
                    comments_enabled = COALESCE($6, comments_enabled), allow_remix = COALESCE($7, allow_remix),
                    allow_duet = COALESCE($8, allow_duet)
             WHERE id = $1",
        )
        .bind(short.id)
        .bind(&caption)
        .bind(rewritten.as_ref().map(|(hashtags, _)| hashtags))
        .bind(rewritten.as_ref().map(|(_, mentions)| mentions_json(mentions)).transpose()?)
        .bind(edit.visibility)
        .bind(edit.comments_enabled)
        .bind(edit.allow_remix)
        .bind(edit.allow_duet)
        .execute(&mut *tx)
        .await?;
        if let Some(cover) = &cover {
            release_cover_in(&mut tx, &current, &mut aftermath).await?;
            let (kind, image, asset_id, offset_ms) = match cover {
                CoverPlan::Poster => (COVER_POSTER, None, None, None),
                CoverPlan::Frame(offset_ms) => (COVER_FRAME, None, None, Some(*offset_ms)),
                CoverPlan::Image { asset_id, image } => (COVER_IMAGE, Some(image), Some(*asset_id), None),
            };
            if let Some(asset_id) = asset_id {
                assets::attach(&mut *tx, asset_id, &holder(short.id)).await?;
            }
            // Without an image of its own the cover is the poster frame
            // (for a FRAME cover: until the frame has been cut).
            sqlx::query(
                "UPDATE shorts SET cover = COALESCE($2, playback -> 'poster'), cover_kind = $3, cover_asset_id = $4,
                        cover_offset_ms = $5, cover_pending_asset_id = NULL
                 WHERE id = $1",
            )
            .bind(short.id)
            .bind(image)
            .bind(kind)
            .bind(asset_id)
            .bind(offset_ms)
            .execute(&mut *tx)
            .await?;
        }
        let row = listing::after_change_in(&mut tx, short.id, false, &context).await?;
        tx.commit().await?;
        aftermath.run(core).await;
        if let (Some(CoverPlan::Frame(offset_ms)), true) = (&cover, row.is_published()) {
            self.queue_cover(row.id, *offset_ms).await?;
        }
        Ok(row)
    }
}

/// Lets go of the image assets behind a short's current cover: the upload
/// of an IMAGE cover is released, a frame this domain cut is deleted.
pub(crate) async fn release_cover_in(tx: &mut Tx<'_>, short: &ShortRow, aftermath: &mut Aftermath) -> AppResult<()> {
    for asset_id in [short.cover_asset_id, short.cover_pending_asset_id].into_iter().flatten() {
        assets::detach(&mut **tx, asset_id, &holder(short.id)).await?;
        let cut_here = short.cover_kind == COVER_FRAME || Some(asset_id) == short.cover_pending_asset_id;
        if cut_here {
            aftermath.delete_if_unused(asset_id, short.owner_id);
        }
    }
    Ok(())
}

// ───────────────────────────── pins ─────────────────────────────

/// Pins a published short to its channel's grid. Returns the pinned row.
pub async fn pin(core: &Core, short: &ShortRow) -> AppResult<ShortRow> {
    ensure!(short.is_published(), precondition, "SHORT_NOT_PUBLISHED", "only published shorts can be pinned");
    let mut tx = core.db.begin().await?;
    // One pin change per channel at a time, so the limit holds.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("shorts:pins:{}", short.channel_id.simple()))
        .execute(&mut *tx)
        .await?;
    let current = find_in(&mut tx, short.id).await?.filter(ShortRow::is_published).ok_or_else(not_found)?;
    if current.pinned_at.is_none() {
        let pinned: i64 =
            sqlx::query_scalar("SELECT count(*) FROM shorts WHERE channel_id = $1 AND pinned_at IS NOT NULL")
                .bind(short.channel_id)
                .fetch_one(&mut *tx)
                .await?;
        ensure!(pinned < MAX_PINS, precondition, "PIN_LIMIT_REACHED", "a channel can pin at most {MAX_PINS} shorts");
        sqlx::query("UPDATE shorts SET pinned_at = now() WHERE id = $1").bind(short.id).execute(&mut *tx).await?;
    }
    let row = find_in(&mut tx, short.id).await?.ok_or_else(not_found)?;
    tx.commit().await?;
    Ok(row)
}

/// Removes a short's pin. Returns the row.
pub async fn unpin(core: &Core, short: &ShortRow) -> AppResult<ShortRow> {
    sqlx::query_as(concat!("UPDATE shorts s SET pinned_at = NULL WHERE s.id = $1 RETURNING ", short_columns!()))
        .bind(short.id)
        .fetch_optional(&core.db)
        .await?
        .ok_or_else(not_found)
}

// ───────────────────────────── deletion ─────────────────────────────

/// Why a short is deleted, as carried by `shorts.short.deleted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removal {
    /// The owner deleted it.
    Owner,
    /// Its channel was deleted.
    ChannelDeleted,
    /// The owner's account was erased.
    AccountDeleted,
    /// A moderator took it down: the upload goes whatever else holds it.
    Moderation,
    /// The media it played was deleted under it.
    AssetDeleted,
}

impl Removal {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::ChannelDeleted => "channel_deleted",
            Self::AccountDeleted => "account_deleted",
            Self::Moderation => "moderation",
            Self::AssetDeleted => "asset_deleted",
        }
    }
}

/// Work left over from a change that can only happen once its transaction
/// has committed: deleting media nobody refers to any more.
#[derive(Debug, Default)]
pub struct Aftermath {
    /// `(asset, on whose behalf)`: `None` deletes regardless of other holders.
    assets: Vec<(Uuid, Option<Uuid>)>,
    /// Whether the change found something to remove (of two deletions
    /// racing for one short, only one does).
    pub removed: bool,
}

impl Aftermath {
    fn delete_if_unused(&mut self, asset_id: Uuid, owner_id: Uuid) {
        self.assets.push((asset_id, Some(owner_id)));
    }

    fn delete_by_force(&mut self, asset_id: Uuid) {
        self.assets.push((asset_id, None));
    }

    pub fn merge(&mut self, other: Aftermath) {
        self.removed |= other.removed;
        self.assets.extend(other.assets);
    }

    /// Deletes the collected assets. An asset something else still holds
    /// (a clip, another short's cover) stays; failures are logged, the
    /// media janitor collects what is left behind.
    pub async fn run(self, core: &Core) {
        for (asset_id, owner_id) in self.assets {
            match assets::delete(core, asset_id, owner_id).await {
                Ok(_) => {}
                Err(error) if error.reason == "ASSET_IN_USE" => {}
                Err(error) => tracing::warn!(%asset_id, %error, "could not delete the media of a deleted short"),
            }
        }
    }
}

/// Deletes a short inside `tx`: it leaves the hashtag and sound pages and
/// the search index, releases its media, and takes its saves, likes and
/// statistics with it. `actor` is who caused it (`None` for the system).
/// Run the returned [`Aftermath`] once the transaction has committed.
pub async fn delete_in(
    tx: &mut Tx<'_>,
    short: &ShortRow,
    actor: Option<Uuid>,
    removal: Removal,
) -> AppResult<Aftermath> {
    let Some(short) = find_in(tx, short.id).await? else { return Ok(Aftermath::default()) };
    let mut aftermath = Aftermath { removed: true, ..Default::default() };
    listing::unlist_in(tx, &short).await?;
    release_cover_in(tx, &short, &mut aftermath).await?;
    assets::detach_holder(&mut **tx, &holder(short.id)).await?;
    sqlx::query("DELETE FROM shorts WHERE id = $1").bind(short.id).execute(&mut **tx).await?;
    if let Some(sound_id) = short.sound_id {
        sounds::drop_if_orphaned_in(tx, sound_id).await?;
    }
    // A withdrawn short was announced as deleted when it was withdrawn.
    if !short.is_withdrawn() {
        core_events::emit(&mut **tx, SHORT_DELETED, actor, &short.event(false, removal.reason())).await?;
    }
    if short.owns_asset && removal != Removal::AssetDeleted {
        if removal == Removal::Moderation {
            aftermath.delete_by_force(short.asset_id);
        } else {
            aftermath.delete_if_unused(short.asset_id, short.owner_id);
        }
    }
    Ok(aftermath)
}

/// Deletes a short in a transaction of its own and cleans up after it.
/// Returns whether this call deleted it (`false`: it was gone already).
pub async fn delete(core: &Core, short: &ShortRow, actor: Option<Uuid>, removal: Removal) -> AppResult<bool> {
    let mut tx = core.db.begin().await?;
    let aftermath = delete_in(&mut tx, short, actor, removal).await?;
    tx.commit().await?;
    let removed = aftermath.removed;
    aftermath.run(core).await;
    Ok(removed)
}

/// Deletes a short on behalf of its owner. A short that has been reported
/// is withdrawn instead ([`withdraw`]): the owner cannot make the evidence
/// of a report disappear.
pub async fn delete_by_owner(core: &Core, short: &ShortRow) -> AppResult<()> {
    let removed = if short.reported_at.is_some() {
        withdraw(core, short).await?
    } else {
        delete(core, short, Some(short.owner_id), Removal::Owner).await?
    };
    // Of two requests deleting the same short, the second finds nothing.
    ensure!(removed, not_found, "SHORT_NOT_FOUND", "short not found");
    Ok(())
}

/// Takes a short out of everyone's sight, its owner's included, while
/// keeping the row and its media: for the world it is deleted
/// (`shorts.short.deleted` is emitted), for moderators it stays reviewable
/// until [`purge_withdrawn`] removes it.
pub async fn withdraw(core: &Core, short: &ShortRow) -> AppResult<bool> {
    let mut tx = core.db.begin().await?;
    let Some(current) = find_in(&mut tx, short.id).await?.filter(|current| !current.is_withdrawn()) else {
        return Ok(false);
    };
    listing::unlist_in(&mut tx, &current).await?;
    sqlx::query("UPDATE shorts SET status = 4, withdrawn_at = now(), pinned_at = NULL WHERE id = $1")
        .bind(current.id)
        .execute(&mut *tx)
        .await?;
    core_events::emit(&mut *tx, SHORT_DELETED, Some(current.owner_id), &current.event(false, Removal::Owner.reason()))
        .await?;
    tx.commit().await?;
    Ok(true)
}

/// Deletes for good the shorts that were withdrawn more than
/// `retention_days` ago. The clock starts at the withdrawal, not at the
/// short's creation. Returns how many were deleted.
pub async fn purge_withdrawn(core: &Core, retention_days: i32) -> AppResult<u64> {
    let rows: Vec<ShortRow> = sqlx::query_as(concat!(
        "SELECT ",
        short_columns!(),
        " FROM shorts s WHERE s.status = 4 AND s.withdrawn_at < now() - make_interval(days => $1) ORDER BY s.id LIMIT 200"
    ))
    .bind(retention_days)
    .fetch_all(&core.db)
    .await?;
    delete_each(core, rows, Removal::Owner).await
}

/// Deletes every short of a channel, one transaction per short. Returns how
/// many were deleted.
pub(crate) async fn delete_of_channel(core: &Core, channel_id: Uuid, removal: Removal) -> AppResult<u64> {
    let rows: Vec<ShortRow> =
        sqlx::query_as(concat!("SELECT ", short_columns!(), " FROM shorts s WHERE s.channel_id = $1 ORDER BY s.id"))
            .bind(channel_id)
            .fetch_all(&core.db)
            .await?;
    delete_each(core, rows, removal).await
}

/// Deletes every short of an account, one transaction per short.
pub(crate) async fn delete_of_owner(core: &Core, owner_id: Uuid, removal: Removal) -> AppResult<u64> {
    let rows: Vec<ShortRow> =
        sqlx::query_as(concat!("SELECT ", short_columns!(), " FROM shorts s WHERE s.owner_id = $1 ORDER BY s.id"))
            .bind(owner_id)
            .fetch_all(&core.db)
            .await?;
    delete_each(core, rows, removal).await
}

async fn delete_each(core: &Core, rows: Vec<ShortRow>, removal: Removal) -> AppResult<u64> {
    let mut count = 0;
    for row in rows {
        count += u64::from(delete(core, &row, None, removal).await?);
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(seconds: f64, width: i32, height: i32) -> VideoInfo {
        VideoInfo { duration_seconds: seconds, width, height, ..Default::default() }
    }

    #[test]
    fn shorts_are_at_most_three_minutes_and_not_wider_than_sixteen_by_nine() {
        let cfg = ShortsConfig::default();
        assert_eq!(format_problem(&cfg, &info(15.0, 1080, 1920)), None);
        assert_eq!(format_problem(&cfg, &info(180.4, 1080, 1920)), None, "containers round durations");
        assert_eq!(format_problem(&cfg, &info(181.0, 1080, 1920)), Some("SHORT_TOO_LONG"));
        assert_eq!(format_problem(&cfg, &info(15.0, 1080, 1080)), None, "square fills the width");
        assert_eq!(format_problem(&cfg, &info(15.0, 1920, 1080)), None, "16:9 is centre-cropped");
        assert_eq!(format_problem(&cfg, &info(15.0, 1280, 720)), None);
        assert_eq!(format_problem(&cfg, &info(15.0, 2560, 1080)), Some("SHORT_NOT_VERTICAL"));
        assert_eq!(format_problem(&cfg, &info(15.0, 0, 0)), None, "unknown sizes are not held against the upload");
        let strict = ShortsConfig { max_aspect_ratio: 1.0, ..Default::default() };
        assert_eq!(format_problem(&strict, &info(15.0, 1920, 1080)), Some("SHORT_NOT_VERTICAL"));
    }

    #[test]
    fn captions_are_cleaned_and_bounded() {
        assert_eq!(validate_caption("  今日のダンス #踊ってみた\r\n").unwrap(), "今日のダンス #踊ってみた");
        assert_eq!(validate_caption(&"あ".repeat(501)).unwrap_err().reason, "CAPTION_TOO_LONG");
        assert_eq!(validate_caption("").unwrap(), "");
    }

    #[test]
    fn visibilities_default_and_reject_unknown_numbers() {
        use tokuly_proto::common::Visibility;
        assert_eq!(validate_visibility(Visibility::VISIBILITY_UNSPECIFIED.into(), access::FRIENDS).unwrap(), 4);
        assert_eq!(validate_visibility(Visibility::VISIBILITY_PUBLIC.into(), access::FRIENDS).unwrap(), 1);
        assert_eq!(
            validate_visibility(buffa::EnumValue::from(9), access::FRIENDS).unwrap_err().reason,
            "VISIBILITY_INVALID"
        );
    }

    #[test]
    fn removal_reasons_are_stable_names() {
        assert_eq!(Removal::Owner.reason(), "owner");
        assert_eq!(Removal::Moderation.reason(), "moderation");
    }
}
