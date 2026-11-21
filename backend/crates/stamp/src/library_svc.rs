//! `StampLibraryService`: the signed-in user's own stamps.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{analytics::AnalyticsEvent, prelude::*};
use tokuly_proto::stamp::*;

use crate::{
    Stamp,
    access::{InstallCheck, Viewer},
    collections, installs,
    model::stamps_to_proto,
    personal, purchases, stamps, validate,
};

/// Sizes of the lists GetLibrary returns.
const LIBRARY_INSTALLED: i32 = 100;
const LIBRARY_RECENTS: usize = 40;
const LIBRARY_FAVORITES: i32 = 60;
const LIBRARY_PERSONAL: i32 = 60;

const SEARCH_DEFAULT: i32 = 30;
const SEARCH_MAX: i32 = 60;

/// The first page of a list with a fixed size.
fn first_page(size: i32) -> Page {
    Page::new(size, "", size as i64, size as i64)
}

impl StampLibraryService for Stamp {
    async fn get_library(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetLibraryRequest>,
    ) -> ServiceResult<GetLibraryResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            let core = &self.core;
            let (installed_page, favorites_page, personal_page) =
                (first_page(LIBRARY_INSTALLED), first_page(LIBRARY_FAVORITES), first_page(LIBRARY_PERSONAL));
            let (installed, recents, favorites, personal) = tokio::try_join!(
                installs::list(core, user_id, &installed_page),
                collections::recents(core, &viewer, user_id, RecentOrder::RECENT_ORDER_LAST_USED, LIBRARY_RECENTS),
                collections::favorites(core, &viewer, user_id, &favorites_page),
                personal::list(core, user_id, &personal_page),
            )?;
            ok(GetLibraryResponse {
                installed: self.summaries(&viewer, &installed.0).await?,
                recents: stamps_to_proto(&recents, &viewer),
                favorites: stamps_to_proto(&favorites.0, &viewer),
                personal: stamps_to_proto(&personal.0, &viewer),
                more_installed: installed.1.has_more,
                more_favorites: favorites.1.has_more,
                more_personal: personal.1.has_more,
                ..Default::default()
            })
        })
        .await
    }

    async fn list_installed_packs(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListInstalledPacksRequest>,
    ) -> ServiceResult<ListInstalledPacksResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            // A picker shows every tab at once, so pages are large by default.
            let page = match request.page.as_option() {
                Some(page) => Page::new(page.limit, page.cursor, 100, 200),
                None => Page::new(0, "", 100, 200),
            };
            let (rows, page) = installs::list(&self.core, user_id, &page).await?;
            ok(ListInstalledPacksResponse {
                packs: self.summaries(&viewer, &rows).await?,
                page: page.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn install_pack(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, InstallPackRequest>,
    ) -> ServiceResult<InstallPackResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            let (pack, holding) = self.visible_pack(&viewer, request.pack_id).await?;
            match viewer.can_install(&pack.facts(), holding) {
                InstallCheck::Allowed => {}
                InstallCheck::PurchaseRequired => {
                    bail!(precondition, "PURCHASE_REQUIRED", "this pack costs {} coins; buy it first", pack.price)
                }
                InstallCheck::Unavailable => bail!(not_found, "PACK_NOT_FOUND", "stamp pack not found"),
            }
            let mut tx = self.core.db.begin().await?;
            installs::ensure_room(&mut *tx, user_id, pack.id).await?;
            let installed = installs::install(&mut tx, user_id, &pack, !viewer.manages(&pack.facts())).await?;
            tx.commit().await?;
            if installed {
                self.core
                    .analytics
                    .track(AnalyticsEvent::new("stamp.pack.install").user(Some(user_id)).subject("stamp_pack", pack.id))
                    .await;
            }
            // Reload: the install changed the pack's counters and the viewer's state.
            let (pack, _) = self.visible_pack(&viewer, &pack.id.to_string()).await?;
            ok(InstallPackResponse { pack: self.summary(&viewer, &pack).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn uninstall_pack(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UninstallPackRequest>,
    ) -> ServiceResult<UninstallPackResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            // Uninstalling must work for packs the user can no longer see
            // (taken down), so the pack is looked up without a visibility check.
            let Some(pack) = crate::packs::find(&self.core, request.pack_id).await? else {
                return ok(UninstallPackResponse::default());
            };
            let mut tx = self.core.db.begin().await?;
            installs::uninstall(&mut tx, user_id, &pack, !viewer.manages(&pack.facts())).await?;
            tx.commit().await?;
            ok(UninstallPackResponse::default())
        })
        .await
    }

    async fn reorder_installed_packs(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReorderInstalledPacksRequest>,
    ) -> ServiceResult<ReorderInstalledPacksResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            ensure!(
                request.pack_ids.len() <= installs::MAX_INSTALLED as usize,
                invalid,
                "ORDER_MISMATCH",
                "too many packs listed"
            );
            let pack_ids = request.pack_ids.iter().map(|id| parse_id(id)).collect::<AppResult<Vec<Uuid>>>()?;
            installs::reorder(&self.core, user_id, &pack_ids).await?;
            ok(ReorderInstalledPacksResponse::default())
        })
        .await
    }

    async fn purchase_pack(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, PurchasePackRequest>,
    ) -> ServiceResult<PurchasePackResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            let (pack, holding) = self.visible_pack(&viewer, request.pack_id).await?;
            let charged = self.purchase(&viewer, user_id, &pack, holding, request.expected_price).await?;
            let (pack, _) = self.visible_pack(&viewer, &pack.id.to_string()).await?;
            ok(PurchasePackResponse { pack: self.summary(&viewer, &pack).await?.into(), charged, ..Default::default() })
        })
        .await
    }

    async fn list_purchased_packs(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListPurchasedPacksRequest>,
    ) -> ServiceResult<ListPurchasedPacksResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            let page = Page::from_view(request.page.as_option(), 100);
            let (rows, page) = purchases::list(&self.core, user_id, &page).await?;
            ok(ListPurchasedPacksResponse {
                packs: self.summaries(&viewer, &rows).await?,
                page: page.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_favorite_stamps(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListFavoriteStampsRequest>,
    ) -> ServiceResult<ListFavoriteStampsResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            let page = Page::from_view(request.page.as_option(), 100);
            let (views, page) = collections::favorites(&self.core, &viewer, user_id, &page).await?;
            ok(ListFavoriteStampsResponse {
                stamps: stamps_to_proto(&views, &viewer),
                page: page.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn favorite_stamp(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, FavoriteStampRequest>,
    ) -> ServiceResult<FavoriteStampResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            collections::favorite(&self.core, &viewer, user_id, parse_id(request.stamp_id)?).await?;
            ok(FavoriteStampResponse::default())
        })
        .await
    }

    async fn unfavorite_stamp(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnfavoriteStampRequest>,
    ) -> ServiceResult<UnfavoriteStampResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            collections::unfavorite(&self.core, user_id, parse_id(request.stamp_id)?).await?;
            ok(UnfavoriteStampResponse::default())
        })
        .await
    }

    async fn list_recent_stamps(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListRecentStampsRequest>,
    ) -> ServiceResult<ListRecentStampsResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            let limit = if request.limit <= 0 {
                LIBRARY_RECENTS
            } else {
                request.limit.min(crate::usage::RECENTS_KEPT as i32) as usize
            };
            let order = request.order.as_known().unwrap_or_default();
            let views = collections::recents(&self.core, &viewer, user_id, order, limit).await?;
            ok(ListRecentStampsResponse { stamps: stamps_to_proto(&views, &viewer), ..Default::default() })
        })
        .await
    }

    async fn clear_recent_stamps(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ClearRecentStampsRequest>,
    ) -> ServiceResult<ClearRecentStampsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            collections::clear_recents(&self.core.db, user_id, parse_opt_id(request.stamp_id)?).await?;
            ok(ClearRecentStampsResponse::default())
        })
        .await
    }

    async fn save_stamp(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SaveStampRequest>,
    ) -> ServiceResult<SaveStampResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            self.core.cache.rate_limit(&format!("stamp-save:{user_id}"), 60, Duration::from_secs(60)).await?;
            let saved = self.save_received_stamp(&viewer, user_id, parse_id(request.stamp_id)?).await?;
            let stamp = stamps::view(&self.core, Some(user_id), saved.stamp_id).await?.ok_or_else(stamps::not_found)?;
            let pack = match &saved.pack {
                Some(pack) => Some(self.summary(&viewer, pack).await?),
                None => None,
            };
            ok(SaveStampResponse {
                outcome: saved.outcome.into(),
                stamp: stamp.to_proto(&viewer).into(),
                pack: pack.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn create_personal_stamp(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreatePersonalStampRequest>,
    ) -> ServiceResult<CreatePersonalStampResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            let keywords: Vec<&str> = request.keywords.iter().copied().collect();
            let text = validate::stamp_text(request.name, request.emoji, &keywords)?;
            self.core.cache.rate_limit(&format!("stamp-personal:{user_id}"), 60, Duration::from_secs(3600)).await?;
            let source = stamps::source_from_upload(&self.core, request.asset_id, user_id).await?;
            let stamp_id = personal::create(&self.core, user_id, &source, &text, self.cfg.max_personal_stamps).await?;
            let view = personal::owned(&self.core, user_id, stamp_id).await?;
            ok(CreatePersonalStampResponse { stamp: view.to_proto(&viewer).into(), ..Default::default() })
        })
        .await
    }

    async fn list_personal_stamps(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListPersonalStampsRequest>,
    ) -> ServiceResult<ListPersonalStampsResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            let page = Page::from_view(request.page.as_option(), 100);
            let (views, page) = personal::list(&self.core, user_id, &page).await?;
            ok(ListPersonalStampsResponse {
                stamps: stamps_to_proto(&views, &viewer),
                page: page.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_personal_stamp(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdatePersonalStampRequest>,
    ) -> ServiceResult<UpdatePersonalStampResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            let req = request.to_owned_message();
            let stamp_id = parse_id(&req.stamp_id)?;
            let name = req.name.as_deref().map(validate::stamp_name).transpose()?;
            let emoji = req.emoji.as_deref().map(validate::emoji).transpose()?;
            let keywords = req.keywords.as_option().map(|list| validate::keywords(&list.keywords)).transpose()?;
            personal::owned(&self.core, user_id, stamp_id).await?;
            stamps::update_text(&self.core.db, stamp_id, name, emoji, keywords).await?;
            let view = personal::owned(&self.core, user_id, stamp_id).await?;
            ok(UpdatePersonalStampResponse { stamp: view.to_proto(&viewer).into(), ..Default::default() })
        })
        .await
    }

    async fn delete_personal_stamp(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeletePersonalStampRequest>,
    ) -> ServiceResult<DeletePersonalStampResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            personal::delete(&self.core, user_id, parse_id(request.stamp_id)?).await?;
            ok(DeletePersonalStampResponse::default())
        })
        .await
    }

    async fn reorder_personal_stamps(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReorderPersonalStampsRequest>,
    ) -> ServiceResult<ReorderPersonalStampsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            ensure!(
                request.stamp_ids.len() as i64 <= self.cfg.max_personal_stamps,
                invalid,
                "ORDER_MISMATCH",
                "too many stamps listed"
            );
            let stamp_ids = request.stamp_ids.iter().map(|id| parse_id(id)).collect::<AppResult<Vec<Uuid>>>()?;
            personal::reorder(&self.core, user_id, &stamp_ids).await?;
            ok(ReorderPersonalStampsResponse::default())
        })
        .await
    }

    async fn search_library_stamps(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SearchLibraryStampsRequest>,
    ) -> ServiceResult<SearchLibraryStampsResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            // Called on every keystroke of a picker's search box.
            self.core
                .cache
                .rate_limit(&format!("stamp-library-search:{user_id}"), 300, Duration::from_secs(60))
                .await?;
            let Some(query) = validate::search_query(request.query)? else {
                return ok(SearchLibraryStampsResponse::default());
            };
            let limit = if request.limit <= 0 { SEARCH_DEFAULT } else { request.limit.min(SEARCH_MAX) };
            let views = collections::search(&self.core, &viewer, user_id, &query, limit as usize).await?;
            ok(SearchLibraryStampsResponse { stamps: stamps_to_proto(&views, &viewer), ..Default::default() })
        })
        .await
    }

    async fn list_usage_log(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListUsageLogRequest>,
    ) -> ServiceResult<ListUsageLogResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            let page = Page::from_view(request.page.as_option(), 100);
            let (entries, page) = collections::usage_log(&self.core, &viewer, user_id, &page).await?;
            ok(ListUsageLogResponse { entries, page: page.into(), ..Default::default() })
        })
        .await
    }
}
