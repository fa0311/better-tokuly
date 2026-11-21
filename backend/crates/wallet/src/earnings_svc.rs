//! `EarningsService`: the creator side — what the signed-in user earned and
//! their payout requests.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::wallet::*;

use crate::{Wallet, earnings::resolve_period, payouts::validate_request, wallet_svc::MAX_PAGE};

impl EarningsService for Wallet {
    async fn get_earnings_summary(&self, ctx: RequestContext, request: ServiceRequest<'_, GetEarningsSummaryRequest>) -> ServiceResult<GetEarningsSummaryResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let period = resolve_period(req.from.as_option().map(from_ts), req.to.as_option().map(from_ts), req.bucket.as_known(), now())?;
            let prefs = self.user_prefs(user_id).await?;
            ok(self.earnings_summary(user_id, period, prefs.time_zone, &prefs.locale).await?)
        })
        .await
    }

    async fn request_payout(&self, ctx: RequestContext, request: ServiceRequest<'_, RequestPayoutRequest>) -> ServiceResult<RequestPayoutResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let input = validate_request(request.coins, request.method.as_known(), request.destination, request.note, request.idempotency_key, self.cfg.payout_min_coins)?;
            let payout = self.open_payout(user_id, &input).await?;
            ok(RequestPayoutResponse { payout: payout.to_proto(&self.core).into(), balance: self.balance_snapshot(user_id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn cancel_payout_request(&self, ctx: RequestContext, request: ServiceRequest<'_, CancelPayoutRequestRequest>) -> ServiceResult<CancelPayoutRequestResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let payout = self.cancel_payout(user_id, parse_id(request.payout_id)?).await?;
            ok(CancelPayoutRequestResponse { payout: payout.to_proto(&self.core).into(), balance: self.balance_snapshot(user_id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn list_payout_requests(&self, ctx: RequestContext, request: ServiceRequest<'_, ListPayoutRequestsRequest>) -> ServiceResult<ListPayoutRequestsResponse> {
        handle(async {
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (payouts, page) = self.list_payouts(ctx.user_id()?, &page).await?;
            ok(ListPayoutRequestsResponse { payouts: payouts.iter().map(|payout| payout.to_proto(&self.core)).collect(), page: page.into(), ..Default::default() })
        })
        .await
    }
}
