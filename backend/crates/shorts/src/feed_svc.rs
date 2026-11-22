//! `ShortFeedService`: the three feeds, playback reports and "not
//! interested".
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{ids::Ref, prelude::*};
use tokuly_proto::shorts::*;

use crate::{
    Shorts,
    access::Viewer,
    feeds::{DEFAULT_PAGE, MAX_PAGE},
    shorts::{self, ShortRow, short_columns},
    signals::{self, Report},
};

/// Most stays one `ReportPlayback` call may carry.
const MAX_REPORTS: usize = 50;

/// A feed page request: ten shorts unless the client asks otherwise.
fn feed_page(page: Option<&tokuly_proto::common::PageRequestView<'_>>) -> Page {
    match page {
        Some(page) => Page::new(page.limit, page.cursor, DEFAULT_PAGE, MAX_PAGE),
        None => Page::new(0, "", DEFAULT_PAGE, MAX_PAGE),
    }
}

impl Shorts {
    /// The shorts named in a batch of reports, by the reference the client
    /// used for each (UUID or short ID). Unknown references are absent.
    async fn reported_shorts(&self, references: &[&str]) -> AppResult<Vec<(String, ShortRow)>> {
        let mut ids: Vec<Uuid> = Vec::new();
        let mut short_ids: Vec<String> = Vec::new();
        for reference in references {
            match Ref::parse(reference.trim()) {
                Ok(Ref::Id(id)) => ids.push(id),
                Ok(Ref::Short(short_id)) => short_ids.push(short_id),
                Err(_) => {}
            }
        }
        let rows: Vec<ShortRow> = sqlx::query_as(concat!(
            "SELECT ",
            short_columns!(),
            " FROM shorts s WHERE s.id = ANY($1) OR s.short_id = ANY($2)"
        ))
        .bind(&ids)
        .bind(&short_ids)
        .fetch_all(&self.core.db)
        .await?;
        Ok(references
            .iter()
            .filter_map(|reference| {
                let reference = reference.trim();
                let row = rows
                    .iter()
                    .find(|row| row.short_id == reference || row.id.to_string() == reference.to_ascii_lowercase())?;
                Some((reference.to_owned(), row.clone()))
            })
            .collect())
    }
}

impl ShortFeedService for Shorts {
    async fn get_for_you_feed(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetForYouFeedRequest>,
    ) -> ServiceResult<GetForYouFeedResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let who = match viewer.user_id {
                Some(user_id) => user_id.to_string(),
                None => format!("ip:{}", ctx.client().ip),
            };
            self.core.cache.rate_limit(&format!("shorts-foryou:{who}"), 120, Duration::from_secs(60)).await?;
            let page = feed_page(request.page.as_option());
            let (items, info) = self.for_you(&viewer, &who, &page).await?;
            ok(GetForYouFeedResponse { items, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn get_following_feed(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetFollowingFeedRequest>,
    ) -> ServiceResult<GetFollowingFeedResponse> {
        handle(async {
            let viewer = Viewer::load(&self.core, Some(ctx.user_id()?)).await?;
            let page = feed_page(request.page.as_option());
            let (items, info) = self.following(&viewer, &page, request.skip_seen).await?;
            ok(GetFollowingFeedResponse { items, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn get_friends_feed(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetFriendsFeedRequest>,
    ) -> ServiceResult<GetFriendsFeedResponse> {
        handle(async {
            let viewer = Viewer::load(&self.core, Some(ctx.user_id()?)).await?;
            let page = feed_page(request.page.as_option());
            let (items, info) = self.friends(&viewer, &page, request.skip_seen).await?;
            ok(GetFriendsFeedResponse { items, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn report_playback(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReportPlaybackRequest>,
    ) -> ServiceResult<ReportPlaybackResponse> {
        handle(async {
            let core = &self.core;
            let viewer = self.viewer(&ctx).await?;
            let ip = ctx.client().ip.to_string();
            let req = request.to_owned_message();
            ensure!(
                req.reports.len() <= MAX_REPORTS,
                invalid,
                "TOO_MANY_REPORTS",
                "at most {MAX_REPORTS} reports per call"
            );
            let who = signals::viewer_key(core, viewer.user_id, &ip);
            core.cache.rate_limit(&format!("shorts-report:{who}"), 120, Duration::from_secs(60)).await?;
            let references: Vec<&str> = req.reports.iter().map(|report| report.short_id.as_str()).collect();
            let named = self.reported_shorts(&references).await?;
            let reports: Vec<Report> = req
                .reports
                .iter()
                .filter_map(|report| {
                    let (_, short) = named.iter().find(|(reference, _)| reference == report.short_id.trim())?;
                    Some(Report {
                        short: short.clone(),
                        source: signals::source_number(report.source),
                        watched_ms: report.watched_ms,
                        completions: report.completions,
                    })
                })
                .collect();
            let recorded = self.record_playback(&viewer, &ip, reports).await?;
            ok(ReportPlaybackResponse {
                accepted: recorded.accepted,
                views_counted: recorded.views,
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
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(&self.core, Some(user_id)).await?;
            let short = shorts::find_watchable(&self.core, &viewer, request.short_id).await?;
            self.core.cache.rate_limit(&format!("shorts-hide:{user_id}"), 120, Duration::from_secs(60)).await?;
            self.hide(user_id, &short).await?;
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
            let user_id = ctx.user_id()?;
            // The same answer whether or not there is such a short or mark.
            if let Some(short_id) = self.named_short(request.short_id).await? {
                self.unhide(user_id, short_id).await?;
            }
            ok(UndoNotInterestedResponse::default())
        })
        .await
    }
}
