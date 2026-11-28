//! Stays, visits and nights as the pipeline derives them, and
//! `HistoryService` over PostgreSQL and ClickHouse.
mod common;

use chrono::{DateTime, Datelike as _, Duration, TimeZone as _, Timelike as _, Utc};
use common::*;
use tokuly_core::time::{from_ts, ts};
use tokuly_geo::math;
use tokuly_media::{assets, testing::ready_image};
use tokuly_proto::{
    common::{PageRequest, Visibility},
    geo::*,
    media::AssetPurpose,
};
use tokuly_testkit::TestUser;

/// A time zone in which it is around noon right now, and today's date there:
/// tests about "a day" must not straddle midnight whenever they happen to run.
fn midday_zone() -> (String, String) {
    let now = Utc::now();
    let shift = 12 - now.hour() as i64; // hours to add to UTC
    let zone = match shift {
        0 => "Etc/GMT".to_owned(),
        s if s > 0 => format!("Etc/GMT-{s}"),
        s => format!("Etc/GMT+{}", -s),
    };
    (zone, (now + Duration::hours(shift)).date_naive().to_string())
}

fn close(a: DateTime<Utc>, b: DateTime<Utc>) -> bool {
    (a - b).num_seconds().abs() <= 1
}

async fn create_place(
    ctx: &Ctx,
    owner: &TestUser,
    name: &str,
    label: PlaceLabel,
    center: Point,
    visibility: Visibility,
) -> Place {
    let request = CreatePlaceRequest {
        name: name.into(),
        emoji: "📍".into(),
        label: label.into(),
        center: lat_lng(center).into(),
        radius_meters: 60.0,
        visibility: visibility.into(),
        ..Default::default()
    };
    let response: CreatePlaceResponse = ctx.places("CreatePlace", owner, &request).await.expect("create place");
    response.place.into_option().expect("place")
}

async fn visits(ctx: &Ctx, user: &TestUser) -> Vec<Visit> {
    let response: ListVisitsResponse =
        ctx.history("ListVisits", user, &ListVisitsRequest::default()).await.expect("list visits");
    response.visits
}

/// Ten minutes of walking east from `from` at 1.4 m/s (a sample every 10 s), then eight minutes at the end.
/// Returns where the walk ended. The whole track lies between `start` and `start + 18 min`.
async fn walk_then_stay(ctx: &Ctx, user: &TestUser, from: Point, start: DateTime<Utc>) -> Point {
    let mut samples: Vec<LocationSample> =
        (0..=60).map(|i| sample(offset(from, 90.0, i as f64 * 14.0), start + Duration::seconds(i * 10))).collect();
    let end = offset(from, 90.0, 840.0);
    samples.extend((1..=16).map(|i| sample(end, start + Duration::seconds(600 + i * 30))));
    let sent = samples.len() as i32;
    assert_eq!(ctx.report(user, samples).await.accepted_count, sent);
    end
}

/// How many footprint cells `user` has within about 20 km of `center`.
async fn cells_around(ctx: &Ctx, user: TestUser, center: Point) -> usize {
    let request = GetCellsRequest {
        bounds: tokuly_proto::common::BoundingBox {
            south_west: lat_lng((center.0 - 0.2, center.1 - 0.2)).into(),
            north_east: lat_lng((center.0 + 0.2, center.1 + 0.2)).into(),
            ..Default::default()
        }
        .into(),
        zoom: 15.0,
        ..Default::default()
    };
    ctx.footprints::<_, GetCellsResponse>("GetCells", &user, &request).await.unwrap().cells.len()
}

#[tokio::test]
async fn a_stay_becomes_a_visit_at_the_matching_place() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let cafe = somewhere();
    let place =
        create_place(&ctx, &aoi, "カフェ モカ", PlaceLabel::PLACE_LABEL_CUSTOM, cafe, Visibility::VISIBILITY_FRIENDS)
            .await;
    let mut rens = ctx.open_app(&ren).await;

    // Four minutes: not a stay yet.
    let arrived = minutes_ago(12);
    ctx.stay(&aoi, cafe, arrived, arrived + Duration::minutes(4)).await;
    assert!(visits(&ctx, &aoi).await.is_empty());
    assert!(ctx.my_location(&aoi).await.location.as_option().unwrap().stationary_since.as_option().is_none());

    // Ten minutes: a stay since the first sample, at the café.
    ctx.stay(&aoi, cafe, arrived + Duration::seconds(270), minutes_ago(2)).await;
    let mine = ctx.my_location(&aoi).await.location.into_option().unwrap();
    assert!(close(from_ts(mine.stationary_since.as_option().expect("time on the spot")), arrived));
    assert_eq!((mine.place_name.as_str(), mine.place_emoji.as_str()), ("カフェ モカ", "📍"));
    assert_eq!(mine.activity, Activity::ACTIVITY_STILL);

    let listed = visits(&ctx, &aoi).await;
    assert_eq!(listed.len(), 1);
    let visit = &listed[0];
    assert_eq!(visit.place.as_option().expect("matched to the place").id, place.id);
    assert_eq!(visit.name, "カフェ モカ");
    assert!(close(from_ts(visit.arrived_at.as_option().unwrap()), arrived));
    assert!(visit.departed_at.as_option().is_none(), "ongoing");
    assert!(visit.duration_seconds >= 600 && !visit.check_in);
    assert!(distance(point(visit.center.as_option()), cafe) < 5.0);
    assert_eq!(visit.place.as_option().unwrap().my_visit_count, 1);

    // The friend sees the place name (the place is shared with friends) and was told about the arrival.
    let seen = ctx.seen_by(&ren, &aoi).await.unwrap();
    assert_eq!(
        (seen.place_name.as_str(), seen.place_label.as_known()),
        ("カフェ モカ", Some(PlaceLabel::PLACE_LABEL_CUSTOM))
    );
    assert!(close(from_ts(seen.stationary_since.as_option().unwrap()), arrived));
    let arrival = pushes(&mut rens, QUIET)
        .await
        .into_iter()
        .find_map(|push| if let geo_event::Event::Visit(visit) = push { Some(*visit) } else { None })
        .expect("a visit event");
    assert!(arrival.arrived);
    assert_eq!((arrival.user_id.as_str(), arrival.place_name.as_str()), (aoi.id.to_string().as_str(), "カフェ モカ"));

    // Leaving: two samples away from the spot end the stay at the last sample inside it.
    let left = minutes_ago(2);
    ctx.report(
        &aoi,
        vec![sample(offset(cafe, 0.0, 250.0), seconds_ago(60)), sample(offset(cafe, 0.0, 400.0), seconds_ago(20))],
    )
    .await;
    let listed = visits(&ctx, &aoi).await;
    assert_eq!(listed.len(), 1);
    assert!((from_ts(listed[0].departed_at.as_option().expect("departed")) - left).num_seconds().abs() <= 31);
    assert!(ctx.my_location(&aoi).await.location.as_option().unwrap().stationary_since.as_option().is_none());
    let departure = pushes(&mut rens, QUIET)
        .await
        .into_iter()
        .find_map(|push| if let geo_event::Event::Visit(visit) = push { Some(*visit) } else { None })
        .expect("a visit event");
    assert!(!departure.arrived);

    // Other people's history is nobody's business: the friend's list is their own (empty).
    assert!(visits(&ctx, &ren).await.is_empty());
}

#[tokio::test]
async fn home_shows_friends_a_badge_but_never_its_name() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    create_place(&ctx, &aoi, "世田谷のアパート", PlaceLabel::PLACE_LABEL_HOME, home, Visibility::VISIBILITY_PUBLIC)
        .await;
    ctx.stay(&aoi, home, minutes_ago(8), seconds_ago(10)).await;
    let seen = ctx.seen_by(&ren, &aoi).await.unwrap();
    assert_eq!(seen.place_label, PlaceLabel::PLACE_LABEL_HOME);
    assert!(seen.place_name.is_empty() && seen.place_emoji.is_empty());
    // Hiding place labels hides the badge as well.
    ctx.set_privacy(&aoi, |privacy| privacy.share_place_labels = false).await;
    assert_eq!(ctx.seen_by(&ren, &aoi).await.unwrap().place_label, PlaceLabel::PLACE_LABEL_UNSPECIFIED);
    // The owner sees their own place by name in their history.
    assert_eq!(visits(&ctx, &aoi).await[0].name, "世田谷のアパート");
}

#[tokio::test]
async fn a_night_at_home_is_recorded() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    // A home in Tokyo (so that "night" is Japanese night), a little off for every run.
    let home = offset(
        (35.6595, 139.7005),
        rand::random::<u16>() as f64 % 360.0,
        150.0 + rand::random::<u16>() as f64 % 3000.0,
    );
    create_place(&ctx, &aoi, "うち", PlaceLabel::PLACE_LABEL_HOME, home, Visibility::VISIBILITY_PRIVATE).await;

    // The night before last, 21:00 to 08:00 Japan time: always in the past, always within the upload window.
    let tokyo = chrono_tz::Asia::Tokyo;
    let evening = (Utc::now().with_timezone(&tokyo) - Duration::days(2)).date_naive();
    let from = tokyo.from_local_datetime(&evening.and_hms_opt(21, 0, 0).unwrap()).unwrap().with_timezone(&Utc);
    let to = from + Duration::hours(11);
    let mut samples: Vec<LocationSample> = (0..=33).map(|i| sample(home, from + Duration::minutes(i * 20))).collect();
    // Then off to work.
    samples.push(sample(offset(home, 90.0, 500.0), to + Duration::minutes(10)));
    samples.push(sample(offset(home, 90.0, 900.0), to + Duration::minutes(12)));
    let sent = samples.len() as i32;
    assert_eq!(ctx.report(&aoi, samples).await.accepted_count, sent);

    let nights: ListNightsResponse = ctx.history("ListNights", &aoi, &ListNightsRequest::default()).await.unwrap();
    assert_eq!(nights.nights.len(), 1);
    let night = &nights.nights[0];
    assert_eq!(night.date, evening.to_string());
    assert!(night.at_home);
    assert_eq!(night.name, "うち");
    assert!(close(from_ts(night.from.as_option().unwrap()), from) && close(from_ts(night.to.as_option().unwrap()), to));
    assert!(distance(point(night.center.as_option()), home) < 5.0);
    assert!(!nights.page.as_option().unwrap().has_more);

    let listed = visits(&ctx, &aoi).await;
    assert_eq!(listed.len(), 1);
    assert!(close(from_ts(listed[0].departed_at.as_option().unwrap()), to));
    assert!(
        !listed[0].locality.is_empty() && !listed[0].locality.is_ascii(),
        "a Japanese area name for a Japanese account: {:?}",
        listed[0].locality
    );

    let stats: GetStatsResponse = ctx.history("GetStats", &aoi, &GetStatsRequest::default()).await.unwrap();
    assert_eq!((stats.visit_count, stats.place_count, stats.nights_away), (1, 1, 0));
}

#[tokio::test]
async fn a_day_has_a_path_visits_and_totals() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    let from = somewhere();
    let start = minutes_ago(25);
    let end = walk_then_stay(&ctx, &aoi, from, start).await;
    let (zone, date) = midday_zone();

    let day: GetDayResponse = ctx
        .history("GetDay", &aoi, &GetDayRequest { date: date.clone(), time_zone: zone.clone(), ..Default::default() })
        .await
        .unwrap();
    let summary = day.summary.as_option().unwrap();
    assert_eq!(summary.date, date);
    assert!((summary.distance_meters - 840.0).abs() < 60.0, "walked {} m", summary.distance_meters);
    assert_eq!(summary.visit_count, 1);
    assert!(summary.new_cell_count >= 3, "{} cells along 840 m", summary.new_cell_count);
    assert!(!summary.has_memo);

    // The path comes from ClickHouse: a straight walk simplifies to very few vertices.
    let path = math::decode_polyline(&day.path_polyline, 5);
    assert!((2..=6).contains(&path.len()), "{} vertices", path.len());
    assert!(distance(path[0], from) < 5.0 && distance(*path.last().unwrap(), end) < 5.0);
    let bounds = day.bounds.as_option().unwrap();
    let (sw, ne) = (point(bounds.south_west.as_option()), point(bounds.north_east.as_option()));
    assert!(sw.1 <= from.1 + 1e-6 && ne.1 >= end.1 - 1e-6 && sw.0 <= ne.0);
    assert!((1.2..=1.6).contains(&day.max_speed_mps), "{}", day.max_speed_mps);
    assert_eq!(day.visits.len(), 1);
    assert!(distance(point(day.visits[0].center.as_option()), end) < 40.0);
    assert!(day.memo.as_option().is_none() && day.friends_met.is_empty());

    // The calendar shows the day.
    let today = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d").unwrap();
    let month = ListActiveDaysRequest {
        year: today.year(),
        month: today.month() as i32,
        time_zone: zone.clone(),
        ..Default::default()
    };
    let days: ListActiveDaysResponse = ctx.history("ListActiveDays", &aoi, &month).await.unwrap();
    assert_eq!(days.days.len(), 1);
    assert_eq!(days.days[0], *summary);

    // Route replay: thinned out evenly, oldest first.
    let range = |max_points: i32| GetRouteRequest {
        from: ts(start - Duration::minutes(1)),
        to: ts(Utc::now()),
        max_points,
        ..Default::default()
    };
    let route: GetRouteResponse = ctx.history("GetRoute", &aoi, &range(0)).await.unwrap();
    assert_eq!(route.points.len(), 77, "every sample");
    assert!(
        route.points.windows(2).all(|pair| from_ts(pair[0].recorded_at.as_option().unwrap())
            < from_ts(pair[1].recorded_at.as_option().unwrap()))
    );
    assert!(distance(point(route.points[0].position.as_option()), from) < 1.0);
    assert_eq!(route.points[30].activity, Activity::ACTIVITY_WALKING);
    let thinned: GetRouteResponse = ctx.history("GetRoute", &aoi, &range(20)).await.unwrap();
    assert!((10..=20).contains(&thinned.points.len()), "{}", thinned.points.len());
    assert!(distance(point(thinned.points[0].position.as_option()), from) < 1.0);

    // Statistics for all time and for a range.
    let stats: GetStatsResponse = ctx.history("GetStats", &aoi, &GetStatsRequest::default()).await.unwrap();
    assert!((stats.distance_meters - 840.0).abs() < 60.0);
    assert_eq!(stats.visit_count, 1);
    assert!(stats.cell_count >= 3);
    let walking = stats.by_activity.iter().find(|a| a.activity == Activity::ACTIVITY_WALKING).expect("walking");
    assert!((walking.distance_meters - 840.0).abs() < 60.0 && (500..=700).contains(&walking.duration_seconds));
    assert!(stats.by_activity.iter().any(|a| a.activity == Activity::ACTIVITY_STILL && a.duration_seconds > 100));
    assert!((1.2..=1.6).contains(&stats.max_speed_mps));
    let before: GetStatsResponse = ctx
        .history("GetStats", &aoi, &GetStatsRequest { to: ts(start - Duration::hours(2)), ..Default::default() })
        .await
        .unwrap();
    assert_eq!((before.distance_meters, before.visit_count, before.cell_count), (0.0, 0, 0));
}

#[tokio::test]
async fn history_requests_are_validated() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    let reason = |error: tokuly_testkit::RpcError| (error.code, error.reason);
    let invalid = |reason: &str| ("invalid_argument".to_owned(), reason.to_owned());
    let (now, earlier) = (Utc::now(), minutes_ago(60));

    let backwards = ListVisitsRequest { from: ts(now), to: ts(earlier), ..Default::default() };
    assert_eq!(
        reason(ctx.history::<_, ListVisitsResponse>("ListVisits", &aoi, &backwards).await.unwrap_err()),
        invalid("INVALID_RANGE")
    );
    let bad_cursor = ListVisitsRequest {
        page: PageRequest { cursor: "!!".into(), ..Default::default() }.into(),
        ..Default::default()
    };
    assert_eq!(
        reason(ctx.history::<_, ListVisitsResponse>("ListVisits", &aoi, &bad_cursor).await.unwrap_err()),
        invalid("INVALID_CURSOR")
    );
    let bad_place = ListVisitsRequest { place_id: "x".into(), ..Default::default() };
    assert_eq!(
        reason(ctx.history::<_, ListVisitsResponse>("ListVisits", &aoi, &bad_place).await.unwrap_err()),
        invalid("INVALID_ID")
    );

    for (year, month) in [(2026, 0), (2026, 13), (1800, 5)] {
        let request = ListActiveDaysRequest { year, month, ..Default::default() };
        assert_eq!(
            reason(ctx.history::<_, ListActiveDaysResponse>("ListActiveDays", &aoi, &request).await.unwrap_err()),
            invalid("INVALID_DATE"),
            "{year}-{month}"
        );
    }
    let bad_zone =
        ListActiveDaysRequest { year: 2026, month: 11, time_zone: "Mars/Olympus".into(), ..Default::default() };
    assert_eq!(
        reason(ctx.history::<_, ListActiveDaysResponse>("ListActiveDays", &aoi, &bad_zone).await.unwrap_err()),
        invalid("INVALID_TIME_ZONE")
    );
    // A month without anything is simply empty.
    let empty: ListActiveDaysResponse = ctx
        .history(
            "ListActiveDays",
            &aoi,
            &ListActiveDaysRequest { year: 2026, month: 11, time_zone: "Asia/Tokyo".into(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(empty.days.is_empty());

    for date in ["", "2026-02-30", "20261120"] {
        let request = GetDayRequest { date: date.into(), ..Default::default() };
        assert_eq!(
            reason(ctx.history::<_, GetDayResponse>("GetDay", &aoi, &request).await.unwrap_err()),
            invalid("INVALID_DATE"),
            "{date:?}"
        );
    }
    let bad_zone = GetDayRequest { date: "2026-11-20".into(), time_zone: "nope".into(), ..Default::default() };
    assert_eq!(
        reason(ctx.history::<_, GetDayResponse>("GetDay", &aoi, &bad_zone).await.unwrap_err()),
        invalid("INVALID_TIME_ZONE")
    );
    let quiet: GetDayResponse =
        ctx.history("GetDay", &aoi, &GetDayRequest { date: "2026-11-20".into(), ..Default::default() }).await.unwrap();
    assert!(quiet.path_polyline.is_empty() && quiet.visits.is_empty() && quiet.bounds.as_option().is_none());

    for request in [
        GetRouteRequest::default(),
        GetRouteRequest { from: ts(earlier), ..Default::default() },
        GetRouteRequest { from: ts(now), to: ts(earlier), ..Default::default() },
        GetRouteRequest { from: ts(now - Duration::days(40)), to: ts(now), ..Default::default() },
    ] {
        assert_eq!(
            reason(ctx.history::<_, GetRouteResponse>("GetRoute", &aoi, &request).await.unwrap_err()),
            invalid("INVALID_RANGE")
        );
    }
    let backwards = GetStatsRequest { from: ts(now), to: ts(earlier), ..Default::default() };
    assert_eq!(
        reason(ctx.history::<_, GetStatsResponse>("GetStats", &aoi, &backwards).await.unwrap_err()),
        invalid("INVALID_RANGE")
    );
    let backwards = DeleteHistoryRequest { from: ts(now), to: ts(earlier), ..Default::default() };
    assert_eq!(
        reason(ctx.history::<_, DeleteHistoryResponse>("DeleteHistory", &aoi, &backwards).await.unwrap_err()),
        invalid("INVALID_RANGE")
    );

    let anonymous = ctx
        .app
        .call::<_, GetStatsResponse>("tokuly.geo.v1.HistoryService/GetStats", None, &GetStatsRequest::default())
        .await
        .unwrap_err();
    assert_eq!(anonymous.reason, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn visits_are_paged_newest_first_and_filtered() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    let spot = somewhere();
    let place =
        create_place(&ctx, &aoi, "図書館", PlaceLabel::PLACE_LABEL_CUSTOM, spot, Visibility::VISIBILITY_PRIVATE).await;
    // Three check-ins: two at the place, one elsewhere.
    let check_in = async |request: CheckInRequest| {
        let response: CheckInResponse = ctx.places("CheckIn", &aoi, &request).await.unwrap();
        response.visit.into_option().unwrap()
    };
    let first = check_in(CheckInRequest {
        r#where: Some(check_in_request::Where::PlaceId(place.id.clone())),
        message: "1".into(),
        ..Default::default()
    })
    .await;
    let second = check_in(CheckInRequest {
        r#where: Some(check_in_request::Where::Position(Box::new(lat_lng(offset(spot, 0.0, 5000.0))))),
        message: "2".into(),
        ..Default::default()
    })
    .await;
    let third = check_in(CheckInRequest {
        r#where: Some(check_in_request::Where::PlaceId(place.id.clone())),
        message: "3".into(),
        ..Default::default()
    })
    .await;
    assert!(first.check_in && first.place.as_option().is_some() && second.place.as_option().is_none());

    let page = |limit: i32, cursor: &str| ListVisitsRequest {
        page: PageRequest { limit, cursor: cursor.into(), ..Default::default() }.into(),
        ..Default::default()
    };
    let one: ListVisitsResponse = ctx.history("ListVisits", &aoi, &page(2, "")).await.unwrap();
    assert_eq!(
        one.visits.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(),
        vec![third.id.as_str(), second.id.as_str()]
    );
    let info = one.page.as_option().unwrap();
    assert!(info.has_more && !info.next_cursor.is_empty());
    let two: ListVisitsResponse = ctx.history("ListVisits", &aoi, &page(2, &info.next_cursor)).await.unwrap();
    assert_eq!(two.visits.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), vec![first.id.as_str()]);
    assert!(!two.page.as_option().unwrap().has_more);

    let at_place: ListVisitsResponse = ctx
        .history("ListVisits", &aoi, &ListVisitsRequest { place_id: place.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(at_place.visits.len(), 2);
    assert!(at_place.visits.iter().all(|v| v.place.as_option().unwrap().my_visit_count == 2));
    let future: ListVisitsResponse = ctx
        .history(
            "ListVisits",
            &aoi,
            &ListVisitsRequest { from: ts(Utc::now() + Duration::hours(1)), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(future.visits.is_empty());
    let past: ListVisitsResponse = ctx
        .history(
            "ListVisits",
            &aoi,
            &ListVisitsRequest {
                from: ts(minutes_ago(5)),
                to: ts(Utc::now() + Duration::minutes(1)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(past.visits.len(), 3);
}

#[tokio::test]
async fn day_memos_keep_text_and_photos() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    let core = &ctx.app.core;
    let (zone, date) = midday_zone();
    let photo = ready_image(core, aoi.id, AssetPurpose::ASSET_PURPOSE_DAY_MEMO).await.unwrap();
    let second = ready_image(core, aoi.id, AssetPurpose::ASSET_PURPOSE_DAY_MEMO).await.unwrap();
    let memo = |text: &str, photos: &[uuid::Uuid]| SetDayMemoRequest {
        date: date.clone(),
        text: text.into(),
        photo_asset_ids: photos.iter().map(|id| id.to_string()).collect(),
        ..Default::default()
    };

    let set: SetDayMemoResponse =
        ctx.history("SetDayMemo", &aoi, &memo("  渋谷でラーメン食べた!  ", &[photo.id])).await.unwrap();
    let stored = set.memo.as_option().unwrap();
    assert_eq!(stored.text, "渋谷でラーメン食べた!");
    assert_eq!(stored.photos.len(), 1);
    assert_eq!(stored.photos[0].asset_id, photo.id.to_string());
    assert!(stored.updated_at.as_option().is_some());
    assert!(
        assets::is_attached(core, photo.id).await.unwrap(),
        "a memo holds its photos, or the media janitor would collect them"
    );

    let day: GetDayResponse = ctx
        .history("GetDay", &aoi, &GetDayRequest { date: date.clone(), time_zone: zone.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(day.memo.as_option().unwrap().text, "渋谷でラーメン食べた!");
    let summary = day.summary.as_option().unwrap();
    assert!(summary.has_memo);
    assert_eq!(summary.thumbnail.as_option().unwrap().asset_id, photo.id.to_string());
    // Someone else's day is empty.
    let theirs: GetDayResponse = ctx
        .history("GetDay", &ren, &GetDayRequest { date: date.clone(), time_zone: zone.clone(), ..Default::default() })
        .await
        .unwrap();
    assert!(theirs.memo.as_option().is_none());

    // Replacing the photos releases the old ones.
    let set: SetDayMemoResponse = ctx.history("SetDayMemo", &aoi, &memo("", &[second.id, second.id])).await.unwrap();
    assert_eq!(set.memo.as_option().unwrap().photos.len(), 1, "duplicates collapse");
    assert!(!assets::is_attached(core, photo.id).await.unwrap() && assets::is_attached(core, second.id).await.unwrap());

    // Validation.
    let reason = async |request: SetDayMemoRequest| {
        ctx.history::<_, SetDayMemoResponse>("SetDayMemo", &aoi, &request).await.unwrap_err().reason
    };
    assert_eq!(reason(SetDayMemoRequest { date: "soon".into(), ..Default::default() }).await, "INVALID_DATE");
    assert_eq!(
        reason(SetDayMemoRequest {
            date: (Utc::now() + Duration::days(3)).date_naive().to_string(),
            text: "x".into(),
            ..Default::default()
        })
        .await,
        "INVALID_DATE"
    );
    assert_eq!(reason(memo(&"あ".repeat(2001), &[])).await, "TEXT_TOO_LONG");
    let many: Vec<uuid::Uuid> = (0..11).map(|_| uuid::Uuid::now_v7()).collect();
    assert_eq!(reason(memo("x", &many)).await, "TOO_MANY_PHOTOS");
    assert_eq!(reason(memo("x", &[uuid::Uuid::now_v7()])).await, "ASSET_NOT_FOUND");
    let rens_photo = ready_image(core, ren.id, AssetPurpose::ASSET_PURPOSE_DAY_MEMO).await.unwrap();
    assert_eq!(reason(memo("x", &[rens_photo.id])).await, "ASSET_NOT_FOUND", "someone else's upload");
    let avatar = ready_image(core, aoi.id, AssetPurpose::ASSET_PURPOSE_AVATAR).await.unwrap();
    assert_eq!(reason(memo("x", &[avatar.id])).await, "ASSET_WRONG_PURPOSE");

    // An empty memo is no memo.
    let cleared: SetDayMemoResponse = ctx.history("SetDayMemo", &aoi, &memo(" ", &[])).await.unwrap();
    assert!(cleared.memo.as_option().is_none());
    assert!(!assets::is_attached(core, second.id).await.unwrap());
    let day: GetDayResponse =
        ctx.history("GetDay", &aoi, &GetDayRequest { date, time_zone: zone, ..Default::default() }).await.unwrap();
    assert!(day.memo.as_option().is_none() && !day.summary.as_option().unwrap().has_memo);
}

#[tokio::test]
async fn deleting_history_removes_it_from_both_stores() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    let from = somewhere();
    // An earlier outing and a later one.
    let earlier = minutes_ago(90);
    walk_then_stay(&ctx, &aoi, from, earlier).await;
    ctx.report(
        &aoi,
        vec![
            sample(offset(from, 180.0, 3000.0), minutes_ago(60)),
            sample(offset(from, 180.0, 3200.0), minutes_ago(59)),
        ],
    )
    .await;
    let later = minutes_ago(30);
    walk_then_stay(&ctx, &aoi, offset(from, 180.0, 6000.0), later).await;

    let route =
        |from: DateTime<Utc>, to: DateTime<Utc>| GetRouteRequest { from: ts(from), to: ts(to), ..Default::default() };
    let points = async |request: GetRouteRequest| {
        ctx.history::<_, GetRouteResponse>("GetRoute", &aoi, &request).await.unwrap().points.len()
    };
    let cells = |user: &TestUser| cells_around(&ctx, user.clone(), from);
    assert_eq!(points(route(earlier - Duration::minutes(1), minutes_ago(65))).await, 77);
    assert_eq!(visits(&ctx, &aoi).await.len(), 2);
    let cells_before = cells(&aoi).await;
    let stats: GetStatsResponse = ctx.history("GetStats", &aoi, &GetStatsRequest::default()).await.unwrap();
    assert!((stats.distance_meters - 1680.0).abs() < 400.0, "{}", stats.distance_meters);

    // Delete the earlier outing, footprints included.
    let request = DeleteHistoryRequest {
        from: ts(earlier - Duration::minutes(5)),
        to: ts(minutes_ago(65)),
        include_footprints: true,
        ..Default::default()
    };
    let _: DeleteHistoryResponse = ctx.history("DeleteHistory", &aoi, &request).await.unwrap();
    assert_eq!(points(route(earlier - Duration::minutes(5), minutes_ago(65))).await, 0, "gone from ClickHouse");
    assert_eq!(points(route(later - Duration::minutes(1), Utc::now())).await, 77, "the rest is untouched");
    let remaining = visits(&ctx, &aoi).await;
    assert_eq!(remaining.len(), 1);
    assert!(from_ts(remaining[0].arrived_at.as_option().unwrap()) > minutes_ago(25));
    let cells_after = cells(&aoi).await;
    assert!(cells_after < cells_before && cells_after >= 3, "{cells_before} -> {cells_after}");
    let footprints: GetFootprintStatsResponse =
        ctx.footprints("GetStats", &aoi, &GetFootprintStatsRequest::default()).await.unwrap();
    assert!(
        footprints.total_cell_count >= cells_after as i64 && footprints.total_cell_count < cells_before as i64 + 3,
        "totals were rebuilt"
    );

    // Without a range: everything, including what is still in the live state.
    let _: DeleteHistoryResponse = ctx.history("DeleteHistory", &aoi, &DeleteHistoryRequest::default()).await.unwrap();
    assert_eq!(points(route(earlier - Duration::minutes(5), Utc::now())).await, 0);
    assert!(visits(&ctx, &aoi).await.is_empty());
    let stats: GetStatsResponse = ctx.history("GetStats", &aoi, &GetStatsRequest::default()).await.unwrap();
    assert_eq!((stats.distance_meters, stats.visit_count), (0.0, 0));
    assert_eq!(cells(&aoi).await, cells_after, "footprints stay unless asked for");
    // The pin itself is not history: friends still see it.
    assert!(ctx.my_location(&aoi).await.location.as_option().is_some());
}

#[tokio::test]
async fn retention_deletes_raw_history_older_than_the_setting() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    let spot = somewhere();
    for user in [&aoi, &ren] {
        let old = Utc::now() - Duration::days(3);
        ctx.report(
            user,
            vec![
                sample(spot, old),
                sample(offset(spot, 0.0, 50.0), old + Duration::seconds(30)),
                sample(offset(spot, 0.0, 100.0), next_fix_time(user.id)),
            ],
        )
        .await;
    }
    ctx.set_privacy(&aoi, |privacy| privacy.location_history_days = 1).await;
    assert!(ctx.geo.enforce_retention().await.unwrap() >= 1);

    let everything =
        GetRouteRequest { from: ts(Utc::now() - Duration::days(5)), to: ts(Utc::now()), ..Default::default() };
    let kept: GetRouteResponse = ctx.history("GetRoute", &aoi, &everything).await.unwrap();
    assert_eq!(kept.points.len(), 1, "only today's point is left");
    let untouched: GetRouteResponse = ctx.history("GetRoute", &ren, &everything).await.unwrap();
    assert_eq!(untouched.points.len(), 3, "0 days means keep forever");
}

#[tokio::test]
async fn nothing_is_recorded_when_history_is_switched_off() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    ctx.set_privacy(&aoi, |privacy| privacy.record_history = false).await;
    let from = somewhere();
    let start = minutes_ago(25);
    walk_then_stay(&ctx, &aoi, from, start).await;

    assert!(visits(&ctx, &aoi).await.is_empty());
    let route: GetRouteResponse = ctx
        .history(
            "GetRoute",
            &aoi,
            &GetRouteRequest { from: ts(start - Duration::minutes(1)), to: ts(Utc::now()), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(route.points.is_empty());
    let stats: GetStatsResponse = ctx.history("GetStats", &aoi, &GetStatsRequest::default()).await.unwrap();
    assert_eq!((stats.distance_meters, stats.cell_count), (0.0, 0));
    // Live sharing is a different matter: friends still see the pin and how long it has been there.
    let seen = ctx.seen_by(&ren, &aoi).await.unwrap();
    assert!(seen.stationary_since.as_option().is_some());

    // Switching it off mid-stay closes the open visit.
    let bob = ctx.app.user("bob").await;
    ctx.stay(&bob, somewhere(), minutes_ago(9), seconds_ago(5)).await;
    assert!(visits(&ctx, &bob).await[0].departed_at.as_option().is_none());
    ctx.set_privacy(&bob, |privacy| privacy.record_history = false).await;
    assert!(visits(&ctx, &bob).await[0].departed_at.as_option().is_some());
}
