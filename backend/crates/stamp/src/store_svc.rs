//! `StampStoreService`: browsing the store. Works without signing in.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::stamp::*;

use crate::{
    Stamp,
    access::Viewer,
    packs, personal, stamps,
    store::{self, Shelf},
    validate,
};

/// Packs per shelf on the store's landing page.
const HOME_SHELF_SIZE: i32 = 12;
const HOME_TAG_COUNT: i64 = 20;
const MAX_PAGE: i64 = 50;

impl Stamp {
    /// The first packs of a shelf as cards for `viewer`.
    async fn home_shelf(&self, viewer: &Viewer, shelf: Shelf) -> AppResult<Vec<PackSummary>> {
        let page = Page::new(HOME_SHELF_SIZE, "", HOME_SHELF_SIZE as i64, HOME_SHELF_SIZE as i64);
        let (rows, _) = store::shelf(&self.core, shelf, None, PriceFilter::PRICE_FILTER_UNSPECIFIED, &page).await?;
        self.summaries(viewer, &rows).await
    }
}

impl StampStoreService for Stamp {
    async fn get_store_home(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetStoreHomeRequest>,
    ) -> ServiceResult<GetStoreHomeResponse> {
        handle(async {
            let viewer = Viewer::of(&ctx)?;
            let (featured, popular, newest, tags) = tokio::try_join!(
                self.home_shelf(&viewer, Shelf::Featured),
                self.home_shelf(&viewer, Shelf::Popular),
                self.home_shelf(&viewer, Shelf::New),
                store::popular_tags(&self.core, HOME_TAG_COUNT),
            )?;
            ok(GetStoreHomeResponse { featured, popular, newest, tags, ..Default::default() })
        })
        .await
    }

    async fn list_store_packs(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListStorePacksRequest>,
    ) -> ServiceResult<ListStorePacksResponse> {
        handle(async {
            let viewer = Viewer::of(&ctx)?;
            let tag = match request.tag.trim() {
                "" => None,
                tag => Some(
                    validate::normalize_word(tag)
                        .ok_or_else(|| AppError::invalid("TAGS_INVALID", "not a valid tag"))?,
                ),
            };
            let shelf = Shelf::from_proto(request.section.as_known().unwrap_or_default());
            let price = request.price.as_known().unwrap_or_default();
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (rows, page) = store::shelf(&self.core, shelf, tag.as_deref(), price, &page).await?;
            ok(ListStorePacksResponse {
                packs: self.summaries(&viewer, &rows).await?,
                page: page.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn search_packs(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SearchPacksRequest>,
    ) -> ServiceResult<SearchPacksResponse> {
        handle(async {
            let viewer = Viewer::of(&ctx)?;
            let limiter = viewer.user_id.map(|id| id.to_string()).unwrap_or_else(|| ctx.client().ip);
            self.core.cache.rate_limit(&format!("stamp-search:{limiter}"), 120, Duration::from_secs(60)).await?;
            let Some(query) = validate::search_query(request.query)? else {
                return ok(SearchPacksResponse::default());
            };
            let price = request.price.as_known().unwrap_or_default();
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (rows, page) = store::search(&self.core, &query, price, &page).await?;
            ok(SearchPacksResponse {
                packs: self.summaries(&viewer, &rows).await?,
                page: page.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_popular_tags(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListPopularTagsRequest>,
    ) -> ServiceResult<ListPopularTagsResponse> {
        handle(async {
            let limit = if request.limit <= 0 { 30 } else { request.limit.min(100) };
            ok(ListPopularTagsResponse {
                tags: store::popular_tags(&self.core, limit as i64).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn list_creator_packs(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListCreatorPacksRequest>,
    ) -> ServiceResult<ListCreatorPacksResponse> {
        handle(async {
            let viewer = Viewer::of(&ctx)?;
            let creator_id = parse_id(request.user_id)?;
            let creator = tokuly_identity::users::find_by_id(&self.core, creator_id)
                .await?
                .filter(|user| user.is_active())
                .or_not_found("USER_NOT_FOUND", "user not found")?;
            // A creator who blocked the viewer does not show them a shop window either.
            ensure!(
                self.identity.can_view(viewer.user_id, creator_id).await?,
                not_found,
                "USER_NOT_FOUND",
                "user not found"
            );
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (rows, page) = store::by_creator(&self.core, creator_id, &page).await?;
            ok(ListCreatorPacksResponse {
                creator: creator.summary().into(),
                packs: self.summaries(&viewer, &rows).await?,
                page: page.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_pack(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetPackRequest>,
    ) -> ServiceResult<GetPackResponse> {
        handle(async {
            let viewer = Viewer::of(&ctx)?;
            let (pack, _) = self.visible_pack(&viewer, request.pack_id).await?;
            ok(GetPackResponse { pack: self.detail(&viewer, &pack).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn get_stamp(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetStampRequest>,
    ) -> ServiceResult<GetStampResponse> {
        handle(async {
            let viewer = Viewer::of(&ctx)?;
            let view = stamps::view(&self.core, viewer.user_id, parse_id(request.stamp_id)?)
                .await?
                .filter(|view| view.visible_to(&viewer))
                .ok_or_else(stamps::not_found)?;
            // The pack card is shown only when the pack page itself would open.
            let pack = match view.pack_id {
                Some(pack_id) => packs::find_by_id(&self.core.db, pack_id)
                    .await?
                    .filter(|pack| viewer.can_view(&pack.facts(), view.holding())),
                None => None,
            };
            let pack = match &pack {
                Some(pack) => Some(self.summary(&viewer, pack).await?),
                None => None,
            };
            let can_save = personal::would_save(&self.core, &viewer, &view).await?;
            ok(GetStampResponse {
                stamp: view.to_proto(&viewer).into(),
                pack: pack.into(),
                can_save,
                ..Default::default()
            })
        })
        .await
    }
}
