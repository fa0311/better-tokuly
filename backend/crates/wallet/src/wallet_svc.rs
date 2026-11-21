//! `WalletService`: the signed-in user's coins — balance, history, buying
//! coins and the daily bonus.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::wallet::*;

use crate::{Wallet, entries::Scope};

/// Largest page of any wallet list.
pub(crate) const MAX_PAGE: i64 = 100;

impl WalletService for Wallet {
    async fn get_balance(&self, ctx: RequestContext, _request: ServiceRequest<'_, GetBalanceRequest>) -> ServiceResult<GetBalanceResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            ok(GetBalanceResponse { balance: self.balance_snapshot(user_id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn list_ledger_entries(&self, ctx: RequestContext, request: ServiceRequest<'_, ListLedgerEntriesRequest>) -> ServiceResult<ListLedgerEntriesResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let locale = self.user_prefs(user_id).await?.locale;
            let (entries, page) = self.list_entries(user_id, Scope::from_filter(request.account.as_known()), &page, &locale).await?;
            ok(ListLedgerEntriesResponse { entries, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn list_coin_packages(&self, _ctx: RequestContext, _request: ServiceRequest<'_, ListCoinPackagesRequest>) -> ServiceResult<ListCoinPackagesResponse> {
        handle(async {
            let packages = self.packages_on_sale().await?.iter().map(|package| package.to_proto()).collect();
            let providers = self.payment_providers().into_iter().map(str::to_owned).collect();
            ok(ListCoinPackagesResponse { packages, providers, ..Default::default() })
        })
        .await
    }

    async fn start_purchase(&self, ctx: RequestContext, request: ServiceRequest<'_, StartPurchaseRequest>) -> ServiceResult<StartPurchaseResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let package_id = parse_id(request.package_id)?;
            let purchase = self.open_purchase(user_id, package_id, request.provider, request.idempotency_key).await?;
            ok(StartPurchaseResponse { purchase: purchase.to_proto().into(), balance: self.balance_snapshot(user_id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn confirm_purchase(&self, ctx: RequestContext, request: ServiceRequest<'_, ConfirmPurchaseRequest>) -> ServiceResult<ConfirmPurchaseResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            // Each call may cost a request to the payment provider.
            self.core.cache.rate_limit(&format!("wallet:confirm:{user_id}"), 60, Duration::from_secs(60)).await?;
            let purchase = self.confirm_payment(user_id, parse_id(request.purchase_id)?, request.provider_proof).await?;
            ok(ConfirmPurchaseResponse { purchase: purchase.to_proto().into(), balance: self.balance_snapshot(user_id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn cancel_purchase(&self, ctx: RequestContext, request: ServiceRequest<'_, CancelPurchaseRequest>) -> ServiceResult<CancelPurchaseResponse> {
        handle(async {
            let purchase = self.abandon_purchase(ctx.user_id()?, parse_id(request.purchase_id)?).await?;
            ok(CancelPurchaseResponse { purchase: purchase.to_proto().into(), ..Default::default() })
        })
        .await
    }

    async fn get_purchase(&self, ctx: RequestContext, request: ServiceRequest<'_, GetPurchaseRequest>) -> ServiceResult<GetPurchaseResponse> {
        handle(async {
            let purchase = self.find_purchase(ctx.user_id()?, parse_id(request.purchase_id)?).await?;
            ok(GetPurchaseResponse { purchase: purchase.to_proto().into(), ..Default::default() })
        })
        .await
    }

    async fn list_purchases(&self, ctx: RequestContext, request: ServiceRequest<'_, ListPurchasesRequest>) -> ServiceResult<ListPurchasesResponse> {
        handle(async {
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (purchases, page) = self.purchases_page(ctx.user_id()?, &page).await?;
            ok(ListPurchasesResponse { purchases: purchases.iter().map(|purchase| purchase.to_proto()).collect(), page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn get_daily_bonus(&self, ctx: RequestContext, _request: ServiceRequest<'_, GetDailyBonusRequest>) -> ServiceResult<GetDailyBonusResponse> {
        handle(async { ok(GetDailyBonusResponse { bonus: self.daily_bonus(ctx.user_id()?).await?.into(), ..Default::default() }) }).await
    }

    async fn claim_daily_bonus(&self, ctx: RequestContext, _request: ServiceRequest<'_, ClaimDailyBonusRequest>) -> ServiceResult<ClaimDailyBonusResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.core.cache.rate_limit(&format!("wallet:daily-bonus:{user_id}"), 20, Duration::from_secs(60)).await?;
            let (coins, bonus) = self.claim_bonus(user_id).await?;
            ok(ClaimDailyBonusResponse { coins, bonus: bonus.into(), balance: self.balance_snapshot(user_id).await?.into(), ..Default::default() })
        })
        .await
    }
}
