//! `LiveGiftService`: gifts (super chat).
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::live::*;
use tokuly_video::channels;

use crate::{Live, access::Viewer, chat, gifts, streams, tiers};

impl LiveGiftService for Live {
    async fn get_gift_tiers(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, GetGiftTiersRequest>,
    ) -> ServiceResult<GetGiftTiersResponse> {
        handle(async {
            ok(GetGiftTiersResponse {
                tiers: tiers::to_proto(),
                min_coins: tiers::MIN_GIFT_COINS,
                max_coins: tiers::MAX_GIFT_COINS,
                platform_fee_percent: self.wallet.platform_fee_percent() as i32,
                ..Default::default()
            })
        })
        .await
    }

    async fn send_gift(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SendGiftRequest>,
    ) -> ServiceResult<SendGiftResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let stream = streams::find_watchable(core, &viewer, request.stream_id).await?;
            let (gift, line) =
                self.give_gift(&stream, user_id, request.coins, request.message, request.idempotency_key).await?;
            let balance = tokuly_wallet::ledger::balance(core, user_id).await?;
            ok(SendGiftResponse {
                gift: gifts::to_protos(core, std::slice::from_ref(&gift), false).await?.pop().into(),
                message: match &line {
                    Some(line) => Some(chat::to_proto_one(core, &stream, line).await?),
                    None => None,
                }
                .into(),
                balance: balance.coins,
                ..Default::default()
            })
        })
        .await
    }

    async fn list_sent_gifts(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListSentGiftsRequest>,
    ) -> ServiceResult<ListSentGiftsResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            // A gift whose payment was interrupted is finished before the history is shown.
            self.settle_pending_gifts(Some(user_id)).await;
            let page = Page::from_view(request.page.as_option(), 100);
            let before: Option<Uuid> = page.decode()?;
            let mut rows = gifts::list_sent(core, user_id, before, page.fetch()).await?;
            let info = page.finish(&mut rows, |row| row.id);
            ok(ListSentGiftsResponse {
                gifts: gifts::to_protos(core, &rows, false).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_received_gifts(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListReceivedGiftsRequest>,
    ) -> ServiceResult<ListReceivedGiftsResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let channel = channels::require_owned_ref(core, user_id, request.channel_id).await?;
            let stream_id = if request.stream_id.trim().is_empty() {
                None
            } else {
                let stream = streams::find_owned(core, user_id, request.stream_id).await?;
                ensure!(stream.channel_id == channel.id, not_found, "STREAM_NOT_FOUND", "stream not found");
                Some(stream.id)
            };
            let page = Page::from_view(request.page.as_option(), 100);
            let before: Option<Uuid> = page.decode()?;
            let mut rows = gifts::list_received(core, channel.id, stream_id, before, page.fetch()).await?;
            let info = page.finish(&mut rows, |row| row.id);
            let (total_coins, total_net_coins) = gifts::received_totals(core, channel.id, stream_id).await?;
            ok(ListReceivedGiftsResponse {
                gifts: gifts::to_protos(core, &rows, true).await?,
                page: info.into(),
                total_coins,
                total_net_coins,
                ..Default::default()
            })
        })
        .await
    }

    async fn list_top_supporters(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListTopSupportersRequest>,
    ) -> ServiceResult<ListTopSupportersResponse> {
        handle(async {
            let core = &self.core;
            let viewer = Viewer::load(core, ctx.maybe_user_id()?).await?;
            let stream = streams::find_watchable(core, &viewer, request.stream_id).await?;
            let limit = if request.limit <= 0 { 10 } else { request.limit.min(50) };
            let mut supporters = gifts::top_supporters(core, stream.id, i64::from(limit)).await?;
            // People in a block relation with the caller are not shown to them.
            supporters.retain(|supporter| {
                supporter
                    .user
                    .as_option()
                    .and_then(|user| Uuid::parse_str(&user.id).ok())
                    .is_none_or(|id| !viewer.hides(id))
            });
            ok(ListTopSupportersResponse { supporters, ..Default::default() })
        })
        .await
    }
}
