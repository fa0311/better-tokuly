//! `CommentService`: reading and writing comments.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::engagement::*;

use crate::{
    Engagement,
    comments::{CommentRow, Found, render::Owners},
    page_of, stats,
    subjects::{SubjectKey, preview_of},
};

/// Page sizes of the comment lists of one subject.
const THREAD_PAGE: i64 = 20;
const THREAD_PAGE_MAX: i64 = 50;
/// Page sizes of "my comments".
const MINE_PAGE: i64 = 30;
const MINE_PAGE_MAX: i64 = 50;

impl Engagement {
    /// The comment of a moderation or author action, as its actor sees it.
    pub(crate) async fn rendered(&self, found: &Found, viewer: Uuid) -> AppResult<Comment> {
        self.render_comment(&found.row, Some(viewer), found.owner_id()).await
    }

    /// Comments of a list spanning subjects, each with a preview of what it
    /// is on. Subjects that no longer resolve for the viewer come back as
    /// "no longer available".
    pub(crate) async fn in_context(&self, rows: &[CommentRow], viewer: Uuid) -> AppResult<Vec<CommentInContext>> {
        let keys: Vec<SubjectKey> = rows.iter().map(CommentRow::key).collect();
        let subjects = self.subjects(&keys, Some(viewer)).await?;
        let owners: Owners = subjects.iter().map(|(key, subject)| (*key, subject.info.owner_id)).collect();
        let comments = self.render_comments(rows, Some(viewer), &owners).await?;
        Ok(rows
            .iter()
            .zip(comments)
            .map(|(row, comment)| CommentInContext {
                comment: comment.into(),
                subject: preview_of(row.key(), subjects.get(&row.key())).into(),
                ..Default::default()
            })
            .collect())
    }
}

impl CommentService for Engagement {
    async fn list_comments(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListCommentsRequest>,
    ) -> ServiceResult<ListCommentsResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            let request = request.to_owned_message();
            let subject = self.requested_subject(request.subject.as_option(), viewer).await?;
            let page = page_of(request.page.as_option(), THREAD_PAGE, THREAD_PAGE_MAX);
            let viewer_can_moderate = viewer.is_some_and(|viewer| subject.is_owned_by(viewer));
            let comments_enabled = subject.info.interactions_enabled;
            if !comments_enabled && !viewer_can_moderate {
                // Comments are turned off: what was written stays out of sight.
                page.decode::<serde_json::Value>()?;
                return ok(ListCommentsResponse { comments_enabled, ..Default::default() });
            }
            let hidden = self.hidden_from(viewer).await?;
            let sort = request.sort.as_known().unwrap_or_default();
            let (mut rows, info) = self.top_level_comments(subject.key, viewer, &hidden, sort, &page).await?;
            if page.cursor.is_none()
                && let Some(pinned) = self.pinned_comment(subject.key, &hidden).await?
            {
                rows.insert(0, pinned);
            }
            let owners = Owners::from([(subject.key, subject.info.owner_id)]);
            ok(ListCommentsResponse {
                comments: self.render_comments(&rows, viewer, &owners).await?,
                page: info.into(),
                total_count: stats::load(&self.core, &[subject.key], self.cfg.reaction_emoji_per_subject)
                    .await?
                    .of(subject.key)
                    .comment_count,
                comments_enabled,
                viewer_can_moderate,
                ..Default::default()
            })
        })
        .await
    }

    async fn list_replies(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListRepliesRequest>,
    ) -> ServiceResult<ListRepliesResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            let found = self.require_comment(request.comment_id, viewer).await?;
            // Threads are one level deep: a reply has no replies of its own.
            ensure!(!found.row.is_reply(), not_found, "COMMENT_NOT_FOUND", "comment not found");
            let page = match request.page.as_option() {
                Some(page) => Page::new(page.limit, page.cursor, THREAD_PAGE, THREAD_PAGE_MAX),
                None => Page::new(0, "", THREAD_PAGE, THREAD_PAGE_MAX),
            };
            let is_owner = viewer.is_some_and(|viewer| found.subject.is_owned_by(viewer));
            if !found.subject.info.interactions_enabled && !is_owner {
                // Comments are turned off: like the top-level list, this one is empty.
                page.decode::<serde_json::Value>()?;
                return ok(ListRepliesResponse::default());
            }
            let (rows, info) = self.replies(found.row.id, viewer, &found.hidden, &page).await?;
            let owners = Owners::from([(found.subject.key, found.owner_id())]);
            ok(ListRepliesResponse {
                replies: self.render_comments(&rows, viewer, &owners).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_comment(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetCommentRequest>,
    ) -> ServiceResult<GetCommentResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            let found = self.require_comment(request.comment_id, viewer).await?;
            let mut rows = vec![found.row.clone()];
            rows.extend(found.visible_parent(viewer).cloned());
            let owners = Owners::from([(found.subject.key, found.owner_id())]);
            let mut rendered = self.render_comments(&rows, viewer, &owners).await?.into_iter();
            ok(GetCommentResponse {
                comment: rendered.next().into(),
                parent: rendered.next().into(),
                subject: found.subject.preview().into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn create_comment(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateCommentRequest>,
    ) -> ServiceResult<CreateCommentResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let request = request.to_owned_message();
            let (row, subject) = Engagement::create_comment(self, user_id, &request).await?;
            ok(CreateCommentResponse {
                comment: self.render_comment(&row, Some(user_id), subject.info.owner_id).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_comment(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateCommentRequest>,
    ) -> ServiceResult<UpdateCommentResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let (row, subject) = Engagement::update_comment(self, user_id, request.comment_id, request.text).await?;
            ok(UpdateCommentResponse {
                comment: self.render_comment(&row, Some(user_id), subject.info.owner_id).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_comment(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteCommentRequest>,
    ) -> ServiceResult<DeleteCommentResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let removed_count = Engagement::delete_comment(self, user_id, request.comment_id).await?;
            ok(DeleteCommentResponse { removed_count, ..Default::default() })
        })
        .await
    }

    async fn pin_comment(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, PinCommentRequest>,
    ) -> ServiceResult<PinCommentResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let found = Engagement::pin_comment(self, user_id, request.comment_id, true).await?;
            ok(PinCommentResponse { comment: self.rendered(&found, user_id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn unpin_comment(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnpinCommentRequest>,
    ) -> ServiceResult<UnpinCommentResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let found = Engagement::pin_comment(self, user_id, request.comment_id, false).await?;
            ok(UnpinCommentResponse { comment: self.rendered(&found, user_id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn set_comment_heart(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetCommentHeartRequest>,
    ) -> ServiceResult<SetCommentHeartResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let found = self.heart_comment(user_id, request.comment_id, request.hearted).await?;
            ok(SetCommentHeartResponse { comment: self.rendered(&found, user_id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn list_my_comments(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListMyCommentsRequest>,
    ) -> ServiceResult<ListMyCommentsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let page = match request.page.as_option() {
                Some(page) => Page::new(page.limit, page.cursor, MINE_PAGE, MINE_PAGE_MAX),
                None => Page::new(0, "", MINE_PAGE, MINE_PAGE_MAX),
            };
            let (rows, info) = self.comments_by(user_id, &page).await?;
            ok(ListMyCommentsResponse {
                comments: self.in_context(&rows, user_id).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }
}
