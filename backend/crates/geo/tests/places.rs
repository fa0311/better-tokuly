//! `PlaceService`: places and their visibility, place cards, search,
//! suggestions, saved places, reverse geocoding and check-ins.
mod common;

use chrono::{Duration, TimeZone as _, Utc};
use common::*;
use tokuly_core::events::Event;
use tokuly_media::{assets, testing::ready_image};
use tokuly_proto::{
    common::{PageRequest, SubjectType, Visibility},
    geo::{check_in_request::Where, geo_event::Event as Push, *},
    media::AssetPurpose,
};
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

fn new_place(name: &str, center: Point, visibility: Visibility) -> CreatePlaceRequest {
    CreatePlaceRequest {
        name: name.into(),
        emoji: "☕".into(),
        center: lat_lng(center).into(),
        visibility: visibility.into(),
        ..Default::default()
    }
}

async fn create(ctx: &Ctx, owner: &TestUser, request: CreatePlaceRequest) -> Place {
    let response: CreatePlaceResponse = ctx.places("CreatePlace", owner, &request).await.expect("create place");
    response.place.into_option().expect("place")
}

async fn card(ctx: &Ctx, viewer: &TestUser, place_id: &str) -> Result<GetPlaceResponse, RpcError> {
    ctx.places("GetPlace", viewer, &GetPlaceRequest { place_id: place_id.to_owned(), ..Default::default() }).await
}

async fn my_places(ctx: &Ctx, owner: &TestUser) -> Vec<Place> {
    let response: ListMyPlacesResponse =
        ctx.places("ListMyPlaces", owner, &ListMyPlacesRequest::default()).await.unwrap();
    response.places
}

async fn check_in(ctx: &Ctx, user: &TestUser, request: CheckInRequest) -> Result<Visit, RpcError> {
    let response: CheckInResponse = ctx.places("CheckIn", user, &request).await?;
    Ok(response.visit.into_option().expect("visit"))
}

/// The latest search-index change for a place.
async fn search_change(ctx: &Ctx, place_id: &str) -> Option<Event> {
    ctx.app
        .events("search.changed")
        .await
        .into_iter()
        .filter(|event| event.data["id"] == serde_json::json!(place_id))
        .next_back()
}

#[tokio::test]
async fn places_are_created_listed_updated_and_deleted() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    let core = &ctx.app.core;
    let spot = somewhere();
    let photo = ready_image(core, aoi.id, AssetPurpose::ASSET_PURPOSE_PLACE_PHOTO).await.unwrap();

    let request = CreatePlaceRequest {
        category: " cafe ".into(),
        photo_asset_id: photo.id.to_string(),
        ..new_place("  カフェ モカ ", spot, Visibility::VISIBILITY_PUBLIC)
    };
    let place = create(&ctx, &aoi, request).await;
    assert_eq!((place.name.as_str(), place.emoji.as_str(), place.category.as_str()), ("カフェ モカ", "☕", "cafe"));
    assert_eq!(place.owner_id, aoi.id.to_string());
    assert_eq!(place.label, PlaceLabel::PLACE_LABEL_CUSTOM, "unspecified means a custom place");
    assert_eq!(place.radius_meters, 80.0, "the default radius");
    assert_eq!(place.visibility, Visibility::VISIBILITY_PUBLIC);
    assert_at!(point(place.center.as_option()), spot);
    assert_eq!(place.photo.as_option().unwrap().asset_id, photo.id.to_string());
    assert!(
        !place.auto_detected && !place.saved && place.my_visit_count == 0 && place.created_at.as_option().is_some()
    );
    assert!(assets::is_attached(core, photo.id).await.unwrap());
    // Public places are searchable.
    let indexed = search_change(&ctx, &place.id).await.expect("a search document");
    assert_eq!(
        (indexed.data["kind"].as_str(), indexed.data["doc"]["title"].as_str(), indexed.data["doc"]["path"].as_str()),
        (Some("place"), Some("カフェ モカ"), Some(format!("/places/{}", place.id).as_str()))
    );

    assert_eq!(my_places(&ctx, &aoi).await, vec![place.clone()]);
    assert!(my_places(&ctx, &ren).await.is_empty());

    // Update: only the fields sent change.
    let moved = offset(spot, 90.0, 40.0);
    let update = UpdatePlaceRequest {
        place_id: place.id.clone(),
        name: Some("喫茶 モカ".into()),
        center: lat_lng(moved).into(),
        radius_meters: Some(120.0),
        visibility: Some(Visibility::VISIBILITY_FRIENDS.into()),
        photo_asset_id: Some(String::new()),
        ..Default::default()
    };
    let updated: UpdatePlaceResponse = ctx.places("UpdatePlace", &aoi, &update).await.unwrap();
    let updated = updated.place.into_option().unwrap();
    assert_eq!((updated.name.as_str(), updated.emoji.as_str(), updated.category.as_str()), ("喫茶 モカ", "☕", "cafe"));
    assert_at!(point(updated.center.as_option()), moved);
    assert_eq!(updated.radius_meters, 120.0);
    assert_eq!(updated.visibility, Visibility::VISIBILITY_FRIENDS);
    assert!(updated.photo.as_option().is_none());
    assert!(!assets::is_attached(core, photo.id).await.unwrap(), "the photo was released");
    // No longer public: removed from the search index.
    assert!(search_change(&ctx, &place.id).await.unwrap().data["doc"].is_null());

    // Only the owner may change or delete it.
    let foreign =
        UpdatePlaceRequest { place_id: place.id.clone(), name: Some("乗っ取り".into()), ..Default::default() };
    let error = ctx.places::<_, UpdatePlaceResponse>("UpdatePlace", &ren, &foreign).await.unwrap_err();
    assert_eq!((error.code.as_str(), error.reason.as_str()), ("not_found", "PLACE_NOT_FOUND"));
    let error = ctx
        .places::<_, DeletePlaceResponse>(
            "DeletePlace",
            &ren,
            &DeletePlaceRequest { place_id: place.id.clone(), ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!(error.reason, "PLACE_NOT_FOUND");
    assert_eq!(my_places(&ctx, &aoi).await[0].name, "喫茶 モカ");

    let _: DeletePlaceResponse = ctx
        .places("DeletePlace", &aoi, &DeletePlaceRequest { place_id: place.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert!(my_places(&ctx, &aoi).await.is_empty());
    assert_eq!(card(&ctx, &aoi, &place.id).await.unwrap_err().reason, "PLACE_NOT_FOUND");
    let error = ctx
        .places::<_, DeletePlaceResponse>(
            "DeletePlace",
            &aoi,
            &DeletePlaceRequest { place_id: place.id.clone(), ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!(error.reason, "PLACE_NOT_FOUND");
}

#[tokio::test]
async fn a_search_reindex_publishes_every_public_place_again() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    let spot = somewhere();
    let public = create(&ctx, &aoi, new_place("展望台", spot, Visibility::VISIBILITY_PUBLIC)).await;
    let club = create(&ctx, &aoi, new_place("部室", offset(spot, 0.0, 500.0), Visibility::VISIBILITY_FRIENDS)).await;
    let published = async |place_id: &str| {
        let events = ctx.app.events("search.changed").await;
        events.into_iter().filter(|event| event.data["id"] == serde_json::json!(place_id)).count()
    };
    assert_eq!((published(&public.id).await, published(&club.id).await), (1, 0));

    // An administrator rebuilds another kind: not ours to answer.
    let admin = Uuid::now_v7();
    ctx.deliver("search.reindex.requested", admin, serde_json::json!({ "kind": "video", "requested_by": admin })).await;
    assert_eq!(published(&public.id).await, 1);
    // Places: every public one is published again, and only those.
    ctx.deliver("search.reindex.requested", admin, serde_json::json!({ "kind": "place", "requested_by": admin })).await;
    assert_eq!((published(&public.id).await, published(&club.id).await), (2, 0));
    let again = search_change(&ctx, &public.id).await.unwrap();
    assert_eq!((again.data["kind"].as_str(), again.data["doc"]["title"].as_str()), (Some("place"), Some("展望台")));
}

#[tokio::test]
async fn place_requests_are_validated() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    let spot = somewhere();
    let ok = || new_place("公園", spot, Visibility::VISIBILITY_PRIVATE);
    let reason = async |request: CreatePlaceRequest| {
        ctx.places::<_, CreatePlaceResponse>("CreatePlace", &aoi, &request).await.unwrap_err().reason
    };

    assert_eq!(reason(CreatePlaceRequest { name: "  ".into(), ..ok() }).await, "INVALID_NAME");
    assert_eq!(reason(CreatePlaceRequest { name: "あ".repeat(61), ..ok() }).await, "INVALID_NAME");
    assert_eq!(reason(CreatePlaceRequest { emoji: "cafe".into(), ..ok() }).await, "INVALID_EMOJI");
    assert_eq!(reason(CreatePlaceRequest { center: Default::default(), ..ok() }).await, "INVALID_LOCATION");
    assert_eq!(reason(CreatePlaceRequest { center: lat_lng((100.0, 0.0)).into(), ..ok() }).await, "INVALID_LOCATION");
    assert_eq!(reason(CreatePlaceRequest { radius_meters: 10.0, ..ok() }).await, "INVALID_RADIUS");
    assert_eq!(reason(CreatePlaceRequest { radius_meters: 5000.0, ..ok() }).await, "INVALID_RADIUS");
    assert_eq!(
        reason(CreatePlaceRequest { label: tokuly_proto::buffa::EnumValue::from(99), ..ok() }).await,
        "INVALID_LABEL"
    );
    assert_eq!(
        reason(CreatePlaceRequest { visibility: Visibility::VISIBILITY_UNLISTED.into(), ..ok() }).await,
        "INVALID_VISIBILITY"
    );
    assert_eq!(reason(CreatePlaceRequest { category: "x".repeat(33), ..ok() }).await, "INVALID_CATEGORY");
    assert_eq!(
        reason(CreatePlaceRequest { photo_asset_id: Uuid::now_v7().to_string(), ..ok() }).await,
        "ASSET_NOT_FOUND"
    );
    let avatar = ready_image(&ctx.app.core, aoi.id, AssetPurpose::ASSET_PURPOSE_AVATAR).await.unwrap();
    assert_eq!(
        reason(CreatePlaceRequest { photo_asset_id: avatar.id.to_string(), ..ok() }).await,
        "ASSET_WRONG_PURPOSE"
    );
    let rens = ready_image(&ctx.app.core, ren.id, AssetPurpose::ASSET_PURPOSE_PLACE_PHOTO).await.unwrap();
    assert_eq!(reason(CreatePlaceRequest { photo_asset_id: rens.id.to_string(), ..ok() }).await, "ASSET_NOT_FOUND");
    assert!(my_places(&ctx, &aoi).await.is_empty(), "nothing was created by the failed attempts");

    // Home is private whatever was asked, and there is only one.
    let home = create(
        &ctx,
        &aoi,
        CreatePlaceRequest {
            label: PlaceLabel::PLACE_LABEL_HOME.into(),
            ..new_place("うち", spot, Visibility::VISIBILITY_PUBLIC)
        },
    )
    .await;
    assert_eq!(home.visibility, Visibility::VISIBILITY_PRIVATE);
    assert!(search_change(&ctx, &home.id).await.is_none(), "a private place is never indexed");
    let second = ctx
        .places::<_, CreatePlaceResponse>(
            "CreatePlace",
            &aoi,
            &CreatePlaceRequest { label: PlaceLabel::PLACE_LABEL_HOME.into(), ..ok() },
        )
        .await
        .unwrap_err();
    assert_eq!((second.code.as_str(), second.reason.as_str()), ("already_exists", "LABEL_TAKEN"));
    // Turning another place into a second home fails the same way; making home public is ignored.
    let park = create(&ctx, &aoi, ok()).await;
    let relabel = UpdatePlaceRequest {
        place_id: park.id.clone(),
        label: Some(PlaceLabel::PLACE_LABEL_HOME.into()),
        ..Default::default()
    };
    assert_eq!(
        ctx.places::<_, UpdatePlaceResponse>("UpdatePlace", &aoi, &relabel).await.unwrap_err().reason,
        "LABEL_TAKEN"
    );
    let publish = UpdatePlaceRequest {
        place_id: home.id.clone(),
        visibility: Some(Visibility::VISIBILITY_PUBLIC.into()),
        ..Default::default()
    };
    let still_private: UpdatePlaceResponse = ctx.places("UpdatePlace", &aoi, &publish).await.unwrap();
    assert_eq!(still_private.place.as_option().unwrap().visibility, Visibility::VISIBILITY_PRIVATE);
    let bad = UpdatePlaceRequest { place_id: park.id.clone(), radius_meters: Some(1.0), ..Default::default() };
    assert_eq!(
        ctx.places::<_, UpdatePlaceResponse>("UpdatePlace", &aoi, &bad).await.unwrap_err().reason,
        "INVALID_RADIUS"
    );
    let bad_id = UpdatePlaceRequest { place_id: "nope".into(), ..Default::default() };
    assert_eq!(
        ctx.places::<_, UpdatePlaceResponse>("UpdatePlace", &aoi, &bad_id).await.unwrap_err().reason,
        "INVALID_ID"
    );
    // Home, work and school come first in the list.
    assert_eq!(my_places(&ctx, &aoi).await.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), vec!["うち", "公園"]);

    let anonymous = ctx
        .app
        .call::<_, ListMyPlacesResponse>(
            "tokuly.geo.v1.PlaceService/ListMyPlaces",
            None,
            &ListMyPlacesRequest::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(anonymous.reason, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn place_cards_follow_visibility_and_name_only_friends_the_viewer_may_see() {
    let ctx = start().await;
    let (aoi, ren, stranger) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("stranger").await);
    ctx.befriend(&aoi, &ren).await;
    let spot = somewhere();
    let private = create(&ctx, &aoi, new_place("ひみつ基地", spot, Visibility::VISIBILITY_PRIVATE)).await;
    let shared =
        create(&ctx, &aoi, new_place("たまり場", offset(spot, 0.0, 2000.0), Visibility::VISIBILITY_FRIENDS)).await;
    let public =
        create(&ctx, &aoi, new_place("カフェ", offset(spot, 0.0, 4000.0), Visibility::VISIBILITY_PUBLIC)).await;

    assert!(card(&ctx, &aoi, &private.id).await.is_ok());
    assert_eq!(card(&ctx, &ren, &private.id).await.unwrap_err().reason, "PLACE_NOT_FOUND");
    assert!(card(&ctx, &ren, &shared.id).await.is_ok());
    assert_eq!(card(&ctx, &stranger, &shared.id).await.unwrap_err().reason, "PLACE_NOT_FOUND");
    assert!(card(&ctx, &stranger, &public.id).await.is_ok());
    assert_eq!(card(&ctx, &ren, "nope").await.unwrap_err().reason, "INVALID_ID");
    assert_eq!(card(&ctx, &ren, &Uuid::now_v7().to_string()).await.unwrap_err().reason, "PLACE_NOT_FOUND");

    // Ren becomes a regular of the café.
    for _ in 0..3 {
        check_in(&ctx, &ren, CheckInRequest { r#where: Some(Where::PlaceId(public.id.clone())), ..Default::default() })
            .await
            .unwrap();
    }
    let owners = card(&ctx, &aoi, &public.id).await.unwrap();
    assert_eq!(owners.total_visit_count, 3);
    assert_eq!(owners.regulars.len(), 1);
    assert_eq!(
        (
            owners.regulars[0].user.as_option().unwrap().id.clone(),
            owners.regulars[0].visit_count,
            owners.regulars[0].rank
        ),
        (ren.id.to_string(), 3, 1)
    );
    assert_eq!(owners.friends_visited.iter().map(|user| user.id.clone()).collect::<Vec<_>>(), vec![ren.id.to_string()]);
    let rens = card(&ctx, &ren, &public.id).await.unwrap();
    assert_eq!(rens.place.as_option().unwrap().my_visit_count, 3);
    assert!(rens.place.as_option().unwrap().my_last_visit_at.as_option().is_some());
    assert_eq!(rens.regulars.len(), 1, "one's own visits count");
    assert!(rens.friends_visited.is_empty());
    // A stranger gets the total, but no names: Ren is not their friend.
    let strangers = card(&ctx, &stranger, &public.id).await.unwrap();
    assert_eq!((strangers.total_visit_count, strangers.regulars.len(), strangers.friends_visited.len()), (3, 0, 0));
    // Under ghost mode Ren's whereabouts, past ones included, are not shown to Aoi either.
    ctx.ghost(&ren, &[&aoi], GhostMode::GHOST_MODE_BLURRED, 0).await.unwrap();
    let owners = card(&ctx, &aoi, &public.id).await.unwrap();
    assert!(owners.regulars.is_empty() && owners.friends_visited.is_empty());
    assert_eq!(owners.total_visit_count, 3);
}

#[tokio::test]
async fn saved_places_follow_what_stays_visible() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let spot = somewhere();
    let shared = create(&ctx, &aoi, new_place("たまり場", spot, Visibility::VISIBILITY_FRIENDS)).await;
    let public =
        create(&ctx, &aoi, new_place("カフェ", offset(spot, 0.0, 3000.0), Visibility::VISIBILITY_PUBLIC)).await;
    let private =
        create(&ctx, &aoi, new_place("ひみつ", offset(spot, 0.0, 6000.0), Visibility::VISIBILITY_PRIVATE)).await;
    let save = |user: &TestUser, place_id: &str| {
        let (user, request) = (user.clone(), SavePlaceRequest { place_id: place_id.to_owned(), ..Default::default() });
        let ctx = &ctx;
        async move { ctx.places::<_, SavePlaceResponse>("SavePlace", &user, &request).await }
    };
    let saved = |limit: i32, cursor: String| {
        let request = ListSavedPlacesRequest {
            page: PageRequest { limit, cursor, ..Default::default() }.into(),
            ..Default::default()
        };
        let ctx = &ctx;
        let ren = ren.clone();
        async move { ctx.places::<_, ListSavedPlacesResponse>("ListSavedPlaces", &ren, &request).await.unwrap() }
    };

    save(&ren, &shared.id).await.unwrap();
    save(&ren, &public.id).await.unwrap();
    save(&ren, &public.id).await.unwrap(); // saving twice is fine
    assert_eq!(save(&ren, &private.id).await.unwrap_err().reason, "PLACE_NOT_FOUND");
    assert_eq!(save(&ren, "x").await.unwrap_err().reason, "INVALID_ID");

    // Newest first, paged.
    let first = saved(1, String::new()).await;
    assert_eq!(first.places.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec![public.id.as_str()]);
    assert!(first.places[0].saved && first.page.as_option().unwrap().has_more);
    let second = saved(1, first.page.as_option().unwrap().next_cursor.clone()).await;
    assert_eq!(second.places.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec![shared.id.as_str()]);
    assert!(!second.page.as_option().unwrap().has_more);
    assert!(card(&ctx, &ren, &public.id).await.unwrap().place.as_option().unwrap().saved);

    // No longer friends: the friends-only place drops out, the public one stays.
    ctx.unfriend(&aoi, &ren).await;
    assert_eq!(
        saved(10, String::new()).await.places.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec![public.id.as_str()]
    );
    let _: UnsavePlaceResponse = ctx
        .places("UnsavePlace", &ren, &UnsavePlaceRequest { place_id: public.id.clone(), ..Default::default() })
        .await
        .unwrap();
    let _: UnsavePlaceResponse = ctx
        .places("UnsavePlace", &ren, &UnsavePlaceRequest { place_id: public.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert!(saved(10, String::new()).await.places.is_empty());
}

#[tokio::test]
async fn search_finds_visible_places_and_falls_back_to_the_gazetteer() {
    let ctx = start().await;
    let (aoi, ren, other) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("other").await);
    ctx.befriend(&aoi, &ren).await;
    let spot = somewhere();
    // A word no other test (or town) uses.
    let word = format!("ぽよ{}", &Uuid::now_v7().simple().to_string()[20..]);
    let named = |suffix: &str| format!("{word}{suffix}");
    let own = create(&ctx, &aoi, new_place(&named("の自分用"), spot, Visibility::VISIBILITY_PRIVATE)).await;
    let friends =
        create(&ctx, &ren, new_place(&named("の友達用"), offset(spot, 0.0, 800.0), Visibility::VISIBILITY_FRIENDS))
            .await;
    let public =
        create(&ctx, &other, new_place(&named("の公開"), offset(spot, 0.0, 1600.0), Visibility::VISIBILITY_PUBLIC))
            .await;
    create(&ctx, &other, new_place(&named("の秘密"), spot, Visibility::VISIBILITY_PRIVATE)).await;
    create(&ctx, &other, new_place(&named("の内輪"), spot, Visibility::VISIBILITY_FRIENDS)).await;
    check_in(&ctx, &ren, CheckInRequest { r#where: Some(Where::PlaceId(public.id.clone())), ..Default::default() })
        .await
        .unwrap();

    let search = |query: &str, near: Option<Point>| {
        let request =
            SearchPlacesRequest { query: query.to_owned(), near: near.map(lat_lng).into(), ..Default::default() };
        let ctx = &ctx;
        let aoi = aoi.clone();
        async move { ctx.places::<_, SearchPlacesResponse>("SearchPlaces", &aoi, &request).await }
    };
    let found = search(&word, Some(spot)).await.unwrap().results;
    let ids: Vec<&str> =
        found.iter().filter_map(|result| result.place.as_option().map(|place| place.id.as_str())).collect();
    assert_eq!(
        ids,
        vec![own.id.as_str(), friends.id.as_str(), public.id.as_str()],
        "own first, then by distance; private and friends-only places of others stay hidden"
    );
    assert_eq!(found[0].distance_meters.map(f64::round), Some(0.0));
    assert!((found[1].distance_meters.unwrap() - 800.0).abs() < 5.0);
    assert_eq!(
        found[2].friends_visited.iter().map(|user| user.id.clone()).collect::<Vec<_>>(),
        vec![ren.id.to_string()]
    );
    assert_eq!(found[2].name, named("の公開"));

    // An empty query with a point lists what is around.
    let around = search("", Some(spot)).await.unwrap().results;
    let ids: Vec<&str> =
        around.iter().filter_map(|result| result.place.as_option().map(|place| place.id.as_str())).collect();
    assert!(ids.contains(&own.id.as_str()) && ids.contains(&public.id.as_str()));
    assert!(search("", None).await.unwrap().results.is_empty());

    // Place names from the bundled gazetteer, in the account's language.
    let towns = search("渋谷", Some((35.66, 139.70))).await.unwrap().results;
    let town = towns.iter().find(|result| result.place.as_option().is_none()).expect("a gazetteer hit");
    assert!(town.address.contains("東京都"), "{}", town.address);
    assert!(distance(point(town.position.as_option()), (35.66, 139.70)) < 5000.0);
    assert!(town.distance_meters.is_some());
    let paris = search("Paris", None).await.unwrap().results;
    assert!(paris.iter().any(|result| distance(point(result.position.as_option()), (48.8534, 2.3488)) < 1000.0));
    assert!(paris.iter().all(|result| result.distance_meters.is_none()));

    assert_eq!(search(&"あ".repeat(101), None).await.unwrap_err().reason, "QUERY_TOO_LONG");
    assert_eq!(search("x", Some((0.0, 500.0))).await.unwrap_err().reason, "INVALID_LOCATION");
}

#[tokio::test]
async fn nights_at_the_same_spot_are_suggested_as_home() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    // Three nights in Tokyo, the latest ending yesterday morning (Japan time).
    let home =
        offset((35.70, 139.60), rand::random::<u16>() as f64 % 360.0, 200.0 + rand::random::<u16>() as f64 % 4000.0);
    let tokyo = chrono_tz::Asia::Tokyo;
    let today = Utc::now().with_timezone(&tokyo).date_naive();
    let mut samples = Vec::new();
    for days_ago in [4, 3, 2] {
        let evening = tokyo
            .from_local_datetime(&(today - Duration::days(days_ago)).and_hms_opt(21, 0, 0).unwrap())
            .unwrap()
            .with_timezone(&Utc);
        samples.extend((0..=33).map(|i| sample(home, evening + Duration::minutes(i * 20))));
        // Out for the day.
        samples.push(sample(offset(home, 90.0, 3000.0), evening + Duration::hours(11) + Duration::minutes(30)));
        samples.push(sample(offset(home, 90.0, 3400.0), evening + Duration::hours(11) + Duration::minutes(32)));
    }
    let sent = samples.len() as i32;
    assert_eq!(ctx.report(&aoi, samples).await.accepted_count, sent);

    let suggestions: ListPlaceSuggestionsResponse =
        ctx.places("ListPlaceSuggestions", &aoi, &ListPlaceSuggestionsRequest::default()).await.unwrap();
    assert_eq!(suggestions.suggestions.len(), 1);
    let suggestion = &suggestions.suggestions[0];
    assert_eq!(suggestion.suggested_label, PlaceLabel::PLACE_LABEL_HOME);
    assert_eq!(suggestion.suggested_name, "自宅");
    assert_eq!((suggestion.visit_count, suggestion.total_seconds), (3, 3 * 11 * 3600));
    assert!(distance(point(suggestion.center.as_option()), home) < 20.0);

    // Accepting it: detected rather than drawn by hand, and the past stays become its visits.
    let request = CreatePlaceRequest {
        label: PlaceLabel::PLACE_LABEL_HOME.into(),
        ..new_place("うち", point(suggestion.center.as_option()), Visibility::VISIBILITY_PRIVATE)
    };
    let place = create(&ctx, &aoi, request).await;
    assert!(place.auto_detected);
    assert_eq!(place.my_visit_count, 3);
    assert!(!place.address.is_empty() && !place.address.is_ascii(), "{:?}", place.address);
    let nights: ListNightsResponse = ctx.history("ListNights", &aoi, &ListNightsRequest::default()).await.unwrap();
    assert_eq!(nights.nights.len(), 3);
    assert!(nights.nights.iter().all(|night| night.at_home), "the nights are nights at home now");
    assert!(nights.nights[0].date > nights.nights[2].date, "newest first");
    let suggestions: ListPlaceSuggestionsResponse =
        ctx.places("ListPlaceSuggestions", &aoi, &ListPlaceSuggestionsRequest::default()).await.unwrap();
    assert!(suggestions.suggestions.is_empty());

    // A place drawn somewhere else is not "detected".
    let elsewhere = create(
        &ctx,
        &aoi,
        CreatePlaceRequest {
            label: PlaceLabel::PLACE_LABEL_WORK.into(),
            ..new_place("職場", offset(home, 0.0, 5000.0), Visibility::VISIBILITY_PRIVATE)
        },
    )
    .await;
    assert!(!elsewhere.auto_detected);
}

#[tokio::test]
async fn reverse_geocoding_works_offline_in_both_languages() {
    let ctx = start().await;
    let aoi = ctx.app.user("aoi").await;
    let lookup = |position: Point, language: &str| {
        let request = ReverseGeocodeRequest {
            position: lat_lng(position).into(),
            language: language.to_owned(),
            ..Default::default()
        };
        let ctx = &ctx;
        let aoi = aoi.clone();
        async move {
            ctx.places::<_, ReverseGeocodeResponse>("ReverseGeocode", &aoi, &request)
                .await
                .map(|response| response.result.into_option().unwrap())
        }
    };
    let shibuya = lookup((35.6595, 139.7005), "").await.unwrap();
    assert_eq!(
        (shibuya.name.as_str(), shibuya.region.as_str(), shibuya.country.as_str()),
        ("渋谷区", "東京都", "日本"),
        "the account's language by default"
    );
    assert_eq!((shibuya.country_code.as_str(), shibuya.time_zone.as_str()), ("JP", "Asia/Tokyo"));
    let english = lookup((35.6595, 139.7005), "en-US").await.unwrap();
    assert_eq!(
        (english.name.as_str(), english.region.as_str(), english.country.as_str()),
        ("Shibuya", "Tokyo", "Japan")
    );
    let new_york = lookup((40.7128, -74.0060), "en").await.unwrap();
    assert_eq!(
        (new_york.name.as_str(), new_york.country_code.as_str(), new_york.time_zone.as_str()),
        ("New York City", "US", "America/New_York")
    );
    let ocean = lookup((0.0, -140.0), "ja").await.unwrap();
    assert!(ocean.name.is_empty() && ocean.country_code.is_empty() && ocean.time_zone.starts_with("Etc/GMT"));
    assert_eq!(lookup((0.0, 200.0), "").await.unwrap_err().reason, "INVALID_LOCATION");
    let missing = ctx
        .places::<_, ReverseGeocodeResponse>("ReverseGeocode", &aoi, &ReverseGeocodeRequest::default())
        .await
        .unwrap_err();
    assert_eq!(missing.reason, "INVALID_LOCATION");
}

#[tokio::test]
async fn a_check_in_tells_friends_who_see_the_pin() {
    let ctx = start().await;
    let (aoi, ren, mio, stranger) = (
        ctx.app.user("aoi").await,
        ctx.app.user("ren").await,
        ctx.app.user("mio").await,
        ctx.app.user("stranger").await,
    );
    ctx.befriend(&aoi, &ren).await;
    ctx.befriend(&aoi, &mio).await;
    let spot = somewhere();
    let cafe = create(&ctx, &aoi, new_place("カフェ モカ", spot, Visibility::VISIBILITY_PUBLIC)).await;
    let mut mios = ctx.app.listen_user(mio.id).await;

    let request = CheckInRequest {
        r#where: Some(Where::PlaceId(cafe.id.clone())),
        message: "  来たよ!  ".into(),
        with_user_ids: vec![ren.id.to_string()],
        ..Default::default()
    };
    let visit = check_in(&ctx, &aoi, request).await.unwrap();
    assert!(visit.check_in);
    assert_eq!((visit.message.as_str(), visit.name.as_str()), ("来たよ!", "カフェ モカ"));
    assert_eq!(visit.place.as_option().unwrap().id, cafe.id);
    assert_eq!(visit.with_friends.iter().map(|user| user.id.clone()).collect::<Vec<_>>(), vec![ren.id.to_string()]);
    assert!(visit.departed_at.as_option().is_some(), "a check-in on its own is a moment, not a stay");

    // The tagged friend, the other friends, the activity feed and the map all hear of it.
    let tagged = ctx.notifications_for(ren.id, "geo.checkin_tagged").await;
    assert_eq!(tagged.len(), 1);
    assert!(tagged[0].data["title"]["ja"].as_str().unwrap().contains("カフェ モカ"));
    assert_eq!(tagged[0].data["body"]["ja"], "来たよ!");
    assert_eq!(ctx.notifications_for(mio.id, "geo.checkin").await.len(), 1);
    assert!(ctx.notifications_for(ren.id, "geo.checkin").await.is_empty(), "the tagged friend is told once");
    assert!(ctx.notifications_for(stranger.id, "geo.checkin").await.is_empty());
    let activity = ctx.activities_of(aoi.id, "geo.visit").await;
    assert_eq!(activity.len(), 1);
    assert_eq!(activity[0].data["audience"], "friends");
    assert_eq!(activity[0].data["data"]["place_name"], "カフェ モカ", "the name the feed's recaps show");
    assert_eq!(activity[0].data["with_user_ids"], serde_json::json!([ren.id]));
    assert_eq!(activity[0].data["subject"], serde_json::json!([SubjectType::SUBJECT_TYPE_PLACE as i32, cafe.id]));
    assert!(activity[0].data["location"].is_array());
    let event = pushes(&mut mios, QUIET)
        .await
        .into_iter()
        .find_map(|push| if let Push::Visit(visit) = push { Some(*visit) } else { None })
        .expect("a visit event on the map");
    assert_eq!(
        (event.user_id.clone(), event.arrived, event.place_name.as_str()),
        (aoi.id.to_string(), true, "カフェ モカ")
    );

    // Validation.
    let reason = async |request: CheckInRequest| check_in(&ctx, &aoi, request).await.unwrap_err().reason;
    let here = || Some(Where::Position(Box::new(lat_lng(spot))));
    assert_eq!(reason(CheckInRequest::default()).await, "MISSING_TARGET");
    assert_eq!(
        reason(CheckInRequest { r#where: Some(Where::PlaceId(Uuid::now_v7().to_string())), ..Default::default() })
            .await,
        "PLACE_NOT_FOUND"
    );
    assert_eq!(
        reason(CheckInRequest {
            r#where: Some(Where::Position(Box::new(lat_lng((0.0, 999.0))))),
            ..Default::default()
        })
        .await,
        "INVALID_LOCATION"
    );
    assert_eq!(
        reason(CheckInRequest { r#where: here(), message: "あ".repeat(201), ..Default::default() }).await,
        "MESSAGE_TOO_LONG"
    );
    assert_eq!(
        reason(CheckInRequest { r#where: here(), with_user_ids: vec![stranger.id.to_string()], ..Default::default() })
            .await,
        "NOT_FRIENDS"
    );
    assert_eq!(
        reason(CheckInRequest {
            r#where: here(),
            with_user_ids: (0..21).map(|_| Uuid::now_v7().to_string()).collect(),
            ..Default::default()
        })
        .await,
        "TOO_MANY_USERS"
    );

    // Under ghost mode a check-in stays between the user and the people tagged.
    ctx.ghost(&aoi, &[], GhostMode::GHOST_MODE_FROZEN, 0).await.unwrap();
    check_in(
        &ctx,
        &aoi,
        CheckInRequest { r#where: here(), with_user_ids: vec![ren.id.to_string()], ..Default::default() },
    )
    .await
    .unwrap();
    assert_eq!(ctx.notifications_for(mio.id, "geo.checkin").await.len(), 1, "no new notification");
    assert_eq!(ctx.notifications_for(ren.id, "geo.checkin_tagged").await.len(), 2);
    let activity = ctx.activities_of(aoi.id, "geo.visit").await;
    assert_eq!(activity[1].data["audience"], "private");
    assert!(activity[1].data["location"].is_null());
    assert!(pushes(&mut mios, QUIET).await.iter().all(|push| !matches!(push, Push::Visit(_))));
}

#[tokio::test]
async fn a_check_in_during_a_stay_belongs_to_that_visit() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let spot = somewhere();
    ctx.stay(&aoi, spot, minutes_ago(8), seconds_ago(5)).await;
    let ongoing: ListVisitsResponse = ctx.history("ListVisits", &aoi, &ListVisitsRequest::default()).await.unwrap();
    assert_eq!(ongoing.visits.len(), 1);

    let request = CheckInRequest {
        r#where: Some(Where::Position(Box::new(lat_lng(offset(spot, 0.0, 20.0))))),
        message: "ここ!".into(),
        with_user_ids: vec![ren.id.to_string()],
        ..Default::default()
    };
    let visit = check_in(&ctx, &aoi, request).await.unwrap();
    assert_eq!(visit.id, ongoing.visits[0].id, "the same visit, now marked as a check-in");
    assert!(visit.check_in && visit.departed_at.as_option().is_none());
    assert_eq!(visit.with_friends.len(), 1);
    let after: ListVisitsResponse = ctx.history("ListVisits", &aoi, &ListVisitsRequest::default()).await.unwrap();
    assert_eq!(after.visits.len(), 1);

    // Somewhere else entirely: a visit of its own.
    let far = CheckInRequest {
        r#where: Some(Where::Position(Box::new(lat_lng(offset(spot, 0.0, 9000.0))))),
        ..Default::default()
    };
    assert_ne!(check_in(&ctx, &aoi, far).await.unwrap().id, visit.id);
}

#[tokio::test]
async fn places_are_subjects_and_can_be_taken_down() {
    let ctx = start().await;
    let (aoi, ren, stranger) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("stranger").await);
    ctx.befriend(&aoi, &ren).await;
    let spot = somewhere();
    let photo = ready_image(&ctx.app.core, aoi.id, AssetPurpose::ASSET_PURPOSE_PLACE_PHOTO).await.unwrap();
    let public = create(
        &ctx,
        &aoi,
        CreatePlaceRequest {
            photo_asset_id: photo.id.to_string(),
            ..new_place("カフェ", spot, Visibility::VISIBILITY_PUBLIC)
        },
    )
    .await;
    let shared =
        create(&ctx, &aoi, new_place("たまり場", offset(spot, 0.0, 2000.0), Visibility::VISIBILITY_FRIENDS)).await;
    let subjects = &ctx.app.core.subjects;
    let place = SubjectType::SUBJECT_TYPE_PLACE;
    assert!(subjects.is_registered(place));

    // What comments, reactions and reports resolve.
    let info = subjects.resolve(place, &public.id, Some(stranger.id)).await.unwrap();
    assert_eq!(
        (info.owner_id, info.title.as_str(), info.path.as_str()),
        (aoi.id, "カフェ", format!("/places/{}", public.id).as_str())
    );
    assert!(info.interactions_enabled && info.image_url.is_some());
    assert!(subjects.resolve(place, &public.id, None).await.is_ok(), "public places are public");
    assert!(subjects.resolve(place, &shared.id, Some(ren.id)).await.is_ok());
    for viewer in [Some(stranger.id), None] {
        assert_eq!(subjects.resolve(place, &shared.id, viewer).await.unwrap_err().reason, "SUBJECT_NOT_FOUND");
    }
    assert_eq!(subjects.resolve(place, "not-a-uuid", Some(aoi.id)).await.unwrap_err().reason, "SUBJECT_NOT_FOUND");

    // The photo is deleted by the media domain (a takedown): the place loses it.
    ctx.deliver("media.asset.deleted", aoi.id, serde_json::json!({ "asset_id": photo.id, "owner_id": aoi.id, "purpose": AssetPurpose::ASSET_PURPOSE_PLACE_PHOTO as i32, "kind": 1 })).await;
    assert!(card(&ctx, &aoi, &public.id).await.unwrap().place.as_option().unwrap().photo.as_option().is_none());

    // A moderator removes the place.
    let removed = serde_json::json!({ "subject_type": place as i32, "subject_id": public.id, "reason": "spam", "moderator_id": Uuid::now_v7() });
    ctx.deliver("admin.content.removed", Uuid::now_v7(), removed.clone()).await;
    assert_eq!(card(&ctx, &aoi, &public.id).await.unwrap_err().reason, "PLACE_NOT_FOUND");
    assert!(search_change(&ctx, &public.id).await.unwrap().data["doc"].is_null(), "and it leaves the search index");
    // Delivered twice, or for something that is not a place: nothing happens.
    ctx.deliver("admin.content.removed", Uuid::now_v7(), removed).await;
    ctx.deliver("admin.content.removed", Uuid::now_v7(), serde_json::json!({ "subject_type": SubjectType::SUBJECT_TYPE_NOTE as i32, "subject_id": shared.id, "reason": "", "moderator_id": Uuid::now_v7() })).await;
    assert!(card(&ctx, &aoi, &shared.id).await.is_ok());
}
