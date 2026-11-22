//! `PlanService`: thin RPC handlers. Each one authenticates the caller,
//! parses the request and delegates to the module that owns the behaviour
//! and its authorization (`plan`, `participants`, `checklist`, `expenses`).
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::plans::*;

use crate::{Plans, checklist::ItemChange};

/// Largest page of any list in this service.
const MAX_PAGE: i64 = 100;

impl PlanService for Plans {
    async fn create_plan(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreatePlanRequest>,
    ) -> ServiceResult<CreatePlanResponse> {
        handle(async {
            let plan = Plans::create_plan(self, ctx.user_id()?, &request.to_owned_message()).await?;
            ok(CreatePlanResponse { plan: plan.into(), ..Default::default() })
        })
        .await
    }

    async fn get_plan(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetPlanRequest>,
    ) -> ServiceResult<GetPlanResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let plan = Plans::get_plan(self, parse_id(request.plan_id)?, viewer).await?;
            ok(GetPlanResponse { plan: plan.into(), ..Default::default() })
        })
        .await
    }

    async fn update_plan(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdatePlanRequest>,
    ) -> ServiceResult<UpdatePlanResponse> {
        handle(async {
            let plan = Plans::update_plan(self, ctx.user_id()?, &request.to_owned_message()).await?;
            ok(UpdatePlanResponse { plan: plan.into(), ..Default::default() })
        })
        .await
    }

    async fn cancel_plan(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CancelPlanRequest>,
    ) -> ServiceResult<CancelPlanResponse> {
        handle(async {
            let plan = Plans::cancel_plan(self, ctx.user_id()?, parse_id(request.plan_id)?, request.reason).await?;
            ok(CancelPlanResponse { plan: plan.into(), ..Default::default() })
        })
        .await
    }

    async fn list_plans(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListPlansRequest>,
    ) -> ServiceResult<ListPlansResponse> {
        handle(async {
            let filter = request.filter.as_known().unwrap_or(PlanListFilter::PLAN_LIST_FILTER_UNSPECIFIED);
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (plans, page) =
                Plans::list_plans(self, ctx.user_id()?, filter, request.include_declined, &page).await?;
            ok(ListPlansResponse { plans, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn invite_to_plan(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, InviteToPlanRequest>,
    ) -> ServiceResult<InviteToPlanResponse> {
        handle(async {
            let req = request.to_owned_message();
            let (plan, invited_count) =
                Plans::invite_to_plan(self, ctx.user_id()?, parse_id(&req.plan_id)?, &req.user_ids).await?;
            ok(InviteToPlanResponse { plan: plan.into(), invited_count, ..Default::default() })
        })
        .await
    }

    async fn remove_plan_participant(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemovePlanParticipantRequest>,
    ) -> ServiceResult<RemovePlanParticipantResponse> {
        handle(async {
            let plan = Plans::remove_plan_participant(
                self,
                ctx.user_id()?,
                parse_id(request.plan_id)?,
                parse_id(request.user_id)?,
            )
            .await?;
            ok(RemovePlanParticipantResponse { plan: plan.into(), ..Default::default() })
        })
        .await
    }

    async fn leave_plan(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, LeavePlanRequest>,
    ) -> ServiceResult<LeavePlanResponse> {
        handle(async {
            Plans::leave_plan(self, ctx.user_id()?, parse_id(request.plan_id)?).await?;
            ok(LeavePlanResponse::default())
        })
        .await
    }

    async fn set_plan_role(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetPlanRoleRequest>,
    ) -> ServiceResult<SetPlanRoleResponse> {
        handle(async {
            let role = request.role.as_known().unwrap_or(PlanRole::PLAN_ROLE_UNSPECIFIED);
            let plan = Plans::set_plan_role(
                self,
                ctx.user_id()?,
                parse_id(request.plan_id)?,
                parse_id(request.user_id)?,
                role,
            )
            .await?;
            ok(SetPlanRoleResponse { plan: plan.into(), ..Default::default() })
        })
        .await
    }

    async fn respond_to_plan(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RespondToPlanRequest>,
    ) -> ServiceResult<RespondToPlanResponse> {
        handle(async {
            let wanted = request.rsvp.as_known().unwrap_or(Rsvp::RSVP_UNSPECIFIED);
            let (plan, rsvp) =
                Plans::respond_to_plan(self, ctx.user_id()?, parse_id(request.plan_id)?, wanted, request.comment)
                    .await?;
            ok(RespondToPlanResponse { plan: plan.into(), rsvp: rsvp.into(), ..Default::default() })
        })
        .await
    }

    async fn add_checklist_item(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AddChecklistItemRequest>,
    ) -> ServiceResult<AddChecklistItemResponse> {
        handle(async {
            let kind = request.kind.as_known().unwrap_or(ChecklistKind::CHECKLIST_KIND_UNSPECIFIED);
            let item =
                Plans::add_checklist_item(self, ctx.user_id()?, parse_id(request.plan_id)?, kind, request.text).await?;
            ok(AddChecklistItemResponse { item: item.into(), ..Default::default() })
        })
        .await
    }

    async fn update_checklist_item(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateChecklistItemRequest>,
    ) -> ServiceResult<UpdateChecklistItemResponse> {
        handle(async {
            let change = ItemChange {
                text: request.text,
                kind: request.kind.and_then(|kind| kind.as_known()),
                claimed: request.claimed,
                done: request.done,
            };
            let item = Plans::update_checklist_item(self, ctx.user_id()?, parse_id(request.item_id)?, change).await?;
            ok(UpdateChecklistItemResponse { item: item.into(), ..Default::default() })
        })
        .await
    }

    async fn remove_checklist_item(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveChecklistItemRequest>,
    ) -> ServiceResult<RemoveChecklistItemResponse> {
        handle(async {
            Plans::remove_checklist_item(self, ctx.user_id()?, parse_id(request.item_id)?).await?;
            ok(RemoveChecklistItemResponse::default())
        })
        .await
    }

    async fn add_expense(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AddExpenseRequest>,
    ) -> ServiceResult<AddExpenseResponse> {
        handle(async {
            let req = request.to_owned_message();
            let input = req.expense.as_option().cloned().unwrap_or_default();
            let expense = Plans::add_expense(self, ctx.user_id()?, parse_id(&req.plan_id)?, &input).await?;
            ok(AddExpenseResponse { expense: expense.into(), ..Default::default() })
        })
        .await
    }

    async fn update_expense(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateExpenseRequest>,
    ) -> ServiceResult<UpdateExpenseResponse> {
        handle(async {
            let req = request.to_owned_message();
            let input = req.expense.as_option().cloned().unwrap_or_default();
            let expense = Plans::update_expense(self, ctx.user_id()?, parse_id(&req.expense_id)?, &input).await?;
            ok(UpdateExpenseResponse { expense: expense.into(), ..Default::default() })
        })
        .await
    }

    async fn remove_expense(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveExpenseRequest>,
    ) -> ServiceResult<RemoveExpenseResponse> {
        handle(async {
            Plans::remove_expense(self, ctx.user_id()?, parse_id(request.expense_id)?).await?;
            ok(RemoveExpenseResponse::default())
        })
        .await
    }

    async fn list_expenses(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListExpensesRequest>,
    ) -> ServiceResult<ListExpensesResponse> {
        handle(async {
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (expenses, page) =
                Plans::list_expenses(self, ctx.user_id()?, parse_id(request.plan_id)?, &page).await?;
            ok(ListExpensesResponse { expenses, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn get_split(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetSplitRequest>,
    ) -> ServiceResult<GetSplitResponse> {
        handle(async { ok(Plans::get_split(self, ctx.user_id()?, parse_id(request.plan_id)?).await?) }).await
    }

    async fn settle_transfer(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SettleTransferRequest>,
    ) -> ServiceResult<SettleTransferResponse> {
        handle(async {
            let settlement = Plans::settle_transfer(self, ctx.user_id()?, &request.to_owned_message()).await?;
            ok(SettleTransferResponse { settlement: settlement.into(), ..Default::default() })
        })
        .await
    }

    async fn remove_settlement(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveSettlementRequest>,
    ) -> ServiceResult<RemoveSettlementResponse> {
        handle(async {
            Plans::remove_settlement(self, ctx.user_id()?, parse_id(request.settlement_id)?).await?;
            ok(RemoveSettlementResponse::default())
        })
        .await
    }
}
