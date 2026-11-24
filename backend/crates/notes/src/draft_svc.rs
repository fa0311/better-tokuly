//! `NoteDraftService` and `NoteMuteService`: thin handlers over
//! [`crate::drafts`] and [`crate::mutes`]. Every RPC needs a signed-in caller.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::notes::*;

use crate::{Notes, timelines::MAX_PAGE};

impl NoteDraftService for Notes {
    async fn save_draft(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SaveDraftRequest>,
    ) -> ServiceResult<SaveDraftResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let request = request.to_owned_message();
            let scheduled_at = request.scheduled_at.as_option().map(from_ts);
            let content = request.content.as_option().cloned().unwrap_or_default();
            let row = self.save_draft(user_id, parse_opt_id(&request.draft_id)?, content, scheduled_at).await?;
            let draft = self.drafts_to_proto(std::slice::from_ref(&row)).await?.pop();
            ok(SaveDraftResponse { draft: draft.into(), ..Default::default() })
        })
        .await
    }

    async fn get_draft(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetDraftRequest>,
    ) -> ServiceResult<GetDraftResponse> {
        handle(async {
            let row = self.draft(ctx.user_id()?, parse_id(request.draft_id)?).await?;
            let draft = self.drafts_to_proto(std::slice::from_ref(&row)).await?.pop();
            ok(GetDraftResponse { draft: draft.into(), ..Default::default() })
        })
        .await
    }

    async fn list_drafts(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListDraftsRequest>,
    ) -> ServiceResult<ListDraftsResponse> {
        handle(async {
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (rows, info) = self.list_drafts(ctx.user_id()?, request.scheduled_only, &page).await?;
            ok(ListDraftsResponse {
                drafts: self.drafts_to_proto(&rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_draft(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteDraftRequest>,
    ) -> ServiceResult<DeleteDraftResponse> {
        handle(async {
            self.delete_draft(ctx.user_id()?, parse_id(request.draft_id)?).await?;
            ok(DeleteDraftResponse::default())
        })
        .await
    }

    async fn publish_draft(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, PublishDraftRequest>,
    ) -> ServiceResult<PublishDraftResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let note = self.publish_draft(user_id, parse_id(request.draft_id)?).await?;
            ok(PublishDraftResponse { note: self.hydrate_one(&viewer, &note).await?.into(), ..Default::default() })
        })
        .await
    }
}

impl NoteMuteService for Notes {
    async fn get_mute_settings(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetMuteSettingsRequest>,
    ) -> ServiceResult<GetMuteSettingsResponse> {
        handle(async {
            let settings = self.mute_settings(ctx.user_id()?).await?;
            ok(GetMuteSettingsResponse {
                words: settings.words,
                renotes_hidden_from: settings.renotes_hidden_from,
                muted_thread_ids: settings.muted_thread_ids.iter().map(Uuid::to_string).collect(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_muted_words(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateMutedWordsRequest>,
    ) -> ServiceResult<UpdateMutedWordsResponse> {
        handle(async {
            let request = request.to_owned_message();
            let words = self.update_muted_words(ctx.user_id()?, &request.words).await?;
            ok(UpdateMutedWordsResponse { words, ..Default::default() })
        })
        .await
    }

    async fn mute_thread(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, MuteThreadRequest>,
    ) -> ServiceResult<MuteThreadResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            self.mute_thread(&viewer, user_id, parse_id(request.note_id)?).await?;
            ok(MuteThreadResponse::default())
        })
        .await
    }

    async fn unmute_thread(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnmuteThreadRequest>,
    ) -> ServiceResult<UnmuteThreadResponse> {
        handle(async {
            self.unmute_thread(ctx.user_id()?, parse_id(request.note_id)?).await?;
            ok(UnmuteThreadResponse::default())
        })
        .await
    }

    async fn hide_renotes_from(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, HideRenotesFromRequest>,
    ) -> ServiceResult<HideRenotesFromResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            self.hide_renotes_from(&viewer, user_id, parse_id(request.user_id)?).await?;
            ok(HideRenotesFromResponse::default())
        })
        .await
    }

    async fn show_renotes_from(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ShowRenotesFromRequest>,
    ) -> ServiceResult<ShowRenotesFromResponse> {
        handle(async {
            self.show_renotes_from(ctx.user_id()?, parse_id(request.user_id)?).await?;
            ok(ShowRenotesFromResponse::default())
        })
        .await
    }
}
