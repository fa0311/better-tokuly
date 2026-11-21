//! `StampStudioService`: the creator studio.
//!
//! Every change to a pack follows the same steps: check the caller manages
//! the pack, validate the input, then — in one transaction — lock the pack,
//! check the editing rules against its current status, apply the change and
//! refresh what is derived from it (cover, counts, search document).
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use sqlx::{Postgres, Transaction};
use tokuly_core::{events, ids::new_short_id, prelude::*};
use tokuly_proto::stamp::*;

use crate::{
    Stamp,
    access::{self, Viewer},
    creators,
    events::{PACK_DELETED, PackEvent},
    installs,
    lifecycle::{self, Approval},
    model::{PackRow, pack_columns, status},
    packs,
    stamps::{self, StampHome},
    stats,
    validate::{self, MAX_PRICE, MAX_STAMPS_PER_PACK, MAX_STAMPS_PER_REQUEST, MIN_PAID_PRICE},
};

/// How long an unfinished purchase protects a pack from deletion.
const PURCHASE_IN_FLIGHT: &str = "15 minutes";

/// Whether somebody bought the pack, or is in the middle of buying it.
async fn has_buyers(tx: &mut Transaction<'_, Postgres>, pack_id: Uuid) -> AppResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM stamp_pack_purchases WHERE pack_id = $1)
             OR EXISTS (SELECT 1 FROM stamp_purchase_intents WHERE pack_id = $1 AND created_at > now() - $2::interval)",
    )
    .bind(pack_id)
    .bind(PURCHASE_IN_FLIGHT)
    .fetch_one(&mut **tx)
    .await?)
}

/// Whether stamps of this pack go live without review: official packs and
/// packs of trusted creators.
async fn skips_review(tx: &mut Transaction<'_, Postgres>, pack: &PackRow) -> AppResult<bool> {
    match pack.creator_id {
        Some(creator_id) => creators::is_trusted(&mut **tx, creator_id).await,
        None => Ok(pack.official),
    }
}

impl Stamp {
    /// The pack page as its manager sees it, freshly loaded.
    async fn managed_detail(&self, viewer: &Viewer, pack_id: Uuid) -> AppResult<Pack> {
        let pack =
            packs::find_by_id(&self.core.db, pack_id).await?.or_not_found("PACK_NOT_FOUND", "stamp pack not found")?;
        self.detail(viewer, &pack).await
    }
}

impl StampStudioService for Stamp {
    async fn get_creator_profile(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetCreatorProfileRequest>,
    ) -> ServiceResult<GetCreatorProfileResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let (totals, trusted) = tokio::try_join!(
                stats::creator_totals(&self.core, user_id),
                creators::is_trusted(&self.core.db, user_id)
            )?;
            let profile = CreatorProfile {
                trusted,
                pack_count: totals.pack_count as i32,
                published_pack_count: totals.published_pack_count as i32,
                install_count: totals.install_count,
                send_count: totals.send_count,
                sale_count: totals.sale_count,
                earnings: totals.earnings,
                fee_percent: self.cfg.fee_percent as i32,
                max_packs: self.cfg.max_packs as i32,
                max_stamps_per_pack: MAX_STAMPS_PER_PACK as i32,
                min_price: MIN_PAID_PRICE,
                max_price: MAX_PRICE,
                ..Default::default()
            };
            ok(GetCreatorProfileResponse { profile: profile.into(), ..Default::default() })
        })
        .await
    }

    async fn list_my_packs(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListMyPacksRequest>,
    ) -> ServiceResult<ListMyPacksResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            if request.official {
                ctx.require_role("admin")?;
            }
            let page = Page::from_view(request.page.as_option(), 100);
            let before: Option<Uuid> = page.decode()?;
            let mut rows: Vec<PackRow> = sqlx::query_as(concat!(
                "SELECT ",
                pack_columns!(),
                " FROM stamp_packs p
                  WHERE (CASE WHEN $2 THEN p.official ELSE p.creator_id = $1 END)
                    AND ($3 = 0 OR p.status = $3)
                    AND ($4::uuid IS NULL OR p.id < $4)
                  ORDER BY p.id DESC
                  LIMIT $5"
            ))
            .bind(user_id)
            .bind(request.official)
            .bind(request.status.to_i32())
            .bind(before)
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let page = page.finish(&mut rows, |pack| pack.id);
            ok(ListMyPacksResponse {
                packs: self.summaries(&viewer, &rows).await?,
                page: page.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn create_pack(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreatePackRequest>,
    ) -> ServiceResult<CreatePackResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            if request.official {
                ctx.require_role("admin")?;
            }
            let name = validate::pack_name(request.name)?;
            let description = validate::description(request.description)?;
            let tags = validate::tags(&request.tags.iter().copied().collect::<Vec<&str>>())?;
            let price = validate::price(request.price)?;
            self.core.cache.rate_limit(&format!("stamp-pack-create:{user_id}"), 20, Duration::from_secs(3600)).await?;

            let creator_id = (!request.official).then_some(user_id);
            let mut tx = self.core.db.begin().await?;
            if let Some(creator_id) = creator_id {
                creators::lock(&mut tx, creator_id).await?;
                let pack_count: i64 = sqlx::query_scalar("SELECT count(*) FROM stamp_packs WHERE creator_id = $1")
                    .bind(creator_id)
                    .fetch_one(&mut *tx)
                    .await?;
                let max = self.cfg.max_packs;
                ensure!(
                    pack_count < max,
                    precondition,
                    "PACK_LIMIT_REACHED",
                    "you can have at most {max} packs; delete one first"
                );
            }
            let pack: PackRow = sqlx::query_as(concat!(
                "INSERT INTO stamp_packs AS p (short_id, creator_id, official, name, description, tags, price)
                 VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING ",
                pack_columns!()
            ))
            .bind(new_short_id())
            .bind(creator_id)
            .bind(request.official)
            .bind(&name)
            .bind(&description)
            .bind(&tags)
            .bind(price)
            .fetch_one(&mut *tx)
            .await?;
            // The draft goes straight into its creator's picker, so they can try
            // the stamps in real conversations while working on it. A full
            // library is no reason to refuse a new pack: it is then simply not installed.
            if let Some(creator_id) = creator_id {
                match installs::ensure_room(&mut *tx, creator_id, pack.id).await {
                    Ok(()) => {
                        installs::install(&mut tx, creator_id, &pack, false).await?;
                    }
                    Err(e) if e.reason == "INSTALL_LIMIT_REACHED" => {}
                    Err(e) => return Err(e),
                }
            }
            tx.commit().await?;
            ok(CreatePackResponse { pack: self.detail(&viewer, &pack).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn update_pack(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdatePackRequest>,
    ) -> ServiceResult<UpdatePackResponse> {
        handle(async {
            let (viewer, _) = Viewer::signed_in(&ctx)?;
            let req = request.to_owned_message();
            let pack = self.managed_pack(&viewer, &req.pack_id).await?;
            let name = req.name.as_deref().map(validate::pack_name).transpose()?;
            let description = req.description.as_deref().map(validate::description).transpose()?;
            let tags = req.tags.as_option().map(|list| validate::tags(&list.tags)).transpose()?;
            let price = req.price.map(validate::price).transpose()?;
            // `Some(None)` goes back to the first stamp, `Some(Some(id))` picks one, `None` leaves the cover.
            let cover: Option<Option<Uuid>> = match req.cover_stamp_id.as_deref() {
                None => None,
                Some("") => Some(None),
                Some(id) => Some(Some(parse_id(id)?)),
            };

            let mut tx = self.core.db.begin().await?;
            let pack = packs::lock(&mut tx, pack.id).await?;
            access::ensure_editable(pack.status)?;
            if let Some(Some(stamp_id)) = cover {
                let in_pack: bool =
                    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM stamps WHERE id = $1 AND pack_id = $2)")
                        .bind(stamp_id)
                        .bind(pack.id)
                        .fetch_one(&mut *tx)
                        .await?;
                ensure!(in_pack, invalid, "COVER_INVALID", "the cover must be a stamp of this pack");
            }
            sqlx::query(
                "UPDATE stamp_packs SET
                    name           = COALESCE($2, name),
                    description    = COALESCE($3, description),
                    tags           = COALESCE($4, tags),
                    price          = COALESCE($5, price),
                    cover_stamp_id = CASE WHEN $6 THEN $7 ELSE cover_stamp_id END
                 WHERE id = $1",
            )
            .bind(pack.id)
            .bind(name)
            .bind(description)
            .bind(tags)
            .bind(price)
            .bind(cover.is_some())
            .bind(cover.flatten())
            .execute(&mut *tx)
            .await?;
            let pack = packs::refresh_derived(&mut tx, pack.id).await?;
            packs::index(&mut tx, &pack).await?;
            tx.commit().await?;
            ok(UpdatePackResponse { pack: self.detail(&viewer, &pack).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_pack(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeletePackRequest>,
    ) -> ServiceResult<DeletePackResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            let pack = self.managed_pack(&viewer, request.pack_id).await?;
            let mut tx = self.core.db.begin().await?;
            let pack = packs::lock(&mut tx, pack.id).await?;
            // People paid for this pack: it may leave the store, but not their libraries.
            ensure!(
                !has_buyers(&mut tx, pack.id).await?,
                precondition,
                "PACK_HAS_BUYERS",
                "a pack that was bought cannot be deleted; unpublish it instead"
            );
            packs::delete(&mut tx, &pack).await?;
            let deleted = PackEvent { pack_id: pack.id, creator_id: pack.creator_id, official: pack.official };
            events::emit(&mut *tx, PACK_DELETED, Some(user_id), &deleted).await?;
            tx.commit().await?;
            ok(DeletePackResponse::default())
        })
        .await
    }

    async fn add_stamps(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AddStampsRequest>,
    ) -> ServiceResult<AddStampsResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            let req = request.to_owned_message();
            ensure!(!req.stamps.is_empty(), invalid, "STAMPS_REQUIRED", "no stamps to add");
            ensure!(
                req.stamps.len() <= MAX_STAMPS_PER_REQUEST,
                invalid,
                "TOO_MANY_STAMPS",
                "add at most {MAX_STAMPS_PER_REQUEST} stamps at a time"
            );
            let pack = self.managed_pack(&viewer, &req.pack_id).await?;
            self.core.cache.rate_limit(&format!("stamp-add:{user_id}"), 600, Duration::from_secs(3600)).await?;

            // Uploads are resolved before the pack is locked: no need to hold
            // the lock across lookups in another domain.
            let mut prepared = Vec::with_capacity(req.stamps.len());
            for new in &req.stamps {
                let text = validate::stamp_text(&new.name, &new.emoji, &new.keywords)?;
                let source = stamps::source_from_upload(&self.core, &new.asset_id, user_id).await?;
                prepared.push((source, text));
            }

            let mut tx = self.core.db.begin().await?;
            let pack = packs::lock(&mut tx, pack.id).await?;
            access::ensure_can_add_stamps(pack.status, skips_review(&mut tx, &pack).await?)?;
            ensure!(
                pack.stamp_count as usize + prepared.len() <= MAX_STAMPS_PER_PACK,
                precondition,
                "PACK_FULL",
                "a pack holds at most {MAX_STAMPS_PER_PACK} stamps"
            );
            // Stamps added to a live pack (only possible without review) are live at once;
            // all others wait for the pack's next approval.
            let approved = pack.is_published();
            let first_position: i32 =
                sqlx::query_scalar("SELECT COALESCE(max(position), -1) + 1 FROM stamps WHERE pack_id = $1")
                    .bind(pack.id)
                    .fetch_one(&mut *tx)
                    .await?;
            for (offset, (source, text)) in prepared.iter().enumerate() {
                let home = StampHome::Pack { pack_id: pack.id, approved };
                stamps::insert(&mut tx, home, source, text, first_position + offset as i32).await?;
            }
            if !approved {
                sqlx::query("UPDATE stamp_packs SET needs_review = true WHERE id = $1")
                    .bind(pack.id)
                    .execute(&mut *tx)
                    .await?;
            }
            let pack = packs::refresh_derived(&mut tx, pack.id).await?;
            packs::index(&mut tx, &pack).await?;
            tx.commit().await?;
            ok(AddStampsResponse { pack: self.detail(&viewer, &pack).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn update_stamp(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateStampRequest>,
    ) -> ServiceResult<UpdateStampResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            let req = request.to_owned_message();
            let stamp_id = parse_id(&req.stamp_id)?;
            let name = req.name.as_deref().map(validate::stamp_name).transpose()?;
            let emoji = req.emoji.as_deref().map(validate::emoji).transpose()?;
            let keywords = req.keywords.as_option().map(|list| validate::keywords(&list.keywords)).transpose()?;
            let pack_id = self.managed_stamp(&viewer, stamp_id).await?;

            let mut tx = self.core.db.begin().await?;
            let pack = packs::lock(&mut tx, pack_id).await?;
            access::ensure_editable(pack.status)?;
            stamps::update_text(&mut *tx, stamp_id, name, emoji, keywords).await?;
            tx.commit().await?;
            let view = stamps::view(&self.core, Some(user_id), stamp_id).await?.ok_or_else(stamps::not_found)?;
            ok(UpdateStampResponse { stamp: view.to_proto(&viewer).into(), ..Default::default() })
        })
        .await
    }

    async fn remove_stamp(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveStampRequest>,
    ) -> ServiceResult<RemoveStampResponse> {
        handle(async {
            let (viewer, _) = Viewer::signed_in(&ctx)?;
            let stamp_id = parse_id(request.stamp_id)?;
            let pack_id = self.managed_stamp(&viewer, stamp_id).await?;

            let mut tx = self.core.db.begin().await?;
            let pack = packs::lock(&mut tx, pack_id).await?;
            access::ensure_editable(pack.status)?;
            let reviewed: Option<bool> =
                sqlx::query_scalar("SELECT approved_at IS NOT NULL FROM stamps WHERE id = $1 AND pack_id = $2")
                    .bind(stamp_id)
                    .bind(pack.id)
                    .fetch_optional(&mut *tx)
                    .await?;
            let reviewed = reviewed.ok_or_else(stamps::not_found)?;
            // Buyers paid for the reviewed stamps; stamps nobody else has seen yet can still go.
            ensure!(
                !(reviewed && has_buyers(&mut tx, pack.id).await?),
                precondition,
                "PACK_HAS_BUYERS",
                "stamps of a pack that was bought cannot be removed"
            );
            ensure!(
                !(pack.is_published() && pack.stamp_count <= 1),
                precondition,
                "PACK_EMPTY",
                "a published pack needs a stamp; unpublish or delete the pack instead"
            );
            stamps::delete(&mut tx, &[stamp_id]).await?;
            let pack = packs::refresh_derived(&mut tx, pack.id).await?;
            packs::index(&mut tx, &pack).await?;
            tx.commit().await?;
            ok(RemoveStampResponse { pack: self.detail(&viewer, &pack).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn reorder_stamps(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReorderStampsRequest>,
    ) -> ServiceResult<ReorderStampsResponse> {
        handle(async {
            let (viewer, _) = Viewer::signed_in(&ctx)?;
            let pack = self.managed_pack(&viewer, request.pack_id).await?;
            ensure!(
                request.stamp_ids.len() <= MAX_STAMPS_PER_PACK,
                invalid,
                "ORDER_MISMATCH",
                "too many stamps listed"
            );
            let ordered_ids = request.stamp_ids.iter().map(|id| parse_id(id)).collect::<AppResult<Vec<Uuid>>>()?;

            let mut tx = self.core.db.begin().await?;
            let pack = packs::lock(&mut tx, pack.id).await?;
            access::ensure_editable(pack.status)?;
            let current: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM stamps WHERE pack_id = $1")
                .bind(pack.id)
                .fetch_all(&mut *tx)
                .await?;
            stamps::ensure_same_set(&current, &ordered_ids)?;
            stamps::apply_order(&mut tx, &ordered_ids).await?;
            // The first stamp may be the cover.
            let pack = packs::refresh_derived(&mut tx, pack.id).await?;
            packs::index(&mut tx, &pack).await?;
            tx.commit().await?;
            ok(ReorderStampsResponse { pack: self.detail(&viewer, &pack).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn submit_pack(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SubmitPackRequest>,
    ) -> ServiceResult<SubmitPackResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            let pack = self.managed_pack(&viewer, request.pack_id).await?;
            self.core.cache.rate_limit(&format!("stamp-submit:{user_id}"), 30, Duration::from_secs(3600)).await?;

            let mut tx = self.core.db.begin().await?;
            let pack = packs::lock(&mut tx, pack.id).await?;
            ensure!(
                matches!(pack.status, status::DRAFT | status::REJECTED | status::REMOVED),
                precondition,
                "PACK_NOT_SUBMITTABLE",
                "the pack is already in review or published"
            );
            ensure!(pack.stamp_count >= 1, precondition, "PACK_EMPTY", "add at least one stamp before submitting");
            let trusted = match pack.creator_id {
                Some(creator_id) => creators::is_trusted(&mut *tx, creator_id).await?,
                None => false,
            };
            if access::submission_needs_moderator(&pack.facts(), pack.needs_review, trusted) {
                lifecycle::send_to_review(&mut tx, &pack, user_id).await?;
            } else {
                lifecycle::publish(&mut tx, &pack, Approval::Automatic, Some(user_id)).await?;
            }
            tx.commit().await?;
            ok(SubmitPackResponse { pack: self.managed_detail(&viewer, pack.id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn unpublish_pack(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnpublishPackRequest>,
    ) -> ServiceResult<UnpublishPackResponse> {
        handle(async {
            let (viewer, user_id) = Viewer::signed_in(&ctx)?;
            let pack = self.managed_pack(&viewer, request.pack_id).await?;
            let mut tx = self.core.db.begin().await?;
            let pack = packs::lock(&mut tx, pack.id).await?;
            ensure!(
                matches!(pack.status, status::PUBLISHED | status::IN_REVIEW),
                precondition,
                "PACK_NOT_PUBLISHED",
                "the pack is neither published nor in review"
            );
            lifecycle::unpublish(&mut tx, &pack, Some(user_id)).await?;
            tx.commit().await?;
            ok(UnpublishPackResponse {
                pack: self.managed_detail(&viewer, pack.id).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_pack_stats(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetPackStatsRequest>,
    ) -> ServiceResult<GetPackStatsResponse> {
        handle(async {
            let (viewer, _) = Viewer::signed_in(&ctx)?;
            let pack = self.managed_pack(&viewer, request.pack_id).await?;
            let days = if request.days <= 0 { 30 } else { request.days.min(90) };
            ok(stats::pack_stats(&self.core, &viewer, &pack, days).await?)
        })
        .await
    }
}

impl Stamp {
    /// The pack of a stamp the viewer may edit. Personal stamps, and stamps
    /// of other people's packs, are `STAMP_NOT_FOUND`.
    async fn managed_stamp(&self, viewer: &Viewer, stamp_id: Uuid) -> AppResult<Uuid> {
        let view = stamps::view(&self.core, viewer.user_id, stamp_id).await?.ok_or_else(stamps::not_found)?;
        match (view.pack_id, view.pack()) {
            (Some(pack_id), Some(pack)) if viewer.manages(&pack) => Ok(pack_id),
            _ => Err(stamps::not_found()),
        }
    }
}
