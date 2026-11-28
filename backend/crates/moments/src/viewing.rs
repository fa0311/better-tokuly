//! Who sees which moment, and how.
//!
//! - **Own moments**: always, every day, in full.
//! - **Someone else's**: only of its owner's day, only to the
//!   owner's friends (or close friends, by the moment's audience), never
//!   across a block either way, never of a suspended or deleted account.
//!   Yes/no checks on one moment read the social graph live; lists use the
//!   viewer's cached friend list and filter in SQL.
//! - **Locked** until the viewer has a moment of their own day (deleting
//!   it, or the next prompt, locks again): the pictures are a blurred
//!   placeholder (`rules::placeholder`), without caption, location or
//!   reactions. The region of the day only on one's own moments.
//! - **Location** goes to a viewer other than the owner only while that
//!   viewer sees the owner's precise live location (`geoindex::seen_precisely`):
//!   anything derived from where someone is follows the ghost rules of the
//!   map, now rather than when the moment was taken.
//! - **Reactions** without people on either side of a block with the
//!   viewer, and of suspended or deleted accounts.
//!
//! Every unlocked moment shown to someone other than its owner is recorded
//! (`moment_deliveries`): they can still report it after it is gone.
use std::collections::HashMap;

use tokuly_core::{geoindex, prelude::*};
use tokuly_identity::users;
use tokuly_media::assets::image_from_json;
use tokuly_proto::{
    common::LatLng,
    moments::{Moment, MomentReaction, Realmoji, moment_reaction::Reaction},
};
use tokuly_social::graph;

use crate::{
    Moments,
    model::{DayRow, MOMENT_COLUMNS, MomentRow, REACTION_COLUMNS, ReactionRow, audience},
    rules,
    settings::audience_enum,
};

/// Reactions shown with a moment.
const REACTIONS_SHOWN: i64 = 3;
/// The most moments the map shows.
pub const MAP_LIMIT: i64 = 500;

/// The keyset of friends' moments: newest first.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Cursor {
    pub posted_at: Time,
    pub id: Uuid,
}

impl Moments {
    /// Whether the viewer has a moment of their day ([`Self::today_of`]:
    /// a prompt on its way to them locks again already).
    pub(crate) async fn unlocked(&self, viewer: Uuid) -> AppResult<bool> {
        let Some(today) = self.today_of(&self.core.db, viewer).await? else { return Ok(false) };
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM moments WHERE user_id = $1 AND day_id = $2 AND deleted_at IS NULL)",
        )
        .bind(viewer)
        .bind(today.day.id)
        .fetch_one(&self.core.db)
        .await?)
    }

    /// The moment if `viewer` may see it now (locked or not), checked live.
    pub(crate) async fn visible_moment(&self, viewer: Uuid, moment_id: Uuid) -> AppResult<Option<MomentRow>> {
        let core = &self.core;
        let Some(moment) = self.find_moment(moment_id).await? else { return Ok(None) };
        if moment.deleted_at.is_some() {
            return Ok(None);
        }
        if moment.user_id == viewer {
            return Ok(Some(moment));
        }
        let owner = moment.user_id;
        let current: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM moment_people WHERE user_id = $1 AND day_id = $2)")
                .bind(owner)
                .bind(moment.day_id)
                .fetch_one(&core.db)
                .await?;
        let allowed = current
            && !users::filter_active(core, &[owner]).await?.is_empty()
            && !graph::is_blocked_either(core, owner, viewer).await?
            && graph::are_friends(core, owner, viewer).await?
            && (moment.audience != audience::CLOSE_FRIENDS || graph::is_close_friend(core, owner, viewer).await?);
        Ok(allowed.then_some(moment))
    }

    /// [`Self::visible_moment`] or MOMENT_NOT_FOUND.
    pub(crate) async fn require_visible(&self, viewer: Uuid, moment_id: Uuid) -> AppResult<MomentRow> {
        self.visible_moment(viewer, moment_id).await?.or_not_found("MOMENT_NOT_FOUND", "moment not found")
    }

    /// Friends' moments of their own days that `viewer` may see,
    /// newest first. `with_location`: only those that carry one.
    pub(crate) async fn friend_moments(
        &self,
        viewer: Uuid,
        after: Option<Cursor>,
        limit: i64,
        with_location: bool,
    ) -> AppResult<Vec<MomentRow>> {
        let audience = tokuly_social::graph::AudienceViewer::load(&self.core, Some(viewer)).await?;
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {MOMENT_COLUMNS} FROM moments m
             JOIN moment_days d ON d.id = m.day_id
             JOIN moment_people p ON p.user_id = m.user_id AND p.day_id = m.day_id
             JOIN users u ON u.id = m.user_id AND u.deleted_at IS NULL AND u.banned_at IS NULL
             WHERE m.deleted_at IS NULL AND m.user_id = ANY($1) AND m.user_id <> ALL($2)
               AND (m.audience = {friends} OR m.user_id = ANY($3))
               AND ($4::timestamptz IS NULL OR (m.posted_at, m.id) < ($4, $5))
               AND (NOT $6 OR m.lat IS NOT NULL)
             ORDER BY m.posted_at DESC, m.id DESC
             LIMIT $7",
            friends = audience::FRIENDS,
        )))
        .bind(audience.friend_ids())
        .bind(audience.hidden_ids())
        .bind(audience.close_friend_of())
        .bind(after.as_ref().map(|cursor| cursor.posted_at))
        .bind(after.as_ref().map(|cursor| cursor.id))
        .bind(with_location)
        .bind(limit)
        .fetch_all(&self.core.db)
        .await?)
    }

    /// The moments as `viewer` sees them. `unlocked`: [`Self::unlocked`]
    /// of the viewer (asked once per page).
    pub(crate) async fn render(&self, viewer: Uuid, unlocked: bool, rows: &[MomentRow]) -> AppResult<Vec<Moment>> {
        let core = &self.core;
        let open: Vec<Uuid> = rows.iter().filter(|m| m.user_id == viewer || unlocked).map(|m| m.id).collect();
        let hidden = graph::blocked_ids_either(core, viewer).await?;
        let reactions = self.reactions_of(&open, viewer, &hidden).await?;
        let stamps = self.stamps_of(reactions.values().flat_map(|(_, shown)| shown)).await?;
        let mut people: Vec<Uuid> = rows.iter().map(|m| m.user_id).collect();
        people.extend(reactions.values().flat_map(|(_, shown)| shown.iter().map(|r| r.user_id)));
        let names = tokuly_identity::summaries(core, &people).await?;
        let placed: Vec<Uuid> =
            rows.iter().filter(|m| m.user_id != viewer && unlocked && m.lat.is_some()).map(|m| m.user_id).collect();
        let seen = geoindex::seen_precisely(&core.cache, &placed, viewer).await?;

        let mut moments = Vec::with_capacity(rows.len());
        let mut delivered = Vec::new();
        for row in rows {
            let mine = row.user_id == viewer;
            let locked = !mine && !unlocked;
            let (back, front) = if locked {
                (rules::placeholder(&row.back()), rules::placeholder(&row.front()))
            } else {
                (row.back(), row.front())
            };
            let location = match row.location() {
                Some((lat, lng)) if !locked && (mine || seen.contains(&row.user_id)) => {
                    Some(LatLng { lat, lng, ..Default::default() })
                }
                _ => None,
            };
            let (count, shown) = reactions.get(&row.id).cloned().unwrap_or_default();
            let render = |r: &ReactionRow| reaction_view(r, &names, &stamps);
            let my_reaction = shown.iter().find(|r| r.user_id == viewer).map(render);
            if !locked && !mine {
                delivered.push(row.id);
            }
            moments.push(Moment {
                id: row.id.to_string(),
                author: users::pick(&names, row.user_id).into(),
                // Someone else's region would say roughly where they are.
                day: self
                    .day_view(&DayRow {
                        id: row.day_id,
                        region: if mine { row.region.clone() } else { String::new() },
                        date: row.date,
                        announced_at: row.announced_at,
                    })
                    .into(),
                posted_at: ts(row.posted_at),
                late_seconds: row.late_seconds,
                retakes: i32::from(row.retakes),
                back: back.into(),
                front: front.into(),
                caption: if locked { String::new() } else { row.caption.clone() },
                location: location.into(),
                locked,
                reaction_count: count,
                // Newest first; the viewer's own, when older, comes after them.
                reactions: shown.iter().take(REACTIONS_SHOWN as usize).map(render).collect(),
                my_reaction: my_reaction.into(),
                mine,
                audience: if mine { audience_enum(row.audience).into() } else { Default::default() },
                ..Default::default()
            });
        }
        if !delivered.is_empty() {
            sqlx::query(
                "INSERT INTO moment_deliveries (moment_id, user_id)
                 SELECT unnest($1::uuid[]), $2 ON CONFLICT DO NOTHING",
            )
            .bind(&delivered)
            .bind(viewer)
            .execute(&core.db)
            .await?;
        }
        Ok(moments)
    }

    /// Per moment: how many reactions the viewer may see, the newest few of
    /// them and the viewer's own.
    async fn reactions_of(
        &self,
        moment_ids: &[Uuid],
        viewer: Uuid,
        hidden: &[Uuid],
    ) -> AppResult<HashMap<Uuid, (i32, Vec<ReactionRow>)>> {
        if moment_ids.is_empty() {
            return Ok(HashMap::new());
        }
        #[derive(sqlx::FromRow)]
        struct Counted {
            #[sqlx(flatten)]
            reaction: ReactionRow,
            total: i64,
        }
        let rows: Vec<Counted> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT * FROM (
                 SELECT {REACTION_COLUMNS},
                        row_number() OVER (PARTITION BY r.moment_id ORDER BY r.created_at DESC, r.id DESC) AS rank,
                        count(*) OVER (PARTITION BY r.moment_id) AS total
                 FROM moment_reactions r
                 JOIN users u ON u.id = r.user_id AND u.deleted_at IS NULL AND u.banned_at IS NULL
                 WHERE r.moment_id = ANY($1) AND r.removed_at IS NULL AND r.user_id <> ALL($2)
             ) shown
             WHERE rank <= $3 OR user_id = $4
             ORDER BY moment_id, created_at DESC, id DESC"
        )))
        .bind(moment_ids)
        .bind(hidden)
        .bind(REACTIONS_SHOWN)
        .bind(viewer)
        .fetch_all(&self.core.db)
        .await?;
        let mut by_moment: HashMap<Uuid, (i32, Vec<ReactionRow>)> = HashMap::new();
        for row in rows {
            let entry = by_moment.entry(row.reaction.moment_id).or_default();
            entry.0 = row.total as i32;
            entry.1.push(row.reaction);
        }
        Ok(by_moment)
    }

    pub(crate) async fn stamps_of<'a>(
        &self,
        reactions: impl IntoIterator<Item = &'a ReactionRow>,
    ) -> AppResult<HashMap<Uuid, tokuly_proto::common::StampRef>> {
        let ids: Vec<Uuid> = reactions.into_iter().filter_map(|r| r.stamp_id).collect();
        tokuly_stamp::stamp_refs(&self.core, &ids).await
    }
}

/// One reaction as shown.
pub(crate) fn reaction_view(
    row: &ReactionRow,
    names: &HashMap<Uuid, tokuly_proto::common::UserSummary>,
    stamps: &HashMap<Uuid, tokuly_proto::common::StampRef>,
) -> MomentReaction {
    let reaction = match (row.realmoji, row.stamp_id) {
        (Some(emoji), _) => Some(Reaction::Realmoji(Box::new(Realmoji {
            emoji: i32::from(emoji).into(),
            image: image_from_json(row.image.as_ref()).unwrap_or_default().into(),
            ..Default::default()
        }))),
        (None, Some(stamp_id)) => stamps.get(&stamp_id).cloned().map(|stamp| Reaction::Stamp(Box::new(stamp))),
        (None, None) => None,
    };
    MomentReaction {
        id: row.id.to_string(),
        user: users::pick(names, row.user_id).into(),
        reaction,
        created_at: ts(row.created_at),
        ..Default::default()
    }
}
