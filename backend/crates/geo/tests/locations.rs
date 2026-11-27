//! `LocationService`: uploads, the friend map, ghost mode end to end, and
//! what friendship changes do to it.
mod common;

use chrono::{Duration, Utc};
use common::*;
use tokuly_core::{geoindex, presence, time::from_ts};
use tokuly_proto::{
    geo::{geo_event::Event as Push, watch_friend_locations_response::Update, *},
    identity::{SetStatusRequest, SetStatusResponse},
};

#[tokio::test]
async fn a_friend_sees_my_pin_and_nobody_else_does() {
    let ctx = start().await;
    let (aoi, ren, stranger) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("stranger").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    let mut rens = ctx.app.listen_user(ren.id).await;
    let mut strangers = ctx.app.listen_user(stranger.id).await;

    let recorded_at = seconds_ago(5);
    let reading = LocationSample {
        battery_percent: Some(64),
        charging: Some(true),
        speed_mps: Some(1.3),
        heading_degrees: Some(90.0),
        ..sample(home, recorded_at)
    };
    let response = ctx.report(&aoi, vec![reading]).await;
    assert_eq!(response.accepted_count, 1);
    assert!(
        response.moving_interval_seconds > 0 && response.stationary_interval_seconds > response.moving_interval_seconds
    );

    // Pushed to the friend, live.
    let pushed = next_pin(&mut rens).await.expect("a pin for the friend");
    assert_eq!(pushed.user_id, aoi.id.to_string());
    assert_at!(pin_point(&pushed), home);
    assert_eq!(pushed.precision, GhostMode::GHOST_MODE_PRECISE);
    assert_eq!(
        (pushed.battery_percent, pushed.charging, pushed.speed_mps, pushed.heading_degrees),
        (Some(64), true, 1.3, 90.0)
    );
    assert_eq!(pushed.activity, Activity::ACTIVITY_WALKING, "inferred from the speed");
    assert_eq!(from_ts(pushed.recorded_at.as_option().unwrap()).timestamp_millis(), recorded_at.timestamp_millis());
    assert!(!pushed.time_zone.is_empty());
    assert!(pins(&mut strangers, QUIET).await.is_empty());

    // And on the friend map.
    let listed = ctx.friend_locations(&ren).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0], pushed);
    assert!(ctx.friend_locations(&stranger).await.is_empty());
    assert!(ctx.friend_locations(&aoi).await.is_empty(), "the friend has not shared a location yet");

    let mine = ctx.my_location(&aoi).await;
    assert!(mine.sharing);
    assert_at!(pin_point(mine.location.as_option().unwrap()), home);
    let none = ctx.my_location(&ren).await;
    assert!(!none.sharing && none.location.as_option().is_none());

    // The shared true-position index other domains use follows.
    let indexed = geoindex::get(&ctx.app.core.cache, aoi.id).await.unwrap().expect("indexed");
    assert!(distance((indexed.lat, indexed.lng), home) < 1.0);

    let anonymous = ctx
        .app
        .call::<_, ListFriendLocationsResponse>(
            "tokuly.geo.v1.LocationService/ListFriendLocations",
            None,
            &ListFriendLocationsRequest::default(),
        )
        .await
        .unwrap_err();
    assert_eq!((anonymous.code.as_str(), anonymous.reason.as_str()), ("unauthenticated", "SIGN_IN_REQUIRED"));
    let anonymous = ctx
        .app
        .call::<_, ReportLocationsResponse>(
            "tokuly.geo.v1.LocationService/ReportLocations",
            None,
            &ReportLocationsRequest::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(anonymous.reason, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn duplicates_stragglers_and_unusable_samples_are_skipped() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    let home = somewhere();
    let base = minutes_ago(3);
    let at = |seconds: i64| sample(offset(home, 90.0, seconds as f64), base + Duration::seconds(seconds));

    assert_eq!(ctx.report(&aoi, vec![at(0), at(30), at(60)]).await.accepted_count, 3);
    let upload = vec![
        at(30),                                             // older than the newest known
        at(60),                                             // duplicate
        sample((95.0, 0.0), base + Duration::seconds(70)),  // not a position
        LocationSample { accuracy_meters: -1.0, ..at(75) }, // nonsense accuracy
        sample(home, Utc::now() + Duration::minutes(10)),   // from the future
        sample(home, Utc::now() - Duration::days(30)),      // too old
        at(90),                                             // good
    ];
    assert_eq!(ctx.report(&aoi, upload).await.accepted_count, 1);
    let mine = ctx.my_location(&aoi).await;
    assert!(distance(pin_point(mine.location.as_option().unwrap()), offset(home, 90.0, 90.0)) < 1.0);

    // Out of order within one upload is sorted, not dropped.
    assert_eq!(ctx.report(&aoi, vec![at(150), at(120)]).await.accepted_count, 2);
    assert_eq!(ctx.report(&aoi, Vec::new()).await.accepted_count, 0);

    let too_many: Vec<LocationSample> = (0..501).map(|i| at(200 + i)).collect();
    let error = ctx
        .location::<_, ReportLocationsResponse>(
            "ReportLocations",
            &aoi,
            &ReportLocationsRequest { samples: too_many, ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!((error.code.as_str(), error.reason.as_str()), ("invalid_argument", "TOO_MANY_SAMPLES"));
}

#[tokio::test]
async fn ingest_processes_a_gateway_report_directly() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let here = somewhere();
    let mut rens = ctx.app.listen_user(ren.id).await;
    // What the realtime gateway puts on the LOCATIONS stream.
    let report = presence::LocationReport {
        user_id: aoi.id,
        session_id: None,
        received_at: Utc::now(),
        sample: sample(here, next_fix_time(aoi.id)),
    };
    ctx.geo.ingest(report.clone()).await.unwrap();
    assert_at!(pin_point(&next_pin(&mut rens).await.unwrap()), here);
    // The same report again (redelivery) changes nothing.
    ctx.geo.ingest(report).await.unwrap();
    assert!(pins(&mut rens, QUIET).await.is_empty());
    // An unusable report is not an error for the stream consumer, just nothing.
    let bad = presence::LocationReport {
        user_id: aoi.id,
        session_id: None,
        received_at: Utc::now(),
        sample: LocationSample::default(),
    };
    ctx.geo.ingest(bad).await.unwrap();
    assert_at!(pin_point(ctx.my_location(&aoi).await.location.as_option().unwrap()), here);
}

#[tokio::test]
async fn a_blurred_friend_gets_a_stand_in_position() {
    let ctx = start().await;
    let (aoi, ren, mio) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("mio").await);
    ctx.befriend(&aoi, &ren).await;
    ctx.befriend(&aoi, &mio).await;
    // (The middle of a blur cell: a few steps from here stay inside it.)
    let anywhere = somewhere();
    let home = tokuly_geo::math::cell_center(
        tokuly_geo::math::cell_at(anywhere.0, anywhere.1, tokuly_geo::visibility::BLUR_RESOLUTION).expect("a cell"),
    );
    let reading = LocationSample { battery_percent: Some(40), speed_mps: Some(1.2), ..sample(home, seconds_ago(20)) };
    ctx.report(&aoi, vec![reading]).await;
    let mut rens = ctx.app.listen_user(ren.id).await;

    let settings = ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_BLURRED, 0).await.unwrap();
    assert_eq!(settings.everyone.as_option().unwrap().mode, GhostMode::GHOST_MODE_PRECISE);
    assert_eq!(settings.overrides.len(), 1);
    assert_eq!(settings.overrides[&ren.id.to_string()].mode, GhostMode::GHOST_MODE_BLURRED);

    // The friend's pin moves to the stand-in at once.
    let pushed = next_pin(&mut rens).await.expect("the blurred pin is pushed when the rule is set");
    assert_eq!(pushed.precision, GhostMode::GHOST_MODE_BLURRED);
    let stand_in = pin_point(&pushed);
    let offset_m = distance(stand_in, home);
    assert!(offset_m > 0.0 && offset_m <= 2000.0, "offset {offset_m}");
    assert_eq!((pushed.speed_mps, pushed.battery_percent), (0.0, Some(40)), "movement is withheld, the battery is not");
    assert_at!(pin_point(&ctx.seen_by(&ren, &aoi).await.unwrap()), stand_in);
    // Everyone else is unaffected.
    assert_at!(pin_point(&ctx.seen_by(&mio, &aoi).await.unwrap()), home);

    // Small movements do not move the stand-in, and nothing is pushed for them.
    ctx.at(&aoi, offset(home, 45.0, 12.0)).await;
    assert!(pins(&mut rens, QUIET).await.iter().all(|pin| pin.recorded_at != pushed.recorded_at));
    assert_at!(pin_point(&ctx.seen_by(&ren, &aoi).await.unwrap()), stand_in);
    assert_at!(pin_point(&ctx.seen_by(&mio, &aoi).await.unwrap()), offset(home, 45.0, 12.0));

    // The owner's own view is never blurred.
    assert_at!(pin_point(ctx.my_location(&aoi).await.location.as_option().unwrap()), offset(home, 45.0, 12.0));

    // Back to precise: the override is gone and the true pin is pushed.
    let settings = ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_PRECISE, 0).await.unwrap();
    assert!(settings.overrides.is_empty());
    let precise = next_pin(&mut rens).await.unwrap();
    assert_at!(pin_point(&precise), offset(home, 45.0, 12.0));
    assert_eq!(precise.precision, GhostMode::GHOST_MODE_PRECISE);
}

#[tokio::test]
async fn a_frozen_pin_stays_put_and_nothing_is_pushed() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    let frozen_at = seconds_ago(90);
    ctx.report(&aoi, vec![LocationSample { battery_percent: Some(71), ..sample(home, frozen_at) }]).await;
    let before = ctx.seen_by(&ren, &aoi).await.unwrap();
    let mut rens = ctx.app.listen_user(ren.id).await;

    let settings = ctx.ghost(&aoi, &[], GhostMode::GHOST_MODE_FROZEN, 0).await.unwrap();
    let everyone = settings.everyone.as_option().unwrap();
    assert_eq!(everyone.mode, GhostMode::GHOST_MODE_FROZEN);
    assert!(everyone.since.as_option().is_some() && everyone.until.as_option().is_none());

    // The owner moves across town.
    let away = offset(home, 200.0, 7000.0);
    ctx.report(&aoi, vec![LocationSample { battery_percent: Some(15), ..sample(away, next_fix_time(aoi.id)) }]).await;
    assert!(pushes(&mut rens, QUIET).await.is_empty(), "freezing and moving while frozen push nothing");

    let seen = ctx.seen_by(&ren, &aoi).await.unwrap();
    assert_eq!(seen, before, "exactly the pin from before: position, time, battery and all");
    assert_eq!(from_ts(seen.recorded_at.as_option().unwrap()).timestamp_millis(), frozen_at.timestamp_millis());
    assert_eq!(seen.precision, GhostMode::GHOST_MODE_PRECISE, "FROZEN is never revealed");
    assert_at!(pin_point(ctx.my_location(&aoi).await.location.as_option().unwrap()), away);

    // Freezing again (with a timer) does not refresh the frozen pin.
    ctx.ghost(&aoi, &[], GhostMode::GHOST_MODE_FROZEN, 3600).await.unwrap();
    assert_eq!(ctx.seen_by(&ren, &aoi).await.unwrap(), before);

    // Unfreezing shows the live pin.
    ctx.ghost(&aoi, &[], GhostMode::GHOST_MODE_PRECISE, 0).await.unwrap();
    let live = next_pin(&mut rens).await.expect("the live pin after unfreezing");
    assert_at!(pin_point(&live), away);
    assert_eq!(live.battery_percent, Some(15));
}

#[tokio::test]
async fn freezing_before_any_location_shows_nothing() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    ctx.ghost(&aoi, &[], GhostMode::GHOST_MODE_FROZEN, 0).await.unwrap();
    ctx.at(&aoi, somewhere()).await;
    assert!(ctx.seen_by(&ren, &aoi).await.is_none(), "there was no pin when freezing began, so there is none now");
}

#[tokio::test]
async fn an_exception_keeps_one_friend_precise_under_a_strict_default() {
    let ctx = start().await;
    let (aoi, ren, mio) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("mio").await);
    ctx.befriend(&aoi, &ren).await;
    ctx.befriend(&aoi, &mio).await;
    let home = somewhere();
    ctx.at(&aoi, home).await;

    ctx.ghost(&aoi, &[], GhostMode::GHOST_MODE_BLURRED, 0).await.unwrap();
    let settings = ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_PRECISE, 0).await.unwrap();
    assert_eq!(settings.everyone.as_option().unwrap().mode, GhostMode::GHOST_MODE_BLURRED);
    assert_eq!(settings.overrides[&ren.id.to_string()].mode, GhostMode::GHOST_MODE_PRECISE, "kept as an exception");
    assert_at!(pin_point(&ctx.seen_by(&ren, &aoi).await.unwrap()), home);
    assert_eq!(ctx.seen_by(&mio, &aoi).await.unwrap().precision, GhostMode::GHOST_MODE_BLURRED);

    // Each blurred friend has their own stand-in.
    ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_BLURRED, 0).await.unwrap();
    assert_ne!(pin_point(&ctx.seen_by(&ren, &aoi).await.unwrap()), pin_point(&ctx.seen_by(&mio, &aoi).await.unwrap()));

    // Precise for everyone again: exceptions saying "precise" are dropped, other overrides stay.
    ctx.ghost(&aoi, &[&mio], GhostMode::GHOST_MODE_PRECISE, 0).await.unwrap();
    let settings = ctx.ghost(&aoi, &[], GhostMode::GHOST_MODE_PRECISE, 0).await.unwrap();
    assert_eq!(settings.everyone.as_option().unwrap().mode, GhostMode::GHOST_MODE_PRECISE);
    assert_eq!(settings.overrides.keys().collect::<Vec<_>>(), vec![&ren.id.to_string()]);
    let stored: GetGhostSettingsResponse =
        ctx.location("GetGhostSettings", &aoi, &GetGhostSettingsRequest::default()).await.unwrap();
    assert_eq!(stored, settings);
}

#[tokio::test]
async fn timed_ghost_rules_end_by_themselves() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    ctx.at(&aoi, home).await;
    let settings = ctx.ghost(&aoi, &[], GhostMode::GHOST_MODE_FROZEN, 600).await.unwrap();
    let until = from_ts(settings.everyone.as_option().unwrap().until.as_option().expect("a timed rule has an end"));
    assert!((until - Utc::now() - Duration::seconds(600)).num_seconds().abs() <= 5);

    let moved = offset(home, 0.0, 2000.0);
    ctx.at(&aoi, moved).await;
    assert_at!(pin_point(&ctx.seen_by(&ren, &aoi).await.unwrap()), home);
    assert_eq!(ctx.geo.expire_ghost_rules().await.unwrap(), 0, "not yet");

    // Time passes; the sweeper removes the rule and tells the friend.
    sqlx::query("UPDATE geo_ghost_rules SET until = now() - interval '1 second' WHERE owner_id = $1")
        .bind(aoi.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let mut rens = ctx.app.listen_user(ren.id).await;
    assert!(ctx.geo.expire_ghost_rules().await.unwrap() >= 1);
    assert_at!(pin_point(&next_pin(&mut rens).await.expect("the live pin when the rule ends")), moved);
    let settings: GetGhostSettingsResponse =
        ctx.location("GetGhostSettings", &aoi, &GetGhostSettingsRequest::default()).await.unwrap();
    assert_eq!(settings.everyone.as_option().unwrap().mode, GhostMode::GHOST_MODE_PRECISE);
}

#[tokio::test]
async fn ghost_mode_requests_are_validated() {
    let ctx = start().await;
    let (aoi, ren, stranger) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("stranger").await);
    ctx.befriend(&aoi, &ren).await;
    let reason = |result: Result<GetGhostSettingsResponse, tokuly_testkit::RpcError>| result.unwrap_err().reason;
    assert_eq!(reason(ctx.ghost(&aoi, &[], GhostMode::GHOST_MODE_UNSPECIFIED, 0).await), "INVALID_GHOST_MODE");
    assert_eq!(reason(ctx.ghost(&aoi, &[], GhostMode::GHOST_MODE_BLURRED, -1).await), "INVALID_DURATION");
    assert_eq!(reason(ctx.ghost(&aoi, &[], GhostMode::GHOST_MODE_BLURRED, 31 * 24 * 3600).await), "INVALID_DURATION");
    assert_eq!(reason(ctx.ghost(&aoi, &[&stranger], GhostMode::GHOST_MODE_BLURRED, 0).await), "NOT_FRIENDS");
    assert_eq!(reason(ctx.ghost(&aoi, &[&aoi], GhostMode::GHOST_MODE_BLURRED, 0).await), "NOT_FRIENDS");
    let request = |user_ids: Vec<String>| SetGhostModeRequest {
        user_ids,
        mode: GhostMode::GHOST_MODE_BLURRED.into(),
        ..Default::default()
    };
    let error =
        ctx.location::<_, SetGhostModeResponse>("SetGhostMode", &aoi, &request(vec!["nope".into()])).await.unwrap_err();
    assert_eq!(error.reason, "INVALID_ID");
    let crowd = (0..201).map(|_| uuid::Uuid::now_v7().to_string()).collect();
    let error = ctx.location::<_, SetGhostModeResponse>("SetGhostMode", &aoi, &request(crowd)).await.unwrap_err();
    assert_eq!(error.reason, "TOO_MANY_USERS");
    // Nothing was stored by the failed attempts.
    let settings: GetGhostSettingsResponse =
        ctx.location("GetGhostSettings", &aoi, &GetGhostSettingsRequest::default()).await.unwrap();
    assert!(settings.overrides.is_empty());
}

#[tokio::test]
async fn stop_sharing_pauses_the_pin_where_it_is() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    ctx.at(&aoi, home).await;
    let mut rens = ctx.app.listen_user(ren.id).await;

    let _: StopSharingResponse = ctx.location("StopSharing", &aoi, &StopSharingRequest::default()).await.unwrap();
    let paused = next_pin(&mut rens).await.expect("friends learn at once");
    assert!(paused.sharing_paused);
    assert_at!(pin_point(&paused), home);
    assert!(ctx.seen_by(&ren, &aoi).await.unwrap().sharing_paused);
    let mine = ctx.my_location(&aoi).await;
    assert!(!mine.sharing && mine.location.as_option().is_some());
    assert!(
        geoindex::get(&ctx.app.core.cache, aoi.id).await.unwrap().is_none(),
        "the true-position index forgets a user who stopped sharing"
    );
    // Stopping twice, or without ever having shared, is fine.
    let _: StopSharingResponse = ctx.location("StopSharing", &aoi, &StopSharingRequest::default()).await.unwrap();
    let _: StopSharingResponse = ctx.location("StopSharing", &ren, &StopSharingRequest::default()).await.unwrap();

    // The next sample resumes sharing.
    ctx.at(&aoi, home).await;
    assert!(!next_pin(&mut rens).await.unwrap().sharing_paused);
    assert!(ctx.my_location(&aoi).await.sharing);
}

#[tokio::test]
async fn a_sample_still_on_its_way_when_sharing_stopped_does_not_resume_it() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    ctx.at(&aoi, home).await;
    let mut rens = ctx.app.listen_user(ren.id).await;

    // The device takes a fix, the user stops sharing, and only then does the fix reach the server.
    let in_flight = sample(offset(home, 0.0, 60.0), next_fix_time(aoi.id));
    let stopped_at = next_fix_time(aoi.id);
    let stop = StopSharingRequest { stopped_at: tokuly_core::time::ts(stopped_at), ..Default::default() };
    let _: StopSharingResponse = ctx.location("StopSharing", &aoi, &stop).await.unwrap();
    assert!(next_pin(&mut rens).await.unwrap().sharing_paused);
    assert_eq!(ctx.report(&aoi, vec![in_flight]).await.accepted_count, 0, "dropped");
    assert!(pins(&mut rens, QUIET).await.is_empty(), "friends keep seeing the paused pin");
    let seen = ctx.seen_by(&ren, &aoi).await.unwrap();
    assert!(seen.sharing_paused);
    assert_at!(pin_point(&seen), home);
    assert!(!ctx.my_location(&aoi).await.sharing);
    assert!(geoindex::get(&ctx.app.core.cache, aoi.id).await.unwrap().is_none());

    // A fix taken after the stop is the device sharing again.
    ctx.at(&aoi, home).await;
    assert!(!next_pin(&mut rens).await.unwrap().sharing_paused);
    assert!(ctx.my_location(&aoi).await.sharing);
}

#[tokio::test]
async fn a_sample_on_its_way_at_a_stop_does_not_resume_sharing_when_the_clock_went_back() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    let numbered = |point, at, sequence| LocationSample { sequence, ..sample(point, at) };
    assert_eq!(ctx.report(&aoi, vec![numbered(home, next_fix_time(aoi.id), 1)]).await.accepted_count, 1);
    let mut rens = ctx.app.listen_user(ren.id).await;

    // The device takes a fix, its clock is set an hour back, the user stops
    // sharing, and only then does the fix reach the server: by the clock it
    // came after the stop, by the device's own count before it.
    let in_flight = numbered(offset(home, 0.0, 60.0), next_fix_time(aoi.id), 2);
    let set_back = Utc::now() - Duration::hours(1);
    let stop = StopSharingRequest { stopped_at: tokuly_core::time::ts(set_back), sequence: 3, ..Default::default() };
    let _: StopSharingResponse = ctx.location("StopSharing", &aoi, &stop).await.unwrap();
    assert!(next_pin(&mut rens).await.unwrap().sharing_paused);
    assert_eq!(ctx.report(&aoi, vec![in_flight]).await.accepted_count, 0, "dropped");
    assert!(pins(&mut rens, QUIET).await.is_empty(), "friends keep seeing the paused pin");
    let seen = ctx.seen_by(&ren, &aoi).await.unwrap();
    assert!(seen.sharing_paused);
    assert_at!(pin_point(&seen), home);
    assert!(!ctx.my_location(&aoi).await.sharing);
    assert!(geoindex::get(&ctx.app.core.cache, aoi.id).await.unwrap().is_none());

    // The device's next fix is sharing again.
    assert_eq!(ctx.report(&aoi, vec![numbered(home, next_fix_time(aoi.id), 4)]).await.accepted_count, 1);
    assert!(!next_pin(&mut rens).await.unwrap().sharing_paused);
    assert!(ctx.my_location(&aoi).await.sharing);
}

#[tokio::test]
async fn a_stop_counts_only_for_the_samples_of_its_own_device() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let tablet = ctx.app.sign_in(aoi.id).await;
    let home = somewhere();
    ctx.at(&aoi, home).await;

    // The phone stops at its number 50; the tablet counts on its own, from 1.
    let stopped_at = Utc::now() - Duration::seconds(1);
    let stop = StopSharingRequest { stopped_at: tokuly_core::time::ts(stopped_at), sequence: 50, ..Default::default() };
    let _: StopSharingResponse = ctx.location("StopSharing", &aoi, &stop).await.unwrap();
    let mut rens = ctx.app.listen_user(ren.id).await;
    let fix = LocationSample { sequence: 1, ..sample(home, next_fix_time(aoi.id)) };
    assert_eq!(ctx.report(&tablet, vec![fix]).await.accepted_count, 1, "the tablet shares");
    assert!(!next_pin(&mut rens).await.unwrap().sharing_paused);
}

#[tokio::test]
async fn a_stop_overtaken_by_another_devices_fix_does_not_pause_sharing() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let tablet = ctx.app.sign_in(aoi.id).await;
    let home = somewhere();
    ctx.at(&aoi, home).await;

    // The phone stops; before its stop reaches the server, the tablet, which
    // goes on sharing, reports a fix taken after it.
    let stopped_at = next_fix_time(aoi.id);
    ctx.at(&tablet, offset(home, 0.0, 60.0)).await;
    let mut rens = ctx.app.listen_user(ren.id).await;
    let stop = StopSharingRequest { stopped_at: tokuly_core::time::ts(stopped_at), ..Default::default() };
    let _: StopSharingResponse = ctx.location("StopSharing", &aoi, &stop).await.unwrap();

    assert!(pins(&mut rens, QUIET).await.is_empty(), "friends are not told about a pause");
    assert!(!ctx.seen_by(&ren, &aoi).await.unwrap().sharing_paused);
    assert!(ctx.my_location(&aoi).await.sharing);
    assert!(geoindex::get(&ctx.app.core.cache, aoi.id).await.unwrap().is_some(), "still in the true-position index");

    // The tablet's own stop is not overtaken by its own fix, however it is timed.
    let _: StopSharingResponse = ctx.location("StopSharing", &tablet, &stop).await.unwrap();
    assert!(next_pin(&mut rens).await.unwrap().sharing_paused);
}

#[tokio::test]
async fn a_stop_from_the_device_of_the_newest_fix_pauses_whatever_the_clocks_say() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    // Fixes carry the positioning clock's time, the stop the system clock's:
    // here the first runs half a minute ahead of the second.
    let fixed_at = Utc::now() + Duration::seconds(30);
    assert_eq!(ctx.report(&aoi, vec![sample(home, fixed_at)]).await.accepted_count, 1);
    let mut rens = ctx.app.listen_user(ren.id).await;

    let stop = StopSharingRequest { stopped_at: tokuly_core::time::ts(Utc::now()), ..Default::default() };
    let _: StopSharingResponse = ctx.location("StopSharing", &aoi, &stop).await.unwrap();
    assert!(next_pin(&mut rens).await.unwrap().sharing_paused, "the only device stopped: the stop stops");
    assert!(ctx.seen_by(&ren, &aoi).await.unwrap().sharing_paused);
    assert!(!ctx.my_location(&aoi).await.sharing);
    assert!(geoindex::get(&ctx.app.core.cache, aoi.id).await.unwrap().is_none());
}

#[tokio::test]
async fn a_stop_is_not_overtaken_by_a_fix_whose_device_is_unknown() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let stopped_at = next_fix_time(aoi.id);
    let report = presence::LocationReport {
        user_id: aoi.id,
        session_id: None,
        received_at: Utc::now(),
        sample: sample(somewhere(), next_fix_time(aoi.id)),
    };
    ctx.geo.ingest(report).await.unwrap();
    let mut rens = ctx.app.listen_user(ren.id).await;

    let stop = StopSharingRequest { stopped_at: tokuly_core::time::ts(stopped_at), ..Default::default() };
    let _: StopSharingResponse = ctx.location("StopSharing", &aoi, &stop).await.unwrap();
    assert!(next_pin(&mut rens).await.unwrap().sharing_paused);
}

#[tokio::test]
async fn a_stop_timed_with_the_newest_fix_wins_over_it() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    let fixed_at = next_fix_time(aoi.id);
    assert_eq!(ctx.report(&aoi, vec![sample(home, fixed_at)]).await.accepted_count, 1);
    let mut rens = ctx.app.listen_user(ren.id).await;

    // The fix was recorded at, not after, the stop: it does not keep sharing on.
    let stop = StopSharingRequest { stopped_at: tokuly_core::time::ts(fixed_at), ..Default::default() };
    let _: StopSharingResponse = ctx.location("StopSharing", &aoi, &stop).await.unwrap();
    assert!(next_pin(&mut rens).await.unwrap().sharing_paused);
    assert!(!ctx.my_location(&aoi).await.sharing);
}

#[tokio::test]
async fn a_stop_without_a_time_pauses_even_a_clock_running_ahead() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    // The phone's clock is a minute ahead of the server's.
    let ahead = Utc::now() + Duration::minutes(1);
    assert_eq!(ctx.report(&aoi, vec![sample(home, ahead)]).await.accepted_count, 1);
    let mut rens = ctx.app.listen_user(ren.id).await;

    let _: StopSharingResponse = ctx.location("StopSharing", &aoi, &StopSharingRequest::default()).await.unwrap();
    assert!(next_pin(&mut rens).await.unwrap().sharing_paused, "a stop the server cannot order still stops");
    assert!(!ctx.my_location(&aoi).await.sharing);

    // The phone's next fix resumes it.
    let next = sample(home, ahead + Duration::seconds(5));
    assert_eq!(ctx.report(&aoi, vec![next]).await.accepted_count, 1);
    assert!(!next_pin(&mut rens).await.unwrap().sharing_paused);
}

#[tokio::test]
async fn privacy_settings_withhold_battery_activity_and_the_online_halo() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    let reading = LocationSample {
        battery_percent: Some(55),
        speed_mps: Some(1.2),
        activity: Activity::ACTIVITY_CYCLING.into(),
        ..sample(home, next_fix_time(aoi.id))
    };
    ctx.report(&aoi, vec![reading]).await;
    presence::mark_online(&ctx.app.core.cache, aoi.id).await.unwrap();
    let seen = ctx.seen_by(&ren, &aoi).await.unwrap();
    assert_eq!((seen.battery_percent, seen.speed_mps), (Some(55), 1.2));
    assert_eq!(seen.activity, Activity::ACTIVITY_CYCLING, "the device's own activity wins over inference");
    assert!(seen.online);

    let mut rens = ctx.app.listen_user(ren.id).await;
    ctx.set_privacy(&aoi, |privacy| {
        privacy.share_battery = false;
        privacy.share_activity = false;
        privacy.share_online_status = false;
    })
    .await;
    // The change reaches friends at once, not with the next sample.
    let pushed = next_pin(&mut rens).await.expect("a refreshed pin");
    assert_eq!((pushed.battery_percent, pushed.speed_mps), (None, 0.0));
    assert_eq!(pushed.activity, Activity::ACTIVITY_UNSPECIFIED);
    assert!(!pushed.online);
    assert_at!(pin_point(&pushed), home, "the position is ghost mode's business, not a privacy switch");
    // The owner still sees everything about themselves.
    let mine = ctx.my_location(&aoi).await;
    assert_eq!(mine.location.as_option().unwrap().battery_percent, Some(55));
    presence::mark_offline(&ctx.app.core.cache, aoi.id).await.unwrap();
}

#[tokio::test]
async fn the_status_emoji_rides_on_the_pin() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    ctx.at(&aoi, somewhere()).await;
    assert!(ctx.seen_by(&ren, &aoi).await.unwrap().status.as_option().is_none());
    let mut rens = ctx.app.listen_user(ren.id).await;
    let _: SetStatusResponse = ctx
        .app
        .call(
            "tokuly.identity.v1.AccountService/SetStatus",
            Some(&aoi),
            &SetStatusRequest { emoji: "🍜".into(), text: "ラーメン".into(), ..Default::default() },
        )
        .await
        .unwrap();
    ctx.deliver("identity.user.status_changed", aoi.id, serde_json::json!({ "user_id": aoi.id })).await;
    let pushed = next_pin(&mut rens).await.expect("the pin with the new status");
    let status = pushed.status.as_option().expect("a status");
    assert_eq!((status.emoji.as_str(), status.text.as_str()), ("🍜", "ラーメン"));
}

#[tokio::test]
async fn watch_friend_locations_streams_a_snapshot_then_updates() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    ctx.at(&aoi, home).await;

    let request = WatchFriendLocationsRequest::default();
    let watch = ctx.app.stream::<_, WatchFriendLocationsResponse>(
        "tokuly.geo.v1.LocationService/WatchFriendLocations",
        Some(&ren),
        &request,
        2,
        SOON * 3,
    );
    tokio::pin!(watch);
    // The friend keeps moving while the stream is open (it may take a moment to start on a busy machine).
    let mut step = 0;
    let messages = loop {
        tokio::select! {
            messages = &mut watch => break messages.unwrap(),
            _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                step += 1;
                ctx.at(&aoi, offset(home, 90.0, 300.0 + step as f64 * 10.0)).await;
            }
        }
    };
    assert_eq!(messages.len(), 2);
    let Some(Update::Snapshot(snapshot)) = &messages[0].update else {
        panic!("the first message is the snapshot: {:?}", messages[0].update)
    };
    assert_eq!(snapshot.locations.len(), 1);
    assert_eq!(snapshot.locations[0].user_id, aoi.id.to_string());
    assert!(snapshot.server_time.as_option().is_some());
    let Some(Update::Location(location)) = &messages[1].update else {
        panic!("then live pins: {:?}", messages[1].update)
    };
    assert_eq!(location.user_id, aoi.id.to_string());
    assert!(distance(pin_point(location), home) >= 300.0, "a live update, not the snapshot again");

    let anonymous = ctx
        .app
        .stream::<_, WatchFriendLocationsResponse>(
            "tokuly.geo.v1.LocationService/WatchFriendLocations",
            None,
            &WatchFriendLocationsRequest::default(),
            1,
            SOON,
        )
        .await;
    assert_eq!(anonymous.unwrap_err().reason, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn friendship_changes_show_and_remove_pins() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    let (home, office) = (somewhere(), somewhere());
    ctx.at(&aoi, home).await;
    ctx.at(&ren, office).await;
    let mut aois = ctx.app.listen_user(aoi.id).await;
    let mut rens = ctx.app.listen_user(ren.id).await;

    // Becoming friends: each gets the other's pin without waiting for a sample.
    ctx.befriend(&aoi, &ren).await;
    ctx.deliver(
        "social.friend.accepted",
        ren.id,
        serde_json::json!({ "user_id": ren.id, "friend_id": aoi.id, "source": 1 }),
    )
    .await;
    assert_at!(pin_point(&next_pin(&mut aois).await.unwrap()), office);
    assert_at!(pin_point(&next_pin(&mut rens).await.unwrap()), home);

    // Something set up between the two.
    ctx.ghost(&aoi, &[&ren], GhostMode::GHOST_MODE_BLURRED, 0).await.unwrap();
    pushes(&mut rens, QUIET).await;

    ctx.unfriend(&aoi, &ren).await;
    let removed = |events: Vec<Push>| {
        events.into_iter().find_map(|event| if let Push::LocationRemovedUserId(id) = event { Some(id) } else { None })
    };
    assert_eq!(removed(pushes(&mut aois, QUIET).await), Some(ren.id.to_string()));
    assert_eq!(removed(pushes(&mut rens, QUIET).await), Some(aoi.id.to_string()));
    assert!(ctx.friend_locations(&ren).await.is_empty() && ctx.friend_locations(&aoi).await.is_empty());
    let settings: GetGhostSettingsResponse =
        ctx.location("GetGhostSettings", &aoi, &GetGhostSettingsRequest::default()).await.unwrap();
    assert!(settings.overrides.is_empty(), "overrides for people who are no longer friends are dropped");
    // New samples no longer reach the former friend.
    ctx.at(&aoi, offset(home, 0.0, 50.0)).await;
    assert!(pins(&mut rens, QUIET).await.is_empty());

    // A block does the same (delivered as its own event).
    ctx.befriend(&aoi, &ren).await;
    ctx.deliver("social.user.blocked", aoi.id, serde_json::json!({ "user_id": aoi.id, "blocked_id": ren.id })).await;
    assert_eq!(removed(pushes(&mut rens, QUIET).await), Some(aoi.id.to_string()));
}

#[tokio::test]
async fn banned_and_deleted_accounts_disappear_from_the_map() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    ctx.at(&aoi, somewhere()).await;
    assert!(ctx.seen_by(&ren, &aoi).await.is_some());

    sqlx::query("UPDATE users SET banned_at = now() WHERE id = $1")
        .bind(aoi.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let mut rens = ctx.app.listen_user(ren.id).await;
    ctx.deliver("identity.user.banned", aoi.id, serde_json::json!({ "user_id": aoi.id })).await;
    assert!(matches!(next_push(&mut rens).await, Some(Push::LocationRemovedUserId(id)) if id == aoi.id.to_string()));
    assert!(ctx.seen_by(&ren, &aoi).await.is_none());
    // Samples of a banned account are ignored.
    let report = presence::LocationReport {
        user_id: aoi.id,
        session_id: None,
        received_at: Utc::now(),
        sample: sample(somewhere(), next_fix_time(aoi.id)),
    };
    ctx.geo.ingest(report).await.unwrap();
    assert!(pins(&mut rens, QUIET).await.is_empty());

    sqlx::query("UPDATE users SET banned_at = NULL WHERE id = $1")
        .bind(aoi.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.deliver("identity.user.unbanned", aoi.id, serde_json::json!({ "user_id": aoi.id })).await;
    assert!(next_pin(&mut rens).await.is_some(), "back on the map when the ban is lifted");
}
