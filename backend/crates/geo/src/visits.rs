//! Visits (stays, matched to places) and nights: what the pipeline writes
//! when a stay begins and ends, and how visit rows become API messages.
use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use tokuly_core::prelude::*;
use tokuly_proto::geo::{Place, Visit};
use tokuly_social::graph;

use crate::{
    Geo,
    ctx::OwnerCtx,
    detect::night_of,
    fanout::VisitNews,
    geocode::Gazetteer,
    math,
    places::{LABEL_HOME, PLACE_COLUMNS, PlaceRow},
    state::LiveState,
};

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct VisitRow {
    pub id: Uuid,
    pub place_id: Option<Uuid>,
    pub place_name: String,
    pub lat: f64,
    pub lng: f64,
    pub arrived_at: DateTime<Utc>,
    pub departed_at: Option<DateTime<Utc>>,
    pub with_user_ids: Vec<Uuid>,
    pub check_in: bool,
    pub message: String,
    pub locality: String,
    pub locality_ja: String,
}

pub(crate) const VISIT_COLUMNS: &str = "id, place_id, place_name, lat, lng, arrived_at, departed_at, with_user_ids, check_in, message, locality, locality_ja";

fn from_millis(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap_or_default()
}

/// The time zone at a stay, for telling nights from days.
fn zone_of(state: &LiveState, ctx: &OwnerCtx) -> Tz {
    state.pin.area.time_zone.parse().or_else(|_| ctx.time_zone.parse()).unwrap_or(chrono_tz::UTC)
}

impl Geo {
    /// A stay was confirmed: match it to a place and, when the user records
    /// history, open a visit.
    pub(crate) async fn begin_stay(&self, user_id: Uuid, ctx: &OwnerCtx, state: &mut LiveState, center: (f64, f64), since_ms: i64) -> AppResult<VisitNews> {
        let place = self.match_place(user_id, ctx, center).await?;
        let since = from_millis(since_ms);
        state.pin.stationary_since_ms = Some(since_ms);
        state.pin.stay_center = Some(center);
        state.pin.place = place.clone();
        state.visit_id = None;
        state.visit_with.clear();
        state.night = None;
        if ctx.privacy.record_history {
            let mut tx = self.core.db.begin().await?;
            // A visit still open can only be a leftover of a lost live state: close it.
            sqlx::query("UPDATE geo_visits SET departed_at = GREATEST(arrived_at, LEAST($2, now())) WHERE user_id = $1 AND departed_at IS NULL")
                .bind(user_id)
                .bind(since)
                .execute(&mut *tx)
                .await?;
            let visit_id: Uuid = sqlx::query_scalar(
                "INSERT INTO geo_visits (user_id, place_id, place_name, lat, lng, arrived_at, locality, locality_ja, country_code)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id",
            )
            .bind(user_id)
            .bind(place.as_ref().map(|place| place.id))
            .bind(place.as_ref().map_or("", |place| place.name.as_str()))
            .bind(center.0)
            .bind(center.1)
            .bind(since)
            .bind(&state.pin.area.locality)
            .bind(&state.pin.area.locality_ja)
            .bind(&state.pin.area.country_code)
            .fetch_one(&mut *tx)
            .await?;
            if let Some(place) = &place {
                sqlx::query("UPDATE geo_places SET visit_count = visit_count + 1 WHERE id = $1").bind(place.id).execute(&mut *tx).await?;
            }
            tx.commit().await?;
            state.visit_id = Some(visit_id);
        }
        Ok(VisitNews { arrived: true, at: since, place })
    }

    /// The stay ended: close the visit (and the night it may have been).
    pub(crate) async fn finish_stay(&self, user_id: Uuid, ctx: &OwnerCtx, state: &mut LiveState, until_ms: i64) -> AppResult<VisitNews> {
        let until = from_millis(until_ms);
        if let Some(visit_id) = state.visit_id {
            self.record_night(user_id, ctx, state, until, true).await?;
            sqlx::query("UPDATE geo_visits SET departed_at = GREATEST(arrived_at, $2) WHERE id = $1 AND departed_at IS NULL")
                .bind(visit_id)
                .bind(until)
                .execute(&self.core.db)
                .await?;
        }
        let place = state.pin.place.take();
        state.pin.stationary_since_ms = None;
        state.pin.stay_center = None;
        state.visit_id = None;
        state.visit_with.clear();
        state.night = None;
        Ok(VisitNews { arrived: false, at: until, place })
    }

    /// Records the night the ongoing stay covers, if it covers one. During
    /// the stay each night is written once, when it first qualifies;
    /// `closing` writes it again with the stay's final end.
    pub(crate) async fn record_night(&self, user_id: Uuid, ctx: &OwnerCtx, state: &mut LiveState, until: DateTime<Utc>, closing: bool) -> AppResult<()> {
        let (Some(visit_id), Some(since_ms), Some(center)) = (state.visit_id, state.pin.stationary_since_ms, state.pin.stay_center) else {
            return Ok(());
        };
        let Some(night) = night_of(from_millis(since_ms), until, zone_of(state, ctx)) else { return Ok(()) };
        if !closing && state.night == Some(night.date) {
            return Ok(());
        }
        let place = state.pin.place.as_ref();
        sqlx::query(
            "INSERT INTO geo_nights (user_id, night, visit_id, place_id, place_name, lat, lng, started_at, ended_at, with_user_ids, at_home, locality, locality_ja)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
             ON CONFLICT (user_id, night) DO UPDATE SET
                 visit_id = EXCLUDED.visit_id, place_id = EXCLUDED.place_id, place_name = EXCLUDED.place_name, lat = EXCLUDED.lat, lng = EXCLUDED.lng,
                 started_at = EXCLUDED.started_at, ended_at = EXCLUDED.ended_at, with_user_ids = EXCLUDED.with_user_ids, at_home = EXCLUDED.at_home,
                 locality = EXCLUDED.locality, locality_ja = EXCLUDED.locality_ja",
        )
        .bind(user_id)
        .bind(night.date)
        .bind(visit_id)
        .bind(place.map(|place| place.id))
        .bind(place.map_or("", |place| place.name.as_str()))
        .bind(center.0)
        .bind(center.1)
        .bind(night.from)
        .bind(night.to)
        .bind(&state.visit_with)
        .bind(place.is_some_and(|place| place.owner_id == user_id && place.label == LABEL_HOME))
        .bind(&state.pin.area.locality)
        .bind(&state.pin.area.locality_ja)
        .execute(&self.core.db)
        .await?;
        state.night = Some(night.date);
        Ok(())
    }

    /// Notes on the ongoing visit which friends are there with the user.
    pub(crate) async fn add_companions(&self, state: &mut LiveState, together: &[Uuid]) -> AppResult<()> {
        let Some(visit_id) = state.visit_id else { return Ok(()) };
        let new: Vec<Uuid> = together.iter().filter(|friend| !state.visit_with.contains(friend)).copied().collect();
        if new.is_empty() {
            return Ok(());
        }
        sqlx::query("UPDATE geo_visits SET with_user_ids = ARRAY(SELECT DISTINCT unnest(with_user_ids || $2)) WHERE id = $1")
            .bind(visit_id)
            .bind(&new)
            .execute(&self.core.db)
            .await?;
        state.visit_with.extend(new);
        Ok(())
    }

    /// Visit rows as messages for their owner: the place where it still
    /// exists and is visible to them, the friends who were there and are
    /// still friends, names in the owner's language.
    pub(crate) async fn visits_for(&self, user_id: Uuid, rows: &[VisitRow], ja: bool) -> AppResult<Vec<Visit>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let place_ids: Vec<Uuid> = rows.iter().filter_map(|row| row.place_id).collect::<HashSet<_>>().into_iter().collect();
        let place_rows: Vec<PlaceRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {PLACE_COLUMNS} FROM geo_places WHERE id = ANY($1)")))
            .bind(&place_ids)
            .fetch_all(&self.core.db)
            .await?;
        let mut visible = Vec::with_capacity(place_rows.len());
        for place in place_rows {
            if self.can_see_place(user_id, &place).await? {
                visible.push(place);
            }
        }
        let places: HashMap<Uuid, Place> = visible.iter().map(|row| row.id).zip(self.places_for(user_id, &visible).await?).collect();

        let friends: HashSet<Uuid> = graph::friend_ids(&self.core, user_id).await?.into_iter().collect();
        let companions: Vec<Uuid> = rows.iter().flat_map(|row| &row.with_user_ids).filter(|id| friends.contains(id)).copied().collect::<HashSet<_>>().into_iter().collect();
        let users = tokuly_identity::summaries(&self.core, &companions).await?;
        let now = Utc::now();
        Ok(rows
            .iter()
            .map(|row| {
                let locality = if ja && !row.locality_ja.is_empty() { &row.locality_ja } else { &row.locality };
                Visit {
                    id: row.id.to_string(),
                    place: row.place_id.and_then(|id| places.get(&id).cloned()).into(),
                    name: if row.place_name.is_empty() { locality.clone() } else { row.place_name.clone() },
                    center: math::lat_lng(row.lat, row.lng).into(),
                    arrived_at: ts(row.arrived_at),
                    departed_at: opt_ts(row.departed_at),
                    duration_seconds: (row.departed_at.unwrap_or(now) - row.arrived_at).num_seconds().max(0),
                    with_friends: row.with_user_ids.iter().filter_map(|id| users.get(id)).filter(|user| !user.deleted).cloned().collect(),
                    check_in: row.check_in,
                    message: row.message.clone(),
                    locality: locality.clone(),
                    ..Default::default()
                }
            })
            .collect())
    }

    /// The time zone at a point, falling back to UTC at sea.
    pub(crate) fn zone_at(point: (f64, f64)) -> Tz {
        Gazetteer::shared().time_zone(point.0, point.1).parse().unwrap_or(chrono_tz::UTC)
    }
}
