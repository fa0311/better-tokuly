//! Channels: creator identities, their pages and subscriptions.
//!
//! An account owns a handful of channels and publishes as one of them; one is
//! the default ("posting identity"). A channel never reveals which account
//! owns it: other domains show a `ChannelSummary`, not the owner.
//!
//! # For other domains (live, shorts, engagement)
//!
//! - [`summaries`] turns channel IDs into `ChannelSummary` for display;
//! - [`require_owned`] checks that a user may act as a channel;
//! - [`default_for`] is the user's posting identity;
//! - [`subscriber_ids`] / [`is_subscribed`] answer who follows a channel
//!   (e.g. whom to tell that a stream went live).
use std::collections::HashMap;

use tokuly_core::{events as core_events, outbound, prelude::*};
use tokuly_media::assets::{self, image_from_json};
use tokuly_proto::{
    common::{ChannelSummary, Image},
    media::AssetPurpose,
    video::{Channel, ChannelLink, ChannelViewerState, NotificationLevel},
};

use crate::{
    access::Viewer,
    events::{CHANNEL_CREATED, CHANNEL_DELETED, CHANNEL_SUBSCRIBED, CHANNEL_UNSUBSCRIBED, CHANNEL_UPDATED, ChannelEvent, SubscriptionEvent},
    search, validate,
    videos::{self, Aftermath},
};

/// How much a subscriber wants to hear from a channel. The numbers are the
/// stored values (`tokuly.video.v1.NotificationLevel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NotifyLevel {
    /// Subscribed, never notified.
    None = 1,
    /// Inbox always, push while they keep watching the channel.
    Personalized = 2,
    /// Everything is pushed.
    All = 3,
}

impl NotifyLevel {
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            1 => Some(Self::None),
            2 => Some(Self::Personalized),
            3 => Some(Self::All),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ChannelRow {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub handle: String,
    pub name: String,
    pub description: String,
    /// `tokuly.common.v1.Image` as proto JSON.
    pub icon: Option<serde_json::Value>,
    pub icon_asset_id: Option<Uuid>,
    pub banner: Option<serde_json::Value>,
    pub banner_asset_id: Option<Uuid>,
    /// `[tokuly.video.v1.ChannelLink]` as proto JSON.
    pub links: serde_json::Value,
    /// The channel, or the account that owns it, carries the verified badge.
    pub verified: bool,
    pub is_default: bool,
    pub trailer_video_id: Option<Uuid>,
    pub subscriber_count: i64,
    pub view_count: i64,
    /// The owner's account is suspended: hidden from everyone else.
    pub suspended: bool,
    pub created_at: Time,
    pub updated_at: Time,
}

macro_rules! channel_columns {
    () => {
        "c.id, c.owner_id, c.handle::text AS handle, c.name, c.description, c.icon, c.icon_asset_id, c.banner, \
         c.banner_asset_id, c.links, \
         (c.verified OR COALESCE((SELECT u.verified FROM users u WHERE u.id = c.owner_id), false)) AS verified, \
         c.is_default, c.trailer_video_id, c.subscriber_count, c.view_count, c.suspended, c.created_at, c.updated_at"
    };
}
pub(crate) use channel_columns;

impl ChannelRow {
    pub fn icon(&self) -> Option<Image> {
        image_from_json(self.icon.as_ref())
    }

    pub fn icon_url(&self) -> Option<String> {
        self.icon().map(|image| image.url).filter(|url| !url.is_empty())
    }

    pub fn summary(&self) -> ChannelSummary {
        ChannelSummary {
            id: self.id.to_string(),
            handle: self.handle.clone(),
            name: self.name.clone(),
            icon: self.icon().into(),
            verified: self.verified,
            subscriber_count: self.subscriber_count,
            ..Default::default()
        }
    }

    pub fn links(&self) -> Vec<ChannelLink> {
        serde_json::from_value(self.links.clone()).unwrap_or_default()
    }

    /// Path of the channel page in the web app.
    pub fn path(&self) -> String {
        format!("/channel/{}", self.handle)
    }

    fn event(&self) -> ChannelEvent {
        ChannelEvent { channel_id: self.id, owner_id: self.owner_id, handle: self.handle.clone() }
    }
}

fn not_found() -> AppError {
    AppError::not_found("CHANNEL_NOT_FOUND", "channel not found")
}

/// Holder under which a channel keeps its icon / banner upload alive.
fn art_holder(slot: &str, channel_id: Uuid) -> String {
    format!("video:channel-{slot}:{channel_id}")
}

pub async fn find_by_id(core: &Core, channel_id: Uuid) -> AppResult<Option<ChannelRow>> {
    Ok(sqlx::query_as(concat!("SELECT ", channel_columns!(), " FROM channels c WHERE c.id = $1"))
        .bind(channel_id)
        .fetch_optional(&core.db)
        .await?)
}

pub async fn find_by_handle(core: &Core, handle: &str) -> AppResult<Option<ChannelRow>> {
    Ok(sqlx::query_as(concat!("SELECT ", channel_columns!(), " FROM channels c WHERE c.handle = $1::citext"))
        .bind(handle)
        .fetch_optional(&core.db)
        .await?)
}

/// A channel by what clients send as `channel_id`: its UUID or its handle,
/// with or without a leading `@`. Anything else names no channel.
pub async fn find(core: &Core, reference: &str) -> AppResult<Option<ChannelRow>> {
    let reference = reference.trim();
    if let Ok(id) = Uuid::parse_str(reference) {
        return find_by_id(core, id).await;
    }
    let handle = reference.trim_start_matches('@');
    let plausible = (1..=64).contains(&handle.len()) && handle.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if !plausible {
        return Ok(None);
    }
    find_by_handle(core, handle).await
}

/// The channel named by a client, as far as `viewer` may see it: channels of
/// suspended accounts and of users in a block relation with the viewer read
/// as not found.
pub async fn find_visible(core: &Core, viewer: &Viewer, reference: &str) -> AppResult<ChannelRow> {
    find(core, reference).await?.filter(|channel| viewer.may_see_owner(channel.owner_id, channel.suspended)).ok_or_else(not_found)
}

/// Summaries of the given channels, keyed by ID. Unknown IDs are absent.
pub async fn summaries(core: &Core, ids: &[Uuid]) -> AppResult<HashMap<Uuid, ChannelSummary>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<ChannelRow> = sqlx::query_as(concat!("SELECT ", channel_columns!(), " FROM channels c WHERE c.id = ANY($1)"))
        .bind(ids)
        .fetch_all(&core.db)
        .await?;
    Ok(rows.into_iter().map(|row| (row.id, row.summary())).collect())
}

/// The channel `channel_id`, which must belong to `user_id`
/// (`CHANNEL_NOT_FOUND` otherwise: other people's channels are not confirmed
/// to exist).
pub async fn require_owned(core: &Core, user_id: Uuid, channel_id: Uuid) -> AppResult<ChannelRow> {
    find_by_id(core, channel_id).await?.filter(|channel| channel.owner_id == user_id).ok_or_else(not_found)
}

/// Like [`require_owned`], for a channel named by UUID or handle.
pub async fn require_owned_ref(core: &Core, user_id: Uuid, reference: &str) -> AppResult<ChannelRow> {
    find(core, reference).await?.filter(|channel| channel.owner_id == user_id).ok_or_else(not_found)
}

/// The user's default channel (their posting identity), if they have one.
pub async fn default_for(core: &Core, user_id: Uuid) -> AppResult<Option<ChannelRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        channel_columns!(),
        " FROM channels c WHERE c.owner_id = $1 ORDER BY c.is_default DESC, c.id LIMIT 1"
    ))
    .bind(user_id)
    .fetch_optional(&core.db)
    .await?)
}

/// Every channel of a user, the default one first, then oldest first.
pub async fn owned_by(core: &Core, user_id: Uuid) -> AppResult<Vec<ChannelRow>> {
    Ok(sqlx::query_as(concat!("SELECT ", channel_columns!(), " FROM channels c WHERE c.owner_id = $1 ORDER BY c.is_default DESC, c.id"))
        .bind(user_id)
        .fetch_all(&core.db)
        .await?)
}

/// IDs of the users subscribed to a channel with at least `min_level`.
pub async fn subscriber_ids(core: &Core, channel_id: Uuid, min_level: NotifyLevel) -> AppResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar("SELECT user_id FROM channel_subscriptions WHERE channel_id = $1 AND notify_level >= $2 ORDER BY user_id")
        .bind(channel_id)
        .bind(min_level as i32)
        .fetch_all(&core.db)
        .await?)
}

pub async fn is_subscribed(core: &Core, user_id: Uuid, channel_id: Uuid) -> AppResult<bool> {
    Ok(sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM channel_subscriptions WHERE channel_id = $1 AND user_id = $2)")
        .bind(channel_id)
        .bind(user_id)
        .fetch_one(&core.db)
        .await?)
}

/// Sets or clears a channel's own verified badge (for staff tooling; a
/// channel of a verified account shows the badge regardless).
pub async fn set_verified(core: &Core, channel_id: Uuid, verified: bool) -> AppResult<()> {
    let mut tx = core.db.begin().await?;
    sqlx::query("UPDATE channels SET verified = $2 WHERE id = $1").bind(channel_id).bind(verified).execute(&mut *tx).await?;
    if let Some(channel) = find_in(&mut tx, channel_id).await? {
        search::index_channel(&mut tx, &channel).await?;
    }
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn find_in(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, channel_id: Uuid) -> AppResult<Option<ChannelRow>> {
    Ok(sqlx::query_as(concat!("SELECT ", channel_columns!(), " FROM channels c WHERE c.id = $1"))
        .bind(channel_id)
        .fetch_optional(&mut **tx)
        .await?)
}

// ───────────────────────────── page data ─────────────────────────────

#[derive(sqlx::FromRow)]
struct SubscriptionState {
    notify_level: i32,
    created_at: Time,
}

/// The channel header and about tab as `viewer` sees them.
pub async fn to_proto(core: &Core, viewer: &Viewer, channel: &ChannelRow) -> AppResult<Channel> {
    let video_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM videos WHERE channel_id = $1 AND status = 3 AND visibility IN (1, 2) AND NOT suspended",
    )
    .bind(channel.id)
    .fetch_one(&core.db)
    .await?;
    let trailer: Option<String> = match channel.trailer_video_id {
        Some(video_id) => {
            sqlx::query_scalar("SELECT short_id FROM videos WHERE id = $1 AND status = 3 AND visibility = 1")
                .bind(video_id)
                .fetch_optional(&core.db)
                .await?
        }
        None => None,
    };
    let viewer_state = match viewer.user_id {
        None => None,
        Some(user_id) => {
            let subscription: Option<SubscriptionState> =
                sqlx::query_as("SELECT notify_level, created_at FROM channel_subscriptions WHERE channel_id = $1 AND user_id = $2")
                    .bind(channel.id)
                    .bind(user_id)
                    .fetch_optional(&core.db)
                    .await?;
            let is_owner = channel.owner_id == user_id;
            Some(ChannelViewerState {
                is_owner,
                subscribed: subscription.is_some(),
                notification_level: subscription.as_ref().map(|s| s.notify_level).unwrap_or(0).into(),
                subscribed_at: opt_ts(subscription.map(|s| s.created_at)),
                is_default: is_owner && channel.is_default,
                ..Default::default()
            })
        }
    };
    Ok(Channel {
        summary: channel.summary().into(),
        description: channel.description.clone(),
        banner: image_from_json(channel.banner.as_ref()).into(),
        links: channel.links(),
        video_count,
        view_count: channel.view_count,
        created_at: ts(channel.created_at),
        trailer_video_id: trailer.unwrap_or_default(),
        viewer: viewer_state.into(),
        ..Default::default()
    })
}

// ───────────────────────────── create / update / delete ─────────────────────────────

/// What a caller wants a channel to look like. `None` = leave as it is.
#[derive(Debug, Default)]
pub struct ChannelEdit {
    pub handle: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    /// `Some(None)` removes the picture.
    pub icon_asset_id: Option<Option<Uuid>>,
    pub banner_asset_id: Option<Option<Uuid>>,
    pub links: Option<Vec<ChannelLink>>,
    /// `Some(None)` removes the trailer.
    pub trailer_video_id: Option<Option<Uuid>>,
}

/// A picture resolved from an upload of the caller: the stored `Image` JSON.
struct Art {
    asset_id: Uuid,
    image: serde_json::Value,
}

async fn resolve_art(core: &Core, owner_id: Uuid, asset_id: Option<Uuid>, purpose: AssetPurpose) -> AppResult<Option<Art>> {
    match asset_id {
        None => Ok(None),
        Some(asset_id) => {
            let image = assets::require_image_json(core, asset_id, owner_id, &[purpose]).await?;
            Ok(Some(Art { asset_id, image }))
        }
    }
}

fn links_json(links: &[ChannelLink]) -> AppResult<serde_json::Value> {
    serde_json::to_value(links).map_err(|e| AppError::internal(anyhow::anyhow!("channel links encode: {e}")))
}

/// Serializes changes to one account's set of channels (the limit, the
/// default flag) for the rest of the transaction.
async fn lock_owner(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, owner_id: Uuid) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("video:channels:{}", owner_id.simple()))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn handle_taken(error: AppError) -> AppError {
    if error.is_already_exists() { AppError::already_exists("HANDLE_TAKEN", "this channel handle is taken") } else { error }
}

/// Creates a channel for `owner_id`. The first channel of an account
/// becomes its default.
pub async fn create(core: &Core, owner_id: Uuid, max_channels: i64, edit: ChannelEdit) -> AppResult<ChannelRow> {
    let handle = validate::handle(edit.handle.as_deref().unwrap_or_default())?;
    let name = validate::channel_name(edit.name.as_deref().unwrap_or_default())?;
    let description = validate::channel_description(edit.description.as_deref().unwrap_or_default())?;
    let links = validate::channel_links(edit.links.as_deref().unwrap_or_default())?;
    let icon = resolve_art(core, owner_id, edit.icon_asset_id.flatten(), AssetPurpose::ASSET_PURPOSE_CHANNEL_ICON).await?;
    let banner = resolve_art(core, owner_id, edit.banner_asset_id.flatten(), AssetPurpose::ASSET_PURPOSE_CHANNEL_BANNER).await?;

    let mut tx = core.db.begin().await?;
    lock_owner(&mut tx, owner_id).await?;
    let owned: i64 = sqlx::query_scalar("SELECT count(*) FROM channels WHERE owner_id = $1").bind(owner_id).fetch_one(&mut *tx).await?;
    ensure!(owned < max_channels, precondition, "CHANNEL_LIMIT_REACHED", "an account can have at most {max_channels} channels");
    let suspended: bool = sqlx::query_scalar("SELECT COALESCE((SELECT banned_at IS NOT NULL FROM users WHERE id = $1), false)")
        .bind(owner_id)
        .fetch_one(&mut *tx)
        .await?;
    let channel_id = new_id();
    sqlx::query(
        "INSERT INTO channels (id, owner_id, handle, name, description, icon, icon_asset_id, banner, banner_asset_id, links, is_default, suspended)
         VALUES ($1, $2, $3::citext, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(channel_id)
    .bind(owner_id)
    .bind(&handle)
    .bind(&name)
    .bind(&description)
    .bind(icon.as_ref().map(|art| &art.image))
    .bind(icon.as_ref().map(|art| art.asset_id))
    .bind(banner.as_ref().map(|art| &art.image))
    .bind(banner.as_ref().map(|art| art.asset_id))
    .bind(links_json(&links)?)
    .bind(owned == 0)
    .bind(suspended)
    .execute(&mut *tx)
    .await
    .map_err(|e| handle_taken(e.into()))?;
    if let Some(art) = &icon {
        assets::attach(&mut *tx, art.asset_id, &art_holder("icon", channel_id)).await?;
    }
    if let Some(art) = &banner {
        assets::attach(&mut *tx, art.asset_id, &art_holder("banner", channel_id)).await?;
    }
    let channel = find_in(&mut tx, channel_id).await?.ok_or_else(not_found)?;
    core_events::emit(&mut *tx, CHANNEL_CREATED, Some(owner_id), &channel.event()).await?;
    search::index_channel(&mut tx, &channel).await?;
    tx.commit().await?;
    Ok(channel)
}

/// Applies `edit` to a channel of `owner_id`.
pub async fn update(core: &Core, owner_id: Uuid, channel: &ChannelRow, edit: ChannelEdit) -> AppResult<ChannelRow> {
    let handle = edit.handle.as_deref().map(validate::handle).transpose()?;
    let name = edit.name.as_deref().map(validate::channel_name).transpose()?;
    let description = edit.description.as_deref().map(validate::channel_description).transpose()?;
    let links = edit.links.as_deref().map(validate::channel_links).transpose()?;
    let icon = match edit.icon_asset_id {
        Some(asset_id) => Some(resolve_art(core, owner_id, asset_id, AssetPurpose::ASSET_PURPOSE_CHANNEL_ICON).await?),
        None => None,
    };
    let banner = match edit.banner_asset_id {
        Some(asset_id) => Some(resolve_art(core, owner_id, asset_id, AssetPurpose::ASSET_PURPOSE_CHANNEL_BANNER).await?),
        None => None,
    };

    let mut tx = core.db.begin().await?;
    if let Some(trailer) = edit.trailer_video_id {
        if let Some(video_id) = trailer {
            // Featured videos are what everyone can see on the page.
            let fits: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM videos WHERE id = $1 AND channel_id = $2 AND status = 3 AND visibility = 1)",
            )
            .bind(video_id)
            .bind(channel.id)
            .fetch_one(&mut *tx)
            .await?;
            ensure!(fits, not_found, "TRAILER_NOT_FOUND", "the trailer must be a public video of this channel");
        }
        sqlx::query("UPDATE channels SET trailer_video_id = $2 WHERE id = $1").bind(channel.id).bind(trailer).execute(&mut *tx).await?;
    }
    sqlx::query(
        "UPDATE channels SET handle = COALESCE($2::citext, handle), name = COALESCE($3, name),
                description = COALESCE($4, description), links = COALESCE($5, links)
         WHERE id = $1",
    )
    .bind(channel.id)
    .bind(&handle)
    .bind(&name)
    .bind(&description)
    .bind(links.as_deref().map(links_json).transpose()?)
    .execute(&mut *tx)
    .await
    .map_err(|e| handle_taken(e.into()))?;
    for (slot, change, current) in [("icon", icon, channel.icon_asset_id), ("banner", banner, channel.banner_asset_id)] {
        let Some(art) = change else { continue };
        let holder = art_holder(slot, channel.id);
        if let Some(previous) = current {
            assets::detach(&mut *tx, previous, &holder).await?;
        }
        if let Some(art) = &art {
            assets::attach(&mut *tx, art.asset_id, &holder).await?;
        }
        let sql = match slot {
            "icon" => "UPDATE channels SET icon = $2, icon_asset_id = $3 WHERE id = $1",
            _ => "UPDATE channels SET banner = $2, banner_asset_id = $3 WHERE id = $1",
        };
        sqlx::query(sql)
            .bind(channel.id)
            .bind(art.as_ref().map(|art| &art.image))
            .bind(art.as_ref().map(|art| art.asset_id))
            .execute(&mut *tx)
            .await?;
    }
    let updated = find_in(&mut tx, channel.id).await?.ok_or_else(not_found)?;
    core_events::emit(&mut *tx, CHANNEL_UPDATED, Some(owner_id), &updated.event()).await?;
    search::index_channel(&mut tx, &updated).await?;
    tx.commit().await?;
    Ok(updated)
}

/// Makes `channel_id` the default channel of its owner.
pub async fn set_default(core: &Core, owner_id: Uuid, channel_id: Uuid) -> AppResult<()> {
    let mut tx = core.db.begin().await?;
    lock_owner(&mut tx, owner_id).await?;
    // Clear first: the partial unique index allows one default at any moment.
    sqlx::query("UPDATE channels SET is_default = false WHERE owner_id = $1 AND is_default AND id <> $2")
        .bind(owner_id)
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;
    let done = sqlx::query("UPDATE channels SET is_default = true WHERE id = $1 AND owner_id = $2")
        .bind(channel_id)
        .bind(owner_id)
        .execute(&mut *tx)
        .await?;
    ensure!(done.rows_affected() == 1, not_found, "CHANNEL_NOT_FOUND", "channel not found");
    tx.commit().await?;
    Ok(())
}

/// Deletes a channel with everything on it, inside `tx`: its videos (with
/// their uploads), its playlists and its subscriptions. If it was the
/// owner's default, the oldest remaining channel takes over. `actor` is who
/// caused it (`None` for the system). Run the returned [`Aftermath`] once
/// the transaction has committed.
pub async fn delete_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    channel: &ChannelRow,
    actor: Option<Uuid>,
    force_assets: bool,
) -> AppResult<Aftermath> {
    lock_owner(tx, channel.owner_id).await?;
    let mut aftermath = Aftermath::default();
    let video_rows = videos::of_channel_in(tx, channel.id).await?;
    for video in &video_rows {
        aftermath.merge(videos::delete_in(tx, video, actor, force_assets).await?);
    }
    let playlist_ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM playlists WHERE channel_id = $1").bind(channel.id).fetch_all(&mut **tx).await?;
    for playlist_id in playlist_ids {
        crate::playlists::delete_in(tx, playlist_id, channel.owner_id, actor).await?;
    }
    for slot in ["icon", "banner"] {
        assets::detach_holder(&mut **tx, &art_holder(slot, channel.id)).await?;
    }
    sqlx::query("DELETE FROM channels WHERE id = $1").bind(channel.id).execute(&mut **tx).await?;
    if channel.is_default {
        sqlx::query(
            "UPDATE channels SET is_default = true
             WHERE id = (SELECT id FROM channels WHERE owner_id = $1 ORDER BY id LIMIT 1)",
        )
        .bind(channel.owner_id)
        .execute(&mut **tx)
        .await?;
    }
    outbound::unindex(&mut **tx, "channel", channel.id).await?;
    core_events::emit(&mut **tx, CHANNEL_DELETED, actor, &channel.event()).await?;
    Ok(aftermath)
}

/// Deletes a channel in a transaction of its own and cleans up after it.
pub async fn delete(core: &Core, channel: &ChannelRow, actor: Option<Uuid>, force_assets: bool) -> AppResult<()> {
    let mut tx = core.db.begin().await?;
    let aftermath = delete_in(&mut tx, channel, actor, force_assets).await?;
    tx.commit().await?;
    aftermath.run(core).await;
    Ok(())
}

// ───────────────────────────── subscriptions ─────────────────────────────

/// Subscribes `user_id` to `channel`, or changes the level of an existing
/// subscription (`level` of `None`: PERSONALIZED for a new one, unchanged
/// otherwise). Returns whether a new subscription was created.
pub async fn subscribe(core: &Core, user_id: Uuid, channel: &ChannelRow, level: Option<NotifyLevel>) -> AppResult<bool> {
    ensure!(channel.owner_id != user_id, invalid, "CANNOT_SUBSCRIBE_TO_OWN_CHANNEL", "you cannot subscribe to your own channel");
    let mut tx = core.db.begin().await?;
    let created: bool = sqlx::query_scalar(
        "INSERT INTO channel_subscriptions (channel_id, user_id, notify_level) VALUES ($1, $2, COALESCE($3, 2))
         ON CONFLICT (channel_id, user_id) DO UPDATE
             SET notify_level = COALESCE($3, channel_subscriptions.notify_level)
         RETURNING (xmax = 0)",
    )
    .bind(channel.id)
    .bind(user_id)
    .bind(level.map(|level| level as i32))
    .fetch_one(&mut *tx)
    .await?;
    if created {
        sqlx::query("UPDATE channels SET subscriber_count = subscriber_count + 1 WHERE id = $1").bind(channel.id).execute(&mut *tx).await?;
        let event = SubscriptionEvent { channel_id: channel.id, owner_id: channel.owner_id, user_id };
        core_events::emit(&mut *tx, CHANNEL_SUBSCRIBED, Some(user_id), &event).await?;
        let subscriber = tokuly_identity::summary(core, user_id).await?;
        let request = outbound::NotificationRequest::new(
            channel.owner_id,
            "video.subscribed",
            outbound::Localized::new("チャンネル登録されたよ!", "New subscriber!"),
            outbound::Localized::new(
                format!("{}さんが「{}」を登録したよ!", subscriber.display_name, channel.name),
                format!("{} subscribed to {}", subscriber.display_name, channel.name),
            ),
            channel.path(),
        )
        .actor(user_id)
        .subject(tokuly_proto::common::SubjectType::SUBJECT_TYPE_CHANNEL, channel.id)
        .collapse(format!("video.subscribed:{}", channel.id))
        .data(serde_json::json!({ "channel_id": channel.id, "channel_handle": channel.handle }));
        outbound::notify(&mut *tx, &request).await?;
    }
    tx.commit().await?;
    Ok(created)
}

/// Ends a subscription. Returns whether there was one.
pub async fn unsubscribe(core: &Core, user_id: Uuid, channel: &ChannelRow) -> AppResult<bool> {
    let mut tx = core.db.begin().await?;
    let removed = remove_subscription_in(&mut tx, user_id, channel.id, channel.owner_id).await?;
    tx.commit().await?;
    Ok(removed)
}

/// Removes one subscription inside `tx`, keeping the counter and the event
/// stream exact. Returns whether there was one.
pub(crate) async fn remove_subscription_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    channel_id: Uuid,
    owner_id: Uuid,
) -> AppResult<bool> {
    let removed = sqlx::query("DELETE FROM channel_subscriptions WHERE channel_id = $1 AND user_id = $2")
        .bind(channel_id)
        .bind(user_id)
        .execute(&mut **tx)
        .await?
        .rows_affected()
        == 1;
    if removed {
        sqlx::query("UPDATE channels SET subscriber_count = GREATEST(subscriber_count - 1, 0) WHERE id = $1")
            .bind(channel_id)
            .execute(&mut **tx)
            .await?;
        let event = SubscriptionEvent { channel_id, owner_id, user_id };
        core_events::emit(&mut **tx, CHANNEL_UNSUBSCRIBED, Some(user_id), &event).await?;
    }
    Ok(removed)
}

/// The level a client asked for, if it named one.
pub fn requested_level(level: buffa::EnumValue<NotificationLevel>) -> AppResult<Option<NotifyLevel>> {
    match level.as_known() {
        Some(NotificationLevel::NOTIFICATION_LEVEL_UNSPECIFIED) => Ok(None),
        Some(known) => Ok(NotifyLevel::from_i32(known as i32)),
        None => Err(AppError::invalid("NOTIFICATION_LEVEL_INVALID", "unknown notification level")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notify_levels_order_from_silent_to_everything() {
        assert!(NotifyLevel::None < NotifyLevel::Personalized && NotifyLevel::Personalized < NotifyLevel::All);
        assert_eq!(NotifyLevel::from_i32(3), Some(NotifyLevel::All));
        assert_eq!(NotifyLevel::from_i32(0), None);
        assert_eq!(requested_level(NotificationLevel::NOTIFICATION_LEVEL_UNSPECIFIED.into()).unwrap(), None);
        assert_eq!(requested_level(NotificationLevel::NOTIFICATION_LEVEL_NONE.into()).unwrap(), Some(NotifyLevel::None));
        assert_eq!(requested_level(buffa::EnumValue::from(9)).unwrap_err().reason, "NOTIFICATION_LEVEL_INVALID");
    }
}
