//! Gatherings: friends in the same spot ("N人が集合中!").
//!
//! Each user's [`crate::detect::ProximityTracker`] decides which friends they
//! are together with. This module turns those pairwise facts into gatherings
//! with stable IDs: the first two people found together found one, others
//! join it, two gatherings whose members meet merge into the older one, and
//! the last-but-one leaving ends it.
//!
//! A gathering is a server-side fact about true positions. What a viewer
//! gets to see of it goes through the same rule as everything else: only
//! members who are their friends *and* whose precise location they may see
//! ([`Geo::precise_friends`]); with fewer than two such members (counting the
//! viewer), there is no gathering as far as that viewer is concerned.
use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use tokuly_core::{
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::geo::{Gathering, GatheringMember, geo_event::Event};
use tokuly_social::{InteractionKind, graph};

use crate::{
    Geo,
    math::{self, Point},
    places::{FRIENDS, LABEL_CUSTOM, PUBLIC, PlaceRow},
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

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct GatheringRow {
    pub id: Uuid,
    pub lat: f64,
    pub lng: f64,
    pub place_id: Option<Uuid>,
    pub place_name: String,
    pub at_private_place: bool,
    pub started_at: DateTime<Utc>,
    pub credited_until: DateTime<Utc>,
}

const GATHERING_COLUMNS: &str = "id, lat, lng, place_id, place_name, at_private_place, started_at, credited_until";

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

type Tx<'a> = sqlx::Transaction<'a, sqlx::Postgres>;

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

impl Geo {
    /// Brings the gatherings in line with who `user_id` is together with
    /// right now (`together`: friends near for long enough, from their
    /// proximity tracker). `position` is where the user is.
    pub(crate) async fn sync_gathering(
        &self,
        user_id: Uuid,
        together: &[Uuid],
        position: Point,
        at: DateTime<Utc>,
    ) -> AppResult<Changes> {
        let mut changes = Changes::default();
        let mut tx = self.core.db.begin().await?;
        // One membership change at a time: founding, joining and merging must
        // not interleave (changes are rare and quick).
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('geo:gatherings', 0))").execute(&mut *tx).await?;
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
        sqlx::query("UPDATE geo_gathering_members SET last_seen_at = $3 WHERE gathering_id = $1 AND user_id = $2")
            .bind(target)
            .bind(user_id)
            .bind(at)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(changes)
    }

    /// After a membership change: describes the spot, announces a new
    /// gathering, reports time together, and tells everyone who can see it.
    pub(crate) async fn publish_gathering_changes(&self, changes: Changes) -> AppResult<()> {
        let now = Utc::now();
        for (row, members) in &changes.ended {
            self.credit_gathering(row, members, now, true).await?;
            let audience = self.gathering_audience(members).await?;
            self.push_many(audience, Event::GatheringEndedId(row.id.to_string())).await;
        }
        let mut current = changes.updated.clone();
        current.extend(changes.formed.filter(|formed| !changes.updated.contains(formed)));
        for gathering_id in current {
            let Some((row, members)) = self.gathering(gathering_id).await? else { continue };
            let row = self.describe_gathering(row, &members).await?;
            if changes.formed == Some(row.id) {
                self.announce_gathering(&row, &members).await?;
            }
            self.push_gathering(&row, &members).await?;
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

    /// Recomputes where a gathering is and what kind of spot that is.
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
               AND ST_Distance(p.geog, ST_SetSRID(ST_MakePoint($2, $1), 4326)::geography) <= p.radius_m + 20
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

    /// Members and all their friends: everyone who might see a gathering.
    async fn gathering_audience(&self, members: &[Uuid]) -> AppResult<Vec<Uuid>> {
        let mut audience: HashSet<Uuid> = members.iter().copied().collect();
        for member in members {
            audience.extend(graph::friend_ids(&self.core, *member).await?);
        }
        Ok(audience.into_iter().collect())
    }

    /// A gathering as `viewer` sees it: members who are the viewer or friends
    /// whose precise location the viewer may see. `None` with fewer than two.
    async fn gathering_for(
        &self,
        viewer: Uuid,
        visible_friends: &HashSet<Uuid>,
        row: &GatheringRow,
        members: &[(Uuid, DateTime<Utc>)],
        place: Option<&PlaceRow>,
    ) -> AppResult<Option<Gathering>> {
        let shown: Vec<&(Uuid, DateTime<Utc>)> =
            members.iter().filter(|(id, _)| *id == viewer || visible_friends.contains(id)).collect();
        if shown.len() < 2 {
            return Ok(None);
        }
        let users =
            tokuly_identity::summaries(&self.core, &shown.iter().map(|(id, _)| *id).collect::<Vec<_>>()).await?;
        // The place is named only to viewers who could see it themselves.
        let place_name = match place {
            Some(place) if self.can_see_place(viewer, place).await? => place.name.clone(),
            _ => String::new(),
        };
        Ok(Some(Gathering {
            id: row.id.to_string(),
            center: math::lat_lng(row.lat, row.lng).into(),
            members: shown
                .iter()
                .map(|(id, arrived_at)| GatheringMember {
                    user: tokuly_identity::users::pick(&users, *id).into(),
                    arrived_at: ts(*arrived_at),
                    ..Default::default()
                })
                .collect(),
            place_name,
            started_at: ts(row.started_at),
            at_private_place: row.at_private_place,
            ..Default::default()
        }))
    }

    /// Pushes a gathering to everyone who can see it — and its end to those who cannot (any more).
    async fn push_gathering(&self, row: &GatheringRow, members: &[(Uuid, DateTime<Utc>)]) -> AppResult<()> {
        let ids: Vec<Uuid> = members.iter().map(|(id, _)| *id).collect();
        let place = match row.place_id {
            Some(place_id) => self.place_row(place_id).await?,
            None => None,
        };
        for viewer in self.gathering_audience(&ids).await? {
            let visible: HashSet<Uuid> = self.precise_friends(viewer).await?.into_iter().collect();
            match self.gathering_for(viewer, &visible, row, members, place.as_ref()).await? {
                Some(gathering) => self.push(viewer, Event::GatheringUpdated(Box::new(gathering))).await,
                None => self.push(viewer, Event::GatheringEndedId(row.id.to_string())).await,
            }
        }
        Ok(())
    }

    /// Notifies mutual friends that two (or more) of their friends are
    /// together — unless they are at someone's home, work or school, or were
    /// announced together a short while ago.
    async fn announce_gathering(&self, row: &GatheringRow, members: &[(Uuid, DateTime<Utc>)]) -> AppResult<()> {
        if row.at_private_place || members.len() < 2 {
            return Ok(());
        }
        let ids: Vec<Uuid> = members.iter().map(|(id, _)| *id).collect();
        let (first, second) = (ids[0].min(ids[1]), ids[0].max(ids[1]));
        if !self
            .core
            .cache
            .set_nx(&format!("geo:gathering:announced:{}:{}", first.simple(), second.simple()), "1", ANNOUNCE_COOLDOWN)
            .await?
        {
            return Ok(());
        }
        // Friends of at least two members, who may see at least two of them precisely.
        let mut seen_by: HashMap<Uuid, usize> = HashMap::new();
        let contexts = self.ctx_many(&ids).await?;
        let now = Utc::now();
        for member in &ids {
            let Some(ctx) = contexts.get(member) else { continue };
            for friend in graph::friend_ids(&self.core, *member).await? {
                if !ids.contains(&friend) && visibility::sees_precisely(ctx, friend, now) {
                    *seen_by.entry(friend).or_default() += 1;
                }
            }
        }
        let mut recipients: Vec<Uuid> =
            seen_by.into_iter().filter(|(_, count)| *count >= 2).map(|(friend, _)| friend).collect();
        recipients.sort();
        recipients.truncate(MAX_ANNOUNCED);
        if recipients.is_empty() {
            return Ok(());
        }
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
        let body = if row.place_name.is_empty() {
            Localized::new("マップで見てみよう!", "See them on the map!")
        } else {
            Localized::new(format!("📍{}", row.place_name), format!("📍{}", row.place_name))
        };
        let mut tx = self.core.db.begin().await?;
        for recipient in recipients {
            let notification = NotificationRequest::new(
                recipient,
                "geo.gathering",
                title.clone(),
                body.clone(),
                format!("/map?gathering={}", row.id),
            )
            .actor(ids[0])
            .collapse(format!("geo.gathering:{}", row.id))
            .data(serde_json::json!({ "gathering_id": row.id, "user_ids": ids }));
            outbound::notify(&mut *tx, &notification).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Reports time together to the social domain for every pair of members,
    /// once per stretch (several workers may try; one wins).
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
        for (index, a) in members.iter().enumerate() {
            for b in &members[index + 1..] {
                // (Pairs who are not friends are ignored by the social domain.)
                graph::record_interaction(&self.core, *a.min(b), *a.max(b), InteractionKind::TimeTogether, minutes)
                    .await?;
            }
        }
        Ok(())
    }

    /// Confirms that `user_id` is still in their gathering and reports time
    /// together when enough has passed. Called by the pipeline about once a
    /// minute for members.
    pub(crate) async fn gathering_heartbeat(&self, user_id: Uuid, at: DateTime<Utc>) -> AppResult<()> {
        let gathering_id: Option<Uuid> =
            sqlx::query_scalar("UPDATE geo_gathering_members SET last_seen_at = $2 WHERE user_id = $1 AND left_at IS NULL RETURNING gathering_id")
                .bind(user_id)
                .bind(at)
                .fetch_optional(&self.core.db)
                .await?;
        let Some(gathering_id) = gathering_id else { return Ok(()) };
        if let Some((row, members)) = self.gathering(gathering_id).await? {
            let ids: Vec<Uuid> = members.iter().map(|(id, _)| *id).collect();
            self.credit_gathering(&row, &ids, Utc::now(), false).await?;
        }
        Ok(())
    }

    /// Takes out members who stopped reporting and ends what is left with
    /// fewer than two people. Returns how many gatherings ended.
    pub async fn expire_gatherings(&self) -> AppResult<u64> {
        let stale: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT gathering_id, user_id FROM geo_gathering_members WHERE left_at IS NULL AND last_seen_at < now() - make_interval(mins => $1) LIMIT 500",
        )
        .bind(STALE_MINUTES)
        .fetch_all(&self.core.db)
        .await?;
        if stale.is_empty() {
            return Ok(0);
        }
        let mut changes = Changes::default();
        let mut tx = self.core.db.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('geo:gatherings', 0))").execute(&mut *tx).await?;
        let now = Utc::now();
        for (gathering_id, user_id) in stale {
            leave(&mut tx, gathering_id, user_id, now, &mut changes).await?;
        }
        tx.commit().await?;
        let ended = changes.ended.len() as u64;
        self.publish_gathering_changes(changes).await?;
        Ok(ended)
    }

    /// Takes a user out of their gathering at once (sharing stopped, account deleted).
    pub(crate) async fn leave_gathering(&self, user_id: Uuid) -> AppResult<()> {
        let changes = self.sync_gathering(user_id, &[], (0.0, 0.0), Utc::now()).await?;
        self.publish_gathering_changes(changes).await
    }

    /// The gatherings `viewer` can see: their own and their friends'.
    pub(crate) async fn list_gatherings(&self, viewer: Uuid) -> AppResult<Vec<Gathering>> {
        let visible: HashSet<Uuid> = self.precise_friends(viewer).await?.into_iter().collect();
        let people: Vec<Uuid> = visible.iter().copied().chain([viewer]).collect();
        let rows: Vec<GatheringRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {GATHERING_COLUMNS} FROM geo_gatherings
             WHERE ended_at IS NULL AND id IN (SELECT gathering_id FROM geo_gathering_members WHERE left_at IS NULL AND user_id = ANY($1))
             ORDER BY started_at"
        )))
        .bind(&people)
        .fetch_all(&self.core.db)
        .await?;
        let mut out = Vec::new();
        for row in rows {
            let members: Vec<(Uuid, DateTime<Utc>)> =
                sqlx::query_as("SELECT user_id, arrived_at FROM geo_gathering_members WHERE gathering_id = $1 AND left_at IS NULL ORDER BY arrived_at, user_id")
                    .bind(row.id)
                    .fetch_all(&self.core.db)
                    .await?;
            let place = match row.place_id {
                Some(place_id) => self.place_row(place_id).await?,
                None => None,
            };
            if let Some(gathering) = self.gathering_for(viewer, &visible, &row, &members, place.as_ref()).await? {
                out.push(gathering);
            }
        }
        Ok(out)
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
}
