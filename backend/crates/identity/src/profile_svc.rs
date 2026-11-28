//! `ProfileService`: public profiles, user lookup and search, profile views.
use std::time::Duration;

use chrono::{DateTime, Utc};
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_media::assets::image_from_json;
use tokuly_proto::{common::UserSummary, identity::*};

use crate::{
    Identity, birthdays, extras, settings,
    users::{self, UserRow},
};

impl Identity {
    /// Whether `viewer` may see `owner` at all, per every registered domain.
    pub async fn can_view(&self, viewer: Option<Uuid>, owner: Uuid) -> AppResult<bool> {
        let Some(viewer) = viewer else {
            return Ok(true);
        };
        if viewer == owner {
            return Ok(true);
        }
        for extension in self.profile_extensions() {
            if !extension.can_view(viewer, owner).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Those among `user_ids` that `viewer` may not see, per every registered
    /// domain: [`Self::can_view`] for a whole list, one question per domain.
    pub async fn hidden_among(&self, viewer: Uuid, user_ids: &[Uuid]) -> AppResult<std::collections::HashSet<Uuid>> {
        let mut hidden = std::collections::HashSet::new();
        if user_ids.is_empty() {
            return Ok(hidden);
        }
        for extension in self.profile_extensions() {
            hidden.extend(extension.hidden_among(viewer, user_ids).await?);
        }
        hidden.remove(&viewer);
        Ok(hidden)
    }

    /// The profile of `user` as `viewer` sees it.
    pub async fn profile_of(&self, user: &UserRow, viewer: Option<Uuid>) -> AppResult<Profile> {
        let days = (Utc::now() - user.created_at).num_days().max(0) as i32;
        let mut profile = Profile {
            user: user.summary().into(),
            bio: user.bio.clone(),
            banner: image_from_json(user.banner.as_ref()).into(),
            joined_at: ts(user.created_at),
            status: user.status().into(),
            relationship: if viewer == Some(user.id) {
                Relationship::RELATIONSHIP_SELF
            } else {
                Relationship::RELATIONSHIP_NONE
            }
            .into(),
            stats: ProfileStats {
                profile_view_count: user.profile_view_count,
                days_since_joined: days,
                ..Default::default()
            }
            .into(),
            school_name: user.school_name.clone(),
            ..Default::default()
        };
        for extension in self.profile_extensions() {
            extension.enrich_profile(viewer, user.id, &mut profile).await?;
        }
        // Cards come from several domains; their own `order` decides the layout.
        profile.cards.sort_by_key(|card| card.order);
        // Music is shared with friends (and shown to yourself), like the map pin.
        let close = matches!(
            profile.relationship.as_known(),
            Some(Relationship::RELATIONSHIP_SELF | Relationship::RELATIONSHIP_FRIENDS)
        );
        if close && settings::privacy_from_json(&user.privacy).share_now_playing {
            profile.now_playing = self.now_playing(user.id).await.into();
        }
        // The プロフ帳, by what the viewer is to the owner (blocks never get
        // here: such a viewer cannot see the profile at all).
        let book = extras::Extras::load(&self.core, user.id, user.birthdate).await?;
        let as_viewer = match profile.relationship.as_known() {
            Some(Relationship::RELATIONSHIP_SELF) => extras::Viewer::Owner,
            _ => self.audience_viewer(viewer, user.id, &book.card_circles(), profile.relationship.as_known()).await?,
        };
        profile.extras = book.view(&as_viewer).into();
        let closeness = match &as_viewer {
            extras::Viewer::Owner => birthdays::Closeness::Owner,
            extras::Viewer::Audience(tie) if tie.tier == extras::Tier::Friend => birthdays::Closeness::Friend,
            _ => birthdays::Closeness::Other,
        };
        if let Some(birthdate) = user.birthdate
            && let Some(shared) =
                birthdays::share(user.id, birthdate, &settings::privacy_from_json(&user.privacy), closeness)
        {
            profile.birthday = BirthdayView {
                month: shared.month as i32,
                day: shared.day as i32,
                year: shared.year.unwrap_or(0),
                ..Default::default()
            }
            .into();
        }
        profile.bot = user.bot;
        if let Some(owner) = user.bot_owner_id {
            // The person behind a bot, unless they and the viewer are in a block.
            let hidden = match viewer {
                Some(viewer) => self.hidden_among(viewer, &[owner]).await?.contains(&owner),
                None => false,
            };
            if !hidden {
                profile.bot_owner = users::summary(&self.core, owner).await?.into();
            }
        }
        Ok(profile)
    }

    /// What `viewer` is to `owner` for the audiences of the プロフ帳, from the
    /// profile extensions; without one that knows (no social domain), a
    /// friend is who `relationship` says and nobody is locked.
    pub(crate) async fn audience_viewer(
        &self,
        viewer: Option<Uuid>,
        owner: Uuid,
        circles: &[Uuid],
        relationship: Option<Relationship>,
    ) -> AppResult<extras::Viewer> {
        if viewer == Some(owner) {
            return Ok(extras::Viewer::Owner);
        }
        for extension in self.profile_extensions() {
            if let Some(tie) = extension.audience_tie(viewer, owner, circles).await? {
                return Ok(extras::Viewer::Audience(tie));
            }
        }
        let tier = match relationship {
            Some(Relationship::RELATIONSHIP_FRIENDS) => extras::Tier::Friend,
            _ => extras::Tier::Other,
        };
        Ok(extras::Viewer::Audience(extras::AudienceTie { tier, ..Default::default() }))
    }

    /// Which of `owners` have locked accounts, per every registered domain.
    pub async fn locked_among(&self, owners: &[Uuid]) -> AppResult<std::collections::HashSet<Uuid>> {
        let mut locked = std::collections::HashSet::new();
        if owners.is_empty() {
            return Ok(locked);
        }
        for extension in self.profile_extensions() {
            locked.extend(extension.locked_among(owners).await?);
        }
        Ok(locked)
    }

    /// The プロフ帳 of `user_id` as far as it reaches anybody — every field
    /// and card whatever its audience, with the audiences, but nothing the
    /// owner keeps to themself ("only me", hidden cards): for a moderator handling a
    /// report about the account. Not an RPC: the admin domain calls it after
    /// checking that an open case is about this account and that the
    /// moderator is not a party to it, and records the look in its audit
    /// log. Suspended accounts included; `USER_NOT_FOUND` for erased ones.
    pub async fn extras_for_moderation(&self, user_id: Uuid) -> AppResult<ProfileExtras> {
        let user = users::get(&self.core, user_id).await?;
        ensure!(user.deleted_at.is_none(), not_found, "USER_NOT_FOUND", "user not found");
        Ok(extras::Extras::load(&self.core, user.id, user.birthdate).await?.view(&extras::Viewer::Moderator))
    }

    /// Relationship and mutual-friend count of `viewer` to each of
    /// `user_ids`, per the registered extensions (the viewer is SELF).
    pub(crate) async fn relationships_of(
        &self,
        viewer: Uuid,
        user_ids: &[Uuid],
    ) -> AppResult<std::collections::HashMap<Uuid, (Relationship, i32)>> {
        let mut out = std::collections::HashMap::new();
        for extension in self.profile_extensions() {
            out.extend(extension.relationships(viewer, user_ids).await?);
        }
        if user_ids.contains(&viewer) {
            out.insert(viewer, (Relationship::RELATIONSHIP_SELF, 0));
        }
        Ok(out)
    }
}

#[derive(sqlx::FromRow)]
struct SearchRow {
    id: Uuid,
    handle: String,
    display_name: String,
    avatar: Option<serde_json::Value>,
    avatar_video_url: Option<String>,
    verified: bool,
}

impl ProfileService for Identity {
    async fn get_profile(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetProfileRequest>,
    ) -> ServiceResult<GetProfileResponse> {
        handle(async {
            use tokuly_proto::identity::get_profile_request::By;
            let viewer = ctx.maybe_user_id()?;
            let req = request.to_owned_message();
            let user = match req.by {
                Some(By::UserId(id)) => users::find_by_id(&self.core, parse_id(&id)?).await?,
                Some(By::Handle(handle)) => users::find_by_handle(&self.core, &handle).await?,
                None => bail!(invalid, "MISSING_TARGET", "user_id or handle is required"),
            };
            let user = user.filter(|u| u.is_active()).or_not_found("USER_NOT_FOUND", "user not found")?;
            ensure!(self.can_view(viewer, user.id).await?, not_found, "USER_NOT_FOUND", "user not found");
            ok(GetProfileResponse { profile: self.profile_of(&user, viewer).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn batch_get_users(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, BatchGetUsersRequest>,
    ) -> ServiceResult<BatchGetUsersResponse> {
        handle(async {
            let ids: Vec<Uuid> = request.user_ids.iter().map(|id| parse_id(id)).collect::<AppResult<_>>()?;
            ensure!(ids.len() <= 200, invalid, "TOO_MANY_IDS", "at most 200 users per request");
            let users = users::summaries_ordered(&self.core, &ids).await?;
            ok(BatchGetUsersResponse { users, ..Default::default() })
        })
        .await
    }

    async fn search_users(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SearchUsersRequest>,
    ) -> ServiceResult<SearchUsersResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            let limiter = viewer.map(|u| u.to_string()).unwrap_or_else(|| ctx.client().ip);
            self.core.cache.rate_limit(&format!("user-search:{limiter}"), 120, Duration::from_secs(60)).await?;
            let query = request.query.trim().trim_start_matches('@');
            if query.is_empty() {
                return ok(SearchUsersResponse::default());
            }
            ensure!(query.chars().count() <= 64, invalid, "QUERY_TOO_LONG", "search query is too long");
            let page = Page::from_view(request.page.as_option(), 50);
            let offset: i64 = page.decode()?.unwrap_or(0);
            // Exact handle first, then prefix matches, then fuzzy (trigram) matches.
            let pattern = format!("%{}%", query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
            let mut rows: Vec<SearchRow> = sqlx::query_as(
                "SELECT id, handle::text AS handle, display_name, avatar, avatar_video_url, verified
             FROM users
             WHERE deleted_at IS NULL AND banned_at IS NULL
               AND (handle::text ILIKE $2 OR display_name ILIKE $2 OR handle::text % $1 OR display_name % $1)
             ORDER BY (handle = $1::citext) DESC,
                      (handle::text ILIKE $1 || '%') DESC,
                      greatest(similarity(handle::text, $1), similarity(display_name, $1)) DESC,
                      id
             LIMIT $3 OFFSET $4",
            )
            .bind(query)
            .bind(&pattern)
            .bind(page.fetch())
            .bind(offset)
            .fetch_all(&self.core.db)
            .await
            .map_err(AppError::from)?;
            let has_more = rows.len() as i64 > page.limit;
            rows.truncate(page.limit as usize);

            let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
            let relationships = match viewer {
                Some(viewer) => self.relationships_of(viewer, &ids).await?,
                None => Default::default(),
            };
            let bots = crate::bots::bots_among(&self.core, &ids).await?;
            let hidden = match viewer {
                Some(viewer) => self.hidden_among(viewer, &ids).await?,
                None => Default::default(),
            };
            let mut results = Vec::with_capacity(rows.len());
            for row in rows {
                let (relationship, mutual) =
                    relationships.get(&row.id).copied().unwrap_or((Relationship::RELATIONSHIP_NONE, 0));
                // People who blocked the searcher (or were blocked) do not show up.
                if relationship == Relationship::RELATIONSHIP_BLOCKED || hidden.contains(&row.id) {
                    continue;
                }
                results.push(UserSearchResult {
                    user: UserSummary {
                        id: row.id.to_string(),
                        handle: row.handle,
                        display_name: row.display_name,
                        avatar: image_from_json(row.avatar.as_ref()).into(),
                        avatar_video_url: row.avatar_video_url.unwrap_or_default(),
                        verified: row.verified,
                        bot: bots.contains(&row.id),
                        ..Default::default()
                    }
                    .into(),
                    relationship: relationship.into(),
                    mutual_friend_count: mutual,
                    bot: bots.contains(&row.id),
                    ..Default::default()
                });
            }
            let next_cursor =
                if has_more { tokuly_core::page::encode_cursor(&(offset + page.limit)) } else { String::new() };
            ok(SearchUsersResponse {
                results,
                page: tokuly_proto::common::PageInfo { next_cursor, has_more, ..Default::default() }.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn record_profile_view(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RecordProfileViewRequest>,
    ) -> ServiceResult<RecordProfileViewResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let viewed = parse_id(request.user_id)?;
            // A blocked pair does not see each other, so a look is not recorded
            // either (the answer is the same, so it does not tell about the block).
            if viewer != viewed && self.can_view(Some(viewer), viewed).await? {
                // One counted view per viewer per day.
                let inserted = sqlx::query(
                    "INSERT INTO profile_views (viewed_id, viewer_id, day)
                 SELECT $1, $2, current_date WHERE EXISTS (SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL)
                 ON CONFLICT DO NOTHING",
                )
                .bind(viewed)
                .bind(viewer)
                .execute(&self.core.db)
                .await
                .map_err(AppError::from)?;
                if inserted.rows_affected() == 1 {
                    sqlx::query("UPDATE users SET profile_view_count = profile_view_count + 1 WHERE id = $1")
                        .bind(viewed)
                        .execute(&self.core.db)
                        .await
                        .map_err(AppError::from)?;
                }
            }
            ok(RecordProfileViewResponse::default())
        })
        .await
    }

    async fn get_profile_views(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetProfileViewsRequest>,
    ) -> ServiceResult<GetProfileViewsResponse> {
        handle(async {
        let user_id = ctx.user_id()?;
        let (total, last_24h): (i64, i64) = sqlx::query_as(
            "SELECT (SELECT profile_view_count FROM users WHERE id = $1),
                    (SELECT count(*) FROM profile_views WHERE viewed_id = $1 AND viewed_at > now() - interval '24 hours')",
        )
        .bind(user_id)
        .fetch_one(&self.core.db)
        .await
        .map_err(AppError::from)?;
        let recent: Vec<(Uuid, DateTime<Utc>)> = sqlx::query_as(
            "SELECT DISTINCT ON (viewer_id) viewer_id, viewed_at FROM profile_views
             WHERE viewed_id = $1 AND viewed_at > now() - interval '30 days'
             ORDER BY viewer_id, viewed_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.core.db)
        .await
        .map_err(AppError::from)?;
        let mut recent = recent;
        recent.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
        recent.truncate(50);
        // Only viewers who share their views are named; the rest just count.
        // Suspended accounts and anyone in a block with the owner are never named.
        let ids: Vec<Uuid> = recent.iter().map(|(id, _)| *id).collect();
        let privacy = settings::privacy_of_many(&self.core, &ids).await?;
        let active: std::collections::HashSet<Uuid> = users::filter_active(&self.core, &ids).await?.into_iter().collect();
        let hidden = self.hidden_among(user_id, &ids).await?;
        let summaries = users::summaries(&self.core, &ids).await?;
        let viewers = recent
            .into_iter()
            .filter(|(id, _)| active.contains(id) && !hidden.contains(id) && privacy.get(id).is_some_and(|p| p.share_views))
            .filter_map(|(id, at)| Some(ProfileViewer { user: summaries.get(&id)?.clone().into(), viewed_at: ts(at), ..Default::default() }))
            .collect();
        ok(GetProfileViewsResponse { total, last_24h: last_24h as i32, viewers, ..Default::default() })
        })
        .await
    }

    async fn list_profile_prompts(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, ListProfilePromptsRequest>,
    ) -> ServiceResult<ListProfilePromptsResponse> {
        handle(async { ok(ListProfilePromptsResponse { prompts: extras::prompt_messages(), ..Default::default() }) })
            .await
    }

    async fn suggest_profile_tags(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SuggestProfileTagsRequest>,
    ) -> ServiceResult<SuggestProfileTagsResponse> {
        handle(async {
            let limiter = ctx.maybe_user_id()?.map(|u| u.to_string()).unwrap_or_else(|| ctx.client().ip);
            self.core.cache.rate_limit(&format!("tag-suggest:{limiter}"), 120, Duration::from_secs(60)).await?;
            let query = request.query.trim();
            ensure!(query.chars().count() <= 24, invalid, "QUERY_TOO_LONG", "search query is too long");
            let category = match request.category.as_known() {
                Some(ProfileTagCategory::PROFILE_TAG_CATEGORY_UNSPECIFIED) | None => {
                    bail!(invalid, "TAG_INVALID", "a category is required")
                }
                Some(category) => category as i32,
            };
            let suggestions = self.suggest_tags(category, query).await?;
            ok(SuggestProfileTagsResponse { suggestions, ..Default::default() })
        })
        .await
    }
}
