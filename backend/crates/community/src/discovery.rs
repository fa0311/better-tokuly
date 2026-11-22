//! Finding communities: the directory, what is trending, where friends
//! are, the caller's school, and popular tags. Only listed communities
//! (public and restricted) ever appear here.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, QueryBuilder};
use tokuly_core::prelude::*;
use tokuly_proto::{
    common::PageInfo,
    community::{
        CommunityCategory, CommunityDiscoveryService, DirectorySort, FriendsCommunity, ListDirectoryRequest,
        ListDirectoryResponse, ListFriendsCommunitiesRequest, ListFriendsCommunitiesResponse, ListPopularTagsRequest,
        ListPopularTagsResponse, ListSchoolCommunitiesRequest, ListSchoolCommunitiesResponse, ListTrendingRequest,
        ListTrendingResponse, TagCount,
    },
};
use tokuly_social::graph;

use crate::{
    Community as Domain,
    access::Caller,
    model::{CommunityRow, community_columns},
    text, validate,
};

/// Tags kept per category in the popular-tags cache.
const CACHED_TAGS: i64 = 100;
const TAGS_TTL: Duration = Duration::from_secs(300);
/// Friends named per community in "where my friends are".
const FRIEND_PREVIEW: usize = 3;

/// Keyset of the directory's three orders.
#[derive(Debug, Serialize, Deserialize)]
enum DirectoryCursor {
    Members(i32, Uuid),
    New(Uuid),
    Active(Time, Uuid),
}

/// What the directory is narrowed to.
#[derive(Debug)]
pub(crate) struct DirectoryFilter {
    /// 0 = every category.
    pub category: i32,
    pub tag: Option<String>,
    pub language: String,
    pub query: String,
    pub sort: DirectorySort,
}

#[derive(sqlx::FromRow)]
struct FriendsRow {
    #[sqlx(flatten)]
    community: CommunityRow,
    friend_count: i64,
    friend_ids: Vec<Uuid>,
}

/// A category filter: unspecified = all; unknown numbers match nothing.
fn category_filter(value: buffa::EnumValue<CommunityCategory>) -> i32 {
    value.to_i32()
}

impl Domain {
    /// Listed communities matching `filter`, in its order.
    pub(crate) async fn directory(
        &self,
        filter: &DirectoryFilter,
        page: &Page,
    ) -> AppResult<(Vec<CommunityRow>, PageInfo)> {
        let cursor: Option<DirectoryCursor> = page.decode()?;
        let mut query: QueryBuilder<Postgres> = QueryBuilder::new(concat!(
            "SELECT ",
            community_columns!(),
            " FROM communities c WHERE c.deleted_at IS NULL AND c.visibility <> 3"
        ));
        if filter.category != 0 {
            query.push(" AND c.category = ").push_bind(filter.category);
        }
        if let Some(tag) = &filter.tag {
            query.push(" AND c.tags @> ARRAY[").push_bind(tag.clone()).push("]::text[]");
        }
        if !filter.language.is_empty() {
            query.push(" AND c.language = ").push_bind(filter.language.clone());
        }
        if !filter.query.is_empty() {
            let pattern = text::like_pattern(&filter.query);
            query
                .push(" AND (c.name ILIKE ")
                .push_bind(pattern.clone())
                .push(" OR c.handle::text ILIKE ")
                .push_bind(pattern.clone())
                .push(" OR c.description ILIKE ")
                .push_bind(pattern)
                .push(")");
        }
        match filter.sort {
            DirectorySort::DIRECTORY_SORT_NEW => {
                if let Some(DirectoryCursor::New(id)) = cursor {
                    query.push(" AND c.id < ").push_bind(id);
                }
                query.push(" ORDER BY c.id DESC");
            }
            DirectorySort::DIRECTORY_SORT_ACTIVE => {
                if let Some(DirectoryCursor::Active(at, id)) = cursor {
                    query.push(" AND (c.last_activity_at, c.id) < (").push_bind(at).push(", ").push_bind(id).push(")");
                }
                query.push(" ORDER BY c.last_activity_at DESC, c.id DESC");
            }
            _ => {
                if let Some(DirectoryCursor::Members(count, id)) = cursor {
                    query.push(" AND (c.member_count, c.id) < (").push_bind(count).push(", ").push_bind(id).push(")");
                }
                query.push(" ORDER BY c.member_count DESC, c.id DESC");
            }
        }
        query.push(" LIMIT ").push_bind(page.fetch());
        let mut rows: Vec<CommunityRow> = query.build_query_as().fetch_all(&self.core.db).await?;
        let info = page.finish(&mut rows, |row| match filter.sort {
            DirectorySort::DIRECTORY_SORT_NEW => DirectoryCursor::New(row.id),
            DirectorySort::DIRECTORY_SORT_ACTIVE => DirectoryCursor::Active(row.last_activity_at, row.id),
            _ => DirectoryCursor::Members(row.member_count, row.id),
        });
        Ok((rows, info))
    }

    /// Listed communities with the highest trend score (see the trend worker).
    pub(crate) async fn trending(&self, category: i32, limit: i64) -> AppResult<Vec<CommunityRow>> {
        Ok(sqlx::query_as(concat!(
            "SELECT ",
            community_columns!(),
            " FROM communities c
              WHERE c.deleted_at IS NULL AND c.visibility <> 3 AND c.trend_score > 0 AND ($1 = 0 OR c.category = $1)
              ORDER BY c.trend_score DESC, c.id DESC LIMIT $2"
        ))
        .bind(category)
        .bind(limit)
        .fetch_all(&self.core.db)
        .await?)
    }

    /// Public communities friends of `user_id` are in and show on their
    /// profile, most friends first: `(community, friends in it, a few of them)`.
    pub(crate) async fn friends_communities(
        &self,
        user_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<(CommunityRow, i64, Vec<Uuid>)>, PageInfo)> {
        let friends = graph::friend_ids(&self.core, user_id).await?;
        if friends.is_empty() {
            return Ok((Vec::new(), PageInfo::default()));
        }
        let offset: i64 = page.decode()?.unwrap_or(0);
        let mut rows: Vec<FriendsRow> = sqlx::query_as(concat!(
            "SELECT ",
            community_columns!(),
            ", f.friend_count, f.friend_ids
              FROM (
                  SELECT m.community_id, count(*) AS friend_count,
                         (array_agg(m.user_id ORDER BY m.joined_at))[1:$5] AS friend_ids
                  FROM community_members m
                  WHERE m.user_id = ANY($1) AND m.show_on_profile
                  GROUP BY m.community_id
              ) f
              JOIN communities c ON c.id = f.community_id
              WHERE c.visibility = 1 AND c.deleted_at IS NULL
                AND NOT EXISTS (SELECT 1 FROM community_members me WHERE me.community_id = c.id AND me.user_id = $2)
                AND NOT EXISTS (SELECT 1 FROM community_bans b WHERE b.community_id = c.id AND b.user_id = $2
                                AND (b.expires_at IS NULL OR b.expires_at > now()))
              ORDER BY f.friend_count DESC, c.id DESC
              LIMIT $3 OFFSET $4"
        ))
        .bind(&friends)
        .bind(user_id)
        .bind(page.fetch())
        .bind(offset)
        .bind(FRIEND_PREVIEW as i32)
        .fetch_all(&self.core.db)
        .await?;
        let has_more = rows.len() as i64 > page.limit;
        rows.truncate(page.limit as usize);
        let info = PageInfo {
            next_cursor: if has_more {
                tokuly_core::page::encode_cursor(&(offset + page.limit))
            } else {
                String::new()
            },
            has_more,
            ..Default::default()
        };
        Ok((rows.into_iter().map(|row| (row.community, row.friend_count, row.friend_ids)).collect(), info))
    }

    /// School and club communities of the school `user_id` put on their
    /// profile, largest first. Empty (with an empty name) when they set none.
    pub(crate) async fn school_communities(
        &self,
        user_id: Uuid,
        page: &Page,
    ) -> AppResult<(String, Vec<CommunityRow>, PageInfo)> {
        let school: String = sqlx::query_scalar("SELECT school_name FROM users WHERE id = $1 AND deleted_at IS NULL")
            .bind(user_id)
            .fetch_optional(&self.core.db)
            .await?
            .unwrap_or_default();
        if school.is_empty() {
            return Ok((school, Vec::new(), PageInfo::default()));
        }
        let after: Option<(i32, Uuid)> = page.decode()?;
        let mut rows: Vec<CommunityRow> = sqlx::query_as(concat!(
            "SELECT ",
            community_columns!(),
            " FROM communities c
              WHERE c.school_name = $1 AND c.school_name <> '' AND c.category IN ($2, $3)
                AND c.deleted_at IS NULL AND c.visibility <> 3
                AND ($4::int IS NULL OR (c.member_count, c.id) < ($4, $5))
              ORDER BY c.member_count DESC, c.id DESC LIMIT $6"
        ))
        .bind(&school)
        .bind(CommunityCategory::COMMUNITY_CATEGORY_SCHOOL as i32)
        .bind(CommunityCategory::COMMUNITY_CATEGORY_CLUB as i32)
        .bind(after.map(|(count, _)| count))
        .bind(after.map(|(_, id)| id))
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.member_count, row.id));
        Ok((school, rows, info))
    }

    /// The most used tags among listed communities of a category (0 = all),
    /// recomputed at most every few minutes.
    pub(crate) async fn popular_tags(&self, category: i32, limit: usize) -> AppResult<Vec<(String, i64)>> {
        let key = format!("community:tags:{category}");
        let cached: Option<Vec<(String, i64)>> = self.core.cache.get_json(&key).await.unwrap_or(None);
        let tags = match cached {
            Some(tags) => tags,
            None => {
                let tags: Vec<(String, i64)> = sqlx::query_as(
                    "SELECT tag, count(*) AS uses FROM communities c, unnest(c.tags) AS tag
                     WHERE c.deleted_at IS NULL AND c.visibility <> 3 AND ($1 = 0 OR c.category = $1)
                     GROUP BY tag ORDER BY uses DESC, tag LIMIT $2",
                )
                .bind(category)
                .bind(CACHED_TAGS)
                .fetch_all(&self.core.db)
                .await?;
                if let Err(error) = self.core.cache.set_json(&key, &tags, Some(TAGS_TTL)).await {
                    tracing::warn!(%error, "caching popular community tags failed");
                }
                tags
            }
        };
        Ok(tags.into_iter().take(limit).collect())
    }
}

impl Domain {
    /// One allowance for all the discovery lists: per account, or per
    /// address for callers who are not signed in.
    async fn browse_limit(&self, ctx: &RequestContext, caller: &Caller) -> AppResult<()> {
        let who = caller.user_id.map(|id| id.to_string()).unwrap_or_else(|| ctx.client().ip);
        self.limit("directory", who, 120, Duration::from_secs(60)).await
    }
}

impl CommunityDiscoveryService for Domain {
    async fn list_directory(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListDirectoryRequest>,
    ) -> ServiceResult<ListDirectoryResponse> {
        handle(async {
            let caller = Caller::of(&ctx)?;
            let req = request.to_owned_message();
            let query = validate::query(&req.query)?;
            let tag = if req.tag.trim().is_empty() { None } else { Some(validate::tag(&req.tag)?) };
            let language =
                if req.language.trim().is_empty() { String::new() } else { validate::language(&req.language)? };
            let sort = match req.sort.as_known() {
                Some(DirectorySort::DIRECTORY_SORT_UNSPECIFIED) => DirectorySort::DIRECTORY_SORT_MEMBERS,
                Some(sort) => sort,
                None => bail!(invalid, "SORT_INVALID", "choose members, new or active"),
            };
            self.browse_limit(&ctx, &caller).await?;
            let filter = DirectoryFilter { category: category_filter(req.category), tag, language, query, sort };
            let page = Page::from_request(req.page.as_option(), 50);
            let (rows, info) = self.directory(&filter, &page).await?;
            ok(ListDirectoryResponse {
                communities: self.render_communities(&caller, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_trending(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListTrendingRequest>,
    ) -> ServiceResult<ListTrendingResponse> {
        handle(async {
            let caller = Caller::of(&ctx)?;
            self.browse_limit(&ctx, &caller).await?;
            let limit = if request.limit <= 0 { 20 } else { i64::from(request.limit.min(50)) };
            let rows = self.trending(category_filter(request.category), limit).await?;
            ok(ListTrendingResponse {
                communities: self.render_communities(&caller, &rows).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn list_friends_communities(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListFriendsCommunitiesRequest>,
    ) -> ServiceResult<ListFriendsCommunitiesResponse> {
        handle(async {
            let (caller, user_id) = Caller::signed_in(&ctx)?;
            self.browse_limit(&ctx, &caller).await?;
            let page = Page::from_view(request.page.as_option(), 50);
            let (found, info) = self.friends_communities(user_id, &page).await?;
            let rows: Vec<CommunityRow> = found.iter().map(|(community, ..)| community.clone()).collect();
            let rendered = self.render_communities(&caller, &rows).await?;
            let friend_ids: Vec<Uuid> = found.iter().flat_map(|(_, _, ids)| ids.iter().copied()).collect();
            let friends = tokuly_identity::summaries(&self.core, &friend_ids).await?;
            let communities = found
                .into_iter()
                .zip(rendered)
                .map(|((_, friend_count, ids), community)| FriendsCommunity {
                    community: community.into(),
                    friends: ids.iter().filter_map(|id| friends.get(id).cloned()).collect(),
                    friend_count: friend_count as i32,
                    ..Default::default()
                })
                .collect();
            ok(ListFriendsCommunitiesResponse { communities, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn list_school_communities(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListSchoolCommunitiesRequest>,
    ) -> ServiceResult<ListSchoolCommunitiesResponse> {
        handle(async {
            let (caller, user_id) = Caller::signed_in(&ctx)?;
            self.browse_limit(&ctx, &caller).await?;
            let page = Page::from_view(request.page.as_option(), 50);
            let (school_name, rows, info) = self.school_communities(user_id, &page).await?;
            ok(ListSchoolCommunitiesResponse {
                communities: self.render_communities(&caller, &rows).await?,
                page: info.into(),
                school_name,
                ..Default::default()
            })
        })
        .await
    }

    async fn list_popular_tags(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListPopularTagsRequest>,
    ) -> ServiceResult<ListPopularTagsResponse> {
        handle(async {
            self.browse_limit(&ctx, &Caller::of(&ctx)?).await?;
            let limit = if request.limit <= 0 { 30 } else { request.limit.min(100) as usize };
            let tags = self.popular_tags(category_filter(request.category), limit).await?;
            ok(ListPopularTagsResponse {
                tags: tags
                    .into_iter()
                    .map(|(tag, count)| TagCount { tag, community_count: count as i32, ..Default::default() })
                    .collect(),
                ..Default::default()
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_round_trip_per_order() {
        let id = Uuid::now_v7();
        for cursor in [DirectoryCursor::Members(12, id), DirectoryCursor::New(id), DirectoryCursor::Active(now(), id)] {
            let encoded = tokuly_core::page::encode_cursor(&cursor);
            let decoded: DirectoryCursor = tokuly_core::page::decode_cursor(&encoded).unwrap();
            assert_eq!(serde_json::to_string(&cursor).unwrap(), serde_json::to_string(&decoded).unwrap());
        }
    }
}
