//! `TripService`: directions, trips with a live ETA, arrival, and who gets
//! to see what of a trip.
mod common;

use chrono::Utc;
use common::*;
use tokuly_core::time::from_ts;
use tokuly_geo::math;
use tokuly_proto::{
    common::Visibility,
    geo::{destination::Target, geo_event::Event as Push, *},
};
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

fn to_point(point: Point) -> Destination {
    Destination { target: Some(Target::Position(Box::new(lat_lng(point)))), ..Default::default() }
}

fn to_user(user: &TestUser) -> Destination {
    Destination { target: Some(Target::UserId(user.id.to_string())), ..Default::default() }
}

async fn start_trip(ctx: &Ctx, user: &TestUser, request: StartTripRequest) -> Result<Trip, RpcError> {
    let response: StartTripResponse = ctx.trips("StartTrip", user, &request).await?;
    Ok(response.trip.into_option().expect("trip"))
}

async fn get_trip(ctx: &Ctx, viewer: &TestUser, trip_id: &str) -> Result<Trip, RpcError> {
    let response: GetTripResponse = ctx.trips("GetTrip", viewer, &GetTripRequest { trip_id: trip_id.to_owned(), ..Default::default() }).await?;
    Ok(response.trip.into_option().expect("trip"))
}

async fn active_trips(ctx: &Ctx, viewer: &TestUser) -> Vec<Trip> {
    let response: ListActiveTripsResponse = ctx.trips("ListActiveTrips", viewer, &ListActiveTripsRequest::default()).await.unwrap();
    response.trips
}

fn trip_updates(pushes: Vec<Push>) -> Vec<Trip> {
    pushes.into_iter().filter_map(|push| if let Push::TripUpdated(trip) = push { Some(*trip) } else { None }).collect()
}

fn seconds_until(trip: &Trip) -> i64 {
    (from_ts(trip.eta.as_option().expect("an ETA")) - Utc::now()).num_seconds()
}

#[tokio::test]
async fn directions_are_estimated_without_a_routing_engine() {
    let ctx = start().await;
    let (aoi, ren, stranger) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("stranger").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    let station = offset(home, 90.0, 6000.0);
    let directions = |request: GetDirectionsRequest| async { ctx.trips::<_, GetDirectionsResponse>("GetDirections", &aoi, &request).await };

    let walk = directions(GetDirectionsRequest { origin: lat_lng(home).into(), destination: to_point(station).into(), mode: TravelMode::TRAVEL_MODE_WALK.into(), ..Default::default() }).await.unwrap();
    assert_eq!(walk.provider, "estimate");
    assert!((walk.distance_meters - 6000.0).abs() < 10.0, "{}", walk.distance_meters);
    assert!((5400..6200).contains(&walk.duration_seconds), "6 km on foot: {} s", walk.duration_seconds);
    let line = math::decode_polyline(&walk.polyline, 5);
    assert!(distance(line[0], home) < 2.0 && distance(*line.last().unwrap(), station) < 2.0);
    assert_eq!(walk.steps.len(), 1);
    assert!(!walk.steps[0].instruction.is_empty());
    let drive = directions(GetDirectionsRequest { origin: lat_lng(home).into(), destination: to_point(station).into(), mode: TravelMode::TRAVEL_MODE_DRIVE.into(), ..Default::default() }).await.unwrap();
    assert!(drive.duration_seconds < walk.duration_seconds / 5);
    // No mode means walking.
    let unspecified = directions(GetDirectionsRequest { origin: lat_lng(home).into(), destination: to_point(station).into(), ..Default::default() }).await.unwrap();
    assert_eq!(unspecified.duration_seconds, walk.duration_seconds);

    // Without an origin the caller's own position is used — which must be known.
    let from_here = GetDirectionsRequest { destination: to_point(station).into(), ..Default::default() };
    assert_eq!(directions(from_here.clone()).await.unwrap_err().reason, "LOCATION_UNKNOWN");
    ctx.at(&aoi, home).await;
    assert!((directions(from_here).await.unwrap().distance_meters - 6000.0).abs() < 10.0);

    // To a friend, as far as the caller may see them.
    assert_eq!(directions(GetDirectionsRequest { destination: to_user(&ren).into(), ..Default::default() }).await.unwrap_err().reason, "DESTINATION_UNAVAILABLE");
    ctx.at(&ren, station).await;
    assert!((directions(GetDirectionsRequest { destination: to_user(&ren).into(), ..Default::default() }).await.unwrap().distance_meters - 6000.0).abs() < 10.0);
    ctx.ghost(&ren, &[&aoi], GhostMode::GHOST_MODE_FROZEN, 0).await.unwrap();
    ctx.at(&ren, offset(station, 0.0, 3000.0)).await;
    assert_eq!(directions(GetDirectionsRequest { destination: to_user(&ren).into(), ..Default::default() }).await.unwrap_err().reason, "DESTINATION_UNAVAILABLE", "a frozen pin is not a destination");

    assert_eq!(directions(GetDirectionsRequest::default()).await.unwrap_err().reason, "MISSING_DESTINATION");
    assert_eq!(directions(GetDirectionsRequest { destination: Destination::default().into(), ..Default::default() }).await.unwrap_err().reason, "MISSING_DESTINATION");
    assert_eq!(directions(GetDirectionsRequest { destination: to_point((0.0, 999.0)).into(), ..Default::default() }).await.unwrap_err().reason, "INVALID_LOCATION");
    assert_eq!(directions(GetDirectionsRequest { origin: lat_lng((99.0, 0.0)).into(), destination: to_point(station).into(), ..Default::default() }).await.unwrap_err().reason, "INVALID_LOCATION");
    assert_eq!(directions(GetDirectionsRequest { destination: to_user(&stranger).into(), ..Default::default() }).await.unwrap_err().reason, "NOT_FRIENDS");
    let nowhere = Destination { target: Some(Target::PlaceId(Uuid::now_v7().to_string())), ..Default::default() };
    assert_eq!(directions(GetDirectionsRequest { destination: nowhere.into(), ..Default::default() }).await.unwrap_err().reason, "PLACE_NOT_FOUND");
}

#[tokio::test]
async fn a_trip_updates_its_eta_and_ends_on_arrival() {
    let ctx = start().await;
    let (aoi, ren, mio) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("mio").await);
    ctx.befriend(&aoi, &ren).await;
    ctx.befriend(&aoi, &mio).await;
    let home = somewhere();
    let cafe = offset(home, 90.0, 2000.0);
    let unknown = start_trip(&ctx, &aoi, StartTripRequest { destination: to_point(cafe).into(), ..Default::default() }).await.unwrap_err();
    assert_eq!((unknown.code.as_str(), unknown.reason.as_str()), ("failed_precondition", "LOCATION_UNKNOWN"));
    ctx.at(&aoi, home).await;
    let mut rens = ctx.app.listen_user(ren.id).await;
    let mut mios = ctx.app.listen_user(mio.id).await;

    let request = StartTripRequest {
        destination: Destination { name: "カフェ".into(), ..to_point(cafe) }.into(),
        mode: TravelMode::TRAVEL_MODE_WALK.into(),
        share_with_user_ids: vec![ren.id.to_string()],
        notify_on_arrival: true,
        ..Default::default()
    };
    let trip = start_trip(&ctx, &aoi, request).await.unwrap();
    assert_eq!((trip.status.as_known(), trip.mode.as_known()), (Some(TripStatus::TRIP_STATUS_ACTIVE), Some(TravelMode::TRAVEL_MODE_WALK)));
    assert_eq!(trip.user.as_option().unwrap().id, aoi.id.to_string());
    assert!((trip.distance_remaining_meters - 2000.0).abs() < 5.0);
    let planned = seconds_until(&trip);
    assert!((1800..2100).contains(&planned), "2 km on foot: {planned} s");
    let destination = trip.destination.as_option().unwrap();
    assert_eq!(destination.name, "カフェ");
    assert!(distance(point(destination.resolved.as_option()), cafe) < 1.0);
    assert!(!trip.route_polyline.is_empty() && trip.notify_on_arrival);
    assert_eq!(trip.shared_with_user_ids, vec![ren.id.to_string()]);

    // The friend it is shared with is told, sees it, and sees it on the pin; the other friend does not.
    assert_eq!(ctx.notifications_for(ren.id, "geo.trip_started").await.len(), 1);
    assert!(ctx.notifications_for(mio.id, "geo.trip_started").await.is_empty());
    assert_eq!(trip_updates(pushes(&mut rens, QUIET).await).last().unwrap().id, trip.id);
    assert!(trip_updates(pushes(&mut mios, QUIET).await).is_empty());
    assert_eq!(get_trip(&ctx, &ren, &trip.id).await.unwrap().id, trip.id);
    assert_eq!(get_trip(&ctx, &mio, &trip.id).await.unwrap_err().reason, "TRIP_NOT_FOUND");
    assert_eq!(active_trips(&ctx, &ren).await.iter().map(|t| t.id.clone()).collect::<Vec<_>>(), vec![trip.id.clone()]);
    assert!(active_trips(&ctx, &mio).await.is_empty());
    assert_eq!(active_trips(&ctx, &aoi).await.len(), 1);
    assert_eq!(ctx.seen_by(&ren, &aoi).await.unwrap().trip_id, trip.id);
    assert!(ctx.seen_by(&mio, &aoi).await.unwrap().trip_id.is_empty());
    assert!(get_trip(&ctx, &ren, &trip.id).await.unwrap().shared_with_user_ids.is_empty(), "who else it is shared with is the traveller's business");

    // Half way there: half the time left, pushed to the friend.
    ctx.at(&aoi, offset(home, 90.0, 1000.0)).await;
    let halfway = trip_updates(pushes(&mut rens, QUIET).await).pop().expect("a trip update");
    assert!((halfway.distance_remaining_meters - 1000.0).abs() < 5.0);
    assert!((seconds_until(&halfway) - planned / 2).abs() < 30, "{} s left", seconds_until(&halfway));
    assert_eq!(halfway.status, TripStatus::TRIP_STATUS_ACTIVE);

    // Arriving ends the trip and tells the friend, as asked.
    ctx.at(&aoi, offset(cafe, 0.0, 20.0)).await;
    let arrived = trip_updates(pushes(&mut rens, QUIET).await).pop().expect("the arrival");
    assert_eq!((arrived.status.as_known(), arrived.distance_remaining_meters), (Some(TripStatus::TRIP_STATUS_ARRIVED), 0.0));
    assert!(arrived.ended_at.as_option().is_some());
    let told = ctx.notifications_for(ren.id, "geo.trip_arrived").await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].data["title"]["ja"].as_str().unwrap(), "aoiさんがカフェに到着しました!");
    assert!(ctx.notifications_for(mio.id, "geo.trip_arrived").await.is_empty());
    assert!(active_trips(&ctx, &aoi).await.is_empty() && active_trips(&ctx, &ren).await.is_empty());
    assert_eq!(get_trip(&ctx, &aoi, &trip.id).await.unwrap().status, TripStatus::TRIP_STATUS_ARRIVED);
    assert!(ctx.seen_by(&ren, &aoi).await.unwrap().trip_id.is_empty());
    // Ending a trip that is over returns it as it ended.
    let ended: EndTripResponse = ctx.trips("EndTrip", &aoi, &EndTripRequest { trip_id: trip.id.clone(), ..Default::default() }).await.unwrap();
    assert_eq!(ended.trip.as_option().unwrap().status, TripStatus::TRIP_STATUS_ARRIVED);
}

#[tokio::test]
async fn trips_shared_with_everyone_can_be_cancelled_and_replaced() {
    let ctx = start().await;
    let (aoi, ren, stranger) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("stranger").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    ctx.at(&aoi, home).await;
    let mut rens = ctx.app.listen_user(ren.id).await;

    // No list: shared with all friends, announced on the map only.
    let first = start_trip(&ctx, &aoi, StartTripRequest { destination: to_point(offset(home, 0.0, 5000.0)).into(), mode: TravelMode::TRAVEL_MODE_DRIVE.into(), ..Default::default() }).await.unwrap();
    assert!(first.shared_with_user_ids.is_empty() && !first.notify_on_arrival);
    assert!(ctx.notifications_for(ren.id, "geo.trip_started").await.is_empty());
    assert_eq!(get_trip(&ctx, &ren, &first.id).await.unwrap().id, first.id);
    assert_eq!(get_trip(&ctx, &stranger, &first.id).await.unwrap_err().reason, "TRIP_NOT_FOUND");
    pushes(&mut rens, QUIET).await;

    // A new trip replaces the old one.
    let second = start_trip(&ctx, &aoi, StartTripRequest { destination: to_point(offset(home, 180.0, 800.0)).into(), ..Default::default() }).await.unwrap();
    assert_eq!(second.mode, TravelMode::TRAVEL_MODE_WALK, "no mode means walking");
    let updates = trip_updates(pushes(&mut rens, QUIET).await);
    assert!(updates.iter().any(|t| t.id == first.id && t.status == TripStatus::TRIP_STATUS_CANCELLED));
    assert!(updates.iter().any(|t| t.id == second.id && t.status == TripStatus::TRIP_STATUS_ACTIVE));
    assert_eq!(active_trips(&ctx, &aoi).await.iter().map(|t| t.id.clone()).collect::<Vec<_>>(), vec![second.id.clone()]);

    // Only the traveller ends a trip.
    let end = |user: &TestUser, trip_id: &str| {
        let (user, request) = (user.clone(), EndTripRequest { trip_id: trip_id.to_owned(), ..Default::default() });
        let ctx = &ctx;
        async move { ctx.trips::<_, EndTripResponse>("EndTrip", &user, &request).await }
    };
    assert_eq!(end(&ren, &second.id).await.unwrap_err().reason, "TRIP_NOT_FOUND");
    assert_eq!(end(&aoi, &Uuid::now_v7().to_string()).await.unwrap_err().reason, "TRIP_NOT_FOUND");
    assert_eq!(end(&aoi, "x").await.unwrap_err().reason, "INVALID_ID");
    let cancelled = end(&aoi, &second.id).await.unwrap().trip.into_option().unwrap();
    assert_eq!(cancelled.status, TripStatus::TRIP_STATUS_CANCELLED);
    assert!(trip_updates(pushes(&mut rens, QUIET).await).iter().any(|t| t.id == second.id && t.status == TripStatus::TRIP_STATUS_CANCELLED));
    assert!(active_trips(&ctx, &ren).await.is_empty());

    // Validation.
    let reason = |request: StartTripRequest| async { start_trip(&ctx, &aoi, request).await.unwrap_err().reason };
    assert_eq!(reason(StartTripRequest::default()).await, "MISSING_DESTINATION");
    assert_eq!(reason(StartTripRequest { destination: to_point((91.0, 0.0)).into(), ..Default::default() }).await, "INVALID_LOCATION");
    assert_eq!(reason(StartTripRequest { destination: to_user(&stranger).into(), ..Default::default() }).await, "NOT_FRIENDS");
    assert_eq!(reason(StartTripRequest { destination: to_user(&aoi).into(), ..Default::default() }).await, "NOT_FRIENDS");
    assert_eq!(reason(StartTripRequest { destination: to_point(home).into(), share_with_user_ids: vec![stranger.id.to_string()], ..Default::default() }).await, "NOT_FRIENDS");
    let crowd = (0..101).map(|_| Uuid::now_v7().to_string()).collect();
    assert_eq!(reason(StartTripRequest { destination: to_point(home).into(), share_with_user_ids: crowd, ..Default::default() }).await, "TOO_MANY_USERS");
    assert!(active_trips(&ctx, &aoi).await.is_empty(), "failed requests start nothing");
}

#[tokio::test]
async fn a_trip_towards_a_friend_follows_them_as_far_as_ghost_mode_allows() {
    let ctx = start().await;
    let (aoi, ren, mio) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("mio").await);
    ctx.befriend(&aoi, &ren).await;
    ctx.befriend(&aoi, &mio).await;
    let start_at = somewhere();
    let rens_spot = offset(start_at, 90.0, 3000.0);
    ctx.at(&aoi, start_at).await;
    ctx.at(&ren, rens_spot).await;

    let trip = start_trip(&ctx, &aoi, StartTripRequest { destination: to_user(&ren).into(), notify_on_arrival: true, ..Default::default() }).await.unwrap();
    let destination = trip.destination.as_option().unwrap();
    assert!(matches!(&destination.target, Some(Target::UserId(id)) if *id == ren.id.to_string()));
    assert_eq!(destination.name, "ren", "named after the friend");
    assert!(distance(point(destination.resolved.as_option()), rens_spot) < 1.0);
    assert!((trip.distance_remaining_meters - 3000.0).abs() < 5.0);

    // Others watching the trip learn that Aoi is heading to Ren, but not where Ren is.
    let seen = get_trip(&ctx, &mio, &trip.id).await.unwrap();
    let hidden = seen.destination.as_option().unwrap();
    assert!(matches!(&hidden.target, Some(Target::UserId(id)) if *id == ren.id.to_string()));
    assert!(hidden.resolved.as_option().is_none() && seen.route_polyline.is_empty());
    assert!(seen.eta.as_option().is_some() && seen.distance_remaining_meters > 0.0);
    // Ren, being the destination, sees all of it.
    assert!(get_trip(&ctx, &ren, &trip.id).await.unwrap().destination.as_option().unwrap().resolved.as_option().is_some());

    // Ren moves closer: the trip follows.
    let closer = offset(start_at, 90.0, 2000.0);
    ctx.at(&ren, closer).await;
    ctx.at(&aoi, offset(start_at, 90.0, 500.0)).await;
    let following = get_trip(&ctx, &aoi, &trip.id).await.unwrap();
    assert!((following.distance_remaining_meters - 1500.0).abs() < 5.0, "{}", following.distance_remaining_meters);
    assert!(distance(point(following.destination.as_option().unwrap().resolved.as_option()), closer) < 1.0);

    // Ren freezes their pin for Aoi and walks away: the trip keeps aiming at the last spot Aoi saw.
    ctx.ghost(&ren, &[&aoi], GhostMode::GHOST_MODE_FROZEN, 0).await.unwrap();
    ctx.at(&ren, offset(start_at, 90.0, 9000.0)).await;
    ctx.at(&aoi, offset(start_at, 90.0, 1000.0)).await;
    let aimed = get_trip(&ctx, &aoi, &trip.id).await.unwrap();
    assert!(distance(point(aimed.destination.as_option().unwrap().resolved.as_option()), closer) < 1.0, "a frozen pin is not refreshed through a trip");
    assert!((aimed.distance_remaining_meters - 1000.0).abs() < 5.0);

    // Reaching that spot counts as arriving.
    ctx.at(&aoi, offset(closer, 180.0, 10.0)).await;
    assert_eq!(get_trip(&ctx, &aoi, &trip.id).await.unwrap().status, TripStatus::TRIP_STATUS_ARRIVED);
    assert_eq!(ctx.notifications_for(ren.id, "geo.trip_arrived").await.len(), 1);

    // Unfriending cancels a trip towards the former friend.
    let again = start_trip(&ctx, &aoi, StartTripRequest { destination: to_user(&mio).into(), ..Default::default() }).await.unwrap_err();
    assert_eq!(again.reason, "DESTINATION_UNAVAILABLE", "Mio has shared no location");
    ctx.at(&mio, offset(start_at, 0.0, 4000.0)).await;
    let to_mio = start_trip(&ctx, &aoi, StartTripRequest { destination: to_user(&mio).into(), ..Default::default() }).await.unwrap();
    ctx.unfriend(&aoi, &mio).await;
    assert_eq!(get_trip(&ctx, &aoi, &to_mio.id).await.unwrap().status, TripStatus::TRIP_STATUS_CANCELLED);
}

#[tokio::test]
async fn a_trip_home_shows_others_an_eta_but_not_the_address() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    let request = CreatePlaceRequest { name: "うち".into(), label: PlaceLabel::PLACE_LABEL_HOME.into(), center: lat_lng(home).into(), radius_meters: 100.0, visibility: Visibility::VISIBILITY_PRIVATE.into(), ..Default::default() };
    let place: CreatePlaceResponse = ctx.places("CreatePlace", &aoi, &request).await.unwrap();
    let place = place.place.into_option().unwrap();
    ctx.at(&aoi, offset(home, 270.0, 1500.0)).await;

    let destination = Destination { target: Some(Target::PlaceId(place.id.clone())), ..Default::default() };
    let trip = start_trip(&ctx, &aoi, StartTripRequest { destination: destination.into(), ..Default::default() }).await.unwrap();
    let own = trip.destination.as_option().unwrap();
    assert_eq!(own.name, "うち");
    assert!(matches!(&own.target, Some(Target::PlaceId(id)) if *id == place.id) && own.resolved.as_option().is_some());

    let seen = get_trip(&ctx, &ren, &trip.id).await.unwrap();
    let theirs = seen.destination.as_option().unwrap();
    assert_eq!(theirs.name, "うち");
    assert!(theirs.target.is_none() && theirs.resolved.as_option().is_none() && seen.route_polyline.is_empty());
    assert!((seen.distance_remaining_meters - 1500.0).abs() < 5.0 && seen.eta.as_option().is_some());

    // Arrival uses the place's own radius.
    ctx.at(&aoi, offset(home, 270.0, 90.0)).await;
    assert_eq!(get_trip(&ctx, &aoi, &trip.id).await.unwrap().status, TripStatus::TRIP_STATUS_ARRIVED);

    // Someone else's private place is not a destination.
    let theirs = Destination { target: Some(Target::PlaceId(place.id.clone())), ..Default::default() };
    assert_eq!(start_trip(&ctx, &ren, StartTripRequest { destination: theirs.into(), ..Default::default() }).await.unwrap_err().reason, "PLACE_NOT_FOUND");
}

#[tokio::test]
async fn abandoned_trips_are_cancelled_by_the_sweeper() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    let home = somewhere();
    ctx.at(&aoi, home).await;
    let trip = start_trip(&ctx, &aoi, StartTripRequest { destination: to_point(offset(home, 0.0, 9000.0)).into(), ..Default::default() }).await.unwrap();
    ctx.geo.expire_trips().await.unwrap();
    assert_eq!(get_trip(&ctx, &aoi, &trip.id).await.unwrap().status, TripStatus::TRIP_STATUS_ACTIVE, "a fresh trip stays");
    sqlx::query("UPDATE geo_trips SET updated_at = now() - interval '7 hours' WHERE id = $1").bind(Uuid::parse_str(&trip.id).unwrap()).execute(&ctx.app.core.db).await.unwrap();
    assert!(ctx.geo.expire_trips().await.unwrap() >= 1);
    assert_eq!(get_trip(&ctx, &aoi, &trip.id).await.unwrap().status, TripStatus::TRIP_STATUS_CANCELLED);
}
