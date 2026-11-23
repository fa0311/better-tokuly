//! Finding rooms: live among friends, public rooms by listeners, upcoming
//! rooms, the caller's own, popular tags, and which friends are inside
//! something the caller could enter.
//!
//! Nothing here shows a room its viewer may not enter: rooms of hosts on
//! either side of a block and rooms the viewer is banned from are left
//! out, friends rooms need the friendship, invited-only rooms the
//! invitation.
use std::collections::HashMap;

use tokuly_core::prelude::*;
use tokuly_identity::settings;
use tokuly_proto::{common::PageInfo, spaces as pb};
use tokuly_social::graph;

use crate::{
    Spaces,
    model::{SPACE, SpaceRow, Venue, audience, member, role, state},
    render::Detail,
    rules,
};

/// Most tags [`Spaces::popular_tags`] returns.
const MAX_POPULAR_TAGS: i64 = 50;
const DEFAULT_POPULAR_TAGS: i64 = 20;

/// A friend who is inside a room or hangout the viewer could enter.
#[derive(Debug, Clone, PartialEq)]
pub struct FriendInRoom {
    pub user_id: Uuid,
    pub venue: Venue,
    /// The room's title or the hangout's name.
    pub title: String,
    /// The hangout's emoji; empty for rooms.
    pub emoji: String,
    /// `tokuly.spaces.v1.SpaceRole` as its number; 0 for hangouts.
    pub role: i16,
    pub since: Time,
}

/// The ranked position a page starts at. Rankings move while people page
/// through them, so pages are cut by position, not by key.
fn offset_of(page: &Page) -> AppResult<i64> {
    Ok(page.decode::<i64>()?.unwrap_or(0).max(0))
}

impl Spaces {
    /// Live rooms `viewer` may enter that have friends of theirs inside,
    /// most listeners first, with those friends per room.
    ///
    /// A friend counts when they are on stage (which everyone in the room
    /// sees anyway) or share their online status.
    async fn rooms_with_friends(
        &self,
        viewer: Uuid,
    ) -> AppResult<(Vec<SpaceRow>, HashMap<Uuid, Vec<(Uuid, i16, Time)>>)> {
        let core = &self.core;
        let friends = graph::friend_ids(core, viewer).await?;
        if friends.is_empty() {
            return Ok((Vec::new(), HashMap::new()));
        }
        let inside: Vec<(Uuid, Uuid, i16, Time, Option<Time>)> = sqlx::query_as(
            "SELECT p.space_id, p.user_id, p.role, p.joined_at, p.stage_since FROM space_participants p
             WHERE p.user_id = ANY($1) AND p.left_at IS NULL
             ORDER BY p.role DESC, p.user_id",
        )
        .bind(&friends)
        .fetch_all(&core.db)
        .await?;
        if inside.is_empty() {
            return Ok((Vec::new(), HashMap::new()));
        }
        let everyone: Vec<Uuid> = inside.iter().map(|(_, user, _, _, _)| *user).collect();
        let privacy = settings::privacy_of_many(core, &everyone).await?;
        let mut by_room: HashMap<Uuid, Vec<(Uuid, i16, Time)>> = HashMap::new();
        for (space_id, friend, role, joined_at, stage_since) in inside {
            let open = privacy.get(&friend).is_none_or(|privacy| privacy.share_online_status);
            // Someone who hides their presence shows from when they came on
            // stage, not from when they quietly entered.
            let since = match (open, rules::on_stage(role)) {
                (true, _) => Some(joined_at),
                (false, true) => Some(stage_since.unwrap_or(joined_at)),
                (false, false) => None,
            };
            if let Some(since) = since {
                by_room.entry(space_id).or_default().push((friend, role, since));
            }
        }
        let candidates: Vec<Uuid> = by_room.keys().copied().collect();
        let hidden = graph::blocked_ids_either(core, viewer).await?;
        let rooms: Vec<SpaceRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {SPACE} FROM spaces s
             WHERE s.id = ANY($1) AND s.state = $2 AND s.host_id <> ALL($3)
               AND (s.audience = $4 OR s.host_id = $5
                    OR (s.audience = $6 AND s.host_id = ANY($7))
                    OR EXISTS (SELECT 1 FROM space_invites i
                               WHERE i.space_id = s.id AND i.user_id = $5 AND i.revoked_at IS NULL)
                    OR EXISTS (SELECT 1 FROM space_participants me
                               WHERE me.space_id = s.id AND me.user_id = $5 AND me.left_at IS NULL))
               AND NOT EXISTS (SELECT 1 FROM space_bans b WHERE b.space_id = s.id AND b.user_id = $5)
             ORDER BY s.listener_count DESC, s.id DESC"
        )))
        .bind(&candidates)
        .bind(state::LIVE)
        .bind(&hidden)
        .bind(audience::PUBLIC)
        .bind(viewer)
        .bind(audience::FRIENDS)
        .bind(&friends)
        .fetch_all(&core.db)
        .await?;
        by_room.retain(|space_id, _| rooms.iter().any(|room| room.id == *space_id));
        Ok((rooms, by_room))
    }

    /// Rooms that are live right now: among friends, or public by listeners.
    pub(crate) async fn list_live(
        &self,
        viewer: Option<Uuid>,
        scope: Option<pb::LiveScope>,
        tag: &str,
        language: &str,
        page: &Page,
    ) -> AppResult<(Vec<pb::Space>, PageInfo)> {
        let offset = offset_of(page)?;
        match scope {
            Some(pb::LiveScope::LIVE_SCOPE_FRIENDS) => {
                let Some(viewer) = viewer else {
                    return Err(AppError::unauthenticated("SIGN_IN_REQUIRED", "sign in required"));
                };
                let (rooms, friends) = self.rooms_with_friends(viewer).await?;
                let mut rows: Vec<SpaceRow> =
                    rooms.into_iter().skip(offset as usize).take(page.fetch() as usize).collect();
                let info = page.finish(&mut rows, |_| offset + page.limit);
                let friends_inside: HashMap<Uuid, Vec<Uuid>> = friends
                    .into_iter()
                    .map(|(space_id, inside)| (space_id, inside.into_iter().map(|(friend, _, _)| friend).collect()))
                    .collect();
                let detail = Detail { full_stage: false, friends_inside: Some(&friends_inside) };
                Ok((self.render_spaces(&rows, Some(viewer), detail).await?, info))
            }
            Some(pb::LiveScope::LIVE_SCOPE_PUBLIC) => {
                let core = &self.core;
                let tag = rules::tag(tag);
                let language = Some(rules::language(language)?).filter(|language| !language.is_empty());
                let hidden = match viewer {
                    Some(viewer) => graph::blocked_ids_either(core, viewer).await?,
                    None => Vec::new(),
                };
                let mut rows: Vec<SpaceRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                    "SELECT {SPACE} FROM spaces s
                     WHERE s.state = $1 AND s.audience = $2
                       AND ($3::text IS NULL OR $3 = ANY(s.tags)) AND ($4::text IS NULL OR s.language = $4)
                       AND s.host_id <> ALL($5)
                       AND NOT EXISTS (SELECT 1 FROM space_bans b WHERE b.space_id = s.id AND b.user_id = $6)
                     ORDER BY s.listener_count DESC, s.id DESC
                     LIMIT $7 OFFSET $8"
                )))
                .bind(state::LIVE)
                .bind(audience::PUBLIC)
                .bind(&tag)
                .bind(&language)
                .bind(&hidden)
                .bind(viewer)
                .bind(page.fetch())
                .bind(offset)
                .fetch_all(&core.db)
                .await?;
                let info = page.finish(&mut rows, |_| offset + page.limit);
                Ok((self.render_spaces(&rows, viewer, Detail::default()).await?, info))
            }
            _ => Err(AppError::invalid("INVALID_SCOPE", "choose which live rooms to list")),
        }
    }

    /// Upcoming rooms `viewer` may enter, soonest first.
    pub(crate) async fn list_upcoming(
        &self,
        viewer: Option<Uuid>,
        tag: &str,
        language: &str,
        reminders_only: bool,
        page: &Page,
    ) -> AppResult<(Vec<pb::Space>, PageInfo)> {
        let core = &self.core;
        let tag = rules::tag(tag);
        let language = Some(rules::language(language)?).filter(|language| !language.is_empty());
        let (friends, hidden) = match viewer {
            Some(viewer) => (graph::friend_ids(core, viewer).await?, graph::blocked_ids_either(core, viewer).await?),
            None => (Vec::new(), Vec::new()),
        };
        let after: Option<(Time, Uuid)> = page.decode()?;
        let mut rows: Vec<SpaceRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {SPACE} FROM spaces s
             WHERE s.state = $1 AND s.host_id <> ALL($2)
               AND (s.audience = $3 OR s.host_id = $4
                    OR (s.audience = $5 AND s.host_id = ANY($6))
                    OR EXISTS (SELECT 1 FROM space_invites i
                               WHERE i.space_id = s.id AND i.user_id = $4 AND i.revoked_at IS NULL))
               AND NOT EXISTS (SELECT 1 FROM space_bans b WHERE b.space_id = s.id AND b.user_id = $4)
               AND ($7::text IS NULL OR $7 = ANY(s.tags)) AND ($8::text IS NULL OR s.language = $8)
               AND (NOT $9 OR EXISTS (SELECT 1 FROM space_reminders r WHERE r.space_id = s.id AND r.user_id = $4))
               AND ($10::timestamptz IS NULL OR (s.scheduled_at, s.id) > ($10, $11))
             ORDER BY s.scheduled_at, s.id
             LIMIT $12"
        )))
        .bind(state::SCHEDULED)
        .bind(&hidden)
        .bind(audience::PUBLIC)
        .bind(viewer)
        .bind(audience::FRIENDS)
        .bind(&friends)
        .bind(&tag)
        .bind(&language)
        .bind(reminders_only)
        .bind(after.map(|(at, _)| at))
        .bind(after.map(|(_, id)| id))
        .bind(page.fetch())
        .fetch_all(&core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.scheduled_at.unwrap_or(row.created_at), row.id));
        Ok((self.render_spaces(&rows, viewer, Detail::default()).await?, info))
    }

    /// The caller's own rooms: live and upcoming first, then past ones,
    /// newest first.
    pub(crate) async fn list_mine(&self, user_id: Uuid, page: &Page) -> AppResult<(Vec<pb::Space>, PageInfo)> {
        // (finished?, id) descending within each group: live and upcoming rooms lead.
        let after: Option<(bool, Uuid)> = page.decode()?;
        let mut rows: Vec<SpaceRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {SPACE} FROM spaces s
             WHERE s.host_id = $1 AND s.deleted_at IS NULL
               AND ($2::boolean IS NULL OR (s.state IN ($4, $5)) > $2
                    OR ((s.state IN ($4, $5)) = $2 AND s.id < $3))
             ORDER BY (s.state IN ($4, $5)), s.id DESC
             LIMIT $6"
        )))
        .bind(user_id)
        .bind(after.map(|(finished, _)| finished))
        .bind(after.map(|(_, id)| id))
        .bind(state::ENDED)
        .bind(state::CANCELLED)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.is_over(), row.id));
        Ok((self.render_spaces(&rows, Some(user_id), Detail::default()).await?, info))
    }

    /// Tags of the public rooms that are live right now, most listeners
    /// first.
    pub(crate) async fn popular_tags(&self, language: &str, limit: i32) -> AppResult<Vec<pb::PopularTag>> {
        let language = Some(rules::language(language)?).filter(|language| !language.is_empty());
        let limit = if limit <= 0 { DEFAULT_POPULAR_TAGS } else { i64::from(limit).min(MAX_POPULAR_TAGS) };
        let rows: Vec<(String, i64, i64)> = sqlx::query_as(
            "SELECT tag, count(*), COALESCE(sum(s.listener_count), 0)::bigint
             FROM spaces s, unnest(s.tags) AS tag
             WHERE s.state = $1 AND s.audience = $2 AND ($3::text IS NULL OR s.language = $3)
             GROUP BY tag
             ORDER BY 3 DESC, 2 DESC, tag
             LIMIT $4",
        )
        .bind(state::LIVE)
        .bind(audience::PUBLIC)
        .bind(&language)
        .bind(limit)
        .fetch_all(&self.core.db)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(tag, spaces, listeners)| pb::PopularTag {
                tag,
                space_count: spaces.min(i64::from(i32::MAX)) as i32,
                listener_count: listeners.min(i64::from(i32::MAX)) as i32,
                ..Default::default()
            })
            .collect())
    }

    /// Friends of `viewer` who are inside a room or hangout `viewer` could
    /// enter right now: rooms first (most listeners first), then hangouts.
    ///
    /// In a room, a friend who hides their online status only shows while
    /// on stage. In a hangout everyone inside shows: its members see that
    /// anyway.
    pub async fn friends_in_rooms(&self, viewer: Uuid) -> AppResult<Vec<FriendInRoom>> {
        let core = &self.core;
        let (rooms, mut friends) = self.rooms_with_friends(viewer).await?;
        let mut found = Vec::new();
        for room in &rooms {
            for (user_id, role, since) in friends.remove(&room.id).unwrap_or_default() {
                found.push(FriendInRoom {
                    user_id,
                    venue: Venue::Space(room.id),
                    title: room.title.clone(),
                    emoji: String::new(),
                    role,
                    since,
                });
            }
        }
        let friend_ids = graph::friend_ids(core, viewer).await?;
        if friend_ids.is_empty() {
            return Ok(found);
        }
        let in_hangouts: Vec<(Uuid, Uuid, String, String, Time)> = sqlx::query_as(
            "SELECT pr.user_id, h.id, h.name, h.emoji, pr.joined_at
             FROM hangout_presence pr
             JOIN hangouts h ON h.id = pr.hangout_id
             JOIN hangout_members mine ON mine.hangout_id = h.id AND mine.user_id = $1 AND mine.state = $2
             WHERE pr.user_id = ANY($3)
             ORDER BY h.inside_count DESC, h.id, pr.joined_at",
        )
        .bind(viewer)
        .bind(member::MEMBER)
        .bind(&friend_ids)
        .fetch_all(&core.db)
        .await?;
        for (user_id, hangout_id, name, emoji, since) in in_hangouts {
            found.push(FriendInRoom { user_id, venue: Venue::Hangout(hangout_id), title: name, emoji, role: 0, since });
        }
        Ok(found)
    }

    /// [`Self::friends_in_rooms`] as the API shows it.
    pub(crate) async fn list_friends_in_rooms(&self, viewer: Uuid) -> AppResult<Vec<pb::FriendInRoom>> {
        let found = self.friends_in_rooms(viewer).await?;
        let ids: Vec<Uuid> = found.iter().map(|friend| friend.user_id).collect();
        let people = tokuly_identity::summaries(&self.core, &ids).await?;
        Ok(found
            .into_iter()
            .map(|friend| {
                let (space_id, hangout_id) = match friend.venue {
                    Venue::Space(id) => (id.to_string(), String::new()),
                    Venue::Hangout(id) => (String::new(), id.to_string()),
                };
                pb::FriendInRoom {
                    user: tokuly_identity::users::pick(&people, friend.user_id).into(),
                    space_id,
                    hangout_id,
                    title: friend.title,
                    emoji: friend.emoji,
                    role: i32::from(friend.role.clamp(0, role::HOST)).into(),
                    since: ts(friend.since),
                    ..Default::default()
                }
            })
            .collect())
    }
}
