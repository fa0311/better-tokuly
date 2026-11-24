//! `ClipService`: thin handlers over [`crate::clips`].
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::notes::*;

use crate::{
    Notes,
    clips::{ClipInput, MAX_PAGE},
};

impl ClipService for Notes {
    async fn create_clip(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateClipRequest>,
    ) -> ServiceResult<CreateClipResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let input = ClipInput::validate(request.name, request.description, request.public)?;
            ok(CreateClipResponse { clip: self.create_clip(user_id, input).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn update_clip(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateClipRequest>,
    ) -> ServiceResult<UpdateClipResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let clip_id = parse_id(request.clip_id)?;
            let input = ClipInput::validate(request.name, request.description, request.public)?;
            ok(UpdateClipResponse {
                clip: self.update_clip(user_id, clip_id, input).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_clip(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteClipRequest>,
    ) -> ServiceResult<DeleteClipResponse> {
        handle(async {
            self.delete_clip(ctx.user_id()?, parse_id(request.clip_id)?).await?;
            ok(DeleteClipResponse::default())
        })
        .await
    }

    async fn get_clip(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetClipRequest>,
    ) -> ServiceResult<GetClipResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let row = self.clip_for(&viewer, parse_id(request.clip_id)?).await?;
            let clip = self.clip_views(std::slice::from_ref(&row), &Default::default()).await?.pop();
            ok(GetClipResponse { clip: clip.into(), ..Default::default() })
        })
        .await
    }

    async fn list_clips(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListClipsRequest>,
    ) -> ServiceResult<ListClipsResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            // Empty = the caller's own clips.
            let owner = match parse_opt_id(request.user_id)? {
                Some(owner) => owner,
                None => ctx.user_id()?,
            };
            let note_id = parse_opt_id(request.note_id)?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (clips, info) = self.list_clips(&viewer, owner, note_id, &page).await?;
            ok(ListClipsResponse { clips, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn add_note_to_clip(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AddNoteToClipRequest>,
    ) -> ServiceResult<AddNoteToClipResponse> {
        handle(async {
            let clip_id = parse_id(request.clip_id)?;
            let note_id = parse_id(request.note_id)?;
            let clip = self.add_note_to_clip(ctx.user_id()?, clip_id, note_id).await?;
            ok(AddNoteToClipResponse { clip: clip.into(), ..Default::default() })
        })
        .await
    }

    async fn remove_note_from_clip(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveNoteFromClipRequest>,
    ) -> ServiceResult<RemoveNoteFromClipResponse> {
        handle(async {
            let clip_id = parse_id(request.clip_id)?;
            let note_id = parse_id(request.note_id)?;
            let clip = self.remove_note_from_clip(ctx.user_id()?, clip_id, note_id).await?;
            ok(RemoveNoteFromClipResponse { clip: clip.into(), ..Default::default() })
        })
        .await
    }

    async fn get_clip_notes(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetClipNotesRequest>,
    ) -> ServiceResult<GetClipNotesResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (notes, info) = self.clip_notes(&viewer, parse_id(request.clip_id)?, &page).await?;
            ok(GetClipNotesResponse { notes, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn list_note_clips(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListNoteClipsRequest>,
    ) -> ServiceResult<ListNoteClipsResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (clips, info) = self.note_clips(&viewer, parse_id(request.note_id)?, &page).await?;
            ok(ListNoteClipsResponse { clips, page: info.into(), ..Default::default() })
        })
        .await
    }
}
