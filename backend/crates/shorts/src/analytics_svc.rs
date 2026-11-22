//! `ShortAnalyticsService`: analytics for the owner of a short or channel.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::shorts::*;
use tokuly_video::channels;

use crate::{
    Shorts,
    access::Viewer,
    analytics::{self, Scope},
    hydrate, shorts,
};

/// How many shorts a "top shorts" list names.
const TOP_SHORTS: u32 = 10;

impl Shorts {
    /// Analytics run half a dozen ClickHouse queries per call.
    async fn limit_analytics(&self, user_id: Uuid) -> AppResult<()> {
        self.core.cache.rate_limit(&format!("shorts-analytics:{user_id}"), 60, std::time::Duration::from_secs(60)).await
    }
}

impl ShortAnalyticsService for Shorts {
    async fn get_short_analytics(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetShortAnalyticsRequest>,
    ) -> ServiceResult<GetShortAnalyticsResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let short = shorts::find_owned(core, user_id, &req.short_id).await?;
            let range = analytics::owner_range(core, user_id, req.range.as_option()).await?;
            self.limit_analytics(user_id).await?;
            let scope = Scope::short(&short);
            let (totals, daily) = analytics::report(core, scope, &range).await?;
            ok(GetShortAnalyticsResponse {
                totals: Some(totals).into(),
                daily,
                traffic_sources: analytics::traffic_sources(core, scope, &range).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn get_channel_shorts_analytics(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetChannelShortsAnalyticsRequest>,
    ) -> ServiceResult<GetChannelShortsAnalyticsResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let channel = channels::require_owned_ref(core, user_id, &req.channel_id).await?;
            let range = analytics::owner_range(core, user_id, req.range.as_option()).await?;
            self.limit_analytics(user_id).await?;
            let scope = Scope::channel(&channel);

            // Shorts deleted since they were watched are left out of the top list.
            let top = analytics::top_shorts(core, &channel, &range, TOP_SHORTS).await?;
            let ids: Vec<Uuid> = top.iter().map(|(id, _)| *id).collect();
            let rows = shorts::find_many(core, &ids).await?;
            let owner = Viewer { user_id: Some(user_id), ..Default::default() };
            let top_shorts = hydrate::shorts(core, &owner, &rows)
                .await?
                .into_iter()
                .filter_map(|short| {
                    let (_, views) = top.iter().find(|(id, _)| id.to_string() == short.id)?;
                    Some(ShortStats { short: Some(short).into(), views: *views, ..Default::default() })
                })
                .collect();
            let short_count: i64 =
                sqlx::query_scalar("SELECT count(*) FROM shorts WHERE channel_id = $1 AND status = 2")
                    .bind(channel.id)
                    .fetch_one(&core.db)
                    .await?;
            let (totals, daily) = analytics::report(core, scope, &range).await?;
            ok(GetChannelShortsAnalyticsResponse {
                totals: Some(totals).into(),
                daily,
                traffic_sources: analytics::traffic_sources(core, scope, &range).await?,
                top_shorts,
                short_count,
                ..Default::default()
            })
        })
        .await
    }
}
