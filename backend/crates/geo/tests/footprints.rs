//! Footprints: uncovering cells through the pipeline, viewport queries,
//! explored share per region, leaderboards and the recap.
mod common;

use std::str::FromStr as _;

use chrono::{DateTime, Duration, Utc};
use common::*;
use rand::RngExt as _;
use tokuly_geo::GeoConfig;
use tokuly_proto::{
    common::BoundingBox,
    geo::{geo_event::Event as Push, *},
    identity::Audience,
};
use tokuly_testkit::{RpcError, TestUser};

/// A box of `half_deg` degrees around `center`.
fn around(center: Point, half_deg: f64) -> BoundingBox {
    BoundingBox {
        south_west: lat_lng((center.0 - half_deg, center.1 - half_deg)).into(),
        north_east: lat_lng((center.0 + half_deg, center.1 + half_deg)).into(),
        ..Default::default()
    }
}

async fn cells(
    ctx: &Ctx,
    viewer: &TestUser,
    owner: Option<&TestUser>,
    bounds: BoundingBox,
    zoom: f64,
) -> Result<GetCellsResponse, RpcError> {
    let request = GetCellsRequest {
        user_id: owner.map(|owner| owner.id.to_string()).unwrap_or_default(),
        bounds: bounds.into(),
        zoom,
        ..Default::default()
    };
    ctx.footprints("GetCells", viewer, &request).await
}

async fn stats(ctx: &Ctx, viewer: &TestUser, owner: Option<&TestUser>) -> GetFootprintStatsResponse {
    let request = GetFootprintStatsRequest {
        user_id: owner.map(|owner| owner.id.to_string()).unwrap_or_default(),
        ..Default::default()
    };
    ctx.footprints("GetStats", viewer, &request).await.expect("footprint stats")
}

/// The fixes of a ride east from `from` for `meters`: one every 50 m.
fn track(from: Point, meters: f64) -> Vec<Point> {
    let steps = (meters / 50.0).round() as i64;
    (0..=steps).map(|i| offset(from, 90.0, i as f64 * 50.0)).collect()
}

/// Reports a ride along `track` at 5 m/s (a sample every 10 s), starting at `start`.
async fn ride_along(ctx: &Ctx, user: &TestUser, track: &[Point], start: DateTime<Utc>) {
    let samples: Vec<LocationSample> =
        track.iter().zip(0..).map(|(point, i)| sample(*point, start + Duration::seconds(i * 10))).collect();
    let sent = samples.len() as i32;
    assert_eq!(ctx.report(user, samples).await.accepted_count, sent);
}

/// Rides east from `from` for `meters`, starting at `start`.
async fn ride(ctx: &Ctx, user: &TestUser, from: Point, meters: f64, start: DateTime<Utc>) {
    ride_along(ctx, user, &track(from, meters), start).await;
}

/// A spot in the open South Pacific: no country, no town, no other test.
fn open_ocean() -> Point {
    let mut rng = rand::rng();
    (rng.random_range(-45.0..-35.0), rng.random_range(-130.0..-110.0))
}

/// Inserts a rectangular region and returns its area in km² as PostGIS computes it.
async fn make_region(
    ctx: &Ctx,
    id: &str,
    kind: i16,
    name: &str,
    name_ja: &str,
    country: &str,
    center: Point,
    half_deg: f64,
) -> f64 {
    let area: f64 = sqlx::query_scalar(
        "INSERT INTO geo_regions (id, kind, name, name_ja, country_code, area_km2, center_lat, center_lng, geom)
         SELECT $1, $2, $3, $4, $5, ST_Area(g::geography) / 1000000.0, $7, $6, g
         FROM (SELECT ST_Multi(ST_MakeEnvelope($6 - $8, $7 - $8, $6 + $8, $7 + $8, 4326)) AS g) s
         RETURNING area_km2",
    )
    .bind(id)
    .bind(kind)
    .bind(name)
    .bind(name_ja)
    .bind(country)
    .bind(center.1)
    .bind(center.0)
    .bind(half_deg)
    .fetch_one(&ctx.app.core.db)
    .await
    .expect("insert region");
    ctx.geo.reload_regions();
    area
}

/// A made-up country with one prefecture inside it, around `center`. Returns their IDs and areas.
async fn make_island(ctx: &Ctx, center: Point) -> ((String, f64), (String, f64)) {
    let tag = tokuly_core::ids::random_base62(6).to_ascii_uppercase();
    let (country, prefecture) = (format!("T{tag}"), format!("T{tag}-01"));
    let country_area = make_region(ctx, &country, 2, "Testland", "テスト国", &country, center, 0.2).await;
    let prefecture_area = make_region(ctx, &prefecture, 3, "Test Prefecture", "テスト県", &country, center, 0.1).await;
    ((country, country_area), (prefecture, prefecture_area))
}

fn footprint_progress(pushes: Vec<Push>) -> Vec<FootprintProgress> {
    pushes
        .into_iter()
        .filter_map(|push| if let Push::FootprintProgress(progress) = push { Some(*progress) } else { None })
        .collect()
}

#[tokio::test]
async fn moving_uncovers_cells_at_three_resolutions() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    let from = somewhere();
    let mut aois = ctx.app.listen_user(aoi.id).await;
    ride(&ctx, &aoi, from, 2000.0, minutes_ago(10)).await;

    // Finest resolution at street zoom.
    let fine = cells(&ctx, &aoi, None, around(from, 0.1), 15.0).await.unwrap();
    assert_eq!(fine.resolution, 9);
    assert!(!fine.truncated);
    assert!((5..=12).contains(&fine.cells.len()), "{} cells along 2 km", fine.cells.len());
    for hex in &fine.cells {
        assert_eq!(hex.len(), 15);
        assert_eq!(u8::from(h3o::CellIndex::from_str(hex).expect("an H3 index").resolution()), 9);
    }
    let start_cell = h3o::LatLng::new(from.0, from.1).unwrap().to_cell(h3o::Resolution::Nine).to_string();
    assert!(fine.cells.contains(&start_cell));

    // Coarser cells further out; each is the parent of cells that were visited.
    let town = cells(&ctx, &aoi, None, around(from, 0.5), 9.0).await.unwrap();
    assert_eq!(town.resolution, 7);
    assert!((1..=3).contains(&town.cells.len()) && town.cells.len() < fine.cells.len());
    let parents: std::collections::HashSet<String> = fine
        .cells
        .iter()
        .map(|hex| h3o::CellIndex::from_str(hex).unwrap().parent(h3o::Resolution::Seven).unwrap().to_string())
        .collect();
    assert_eq!(town.cells.iter().cloned().collect::<std::collections::HashSet<_>>(), parents);
    let country = cells(&ctx, &aoi, None, around(from, 5.0), 4.0).await.unwrap();
    assert_eq!(country.resolution, 5);
    assert!((1..=2).contains(&country.cells.len()));

    // Only what is inside the viewport.
    let elsewhere = (from.0 + 1.0, from.1);
    assert!(cells(&ctx, &aoi, None, around(elsewhere, 0.1), 15.0).await.unwrap().cells.is_empty());
    let half = cells(
        &ctx,
        &aoi,
        None,
        BoundingBox {
            south_west: lat_lng((from.0 - 0.1, from.1 - 0.1)).into(),
            north_east: lat_lng((from.0 + 0.1, offset(from, 90.0, 1000.0).1)).into(),
            ..Default::default()
        },
        15.0,
    )
    .await
    .unwrap();
    assert!(half.cells.len() < fine.cells.len() && !half.cells.is_empty());
    assert!(
        cells(&ctx, &ren, None, around(from, 0.1), 15.0).await.unwrap().cells.is_empty(),
        "someone else's map is empty here"
    );

    // The user was told as the cells came up.
    let progress = footprint_progress(pushes(&mut aois, QUIET).await);
    assert!(!progress.is_empty());
    let announced: usize = progress.iter().map(|p| p.new_cells.len()).sum();
    assert_eq!(announced, fine.cells.len());
    assert_eq!(progress.last().unwrap().total_cell_count, fine.cells.len() as i64);
    assert_eq!(stats(&ctx, &aoi, None).await.total_cell_count, fine.cells.len() as i64);

    // Riding back over the same ground uncovers nothing.
    let way_back: Vec<Point> = track(from, 2000.0).into_iter().rev().take(17).collect();
    ride_along(&ctx, &aoi, &way_back, minutes_ago(3)).await;
    assert_eq!(stats(&ctx, &aoi, None).await.total_cell_count, fine.cells.len() as i64);
    assert!(footprint_progress(pushes(&mut aois, QUIET).await).is_empty());
}

#[tokio::test]
async fn gaps_are_bridged_but_jumps_and_mock_locations_leave_nothing() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    let from = somewhere();
    // Two fixes a minute and 1.5 km apart (a drive): the cells in between count.
    ctx.report(&aoi, vec![sample(from, minutes_ago(30)), sample(offset(from, 90.0, 1500.0), minutes_ago(29))]).await;
    let bridged = cells(&ctx, &aoi, None, around(from, 0.2), 15.0).await.unwrap().cells.len();
    assert!((4..=10).contains(&bridged), "{bridged} cells");

    // An hour later 40 km away: not connected, just the one new cell.
    let far = offset(from, 0.0, 40_000.0);
    ctx.report(&aoi, vec![sample(far, minutes_ago(20))]).await;
    let total = stats(&ctx, &aoi, None).await.total_cell_count;
    assert_eq!(total, bridged as i64 + 1);

    // A jump no vehicle could make (300 km in ten seconds) is a glitch: no footprint.
    let glitch = offset(far, 0.0, 300_000.0);
    ctx.report(&aoi, vec![sample(glitch, minutes_ago(20) + Duration::seconds(10))]).await;
    assert_eq!(stats(&ctx, &aoi, None).await.total_cell_count, total);

    // Mock locations move the pin but do not scratch the map.
    let spoofed = offset(from, 180.0, 60_000.0);
    ctx.report(&aoi, vec![LocationSample { mocked: true, ..sample(spoofed, minutes_ago(5)) }]).await;
    assert_eq!(stats(&ctx, &aoi, None).await.total_cell_count, total);
    assert!(distance(pin_point(ctx.my_location(&aoi).await.location.as_option().unwrap()), spoofed) < 1.0);

    // Neither do fixes too coarse to say which cell the user was in.
    ctx.report(
        &aoi,
        vec![LocationSample { accuracy_meters: 900.0, ..sample(offset(from, 270.0, 80_000.0), minutes_ago(1)) }],
    )
    .await;
    assert_eq!(stats(&ctx, &aoi, None).await.total_cell_count, total);
}

#[tokio::test]
async fn viewport_requests_are_validated_and_capped() {
    let ctx = start_with(GeoConfig { max_cells: 3, ..GeoConfig::default() }).await;
    let aoi = ctx.app.user("aoi").await;
    let from = somewhere();
    ride(&ctx, &aoi, from, 2000.0, minutes_ago(10)).await;

    let capped = cells(&ctx, &aoi, None, around(from, 0.1), 15.0).await.unwrap();
    assert_eq!(capped.cells.len(), 3);
    assert!(capped.truncated);
    let coarse = cells(&ctx, &aoi, None, around(from, 0.5), 9.0).await.unwrap();
    assert!(!coarse.truncated, "zooming out fits the same ground into fewer cells");

    let reason = |bounds: Option<BoundingBox>| {
        let request = GetCellsRequest { bounds: bounds.into(), zoom: 15.0, ..Default::default() };
        let (ctx, aoi) = (&ctx, aoi.clone());
        async move { ctx.footprints::<_, GetCellsResponse>("GetCells", &aoi, &request).await.unwrap_err().reason }
    };
    assert_eq!(reason(None).await, "INVALID_BOUNDS");
    assert_eq!(
        reason(Some(BoundingBox { south_west: lat_lng(from).into(), ..Default::default() })).await,
        "INVALID_BOUNDS"
    );
    let upside_down = BoundingBox {
        south_west: lat_lng((from.0 + 0.1, from.1)).into(),
        north_east: lat_lng((from.0 - 0.1, from.1 + 0.1)).into(),
        ..Default::default()
    };
    assert_eq!(reason(Some(upside_down)).await, "INVALID_BOUNDS");
    let off_the_map = BoundingBox {
        south_west: lat_lng((0.0, 0.0)).into(),
        north_east: lat_lng((0.0, 400.0)).into(),
        ..Default::default()
    };
    assert_eq!(reason(Some(off_the_map)).await, "INVALID_BOUNDS");
    let bad_user = GetCellsRequest { user_id: "x".into(), bounds: around(from, 0.1).into(), ..Default::default() };
    assert_eq!(
        ctx.footprints::<_, GetCellsResponse>("GetCells", &aoi, &bad_user).await.unwrap_err().reason,
        "INVALID_ID"
    );
    let anonymous = ctx
        .app
        .call::<_, GetCellsResponse>("tokuly.geo.v1.FootprintService/GetCells", None, &GetCellsRequest::default())
        .await
        .unwrap_err();
    assert_eq!(anonymous.reason, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn a_viewport_across_the_antimeridian_finds_cells_on_both_sides() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    let lat = rand::rng().random_range(-50.0..-40.0);
    ctx.report(&aoi, vec![sample((lat, 179.95), minutes_ago(9)), sample((lat + 0.5, -179.95), minutes_ago(4))]).await;
    let crossing = BoundingBox {
        south_west: lat_lng((lat - 1.0, 179.0)).into(),
        north_east: lat_lng((lat + 1.0, -179.0)).into(),
        ..Default::default()
    };
    assert_eq!(cells(&ctx, &aoi, None, crossing, 15.0).await.unwrap().cells.len(), 2);
    let east_only = BoundingBox {
        south_west: lat_lng((lat - 1.0, 179.0)).into(),
        north_east: lat_lng((lat + 1.0, 180.0)).into(),
        ..Default::default()
    };
    assert_eq!(cells(&ctx, &aoi, None, east_only, 15.0).await.unwrap().cells.len(), 1);
}

#[tokio::test]
async fn the_explored_share_is_cell_area_over_region_area() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    let center = open_ocean();
    let ((country, country_area), (prefecture, prefecture_area)) = make_island(&ctx, center).await;
    // The area of a 0.2° box, worked out by hand: width shrinks with the cosine of the latitude.
    let by_hand = 0.2 * 111.32 * center.0.to_radians().cos() * 0.2 * 110.9;
    assert!((prefecture_area / by_hand - 1.0).abs() < 0.01, "{prefecture_area} km² vs {by_hand} km²");
    assert!((country_area / prefecture_area - 4.0).abs() < 0.05, "the country box is twice as wide and high");

    ride(&ctx, &aoi, center, 3000.0, minutes_ago(12)).await;
    let uncovered = cells(&ctx, &aoi, None, around(center, 0.1), 15.0).await.unwrap().cells;
    let explored_km2: f64 = uncovered.iter().map(|hex| h3o::CellIndex::from_str(hex).unwrap().area_km2()).sum();
    assert!(
        uncovered.len() >= 8 && (0.8..1.6).contains(&(explored_km2 / uncovered.len() as f64 * 10.0)),
        "resolution-9 cells are about 0.1 km²"
    );

    let footprint = stats(&ctx, &aoi, None).await;
    assert_eq!(footprint.total_cell_count, uncovered.len() as i64);
    assert_eq!(footprint.country_count, 1);
    assert_eq!(footprint.regions.len(), 2);
    // Sorted by share: the prefecture (a quarter of the area, the same cells) comes first.
    let (first, second) = (&footprint.regions[0], &footprint.regions[1]);
    let region = |stat: &RegionStat| stat.region.as_option().unwrap().clone();
    assert_eq!(
        (region(first).id, region(first).kind.as_known()),
        (prefecture.clone(), Some(RegionKind::REGION_KIND_ADMIN1))
    );
    assert_eq!(
        (region(second).id, region(second).kind.as_known()),
        (country.clone(), Some(RegionKind::REGION_KIND_COUNTRY))
    );
    assert_eq!(
        (region(first).name.as_str(), region(first).local_name.as_str(), region(first).country_code.as_str()),
        ("Test Prefecture", "テスト県", country.as_str())
    );
    for (stat, area) in [(first, prefecture_area), (second, country_area)] {
        assert_eq!(stat.cell_count, uncovered.len() as i64);
        assert!((stat.explored_km2 - explored_km2).abs() < 1e-6, "{} vs {explored_km2}", stat.explored_km2);
        assert!((stat.percent - explored_km2 / area * 100.0).abs() < 1e-9, "{} %", stat.percent);
        assert!((region(stat).area_km2 - area).abs() < 1e-6);
        assert_eq!(stat.friend_rank, 1, "nobody else has been here");
    }
    assert!((first.percent / second.percent - country_area / prefecture_area).abs() < 1e-6);
    let world = footprint.world.as_option().unwrap();
    assert_eq!(
        (world.cell_count, world.region.as_option().unwrap().kind.as_known()),
        (uncovered.len() as i64, Some(RegionKind::REGION_KIND_WORLD))
    );
    assert!((world.explored_km2 - explored_km2).abs() < 1e-6 && world.percent > 0.0 && world.percent < first.percent);

    // Filters: by kind, and by the regions containing a point.
    let only = |kind: RegionKind, at: Option<Point>| {
        let request = GetFootprintStatsRequest { kind: kind.into(), at: at.map(lat_lng).into(), ..Default::default() };
        let (ctx, aoi) = (&ctx, aoi.clone());
        async move { ctx.footprints::<_, GetFootprintStatsResponse>("GetStats", &aoi, &request).await }
    };
    let ids = |response: GetFootprintStatsResponse| {
        response.regions.iter().map(|stat| stat.region.as_option().unwrap().id.clone()).collect::<Vec<_>>()
    };
    assert_eq!(ids(only(RegionKind::REGION_KIND_COUNTRY, None).await.unwrap()), vec![country.clone()]);
    assert_eq!(ids(only(RegionKind::REGION_KIND_ADMIN1, None).await.unwrap()), vec![prefecture.clone()]);
    assert!(ids(only(RegionKind::REGION_KIND_CITY, None).await.unwrap()).is_empty(), "city boundaries are not bundled");
    // A point in the country but outside the prefecture.
    let outskirts = (center.0 + 0.15, center.1);
    assert_eq!(ids(only(RegionKind::REGION_KIND_UNSPECIFIED, Some(outskirts)).await.unwrap()), vec![country.clone()]);
    assert!(ids(only(RegionKind::REGION_KIND_UNSPECIFIED, Some((center.0 + 3.0, center.1))).await.unwrap()).is_empty());
    assert_eq!(
        only(RegionKind::REGION_KIND_UNSPECIFIED, Some((0.0, 999.0))).await.unwrap_err().reason,
        "INVALID_LOCATION"
    );
}

#[tokio::test]
async fn real_boundaries_attribute_cells_and_new_prefectures_are_news() {
    let ctx = start().await;
    ctx.geo.seed_regions().await.unwrap();
    ctx.geo.seed_regions().await.unwrap(); // idempotent
    let aoi = ctx.app.user("aoi").await;
    let regions: i64 = sqlx::query_scalar("SELECT count(*) FROM geo_regions WHERE kind IN (2, 3) AND id !~ '^T'")
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(regions > 4000, "{regions} regions seeded");

    // Setagaya, Tokyo.
    let setagaya = (35.6466, 139.6532);
    ride(&ctx, &aoi, setagaya, 500.0, minutes_ago(40)).await;
    let footprint = stats(&ctx, &aoi, None).await;
    let names: Vec<(String, String)> = footprint
        .regions
        .iter()
        .map(|stat| stat.region.as_option().unwrap())
        .map(|region| (region.id.clone(), region.local_name.clone()))
        .collect();
    assert!(
        names.contains(&("JP".to_owned(), "日本".to_owned()))
            && names.contains(&("JP-13".to_owned(), "東京都".to_owned())),
        "{names:?}"
    );
    let tokyo = footprint.regions.iter().find(|stat| stat.region.as_option().unwrap().id == "JP-13").unwrap();
    let tokyo_area = tokyo.region.as_option().unwrap().area_km2;
    assert!(
        (1700.0..2500.0).contains(&tokyo_area),
        "Tokyo is about 2,194 km²; the simplified boundary gives {tokyo_area}"
    );
    assert!((tokyo.percent - tokyo.explored_km2 / tokyo_area * 100.0).abs() < 1e-9);
    assert_eq!(footprint.country_count, 1);
    // The first country and prefecture of an account are where it starts, not an achievement.
    assert!(ctx.notifications_for(aoi.id, "geo.new_region").await.is_empty());
    assert!(ctx.activities_of(aoi.id, "geo.new_region").await.is_empty());

    // Yokohama, Kanagawa: a new prefecture.
    let yokohama = (35.4660, 139.6223);
    let mut aois = ctx.app.listen_user(aoi.id).await;
    ride(&ctx, &aoi, yokohama, 300.0, minutes_ago(10)).await;
    let news = ctx.notifications_for(aoi.id, "geo.new_region").await;
    assert_eq!(news.len(), 1);
    assert_eq!(news[0].data["title"]["ja"], "神奈川県に初上陸!🎉");
    assert_eq!(news[0].data["title"]["en"], "First time in Kanagawa Prefecture! 🎉");
    let activity = ctx.activities_of(aoi.id, "geo.new_region").await;
    assert_eq!(activity.len(), 1);
    assert_eq!(
        (activity[0].data["audience"].as_str(), activity[0].data["data"]["region_id"].as_str()),
        (Some("friends"), Some("JP-14"))
    );
    // What the feed's recaps read.
    assert_eq!(
        (activity[0].data["data"]["region_code"].as_str(), activity[0].data["data"]["region_name"].as_str()),
        (Some("JP-14"), Some("神奈川県"))
    );
    let progress = footprint_progress(pushes(&mut aois, QUIET).await);
    let entered: Vec<String> =
        progress.iter().filter_map(|p| p.new_region.as_option().map(|region| region.local_name.clone())).collect();
    assert_eq!(entered, vec!["神奈川県".to_owned()]);
    let footprint = stats(&ctx, &aoi, None).await;
    assert_eq!((footprint.regions.len(), footprint.country_count), (3, 1));

    // People who show their scratch map to nobody keep such news to themselves.
    let shy = ctx.app.user("shy").await;
    ctx.set_privacy(&shy, |privacy| privacy.footprints_visible_to = Audience::AUDIENCE_NOBODY.into()).await;
    ride(&ctx, &shy, setagaya, 100.0, minutes_ago(40)).await;
    ride(&ctx, &shy, yokohama, 100.0, minutes_ago(10)).await;
    assert_eq!(ctx.activities_of(shy.id, "geo.new_region").await[0].data["audience"], "private");
}

#[tokio::test]
async fn leaderboards_rank_friends_and_respect_who_shows_their_map() {
    let ctx = start().await;
    let center = open_ocean();
    let ((country, country_area), _) = make_island(&ctx, center).await;
    let aoi = ctx.app.user("aoi").await;
    let (ren, mio, kai, star) =
        (ctx.app.user("ren").await, ctx.app.user("mio").await, ctx.app.user("kai").await, ctx.app.user("star").await);
    for friend in [&ren, &mio, &kai] {
        ctx.befriend(&aoi, friend).await;
    }
    ctx.set_privacy(&kai, |privacy| privacy.footprints_visible_to = Audience::AUDIENCE_NOBODY.into()).await;
    ctx.set_privacy(&star, |privacy| privacy.footprints_visible_to = Audience::AUDIENCE_EVERYONE.into()).await;
    // Everyone rides a different distance along their own parallel.
    for (index, (user, meters)) in
        [(&aoi, 1500.0), (&ren, 3000.0), (&mio, 600.0), (&kai, 5000.0), (&star, 4000.0)].into_iter().enumerate()
    {
        ride(&ctx, user, (center.0 - 0.05 + index as f64 * 0.02, center.1 - 0.05), meters, minutes_ago(30)).await;
    }

    let board = |region_id: &str, scope: LeaderboardScope, limit: i32| {
        let request =
            GetLeaderboardRequest { region_id: region_id.to_owned(), scope: scope.into(), limit, ..Default::default() };
        let (ctx, aoi) = (&ctx, aoi.clone());
        async move { ctx.footprints::<_, GetLeaderboardResponse>("GetLeaderboard", &aoi, &request).await }
    };
    let ranking = |response: &GetLeaderboardResponse| {
        response
            .entries
            .iter()
            .map(|entry| (entry.rank, entry.user.as_option().unwrap().id.clone()))
            .collect::<Vec<_>>()
    };

    // Among friends: Kai shows their map to nobody and stays out; Star is not a friend.
    let friends = board(&country, LeaderboardScope::LEADERBOARD_SCOPE_FRIENDS, 0).await.unwrap();
    assert_eq!(ranking(&friends), vec![(1, ren.id.to_string()), (2, aoi.id.to_string()), (3, mio.id.to_string())]);
    assert_eq!(friends.region.as_option().unwrap().id, country);
    let me = friends.me.as_option().unwrap();
    assert_eq!((me.rank, me.user.as_option().unwrap().id.clone()), (2, aoi.id.to_string()));
    assert!(
        (me.percent - me.cell_count as f64 * 0.105 / country_area * 100.0).abs() / me.percent < 0.25,
        "share of the country"
    );
    assert!(
        friends.entries[0].cell_count > friends.entries[1].cell_count
            && friends.entries[0].percent > friends.entries[1].percent
    );
    // The unspecified scope means friends; a limit cuts the list but not the caller's own entry.
    let top: GetLeaderboardResponse =
        board(&country, LeaderboardScope::LEADERBOARD_SCOPE_UNSPECIFIED, 1).await.unwrap();
    assert_eq!(ranking(&top), vec![(1, ren.id.to_string())]);
    assert_eq!(top.me.as_option().unwrap().rank, 2);

    // Globally: only people who show their map to everyone, plus the caller.
    let global = board(&country, LeaderboardScope::LEADERBOARD_SCOPE_GLOBAL, 0).await.unwrap();
    assert_eq!(ranking(&global), vec![(1, star.id.to_string()), (2, aoi.id.to_string())]);

    // The world, among friends: the same order by total cells.
    let world = board("", LeaderboardScope::LEADERBOARD_SCOPE_FRIENDS, 0).await.unwrap();
    assert_eq!(ranking(&world), ranking(&friends));
    assert_eq!(world.region.as_option().unwrap().kind, RegionKind::REGION_KIND_WORLD);
    assert_eq!(board("world", LeaderboardScope::LEADERBOARD_SCOPE_FRIENDS, 0).await.unwrap().entries.len(), 3);
    assert_eq!(
        board("NOPE", LeaderboardScope::LEADERBOARD_SCOPE_FRIENDS, 0).await.unwrap_err().reason,
        "REGION_NOT_FOUND"
    );

    // The rank among friends also shows in the caller's own statistics.
    let mine = stats(&ctx, &aoi, None).await;
    assert!(
        mine.regions.iter().all(|stat| stat.friend_rank == 2),
        "{:?}",
        mine.regions.iter().map(|s| s.friend_rank).collect::<Vec<_>>()
    );
    assert_eq!(mine.world.as_option().unwrap().friend_rank, 2);
    // Someone who has not been there is not ranked.
    let newcomer = ctx.app.user("newcomer").await;
    let empty: GetLeaderboardResponse = ctx
        .footprints(
            "GetLeaderboard",
            &newcomer,
            &GetLeaderboardRequest { region_id: country.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(empty.entries.is_empty() && empty.me.as_option().is_none());
}

#[tokio::test]
async fn other_peoples_cells_need_permission_but_their_numbers_do_not() {
    let ctx = start().await;
    let (aoi, ren, kai, star, stranger) = (
        ctx.app.user("aoi").await,
        ctx.app.user("ren").await,
        ctx.app.user("kai").await,
        ctx.app.user("star").await,
        ctx.app.user("stranger").await,
    );
    ctx.befriend(&aoi, &ren).await;
    ctx.befriend(&aoi, &kai).await;
    ctx.set_privacy(&kai, |privacy| privacy.footprints_visible_to = Audience::AUDIENCE_NOBODY.into()).await;
    ctx.set_privacy(&star, |privacy| privacy.footprints_visible_to = Audience::AUDIENCE_EVERYONE.into()).await;
    let from = somewhere();
    for user in [&ren, &kai, &star] {
        ride(&ctx, user, from, 500.0, minutes_ago(10)).await;
    }
    let view = around(from, 0.1);

    // Friends see each other's scratch map by default; strangers do not.
    assert!(!cells(&ctx, &aoi, Some(&ren), view.clone(), 15.0).await.unwrap().cells.is_empty());
    let denied = cells(&ctx, &stranger, Some(&ren), view.clone(), 15.0).await.unwrap_err();
    assert_eq!((denied.code.as_str(), denied.reason.as_str()), ("permission_denied", "FOOTPRINTS_PRIVATE"));
    // "Nobody" means not even friends; "everyone" means strangers too.
    assert_eq!(cells(&ctx, &aoi, Some(&kai), view.clone(), 15.0).await.unwrap_err().reason, "FOOTPRINTS_PRIVATE");
    assert!(!cells(&ctx, &stranger, Some(&star), view.clone(), 15.0).await.unwrap().cells.is_empty());
    assert_eq!(
        cells(&ctx, &aoi, Some(&aoi), view.clone(), 15.0).await.unwrap().cells.len(),
        0,
        "naming oneself is the same as leaving the field empty"
    );

    // The numbers are for anyone who can see the profile.
    assert!(stats(&ctx, &stranger, Some(&kai)).await.total_cell_count > 0);
    assert!(
        stats(&ctx, &stranger, Some(&kai)).await.world.as_option().unwrap().friend_rank == 0,
        "ranks among friends are the caller's own business"
    );

    // Unknown accounts, and accounts in a block with the caller, do not exist.
    let ghost = TestUser { id: uuid::Uuid::now_v7(), ..stranger.clone() };
    assert_eq!(cells(&ctx, &aoi, Some(&ghost), view.clone(), 15.0).await.unwrap_err().reason, "USER_NOT_FOUND");
    let _: tokuly_proto::social::BlockUserResponse = ctx
        .app
        .call(
            "tokuly.social.v1.FriendService/BlockUser",
            Some(&star),
            &tokuly_proto::social::BlockUserRequest { user_id: stranger.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    let blocked = cells(&ctx, &stranger, Some(&star), view, 15.0).await.unwrap_err();
    assert_eq!((blocked.code.as_str(), blocked.reason.as_str()), ("not_found", "USER_NOT_FOUND"));
    let request = GetFootprintStatsRequest { user_id: star.id.to_string(), ..Default::default() };
    assert_eq!(
        ctx.footprints::<_, GetFootprintStatsResponse>("GetStats", &stranger, &request).await.unwrap_err().reason,
        "USER_NOT_FOUND"
    );
}

#[tokio::test]
async fn the_recap_reports_what_is_new_since_it_was_last_seen() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    let center = open_ocean();
    let ((country, _), (prefecture, _)) = make_island(&ctx, center).await;
    let recap = |mark_seen: bool| {
        let (ctx, aoi) = (&ctx, aoi.clone());
        async move {
            ctx.footprints::<_, GetRecapResponse>(
                "GetRecap",
                &aoi,
                &GetRecapRequest { mark_seen, ..Default::default() },
            )
            .await
            .unwrap()
        }
    };
    // Nothing yet.
    let empty = recap(false).await;
    assert_eq!((empty.new_cell_count, empty.new_cells.len(), empty.distance_meters), (0, 0, 0.0));
    assert!(empty.bounds.as_option().is_none() && empty.since.as_option().is_some());

    ride(&ctx, &aoi, center, 1000.0, minutes_ago(20)).await;
    let first = recap(false).await;
    assert!(first.new_cell_count >= 3 && first.new_cells.len() as i64 == first.new_cell_count);
    assert!((first.distance_meters - 1000.0).abs() < 60.0, "{}", first.distance_meters);
    let mut new_regions: Vec<String> = first.new_regions.iter().map(|region| region.id.clone()).collect();
    new_regions.sort();
    assert_eq!(new_regions, vec![country.clone(), prefecture.clone()]);
    assert_eq!(first.regions.len(), 2);
    assert!(first.regions.iter().all(|stat| stat.cell_count == first.new_cell_count));
    let bounds = first.bounds.as_option().unwrap();
    let (sw, ne) = (point(bounds.south_west.as_option()), point(bounds.north_east.as_option()));
    assert!(sw.1 <= center.1 + 0.005 && ne.1 > center.1 && (ne.0 - sw.0).abs() < 0.01);

    // Looking does not consume it; marking it seen does.
    assert_eq!(recap(true).await.new_cell_count, first.new_cell_count);
    let seen = recap(false).await;
    assert_eq!((seen.new_cell_count, seen.new_regions.len(), seen.regions.len()), (0, 0, 0));

    // New ground after that.
    ctx.at(&aoi, offset(center, 0.0, 3000.0)).await;
    let second = recap(false).await;
    assert_eq!(second.new_cell_count, 1);
    assert!(second.new_regions.is_empty(), "the same regions as before");
    assert_eq!(second.regions.len(), 2);
    assert!(second.regions.iter().all(|stat| stat.cell_count == first.new_cell_count + 1), "totals as they stand now");
}
