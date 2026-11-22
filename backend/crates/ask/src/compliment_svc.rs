//! `ComplimentService`: thin handlers over `polls`.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::ask::*;

use crate::{Ask, box_svc::MAX_IDS, questions::MAX_PAGE};

impl ComplimentService for Ask {
    async fn get_round(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetRoundRequest>,
    ) -> ServiceResult<GetRoundResponse> {
        handle(async {
            ok(GetRoundResponse { round: self.current_round(ctx.user_id()?).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn vote(&self, ctx: RequestContext, request: ServiceRequest<'_, VoteRequest>) -> ServiceResult<VoteResponse> {
        handle(async {
            let round = self.cast_vote(ctx.user_id()?, parse_id(request.poll_id)?, parse_id(request.user_id)?).await?;
            ok(VoteResponse { round: round.into(), ..Default::default() })
        })
        .await
    }

    async fn skip_poll(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SkipPollRequest>,
    ) -> ServiceResult<SkipPollResponse> {
        handle(async {
            let round = self.pass_poll(ctx.user_id()?, parse_id(request.poll_id)?).await?;
            ok(SkipPollResponse { round: round.into(), ..Default::default() })
        })
        .await
    }

    async fn shuffle_poll(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ShufflePollRequest>,
    ) -> ServiceResult<ShufflePollResponse> {
        handle(async {
            let round = self.reshuffle_poll(ctx.user_id()?, parse_id(request.poll_id)?).await?;
            ok(ShufflePollResponse { round: round.into(), ..Default::default() })
        })
        .await
    }

    async fn list_flames(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListFlamesRequest>,
    ) -> ServiceResult<ListFlamesResponse> {
        handle(async {
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (rows, info) = self.flames_page(ctx.user_id()?, &page).await?;
            ok(ListFlamesResponse { flames: self.render_flames(&rows).await?, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn get_flame_summary(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetFlameSummaryRequest>,
    ) -> ServiceResult<GetFlameSummaryResponse> {
        handle(async {
            ok(GetFlameSummaryResponse {
                summary: self.flame_summary(ctx.user_id()?).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn mark_flames_seen(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, MarkFlamesSeenRequest>,
    ) -> ServiceResult<MarkFlamesSeenResponse> {
        handle(async {
            let receiver = ctx.user_id()?;
            ensure!(request.flame_ids.len() <= MAX_IDS, invalid, "TOO_MANY_IDS", "at most {MAX_IDS} flames per call");
            let ids = request.flame_ids.iter().map(|id| parse_id(id)).collect::<AppResult<Vec<Uuid>>>()?;
            ok(MarkFlamesSeenResponse {
                unseen: self.mark_seen(receiver, &ids, request.all).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn buy_hint(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BuyHintRequest>,
    ) -> ServiceResult<BuyHintResponse> {
        handle(async {
            let (flame, hint, coins_spent) =
                self.purchase_hint(ctx.user_id()?, parse_id(request.flame_id)?, request.idempotency_key).await?;
            ok(BuyHintResponse { flame: flame.into(), hint: hint.into(), coins_spent, ..Default::default() })
        })
        .await
    }

    async fn get_top_flames(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetTopFlamesRequest>,
    ) -> ServiceResult<GetTopFlamesResponse> {
        handle(async {
            let (entries, week_start, my_count) = self.top_flames(ctx.user_id()?).await?;
            ok(GetTopFlamesResponse { entries, week_start: ts(week_start), my_count, ..Default::default() })
        })
        .await
    }
}
