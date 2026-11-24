//! `AntennaService`: thin handlers over [`crate::antennas`]. Every RPC needs
//! a signed-in caller, and only the owner sees an antenna.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::notes::*;

use crate::{Notes, antennas::AntennaSpec, timelines::MAX_PAGE};

/// The antenna of a create or update request, checked.
fn spec(input: Option<&AntennaInput>) -> AppResult<AntennaSpec> {
    AntennaSpec::parse(&input.cloned().unwrap_or_default())
}

impl AntennaService for Notes {
    async fn create_antenna(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateAntennaRequest>,
    ) -> ServiceResult<CreateAntennaResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let request = request.to_owned_message();
            let row = Notes::create_antenna(self, user_id, spec(request.antenna.as_option())?).await?;
            let antenna = self.antennas_to_proto(&viewer, std::slice::from_ref(&row)).await?.pop();
            ok(CreateAntennaResponse { antenna: antenna.into(), ..Default::default() })
        })
        .await
    }

    async fn update_antenna(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateAntennaRequest>,
    ) -> ServiceResult<UpdateAntennaResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let request = request.to_owned_message();
            let antenna_id = parse_id(&request.antenna_id)?;
            let row = Notes::update_antenna(self, user_id, antenna_id, spec(request.antenna.as_option())?).await?;
            let antenna = self.antennas_to_proto(&viewer, std::slice::from_ref(&row)).await?.pop();
            ok(UpdateAntennaResponse { antenna: antenna.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_antenna(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteAntennaRequest>,
    ) -> ServiceResult<DeleteAntennaResponse> {
        handle(async {
            Notes::delete_antenna(self, ctx.user_id()?, parse_id(request.antenna_id)?).await?;
            ok(DeleteAntennaResponse::default())
        })
        .await
    }

    async fn get_antenna(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetAntennaRequest>,
    ) -> ServiceResult<GetAntennaResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let row = self.require_antenna(user_id, parse_id(request.antenna_id)?).await?;
            let antenna = self.antennas_to_proto(&viewer, std::slice::from_ref(&row)).await?.pop();
            ok(GetAntennaResponse { antenna: antenna.into(), ..Default::default() })
        })
        .await
    }

    async fn list_antennas(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListAntennasRequest>,
    ) -> ServiceResult<ListAntennasResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let rows = self.antennas_of(user_id).await?;
            ok(ListAntennasResponse { antennas: self.antennas_to_proto(&viewer, &rows).await?, ..Default::default() })
        })
        .await
    }

    async fn get_antenna_timeline(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetAntennaTimelineRequest>,
    ) -> ServiceResult<GetAntennaTimelineResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (entries, info) = self.antenna_timeline(&viewer, user_id, parse_id(request.antenna_id)?, &page).await?;
            ok(GetAntennaTimelineResponse { entries, page: info.into(), ..Default::default() })
        })
        .await
    }
}
