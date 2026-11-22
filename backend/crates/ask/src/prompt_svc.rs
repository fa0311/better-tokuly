//! `PromptService`: thin handlers over `prompts`.
use std::collections::HashMap;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::{
    ask::*,
    common::{PageInfo, UserSummary},
};
use tokuly_social::graph;

use crate::{
    Ask,
    prompts::{Filter, PromptRow, ResponseRow},
    questions::MAX_PAGE,
};

/// Published responses returned with a prompt's page before paging starts.
const FIRST_RESPONSES: i32 = 20;

impl Ask {
    /// Prompts as `viewer` sees them, with their owners' names.
    async fn render_prompts(&self, rows: &[PromptRow], viewer: Uuid) -> AppResult<Vec<Prompt>> {
        let mut owner_ids: Vec<Uuid> = rows.iter().map(|row| row.owner_id).collect();
        owner_ids.sort_unstable();
        owner_ids.dedup();
        let owners = tokuly_identity::summaries(&self.core, &owner_ids).await?;
        // Every list of other people's prompts is a list of friends' prompts.
        Ok(rows
            .iter()
            .map(|row| {
                row.to_proto(Some(viewer), tokuly_identity::users::pick(&owners, row.owner_id), row.owner_id != viewer)
            })
            .collect())
    }

    pub(crate) async fn render_prompt(
        &self,
        row: &PromptRow,
        viewer: Uuid,
        viewer_is_friend: bool,
    ) -> AppResult<Prompt> {
        let owner = tokuly_identity::summary(&self.core, row.owner_id).await?;
        Ok(row.to_proto(Some(viewer), owner, viewer_is_friend))
    }

    /// Responses as `viewer` sees them. Only signed responders are looked
    /// up, and a responder in a block with the viewer stays unnamed.
    async fn render_responses(&self, rows: &[ResponseRow], viewer: Uuid) -> AppResult<Vec<PromptResponse>> {
        let mut signed_ids: Vec<Uuid> =
            rows.iter().filter(|row| row.signed).filter_map(|row| row.responder_id).collect();
        signed_ids.sort_unstable();
        signed_ids.dedup();
        let names: HashMap<Uuid, UserSummary> = tokuly_identity::summaries(&self.core, &signed_ids).await?;
        let hidden = graph::blocked_ids_either(&self.core, viewer).await?;
        Ok(rows
            .iter()
            .map(|row| {
                let responder =
                    row.responder_id.filter(|id| !hidden.contains(id)).and_then(|id| names.get(&id).cloned());
                row.to_proto(Some(viewer), responder)
            })
            .collect())
    }

    pub(crate) async fn render_response(&self, row: &ResponseRow, viewer: Uuid) -> AppResult<PromptResponse> {
        let mut rendered = self.render_responses(std::slice::from_ref(row), viewer).await?;
        rendered.pop().ok_or_else(|| AppError::internal(anyhow::anyhow!("a rendered response went missing")))
    }

    async fn published_responses(
        &self,
        prompt_id: Uuid,
        viewer: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<PromptResponse>, PageInfo)> {
        let (rows, info) = self.published_page(prompt_id, viewer, page).await?;
        Ok((self.render_responses(&rows, viewer).await?, info))
    }
}

impl PromptService for Ask {
    async fn create_prompt(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreatePromptRequest>,
    ) -> ServiceResult<CreatePromptResponse> {
        handle(async {
            let owner = ctx.user_id()?;
            let prompt = self.open_prompt(owner, request.text, request.emoji, request.duration_seconds).await?;
            ok(CreatePromptResponse {
                prompt: self.render_prompt(&prompt, owner, false).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn close_prompt(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ClosePromptRequest>,
    ) -> ServiceResult<ClosePromptResponse> {
        handle(async {
            let owner = ctx.user_id()?;
            let prompt = self.end_prompt(owner, parse_id(request.prompt_id)?).await?;
            ok(ClosePromptResponse {
                prompt: self.render_prompt(&prompt, owner, false).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_prompt(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeletePromptRequest>,
    ) -> ServiceResult<DeletePromptResponse> {
        handle(async {
            self.remove_prompt(ctx.user_id()?, parse_id(request.prompt_id)?).await?;
            ok(DeletePromptResponse::default())
        })
        .await
    }

    async fn get_prompt(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetPromptRequest>,
    ) -> ServiceResult<GetPromptResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let prompt_id = parse_id(request.prompt_id)?;
            let (prompt, is_friend) = self.visible_prompt(viewer, prompt_id).await?;
            let first_page = Page::new(FIRST_RESPONSES, "", FIRST_RESPONSES as i64, MAX_PAGE);
            let (responses, page) = self.published_responses(prompt_id, viewer, &first_page).await?;
            let my_response = match self.own_response(viewer, prompt_id).await? {
                Some(mine) => Some(self.render_response(&mine, viewer).await?),
                None => None,
            };
            ok(GetPromptResponse {
                prompt: self.render_prompt(&prompt, viewer, is_friend).await?.into(),
                responses,
                page: page.into(),
                my_response: my_response.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_prompts(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListPromptsRequest>,
    ) -> ServiceResult<ListPromptsResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let owner = if request.user.trim().is_empty() {
                viewer
            } else {
                self.visible_user(request.user, Some(viewer))
                    .await?
                    .or_not_found("USER_NOT_FOUND", "user not found")?
                    .id
            };
            // Prompts are for friends: anyone else gets an empty list, the
            // same as for a friend who never posted one.
            if owner != viewer && !graph::are_friends(&self.core, owner, viewer).await? {
                return ok(ListPromptsResponse { page: PageInfo::default().into(), ..Default::default() });
            }
            let (rows, info) = self.prompts_page(owner, viewer, &page).await?;
            ok(ListPromptsResponse {
                prompts: self.render_prompts(&rows, viewer).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_friend_prompts(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListFriendPromptsRequest>,
    ) -> ServiceResult<ListFriendPromptsResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (rows, info) = self.friend_prompts_page(viewer, &page).await?;
            ok(ListFriendPromptsResponse {
                prompts: self.render_prompts(&rows, viewer).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn respond_to_prompt(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RespondToPromptRequest>,
    ) -> ServiceResult<RespondToPromptResponse> {
        handle(async {
            let sender = ctx.user_id()?;
            let response = self.respond(sender, parse_id(request.prompt_id)?, request.text, request.signed).await?;
            ok(RespondToPromptResponse {
                response: self.render_response(&response, sender).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_prompt_responses(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListPromptResponsesRequest>,
    ) -> ServiceResult<ListPromptResponsesResponse> {
        handle(async {
            let owner = ctx.user_id()?;
            let filter = Filter::of(request.filter)?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (rows, info) = self.responses_page(owner, parse_id(request.prompt_id)?, filter, &page).await?;
            ok(ListPromptResponsesResponse {
                responses: self.render_responses(&rows, owner).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_published_responses(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListPublishedResponsesRequest>,
    ) -> ServiceResult<ListPublishedResponsesResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let prompt_id = parse_id(request.prompt_id)?;
            self.visible_prompt(viewer, prompt_id).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (responses, info) = self.published_responses(prompt_id, viewer, &page).await?;
            ok(ListPublishedResponsesResponse { responses, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn set_response_published(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetResponsePublishedRequest>,
    ) -> ServiceResult<SetResponsePublishedResponse> {
        handle(async {
            let owner = ctx.user_id()?;
            let response = self.set_published(owner, parse_id(request.response_id)?, request.published).await?;
            ok(SetResponsePublishedResponse {
                response: self.render_response(&response, owner).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn release_response(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReleaseResponseRequest>,
    ) -> ServiceResult<ReleaseResponseResponse> {
        handle(async {
            let owner = ctx.user_id()?;
            let response = self.unhold_response(owner, parse_id(request.response_id)?).await?;
            ok(ReleaseResponseResponse {
                response: self.render_response(&response, owner).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_response(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteResponseRequest>,
    ) -> ServiceResult<DeleteResponseResponse> {
        handle(async {
            self.remove_response(ctx.user_id()?, parse_id(request.response_id)?).await?;
            ok(DeleteResponseResponse::default())
        })
        .await
    }
}
