//! `ClipService`: short cuts of streams and videos.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::live::{list_clips_request::Filter as RequestFilter, *};
use tokuly_video::{channels, videos};

use crate::{
    Live,
    access::Viewer,
    clips::{self, Filter, NewSource},
    search,
    streams::{self, Aftermath},
};

fn video_not_found() -> AppError {
    AppError::not_found("VIDEO_NOT_FOUND", "video not found")
}

fn source_not_ready() -> AppError {
    AppError::precondition("CLIP_SOURCE_NOT_READY", "this stream cannot be clipped right now")
}

impl ClipService for Live {
    async fn create_clip(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateClipRequest>,
    ) -> ServiceResult<CreateClipResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let (stream_ref, video_ref) = (request.stream_id.trim(), request.video_id.trim());
            let source = match (stream_ref.is_empty(), video_ref.is_empty()) {
                (false, true) => {
                    let stream = streams::find_watchable(core, &viewer, stream_ref).await?;
                    if stream.is_live() {
                        NewSource::Stream(stream)
                    } else {
                        // A stream that is over is clipped from its archive.
                        let video = match stream.archive_video_id {
                            Some(video_id) => videos::find_by_id(core, video_id).await?,
                            None => None,
                        };
                        let video =
                            video.filter(|video| viewer.can_watch(&video.facts())).ok_or_else(source_not_ready)?;
                        NewSource::Video { video, stream_id: Some(stream.id) }
                    }
                }
                (true, false) => {
                    let video =
                        videos::find_ref(core, video_ref).await?.filter(|video| viewer.can_watch(&video.facts()));
                    let video = video.ok_or_else(video_not_found)?;
                    // An archive's clips are also listed under its stream.
                    let stream_id = match video.origin_id {
                        Some(stream_id) => streams::find_by_id(core, stream_id).await?.map(|stream| stream.id),
                        None => None,
                    };
                    NewSource::Video { video, stream_id }
                }
                _ => bail!(invalid, "CLIP_SOURCE_REQUIRED", "name the stream or the video to cut from, not both"),
            };
            let clip = self
                .start_clip(user_id, source, request.title, request.start_seconds, request.duration_seconds)
                .await?;
            ok(CreateClipResponse {
                clip: Some(clips::to_proto(core, &viewer, &clip).await?).into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_clip(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetClipRequest>,
    ) -> ServiceResult<GetClipResponse> {
        handle(async {
            let core = &self.core;
            let viewer = Viewer::load(core, ctx.maybe_user_id()?).await?;
            let clip = clips::find_ref(core, request.clip_id).await?.ok_or_else(clips::not_found)?;
            ok(GetClipResponse {
                clip: Some(clips::to_proto(core, &viewer, &clip).await?).into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_clip(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateClipRequest>,
    ) -> ServiceResult<UpdateClipResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let clip = clips::find_ref(core, request.clip_id)
                .await?
                .filter(|clip| clip.creator_id == user_id)
                .ok_or_else(clips::not_found)?;
            let title = clips::validate_title(request.title)?;
            core.cache.rate_limit(&format!("live-clip-edit:{user_id}"), 60, Duration::from_secs(60)).await?;
            let viewer = Viewer { user_id: Some(user_id), ..Default::default() };
            let renamed = clips::ClipRow { title, ..clip };
            // Read before the transaction, which then needs no second connection.
            let public =
                clips::source_of(core, &Viewer::anonymous(), &renamed).await?.is_some_and(|source| source.public);
            let mut tx = core.db.begin().await?;
            sqlx::query("UPDATE live_clips SET title = $2 WHERE id = $1")
                .bind(renamed.id)
                .bind(&renamed.title)
                .execute(&mut *tx)
                .await?;
            search::sync_clip(&mut tx, &renamed, public).await?;
            tx.commit().await?;
            ok(UpdateClipResponse {
                clip: Some(clips::to_proto(core, &viewer, &renamed).await?).into(),
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
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let clip = clips::find_ref(core, request.clip_id).await?;
            // Its creator, or the owner of the channel it was cut from.
            let clip = clip
                .filter(|clip| clip.creator_id == user_id || clip.owner_id == user_id)
                .ok_or_else(clips::not_found)?;
            let mut tx = core.db.begin().await?;
            let removal = clips::delete_in(&mut tx, &clip, Some(user_id), false).await?;
            tx.commit().await?;
            self.clean_up(Aftermath { assets: removal.into_iter().collect(), ..Default::default() }).await;
            ok(DeleteClipResponse::default())
        })
        .await
    }

    async fn list_clips(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListClipsRequest>,
    ) -> ServiceResult<ListClipsResponse> {
        handle(async {
            let core = &self.core;
            let viewer = Viewer::load(core, ctx.maybe_user_id()?).await?;
            let req = request.to_owned_message();
            let filter = match &req.filter {
                Some(RequestFilter::StreamId(reference)) => {
                    Filter::Stream(streams::find_watchable(core, &viewer, reference).await?.id)
                }
                Some(RequestFilter::VideoId(reference)) => {
                    let video =
                        videos::find_ref(core, reference).await?.filter(|video| viewer.can_watch(&video.facts()));
                    Filter::Video(video.ok_or_else(video_not_found)?.id)
                }
                Some(RequestFilter::ChannelId(reference)) => {
                    Filter::Channel(channels::find_visible(core, &viewer, reference).await?.id)
                }
                Some(RequestFilter::CreatorId(user_id)) => Filter::Creator(parse_id(user_id)?),
                None => bail!(
                    invalid,
                    "CLIP_FILTER_REQUIRED",
                    "say which clips to list: of a stream, a video, a channel or a creator"
                ),
            };
            let page = Page::from_request(req.page.as_option(), 100);
            let before: Option<Uuid> = page.decode()?;
            let mut rows = clips::list(core, filter, before, page.fetch()).await?;
            let info = page.finish(&mut rows, |row| row.id);
            // What the caller may not see (or what is not ready yet) is
            // dropped after paging, so a page can come back short.
            ok(ListClipsResponse {
                clips: clips::to_protos(core, &viewer, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }
}
