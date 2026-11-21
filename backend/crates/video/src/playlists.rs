//! Playlists, including each account's two built-in lists: watch later
//! (`WL`) and liked videos (`LL`, maintained by video ratings).
//!
//! Order within a list is `playlist_items.position`, ascending. Custom lists
//! append at `max + 1`; built-in lists store the negated insertion time, so
//! the same ordering shows them newest first.
use std::collections::HashMap;

use tokuly_core::{events as core_events, ids::Ref, outbound, prelude::*};
use tokuly_media::assets::image_from_json;
use tokuly_proto::{
    common::Visibility,
    video::{Playlist, PlaylistKind},
};

use crate::{
    access::{self, Viewer, watchable},
    channels,
    events::{PLAYLIST_DELETED, PlaylistEvent},
    search,
};

pub const KIND_CUSTOM: i32 = PlaylistKind::PLAYLIST_KIND_CUSTOM as i32;
pub const KIND_WATCH_LATER: i32 = PlaylistKind::PLAYLIST_KIND_WATCH_LATER as i32;
pub const KIND_LIKED: i32 = PlaylistKind::PLAYLIST_KIND_LIKED as i32;

/// What clients send as `playlist_id` for the caller's built-in lists.
pub const WATCH_LATER_ALIAS: &str = "WL";
pub const LIKED_ALIAS: &str = "LL";

type Tx<'a> = sqlx::Transaction<'a, sqlx::Postgres>;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PlaylistRow {
    pub id: Uuid,
    pub short_id: String,
    pub owner_id: Uuid,
    /// The channel whose page shows the playlist; `None` for personal lists.
    pub channel_id: Option<Uuid>,
    pub kind: i32,
    pub title: String,
    pub description: String,
    pub visibility: i32,
    pub item_count: i32,
    pub like_count: i64,
    /// The owner's account is suspended.
    pub suspended: bool,
    pub created_at: Time,
    pub updated_at: Time,
}

macro_rules! playlist_columns {
    () => {
        "p.id, p.short_id, p.owner_id, p.channel_id, p.kind, p.title, p.description, p.visibility, p.item_count, \
         p.like_count, COALESCE((SELECT u.banned_at IS NOT NULL FROM users u WHERE u.id = p.owner_id), false) AS suspended, \
         p.created_at, p.updated_at"
    };
}
pub(crate) use playlist_columns;

impl PlaylistRow {
    pub fn is_builtin(&self) -> bool {
        self.kind != KIND_CUSTOM
    }

    /// Listed for everyone: on the channel page and in search.
    pub fn is_public(&self) -> bool {
        self.kind == KIND_CUSTOM && self.visibility == access::PUBLIC && !self.suspended
    }

    /// Path of the playlist page in the web app.
    pub fn path(&self) -> String {
        format!("/playlist/{}", self.short_id)
    }

    /// Whether `viewer` may open the playlist: the owner always; everyone
    /// else when it is public or unlisted and its owner is not hidden from them.
    pub fn visible_to(&self, viewer: &Viewer) -> bool {
        viewer.is(self.owner_id)
            || (self.kind == KIND_CUSTOM
                && matches!(self.visibility, access::PUBLIC | access::UNLISTED)
                && viewer.may_see_owner(self.owner_id, self.suspended))
    }
}

fn not_found() -> AppError {
    AppError::not_found("PLAYLIST_NOT_FOUND", "playlist not found")
}

pub async fn find_by_id(core: &Core, playlist_id: Uuid) -> AppResult<Option<PlaylistRow>> {
    Ok(sqlx::query_as(concat!("SELECT ", playlist_columns!(), " FROM playlists p WHERE p.id = $1"))
        .bind(playlist_id)
        .fetch_optional(&core.db)
        .await?)
}

pub(crate) async fn find_in(tx: &mut Tx<'_>, playlist_id: Uuid) -> AppResult<Option<PlaylistRow>> {
    Ok(sqlx::query_as(concat!("SELECT ", playlist_columns!(), " FROM playlists p WHERE p.id = $1 FOR UPDATE OF p"))
        .bind(playlist_id)
        .fetch_optional(&mut **tx)
        .await?)
}

/// A custom playlist by UUID or short ID, whoever may see it.
pub async fn find(core: &Core, reference: &Ref) -> AppResult<Option<PlaylistRow>> {
    Ok(match reference {
        Ref::Id(id) => find_by_id(core, *id).await?,
        Ref::Short(short_id) => {
            sqlx::query_as(concat!("SELECT ", playlist_columns!(), " FROM playlists p WHERE p.short_id = $1"))
                .bind(short_id)
                .fetch_optional(&core.db)
                .await?
        }
    })
}

/// The built-in list of `kind` of a user, created on first use.
pub(crate) async fn builtin_in(tx: &mut Tx<'_>, user_id: Uuid, kind: i32) -> AppResult<Uuid> {
    debug_assert!(kind == KIND_WATCH_LATER || kind == KIND_LIKED);
    // The no-op update makes the statement return the row either way.
    Ok(sqlx::query_scalar(
        "INSERT INTO playlists (short_id, owner_id, kind, visibility) VALUES ($1, $2, $3, 5)
         ON CONFLICT (owner_id, kind) WHERE kind <> 1 DO UPDATE SET kind = EXCLUDED.kind
         RETURNING id",
    )
    .bind(tokuly_core::ids::new_short_id())
    .bind(user_id)
    .bind(kind)
    .fetch_one(&mut **tx)
    .await?)
}

async fn builtin(core: &Core, user_id: Uuid, kind: i32) -> AppResult<PlaylistRow> {
    let mut tx = core.db.begin().await?;
    let playlist_id = builtin_in(&mut tx, user_id, kind).await?;
    tx.commit().await?;
    find_by_id(core, playlist_id).await?.ok_or_else(not_found)
}

/// The playlist a client named — `WL`, `LL`, a UUID or a short ID — if
/// `viewer` may open it (`PLAYLIST_NOT_FOUND` otherwise).
pub async fn resolve(core: &Core, viewer: &Viewer, reference: &str) -> AppResult<PlaylistRow> {
    let reference = reference.trim();
    let alias = match reference {
        WATCH_LATER_ALIAS => Some(KIND_WATCH_LATER),
        LIKED_ALIAS => Some(KIND_LIKED),
        _ => None,
    };
    if let Some(kind) = alias {
        let user_id = viewer.user_id.ok_or_else(|| AppError::unauthenticated("SIGN_IN_REQUIRED", "sign in required"))?;
        return builtin(core, user_id, kind).await;
    }
    let Ok(reference) = Ref::parse(reference) else { return Err(not_found()) };
    find(core, &reference).await?.filter(|playlist| playlist.visible_to(viewer)).ok_or_else(not_found)
}

/// The playlist a client named, which must belong to `user_id`.
pub async fn resolve_owned(core: &Core, user_id: Uuid, reference: &str) -> AppResult<PlaylistRow> {
    let viewer = Viewer { user_id: Some(user_id), ..Default::default() };
    resolve(core, &viewer, reference).await.and_then(|playlist| if playlist.owner_id == user_id { Ok(playlist) } else { Err(not_found()) })
}

/// Adds a video to a list (at the end of a custom list, at the top of a
/// built-in one). Returns whether it was added (`false`: already there).
pub(crate) async fn add_item_in(tx: &mut Tx<'_>, playlist_id: Uuid, video_id: Uuid) -> AppResult<bool> {
    let added = sqlx::query(
        "INSERT INTO playlist_items (playlist_id, video_id, position)
         SELECT p.id, $2,
                CASE WHEN p.kind = 1
                     THEN COALESCE((SELECT max(position) FROM playlist_items WHERE playlist_id = p.id), 0) + 1
                     ELSE -(extract(epoch FROM clock_timestamp()) * 1000000)::bigint END
         FROM playlists p WHERE p.id = $1
         ON CONFLICT (playlist_id, video_id) DO NOTHING",
    )
    .bind(playlist_id)
    .bind(video_id)
    .execute(&mut **tx)
    .await?;
    Ok(added.rows_affected() == 1)
}

pub(crate) async fn remove_item_in(tx: &mut Tx<'_>, playlist_id: Uuid, video_id: Uuid) -> AppResult<bool> {
    let removed = sqlx::query("DELETE FROM playlist_items WHERE playlist_id = $1 AND video_id = $2")
        .bind(playlist_id)
        .bind(video_id)
        .execute(&mut **tx)
        .await?;
    Ok(removed.rows_affected() == 1)
}

/// Moves `video_id` to right after `after` (`None`: to the front).
/// Returns whether both items were found.
pub(crate) async fn move_item_in(tx: &mut Tx<'_>, playlist_id: Uuid, video_id: Uuid, after: Option<Uuid>) -> AppResult<bool> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM playlist_items WHERE playlist_id = $1 AND video_id = $2)")
        .bind(playlist_id)
        .bind(video_id)
        .fetch_one(&mut **tx)
        .await?;
    if !exists {
        return Ok(false);
    }
    let position: i64 = match after {
        None => {
            let first: Option<i64> = sqlx::query_scalar("SELECT min(position) FROM playlist_items WHERE playlist_id = $1")
                .bind(playlist_id)
                .fetch_one(&mut **tx)
                .await?;
            first.unwrap_or(1) - 1
        }
        Some(after) if after == video_id => return Ok(true),
        Some(after) => {
            let anchor: Option<i64> = sqlx::query_scalar("SELECT position FROM playlist_items WHERE playlist_id = $1 AND video_id = $2")
                .bind(playlist_id)
                .bind(after)
                .fetch_optional(&mut **tx)
                .await?;
            let Some(anchor) = anchor else { return Ok(false) };
            // Open a gap after the anchor. Items sharing the anchor's position
            // (never produced here, but allowed by the schema) move with the rest.
            sqlx::query("UPDATE playlist_items SET position = position + 1 WHERE playlist_id = $1 AND position > $2 AND video_id <> $3")
                .bind(playlist_id)
                .bind(anchor)
                .bind(video_id)
                .execute(&mut **tx)
                .await?;
            anchor + 1
        }
    };
    sqlx::query("UPDATE playlist_items SET position = $3 WHERE playlist_id = $1 AND video_id = $2")
        .bind(playlist_id)
        .bind(video_id)
        .bind(position)
        .execute(&mut **tx)
        .await?;
    sqlx::query("UPDATE playlists SET updated_at = now() WHERE id = $1").bind(playlist_id).execute(&mut **tx).await?;
    Ok(true)
}

/// Thumbnail URL of the first public video in a playlist (for its search document).
pub(crate) async fn public_thumbnail_in(tx: &mut Tx<'_>, playlist_id: Uuid) -> AppResult<Option<String>> {
    let thumbnail: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT v.thumbnail FROM playlist_items i JOIN videos v ON v.id = i.video_id
         WHERE i.playlist_id = $1 AND v.status = 3 AND v.visibility = 1 AND NOT v.suspended AND v.thumbnail IS NOT NULL
         ORDER BY i.position, i.video_id LIMIT 1",
    )
    .bind(playlist_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(image_from_json(thumbnail.as_ref()).map(|image| image.url).filter(|url| !url.is_empty()))
}

/// Refreshes the search document of a playlist after a change.
pub(crate) async fn sync_search_in(tx: &mut Tx<'_>, was_public: bool, playlist_id: Uuid) -> AppResult<()> {
    let Some(playlist) = find_in(tx, playlist_id).await? else { return Ok(()) };
    if playlist.is_public() || was_public {
        let image = public_thumbnail_in(tx, playlist.id).await?;
        search::sync_playlist(tx, was_public, &playlist, image).await?;
    }
    Ok(())
}

/// Deletes a playlist inside `tx`.
pub(crate) async fn delete_in(tx: &mut Tx<'_>, playlist_id: Uuid, owner_id: Uuid, actor: Option<Uuid>) -> AppResult<()> {
    let deleted = sqlx::query("DELETE FROM playlists WHERE id = $1").bind(playlist_id).execute(&mut **tx).await?;
    if deleted.rows_affected() == 1 {
        outbound::unindex(&mut **tx, "playlist", playlist_id).await?;
        core_events::emit(&mut **tx, PLAYLIST_DELETED, actor, &PlaylistEvent { playlist_id, owner_id }).await?;
    }
    Ok(())
}

/// Playlists as `viewer` sees them, in order. `contains_video` fills
/// `Playlist.contains_video` for that video.
pub async fn to_protos(core: &Core, viewer: &Viewer, rows: &[PlaylistRow], contains_video: Option<Uuid>) -> AppResult<Vec<Playlist>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    // The thumbnail is that of the first video the viewer may watch.
    let thumbnails: Vec<(Uuid, Option<serde_json::Value>)> = sqlx::query_as(concat!(
        "SELECT p.id, t.thumbnail FROM unnest($1::uuid[]) AS p(id)
         LEFT JOIN LATERAL (
             SELECT v.thumbnail FROM playlist_items i JOIN videos v ON v.id = i.video_id
             WHERE i.playlist_id = p.id AND v.thumbnail IS NOT NULL AND ",
        watchable!(2, 3, 4),
        " ORDER BY i.position, i.video_id LIMIT 1
         ) t ON true"
    ))
    .bind(&ids)
    .bind(viewer.user_id)
    .bind(&viewer.friends)
    .bind(&viewer.blocked)
    .fetch_all(&core.db)
    .await?;
    let thumbnails: HashMap<Uuid, Option<serde_json::Value>> = thumbnails.into_iter().collect();
    let containing: Vec<Uuid> = match contains_video {
        Some(video_id) => {
            sqlx::query_scalar("SELECT playlist_id FROM playlist_items WHERE video_id = $1 AND playlist_id = ANY($2)")
                .bind(video_id)
                .bind(&ids)
                .fetch_all(&core.db)
                .await?
        }
        None => Vec::new(),
    };
    let channel_ids: Vec<Uuid> = rows.iter().filter_map(|row| row.channel_id).collect();
    let channel_summaries = channels::summaries(core, &channel_ids).await?;
    // Channels do not reveal their owner: only personal playlists (and the
    // viewer's own) name the account.
    let owner_ids: Vec<Uuid> = rows.iter().filter(|row| row.channel_id.is_none() || viewer.is(row.owner_id)).map(|row| row.owner_id).collect();
    let owners = tokuly_identity::summaries(core, &owner_ids).await?;
    Ok(rows
        .iter()
        .map(|row| Playlist {
            id: row.id.to_string(),
            short_id: row.short_id.clone(),
            kind: row.kind.into(),
            title: row.title.clone(),
            description: row.description.clone(),
            visibility: row.visibility.into(),
            item_count: row.item_count,
            thumbnail: image_from_json(thumbnails.get(&row.id).and_then(|thumbnail| thumbnail.as_ref())).into(),
            owner: owners.get(&row.owner_id).cloned().into(),
            channel: row.channel_id.and_then(|id| channel_summaries.get(&id).cloned()).into(),
            like_count: row.like_count,
            created_at: ts(row.created_at),
            updated_at: ts(row.updated_at),
            is_owner: viewer.is(row.owner_id),
            contains_video: containing.contains(&row.id),
            ..Default::default()
        })
        .collect())
}

pub async fn to_proto(core: &Core, viewer: &Viewer, row: &PlaylistRow) -> AppResult<Playlist> {
    Ok(to_protos(core, viewer, std::slice::from_ref(row), None).await?.pop().unwrap_or_default())
}

/// The stored visibility of a built-in list.
pub const BUILTIN_VISIBILITY: i32 = Visibility::VISIBILITY_PRIVATE as i32;

#[cfg(test)]
mod tests {
    use super::*;

    fn playlist(owner_id: Uuid, kind: i32, visibility: i32) -> PlaylistRow {
        PlaylistRow {
            id: Uuid::now_v7(),
            short_id: "abcdefghijk".into(),
            owner_id,
            channel_id: None,
            kind,
            title: "t".into(),
            description: String::new(),
            visibility,
            item_count: 0,
            like_count: 0,
            suspended: false,
            created_at: now(),
            updated_at: now(),
        }
    }

    #[test]
    fn playlists_open_for_their_owner_and_by_visibility() {
        let owner = Uuid::now_v7();
        let me = Viewer { user_id: Some(owner), ..Default::default() };
        let stranger = Viewer { user_id: Some(Uuid::now_v7()), ..Default::default() };
        let blocked = Viewer { user_id: Some(Uuid::now_v7()), blocked: vec![owner], ..Default::default() };
        for (visibility, open) in [(access::PUBLIC, true), (access::UNLISTED, true), (access::PRIVATE, false)] {
            let list = playlist(owner, KIND_CUSTOM, visibility);
            assert!(list.visible_to(&me));
            assert_eq!(list.visible_to(&stranger), open);
            assert_eq!(list.visible_to(&Viewer::anonymous()), open);
            assert!(!list.visible_to(&blocked));
        }
        assert!(playlist(owner, KIND_CUSTOM, access::PUBLIC).is_public());
        assert!(!playlist(owner, KIND_CUSTOM, access::UNLISTED).is_public());
        // Built-in lists are the owner's alone.
        let liked = playlist(owner, KIND_LIKED, BUILTIN_VISIBILITY);
        assert!(liked.is_builtin() && liked.visible_to(&me) && !liked.visible_to(&stranger));
        let mut suspended = playlist(owner, KIND_CUSTOM, access::PUBLIC);
        suspended.suspended = true;
        assert!(!suspended.visible_to(&stranger) && suspended.visible_to(&me) && !suspended.is_public());
    }
}
