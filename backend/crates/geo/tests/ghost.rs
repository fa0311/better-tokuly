//! Ghost mode against side channels: one regression test per way in which
//! something derived from a person's true position used to reach a viewer
//! who does not see that person precisely. (The rule itself is tested as a
//! property in `tests/ghost_property.rs`.)
mod common;

use chrono::{DateTime, Duration, Timelike as _, Utc};
use common::*;
use tokuly_core::time::ts;
use tokuly_proto::{
    common::{BoundingBox, Visibility},
    geo::{check_in_request::Where, destination::Target, geo_event::Event as Push, *},
};
use tokuly_testkit::{RpcError, TestUser};

async fn gatherings(ctx: &Ctx, viewer: &TestUser) -> Vec<Gathering> {
    let response: ListGatheringsResponse =
        ctx.location("ListGatherings", viewer, &ListGatheringsRequest::default()).await.unwrap();
    response.gatherings
}

async fn visits(ctx: &Ctx, user: &TestUser) -> Vec<Visit> {
    let response: ListVisitsResponse = ctx.history("ListVisits", user, &ListVisitsRequest::default()).await.unwrap();
    response.visits
}

/// A time zone in which it is around noon right now, and today's date there.
fn midday_zone() -> (String, String) {
    let now = Utc::now();
    let shift = 12 - now.hour() as i64;
    let zone = match shift {
        0 => "Etc/GMT".to_owned(),
        s if s > 0 => format!("Etc/GMT-{s}"),
        s => format!("Etc/GMT+{}", -s),
    };
    (zone, (now + Duration::hours(shift)).date_naive().to_string())
}

async fn today(ctx: &Ctx, user: &TestUser) -> GetDayResponse {
    let (zone, date) = midday_zone();
    ctx.history("GetDay", user, &GetDayRequest { date, time_zone: zone, ..Default::default() }).await.unwrap()
}

/// Two people standing next to each other for six minutes: long enough to
/// be together, and long enough for each to be on a visit there.
async fn stand_together(ctx: &Ctx, a: &TestUser, b: &TestUser, spot: Point, t: DateTime<Utc>) {
    meet(ctx, a, b, spot, t).await;
    seen_at(ctx, b, offset(spot, 90.0, 8.0), t, &[-15]).await;
    seen_at(ctx, a, spot, t, &[-30]).await;
    seen_at(ctx, b, offset(spot, 90.0, 8.0), t, &[-45]).await;
}

fn gathering_pushes(pushes: &[Push]) -> usize {
    pushes.iter().filter(|push| matches!(push, Push::GatheringUpdated(_) | Push::GatheringEndedId(_))).count()
}

async fn card(ctx: &Ctx, viewer: &TestUser, place_id: &str) -> GetPlaceResponse {
    ctx.places("GetPlace", viewer, &GetPlaceRequest { place_id: place_id.to_owned(), ..Default::default() })
        .await
        .expect("place card")
}

async fn create_place(
    ctx: &Ctx,
    owner: &TestUser,
    name: &str,
    center: Point,
    radius: f64,
    visibility: Visibility,
) -> Place {
    let request = CreatePlaceRequest {
        name: name.into(),
        center: lat_lng(center).into(),
        radius_meters: radius,
        visibility: visibility.into(),
        ..Default::default()
    };
    let response: CreatePlaceResponse = ctx.places("CreatePlace", owner, &request).await.expect("create place");
    response.place.into_option().unwrap()
}

async fn cells_of(ctx: &Ctx, viewer: &TestUser, owner: &TestUser, around: Point) -> Result<Vec<String>, RpcError> {
    let request = GetCellsRequest {
        user_id: owner.id.to_string(),
        bounds: BoundingBox {
            south_west: lat_lng((around.0 - 0.3, around.1 - 0.3)).into(),
            north_east: lat_lng((around.0 + 0.3, around.1 + 0.3)).into(),
            ..Default::default()
        }
        .into(),
        zoom: 15.0,
        ..Default::default()
    };
    let response: GetCellsResponse = ctx.footprints("GetCells", viewer, &request).await?;
    Ok(response.cells)
}

async fn stats_of(ctx: &Ctx, viewer: &TestUser, owner: &TestUser) -> GetFootprintStatsResponse {
    let request = GetFootprintStatsRequest { user_id: owner.id.to_string(), ..Default::default() };
    ctx.footprints("GetStats", viewer, &request).await.expect("stats")
}

async fn leaderboard(ctx: &Ctx, viewer: &TestUser, scope: LeaderboardScope) -> Vec<(String, i64)> {
    let request = GetLeaderboardRequest { scope: scope.into(), limit: 100, ..Default::default() };
    let response: GetLeaderboardResponse = ctx.footprints("GetLeaderboard", viewer, &request).await.unwrap();
    response.entries.iter().map(|entry| (entry.user.as_option().unwrap().id.clone(), entry.cell_count)).collect()
}

/// Rides east from `from` for `meters` (a fix every 50 m and 10 s), starting at `start`.
async fn ride(ctx: &Ctx, user: &TestUser, from: Point, meters: f64, start: DateTime<Utc>) {
    let steps = (meters / 50.0).round() as i64;
    let samples: Vec<LocationSample> =
        (0..=steps).map(|i| sample(offset(from, 90.0, i as f64 * 50.0), start + Duration::seconds(i * 10))).collect();
    let sent = samples.len() as i32;
    assert_eq!(ctx.report(user, samples).await.accepted_count, sent);
}

/// C1: proximity must not be an oracle. A frozen friend is never "near",
/// wherever they really are, and nothing a prober does next to them shows.
#[tokio::test]
async fn standing_next_to_a_frozen_friend_reveals_nothing() {
    let ctx = start().await;
    let (aoi, ren, kai, mio) =
        (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("kai").await, ctx.app.user("mio").await);
    ctx.befriend(&aoi, &ren).await;
    ctx.befriend(&kai, &mio).await;
    let home = somewhere();
    ctx.report(&aoi, vec![sample(home, minutes_ago(10))]).await;
    ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_FROZEN, 0).await.unwrap();
    let mut rens = ctx.open_app(&ren).await;

    // Aoi goes somewhere else; Ren guesses where and stands right there with her.
    let hideout = offset(home, 30.0, 4000.0);
    let t = Utc::now() + Duration::seconds(50);
    stand_together(&ctx, &aoi, &ren, hideout, t).await;
    // The same without ghost mode, to know what would have shown.
    let elsewhere = offset(home, 200.0, 6000.0);
    stand_together(&ctx, &kai, &mio, elsewhere, t).await;
    assert_eq!(gatherings(&ctx, &kai).await.len(), 1);
    assert_eq!(visits(&ctx, &mio).await[0].with_friends.len(), 1);
    assert_eq!(today(&ctx, &mio).await.friends_met.len(), 1);

    // For Ren: no gathering, no event of any kind about one, nobody met.
    assert!(gatherings(&ctx, &ren).await.is_empty() && gatherings(&ctx, &aoi).await.is_empty());
    assert_eq!(gathering_pushes(&pushes(&mut rens, QUIET).await), 0);
    let rens_visits = visits(&ctx, &ren).await;
    assert_eq!(rens_visits.len(), 1, "Ren's own stay is his own");
    assert!(rens_visits[0].with_friends.is_empty());
    assert!(today(&ctx, &ren).await.friends_met.is_empty());
    // Not on Aoi's side either: being "together" is something both would learn.
    assert!(visits(&ctx, &aoi).await.iter().all(|visit| visit.with_friends.is_empty()));
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM geo_gathering_members WHERE user_id = ANY($1)")
        .bind(vec![aoi.id, ren.id])
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(rows, 0, "the server itself never considered them together");
    // Ren's pin of Aoi is where it was.
    assert_at!(pin_point(&ctx.seen_by(&ren, &aoi).await.unwrap()), home);
}

/// C1: a rule that begins during a gathering ends it for the two at once,
/// and what the hidden one does afterwards changes nothing for the other.
#[tokio::test]
async fn hiding_from_someone_ends_what_the_two_shared_by_being_near() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let spot = somewhere();
    let t = Utc::now();
    meet(&ctx, &aoi, &ren, spot, t).await;
    let before = gatherings(&ctx, &ren).await;
    assert_eq!(before.len(), 1);
    let mut rens = ctx.open_app(&ren).await;

    ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_BLURRED, 0).await.unwrap();
    let ended: Vec<String> = pushes(&mut rens, QUIET)
        .await
        .into_iter()
        .filter_map(|push| if let Push::GatheringEndedId(id) = push { Some(id) } else { None })
        .collect();
    assert_eq!(ended, vec![before[0].id.clone()], "over, at the moment the rule began");
    assert!(gatherings(&ctx, &ren).await.is_empty() && gatherings(&ctx, &aoi).await.is_empty());
    // Aoi stays, leaves, comes back: Ren hears nothing about gatherings.
    for (bearing, meters) in [(0.0, 3.0), (0.0, 900.0), (0.0, 5.0)] {
        ctx.at(&aoi, offset(spot, bearing, meters)).await;
        ctx.at(&ren, offset(spot, 90.0, 8.0)).await;
    }
    assert_eq!(gathering_pushes(&pushes(&mut rens, QUIET).await), 0);
}

/// C2: a trip is made of the traveller's position.
#[tokio::test]
async fn a_trip_is_shown_only_to_friends_who_see_the_traveller() {
    let ctx = start().await;
    let (aoi, ren, kai) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("kai").await);
    ctx.befriend(&aoi, &ren).await;
    ctx.befriend(&aoi, &kai).await;
    let home = somewhere();
    let station = offset(home, 90.0, 2000.0);
    ctx.at(&aoi, home).await;
    // Frozen for everyone, except Kai.
    ctx.ghost(&aoi, &[], GhostMode::GHOST_MODE_FROZEN, 0).await.unwrap();
    ctx.ghost(&aoi, &[&kai], GhostMode::GHOST_MODE_PRECISE, 0).await.unwrap();
    let (mut rens, mut kais) = (ctx.open_app(&ren).await, ctx.open_app(&kai).await);

    let request = StartTripRequest {
        destination: Destination { target: Some(Target::Position(Box::new(lat_lng(station)))), ..Default::default() }
            .into(),
        share_with_user_ids: vec![ren.id.to_string(), kai.id.to_string()],
        notify_on_arrival: true,
        ..Default::default()
    };
    let started: StartTripResponse = ctx.trips("StartTrip", &aoi, &request).await.unwrap();
    let trip_id = started.trip.into_option().unwrap().id;
    ctx.at(&aoi, offset(home, 90.0, 1000.0)).await;
    ctx.at(&aoi, offset(station, 0.0, 5.0)).await; // arrives

    let trip_events =
        |pushes: Vec<Push>| pushes.into_iter().filter(|push| matches!(push, Push::TripUpdated(_))).count();
    // Kai sees her: the trip, its progress, its end, the notifications.
    assert!(trip_events(pushes(&mut kais, QUIET).await) >= 2);
    assert_eq!(ctx.notifications_for(kai.id, "geo.trip_started").await.len(), 1);
    assert_eq!(ctx.notifications_for(kai.id, "geo.trip_arrived").await.len(), 1);
    let seen: GetTripResponse =
        ctx.trips("GetTrip", &kai, &GetTripRequest { trip_id: trip_id.clone(), ..Default::default() }).await.unwrap();
    assert_eq!(seen.trip.as_option().unwrap().status, TripStatus::TRIP_STATUS_ARRIVED);
    // Ren does not, though the trip was shared with him by name: nothing at all.
    assert_eq!(trip_events(pushes(&mut rens, QUIET).await), 0);
    assert!(ctx.notifications_for(ren.id, "geo.trip_started").await.is_empty());
    assert!(ctx.notifications_for(ren.id, "geo.trip_arrived").await.is_empty());
    let hidden = ctx
        .trips::<_, GetTripResponse>("GetTrip", &ren, &GetTripRequest { trip_id, ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!(hidden.reason, "TRIP_NOT_FOUND");
    let listed: ListActiveTripsResponse =
        ctx.trips("ListActiveTrips", &ren, &ListActiveTripsRequest::default()).await.unwrap();
    assert!(listed.trips.is_empty());
    let pin = ctx.seen_by(&ren, &aoi).await.unwrap();
    assert!(pin.trip_id.is_empty());
    assert_at!(pin_point(&pin), home);
}

/// C3: a gathering as one viewer sees it is made of the members they see.
#[tokio::test]
async fn a_hidden_member_has_no_pull_on_what_a_viewer_sees_of_a_gathering() {
    let ctx = start().await;
    let (aoi, ren, kai, mio) =
        (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("kai").await, ctx.app.user("mio").await);
    for (a, b) in [(&aoi, &ren), (&aoi, &kai), (&ren, &kai), (&mio, &aoi), (&mio, &ren), (&mio, &kai)] {
        ctx.befriend(a, b).await;
    }
    // Mio is not to know where Kai is.
    ctx.ghost(&kai, &[&mio], GhostMode::GHOST_MODE_BLURRED, 0).await.unwrap();
    let spot = somewhere();
    let t = Utc::now();
    let mut mios = ctx.open_app(&mio).await;

    // Kai and Aoi meet first: for Mio that is one visible friend, so nothing.
    meet(&ctx, &kai, &aoi, spot, t).await;
    assert!(gatherings(&ctx, &mio).await.is_empty());
    assert_eq!(gathering_pushes(&pushes(&mut mios, QUIET).await), 0);
    assert!(ctx.notifications_for(mio.id, "geo.gathering").await.is_empty());

    // Ren joins, 25 m to the other side of Aoi.
    let rens = offset(spot, 90.0, 33.0);
    seen_at(&ctx, &ren, rens, t, &[250, 220]).await;
    seen_at(&ctx, &ren, rens, t, &[40]).await;
    let full = gatherings(&ctx, &aoi).await.pop().expect("the gathering");
    assert_eq!(full.members.len(), 3);
    let seen = gatherings(&ctx, &mio).await.pop().expect("Mio sees two of them");
    let ids: Vec<&str> = seen.members.iter().map(|member| member.user.as_option().unwrap().id.as_str()).collect();
    assert!(ids.len() == 2 && !ids.contains(&kai.id.to_string().as_str()));
    // The centre Mio gets is the middle of Aoi's and Ren's pins, exactly:
    // Kai (8 m on the other side of Aoi) does not shift it.
    let aois = pin_point(&ctx.seen_by(&mio, &aoi).await.unwrap());
    let middle = ((aois.0 + rens.0) / 2.0, (aois.1 + rens.1) / 2.0);
    assert!(distance(point(seen.center.as_option()), middle) < 0.01, "{:?} vs {middle:?}", seen.center);
    assert!(distance(point(full.center.as_option()), middle) > 1.0, "the full view is centred elsewhere");
    // It began, for her, when Ren joined Aoi — not when Kai and Aoi met.
    assert!(seen.started_at.as_option().unwrap().seconds > full.started_at.as_option().unwrap().seconds);
    // Ren leaves again: for Mio the gathering is over, though two people remain.
    let later = Utc::now() + Duration::seconds(50);
    ctx.report(&ren, vec![sample(offset(spot, 90.0, 900.0), later)]).await;
    let (updated, ended): (Vec<Push>, Vec<Push>) = pushes(&mut mios, QUIET)
        .await
        .into_iter()
        .filter(|push| matches!(push, Push::GatheringUpdated(_) | Push::GatheringEndedId(_)))
        .partition(|push| matches!(push, Push::GatheringUpdated(_)));
    assert_eq!((updated.len(), ended.len()), (1, 1), "shown once with two members, then ended");
    assert!(gatherings(&ctx, &mio).await.is_empty());
    assert_eq!(gatherings(&ctx, &aoi).await.len(), 1);
}

/// C4: footprints say where someone has been since.
#[tokio::test]
async fn footprints_stop_for_a_viewer_where_their_view_of_the_person_stopped() {
    let ctx = start().await;
    let (aoi, ren, kai, stranger) = (
        ctx.app.user("aoi").await,
        ctx.app.user("ren").await,
        ctx.app.user("kai").await,
        ctx.app.user("stranger").await,
    );
    ctx.befriend(&aoi, &ren).await;
    ctx.befriend(&aoi, &kai).await;
    ctx.set_privacy(&aoi, |privacy| {
        privacy.footprints_visible_to = tokuly_proto::identity::Audience::AUDIENCE_EVERYONE.into()
    })
    .await;
    // Two made-up countries far out at sea.
    let (first, second) = (somewhere_at_sea(), somewhere_at_sea());
    for (center, name) in [(first, "Firstland"), (second, "Secondland")] {
        make_country(&ctx, center, name).await;
    }
    ride(&ctx, &aoi, first, 1000.0, minutes_ago(30)).await;
    ride(&ctx, &ren, first, 300.0, minutes_ago(30)).await;

    let before_cells = cells_of(&ctx, &ren, &aoi, first).await.unwrap();
    let before_stats = stats_of(&ctx, &ren, &aoi).await;
    let before_board = leaderboard(&ctx, &ren, LeaderboardScope::LEADERBOARD_SCOPE_FRIENDS).await;
    let before_global = leaderboard(&ctx, &ren, LeaderboardScope::LEADERBOARD_SCOPE_GLOBAL).await;
    assert!(before_cells.len() >= 3 && before_stats.country_count == 1);
    assert!(before_global.iter().any(|(id, _)| *id == aoi.id.to_string()));

    // Frozen for Ren (Kai goes on seeing her). She keeps exploring, into a new country.
    ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_FROZEN, 0).await.unwrap();
    ride(&ctx, &aoi, offset(first, 0.0, 3000.0), 1500.0, minutes_ago(15)).await;
    ride(&ctx, &aoi, second, 1000.0, minutes_ago(8)).await;

    // Ren: the map, the numbers and the rankings of the moment she froze.
    assert_eq!(cells_of(&ctx, &ren, &aoi, first).await.unwrap(), before_cells);
    assert!(cells_of(&ctx, &ren, &aoi, second).await.unwrap().is_empty());
    assert_eq!(stats_of(&ctx, &ren, &aoi).await, before_stats);
    assert_eq!(leaderboard(&ctx, &ren, LeaderboardScope::LEADERBOARD_SCOPE_FRIENDS).await, before_board);
    assert_eq!(leaderboard(&ctx, &ren, LeaderboardScope::LEADERBOARD_SCOPE_GLOBAL).await, before_global);
    // Kai: all of it.
    assert!(cells_of(&ctx, &kai, &aoi, first).await.unwrap().len() > before_cells.len());
    assert!(!cells_of(&ctx, &kai, &aoi, second).await.unwrap().is_empty());
    assert_eq!(stats_of(&ctx, &kai, &aoi).await.country_count, 2);
    // The new country is news for herself; it is not published to "friends",
    // because one of them does not see where she is.
    assert_eq!(ctx.notifications_for(aoi.id, "geo.new_region").await.len(), 1);
    let entries = ctx.activities_of(aoi.id, "geo.new_region").await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].data["audience"], "private");

    // Whoever looks without being a friend (a stranger, someone signed out —
    // Ren on a second account) gets the public footprints as of the last
    // moment every friend saw her precisely: the public view is no way
    // around a ghost rule. And she is not in the public ranking meanwhile.
    assert_eq!(stats_of(&ctx, &stranger, &aoi).await, before_stats);
    assert_eq!(cells_of(&ctx, &stranger, &aoi, first).await.unwrap(), before_cells);
    assert!(cells_of(&ctx, &stranger, &aoi, second).await.unwrap().is_empty());
    let signed_out: tokuly_proto::identity::GetProfileResponse = ctx
        .app
        .call(
            "tokuly.identity.v1.ProfileService/GetProfile",
            None,
            &tokuly_proto::identity::GetProfileRequest {
                by: Some(tokuly_proto::identity::get_profile_request::By::UserId(aoi.id.to_string())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(signed_out.profile.as_option().unwrap().stats.as_option().unwrap().visited_country_count, 1);
    assert!(
        !leaderboard(&ctx, &stranger, LeaderboardScope::LEADERBOARD_SCOPE_GLOBAL)
            .await
            .iter()
            .any(|(id, _)| *id == aoi.id.to_string())
    );
    // Kai, a friend who sees her, finds her there with her live numbers.
    let kais_board = leaderboard(&ctx, &kai, LeaderboardScope::LEADERBOARD_SCOPE_GLOBAL).await;
    let live = stats_of(&ctx, &kai, &aoi).await.total_cell_count;
    assert!(kais_board.contains(&(aoi.id.to_string(), live)), "{kais_board:?}");
    ctx.ghost(&aoi, &[], GhostMode::GHOST_MODE_BLURRED, 0).await.unwrap();
    ride(&ctx, &aoi, offset(second, 0.0, 3000.0), 1000.0, minutes_ago(2)).await;
    assert_eq!(stats_of(&ctx, &stranger, &aoi).await, before_stats);
    // Ren's view has not moved through any of this.
    assert_eq!(stats_of(&ctx, &ren, &aoi).await, before_stats);
    assert_eq!(leaderboard(&ctx, &ren, LeaderboardScope::LEADERBOARD_SCOPE_GLOBAL).await, before_global);

    // "Nobody" means nobody, numbers included.
    ctx.set_privacy(&aoi, |privacy| {
        privacy.footprints_visible_to = tokuly_proto::identity::Audience::AUDIENCE_NOBODY.into()
    })
    .await;
    for viewer in [&ren, &kai, &stranger] {
        let request = GetFootprintStatsRequest { user_id: aoi.id.to_string(), ..Default::default() };
        let denied = ctx.footprints::<_, GetFootprintStatsResponse>("GetStats", viewer, &request).await.unwrap_err();
        assert_eq!(denied.reason, "FOOTPRINTS_PRIVATE");
    }
}

fn somewhere_at_sea() -> Point {
    use rand::RngExt as _;
    let mut rng = rand::rng();
    (rng.random_range(-48.0..-32.0), rng.random_range(-135.0..-105.0))
}

async fn make_country(ctx: &Ctx, center: Point, name: &str) {
    let id = format!("G{}", tokuly_core::ids::random_base62(6).to_ascii_uppercase());
    sqlx::query(
        "INSERT INTO geo_regions (id, kind, name, country_code, area_km2, center_lat, center_lng, geom)
         SELECT $1, 2, $4, $1, ST_Area(g::geography) / 1000000.0, $3, $2, g
         FROM (SELECT ST_Multi(ST_MakeEnvelope($2 - 0.2, $3 - 0.2, $2 + 0.2, $3 + 0.2, 4326)) AS g) s",
    )
    .bind(&id)
    .bind(center.1)
    .bind(center.0)
    .bind(name)
    .execute(&ctx.app.core.db)
    .await
    .expect("insert region");
    ctx.geo.reload_regions();
}

/// C5: a place must not work as a tripwire.
#[tokio::test]
async fn a_place_tells_its_owner_nothing_about_people_they_do_not_see() {
    let ctx = start().await;
    let (aoi, ren, kai, stranger) = (
        ctx.app.user("aoi").await,
        ctx.app.user("ren").await,
        ctx.app.user("kai").await,
        ctx.app.user("stranger").await,
    );
    ctx.befriend(&aoi, &ren).await;
    ctx.befriend(&aoi, &kai).await;
    ctx.befriend(&ren, &kai).await;
    let spot = somewhere();
    // Ren lays a large place over the area; a stranger a public one next to it.
    let trap = create_place(&ctx, &ren, "見張り", spot, 1000.0, Visibility::VISIBILITY_FRIENDS).await;
    let far = offset(spot, 0.0, 5000.0);
    let public_trap = create_place(&ctx, &stranger, "公開の罠", far, 1000.0, Visibility::VISIBILITY_PUBLIC).await;
    ctx.report(&aoi, vec![sample(offset(spot, 180.0, 9000.0), minutes_ago(40))]).await;
    ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_FROZEN, 0).await.unwrap();

    // Aoi stays inside both, one after the other.
    ctx.stay(&aoi, offset(far, 45.0, 300.0), minutes_ago(20), minutes_ago(12)).await;
    ctx.stay(&aoi, offset(spot, 45.0, 300.0), minutes_ago(8), Utc::now()).await;
    let stays = visits(&ctx, &aoi).await;
    assert_eq!(stays.len(), 2, "her own history has both stays");

    // The owners learn nothing: not a name, not a number, now or later.
    for (owner, place) in [(&ren, &trap), (&stranger, &public_trap)] {
        let seen = card(&ctx, owner, &place.id).await;
        assert_eq!((seen.total_visit_count, seen.regulars.len(), seen.friends_visited.len()), (0, 0, 0));
        assert!(seen.badges.is_empty());
    }
    // Kai sees her precisely and may open Ren's place: he sees she was there.
    let kais = card(&ctx, &kai, &trap.id).await;
    assert_eq!(kais.total_visit_count, 1);
    assert_eq!(kais.friends_visited.iter().map(|user| user.id.clone()).collect::<Vec<_>>(), vec![aoi.id.to_string()]);
    // The search index does not rank places by visits either.
    let indexed = ctx.app.events("search.changed").await;
    let latest = indexed.iter().rfind(|event| event.data["id"] == serde_json::json!(public_trap.id)).unwrap();
    assert_eq!(latest.data["doc"]["score"], 0.0);
    // Lifting the rule later does not bring those stays to light.
    ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_PRECISE, 0).await.unwrap();
    assert_eq!(card(&ctx, &ren, &trap.id).await.total_visit_count, 0);
}

/// M5: approval of an alert on someone's home does not outlive their
/// decision to stop sharing where home is.
#[tokio::test]
async fn a_home_alert_stops_when_place_labels_are_no_longer_shared() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    let request = CreatePlaceRequest {
        name: "うち".into(),
        label: PlaceLabel::PLACE_LABEL_HOME.into(),
        center: lat_lng(home).into(),
        ..Default::default()
    };
    let _: CreatePlaceResponse = ctx.places("CreatePlace", &aoi, &request).await.unwrap();
    let away = offset(home, 0.0, 2000.0);
    ctx.at(&aoi, away).await;
    let alert = CreatePlaceAlertRequest {
        subject_user_id: aoi.id.to_string(),
        r#where: Some(create_place_alert_request::Where::SubjectPlaceLabel(PlaceLabel::PLACE_LABEL_HOME.into())),
        trigger: AlertTrigger::ALERT_TRIGGER_ARRIVE.into(),
        recurring: true,
        ..Default::default()
    };
    let created: CreatePlaceAlertResponse = ctx.safety("CreatePlaceAlert", &ren, &alert).await.unwrap();
    let alert_id = created.alert.into_option().unwrap().id;
    let _: RespondPlaceAlertResponse = ctx
        .safety("RespondPlaceAlert", &aoi, &RespondPlaceAlertRequest { alert_id, approve: true, ..Default::default() })
        .await
        .unwrap();
    ctx.at(&aoi, away).await;
    ctx.at(&aoi, home).await;
    assert_eq!(ctx.notifications_for(ren.id, "geo.place_alert").await.len(), 1, "it works while labels are shared");

    ctx.set_privacy(&aoi, |privacy| privacy.share_place_labels = false).await;
    let mut rens = ctx.open_app(&ren).await;
    for spot in [away, home, away, home] {
        ctx.at(&aoi, spot).await;
    }
    assert_eq!(ctx.notifications_for(ren.id, "geo.place_alert").await.len(), 1, "and not after");
    assert!(pushes(&mut rens, QUIET).await.iter().all(|push| !matches!(push, Push::PlaceAlertFired(_))));
}

/// M6: a check-in says where the user is.
#[tokio::test]
async fn a_check_in_is_no_map_story_while_one_friend_is_frozen_out() {
    let ctx = start().await;
    let (aoi, ren, mio) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("mio").await);
    ctx.befriend(&aoi, &ren).await;
    ctx.befriend(&aoi, &mio).await;
    let spot = somewhere();
    ctx.at(&aoi, spot).await;
    ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_FROZEN, 0).await.unwrap();
    let mut rens = ctx.open_app(&ren).await;
    let here = offset(spot, 0.0, 3000.0);
    ctx.at(&aoi, here).await;
    let request = CheckInRequest {
        r#where: Some(Where::Position(Box::new(lat_lng(here)))),
        with_user_ids: vec![ren.id.to_string()],
        message: "ここだよ".into(),
        ..Default::default()
    };
    let _: CheckInResponse = ctx.places("CheckIn", &aoi, &request).await.unwrap();

    // Mio sees the pin anyway and is told; Ren is not, tagged or not.
    assert_eq!(ctx.notifications_for(mio.id, "geo.checkin").await.len(), 1);
    assert!(ctx.notifications_for(ren.id, "geo.checkin").await.is_empty());
    assert!(ctx.notifications_for(ren.id, "geo.checkin_tagged").await.is_empty());
    assert!(pushes(&mut rens, QUIET).await.is_empty());
    // And it is not a story on friends' maps: that could not leave Ren out.
    let story = ctx.activities_of(aoi.id, "geo.visit").await.pop().unwrap();
    assert_eq!(story.data["audience"], "private");
    assert!(story.data["location"].is_null());
}

/// M4: the same samples processed twice (a redelivery after a failure
/// between the writes and saving the live state) count once.
#[tokio::test]
async fn a_batch_processed_twice_counts_once() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    let from = somewhere();
    let place = create_place(&ctx, &aoi, "公園", offset(from, 90.0, 840.0), 80.0, Visibility::VISIBILITY_PRIVATE).await;
    let start = minutes_ago(25);
    let mut samples: Vec<LocationSample> =
        (0..=60).map(|i| sample(offset(from, 90.0, i as f64 * 14.0), start + Duration::seconds(i * 10))).collect();
    samples.extend((1..=16).map(|i| sample(offset(from, 90.0, 840.0), start + Duration::seconds(600 + i * 30))));
    let sent = samples.len() as i32;

    let state_key = format!("geo:state:{}", aoi.id.simple());
    assert_eq!(ctx.report(&aoi, samples.clone()).await.accepted_count, sent);
    let once = (today(&ctx, &aoi).await, card(&ctx, &aoi, &place.id).await.total_visit_count);
    let cells_once = stats_of(&ctx, &aoi, &aoi).await.total_cell_count;
    // The live state is lost, as if it had never been saved; the batch comes again.
    ctx.app.core.cache.del(&state_key).await.unwrap();
    assert_eq!(ctx.report(&aoi, samples).await.accepted_count, sent);
    let twice = (today(&ctx, &aoi).await, card(&ctx, &aoi, &place.id).await.total_visit_count);

    let (summary_once, summary_twice) = (once.0.summary.as_option().unwrap(), twice.0.summary.as_option().unwrap());
    assert!(summary_once.distance_meters > 700.0);
    assert_eq!(summary_twice.distance_meters, summary_once.distance_meters);
    assert_eq!((once.0.visits.len(), twice.0.visits.len()), (1, 1));
    assert_eq!(twice.0.visits[0].id, once.0.visits[0].id, "the same visit, not a second one");
    assert_eq!((once.1, twice.1), (1, 1));
    assert_eq!(stats_of(&ctx, &aoi, &aoi).await.total_cell_count, cells_once);
}

/// Minor findings, each in a few lines.
#[tokio::test]
async fn smaller_leaks_and_loose_ends_are_closed() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();

    // A flood of fixes: at most one a second (device time) is processed.
    let burst: Vec<LocationSample> = (0..50)
        .map(|i| sample(offset(home, 0.0, i as f64), minutes_ago(40) + Duration::milliseconds(i * 100)))
        .collect();
    assert_eq!(ctx.report(&aoi, burst).await.accepted_count, 5);

    // A fix after an impossible jump starts over: no line of cells from where the last footprint was.
    let before = stats_of(&ctx, &aoi, &aoi).await.total_cell_count;
    let far = offset(home, 90.0, 4500.0);
    ctx.report(
        &aoi,
        vec![sample(far, minutes_ago(40) + Duration::seconds(6)), sample(offset(far, 90.0, 30.0), minutes_ago(39))],
    )
    .await;
    assert_eq!(stats_of(&ctx, &aoi, &aoi).await.total_cell_count, before + 1, "one new spot, not a bridge of cells");

    // A visit that began before a deleted range and reaches into it goes with it.
    ctx.stay(&aoi, home, minutes_ago(30), minutes_ago(10)).await;
    ctx.at(&aoi, offset(home, 0.0, 600.0)).await;
    assert_eq!(visits(&ctx, &aoi).await.len(), 1);
    let range = DeleteHistoryRequest { from: ts(minutes_ago(20)), to: ts(minutes_ago(15)), ..Default::default() };
    let _: DeleteHistoryResponse = ctx.history("DeleteHistory", &aoi, &range).await.unwrap();
    assert!(visits(&ctx, &aoi).await.is_empty());

    // Share links: the table holds no token, and the link shows what the privacy switches allow.
    ctx.set_privacy(&aoi, |privacy| {
        privacy.share_battery = false;
        privacy.share_activity = false;
    })
    .await;
    ctx.report(
        &aoi,
        vec![LocationSample { battery_percent: Some(41), speed_mps: Some(1.2), ..sample(home, next_fix_time(aoi.id)) }],
    )
    .await;
    let created: CreateLiveShareResponse = ctx
        .location("CreateLiveShare", &aoi, &CreateLiveShareRequest { duration_seconds: 600, ..Default::default() })
        .await
        .unwrap();
    let token = created.share.into_option().unwrap().url.rsplit('/').next().unwrap().to_owned();
    let stored: (String, Vec<u8>) =
        sqlx::query_as("SELECT token_hash, token_sealed FROM geo_live_shares WHERE user_id = $1")
            .bind(aoi.id)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert!(stored.0 != token && !stored.1.windows(token.len()).any(|window| window == token.as_bytes()));
    let opened: GetLiveShareResponse = ctx
        .app
        .call("tokuly.geo.v1.LocationService/GetLiveShare", None, &GetLiveShareRequest { token, ..Default::default() })
        .await
        .unwrap();
    let pin = opened.location.into_option().expect("the pin behind the link");
    assert_at!(pin_point(&pin), home);
    assert_eq!((pin.battery_percent, pin.speed_mps), (None, 0.0));

    // A deleted account's samples that were still on their way leave nothing behind.
    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1")
        .bind(ren.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.deliver("identity.user.deleted", ren.id, serde_json::json!({ "user_id": ren.id })).await;
    let report = tokuly_core::presence::LocationReport {
        user_id: ren.id,
        session_id: None,
        received_at: Utc::now(),
        sample: sample(home, Utc::now()),
    };
    ctx.geo.ingest(report).await.unwrap();
    let state: Option<serde_json::Value> =
        ctx.app.core.cache.get_json(&format!("geo:state:{}", ren.id.simple())).await.unwrap();
    assert!(state.is_none(), "no live state comes back for a deleted account");
}

async fn json_call(ctx: &Ctx, user: &TestUser, path: &str, request: serde_json::Value) -> serde_json::Value {
    match ctx.app.call::<serde_json::Value, serde_json::Value>(&format!("tokuly.{path}"), Some(user), &request).await {
        Ok(response) => response,
        Err(error) => serde_json::json!({ "error": error.reason }),
    }
}

/// Re-check 1: Buzz (the social domain) asks the shared position index
/// whether two friends are near each other. The index answers for a pair
/// only if each sees the other precisely — and the limit on asking comes
/// before the answer.
#[tokio::test]
async fn buzz_cannot_find_a_frozen_friend() {
    let ctx = start().await;
    let (aoi, ren, kai) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("kai").await);
    ctx.befriend(&aoi, &ren).await;
    ctx.befriend(&aoi, &kai).await;
    let home = somewhere();
    ctx.at(&aoi, home).await;
    ctx.at(&ren, offset(home, 90.0, 3000.0)).await;
    ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_FROZEN, 0).await.unwrap();
    let hideout = offset(home, 0.0, 5000.0);
    ctx.at(&aoi, hideout).await;
    let buzz = |target: &TestUser| serde_json::json!({ "userId": target.id });

    // Ren reports himself at wrong guesses, at her frozen pin, and 150 m from
    // where she really is: the same answer everywhere.
    for spot in [offset(home, 90.0, 5000.0), offset(home, 0.0, 20.0), offset(hideout, 90.0, 150.0)] {
        ctx.at(&ren, spot).await;
        let answer = json_call(&ctx, &ren, "social.v1.InteractionService/Buzz", buzz(&aoi)).await;
        assert_eq!(answer["error"], "NOT_NEARBY", "at {spot:?}");
    }
    // Neither can she buzz him: "near" would be something both learn.
    assert_eq!(json_call(&ctx, &aoi, "social.v1.InteractionService/Buzz", buzz(&ren)).await["error"], "NOT_NEARBY");
    // Kai sees her and stands next to her: that works as ever.
    ctx.at(&kai, offset(hideout, 180.0, 100.0)).await;
    assert!(json_call(&ctx, &kai, "social.v1.InteractionService/Buzz", buzz(&aoi)).await["error"].is_null());
    // The same under a timed rule and a rule for everyone; and once the rule is gone it works again.
    ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_PRECISE, 0).await.unwrap();
    ctx.at(&ren, offset(hideout, 90.0, 150.0)).await;
    assert!(json_call(&ctx, &ren, "social.v1.InteractionService/Buzz", buzz(&aoi)).await["error"].is_null());
    ctx.ghost(&aoi, &[], GhostMode::GHOST_MODE_BLURRED, 600).await.unwrap();
    assert_eq!(json_call(&ctx, &ren, "social.v1.InteractionService/Buzz", buzz(&aoi)).await["error"], "NOT_NEARBY");
    // Asking is limited before it is answered: after ten tries in a minute
    // there is no answer about distance at all.
    let mut answers = Vec::new();
    for _ in 0..12 {
        answers.push(json_call(&ctx, &ren, "social.v1.InteractionService/Buzz", buzz(&aoi)).await["error"].clone());
    }
    assert_eq!(answers.last().unwrap(), "RATE_LIMITED", "{answers:?}");
}

/// Re-check 2: two friends the viewer sees, 80 m apart, are not a gathering
/// for that viewer just because someone they do not see stands in between.
#[tokio::test]
async fn a_hidden_member_does_not_bridge_a_gathering() {
    let ctx = start().await;
    let (aoi, ren, kai, mio) =
        (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("kai").await, ctx.app.user("mio").await);
    for (a, b) in [(&aoi, &ren), (&aoi, &kai), (&aoi, &mio), (&ren, &kai), (&ren, &mio), (&kai, &mio)] {
        ctx.befriend(a, b).await;
    }
    let spot = somewhere();
    let (west, east) = (offset(spot, 270.0, 40.0), offset(spot, 90.0, 40.0));
    let t = Utc::now();
    // Aoi is frozen for Ren somewhere else, then stands between Kai and Mio.
    seen_at(&ctx, &aoi, offset(spot, 0.0, 6000.0), t, &[400]).await;
    ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_FROZEN, 0).await.unwrap();
    let mut rens = ctx.open_app(&ren).await;
    for (user, at, seconds) in
        [(&aoi, spot, vec![300, 270, 240]), (&kai, west, vec![300, 270, 240]), (&mio, east, vec![300, 270, 240])]
    {
        seen_at(&ctx, user, at, t, &seconds).await;
    }
    for (user, at, seconds) in
        [(&aoi, spot, 200), (&kai, west, 110), (&mio, east, 110), (&aoi, spot, 60), (&kai, west, 40), (&mio, east, 40)]
    {
        seen_at(&ctx, user, at, t, &[seconds]).await;
    }
    // The three are a gathering — for those who see all three.
    assert_eq!(gatherings(&ctx, &kai).await.pop().expect("the gathering").members.len(), 3);
    // Ren sees Kai and Mio, who are not within reach of each other: nothing, by list or by push.
    assert!(gatherings(&ctx, &ren).await.is_empty());
    assert_eq!(gathering_pushes(&pushes(&mut rens, QUIET).await), 0);
    assert!(ctx.notifications_for(ren.id, "geo.gathering").await.is_empty());
    // Mio steps over to Kai: now the two of them are together by themselves, centred between them.
    let beside = offset(west, 90.0, 10.0);
    seen_at(&ctx, &mio, beside, t, &[20, -5]).await;
    let seen = gatherings(&ctx, &ren).await.pop().expect("Kai and Mio together");
    assert_eq!(seen.members.len(), 2);
    let middle = ((west.0 + beside.0) / 2.0, (west.1 + beside.1) / 2.0);
    assert!(distance(point(seen.center.as_option()), middle) < 0.01);
}

/// Re-check 3: a timed override that ends under a default set later keeps
/// its own copy of the pin and its own watermark, from the first moment —
/// not only once the sweeper has been by.
#[tokio::test]
async fn an_override_that_ends_under_a_stricter_default_shows_nothing_newer() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    ride(&ctx, &aoi, home, 300.0, minutes_ago(20)).await;
    ctx.at(&aoi, home).await;
    ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_FROZEN, 2).await.unwrap();
    let before = ctx.seen_by(&ren, &aoi).await.unwrap();
    let cells_before = cells_of(&ctx, &ren, &aoi, home).await.unwrap();
    let cafe = offset(home, 45.0, 4000.0);
    ride(&ctx, &aoi, cafe, 300.0, Utc::now() + Duration::seconds(5)).await;
    ctx.ghost(&aoi, &[], GhostMode::GHOST_MODE_FROZEN, 0).await.unwrap();
    assert_eq!(ctx.seen_by(&ren, &aoi).await.unwrap(), before);

    // The override's time is up; the sweeper has not run.
    tokio::time::sleep(std::time::Duration::from_millis(2300)).await;
    assert_eq!(
        ctx.seen_by(&ren, &aoi).await.unwrap(),
        before,
        "the pin Ren was left with, not the default's newer copy"
    );
    assert_eq!(cells_of(&ctx, &ren, &aoi, home).await.unwrap(), cells_before);
    // (Nor footprints from in between: the box around the café holds what the box around home held.)
    assert_eq!(cells_of(&ctx, &ren, &aoi, cafe).await.unwrap(), cells_before);
    // After the sweeper: the same.
    ctx.geo.sweep().await;
    assert_eq!(ctx.seen_by(&ren, &aoi).await.unwrap(), before);
    assert_eq!(cells_of(&ctx, &ren, &aoi, cafe).await.unwrap(), cells_before);
    assert!(
        cells_of(&ctx, &aoi, &aoi, cafe).await.unwrap().len() > cells_before.len(),
        "she did leave footprints there"
    );
}

/// Re-check 4: a blurred viewer is sent the stand-in when what they see
/// changes, not whenever the person moves, and its time is coarse.
#[tokio::test]
async fn pushes_to_a_blurred_viewer_do_not_follow_movement() {
    let ctx = start().await;
    let mut counts = Vec::new();
    for moving in [false, true] {
        let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
        ctx.befriend(&aoi, &ren).await;
        let anywhere = somewhere();
        let cell = tokuly_geo::math::cell_at(anywhere.0, anywhere.1, tokuly_geo::visibility::BLUR_RESOLUTION).unwrap();
        let base = tokuly_geo::math::cell_center(cell);
        // Twelve fixes in a minute that lies inside one ten-minute step of the clock.
        let step_ms = tokuly_geo::visibility::BLUR_TIME_STEP_MS;
        let now_ms = Utc::now().timestamp_millis();
        let t = DateTime::from_timestamp_millis((now_ms - 200_000).div_euclid(step_ms) * step_ms + 60_000).unwrap();
        ctx.report(&aoi, vec![sample(base, t - Duration::seconds(30))]).await;
        ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_BLURRED, 0).await.unwrap();
        let mut rens = ctx.open_app(&ren).await;
        pushes(&mut rens, QUIET).await;
        let mut seen = Vec::new();
        for step in 0..12i64 {
            let here = if moving { offset(base, 90.0, 10.0 * step as f64) } else { base };
            let fix = LocationSample { battery_percent: Some(50), ..sample(here, t + Duration::seconds(5 * step)) };
            ctx.report(&aoi, vec![fix]).await;
            seen.extend(pins(&mut rens, std::time::Duration::from_millis(120)).await);
        }
        counts.push(seen.len());
        // What is listed is coarse in time as well.
        let listed = ctx.seen_by(&ren, &aoi).await.unwrap();
        assert_eq!(listed.recorded_at.as_option().unwrap().seconds * 1000 % step_ms, 0);
        assert_eq!(listed.precision, GhostMode::GHOST_MODE_BLURRED);
    }
    assert_eq!(counts[0], counts[1], "as many pushes walking as standing still: {counts:?}");
    assert!(counts[0] <= 1, "{counts:?}");
}

/// A blurred viewer away from the app gets no push, and back with the
/// snapshot, the next push still waits for a change they can see: a push
/// of what they already have would say the person sent a fix just now.
#[tokio::test]
async fn a_blurred_viewer_back_in_the_app_is_not_told_of_movement_they_cannot_see() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let cell_of = |p: (f64, f64)| tokuly_geo::math::cell_at(p.0, p.1, tokuly_geo::visibility::BLUR_RESOLUTION).unwrap();
    let base = tokuly_geo::math::cell_center(cell_of(somewhere()));
    let elsewhere = tokuly_geo::math::cell_center(cell_of(offset(base, 90.0, 30_000.0)));
    let step_ms = tokuly_geo::visibility::BLUR_TIME_STEP_MS;
    let now_ms = Utc::now().timestamp_millis();
    let t = DateTime::from_timestamp_millis((now_ms - 200_000).div_euclid(step_ms) * step_ms + 60_000).unwrap();
    ctx.report(&aoi, vec![sample(base, t - Duration::seconds(30))]).await;
    ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_BLURRED, 0).await.unwrap();

    let mut away = ctx.app.listen_user(ren.id).await;
    ctx.report(&aoi, vec![sample(elsewhere, t)]).await;
    assert!(pins(&mut away, QUIET).await.is_empty(), "nothing while the app is closed");

    let mut rens = ctx.open_app(&ren).await;
    let listed = ctx.seen_by(&ren, &aoi).await.unwrap();
    assert_eq!(listed.precision, GhostMode::GHOST_MODE_BLURRED);
    ctx.report(&aoi, vec![sample(offset(elsewhere, 0.0, 20.0), t + Duration::seconds(10))]).await;
    assert!(pins(&mut rens, QUIET).await.is_empty(), "what they see did not change");
}

/// Re-check 6: when an SOS ends, the people it alerted keep the pin they
/// last saw; it does not jump back to where it was frozen before.
#[tokio::test]
async fn after_an_sos_the_frozen_pin_is_where_it_was_last_seen() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    ctx.at(&aoi, home).await;
    ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_FROZEN, 0).await.unwrap();
    let away = offset(home, 200.0, 7000.0);
    ctx.at(&aoi, away).await;
    let sos =
        json_call(&ctx, &aoi, "geo.v1.SafetyService/TriggerSos", serde_json::json!({ "notifyUserIds": [ren.id] }))
            .await;
    let during = ctx.seen_by(&ren, &aoi).await.unwrap();
    assert_at!(pin_point(&during), away);
    let resolved =
        json_call(&ctx, &aoi, "geo.v1.SafetyService/ResolveSos", serde_json::json!({ "sosId": sos["sos"]["id"] }))
            .await;
    assert!(resolved["error"].is_null(), "{resolved}");
    let after = ctx.seen_by(&ren, &aoi).await.unwrap();
    assert_at!(pin_point(&after), away);
    assert_eq!(after.recorded_at, during.recorded_at, "time does not run backwards on Ren's map");
    // And from here on it is frozen again.
    ctx.at(&aoi, offset(away, 0.0, 3000.0)).await;
    assert_eq!(ctx.seen_by(&ren, &aoi).await.unwrap().position, after.position);
}
