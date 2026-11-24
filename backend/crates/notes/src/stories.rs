//! Stories (記憶): a title plus 1–10 photos or videos that the owner's
//! friends see for a day, and that stay in the owner's archive afterwards.
//!
//! Who may see a story is decided by [`crate::visibility::sees_story`].
//! Replies to a story are ordinary direct messages and never pass through
//! here.
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use chrono::{DateTime, LocalResult, NaiveDate, TimeZone as _, Utc};
use chrono_tz::Tz;
use sqlx::{Postgres, Transaction};
use tokuly_core::{analytics::AnalyticsEvent, events, outbound, prelude::*};
use tokuly_media::assets;
use tokuly_proto::{
    common::{PageInfo, UserSummary},
    media::{AssetKind, AssetPurpose},
    notes::{
        Story, StoryDay, StoryItem, StoryReaction, StoryReel, StoryRef, StoryViewCount, StoryViewer,
        notes_event::Event as Push,
    },
};
use tokuly_social::graph;

use crate::{
    Notes, copy,
    events::{RemovalReason, STORY_CREATED, STORY_DELETED, StoryCreated, StoryDeleted},
    model::{STORY_COLUMNS, StoryRow, story_holder, to_json},
    posting::MAX_IDEMPOTENCY_KEY_BYTES,
    text,
    visibility::{AUDIENCE_CLOSE_FRIENDS, AUDIENCE_FRIENDS, Viewer, not_hidden, sees_story},
};

pub const MAX_TITLE_CHARS: usize = 140;
pub const MAX_ITEMS: usize = 10;
pub const MAX_CAPTION_CHARS: usize = 200;
/// Most friends a close-friends story can be shown to.
pub const MAX_AUDIENCE: usize = 150;
/// Reactions (changed and removed ones included) one user may make per minute.
const REACTIONS_PER_MINUTE: u32 = 60;
/// Live stories returned in one tray. Far above what a circle of friends
/// produces in a day; a guard, not a page size.
const TRAY_LIMIT: i64 = 2000;
const IDEMPOTENCY_WINDOW: Duration = Duration::from_secs(600);
/// Time zone of users whose own is unknown: Tokuly is a Japanese product.
const DEFAULT_TIME_ZONE: Tz = chrono_tz::Asia::Tokyo;

/// What a user asks to post as a story.
#[derive(Debug, Clone, Default)]
pub(crate) struct NewStory {
    pub title: String,
    /// `(asset ID, caption)` in playing order.
    pub items: Vec<(Uuid, String)>,
    /// A stored audience value ([`AUDIENCE_FRIENDS`] or [`AUDIENCE_CLOSE_FRIENDS`]).
    pub audience: i32,
    pub audience_user_ids: Vec<Uuid>,
}

/// A deleted story and the people it was being shown to, for the realtime
/// announcement after the commit.
#[derive(Debug)]
pub(crate) struct RemovedStory {
    pub id: Uuid,
    pub owner_id: Uuid,
    /// It had not expired yet: friends may have it on screen.
    live: bool,
    /// The chosen friends of a close-friends story; `None` = every friend.
    chosen: Option<Vec<Uuid>>,
}

/// What the viewer of a list of stories did with them, and what only an
/// owner is told about their own.
#[derive(Default)]
struct StoryExtras {
    /// The viewer's reaction per watched story (empty = watched, no reaction).
    watched: HashMap<Uuid, String>,
    chosen: HashMap<Uuid, Vec<String>>,
    highlighted: HashSet<Uuid>,
}

fn story_proto(row: &StoryRow, owner: UserSummary, viewer: Option<Uuid>, extras: &StoryExtras) -> Story {
    let own = viewer == Some(row.owner_id);
    let mut story = Story {
        id: row.id.to_string(),
        owner: owner.into(),
        title: row.title.clone(),
        items: row.items(),
        audience: row.audience.into(),
        created_at: ts(row.created_at),
        expires_at: ts(row.expires_at),
        expired: row.is_expired(),
        // Owners are not counted as viewers of their own stories; to them
        // nothing of theirs is "unseen".
        viewed: own || extras.watched.contains_key(&row.id),
        my_reaction: extras.watched.get(&row.id).cloned().unwrap_or_default(),
        ..Default::default()
    };
    if own {
        story.view_count = row.view_count;
        story.reaction_count = row.reaction_count;
        story.audience_user_ids = extras.chosen.get(&row.id).cloned().unwrap_or_default();
        story.highlighted = extras.highlighted.contains(&row.id);
    }
    story
}

/// The caller's time zone (for "which day was this story posted on").
pub(crate) async fn time_zone_of(core: &Core, user_id: Uuid) -> AppResult<Tz> {
    let name: Option<String> =
        sqlx::query_scalar("SELECT time_zone FROM users WHERE id = $1").bind(user_id).fetch_optional(&core.db).await?;
    Ok(name.and_then(|name| name.parse().ok()).unwrap_or(DEFAULT_TIME_ZONE))
}

/// The instant a calendar day starts in a time zone. Where a clock change
/// skips midnight, the day starts at the first moment that exists.
fn day_start(time_zone: Tz, day: NaiveDate) -> DateTime<Utc> {
    let midnight = day.and_time(chrono::NaiveTime::MIN);
    for offset_hours in 0..=3 {
        match time_zone.from_local_datetime(&(midnight + chrono::Duration::hours(offset_hours))) {
            LocalResult::Single(time) | LocalResult::Ambiguous(time, _) => return time.with_timezone(&Utc),
            LocalResult::None => {}
        }
    }
    midnight.and_utc()
}

/// The half-open range of instants that make up one calendar day.
fn day_range(time_zone: Tz, day: NaiveDate) -> (DateTime<Utc>, DateTime<Utc>) {
    (day_start(time_zone, day), day_start(time_zone, day.succ_opt().unwrap_or(day)))
}

/// The half-open range of instants that make up one calendar month, or
/// `None` when year or month are not ones a story can have been posted in.
fn month_range(time_zone: Tz, year: i32, month: i32) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    if !(2000..=2200).contains(&year) || !(1..=12).contains(&month) {
        return None;
    }
    let first = NaiveDate::from_ymd_opt(year, month as u32, 1)?;
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)?
    } else {
        NaiveDate::from_ymd_opt(year, month as u32 + 1, 1)?
    };
    Some((day_start(time_zone, first), day_start(time_zone, next)))
}

impl Notes {
    /// Stories as `viewer` gets them, in the order of `rows` (which must be
    /// stories the viewer may see).
    pub(crate) async fn stories_to_proto(&self, viewer: &Viewer, rows: &[StoryRow]) -> AppResult<Vec<Story>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let db = &self.core.db;
        let mut owner_ids: Vec<Uuid> = rows.iter().map(|row| row.owner_id).collect();
        owner_ids.sort();
        owner_ids.dedup();
        let owners = tokuly_identity::summaries(&self.core, &owner_ids).await?;
        let mut extras = StoryExtras::default();
        if let Some(viewer_id) = viewer.id {
            let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
            let watched: Vec<(Uuid, String)> = sqlx::query_as(
                "SELECT story_id, reaction FROM story_views WHERE viewer_id = $1 AND story_id = ANY($2)",
            )
            .bind(viewer_id)
            .bind(&ids)
            .fetch_all(db)
            .await?;
            extras.watched = watched.into_iter().collect();
            let own: Vec<Uuid> = rows.iter().filter(|row| row.owner_id == viewer_id).map(|row| row.id).collect();
            if !own.is_empty() {
                let chosen: Vec<(Uuid, Uuid)> = sqlx::query_as(
                    "SELECT story_id, user_id FROM story_audience WHERE story_id = ANY($1) ORDER BY story_id, user_id",
                )
                .bind(&own)
                .fetch_all(db)
                .await?;
                for (story_id, user_id) in chosen {
                    extras.chosen.entry(story_id).or_default().push(user_id.to_string());
                }
                let highlighted: Vec<Uuid> =
                    sqlx::query_scalar("SELECT DISTINCT story_id FROM story_highlight_items WHERE story_id = ANY($1)")
                        .bind(&own)
                        .fetch_all(db)
                        .await?;
                extras.highlighted = highlighted.into_iter().collect();
            }
        }
        Ok(rows
            .iter()
            .map(|row| story_proto(row, tokuly_identity::users::pick(&owners, row.owner_id), viewer.id, &extras))
            .collect())
    }

    /// The story, or `STORY_NOT_FOUND` — also when the viewer may not see it.
    pub(crate) async fn require_story(&self, viewer: &Viewer, story_id: Uuid) -> AppResult<StoryRow> {
        let sql = format!("SELECT {STORY_COLUMNS} FROM stories s WHERE s.id = $7 AND {}", sees_story("s"));
        let row: Option<StoryRow> = viewer.query_as(sql).bind(story_id).fetch_optional(&self.core.db).await?;
        row.or_not_found("STORY_NOT_FOUND", "story not found")
    }

    /// One of `user_id`'s own stories, or `STORY_NOT_FOUND`: other people's
    /// stories do not exist as far as owner-only operations are concerned.
    pub(crate) async fn own_story(&self, user_id: Uuid, story_id: Uuid) -> AppResult<StoryRow> {
        let sql = format!("SELECT {STORY_COLUMNS} FROM stories s WHERE s.id = $1 AND s.owner_id = $2");
        let row: Option<StoryRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(story_id).bind(user_id).fetch_optional(&self.core.db).await?;
        row.or_not_found("STORY_NOT_FOUND", "story not found")
    }

    /// Validates a story's items and resolves their uploads.
    async fn resolve_items(&self, owner: Uuid, items: &[(Uuid, String)]) -> AppResult<Vec<StoryItem>> {
        ensure!(!items.is_empty(), invalid, "STORY_EMPTY", "a story needs at least one photo or video");
        ensure!(
            items.len() <= MAX_ITEMS,
            invalid,
            "TOO_MANY_ITEMS",
            "a story has at most {MAX_ITEMS} photos and videos"
        );
        let mut resolved = Vec::with_capacity(items.len());
        for (asset_id, caption) in items {
            let caption = text::clean(caption);
            ensure!(
                text::fits(&caption, MAX_CAPTION_CHARS),
                invalid,
                "CAPTION_TOO_LONG",
                "a caption has at most {MAX_CAPTION_CHARS} characters"
            );
            let asset =
                assets::require_ready(&self.core, *asset_id, owner, &[AssetPurpose::ASSET_PURPOSE_STORY]).await?;
            ensure!(
                matches!(
                    asset.asset_kind(),
                    AssetKind::ASSET_KIND_IMAGE | AssetKind::ASSET_KIND_ANIMATION | AssetKind::ASSET_KIND_VIDEO
                ),
                invalid,
                "ATTACHMENT_UNSUPPORTED",
                "stories are made of photos and videos"
            );
            resolved.push(StoryItem { media: asset.to_attachment().into(), caption, ..Default::default() });
        }
        Ok(resolved)
    }

    /// [`Self::create_story`] made safe to retry (see `CreateStoryRequest.idempotency_key`).
    pub(crate) async fn create_story_once(
        &self,
        owner: Uuid,
        idempotency_key: &str,
        input: NewStory,
    ) -> AppResult<StoryRow> {
        if idempotency_key.is_empty() {
            return self.create_story(owner, new_id(), input).await;
        }
        ensure!(
            idempotency_key.len() <= MAX_IDEMPOTENCY_KEY_BYTES,
            invalid,
            "INVALID_IDEMPOTENCY_KEY",
            "the idempotency key is too long"
        );
        let key = format!("notes:idem:story:{owner}:{idempotency_key}");
        let story_id = new_id();
        if !self.core.cache.set_nx(&key, &story_id.to_string(), IDEMPOTENCY_WINDOW).await? {
            let earlier = self.core.cache.get_string(&key).await?.and_then(|id| Uuid::parse_str(&id).ok());
            return match earlier {
                Some(id) => self.own_story(owner, id).await.map_err(|_| {
                    AppError::unavailable(
                        "REQUEST_IN_PROGRESS",
                        "the first request with this key has not finished; retry shortly",
                    )
                }),
                None => Err(AppError::unavailable("REQUEST_IN_PROGRESS", "retry shortly")),
            };
        }
        let result = self.create_story(owner, story_id, input).await;
        if result.is_err() {
            let _ = self.core.cache.del(&key).await;
        }
        result
    }

    /// Posts a story and shows it to its audience.
    pub(crate) async fn create_story(&self, owner: Uuid, story_id: Uuid, input: NewStory) -> AppResult<StoryRow> {
        let core = &self.core;
        let title = text::clean(&input.title).split_whitespace().collect::<Vec<_>>().join(" ");
        ensure!(
            text::fits(&title, MAX_TITLE_CHARS),
            invalid,
            "TITLE_TOO_LONG",
            "a story title has at most {MAX_TITLE_CHARS} characters"
        );
        let items = self.resolve_items(owner, &input.items).await?;
        let mut asset_ids: Vec<Uuid> = input.items.iter().map(|(asset_id, _)| *asset_id).collect();
        asset_ids.sort();
        asset_ids.dedup();

        let viewer = Viewer::load(core, Some(owner)).await?;
        let mut chosen = Vec::new();
        if input.audience == AUDIENCE_CLOSE_FRIENDS {
            chosen = input.audience_user_ids.clone();
            chosen.sort();
            chosen.dedup();
            ensure!(!chosen.is_empty(), invalid, "AUDIENCE_EMPTY", "choose the friends who may see the story");
            ensure!(
                chosen.len() <= MAX_AUDIENCE,
                invalid,
                "AUDIENCE_TOO_LARGE",
                "a story can be shown to at most {MAX_AUDIENCE} chosen friends"
            );
            ensure!(
                chosen.iter().all(|user_id| viewer.is_friend(*user_id)),
                invalid,
                "AUDIENCE_NOT_FRIENDS",
                "a story can only be shown to friends"
            );
        }
        core.cache
            .rate_limit(&format!("notes:story:{owner}"), self.cfg.stories_per_hour, Duration::from_secs(3600))
            .await?;

        let mut tx = core.db.begin().await?;
        // The lifetime is counted from the database's `created_at`, so a
        // story lives for exactly the configured time.
        let sql = format!(
            "INSERT INTO stories AS s (id, owner_id, title, items, asset_ids, audience, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, now() + make_interval(hours => $7)) RETURNING {STORY_COLUMNS}"
        );
        let story: StoryRow = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(story_id)
            .bind(owner)
            .bind(&title)
            .bind(to_json(&items)?)
            .bind(&asset_ids)
            .bind(input.audience)
            .bind(self.cfg.story_ttl_hours.clamp(1, i64::from(i32::MAX)) as i32)
            .fetch_one(&mut *tx)
            .await?;
        for asset_id in &asset_ids {
            assets::attach(&mut *tx, *asset_id, &story_holder(story.id)).await?;
        }
        if !chosen.is_empty() {
            sqlx::query("INSERT INTO story_audience (story_id, user_id) SELECT $1, * FROM unnest($2::uuid[])")
                .bind(story.id)
                .bind(&chosen)
                .execute(&mut *tx)
                .await?;
        }
        events::emit(
            &mut *tx,
            STORY_CREATED,
            Some(owner),
            &StoryCreated {
                story_id: story.id,
                owner_id: owner,
                audience: story.audience,
                expires_at: story.expires_at,
            },
        )
        .await?;
        outbound::activity(&mut *tx, &copy::story_activity(&story)).await?;
        tx.commit().await?;

        // Everyone the story is shown to gets it as they will see it:
        // unwatched, without the owner's numbers.
        let summary = tokuly_identity::summary(core, owner).await?;
        let recipients = if story.is_close_friends() { chosen } else { viewer.friends().to_vec() };
        let for_friends = story_proto(&story, summary, None, &StoryExtras::default());
        self.push_users(recipients, Push::StoryPosted(Box::new(for_friends))).await;
        // The owner's other devices get the owner's view.
        if let Some(own) = self.stories_to_proto(&viewer, std::slice::from_ref(&story)).await?.pop() {
            self.push_user(owner, Push::StoryPosted(Box::new(own))).await;
        }
        core.analytics
            .track(AnalyticsEvent::new("story.post").user(Some(owner)).subject("story", story.id).props(
                serde_json::json!({
                    "items": story.items().len(),
                    "close_friends": story.is_close_friends(),
                }),
            ))
            .await;
        Ok(story)
    }

    /// Deletes stories inside `tx` with everything that hangs off them
    /// (audience, viewers, reactions, highlight entries — and highlights
    /// left empty), releases their uploads and emits `notes.story.deleted`.
    /// Call [`Self::announce_removed_stories`] after the commit.
    pub(crate) async fn delete_stories(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        story_ids: &[Uuid],
        reason: RemovalReason,
        actor: Option<Uuid>,
    ) -> AppResult<Vec<RemovedStory>> {
        if story_ids.is_empty() {
            return Ok(Vec::new());
        }
        let audience: Vec<(Uuid, Uuid)> =
            sqlx::query_as("SELECT story_id, user_id FROM story_audience WHERE story_id = ANY($1)")
                .bind(story_ids)
                .fetch_all(&mut **tx)
                .await?;
        let gone: Vec<(Uuid, Uuid, i32, bool)> = sqlx::query_as(
            "DELETE FROM stories WHERE id = ANY($1) RETURNING id, owner_id, audience, expires_at > now()",
        )
        .bind(story_ids)
        .fetch_all(&mut **tx)
        .await?;
        let mut removed = Vec::with_capacity(gone.len());
        for (id, owner_id, story_audience, live) in gone {
            assets::detach_holder(&mut **tx, &story_holder(id)).await?;
            events::emit(&mut **tx, STORY_DELETED, actor, &StoryDeleted { story_id: id, owner_id, reason }).await?;
            let chosen = (story_audience == AUDIENCE_CLOSE_FRIENDS).then(|| {
                audience.iter().filter(|(story_id, _)| *story_id == id).map(|(_, user_id)| *user_id).collect()
            });
            removed.push(RemovedStory { id, owner_id, live, chosen });
        }
        // A highlight exists to show stories; one that lost its last story goes too.
        let mut owners: Vec<Uuid> = removed.iter().map(|story| story.owner_id).collect();
        owners.sort();
        owners.dedup();
        sqlx::query(
            "DELETE FROM story_highlights h WHERE h.owner_id = ANY($1)
             AND NOT EXISTS (SELECT 1 FROM story_highlight_items i WHERE i.highlight_id = h.id)",
        )
        .bind(&owners)
        .execute(&mut **tx)
        .await?;
        Ok(removed)
    }

    /// Tells the people a deleted story was being shown to (and its owner's
    /// other devices) that it is gone.
    pub(crate) async fn announce_removed_stories(&self, removed: Vec<RemovedStory>) {
        for story in removed {
            let event = || {
                Push::StoryDeleted(Box::new(StoryRef {
                    story_id: story.id.to_string(),
                    owner_id: story.owner_id.to_string(),
                    ..Default::default()
                }))
            };
            self.push_user(story.owner_id, event()).await;
            if !story.live {
                continue;
            }
            let recipients = match story.chosen {
                Some(chosen) => chosen,
                None => graph::friend_ids(&self.core, story.owner_id).await.unwrap_or_default(),
            };
            self.push_users(recipients, event()).await;
        }
    }

    /// Deletes one of `user_id`'s stories, live or archived.
    pub(crate) async fn delete_own_story(&self, user_id: Uuid, story_id: Uuid) -> AppResult<()> {
        let story = self.own_story(user_id, story_id).await?;
        let mut tx = self.core.db.begin().await?;
        let removed = self.delete_stories(&mut tx, &[story.id], RemovalReason::Author, Some(user_id)).await?;
        tx.commit().await?;
        self.announce_removed_stories(removed).await;
        Ok(())
    }

    /// The story tray: the viewer's own reel first, then friends with
    /// stories the viewer has not watched, then the rest; newest first
    /// within each group.
    pub(crate) async fn story_tray(&self, viewer: &Viewer, user_id: Uuid) -> AppResult<Vec<StoryReel>> {
        let sql = format!(
            "SELECT {STORY_COLUMNS} FROM stories s
             WHERE s.owner_id = ANY($7) AND s.expires_at > now() AND {}
             ORDER BY s.id LIMIT $8",
            sees_story("s")
        );
        let rows: Vec<StoryRow> =
            viewer.query_as(sql).bind(viewer.me_and_friends()).bind(TRAY_LIMIT).fetch_all(&self.core.db).await?;
        let stories = self.stories_to_proto(viewer, &rows).await?;

        // Rows are oldest first, so each reel fills in playing order.
        let mut reels: Vec<(Uuid, StoryReel)> = Vec::new();
        for (row, story) in rows.iter().zip(stories) {
            let position = match reels.iter().position(|(owner, _)| *owner == row.owner_id) {
                Some(position) => position,
                None => {
                    reels.push((row.owner_id, StoryReel { user: story.owner.clone(), ..Default::default() }));
                    reels.len() - 1
                }
            };
            let reel = &mut reels[position].1;
            reel.has_unseen |= !story.viewed;
            reel.latest_at = ts(row.created_at);
            reel.stories.push(story);
        }
        let newest = |reel: &StoryReel| reel.stories.last().map(|story| story.id.clone()).unwrap_or_default();
        reels.sort_by(|(a_owner, a), (b_owner, b)| {
            let rank = |owner: &Uuid, reel: &StoryReel| (*owner != user_id, !reel.has_unseen);
            // Story IDs are time-ordered, so the newest story has the greatest ID.
            rank(a_owner, a).cmp(&rank(b_owner, b)).then_with(|| newest(b).cmp(&newest(a)))
        });
        Ok(reels.into_iter().map(|(_, reel)| reel).collect())
    }

    /// The live stories of one user that the viewer may see, oldest first.
    pub(crate) async fn user_stories(&self, viewer: &Viewer, user_id: Uuid) -> AppResult<Vec<Story>> {
        let sql = format!(
            "SELECT {STORY_COLUMNS} FROM stories s
             WHERE s.owner_id = $7 AND s.expires_at > now() AND {}
             ORDER BY s.id",
            sees_story("s")
        );
        let rows: Vec<StoryRow> = viewer.query_as(sql).bind(user_id).fetch_all(&self.core.db).await?;
        self.stories_to_proto(viewer, &rows).await
    }

    /// Records that `viewer_id` watched a story. Only the first time counts;
    /// the owner's own views do not.
    pub(crate) async fn mark_story_viewed(&self, viewer: &Viewer, viewer_id: Uuid, story_id: Uuid) -> AppResult<()> {
        let story = self.require_story(viewer, story_id).await?;
        if story.owner_id == viewer_id {
            return Ok(());
        }
        let view_count: Option<i32> = sqlx::query_scalar(
            "WITH seen AS (
                 INSERT INTO story_views (story_id, viewer_id) VALUES ($1, $2) ON CONFLICT DO NOTHING RETURNING story_id
             )
             UPDATE stories SET view_count = view_count + 1 WHERE id IN (SELECT story_id FROM seen) RETURNING view_count",
        )
        .bind(story.id)
        .bind(viewer_id)
        .fetch_optional(&self.core.db)
        .await?;
        if let Some(view_count) = view_count {
            self.push_story_views(&story, view_count).await;
            self.core
                .analytics
                .track(
                    AnalyticsEvent::new("story.view")
                        .user(Some(viewer_id))
                        .subject("story", story.id)
                        .owner(story.owner_id),
                )
                .await;
        }
        Ok(())
    }

    async fn push_story_views(&self, story: &StoryRow, view_count: i32) {
        let event = Push::StoryViewed(Box::new(StoryViewCount {
            story_id: story.id.to_string(),
            view_count,
            ..Default::default()
        }));
        self.push_user(story.owner_id, event).await;
    }

    /// Who watched one of `owner`'s stories and how they reacted, most
    /// recent first, with the story's totals.
    pub(crate) async fn story_viewers(
        &self,
        viewer: &Viewer,
        owner: Uuid,
        story_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<StoryViewer>, PageInfo, StoryRow)> {
        let story = self.own_story(owner, story_id).await?;
        let cursor: Option<(DateTime<Utc>, Uuid)> = page.decode()?;
        let (after_time, after_id) = cursor.unzip();
        // People who blocked the owner since (or were blocked) drop out of the list.
        let sql = format!(
            "SELECT v.viewer_id, v.viewed_at, v.reaction FROM story_views v
             JOIN stories s ON s.id = v.story_id
             WHERE v.story_id = $7 AND s.owner_id = $1 AND {not_hidden}
               AND ($8::timestamptz IS NULL OR (v.viewed_at, v.viewer_id) < ($8, $9))
             ORDER BY v.viewed_at DESC, v.viewer_id DESC LIMIT $10",
            not_hidden = not_hidden("v.viewer_id"),
        );
        let mut rows: Vec<(Uuid, DateTime<Utc>, String)> = viewer
            .query_as(sql)
            .bind(story.id)
            .bind(after_time)
            .bind(after_id)
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
        let info = page.finish(&mut rows, |row| (row.1, row.0));
        let user_ids: Vec<Uuid> = rows.iter().map(|row| row.0).collect();
        let users = tokuly_identity::summaries(&self.core, &user_ids).await?;
        let viewers = rows
            .into_iter()
            .map(|(viewer_id, viewed_at, reaction)| StoryViewer {
                user: tokuly_identity::users::pick(&users, viewer_id).into(),
                viewed_at: ts(viewed_at),
                reaction,
                ..Default::default()
            })
            .collect();
        Ok((viewers, info, story))
    }

    /// Sets `user_id`'s reaction to a friend's story (one emoji, replacing
    /// an earlier one). Reacting also counts as having watched it.
    pub(crate) async fn react_to_story(
        &self,
        viewer: &Viewer,
        user_id: Uuid,
        story_id: Uuid,
        emoji: &str,
    ) -> AppResult<()> {
        let core = &self.core;
        let story = self.require_story(viewer, story_id).await?;
        ensure!(
            story.owner_id != user_id,
            precondition,
            "CANNOT_REACT_TO_OWN_STORY",
            "reactions come from the people watching a story"
        );
        ensure!(text::is_single_emoji(emoji), invalid, "INVALID_REACTION", "a reaction is exactly one emoji");
        core.cache
            .rate_limit(&format!("notes:story-react:{user_id}"), REACTIONS_PER_MINUTE, Duration::from_secs(60))
            .await?;

        // Looked up before the transaction opens: while it holds its
        // connection nothing here waits for a second one.
        let actor = tokuly_identity::summary(core, user_id).await?;
        let mut tx = core.db.begin().await?;
        // The story row serializes its counters: two taps arriving together
        // count one view and one reaction.
        let alive: Option<Uuid> = sqlx::query_scalar("SELECT id FROM stories WHERE id = $1 FOR UPDATE")
            .bind(story.id)
            .fetch_optional(&mut *tx)
            .await?;
        alive.or_not_found("STORY_NOT_FOUND", "story not found")?;
        let previous: Option<String> =
            sqlx::query_scalar("SELECT reaction FROM story_views WHERE story_id = $1 AND viewer_id = $2")
                .bind(story.id)
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await?;
        if previous.as_deref() == Some(emoji) {
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO story_views (story_id, viewer_id, reaction, reacted_at) VALUES ($1, $2, $3, now())
             ON CONFLICT (story_id, viewer_id) DO UPDATE SET reaction = EXCLUDED.reaction, reacted_at = now()",
        )
        .bind(story.id)
        .bind(user_id)
        .bind(emoji)
        .execute(&mut *tx)
        .await?;
        let first_view = previous.is_none();
        let first_reaction = previous.as_deref().is_none_or(str::is_empty);
        let view_count: i32 = sqlx::query_scalar("UPDATE stories SET view_count = view_count + $2, reaction_count = reaction_count + $3 WHERE id = $1 RETURNING view_count")
            .bind(story.id)
            .bind(i32::from(first_view))
            .bind(i32::from(first_reaction))
            .fetch_one(&mut *tx)
            .await?;
        outbound::notify(&mut *tx, &copy::story_reaction(&actor, user_id, &story, emoji)).await?;
        tx.commit().await?;

        if first_view {
            self.push_story_views(&story, view_count).await;
        }
        let reaction = StoryReaction {
            story_id: story.id.to_string(),
            user: actor.into(),
            emoji: emoji.to_owned(),
            reacted_at: ts(Utc::now()),
            ..Default::default()
        };
        self.push_user(story.owner_id, Push::StoryReaction(Box::new(reaction))).await;
        Ok(())
    }

    /// Takes `user_id`'s reaction to a story back. Succeeds when there was
    /// none — also on a story that expired since, so a reaction can always
    /// be withdrawn.
    pub(crate) async fn remove_story_reaction(&self, viewer: &Viewer, user_id: Uuid, story_id: Uuid) -> AppResult<()> {
        let visible = viewer.can_see_story(&self.core.db, story_id).await?;
        let mut tx = self.core.db.begin().await?;
        let removed = sqlx::query("UPDATE story_views SET reaction = '', reacted_at = NULL WHERE story_id = $1 AND viewer_id = $2 AND reaction <> ''")
            .bind(story_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            > 0;
        if !removed {
            ensure!(visible, not_found, "STORY_NOT_FOUND", "story not found");
            return Ok(());
        }
        sqlx::query("UPDATE stories SET reaction_count = GREATEST(reaction_count - 1, 0) WHERE id = $1")
            .bind(story_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The days of one month on which `owner` posted stories, counted in
    /// their time zone, each with its picture of the day.
    pub(crate) async fn story_calendar(&self, owner: Uuid, year: i32, month: i32) -> AppResult<Vec<StoryDay>> {
        let time_zone = time_zone_of(&self.core, owner).await?;
        let (from, to) = month_range(time_zone, year, month)
            .ok_or_else(|| AppError::invalid("INVALID_MONTH", "not a month of the calendar"))?;
        let days: Vec<(NaiveDate, i32, Uuid)> = sqlx::query_as(
            "SELECT (created_at AT TIME ZONE $4)::date AS day, count(*)::int, (array_agg(id ORDER BY id))[1]
             FROM stories WHERE owner_id = $1 AND created_at >= $2 AND created_at < $3
             GROUP BY day ORDER BY day",
        )
        .bind(owner)
        .bind(from)
        .bind(to)
        .bind(time_zone.name())
        .fetch_all(&self.core.db)
        .await?;
        let first_ids: Vec<Uuid> = days.iter().map(|day| day.2).collect();
        let sql = format!("SELECT {STORY_COLUMNS} FROM stories s WHERE s.id = ANY($1)");
        let firsts: Vec<StoryRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(&first_ids).fetch_all(&self.core.db).await?;
        let covers: HashMap<Uuid, StoryRow> = firsts.into_iter().map(|row| (row.id, row)).collect();
        Ok(days
            .into_iter()
            .map(|(day, story_count, first_id)| StoryDay {
                date: day.format("%Y-%m-%d").to_string(),
                story_count,
                cover: covers.get(&first_id).and_then(StoryRow::cover).into(),
                ..Default::default()
            })
            .collect())
    }

    /// `owner`'s own stories, live and archived, newest first; optionally
    /// only those posted on one day (`YYYY-MM-DD` in their time zone).
    pub(crate) async fn archived_stories(
        &self,
        viewer: &Viewer,
        owner: Uuid,
        date: &str,
        page: &Page,
    ) -> AppResult<(Vec<Story>, PageInfo)> {
        let range = if date.is_empty() {
            None
        } else {
            let day = NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .map_err(|_| AppError::invalid("INVALID_DATE", "dates look like 2026-03-14"))?;
            Some(day_range(time_zone_of(&self.core, owner).await?, day))
        };
        let (from, to) = range.unzip();
        let cursor: Option<Uuid> = page.decode()?;
        let sql = format!(
            "SELECT {STORY_COLUMNS} FROM stories s
             WHERE s.owner_id = $1 AND ($2::uuid IS NULL OR s.id < $2)
               AND ($3::timestamptz IS NULL OR (s.created_at >= $3 AND s.created_at < $4))
             ORDER BY s.id DESC LIMIT $5"
        );
        let mut rows: Vec<StoryRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(owner)
            .bind(cursor)
            .bind(from)
            .bind(to)
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
        let info = page.finish(&mut rows, |row| row.id);
        Ok((self.stories_to_proto(viewer, &rows).await?, info))
    }
}

/// The stored audience value for a requested `StoryAudience`.
pub(crate) fn requested_audience(value: buffa::EnumValue<tokuly_proto::notes::StoryAudience>) -> i32 {
    use tokuly_proto::notes::StoryAudience;
    match value.as_known() {
        Some(StoryAudience::STORY_AUDIENCE_CLOSE_FRIENDS) => AUDIENCE_CLOSE_FRIENDS,
        // Unspecified (and values from a newer client) mean every friend.
        _ => AUDIENCE_FRIENDS,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Datelike as _;

    use super::*;

    #[test]
    fn days_are_cut_in_the_users_time_zone() {
        let tokyo: Tz = chrono_tz::Asia::Tokyo;
        let day = NaiveDate::from_ymd_opt(2026, 3, 14).unwrap();
        let (from, to) = day_range(tokyo, day);
        assert_eq!(from.to_rfc3339(), "2026-03-13T15:00:00+00:00");
        assert_eq!(to.to_rfc3339(), "2026-03-14T15:00:00+00:00");
        let (from, to) = day_range(chrono_tz::UTC, day);
        assert_eq!((to - from).num_hours(), 24);
    }

    #[test]
    fn days_survive_clock_changes() {
        // Midnight does not exist on this day in Santiago (clocks jump from 00:00 to 01:00).
        let santiago: Tz = chrono_tz::America::Santiago;
        let day = NaiveDate::from_ymd_opt(2024, 9, 8).unwrap();
        let (from, to) = day_range(santiago, day);
        assert_eq!((to - from).num_hours(), 23);
        assert_eq!(from.with_timezone(&santiago).day(), 8);
    }

    #[test]
    fn months_are_validated_and_ranged() {
        let tokyo: Tz = chrono_tz::Asia::Tokyo;
        let (from, to) = month_range(tokyo, 2026, 12).unwrap();
        assert_eq!(from.to_rfc3339(), "2026-11-30T15:00:00+00:00");
        assert_eq!(to.to_rfc3339(), "2026-12-31T15:00:00+00:00");
        assert_eq!((month_range(tokyo, 2024, 2).unwrap().1 - month_range(tokyo, 2024, 2).unwrap().0).num_days(), 29);
        for (year, month) in [(2026, 0), (2026, 13), (1999, 5), (99999, 1), (2026, -1)] {
            assert!(month_range(tokyo, year, month).is_none(), "{year}-{month}");
        }
    }

    #[test]
    fn unknown_audiences_mean_every_friend() {
        use tokuly_proto::notes::StoryAudience;
        assert_eq!(requested_audience(StoryAudience::STORY_AUDIENCE_UNSPECIFIED.into()), AUDIENCE_FRIENDS);
        assert_eq!(requested_audience(StoryAudience::STORY_AUDIENCE_FRIENDS.into()), AUDIENCE_FRIENDS);
        assert_eq!(requested_audience(StoryAudience::STORY_AUDIENCE_CLOSE_FRIENDS.into()), AUDIENCE_CLOSE_FRIENDS);
        assert_eq!(requested_audience(buffa::EnumValue::from(42)), AUDIENCE_FRIENDS);
    }
}
