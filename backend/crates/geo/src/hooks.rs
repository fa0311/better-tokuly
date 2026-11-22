//! What the geo domain plugs into other domains: profile fields (distance,
//! explored share, countries), the subject resolver for places (comments,
//! reactions, reports) and the `geo` section of a personal data export.
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokuly_core::{
    export::DataExporter,
    prelude::*,
    subject::{SubjectInfo, SubjectResolver},
};
use tokuly_identity::ProfileExtension;
use tokuly_media::assets::image_from_json;
use tokuly_proto::identity::Profile;
use tokuly_social::graph;

use crate::{Geo, math, places::PUBLIC};

/// Raw points included in an export at most (the newest ones).
const EXPORT_POINTS: usize = 50_000;
/// Footprint cells included in an export at most.
const EXPORT_CELLS: i64 = 100_000;

#[async_trait]
impl ProfileExtension for Geo {
    /// Adds the explored share of the home country and the number of
    /// countries visited to every profile, and for friends the distance to
    /// the viewer — to where the viewer is allowed to see the owner: the true
    /// position, or the blurred point; never to a frozen pin.
    async fn enrich_profile(&self, viewer: Option<Uuid>, owner: Uuid, profile: &mut Profile) -> AppResult<()> {
        let (explored_percent, country_count) = self.explored_summary(owner).await?;
        let stats = profile.stats.get_or_insert_default();
        stats.explored_percent = explored_percent;
        stats.visited_country_count = country_count;

        let Some(viewer) = viewer.filter(|viewer| *viewer != owner) else { return Ok(()) };
        if !graph::are_friends(&self.core, viewer, owner).await? {
            return Ok(());
        }
        // Both must be sharing: a paused or unknown pin on either side gives no distance.
        let states = self.load_states(&[viewer, owner]).await?;
        let sharing = |index: usize| {
            states[index]
                .as_ref()
                .filter(|state| state.located() && state.pin.sharing)
                .map(|state| state.pin.position())
        };
        let (Some(mine), Some(_)) = (sharing(0), sharing(1)) else { return Ok(()) };
        if let Some((theirs, _)) = self.seen_position(owner, viewer).await? {
            profile.distance_meters = Some(math::distance(mine, theirs).round());
        }
        Ok(())
    }
}

/// Resolves `SUBJECT_TYPE_PLACE` for the generic domains.
pub(crate) struct PlaceSubjects(pub Geo);

#[async_trait]
impl SubjectResolver for PlaceSubjects {
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let Ok(place_id) = Uuid::parse_str(id) else { return Ok(None) };
        let Some(place) = self.0.place_row(place_id).await? else { return Ok(None) };
        let visible = match viewer {
            Some(viewer) => self.0.can_see_place(viewer, &place).await?,
            None => place.visibility == PUBLIC,
        };
        Ok(visible.then(|| SubjectInfo {
            id: place.id,
            owner_id: place.owner_id,
            title: place.name.clone(),
            path: format!("/places/{}", place.id),
            image_url: image_from_json(place.photo.as_ref()).map(|image| image.url),
            interactions_enabled: true,
        }))
    }
}

/// The `geo` section of a personal data export.
pub(crate) struct GeoExporter(pub Geo);

impl GeoExporter {
    /// Runs a query that aggregates its rows into one JSON array.
    async fn list(&self, sql: &'static str, user_id: Uuid) -> AppResult<serde_json::Value> {
        Ok(sqlx::query_scalar(sql).bind(user_id).fetch_one(&self.0.core.db).await?)
    }
}

#[async_trait]
impl DataExporter for GeoExporter {
    fn name(&self) -> &'static str {
        "geo"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        let geo = &self.0;
        geo.settle_odometer(user_id).await?;
        geo.flush_history().await?;
        let state = geo.load_state(user_id).await?;
        let last_position = state.as_ref().filter(|state| state.located()).map(|state| {
            serde_json::json!({
                "lat": state.pin.lat, "lng": state.pin.lng, "accuracy_meters": state.pin.accuracy,
                "recorded_at": DateTime::from_timestamp_millis(state.pin.recorded_ms), "sharing": state.pin.sharing,
                "locality": state.pin.area.locality, "country_code": state.pin.area.country_code,
            })
        });
        let cells: Vec<i64> =
            sqlx::query_scalar("SELECT cell FROM geo_cells WHERE user_id = $1 AND res = 9 ORDER BY first_at LIMIT $2")
                .bind(user_id)
                .bind(EXPORT_CELLS)
                .fetch_all(&geo.core.db)
                .await?;
        let totals: Option<(i64, f64)> =
            sqlx::query_as("SELECT cell_count, explored_km2 FROM geo_users WHERE user_id = $1")
                .bind(user_id)
                .fetch_optional(&geo.core.db)
                .await?;
        // Raw history: the newest points, oldest first. A failing ClickHouse must not fail the whole export.
        let now = Utc::now();
        let (point_count, points) = match geo.point_count(user_id, DateTime::UNIX_EPOCH, now).await {
            Ok(count) => {
                let rows = geo.points(user_id, DateTime::UNIX_EPOCH, now, EXPORT_POINTS).await.unwrap_or_default();
                let points: Vec<serde_json::Value> = rows
                    .iter()
                    .map(|row| serde_json::json!({ "at": row.recorded_at, "lat": row.lat, "lng": row.lng, "accuracy_meters": row.accuracy, "speed_mps": row.speed, "activity": row.activity }))
                    .collect();
                (serde_json::json!(count), serde_json::json!(points))
            }
            Err(_) => {
                (serde_json::Value::Null, serde_json::json!({ "error": "raw history is temporarily unavailable" }))
            }
        };
        Ok(serde_json::json!({
            "last_position": last_position,
            "ghost_rules": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('viewer_id', NULLIF(viewer_id, '00000000-0000-0000-0000-000000000000'::uuid),
                     'mode', mode, 'since', since, 'until', until) ORDER BY since), '[]'::jsonb)
                 FROM geo_ghost_rules WHERE owner_id = $1", user_id).await?,
            "places": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('id', id, 'name', name, 'emoji', emoji, 'label', label, 'lat', lat, 'lng', lng,
                     'radius_meters', radius_m, 'visibility', visibility, 'address', address, 'category', category, 'created_at', created_at)
                     ORDER BY created_at), '[]'::jsonb)
                 FROM geo_places WHERE owner_id = $1", user_id).await?,
            "saved_places": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('place_id', place_id, 'saved_at', created_at) ORDER BY created_at), '[]'::jsonb)
                 FROM geo_saved_places WHERE user_id = $1", user_id).await?,
            "visits": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('id', id, 'place_id', place_id, 'place_name', place_name, 'lat', lat, 'lng', lng,
                     'arrived_at', arrived_at, 'departed_at', departed_at, 'with_user_ids', with_user_ids, 'check_in', check_in, 'message', message,
                     'locality', locality) ORDER BY arrived_at), '[]'::jsonb)
                 FROM geo_visits WHERE user_id = $1", user_id).await?,
            "nights": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('night', night, 'place_name', place_name, 'lat', lat, 'lng', lng, 'from', started_at,
                     'to', ended_at, 'with_user_ids', with_user_ids, 'at_home', at_home) ORDER BY night), '[]'::jsonb)
                 FROM geo_nights WHERE user_id = $1", user_id).await?,
            "day_memos": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('day', day, 'text', text, 'photos', photos, 'updated_at', updated_at) ORDER BY day), '[]'::jsonb)
                 FROM geo_day_memos WHERE user_id = $1", user_id).await?,
            "movement_by_hour": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('hour', hour, 'activity', activity, 'distance_meters', distance_m, 'duration_seconds', duration_s,
                     'max_speed_mps', max_speed_mps, 'locality', locality, 'country_code', country_code) ORDER BY hour, activity), '[]'::jsonb)
                 FROM geo_activity_hours WHERE user_id = $1", user_id).await?,
            "footprints": {
                "cell_count": totals.map(|t| t.0).unwrap_or(0),
                "explored_km2": totals.map(|t| t.1).unwrap_or(0.0),
                "h3_resolution": 9,
                "cells": cells.into_iter().map(math::cell_hex).collect::<Vec<_>>(),
                "regions": self.list(
                    "SELECT COALESCE(jsonb_agg(jsonb_build_object('region_id', region_id, 'cell_count', cell_count, 'explored_km2', explored_km2,
                         'first_at', first_at) ORDER BY first_at), '[]'::jsonb)
                     FROM geo_region_stats WHERE user_id = $1", user_id).await?,
            },
            "trips": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('id', id, 'destination', dest_name, 'dest_lat', dest_lat, 'dest_lng', dest_lng,
                     'dest_user_id', dest_user_id, 'mode', mode, 'status', status, 'started_at', started_at, 'ended_at', ended_at,
                     'shared_with', shared_with) ORDER BY started_at), '[]'::jsonb)
                 FROM geo_trips WHERE user_id = $1", user_id).await?,
            "place_alerts_set": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('id', id, 'subject_id', subject_id, 'place_name', place_name, 'lat', lat, 'lng', lng,
                     'radius_meters', radius_m, 'subject_place_label', subject_place_label, 'trigger', trigger, 'recurring', recurring,
                     'status', status, 'created_at', created_at, 'last_fired_at', last_fired_at) ORDER BY created_at), '[]'::jsonb)
                 FROM geo_place_alerts WHERE owner_id = $1", user_id).await?,
            "place_alerts_about_me": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('id', id, 'owner_id', owner_id, 'place_name', place_name, 'trigger', trigger,
                     'recurring', recurring, 'status', status, 'created_at', created_at) ORDER BY created_at), '[]'::jsonb)
                 FROM geo_place_alerts WHERE subject_id = $1", user_id).await?,
            "sos": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('id', id, 'message', message, 'notified_user_ids', notify_user_ids,
                     'started_at', started_at, 'resolved_at', resolved_at) ORDER BY started_at), '[]'::jsonb)
                 FROM geo_sos WHERE user_id = $1", user_id).await?,
            "live_shares": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('id', id, 'label', label, 'created_at', created_at, 'expires_at', expires_at,
                     'ended_at', ended_at, 'view_count', view_count) ORDER BY created_at), '[]'::jsonb)
                 FROM geo_live_shares WHERE user_id = $1", user_id).await?,
            "gatherings": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('gathering_id', m.gathering_id, 'arrived_at', m.arrived_at, 'left_at', m.left_at,
                     'lat', g.lat, 'lng', g.lng, 'place_name', g.place_name) ORDER BY m.arrived_at), '[]'::jsonb)
                 FROM geo_gathering_members m JOIN geo_gatherings g ON g.id = m.gathering_id WHERE m.user_id = $1", user_id).await?,
            "raw_point_count": point_count,
            "raw_points": points,
        }))
    }
}
