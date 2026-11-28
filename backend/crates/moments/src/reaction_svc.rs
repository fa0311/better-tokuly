//! `MomentReactionService`: thin handlers over `reactions`.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::moments::*;

use crate::Moments;

impl MomentReactionService for Moments {
    async fn react(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReactRequest>,
    ) -> ServiceResult<ReactResponse> {
        handle(async {
            let reaction = Moments::react(self, ctx.user_id()?, request.to_owned_message()).await?;
            ok(ReactResponse { reaction: reaction.into(), ..Default::default() })
        })
        .await
    }

    async fn remove_reaction(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveReactionRequest>,
    ) -> ServiceResult<RemoveReactionResponse> {
        handle(async {
            Moments::remove_reaction(self, ctx.user_id()?, parse_id(request.moment_id)?).await?;
            ok(RemoveReactionResponse::default())
        })
        .await
    }

    async fn list_reactions(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListReactionsRequest>,
    ) -> ServiceResult<ListReactionsResponse> {
        handle(async {
            let page = Page::from_view(request.page.as_option(), 100);
            let (reactions, info) =
                Moments::list_reactions(self, ctx.user_id()?, parse_id(request.moment_id)?, page).await?;
            ok(ListReactionsResponse { reactions, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn list_realmojis(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListRealmojisRequest>,
    ) -> ServiceResult<ListRealmojisResponse> {
        handle(async {
            ok(ListRealmojisResponse { realmojis: self.realmojis(ctx.user_id()?).await?, ..Default::default() })
        })
        .await
    }

    async fn set_realmoji(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetRealmojiRequest>,
    ) -> ServiceResult<SetRealmojiResponse> {
        handle(async {
            let realmoji =
                Moments::set_realmoji(self, ctx.user_id()?, request.emoji.as_known(), request.asset_id).await?;
            ok(SetRealmojiResponse { realmoji: realmoji.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_realmoji(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteRealmojiRequest>,
    ) -> ServiceResult<DeleteRealmojiResponse> {
        handle(async {
            Moments::delete_realmoji(self, ctx.user_id()?, request.emoji.as_known()).await?;
            ok(DeleteRealmojiResponse::default())
        })
        .await
    }
}
