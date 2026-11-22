//! `CommunityModerationService`: the staff's tools.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::community::*;

use crate::{
    Community as Domain,
    access::{Caller, permission_denied},
    validate,
};

impl CommunityModerationService for Domain {
    async fn remove_member(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveMemberRequest>,
    ) -> ServiceResult<RemoveMemberResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let access = self.access(&caller, request.community_id).await?;
            self.kick_member(&access, parse_id(request.user_id)?, request.reason).await?;
            ok(RemoveMemberResponse::default())
        })
        .await
    }

    async fn ban_member(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BanMemberRequest>,
    ) -> ServiceResult<BanMemberResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let req = request.to_owned_message();
            let access = self.access(&caller, &req.community_id).await?;
            let expires_at = req.expires_at.as_option().map(from_ts);
            let ban = self.ban_member(&access, parse_id(&req.user_id)?, &req.reason, expires_at).await?;
            ok(BanMemberResponse { ban: ban.into(), ..Default::default() })
        })
        .await
    }

    async fn unban_member(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnbanMemberRequest>,
    ) -> ServiceResult<UnbanMemberResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let access = self.access(&caller, request.community_id).await?;
            self.unban_member(&access, parse_id(request.user_id)?).await?;
            ok(UnbanMemberResponse::default())
        })
        .await
    }

    async fn list_bans(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListBansRequest>,
    ) -> ServiceResult<ListBansResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let access = self.access(&caller, request.community_id).await?;
            let page = Page::from_view(request.page.as_option(), 100);
            let (bans, info) = self.list_bans(&access, &page).await?;
            ok(ListBansResponse { bans, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn timeout_member(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, TimeoutMemberRequest>,
    ) -> ServiceResult<TimeoutMemberResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let access = self.access(&caller, request.community_id).await?;
            let member = self
                .set_timeout(&access, parse_id(request.user_id)?, Some(request.duration_seconds), request.reason)
                .await?;
            ok(TimeoutMemberResponse { member: member.into(), ..Default::default() })
        })
        .await
    }

    async fn clear_timeout(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ClearTimeoutRequest>,
    ) -> ServiceResult<ClearTimeoutResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let access = self.access(&caller, request.community_id).await?;
            let member = self.set_timeout(&access, parse_id(request.user_id)?, None, "").await?;
            ok(ClearTimeoutResponse { member: member.into(), ..Default::default() })
        })
        .await
    }

    async fn get_safety_settings(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetSafetySettingsRequest>,
    ) -> ServiceResult<GetSafetySettingsResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let access = self.access(&caller, request.community_id).await?;
            access.require(Permission::PERMISSION_MANAGE_SAFETY)?;
            ok(GetSafetySettingsResponse {
                settings: self.safety_proto(&access.community).into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_safety_settings(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateSafetySettingsRequest>,
    ) -> ServiceResult<UpdateSafetySettingsResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let req = request.to_owned_message();
            let access = self.access(&caller, &req.community_id).await?;
            access.require(Permission::PERMISSION_MANAGE_SAFETY)?;
            let muted_words =
                req.muted_words.into_option().map(|list| validate::muted_words(&list.values)).transpose()?;
            let threshold = req.auto_hide_reports.map(validate::auto_hide_reports).transpose()?;
            let settings = self.update_safety(&access, muted_words, threshold).await?;
            ok(UpdateSafetySettingsResponse { settings: settings.into(), ..Default::default() })
        })
        .await
    }

    async fn set_slow_mode(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetSlowModeRequest>,
    ) -> ServiceResult<SetSlowModeResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let (channel, access) = self.open_channel(&caller, request.channel_id).await?;
            let channel = self.set_slow_mode(&access, &channel, request.seconds).await?;
            ok(SetSlowModeResponse { channel: channel.into(), ..Default::default() })
        })
        .await
    }

    async fn list_review_queue(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListReviewQueueRequest>,
    ) -> ServiceResult<ListReviewQueueResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let access = self.access(&caller, request.community_id).await?;
            let page = Page::from_view(request.page.as_option(), 50);
            let (items, info) = self.review_queue(&access, &page).await?;
            ok(ListReviewQueueResponse { items, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn review_post(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReviewPostRequest>,
    ) -> ServiceResult<ReviewPostResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let req = request.to_owned_message();
            let (post, access) = self.open_post(&caller, req.post.as_option()).await?;
            let decision = req.decision.as_known().unwrap_or(ReviewDecision::REVIEW_DECISION_UNSPECIFIED);
            self.review_post(&access, &post, decision, &req.reason).await?;
            ok(ReviewPostResponse::default())
        })
        .await
    }

    async fn reveal_author(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RevealAuthorRequest>,
    ) -> ServiceResult<RevealAuthorResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let req = request.to_owned_message();
            // A removed post still has an author: deleting it does not
            // put them out of the staff's reach.
            let (post, access) = self.open_post_for_staff(&caller, req.post.as_option()).await?;
            let (author, role) = self.reveal_author(&access, &post, &req.reason).await?;
            ok(RevealAuthorResponse { author: author.into(), role: role.into(), ..Default::default() })
        })
        .await
    }

    async fn get_content_history(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetContentHistoryRequest>,
    ) -> ServiceResult<GetContentHistoryResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let req = request.to_owned_message();
            ok(self.content_history(&caller, req.content.as_option()).await?)
        })
        .await
    }

    async fn list_moderation_log(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListModerationLogRequest>,
    ) -> ServiceResult<ListModerationLogResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let access = self.access(&caller, request.community_id).await?;
            // The log is for the staff, and for site moderation following up a report.
            if !access.has(Permission::PERMISSION_VIEW_MODERATION_LOG) && !access.site_moderator {
                return Err(permission_denied());
            }
            let page = Page::from_view(request.page.as_option(), 100);
            let (entries, info) = self.moderation_log(access.community.id, request.action.to_i32(), &page).await?;
            ok(ListModerationLogResponse { entries, page: info.into(), ..Default::default() })
        })
        .await
    }
}
