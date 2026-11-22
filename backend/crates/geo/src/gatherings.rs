//! Gatherings: friends in the same spot ("N人が集合中!").
//!
//! Each user's [`crate::detect::ProximityTracker`] decides which friends they
//! are together with — only friends who see each other's precise location
//! count (see [`Geo::mutually_precise`]). This module turns those pairwise
//! facts into gatherings with stable IDs: the first two people found together
//! found one, others join it, two gatherings whose members meet merge into
//! the older one, and the last-but-one leaving ends it.
//!
//! The rows are the server's bookkeeping. What a viewer gets is a view of
//! their own ([`Scene::view`]): the members they are a friend of and see
//! precisely (and themselves), the middle of *those* members, when *those*
//! came together, and a place name only if it follows from all that. With
//! fewer than two such members there is no gathering as far as the viewer is
//! concerned, and a viewer is told that a gathering ended only if they were
//! shown it. Nothing about a member a viewer does not see precisely — not
//! their name, not their existence, not their pull on the centre — reaches
//! that viewer.
use std::collections::HashSet;

use chrono::{DateTime, Utc};
use h3o::Resolution;
use tokuly_core::{
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::geo::{Gathering, GatheringMember, geo_event::Event};
use tokuly_social::{InteractionKind, graph};

use crate::{
    Geo,
    ctx::OwnerCtx,
    math::{self, Point},
    places::{FRIENDS, LABEL_CUSTOM, PLACE_COLUMNS, PUBLIC, PlaceRow},
    visibility,
};

/// Time together is reported to the social domain in steps of at least this many minutes...
const CREDIT_STEP_MINUTES: i64 = 5;
/// ...and never more than this at once: after a long silence nobody knows
/// whether the members were still together.
const CREDIT_MAX_MINUTES: i64 = 20;
/// Members not seen together with anyone for this long are taken out by the sweeper.
const STALE_MINUTES: i32 = 45;
/// The same two people are announced as "together" at most this often.
const ANNOUNCE_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(3 * 3600);
/// Friends told about a new gathering at most.
const MAX_ANNOUNCED: usize = 100;
/// Two members count as within reach of each other up to the gathering
/// radius plus this (GPS error, as in the proximity check).
const LINK_ALLOWANCE_M: f64 = 20.0;
/// A place is named on a gathering when the gathering is this close to its edge.
const PLACE_ALLOWANCE_M: f64 = 20.0;
/// Membership changes are serialized per neighbourhood, not globally: cells
/// of this size (edges of about 8 km) around the spot.
const LOCK_RESOLUTION: Resolution = Resolution::Five;
/// How long the record of who was shown a gathering is kept without news.
const SHOWN_TTL_SECONDS: i64 = 7 * 24 * 3600;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct GatheringRow {
    pub id: Uuid,
    pub lat: f64,
    pub lng: f64,
    pub place_id: Option<Uuid>,
    pub place_name: String,
    pub at_private_place: bool,
    pub credited_until: DateTime<Utc>,
}

const GATHERING_COLUMNS: &str = "id, lat, lng, place_id, place_name, at_private_place, credited_until";

/// What a membership sync changed.
#[derive(Debug, Default)]
pub(crate) struct Changes {
    /// A gathering was founded.
    pub formed: Option<Uuid>,
    /// Gatherings whose member list changed.
    pub updated: Vec<Uuid>,
    /// Gatherings that ended, with the members they had.
    pub ended: Vec<(GatheringRow, Vec<Uuid>)>,
}

impl Changes {
    fn touch(&mut self, gathering_id: Uuid) {
        if !self.updated.contains(&gathering_id) {
            self.updated.push(gathering_id);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.formed.is_none() && self.updated.is_empty() && self.ended.is_empty()
    }
}

/// Minutes of time together to credit for the stretch since `credited_until`,
/// and the new mark. `None` while the stretch is too short to bother (unless
/// the gathering is ending, when whatever is left counts).
pub(crate) fn credit(credited_until: DateTime<Utc>, now: DateTime<Utc>, closing: bool) -> Option<(i32, DateTime<Utc>)> {
    let minutes = (now - credited_until).num_minutes();
    if minutes < if closing { 1 } else { CREDIT_STEP_MINUTES } {
        return None;
    }
    if minutes > CREDIT_MAX_MINUTES {
        // Too long since anyone confirmed it: credit a capped amount and start over from now.
        return Some((CREDIT_MAX_MINUTES as i32, now));
    }
    Some((minutes as i32, credited_until + chrono::Duration::minutes(minutes)))
}

/// One member of a gathering as the server knows them.
#[derive(Debug, Clone)]
pub(crate) struct SceneMember {
    pub id: Uuid,
    pub arrived_at: DateTime<Utc>,
    /// Their live position, if they have one.
    pub position: Option<Point>,
    pub ctx: OwnerCtx,
    pub friends: HashSet<Uuid>,
}

/// A gathering with everything needed to work out what any one viewer sees
/// of it. Pure: [`Scene::view`] is the per-viewer rule, tested below.
#[derive(Debug, Clone)]
pub(crate) struct Scene {
    pub id: Uuid,
    pub members: Vec<SceneMember>,
    /// Named places around the spot (public ones, and members' own shared
    /// with friends): each view picks from them by its own centre.
    pub places: Vec<ScenePlace>,
    pub radius_m: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct ScenePlace {
    pub name: String,
    pub center: Point,
    pub radius_m: f64,
    pub owner_id: Uuid,
    /// The place is public (anyone not blocked may see it); otherwise it is
    /// shared with its owner's friends.
    pub public: bool,
}

/// What one viewer sees of a gathering.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub(crate) struct SceneView {
    /// The members shown, in order of arrival.
    pub members: Vec<(Uuid, DateTime<Utc>)>,
    pub center: Point,
    /// When two of the members shown were first both there.
    pub started_at: DateTime<Utc>,
    pub place_name: String,
    pub at_private_place: bool,
}

impl Scene {
    /// The gathering as `viewer` sees it at `now`, or `None` if they see
    /// fewer than two of its members. `blocked` are the people in a block
    /// with the viewer (their public places are not named).
    pub fn view(&self, viewer: Uuid, blocked: &[Uuid], now: DateTime<Utc>) -> Option<SceneView> {
        // The members the viewer sees, with where they are.
        let visible: Vec<(&SceneMember, Point)> = self
            .members
            .iter()
            .filter(|member| {
                member.id == viewer
                    || (member.friends.contains(&viewer) && visibility::sees_precisely(&member.ctx, viewer, now))
            })
            .filter_map(|member| member.position.map(|position| (member, position)))
            .collect();
        // Of those, a group that holds together by itself: each within reach
        // of another one of the group. Two people the viewer sees who are
        // only connected through someone the viewer does not see are not
        // together as far as the viewer can tell — showing them as a
        // gathering would point at the one in between.
        let reach = self.radius_m + LINK_ALLOWANCE_M;
        let mut group_of: Vec<usize> = (0..visible.len()).collect();
        for i in 0..visible.len() {
            for j in i + 1..visible.len() {
                if math::distance(visible[i].1, visible[j].1) <= reach {
                    let (from, to) = (group_of[j], group_of[i]);
                    for group in &mut group_of {
                        if *group == from {
                            *group = to;
                        }
                    }
                }
            }
        }
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for group in group_of.iter().copied().collect::<std::collections::BTreeSet<_>>() {
            let members: Vec<usize> = (0..visible.len()).filter(|index| group_of[*index] == group).collect();
            if members.len() >= 2 {
                groups.push(members);
            }
        }
        // The viewer's own group if they are in one, else the largest (the earliest among equals).
        let second_arrival = |group: &Vec<usize>| {
            let mut arrivals: Vec<DateTime<Utc>> = group.iter().map(|index| visible[*index].0.arrived_at).collect();
            arrivals.sort();
            arrivals[1]
        };
        groups.sort_by_key(|group| {
            let own = group.iter().any(|index| visible[*index].0.id == viewer);
            (!own, std::cmp::Reverse(group.len()), second_arrival(group))
        });
        let group = groups.first()?;
        let shown: Vec<&SceneMember> = group.iter().map(|index| visible[*index].0).collect();
        let positions: Vec<Point> = group.iter().map(|index| visible[*index].1).collect();
        // The middle of the members shown: a hidden member has no pull on it.
        let center = (
            positions.iter().map(|p| p.0).sum::<f64>() / positions.len() as f64,
            positions.iter().map(|p| p.1).sum::<f64>() / positions.len() as f64,
        );
        // A place is named when the viewer could look it up and the members
        // they see are there: the closest such place to THEIR centre.
        let place_name = self
            .places
            .iter()
            .filter(|place| math::distance(center, place.center) <= place.radius_m + PLACE_ALLOWANCE_M)
            .filter(|place| {
                place.owner_id == viewer
                    || if place.public {
                        !blocked.contains(&place.owner_id)
                    } else {
                        shown.iter().any(|member| member.id == place.owner_id && member.friends.contains(&viewer))
                    }
            })
            .min_by(|a, b| {
                let ratio = |place: &ScenePlace| math::distance(center, place.center) / place.radius_m.max(1.0);
                ratio(a).total_cmp(&ratio(b))
            })
            .map(|place| place.name.clone())
            .unwrap_or_default();
        // At the home, work or school of a member shown — as far as that
        // member lets friends know where those are.
        let at_private_place = shown.iter().any(|member| {
            (member.id == viewer || member.ctx.privacy.share_place_labels)
                && member.ctx.places.iter().any(|place| {
                    place.label != LABEL_CUSTOM
                        && math::distance(center, (place.lat, place.lng)) <= place.radius_m + self.radius_m
                })
        });
        // For this viewer it began when two of the members they see were
        // first both there; nobody they see "arrived" before that (it would
        // say that someone else was there already).
        let started_at = second_arrival(group);
        Some(SceneView {
            members: shown.iter().map(|member| (member.id, member.arrived_at.max(started_at))).collect(),
            center,
            started_at,
            place_name,
            at_private_place,
        })
    }

    /// Everyone who might see the gathering: its members and their friends.
    fn candidates(&self) -> Vec<Uuid> {
        let mut out: HashSet<Uuid> = self.members.iter().map(|member| member.id).collect();
        for member in &self.members {
            out.extend(member.friends.iter().copied());
        }
        out.into_iter().collect()
    }
}

type Tx<'a> = sqlx::Transaction<'a, sqlx::Postgres>;

/// Serializes membership changes around `center`: the cell it is in and the
/// cells around that one, always in the same order. Two changes within a few
/// kilometers of each other wait for one another; the rest of the world does
/// not.
async fn lock_area(tx: &mut Tx<'_>, center: Point) -> AppResult<()> {
    let Some(cell) = math::cell_at(center.0, center.1, LOCK_RESOLUTION) else { return Ok(()) };
    let mut keys: Vec<String> =
        cell.grid_disk::<Vec<_>>(1).into_iter().map(|cell| format!("geo:gatherings:{cell}")).collect();
    keys.sort();
    sqlx::query(
        "SELECT pg_advisory_xact_lock(hashtextextended(k, 0)) FROM (SELECT k FROM unnest($1::text[]) AS k ORDER BY k OFFSET 0) keys",
    )
    .bind(keys)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn active_members(tx: &mut Tx<'_>, gathering_id: Uuid) -> AppResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar("SELECT user_id FROM geo_gathering_members WHERE gathering_id = $1 AND left_at IS NULL ORDER BY arrived_at, user_id")
        .bind(gathering_id)
        .fetch_all(&mut **tx)
        .await?)
}

async fn join(tx: &mut Tx<'_>, gathering_id: Uuid, user_id: Uuid, at: DateTime<Utc>) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO geo_gathering_members (gathering_id, user_id, arrived_at, last_seen_at) VALUES ($1, $2, $3, $3)
         ON CONFLICT (gathering_id, user_id) DO UPDATE SET left_at = NULL, last_seen_at = EXCLUDED.last_seen_at",
    )
    .bind(gathering_id)
    .bind(user_id)
    .bind(at)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE geo_gatherings SET peak_size = GREATEST(peak_size,
             (SELECT count(*) FROM geo_gathering_members WHERE gathering_id = $1 AND left_at IS NULL)::int)
         WHERE id = $1",
    )
    .bind(gathering_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Ends a gathering: everyone still in it leaves.
async fn end(tx: &mut Tx<'_>, gathering_id: Uuid, at: DateTime<Utc>, changes: &mut Changes) -> AppResult<()> {
    let members = active_members(tx, gathering_id).await?;
    sqlx::query("UPDATE geo_gathering_members SET left_at = $2 WHERE gathering_id = $1 AND left_at IS NULL")
        .bind(gathering_id)
        .bind(at)
        .execute(&mut **tx)
        .await?;
    let row: Option<GatheringRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "UPDATE geo_gatherings SET ended_at = $2 WHERE id = $1 AND ended_at IS NULL RETURNING {GATHERING_COLUMNS}"
    )))
    .bind(gathering_id)
    .bind(at)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(row) = row {
        changes.updated.retain(|id| *id != gathering_id);
        changes.ended.push((row, members));
    }
    Ok(())
}

/// Takes one member out; a gathering left with fewer than two people ends.
async fn leave(
    tx: &mut Tx<'_>,
    gathering_id: Uuid,
    user_id: Uuid,
    at: DateTime<Utc>,
    changes: &mut Changes,
) -> AppResult<()> {
    let before = active_members(tx, gathering_id).await?;
    if !before.contains(&user_id) {
        return Ok(());
    }
    if before.len() <= 2 {
        return end(tx, gathering_id, at, changes).await;
    }
    sqlx::query(
        "UPDATE geo_gathering_members SET left_at = $3 WHERE gathering_id = $1 AND user_id = $2 AND left_at IS NULL",
    )
    .bind(gathering_id)
    .bind(user_id)
    .bind(at)
    .execute(&mut **tx)
    .await?;
    changes.touch(gathering_id);
    Ok(())
}

fn shown_key(gathering_id: Uuid) -> String {
    format!("geo:gathering:shown:{}", gathering_id.simple())
}

impl Geo {
    /// Brings the gatherings in line with who `user_id` is together with
    /// right now (`together`: friends near for long enough, from their
    /// proximity tracker — each of them sees the user's precise location and
    /// the user theirs). `position` is where the user is.
    pub(crate) async fn sync_gathering(
        &self,
        user_id: Uuid,
        together: &[Uuid],
        position: Point,
        at: DateTime<Utc>,
    ) -> AppResult<Changes> {
        let mut changes = Changes::default();
        // Where the change happens: here, or (when the user only leaves)
        // where their gathering is.
        let area = if together.is_empty() {
            let center: Option<(f64, f64)> = sqlx::query_as(
                "SELECT g.lat, g.lng FROM geo_gathering_members m JOIN geo_gatherings g ON g.id = m.gathering_id
                 WHERE m.user_id = $1 AND m.left_at IS NULL",
            )
            .bind(user_id)
            .fetch_optional(&self.core.db)
            .await?;
            let Some(center) = center else { return Ok(changes) };
            center
        } else {
            position
        };
        let mut tx = self.core.db.begin().await?;
        // Founding, joining and merging in one neighbourhood must not interleave.
        lock_area(&mut tx, area).await?;
        let mine: Option<Uuid> =
            sqlx::query_scalar("SELECT gathering_id FROM geo_gathering_members WHERE user_id = $1 AND left_at IS NULL")
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await?;

        if together.is_empty() {
            if let Some(gathering_id) = mine {
                leave(&mut tx, gathering_id, user_id, at, &mut changes).await?;
            }
            tx.commit().await?;
            return Ok(changes);
        }

        let theirs: Vec<(Uuid, Uuid, DateTime<Utc>)> = sqlx::query_as(
            "SELECT m.user_id, m.gathering_id, g.started_at FROM geo_gathering_members m JOIN geo_gatherings g ON g.id = m.gathering_id
             WHERE m.user_id = ANY($1) AND m.left_at IS NULL",
        )
        .bind(together)
        .fetch_all(&mut *tx)
        .await?;
        let mut candidates: Vec<(DateTime<Utc>, Uuid)> =
            theirs.iter().map(|(_, gathering_id, started_at)| (*started_at, *gathering_id)).collect();
        if let Some(gathering_id) = mine {
            let started_at: DateTime<Utc> = sqlx::query_scalar("SELECT started_at FROM geo_gatherings WHERE id = $1")
                .bind(gathering_id)
                .fetch_one(&mut *tx)
                .await?;
            candidates.push((started_at, gathering_id));
        }
        candidates.sort();
        candidates.dedup();

        let target = match candidates.first() {
            // The oldest gathering involved keeps its ID.
            Some((_, oldest)) => *oldest,
            None => {
                let gathering_id: Uuid = sqlx::query_scalar("INSERT INTO geo_gatherings (lat, lng, started_at, credited_until) VALUES ($1, $2, $3, $3) RETURNING id")
                    .bind(position.0)
                    .bind(position.1)
                    .bind(at)
                    .fetch_one(&mut *tx)
                    .await?;
                changes.formed = Some(gathering_id);
                gathering_id
            }
        };
        // Everyone in the other gatherings involved moves over.
        for (_, other) in candidates.iter().filter(|(_, id)| *id != target) {
            let movers = active_members(&mut tx, *other).await?;
            end(&mut tx, *other, at, &mut changes).await?;
            for mover in movers {
                join(&mut tx, target, mover, at).await?;
            }
            changes.touch(target);
        }
        let members = active_members(&mut tx, target).await?;
        for joiner in together.iter().chain([&user_id]).filter(|id| !members.contains(id)) {
            join(&mut tx, target, *joiner, at).await?;
            changes.touch(target);
        }
        // Who this member was with, by their own check: what "met" means later.
        sqlx::query(
            "UPDATE geo_gathering_members SET last_seen_at = $3, with_user_ids = ARRAY(SELECT DISTINCT unnest(with_user_ids || $4))
             WHERE gathering_id = $1 AND user_id = $2",
        )
        .bind(target)
        .bind(user_id)
        .bind(at)
        .bind(together)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(changes)
    }

    /// After a membership change: describes the spot, announces a new
    /// gathering, reports time together, and tells everyone what they now
    /// see of it.
    pub(crate) async fn publish_gathering_changes(&self, changes: Changes) -> AppResult<()> {
        let now = Utc::now();
        for (row, members) in &changes.ended {
            self.credit_gathering(row, members, now, true).await?;
            // Only the people who were shown it are told that it is over.
            let shown = self.take_shown(row.id).await?;
            self.push_many(shown, Event::GatheringEndedId(row.id.to_string())).await;
        }
        let mut current = changes.updated.clone();
        current.extend(changes.formed.filter(|formed| !changes.updated.contains(formed)));
        for gathering_id in current {
            let Some((row, members)) = self.gathering(gathering_id).await? else { continue };
            let row = self.describe_gathering(row, &members).await?;
            let scene = self.scene(&row, &members).await?;
            if changes.formed == Some(row.id) && !row.at_private_place {
                self.announce_gathering(&scene, now).await?;
            }
            self.push_scene(&scene, now).await?;
        }
        Ok(())
    }

    /// An active gathering and its members with their arrival times.
    async fn gathering(&self, gathering_id: Uuid) -> AppResult<Option<(GatheringRow, Vec<(Uuid, DateTime<Utc>)>)>> {
        let row: Option<GatheringRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {GATHERING_COLUMNS} FROM geo_gatherings WHERE id = $1 AND ended_at IS NULL"
        )))
        .bind(gathering_id)
        .fetch_optional(&self.core.db)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let members: Vec<(Uuid, DateTime<Utc>)> =
            sqlx::query_as("SELECT user_id, arrived_at FROM geo_gathering_members WHERE gathering_id = $1 AND left_at IS NULL ORDER BY arrived_at, user_id")
                .bind(gathering_id)
                .fetch_all(&self.core.db)
                .await?;
        Ok(Some((row, members)))
    }

    /// Recomputes where a gathering is and what kind of spot that is, for
    /// the server's own use (matching a place, deciding whether to announce
    /// it). Viewers get their own centre and place, see [`Scene::view`].
    async fn describe_gathering(
        &self,
        mut row: GatheringRow,
        members: &[(Uuid, DateTime<Utc>)],
    ) -> AppResult<GatheringRow> {
        let ids: Vec<Uuid> = members.iter().map(|(id, _)| *id).collect();
        let states = self.load_states(&ids).await?;
        let positions: Vec<Point> =
            states.iter().flatten().filter(|state| state.located()).map(|state| state.pin.position()).collect();
        if !positions.is_empty() {
            row.lat = positions.iter().map(|p| p.0).sum::<f64>() / positions.len() as f64;
            row.lng = positions.iter().map(|p| p.1).sum::<f64>() / positions.len() as f64;
        }
        let center = (row.lat, row.lng);
        // At someone's home, work or school: no party, no announcement.
        let contexts = self.ctx_many(&ids).await?;
        row.at_private_place = contexts.values().any(|ctx| {
            ctx.places.iter().any(|place| {
                place.label != LABEL_CUSTOM
                    && math::distance(center, (place.lat, place.lng))
                        <= place.radius_m + self.cfg.gathering_radius_meters
            })
        });
        // A named place there: public, or a member's own shared with friends.
        let place: Option<(Uuid, String)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT p.id, p.name FROM geo_places p
             WHERE ST_DWithin(p.geog, ST_SetSRID(ST_MakePoint($2, $1), 4326)::geography, 1030)
               AND ST_Distance(p.geog, ST_SetSRID(ST_MakePoint($2, $1), 4326)::geography) <= p.radius_m + {PLACE_ALLOWANCE_M}
               AND p.label = {LABEL_CUSTOM} AND (p.visibility = {PUBLIC} OR (p.visibility = {FRIENDS} AND p.owner_id = ANY($3)))
             ORDER BY ST_Distance(p.geog, ST_SetSRID(ST_MakePoint($2, $1), 4326)::geography) / p.radius_m LIMIT 1"
        )))
        .bind(row.lat)
        .bind(row.lng)
        .bind(&ids)
        .fetch_optional(&self.core.db)
        .await?;
        (row.place_id, row.place_name) = match place {
            Some((id, name)) => (Some(id), name),
            None => (None, String::new()),
        };
        sqlx::query("UPDATE geo_gatherings SET lat = $2, lng = $3, place_id = $4, place_name = $5, at_private_place = $6 WHERE id = $1")
            .bind(row.id)
            .bind(row.lat)
            .bind(row.lng)
            .bind(row.place_id)
            .bind(&row.place_name)
            .bind(row.at_private_place)
            .execute(&self.core.db)
            .await?;
        Ok(row)
    }

    /// Loads what the per-viewer rule needs to know about a gathering.
    async fn scene(&self, row: &GatheringRow, members: &[(Uuid, DateTime<Utc>)]) -> AppResult<Scene> {
        let ids: Vec<Uuid> = members.iter().map(|(id, _)| *id).collect();
        let states = self.load_states(&ids).await?;
        let mut contexts = self.ctx_many(&ids).await?;
        let mut scene_members = Vec::with_capacity(members.len());
        for ((id, arrived_at), state) in members.iter().zip(states) {
            scene_members.push(SceneMember {
                id: *id,
                arrived_at: *arrived_at,
                position: state.filter(|state| state.located() && state.pin.sharing).map(|state| state.pin.position()),
                ctx: contexts.remove(id).unwrap_or_default(),
                friends: graph::friend_ids(&self.core, *id).await?.into_iter().collect(),
            });
        }
        // Every named place around: a view picks its own by where its members are.
        let around: Vec<PlaceRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {PLACE_COLUMNS} FROM geo_places p
             WHERE ST_DWithin(p.geog, ST_SetSRID(ST_MakePoint($2, $1), 4326)::geography, 1500)
               AND p.label = {LABEL_CUSTOM} AND (p.visibility = {PUBLIC} OR (p.visibility = {FRIENDS} AND p.owner_id = ANY($3)))
             ORDER BY p.id LIMIT 50"
        )))
        .bind(row.lat)
        .bind(row.lng)
        .bind(&ids)
        .fetch_all(&self.core.db)
        .await?;
        let places = around
            .into_iter()
            .map(|place| ScenePlace {
                name: place.name.clone(),
                center: place.center(),
                radius_m: place.radius_m,
                owner_id: place.owner_id,
                public: place.visibility == PUBLIC,
            })
            .collect();
        Ok(Scene { id: row.id, members: scene_members, places, radius_m: self.cfg.gathering_radius_meters })
    }

    /// A viewer's view as the API message.
    async fn gathering_message(&self, gathering_id: Uuid, view: &SceneView) -> AppResult<Gathering> {
        let ids: Vec<Uuid> = view.members.iter().map(|(id, _)| *id).collect();
        let users = tokuly_identity::summaries(&self.core, &ids).await?;
        Ok(Gathering {
            id: gathering_id.to_string(),
            center: math::lat_lng(view.center.0, view.center.1).into(),
            members: view
                .members
                .iter()
                .map(|(id, arrived_at)| GatheringMember {
                    user: tokuly_identity::users::pick(&users, *id).into(),
                    arrived_at: ts(*arrived_at),
                    ..Default::default()
                })
                .collect(),
            place_name: view.place_name.clone(),
            started_at: ts(view.started_at),
            at_private_place: view.at_private_place,
            ..Default::default()
        })
    }

    /// Remembers what `viewer` was shown of a gathering. Returns whether
    /// that is news to them (they had not been shown it, or not like this).
    async fn mark_shown(&self, gathering_id: Uuid, viewer: Uuid, view: &SceneView) -> AppResult<bool> {
        let key = shown_key(gathering_id);
        let shown = serde_json::to_string(view).unwrap_or_default();
        let mut conn = self.core.cache.conn();
        let before: Option<String> =
            redis::cmd("HGET").arg(&key).arg(viewer.simple().to_string()).query_async(&mut conn).await?;
        if before.as_deref() == Some(shown.as_str()) {
            return Ok(false);
        }
        redis::cmd("HSET").arg(&key).arg(viewer.simple().to_string()).arg(shown).query_async::<()>(&mut conn).await?;
        redis::cmd("EXPIRE").arg(&key).arg(SHOWN_TTL_SECONDS).query_async::<()>(&mut conn).await?;
        Ok(true)
    }

    /// Forgets that `viewer` was shown a gathering. Returns whether they had been.
    async fn unmark_shown(&self, gathering_id: Uuid, viewer: Uuid) -> AppResult<bool> {
        let removed: i64 = redis::cmd("HDEL")
            .arg(shown_key(gathering_id))
            .arg(viewer.simple().to_string())
            .query_async(&mut self.core.cache.conn())
            .await?;
        Ok(removed > 0)
    }

    /// Everyone who was shown a gathering; the record is dropped.
    async fn take_shown(&self, gathering_id: Uuid) -> AppResult<Vec<Uuid>> {
        let key = shown_key(gathering_id);
        let mut conn = self.core.cache.conn();
        let members: Vec<String> = redis::cmd("HKEYS").arg(&key).query_async(&mut conn).await?;
        redis::cmd("DEL").arg(&key).query_async::<()>(&mut conn).await?;
        Ok(members.iter().filter_map(|member| Uuid::parse_str(member).ok()).collect())
    }

    /// Shows a gathering to everyone who can see it, each in their own view,
    /// and tells those who were shown it and no longer see it that it ended
    /// (for them).
    async fn push_scene(&self, scene: &Scene, now: DateTime<Utc>) -> AppResult<()> {
        let mut viewers: HashSet<Uuid> = scene.candidates().into_iter().collect();
        // People who were shown it and are no longer anyone's friend here.
        let shown: Vec<String> =
            redis::cmd("HKEYS").arg(shown_key(scene.id)).query_async(&mut self.core.cache.conn()).await?;
        viewers.extend(shown.iter().filter_map(|member| Uuid::parse_str(member).ok()));
        for viewer in viewers {
            // (A viewer who sees nobody here cannot be named a blocked place either: skip the lookup.)
            let blocked = if scene.places.iter().any(|place| place.public) {
                graph::blocked_ids_either(&self.core, viewer).await?
            } else {
                Vec::new()
            };
            match scene.view(viewer, &blocked, now) {
                Some(view) => {
                    // Only if what they see changed: a member they do not see
                    // coming or going must not even cause a repeat.
                    if self.mark_shown(scene.id, viewer, &view).await? {
                        let message = self.gathering_message(scene.id, &view).await?;
                        self.push(viewer, Event::GatheringUpdated(Box::new(message))).await;
                    }
                }
                None => {
                    if self.unmark_shown(scene.id, viewer).await? {
                        self.push(viewer, Event::GatheringEndedId(scene.id.to_string())).await;
                    }
                }
            }
        }
        Ok(())
    }

    /// Shows everyone concerned the gathering `user_id` is in again, after
    /// who sees whom changed without anyone moving.
    pub(crate) async fn refresh_gathering_of(&self, user_id: Uuid) -> AppResult<()> {
        let gathering_id: Option<Uuid> =
            sqlx::query_scalar("SELECT gathering_id FROM geo_gathering_members WHERE user_id = $1 AND left_at IS NULL")
                .bind(user_id)
                .fetch_optional(&self.core.db)
                .await?;
        let Some(gathering_id) = gathering_id else { return Ok(()) };
        let Some((row, members)) = self.gathering(gathering_id).await? else { return Ok(()) };
        let scene = self.scene(&row, &members).await?;
        self.push_scene(&scene, Utc::now()).await
    }

    /// Notifies friends that two (or more) of their friends are together —
    /// unless the two were announced together a short while ago. Each
    /// recipient is told about the members they see, and nobody else.
    async fn announce_gathering(&self, scene: &Scene, now: DateTime<Utc>) -> AppResult<()> {
        if scene.members.len() < 2 {
            return Ok(());
        }
        let (a, b) = (scene.members[0].id, scene.members[1].id);
        let (first, second) = (a.min(b), a.max(b));
        if !self
            .core
            .cache
            .set_nx(&format!("geo:gathering:announced:{}:{}", first.simple(), second.simple()), "1", ANNOUNCE_COOLDOWN)
            .await?
        {
            return Ok(());
        }
        let member_ids: HashSet<Uuid> = scene.members.iter().map(|member| member.id).collect();
        let mut recipients: Vec<Uuid> = scene.candidates().into_iter().filter(|id| !member_ids.contains(id)).collect();
        recipients.sort();
        let mut notifications = Vec::new();
        for recipient in recipients {
            if notifications.len() >= MAX_ANNOUNCED {
                break;
            }
            let blocked = if scene.places.iter().any(|place| place.public) {
                graph::blocked_ids_either(&self.core, recipient).await?
            } else {
                Vec::new()
            };
            let Some(view) = scene.view(recipient, &blocked, now) else { continue };
            if view.at_private_place {
                continue;
            }
            let ids: Vec<Uuid> = view.members.iter().map(|(id, _)| *id).collect();
            let users = tokuly_identity::summaries(&self.core, &ids).await?;
            let name = |id: &Uuid| users.get(id).map(|user| user.display_name.clone()).unwrap_or_default();
            let title = if ids.len() == 2 {
                Localized::new(
                    format!("{}さんと{}さんが集合中!", name(&ids[0]), name(&ids[1])),
                    format!("{} and {} are together!", name(&ids[0]), name(&ids[1])),
                )
            } else {
                Localized::new(
                    format!("{}さんたち{}人が集合中!", name(&ids[0]), ids.len()),
                    format!("{} and {} others are together!", name(&ids[0]), ids.len() - 1),
                )
            };
            let body = if view.place_name.is_empty() {
                Localized::new("マップで見てみよう!", "See them on the map!")
            } else {
                Localized::new(format!("📍{}", view.place_name), format!("📍{}", view.place_name))
            };
            notifications.push(
                NotificationRequest::new(
                    recipient,
                    "geo.gathering",
                    title,
                    body,
                    format!("/map?gathering={}", scene.id),
                )
                .actor(ids[0])
                .collapse(format!("geo.gathering:{}", scene.id))
                .data(serde_json::json!({ "gathering_id": scene.id, "user_ids": ids })),
            );
        }
        if notifications.is_empty() {
            return Ok(());
        }
        let mut tx = self.core.db.begin().await?;
        for notification in &notifications {
            outbound::notify(&mut *tx, notification).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Reports time together to the social domain for the pairs of members
    /// who see each other's precise location, once per stretch (several
    /// workers may try; one wins).
    async fn credit_gathering(
        &self,
        row: &GatheringRow,
        members: &[Uuid],
        now: DateTime<Utc>,
        closing: bool,
    ) -> AppResult<()> {
        let Some((minutes, new_mark)) = credit(row.credited_until, now, closing) else { return Ok(()) };
        let won = sqlx::query("UPDATE geo_gatherings SET credited_until = $3 WHERE id = $1 AND credited_until = $2")
            .bind(row.id)
            .bind(row.credited_until)
            .bind(new_mark)
            .execute(&self.core.db)
            .await?
            .rows_affected()
            == 1;
        if !won {
            return Ok(());
        }
        let contexts = self.ctx_many(members).await?;
        let sees = |owner: &Uuid, viewer: &Uuid| {
            contexts.get(owner).is_some_and(|ctx| visibility::sees_precisely(ctx, *viewer, now))
        };
        let arrivals: std::collections::HashMap<Uuid, DateTime<Utc>> = sqlx::query_as(
            "SELECT user_id, arrived_at FROM geo_gathering_members WHERE gathering_id = $1 AND user_id = ANY($2)",
        )
        .bind(row.id)
        .bind(members)
        .fetch_all(&self.core.db)
        .await?
        .into_iter()
        .collect();
        for (index, a) in members.iter().enumerate() {
            for b in &members[index + 1..] {
                // Time together is something both learn (it moves their
                // ranking of each other): only for pairs who see each other,
                // and only for the time the two of them were both there —
                // not for how long the gathering had been going with others.
                // (Pairs who are not friends are ignored by the social domain.)
                let both_since = arrivals.get(a).zip(arrivals.get(b)).map(|(a, b)| *a.max(b));
                let shared =
                    both_since.map_or(0, |since| (new_mark - since).num_minutes().clamp(0, minutes as i64)) as i32;
                if shared > 0 && sees(a, b) && sees(b, a) {
                    graph::record_interaction(&self.core, *a.min(b), *a.max(b), InteractionKind::TimeTogether, shared)
                        .await?;
                }
            }
        }
        Ok(())
    }

    /// Confirms that `user_id` is still in their gathering (at `position`)
    /// and reports time together when enough has passed. Called by the
    /// pipeline about once a minute for members.
    pub(crate) async fn gathering_heartbeat(&self, user_id: Uuid, position: Point, at: DateTime<Utc>) -> AppResult<()> {
        let gathering_id: Option<Uuid> =
            sqlx::query_scalar("UPDATE geo_gathering_members SET last_seen_at = $2 WHERE user_id = $1 AND left_at IS NULL RETURNING gathering_id")
                .bind(user_id)
                .bind(at)
                .fetch_optional(&self.core.db)
                .await?;
        let Some(gathering_id) = gathering_id else { return Ok(()) };
        // A gathering on the move takes its (approximate) position along:
        // membership changes are serialized by where it is.
        sqlx::query("UPDATE geo_gatherings SET lat = $2, lng = $3 WHERE id = $1 AND ended_at IS NULL")
            .bind(gathering_id)
            .bind(position.0)
            .bind(position.1)
            .execute(&self.core.db)
            .await?;
        if let Some((row, members)) = self.gathering(gathering_id).await? {
            let ids: Vec<Uuid> = members.iter().map(|(id, _)| *id).collect();
            self.credit_gathering(&row, &ids, Utc::now(), false).await?;
        }
        Ok(())
    }

    /// Takes out members who stopped reporting and ends what is left with
    /// fewer than two people. Returns how many gatherings ended.
    pub async fn expire_gatherings(&self) -> AppResult<u64> {
        let stale: Vec<(Uuid, Uuid, f64, f64)> = sqlx::query_as(
            "SELECT m.gathering_id, m.user_id, g.lat, g.lng FROM geo_gathering_members m JOIN geo_gatherings g ON g.id = m.gathering_id
             WHERE m.left_at IS NULL AND m.last_seen_at < now() - make_interval(mins => $1) ORDER BY m.gathering_id LIMIT 500",
        )
        .bind(STALE_MINUTES)
        .fetch_all(&self.core.db)
        .await?;
        let mut ended = 0;
        for (gathering_id, user_id, lat, lng) in stale {
            let mut changes = Changes::default();
            let mut tx = self.core.db.begin().await?;
            lock_area(&mut tx, (lat, lng)).await?;
            // (Under the lock: still silent?)
            let still: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM geo_gathering_members WHERE gathering_id = $1 AND user_id = $2 AND left_at IS NULL
                               AND last_seen_at < now() - make_interval(mins => $3))",
            )
            .bind(gathering_id)
            .bind(user_id)
            .bind(STALE_MINUTES)
            .fetch_one(&mut *tx)
            .await?;
            if still {
                leave(&mut tx, gathering_id, user_id, Utc::now(), &mut changes).await?;
            }
            tx.commit().await?;
            ended += changes.ended.len() as u64;
            self.publish_gathering_changes(changes).await?;
        }
        Ok(ended)
    }

    /// Takes a user out of their gathering at once (sharing stopped, account deleted).
    pub(crate) async fn leave_gathering(&self, user_id: Uuid) -> AppResult<()> {
        let changes = self.sync_gathering(user_id, &[], (0.0, 0.0), Utc::now()).await?;
        self.publish_gathering_changes(changes).await
    }

    /// The gatherings `viewer` can see: their own and their friends', each
    /// as the viewer sees it.
    pub(crate) async fn list_gatherings(&self, viewer: Uuid) -> AppResult<Vec<Gathering>> {
        let mut people = self.precise_friends(viewer).await?;
        people.push(viewer);
        let rows: Vec<GatheringRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {GATHERING_COLUMNS} FROM geo_gatherings
             WHERE ended_at IS NULL AND id IN (SELECT gathering_id FROM geo_gathering_members WHERE left_at IS NULL AND user_id = ANY($1))
             ORDER BY started_at"
        )))
        .bind(&people)
        .fetch_all(&self.core.db)
        .await?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let blocked = graph::blocked_ids_either(&self.core, viewer).await?;
        let now = Utc::now();
        let mut out = Vec::new();
        for row in rows {
            let members: Vec<(Uuid, DateTime<Utc>)> =
                sqlx::query_as("SELECT user_id, arrived_at FROM geo_gathering_members WHERE gathering_id = $1 AND left_at IS NULL ORDER BY arrived_at, user_id")
                    .bind(row.id)
                    .fetch_all(&self.core.db)
                    .await?;
            let scene = self.scene(&row, &members).await?;
            if let Some(view) = scene.view(viewer, &blocked, now) {
                self.mark_shown(row.id, viewer, &view).await?;
                out.push(self.gathering_message(row.id, &view).await?);
            }
        }
        Ok(out)
    }

    /// The friends `user_id` was in a gathering with between `from` and
    /// `to`, by their own proximity check (people who saw each other's
    /// precise location at the time).
    pub(crate) async fn met_in_gatherings(
        &self,
        user_id: Uuid,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> AppResult<Vec<Uuid>> {
        Ok(sqlx::query_scalar(
            "SELECT DISTINCT unnest(with_user_ids) FROM geo_gathering_members
             WHERE user_id = $1 AND arrived_at < $3 AND COALESCE(left_at, now()) >= $2",
        )
        .bind(user_id)
        .bind(from)
        .bind(to)
        .fetch_all(&self.core.db)
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    #[test]
    fn time_together_is_credited_in_steps() {
        let start = Utc::now();
        assert_eq!(credit(start, start + Duration::minutes(4), false), None, "not worth a report yet");
        assert_eq!(
            credit(start, start + Duration::seconds(7 * 60 + 40), false),
            Some((7, start + Duration::minutes(7))),
            "whole minutes; the rest carries over"
        );
        assert_eq!(credit(start, start + Duration::minutes(20), false), Some((20, start + Duration::minutes(20))));
    }

    #[test]
    fn a_long_silence_is_credited_only_up_to_the_cap() {
        let start = Utc::now();
        let later = start + Duration::hours(3);
        assert_eq!(credit(start, later, false), Some((20, later)));
    }

    #[test]
    fn what_is_left_counts_when_a_gathering_ends() {
        let start = Utc::now();
        assert_eq!(credit(start, start + Duration::minutes(2), true), Some((2, start + Duration::minutes(2))));
        assert_eq!(credit(start, start + Duration::seconds(40), true), None);
    }

    #[test]
    fn changes_track_each_gathering_once() {
        let mut changes = Changes::default();
        assert!(changes.is_empty());
        let id = Uuid::now_v7();
        changes.touch(id);
        changes.touch(id);
        assert_eq!(changes.updated, vec![id]);
        assert!(!changes.is_empty());
    }

    use crate::ctx::{GhostRule, OwnPlace};

    fn member(id: Uuid, minutes_ago: i64, position: Point, friends: &[Uuid]) -> SceneMember {
        SceneMember {
            id,
            arrived_at: Utc::now() - Duration::minutes(minutes_ago),
            position: Some(position),
            ctx: OwnerCtx { active: true, ..Default::default() },
            friends: friends.iter().copied().collect(),
        }
    }

    #[test]
    fn a_viewer_gets_the_members_they_see_and_nothing_shaped_by_the_others() {
        let (aoi, ren, kai, mio, stranger) =
            (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let spot = (35.6595, 139.7005);
        let (aois, rens, kais) =
            (spot, math::destination(spot.0, spot.1, 90.0, 20.0), math::destination(spot.0, spot.1, 270.0, 25.0));
        // Kai was there first, with Aoi; Ren came later. Kai is blurred for Mio.
        let mut hidden = member(kai, 30, kais, &[aoi, ren, mio]);
        hidden
            .ctx
            .ghost
            .overrides
            .insert(mio, GhostRule { mode: 2, since_ms: 0, until_ms: None, frozen: None, hidden_since_ms: 0 });
        // The spot is inside Kai's home, and a café Kai shares with friends.
        hidden.ctx.places.push(OwnPlace {
            id: Uuid::now_v7(),
            label: 1,
            name: "うち".into(),
            emoji: String::new(),
            lat: kais.0,
            lng: kais.1,
            radius_m: 50.0,
            visibility: 5,
        });
        let scene = Scene {
            id: Uuid::now_v7(),
            members: vec![hidden, member(aoi, 20, aois, &[ren, kai, mio]), member(ren, 5, rens, &[aoi, kai, mio])],
            places: vec![ScenePlace {
                name: "カイの店".into(),
                center: spot,
                radius_m: 60.0,
                owner_id: kai,
                public: false,
            }],
            radius_m: 30.0,
        };
        let now = Utc::now();

        // A member sees everyone (all three are friends and precise for each other).
        let full = scene.view(aoi, &[], now).unwrap();
        assert_eq!(full.members.len(), 3);
        assert_eq!(full.started_at, scene.members[1].arrived_at, "when the second of them arrived");
        assert_eq!(full.members[0].1, full.started_at, "nobody is shown as there before it began");
        assert!(full.at_private_place);
        assert_eq!(full.place_name, "カイの店");

        // Mio sees Aoi and Ren. Nothing in her view depends on Kai:
        let seen = scene.view(mio, &[], now).unwrap();
        assert_eq!(seen.members.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![aoi, ren]);
        // the centre is the middle of the two she sees,
        assert_eq!(seen.center, ((aois.0 + rens.0) / 2.0, (aois.1 + rens.1) / 2.0));
        assert_ne!(seen.center, full.center);
        // it began when Ren joined Aoi, and Aoi was not "there before",
        assert_eq!(seen.started_at, scene.members[2].arrived_at);
        assert!(seen.members.iter().all(|(_, arrived)| *arrived == seen.started_at));
        // and neither Kai's home nor Kai's friends-only place is hinted at.
        assert!(!seen.at_private_place);
        assert_eq!(seen.place_name, "");
        // Take Kai out of the scene altogether: Mio's view is the same, field for field.
        let without = Scene { members: scene.members[1..].to_vec(), ..scene.clone() };
        assert_eq!(without.view(mio, &[], now), Some(seen));

        // One visible member is no gathering; a stranger sees none at all.
        let mut alone = scene.clone();
        alone.members.truncate(2);
        assert_eq!(alone.view(mio, &[], now), None);
        assert_eq!(scene.view(stranger, &[], now), None);
        // A member's home shows as "private" only if they share place labels.
        let mut shy = scene.clone();
        shy.members[0].ctx.privacy.share_place_labels = false;
        assert!(!shy.view(aoi, &[], now).unwrap().at_private_place);
        assert!(shy.view(kai, &[], now).unwrap().at_private_place, "one's own home is no secret to oneself");
        // Two people the viewer sees who are only connected through someone
        // the viewer does not see are not a gathering for that viewer.
        let (west, east) =
            (math::destination(spot.0, spot.1, 270.0, 40.0), math::destination(spot.0, spot.1, 90.0, 40.0));
        let mut bridge = member(kai, 30, spot, &[aoi, ren, mio]);
        bridge
            .ctx
            .ghost
            .overrides
            .insert(mio, GhostRule { mode: 3, since_ms: 0, until_ms: None, frozen: None, hidden_since_ms: 0 });
        let bridged = Scene {
            id: Uuid::now_v7(),
            members: vec![bridge, member(aoi, 20, west, &[ren, kai, mio]), member(ren, 5, east, &[aoi, kai, mio])],
            places: Vec::new(),
            radius_m: 30.0,
        };
        assert_eq!(bridged.view(aoi, &[], now).unwrap().members.len(), 3, "the members see all of it");
        assert_eq!(bridged.view(mio, &[], now), None, "80 m apart, with nobody she sees in between");

        // A public place is named unless its owner and the viewer are in a block.
        let mut public = scene.clone();
        public.places =
            vec![ScenePlace { name: "駅前".into(), center: spot, radius_m: 60.0, owner_id: stranger, public: true }];
        assert_eq!(public.view(mio, &[], now).unwrap().place_name, "駅前");
        assert_eq!(public.view(mio, &[stranger], now).unwrap().place_name, "");
    }
}
