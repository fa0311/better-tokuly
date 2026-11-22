//! Places: the user's own (home, work, school, custom), friends' and public
//! ones. Rows, visibility, matching a stay to a place, and what changes when
//! a place is created, edited or removed.
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use tokuly_core::{
    outbound::{self, SearchDoc},
    prelude::*,
};
use tokuly_media::assets::{self, image_from_json};
use tokuly_proto::{
    common::Visibility,
    geo::{Place, PlaceLabel},
    media::AssetPurpose,
};
use tokuly_social::graph;

use crate::{
    Geo,
    ctx::OwnerCtx,
    geocode::Gazetteer,
    math::{self, Point},
    state::StayPlace,
};

pub(crate) const PUBLIC: i16 = Visibility::VISIBILITY_PUBLIC as i16;
pub(crate) const FRIENDS: i16 = Visibility::VISIBILITY_FRIENDS as i16;
pub(crate) const PRIVATE: i16 = Visibility::VISIBILITY_PRIVATE as i16;
pub(crate) const LABEL_HOME: i32 = PlaceLabel::PLACE_LABEL_HOME as i32;
pub(crate) const LABEL_CUSTOM: i32 = PlaceLabel::PLACE_LABEL_CUSTOM as i32;

pub(crate) const DEFAULT_RADIUS_M: f64 = 80.0;
pub(crate) const MIN_RADIUS_M: f64 = 25.0;
pub(crate) const MAX_RADIUS_M: f64 = 1000.0;
/// GPS error allowed on top of a place's radius when matching a stay.
const MATCH_ALLOWANCE_M: f64 = 20.0;
const MAX_NAME_CHARS: usize = 60;
const MAX_EMOJI_CHARS: usize = 16;
const MAX_CATEGORY_CHARS: usize = 32;
pub(crate) const MAX_PLACES: i64 = 200;

pub(crate) const PLACE_COLUMNS: &str = "id, owner_id, name, emoji, label, lat, lng, radius_m, visibility, address, category, auto_detected, photo, photo_asset_id, created_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct PlaceRow {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub name: String,
    pub emoji: String,
    pub label: i16,
    pub lat: f64,
    pub lng: f64,
    pub radius_m: f64,
    pub visibility: i16,
    pub address: String,
    pub category: String,
    pub auto_detected: bool,
    pub photo: Option<serde_json::Value>,
    pub photo_asset_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

impl PlaceRow {
    pub fn center(&self) -> Point {
        (self.lat, self.lng)
    }

    pub fn stay_place(&self) -> StayPlace {
        StayPlace {
            id: self.id,
            owner_id: self.owner_id,
            label: self.label as i32,
            name: self.name.clone(),
            emoji: self.emoji.clone(),
            visibility: self.visibility as i32,
        }
    }

    /// The place without anything specific to who is asking.
    pub fn to_proto(&self) -> Place {
        Place {
            id: self.id.to_string(),
            owner_id: self.owner_id.to_string(),
            name: self.name.clone(),
            emoji: self.emoji.clone(),
            label: (self.label as i32).into(),
            center: math::lat_lng(self.lat, self.lng).into(),
            radius_meters: self.radius_m,
            visibility: (self.visibility as i32).into(),
            address: self.address.clone(),
            category: self.category.clone(),
            auto_detected: self.auto_detected,
            created_at: ts(self.created_at),
            photo: image_from_json(self.photo.as_ref()).into(),
            ..Default::default()
        }
    }
}

pub(crate) fn photo_holder(place_id: Uuid) -> String {
    format!("geo:place:{place_id}")
}

pub(crate) fn place_not_found() -> AppError {
    AppError::not_found("PLACE_NOT_FOUND", "place not found")
}

// ── Validation ──────────────────────────────────────────────────────────────

pub(crate) fn clean_name(input: &str) -> AppResult<String> {
    let name = input.trim();
    ensure!(
        !name.is_empty() && name.chars().count() <= MAX_NAME_CHARS && !name.chars().any(char::is_control),
        invalid,
        "INVALID_NAME",
        "a place name has 1-{MAX_NAME_CHARS} characters"
    );
    Ok(name.to_owned())
}

pub(crate) fn clean_emoji(input: &str) -> AppResult<String> {
    let emoji = input.trim();
    ensure!(
        emoji.chars().count() <= MAX_EMOJI_CHARS
            && !emoji.chars().any(|c| c.is_control() || c.is_whitespace() || c.is_ascii_alphanumeric()),
        invalid,
        "INVALID_EMOJI",
        "the place icon must be a single emoji"
    );
    Ok(emoji.to_owned())
}

pub(crate) fn clean_category(input: &str) -> AppResult<String> {
    let category = input.trim();
    ensure!(
        category.chars().count() <= MAX_CATEGORY_CHARS && !category.chars().any(char::is_control),
        invalid,
        "INVALID_CATEGORY",
        "a category has at most {MAX_CATEGORY_CHARS} characters"
    );
    Ok(category.to_owned())
}

/// A known label; unspecified means a custom place.
pub(crate) fn clean_label(label: buffa::EnumValue<PlaceLabel>) -> AppResult<i16> {
    match label.as_known() {
        Some(PlaceLabel::PLACE_LABEL_UNSPECIFIED) => Ok(LABEL_CUSTOM as i16),
        Some(label) => Ok(label as i16),
        None => Err(AppError::invalid("INVALID_LABEL", "unknown place label")),
    }
}

/// 0 means the default radius.
pub(crate) fn clean_radius(radius_m: f64) -> AppResult<f64> {
    if radius_m == 0.0 {
        return Ok(DEFAULT_RADIUS_M);
    }
    ensure!(
        radius_m.is_finite() && (MIN_RADIUS_M..=MAX_RADIUS_M).contains(&radius_m),
        invalid,
        "INVALID_RADIUS",
        "the radius must be {MIN_RADIUS_M}-{MAX_RADIUS_M} meters"
    );
    Ok(radius_m)
}

/// Public, friends or private; unspecified means private. Home, work and
/// school are private whatever was asked.
pub(crate) fn clean_visibility(visibility: buffa::EnumValue<Visibility>, label: i16) -> AppResult<i16> {
    let asked = match visibility.as_known() {
        Some(Visibility::VISIBILITY_UNSPECIFIED | Visibility::VISIBILITY_PRIVATE) => PRIVATE,
        Some(Visibility::VISIBILITY_PUBLIC) => PUBLIC,
        Some(Visibility::VISIBILITY_FRIENDS) => FRIENDS,
        _ => return Err(AppError::invalid("INVALID_VISIBILITY", "a place is public, for friends, or private")),
    };
    Ok(if label as i32 == LABEL_CUSTOM { asked } else { PRIVATE })
}

pub(crate) fn clean_point(point: Option<&tokuly_proto::common::LatLng>) -> AppResult<Point> {
    let point = point.ok_or_else(|| AppError::invalid("INVALID_LOCATION", "a location is required"))?;
    ensure!(math::valid_coordinates(point.lat, point.lng), invalid, "INVALID_LOCATION", "a valid location is required");
    Ok((point.lat, point.lng))
}

/// Places republished per transaction when the search index is rebuilt.
const REINDEX_PAGE: i64 = 200;

/// The search document of a public place.
fn search_doc(place: &PlaceRow) -> SearchDoc {
    SearchDoc {
        kind: "place".into(),
        id: place.id,
        title: place.name.clone(),
        text: [place.address.as_str(), place.category.as_str()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        tags: if place.category.is_empty() { Vec::new() } else { vec![place.category.clone()] },
        owner_id: Some(place.owner_id),
        public: place.visibility == PUBLIC,
        image_url: image_from_json(place.photo.as_ref()).map(|image| image.url),
        path: format!("/places/{}", place.id),
        created_at: place.created_at.timestamp(),
        // Not how often people go there: a number everyone can see must not
        // move when someone visits (it would say where they are).
        score: 0.0,
        extra: serde_json::json!({ "emoji": place.emoji, "lat": place.lat, "lng": place.lng }),
    }
}

/// The best of the user's own places for a stay at `at`: the one whose
/// center is closest relative to its radius.
pub(crate) fn match_own_place(owner: Uuid, ctx: &OwnerCtx, at: Point) -> Option<StayPlace> {
    ctx.places
        .iter()
        .filter_map(|place| {
            let distance = math::distance(at, (place.lat, place.lng));
            (distance <= place.radius_m + MATCH_ALLOWANCE_M).then(|| (distance / place.radius_m.max(1.0), place))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, place)| StayPlace {
            id: place.id,
            owner_id: owner,
            label: place.label,
            name: place.name.clone(),
            emoji: place.emoji.clone(),
            visibility: place.visibility,
        })
}

impl Geo {
    pub(crate) async fn place_row(&self, place_id: Uuid) -> AppResult<Option<PlaceRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {PLACE_COLUMNS} FROM geo_places WHERE id = $1")))
            .bind(place_id)
            .fetch_optional(&self.core.db)
            .await?)
    }

    /// Whether `viewer` may see a place: their own, a public one (unless they
    /// and the owner are in a block), or a friend's place shared with friends.
    pub(crate) async fn can_see_place(&self, viewer: Uuid, place: &PlaceRow) -> AppResult<bool> {
        if place.owner_id == viewer {
            return Ok(true);
        }
        match place.visibility {
            PUBLIC => Ok(!graph::is_blocked_either(&self.core, viewer, place.owner_id).await?),
            FRIENDS => graph::are_friends(&self.core, viewer, place.owner_id).await,
            _ => Ok(false),
        }
    }

    /// A place by ID as `viewer` may see it, or `PLACE_NOT_FOUND`.
    pub(crate) async fn visible_place(&self, viewer: Uuid, place_id: &str) -> AppResult<PlaceRow> {
        let place = self.place_row(parse_id(place_id)?).await?.ok_or_else(place_not_found)?;
        ensure!(self.can_see_place(viewer, &place).await?, not_found, "PLACE_NOT_FOUND", "place not found");
        Ok(place)
    }

    /// A place by ID that `owner` owns, or `PLACE_NOT_FOUND`.
    pub(crate) async fn own_place(&self, owner: Uuid, place_id: &str) -> AppResult<PlaceRow> {
        self.place_row(parse_id(place_id)?).await?.filter(|place| place.owner_id == owner).ok_or_else(place_not_found)
    }

    /// Places as messages for `viewer`, with their own visit statistics and
    /// saved flag filled in.
    pub(crate) async fn places_for(&self, viewer: Uuid, rows: &[PlaceRow]) -> AppResult<Vec<Place>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        let visits: Vec<(Uuid, i64, Option<DateTime<Utc>>)> =
            sqlx::query_as("SELECT place_id, count(*), max(arrived_at) FROM geo_visits WHERE user_id = $1 AND place_id = ANY($2) GROUP BY place_id")
                .bind(viewer)
                .bind(&ids)
                .fetch_all(&self.core.db)
                .await?;
        let visits: HashMap<Uuid, (i64, Option<DateTime<Utc>>)> =
            visits.into_iter().map(|(id, count, last)| (id, (count, last))).collect();
        let saved: Vec<Uuid> =
            sqlx::query_scalar("SELECT place_id FROM geo_saved_places WHERE user_id = $1 AND place_id = ANY($2)")
                .bind(viewer)
                .bind(&ids)
                .fetch_all(&self.core.db)
                .await?;
        Ok(rows
            .iter()
            .map(|row| {
                let mut place = row.to_proto();
                if let Some((count, last)) = visits.get(&row.id) {
                    place.my_visit_count = *count as i32;
                    place.my_last_visit_at = opt_ts(*last);
                }
                place.saved = saved.contains(&row.id);
                place
            })
            .collect())
    }

    pub(crate) async fn place_for(&self, viewer: Uuid, row: &PlaceRow) -> AppResult<Place> {
        Ok(self.places_for(viewer, std::slice::from_ref(row)).await?.pop().unwrap_or_default())
    }

    /// The place a stay at `at` belongs to: one of the user's own first, then
    /// the nearest public place or friend's place shared with friends.
    pub(crate) async fn match_place(&self, user_id: Uuid, ctx: &OwnerCtx, at: Point) -> AppResult<Option<StayPlace>> {
        if let Some(own) = match_own_place(user_id, ctx, at) {
            return Ok(Some(own));
        }
        let friends = graph::friend_ids(&self.core, user_id).await?;
        let hidden = graph::blocked_ids_either(&self.core, user_id).await?;
        let nearest: Option<PlaceRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {PLACE_COLUMNS} FROM geo_places p
             WHERE ST_DWithin(p.geog, ST_SetSRID(ST_MakePoint($2, $1), 4326)::geography, $3)
               AND ST_Distance(p.geog, ST_SetSRID(ST_MakePoint($2, $1), 4326)::geography) <= p.radius_m + $4
               AND p.owner_id <> $5
               AND (p.visibility = {PUBLIC} OR (p.visibility = {FRIENDS} AND p.owner_id = ANY($6)))
               AND NOT (p.owner_id = ANY($7))
             ORDER BY ST_Distance(p.geog, ST_SetSRID(ST_MakePoint($2, $1), 4326)::geography) / p.radius_m
             LIMIT 1"
        )))
        .bind(at.0)
        .bind(at.1)
        .bind(MAX_RADIUS_M + MATCH_ALLOWANCE_M)
        .bind(MATCH_ALLOWANCE_M)
        .bind(user_id)
        .bind(&friends)
        .bind(&hidden)
        .fetch_optional(&self.core.db)
        .await?;
        Ok(nearest.map(|place| place.stay_place()))
    }

    /// The name of a place at `at` that both of two users may know: a public
    /// place, or one of theirs shared with friends. For naming where a bump
    /// happened.
    pub(crate) async fn shared_place_name(&self, a: Uuid, b: Uuid, at: Point) -> AppResult<Option<String>> {
        Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT p.name FROM geo_places p
             WHERE ST_DWithin(p.geog, ST_SetSRID(ST_MakePoint($2, $1), 4326)::geography, $3)
               AND ST_Distance(p.geog, ST_SetSRID(ST_MakePoint($2, $1), 4326)::geography) <= p.radius_m + $4
               AND p.label = {LABEL_CUSTOM}
               AND (p.visibility = {PUBLIC} OR (p.visibility = {FRIENDS} AND p.owner_id IN ($5, $6)))
             ORDER BY ST_Distance(p.geog, ST_SetSRID(ST_MakePoint($2, $1), 4326)::geography) / p.radius_m
             LIMIT 1"
        )))
        .bind(at.0)
        .bind(at.1)
        .bind(MAX_RADIUS_M + MATCH_ALLOWANCE_M)
        .bind(MATCH_ALLOWANCE_M)
        .bind(a)
        .bind(b)
        .fetch_optional(&self.core.db)
        .await?)
    }

    /// Resolves a photo upload for a place into the stored image.
    pub(crate) async fn place_photo(
        &self,
        owner: Uuid,
        asset_id: &str,
    ) -> AppResult<Option<(Uuid, serde_json::Value)>> {
        let Some(asset_id) = parse_opt_id(asset_id)? else { return Ok(None) };
        let image =
            assets::require_image_json(&self.core, asset_id, owner, &[AssetPurpose::ASSET_PURPOSE_PLACE_PHOTO]).await?;
        Ok(Some((asset_id, image)))
    }

    /// One-line address of a point in the owner's language.
    pub(crate) fn address_at(&self, at: Point, ja: bool) -> String {
        Gazetteer::shared().nearest(at.0, at.1).map(|found| found.address(ja)).unwrap_or_default()
    }

    /// Keeps the search index in line with a place: public places are
    /// indexed, everything else is removed.
    pub(crate) async fn index_place(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        place: &PlaceRow,
        was_public: bool,
    ) -> AppResult<()> {
        if place.visibility == PUBLIC {
            outbound::index(&mut **tx, search_doc(place)).await
        } else if was_public {
            outbound::unindex(&mut **tx, "place", place.id).await
        } else {
            Ok(())
        }
    }

    /// Publishes the search document of every public place again (an
    /// administrator asked for the kind to be rebuilt; documents that did not
    /// change cost the search domain nothing). Returns how many there were.
    pub(crate) async fn reindex_places(&self) -> AppResult<u64> {
        let mut published = 0;
        let mut after = Uuid::nil();
        loop {
            // Read and publish under a row lock: a place that stops being
            // public meanwhile is unpublished after this, not before.
            let mut tx = self.core.db.begin().await?;
            let page: Vec<PlaceRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {PLACE_COLUMNS} FROM geo_places WHERE visibility = $1 AND id > $2 ORDER BY id LIMIT {REINDEX_PAGE} FOR SHARE"
            )))
            .bind(PUBLIC)
            .bind(after)
            .fetch_all(&mut *tx)
            .await?;
            let Some(last) = page.last() else { break };
            after = last.id;
            for place in &page {
                outbound::index(&mut *tx, search_doc(place)).await?;
            }
            tx.commit().await?;
            published += page.len() as u64;
        }
        Ok(published)
    }

    /// Past stays of the owner inside a new or moved place become visits to
    /// it, so its visit count starts with the truth and it stops being
    /// suggested.
    pub(crate) async fn adopt_visits(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        place: &PlaceRow,
    ) -> AppResult<()> {
        let adopted = sqlx::query(
            "UPDATE geo_visits v SET place_id = $1, place_name = $2
             WHERE v.user_id = $3 AND v.place_id IS NULL
               AND ST_DWithin(ST_SetSRID(ST_MakePoint(v.lng, v.lat), 4326)::geography, ST_SetSRID(ST_MakePoint($5, $4), 4326)::geography, $6)",
        )
        .bind(place.id)
        .bind(&place.name)
        .bind(place.owner_id)
        .bind(place.lat)
        .bind(place.lng)
        .bind(place.radius_m + MATCH_ALLOWANCE_M)
        .execute(&mut **tx)
        .await?
        .rows_affected();
        if adopted > 0 {
            sqlx::query(
                "UPDATE geo_nights n SET place_id = v.place_id, place_name = v.place_name, at_home = $3
                 FROM geo_visits v WHERE n.visit_id = v.id AND v.place_id = $1 AND n.user_id = $2",
            )
            .bind(place.id)
            .bind(place.owner_id)
            .bind(place.label as i32 == LABEL_HOME)
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    /// After the owner's places changed: forget the cached context, match the
    /// current stay again and show friends the result.
    pub(crate) async fn places_changed(&self, owner: Uuid) -> AppResult<()> {
        self.invalidate_ctx(&[owner]).await;
        let ctx = self.ctx_of(owner).await?;
        let changed = self
            .with_state(owner, async |state| {
                let Some(center) = state.stay.current().map(|anchor| anchor.center()) else { return Ok(false) };
                let place = self.match_place(owner, &ctx, center).await?;
                if place == state.pin.place {
                    return Ok(false);
                }
                if let Some(visit_id) = state.visit_id {
                    sqlx::query("UPDATE geo_visits SET place_id = $2, place_name = $3 WHERE id = $1")
                        .bind(visit_id)
                        .bind(place.as_ref().map(|p| p.id))
                        .bind(place.as_ref().map_or("", |p| p.name.as_str()))
                        .execute(&self.core.db)
                        .await?;
                }
                state.pin.place = place;
                Ok(true)
            })
            .await?;
        if changed {
            self.refresh_friends_live(owner).await?;
        }
        Ok(())
    }

    /// Removes a place for good (by its owner, a moderator, or account deletion).
    pub(crate) async fn remove_place(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        place: &PlaceRow,
    ) -> AppResult<()> {
        assets::detach_holder(&mut **tx, &photo_holder(place.id)).await?;
        sqlx::query("DELETE FROM geo_places WHERE id = $1").bind(place.id).execute(&mut **tx).await?;
        if place.visibility == PUBLIC {
            outbound::unindex(&mut **tx, "place", place.id).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::OwnPlace;

    #[test]
    fn names_emoji_and_categories_are_validated() {
        assert_eq!(clean_name("  カフェ モカ ").unwrap(), "カフェ モカ");
        assert_eq!(clean_name("").unwrap_err().reason, "INVALID_NAME");
        assert_eq!(clean_name(&"あ".repeat(61)).unwrap_err().reason, "INVALID_NAME");
        assert_eq!(clean_name("a\nb").unwrap_err().reason, "INVALID_NAME");
        assert_eq!(clean_emoji("☕").unwrap(), "☕");
        assert_eq!(clean_emoji("👨‍👩‍👧‍👦").unwrap(), "👨‍👩‍👧‍👦");
        assert_eq!(clean_emoji("").unwrap(), "");
        assert_eq!(clean_emoji("cafe").unwrap_err().reason, "INVALID_EMOJI");
        assert_eq!(clean_emoji("☕ ☕").unwrap_err().reason, "INVALID_EMOJI");
        assert_eq!(clean_category(" cafe ").unwrap(), "cafe");
        assert_eq!(clean_category(&"x".repeat(33)).unwrap_err().reason, "INVALID_CATEGORY");
    }

    #[test]
    fn radius_label_and_visibility_have_defaults_and_bounds() {
        assert_eq!(clean_radius(0.0).unwrap(), DEFAULT_RADIUS_M);
        assert_eq!(clean_radius(150.0).unwrap(), 150.0);
        for bad in [10.0, 5000.0, -1.0, f64::NAN] {
            assert_eq!(clean_radius(bad).unwrap_err().reason, "INVALID_RADIUS", "{bad}");
        }
        assert_eq!(clean_label(PlaceLabel::PLACE_LABEL_UNSPECIFIED.into()).unwrap() as i32, LABEL_CUSTOM);
        assert_eq!(clean_label(PlaceLabel::PLACE_LABEL_HOME.into()).unwrap() as i32, LABEL_HOME);
        assert_eq!(clean_label(buffa::EnumValue::from(77)).unwrap_err().reason, "INVALID_LABEL");

        let custom = LABEL_CUSTOM as i16;
        assert_eq!(clean_visibility(Visibility::VISIBILITY_UNSPECIFIED.into(), custom).unwrap(), PRIVATE);
        assert_eq!(clean_visibility(Visibility::VISIBILITY_PUBLIC.into(), custom).unwrap(), PUBLIC);
        assert_eq!(clean_visibility(Visibility::VISIBILITY_FRIENDS.into(), custom).unwrap(), FRIENDS);
        assert_eq!(
            clean_visibility(Visibility::VISIBILITY_UNLISTED.into(), custom).unwrap_err().reason,
            "INVALID_VISIBILITY"
        );
        // Home is private whatever was asked.
        assert_eq!(clean_visibility(Visibility::VISIBILITY_PUBLIC.into(), LABEL_HOME as i16).unwrap(), PRIVATE);
    }

    #[test]
    fn points_must_be_real_coordinates() {
        assert_eq!(clean_point(Some(&math::lat_lng(35.0, 139.0))).unwrap(), (35.0, 139.0));
        assert_eq!(clean_point(None).unwrap_err().reason, "INVALID_LOCATION");
        assert_eq!(clean_point(Some(&math::lat_lng(95.0, 0.0))).unwrap_err().reason, "INVALID_LOCATION");
    }

    fn own(label: i32, name: &str, at: Point, radius_m: f64) -> OwnPlace {
        OwnPlace {
            id: Uuid::now_v7(),
            label,
            name: name.into(),
            emoji: String::new(),
            lat: at.0,
            lng: at.1,
            radius_m,
            visibility: 5,
        }
    }

    #[test]
    fn a_stay_matches_the_own_place_it_is_deepest_inside() {
        let home_at = (35.6595, 139.7005);
        let park_at = math::destination(home_at.0, home_at.1, 90.0, 150.0);
        let ctx = OwnerCtx {
            places: vec![own(1, "うち", home_at, 80.0), own(4, "公園", park_at, 300.0)],
            ..Default::default()
        };
        let owner = Uuid::now_v7();
        // At home: inside both, but relatively closer to home's center.
        assert_eq!(
            match_own_place(owner, &ctx, math::destination(home_at.0, home_at.1, 0.0, 10.0)).unwrap().name,
            "うち"
        );
        // In the park, outside home's radius.
        let in_park = math::destination(park_at.0, park_at.1, 90.0, 100.0);
        let matched = match_own_place(owner, &ctx, in_park).unwrap();
        assert_eq!((matched.name.as_str(), matched.owner_id), ("公園", owner));
        // Far away: nothing.
        assert!(match_own_place(owner, &ctx, (35.0, 139.0)).is_none());
        // Just outside a radius the allowance for GPS error still matches; beyond it nothing does.
        let home_only = OwnerCtx { places: vec![own(1, "うち", home_at, 80.0)], ..Default::default() };
        assert_eq!(
            match_own_place(owner, &home_only, math::destination(home_at.0, home_at.1, 270.0, 95.0)).unwrap().name,
            "うち"
        );
        assert!(match_own_place(owner, &home_only, math::destination(home_at.0, home_at.1, 270.0, 105.0)).is_none());
    }
}
