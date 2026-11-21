//! `StoryService`: thin handlers over [`crate::stories`] and
//! [`crate::highlights`]. Every RPC needs a signed-in caller.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::notes::*;

use crate::{
    Notes,
    stories::{NewStory, requested_audience},
};

/// Largest page of the viewer list and the archive.
const MAX_PAGE: i64 = 100;

impl StoryService for Notes {
    async fn create_story(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateStoryRequest>,
    ) -> ServiceResult<CreateStoryResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let request = request.to_owned_message();
            let items = request
                .items
                .iter()
                .map(|item| Ok((parse_id(&item.asset_id)?, item.caption.clone())))
                .collect::<AppResult<Vec<_>>>()?;
            let input = NewStory {
                title: request.title,
                items,
                audience: requested_audience(request.audience),
                audience_user_ids: parse_ids(&request.audience_user_ids)?,
            };
            let story = self.create_story_once(user_id, &request.idempotency_key, input).await?;
            let story = self.stories_to_proto(&viewer, std::slice::from_ref(&story)).await?.pop();
            ok(CreateStoryResponse { story: story.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_story(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteStoryRequest>,
    ) -> ServiceResult<DeleteStoryResponse> {
        handle(async {
            self.delete_own_story(ctx.user_id()?, parse_id(request.story_id)?).await?;
            ok(DeleteStoryResponse::default())
        })
        .await
    }

    async fn get_story(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetStoryRequest>,
    ) -> ServiceResult<GetStoryResponse> {
        handle(async {
            let (viewer, _) = self.signed_in(&ctx).await?;
            let story = self.require_story(&viewer, parse_id(request.story_id)?).await?;
            let story = self.stories_to_proto(&viewer, std::slice::from_ref(&story)).await?.pop();
            ok(GetStoryResponse { story: story.into(), ..Default::default() })
        })
        .await
    }

    async fn get_story_tray(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetStoryTrayRequest>,
    ) -> ServiceResult<GetStoryTrayResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            ok(GetStoryTrayResponse { reels: self.story_tray(&viewer, user_id).await?, ..Default::default() })
        })
        .await
    }

    async fn list_user_stories(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListUserStoriesRequest>,
    ) -> ServiceResult<ListUserStoriesResponse> {
        handle(async {
            let (viewer, _) = self.signed_in(&ctx).await?;
            ok(ListUserStoriesResponse {
                stories: self.user_stories(&viewer, parse_id(request.user_id)?).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn mark_story_viewed(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, MarkStoryViewedRequest>,
    ) -> ServiceResult<MarkStoryViewedResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            self.mark_story_viewed(&viewer, user_id, parse_id(request.story_id)?).await?;
            ok(MarkStoryViewedResponse::default())
        })
        .await
    }

    async fn list_story_viewers(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListStoryViewersRequest>,
    ) -> ServiceResult<ListStoryViewersResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (viewers, info, story) =
                self.story_viewers(&viewer, user_id, parse_id(request.story_id)?, &page).await?;
            ok(ListStoryViewersResponse {
                viewers,
                page: info.into(),
                view_count: story.view_count,
                reaction_count: story.reaction_count,
                ..Default::default()
            })
        })
        .await
    }

    async fn react_to_story(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReactToStoryRequest>,
    ) -> ServiceResult<ReactToStoryResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            self.react_to_story(&viewer, user_id, parse_id(request.story_id)?, request.emoji).await?;
            ok(ReactToStoryResponse::default())
        })
        .await
    }

    async fn remove_story_reaction(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveStoryReactionRequest>,
    ) -> ServiceResult<RemoveStoryReactionResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            self.remove_story_reaction(&viewer, user_id, parse_id(request.story_id)?).await?;
            ok(RemoveStoryReactionResponse::default())
        })
        .await
    }

    async fn get_story_calendar(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetStoryCalendarRequest>,
    ) -> ServiceResult<GetStoryCalendarResponse> {
        handle(async {
            let days = self.story_calendar(ctx.user_id()?, request.year, request.month).await?;
            ok(GetStoryCalendarResponse { days, ..Default::default() })
        })
        .await
    }

    async fn list_archived_stories(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListArchivedStoriesRequest>,
    ) -> ServiceResult<ListArchivedStoriesResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (stories, info) = self.archived_stories(&viewer, user_id, request.date, &page).await?;
            ok(ListArchivedStoriesResponse { stories, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn create_highlight(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateHighlightRequest>,
    ) -> ServiceResult<CreateHighlightResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let story_ids = request.story_ids.iter().map(|id| parse_id(id)).collect::<AppResult<Vec<_>>>()?;
            let highlight = self.create_highlight(&viewer, user_id, request.title, &story_ids).await?;
            ok(CreateHighlightResponse { highlight: highlight.into(), ..Default::default() })
        })
        .await
    }

    async fn update_highlight(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateHighlightRequest>,
    ) -> ServiceResult<UpdateHighlightResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let story_ids = request.story_ids.iter().map(|id| parse_id(id)).collect::<AppResult<Vec<_>>>()?;
            let highlight = self
                .update_highlight(&viewer, user_id, parse_id(request.highlight_id)?, request.title, &story_ids)
                .await?;
            ok(UpdateHighlightResponse { highlight: highlight.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_highlight(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteHighlightRequest>,
    ) -> ServiceResult<DeleteHighlightResponse> {
        handle(async {
            self.delete_highlight(ctx.user_id()?, parse_id(request.highlight_id)?).await?;
            ok(DeleteHighlightResponse::default())
        })
        .await
    }

    async fn reorder_highlights(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReorderHighlightsRequest>,
    ) -> ServiceResult<ReorderHighlightsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            // An ID that is not even an ID cannot be one of the caller's highlights.
            let ordered =
                request.highlight_ids.iter().map(|id| parse_id(id)).collect::<AppResult<Vec<_>>>().map_err(|_| {
                    AppError::invalid("HIGHLIGHTS_MISMATCH", "name each of your highlights exactly once")
                })?;
            self.reorder_highlights(user_id, &ordered).await?;
            ok(ReorderHighlightsResponse::default())
        })
        .await
    }

    async fn list_highlights(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListHighlightsRequest>,
    ) -> ServiceResult<ListHighlightsResponse> {
        handle(async {
            let (viewer, _) = self.signed_in(&ctx).await?;
            ok(ListHighlightsResponse {
                highlights: self.list_highlights(&viewer, parse_id(request.user_id)?).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn get_highlight(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetHighlightRequest>,
    ) -> ServiceResult<GetHighlightResponse> {
        handle(async {
            let (viewer, _) = self.signed_in(&ctx).await?;
            let (highlight, stories) = self.get_highlight(&viewer, parse_id(request.highlight_id)?).await?;
            ok(GetHighlightResponse { highlight: highlight.into(), stories, ..Default::default() })
        })
        .await
    }
}
