//! `FollowService` and `CircleService`: thin RPC handlers over [`crate::follows`],
//! [`crate::follow_suggestions`] and [`crate::circles`].
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::social::{follow_request::Target, *};

use crate::{Social, follows::FollowOutcome, people};

/// Parses a list of user IDs (`INVALID_ID` for any that is not one).
fn parse_ids(ids: &[&str]) -> AppResult<Vec<Uuid>> {
    ids.iter().map(|id| parse_id(id)).collect()
}

impl FollowService for Social {
    async fn follow(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, FollowRequest>,
    ) -> ServiceResult<FollowResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let target = match &req.target {
                Some(Target::UserId(id)) => parse_id(id)?,
                Some(Target::Handle(handle)) => people::visible_by_handle(&self.core, user_id, handle).await?,
                None => bail!(invalid, "MISSING_TARGET", "user_id or handle is required"),
            };
            let response = match Social::follow(self, user_id, target).await? {
                FollowOutcome::Following => {
                    FollowResponse { state: FollowState::FOLLOW_STATE_FOLLOWING.into(), ..Default::default() }
                }
                FollowOutcome::Requested(request) => FollowResponse {
                    state: FollowState::FOLLOW_STATE_REQUESTED.into(),
                    request: request.into(),
                    ..Default::default()
                },
            };
            ok(response)
        })
        .await
    }

    async fn unfollow(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnfollowRequest>,
    ) -> ServiceResult<UnfollowResponse> {
        handle(async {
            Social::unfollow(self, ctx.user_id()?, parse_id(request.user_id)?).await?;
            ok(UnfollowResponse::default())
        })
        .await
    }

    async fn remove_follower(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveFollowerRequest>,
    ) -> ServiceResult<RemoveFollowerResponse> {
        handle(async {
            Social::remove_follower(self, ctx.user_id()?, parse_id(request.user_id)?).await?;
            ok(RemoveFollowerResponse::default())
        })
        .await
    }

    async fn list_follow_requests(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListFollowRequestsRequest>,
    ) -> ServiceResult<ListFollowRequestsResponse> {
        handle(async {
            let direction =
                request.direction.as_known().unwrap_or(FollowRequestDirection::FOLLOW_REQUEST_DIRECTION_UNSPECIFIED);
            let page = Self::follows_page(request.page.as_option());
            let (requests, page, incoming_count) = self.list_follow_requests(ctx.user_id()?, direction, &page).await?;
            ok(ListFollowRequestsResponse { requests, page: page.into(), incoming_count, ..Default::default() })
        })
        .await
    }

    async fn respond_follow_request(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RespondFollowRequestRequest>,
    ) -> ServiceResult<RespondFollowRequestResponse> {
        handle(async {
            let follower =
                self.respond_follow_request(ctx.user_id()?, parse_id(request.request_id)?, request.approve).await?;
            ok(RespondFollowRequestResponse { follower: follower.into(), ..Default::default() })
        })
        .await
    }

    async fn cancel_follow_request(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CancelFollowRequestRequest>,
    ) -> ServiceResult<CancelFollowRequestResponse> {
        handle(async {
            self.cancel_follow_request(ctx.user_id()?, parse_id(request.request_id)?).await?;
            ok(CancelFollowRequestResponse::default())
        })
        .await
    }

    async fn list_follows(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListFollowsRequest>,
    ) -> ServiceResult<ListFollowsResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let owner = parse_opt_id(request.user_id)?.unwrap_or(viewer);
            let list = request.list.as_known().unwrap_or(FollowList::FOLLOW_LIST_UNSPECIFIED);
            let page = Self::follows_page(request.page.as_option());
            let (entries, page, total) = Social::list_follows(self, viewer, owner, list, request.query, &page).await?;
            ok(ListFollowsResponse { entries, page: page.into(), total, ..Default::default() })
        })
        .await
    }

    async fn get_follow_summary(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetFollowSummaryRequest>,
    ) -> ServiceResult<GetFollowSummaryResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let owner = parse_opt_id(request.user_id)?.unwrap_or(viewer);
            ok(GetFollowSummaryResponse {
                summary: self.follow_summary(viewer, owner).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_follow_prefs(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateFollowPrefsRequest>,
    ) -> ServiceResult<UpdateFollowPrefsResponse> {
        handle(async {
            let notify = request
                .notify_level
                .map(|level| level.as_known().unwrap_or(FollowNotifyLevel::FOLLOW_NOTIFY_LEVEL_UNSPECIFIED));
            let prefs = Social::update_follow_prefs(
                self,
                ctx.user_id()?,
                parse_id(request.user_id)?,
                request.muted,
                request.mute_seconds,
                notify,
            )
            .await?;
            ok(UpdateFollowPrefsResponse { prefs: prefs.into(), ..Default::default() })
        })
        .await
    }

    async fn get_follow_settings(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetFollowSettingsRequest>,
    ) -> ServiceResult<GetFollowSettingsResponse> {
        handle(async {
            let (settings, pending_request_count) = self.follow_settings(ctx.user_id()?).await?;
            ok(GetFollowSettingsResponse { settings: settings.into(), pending_request_count, ..Default::default() })
        })
        .await
    }

    async fn update_follow_settings(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateFollowSettingsRequest>,
    ) -> ServiceResult<UpdateFollowSettingsResponse> {
        handle(async {
            let locked = request.settings.as_option().is_some_and(|settings| settings.locked);
            let (settings, approved_request_count) =
                Social::update_follow_settings(self, ctx.user_id()?, locked).await?;
            ok(UpdateFollowSettingsResponse { settings: settings.into(), approved_request_count, ..Default::default() })
        })
        .await
    }

    async fn list_follow_suggestions(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListFollowSuggestionsRequest>,
    ) -> ServiceResult<ListFollowSuggestionsResponse> {
        handle(async {
            let suggestions = self.list_follow_suggestions(ctx.user_id()?, request.limit).await?;
            ok(ListFollowSuggestionsResponse { suggestions, ..Default::default() })
        })
        .await
    }

    async fn dismiss_follow_suggestion(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DismissFollowSuggestionRequest>,
    ) -> ServiceResult<DismissFollowSuggestionResponse> {
        handle(async {
            self.dismiss_follow_suggestion(ctx.user_id()?, parse_id(request.user_id)?).await?;
            ok(DismissFollowSuggestionResponse::default())
        })
        .await
    }
}

impl CircleService for Social {
    async fn list_circles(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListCirclesRequest>,
    ) -> ServiceResult<ListCirclesResponse> {
        handle(async {
            ok(ListCirclesResponse { circles: Social::list_circles(self, ctx.user_id()?).await?, ..Default::default() })
        })
        .await
    }

    async fn get_circle(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetCircleRequest>,
    ) -> ServiceResult<GetCircleResponse> {
        handle(async {
            let circle = Social::get_circle(self, ctx.user_id()?, parse_id(request.circle_id)?).await?;
            ok(GetCircleResponse { circle: circle.into(), ..Default::default() })
        })
        .await
    }

    async fn create_circle(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateCircleRequest>,
    ) -> ServiceResult<CreateCircleResponse> {
        handle(async {
            let circle =
                Social::create_circle(self, ctx.user_id()?, request.name, request.emoji, request.color.as_known())
                    .await?;
            ok(CreateCircleResponse { circle: circle.into(), ..Default::default() })
        })
        .await
    }

    async fn update_circle(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateCircleRequest>,
    ) -> ServiceResult<UpdateCircleResponse> {
        handle(async {
            let color = request.color.map(|color| color.as_known().unwrap_or(CircleColor::CIRCLE_COLOR_UNSPECIFIED));
            let circle = Social::update_circle(
                self,
                ctx.user_id()?,
                parse_id(request.circle_id)?,
                request.name,
                request.emoji,
                color,
            )
            .await?;
            ok(UpdateCircleResponse { circle: circle.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_circle(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteCircleRequest>,
    ) -> ServiceResult<DeleteCircleResponse> {
        handle(async {
            Social::delete_circle(self, ctx.user_id()?, parse_id(request.circle_id)?).await?;
            ok(DeleteCircleResponse::default())
        })
        .await
    }

    async fn reorder_circles(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReorderCirclesRequest>,
    ) -> ServiceResult<ReorderCirclesResponse> {
        handle(async {
            let order = parse_ids(&request.circle_ids.iter().copied().collect::<Vec<_>>())?;
            ok(ReorderCirclesResponse {
                circles: Social::reorder_circles(self, ctx.user_id()?, &order).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn add_circle_members(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AddCircleMembersRequest>,
    ) -> ServiceResult<AddCircleMembersResponse> {
        handle(async {
            let ids: Vec<&str> = request.user_ids.iter().copied().collect();
            ensure!(
                ids.len() <= crate::circles::MAX_IDS,
                invalid,
                "TOO_MANY_IDS",
                "at most {} people at a time",
                crate::circles::MAX_IDS
            );
            let circle =
                Social::add_circle_members(self, ctx.user_id()?, parse_id(request.circle_id)?, &parse_ids(&ids)?)
                    .await?;
            ok(AddCircleMembersResponse { circle: circle.into(), ..Default::default() })
        })
        .await
    }

    async fn remove_circle_members(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveCircleMembersRequest>,
    ) -> ServiceResult<RemoveCircleMembersResponse> {
        handle(async {
            let ids: Vec<&str> = request.user_ids.iter().copied().collect();
            ensure!(
                ids.len() <= crate::circles::MAX_IDS,
                invalid,
                "TOO_MANY_IDS",
                "at most {} people at a time",
                crate::circles::MAX_IDS
            );
            let circle =
                Social::remove_circle_members(self, ctx.user_id()?, parse_id(request.circle_id)?, &parse_ids(&ids)?)
                    .await?;
            ok(RemoveCircleMembersResponse { circle: circle.into(), ..Default::default() })
        })
        .await
    }

    async fn list_circle_members(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListCircleMembersRequest>,
    ) -> ServiceResult<ListCircleMembersResponse> {
        handle(async {
            let page = Self::circles_page(request.page.as_option());
            let (members, page, total) =
                Social::list_circle_members(self, ctx.user_id()?, parse_id(request.circle_id)?, &page).await?;
            ok(ListCircleMembersResponse { members, page: page.into(), total, ..Default::default() })
        })
        .await
    }

    async fn list_circle_candidates(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListCircleCandidatesRequest>,
    ) -> ServiceResult<ListCircleCandidatesResponse> {
        handle(async {
            let page = Self::circles_page(request.page.as_option());
            let (candidates, page) = Social::list_circle_candidates(
                self,
                ctx.user_id()?,
                parse_id(request.circle_id)?,
                request.query,
                &page,
            )
            .await?;
            ok(ListCircleCandidatesResponse { candidates, page: page.into(), ..Default::default() })
        })
        .await
    }
}
