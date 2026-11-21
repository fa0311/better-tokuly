//! Trips ("on my way"): a destination, a live ETA, and arrival detection.
//! Also `TripService`.
//!
//! A trip's ETA is recomputed by the pipeline from the traveller's live
//! position ([`Geo::advance_trip`]): the remaining straight-line distance,
//! scaled by how the planned route compared to the straight line at the
//! start. A trip towards a friend follows them — as far as the traveller is
//! allowed to see them (ghost mode applies).
use std::time::Duration;

use chrono::{DateTime, Utc};
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::geo::{destination::Target, geo_event::Event, *};
use tokuly_social::graph;

use crate::{
    Geo,
    ctx::{OwnerCtx, TripCtx},
    geocode::Gazetteer,
    math::{self, Point},
    places::{self, PRIVATE},
    routing,
    state::LiveState,
};

const ACTIVE: i16 = TripStatus::TRIP_STATUS_ACTIVE as i16;
const ARRIVED: i16 = TripStatus::TRIP_STATUS_ARRIVED as i16;
const CANCELLED: i16 = TripStatus::TRIP_STATUS_CANCELLED as i16;

/// Arrival radius around a bare point, and around a friend.
const POINT_RADIUS_M: f64 = 60.0;
const FRIEND_RADIUS_M: f64 = 50.0;
const MAX_SHAREES: usize = 100;
/// Trip progress is stored and pushed at most this often...
const PROGRESS_EVERY_MS: i64 = 20_000;
/// ...unless the ETA moved by at least this much.
const ETA_JUMP_MS: i64 = 60_000;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct TripRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub dest_user_id: Option<Uuid>,
    pub dest_place_id: Option<Uuid>,
    pub dest_name: String,
    pub dest_lat: f64,
    pub dest_lng: f64,
    pub dest_hidden: bool,
    pub mode: i16,
    pub status: i16,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub eta: Option<DateTime<Utc>>,
    pub distance_remaining_m: f64,
    pub route_polyline: String,
    pub shared_with: Vec<Uuid>,
    pub notify_on_arrival: bool,
}

const TRIP_COLUMNS: &str = "id, user_id, dest_user_id, dest_place_id, dest_name, dest_lat, dest_lng, dest_hidden, mode, status, started_at, ended_at, eta, \
                            distance_remaining_m, route_polyline, shared_with, notify_on_arrival";

fn trip_not_found() -> AppError {
    AppError::not_found("TRIP_NOT_FOUND", "trip not found")
}

/// A destination resolved to coordinates.
struct Resolved {
    user_id: Option<Uuid>,
    place_id: Option<Uuid>,
    point: Point,
    name: String,
    arrive_radius_m: f64,
    /// Coordinates are for the traveller only.
    hidden: bool,
}

/// The ETA for what is left of a trip: the planned pace applied to the
/// remaining straight-line distance (a typical pace when there is no plan).
pub(crate) fn eta_seconds(trip: &TripCtx, remaining_m: f64) -> f64 {
    if trip.plan_distance_m > 1.0 && trip.plan_duration_s > 0 {
        remaining_m * trip.plan_duration_s as f64 / trip.plan_distance_m
    } else {
        let mode = routing::travel_mode(trip.mode.into());
        routing::typical_duration(mode, remaining_m)
    }
}

impl TripRow {
    /// The trip as `viewer` may see it. Where the destination is a friend or
    /// a private place, only the traveller gets coordinates and the route.
    fn message(&self, user: tokuly_proto::common::UserSummary, viewer: Uuid) -> Trip {
        let reveal = viewer == self.user_id || !self.dest_hidden || self.dest_user_id == Some(viewer);
        let target = match (self.dest_user_id, self.dest_place_id) {
            (Some(user_id), _) => Some(Target::UserId(user_id.to_string())),
            (None, Some(place_id)) if reveal => Some(Target::PlaceId(place_id.to_string())),
            (None, None) => Some(Target::Position(Box::new(math::lat_lng(self.dest_lat, self.dest_lng)))),
            _ => None,
        };
        Trip {
            id: self.id.to_string(),
            user: user.into(),
            destination: Destination {
                target,
                name: self.dest_name.clone(),
                resolved: reveal.then(|| math::lat_lng(self.dest_lat, self.dest_lng)).into(),
                ..Default::default()
            }
            .into(),
            mode: (self.mode as i32).into(),
            status: (self.status as i32).into(),
            started_at: ts(self.started_at),
            eta: opt_ts(self.eta),
            distance_remaining_meters: self.distance_remaining_m,
            route_polyline: if reveal { self.route_polyline.clone() } else { String::new() },
            ended_at: opt_ts(self.ended_at),
            shared_with_user_ids: if viewer == self.user_id { self.shared_with.iter().map(Uuid::to_string).collect() } else { Vec::new() },
            notify_on_arrival: self.notify_on_arrival,
            ..Default::default()
        }
    }
}

impl Geo {
    async fn trip_row(&self, trip_id: Uuid) -> AppResult<Option<TripRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {TRIP_COLUMNS} FROM geo_trips WHERE id = $1"))).bind(trip_id).fetch_optional(&self.core.db).await?)
    }

    async fn trip_message(&self, row: &TripRow, viewer: Uuid) -> AppResult<Trip> {
        Ok(row.message(tokuly_identity::summary(&self.core, row.user_id).await?, viewer))
    }

    /// The friends a trip is shared with (all of them when the list is empty).
    async fn trip_audience(&self, row: &TripRow) -> AppResult<Vec<Uuid>> {
        let friends = graph::friend_ids(&self.core, row.user_id).await?;
        Ok(if row.shared_with.is_empty() { friends } else { friends.into_iter().filter(|friend| row.shared_with.contains(friend)).collect() })
    }

    /// Pushes the trip to its traveller and audience, each in their own view.
    async fn push_trip(&self, row: &TripRow) -> AppResult<()> {
        let user = tokuly_identity::summary(&self.core, row.user_id).await?;
        for viewer in self.trip_audience(row).await?.into_iter().chain([row.user_id]) {
            self.push(viewer, Event::TripUpdated(Box::new(row.message(user.clone(), viewer)))).await;
        }
        Ok(())
    }

    async fn resolve_destination(&self, caller: Uuid, destination: Option<&Destination>, ja: bool) -> AppResult<Resolved> {
        let missing = || AppError::invalid("MISSING_DESTINATION", "a destination is required");
        let destination = destination.ok_or_else(missing)?;
        let given_name = destination.name.trim();
        ensure!(given_name.chars().count() <= 60 && !given_name.chars().any(char::is_control), invalid, "INVALID_NAME", "a destination name has at most 60 characters");
        let named = |fallback: String| if given_name.is_empty() { fallback } else { given_name.to_owned() };
        match destination.target.as_ref().ok_or_else(missing)? {
            Target::UserId(user_id) => {
                let friend = parse_id(user_id)?;
                ensure!(friend != caller && graph::are_friends(&self.core, caller, friend).await?, precondition, "NOT_FRIENDS", "you can only head to a friend");
                let (point, _) = self
                    .seen_position(friend, caller)
                    .await?
                    .ok_or_else(|| AppError::precondition("DESTINATION_UNAVAILABLE", "this friend's location is not available to you right now"))?;
                let name = tokuly_identity::summary(&self.core, friend).await?.display_name;
                Ok(Resolved { user_id: Some(friend), place_id: None, point, name: named(name), arrive_radius_m: FRIEND_RADIUS_M, hidden: true })
            }
            Target::PlaceId(place_id) => {
                let place = self.visible_place(caller, place_id).await?;
                Ok(Resolved {
                    user_id: None,
                    place_id: Some(place.id),
                    point: place.center(),
                    name: named(place.name.clone()),
                    arrive_radius_m: place.radius_m,
                    hidden: place.visibility == PRIVATE,
                })
            }
            Target::Position(position) => {
                let point = places::clean_point(Some(position))?;
                let locality = Gazetteer::shared().nearest(point.0, point.1).map(|found| found.locality(ja).to_owned()).unwrap_or_default();
                Ok(Resolved { user_id: None, place_id: None, point, name: named(locality), arrive_radius_m: POINT_RADIUS_M, hidden: false })
            }
        }
    }

    /// The caller's last known position, or `LOCATION_UNKNOWN`.
    async fn position_of(&self, user_id: Uuid) -> AppResult<Point> {
        self.load_state(user_id)
            .await?
            .filter(|state| state.located())
            .map(|state| state.pin.position())
            .ok_or_else(|| AppError::precondition("LOCATION_UNKNOWN", "your location is not known yet"))
    }

    /// Ends a trip with `status` and tells everyone watching it.
    async fn finish_trip(&self, trip_id: Uuid, status: i16) -> AppResult<Option<TripRow>> {
        let row: Option<TripRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE geo_trips SET status = $2, ended_at = now(),
                    distance_remaining_m = CASE WHEN $2 = {ARRIVED} THEN 0 ELSE distance_remaining_m END
             WHERE id = $1 AND status = {ACTIVE} RETURNING {TRIP_COLUMNS}"
        )))
        .bind(trip_id)
        .bind(status)
        .fetch_optional(&self.core.db)
        .await?;
        if let Some(row) = &row {
            self.invalidate_ctx(&[row.user_id]).await;
            self.push_trip(row).await?;
        }
        Ok(row)
    }

    /// Recomputes the active trip of `user_id` from their new position:
    /// follows a moving friend, updates the ETA, detects arrival. Called by
    /// the pipeline for live samples.
    pub(crate) async fn advance_trip(&self, user_id: Uuid, ctx: &OwnerCtx, state: &mut LiveState) -> AppResult<()> {
        let Some(trip) = &ctx.trip else { return Ok(()) };
        let now = Utc::now();
        let mut destination = (trip.dest_lat, trip.dest_lng);
        if let Some(friend) = trip.dest_user_id {
            // Only as far as the traveller may see the friend; otherwise the last known spot stands.
            if let Some((point, _)) = self.seen_position(friend, user_id).await? {
                destination = point;
            }
        }
        let remaining_m = math::distance(state.pin.position(), destination);
        if remaining_m <= trip.arrive_radius_m {
            return self.arrive(trip.id).await;
        }
        let eta_ms = now.timestamp_millis() + (eta_seconds(trip, remaining_m) * 1000.0) as i64;
        let due = state.pin.recorded_ms - state.trip_pushed_ms >= PROGRESS_EVERY_MS || (eta_ms - state.trip_eta_ms).abs() >= ETA_JUMP_MS;
        if !due {
            return Ok(());
        }
        let row: Option<TripRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE geo_trips SET eta = $2, distance_remaining_m = $3, dest_lat = $4, dest_lng = $5, updated_at = now()
             WHERE id = $1 AND status = {ACTIVE} RETURNING {TRIP_COLUMNS}"
        )))
        .bind(trip.id)
        .bind(DateTime::from_timestamp_millis(eta_ms))
        .bind(remaining_m)
        .bind(destination.0)
        .bind(destination.1)
        .fetch_optional(&self.core.db)
        .await?;
        state.trip_pushed_ms = state.pin.recorded_ms;
        state.trip_eta_ms = eta_ms;
        if let Some(row) = row {
            if destination != (trip.dest_lat, trip.dest_lng) {
                self.invalidate_ctx(&[user_id]).await;
            }
            self.push_trip(&row).await?;
        }
        Ok(())
    }

    /// The traveller reached the destination.
    async fn arrive(&self, trip_id: Uuid) -> AppResult<()> {
        let Some(row) = self.finish_trip(trip_id, ARRIVED).await? else { return Ok(()) };
        if !row.notify_on_arrival {
            return Ok(());
        }
        let name = tokuly_identity::summary(&self.core, row.user_id).await?.display_name;
        let title = if row.dest_name.is_empty() {
            Localized::new(format!("{name}さんが到着しました!"), format!("{name} has arrived"))
        } else {
            Localized::new(format!("{name}さんが{}に到着しました!", row.dest_name), format!("{name} arrived at {}", row.dest_name))
        };
        let mut tx = self.core.db.begin().await?;
        for recipient in self.trip_audience(&row).await? {
            let notification = NotificationRequest::new(recipient, "geo.trip_arrived", title.clone(), Localized::default(), "/map")
                .actor(row.user_id)
                .collapse(format!("geo.trip:{}", row.id))
                .data(serde_json::json!({ "trip_id": row.id }));
            outbound::notify(&mut *tx, &notification).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Cancels trips that were abandoned: running for a day, or without
    /// progress for six hours.
    pub async fn expire_trips(&self) -> AppResult<u64> {
        let stale: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM geo_trips WHERE status = 1 AND (started_at < now() - interval '24 hours' OR updated_at < now() - interval '6 hours') LIMIT 500",
        )
        .fetch_all(&self.core.db)
        .await?;
        let mut ended = 0;
        for trip_id in stale {
            if self.finish_trip(trip_id, CANCELLED).await?.is_some() {
                ended += 1;
            }
        }
        Ok(ended)
    }

    /// A friendship ended: trips towards the other are cancelled and neither
    /// keeps the other on a share list.
    pub(crate) async fn drop_trips_between(&self, a: Uuid, b: Uuid) -> AppResult<()> {
        let towards: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM geo_trips WHERE status = 1 AND ((user_id = $1 AND dest_user_id = $2) OR (user_id = $2 AND dest_user_id = $1))")
            .bind(a)
            .bind(b)
            .fetch_all(&self.core.db)
            .await?;
        for trip_id in towards {
            self.finish_trip(trip_id, CANCELLED).await?;
        }
        let unshared = sqlx::query(
            "UPDATE geo_trips SET shared_with = array_remove(array_remove(shared_with, $1), $2)
             WHERE status = 1 AND ((user_id = $1 AND $2 = ANY(shared_with)) OR (user_id = $2 AND $1 = ANY(shared_with)))",
        )
        .bind(a)
        .bind(b)
        .execute(&self.core.db)
        .await?
        .rows_affected();
        if unshared > 0 {
            self.invalidate_ctx(&[a, b]).await;
        }
        Ok(())
    }
}

impl TripService for Geo {
    async fn start_trip(&self, ctx: RequestContext, request: ServiceRequest<'_, StartTripRequest>) -> ServiceResult<StartTripResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let ja = self.speaks_japanese(user_id).await?;
            let mode = routing::travel_mode(req.mode);
            ensure!(req.share_with_user_ids.len() <= MAX_SHAREES, invalid, "TOO_MANY_USERS", "a trip is shared with at most {MAX_SHAREES} friends");
            let mut sharees = parse_ids(&req.share_with_user_ids)?;
            sharees.sort();
            sharees.dedup();
            if !sharees.is_empty() {
                let friends = graph::friend_ids(&self.core, user_id).await?;
                ensure!(sharees.iter().all(|id| friends.contains(id)), precondition, "NOT_FRIENDS", "a trip can only be shared with friends");
            }
            let destination = self.resolve_destination(user_id, req.destination.as_option(), ja).await?;
            let origin = self.position_of(user_id).await?;
            self.core.cache.rate_limit(&format!("geo-trip:{user_id}"), 30, Duration::from_secs(3600)).await?;

            let route = self.route(origin, destination.point, mode, ja).await;
            let straight_m = math::distance(origin, destination.point);
            let mut tx = self.core.db.begin().await?;
            let replaced: Option<TripRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "UPDATE geo_trips SET status = {CANCELLED}, ended_at = now() WHERE user_id = $1 AND status = {ACTIVE} RETURNING {TRIP_COLUMNS}"
            )))
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
            let row: TripRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "INSERT INTO geo_trips
                     (user_id, dest_user_id, dest_place_id, dest_name, dest_lat, dest_lng, arrive_radius_m, dest_hidden, mode, eta,
                      distance_remaining_m, route_polyline, plan_distance_m, plan_duration_s, shared_with, notify_on_arrival)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now() + make_interval(secs => $10), $11, $12, $13, $14, $15, $16)
                 RETURNING {TRIP_COLUMNS}"
            )))
            .bind(user_id)
            .bind(destination.user_id)
            .bind(destination.place_id)
            .bind(&destination.name)
            .bind(destination.point.0)
            .bind(destination.point.1)
            .bind(destination.arrive_radius_m)
            .bind(destination.hidden)
            .bind(mode as i16)
            .bind(route.duration_s as f64)
            .bind(straight_m)
            .bind(&route.polyline)
            .bind(straight_m)
            .bind(route.duration_s)
            .bind(&sharees)
            .bind(req.notify_on_arrival)
            .fetch_one(&mut *tx)
            .await?;
            // People picked by name are told; "all friends" just see it on the map.
            if !sharees.is_empty() {
                let name = tokuly_identity::summary(&self.core, user_id).await?.display_name;
                let title = if row.dest_name.is_empty() {
                    Localized::new(format!("{name}さんが移動を開始しました"), format!("{name} is on the way"))
                } else {
                    Localized::new(format!("{name}さんが{}に向かっています", row.dest_name), format!("{name} is heading to {}", row.dest_name))
                };
                for recipient in &sharees {
                    let notification = NotificationRequest::new(*recipient, "geo.trip_started", title.clone(), Localized::default(), "/map")
                        .actor(user_id)
                        .collapse(format!("geo.trip:{}", row.id))
                        .data(serde_json::json!({ "trip_id": row.id }));
                    outbound::notify(&mut *tx, &notification).await?;
                }
            }
            tx.commit().await?;
            self.invalidate_ctx(&[user_id]).await;
            if let Some(replaced) = &replaced {
                self.push_trip(replaced).await?;
            }
            self.push_trip(&row).await?;
            // The pin now carries the trip.
            self.refresh_friends(user_id).await?;
            ok(StartTripResponse { trip: self.trip_message(&row, user_id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn end_trip(&self, ctx: RequestContext, request: ServiceRequest<'_, EndTripRequest>) -> ServiceResult<EndTripResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let trip_id = parse_id(request.trip_id)?;
            let existing = self.trip_row(trip_id).await?.filter(|row| row.user_id == user_id).ok_or_else(trip_not_found)?;
            let row = match self.finish_trip(trip_id, CANCELLED).await? {
                Some(row) => {
                    self.refresh_friends(user_id).await?;
                    row
                }
                // Already over: return it as it ended.
                None => self.trip_row(trip_id).await?.unwrap_or(existing),
            };
            ok(EndTripResponse { trip: self.trip_message(&row, user_id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn get_trip(&self, ctx: RequestContext, request: ServiceRequest<'_, GetTripRequest>) -> ServiceResult<GetTripResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let row = self.trip_row(parse_id(request.trip_id)?).await?.ok_or_else(trip_not_found)?;
            if row.user_id != viewer {
                let shared = row.shared_with.is_empty() || row.shared_with.contains(&viewer);
                ensure!(shared && graph::are_friends(&self.core, viewer, row.user_id).await?, not_found, "TRIP_NOT_FOUND", "trip not found");
            }
            ok(GetTripResponse { trip: self.trip_message(&row, viewer).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn list_active_trips(&self, ctx: RequestContext, _request: ServiceRequest<'_, ListActiveTripsRequest>) -> ServiceResult<ListActiveTripsResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let friends = graph::friend_ids(&self.core, viewer).await?;
            let rows: Vec<TripRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {TRIP_COLUMNS} FROM geo_trips
                 WHERE status = {ACTIVE} AND (user_id = $1 OR (user_id = ANY($2) AND (shared_with = '{{}}' OR $1 = ANY(shared_with))))
                 ORDER BY (user_id = $1) DESC, started_at DESC"
            )))
            .bind(viewer)
            .bind(&friends)
            .fetch_all(&self.core.db)
            .await?;
            let users = tokuly_identity::summaries(&self.core, &rows.iter().map(|row| row.user_id).collect::<Vec<_>>()).await?;
            let trips = rows.iter().map(|row| row.message(tokuly_identity::users::pick(&users, row.user_id), viewer)).collect();
            ok(ListActiveTripsResponse { trips, ..Default::default() })
        })
        .await
    }

    async fn get_directions(&self, ctx: RequestContext, request: ServiceRequest<'_, GetDirectionsRequest>) -> ServiceResult<GetDirectionsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            self.core.cache.rate_limit(&format!("geo-directions:{user_id}"), 60, Duration::from_secs(60)).await?;
            let ja = self.speaks_japanese(user_id).await?;
            let destination = self.resolve_destination(user_id, req.destination.as_option(), ja).await?;
            let origin = match req.origin.as_option() {
                Some(origin) => places::clean_point(Some(origin))?,
                None => self.position_of(user_id).await?,
            };
            let route = self.route(origin, destination.point, routing::travel_mode(req.mode), ja).await;
            ok(GetDirectionsResponse {
                distance_meters: route.distance_m,
                duration_seconds: route.duration_s,
                polyline: route.polyline,
                provider: route.provider.to_owned(),
                steps: route.steps,
                ..Default::default()
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trip(plan_distance_m: f64, plan_duration_s: i32, mode: TravelMode) -> TripCtx {
        TripCtx {
            id: Uuid::now_v7(),
            dest_user_id: None,
            dest_lat: 35.68,
            dest_lng: 139.76,
            arrive_radius_m: 60.0,
            mode: mode as i32,
            started_ms: 0,
            plan_distance_m,
            plan_duration_s,
            shared_with: vec![],
            notify_on_arrival: false,
        }
    }

    #[test]
    fn the_eta_scales_the_plan_by_what_is_left() {
        // Planned: 6 km in 30 minutes. Half way: 15 minutes.
        let planned = trip(6000.0, 1800, TravelMode::TRAVEL_MODE_DRIVE);
        assert_eq!(eta_seconds(&planned, 3000.0), 900.0);
        assert_eq!(eta_seconds(&planned, 0.0), 0.0);
        // Further away than at the start (a detour): proportionally longer.
        assert_eq!(eta_seconds(&planned, 9000.0), 2700.0);
    }

    #[test]
    fn without_a_plan_the_eta_is_a_typical_pace() {
        let unplanned = trip(0.0, 0, TravelMode::TRAVEL_MODE_WALK);
        let seconds = eta_seconds(&unplanned, 1000.0);
        assert!((900.0..1000.0).contains(&seconds), "{seconds}");
    }

    fn row(dest_user: Option<Uuid>, dest_place: Option<Uuid>, hidden: bool) -> TripRow {
        TripRow {
            id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
            dest_user_id: dest_user,
            dest_place_id: dest_place,
            dest_name: "自宅".into(),
            dest_lat: 35.68,
            dest_lng: 139.76,
            dest_hidden: hidden,
            mode: 1,
            status: ACTIVE,
            started_at: Utc::now(),
            ended_at: None,
            eta: Some(Utc::now()),
            distance_remaining_m: 1200.0,
            route_polyline: "abc".into(),
            shared_with: vec![Uuid::now_v7()],
            notify_on_arrival: true,
        }
    }

    #[test]
    fn a_private_destination_is_only_a_name_and_an_eta_to_others() {
        let home_trip = row(None, Some(Uuid::now_v7()), true);
        let user = tokuly_proto::common::UserSummary::default();
        let own = home_trip.message(user.clone(), home_trip.user_id);
        let destination = own.destination.as_option().unwrap();
        assert!(destination.resolved.as_option().is_some() && matches!(destination.target, Some(Target::PlaceId(_))));
        assert_eq!(own.route_polyline, "abc");
        assert_eq!(own.shared_with_user_ids.len(), 1);

        let seen = home_trip.message(user, Uuid::now_v7());
        let destination = seen.destination.as_option().unwrap();
        assert!(destination.resolved.as_option().is_none() && destination.target.is_none());
        assert_eq!((destination.name.as_str(), seen.route_polyline.as_str()), ("自宅", ""));
        assert_eq!(seen.distance_remaining_meters, 1200.0);
        assert!(seen.eta.as_option().is_some() && seen.shared_with_user_ids.is_empty());
    }

    #[test]
    fn a_trip_towards_a_friend_shows_their_position_only_to_the_two() {
        let friend = Uuid::now_v7();
        let to_friend = row(Some(friend), None, true);
        let user = tokuly_proto::common::UserSummary::default();
        let outsider = to_friend.message(user.clone(), Uuid::now_v7());
        let destination = outsider.destination.as_option().unwrap();
        assert!(matches!(&destination.target, Some(Target::UserId(id)) if *id == friend.to_string()));
        assert!(destination.resolved.as_option().is_none());
        let the_friend = to_friend.message(user, friend);
        assert!(the_friend.destination.as_option().unwrap().resolved.as_option().is_some());
    }

    #[test]
    fn a_public_destination_is_shown_to_everyone_the_trip_is_shared_with() {
        let to_point = row(None, None, false);
        let seen = to_point.message(tokuly_proto::common::UserSummary::default(), Uuid::now_v7());
        let destination = seen.destination.as_option().unwrap();
        assert!(matches!(destination.target, Some(Target::Position(_))) && destination.resolved.as_option().is_some());
        assert_eq!(seen.route_polyline, "abc");
    }
}
