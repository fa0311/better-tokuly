//! `NoteChannelService`: thin handlers over [`crate::channels`].
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::notes::*;

use crate::{
    Notes,
    channels::{ChannelInput, MAX_LIST_PAGE, MAX_NAME_CHARS},
    timelines::MAX_PAGE,
    visibility::Viewer,
};

impl NoteChannelService for Notes {
    async fn create_channel(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateChannelRequest>,
    ) -> ServiceResult<CreateChannelResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let input =
                ChannelInput::validate(request.name, request.description, request.banner_asset_id, request.color)?;
            let row = self.create_channel(user_id, input).await?;
            let viewer = Viewer::load(&self.core, Some(user_id)).await?;
            ok(CreateChannelResponse { channel: self.channel_view(&viewer, &row).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn update_channel(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateChannelRequest>,
    ) -> ServiceResult<UpdateChannelResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let channel_id = parse_id(request.channel_id)?;
            let input =
                ChannelInput::validate(request.name, request.description, request.banner_asset_id, request.color)?;
            let row = self.update_channel(user_id, channel_id, input).await?;
            ok(UpdateChannelResponse { channel: self.channel_view(&viewer, &row).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn archive_channel(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ArchiveChannelRequest>,
    ) -> ServiceResult<ArchiveChannelResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let row = self.archive_channel(user_id, parse_id(request.channel_id)?, request.archived).await?;
            ok(ArchiveChannelResponse { channel: self.channel_view(&viewer, &row).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn get_channel(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetChannelRequest>,
    ) -> ServiceResult<GetChannelResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let row = self.channel_for(&viewer, parse_id(request.channel_id)?).await?;
            ok(GetChannelResponse { channel: self.channel_view(&viewer, &row).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn list_channels(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListChannelsRequest>,
    ) -> ServiceResult<ListChannelsResponse> {
        handle(async {
            let filter = request.filter.as_known().unwrap_or_default();
            // Followed and managed channels are someone's: sign in for those.
            let viewer = match filter {
                ChannelFilter::CHANNEL_FILTER_FOLLOWED | ChannelFilter::CHANNEL_FILTER_MANAGED => {
                    self.signed_in(&ctx).await?.0
                }
                _ => self.viewer(&ctx).await?,
            };
            ensure!(
                request.query.chars().count() <= MAX_NAME_CHARS,
                invalid,
                "QUERY_TOO_LONG",
                "search for at most {MAX_NAME_CHARS} characters"
            );
            let page = Page::from_view(request.page.as_option(), MAX_LIST_PAGE);
            let (channels, info) = self.list_channels(&viewer, filter, request.query, &page).await?;
            ok(ListChannelsResponse { channels, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn follow_channel(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, FollowChannelRequest>,
    ) -> ServiceResult<FollowChannelResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let row = self.follow_channel(user_id, parse_id(request.channel_id)?).await?;
            ok(FollowChannelResponse { channel: self.channel_view(&viewer, &row).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn unfollow_channel(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnfollowChannelRequest>,
    ) -> ServiceResult<UnfollowChannelResponse> {
        handle(async {
            self.unfollow_channel(ctx.user_id()?, parse_id(request.channel_id)?).await?;
            ok(UnfollowChannelResponse::default())
        })
        .await
    }

    async fn add_channel_moderator(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AddChannelModeratorRequest>,
    ) -> ServiceResult<AddChannelModeratorResponse> {
        handle(async {
            let channel_id = parse_id(request.channel_id)?;
            let target = parse_id(request.user_id)?;
            self.add_channel_moderator(ctx.user_id()?, channel_id, target).await?;
            ok(AddChannelModeratorResponse::default())
        })
        .await
    }

    async fn remove_channel_moderator(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveChannelModeratorRequest>,
    ) -> ServiceResult<RemoveChannelModeratorResponse> {
        handle(async {
            let channel_id = parse_id(request.channel_id)?;
            let target = parse_id(request.user_id)?;
            self.remove_channel_moderator(ctx.user_id()?, channel_id, target).await?;
            ok(RemoveChannelModeratorResponse::default())
        })
        .await
    }

    async fn list_channel_moderators(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListChannelModeratorsRequest>,
    ) -> ServiceResult<ListChannelModeratorsResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let moderators = self.channel_moderators(&viewer, parse_id(request.channel_id)?).await?;
            ok(ListChannelModeratorsResponse { moderators, ..Default::default() })
        })
        .await
    }

    async fn pin_channel_note(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, PinChannelNoteRequest>,
    ) -> ServiceResult<PinChannelNoteResponse> {
        handle(async {
            let channel_id = parse_id(request.channel_id)?;
            let note_id = parse_id(request.note_id)?;
            self.pin_channel_note(ctx.user_id()?, channel_id, note_id).await?;
            ok(PinChannelNoteResponse::default())
        })
        .await
    }

    async fn unpin_channel_note(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnpinChannelNoteRequest>,
    ) -> ServiceResult<UnpinChannelNoteResponse> {
        handle(async {
            let channel_id = parse_id(request.channel_id)?;
            let note_id = parse_id(request.note_id)?;
            self.unpin_channel_note(ctx.user_id()?, channel_id, note_id).await?;
            ok(UnpinChannelNoteResponse::default())
        })
        .await
    }

    async fn remove_channel_note(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveChannelNoteRequest>,
    ) -> ServiceResult<RemoveChannelNoteResponse> {
        handle(async {
            let channel_id = parse_id(request.channel_id)?;
            let note_id = parse_id(request.note_id)?;
            self.remove_channel_note(ctx.user_id()?, channel_id, note_id).await?;
            ok(RemoveChannelNoteResponse::default())
        })
        .await
    }

    async fn get_channel_timeline(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetChannelTimelineRequest>,
    ) -> ServiceResult<GetChannelTimelineResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (entries, info, pinned_notes) =
                self.channel_timeline(&viewer, parse_id(request.channel_id)?, &page).await?;
            ok(GetChannelTimelineResponse { entries, page: info.into(), pinned_notes, ..Default::default() })
        })
        .await
    }
}
