//! `CommentModerationService`: the owner's comment manager, review queue
//! and screening rules.
use std::{collections::HashMap, time::Duration};

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::engagement::{__buffa::oneof::block_commenter_request::Target, *};

use crate::{
    Engagement,
    comments::{
        self,
        list::{ReceivedFilter, status_filter},
        moderate::{BlockRow, BlockTarget},
        render::author_for,
        screening::{self, SettingsRow},
    },
    page_of, text,
};

const PAGE: i64 = 30;
const PAGE_MAX: i64 = 100;
/// Changes to the screening rules one owner may make per minute.
const SETTINGS_CHANGES_PER_MINUTE: u32 = 30;

impl Engagement {
    /// Blocks as the owner sees them: under the identity they blocked.
    async fn blocked_commenters_proto(&self, blocks: &[BlockRow]) -> AppResult<Vec<BlockedCommenter>> {
        let user_ids: Vec<Uuid> = blocks.iter().filter(|block| block.channel_id.is_none()).map(|block| block.blocked_user_id).collect();
        let channel_ids: Vec<Uuid> = blocks.iter().filter_map(|block| block.channel_id).collect();
        let users = if user_ids.is_empty() { HashMap::new() } else { tokuly_identity::summaries(&self.core, &user_ids).await? };
        let channels = self.channel_summaries(&channel_ids).await?;
        Ok(blocks
            .iter()
            .map(|block| BlockedCommenter {
                id: block.id.to_string(),
                // Never the account behind a channel: the owner is "nobody" here.
                commenter: author_for(None, block.blocked_user_id, block.channel_id, &users, &channels).into(),
                created_at: ts(block.created_at),
                ..Default::default()
            })
            .collect())
    }
}

impl CommentModerationService for Engagement {
    async fn list_received_comments(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListReceivedCommentsRequest>,
    ) -> ServiceResult<ListReceivedCommentsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let request = request.to_owned_message();
            let page = page_of(request.page.as_option(), PAGE, PAGE_MAX);
            let mut filter = ReceivedFilter {
                status: status_filter(request.status.as_known().unwrap_or_default()),
                pattern: text::contains_pattern(&request.query),
                as_channel: match request.author_kind.as_known().unwrap_or_default() {
                    AuthorKind::AUTHOR_KIND_UNSPECIFIED => None,
                    AuthorKind::AUTHOR_KIND_USER => Some(false),
                    AuthorKind::AUTHOR_KIND_CHANNEL => Some(true),
                },
                created_after: request.created_after.as_option().map(from_ts),
                created_before: request.created_before.as_option().map(from_ts),
                ..Default::default()
            };
            if let Some(subject) = request.subject.as_option() {
                let subject = self.requested_subject(Some(subject), Some(user_id)).await?;
                ensure!(subject.is_owned_by(user_id), denied, "NOT_SUBJECT_OWNER", "only the owner of the content can do this");
                filter.subject = Some(subject.key);
            }
            if !request.author_of_comment_id.is_empty() {
                let found = self.require_comment(&request.author_of_comment_id, Some(user_id)).await?;
                ensure!(found.subject.is_owned_by(user_id), not_found, "COMMENT_NOT_FOUND", "comment not found");
                filter.identity = Some((found.row.author_id, found.row.channel_id));
            }
            let hidden = self.hidden_from(Some(user_id)).await?;
            let (rows, info) = self.received_comments(user_id, &hidden, &filter, &page).await?;
            ok(ListReceivedCommentsResponse {
                comments: self.in_context(&rows, user_id).await?,
                page: info.into(),
                held_count: comments::held_count(&self.core.db, user_id).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn approve_comment(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ApproveCommentRequest>,
    ) -> ServiceResult<ApproveCommentResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let found = Engagement::approve_comment(self, user_id, request.comment_id).await?;
            ok(ApproveCommentResponse { comment: self.rendered(&found, user_id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn hide_comment(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, HideCommentRequest>,
    ) -> ServiceResult<HideCommentResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let found = Engagement::hide_comment(self, user_id, request.comment_id).await?;
            ok(HideCommentResponse { comment: self.rendered(&found, user_id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn get_moderation_settings(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetModerationSettingsRequest>,
    ) -> ServiceResult<GetModerationSettingsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let settings = screening::settings_of(&self.core.db, user_id).await?;
            ok(GetModerationSettingsResponse { settings: settings.to_proto().into(), ..Default::default() })
        })
        .await
    }

    async fn update_moderation_settings(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateModerationSettingsRequest>,
    ) -> ServiceResult<UpdateModerationSettingsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let request = request.to_owned_message();
            let wanted = request.settings.as_option().cloned().unwrap_or_default();
            let settings = SettingsRow {
                hold_all: wanted.hold_all,
                hold_links: wanted.hold_links,
                blocked_words: text::blocked_words(&wanted.blocked_words)?,
            };
            self.core
                .cache
                .rate_limit(&format!("engagement:comment:settings:{user_id}"), SETTINGS_CHANGES_PER_MINUTE, Duration::from_secs(60))
                .await?;
            screening::save_settings(&self.core.db, user_id, &settings).await?;
            ok(UpdateModerationSettingsResponse { settings: settings.to_proto().into(), ..Default::default() })
        })
        .await
    }

    async fn block_commenter(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BlockCommenterRequest>,
    ) -> ServiceResult<BlockCommenterResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let request = request.to_owned_message();
            let target = match &request.target {
                Some(Target::UserId(blocked_id)) => BlockTarget::User(blocked_id),
                Some(Target::CommentId(comment_id)) => BlockTarget::AuthorOf(comment_id),
                None => bail!(not_found, "USER_NOT_FOUND", "say whom to block"),
            };
            let block = Engagement::block_commenter(self, user_id, target).await?;
            let blocked = self.blocked_commenters_proto(std::slice::from_ref(&block)).await?.pop();
            ok(BlockCommenterResponse { blocked: blocked.into(), ..Default::default() })
        })
        .await
    }

    async fn unblock_commenter(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnblockCommenterRequest>,
    ) -> ServiceResult<UnblockCommenterResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            Engagement::unblock_commenter(self, user_id, request.block_id).await?;
            ok(UnblockCommenterResponse::default())
        })
        .await
    }

    async fn list_blocked_commenters(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListBlockedCommentersRequest>,
    ) -> ServiceResult<ListBlockedCommentersResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let page = match request.page.as_option() {
                Some(page) => Page::new(page.limit, page.cursor, PAGE, PAGE_MAX),
                None => Page::new(0, "", PAGE, PAGE_MAX),
            };
            let (blocks, info) = self.blocked_commenters(user_id, &page).await?;
            ok(ListBlockedCommentersResponse {
                commenters: self.blocked_commenters_proto(&blocks).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }
}
