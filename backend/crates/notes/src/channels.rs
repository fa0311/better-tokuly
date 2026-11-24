//! Channels (チャンネル): topic timelines anyone can post into, with an
//! owner, moderators, followers and pinned notes.
//!
//! The first part is what the rest of the crate needs from channels:
//! whether a note may be posted into one, the reference shown on a note,
//! and the SQL that keeps channel notes out of the home, following and
//! global timelines unless the reader follows the channel. The rest is the
//! channels themselves, behind `NoteChannelService` (`channel_svc`).
//!
//! Who sees a channel: everyone, signed in or not — except that a channel
//! of a suspended or deleted owner is gone for everyone, and one whose owner
//! is in a block with the viewer is gone for that viewer (and they cannot
//! post into it). Who sees a channel's notes is [`crate::visibility`]'s
//! business as everywhere else.
//!
//! Locks: a channel's row (`FOR UPDATE`) serializes changes to it, its
//! moderators and its pins; an advisory lock per user serializes creating
//! channels (`notes:channels:<owner>`) and following them
//! (`notes:channel-follows:<user>`), so the limits hold under concurrency.
//! Nothing here reads through the pool while one of those is held.
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use tokuly_core::{
    outbound::{self, Localized, NotificationRequest, SearchDoc},
    prelude::*,
};
use tokuly_media::assets;
use tokuly_proto::{
    common::{SubjectType, UserSummary},
    media::AssetPurpose,
    notes::{ChannelFilter, ChannelRef, Note, NoteChannel, TimelineEntry},
};

use crate::{
    Notes, copy,
    model::{NOTE_COLUMNS, NoteRow, channel_holder, channel_path, note_path},
    text,
    visibility::{Viewer, sees_note},
};

/// SQL condition: the note in alias `n` belongs in a general timeline for
/// the viewer (`$1`): it is in no channel, a moderator took it out of its
/// channel, or the viewer follows the channel.
pub fn outside_unfollowed_channels(n: &str) -> String {
    format!(
        "({n}.channel_id IS NULL OR {n}.channel_removed_at IS NOT NULL \
          OR EXISTS (SELECT 1 FROM note_channel_follows cf_ WHERE cf_.channel_id = {n}.channel_id AND cf_.user_id = $1))"
    )
}

/// SQL condition: the note in alias `n` is a top-level note in a channel the
/// viewer (`$1`) follows (the home timeline shows those).
pub fn in_followed_channel(n: &str) -> String {
    format!(
        "({n}.channel_id IS NOT NULL AND {n}.channel_removed_at IS NULL AND {n}.reply_to_id IS NULL \
          AND EXISTS (SELECT 1 FROM note_channel_follows cf_ WHERE cf_.channel_id = {n}.channel_id AND cf_.user_id = $1))"
    )
}

/// A channel `author` is about to post into: it must exist for them (its
/// owner in good standing, no block between the two). Whether it is
/// archived is checked again when the note is counted ([`count_new_note`]),
/// inside the posting transaction.
pub(crate) async fn require_open(core: &Core, channel_id: Uuid, author: Uuid) -> AppResult<()> {
    let found: Option<(Uuid, bool, bool)> = sqlx::query_as(
        "SELECT c.owner_id, c.archived_at IS NOT NULL,
                EXISTS (SELECT 1 FROM users u WHERE u.id = c.owner_id AND u.banned_at IS NULL AND u.deleted_at IS NULL)
         FROM note_channels c WHERE c.id = $1",
    )
    .bind(channel_id)
    .fetch_optional(&core.db)
    .await?;
    let Some((owner, archived, owner_active)) = found else {
        bail!(not_found, "CHANNEL_NOT_FOUND", "channel not found");
    };
    if owner != author && (!owner_active || tokuly_social::graph::is_blocked_either(core, owner, author).await?) {
        bail!(not_found, "CHANNEL_NOT_FOUND", "channel not found");
    }
    ensure!(!archived, precondition, "CHANNEL_ARCHIVED", "this channel is archived");
    Ok(())
}

/// Counts a new note towards its channel, refusing (`CHANNEL_ARCHIVED`) a
/// channel archived since it was checked.
pub(crate) async fn count_new_note(tx: &mut Transaction<'_, Postgres>, channel_id: Uuid) -> AppResult<()> {
    let counted = sqlx::query(
        "UPDATE note_channels SET note_count = note_count + 1, last_note_at = now() WHERE id = $1 AND archived_at IS NULL",
    )
    .bind(channel_id)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    ensure!(counted == 1, precondition, "CHANNEL_ARCHIVED", "this channel is archived");
    Ok(())
}

/// Takes notes that left their channels (deleted, or taken out) off the
/// channels' counts: `(channel, how many)`.
pub(crate) async fn uncount_notes(tx: &mut Transaction<'_, Postgres>, gone: &HashMap<Uuid, i64>) -> AppResult<()> {
    if gone.is_empty() {
        return Ok(());
    }
    let mut ids: Vec<Uuid> = gone.keys().copied().collect();
    ids.sort();
    let counts: Vec<i64> = ids.iter().map(|id| gone[id]).collect();
    sqlx::query(
        "UPDATE note_channels c SET note_count = GREATEST(c.note_count - d.n, 0)
         FROM unnest($1::uuid[], $2::bigint[]) AS d(id, n) WHERE c.id = d.id",
    )
    .bind(&ids)
    .bind(&counts)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The references shown on notes for these channels.
pub(crate) async fn refs(core: &Core, channel_ids: &[Uuid]) -> AppResult<HashMap<Uuid, ChannelRef>> {
    if channel_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(Uuid, String, String)> =
        sqlx::query_as("SELECT id, name, color FROM note_channels WHERE id = ANY($1)")
            .bind(channel_ids)
            .fetch_all(&core.db)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name, color)| (id, ChannelRef { id: id.to_string(), name, color, ..Default::default() }))
        .collect())
}

// ───────────────────────────── The channels themselves ─────────────────────────────

pub const MAX_NAME_CHARS: usize = 50;
pub const MAX_DESCRIPTION_CHARS: usize = 500;
/// Channels one person may own (there is no deleting one: archive it).
pub const MAX_CHANNELS_PER_OWNER: i64 = 10;
/// Channels one person may follow.
pub const MAX_FOLLOWED: i64 = 500;
/// Moderators of one channel, the owner not counted.
pub const MAX_MODERATORS: i64 = 20;
/// Notes pinned to one channel.
pub const MAX_PINNED: i64 = 5;
/// Largest page of a channel list.
pub const MAX_LIST_PAGE: i64 = 50;
/// How deep the featured list goes.
const MAX_FEATURED_OFFSET: i64 = 1000;
/// A channel with a note this recent counts as active in the featured list.
const FEATURED_ACTIVE_DAYS: i32 = 7;

const CREATES_PER_HOUR: u32 = 10;
const UPDATES_PER_HOUR: u32 = 60;
const FOLLOWS_PER_MINUTE: u32 = 60;
const MODERATOR_CHANGES_PER_HOUR: u32 = 60;
const PINS_PER_MINUTE: u32 = 30;
const REMOVALS_PER_MINUTE: u32 = 60;

/// Search document kind of a channel.
pub const SEARCH_KIND: &str = "note_channel";

/// Every column of `note_channels`, for queries that alias the table as `c`.
pub(crate) const CHANNEL_COLUMNS: &str = "c.id, c.owner_id, c.name, c.description, c.banner_asset_id, c.banner, c.color, \
     c.follower_count, c.note_count, c.archived_at, c.last_note_at, c.created_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ChannelRow {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub name: String,
    pub description: String,
    pub banner_asset_id: Option<Uuid>,
    /// `tokuly.common.v1.Image` as proto JSON.
    pub banner: Option<serde_json::Value>,
    pub color: String,
    pub follower_count: i64,
    pub note_count: i64,
    pub archived_at: Option<DateTime<Utc>>,
    pub last_note_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl ChannelRow {
    fn banner(&self) -> Option<tokuly_proto::common::Image> {
        assets::image_from_json(self.banner.as_ref())
    }

    /// The channel in search: findable by everyone while it is not archived.
    fn search_doc(&self) -> SearchDoc {
        SearchDoc {
            kind: SEARCH_KIND.to_owned(),
            id: self.id,
            title: self.name.clone(),
            text: self.description.clone(),
            tags: Vec::new(),
            owner_id: Some(self.owner_id),
            public: self.archived_at.is_none(),
            image_url: self.banner().map(|image| image.url).filter(|url| !url.is_empty()),
            path: channel_path(self.id),
            created_at: self.created_at.timestamp(),
            score: self.follower_count as f64,
            extra: serde_json::json!({ "color": self.color, "follower_count": self.follower_count }),
        }
    }
}

/// SQL condition: the channel in alias `c` exists for the viewer (`$1`,
/// hidden people `$3`): theirs, or its owner is in good standing and not
/// in a block with them.
fn shown(c: &str) -> String {
    format!(
        "({c}.owner_id = $1 OR ({c}.owner_id <> ALL($3) AND EXISTS (SELECT 1 FROM users cu_ WHERE cu_.id = {c}.owner_id \
           AND cu_.banned_at IS NULL AND cu_.deleted_at IS NULL)))"
    )
}

/// What a channel is made of, as asked for and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChannelInput {
    pub name: String,
    pub description: String,
    pub banner_asset_id: Option<Uuid>,
    pub color: String,
}

impl ChannelInput {
    pub(crate) fn validate(name: &str, description: &str, banner_asset_id: &str, color: &str) -> AppResult<Self> {
        let name = text::clean(name).split_whitespace().collect::<Vec<_>>().join(" ");
        ensure!(
            !name.is_empty() && text::fits(&name, MAX_NAME_CHARS),
            invalid,
            "INVALID_NAME",
            "a channel name has 1-{MAX_NAME_CHARS} characters"
        );
        let description = text::clean(description);
        ensure!(
            text::fits(&description, MAX_DESCRIPTION_CHARS),
            invalid,
            "DESCRIPTION_TOO_LONG",
            "a channel description has at most {MAX_DESCRIPTION_CHARS} characters"
        );
        Ok(Self { name, description, banner_asset_id: parse_opt_id(banner_asset_id)?, color: validate_color(color)? })
    }
}

/// `#rrggbb` (lowercased), or empty for the default.
pub(crate) fn validate_color(input: &str) -> AppResult<String> {
    let color = input.trim();
    if color.is_empty() {
        return Ok(String::new());
    }
    let valid = color.len() == 7 && color.starts_with('#') && color[1..].chars().all(|c| c.is_ascii_hexdigit());
    ensure!(valid, invalid, "INVALID_COLOR", "a colour is written #rrggbb");
    Ok(color.to_ascii_lowercase())
}

/// A `LIKE` pattern matching names that contain `query` literally
/// (lowercased), or `None` for no filter.
pub(crate) fn like_pattern(query: &str) -> Option<String> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return None;
    }
    let mut pattern = String::with_capacity(query.len() + 2);
    pattern.push('%');
    for c in query.chars() {
        if matches!(c, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('%');
    Some(pattern)
}

/// What someone is to a channel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Role {
    pub owner: bool,
    pub moderator: bool,
}

impl Role {
    /// Pins notes, takes notes out, edits the channel.
    pub fn moderates(self) -> bool {
        self.owner || self.moderator
    }
}

async fn role_of<'e, E: sqlx::PgExecutor<'e>>(executor: E, channel: &ChannelRow, user_id: Uuid) -> AppResult<Role> {
    if channel.owner_id == user_id {
        return Ok(Role { owner: true, moderator: false });
    }
    let moderator: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM note_channel_moderators WHERE channel_id = $1 AND user_id = $2)",
    )
    .bind(channel.id)
    .bind(user_id)
    .fetch_one(executor)
    .await?;
    Ok(Role { owner: false, moderator })
}

fn require_moderator(role: Role) -> AppResult<()> {
    ensure!(role.moderates(), denied, "NOT_CHANNEL_MODERATOR", "only the channel's owner and moderators can do this");
    Ok(())
}

/// Locks a channel's row inside `tx` and returns it as it is now.
async fn lock_channel(tx: &mut Transaction<'_, Postgres>, channel_id: Uuid) -> AppResult<ChannelRow> {
    let sql = format!("SELECT {CHANNEL_COLUMNS} FROM note_channels c WHERE c.id = $1 FOR UPDATE");
    sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(channel_id)
        .fetch_optional(&mut **tx)
        .await?
        .or_not_found("CHANNEL_NOT_FOUND", "channel not found")
}

/// Takes a transaction-scoped advisory lock on `key`.
async fn advisory_lock(tx: &mut Transaction<'_, Postgres>, key: &str) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))").bind(key).execute(&mut **tx).await?;
    Ok(())
}

/// What the banner of a channel becomes in an update.
enum BannerChange {
    Keep,
    Remove,
    Set(Uuid, serde_json::Value),
}

/// `notes.channel_removed`: a moderator took the recipient's note out of a channel.
fn removed_notification(actor_id: Uuid, channel: &ChannelRow, note: &NoteRow) -> NotificationRequest {
    let name = &channel.name;
    let title = Localized::new(
        format!("ノートがチャンネル「{name}」から外されたよ"),
        format!("Your note was taken out of the channel “{name}”"),
    );
    let excerpt = text::excerpt(&note.text, 80);
    let body = if excerpt.is_empty() {
        Localized::new("ノートはプロフィールに残っているよ", "It is still on your profile")
    } else {
        Localized::same(excerpt)
    };
    NotificationRequest::new(note.author_id, "notes.channel_removed", title, body, note_path(note.id))
        .actor(actor_id)
        .subject(SubjectType::SUBJECT_TYPE_NOTE, note.id)
        .data(serde_json::json!({ "note_id": note.id, "channel_id": channel.id }))
}

/// `notes.channel_moderator`: the owner made the recipient a moderator.
fn moderator_notification(
    actor: &UserSummary,
    actor_id: Uuid,
    recipient: Uuid,
    channel: &ChannelRow,
) -> NotificationRequest {
    let who = copy::display_name(actor);
    let name = &channel.name;
    let title = Localized::new(
        format!("{who}さんがチャンネル「{name}」のモデレーターにしてくれたよ!"),
        format!("{who} made you a moderator of the channel “{name}”"),
    );
    let body = Localized::new("ピン留めやノートの整理を手伝ってね!", "Help pin and tidy up its notes!");
    NotificationRequest::new(recipient, "notes.channel_moderator", title, body, channel_path(channel.id))
        .actor(actor_id)
        .data(serde_json::json!({ "channel_id": channel.id }))
        .collapse(format!("notes.channel_moderator:{}", channel.id))
}

/// A channel list row: the key it is paginated by, and the channel.
#[derive(sqlx::FromRow)]
struct ListedChannel {
    key_id: Uuid,
    #[sqlx(flatten)]
    channel: ChannelRow,
}

impl Notes {
    /// The channel as the viewer may see it, or `CHANNEL_NOT_FOUND`.
    pub(crate) async fn channel_for(&self, viewer: &Viewer, channel_id: Uuid) -> AppResult<ChannelRow> {
        let sql = format!("SELECT {CHANNEL_COLUMNS} FROM note_channels c WHERE c.id = $7 AND {}", shown("c"));
        let row: Option<ChannelRow> = viewer.query_as(sql).bind(channel_id).fetch_optional(&self.core.db).await?;
        row.or_not_found("CHANNEL_NOT_FOUND", "channel not found")
    }

    /// Channels as `viewer` gets them, in the order of `rows`.
    pub(crate) async fn channel_views(&self, viewer: &Viewer, rows: &[ChannelRow]) -> AppResult<Vec<NoteChannel>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let db = &self.core.db;
        let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        let mut owners: Vec<Uuid> = rows.iter().map(|row| row.owner_id).collect();
        owners.sort_unstable();
        owners.dedup();
        let users = tokuly_identity::summaries(&self.core, &owners).await?;
        let (following, moderating): (HashSet<Uuid>, HashSet<Uuid>) = match viewer.id {
            Some(user_id) => {
                let ties: Vec<(Uuid, bool)> = sqlx::query_as(
                    "SELECT channel_id, true FROM note_channel_follows WHERE user_id = $1 AND channel_id = ANY($2)
                     UNION ALL
                     SELECT channel_id, false FROM note_channel_moderators WHERE user_id = $1 AND channel_id = ANY($2)",
                )
                .bind(user_id)
                .bind(&ids)
                .fetch_all(db)
                .await?;
                (
                    ties.iter().filter(|(_, follow)| *follow).map(|(id, _)| *id).collect(),
                    ties.iter().filter(|(_, follow)| !*follow).map(|(id, _)| *id).collect(),
                )
            }
            None => (HashSet::new(), HashSet::new()),
        };
        // Pinned notes the viewer may see, latest pinned first.
        let sql = format!(
            "SELECT p.channel_id, p.note_id FROM note_channel_pins p JOIN notes n ON n.id = p.note_id
             WHERE p.channel_id = ANY($7) AND n.channel_id = p.channel_id AND n.channel_removed_at IS NULL AND {}
             ORDER BY p.pinned_at DESC, p.note_id DESC",
            sees_note("n")
        );
        let pins: Vec<(Uuid, Uuid)> = viewer.query_as(sql).bind(&ids).fetch_all(db).await?;
        let mut pinned: HashMap<Uuid, Vec<String>> = HashMap::new();
        for (channel_id, note_id) in pins {
            pinned.entry(channel_id).or_default().push(note_id.to_string());
        }
        Ok(rows
            .iter()
            .map(|row| NoteChannel {
                id: row.id.to_string(),
                owner: tokuly_identity::users::pick(&users, row.owner_id).into(),
                name: row.name.clone(),
                description: row.description.clone(),
                banner: row.banner().into(),
                color: row.color.clone(),
                follower_count: row.follower_count,
                note_count: row.note_count,
                archived: row.archived_at.is_some(),
                last_note_at: opt_ts(row.last_note_at),
                created_at: ts(row.created_at),
                following: following.contains(&row.id),
                is_owner: viewer.is(row.owner_id),
                is_moderator: moderating.contains(&row.id),
                pinned_note_ids: pinned.remove(&row.id).unwrap_or_default(),
                ..Default::default()
            })
            .collect())
    }

    pub(crate) async fn channel_view(&self, viewer: &Viewer, row: &ChannelRow) -> AppResult<NoteChannel> {
        Ok(self.channel_views(viewer, std::slice::from_ref(row)).await?.pop().expect("one channel in, one out"))
    }

    /// Resolves a banner upload of `user_id` (a NOTE_IMAGE) to its picture.
    async fn banner_image(&self, user_id: Uuid, asset_id: Uuid) -> AppResult<serde_json::Value> {
        assets::require_image_json(&self.core, asset_id, user_id, &[AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE]).await
    }

    /// Creates a channel owned by `owner` (at most [`MAX_CHANNELS_PER_OWNER`]).
    pub(crate) async fn create_channel(&self, owner: Uuid, input: ChannelInput) -> AppResult<ChannelRow> {
        self.core
            .cache
            .rate_limit(&format!("notes:channel-create:{owner}"), CREATES_PER_HOUR, Duration::from_secs(3600))
            .await?;
        let banner = match input.banner_asset_id {
            Some(asset_id) => Some(self.banner_image(owner, asset_id).await?),
            None => None,
        };
        let mut tx = self.core.db.begin().await?;
        advisory_lock(&mut tx, &format!("notes:channels:{owner}")).await?;
        let owned: i64 = sqlx::query_scalar("SELECT count(*) FROM note_channels WHERE owner_id = $1")
            .bind(owner)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(
            owned < MAX_CHANNELS_PER_OWNER,
            precondition,
            "TOO_MANY_CHANNELS",
            "one person can own at most {MAX_CHANNELS_PER_OWNER} channels"
        );
        let sql = format!(
            "INSERT INTO note_channels AS c (owner_id, name, description, banner_asset_id, banner, color)
             VALUES ($1, $2, $3, $4, $5, $6) RETURNING {CHANNEL_COLUMNS}"
        );
        let row: ChannelRow = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(owner)
            .bind(&input.name)
            .bind(&input.description)
            .bind(input.banner_asset_id)
            .bind(banner)
            .bind(&input.color)
            .fetch_one(&mut *tx)
            .await?;
        if let Some(asset_id) = row.banner_asset_id {
            assets::attach(&mut *tx, asset_id, &channel_holder(row.id)).await?;
        }
        outbound::index(&mut *tx, row.search_doc()).await?;
        tx.commit().await?;
        Ok(row)
    }

    /// The owner or a moderator changes a channel's name, description,
    /// banner and colour. An archived channel is not changed.
    pub(crate) async fn update_channel(
        &self,
        user_id: Uuid,
        channel_id: Uuid,
        input: ChannelInput,
    ) -> AppResult<ChannelRow> {
        let viewer = Viewer::load(&self.core, Some(user_id)).await?;
        let before = self.channel_for(&viewer, channel_id).await?;
        require_moderator(role_of(&self.core.db, &before, user_id).await?)?;
        self.core
            .cache
            .rate_limit(&format!("notes:channel-update:{user_id}"), UPDATES_PER_HOUR, Duration::from_secs(3600))
            .await?;
        // The picture for the banner asked for, resolved before anything is
        // locked: the current one as stored, a new one as the caller's upload.
        let wanted = match input.banner_asset_id {
            Some(asset_id) if Some(asset_id) == before.banner_asset_id => Some((asset_id, before.banner.clone())),
            Some(asset_id) => Some((asset_id, Some(self.banner_image(user_id, asset_id).await?))),
            None => None,
        };

        let mut tx = self.core.db.begin().await?;
        let channel = lock_channel(&mut tx, channel_id).await?;
        require_moderator(role_of(&mut *tx, &channel, user_id).await?)?;
        ensure!(channel.archived_at.is_none(), precondition, "CHANNEL_ARCHIVED", "this channel is archived");
        let change = match wanted {
            Some((asset_id, _)) if Some(asset_id) == channel.banner_asset_id => BannerChange::Keep,
            Some((asset_id, Some(image))) => BannerChange::Set(asset_id, image),
            Some((_, None)) => BannerChange::Remove,
            None if channel.banner_asset_id.is_some() => BannerChange::Remove,
            None => BannerChange::Keep,
        };
        let holder = channel_holder(channel.id);
        let (banner_asset_id, banner) = match &change {
            BannerChange::Keep => (channel.banner_asset_id, channel.banner.clone()),
            BannerChange::Remove => (None, None),
            BannerChange::Set(asset_id, image) => (Some(*asset_id), Some(image.clone())),
        };
        if !matches!(change, BannerChange::Keep) {
            if let Some(old) = channel.banner_asset_id {
                assets::detach(&mut *tx, old, &holder).await?;
            }
            if let Some(new) = banner_asset_id {
                assets::attach(&mut *tx, new, &holder).await?;
            }
        }
        let sql = format!(
            "UPDATE note_channels AS c SET name = $2, description = $3, color = $4, banner_asset_id = $5, banner = $6
             WHERE c.id = $1 RETURNING {CHANNEL_COLUMNS}"
        );
        let row: ChannelRow = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(channel.id)
            .bind(&input.name)
            .bind(&input.description)
            .bind(&input.color)
            .bind(banner_asset_id)
            .bind(banner)
            .fetch_one(&mut *tx)
            .await?;
        outbound::index(&mut *tx, row.search_doc()).await?;
        tx.commit().await?;
        Ok(row)
    }

    /// The owner archives a channel (no new notes, no more edits) or opens it again.
    pub(crate) async fn archive_channel(
        &self,
        user_id: Uuid,
        channel_id: Uuid,
        archived: bool,
    ) -> AppResult<ChannelRow> {
        let viewer = Viewer::load(&self.core, Some(user_id)).await?;
        let channel = self.channel_for(&viewer, channel_id).await?;
        ensure!(channel.owner_id == user_id, denied, "NOT_CHANNEL_OWNER", "only the channel's owner can do this");
        self.core
            .cache
            .rate_limit(&format!("notes:channel-update:{user_id}"), UPDATES_PER_HOUR, Duration::from_secs(3600))
            .await?;
        let mut tx = self.core.db.begin().await?;
        let sql = format!(
            "UPDATE note_channels AS c SET archived_at = CASE WHEN $2 THEN COALESCE(c.archived_at, now()) END
             WHERE c.id = $1 RETURNING {CHANNEL_COLUMNS}"
        );
        let row: ChannelRow =
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(channel.id).bind(archived).fetch_one(&mut *tx).await?;
        outbound::index(&mut *tx, row.search_doc()).await?;
        tx.commit().await?;
        Ok(row)
    }

    /// Channels by filter (featured for anonymous callers), optionally
    /// those whose name contains `query`.
    pub(crate) async fn list_channels(
        &self,
        viewer: &Viewer,
        filter: ChannelFilter,
        query: &str,
        page: &Page,
    ) -> AppResult<(Vec<NoteChannel>, tokuly_proto::common::PageInfo)> {
        let pattern = like_pattern(query);
        let visible = shown("c");
        let named = "($7::text IS NULL OR lower(c.name) LIKE $7 ESCAPE '\\')";
        let (rows, info) = match filter {
            ChannelFilter::CHANNEL_FILTER_FOLLOWED | ChannelFilter::CHANNEL_FILTER_MANAGED => {
                let cursor: Option<Uuid> = page.decode()?;
                let sql = if filter == ChannelFilter::CHANNEL_FILTER_FOLLOWED {
                    format!(
                        "SELECT f.id AS key_id, {CHANNEL_COLUMNS} FROM note_channel_follows f JOIN note_channels c ON c.id = f.channel_id
                         WHERE f.user_id = $1 AND {visible} AND {named} AND ($8::uuid IS NULL OR f.id < $8)
                         ORDER BY f.id DESC LIMIT $9"
                    )
                } else {
                    format!(
                        "SELECT c.id AS key_id, {CHANNEL_COLUMNS} FROM note_channels c
                         WHERE (c.owner_id = $1 OR EXISTS (SELECT 1 FROM note_channel_moderators m WHERE m.channel_id = c.id AND m.user_id = $1))
                           AND {visible} AND {named} AND ($8::uuid IS NULL OR c.id < $8)
                         ORDER BY c.id DESC LIMIT $9"
                    )
                };
                let mut rows: Vec<ListedChannel> =
                    viewer.query_as(sql).bind(pattern).bind(cursor).bind(page.fetch()).fetch_all(&self.core.db).await?;
                let info = page.finish(&mut rows, |row| row.key_id);
                (rows.into_iter().map(|row| row.channel).collect::<Vec<_>>(), info)
            }
            _ => {
                // Ranked rather than keyed: the position is the cursor.
                let offset = page.decode::<i64>()?.unwrap_or(0).max(0);
                if offset >= MAX_FEATURED_OFFSET {
                    return Ok((Vec::new(), Default::default()));
                }
                let sql = format!(
                    "SELECT {CHANNEL_COLUMNS} FROM note_channels c
                     WHERE c.archived_at IS NULL AND {visible} AND {named}
                     ORDER BY (c.last_note_at > now() - make_interval(days => {FEATURED_ACTIVE_DAYS})) DESC NULLS LAST,
                              c.follower_count DESC, c.last_note_at DESC NULLS LAST, c.id DESC
                     OFFSET $8 LIMIT $9"
                );
                let mut rows: Vec<ChannelRow> =
                    viewer.query_as(sql).bind(pattern).bind(offset).bind(page.fetch()).fetch_all(&self.core.db).await?;
                let next = offset + page.limit;
                let info = page.finish(&mut rows, |_| next);
                (rows, info)
            }
        };
        Ok((self.channel_views(viewer, &rows).await?, info))
    }

    /// `user_id` follows a channel (at most [`MAX_FOLLOWED`]). Following
    /// twice changes nothing.
    pub(crate) async fn follow_channel(&self, user_id: Uuid, channel_id: Uuid) -> AppResult<ChannelRow> {
        self.core
            .cache
            .rate_limit(&format!("notes:channel-follow:{user_id}"), FOLLOWS_PER_MINUTE, Duration::from_secs(60))
            .await?;
        let viewer = Viewer::load(&self.core, Some(user_id)).await?;
        let channel = self.channel_for(&viewer, channel_id).await?;
        let mut tx = self.core.db.begin().await?;
        advisory_lock(&mut tx, &format!("notes:channel-follows:{user_id}")).await?;
        let (followed, this_one): (i64, bool) = sqlx::query_as(
            "SELECT count(*), coalesce(bool_or(channel_id = $2), false) FROM note_channel_follows WHERE user_id = $1",
        )
        .bind(user_id)
        .bind(channel.id)
        .fetch_one(&mut *tx)
        .await?;
        if !this_one {
            ensure!(
                followed < MAX_FOLLOWED,
                precondition,
                "TOO_MANY_FOLLOWED_CHANNELS",
                "you can follow at most {MAX_FOLLOWED} channels"
            );
            // The count moves only for a follow that is new.
            sqlx::query(
                "WITH added AS (
                     INSERT INTO note_channel_follows (channel_id, user_id) VALUES ($1, $2)
                     ON CONFLICT DO NOTHING RETURNING channel_id
                 )
                 UPDATE note_channels SET follower_count = follower_count + 1 WHERE id IN (SELECT channel_id FROM added)",
            )
            .bind(channel.id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        self.channel_for(&viewer, channel.id).await
    }

    /// `user_id` stops following a channel, if they did.
    pub(crate) async fn unfollow_channel(&self, user_id: Uuid, channel_id: Uuid) -> AppResult<()> {
        self.core
            .cache
            .rate_limit(&format!("notes:channel-follow:{user_id}"), FOLLOWS_PER_MINUTE, Duration::from_secs(60))
            .await?;
        sqlx::query(
            "WITH gone AS (DELETE FROM note_channel_follows WHERE channel_id = $1 AND user_id = $2 RETURNING channel_id)
             UPDATE note_channels SET follower_count = GREATEST(follower_count - 1, 0) WHERE id IN (SELECT channel_id FROM gone)",
        )
        .bind(channel_id)
        .bind(user_id)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    /// The owner makes `target` a moderator (at most [`MAX_MODERATORS`]);
    /// they are told. Nobody in a block with the owner can be one.
    pub(crate) async fn add_channel_moderator(&self, owner: Uuid, channel_id: Uuid, target: Uuid) -> AppResult<()> {
        let viewer = Viewer::load(&self.core, Some(owner)).await?;
        let channel = self.channel_for(&viewer, channel_id).await?;
        ensure!(channel.owner_id == owner, denied, "NOT_CHANNEL_OWNER", "only the channel's owner can do this");
        self.core
            .cache
            .rate_limit(
                &format!("notes:channel-moderators:{owner}"),
                MODERATOR_CHANGES_PER_HOUR,
                Duration::from_secs(3600),
            )
            .await?;
        if target == owner {
            return Ok(());
        }
        self.require_user(&viewer, target).await?;
        let actor = tokuly_identity::summary(&self.core, owner).await?;
        let mut tx = self.core.db.begin().await?;
        let channel = lock_channel(&mut tx, channel.id).await?;
        let (count, already): (i64, bool) = sqlx::query_as(
            "SELECT count(*), coalesce(bool_or(user_id = $2), false) FROM note_channel_moderators WHERE channel_id = $1",
        )
        .bind(channel.id)
        .bind(target)
        .fetch_one(&mut *tx)
        .await?;
        if already {
            return Ok(());
        }
        ensure!(
            count < MAX_MODERATORS,
            precondition,
            "TOO_MANY_MODERATORS",
            "a channel has at most {MAX_MODERATORS} moderators"
        );
        sqlx::query("INSERT INTO note_channel_moderators (channel_id, user_id) VALUES ($1, $2)")
            .bind(channel.id)
            .bind(target)
            .execute(&mut *tx)
            .await?;
        outbound::notify(&mut *tx, &moderator_notification(&actor, owner, target, &channel)).await?;
        tx.commit().await?;
        Ok(())
    }

    /// The owner removes a moderator, or a moderator steps down.
    pub(crate) async fn remove_channel_moderator(
        &self,
        user_id: Uuid,
        channel_id: Uuid,
        target: Uuid,
    ) -> AppResult<()> {
        let viewer = Viewer::load(&self.core, Some(user_id)).await?;
        let channel = self.channel_for(&viewer, channel_id).await?;
        ensure!(
            channel.owner_id == user_id || target == user_id,
            denied,
            "NOT_CHANNEL_OWNER",
            "only the channel's owner can remove other moderators"
        );
        self.core
            .cache
            .rate_limit(
                &format!("notes:channel-moderators:{user_id}"),
                MODERATOR_CHANGES_PER_HOUR,
                Duration::from_secs(3600),
            )
            .await?;
        sqlx::query("DELETE FROM note_channel_moderators WHERE channel_id = $1 AND user_id = $2")
            .bind(channel.id)
            .bind(target)
            .execute(&self.core.db)
            .await?;
        Ok(())
    }

    /// The moderators of a channel the viewer may see (people hidden from
    /// them by a block, and accounts not in good standing, left out).
    pub(crate) async fn channel_moderators(&self, viewer: &Viewer, channel_id: Uuid) -> AppResult<Vec<UserSummary>> {
        let channel = self.channel_for(viewer, channel_id).await?;
        let sql = "SELECT m.user_id FROM note_channel_moderators m
                   JOIN users u ON u.id = m.user_id AND u.banned_at IS NULL AND u.deleted_at IS NULL
                   WHERE m.channel_id = $7 AND m.user_id <> ALL($3)
                   ORDER BY m.added_at, m.user_id"
            .to_owned();
        let ids: Vec<Uuid> = viewer.query_scalar(sql).bind(channel.id).fetch_all(&self.core.db).await?;
        let users = tokuly_identity::summaries(&self.core, &ids).await?;
        Ok(ids.into_iter().map(|id| tokuly_identity::users::pick(&users, id)).collect())
    }

    /// A note of the channel the moderator may see, or `NOTE_NOT_FOUND`.
    async fn channel_note(&self, viewer: &Viewer, channel: &ChannelRow, note_id: Uuid) -> AppResult<NoteRow> {
        self.find_note(viewer, note_id)
            .await?
            .filter(|note| note.shown_channel() == Some(channel.id))
            .or_not_found("NOTE_NOT_FOUND", "note not found")
    }

    /// Locks a note of the channel, refusing one that left it. Notes are
    /// locked before their channel, as posting and deleting do.
    async fn lock_channel_note(tx: &mut Transaction<'_, Postgres>, channel_id: Uuid, note_id: Uuid) -> AppResult<()> {
        let found: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM notes WHERE id = $1 AND channel_id = $2 AND channel_removed_at IS NULL FOR UPDATE",
        )
        .bind(note_id)
        .bind(channel_id)
        .fetch_optional(&mut **tx)
        .await?;
        found.map(|_| ()).or_not_found("NOTE_NOT_FOUND", "note not found")
    }

    /// A moderator pins a note of the channel (at most [`MAX_PINNED`]).
    /// Pinning a pinned note changes nothing.
    pub(crate) async fn pin_channel_note(&self, user_id: Uuid, channel_id: Uuid, note_id: Uuid) -> AppResult<()> {
        let viewer = Viewer::load(&self.core, Some(user_id)).await?;
        let channel = self.channel_for(&viewer, channel_id).await?;
        require_moderator(role_of(&self.core.db, &channel, user_id).await?)?;
        self.core
            .cache
            .rate_limit(&format!("notes:channel-pin:{user_id}"), PINS_PER_MINUTE, Duration::from_secs(60))
            .await?;
        let note = self.channel_note(&viewer, &channel, note_id).await?;
        let mut tx = self.core.db.begin().await?;
        Self::lock_channel_note(&mut tx, channel.id, note.id).await?;
        let channel = lock_channel(&mut tx, channel.id).await?;
        require_moderator(role_of(&mut *tx, &channel, user_id).await?)?;
        let (pinned, this_one): (i64, bool) = sqlx::query_as(
            "SELECT count(*), coalesce(bool_or(note_id = $2), false) FROM note_channel_pins WHERE channel_id = $1",
        )
        .bind(channel.id)
        .bind(note.id)
        .fetch_one(&mut *tx)
        .await?;
        if this_one {
            return Ok(());
        }
        ensure!(pinned < MAX_PINNED, precondition, "TOO_MANY_PINNED", "a channel pins at most {MAX_PINNED} notes");
        sqlx::query(
            "INSERT INTO note_channel_pins (channel_id, note_id, pinned_at) VALUES ($1, $2, clock_timestamp())",
        )
        .bind(channel.id)
        .bind(note.id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// A moderator unpins a note of the channel, if it was pinned.
    pub(crate) async fn unpin_channel_note(&self, user_id: Uuid, channel_id: Uuid, note_id: Uuid) -> AppResult<()> {
        let viewer = Viewer::load(&self.core, Some(user_id)).await?;
        let channel = self.channel_for(&viewer, channel_id).await?;
        require_moderator(role_of(&self.core.db, &channel, user_id).await?)?;
        self.core
            .cache
            .rate_limit(&format!("notes:channel-pin:{user_id}"), PINS_PER_MINUTE, Duration::from_secs(60))
            .await?;
        sqlx::query("DELETE FROM note_channel_pins WHERE channel_id = $1 AND note_id = $2")
            .bind(channel.id)
            .bind(note_id)
            .execute(&self.core.db)
            .await?;
        Ok(())
    }

    /// A moderator takes a note out of the channel: it stays on its
    /// author's profile, leaves the channel's count and pins, and its author
    /// is told (unless they took it out themself).
    pub(crate) async fn remove_channel_note(&self, user_id: Uuid, channel_id: Uuid, note_id: Uuid) -> AppResult<()> {
        let viewer = Viewer::load(&self.core, Some(user_id)).await?;
        let channel = self.channel_for(&viewer, channel_id).await?;
        require_moderator(role_of(&self.core.db, &channel, user_id).await?)?;
        self.core
            .cache
            .rate_limit(&format!("notes:channel-remove:{user_id}"), REMOVALS_PER_MINUTE, Duration::from_secs(60))
            .await?;
        let note = self.channel_note(&viewer, &channel, note_id).await?;
        let mut tx = self.core.db.begin().await?;
        Self::lock_channel_note(&mut tx, channel.id, note.id).await?;
        let channel = lock_channel(&mut tx, channel.id).await?;
        require_moderator(role_of(&mut *tx, &channel, user_id).await?)?;
        sqlx::query("UPDATE notes SET channel_removed_at = now() WHERE id = $1")
            .bind(note.id)
            .execute(&mut *tx)
            .await?;
        uncount_notes(&mut tx, &HashMap::from([(channel.id, 1)])).await?;
        sqlx::query("DELETE FROM note_channel_pins WHERE channel_id = $1 AND note_id = $2")
            .bind(channel.id)
            .bind(note.id)
            .execute(&mut *tx)
            .await?;
        if note.author_id != user_id {
            outbound::notify(&mut *tx, &removed_notification(user_id, &channel, &note)).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// The channel's notes the viewer may see, newest first (replies
    /// included), and on the first page its pinned notes, latest pinned first.
    pub(crate) async fn channel_timeline(
        &self,
        viewer: &Viewer,
        channel_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<TimelineEntry>, tokuly_proto::common::PageInfo, Vec<Note>)> {
        let channel = self.channel_for(viewer, channel_id).await?;
        let visible = sees_note("n");
        let sql = format!(
            "SELECT n.id AS entry_id, NULL::uuid AS renoter_id, NULL::timestamptz AS renoted_at, {NOTE_COLUMNS}
             FROM notes n
             WHERE n.channel_id = $7 AND n.channel_removed_at IS NULL AND ($8::uuid IS NULL OR n.id < $8) AND {visible}
             ORDER BY n.id DESC LIMIT $9"
        );
        let first_page = page.cursor.is_none();
        let (entries, info) = self.entries(viewer, page, sql, channel.id).await?;
        let pinned = if first_page {
            let sql = format!(
                "SELECT {NOTE_COLUMNS} FROM note_channel_pins p JOIN notes n ON n.id = p.note_id
                 WHERE p.channel_id = $7 AND n.channel_id = $7 AND n.channel_removed_at IS NULL AND {visible}
                 ORDER BY p.pinned_at DESC, p.note_id DESC"
            );
            let mut rows: Vec<NoteRow> = viewer.query_as(sql).bind(channel.id).fetch_all(&self.core.db).await?;
            viewer.drop_newly_locked(&self.core, &mut rows, |row| row).await?;
            self.hydrate(viewer, &rows).await?
        } else {
            Vec::new()
        };
        Ok((entries, info, pinned))
    }

    // ── When accounts and uploads go away ──

    /// `identity.user.deleted`: the account's follows (counts put right)
    /// and moderator seats go, and its channels are deleted. Notes other
    /// people posted into those channels stay, outside any channel.
    pub(crate) async fn erase_channels_of(&self, user_id: Uuid) -> AppResult<()> {
        let db = &self.core.db;
        let mut tx = db.begin().await?;
        let followed: Vec<Uuid> =
            sqlx::query_scalar("DELETE FROM note_channel_follows WHERE user_id = $1 RETURNING channel_id")
                .bind(user_id)
                .fetch_all(&mut *tx)
                .await?;
        uncount_followers(&mut tx, followed).await?;
        sqlx::query("DELETE FROM note_channel_moderators WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
        tx.commit().await?;

        let owned: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM note_channels WHERE owner_id = $1 ORDER BY id")
            .bind(user_id)
            .fetch_all(db)
            .await?;
        for channel_id in owned {
            let mut tx = db.begin().await?;
            // The notes first (they lose their channel), then the channel.
            sqlx::query("SELECT id FROM notes WHERE channel_id = $1 ORDER BY id FOR UPDATE")
                .bind(channel_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM note_channels WHERE id = $1").bind(channel_id).execute(&mut *tx).await?;
            assets::detach_holder(&mut *tx, &channel_holder(channel_id)).await?;
            outbound::unindex(&mut *tx, SEARCH_KIND, channel_id).await?;
            tx.commit().await?;
        }
        Ok(())
    }

    /// `media.asset.deleted`: a banner upload is gone; the channels showing
    /// it go without a banner.
    pub(crate) async fn forget_channel_banner(&self, asset_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let sql = format!(
            "UPDATE note_channels AS c SET banner_asset_id = NULL, banner = NULL WHERE c.banner_asset_id = $1 RETURNING {CHANNEL_COLUMNS}"
        );
        let rows: Vec<ChannelRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(asset_id).fetch_all(&mut *tx).await?;
        for row in rows {
            outbound::index(&mut *tx, row.search_doc()).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// `social.user.blocked`: neither of the two moderates the other's channels any more.
    pub(crate) async fn drop_channel_moderators_across(&self, a: Uuid, b: Uuid) -> AppResult<()> {
        sqlx::query(
            "DELETE FROM note_channel_moderators m USING note_channels c
             WHERE c.id = m.channel_id AND ((c.owner_id = $1 AND m.user_id = $2) OR (c.owner_id = $2 AND m.user_id = $1))",
        )
        .bind(a)
        .bind(b)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }
}

/// Takes one follower per entry off those channels, in ID order.
async fn uncount_followers(tx: &mut Transaction<'_, Postgres>, mut channel_ids: Vec<Uuid>) -> AppResult<()> {
    if channel_ids.is_empty() {
        return Ok(());
    }
    channel_ids.sort_unstable();
    sqlx::query(
        "UPDATE note_channels c SET follower_count = GREATEST(c.follower_count - 1, 0)
         FROM (SELECT id FROM note_channels WHERE id = ANY($1) ORDER BY id FOR UPDATE) locked
         WHERE c.id = locked.id",
    )
    .bind(&channel_ids)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The `channels` and `channel_follows` sections of the personal data export.
pub(crate) async fn export(db: &sqlx::PgPool, user_id: Uuid) -> AppResult<(serde_json::Value, serde_json::Value)> {
    let owned: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', c.id, 'name', c.name, 'description', c.description, 'color', c.color,
                'banner_asset_id', c.banner_asset_id, 'archived_at', c.archived_at, 'created_at', c.created_at,
                'moderator_ids', (SELECT COALESCE(jsonb_agg(m.user_id ORDER BY m.added_at), '[]') FROM note_channel_moderators m WHERE m.channel_id = c.id),
                'pinned_note_ids', (SELECT COALESCE(jsonb_agg(p.note_id ORDER BY p.pinned_at DESC), '[]') FROM note_channel_pins p WHERE p.channel_id = c.id))
         FROM note_channels c WHERE c.owner_id = $1 ORDER BY c.id",
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;
    let follows: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('channel_id', channel_id, 'followed_at', followed_at)
         FROM note_channel_follows WHERE user_id = $1 ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;
    Ok((serde_json::Value::Array(owned), serde_json::Value::Array(follows)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_are_hex_triplets() {
        assert_eq!(validate_color("").unwrap(), "");
        assert_eq!(validate_color(" #A1b2C3 ").unwrap(), "#a1b2c3");
        for bad in ["red", "#12345", "#1234567", "123456", "#ggg000"] {
            assert_eq!(validate_color(bad).unwrap_err().reason, "INVALID_COLOR", "{bad}");
        }
    }

    #[test]
    fn names_and_descriptions_are_checked() {
        let input = ChannelInput::validate("  ゲーム   部 ", "", "", "").unwrap();
        assert_eq!(input.name, "ゲーム 部");
        assert_eq!(ChannelInput::validate(" ", "", "", "").unwrap_err().reason, "INVALID_NAME");
        assert_eq!(ChannelInput::validate(&"あ".repeat(51), "", "", "").unwrap_err().reason, "INVALID_NAME");
        assert_eq!(ChannelInput::validate("ok", &"x".repeat(501), "", "").unwrap_err().reason, "DESCRIPTION_TOO_LONG");
    }

    #[test]
    fn name_queries_match_literally() {
        assert_eq!(like_pattern("  "), None);
        assert_eq!(like_pattern("A_b%c\\").unwrap(), "%a\\_b\\%c\\\\%");
    }

    #[test]
    fn channel_conditions_only_use_the_viewer_parameters() {
        let sql = shown("c");
        assert!(sql.contains("$1") && sql.contains("$3") && !sql.contains("$7"));
        assert_eq!(sql.matches('(').count(), sql.matches(')').count());
    }
}
