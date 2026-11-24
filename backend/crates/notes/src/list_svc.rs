//! `NoteListService`: thin handlers over [`crate::lists`].
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::notes::*;

use crate::{
    Notes,
    lists::{ListInput, member_ids},
    timelines::MAX_PAGE,
};

impl NoteListService for Notes {
    async fn create_list(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateListRequest>,
    ) -> ServiceResult<CreateListResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let input = ListInput::parse(request.name, request.description, request.public)?;
            let row = Notes::create_list(self, user_id, input).await?;
            ok(CreateListResponse { list: self.list_to_proto(&viewer, &row).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn update_list(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateListRequest>,
    ) -> ServiceResult<UpdateListResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let list_id = parse_id(request.list_id)?;
            let input = ListInput::parse(request.name, request.description, request.public)?;
            let row = Notes::update_list(self, user_id, list_id, input).await?;
            ok(UpdateListResponse { list: self.list_to_proto(&viewer, &row).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_list(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteListRequest>,
    ) -> ServiceResult<DeleteListResponse> {
        handle(async {
            Notes::delete_list(self, ctx.user_id()?, parse_id(request.list_id)?).await?;
            ok(DeleteListResponse::default())
        })
        .await
    }

    async fn get_list(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetListRequest>,
    ) -> ServiceResult<GetListResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let row = self.require_list(&viewer, parse_id(request.list_id)?).await?;
            ok(GetListResponse { list: self.list_to_proto(&viewer, &row).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn list_lists(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListListsRequest>,
    ) -> ServiceResult<ListListsResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let user_id = match parse_opt_id(request.user_id)? {
                Some(user_id) => user_id,
                None => ctx.user_id()?,
            };
            ok(ListListsResponse { lists: self.lists_of(&viewer, user_id).await?, ..Default::default() })
        })
        .await
    }

    async fn list_followed_lists(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListFollowedListsRequest>,
    ) -> ServiceResult<ListFollowedListsResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (lists, info) = self.followed_lists(&viewer, user_id, &page).await?;
            ok(ListFollowedListsResponse { lists, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn list_lists_containing_me(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListListsContainingMeRequest>,
    ) -> ServiceResult<ListListsContainingMeResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (lists, info) = self.lists_containing(&viewer, user_id, &page).await?;
            ok(ListListsContainingMeResponse { lists, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn add_list_members(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AddListMembersRequest>,
    ) -> ServiceResult<AddListMembersResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let request = request.to_owned_message();
            let list_id = parse_id(&request.list_id)?;
            let members = member_ids(&request.user_ids)?;
            let row = Notes::add_list_members(self, user_id, list_id, &members).await?;
            ok(AddListMembersResponse { list: self.list_to_proto(&viewer, &row).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn remove_list_members(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveListMembersRequest>,
    ) -> ServiceResult<RemoveListMembersResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let request = request.to_owned_message();
            let list_id = parse_id(&request.list_id)?;
            let members = member_ids(&request.user_ids)?;
            let row = Notes::remove_list_members(self, user_id, list_id, &members).await?;
            ok(RemoveListMembersResponse {
                list: self.list_to_proto(&viewer, &row).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_list_members(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListListMembersRequest>,
    ) -> ServiceResult<ListListMembersResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (members, info) = self.list_members(&viewer, parse_id(request.list_id)?, &page).await?;
            ok(ListListMembersResponse { members, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn leave_list(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, LeaveListRequest>,
    ) -> ServiceResult<LeaveListResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            Notes::leave_list(self, &viewer, user_id, parse_id(request.list_id)?).await?;
            ok(LeaveListResponse::default())
        })
        .await
    }

    async fn follow_list(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, FollowListRequest>,
    ) -> ServiceResult<FollowListResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let row = Notes::follow_list(self, &viewer, user_id, parse_id(request.list_id)?).await?;
            ok(FollowListResponse { list: self.list_to_proto(&viewer, &row).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn unfollow_list(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnfollowListRequest>,
    ) -> ServiceResult<UnfollowListResponse> {
        handle(async {
            Notes::unfollow_list(self, ctx.user_id()?, parse_id(request.list_id)?).await?;
            ok(UnfollowListResponse::default())
        })
        .await
    }

    async fn get_list_timeline(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetListTimelineRequest>,
    ) -> ServiceResult<GetListTimelineResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (entries, info) = self.list_timeline(&viewer, parse_id(request.list_id)?, &page).await?;
            ok(GetListTimelineResponse { entries, page: info.into(), ..Default::default() })
        })
        .await
    }
}
