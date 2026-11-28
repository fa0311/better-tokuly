//! `MomentAdminService`: thin handlers over `staff`.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::moments::*;

use crate::Moments;

const ROLE: &str = "admin";

impl MomentAdminService for Moments {
    async fn list_prompt_regions(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListPromptRegionsRequest>,
    ) -> ServiceResult<ListPromptRegionsResponse> {
        handle(async {
            ctx.require_role(ROLE)?;
            ok(ListPromptRegionsResponse { regions: self.prompt_regions().await?, ..Default::default() })
        })
        .await
    }

    async fn send_prompt_now(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SendPromptNowRequest>,
    ) -> ServiceResult<SendPromptNowResponse> {
        handle(async {
            let admin = ctx.require_role(ROLE)?.user_id;
            let region = self.send_now(admin, request.region).await?;
            ok(SendPromptNowResponse { region: region.into(), ..Default::default() })
        })
        .await
    }
}
