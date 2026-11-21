//! `ConversationService`: thin RPC handlers. Each one authenticates the
//! caller, parses the request and delegates to the module that owns the
//! behaviour (and the authorization that goes with it).
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::{chat::*, common::PageRequestView};

use crate::{Chat, content, reactions::MAX_PINNED_MESSAGES, state::PreferenceChange};

/// A page request with this RPC's default and maximum size.
pub(crate) fn page_of(page: Option<&PageRequestView<'_>>, default: i64, max: i64) -> Page {
    match page {
        Some(page) => Page::new(page.limit, page.cursor, default, max),
        None => Page::new(0, "", default, max),
    }
}

impl Chat {
    /// Limits and optional features of this server.
    pub fn capabilities(&self) -> Capabilities {
        let seconds = |value: i64| value.clamp(0, i64::from(i32::MAX)) as i32;
        Capabilities {
            max_text_length: content::MAX_TEXT_CHARS as i32,
            max_attachments: content::MAX_ATTACHMENTS as i32,
            max_group_members: self.cfg.max_group_members,
            edit_window_seconds: seconds(self.cfg.edit_window_seconds),
            request_message_limit: self.cfg.request_message_limit,
            max_pinned_messages: MAX_PINNED_MESSAGES as i32,
            pov_answer_seconds: seconds(self.cfg.pov_answer_seconds),
            pov_reply_seconds: seconds(self.cfg.pov_reply_seconds),
            calls_enabled: self.livekit.is_some(),
            ring_seconds: seconds(self.cfg.ring_seconds),
            assist_enabled: self.assist.is_some(),
            ..Default::default()
        }
    }
}

impl ConversationService for Chat {
    async fn get_capabilities(&self, ctx: RequestContext, _request: ServiceRequest<'_, GetCapabilitiesRequest>) -> ServiceResult<GetCapabilitiesResponse> {
        handle(async {
            ctx.user_id()?;
            ok(GetCapabilitiesResponse { capabilities: self.capabilities().into(), ..Default::default() })
        })
        .await
    }

    async fn open_direct(&self, ctx: RequestContext, request: ServiceRequest<'_, OpenDirectRequest>) -> ServiceResult<OpenDirectResponse> {
        handle(async {
            let access = self.open_direct_as(ctx.user_id()?, parse_id(request.user_id)?).await?;
            ok(OpenDirectResponse { conversation: self.conversation_for(&access).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn create_group(&self, ctx: RequestContext, request: ServiceRequest<'_, CreateGroupRequest>) -> ServiceResult<CreateGroupResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let access = self.create_group(user_id, &req.name, &parse_ids(&req.member_ids)?, &req.icon_asset_id).await?;
            ok(CreateGroupResponse { conversation: self.conversation_for(&access).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn get_conversation(&self, ctx: RequestContext, request: ServiceRequest<'_, GetConversationRequest>) -> ServiceResult<GetConversationResponse> {
        handle(async {
            let access = self.access(ctx.user_id()?, parse_id(request.conversation_id)?).await?;
            let conversation_id = access.conv.id;
            let open_pov_request = self.open_pov(conversation_id).await?.map(|request| request.to_proto());
            let active_call = match self.live_call(conversation_id).await? {
                Some(call) => Some(self.call_to_proto(&call).await?),
                None => None,
            };
            ok(GetConversationResponse {
                conversation: self.conversation_for(&access).await?.into(),
                read_cursors: self.read_cursors(conversation_id).await?,
                present_user_ids: self.visible_present_users(conversation_id).await?.iter().map(Uuid::to_string).collect(),
                open_pov_request: open_pov_request.into(),
                active_call: active_call.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_conversations(&self, ctx: RequestContext, request: ServiceRequest<'_, ListConversationsRequest>) -> ServiceResult<ListConversationsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let filter = request.filter.as_known().unwrap_or(ConversationFilter::CONVERSATION_FILTER_INBOX);
            let page = page_of(request.page.as_option(), 30, 100);
            let (conversations, page) = self.list_conversations(user_id, filter, &page).await?;
            ok(ListConversationsResponse { conversations, page: page.into(), unread: self.unread_counts(user_id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn update_group(&self, ctx: RequestContext, request: ServiceRequest<'_, UpdateGroupRequest>) -> ServiceResult<UpdateGroupResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let access = self.update_group(user_id, parse_id(&req.conversation_id)?, req.name.as_deref(), req.icon_asset_id.as_deref()).await?;
            ok(UpdateGroupResponse { conversation: self.conversation_for(&access).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn list_members(&self, ctx: RequestContext, request: ServiceRequest<'_, ListMembersRequest>) -> ServiceResult<ListMembersResponse> {
        handle(async {
            let page = page_of(request.page.as_option(), 50, 100);
            let (members, page) = self.list_members(ctx.user_id()?, parse_id(request.conversation_id)?, &page).await?;
            ok(ListMembersResponse { members, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn add_members(&self, ctx: RequestContext, request: ServiceRequest<'_, AddMembersRequest>) -> ServiceResult<AddMembersResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let (access, added) = self.add_members(user_id, parse_id(&req.conversation_id)?, &parse_ids(&req.user_ids)?).await?;
            ok(AddMembersResponse { conversation: self.conversation_for(&access).await?.into(), added, ..Default::default() })
        })
        .await
    }

    async fn remove_member(&self, ctx: RequestContext, request: ServiceRequest<'_, RemoveMemberRequest>) -> ServiceResult<RemoveMemberResponse> {
        handle(async {
            let access = self.remove_member(ctx.user_id()?, parse_id(request.conversation_id)?, parse_id(request.user_id)?).await?;
            ok(RemoveMemberResponse { conversation: self.conversation_for(&access).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn set_member_role(&self, ctx: RequestContext, request: ServiceRequest<'_, SetMemberRoleRequest>) -> ServiceResult<SetMemberRoleResponse> {
        handle(async {
            let role = request.role.as_known().unwrap_or(MemberRole::MEMBER_ROLE_UNSPECIFIED);
            let member = self.set_member_role(ctx.user_id()?, parse_id(request.conversation_id)?, parse_id(request.user_id)?, role).await?;
            ok(SetMemberRoleResponse { member: member.into(), ..Default::default() })
        })
        .await
    }

    async fn leave_conversation(&self, ctx: RequestContext, request: ServiceRequest<'_, LeaveConversationRequest>) -> ServiceResult<LeaveConversationResponse> {
        handle(async {
            Chat::leave_conversation(self, ctx.user_id()?, parse_id(request.conversation_id)?).await?;
            ok(LeaveConversationResponse::default())
        })
        .await
    }

    async fn update_preferences(&self, ctx: RequestContext, request: ServiceRequest<'_, UpdatePreferencesRequest>) -> ServiceResult<UpdatePreferencesResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let change = PreferenceChange { pinned: req.pinned, muted: req.muted, muted_until: req.muted_until.as_option().map(from_ts), archived: req.archived };
            let access = Chat::update_preferences(self, user_id, parse_id(&req.conversation_id)?, change).await?;
            ok(UpdatePreferencesResponse { conversation: self.conversation_for(&access).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn hide_conversation(&self, ctx: RequestContext, request: ServiceRequest<'_, HideConversationRequest>) -> ServiceResult<HideConversationResponse> {
        handle(async {
            Chat::hide_conversation(self, ctx.user_id()?, parse_id(request.conversation_id)?, request.clear_history).await?;
            ok(HideConversationResponse::default())
        })
        .await
    }

    async fn respond_to_request(&self, ctx: RequestContext, request: ServiceRequest<'_, RespondToRequestRequest>) -> ServiceResult<RespondToRequestResponse> {
        handle(async {
            let accepted = Chat::respond_to_request(self, ctx.user_id()?, parse_id(request.conversation_id)?, request.accept).await?;
            let conversation = match accepted {
                Some(access) => Some(self.conversation_for(&access).await?),
                None => None,
            };
            ok(RespondToRequestResponse { conversation: conversation.into(), ..Default::default() })
        })
        .await
    }

    async fn mark_read(&self, ctx: RequestContext, request: ServiceRequest<'_, MarkReadRequest>) -> ServiceResult<MarkReadResponse> {
        handle(async { ok(Chat::mark_read(self, ctx.user_id()?, parse_id(request.conversation_id)?, parse_opt_id(request.message_id)?).await?) }).await
    }

    async fn get_unread_counts(&self, ctx: RequestContext, _request: ServiceRequest<'_, GetUnreadCountsRequest>) -> ServiceResult<GetUnreadCountsResponse> {
        handle(async { ok(GetUnreadCountsResponse { unread: self.unread_counts(ctx.user_id()?).await?.into(), ..Default::default() }) }).await
    }
}
