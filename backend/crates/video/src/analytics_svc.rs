//! `StudioAnalyticsService`: analytics for the owner of a channel.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::video::*;

use crate::{
    Video,
    access::Viewer,
    analytics::{self, Range, Scope},
    channels::{self, ChannelRow},
    feeds, videos,
};

/// How many videos a "top videos" list names.
const TOP_VIDEOS: u32 = 10;

impl Video {
    /// The most viewed videos of a channel between two instants, as the
    /// owner sees them. Videos deleted since are left out.
    async fn top_videos(
        &self,
        owner: &Viewer,
        channel: &ChannelRow,
        from_ms: i64,
        until_ms: i64,
    ) -> AppResult<Vec<VideoStats>> {
        let top = analytics::top_videos(&self.core, channel, from_ms, until_ms, TOP_VIDEOS).await?;
        let ids: Vec<Uuid> = top.iter().map(|(id, _, _)| *id).collect();
        let rows = feeds::watchable_in_order(&self.core, owner, &ids).await?;
        let summaries = videos::summaries(&self.core, owner, &rows).await?;
        Ok(rows
            .iter()
            .zip(summaries)
            .filter_map(|(row, summary)| {
                let (_, views, watch_seconds) = top.iter().find(|(id, _, _)| *id == row.id)?;
                Some(VideoStats {
                    video: summary.into(),
                    views: *views,
                    watch_time_seconds: *watch_seconds,
                    ..Default::default()
                })
            })
            .collect())
    }
}

impl StudioAnalyticsService for Video {
    async fn get_channel_analytics(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetChannelAnalyticsRequest>,
    ) -> ServiceResult<GetChannelAnalyticsResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let channel = channels::require_owned_ref(core, user_id, &req.channel_id).await?;
            let range = Range::parse(req.range.as_option())?;
            let scope = Scope::channel(&channel);
            let owner = Viewer { user_id: Some(user_id), ..Default::default() };
            let (totals, daily, traffic_sources, top_videos) = tokio::try_join!(
                analytics::totals(core, scope, &range),
                analytics::daily(core, scope, &range),
                analytics::traffic_sources(core, scope, &range),
                self.top_videos(&owner, &channel, range.from_ms, range.until_ms),
            )?;
            ok(GetChannelAnalyticsResponse {
                totals: totals.into(),
                daily,
                traffic_sources,
                top_videos,
                subscriber_count: channel.subscriber_count,
                ..Default::default()
            })
        })
        .await
    }

    async fn get_video_analytics(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetVideoAnalyticsRequest>,
    ) -> ServiceResult<GetVideoAnalyticsResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let video = videos::find_owned(core, user_id, &req.video_id).await?;
            let range = Range::parse(req.range.as_option())?;
            let scope = Scope::video(&video);
            let (totals, daily, traffic_sources, retention) = tokio::try_join!(
                analytics::totals(core, scope, &range),
                analytics::daily(core, scope, &range),
                analytics::traffic_sources(core, scope, &range),
                analytics::retention(core, &video, &range),
            )?;
            ok(GetVideoAnalyticsResponse {
                totals: totals.into(),
                daily,
                traffic_sources,
                retention,
                ..Default::default()
            })
        })
        .await
    }

    async fn get_realtime_analytics(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetRealtimeAnalyticsRequest>,
    ) -> ServiceResult<GetRealtimeAnalyticsResponse> {
        handle(async {
            use get_realtime_analytics_request::Target;
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let (scope, channel) = match &req.target {
                Some(Target::ChannelId(reference)) => {
                    let channel = channels::require_owned_ref(core, user_id, reference).await?;
                    (Scope::channel(&channel), Some(channel))
                }
                Some(Target::VideoId(reference)) => {
                    (Scope::video(&videos::find_owned(core, user_id, reference).await?), None)
                }
                None => bail!(invalid, "TARGET_REQUIRED", "a channel or a video is required"),
            };
            let (views_per_hour, views_per_minute) = analytics::realtime(core, scope).await?;
            let top_videos = match &channel {
                Some(channel) => {
                    let owner = Viewer { user_id: Some(user_id), ..Default::default() };
                    let until = now();
                    let from = until - chrono::Duration::hours(48);
                    self.top_videos(&owner, channel, from.timestamp_millis(), until.timestamp_millis() + 1).await?
                }
                None => Vec::new(),
            };
            ok(GetRealtimeAnalyticsResponse {
                views_last_48_hours: views_per_hour.iter().sum(),
                views_last_60_minutes: views_per_minute.iter().sum(),
                views_per_hour,
                views_per_minute,
                top_videos,
                ..Default::default()
            })
        })
        .await
    }
}
