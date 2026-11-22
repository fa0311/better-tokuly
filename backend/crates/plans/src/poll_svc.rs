//! `PollService`: thin RPC handlers over [`crate::polls`].
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::plans::*;

use crate::Plans;

/// Largest page of the poll list.
const MAX_PAGE: i64 = 50;

impl PollService for Plans {
    async fn create_poll(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreatePollRequest>,
    ) -> ServiceResult<CreatePollResponse> {
        handle(async {
            let poll = Plans::create_poll(self, ctx.user_id()?, &request.to_owned_message()).await?;
            ok(CreatePollResponse { poll: poll.into(), ..Default::default() })
        })
        .await
    }

    async fn get_poll(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetPollRequest>,
    ) -> ServiceResult<GetPollResponse> {
        handle(async {
            let poll = Plans::get_poll(self, ctx.user_id()?, request.poll_id).await?;
            ok(GetPollResponse { poll: poll.into(), ..Default::default() })
        })
        .await
    }

    async fn update_poll(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdatePollRequest>,
    ) -> ServiceResult<UpdatePollResponse> {
        handle(async {
            let poll = Plans::update_poll(self, ctx.user_id()?, &request.to_owned_message()).await?;
            ok(UpdatePollResponse { poll: poll.into(), ..Default::default() })
        })
        .await
    }

    async fn list_polls(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListPollsRequest>,
    ) -> ServiceResult<ListPollsResponse> {
        handle(async {
            let (polls, page) =
                Plans::list_polls(self, ctx.user_id()?, &Page::from_view(request.page.as_option(), MAX_PAGE)).await?;
            ok(ListPollsResponse { polls, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn invite_to_poll(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, InviteToPollRequest>,
    ) -> ServiceResult<InviteToPollResponse> {
        handle(async {
            let req = request.to_owned_message();
            let (poll, invited_count) =
                Plans::invite_to_poll(self, ctx.user_id()?, &req.poll_id, &req.user_ids).await?;
            ok(InviteToPollResponse { poll: poll.into(), invited_count, ..Default::default() })
        })
        .await
    }

    async fn answer_poll(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AnswerPollRequest>,
    ) -> ServiceResult<AnswerPollResponse> {
        handle(async {
            let poll = Plans::answer_poll(self, ctx.user_id()?, &request.to_owned_message()).await?;
            ok(AnswerPollResponse { poll: poll.into(), ..Default::default() })
        })
        .await
    }

    async fn close_poll(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ClosePollRequest>,
    ) -> ServiceResult<ClosePollResponse> {
        handle(async {
            let poll = self.set_poll_closed(ctx.user_id()?, request.poll_id, true).await?;
            ok(ClosePollResponse { poll: poll.into(), ..Default::default() })
        })
        .await
    }

    async fn reopen_poll(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReopenPollRequest>,
    ) -> ServiceResult<ReopenPollResponse> {
        handle(async {
            let poll = self.set_poll_closed(ctx.user_id()?, request.poll_id, false).await?;
            ok(ReopenPollResponse { poll: poll.into(), ..Default::default() })
        })
        .await
    }

    async fn finalize_poll(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, FinalizePollRequest>,
    ) -> ServiceResult<FinalizePollResponse> {
        handle(async {
            let (poll, plan) = Plans::finalize_poll(self, ctx.user_id()?, &request.to_owned_message()).await?;
            ok(FinalizePollResponse { poll: poll.into(), plan: plan.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_poll(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeletePollRequest>,
    ) -> ServiceResult<DeletePollResponse> {
        handle(async {
            Plans::delete_poll(self, ctx.user_id()?, request.poll_id).await?;
            ok(DeletePollResponse::default())
        })
        .await
    }

    async fn leave_poll(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, LeavePollRequest>,
    ) -> ServiceResult<LeavePollResponse> {
        handle(async {
            Plans::leave_poll(self, ctx.user_id()?, request.poll_id).await?;
            ok(LeavePollResponse::default())
        })
        .await
    }

    async fn remove_poll_participant(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemovePollParticipantRequest>,
    ) -> ServiceResult<RemovePollParticipantResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let poll =
                Plans::remove_poll_participant(self, user_id, request.poll_id, parse_id(request.user_id)?).await?;
            ok(RemovePollParticipantResponse { poll: poll.into(), ..Default::default() })
        })
        .await
    }

    async fn remind_poll_participants(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemindPollParticipantsRequest>,
    ) -> ServiceResult<RemindPollParticipantsResponse> {
        handle(async {
            let reminded_count = Plans::remind_poll_participants(self, ctx.user_id()?, request.poll_id).await?;
            ok(RemindPollParticipantsResponse { reminded_count, ..Default::default() })
        })
        .await
    }
}
