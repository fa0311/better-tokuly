//! `SystemService`: server stats and the audit log. Every RPC needs the
//! `admin` role.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::admin::*;

use crate::{Admin, audit::AuditFilter, staff::Staff};

impl SystemService for Admin {
    async fn get_server_stats(&self, ctx: RequestContext, _request: ServiceRequest<'_, GetServerStatsRequest>) -> ServiceResult<GetServerStatsResponse> {
        handle(async {
            let staff = Staff::admin(&ctx)?;
            // The snapshot runs several full-table counts; keep a dashboard
            // left on auto-refresh from hammering the database.
            self.core.cache.rate_limit(&format!("admin-stats:{}", staff.id), 30, Duration::from_secs(60)).await?;
            ok(self.server_stats().await?)
        })
        .await
    }

    async fn list_audit_log(&self, ctx: RequestContext, request: ServiceRequest<'_, ListAuditLogRequest>) -> ServiceResult<ListAuditLogResponse> {
        handle(async {
            Staff::admin(&ctx)?;
            let request = request.to_owned_message();
            let filter = AuditFilter {
                actor_id: parse_opt_id(&request.actor_id)?,
                action: request.action.trim().to_owned(),
                target_type: request.target_type.trim().to_owned(),
                target_id: request.target_id.trim().to_owned(),
                target_user_id: parse_opt_id(&request.target_user_id)?,
                since: request.since.as_option().map(from_ts),
                until: request.until.as_option().map(from_ts),
            };
            if let (Some(since), Some(until)) = (filter.since, filter.until) {
                ensure!(since < until, invalid, "TIME_RANGE_INVALID", "`since` must be before `until`");
            }
            let page = Page::from_request(request.page.as_option(), 200);
            let mut rows = self.list_audit(&filter, &page).await?;
            let info = page.finish(&mut rows, |r| r.id);
            ok(ListAuditLogResponse { entries: self.audit_entries(&filter, rows).await?, page: info.into(), ..Default::default() })
        })
        .await
    }
}
