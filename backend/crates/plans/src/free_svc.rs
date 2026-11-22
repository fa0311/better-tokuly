//! `FreeService`: thin RPC handlers over [`crate::free`].
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::plans::*;

use crate::Plans;

/// Largest page of any list in this service.
const MAX_PAGE: i64 = 100;

impl FreeService for Plans {
    async fn set_free_status(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetFreeStatusRequest>,
    ) -> ServiceResult<SetFreeStatusResponse> {
        handle(async {
            let status = Plans::set_free_status(self, ctx.user_id()?, &request.to_owned_message()).await?;
            ok(SetFreeStatusResponse { status: status.into(), ..Default::default() })
        })
        .await
    }

    async fn clear_free_status(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ClearFreeStatusRequest>,
    ) -> ServiceResult<ClearFreeStatusResponse> {
        handle(async {
            Plans::clear_free_status(self, ctx.user_id()?).await?;
            ok(ClearFreeStatusResponse::default())
        })
        .await
    }

    async fn get_free_status(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetFreeStatusRequest>,
    ) -> ServiceResult<GetFreeStatusResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let owner = parse_opt_id(request.user_id)?.unwrap_or(viewer);
            let status = Plans::get_free_status(self, viewer, owner).await?;
            ok(GetFreeStatusResponse { status: status.into(), ..Default::default() })
        })
        .await
    }

    async fn list_free_friends(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListFreeFriendsRequest>,
    ) -> ServiceResult<ListFreeFriendsResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let first_page = page.cursor.is_none();
            let (statuses, page) = Plans::list_free_friends(self, viewer, &page).await?;
            let mine = if first_page { Plans::get_free_status(self, viewer, viewer).await? } else { None };
            ok(ListFreeFriendsResponse { statuses, page: page.into(), mine: mine.into(), ..Default::default() })
        })
        .await
    }

    async fn answer_free_status(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AnswerFreeStatusRequest>,
    ) -> ServiceResult<AnswerFreeStatusResponse> {
        handle(async {
            ok(Plans::answer_free_status(self, ctx.user_id()?, parse_id(request.user_id)?, request.message).await?)
        })
        .await
    }

    async fn list_free_answers(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListFreeAnswersRequest>,
    ) -> ServiceResult<ListFreeAnswersResponse> {
        handle(async {
            let (answers, page) =
                Plans::list_free_answers(self, ctx.user_id()?, &Page::from_view(request.page.as_option(), MAX_PAGE))
                    .await?;
            ok(ListFreeAnswersResponse { answers, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn set_free_alert(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetFreeAlertRequest>,
    ) -> ServiceResult<SetFreeAlertResponse> {
        handle(async {
            Plans::set_free_alert(self, ctx.user_id()?, parse_id(request.user_id)?, request.enabled).await?;
            ok(SetFreeAlertResponse::default())
        })
        .await
    }

    async fn list_free_alerts(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListFreeAlertsRequest>,
    ) -> ServiceResult<ListFreeAlertsResponse> {
        handle(async {
            let (users, page) =
                Plans::list_free_alerts(self, ctx.user_id()?, &Page::from_view(request.page.as_option(), MAX_PAGE))
                    .await?;
            ok(ListFreeAlertsResponse { users, page: page.into(), ..Default::default() })
        })
        .await
    }
}
