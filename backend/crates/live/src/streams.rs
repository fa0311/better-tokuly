//! Streams: rows, lookups, API representations, and creating, editing and
//! deleting them. Going on air and ending live in [`crate::lifecycle`].
//!
//! ```text
//! UPCOMING ──encoder connects (auto_start) or StartStream──▶ LIVE ──EndStream, encoder gone, raid──▶ ENDED
//!     └───────────────────────EndStream (cancel)──────────────────────────────────────────────────▶ ENDED
//! ```
use std::collections::HashMap;

use tokuly_core::{events as core_events, ids::Ref, outbound, prelude::*};
use tokuly_media::assets::{self, image_from_json};
use tokuly_proto::{
    common::{ChannelSummary, Image, SubjectType},
    live::{ArchiveStatus, ChatAudience, ChatSettings, LinkPolicy, StreamSummary},
    media::AssetPurpose,
};
use tokuly_video::{
    channels::{self, ChannelRow},
    validate as video_validate,
};

use crate::{
    access::{self, ENDED, LIVE, StreamFacts, UPCOMING, Viewer},
    events::{STREAM_DELETED, StreamEvent},
    search,
};

pub const ARCHIVE_NONE: i32 = ArchiveStatus::ARCHIVE_STATUS_NONE as i32;
pub const ARCHIVE_PENDING: i32 = ArchiveStatus::ARCHIVE_STATUS_PENDING as i32;
pub const ARCHIVE_PROCESSING: i32 = ArchiveStatus::ARCHIVE_STATUS_PROCESSING as i32;
pub const ARCHIVE_READY: i32 = ArchiveStatus::ARCHIVE_STATUS_READY as i32;
pub const ARCHIVE_FAILED: i32 = ArchiveStatus::ARCHIVE_STATUS_FAILED as i32;

/// How far ahead a stream may be scheduled.
pub const MAX_SCHEDULE_DAYS: i64 = 366;
pub const MAX_SLOW_MODE_SECONDS: i32 = 300;
/// Longest account age / subscription age a chat may ask for: 30 days.
pub const MAX_CHAT_AGE_MINUTES: i32 = 30 * 24 * 60;
/// Account age asked for by CHAT_AUDIENCE_MEMBERS when none is given: a day.
pub const DEFAULT_MEMBER_AGE_MINUTES: i32 = 24 * 60;

pub type Tx<'a> = sqlx::Transaction<'a, sqlx::Postgres>;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StreamRow {
    pub id: Uuid,
    /// Public ID and name on the media server.
    pub short_id: String,
    pub channel_id: Uuid,
    /// The channel's owner.
    pub owner_id: Uuid,
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    pub category: i32,
    pub game: String,
    pub thumbnail: Option<serde_json::Value>,
    pub thumbnail_asset_id: Option<Uuid>,
    pub visibility: i32,
    pub status: i32,
    pub scheduled_at: Option<Time>,
    pub low_latency: bool,
    pub dvr_enabled: bool,
    pub auto_archive: bool,
    pub auto_start: bool,
    pub gifts_enabled: bool,
    /// `tokuly.live.v1.ChatSettings` as proto JSON.
    pub chat: serde_json::Value,
    pub stream_key_sealed: Vec<u8>,
    pub stream_key_hash: String,
    pub key_rotated_at: Time,
    pub encoder_connected: bool,
    pub encoder_protocol: String,
    pub encoder_lost_at: Option<Time>,
    pub started_at: Option<Time>,
    pub ended_at: Option<Time>,
    pub end_reason: i32,
    pub viewer_count: i64,
    pub peak_viewer_count: i64,
    pub like_count: i64,
    pub chat_message_count: i64,
    pub gift_coins: i64,
    pub gift_net_coins: i64,
    pub gift_count: i64,
    pub new_subscribers: i64,
    pub pinned_message_id: Option<Uuid>,
    pub archive_status: i32,
    pub archive_asset_id: Option<Uuid>,
    pub archive_video_id: Option<Uuid>,
    pub archive_error: String,
    pub raid_target_id: Option<Uuid>,
    pub announced_at: Option<Time>,
    pub suspended: bool,
    /// Deleted: kept (with its chat) until the retention period is over.
    pub deleted_at: Option<Time>,
    pub created_at: Time,
    pub updated_at: Time,
}

macro_rules! stream_columns {
    () => {
        "s.id, s.short_id, s.channel_id, s.owner_id, s.title, s.description, s.tags, s.category, s.game, s.thumbnail, \
         s.thumbnail_asset_id, s.visibility, s.status, s.scheduled_at, s.low_latency, s.dvr_enabled, s.auto_archive, \
         s.auto_start, s.gifts_enabled, s.chat, s.stream_key_sealed, s.stream_key_hash, s.key_rotated_at, \
         s.encoder_connected, s.encoder_protocol, s.encoder_lost_at, s.started_at, s.ended_at, s.end_reason, \
         s.viewer_count, s.peak_viewer_count, s.like_count, s.chat_message_count, s.gift_coins, s.gift_net_coins, \
         s.gift_count, s.new_subscribers, s.pinned_message_id, s.archive_status, s.archive_asset_id, \
         s.archive_video_id, s.archive_error, s.raid_target_id, s.announced_at, s.suspended, s.deleted_at, s.created_at, \
         s.updated_at"
    };
}
pub(crate) use stream_columns;

impl StreamRow {
    pub fn facts(&self) -> StreamFacts {
        StreamFacts { owner_id: self.owner_id, visibility: self.visibility, suspended: self.suspended }
    }

    pub fn is_live(&self) -> bool {
        self.status == LIVE
    }

    pub fn is_upcoming(&self) -> bool {
        self.status == UPCOMING
    }

    pub fn is_ended(&self) -> bool {
        self.status == ENDED
    }

    pub fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }

    /// Listed in the directories and in search: public, not over, not suspended.
    pub fn is_discoverable(&self) -> bool {
        self.visibility == access::PUBLIC && !self.is_ended() && !self.suspended && !self.is_deleted()
    }

    /// Path of the stream page in the web app.
    pub fn path(&self) -> String {
        format!("/live/{}", self.short_id)
    }

    /// The stream's path on the media server.
    pub fn media_path(&self) -> String {
        media_path(&self.short_id)
    }

    /// Topic carrying the stream's realtime events.
    pub fn topic(&self) -> String {
        topic(self.id)
    }

    pub fn thumbnail(&self) -> Option<Image> {
        image_from_json(self.thumbnail.as_ref())
    }

    pub fn thumbnail_url(&self) -> Option<String> {
        self.thumbnail().map(|image| image.url).filter(|url| !url.is_empty())
    }

    /// The chat rules (defaults if the stored value cannot be read).
    pub fn chat_settings(&self) -> ChatSettings {
        serde_json::from_value(self.chat.clone()).unwrap_or_else(|_| default_chat_settings())
    }

    pub(crate) fn event(&self) -> StreamEvent {
        StreamEvent {
            stream_id: self.id,
            short_id: self.short_id.clone(),
            channel_id: self.channel_id,
            owner_id: self.owner_id,
            visibility: self.visibility,
            end_reason: self.end_reason,
            went_live: self.started_at.is_some(),
        }
    }

    /// The card, given the channel's summary.
    pub fn summary_with(&self, channel: Option<ChannelSummary>) -> StreamSummary {
        StreamSummary {
            id: self.id.to_string(),
            short_id: self.short_id.clone(),
            title: self.title.clone(),
            thumbnail: self.thumbnail().into(),
            channel: channel.into(),
            status: self.status.into(),
            visibility: self.visibility.into(),
            category: self.category.into(),
            game: self.game.clone(),
            viewer_count: if self.is_live() { self.viewer_count } else { 0 },
            peak_viewer_count: self.peak_viewer_count,
            like_count: self.like_count,
            scheduled_at: opt_ts(self.scheduled_at),
            started_at: opt_ts(self.started_at),
            ended_at: opt_ts(self.ended_at),
            low_latency: self.low_latency,
            created_at: ts(self.created_at),
            ..Default::default()
        }
    }
}

/// A stream's path on the media server.
pub fn media_path(short_id: &str) -> String {
    format!("live/{short_id}")
}

/// The short ID in a media server path, if it is a stream path.
pub fn short_id_of_path(path: &str) -> Option<&str> {
    let short_id = path.strip_prefix("live/")?;
    (short_id.len() == 11 && short_id.bytes().all(|b| b.is_ascii_alphanumeric())).then_some(short_id)
}

/// Topic carrying a stream's realtime events.
pub fn topic(stream_id: Uuid) -> String {
    format!("live:{stream_id}")
}

/// Holder under which a stream keeps its thumbnail upload alive.
pub fn holder(stream_id: Uuid) -> String {
    format!("live:stream:{stream_id}")
}

pub(crate) fn not_found() -> AppError {
    AppError::not_found("STREAM_NOT_FOUND", "stream not found")
}

// ───────────────────────────── lookups ─────────────────────────────

/// A stream by UUID or short ID, whatever its state. Callers decide who may
/// see it ([`access::can_watch`]). Deleted streams are not found.
pub async fn find(core: &Core, reference: &Ref) -> AppResult<Option<StreamRow>> {
    Ok(find_with_deleted(core, reference).await?.filter(|stream| !stream.is_deleted()))
}

/// Like [`find`], but also returns a stream that was deleted and is being
/// kept for its retention period (for reports and the purge).
pub async fn find_with_deleted(core: &Core, reference: &Ref) -> AppResult<Option<StreamRow>> {
    Ok(match reference {
        Ref::Id(id) => {
            sqlx::query_as(concat!("SELECT ", stream_columns!(), " FROM live_streams s WHERE s.id = $1"))
                .bind(id)
                .fetch_optional(&core.db)
                .await?
        }
        Ref::Short(short_id) => {
            sqlx::query_as(concat!("SELECT ", stream_columns!(), " FROM live_streams s WHERE s.short_id = $1"))
                .bind(short_id)
                .fetch_optional(&core.db)
                .await?
        }
    })
}

pub async fn find_by_id(core: &Core, stream_id: Uuid) -> AppResult<Option<StreamRow>> {
    find(core, &Ref::Id(stream_id)).await
}

/// A stream by what clients send as `stream_id`. A malformed reference
/// names no stream.
pub async fn find_ref(core: &Core, reference: &str) -> AppResult<Option<StreamRow>> {
    match Ref::parse(reference.trim()) {
        Ok(reference) => find(core, &reference).await,
        Err(_) => Ok(None),
    }
}

/// The stream a client named, if `viewer` may open it (`STREAM_NOT_FOUND`
/// otherwise, whatever the reason).
pub async fn find_watchable(core: &Core, viewer: &Viewer, reference: &str) -> AppResult<StreamRow> {
    find_ref(core, reference).await?.filter(|stream| access::can_watch(viewer, &stream.facts())).ok_or_else(not_found)
}

/// The stream a client named, which must belong to `owner_id`.
pub async fn find_owned(core: &Core, owner_id: Uuid, reference: &str) -> AppResult<StreamRow> {
    find_ref(core, reference).await?.filter(|stream| stream.owner_id == owner_id).ok_or_else(not_found)
}

/// The row inside a transaction, locked for the rest of it (`None` for a
/// stream that does not exist or was deleted).
pub(crate) async fn lock(tx: &mut Tx<'_>, stream_id: Uuid) -> AppResult<Option<StreamRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        stream_columns!(),
        " FROM live_streams s WHERE s.id = $1 AND s.deleted_at IS NULL FOR UPDATE"
    ))
    .bind(stream_id)
    .fetch_optional(&mut **tx)
    .await?)
}

/// The stream of a channel that is on air, or else the next one waiting
/// with its encoder connected.
pub async fn current_of_channel(core: &Core, channel_id: Uuid) -> AppResult<Option<StreamRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        stream_columns!(),
        " FROM live_streams s WHERE s.channel_id = $1 AND (s.status = 2 OR (s.status = 1 AND s.encoder_connected))
          ORDER BY s.status DESC, s.id DESC LIMIT 1"
    ))
    .bind(channel_id)
    .fetch_optional(&core.db)
    .await?)
}

/// Cards for `rows`, in order.
pub async fn summaries(core: &Core, rows: &[StreamRow]) -> AppResult<Vec<StreamSummary>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut channel_ids: Vec<Uuid> = rows.iter().map(|row| row.channel_id).collect();
    channel_ids.sort_unstable();
    channel_ids.dedup();
    let channels: HashMap<Uuid, ChannelSummary> = channels::summaries(core, &channel_ids).await?;
    Ok(rows.iter().map(|row| row.summary_with(channels.get(&row.channel_id).cloned())).collect())
}

pub async fn summary(core: &Core, row: &StreamRow) -> AppResult<StreamSummary> {
    Ok(summaries(core, std::slice::from_ref(row)).await?.pop().unwrap_or_default())
}

// ───────────────────────────── chat settings ─────────────────────────────

/// Chat open to every member, links allowed, stamps on.
pub fn default_chat_settings() -> ChatSettings {
    ChatSettings {
        enabled: true,
        slow_mode_seconds: 0,
        audience: ChatAudience::CHAT_AUDIENCE_EVERYONE.into(),
        min_account_age_minutes: 0,
        link_policy: LinkPolicy::LINK_POLICY_ALLOW.into(),
        stamps_enabled: true,
        ..Default::default()
    }
}

/// Checks chat rules sent by a client and fills in what was left
/// unspecified (`CHAT_SETTINGS_INVALID`).
pub fn validate_chat_settings(input: &ChatSettings) -> AppResult<ChatSettings> {
    let bad = |message: &'static str| AppError::invalid("CHAT_SETTINGS_INVALID", message);
    let audience = match input.audience.as_known() {
        Some(ChatAudience::CHAT_AUDIENCE_UNSPECIFIED) => ChatAudience::CHAT_AUDIENCE_EVERYONE,
        Some(known) => known,
        None => return Err(bad("unknown chat audience")),
    };
    let link_policy = match input.link_policy.as_known() {
        Some(LinkPolicy::LINK_POLICY_UNSPECIFIED) => LinkPolicy::LINK_POLICY_ALLOW,
        Some(known) => known,
        None => return Err(bad("unknown link policy")),
    };
    if !(0..=MAX_SLOW_MODE_SECONDS).contains(&input.slow_mode_seconds) {
        return Err(bad("slow mode is 0 to 300 seconds"));
    }
    if !(0..=MAX_CHAT_AGE_MINUTES).contains(&input.min_account_age_minutes) {
        return Err(bad("the minimum account age is 0 to 43200 minutes"));
    }
    Ok(ChatSettings {
        enabled: input.enabled,
        slow_mode_seconds: input.slow_mode_seconds,
        audience: audience.into(),
        min_account_age_minutes: if audience == ChatAudience::CHAT_AUDIENCE_MEMBERS {
            if input.min_account_age_minutes == 0 { DEFAULT_MEMBER_AGE_MINUTES } else { input.min_account_age_minutes }
        } else {
            0
        },
        link_policy: link_policy.into(),
        stamps_enabled: input.stamps_enabled,
        ..Default::default()
    })
}

pub(crate) fn chat_json(settings: &ChatSettings) -> AppResult<serde_json::Value> {
    serde_json::to_value(settings).map_err(|e| AppError::internal(anyhow::anyhow!("chat settings encode: {e}")))
}

// ───────────────────────────── stream keys ─────────────────────────────

const KEY_LABEL: &str = "live/stream-key";

/// A new stream key: the secret, its sealed form and its hash.
pub(crate) fn new_stream_key(core: &Core) -> (String, Vec<u8>, String) {
    let key = tokuly_core::keys::random_token(24);
    let sealed = core.keys.seal(KEY_LABEL, key.as_bytes());
    let hash = core.keys.token_hash(&key);
    (key, sealed, hash)
}

/// The stream key in the clear, for its owner.
pub(crate) fn open_stream_key(core: &Core, row: &StreamRow) -> AppResult<String> {
    core.keys
        .open(KEY_LABEL, &row.stream_key_sealed)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or_else(|| AppError::internal(anyhow::anyhow!("stream key of {} cannot be opened", row.id)))
}

/// Whether `candidate` is the stream's key.
pub fn key_matches(core: &Core, row: &StreamRow, candidate: &str) -> bool {
    !candidate.is_empty()
        && tokuly_core::keys::secure_eq(core.keys.token_hash(candidate).as_bytes(), row.stream_key_hash.as_bytes())
}

// ───────────────────────────── create / update / delete ─────────────────────────────

/// What a caller wants a stream to look like. `None` = default (create) or
/// leave as it is (update).
#[derive(Debug, Default)]
pub struct StreamEdit {
    pub title: Option<String>,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
    pub category: Option<i32>,
    pub game: Option<String>,
    /// `Some(None)` removes the thumbnail.
    pub thumbnail_asset_id: Option<Option<Uuid>>,
    pub visibility: Option<i32>,
    /// `Some(None)` removes the planned start.
    pub scheduled_at: Option<Option<Time>>,
    pub low_latency: Option<bool>,
    pub dvr_enabled: Option<bool>,
    pub auto_archive: Option<bool>,
    pub auto_start: Option<bool>,
    pub gifts_enabled: Option<bool>,
    pub chat: Option<ChatSettings>,
}

/// A planned start must lie in the future, at most a year ahead.
pub fn validate_schedule(at: Time) -> AppResult<Time> {
    let now = now();
    ensure!(
        at > now - chrono::Duration::minutes(1) && at <= now + chrono::Duration::days(MAX_SCHEDULE_DAYS),
        invalid,
        "SCHEDULE_INVALID",
        "the planned start must be in the future, at most a year ahead"
    );
    Ok(at)
}

/// A picture resolved from an upload of the owner.
struct Thumbnail {
    asset_id: Uuid,
    image: serde_json::Value,
}

async fn resolve_thumbnail(core: &Core, owner_id: Uuid, asset_id: Option<Uuid>) -> AppResult<Option<Thumbnail>> {
    match asset_id {
        None => Ok(None),
        Some(asset_id) => {
            let image =
                assets::require_image_json(core, asset_id, owner_id, &[AssetPurpose::ASSET_PURPOSE_THUMBNAIL]).await?;
            Ok(Some(Thumbnail { asset_id, image }))
        }
    }
}

/// Creates an UPCOMING stream on `channel` (which must be the caller's).
pub async fn create(core: &Core, channel: &ChannelRow, max_open: i64, edit: StreamEdit) -> AppResult<StreamRow> {
    let title = video_validate::title(edit.title.as_deref().unwrap_or_default())?;
    let description = video_validate::description(edit.description.as_deref().unwrap_or_default())?;
    let tags = video_validate::tags(edit.tags.as_deref().unwrap_or_default())?;
    let game = video_validate::game(edit.game.as_deref().unwrap_or_default())?;
    let scheduled_at = edit.scheduled_at.flatten().map(validate_schedule).transpose()?;
    let chat = match &edit.chat {
        Some(chat) => validate_chat_settings(chat)?,
        None => default_chat_settings(),
    };
    let thumbnail = resolve_thumbnail(core, channel.owner_id, edit.thumbnail_asset_id.flatten()).await?;
    let (_, sealed, hash) = new_stream_key(core);

    let stream_id = new_id();
    let mut tx = core.db.begin().await?;
    // Serializes creations on one channel, so the limit holds.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("live:streams:{}", channel.id.simple()))
        .execute(&mut *tx)
        .await?;
    let open: i64 = sqlx::query_scalar("SELECT count(*) FROM live_streams WHERE channel_id = $1 AND status < 3")
        .bind(channel.id)
        .fetch_one(&mut *tx)
        .await?;
    ensure!(
        open < max_open,
        precondition,
        "STREAM_LIMIT_REACHED",
        "a channel can have at most {max_open} streams that are not over; end or delete one first"
    );
    sqlx::query(
        "INSERT INTO live_streams (id, short_id, channel_id, owner_id, title, description, tags, category, game, thumbnail,
                                   thumbnail_asset_id, visibility, scheduled_at, low_latency, dvr_enabled, auto_archive,
                                   auto_start, gifts_enabled, chat, stream_key_sealed, stream_key_hash, suspended)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22)",
    )
    .bind(stream_id)
    .bind(tokuly_core::ids::new_short_id())
    .bind(channel.id)
    .bind(channel.owner_id)
    .bind(&title)
    .bind(&description)
    .bind(&tags)
    .bind(edit.category.unwrap_or(0))
    .bind(&game)
    .bind(thumbnail.as_ref().map(|t| &t.image))
    .bind(thumbnail.as_ref().map(|t| t.asset_id))
    .bind(edit.visibility.unwrap_or(access::PUBLIC))
    .bind(scheduled_at)
    .bind(edit.low_latency.unwrap_or(true))
    .bind(edit.dvr_enabled.unwrap_or(true))
    .bind(edit.auto_archive.unwrap_or(true))
    .bind(edit.auto_start.unwrap_or(true))
    .bind(edit.gifts_enabled.unwrap_or(true))
    .bind(chat_json(&chat)?)
    .bind(&sealed)
    .bind(&hash)
    .bind(channel.suspended)
    .execute(&mut *tx)
    .await?;
    if let Some(thumbnail) = &thumbnail {
        assets::attach(&mut *tx, thumbnail.asset_id, &holder(stream_id)).await?;
    }
    let row = lock(&mut tx, stream_id).await?.ok_or_else(not_found)?;
    search::sync_stream(&mut tx, &row, Some(channel)).await?;
    tx.commit().await?;
    Ok(row)
}

/// Applies `edit` to a stream of its owner. The schedule and `auto_start`
/// only change while the stream is upcoming.
pub async fn update(core: &Core, stream: &StreamRow, edit: StreamEdit) -> AppResult<StreamRow> {
    let title = edit.title.as_deref().map(video_validate::title).transpose()?;
    let description = edit.description.as_deref().map(video_validate::description).transpose()?;
    let tags = edit.tags.as_deref().map(video_validate::tags).transpose()?;
    let game = edit.game.as_deref().map(video_validate::game).transpose()?;
    let scheduled_at = match edit.scheduled_at {
        Some(Some(at)) => Some(Some(validate_schedule(at)?)),
        other => other,
    };
    let thumbnail = match edit.thumbnail_asset_id {
        Some(asset_id) => Some(resolve_thumbnail(core, stream.owner_id, asset_id).await?),
        None => None,
    };
    // Read before the transaction: inside it, nothing asks the pool for a
    // second connection.
    let channel = channels::find_by_id(core, stream.channel_id).await?;

    let mut tx = core.db.begin().await?;
    let current = lock(&mut tx, stream.id).await?.ok_or_else(not_found)?;
    if scheduled_at.is_some() || edit.auto_start.is_some() {
        ensure!(
            current.is_upcoming(),
            precondition,
            "STREAM_ALREADY_STARTED",
            "the schedule and auto-start can only change before the stream is on air"
        );
    }
    sqlx::query(
        "UPDATE live_streams SET
             title = COALESCE($2, title), description = COALESCE($3, description), tags = COALESCE($4, tags),
             category = COALESCE($5, category), game = COALESCE($6, game), visibility = COALESCE($7, visibility),
             low_latency = COALESCE($8, low_latency), dvr_enabled = COALESCE($9, dvr_enabled),
             auto_archive = COALESCE($10, auto_archive), auto_start = COALESCE($11, auto_start),
             gifts_enabled = COALESCE($12, gifts_enabled)
         WHERE id = $1",
    )
    .bind(stream.id)
    .bind(&title)
    .bind(&description)
    .bind(&tags)
    .bind(edit.category)
    .bind(&game)
    .bind(edit.visibility)
    .bind(edit.low_latency)
    .bind(edit.dvr_enabled)
    .bind(edit.auto_archive)
    .bind(edit.auto_start)
    .bind(edit.gifts_enabled)
    .execute(&mut *tx)
    .await?;
    if let Some(scheduled_at) = scheduled_at {
        sqlx::query("UPDATE live_streams SET scheduled_at = $2 WHERE id = $1")
            .bind(stream.id)
            .bind(scheduled_at)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(thumbnail) = thumbnail {
        let holder = holder(stream.id);
        if let Some(previous) = current.thumbnail_asset_id {
            assets::detach(&mut *tx, previous, &holder).await?;
        }
        if let Some(thumbnail) = &thumbnail {
            assets::attach(&mut *tx, thumbnail.asset_id, &holder).await?;
        }
        sqlx::query("UPDATE live_streams SET thumbnail = $2, thumbnail_asset_id = $3 WHERE id = $1")
            .bind(stream.id)
            .bind(thumbnail.as_ref().map(|t| &t.image))
            .bind(thumbnail.as_ref().map(|t| t.asset_id))
            .execute(&mut *tx)
            .await?;
    }
    let updated = lock(&mut tx, stream.id).await?.ok_or_else(not_found)?;
    search::sync_stream(&mut tx, &updated, channel.as_ref()).await?;
    tx.commit().await?;
    Ok(updated)
}

/// The upload of a deleted clip, to be removed after the deletion committed.
#[derive(Debug, Clone, Copy)]
pub struct AssetRemoval {
    pub asset_id: Uuid,
    /// The upload's owner: it is removed unless something else still holds
    /// it. `None` removes it regardless (takedowns).
    pub owner_id: Option<Uuid>,
}

/// What is left to do once a deletion has committed.
#[derive(Debug, Default)]
pub struct Aftermath {
    /// Short IDs of streams whose recordings can go.
    pub recordings: Vec<String>,
    /// Uploads of deleted clips.
    pub assets: Vec<AssetRemoval>,
    /// Streams that are gone: their audience is forgotten and an encoder
    /// still publishing is disconnected.
    pub streams: Vec<StreamRow>,
}

impl Aftermath {
    pub fn merge(&mut self, other: Aftermath) {
        self.recordings.extend(other.recordings);
        self.assets.extend(other.assets);
        self.streams.extend(other.streams);
    }
}

/// Deletes a stream inside `tx`: it is gone for everyone from now on. Its
/// reminders, its thumbnail hold and the clips that were cut from the stream
/// itself go at once (clips of its archive video stay with the video). The
/// row and its chat stay until the retention period is over
/// ([`purge_deleted`]), so that what was said in the stream can still be
/// reported and judged. The caller ends a stream that is on air first.
/// `actor` is who caused it (`None` for the system). Run the returned
/// [`Aftermath`] ([`crate::Live::clean_up`]) once the transaction has
/// committed.
pub async fn delete_in(tx: &mut Tx<'_>, stream: &StreamRow, actor: Option<Uuid>) -> AppResult<Aftermath> {
    let mut aftermath = Aftermath { recordings: vec![stream.short_id.clone()], ..Default::default() };
    let orphans: Vec<crate::clips::ClipRow> = sqlx::query_as(concat!(
        "SELECT ",
        crate::clips::clip_columns!(),
        " FROM live_clips c WHERE c.stream_id = $1 AND c.video_id IS NULL FOR UPDATE"
    ))
    .bind(stream.id)
    .fetch_all(&mut **tx)
    .await?;
    for clip in &orphans {
        aftermath.assets.extend(crate::clips::delete_in(tx, clip, actor, false).await?);
    }
    assets::detach_holder(&mut **tx, &holder(stream.id)).await?;
    sqlx::query("DELETE FROM live_reminders WHERE stream_id = $1").bind(stream.id).execute(&mut **tx).await?;
    sqlx::query(
        "UPDATE live_streams SET deleted_at = now(), thumbnail = NULL, thumbnail_asset_id = NULL, pinned_message_id = NULL
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(stream.id)
    .execute(&mut **tx)
    .await?;
    outbound::unindex(&mut **tx, "live", stream.id).await?;
    core_events::emit(&mut **tx, STREAM_DELETED, actor, &stream.event()).await?;
    aftermath.streams.push(stream.clone());
    Ok(aftermath)
}

/// Erases a stream for good, with its chat, polls and samples (an account
/// is being erased, or the retention period of a deleted stream is over).
pub async fn erase_in(tx: &mut Tx<'_>, stream_id: Uuid) -> AppResult<()> {
    sqlx::query("DELETE FROM live_streams WHERE id = $1").bind(stream_id).execute(&mut **tx).await?;
    Ok(())
}

/// Erases the streams that were deleted more than `retention_days` ago.
/// The clock starts at the deletion, not at the stream's creation: a
/// report filed about an old stream just before it was deleted can still
/// be acted on. Returns how many were erased.
pub async fn purge_deleted(core: &Core, retention_days: i32) -> AppResult<u64> {
    Ok(sqlx::query(
        "DELETE FROM live_streams WHERE deleted_at IS NOT NULL AND deleted_at < now() - make_interval(days => $1)",
    )
    .bind(retention_days)
    .execute(&core.db)
    .await?
    .rows_affected())
}

/// Subject of notifications and activity entries about a stream.
pub(crate) const SUBJECT: SubjectType = SubjectType::SUBJECT_TYPE_LIVE_STREAM;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_paths_round_trip_and_reject_everything_else() {
        assert_eq!(media_path("abcDEF12345"), "live/abcDEF12345");
        assert_eq!(short_id_of_path("live/abcDEF12345"), Some("abcDEF12345"));
        for path in ["live/short", "live/abcDEF12345/x", "other/abcDEF12345", "live/abc-EF12345", "abcDEF12345", ""] {
            assert_eq!(short_id_of_path(path), None, "{path}");
        }
    }

    #[test]
    fn chat_settings_are_normalized() {
        let defaults = default_chat_settings();
        assert!(defaults.enabled && defaults.stamps_enabled);
        // Everything unspecified: open chat, links allowed.
        let settings = validate_chat_settings(&ChatSettings { enabled: true, ..Default::default() }).unwrap();
        assert_eq!(settings.audience, ChatAudience::CHAT_AUDIENCE_EVERYONE);
        assert_eq!(settings.link_policy, LinkPolicy::LINK_POLICY_ALLOW);
        // MEMBERS without an age asks for a day; the age means nothing for other audiences.
        let members = validate_chat_settings(&ChatSettings {
            audience: ChatAudience::CHAT_AUDIENCE_MEMBERS.into(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(members.min_account_age_minutes, DEFAULT_MEMBER_AGE_MINUTES);
        let subscribers = validate_chat_settings(&ChatSettings {
            audience: ChatAudience::CHAT_AUDIENCE_SUBSCRIBERS.into(),
            min_account_age_minutes: 5,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(subscribers.min_account_age_minutes, 0);
        for bad in [
            ChatSettings { slow_mode_seconds: 301, ..Default::default() },
            ChatSettings { slow_mode_seconds: -1, ..Default::default() },
            ChatSettings { min_account_age_minutes: MAX_CHAT_AGE_MINUTES + 1, ..Default::default() },
            ChatSettings { audience: buffa::EnumValue::from(99), ..Default::default() },
            ChatSettings { link_policy: buffa::EnumValue::from(99), ..Default::default() },
        ] {
            assert_eq!(validate_chat_settings(&bad).unwrap_err().reason, "CHAT_SETTINGS_INVALID");
        }
    }

    #[test]
    fn schedules_must_be_ahead_but_not_too_far() {
        assert!(validate_schedule(now() + chrono::Duration::hours(1)).is_ok());
        assert_eq!(validate_schedule(now() - chrono::Duration::hours(1)).unwrap_err().reason, "SCHEDULE_INVALID");
        assert_eq!(validate_schedule(now() + chrono::Duration::days(400)).unwrap_err().reason, "SCHEDULE_INVALID");
    }
}
