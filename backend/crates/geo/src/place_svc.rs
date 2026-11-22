//! `PlaceService`: the caller's places, place cards, search, suggestions,
//! saved places, reverse geocoding and check-ins.
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use chrono::{DateTime, Utc};
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{
    outbound::{self, ActivityAudience, ActivityEntry, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_media::assets;
use tokuly_proto::{
    common::SubjectType,
    geo::{check_in_request::Where, *},
};
use tokuly_social::graph;

use crate::{
    Geo,
    fanout::VisitNews,
    geocode::Gazetteer,
    math::{self, Point},
    places::{self, FRIENDS, LABEL_CUSTOM, MAX_PLACES, PLACE_COLUMNS, PRIVATE, PUBLIC, PlaceRow, photo_holder},
    suggest,
    visibility::Audience,
    visits::{VISIT_COLUMNS, VisitRow},
};

const MAX_SAVED: i64 = 500;
const MAX_QUERY_CHARS: usize = 100;
const MAX_MESSAGE_CHARS: usize = 200;
const MAX_TAGGED: usize = 20;
/// Friends told about a check-in at most (besides the tagged ones).
const MAX_CHECK_IN_AUDIENCE: usize = 200;
/// A check-in this close to the ongoing stay belongs to it.
const SAME_SPOT_M: f64 = 150.0;
/// Unmatched stays this far back feed the suggestions.
const SUGGEST_DAYS: i32 = 60;
/// A new home/work/school place this close to a suggestion of the same kind counts as detected from it.
const SUGGESTION_MATCH_M: f64 = 150.0;
/// An empty search with a point lists places this close.
const NEARBY_M: f64 = 5_000.0;

/// A hit of an external Photon geocoder.
#[derive(Debug, Clone, PartialEq)]
struct PhotonHit {
    name: String,
    locality: String,
    region: String,
    country: String,
    country_code: String,
    position: Point,
}

impl PhotonHit {
    fn address(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        for part in [self.locality.as_str(), self.region.as_str(), self.country.as_str()] {
            if !part.is_empty() && part != self.name && !parts.contains(&part) {
                parts.push(part);
            }
        }
        parts.join(", ")
    }
}

/// Reads the features of a Photon (GeoJSON) answer.
fn parse_photon(body: &serde_json::Value) -> Vec<PhotonHit> {
    let text = |properties: &serde_json::Value, key: &str| {
        properties.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_owned()
    };
    body.get("features")
        .and_then(|features| features.as_array())
        .into_iter()
        .flatten()
        .filter_map(|feature| {
            let coordinates = feature.pointer("/geometry/coordinates")?.as_array()?;
            let (lng, lat) = (coordinates.first()?.as_f64()?, coordinates.get(1)?.as_f64()?);
            let properties = feature.get("properties")?;
            let first_of = |keys: &[&str]| {
                keys.iter().map(|key| text(properties, key)).find(|value| !value.is_empty()).unwrap_or_default()
            };
            let locality = first_of(&["city", "district", "locality", "county"]);
            let name = first_of(&["name", "district", "city", "county", "state"]);
            (math::valid_coordinates(lat, lng) && !name.is_empty()).then(|| PhotonHit {
                name,
                locality,
                region: text(properties, "state"),
                country: text(properties, "country"),
                country_code: text(properties, "countrycode").to_ascii_uppercase(),
                position: (lat, lng),
            })
        })
        .collect()
}

fn suggestion_name(label: PlaceLabel, locality: &str, ja: bool) -> String {
    match (label, ja) {
        (PlaceLabel::PLACE_LABEL_HOME, true) => "自宅".to_owned(),
        (PlaceLabel::PLACE_LABEL_HOME, false) => "Home".to_owned(),
        (PlaceLabel::PLACE_LABEL_WORK, true) => "職場".to_owned(),
        (PlaceLabel::PLACE_LABEL_WORK, false) => "Work".to_owned(),
        (PlaceLabel::PLACE_LABEL_SCHOOL, true) => "学校".to_owned(),
        (PlaceLabel::PLACE_LABEL_SCHOOL, false) => "School".to_owned(),
        (_, true) if locality.is_empty() => "よく行く場所".to_owned(),
        (_, false) if locality.is_empty() => "Frequent spot".to_owned(),
        (_, true) => format!("{locality}のよく行く場所"),
        (_, false) => format!("Frequent spot in {locality}"),
    }
}

impl Geo {
    /// Asks the external geocoder (when configured). `None` on any failure:
    /// callers fall back to the bundled gazetteer.
    async fn photon(&self, path: &str, query: &[(&str, String)], ja: bool) -> Option<Vec<PhotonHit>> {
        if self.cfg.geocoder_url.is_empty() {
            return None;
        }
        let url = format!("{}/{path}", self.cfg.geocoder_url.trim_end_matches('/'));
        let mut request = self.core.http.get(url).query(query).timeout(Duration::from_secs(4));
        // Photon's default language is the local one, which is what Japanese users want in Japan.
        if !ja {
            request = request.query(&[("lang", "en")]);
        }
        let answer: Result<serde_json::Value, reqwest::Error> =
            async { request.send().await?.error_for_status()?.json().await }.await;
        match answer {
            Ok(body) => Some(parse_photon(&body)),
            Err(e) => {
                tracing::warn!(error = %e, "external geocoder failed; using the bundled gazetteer");
                None
            }
        }
    }

    /// The caller's suggestions: frequent stays without a place.
    async fn place_suggestions(&self, user_id: Uuid) -> AppResult<Vec<suggest::Suggestion>> {
        let stays: Vec<(f64, f64, DateTime<Utc>, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT lat, lng, arrived_at, departed_at FROM geo_visits
             WHERE user_id = $1 AND place_id IS NULL AND NOT check_in AND arrived_at > now() - make_interval(days => $2)
             ORDER BY arrived_at LIMIT 2000",
        )
        .bind(user_id)
        .bind(SUGGEST_DAYS)
        .fetch_all(&self.core.db)
        .await?;
        if stays.is_empty() {
            return Ok(Vec::new());
        }
        let now = Utc::now();
        let stays: Vec<suggest::Stay> = stays
            .into_iter()
            .map(|(lat, lng, arrived, departed)| suggest::Stay {
                position: (lat, lng),
                arrived,
                departed: departed.unwrap_or(now),
            })
            .collect();
        let ctx = self.ctx_of(user_id).await?;
        // Students get "school" where others get "work".
        let (age, school): (Option<i32>, String) =
            sqlx::query_as("SELECT date_part('year', age(birthdate))::int, school_name FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_optional(&self.core.db)
                .await?
                .unwrap_or_default();
        let rules = suggest::Rules {
            has_home: ctx.labelled_place(PlaceLabel::PLACE_LABEL_HOME as i32).is_some(),
            has_work: ctx.labelled_place(PlaceLabel::PLACE_LABEL_WORK as i32).is_some(),
            has_school: ctx.labelled_place(PlaceLabel::PLACE_LABEL_SCHOOL as i32).is_some(),
            student: age.is_some_and(|age| age < 19) || (!school.is_empty() && age.is_none_or(|age| age < 23)),
            home_nights: self.cfg.home_nights as usize,
        };
        Ok(suggest::suggestions(&stays, rules, Geo::zone_at, 10))
    }

    /// Friends of the viewer who have been to each place, as far as the viewer saw them there.
    async fn friends_visited(
        &self,
        viewer: Uuid,
        place_ids: &[Uuid],
    ) -> AppResult<HashMap<Uuid, Vec<tokuly_proto::common::UserSummary>>> {
        let mut out: HashMap<Uuid, Vec<tokuly_proto::common::UserSummary>> = HashMap::new();
        if place_ids.is_empty() {
            return Ok(out);
        }
        let friends = graph::friend_ids(&self.core, viewer).await?;
        // Visits the viewer was in the audience of when they began (see
        // `visibility::Audience`): those are the only ones they ever hear of.
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT place_id, user_id FROM geo_visits WHERE place_id = ANY($1) AND user_id = ANY($2) AND {}
             GROUP BY place_id, user_id ORDER BY count(*) DESC",
            Audience::sql_includes("audience", "audience_ids", "$3")
        )))
        .bind(place_ids)
        .bind(&friends)
        .bind(viewer)
        .fetch_all(&self.core.db)
        .await?;
        let users = tokuly_identity::summaries(&self.core, &rows.iter().map(|row| row.1).collect::<Vec<_>>()).await?;
        for (place_id, user_id) in rows {
            let visitors = out.entry(place_id).or_default();
            if let Some(user) = users.get(&user_id).filter(|user| !user.deleted) {
                if visitors.len() < 20 {
                    visitors.push(user.clone());
                }
            }
        }
        Ok(out)
    }
}

impl PlaceService for Geo {
    async fn list_my_places(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListMyPlacesRequest>,
    ) -> ServiceResult<ListMyPlacesResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            // Home, work and school first, then the rest in the order they were made.
            let rows: Vec<PlaceRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {PLACE_COLUMNS} FROM geo_places WHERE owner_id = $1 ORDER BY label, created_at"
            )))
            .bind(user_id)
            .fetch_all(&self.core.db)
            .await?;
            ok(ListMyPlacesResponse { places: self.places_for(user_id, &rows).await?, ..Default::default() })
        })
        .await
    }

    async fn create_place(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreatePlaceRequest>,
    ) -> ServiceResult<CreatePlaceResponse> {
        handle(async {
            let owner = ctx.user_id()?;
            let req = request.to_owned_message();
            let name = places::clean_name(&req.name)?;
            let emoji = places::clean_emoji(&req.emoji)?;
            let label = places::clean_label(req.label)?;
            let center = places::clean_point(req.center.as_option())?;
            let radius = places::clean_radius(req.radius_meters)?;
            let visibility = places::clean_visibility(req.visibility, label)?;
            let category = places::clean_category(&req.category)?;
            let photo = self.place_photo(owner, &req.photo_asset_id).await?;
            self.core.cache.rate_limit(&format!("geo-place:{owner}"), 60, Duration::from_secs(3600)).await?;
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM geo_places WHERE owner_id = $1").bind(owner).fetch_one(&self.core.db).await?;
            ensure!(count < MAX_PLACES, precondition, "TOO_MANY_PLACES", "at most {MAX_PLACES} places per account");

            // Home, work or school right where the stay patterns point: detected rather than drawn by hand.
            let auto_detected = label as i32 != LABEL_CUSTOM
                && self.place_suggestions(owner).await?.iter().any(|s| s.label as i32 == label as i32 && math::distance(s.cluster.center, center) <= SUGGESTION_MATCH_M);
            let address = self.address_at(center, self.speaks_japanese(owner).await?);

            let mut tx = self.core.db.begin().await?;
            let inserted: Result<PlaceRow, sqlx::Error> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "INSERT INTO geo_places (owner_id, name, emoji, label, lat, lng, radius_m, visibility, address, category, auto_detected, photo, photo_asset_id)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) RETURNING {PLACE_COLUMNS}"
            )))
            .bind(owner)
            .bind(&name)
            .bind(&emoji)
            .bind(label)
            .bind(center.0)
            .bind(center.1)
            .bind(radius)
            .bind(visibility)
            .bind(&address)
            .bind(&category)
            .bind(auto_detected)
            .bind(photo.as_ref().map(|(_, image)| image))
            .bind(photo.as_ref().map(|(asset_id, _)| *asset_id))
            .fetch_one(&mut *tx)
            .await;
            let place = match inserted {
                Ok(place) => place,
                Err(sqlx::Error::Database(e)) if e.is_unique_violation() => bail!(already_exists, "LABEL_TAKEN", "you already have a place with this label"),
                Err(e) => return Err(e.into()),
            };
            if let Some((asset_id, _)) = &photo {
                assets::attach(&mut *tx, *asset_id, &photo_holder(place.id)).await?;
            }
            self.adopt_visits(&mut tx, &place).await?;
            self.index_place(&mut tx, &place, false).await?;
            tx.commit().await?;
            self.places_changed(owner).await?;
            let place = self.place_row(place.id).await?.unwrap_or(place);
            ok(CreatePlaceResponse { place: self.place_for(owner, &place).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn update_place(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdatePlaceRequest>,
    ) -> ServiceResult<UpdatePlaceResponse> {
        handle(async {
            let owner = ctx.user_id()?;
            let req = request.to_owned_message();
            let before = self.own_place(owner, &req.place_id).await?;
            let mut place = before.clone();
            if let Some(name) = &req.name {
                place.name = places::clean_name(name)?;
            }
            if let Some(emoji) = &req.emoji {
                place.emoji = places::clean_emoji(emoji)?;
            }
            if let Some(label) = req.label {
                place.label = places::clean_label(label)?;
            }
            if let Some(center) = req.center.as_option() {
                (place.lat, place.lng) = places::clean_point(Some(center))?;
            }
            if let Some(radius) = req.radius_meters {
                place.radius_m = places::clean_radius(radius)?;
            }
            if let Some(category) = &req.category {
                place.category = places::clean_category(category)?;
            }
            // Home, work and school are private whatever the visibility was or is asked to be.
            let asked = req.visibility.unwrap_or_else(|| buffa::EnumValue::from(place.visibility as i32));
            place.visibility = places::clean_visibility(asked, place.label)?;
            // Some("") removes the photo; Some(id) replaces it.
            let mut new_photo = None;
            if let Some(asset_id) = &req.photo_asset_id {
                new_photo = self.place_photo(owner, asset_id).await?;
                place.photo = new_photo.as_ref().map(|(_, image)| image.clone());
                place.photo_asset_id = new_photo.as_ref().map(|(asset_id, _)| *asset_id);
            }
            let moved = place.center() != before.center() || place.radius_m != before.radius_m;
            if place.center() != before.center() {
                place.address = self.address_at(place.center(), self.speaks_japanese(owner).await?);
            }

            let mut tx = self.core.db.begin().await?;
            let updated = sqlx::query(
                "UPDATE geo_places SET name = $2, emoji = $3, label = $4, lat = $5, lng = $6, radius_m = $7, visibility = $8, address = $9,
                        category = $10, photo = $11, photo_asset_id = $12
                 WHERE id = $1",
            )
            .bind(place.id)
            .bind(&place.name)
            .bind(&place.emoji)
            .bind(place.label)
            .bind(place.lat)
            .bind(place.lng)
            .bind(place.radius_m)
            .bind(place.visibility)
            .bind(&place.address)
            .bind(&place.category)
            .bind(&place.photo)
            .bind(place.photo_asset_id)
            .execute(&mut *tx)
            .await;
            match updated {
                Ok(_) => {}
                Err(sqlx::Error::Database(e)) if e.is_unique_violation() => bail!(already_exists, "LABEL_TAKEN", "you already have a place with this label"),
                Err(e) => return Err(e.into()),
            }
            if req.photo_asset_id.is_some() && place.photo_asset_id != before.photo_asset_id {
                if let Some(old) = before.photo_asset_id {
                    assets::detach(&mut *tx, old, &photo_holder(place.id)).await?;
                }
                if let Some((asset_id, _)) = &new_photo {
                    assets::attach(&mut *tx, *asset_id, &photo_holder(place.id)).await?;
                }
            }
            if place.name != before.name {
                sqlx::query("UPDATE geo_visits SET place_name = $2 WHERE place_id = $1 AND user_id = $3").bind(place.id).bind(&place.name).bind(owner).execute(&mut *tx).await?;
            }
            if moved {
                self.adopt_visits(&mut tx, &place).await?;
            }
            self.index_place(&mut tx, &place, before.visibility == PUBLIC).await?;
            tx.commit().await?;
            self.places_changed(owner).await?;
            let place = self.place_row(place.id).await?.unwrap_or(place);
            ok(UpdatePlaceResponse { place: self.place_for(owner, &place).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_place(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeletePlaceRequest>,
    ) -> ServiceResult<DeletePlaceResponse> {
        handle(async {
            let owner = ctx.user_id()?;
            let place = self.own_place(owner, request.place_id).await?;
            let mut tx = self.core.db.begin().await?;
            self.remove_place(&mut tx, &place).await?;
            tx.commit().await?;
            self.places_changed(owner).await?;
            ok(DeletePlaceResponse::default())
        })
        .await
    }

    async fn get_place(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetPlaceRequest>,
    ) -> ServiceResult<GetPlaceResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let place = self.visible_place(viewer, request.place_id).await?;
            // The card is made of the caller's own visits and the visits of
            // friends who let the caller see where they were at the time
            // (the audience recorded on each visit). Nobody else's stay ever
            // shows here — not by name and not in a number — whoever owns the
            // place: a place must not work as a tripwire.
            let friends = graph::friend_ids(&self.core, viewer).await?;
            let visitors: Vec<(Uuid, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT user_id, count(*) FROM geo_visits
                 WHERE place_id = $1 AND (user_id = $3 OR (user_id = ANY($2) AND {}))
                 GROUP BY user_id ORDER BY count(*) DESC, user_id LIMIT 50",
                Audience::sql_includes("audience", "audience_ids", "$3")
            )))
            .bind(place.id)
            .bind(&friends)
            .bind(viewer)
            .fetch_all(&self.core.db)
            .await?;
            let users = tokuly_identity::summaries(&self.core, &visitors.iter().map(|v| v.0).collect::<Vec<_>>()).await?;
            let visitors: Vec<(Uuid, i64)> = visitors.into_iter().filter(|(id, _)| users.get(id).is_some_and(|user| !user.deleted)).collect();
            let regulars: Vec<PlaceRegular> = visitors
                .iter()
                .take(10)
                .enumerate()
                .map(|(index, (id, count))| PlaceRegular { user: users.get(id).cloned().into(), visit_count: *count as i32, rank: index as i32 + 1, ..Default::default() })
                .collect();
            let friends_visited: Vec<_> = visitors.iter().filter(|(id, _)| *id != viewer).take(20).filter_map(|(id, _)| users.get(id).cloned()).collect();

            let mut badges = Vec::new();
            // The caller's own visits here in company of two or more friends.
            let groups: i64 = sqlx::query_scalar("SELECT count(*) FROM geo_visits WHERE place_id = $1 AND user_id = $2 AND cardinality(with_user_ids) >= 2")
                .bind(place.id)
                .bind(viewer)
                .fetch_one(&self.core.db)
                .await?;
            if groups >= 2 {
                badges.push("GOOD_FOR_GROUPS".to_owned());
            }
            if visitors.iter().filter(|(id, count)| *id != viewer && *count >= 3).count() >= 2 {
                badges.push("BEST_FRIENDS_HANG_HERE".to_owned());
            }
            if visitors.first().is_some_and(|(_, count)| *count >= 10) {
                badges.push("LOYAL_REGULARS".to_owned());
            }
            ok(GetPlaceResponse {
                place: self.place_for(viewer, &place).await?.into(),
                regulars,
                friends_visited,
                total_visit_count: visitors.iter().map(|(_, count)| *count as i32).sum(),
                badges,
                ..Default::default()
            })
        })
        .await
    }

    async fn search_places(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SearchPlacesRequest>,
    ) -> ServiceResult<SearchPlacesResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let req = request.to_owned_message();
            let query = req.query.trim();
            ensure!(query.chars().count() <= MAX_QUERY_CHARS, invalid, "QUERY_TOO_LONG", "the search text is too long");
            let near = match req.near.as_option() {
                Some(point) => Some(places::clean_point(Some(point))?),
                None => None,
            };
            let limit = if req.limit <= 0 { 20 } else { req.limit.min(50) } as usize;
            self.core.cache.rate_limit(&format!("geo-place-search:{viewer}"), 120, Duration::from_secs(60)).await?;
            if query.is_empty() && near.is_none() {
                return ok(SearchPlacesResponse::default());
            }
            let ja = self.speaks_japanese(viewer).await?;
            let friends = graph::friend_ids(&self.core, viewer).await?;
            let hidden = graph::blocked_ids_either(&self.core, viewer).await?;
            let pattern = format!("%{}%", query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));

            // Tokuly places the caller may see: their own first, then by distance (when asked) and popularity.
            let rows: Vec<PlaceRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {PLACE_COLUMNS} FROM geo_places p
                 WHERE (p.owner_id = $1 OR ((p.visibility = {PUBLIC} OR (p.visibility = {FRIENDS} AND p.owner_id = ANY($2))) AND NOT (p.owner_id = ANY($3))))
                   AND ($4 = '' OR p.name ILIKE $5 OR p.category ILIKE $5 OR p.address ILIKE $5)
                   AND ($4 <> '' OR ST_DWithin(p.geog, ST_SetSRID(ST_MakePoint($7, $6), 4326)::geography, $8))
                 ORDER BY (p.owner_id = $1) DESC,
                          CASE WHEN $6::float8 IS NULL THEN 0 ELSE ST_Distance(p.geog, ST_SetSRID(ST_MakePoint($7, $6), 4326)::geography) END,
                          p.created_at, p.id
                 LIMIT $9"
            )))
            .bind(viewer)
            .bind(&friends)
            .bind(&hidden)
            .bind(query)
            .bind(&pattern)
            .bind(near.map(|p| p.0))
            .bind(near.map(|p| p.1))
            .bind(NEARBY_M)
            .bind(limit as i64)
            .fetch_all(&self.core.db)
            .await?;
            let mut visited = self.friends_visited(viewer, &rows.iter().map(|row| row.id).collect::<Vec<_>>()).await?;
            let distance_from = |position: Point| near.map(|near| math::distance(near, position));
            let mut results: Vec<PlaceSearchResult> = rows
                .iter()
                .zip(self.places_for(viewer, &rows).await?)
                .map(|(row, place)| PlaceSearchResult {
                    place: place.into(),
                    name: row.name.clone(),
                    address: row.address.clone(),
                    position: math::lat_lng(row.lat, row.lng).into(),
                    distance_meters: distance_from(row.center()),
                    friends_visited: visited.remove(&row.id).unwrap_or_default(),
                    ..Default::default()
                })
                .collect();

            // Then the geocoder: the external one when configured, the bundled gazetteer otherwise.
            if !query.is_empty() && results.len() < limit {
                let wanted = limit - results.len();
                let mut params = vec![("q", query.to_owned()), ("limit", wanted.to_string())];
                if let Some((lat, lng)) = near {
                    params.push(("lat", lat.to_string()));
                    params.push(("lon", lng.to_string()));
                }
                match self.photon("api", &params, ja).await {
                    Some(hits) => results.extend(hits.into_iter().take(wanted).map(|hit| PlaceSearchResult {
                        address: hit.address(),
                        position: math::lat_lng(hit.position.0, hit.position.1).into(),
                        distance_meters: distance_from(hit.position),
                        name: hit.name,
                        ..Default::default()
                    })),
                    None => results.extend(Gazetteer::shared().search(query, near, wanted).into_iter().map(|found| PlaceSearchResult {
                        name: found.locality(ja).to_owned(),
                        address: found.address(ja),
                        position: math::lat_lng(found.position.0, found.position.1).into(),
                        distance_meters: distance_from(found.position),
                        ..Default::default()
                    })),
                }
            }
            ok(SearchPlacesResponse { results, ..Default::default() })
        })
        .await
    }

    async fn list_place_suggestions(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListPlaceSuggestionsRequest>,
    ) -> ServiceResult<ListPlaceSuggestionsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let ja = self.speaks_japanese(user_id).await?;
            let suggestions = self
                .place_suggestions(user_id)
                .await?
                .into_iter()
                .map(|suggestion| {
                    let center = suggestion.cluster.center;
                    let locality = Gazetteer::shared()
                        .nearest(center.0, center.1)
                        .map(|found| found.locality(ja).to_owned())
                        .unwrap_or_default();
                    PlaceSuggestion {
                        center: math::lat_lng(center.0, center.1).into(),
                        suggested_name: suggestion_name(suggestion.label, &locality, ja),
                        suggested_label: suggestion.label.into(),
                        visit_count: suggestion.cluster.visit_count as i32,
                        total_seconds: suggestion.cluster.total_seconds,
                        ..Default::default()
                    }
                })
                .collect();
            ok(ListPlaceSuggestionsResponse { suggestions, ..Default::default() })
        })
        .await
    }

    async fn save_place(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SavePlaceRequest>,
    ) -> ServiceResult<SavePlaceResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let place = self.visible_place(user_id, request.place_id).await?;
            let saved: i64 = sqlx::query_scalar("SELECT count(*) FROM geo_saved_places WHERE user_id = $1")
                .bind(user_id)
                .fetch_one(&self.core.db)
                .await?;
            ensure!(saved < MAX_SAVED, precondition, "TOO_MANY_SAVED", "at most {MAX_SAVED} saved places");
            sqlx::query("INSERT INTO geo_saved_places (user_id, place_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(user_id)
                .bind(place.id)
                .execute(&self.core.db)
                .await?;
            ok(SavePlaceResponse::default())
        })
        .await
    }

    async fn unsave_place(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnsavePlaceRequest>,
    ) -> ServiceResult<UnsavePlaceResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            sqlx::query("DELETE FROM geo_saved_places WHERE user_id = $1 AND place_id = $2")
                .bind(user_id)
                .bind(parse_id(request.place_id)?)
                .execute(&self.core.db)
                .await?;
            ok(UnsavePlaceResponse::default())
        })
        .await
    }

    async fn list_saved_places(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListSavedPlacesRequest>,
    ) -> ServiceResult<ListSavedPlacesResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let page = Page::from_view(request.page.as_option(), 100);
            let cursor: Option<(DateTime<Utc>, Uuid)> = page.decode()?;
            let friends = graph::friend_ids(&self.core, user_id).await?;
            let hidden = graph::blocked_ids_either(&self.core, user_id).await?;
            // Places that stopped being visible to the caller (made private, unfriended, blocked) drop out.
            let mut rows: Vec<(DateTime<Utc>, PlaceRow)> = sqlx::query_as::<_, SavedRow>(sqlx::AssertSqlSafe(format!(
                "SELECT s.created_at AS saved_at, {columns} FROM geo_saved_places s JOIN geo_places p ON p.id = s.place_id
                 WHERE s.user_id = $1
                   AND (p.owner_id = $1 OR ((p.visibility = {PUBLIC} OR (p.visibility = {FRIENDS} AND p.owner_id = ANY($2))) AND NOT (p.owner_id = ANY($3))))
                   AND ($4::timestamptz IS NULL OR (s.created_at, s.place_id) < ($4, $5))
                 ORDER BY s.created_at DESC, s.place_id DESC LIMIT $6",
                columns = PLACE_COLUMNS.split(", ").map(|column| format!("p.{column}")).collect::<Vec<_>>().join(", ")
            )))
            .bind(user_id)
            .bind(&friends)
            .bind(&hidden)
            .bind(cursor.map(|c| c.0))
            .bind(cursor.map(|c| c.1))
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?
            .into_iter()
            .map(|row| (row.saved_at, row.place))
            .collect();
            let info = page.finish(&mut rows, |(saved_at, place)| (*saved_at, place.id));
            let places: Vec<PlaceRow> = rows.into_iter().map(|(_, place)| place).collect();
            ok(ListSavedPlacesResponse { places: self.places_for(user_id, &places).await?, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn reverse_geocode(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReverseGeocodeRequest>,
    ) -> ServiceResult<ReverseGeocodeResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let (lat, lng) = places::clean_point(req.position.as_option())?;
            self.core.cache.rate_limit(&format!("geo-geocode:{user_id}"), 240, Duration::from_secs(60)).await?;
            let ja = match req.language.trim() {
                "" => self.speaks_japanese(user_id).await?,
                language => language.to_ascii_lowercase().starts_with("ja"),
            };
            let gazetteer = Gazetteer::shared();
            let external = self
                .photon("reverse", &[("lat", lat.to_string()), ("lon", lng.to_string())], ja)
                .await
                .and_then(|hits| hits.into_iter().next());
            let result = match external {
                Some(hit) => GeocodeResult {
                    name: hit.name,
                    locality: hit.locality,
                    region: hit.region,
                    country: hit.country,
                    country_code: hit.country_code,
                    time_zone: gazetteer.time_zone(lat, lng),
                    ..Default::default()
                },
                None => gazetteer.reverse(lat, lng, ja),
            };
            ok(ReverseGeocodeResponse { result: result.into(), ..Default::default() })
        })
        .await
    }

    async fn check_in(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CheckInRequest>,
    ) -> ServiceResult<CheckInResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let message = req.message.trim().to_owned();
            ensure!(message.chars().count() <= MAX_MESSAGE_CHARS, invalid, "MESSAGE_TOO_LONG", "a check-in message has at most {MAX_MESSAGE_CHARS} characters");
            ensure!(req.with_user_ids.len() <= MAX_TAGGED, invalid, "TOO_MANY_USERS", "at most {MAX_TAGGED} friends per check-in");
            let mut tagged = parse_ids(&req.with_user_ids)?;
            tagged.sort();
            tagged.dedup();
            let friends = graph::friend_ids(&self.core, user_id).await?;
            ensure!(tagged.iter().all(|id| friends.contains(id)), precondition, "NOT_FRIENDS", "you can only tag friends");
            let owner_ctx = self.ctx_of(user_id).await?;
            let (place, center): (Option<crate::state::StayPlace>, Point) = match &req.r#where {
                Some(Where::PlaceId(place_id)) => {
                    let place = self.visible_place(user_id, place_id).await?;
                    (Some(place.stay_place()), place.center())
                }
                Some(Where::Position(position)) => {
                    let center = places::clean_point(Some(position))?;
                    (self.match_place(user_id, &owner_ctx, center).await?, center)
                }
                None => bail!(invalid, "MISSING_TARGET", "say where you are: a place or a position"),
            };
            self.core.cache.rate_limit(&format!("geo-checkin:{user_id}"), 30, Duration::from_secs(3600)).await?;
            let area = Gazetteer::shared().area(center.0, center.1);
            let ja = owner_ctx.ja;
            // Who sees the user's precise pin right now: the only friends this
            // check-in is for, now and whenever the place card is opened.
            let precise = Audience::of(&owner_ctx, Utc::now());
            let (audience_kind, audience_ids) = precise.to_stored();

            // At the spot of the ongoing stay the check-in belongs to that
            // visit; anywhere else it is a visit of its own.
            let row: VisitRow = self
                .with_state(user_id, async |state| {
                    let ongoing = state.visit_id.filter(|_| state.pin.stay_center.is_some_and(|stay| math::distance(stay, center) <= SAME_SPOT_M));
                    if let Some(visit_id) = ongoing {
                        let updated: Option<VisitRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                            "UPDATE geo_visits SET check_in = true, message = $2, with_user_ids = ARRAY(SELECT DISTINCT unnest(with_user_ids || $3)),
                                    place_id = COALESCE(place_id, $4), place_name = CASE WHEN place_id IS NULL THEN $5 ELSE place_name END
                             WHERE id = $1 RETURNING {VISIT_COLUMNS}"
                        )))
                        .bind(visit_id)
                        .bind(&message)
                        .bind(&tagged)
                        .bind(place.as_ref().map(|p| p.id))
                        .bind(place.as_ref().map_or("", |p| p.name.as_str()))
                        .fetch_optional(&self.core.db)
                        .await?;
                        if let Some(row) = updated {
                            state.visit_with.extend(tagged.iter().filter(|id| !state.visit_with.contains(id)).copied().collect::<Vec<_>>());
                            return Ok(row);
                        }
                    }
                    let mut tx = self.core.db.begin().await?;
                    let row: VisitRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                        "INSERT INTO geo_visits (user_id, place_id, place_name, lat, lng, arrived_at, departed_at, with_user_ids, check_in, message, locality, locality_ja, country_code, audience, audience_ids)
                         VALUES ($1, $2, $3, $4, $5, now(), now(), $6, true, $7, $8, $9, $10, $11, $12) RETURNING {VISIT_COLUMNS}"
                    )))
                    .bind(user_id)
                    .bind(place.as_ref().map(|p| p.id))
                    .bind(place.as_ref().map_or("", |p| p.name.as_str()))
                    .bind(center.0)
                    .bind(center.1)
                    .bind(&tagged)
                    .bind(&message)
                    .bind(&area.locality)
                    .bind(&area.locality_ja)
                    .bind(&area.country_code)
                    .bind(audience_kind)
                    .bind(audience_ids)
                    .fetch_one(&mut *tx)
                    .await?;
                    tx.commit().await?;
                    Ok(row)
                })
                .await?;

            // What others are told: the place by name unless it is private
            // (home is never named), else just the area.
            let shareable = place.as_ref().filter(|p| p.visibility != PRIVATE as i32);
            let (where_ja, where_en) = match shareable {
                Some(place) => (format!("「{}」", place.name), place.name.clone()),
                None => (area.label(true), area.label(false)),
            };
            let name = tokuly_identity::summary(&self.core, user_id).await?.display_name;
            // A check-in says where the user is: it is for the friends who see
            // the precise pin anyway, tagged or not. It becomes a story in
            // friends' feeds (with the spot on the map) only if that is every
            // friend — an activity entry cannot be kept from some of them.
            let audience: HashSet<Uuid> = friends.iter().copied().filter(|friend| precise.includes(*friend)).collect();
            let open = precise.is_all_friends();
            let mut tx = self.core.db.begin().await?;
            for friend in tagged.iter().filter(|friend| audience.contains(friend)) {
                let notification = NotificationRequest::new(
                    *friend,
                    "geo.checkin_tagged",
                    Localized::new(format!("{name}さんが{where_ja}であなたと一緒にチェックインしました!"), format!("{name} checked in at {where_en} with you")),
                    Localized::same(message.clone()),
                    "/map",
                )
                .actor(user_id)
                .data(serde_json::json!({ "visit_id": row.id }));
                outbound::notify(&mut *tx, &notification).await?;
            }
            let mut others: Vec<Uuid> = audience.iter().filter(|friend| !tagged.contains(friend)).copied().collect();
            others.sort();
            for friend in others.into_iter().take(MAX_CHECK_IN_AUDIENCE) {
                let notification = NotificationRequest::new(
                    friend,
                    "geo.checkin",
                    Localized::new(format!("{name}さんが{where_ja}にチェックインしました!"), format!("{name} checked in at {where_en}")),
                    Localized::same(message.clone()),
                    "/map",
                )
                .actor(user_id)
                .collapse(format!("geo.checkin:{user_id}"))
                .data(serde_json::json!({ "visit_id": row.id }));
                outbound::notify(&mut *tx, &notification).await?;
            }
            let summary = Localized::new(format!("{where_ja}にチェックイン!"), format!("Checked in at {where_en}"));
            // `place_name` is what the feed's recaps read.
            let place_name = shareable.map_or_else(|| area.label(ja), |place| place.name.clone());
            let mut entry = ActivityEntry::new(user_id, "geo.visit", if open { ActivityAudience::Friends } else { ActivityAudience::Private }, summary)
                .with(tagged.iter().copied())
                .path("/map")
                .data(serde_json::json!({ "visit_id": row.id, "message": message, "place_name": place_name, "check_in": true }));
            if open {
                entry = entry.at(center.0, center.1);
            }
            if let Some(place) = shareable {
                entry = entry.subject(SubjectType::SUBJECT_TYPE_PLACE, place.id);
            }
            outbound::activity(&mut *tx, &entry).await?;
            tx.commit().await?;
            self.push_visit_news(user_id, &owner_ctx, &[VisitNews { arrived: true, at: Utc::now(), place }]).await?;
            let visit = self.visits_for(user_id, std::slice::from_ref(&row), ja).await?.pop();
            ok(CheckInResponse { visit: visit.into(), ..Default::default() })
        })
        .await
    }
}

/// A saved place with the time it was saved (the pagination key).
struct SavedRow {
    saved_at: DateTime<Utc>,
    place: PlaceRow,
}

impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for SavedRow {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row as _;
        Ok(Self { saved_at: row.try_get("saved_at")?, place: PlaceRow::from_row(row)? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photon_answers_are_read_feature_by_feature() {
        let body = serde_json::json!({
            "type": "FeatureCollection",
            "features": [
                { "geometry": { "type": "Point", "coordinates": [139.7005, 35.6595] },
                  "properties": { "name": "渋谷スクランブル交差点", "city": "渋谷区", "state": "東京都", "country": "日本", "countrycode": "jp", "osm_value": "attraction" } },
                { "geometry": { "type": "Point", "coordinates": [2.3488, 48.8534] },
                  "properties": { "name": "Paris", "state": "Île-de-France", "country": "France", "countrycode": "FR" } },
                { "geometry": { "type": "Point", "coordinates": [500.0, 0.0] }, "properties": { "name": "Nowhere" } },
                { "geometry": { "type": "Point", "coordinates": [1.0, 1.0] }, "properties": {} },
                { "properties": { "name": "No geometry" } }
            ]
        });
        let hits = parse_photon(&body);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].name, "渋谷スクランブル交差点");
        assert_eq!(
            (hits[0].locality.as_str(), hits[0].region.as_str(), hits[0].country_code.as_str()),
            ("渋谷区", "東京都", "JP")
        );
        assert_eq!(hits[0].position, (35.6595, 139.7005));
        assert_eq!(hits[0].address(), "渋谷区, 東京都, 日本");
        assert_eq!(hits[1].address(), "Île-de-France, France", "the name is not repeated in the address");
        assert!(parse_photon(&serde_json::json!({ "error": "nope" })).is_empty());
    }

    #[test]
    fn suggestions_are_named_by_role_or_area() {
        assert_eq!(suggestion_name(PlaceLabel::PLACE_LABEL_HOME, "渋谷区", true), "自宅");
        assert_eq!(suggestion_name(PlaceLabel::PLACE_LABEL_SCHOOL, "", false), "School");
        assert_eq!(suggestion_name(PlaceLabel::PLACE_LABEL_CUSTOM, "渋谷区", true), "渋谷区のよく行く場所");
        assert_eq!(suggestion_name(PlaceLabel::PLACE_LABEL_CUSTOM, "", false), "Frequent spot");
    }
}
