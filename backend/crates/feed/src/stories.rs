//! `MapStoryService`: friends' recent activity with a location, as reels.
//!
//! Nothing is stored for a map story: a reel is a read of the `activities`
//! table (entries with a location from the last `TOKULY_FEED_MAP_STORY_HOURS`
//! hours that the reader may see). The only state is which items a viewer has
//! watched (`map_story_views`), and that is private to the viewer: no event,
//! notification or counter tells the author.
use std::{collections::HashMap, time::Duration};

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use sqlx::{Postgres, QueryBuilder};
use tokuly_core::prelude::*;
use tokuly_proto::{
    common::{LatLng, UserSummary},
    feed::*,
};
use tokuly_social::graph;

use crate::{
    Feed,
    activity::{
        AUDIENCE_FRIENDS, ActivityRow, Reader, activity_columns, author_active, kind_shown, push_not_involving,
        to_activity,
    },
};

/// Items one reel shows at most (the newest ones).
const MAX_REEL_ITEMS: usize = 50;
/// Rows read for one request, across all friends.
const MAX_STORY_ROWS: i64 = 5000;
/// IDs accepted by one `MarkMapStorySeen` call.
const MAX_SEEN_IDS: usize = 200;

#[derive(sqlx::FromRow)]
struct StoryRow {
    #[sqlx(flatten)]
    row: ActivityRow,
    seen: bool,
}

/// One person's story items, oldest first.
pub(crate) struct Reel {
    pub user_id: Uuid,
    pub items: Vec<(ActivityRow, bool)>,
}

impl Reel {
    pub fn unseen(&self) -> usize {
        self.items.iter().filter(|(_, seen)| !seen).count()
    }

    /// The newest item (reels are never empty).
    pub fn latest(&self) -> &ActivityRow {
        &self.items.last().expect("a reel has at least one item").0
    }
}

/// Groups story rows (newest first) into reels: per person the newest
/// [`MAX_REEL_ITEMS`] items, oldest first. Reels with something unseen come
/// first, then the most recent ones.
pub(crate) fn group_reels(rows: Vec<(ActivityRow, bool)>) -> Vec<Reel> {
    let mut order: Vec<Uuid> = Vec::new();
    let mut by_user: HashMap<Uuid, Vec<(ActivityRow, bool)>> = HashMap::new();
    for (row, seen) in rows {
        let items = by_user.entry(row.user_id).or_insert_with(|| {
            order.push(row.user_id);
            Vec::new()
        });
        if items.len() < MAX_REEL_ITEMS {
            items.push((row, seen));
        }
    }
    let mut reels: Vec<Reel> = order
        .into_iter()
        .filter_map(|user_id| {
            let mut items = by_user.remove(&user_id)?;
            items.reverse();
            Some(Reel { user_id, items })
        })
        .collect();
    reels.sort_by(|a, b| (b.unseen() > 0).cmp(&(a.unseen() > 0)).then_with(|| b.latest().key().cmp(&a.latest().key())));
    reels
}

fn location_of(row: &ActivityRow) -> Option<LatLng> {
    match (row.lat, row.lng) {
        (Some(lat), Some(lng)) => Some(LatLng { lat, lng, ..Default::default() }),
        _ => None,
    }
}

impl Feed {
    /// Adds the conditions that make an entry `a` a map story the reader may
    /// see: a location, inside the window, shown to friends.
    fn push_story_conditions(&self, query: &mut QueryBuilder<Postgres>, hidden: &[Uuid]) {
        query
            .push(" AND a.lat IS NOT NULL AND a.occurred_at > now() - make_interval(hours => ")
            .push_bind(self.cfg.map_story_hours as i32);
        query.push(") AND a.audience >= ").push_bind(AUDIENCE_FRIENDS);
        query.push(concat!(" AND ", kind_shown!("a"), " AND ", author_active!("a")));
        push_not_involving(query, "a", hidden);
    }

    /// The reels of `authors` as `viewer` sees them.
    pub(crate) async fn load_reels(&self, viewer: Uuid, authors: &[Uuid], hidden: &[Uuid]) -> AppResult<Vec<Reel>> {
        if authors.is_empty() {
            return Ok(Vec::new());
        }
        let mut query: QueryBuilder<Postgres> = QueryBuilder::new(concat!(
            "SELECT ",
            activity_columns!(),
            ", (v.activity_id IS NOT NULL) AS seen
             FROM activities a LEFT JOIN map_story_views v ON v.activity_id = a.id AND v.viewer_id = "
        ));
        query.push_bind(viewer).push(" WHERE a.user_id = ANY(").push_bind(authors.to_vec()).push(")");
        self.push_story_conditions(&mut query, hidden);
        query.push(" ORDER BY a.occurred_at DESC, a.id DESC LIMIT ").push_bind(MAX_STORY_ROWS);
        let rows: Vec<StoryRow> = query.build_query_as().fetch_all(&self.core.db).await?;
        Ok(group_reels(rows.into_iter().map(|story| (story.row, story.seen)).collect()))
    }

    /// The caller's own reel as friends see it. Own items never count as unseen.
    async fn own_reel(&self, user_id: Uuid) -> AppResult<Option<Reel>> {
        let mut reels = self.load_reels(user_id, &[user_id], &[]).await?;
        Ok(reels.pop().map(|mut reel| {
            reel.items.iter_mut().for_each(|(_, seen)| *seen = true);
            reel
        }))
    }

    async fn reels_to_proto(&self, reader: &Reader, reels: &[Reel]) -> AppResult<Vec<MapStoryReel>> {
        let rows: Vec<&ActivityRow> = reels.iter().flat_map(|reel| reel.items.iter().map(|(row, _)| row)).collect();
        let people = self.people_of(&rows).await?;
        Ok(reels.iter().map(|reel| reel_to_proto(reel, reader, &people)).collect())
    }
}

fn reel_to_proto(reel: &Reel, reader: &Reader, people: &HashMap<Uuid, UserSummary>) -> MapStoryReel {
    let latest = reel.latest();
    MapStoryReel {
        user: tokuly_identity::users::pick(people, reel.user_id).into(),
        items: reel
            .items
            .iter()
            .map(|(row, seen)| MapStoryItem {
                activity: to_activity(row, reader, people, None).into(),
                seen: *seen,
                ..Default::default()
            })
            .collect(),
        unseen_count: reel.unseen() as i32,
        latest_at: ts(latest.occurred_at),
        latest_location: location_of(latest).into(),
        ..Default::default()
    }
}

fn tray_entry(reel: &Reel, people: &HashMap<Uuid, UserSummary>) -> MapStoryTrayEntry {
    let latest = reel.latest();
    MapStoryTrayEntry {
        user: tokuly_identity::users::pick(people, reel.user_id).into(),
        item_count: reel.items.len() as i32,
        unseen_count: reel.unseen() as i32,
        latest_at: ts(latest.occurred_at),
        latest_location: location_of(latest).into(),
        preview_image_url: reel.items.iter().rev().find_map(|(row, _)| row.image_url.clone()).unwrap_or_default(),
        ..Default::default()
    }
}

impl MapStoryService for Feed {
    async fn list_map_stories(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListMapStoriesRequest>,
    ) -> ServiceResult<ListMapStoriesResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let only = parse_opt_id(request.user_id)?;
            let reader = self.reader_for(user_id).await?;

            let authors = match only {
                Some(other) if other == user_id => Vec::new(),
                Some(other) => {
                    // Strangers, blocked people and unknown IDs all look the same.
                    let visible = !reader.is_hidden(other) && graph::are_friends(&self.core, user_id, other).await?;
                    ensure!(visible, not_found, "USER_NOT_FOUND", "user not found");
                    vec![other]
                }
                None => self.visible_friends(user_id, &reader.hidden).await?,
            };
            let reels = self.load_reels(user_id, &authors, &reader.hidden).await?;
            let own = self.own_reel(user_id).await?;

            let my_reel = match &own {
                Some(reel) => self.reels_to_proto(&reader, std::slice::from_ref(reel)).await?.pop(),
                None => None,
            };
            ok(ListMapStoriesResponse {
                reels: self.reels_to_proto(&reader, &reels).await?,
                my_reel: my_reel.into(),
                window_hours: self.cfg.map_story_hours as i32,
                ..Default::default()
            })
        })
        .await
    }

    async fn get_map_story_tray(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetMapStoryTrayRequest>,
    ) -> ServiceResult<GetMapStoryTrayResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let reader = self.reader_for(user_id).await?;
            let friends = self.visible_friends(user_id, &reader.hidden).await?;
            let reels = self.load_reels(user_id, &friends, &reader.hidden).await?;
            let people =
                tokuly_identity::summaries(&self.core, &reels.iter().map(|reel| reel.user_id).collect::<Vec<_>>())
                    .await?;
            ok(GetMapStoryTrayResponse {
                entries: reels.iter().map(|reel| tray_entry(reel, &people)).collect(),
                unseen_reel_count: reels.iter().filter(|reel| reel.unseen() > 0).count() as i32,
                ..Default::default()
            })
        })
        .await
    }

    async fn mark_map_story_seen(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, MarkMapStorySeenRequest>,
    ) -> ServiceResult<MarkMapStorySeenResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            ensure!(!request.activity_ids.is_empty(), invalid, "IDS_REQUIRED", "no story items given");
            ensure!(
                request.activity_ids.len() <= MAX_SEEN_IDS,
                invalid,
                "TOO_MANY_IDS",
                "at most {MAX_SEEN_IDS} story items per request"
            );
            let activity_ids = request.activity_ids.iter().map(|id| parse_id(id)).collect::<AppResult<Vec<Uuid>>>()?;
            self.core.cache.rate_limit(&format!("feed:story-seen:{user_id}"), 120, Duration::from_secs(60)).await?;

            let reader = self.reader_for(user_id).await?;
            let friends = self.visible_friends(user_id, &reader.hidden).await?;
            if friends.is_empty() {
                return ok(MarkMapStorySeenResponse::default());
            }
            // Only what the caller could actually have watched is recorded:
            // a friend's item that is a map story for them right now.
            let mut query: QueryBuilder<Postgres> =
                QueryBuilder::new("INSERT INTO map_story_views (viewer_id, activity_id, author_id) SELECT ");
            query
                .push_bind(user_id)
                .push(", a.id, a.user_id FROM activities a WHERE a.id = ANY(")
                .push_bind(activity_ids);
            query.push(") AND a.user_id = ANY(").push_bind(friends).push(")");
            self.push_story_conditions(&mut query, &reader.hidden);
            query.push(" ON CONFLICT DO NOTHING");
            let marked = query.build().execute(&self.core.db).await?.rows_affected();
            ok(MarkMapStorySeenResponse { marked_count: marked as i32, ..Default::default() })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration as Span;

    use super::*;

    fn item(user: Uuid, minutes_ago: i64, seen: bool) -> (ActivityRow, bool) {
        let row = ActivityRow {
            id: Uuid::now_v7(),
            user_id: user,
            kind: "social.bumped".into(),
            audience: AUDIENCE_FRIENDS,
            subject_type: None,
            subject_id: None,
            with_user_ids: Vec::new(),
            summary_ja: String::new(),
            summary_en: String::new(),
            path: String::new(),
            image_url: None,
            lat: Some(35.0),
            lng: Some(139.0),
            data: serde_json::Value::Null,
            occurred_at: now() - Span::minutes(minutes_ago),
        };
        (row, seen)
    }

    /// Rows as the query returns them: newest first.
    fn newest_first(mut rows: Vec<(ActivityRow, bool)>) -> Vec<(ActivityRow, bool)> {
        rows.sort_by(|a, b| b.0.key().cmp(&a.0.key()));
        rows
    }

    #[test]
    fn reels_play_oldest_first_and_unseen_reels_lead() {
        let (a, b, c) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let rows = newest_first(vec![
            item(a, 5, true), // newest overall, but everything of `a` is seen
            item(a, 50, true),
            item(b, 30, false),
            item(b, 90, true),
            item(c, 10, false),
        ]);
        let reels = group_reels(rows);
        assert_eq!(
            reels.iter().map(|reel| reel.user_id).collect::<Vec<_>>(),
            vec![c, b, a],
            "unseen first, then by recency"
        );
        let b_reel = &reels[1];
        assert_eq!(b_reel.unseen(), 1);
        assert!(b_reel.items[0].0.occurred_at < b_reel.items[1].0.occurred_at, "oldest first");
        assert_eq!(b_reel.latest().id, b_reel.items[1].0.id);
        assert_eq!(reels[2].unseen(), 0);
    }

    #[test]
    fn a_reel_keeps_only_the_newest_items() {
        let user = Uuid::from_u128(7);
        let rows = newest_first((0..(MAX_REEL_ITEMS as i64 + 10)).map(|i| item(user, i, false)).collect());
        let newest = rows[0].0.id;
        let reels = group_reels(rows);
        assert_eq!(reels.len(), 1);
        assert_eq!(reels[0].items.len(), MAX_REEL_ITEMS);
        assert_eq!(reels[0].latest().id, newest);
        assert!(group_reels(Vec::new()).is_empty());
    }

    #[test]
    fn the_tray_previews_the_newest_item_with_an_image() {
        let user = Uuid::from_u128(9);
        let mut rows = newest_first(vec![item(user, 1, false), item(user, 20, true), item(user, 40, false)]);
        rows[1].0.image_url = Some("https://cdn.example/middle.webp".into());
        rows[2].0.image_url = Some("https://cdn.example/oldest.webp".into());
        let reels = group_reels(rows);
        let entry = tray_entry(&reels[0], &HashMap::new());
        assert_eq!((entry.item_count, entry.unseen_count), (3, 2));
        assert_eq!(entry.preview_image_url, "https://cdn.example/middle.webp");
        assert_eq!(entry.latest_location.as_option().unwrap().lat, 35.0);
    }
}
