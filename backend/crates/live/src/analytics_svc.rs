//! `LiveAnalyticsService`: numbers for the streamer.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::live::*;
use tokuly_video::channels;

use crate::{
    Live, analytics, audience,
    streams::{self, StreamRow, stream_columns},
};

impl LiveAnalyticsService for Live {
    async fn get_realtime_stats(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetRealtimeStatsRequest>,
    ) -> ServiceResult<GetRealtimeStatsResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let stream = streams::find_owned(core, user_id, request.stream_id).await?;
            core.cache.rate_limit(&format!("live-dashboard:{user_id}"), 120, Duration::from_secs(60)).await?;
            if stream.is_live() {
                // Polling the dashboard is looking at it: gifts need no extra notification.
                audience::mark_dashboard_open(core, stream.id, self.cfg.report_interval_seconds).await?;
            }
            ok(GetRealtimeStatsResponse {
                stats: Some(analytics::realtime(core, &stream).await?).into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_stream_analytics(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetStreamAnalyticsRequest>,
    ) -> ServiceResult<GetStreamAnalyticsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let stream = streams::find_owned(&self.core, user_id, request.stream_id).await?;
            // The report is computed from the analytics store on every call.
            self.core.cache.rate_limit(&format!("live-analytics:{user_id}"), 60, Duration::from_secs(60)).await?;
            let analytics = analytics::stream_analytics(&self.core, &stream).await?;
            ok(GetStreamAnalyticsResponse { analytics: Some(analytics).into(), ..Default::default() })
        })
        .await
    }

    async fn list_stream_reports(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListStreamReportsRequest>,
    ) -> ServiceResult<ListStreamReportsResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let channel = channels::require_owned_ref(core, user_id, request.channel_id).await?;
            let page = Page::from_view(request.page.as_option(), 100);
            let before: Option<Uuid> = page.decode()?;
            let mut rows: Vec<StreamRow> = sqlx::query_as(concat!(
                "SELECT ",
                stream_columns!(),
                " FROM live_streams s
                  WHERE s.channel_id = $1 AND s.deleted_at IS NULL AND s.started_at IS NOT NULL
                    AND ($2::uuid IS NULL OR s.id < $2)
                  ORDER BY s.id DESC LIMIT $3"
            ))
            .bind(channel.id)
            .bind(before)
            .bind(page.fetch())
            .fetch_all(&core.db)
            .await?;
            let info = page.finish(&mut rows, |row| row.id);
            ok(ListStreamReportsResponse {
                reports: analytics::reports(core, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }
}
