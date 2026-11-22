//! `FriendService`: thin RPC handlers. Each one authenticates the caller,
//! parses the request and delegates to the feature module that owns the
//! behaviour (and the authorization that goes with it).
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult, ServiceStream};
use tokuly_core::prelude::*;
use tokuly_proto::{
    identity::Relationship,
    social::{send_friend_request_request::Target, *},
};

use crate::{
    Social,
    friends::PrefsChange,
    people,
    requests::{RequestOutcome, RequestVia},
};

impl FriendService for Social {
    async fn send_friend_request(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SendFriendRequestRequest>,
    ) -> ServiceResult<SendFriendRequestResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let target = match &req.target {
                Some(Target::UserId(id)) => {
                    let id = parse_id(id)?;
                    people::ensure_visible(&self.core, user_id, id).await?;
                    id
                }
                Some(Target::Handle(handle)) => people::visible_by_handle(&self.core, user_id, handle).await?,
                None => bail!(invalid, "MISSING_TARGET", "user_id or handle is required"),
            };
            let source = req.source.as_known().unwrap_or(FriendSource::FRIEND_SOURCE_UNSPECIFIED);
            let response =
                match self.request_friendship(user_id, target, source, &req.message, RequestVia::User).await? {
                    RequestOutcome::Pending { request, .. } => SendFriendRequestResponse {
                        relationship: Relationship::RELATIONSHIP_REQUEST_SENT.into(),
                        request: request.into(),
                        ..Default::default()
                    },
                    RequestOutcome::Friends => SendFriendRequestResponse {
                        relationship: Relationship::RELATIONSHIP_FRIENDS.into(),
                        ..Default::default()
                    },
                };
            ok(response)
        })
        .await
    }

    async fn cancel_friend_request(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CancelFriendRequestRequest>,
    ) -> ServiceResult<CancelFriendRequestResponse> {
        handle(async {
            self.cancel_request(ctx.user_id()?, parse_id(request.request_id)?).await?;
            ok(CancelFriendRequestResponse::default())
        })
        .await
    }

    async fn respond_friend_request(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RespondFriendRequestRequest>,
    ) -> ServiceResult<RespondFriendRequestResponse> {
        handle(async {
            let friend = self.respond_to_request(ctx.user_id()?, parse_id(request.request_id)?, request.accept).await?;
            ok(RespondFriendRequestResponse { friend: friend.into(), ..Default::default() })
        })
        .await
    }

    async fn list_friend_requests(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListFriendRequestsRequest>,
    ) -> ServiceResult<ListFriendRequestsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let direction =
                request.direction.as_known().unwrap_or(FriendRequestDirection::FRIEND_REQUEST_DIRECTION_UNSPECIFIED);
            let page = Self::requests_page(request.page.as_option());
            let (requests, page, incoming_count) = self.list_requests(user_id, direction, &page).await?;
            ok(ListFriendRequestsResponse { requests, page: page.into(), incoming_count, ..Default::default() })
        })
        .await
    }

    async fn list_friends(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListFriendsRequest>,
    ) -> ServiceResult<ListFriendsResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let owner = parse_opt_id(request.user_id)?.unwrap_or(viewer);
            let page = Self::friends_page(request.page.as_option());
            let (friends, page, total) = self.list_friends(viewer, owner, request.query, &page).await?;
            ok(ListFriendsResponse { friends, page: page.into(), total, ..Default::default() })
        })
        .await
    }

    async fn remove_friend(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveFriendRequest>,
    ) -> ServiceResult<RemoveFriendResponse> {
        handle(async {
            Social::remove_friend(self, ctx.user_id()?, parse_id(request.user_id)?).await?;
            ok(RemoveFriendResponse::default())
        })
        .await
    }

    async fn get_friendship(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetFriendshipRequest>,
    ) -> ServiceResult<GetFriendshipResponse> {
        handle(async {
            let friendship = self.friendship(ctx.user_id()?, parse_id(request.user_id)?).await?;
            ok(GetFriendshipResponse { friendship: friendship.into(), ..Default::default() })
        })
        .await
    }

    async fn update_friend_prefs(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateFriendPrefsRequest>,
    ) -> ServiceResult<UpdateFriendPrefsResponse> {
        handle(async {
            let change = PrefsChange { nickname: request.nickname, favorite: request.favorite, muted: request.muted };
            let friend = Social::update_friend_prefs(self, ctx.user_id()?, parse_id(request.user_id)?, change).await?;
            ok(UpdateFriendPrefsResponse { friend: friend.into(), ..Default::default() })
        })
        .await
    }

    async fn list_best_friends(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListBestFriendsRequest>,
    ) -> ServiceResult<ListBestFriendsResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let owner = parse_opt_id(request.user_id)?.unwrap_or(viewer);
            ok(ListBestFriendsResponse {
                friends: Social::list_best_friends(self, viewer, owner).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn list_suggestions(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListSuggestionsRequest>,
    ) -> ServiceResult<ListSuggestionsResponse> {
        handle(async {
            let suggestions = Social::list_suggestions(self, ctx.user_id()?, request.limit).await?;
            ok(ListSuggestionsResponse { suggestions, ..Default::default() })
        })
        .await
    }

    async fn dismiss_suggestion(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DismissSuggestionRequest>,
    ) -> ServiceResult<DismissSuggestionResponse> {
        handle(async {
            Social::dismiss_suggestion(self, ctx.user_id()?, parse_id(request.user_id)?).await?;
            ok(DismissSuggestionResponse::default())
        })
        .await
    }

    async fn match_contacts(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, MatchContactsRequest>,
    ) -> ServiceResult<MatchContactsResponse> {
        handle(async {
            let matches = Social::match_contacts(self, ctx.user_id()?, &request.contact_hashes).await?;
            ok(MatchContactsResponse { matches, ..Default::default() })
        })
        .await
    }

    async fn block_user(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BlockUserRequest>,
    ) -> ServiceResult<BlockUserResponse> {
        handle(async {
            Social::block_user(self, ctx.user_id()?, parse_id(request.user_id)?).await?;
            ok(BlockUserResponse::default())
        })
        .await
    }

    async fn unblock_user(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnblockUserRequest>,
    ) -> ServiceResult<UnblockUserResponse> {
        handle(async {
            Social::unblock_user(self, ctx.user_id()?, parse_id(request.user_id)?).await?;
            ok(UnblockUserResponse::default())
        })
        .await
    }

    async fn list_blocked_users(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListBlockedUsersRequest>,
    ) -> ServiceResult<ListBlockedUsersResponse> {
        handle(async {
            let page = Self::blocks_page(request.page.as_option());
            let (users, page) = self.list_blocked(ctx.user_id()?, &page).await?;
            ok(ListBlockedUsersResponse { users, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn get_my_invite(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetMyInviteRequest>,
    ) -> ServiceResult<GetMyInviteResponse> {
        handle(async {
            ok(GetMyInviteResponse { invite: self.my_invite(ctx.user_id()?).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn reset_my_invite(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ResetMyInviteRequest>,
    ) -> ServiceResult<ResetMyInviteResponse> {
        handle(async {
            ok(ResetMyInviteResponse { invite: self.reset_invite(ctx.user_id()?).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn resolve_invite(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ResolveInviteRequest>,
    ) -> ServiceResult<ResolveInviteResponse> {
        handle(async {
            // Public: the sign-up page shows who invited the visitor.
            let viewer = ctx.maybe_user_id()?;
            let (inviter, relationship) = Social::resolve_invite(self, viewer, request.code, &ctx.client().ip).await?;
            ok(ResolveInviteResponse {
                inviter: inviter.into(),
                relationship: relationship.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_intros(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListIntrosRequest>,
    ) -> ServiceResult<ListIntrosResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let owner = parse_opt_id(request.user_id)?.unwrap_or(viewer);
            let page = Self::intros_page(request.page.as_option());
            let (users, page, total) = Social::list_intros(self, viewer, owner, &page).await?;
            ok(ListIntrosResponse { users, page: page.into(), total, ..Default::default() })
        })
        .await
    }

    async fn discover_nearby(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DiscoverNearbyRequest>,
    ) -> ServiceResult<ServiceStream<DiscoverNearbyResponse>> {
        handle(async {
            let user_id = ctx.user_id()?;
            let location = request
                .location
                .as_option()
                .ok_or_else(|| AppError::invalid("INVALID_LOCATION", "a valid location is required"))?;
            let stream =
                Social::discover_nearby(self, user_id, location.lat, location.lng, request.accuracy_meters).await?;
            Ok(Response::new(stream))
        })
        .await
    }
}
