//! What the geo domain plugs into the others and what it does on their
//! events: profiles, bumps, the data export, account deletion, the sweeper.
mod common;

use chrono::Utc;
use common::*;
use tokuly_core::{geoindex, time::ts};
use tokuly_media::{assets, testing::ready_image};
use tokuly_proto::{
    common::Visibility,
    geo::{check_in_request, create_place_alert_request, destination, *},
    identity::{GetProfileRequest, GetProfileResponse, Profile, get_profile_request},
    media::AssetPurpose,
    social::{BumpRequest, BumpResponse, ListBumpsRequest, ListBumpsResponse},
};
use tokuly_testkit::TestUser;
use uuid::Uuid;

async fn profile(ctx: &Ctx, viewer: Option<&TestUser>, owner: &TestUser) -> Profile {
    let request =
        GetProfileRequest { by: Some(get_profile_request::By::UserId(owner.id.to_string())), ..Default::default() };
    let response: GetProfileResponse =
        ctx.app.call("tokuly.identity.v1.ProfileService/GetProfile", viewer, &request).await.expect("profile");
    response.profile.into_option().expect("profile")
}

#[tokio::test]
async fn profiles_show_the_distance_a_friend_is_allowed_to_know() {
    let ctx = start().await;
    let (aoi, ren, stranger) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("stranger").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    let rens = offset(home, 90.0, 3000.0);
    ctx.at(&aoi, home).await;
    assert_eq!(profile(&ctx, Some(&ren), &aoi).await.distance_meters, None, "the viewer shares no location yet");
    ctx.at(&ren, rens).await;
    ctx.at(&stranger, offset(home, 0.0, 500.0)).await;

    // Precise: the true distance, for friends only.
    let seen = profile(&ctx, Some(&ren), &aoi).await;
    assert!((seen.distance_meters.expect("a distance") - 3000.0).abs() < 5.0);
    assert_eq!(profile(&ctx, Some(&stranger), &aoi).await.distance_meters, None);
    assert_eq!(profile(&ctx, None, &aoi).await.distance_meters, None);
    assert_eq!(profile(&ctx, Some(&aoi), &aoi).await.distance_meters, None);

    // Blurred: the distance to the stand-in, exactly what the map shows — no closer look through the profile.
    ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_BLURRED, 0).await.unwrap();
    let stand_in = pin_point(&ctx.seen_by(&ren, &aoi).await.unwrap());
    let blurred = profile(&ctx, Some(&ren), &aoi).await.distance_meters.expect("a distance to the blurred point");
    assert!((blurred - distance(rens, stand_in)).abs() < 1.0);
    assert!((blurred - 3000.0).abs() > 1.0);

    // Frozen: the distance to the pin the viewer was left with, as for any
    // phone that stopped reporting. Wherever the owner goes, and whether or
    // not they pause sharing, it does not change — and it does not disappear.
    ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_PRECISE, 0).await.unwrap();
    ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_FROZEN, 0).await.unwrap();
    let at_freeze = profile(&ctx, Some(&ren), &aoi).await.distance_meters;
    assert!((at_freeze.expect("a distance to the frozen pin") - 3000.0).abs() < 5.0);
    ctx.at(&aoi, offset(home, 270.0, 9000.0)).await;
    assert_eq!(profile(&ctx, Some(&ren), &aoi).await.distance_meters, at_freeze);
    let _: StopSharingResponse = ctx.location("StopSharing", &aoi, &StopSharingRequest::default()).await.unwrap();
    assert_eq!(profile(&ctx, Some(&ren), &aoi).await.distance_meters, at_freeze);
    ctx.at(&aoi, home).await;
    ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_PRECISE, 0).await.unwrap();
    assert!(profile(&ctx, Some(&ren), &aoi).await.distance_meters.is_some());

    // Either side pausing sharing: none.
    let _: StopSharingResponse = ctx.location("StopSharing", &ren, &StopSharingRequest::default()).await.unwrap();
    assert_eq!(profile(&ctx, Some(&ren), &aoi).await.distance_meters, None);
    ctx.at(&ren, rens).await;
    let _: StopSharingResponse = ctx.location("StopSharing", &aoi, &StopSharingRequest::default()).await.unwrap();
    assert_eq!(profile(&ctx, Some(&ren), &aoi).await.distance_meters, None);
}

#[tokio::test]
async fn profiles_carry_footprint_numbers() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    let fresh = profile(&ctx, Some(&ren), &aoi).await;
    let stats = fresh.stats.as_option().unwrap();
    assert_eq!((stats.explored_percent, stats.visited_country_count), (0.0, 0));

    // A made-up country in the open ocean.
    let center = (-40.0 - rand::random::<u16>() as f64 / 20000.0, -120.0 - rand::random::<u16>() as f64 / 20000.0);
    let country = format!("H{}", tokuly_core::ids::random_base62(6).to_ascii_uppercase());
    sqlx::query(
        "INSERT INTO geo_regions (id, kind, name, country_code, area_km2, center_lat, center_lng, geom)
         SELECT $1, 2, 'Hookland', $1, ST_Area(g::geography) / 1000000.0, $3, $2, g
         FROM (SELECT ST_Multi(ST_MakeEnvelope($2 - 0.1, $3 - 0.1, $2 + 0.1, $3 + 0.1, 4326)) AS g) s",
    )
    .bind(&country)
    .bind(center.1)
    .bind(center.0)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
    ctx.geo.reload_regions();
    let samples: Vec<LocationSample> = (0..=20)
        .map(|i| sample(offset(center, 90.0, i as f64 * 50.0), minutes_ago(10) + chrono::Duration::seconds(i * 10)))
        .collect();
    ctx.report(&aoi, samples).await;

    let footprint: GetFootprintStatsResponse =
        ctx.footprints("GetStats", &aoi, &GetFootprintStatsRequest::default()).await.unwrap();
    let explored = footprint
        .regions
        .iter()
        .find(|stat| stat.region.as_option().unwrap().id == country)
        .expect("the country")
        .percent;
    assert!(explored > 0.0);
    // Shown to the audience of the scratch map: friends by default.
    let numbers = async |viewer: Option<&TestUser>| {
        let seen = profile(&ctx, viewer, &aoi).await;
        let stats = seen.stats.as_option().unwrap();
        (stats.explored_percent, stats.visited_country_count)
    };
    assert_eq!(numbers(Some(&ren)).await, (0.0, 0), "not a friend yet");
    assert_eq!(numbers(None).await, (0.0, 0));
    ctx.befriend(&aoi, &ren).await;
    let (percent, countries) = numbers(Some(&ren)).await;
    assert_eq!(countries, 1);
    assert!((percent - explored).abs() < 1e-9, "{percent} vs {explored}");
    assert_eq!(numbers(Some(&aoi)).await, (percent, 1));
    ctx.set_privacy(&aoi, |privacy| {
        privacy.footprints_visible_to = tokuly_proto::identity::Audience::AUDIENCE_EVERYONE.into()
    })
    .await;
    assert_eq!(numbers(None).await, (percent, 1));
    ctx.set_privacy(&aoi, |privacy| {
        privacy.footprints_visible_to = tokuly_proto::identity::Audience::AUDIENCE_NOBODY.into()
    })
    .await;
    assert_eq!(numbers(Some(&ren)).await, (0.0, 0));
}

#[tokio::test]
async fn a_bump_is_named_after_where_it_happened() {
    let ctx = start().await;
    let (aoi, ren, owner) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("owner").await);
    ctx.befriend(&aoi, &ren).await;
    let spot = somewhere();
    let request = CreatePlaceRequest {
        name: "ゲームセンター".into(),
        center: lat_lng(spot).into(),
        visibility: Visibility::VISIBILITY_PUBLIC.into(),
        ..Default::default()
    };
    let _: CreatePlaceResponse = ctx.places("CreatePlace", &owner, &request).await.unwrap();

    // The two shake their phones (the social domain records the bump)...
    let shake =
        BumpRequest { location: lat_lng(spot).into(), accuracy_meters: 5.0, intensity: 2.0, ..Default::default() };
    let _: BumpResponse = ctx.app.call("tokuly.social.v1.InteractionService/Bump", Some(&aoi), &shake).await.unwrap();
    let bumped: BumpResponse =
        ctx.app.call("tokuly.social.v1.InteractionService/Bump", Some(&ren), &shake).await.unwrap();
    let bump = bumped.bumps.first().expect("a bump");
    assert!(bump.place_name.is_empty());
    // ...and the geo domain answers the event with the name of the place.
    let event = ctx
        .app
        .events_by("social.bump.created", ren.id)
        .await
        .pop()
        .or(ctx.app.events_by("social.bump.created", aoi.id).await.pop())
        .expect("the bump event");
    ctx.geo.handle_event(&event).await.unwrap();
    ctx.geo.handle_event(&event).await.unwrap(); // delivered twice: no harm
    let listed: ListBumpsResponse = ctx
        .app
        .call("tokuly.social.v1.InteractionService/ListBumps", Some(&aoi), &ListBumpsRequest::default())
        .await
        .unwrap();
    assert_eq!(listed.bumps[0].place_name, "ゲームセンター");
    // It counts in both users' statistics, once.
    for user in [&aoi, &ren] {
        let stats: GetStatsResponse = ctx.history("GetStats", user, &GetStatsRequest::default()).await.unwrap();
        assert_eq!(stats.bump_count, 1);
    }
    let later: GetStatsResponse = ctx
        .history(
            "GetStats",
            &aoi,
            &GetStatsRequest { from: ts(Utc::now() + chrono::Duration::hours(1)), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(later.bump_count, 0);
}

#[tokio::test]
async fn the_data_export_has_a_geo_section() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    let home = somewhere();
    let request = CreatePlaceRequest {
        name: "うち".into(),
        label: PlaceLabel::PLACE_LABEL_HOME.into(),
        center: lat_lng(home).into(),
        ..Default::default()
    };
    let _: CreatePlaceResponse = ctx.places("CreatePlace", &aoi, &request).await.unwrap();
    ctx.stay(&aoi, home, minutes_ago(9), seconds_ago(10)).await;
    ctx.ghost(&aoi, &[], GhostMode::GHOST_MODE_BLURRED, 0).await.unwrap();

    let exporter = ctx
        .app
        .core
        .exporters
        .all()
        .into_iter()
        .find(|exporter| exporter.name() == "geo")
        .expect("the geo exporter is registered");
    let export = exporter.export(aoi.id).await.unwrap();
    assert_eq!(export["places"][0]["name"], "うち");
    assert_eq!(export["visits"].as_array().unwrap().len(), 1);
    assert_eq!(export["visits"][0]["place_name"], "うち");
    assert_eq!(export["ghost_rules"][0]["mode"], GhostMode::GHOST_MODE_BLURRED as i32);
    assert!(export["ghost_rules"][0]["viewer_id"].is_null(), "the default for everyone");
    assert!(export["last_position"]["lat"].is_number());
    assert!(export["footprints"]["cell_count"].as_i64().unwrap() >= 1);
    assert_eq!(
        export["footprints"]["cells"].as_array().unwrap().len() as i64,
        export["footprints"]["cell_count"].as_i64().unwrap()
    );
    assert!(export["raw_point_count"].as_u64().unwrap() >= 19, "{}", export["raw_point_count"]);
    assert_eq!(export["raw_points"].as_array().unwrap().len() as u64, export["raw_point_count"].as_u64().unwrap());
    for section in [
        "saved_places",
        "nights",
        "day_memos",
        "movement_by_hour",
        "trips",
        "place_alerts_set",
        "place_alerts_about_me",
        "sos",
        "live_shares",
        "gatherings",
    ] {
        assert!(export[section].is_array(), "{section}");
    }
    // Someone with nothing exports empty sections, not errors.
    let nobody = exporter.export(Uuid::now_v7()).await.unwrap();
    assert!(nobody["last_position"].is_null() && nobody["places"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn deleting_an_account_removes_everything_the_geo_domain_holds() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let core = &ctx.app.core;
    let home = somewhere();

    // A bit of everything.
    let photo = ready_image(core, aoi.id, AssetPurpose::ASSET_PURPOSE_PLACE_PHOTO).await.unwrap();
    let request = CreatePlaceRequest {
        name: "カフェ".into(),
        center: lat_lng(home).into(),
        visibility: Visibility::VISIBILITY_PUBLIC.into(),
        photo_asset_id: photo.id.to_string(),
        ..Default::default()
    };
    let place: CreatePlaceResponse = ctx.places("CreatePlace", &aoi, &request).await.unwrap();
    let place = place.place.into_option().unwrap();
    let _: SavePlaceResponse = ctx
        .places("SavePlace", &ren, &SavePlaceRequest { place_id: place.id.clone(), ..Default::default() })
        .await
        .unwrap();
    ctx.stay(&aoi, home, minutes_ago(9), seconds_ago(10)).await;
    ctx.at(&ren, offset(home, 0.0, 3000.0)).await;
    let memo_photo = ready_image(core, aoi.id, AssetPurpose::ASSET_PURPOSE_DAY_MEMO).await.unwrap();
    let memo = SetDayMemoRequest {
        date: Utc::now().date_naive().to_string(),
        text: "メモ".into(),
        photo_asset_ids: vec![memo_photo.id.to_string()],
        ..Default::default()
    };
    let _: SetDayMemoResponse = ctx.history("SetDayMemo", &aoi, &memo).await.unwrap();
    ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_BLURRED, 0).await.unwrap();
    ctx.ghost(&ren, &[&aoi], GhostMode::GHOST_MODE_BLURRED, 0).await.unwrap();
    let alert = |subject: &TestUser| CreatePlaceAlertRequest {
        subject_user_id: subject.id.to_string(),
        r#where: Some(create_place_alert_request::Where::Center(Box::new(lat_lng(home)))),
        trigger: AlertTrigger::ALERT_TRIGGER_ARRIVE.into(),
        ..Default::default()
    };
    let _: CreatePlaceAlertResponse = ctx.safety("CreatePlaceAlert", &aoi, &alert(&ren)).await.unwrap();
    let _: CreatePlaceAlertResponse = ctx.safety("CreatePlaceAlert", &ren, &alert(&aoi)).await.unwrap();
    let _: TriggerSosResponse = ctx.safety("TriggerSos", &ren, &TriggerSosRequest::default()).await.unwrap();
    let share: CreateLiveShareResponse = ctx
        .location("CreateLiveShare", &aoi, &CreateLiveShareRequest { duration_seconds: 600, ..Default::default() })
        .await
        .unwrap();
    let token = share.share.as_option().unwrap().url.rsplit('/').next().unwrap().to_owned();
    let to_ren = Destination { target: Some(destination::Target::UserId(ren.id.to_string())), ..Default::default() };
    let _: StartTripResponse = ctx
        .trips("StartTrip", &aoi, &StartTripRequest { destination: to_ren.into(), ..Default::default() })
        .await
        .unwrap();
    let to_aoi =
        Destination { target: Some(destination::Target::Position(Box::new(lat_lng(home)))), ..Default::default() };
    let rens_trip: StartTripResponse = ctx
        .trips(
            "StartTrip",
            &ren,
            &StartTripRequest {
                destination: to_aoi.into(),
                share_with_user_ids: vec![aoi.id.to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let _: CheckInResponse = ctx
        .places(
            "CheckIn",
            &ren,
            &CheckInRequest {
                r#where: Some(check_in_request::Where::PlaceId(place.id.clone())),
                with_user_ids: vec![aoi.id.to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // The account is purged (identity anonymizes the row and announces it).
    ctx.deliver("identity.user.deleted", aoi.id, serde_json::json!({ "user_id": aoi.id })).await;
    ctx.deliver("identity.user.deleted", aoi.id, serde_json::json!({ "user_id": aoi.id })).await; // delivered twice: no harm

    for (table, columns) in [
        ("geo_users", "user_id = $1"),
        ("geo_ghost_rules", "owner_id = $1 OR viewer_id = $1"),
        ("geo_places", "owner_id = $1"),
        ("geo_saved_places", "user_id = $1"),
        ("geo_visits", "user_id = $1"),
        ("geo_nights", "user_id = $1"),
        ("geo_activity_hours", "user_id = $1"),
        ("geo_day_memos", "user_id = $1"),
        ("geo_cells", "user_id = $1"),
        ("geo_region_stats", "user_id = $1"),
        ("geo_trips", "user_id = $1 OR dest_user_id = $1"),
        ("geo_place_alerts", "owner_id = $1 OR subject_id = $1"),
        ("geo_sos", "user_id = $1"),
        ("geo_live_shares", "user_id = $1"),
        ("geo_gathering_members", "user_id = $1"),
        ("geo_bumps", "user_a = $1 OR user_b = $1"),
    ] {
        let left: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table} WHERE {columns}")))
                .bind(aoi.id)
                .fetch_one(&core.db)
                .await
                .unwrap();
        assert_eq!(left, 0, "{table}");
    }
    // Raw history, live state and the shared position index.
    let route: GetRouteResponse = ctx
        .history(
            "GetRoute",
            &aoi,
            &GetRouteRequest { from: ts(minutes_ago(60)), to: ts(Utc::now()), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(route.points.is_empty());
    assert!(ctx.my_location(&aoi).await.location.as_option().is_none());
    assert!(geoindex::get(&core.cache, aoi.id).await.unwrap().is_none());
    // Uploads the account's content held are released.
    assert!(
        !assets::is_attached(core, photo.id).await.unwrap() && !assets::is_attached(core, memo_photo.id).await.unwrap()
    );
    // What others had that involved the account.
    let saved: ListSavedPlacesResponse =
        ctx.places("ListSavedPlaces", &ren, &ListSavedPlacesRequest::default()).await.unwrap();
    assert!(saved.places.is_empty());
    let alerts: ListPlaceAlertsResponse =
        ctx.safety("ListPlaceAlerts", &ren, &ListPlaceAlertsRequest::default()).await.unwrap();
    assert!(alerts.mine.is_empty() && alerts.about_me.is_empty());
    let sos: GetActiveSosResponse = ctx.safety("GetActiveSos", &ren, &GetActiveSosRequest::default()).await.unwrap();
    assert!(sos.mine.as_option().unwrap().notified_user_ids.is_empty(), "the deleted friend is no longer on the SOS");
    let link = ctx
        .app
        .call::<_, GetLiveShareResponse>(
            "tokuly.geo.v1.LocationService/GetLiveShare",
            None,
            &GetLiveShareRequest { token, ..Default::default() },
        )
        .await;
    assert_eq!(link.unwrap_err().reason, "SHARE_NOT_FOUND");
    let settings: GetGhostSettingsResponse =
        ctx.location("GetGhostSettings", &ren, &GetGhostSettingsRequest::default()).await.unwrap();
    assert!(settings.overrides.is_empty());
    // Ren's own data is untouched.
    let visits: ListVisitsResponse = ctx.history("ListVisits", &ren, &ListVisitsRequest::default()).await.unwrap();
    assert_eq!(visits.visits.len(), 1);
    let trip: GetTripResponse = ctx
        .trips(
            "GetTrip",
            &ren,
            &GetTripRequest { trip_id: rens_trip.trip.as_option().unwrap().id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(trip.trip.as_option().unwrap().status, TripStatus::TRIP_STATUS_ACTIVE);
}

#[tokio::test]
async fn the_sweeper_and_the_daily_pass_run_cleanly() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    ctx.at(&aoi, somewhere()).await;
    ctx.geo.sweep().await;
    ctx.geo.prune().await.unwrap();
    ctx.geo.enforce_retention().await.unwrap();
    ctx.geo.flush_history().await.unwrap();
    assert!(ctx.my_location(&aoi).await.sharing, "nothing that is still current was touched");
    // Events the geo domain does not know are ignored rather than failing the consumer.
    ctx.deliver("chat.message.created", aoi.id, serde_json::json!({ "anything": true })).await;
}
