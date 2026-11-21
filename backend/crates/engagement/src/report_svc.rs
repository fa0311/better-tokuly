//! `ReportService`: reporting subjects to the moderation team.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::engagement::*;

use crate::{
    Engagement,
    reports::{self, ReportRow},
};

const PAGE: i64 = 30;
const PAGE_MAX: i64 = 100;

impl ReportService for Engagement {
    async fn create_report(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateReportRequest>,
    ) -> ServiceResult<CreateReportResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let request = request.to_owned_message();
            let category = reports::requested_category(request.category)?;
            // Blocks do not stand in the way of a report: whatever the
            // owning domain still shows the caller can be reported.
            let subject = self.requested_subject_unguarded(request.subject.as_option(), Some(user_id)).await?;
            let report = Engagement::create_report(self, user_id, &subject, category, &request.text).await?;
            ok(CreateReportResponse { report: report.to_proto().into(), ..Default::default() })
        })
        .await
    }

    async fn list_my_reports(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListMyReportsRequest>,
    ) -> ServiceResult<ListMyReportsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let page = match request.page.as_option() {
                Some(page) => Page::new(page.limit, page.cursor, PAGE, PAGE_MAX),
                None => Page::new(0, "", PAGE, PAGE_MAX),
            };
            let (rows, info) = self.reports_by(user_id, &page).await?;
            ok(ListMyReportsResponse {
                reports: rows.iter().map(ReportRow::to_proto).collect(),
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }
}
