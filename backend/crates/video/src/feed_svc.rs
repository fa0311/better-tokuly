//! `VideoFeedService`: home, subscriptions, trending, related, "not interested".
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::{common::PageInfo, video::*};

use crate::{Video, access::Viewer, channels, feeds, validate, videos};

/// What a "not interested" names.
enum Target {
    Video(Uuid),
    Channel(Uuid),
}

impl Video {
    /// Resolves the target of a "not interested" request. With `must_exist`
    /// the caller has to be able to see it; an undo only needs to name it.
    async fn feed_target(
        &self,
        viewer: &Viewer,
        video: Option<&str>,
        channel: Option<&str>,
        must_exist: bool,
    ) -> AppResult<Option<Target>> {
        let core = &self.core;
        match (video, channel) {
            (Some(reference), _) => {
                let found = videos::find_ref(core, reference)
                    .await?
                    .filter(|video| !must_exist || viewer.can_watch(&video.facts()));
                match found {
                    Some(video) => Ok(Some(Target::Video(video.id))),
                    None if must_exist => Err(AppError::not_found("VIDEO_NOT_FOUND", "video not found")),
                    None => Ok(None),
                }
            }
            (None, Some(reference)) => {
                let found = channels::find(core, reference)
                    .await?
                    .filter(|channel| !must_exist || viewer.may_see_owner(channel.owner_id, channel.suspended));
                match found {
                    Some(channel) => Ok(Some(Target::Channel(channel.id))),
                    None if must_exist => Err(AppError::not_found("CHANNEL_NOT_FOUND", "channel not found")),
                    None => Ok(None),
                }
            }
            (None, None) => Err(AppError::invalid("TARGET_REQUIRED", "a video or a channel is required")),
        }
    }
}

impl VideoFeedService for Video {
    async fn get_home_feed(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetHomeFeedRequest>,
    ) -> ServiceResult<GetHomeFeedResponse> {
        handle(async {
            let viewer = Viewer::load(&self.core, ctx.maybe_user_id()?).await?;
            // Every visit ranks a feed of its own, which is not free.
            let limiter = viewer.user_id.map(|id| id.to_string()).unwrap_or_else(|| ctx.client().ip);
            self.core.cache.rate_limit(&format!("video-home:{limiter}"), 120, Duration::from_secs(60)).await?;
            let page = Page::from_view(request.page.as_option(), 50);
            let (rows, next) = feeds::home_page(&self.core, &viewer, &page).await?;
            let info =
                PageInfo { has_more: next.is_some(), next_cursor: next.unwrap_or_default(), ..Default::default() };
            ok(GetHomeFeedResponse {
                videos: videos::summaries(&self.core, &viewer, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_subscriptions_feed(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetSubscriptionsFeedRequest>,
    ) -> ServiceResult<GetSubscriptionsFeedResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(&self.core, Some(user_id)).await?;
            let page = Page::from_view(request.page.as_option(), 50);
            let before: Option<(Time, Uuid)> = page.decode()?;
            let mut rows = feeds::subscriptions_page(&self.core, &viewer, user_id, before, page.fetch()).await?;
            let info = page.finish(&mut rows, |row| (row.published_at.unwrap_or(row.created_at), row.id));
            ok(GetSubscriptionsFeedResponse {
                videos: videos::summaries(&self.core, &viewer, &rows).await?,
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
            let viewer = Viewer::load(&self.core, ctx.maybe_user_id()?).await?;
            let category = validate::category(request.category)?;
            let page = Page::from_view(request.page.as_option(), 50);
            let offset: i64 = page.decode()?.unwrap_or(0);
            ensure!(
                (0..=feeds::TRENDING_MAX_OFFSET).contains(&offset),
                invalid,
                "INVALID_CURSOR",
                "invalid page cursor"
            );
            let mut rows = feeds::trending_page(&self.core, &viewer, category, offset, page.fetch()).await?;
            let has_more = rows.len() as i64 > page.limit && offset + page.limit <= feeds::TRENDING_MAX_OFFSET;
            rows.truncate(page.limit as usize);
            let next_cursor =
                if has_more { tokuly_core::page::encode_cursor(&(offset + page.limit)) } else { String::new() };
            let info = PageInfo { next_cursor, has_more, ..Default::default() };
            ok(ListTrendingResponse {
                videos: videos::summaries(&self.core, &viewer, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_related_videos(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListRelatedVideosRequest>,
    ) -> ServiceResult<ListRelatedVideosResponse> {
        handle(async {
            let viewer = Viewer::load(&self.core, ctx.maybe_user_id()?).await?;
            let video = videos::find_watchable(&self.core, &viewer, request.video_id).await?;
            let limit = if request.limit <= 0 { 20 } else { request.limit.min(50) } as i64;
            let rows = feeds::related(&self.core, &viewer, &video, limit).await?;
            ok(ListRelatedVideosResponse {
                videos: videos::summaries(&self.core, &viewer, &rows).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn mark_not_interested(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, MarkNotInterestedRequest>,
    ) -> ServiceResult<MarkNotInterestedResponse> {
        handle(async {
            use mark_not_interested_request::Target as Named;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(&self.core, Some(user_id)).await?;
            let req = request.to_owned_message();
            let (video, channel) = match &req.target {
                Some(Named::VideoId(id)) => (Some(id.as_str()), None),
                Some(Named::ChannelId(id)) => (None, Some(id.as_str())),
                None => (None, None),
            };
            match self.feed_target(&viewer, video, channel, true).await? {
                Some(Target::Video(video_id)) => {
                    sqlx::query(
                        "INSERT INTO video_feed_hidden (user_id, video_id) VALUES ($1, $2)
                         ON CONFLICT (user_id, video_id) WHERE video_id IS NOT NULL DO NOTHING",
                    )
                    .bind(user_id)
                    .bind(video_id)
                    .execute(&self.core.db)
                    .await?;
                }
                Some(Target::Channel(channel_id)) => {
                    sqlx::query(
                        "INSERT INTO video_feed_hidden (user_id, channel_id) VALUES ($1, $2)
                         ON CONFLICT (user_id, channel_id) WHERE channel_id IS NOT NULL DO NOTHING",
                    )
                    .bind(user_id)
                    .bind(channel_id)
                    .execute(&self.core.db)
                    .await?;
                }
                None => {}
            }
            ok(MarkNotInterestedResponse::default())
        })
        .await
    }

    async fn undo_not_interested(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UndoNotInterestedRequest>,
    ) -> ServiceResult<UndoNotInterestedResponse> {
        handle(async {
            use undo_not_interested_request::Target as Named;
            let user_id = ctx.user_id()?;
            let viewer = Viewer { user_id: Some(user_id), ..Default::default() };
            let req = request.to_owned_message();
            let (video, channel) = match &req.target {
                Some(Named::VideoId(id)) => (Some(id.as_str()), None),
                Some(Named::ChannelId(id)) => (None, Some(id.as_str())),
                None => (None, None),
            };
            match self.feed_target(&viewer, video, channel, false).await? {
                Some(Target::Video(video_id)) => {
                    sqlx::query("DELETE FROM video_feed_hidden WHERE user_id = $1 AND video_id = $2")
                        .bind(user_id)
                        .bind(video_id)
                        .execute(&self.core.db)
                        .await?;
                }
                Some(Target::Channel(channel_id)) => {
                    sqlx::query("DELETE FROM video_feed_hidden WHERE user_id = $1 AND channel_id = $2")
                        .bind(user_id)
                        .bind(channel_id)
                        .execute(&self.core.db)
                        .await?;
                }
                None => {}
            }
            ok(UndoNotInterestedResponse::default())
        })
        .await
    }
}
