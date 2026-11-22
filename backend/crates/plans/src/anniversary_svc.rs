//! `AnniversaryService`: thin RPC handlers over [`crate::countdowns`],
//! [`crate::anniversaries`] and [`crate::birthdays`].
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::plans::*;

use crate::Plans;

/// Largest page of the countdown list.
const MAX_PAGE: i64 = 100;

impl AnniversaryService for Plans {
    async fn create_countdown(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateCountdownRequest>,
    ) -> ServiceResult<CreateCountdownResponse> {
        handle(async {
            let countdown = Plans::create_countdown(self, ctx.user_id()?, &request.to_owned_message()).await?;
            ok(CreateCountdownResponse { countdown: countdown.into(), ..Default::default() })
        })
        .await
    }

    async fn update_countdown(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateCountdownRequest>,
    ) -> ServiceResult<UpdateCountdownResponse> {
        handle(async {
            let countdown = Plans::update_countdown(self, ctx.user_id()?, &request.to_owned_message()).await?;
            ok(UpdateCountdownResponse { countdown: countdown.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_countdown(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteCountdownRequest>,
    ) -> ServiceResult<DeleteCountdownResponse> {
        handle(async {
            Plans::delete_countdown(self, ctx.user_id()?, parse_id(request.countdown_id)?).await?;
            ok(DeleteCountdownResponse::default())
        })
        .await
    }

    async fn get_countdown(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetCountdownRequest>,
    ) -> ServiceResult<GetCountdownResponse> {
        handle(async {
            let countdown = Plans::get_countdown(self, ctx.user_id()?, parse_id(request.countdown_id)?).await?;
            ok(GetCountdownResponse { countdown: countdown.into(), ..Default::default() })
        })
        .await
    }

    async fn list_countdowns(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListCountdownsRequest>,
    ) -> ServiceResult<ListCountdownsResponse> {
        handle(async {
            let (countdowns, page) =
                Plans::list_countdowns(self, ctx.user_id()?, &Page::from_view(request.page.as_option(), MAX_PAGE))
                    .await?;
            ok(ListCountdownsResponse { countdowns, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn start_friendship_counter(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, StartFriendshipCounterRequest>,
    ) -> ServiceResult<StartFriendshipCounterResponse> {
        handle(async {
            let anniversary =
                Plans::start_friendship_counter(self, ctx.user_id()?, parse_id(request.user_id)?, request.start_date)
                    .await?;
            ok(StartFriendshipCounterResponse { anniversary: anniversary.into(), ..Default::default() })
        })
        .await
    }

    async fn invite_couple(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, InviteCoupleRequest>,
    ) -> ServiceResult<InviteCoupleResponse> {
        handle(async {
            let anniversary =
                Plans::invite_couple(self, ctx.user_id()?, parse_id(request.user_id)?, request.start_date).await?;
            ok(InviteCoupleResponse { anniversary: anniversary.into(), ..Default::default() })
        })
        .await
    }

    async fn respond_couple_invite(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RespondCoupleInviteRequest>,
    ) -> ServiceResult<RespondCoupleInviteResponse> {
        handle(async {
            let anniversary =
                Plans::respond_couple_invite(self, ctx.user_id()?, parse_id(request.anniversary_id)?, request.accept)
                    .await?;
            ok(RespondCoupleInviteResponse { anniversary: anniversary.into(), ..Default::default() })
        })
        .await
    }

    async fn update_anniversary(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateAnniversaryRequest>,
    ) -> ServiceResult<UpdateAnniversaryResponse> {
        handle(async {
            let anniversary = Plans::update_anniversary(self, ctx.user_id()?, &request.to_owned_message()).await?;
            ok(UpdateAnniversaryResponse { anniversary: anniversary.into(), ..Default::default() })
        })
        .await
    }

    async fn end_anniversary(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, EndAnniversaryRequest>,
    ) -> ServiceResult<EndAnniversaryResponse> {
        handle(async {
            Plans::end_anniversary(self, ctx.user_id()?, parse_id(request.anniversary_id)?).await?;
            ok(EndAnniversaryResponse::default())
        })
        .await
    }

    async fn list_anniversaries(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListAnniversariesRequest>,
    ) -> ServiceResult<ListAnniversariesResponse> {
        handle(async {
            let anniversaries = Plans::list_anniversaries(self, ctx.user_id()?).await?;
            ok(ListAnniversariesResponse { anniversaries, ..Default::default() })
        })
        .await
    }

    async fn list_birthdays(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListBirthdaysRequest>,
    ) -> ServiceResult<ListBirthdaysResponse> {
        handle(async {
            let birthdays = Plans::list_birthdays(self, ctx.user_id()?).await?;
            ok(ListBirthdaysResponse { birthdays, ..Default::default() })
        })
        .await
    }

    async fn get_plans_settings(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetPlansSettingsRequest>,
    ) -> ServiceResult<GetPlansSettingsResponse> {
        handle(async {
            let settings = self.plans_settings(ctx.user_id()?).await?;
            ok(GetPlansSettingsResponse { settings: settings.into(), ..Default::default() })
        })
        .await
    }

    async fn update_plans_settings(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdatePlansSettingsRequest>,
    ) -> ServiceResult<UpdatePlansSettingsResponse> {
        handle(async {
            let settings = Plans::update_plans_settings(self, ctx.user_id()?, request.share_birthday).await?;
            ok(UpdatePlansSettingsResponse { settings: settings.into(), ..Default::default() })
        })
        .await
    }
}
