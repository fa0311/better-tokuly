//! `WalletAdminService`: payout review, wallet lookups for support, manual
//! adjustments and the coin shop.
//!
//! Roles: payout review and lookups need `moderator`; adjustments and shop
//! management need `admin` (admins hold every role). The check is the first
//! line of every handler.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::wallet::*;

use crate::{
    Wallet,
    adjustments::validate_adjustment,
    entries::Scope,
    packages::validate_package,
    payouts::{Decision, status_filter},
    wallet_svc::MAX_PAGE,
};

const MODERATOR: &str = "moderator";
const ADMIN: &str = "admin";

impl Wallet {
    /// One payout request as staff sees it.
    async fn staff_payout(&self, payout: crate::payouts::PayoutRow) -> AppResult<PayoutRequest> {
        let mut payouts = self.payouts_for_staff(std::slice::from_ref(&payout)).await?;
        Ok(payouts.pop().unwrap_or_default())
    }
}

impl WalletAdminService for Wallet {
    async fn list_payout_queue(&self, ctx: RequestContext, request: ServiceRequest<'_, ListPayoutQueueRequest>) -> ServiceResult<ListPayoutQueueResponse> {
        handle(async {
            ctx.require_role(MODERATOR)?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (rows, page) = self.payout_queue_page(status_filter(request.status.as_known()), &page).await?;
            let pending_count: i64 = sqlx::query_scalar("SELECT count(*) FROM wallet_payout_requests WHERE status = 'pending'").fetch_one(&self.core.db).await?;
            ok(ListPayoutQueueResponse {
                payouts: self.payouts_for_staff(&rows).await?,
                page: page.into(),
                pending_count: i32::try_from(pending_count).unwrap_or(i32::MAX),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_payout_request(&self, ctx: RequestContext, request: ServiceRequest<'_, GetPayoutRequestRequest>) -> ServiceResult<GetPayoutRequestResponse> {
        handle(async {
            ctx.require_role(MODERATOR)?;
            let payout = self.find_payout(parse_id(request.payout_id)?).await?;
            ok(GetPayoutRequestResponse { payout: self.staff_payout(payout).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn approve_payout(&self, ctx: RequestContext, request: ServiceRequest<'_, ApprovePayoutRequest>) -> ServiceResult<ApprovePayoutResponse> {
        handle(async {
            let reviewer_id = ctx.require_role(MODERATOR)?.user_id;
            let payout = self.review_payout(reviewer_id, parse_id(request.payout_id)?, Decision::Approve, request.review_note).await?;
            ok(ApprovePayoutResponse { payout: self.staff_payout(payout).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn reject_payout(&self, ctx: RequestContext, request: ServiceRequest<'_, RejectPayoutRequest>) -> ServiceResult<RejectPayoutResponse> {
        handle(async {
            let reviewer_id = ctx.require_role(MODERATOR)?.user_id;
            let payout = self.review_payout(reviewer_id, parse_id(request.payout_id)?, Decision::Reject, request.review_note).await?;
            ok(RejectPayoutResponse { payout: self.staff_payout(payout).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn mark_payout_paid(&self, ctx: RequestContext, request: ServiceRequest<'_, MarkPayoutPaidRequest>) -> ServiceResult<MarkPayoutPaidResponse> {
        handle(async {
            let staff_id = ctx.require_role(MODERATOR)?.user_id;
            let payout = self.record_payout_paid(staff_id, parse_id(request.payout_id)?, request.payment_reference).await?;
            ok(MarkPayoutPaidResponse { payout: self.staff_payout(payout).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn get_user_wallet(&self, ctx: RequestContext, request: ServiceRequest<'_, GetUserWalletRequest>) -> ServiceResult<GetUserWalletResponse> {
        handle(async {
            ctx.require_role(MODERATOR)?;
            let user_id = parse_id(request.user_id)?;
            let user = tokuly_identity::summaries(&self.core, &[user_id]).await?.remove(&user_id).or_not_found("USER_NOT_FOUND", "user not found")?;
            let (pending_payout, lifetime_paid_out) = self.payout_totals(user_id).await?;
            let lifetime_purchased: i64 = sqlx::query_scalar("SELECT COALESCE(sum(coins + bonus_coins), 0)::bigint FROM wallet_purchases WHERE user_id = $1 AND status = 'completed'")
                .bind(user_id)
                .fetch_one(&self.core.db)
                .await?;
            ok(GetUserWalletResponse {
                user: user.into(),
                balance: self.balance_snapshot(user_id).await?.into(),
                pending_payout,
                lifetime_earned: self.lifetime_earned(user_id).await?,
                lifetime_paid_out,
                lifetime_purchased,
                ..Default::default()
            })
        })
        .await
    }

    async fn list_user_ledger_entries(&self, ctx: RequestContext, request: ServiceRequest<'_, ListUserLedgerEntriesRequest>) -> ServiceResult<ListUserLedgerEntriesResponse> {
        handle(async {
            let staff_id = ctx.require_role(MODERATOR)?.user_id;
            let user_id = parse_id(request.user_id)?;
            let known: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)").bind(user_id).fetch_one(&self.core.db).await?;
            ensure!(known, not_found, "USER_NOT_FOUND", "user not found");
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            // Worded in the language of the person reading it, not the account owner's.
            let locale = self.user_prefs(staff_id).await?.locale;
            // Staff trace payments: they are shown both parties, also a counterpart the owner is not told
            // (unless it was kept from the staff member themselves).
            let (entries, page) = self.list_entries_for_staff(staff_id, user_id, Scope::from_filter(request.account.as_known()), &page, &locale).await?;
            ok(ListUserLedgerEntriesResponse { entries, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn adjust_balance(&self, ctx: RequestContext, request: ServiceRequest<'_, AdjustBalanceRequest>) -> ServiceResult<AdjustBalanceResponse> {
        handle(async {
            let admin_id = ctx.require_role(ADMIN)?.user_id;
            let input = validate_adjustment(request.user_id, request.account.as_known(), request.delta, request.reason, request.idempotency_key)?;
            let adjustment = self.apply_adjustment(admin_id, &input).await?;
            let mut adjustments = self.adjustments_to_proto(std::slice::from_ref(&adjustment)).await?;
            ok(AdjustBalanceResponse {
                adjustment: adjustments.pop().unwrap_or_default().into(),
                balance: self.balance_snapshot(input.user_id).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_adjustments(&self, ctx: RequestContext, request: ServiceRequest<'_, ListAdjustmentsRequest>) -> ServiceResult<ListAdjustmentsResponse> {
        handle(async {
            ctx.require_role(ADMIN)?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (rows, page) = self.adjustments_page(parse_opt_id(request.user_id)?, &page).await?;
            ok(ListAdjustmentsResponse { adjustments: self.adjustments_to_proto(&rows).await?, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn list_all_coin_packages(&self, ctx: RequestContext, _request: ServiceRequest<'_, ListAllCoinPackagesRequest>) -> ServiceResult<ListAllCoinPackagesResponse> {
        handle(async {
            ctx.require_role(ADMIN)?;
            let packages = self.all_packages().await?.iter().map(|package| package.to_proto()).collect();
            ok(ListAllCoinPackagesResponse { packages, ..Default::default() })
        })
        .await
    }

    async fn save_coin_package(&self, ctx: RequestContext, request: ServiceRequest<'_, SaveCoinPackageRequest>) -> ServiceResult<SaveCoinPackageResponse> {
        handle(async {
            let admin_id = ctx.require_role(ADMIN)?.user_id;
            let request = request.to_owned_message();
            // A request without a package is validated as an empty one, which fails on its SKU.
            let package = request.package.as_option().cloned().unwrap_or_default();
            let input = validate_package(&package)?;
            let saved = self.save_package(parse_opt_id(&package.id)?, &input).await?;
            tracing::info!(package_id = %saved.id, sku = %saved.sku, %admin_id, "coin package saved");
            ok(SaveCoinPackageResponse { package: saved.to_proto().into(), ..Default::default() })
        })
        .await
    }
}
