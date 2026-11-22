//! `LivePollService`: polls in a stream.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::live::*;

use crate::{
    Live,
    access::{self, Viewer},
    polls::{self, PollRow},
    streams::{self, StreamRow},
};

fn poll_not_found() -> AppError {
    AppError::not_found("POLL_NOT_FOUND", "poll not found")
}

impl Live {
    /// A poll with its stream, if `viewer` may see the stream.
    async fn visible_poll(&self, viewer: &Viewer, poll_id: &str) -> AppResult<(PollRow, StreamRow)> {
        let Ok(poll_id) = Uuid::parse_str(poll_id.trim()) else { return Err(poll_not_found()) };
        let poll = polls::find(&self.core, poll_id).await?.ok_or_else(poll_not_found)?;
        let stream = streams::find_by_id(&self.core, poll.stream_id).await?.ok_or_else(poll_not_found)?;
        ensure!(access::can_watch(viewer, &stream.facts()), not_found, "POLL_NOT_FOUND", "poll not found");
        Ok((poll, stream))
    }
}

impl LivePollService for Live {
    async fn create_poll(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreatePollRequest>,
    ) -> ServiceResult<CreatePollResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let stream = streams::find_watchable(core, &viewer, &req.stream_id).await?;
            access::require_moderator(core, stream.channel_id, stream.owner_id, user_id).await?;
            core.cache.rate_limit(&format!("live-poll:{user_id}"), 30, Duration::from_secs(3600)).await?;
            let poll = self.open_poll(&stream, user_id, &req.question, &req.options, req.duration_seconds).await?;
            ok(CreatePollResponse { poll: Some(poll.to_proto(0)).into(), ..Default::default() })
        })
        .await
    }

    async fn vote_poll(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, VotePollRequest>,
    ) -> ServiceResult<VotePollResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let (poll, stream) = self.visible_poll(&viewer, request.poll_id).await?;
            core.cache.rate_limit(&format!("live-vote:{user_id}"), 60, Duration::from_secs(60)).await?;
            let updated = self.cast_vote(&stream, &poll, user_id, request.option).await?;
            ok(VotePollResponse { poll: Some(updated.to_proto(request.option)).into(), ..Default::default() })
        })
        .await
    }

    async fn end_poll(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, EndPollRequest>,
    ) -> ServiceResult<EndPollResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let (poll, stream) = self.visible_poll(&viewer, request.poll_id).await?;
            access::require_moderator(core, stream.channel_id, stream.owner_id, user_id).await?;
            core.cache.rate_limit(&format!("live-moderate:{user_id}"), 120, Duration::from_secs(60)).await?;
            let closed = self.close_poll(&stream, &poll).await?;
            ok(EndPollResponse {
                poll: Some(polls::to_proto_for(core, &closed, Some(user_id)).await?).into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_polls(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListPollsRequest>,
    ) -> ServiceResult<ListPollsResponse> {
        handle(async {
            let core = &self.core;
            let viewer = Viewer::load(core, ctx.maybe_user_id()?).await?;
            let stream = streams::find_watchable(core, &viewer, request.stream_id).await?;
            let page = Page::from_view(request.page.as_option(), 100);
            let before: Option<Uuid> = page.decode()?;
            let mut rows = polls::list(core, stream.id, before, page.fetch()).await?;
            let info = page.finish(&mut rows, |row| row.id);
            let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
            let choices = polls::choices_of(core, &ids, viewer.user_id).await?;
            ok(ListPollsResponse {
                polls: rows.iter().map(|row| row.to_proto(choices.get(&row.id).copied().unwrap_or(0))).collect(),
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }
}
