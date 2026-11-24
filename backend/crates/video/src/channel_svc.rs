//! `ChannelService`: channels, their pages and subscriptions.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use serde::{Deserialize, Serialize};
use tokuly_core::prelude::*;
use tokuly_proto::video::*;

use crate::{
    Video,
    access::{Viewer, listed},
    analytics,
    channels::{self, ChannelEdit, ChannelRow, channel_columns},
    playlists::{self, PlaylistRow, playlist_columns},
    validate,
    videos::{self, VideoRow, video_columns},
};

/// Videos and playlists per section of the channel home tab.
const SECTION_SIZE: i64 = 12;

const SORT_NEWEST: i32 = ChannelVideoSort::CHANNEL_VIDEO_SORT_NEWEST as i32;
const SORT_POPULAR: i32 = ChannelVideoSort::CHANNEL_VIDEO_SORT_POPULAR as i32;
const SORT_OLDEST: i32 = ChannelVideoSort::CHANNEL_VIDEO_SORT_OLDEST as i32;

/// Keyset of a channel video listing: the sort value and ID of the last row.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct VideoCursor {
    at: Option<Time>,
    views: i64,
    id: Uuid,
}

impl VideoCursor {
    fn of(video: &VideoRow) -> Self {
        Self { at: video.published_at, views: video.view_count, id: video.id }
    }
}

#[derive(sqlx::FromRow)]
struct SubscriptionRow {
    #[sqlx(flatten)]
    channel: ChannelRow,
    notify_level: i32,
    subscribed_at: Time,
}

/// An asset ID as update requests carry it: unset = keep, empty = remove.
fn asset_change(value: Option<&str>) -> AppResult<Option<Option<Uuid>>> {
    value.map(parse_opt_id).transpose()
}

impl Video {
    /// The videos listed for `viewer` on a channel's page, sorted and paged.
    /// `origin` 0 = every origin.
    async fn channel_videos(
        &self,
        viewer: &Viewer,
        channel_id: Uuid,
        sort: i32,
        origin: i32,
        after: Option<&VideoCursor>,
        limit: i64,
    ) -> AppResult<Vec<VideoRow>> {
        // The three orders share everything but the keyset comparison.
        let (sql, by_views) = match sort {
            SORT_POPULAR => (
                concat!(
                    "SELECT ",
                    video_columns!(),
                    " FROM videos v
                      WHERE v.channel_id = $7 AND ($8 = 0 OR v.origin = $8) AND ",
                    listed!(),
                    "   AND ($10::uuid IS NULL OR (v.view_count, v.id) < ($9, $10))
                      ORDER BY v.view_count DESC, v.id DESC LIMIT $11"
                ),
                true,
            ),
            SORT_OLDEST => (
                concat!(
                    "SELECT ",
                    video_columns!(),
                    " FROM videos v
                      WHERE v.channel_id = $7 AND ($8 = 0 OR v.origin = $8) AND ",
                    listed!(),
                    "   AND ($10::uuid IS NULL OR (v.published_at, v.id) > ($9, $10))
                      ORDER BY v.published_at, v.id LIMIT $11"
                ),
                false,
            ),
            _ => (
                concat!(
                    "SELECT ",
                    video_columns!(),
                    " FROM videos v
                      WHERE v.channel_id = $7 AND ($8 = 0 OR v.origin = $8) AND ",
                    listed!(),
                    "   AND ($10::uuid IS NULL OR (v.published_at, v.id) < ($9, $10))
                      ORDER BY v.published_at DESC, v.id DESC LIMIT $11"
                ),
                false,
            ),
        };
        let query = viewer.query_as::<VideoRow>(sql).bind(channel_id).bind(origin);
        let query = if by_views {
            query.bind(after.map(|cursor| cursor.views))
        } else {
            query.bind(after.and_then(|cursor| cursor.at))
        };
        let mut rows = query.bind(after.map(|cursor| cursor.id)).bind(limit).fetch_all(&self.core.db).await?;
        viewer.drop_newly_locked(&self.core, &mut rows, VideoRow::lock_facts).await?;
        Ok(rows)
    }

    async fn channel_response(&self, viewer: &Viewer, channel_id: Uuid) -> AppResult<Channel> {
        let channel = channels::find_by_id(&self.core, channel_id)
            .await?
            .or_not_found("CHANNEL_NOT_FOUND", "channel not found")?;
        channels::to_proto(&self.core, viewer, &channel).await
    }
}

impl ChannelService for Video {
    async fn create_channel(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateChannelRequest>,
    ) -> ServiceResult<CreateChannelResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            // Check what people get wrong first, so a typo does not use up the hourly allowance.
            validate::handle(&req.handle)?;
            validate::channel_name(&req.name)?;
            self.core
                .cache
                .rate_limit(&format!("video-channel-create:{user_id}"), 10, Duration::from_secs(3600))
                .await?;
            let edit = ChannelEdit {
                handle: Some(req.handle),
                name: Some(req.name),
                description: Some(req.description),
                icon_asset_id: Some(parse_opt_id(&req.icon_asset_id)?),
                banner_asset_id: Some(parse_opt_id(&req.banner_asset_id)?),
                links: Some(req.links),
                trailer_video_id: None,
            };
            let channel = channels::create(&self.core, user_id, self.cfg.max_channels, edit).await?;
            let viewer = Viewer { user_id: Some(user_id), ..Default::default() };
            ok(CreateChannelResponse {
                channel: channels::to_proto(&self.core, &viewer, &channel).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_channel(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateChannelRequest>,
    ) -> ServiceResult<UpdateChannelResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let channel = channels::require_owned_ref(&self.core, user_id, &req.channel_id).await?;
            let trailer_video_id = match req.trailer_video_id.as_deref().map(str::trim) {
                None => None,
                Some("") => Some(None),
                Some(reference) => {
                    let video = videos::find_ref(&self.core, reference)
                        .await?
                        .filter(|video| video.channel_id == channel.id)
                        .or_not_found("TRAILER_NOT_FOUND", "the trailer must be a public video of this channel")?;
                    Some(Some(video.id))
                }
            };
            let edit = ChannelEdit {
                handle: req.handle,
                name: req.name,
                description: req.description,
                icon_asset_id: asset_change(req.icon_asset_id.as_deref())?,
                banner_asset_id: asset_change(req.banner_asset_id.as_deref())?,
                links: req.links.into_option().map(|list| list.links),
                trailer_video_id,
            };
            let updated = channels::update(&self.core, user_id, &channel, edit).await?;
            let viewer = Viewer { user_id: Some(user_id), ..Default::default() };
            ok(UpdateChannelResponse {
                channel: channels::to_proto(&self.core, &viewer, &updated).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_channel(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteChannelRequest>,
    ) -> ServiceResult<DeleteChannelResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let channel = channels::require_owned_ref(&self.core, user_id, request.channel_id).await?;
            channels::delete(&self.core, &channel, Some(user_id), false).await?;
            ok(DeleteChannelResponse::default())
        })
        .await
    }

    async fn list_my_channels(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListMyChannelsRequest>,
    ) -> ServiceResult<ListMyChannelsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let viewer = Viewer { user_id: Some(user_id), ..Default::default() };
            let mut channels = Vec::new();
            for channel in channels::owned_by(&self.core, user_id).await? {
                channels.push(channels::to_proto(&self.core, &viewer, &channel).await?);
            }
            ok(ListMyChannelsResponse { channels, max_channels: self.cfg.max_channels as i32, ..Default::default() })
        })
        .await
    }

    async fn set_default_channel(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetDefaultChannelRequest>,
    ) -> ServiceResult<SetDefaultChannelResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let channel = channels::require_owned_ref(&self.core, user_id, request.channel_id).await?;
            channels::set_default(&self.core, user_id, channel.id).await?;
            let viewer = Viewer { user_id: Some(user_id), ..Default::default() };
            ok(SetDefaultChannelResponse {
                channel: self.channel_response(&viewer, channel.id).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn check_channel_handle(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CheckChannelHandleRequest>,
    ) -> ServiceResult<CheckChannelHandleResponse> {
        handle(async {
            let limiter = ctx.maybe_user_id()?.map(|id| id.to_string()).unwrap_or_else(|| ctx.client().ip);
            self.core.cache.rate_limit(&format!("video-handle-check:{limiter}"), 120, Duration::from_secs(60)).await?;
            let response = match validate::handle(request.handle) {
                Err(error) => CheckChannelHandleResponse { reason: error.reason.into_owned(), ..Default::default() },
                Ok(normalized) => {
                    let taken = channels::find_by_handle(&self.core, &normalized).await?.is_some();
                    CheckChannelHandleResponse {
                        available: !taken,
                        reason: if taken { "HANDLE_TAKEN".to_owned() } else { String::new() },
                        normalized,
                        ..Default::default()
                    }
                }
            };
            ok(response)
        })
        .await
    }

    async fn get_channel(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetChannelRequest>,
    ) -> ServiceResult<GetChannelResponse> {
        handle(async {
            let viewer = Viewer::load(&self.core, ctx.maybe_user_id()?).await?;
            let channel = channels::find_visible(&self.core, &viewer, request.channel_id).await?;
            ok(GetChannelResponse {
                channel: channels::to_proto(&self.core, &viewer, &channel).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_channel_page(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetChannelPageRequest>,
    ) -> ServiceResult<GetChannelPageResponse> {
        handle(async {
            let core = &self.core;
            let viewer = Viewer::load(core, ctx.maybe_user_id()?).await?;
            let channel = channels::find_visible(core, &viewer, request.channel_id).await?;
            let mut sections = Vec::new();
            let mut video_section = |kind: ChannelSectionKind, videos: Vec<VideoSummary>| {
                if !videos.is_empty() {
                    sections.push(ChannelSection { kind: kind.into(), videos, ..Default::default() });
                }
            };

            let trailer = match channel.trailer_video_id {
                Some(video_id) => videos::find_by_id(core, video_id)
                    .await?
                    .filter(|video| video.is_public() && !viewer.hides(video.owner_id)),
                None => None,
            };
            video_section(
                ChannelSectionKind::CHANNEL_SECTION_KIND_TRAILER,
                videos::summaries(core, &viewer, trailer.as_slice()).await?,
            );
            let latest = self.channel_videos(&viewer, channel.id, SORT_NEWEST, 0, None, SECTION_SIZE).await?;
            video_section(
                ChannelSectionKind::CHANNEL_SECTION_KIND_LATEST,
                videos::summaries(core, &viewer, &latest).await?,
            );
            let popular = self.channel_videos(&viewer, channel.id, SORT_POPULAR, 0, None, SECTION_SIZE).await?;
            // A "popular" row that merely repeats the latest uploads adds nothing.
            if popular.len() > 1 && popular.iter().any(|video| video.view_count > 0) {
                video_section(
                    ChannelSectionKind::CHANNEL_SECTION_KIND_POPULAR,
                    videos::summaries(core, &viewer, &popular).await?,
                );
            }
            let archives = self
                .channel_videos(
                    &viewer,
                    channel.id,
                    SORT_NEWEST,
                    VideoOrigin::VIDEO_ORIGIN_LIVE_ARCHIVE as i32,
                    None,
                    SECTION_SIZE,
                )
                .await?;
            video_section(
                ChannelSectionKind::CHANNEL_SECTION_KIND_LIVE_ARCHIVES,
                videos::summaries(core, &viewer, &archives).await?,
            );

            let listed: Vec<PlaylistRow> = sqlx::query_as(concat!(
                "SELECT ",
                playlist_columns!(),
                " FROM playlists p WHERE p.channel_id = $1 AND p.kind = 1 AND p.visibility = 1 AND p.item_count > 0
                  ORDER BY p.updated_at DESC, p.id DESC LIMIT $2"
            ))
            .bind(channel.id)
            .bind(SECTION_SIZE)
            .fetch_all(&core.db)
            .await?;
            if !listed.is_empty() {
                sections.push(ChannelSection {
                    kind: ChannelSectionKind::CHANNEL_SECTION_KIND_PLAYLISTS.into(),
                    playlists: playlists::to_protos(core, &viewer, &listed, None).await?,
                    ..Default::default()
                });
            }
            ok(GetChannelPageResponse {
                channel: channels::to_proto(core, &viewer, &channel).await?.into(),
                sections,
                ..Default::default()
            })
        })
        .await
    }

    async fn list_channel_videos(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListChannelVideosRequest>,
    ) -> ServiceResult<ListChannelVideosResponse> {
        handle(async {
            let viewer = Viewer::load(&self.core, ctx.maybe_user_id()?).await?;
            let channel = channels::find_visible(&self.core, &viewer, request.channel_id).await?;
            let sort = match request.sort.as_known() {
                Some(ChannelVideoSort::CHANNEL_VIDEO_SORT_UNSPECIFIED) => SORT_NEWEST,
                Some(known) => known as i32,
                None => bail!(invalid, "SORT_INVALID", "unknown sort order"),
            };
            ensure!(request.origin.as_known().is_some(), invalid, "ORIGIN_INVALID", "unknown video origin");
            let page = Page::from_view(request.page.as_option(), 100);
            let after: Option<VideoCursor> = page.decode()?;
            let mut rows = self
                .channel_videos(&viewer, channel.id, sort, request.origin.to_i32(), after.as_ref(), page.fetch())
                .await?;
            let info = page.finish(&mut rows, VideoCursor::of);
            ok(ListChannelVideosResponse {
                videos: videos::summaries(&self.core, &viewer, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn subscribe(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SubscribeRequest>,
    ) -> ServiceResult<SubscribeResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let channel = channels::find_visible(core, &viewer, request.channel_id).await?;
            let level = channels::requested_level(request.notification_level)?;
            core.cache.rate_limit(&format!("video-subscribe:{user_id}"), 60, Duration::from_secs(60)).await?;
            if channels::subscribe(core, user_id, &channel, level).await? {
                // Credit the video they were watching, if it is one of this channel's.
                let source = match request.source_video_id.trim() {
                    "" => None,
                    reference => videos::find_ref(core, reference)
                        .await?
                        .filter(|video| video.channel_id == channel.id)
                        .map(|video| video.id),
                };
                analytics::track_subscription(core, &channel, user_id, true, source).await;
            }
            ok(SubscribeResponse {
                channel: self.channel_response(&viewer, channel.id).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn unsubscribe(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnsubscribeRequest>,
    ) -> ServiceResult<UnsubscribeResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            // No visibility check: one can always leave, also a channel that has since been hidden.
            let channel = channels::find(core, request.channel_id)
                .await?
                .or_not_found("CHANNEL_NOT_FOUND", "channel not found")?;
            if channels::unsubscribe(core, user_id, &channel).await? {
                analytics::track_subscription(core, &channel, user_id, false, None).await;
            }
            ok(UnsubscribeResponse {
                channel: self.channel_response(&viewer, channel.id).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_subscriptions(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListSubscriptionsRequest>,
    ) -> ServiceResult<ListSubscriptionsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(&self.core, Some(user_id)).await?;
            let page = Page::from_view(request.page.as_option(), 100);
            let before: Option<(Time, Uuid)> = page.decode()?;
            let mut rows: Vec<SubscriptionRow> = sqlx::query_as(concat!(
                "SELECT ",
                channel_columns!(),
                ", s.notify_level, s.created_at AS subscribed_at
                 FROM channel_subscriptions s JOIN channels c ON c.id = s.channel_id
                 WHERE s.user_id = $1 AND NOT c.suspended AND NOT (c.owner_id = ANY($2))
                   AND ($3::timestamptz IS NULL OR (s.created_at, s.channel_id) < ($3, $4))
                 ORDER BY s.created_at DESC, s.channel_id DESC
                 LIMIT $5"
            ))
            .bind(user_id)
            .bind(&viewer.blocked)
            .bind(before.map(|(at, _)| at))
            .bind(before.map(|(_, id)| id))
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let info = page.finish(&mut rows, |row| (row.subscribed_at, row.channel.id));
            let subscriptions = rows
                .into_iter()
                .map(|row| Subscription {
                    channel: row.channel.summary().into(),
                    notification_level: row.notify_level.into(),
                    subscribed_at: ts(row.subscribed_at),
                    ..Default::default()
                })
                .collect();
            ok(ListSubscriptionsResponse { subscriptions, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn list_subscribers(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListSubscribersRequest>,
    ) -> ServiceResult<ListSubscribersResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let channel = channels::require_owned_ref(&self.core, user_id, request.channel_id).await?;
            let page = Page::from_view(request.page.as_option(), 100);
            let before: Option<(Time, Uuid)> = page.decode()?;
            let mut rows: Vec<(Uuid, Time)> = sqlx::query_as(
                "SELECT s.user_id, s.created_at FROM channel_subscriptions s
                 WHERE s.channel_id = $1 AND ($2::timestamptz IS NULL OR (s.created_at, s.user_id) < ($2, $3))
                 ORDER BY s.created_at DESC, s.user_id DESC
                 LIMIT $4",
            )
            .bind(channel.id)
            .bind(before.map(|(at, _)| at))
            .bind(before.map(|(_, id)| id))
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let info = page.finish(&mut rows, |(subscriber, at)| (*at, *subscriber));
            let ids: Vec<Uuid> = rows.iter().map(|(id, _)| *id).collect();
            let users = tokuly_identity::summaries(&self.core, &ids).await?;
            let subscribers = rows
                .into_iter()
                .map(|(id, at)| Subscriber {
                    user: tokuly_identity::users::pick(&users, id).into(),
                    subscribed_at: ts(at),
                    ..Default::default()
                })
                .collect();
            ok(ListSubscribersResponse {
                subscribers,
                page: info.into(),
                subscriber_count: channel.subscriber_count,
                ..Default::default()
            })
        })
        .await
    }

    async fn list_recommended_channels(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListRecommendedChannelsRequest>,
    ) -> ServiceResult<ListRecommendedChannelsResponse> {
        handle(async {
            let viewer = Viewer::load(&self.core, ctx.maybe_user_id()?).await?;
            let limit = if request.limit <= 0 { 12 } else { request.limit.min(50) } as i64;
            // Popular channels with something to watch, minus the viewer's own,
            // the ones they follow already and the ones they asked not to see.
            let rows: Vec<ChannelRow> = sqlx::query_as(concat!(
                "SELECT ",
                channel_columns!(),
                " FROM channels c
                  WHERE NOT c.suspended AND NOT (c.owner_id = ANY($2)) AND ($1::uuid IS NULL OR c.owner_id <> $1)
                    AND EXISTS (SELECT 1 FROM videos v WHERE v.channel_id = c.id AND v.status = 3 AND v.visibility = 1 AND NOT v.owner_locked)
                    AND NOT EXISTS (SELECT 1 FROM channel_subscriptions s WHERE s.channel_id = c.id AND s.user_id = $1)
                    AND NOT EXISTS (SELECT 1 FROM video_feed_hidden f WHERE f.channel_id = c.id AND f.user_id = $1)
                  ORDER BY c.subscriber_count DESC, c.id DESC
                  LIMIT $3"
            ))
            .bind(viewer.user_id)
            .bind(&viewer.blocked)
            .bind(limit)
            .fetch_all(&self.core.db)
            .await?;
            ok(ListRecommendedChannelsResponse {
                channels: rows.iter().map(ChannelRow::summary).collect(),
                ..Default::default()
            })
        })
        .await
    }
}
