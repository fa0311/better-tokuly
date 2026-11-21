//! `StampReviewService`: what moderators do to the store.
use std::collections::HashMap;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{
    events,
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::stamp::*;

use crate::{
    Stamp,
    access::Viewer,
    creators,
    events::{CREATOR_TRUST_CHANGED, CreatorTrust},
    lifecycle::{self, Approval},
    model::{PackRow, pack_columns, status},
    packs, validate,
};

const MODERATOR: &str = "moderator";

/// The moderator calling an RPC of this service.
fn moderator(ctx: &RequestContext) -> AppResult<(Viewer, Uuid)> {
    ctx.require_role(MODERATOR)?;
    Viewer::signed_in(ctx)
}

#[derive(sqlx::FromRow)]
struct ReviewRow {
    pack_id: Uuid,
    moderator_id: Option<Uuid>,
    decision: i32,
    note: String,
    created_at: Time,
}

impl Stamp {
    /// Earlier decisions on each pack, newest first.
    async fn review_history(&self, pack_ids: &[Uuid]) -> AppResult<HashMap<Uuid, Vec<ReviewRecord>>> {
        let rows: Vec<ReviewRow> =
            sqlx::query_as("SELECT pack_id, moderator_id, decision, note, created_at FROM stamp_pack_reviews WHERE pack_id = ANY($1) ORDER BY id DESC")
                .bind(pack_ids)
                .fetch_all(&self.core.db)
                .await?;
        let moderator_ids: Vec<Uuid> = rows.iter().filter_map(|row| row.moderator_id).collect();
        let moderators = tokuly_identity::summaries(&self.core, &moderator_ids).await?;
        let mut history: HashMap<Uuid, Vec<ReviewRecord>> = HashMap::new();
        for row in rows {
            history.entry(row.pack_id).or_default().push(ReviewRecord {
                decision: row.decision.into(),
                note: row.note,
                moderator: row.moderator_id.and_then(|id| moderators.get(&id).cloned()).into(),
                decided_at: ts(row.created_at),
                ..Default::default()
            });
        }
        Ok(history)
    }

    /// Runs a moderator's decision on a pack that is in review.
    async fn decide(
        &self,
        viewer: &Viewer,
        moderator_id: Uuid,
        pack_ref: &str,
        rejection: Option<&str>,
    ) -> AppResult<Pack> {
        let pack = packs::find(&self.core, pack_ref).await?.or_not_found("PACK_NOT_FOUND", "stamp pack not found")?;
        let mut tx = self.core.db.begin().await?;
        let pack = packs::lock(&mut tx, pack.id).await?;
        ensure!(
            pack.status == status::IN_REVIEW,
            precondition,
            "PACK_NOT_IN_REVIEW",
            "the pack is not waiting for review"
        );
        match rejection {
            Some(reason) => lifecycle::reject(&mut tx, &pack, moderator_id, reason).await?,
            None => {
                lifecycle::publish(&mut tx, &pack, Approval::Moderator(moderator_id), Some(moderator_id)).await?;
            }
        }
        tx.commit().await?;
        let pack =
            packs::find_by_id(&self.core.db, pack.id).await?.or_not_found("PACK_NOT_FOUND", "stamp pack not found")?;
        self.detail(viewer, &pack).await
    }
}

impl StampReviewService for Stamp {
    async fn list_review_queue(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListReviewQueueRequest>,
    ) -> ServiceResult<ListReviewQueueResponse> {
        handle(async {
            let (viewer, _) = moderator(&ctx)?;
            // Items carry every stamp of a pack, so pages are small.
            let page = match request.page.as_option() {
                Some(page) => Page::new(page.limit, page.cursor, 10, 30),
                None => Page::new(0, "", 10, 30),
            };
            let cursor: Option<(Time, Uuid)> = page.decode()?;
            let (after_at, after_id) = cursor.unzip();
            let mut rows: Vec<PackRow> = sqlx::query_as(concat!(
                "SELECT ",
                pack_columns!(),
                " FROM stamp_packs p
                  WHERE p.status = 2 AND ($1::timestamptz IS NULL OR (p.submitted_at, p.id) > ($1, $2))
                  ORDER BY p.submitted_at, p.id
                  LIMIT $3"
            ))
            .bind(after_at)
            .bind(after_id)
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let page = page.finish(&mut rows, |pack| (pack.submitted_at.unwrap_or(pack.created_at), pack.id));
            let pending_count: i64 = sqlx::query_scalar("SELECT count(*) FROM stamp_packs WHERE status = 2")
                .fetch_one(&self.core.db)
                .await?;

            let mut history = self.review_history(&rows.iter().map(|pack| pack.id).collect::<Vec<_>>()).await?;
            let mut items = Vec::with_capacity(rows.len());
            for pack in &rows {
                items.push(ReviewItem {
                    pack: self.detail(&viewer, pack).await?.into(),
                    history: history.remove(&pack.id).unwrap_or_default(),
                    ..Default::default()
                });
            }
            ok(ListReviewQueueResponse {
                items,
                page: page.into(),
                pending_count: pending_count as i32,
                ..Default::default()
            })
        })
        .await
    }

    async fn approve_pack(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ApprovePackRequest>,
    ) -> ServiceResult<ApprovePackResponse> {
        handle(async {
            let (viewer, moderator_id) = moderator(&ctx)?;
            let pack = self.decide(&viewer, moderator_id, request.pack_id, None).await?;
            ok(ApprovePackResponse { pack: pack.into(), ..Default::default() })
        })
        .await
    }

    async fn reject_pack(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RejectPackRequest>,
    ) -> ServiceResult<RejectPackResponse> {
        handle(async {
            let (viewer, moderator_id) = moderator(&ctx)?;
            let reason = validate::reason(request.reason)?;
            let pack = self.decide(&viewer, moderator_id, request.pack_id, Some(&reason)).await?;
            ok(RejectPackResponse { pack: pack.into(), ..Default::default() })
        })
        .await
    }

    async fn set_creator_trusted(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetCreatorTrustedRequest>,
    ) -> ServiceResult<SetCreatorTrustedResponse> {
        handle(async {
            let (_, moderator_id) = moderator(&ctx)?;
            let user_id = parse_id(request.user_id)?;
            let trusted = request.trusted;
            let active = tokuly_identity::users::filter_active(&self.core, &[user_id]).await?;
            ensure!(active.contains(&user_id), not_found, "USER_NOT_FOUND", "user not found");

            let mut tx = self.core.db.begin().await?;
            if creators::set_trusted(&mut tx, user_id, trusted, moderator_id).await? {
                events::emit(
                    &mut *tx,
                    CREATOR_TRUST_CHANGED,
                    Some(moderator_id),
                    &CreatorTrust { user_id, trusted, moderator_id },
                )
                .await?;
                if trusted {
                    let notice = NotificationRequest::new(
                        user_id,
                        "stamp.creator.trusted",
                        Localized::new("認定クリエイターになったよ!", "You are now a trusted creator!"),
                        Localized::new(
                            "これからは審査なしでスタンプを公開できるよ!",
                            "Your stamp packs are now published without review.",
                        ),
                        "/stamps/studio",
                    );
                    outbound::notify(&mut *tx, &notice).await?;
                }
            }
            tx.commit().await?;
            ok(SetCreatorTrustedResponse::default())
        })
        .await
    }

    async fn set_pack_featured(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetPackFeaturedRequest>,
    ) -> ServiceResult<SetPackFeaturedResponse> {
        handle(async {
            let (viewer, _) = moderator(&ctx)?;
            let pack = packs::find(&self.core, request.pack_id).await?.or_not_found("PACK_NOT_FOUND", "stamp pack not found")?;
            let mut tx = self.core.db.begin().await?;
            let pack = packs::lock(&mut tx, pack.id).await?;
            ensure!(!request.featured || pack.is_published(), precondition, "PACK_NOT_PUBLISHED", "only published packs can be featured");
            // Featuring again keeps the pack's place on the shelf.
            let pack: PackRow = sqlx::query_as(concat!(
                "UPDATE stamp_packs p SET featured_at = CASE WHEN $2 THEN COALESCE(p.featured_at, now()) END WHERE p.id = $1 RETURNING ",
                pack_columns!()
            ))
            .bind(pack.id)
            .bind(request.featured)
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;
            ok(SetPackFeaturedResponse { pack: self.summary(&viewer, &pack).await?.into(), ..Default::default() })
        })
        .await
    }
}
