//! What follows from a short being visible, kept in step with the short.
//!
//! Several things are derived from a short's state, and every change to a
//! short (going live, an edit, a suspension, deletion) ends by bringing
//! them up to date through [`after_change_in`] or [`unlist_in`]:
//!
//! - **the `listed` flag**: the short is public right now (PUBLISHED,
//!   PUBLIC, not suspended, its owner not locked). While it is set, the short is counted in its
//!   sound's `use_count` and in the `remix_count` of the short it remixes;
//! - **hashtag pages**: a public short has one `short_hashtag_uses` row per
//!   hashtag, and each hashtag's `use_count` counts those rows;
//! - **the search document** (`kind: "short"`), present while public;
//! - **the announcement**, made once, the first time the short is published
//!   and PUBLIC or shared with an audience (followers, friends, close
//!   friends, a circle): an activity entry for the owner's friends (the
//!   owner's own for close friends and circles) and
//!   `shorts.short.published` with `first_publication`, from which the
//!   subscriber notifications fan out (see `workers`);
//! - **mention notifications**: each mentioned account is told once, the
//!   first time it can see the short. Who has been told is remembered for
//!   good (`mentions_told`): taking a mention out of the caption and
//!   putting it back does not tell that person again.
//!
//! Both entry points run inside the caller's transaction and never touch
//! the connection pool: a transaction that waits for a second connection
//! is how a few concurrent requests stall everyone. What they need from
//! other domains comes in as a [`Context`], loaded before the transaction.
use tokuly_core::{
    events as core_events,
    outbound::{self, SearchDoc},
    prelude::*,
};
use tokuly_proto::common::SubjectType;
use tokuly_social::graph;
use tokuly_video::channels::{self, ChannelRow};

use std::collections::HashSet;

use tokuly_social::graph::ContentAudience;

use crate::{
    access::{self, FRIENDS, PUBLIC},
    events::{SHORT_PUBLISHED, SHORT_VISIBILITY_CHANGED, ShortVisibilityChanged},
    shorts::{self, ShortRow, Tx},
    text,
};

/// What a change to a short needs to know from other domains: the channel
/// it is on, whether the owner is locked, who is in the owner's audiences
/// and who is in a block with them (who may be told about it). Loaded
/// before the transaction of the change begins.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// `None` when the channel is gone already (its shorts are deleted
    /// right after it): nothing is published in its name then.
    pub channel: Option<ChannelRow>,
    /// The owner's account is locked, as the social domain says now.
    pub locked: bool,
    /// The owner's friends.
    pub friends: HashSet<Uuid>,
    /// The owner's followers and friends.
    pub tied: HashSet<Uuid>,
    /// The owner's close friends (still tied to them).
    pub close_friends: HashSet<Uuid>,
    /// The members of the circle the short is (about to be) shared with.
    pub circle: Option<(Uuid, HashSet<Uuid>)>,
    /// Users in a block with the owner, either way.
    pub hidden: Vec<Uuid>,
}

impl Context {
    pub async fn load(core: &Core, short: &ShortRow) -> AppResult<Self> {
        Self::of(core, short.channel_id, short.owner_id, short.circle_id).await
    }

    /// `circle_id`: the circle the short is or will be shared with, if any.
    pub async fn of(core: &Core, channel_id: Uuid, owner_id: Uuid, circle_id: Option<Uuid>) -> AppResult<Self> {
        let channel = channels::find_by_id(core, channel_id).await?;
        let (friends, tied, close_friends, hidden, locked) = tokio::try_join!(
            graph::friend_ids(core, owner_id),
            graph::audience_member_ids(core, owner_id, ContentAudience::Followers),
            graph::audience_member_ids(core, owner_id, ContentAudience::CloseFriends),
            graph::blocked_ids_either(core, owner_id),
            tokuly_video::access::is_locked_now(core, owner_id),
        )?;
        let circle = match circle_id {
            Some(circle_id) => Some((
                circle_id,
                graph::audience_member_ids(core, owner_id, ContentAudience::Circle(circle_id))
                    .await?
                    .into_iter()
                    .collect(),
            )),
            None => None,
        };
        Ok(Self {
            channel,
            locked,
            friends: friends.into_iter().collect(),
            tied: tied.into_iter().collect(),
            close_friends: close_friends.into_iter().collect(),
            circle,
            hidden,
        })
    }

    /// Who besides the owner is in `audience` (`None`: everyone), people in
    /// a block with the owner left out. Sorted.
    pub fn members(&self, audience: ContentAudience) -> Option<Vec<Uuid>> {
        let members: Vec<Uuid> = match audience {
            ContentAudience::Public => return None,
            ContentAudience::Followers => self.tied.iter().copied().collect(),
            ContentAudience::Friends => self.friends.iter().copied().collect(),
            ContentAudience::CloseFriends => self.close_friends.iter().copied().collect(),
            ContentAudience::Circle(id) => match &self.circle {
                Some((circle_id, members)) if *circle_id == id => members.iter().copied().collect(),
                _ => Vec::new(),
            },
            ContentAudience::Private => Vec::new(),
        };
        let mut members: Vec<Uuid> = members.into_iter().filter(|id| !self.hidden.contains(id)).collect();
        members.sort_unstable();
        Some(members)
    }

    /// Whether `user_id` (not the owner) may open `row` as it is now.
    pub fn reaches(&self, row: &ShortRow, user_id: Uuid) -> bool {
        if row.status != access::PUBLISHED || row.suspended || self.hidden.contains(&user_id) {
            return false;
        }
        match self.members(row.audience(self.locked)) {
            None => true,
            Some(members) => members.binary_search(&user_id).is_ok(),
        }
    }
}

/// Emits `shorts.short.visibility_changed` for a published short whose
/// audience went from `before` to `after` (nothing when it did not).
pub(crate) async fn announce_audience_in(
    tx: &mut Tx<'_>,
    before: &ShortRow,
    after: &ShortRow,
    context: &Context,
) -> AppResult<()> {
    let (was, now) = (before.audience(before.owner_locked), after.audience(after.owner_locked));
    if !(before.is_published() && after.is_published() && was != now) {
        return Ok(());
    }
    let visible_to = context.members(now).map(|members| tokuly_core::events::bounded_audience(members, None));
    emit_visibility_changed_in(tx, after, visible_to).await
}

/// Emits `shorts.short.visibility_changed` for `row` as it is now.
pub(crate) async fn emit_visibility_changed_in(
    tx: &mut Tx<'_>,
    row: &ShortRow,
    visible_to: Option<Vec<Uuid>>,
) -> AppResult<()> {
    let changed_at: Time = sqlx::query_scalar("SELECT clock_timestamp()").fetch_one(&mut **tx).await?;
    let mut short = row.event(false, "");
    short.visibility = row.effective_visibility();
    let payload = ShortVisibilityChanged { short, visible_to, changed_at };
    core_events::emit(&mut **tx, SHORT_VISIBILITY_CHANGED, Some(row.owner_id), &payload).await
}

/// Brings everything derived from a short up to date after a change to it.
/// `went_live` says that this change published the short. Returns the row
/// as it is now.
pub(crate) async fn after_change_in(
    tx: &mut Tx<'_>,
    short_id: Uuid,
    went_live: bool,
    context: &Context,
) -> AppResult<ShortRow> {
    let mut row = shorts::find_in(tx, short_id).await?.ok_or_else(shorts::not_found)?;
    let was_listed = row.listed;
    let public = row.is_public();
    sync_counts_in(tx, &mut row, public).await?;
    let hashtags = if public { row.hashtags.clone() } else { Vec::new() };
    sync_hashtags_in(tx, &row, &hashtags).await?;

    let channel = context.channel.as_ref();
    if (public || was_listed)
        && let Some(channel) = channel
    {
        outbound::index(&mut **tx, search_doc(&row, channel)).await?;
    } else if was_listed {
        outbound::unindex(&mut **tx, "short", row.id).await?;
    }

    let announce = row.is_published()
        && !row.suspended
        && row.announced_at.is_none()
        && matches!(row.visibility, PUBLIC | FRIENDS | access::FOLLOWERS | access::CLOSE_FRIENDS | access::CIRCLE)
        && channel.is_some();
    if announce && let Some(channel) = channel {
        let at: Time =
            sqlx::query_scalar("UPDATE shorts SET announced_at = now() WHERE id = $1 RETURNING announced_at")
                .bind(row.id)
                .fetch_one(&mut **tx)
                .await?;
        row.announced_at = Some(at);
        outbound::activity(&mut **tx, &activity_entry(&row, channel)).await?;
    }
    if went_live || announce {
        core_events::emit(&mut **tx, SHORT_PUBLISHED, Some(row.owner_id), &row.event(announce, "")).await?;
    }
    if let Some(channel) = channel {
        notify_mentions_in(tx, &mut row, channel, context).await?;
    }
    Ok(row)
}

/// Takes a short that is about to be deleted out of everything it is
/// counted and listed in.
pub(crate) async fn unlist_in(tx: &mut Tx<'_>, short: &ShortRow) -> AppResult<()> {
    let mut row = short.clone();
    let was_listed = row.listed;
    sync_counts_in(tx, &mut row, false).await?;
    sync_hashtags_in(tx, &row, &[]).await?;
    if was_listed {
        outbound::unindex(&mut **tx, "short", row.id).await?;
    }
    Ok(())
}

/// Moves the `listed` flag to `public`, and with it the counters that count
/// public shorts: the sound's uses and the original's remixes.
async fn sync_counts_in(tx: &mut Tx<'_>, row: &mut ShortRow, public: bool) -> AppResult<()> {
    if row.listed == public {
        return Ok(());
    }
    let delta: i64 = if public { 1 } else { -1 };
    sqlx::query("UPDATE shorts SET listed = $2 WHERE id = $1").bind(row.id).bind(public).execute(&mut **tx).await?;
    if let Some(sound_id) = row.sound_id {
        sqlx::query("UPDATE short_sounds SET use_count = GREATEST(use_count + $2, 0) WHERE id = $1")
            .bind(sound_id)
            .bind(delta)
            .execute(&mut **tx)
            .await?;
    }
    if let Some(original_id) = row.remix_of_id {
        sqlx::query("UPDATE shorts SET remix_count = GREATEST(remix_count + $2, 0) WHERE id = $1")
            .bind(original_id)
            .bind(delta)
            .execute(&mut **tx)
            .await?;
    }
    row.listed = public;
    Ok(())
}

/// Makes the short's rows on hashtag pages exactly `hashtags`, moving each
/// hashtag's `use_count` with them.
async fn sync_hashtags_in(tx: &mut Tx<'_>, row: &ShortRow, hashtags: &[String]) -> AppResult<()> {
    let current: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT h.id, h.tag FROM short_hashtag_uses u JOIN short_hashtags h ON h.id = u.hashtag_id WHERE u.short_id = $1",
    )
    .bind(row.id)
    .fetch_all(&mut **tx)
    .await?;
    let mut added: Vec<&String> = hashtags.iter().filter(|tag| !current.iter().any(|(_, have)| have == *tag)).collect();
    added.sort();
    let gone: Vec<&(Uuid, String)> = current.iter().filter(|(_, have)| !hashtags.contains(have)).collect();
    if added.is_empty() && gone.is_empty() {
        return Ok(());
    }
    // Every hashtag this touches is locked in one pass, in tag order (new
    // ones are created in that order too): the order other shorts and
    // `hashtags::refresh_trending` lock them in, so they wait for each
    // other instead of deadlocking.
    let touched: Vec<&str> =
        added.iter().map(|tag| tag.as_str()).chain(gone.iter().map(|(_, tag)| tag.as_str())).collect();
    sqlx::query("SELECT 1 FROM short_hashtags WHERE tag = ANY($1) ORDER BY tag FOR UPDATE")
        .bind(&touched)
        .execute(&mut **tx)
        .await?;
    let removed: Vec<Uuid> = gone.iter().map(|(id, _)| *id).collect();
    if !added.is_empty() {
        sqlx::query(
            "WITH tags AS (
                 INSERT INTO short_hashtags (tag, use_count)
                 SELECT tag, 1 FROM unnest($2::text[]) AS tag ORDER BY tag
                 ON CONFLICT (tag) DO UPDATE SET use_count = short_hashtags.use_count + 1
                 RETURNING id
             )
             INSERT INTO short_hashtag_uses (hashtag_id, short_id, published_at) SELECT id, $1, $3 FROM tags",
        )
        .bind(row.id)
        .bind(&added)
        .bind(row.published_at.unwrap_or(row.created_at))
        .execute(&mut **tx)
        .await?;
    }
    if !removed.is_empty() {
        sqlx::query("DELETE FROM short_hashtag_uses WHERE short_id = $1 AND hashtag_id = ANY($2)")
            .bind(row.id)
            .bind(&removed)
            .execute(&mut **tx)
            .await?;
        sqlx::query("UPDATE short_hashtags SET use_count = GREATEST(use_count - 1, 0) WHERE id = ANY($1)")
            .bind(&removed)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

pub(crate) fn search_doc(row: &ShortRow, channel: &ChannelRow) -> SearchDoc {
    let title = text::excerpt(&row.caption, 80);
    SearchDoc {
        kind: "short".to_owned(),
        id: row.id,
        // A short without a caption is found by its channel.
        title: if title.is_empty() { channel.name.clone() } else { title },
        text: row.caption.clone(),
        tags: row.hashtags.clone(),
        owner_id: Some(row.owner_id),
        public: row.is_public(),
        image_url: row.cover_url(),
        path: row.path(),
        created_at: row.published_at.unwrap_or(row.created_at).timestamp(),
        score: row.view_count as f64,
        extra: serde_json::json!({
            "short_id": row.short_id,
            "channel_id": channel.id,
            "channel_handle": channel.handle,
            "channel_name": channel.name,
            "duration_seconds": row.duration_seconds,
        }),
    }
}

/// What every notification and activity entry about a short carries.
pub(crate) fn short_data(row: &ShortRow, channel: &ChannelRow) -> serde_json::Value {
    serde_json::json!({
        "id": row.id,
        "short_id": row.short_id,
        "channel_id": channel.id,
        "channel_handle": channel.handle,
        "channel_name": channel.name,
        "channel_icon_url": channel.icon_url(),
        "duration_seconds": row.duration_seconds,
    })
}

/// The activity entry for a newly announced short. It goes to the owner's
/// friends only, also for public shorts: a channel does not reveal which
/// account is behind it to strangers.
fn activity_entry(row: &ShortRow, channel: &ChannelRow) -> outbound::ActivityEntry {
    let caption = text::excerpt(&row.caption, 60);
    let summary = if caption.is_empty() {
        outbound::Localized::new("ショートを投稿したよ!", "posted a short")
    } else {
        outbound::Localized::new(format!("ショートを投稿したよ!「{caption}」"), format!("posted a short: {caption}"))
    };
    // Friends are inside the public, followers and friends audiences; the
    // feed cannot tell who is in a circle, so those entries are the owner's.
    let audience = if matches!(row.visibility, access::CLOSE_FRIENDS | access::CIRCLE) {
        outbound::ActivityAudience::Private
    } else {
        outbound::ActivityAudience::Friends
    };
    let mut entry = outbound::ActivityEntry::new(row.owner_id, "short.published", audience, summary)
        .subject(SubjectType::SUBJECT_TYPE_SHORT, row.id)
        .path(row.path())
        .data(short_data(row, channel));
    if let Some(url) = row.cover_url() {
        entry = entry.image(url);
    }
    entry
}

/// Tells the accounts mentioned in the caption, each once, as soon as the
/// short is something they can open. The notification speaks for the
/// channel: it names no account.
async fn notify_mentions_in(
    tx: &mut Tx<'_>,
    row: &mut ShortRow,
    channel: &ChannelRow,
    context: &Context,
) -> AppResult<()> {
    if !row.is_published() || row.suspended {
        return Ok(());
    }
    let mut told: Vec<Uuid> = Vec::new();
    for mention in row.mentions() {
        let recipient = mention.user_id;
        let already = row.mentions_told.contains(&recipient) || told.contains(&recipient);
        // A block made after the caption was written still counts.
        let barred = recipient == row.owner_id || context.hidden.contains(&recipient);
        if already || barred || !context.reaches(row, recipient) {
            continue;
        }
        let caption = text::excerpt(&row.caption, 60);
        let mut request = outbound::NotificationRequest::new(
            recipient,
            "shorts.mention",
            outbound::Localized::same(channel.name.clone()),
            outbound::Localized::new(
                format!("ショートであなたをメンションしたよ!「{caption}」"),
                format!("mentioned you in a short: {caption}"),
            ),
            row.path(),
        )
        .subject(SubjectType::SUBJECT_TYPE_SHORT, row.id)
        .collapse(format!("shorts.mention:{}:{recipient}", row.id))
        .data(short_data(row, channel));
        if let Some(url) = row.cover_url() {
            request = request.image(url);
        }
        outbound::notify(&mut **tx, &request).await?;
        told.push(recipient);
    }
    if !told.is_empty() {
        sqlx::query("UPDATE shorts SET mentions_told = mentions_told || $2 WHERE id = $1")
            .bind(row.id)
            .bind(&told)
            .execute(&mut **tx)
            .await?;
        row.mentions_told.extend(told);
    }
    Ok(())
}
