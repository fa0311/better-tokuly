//! `PovService` and `CallService`: thin RPC handlers over [`crate::pov`]
//! and [`crate::calls`].
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::chat::*;

use crate::{Chat, conversation_svc::page_of};

impl PovService for Chat {
    async fn request_pov(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RequestPovRequest>,
    ) -> ServiceResult<RequestPovResponse> {
        handle(async {
            let (access, pov, row) =
                Chat::request_pov(self, ctx.user_id()?, parse_id(request.conversation_id)?, request.note).await?;
            let message = self.render_for(&access, std::slice::from_ref(&row)).await?.pop();
            ok(RequestPovResponse { request: pov.to_proto().into(), message: message.into(), ..Default::default() })
        })
        .await
    }

    async fn answer_pov(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AnswerPovRequest>,
    ) -> ServiceResult<AnswerPovResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let (access, pov, row) = Chat::answer_pov(
                self,
                user_id,
                parse_id(&req.request_id)?,
                &req.attachment_asset_ids,
                &req.client_id,
                &req.text,
            )
            .await?;
            let message = self.render_for(&access, std::slice::from_ref(&row)).await?.pop();
            ok(AnswerPovResponse { request: pov.to_proto().into(), message: message.into(), ..Default::default() })
        })
        .await
    }

    async fn decline_pov(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeclinePovRequest>,
    ) -> ServiceResult<DeclinePovResponse> {
        handle(async {
            let pov = Chat::decline_pov(self, ctx.user_id()?, parse_id(request.request_id)?).await?;
            ok(DeclinePovResponse { request: pov.to_proto().into(), ..Default::default() })
        })
        .await
    }

    async fn cancel_pov(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CancelPovRequest>,
    ) -> ServiceResult<CancelPovResponse> {
        handle(async {
            let pov = Chat::cancel_pov(self, ctx.user_id()?, parse_id(request.request_id)?).await?;
            ok(CancelPovResponse { request: pov.to_proto().into(), ..Default::default() })
        })
        .await
    }

    async fn save_pov(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SavePovRequest>,
    ) -> ServiceResult<SavePovResponse> {
        handle(async {
            let pov = Chat::save_pov(self, ctx.user_id()?, parse_id(request.request_id)?).await?;
            ok(SavePovResponse { request: pov.to_proto().into(), ..Default::default() })
        })
        .await
    }
}

impl CallService for Chat {
    async fn start_call(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, StartCallRequest>,
    ) -> ServiceResult<StartCallResponse> {
        handle(async {
            let (call, credentials) =
                Chat::start_call(self, ctx.user_id()?, parse_id(request.conversation_id)?, request.video).await?;
            ok(StartCallResponse {
                call: self.call_to_proto(&call).await?.into(),
                credentials: credentials.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn join_call(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, JoinCallRequest>,
    ) -> ServiceResult<JoinCallResponse> {
        handle(async {
            let (call, credentials) = Chat::join_call(self, ctx.user_id()?, parse_id(request.call_id)?).await?;
            ok(JoinCallResponse {
                call: self.call_to_proto(&call).await?.into(),
                credentials: credentials.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn decline_call(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeclineCallRequest>,
    ) -> ServiceResult<DeclineCallResponse> {
        handle(async {
            let call = Chat::decline_call(self, ctx.user_id()?, parse_id(request.call_id)?).await?;
            ok(DeclineCallResponse { call: self.call_to_proto(&call).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn leave_call(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, LeaveCallRequest>,
    ) -> ServiceResult<LeaveCallResponse> {
        handle(async {
            let call = Chat::leave_call(self, ctx.user_id()?, parse_id(request.call_id)?).await?;
            ok(LeaveCallResponse { call: self.call_to_proto(&call).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn get_call(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetCallRequest>,
    ) -> ServiceResult<GetCallResponse> {
        handle(async {
            let call = self.call_for(ctx.user_id()?, parse_id(request.call_id)?).await?;
            ok(GetCallResponse { call: self.call_to_proto(&call).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn list_calls(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListCallsRequest>,
    ) -> ServiceResult<ListCallsResponse> {
        handle(async {
            let page = page_of(request.page.as_option(), 30, 100);
            let (calls, page) = Chat::list_calls(self, ctx.user_id()?, request.missed_only, &page).await?;
            ok(ListCallsResponse { calls: self.calls_to_proto(&calls).await?, page: page.into(), ..Default::default() })
        })
        .await
    }
}
