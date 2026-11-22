//! `CommunityService`: communities, membership, invites and roles.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::community::*;
use tokuly_social::graph;

use crate::{Community as Domain, access::Caller, communities::CommunityEdit, validate};

/// An asset ID as update requests carry it: unset = keep, empty = remove.
fn asset_change(value: Option<&str>) -> AppResult<Option<Option<Uuid>>> {
    value.map(parse_opt_id).transpose()
}

impl CommunityService for Domain {
    async fn create_community(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateCommunityRequest>,
    ) -> ServiceResult<CreateCommunityResponse> {
        handle(async {
            let (caller, user_id) = Caller::signed_in(&ctx)?;
            let community = self.create_community(user_id, request.to_owned_message()).await?;
            let access = self.stance(&caller, community).await?;
            ok(CreateCommunityResponse {
                community: self.render_community(&access).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_community(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetCommunityRequest>,
    ) -> ServiceResult<GetCommunityResponse> {
        handle(async {
            let caller = Caller::of(&ctx)?;
            let access = self.access(&caller, request.community_id).await?;
            ok(GetCommunityResponse { community: self.render_community(&access).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn update_community(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateCommunityRequest>,
    ) -> ServiceResult<UpdateCommunityResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let req = request.to_owned_message();
            let access = self.access(&caller, &req.community_id).await?;
            let edit = CommunityEdit {
                name: req.name.as_deref().map(validate::community_name).transpose()?,
                handle: req.handle.as_deref().map(validate::handle).transpose()?,
                description: req
                    .description
                    .as_deref()
                    .map(|d| validate::description(d, validate::MAX_DESCRIPTION_CHARS))
                    .transpose()?,
                category: req.category.map(validate::category).transpose()?,
                visibility: req.visibility.map(validate::visibility).transpose()?,
                language: req.language.as_deref().map(validate::language).transpose()?,
                tags: req
                    .tags
                    .into_option()
                    .map(|list| validate::tags(&list.values, validate::MAX_TAGS))
                    .transpose()?,
                rules: req.rules.into_option().map(|list| validate::rules(list.rules)).transpose()?,
                join_questions: req
                    .join_questions
                    .into_option()
                    .map(|list| validate::join_questions(&list.values))
                    .transpose()?,
                icon_asset_id: asset_change(req.icon_asset_id.as_deref())?,
                banner_asset_id: asset_change(req.banner_asset_id.as_deref())?,
                school_name: req.school_name,
                settings: req.settings.into_option(),
            };
            let updated = self.update_community(&access, edit).await?;
            let access = self.stance(&caller, updated).await?;
            ok(UpdateCommunityResponse {
                community: self.render_community(&access).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_community(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteCommunityRequest>,
    ) -> ServiceResult<DeleteCommunityResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let access = self.access(&caller, request.community_id).await?;
            self.delete_community(&access).await?;
            ok(DeleteCommunityResponse::default())
        })
        .await
    }

    async fn check_handle(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CheckHandleRequest>,
    ) -> ServiceResult<CheckHandleResponse> {
        handle(async {
            let limiter = ctx.maybe_user_id()?.map(|id| id.to_string()).unwrap_or_else(|| ctx.client().ip);
            self.core.cache.rate_limit(&format!("community:handle:{limiter}"), 60, Duration::from_secs(60)).await?;
            let (available, reason, normalized) = self.check_handle(request.handle).await?;
            ok(CheckHandleResponse { available, reason: reason.to_owned(), normalized, ..Default::default() })
        })
        .await
    }

    async fn transfer_ownership(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, TransferOwnershipRequest>,
    ) -> ServiceResult<TransferOwnershipResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let access = self.access(&caller, request.community_id).await?;
            let updated = self.transfer_ownership(&access, parse_id(request.user_id)?).await?;
            let access = self.stance(&caller, updated).await?;
            ok(TransferOwnershipResponse {
                community: self.render_community(&access).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_my_communities(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListMyCommunitiesRequest>,
    ) -> ServiceResult<ListMyCommunitiesResponse> {
        handle(async {
            let (caller, user_id) = Caller::signed_in(&ctx)?;
            let page = Page::from_view(request.page.as_option(), 100);
            let first_page = page.cursor.is_none();
            let (rows, info) = self.my_communities(user_id, &page).await?;
            let requested = if first_page { self.requested_communities(user_id).await? } else { Vec::new() };
            ok(ListMyCommunitiesResponse {
                communities: self.render_communities(&caller, &rows).await?,
                requested: self.render_communities(&caller, &requested).await?,
                page: info.into(),
                max_owned: self.cfg.max_owned as i32,
                owned_count: self.owned_count(&self.core.db, user_id).await? as i32,
                ..Default::default()
            })
        })
        .await
    }

    async fn list_user_communities(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListUserCommunitiesRequest>,
    ) -> ServiceResult<ListUserCommunitiesResponse> {
        handle(async {
            let caller = Caller::of(&ctx)?;
            let user_id = parse_id(request.user_id)?;
            let not_found = || AppError::not_found("USER_NOT_FOUND", "user not found");
            if tokuly_identity::users::filter_active(&self.core, &[user_id]).await?.is_empty() {
                return Err(not_found());
            }
            if let Some(viewer) = caller.user_id.filter(|viewer| *viewer != user_id)
                && graph::is_blocked_either(&self.core, viewer, user_id).await?
            {
                return Err(not_found());
            }
            let page = Page::from_view(request.page.as_option(), 100);
            let (rows, info) = self.shown_communities(user_id, &page).await?;
            ok(ListUserCommunitiesResponse {
                communities: self.render_communities(&caller, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn join_community(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, JoinCommunityRequest>,
    ) -> ServiceResult<JoinCommunityResponse> {
        handle(async {
            let (caller, user_id) = Caller::signed_in(&ctx)?;
            let req = request.to_owned_message();
            let (status, community) = self.join(&caller, user_id, &req).await?;
            // The community as it is now: its member count includes the caller.
            let community = self.community_by_id(community.id).await?.unwrap_or(community);
            let access = self.stance(&caller, community).await?;
            ok(JoinCommunityResponse {
                status: status.into(),
                community: self.render_community(&access).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn preview_invite(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, PreviewInviteRequest>,
    ) -> ServiceResult<PreviewInviteResponse> {
        handle(async {
            let caller = Caller::of(&ctx)?;
            // Codes are guessable only by trying: keep the tries few.
            let limiter = caller.user_id.map(|id| id.to_string()).unwrap_or_else(|| ctx.client().ip);
            self.core
                .cache
                .rate_limit(&format!("community:invite-preview:{limiter}"), 30, Duration::from_secs(60))
                .await?;
            let (community, inviter) = self.preview_invite(&caller, request.invite_code).await?;
            ok(PreviewInviteResponse { community: community.into(), inviter: inviter.into(), ..Default::default() })
        })
        .await
    }

    async fn leave_community(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, LeaveCommunityRequest>,
    ) -> ServiceResult<LeaveCommunityResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let access = self.access(&caller, request.community_id).await?;
            self.leave(&access).await?;
            ok(LeaveCommunityResponse::default())
        })
        .await
    }

    async fn cancel_join_request(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CancelJoinRequestRequest>,
    ) -> ServiceResult<CancelJoinRequestResponse> {
        handle(async {
            let (caller, user_id) = Caller::signed_in(&ctx)?;
            self.cancel_request(&caller, user_id, request.community_id).await?;
            ok(CancelJoinRequestResponse::default())
        })
        .await
    }

    async fn list_join_requests(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListJoinRequestsRequest>,
    ) -> ServiceResult<ListJoinRequestsResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let access = self.access(&caller, request.community_id).await?;
            let page = Page::from_view(request.page.as_option(), 100);
            let (requests, info) = self.list_requests(&access, &page).await?;
            ok(ListJoinRequestsResponse { requests, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn respond_join_request(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RespondJoinRequestRequest>,
    ) -> ServiceResult<RespondJoinRequestResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let member = self.respond_request(&caller, parse_id(request.request_id)?, request.approve).await?;
            ok(RespondJoinRequestResponse { member: member.into(), ..Default::default() })
        })
        .await
    }

    async fn list_members(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListMembersRequest>,
    ) -> ServiceResult<ListMembersResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let access = self.access(&caller, request.community_id).await?;
            let page = Page::from_view(request.page.as_option(), 100);
            let role = request.role.to_i32();
            let (members, info) = self.list_members(&access, request.query, role, &page).await?;
            ok(ListMembersResponse { members, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn get_member(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetMemberRequest>,
    ) -> ServiceResult<GetMemberResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let access = self.access(&caller, request.community_id).await?;
            let member = self.get_member(&access, parse_id(request.user_id)?).await?;
            ok(GetMemberResponse { member: member.into(), ..Default::default() })
        })
        .await
    }

    async fn set_member_role(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetMemberRoleRequest>,
    ) -> ServiceResult<SetMemberRoleResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let access = self.access(&caller, request.community_id).await?;
            let role = request.role.as_known().unwrap_or(MemberRole::MEMBER_ROLE_UNSPECIFIED);
            let member = self.set_member_role(&access, parse_id(request.user_id)?, role).await?;
            ok(SetMemberRoleResponse { member: member.into(), ..Default::default() })
        })
        .await
    }

    async fn update_membership(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateMembershipRequest>,
    ) -> ServiceResult<UpdateMembershipResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let req = request.to_owned_message();
            let access = self.access(&caller, &req.community_id).await?;
            let level = req.notification_level.map(validate::notification_level).transpose()?;
            let viewer = self.update_membership(&access, level, req.show_on_profile).await?;
            ok(UpdateMembershipResponse { viewer: viewer.into(), ..Default::default() })
        })
        .await
    }

    async fn create_invite(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateInviteRequest>,
    ) -> ServiceResult<CreateInviteResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let req = request.to_owned_message();
            let access = self.access(&caller, &req.community_id).await?;
            let invite = self.create_invite(&access, &req).await?;
            ok(CreateInviteResponse { invite: invite.into(), ..Default::default() })
        })
        .await
    }

    async fn list_invites(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListInvitesRequest>,
    ) -> ServiceResult<ListInvitesResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let access = self.access(&caller, request.community_id).await?;
            let page = Page::from_view(request.page.as_option(), 100);
            let (invites, info) = self.list_invites(&access, &page).await?;
            ok(ListInvitesResponse { invites, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn revoke_invite(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RevokeInviteRequest>,
    ) -> ServiceResult<RevokeInviteResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            self.revoke_invite(&caller, parse_id(request.invite_id)?).await?;
            ok(RevokeInviteResponse::default())
        })
        .await
    }
}
