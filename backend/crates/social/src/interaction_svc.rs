//! `InteractionService`: thin RPC handlers for bumps, buzzes, knocks, map
//! reactions and map views. The behaviour lives in the feature modules.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{prelude::*, time::from_unix};
use tokuly_proto::social::*;

use crate::Social;

impl InteractionService for Social {
    async fn bump(&self, ctx: RequestContext, request: ServiceRequest<'_, BumpRequest>) -> ServiceResult<BumpResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let location = request
                .location
                .as_option()
                .ok_or_else(|| AppError::invalid("INVALID_LOCATION", "a valid location is required"))?;
            let shaken_at = request.shaken_at.as_option().map(|at| from_unix(at.seconds, at.nanos));
            let bumps = Social::bump(
                self,
                user_id,
                location.lat,
                location.lng,
                request.accuracy_meters,
                shaken_at,
                request.intensity,
            )
            .await?;
            ok(BumpResponse { bumps, ..Default::default() })
        })
        .await
    }

    async fn list_bumps(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListBumpsRequest>,
    ) -> ServiceResult<ListBumpsResponse> {
        handle(async {
            let with = parse_opt_id(request.with_user_id)?;
            let page = Self::bumps_page(request.page.as_option());
            let (bumps, page) = Social::list_bumps(self, ctx.user_id()?, with, &page).await?;
            ok(ListBumpsResponse { bumps, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn buzz(&self, ctx: RequestContext, request: ServiceRequest<'_, BuzzRequest>) -> ServiceResult<BuzzResponse> {
        handle(async {
            Social::buzz(self, ctx.user_id()?, parse_id(request.user_id)?).await?;
            ok(BuzzResponse::default())
        })
        .await
    }

    async fn knock(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, KnockRequest>,
    ) -> ServiceResult<KnockResponse> {
        handle(async {
            Social::knock(self, ctx.user_id()?, parse_id(request.user_id)?, request.count).await?;
            ok(KnockResponse::default())
        })
        .await
    }

    async fn send_map_reaction(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SendMapReactionRequest>,
    ) -> ServiceResult<SendMapReactionResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let recipients = parse_ids(&request.user_ids)?;
            let reactions = Social::send_map_reaction(
                self,
                user_id,
                &recipients,
                request.kind.as_known(),
                request.emoji,
                request.stamp_id,
                request.intensity,
            )
            .await?;
            ok(SendMapReactionResponse { reactions, ..Default::default() })
        })
        .await
    }

    async fn list_pending_map_reactions(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListPendingMapReactionsRequest>,
    ) -> ServiceResult<ListPendingMapReactionsResponse> {
        handle(async {
            ok(ListPendingMapReactionsResponse {
                reactions: self.pending_map_reactions(ctx.user_id()?).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn ack_map_reactions(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AckMapReactionsRequest>,
    ) -> ServiceResult<AckMapReactionsResponse> {
        handle(async {
            Social::ack_map_reactions(self, ctx.user_id()?, &parse_ids(&request.reaction_ids)?).await?;
            ok(AckMapReactionsResponse::default())
        })
        .await
    }

    async fn record_map_view(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RecordMapViewRequest>,
    ) -> ServiceResult<RecordMapViewResponse> {
        handle(async {
            Social::record_map_view(self, ctx.user_id()?, parse_id(request.user_id)?).await?;
            ok(RecordMapViewResponse::default())
        })
        .await
    }

    async fn get_map_views(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetMapViewsRequest>,
    ) -> ServiceResult<GetMapViewsResponse> {
        handle(async { ok(self.map_views(ctx.user_id()?).await?) }).await
    }
}
