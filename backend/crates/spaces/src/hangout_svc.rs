//! `HangoutService`: thin handlers over [`crate::hangouts`].
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::spaces::*;

use crate::Spaces;

impl HangoutService for Spaces {
    async fn create_hangout(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateHangoutRequest>,
    ) -> ServiceResult<CreateHangoutResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let hangout = Spaces::create_hangout(self, user_id, request.to_owned_message()).await?;
            ok(CreateHangoutResponse {
                hangout: self.render_hangout(&hangout, Some(user_id)).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_hangout(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateHangoutRequest>,
    ) -> ServiceResult<UpdateHangoutResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let hangout = Spaces::update_hangout(self, user_id, request.to_owned_message()).await?;
            ok(UpdateHangoutResponse {
                hangout: self.render_hangout(&hangout, Some(user_id)).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_hangout(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteHangoutRequest>,
    ) -> ServiceResult<DeleteHangoutResponse> {
        handle(async {
            Spaces::delete_hangout(self, ctx.user_id()?, parse_id(request.hangout_id)?).await?;
            ok(DeleteHangoutResponse::default())
        })
        .await
    }

    async fn get_hangout(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetHangoutRequest>,
    ) -> ServiceResult<GetHangoutResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let (hangout, _) = self.my_hangout(user_id, parse_id(request.hangout_id)?).await?;
            ok(GetHangoutResponse {
                hangout: self.render_hangout(&hangout, Some(user_id)).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_hangouts(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListHangoutsRequest>,
    ) -> ServiceResult<ListHangoutsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_discovery(Some(user_id), "").await?;
            let (hangouts, invitations) = Spaces::list_hangouts(self, user_id).await?;
            ok(ListHangoutsResponse { hangouts, invitations, ..Default::default() })
        })
        .await
    }

    async fn invite_to_hangout(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, InviteToHangoutRequest>,
    ) -> ServiceResult<InviteToHangoutResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let request = request.to_owned_message();
            let (hangout, invited_count) =
                Spaces::invite_to_hangout(self, user_id, parse_id(&request.hangout_id)?, &request.user_ids).await?;
            ok(InviteToHangoutResponse {
                hangout: self.render_hangout(&hangout, Some(user_id)).await?.into(),
                invited_count,
                ..Default::default()
            })
        })
        .await
    }

    async fn respond_to_hangout_invite(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RespondToHangoutInviteRequest>,
    ) -> ServiceResult<RespondToHangoutInviteResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let accepted =
                Spaces::respond_to_hangout_invite(self, user_id, parse_id(request.hangout_id)?, request.accept).await?;
            let hangout = match accepted {
                Some(hangout) => Some(self.render_hangout(&hangout, Some(user_id)).await?),
                None => None,
            };
            ok(RespondToHangoutInviteResponse { hangout: hangout.into(), ..Default::default() })
        })
        .await
    }

    async fn remove_hangout_member(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveHangoutMemberRequest>,
    ) -> ServiceResult<RemoveHangoutMemberResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let hangout =
                Spaces::remove_hangout_member(self, user_id, parse_id(request.hangout_id)?, parse_id(request.user_id)?)
                    .await?;
            ok(RemoveHangoutMemberResponse {
                hangout: self.render_hangout(&hangout, Some(user_id)).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn quit_hangout(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, QuitHangoutRequest>,
    ) -> ServiceResult<QuitHangoutResponse> {
        handle(async {
            Spaces::quit_hangout(self, ctx.user_id()?, parse_id(request.hangout_id)?).await?;
            ok(QuitHangoutResponse::default())
        })
        .await
    }

    async fn transfer_hangout(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, TransferHangoutRequest>,
    ) -> ServiceResult<TransferHangoutResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let hangout =
                Spaces::transfer_hangout(self, user_id, parse_id(request.hangout_id)?, parse_id(request.user_id)?)
                    .await?;
            ok(TransferHangoutResponse {
                hangout: self.render_hangout(&hangout, Some(user_id)).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn set_hangout_muted(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetHangoutMutedRequest>,
    ) -> ServiceResult<SetHangoutMutedResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let hangout =
                Spaces::set_hangout_muted(self, user_id, parse_id(request.hangout_id)?, request.muted).await?;
            ok(SetHangoutMutedResponse {
                hangout: self.render_hangout(&hangout, Some(user_id)).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn join_hangout(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, JoinHangoutRequest>,
    ) -> ServiceResult<JoinHangoutResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let (hangout, credentials) = Spaces::join_hangout(self, user_id, parse_id(request.hangout_id)?).await?;
            ok(JoinHangoutResponse {
                hangout: self.render_hangout(&hangout, Some(user_id)).await?.into(),
                credentials: credentials.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn leave_hangout(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, LeaveHangoutRequest>,
    ) -> ServiceResult<LeaveHangoutResponse> {
        handle(async {
            Spaces::leave_hangout(self, ctx.user_id()?, parse_id(request.hangout_id)?).await?;
            ok(LeaveHangoutResponse::default())
        })
        .await
    }
}
