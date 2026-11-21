//! Loading packs and turning them into API messages; the derived columns
//! and the search document every pack change keeps in step.
use std::collections::{HashMap, HashSet};

use sqlx::{PgExecutor, Postgres, Transaction};
use tokuly_core::{ids::Ref, outbound, prelude::*};
use tokuly_proto::{common::StampRef, stamp as pb};

use crate::{
    Stamp,
    access::{Holding, Viewer},
    creators,
    model::{PackRow, pack_columns},
    stamps,
};

/// Stamps shown on a pack card.
const PREVIEWS_PER_PACK: i64 = 5;
/// The search index packs live in (`SearchDoc::kind`).
const SEARCH_KIND: &str = "stamp_pack";

fn not_found() -> AppError {
    AppError::not_found("PACK_NOT_FOUND", "stamp pack not found")
}

pub async fn find_by_id<'e>(executor: impl PgExecutor<'e>, pack_id: Uuid) -> AppResult<Option<PackRow>> {
    Ok(sqlx::query_as(concat!("SELECT ", pack_columns!(), " FROM stamp_packs p WHERE p.id = $1"))
        .bind(pack_id)
        .fetch_optional(executor)
        .await?)
}

/// Looks a pack up by UUID or short ID.
pub async fn find(core: &Core, pack_ref: &str) -> AppResult<Option<PackRow>> {
    match Ref::parse(pack_ref)? {
        Ref::Id(id) => find_by_id(&core.db, id).await,
        Ref::Short(short_id) => {
            Ok(sqlx::query_as(concat!("SELECT ", pack_columns!(), " FROM stamp_packs p WHERE p.short_id = $1"))
                .bind(short_id)
                .fetch_optional(&core.db)
                .await?)
        }
    }
}

/// Locks a pack for the rest of the transaction and returns its current row.
/// Every change to a pack starts here, so edits, submissions and moderation
/// decisions on the same pack happen one after the other.
pub async fn lock(tx: &mut Transaction<'_, Postgres>, pack_id: Uuid) -> AppResult<PackRow> {
    sqlx::query_as(concat!("SELECT ", pack_columns!(), " FROM stamp_packs p WHERE p.id = $1 FOR UPDATE"))
        .bind(pack_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(not_found)
}

/// Deletes a pack with its stamps: their uploads are released, and a
/// published pack leaves the search index. The pack must be locked.
pub async fn delete(tx: &mut Transaction<'_, Postgres>, pack: &PackRow) -> AppResult<()> {
    stamps::delete_of_pack(tx, pack.id).await?;
    sqlx::query("DELETE FROM stamp_packs WHERE id = $1").bind(pack.id).execute(&mut **tx).await?;
    if pack.is_published() {
        unindex(tx, pack.id).await?;
    }
    Ok(())
}

/// Recomputes the columns derived from a pack's stamps — cover, counts and
/// the animated flag — and returns the updated row. Call after every change
/// to the pack's stamps or cover choice.
///
/// The cover prefers reviewed stamps, so that a pack other people already
/// hold never shows them a picture no moderator has seen.
pub async fn refresh_derived(tx: &mut Transaction<'_, Postgres>, pack_id: Uuid) -> AppResult<PackRow> {
    let row = sqlx::query_as(concat!(
        "UPDATE stamp_packs p SET
            stamp_count    = (SELECT count(*) FROM stamps s WHERE s.pack_id = p.id),
            approved_count = (SELECT count(*) FROM stamps s WHERE s.pack_id = p.id AND s.approved_at IS NOT NULL),
            animated       = EXISTS (SELECT 1 FROM stamps s WHERE s.pack_id = p.id AND s.format <> 1),
            cover          = COALESCE(
                (SELECT s.image FROM stamps s WHERE s.id = p.cover_stamp_id AND s.approved_at IS NOT NULL),
                (SELECT s.image FROM stamps s WHERE s.pack_id = p.id AND s.approved_at IS NOT NULL ORDER BY s.position, s.id LIMIT 1),
                (SELECT s.image FROM stamps s WHERE s.id = p.cover_stamp_id),
                (SELECT s.image FROM stamps s WHERE s.pack_id = p.id ORDER BY s.position, s.id LIMIT 1))
         WHERE p.id = $1 RETURNING ",
        pack_columns!()
    ))
    .bind(pack_id)
    .fetch_optional(&mut **tx)
    .await?;
    row.ok_or_else(not_found)
}

/// Writes the pack's search document. Only published packs are searchable,
/// so this does nothing for any other status; a pack leaving the store is
/// taken out of the index with [`unindex`].
pub async fn index(tx: &mut Transaction<'_, Postgres>, pack: &PackRow) -> AppResult<()> {
    if !pack.is_published() {
        return Ok(());
    }
    let doc = outbound::SearchDoc {
        kind: SEARCH_KIND.into(),
        id: pack.id,
        title: pack.name.clone(),
        text: pack.description.clone(),
        tags: pack.tags.clone(),
        owner_id: pack.creator_id,
        public: true,
        image_url: pack.cover_url(),
        path: pack.path(),
        created_at: pack.published_at.unwrap_or(pack.created_at).timestamp(),
        score: pack.install_count as f64,
        extra: serde_json::json!({
            "short_id": pack.short_id,
            "price": pack.price,
            "official": pack.official,
            "animated": pack.animated,
            "stamp_count": pack.approved_count,
        }),
    };
    outbound::index(&mut **tx, doc).await
}

/// Removes the pack from the search index.
pub async fn unindex(tx: &mut Transaction<'_, Postgres>, pack_id: Uuid) -> AppResult<()> {
    outbound::unindex(&mut **tx, SEARCH_KIND, pack_id).await
}

/// What `user_id` holds of one pack.
pub async fn holding(core: &Core, user_id: Option<Uuid>, pack_id: Uuid) -> AppResult<Holding> {
    Ok(holdings(core, user_id, &[pack_id]).await?.remove(&pack_id).unwrap_or_default())
}

/// What `user_id` holds of each pack. Packs they hold nothing of are absent.
pub async fn holdings(core: &Core, user_id: Option<Uuid>, pack_ids: &[Uuid]) -> AppResult<HashMap<Uuid, Holding>> {
    let Some(user_id) = user_id else { return Ok(HashMap::new()) };
    if pack_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(Uuid, bool, bool)> = sqlx::query_as(
        "SELECT pack.id,
                EXISTS (SELECT 1 FROM stamp_pack_installs i WHERE i.pack_id = pack.id AND i.user_id = $1),
                EXISTS (SELECT 1 FROM stamp_pack_purchases o WHERE o.pack_id = pack.id AND o.user_id = $1)
         FROM unnest($2::uuid[]) AS pack(id)",
    )
    .bind(user_id)
    .bind(pack_ids)
    .fetch_all(&core.db)
    .await?;
    Ok(rows.into_iter().map(|(id, installed, owned)| (id, Holding { installed, owned })).collect())
}

#[derive(sqlx::FromRow)]
struct PreviewRow {
    id: Uuid,
    pack_id: Uuid,
    name: String,
    image_url: String,
    animation_url: String,
    format: i32,
    width: i32,
    height: i32,
}

/// The first stamps of each pack. Unreviewed stamps are included only for
/// the packs in `with_unapproved`.
async fn previews(core: &Core, pack_ids: &[Uuid], with_unapproved: &[Uuid]) -> AppResult<HashMap<Uuid, Vec<StampRef>>> {
    let rows: Vec<PreviewRow> = sqlx::query_as(
        "SELECT id, pack_id, name, image_url, animation_url, format, width, height
         FROM (SELECT s.*, row_number() OVER (PARTITION BY s.pack_id ORDER BY s.position, s.id) AS rank
               FROM stamps s
               WHERE s.pack_id = ANY($1) AND (s.approved_at IS NOT NULL OR s.pack_id = ANY($2))) ranked
         WHERE rank <= $3
         ORDER BY pack_id, rank",
    )
    .bind(pack_ids)
    .bind(with_unapproved)
    .bind(PREVIEWS_PER_PACK)
    .fetch_all(&core.db)
    .await?;
    let mut out: HashMap<Uuid, Vec<StampRef>> = HashMap::new();
    for row in rows {
        out.entry(row.pack_id).or_default().push(StampRef {
            id: row.id.to_string(),
            pack_id: row.pack_id.to_string(),
            name: row.name,
            image_url: row.image_url,
            animation_url: row.animation_url,
            format: row.format.into(),
            width: row.width,
            height: row.height,
            ..Default::default()
        });
    }
    Ok(out)
}

impl Stamp {
    /// Pack cards for `viewer`, in the order of `rows`: creators, preview
    /// stamps and the viewer's own state are fetched in three batches.
    pub(crate) async fn summaries(&self, viewer: &Viewer, rows: &[PackRow]) -> AppResult<Vec<pb::PackSummary>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let core = &self.core;
        let pack_ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
        let creator_ids: Vec<Uuid> =
            rows.iter().filter_map(|r| r.creator_id).collect::<HashSet<_>>().into_iter().collect();
        let with_unapproved: Vec<Uuid> =
            rows.iter().filter(|r| viewer.sees_unapproved(&r.facts())).map(|r| r.id).collect();

        let (holdings, creators, mut previews) = tokio::try_join!(
            holdings(core, viewer.user_id, &pack_ids),
            tokuly_identity::summaries(core, &creator_ids),
            previews(core, &pack_ids, &with_unapproved),
        )?;
        Ok(rows
            .iter()
            .map(|row| {
                let creator = row.creator_id.map(|id| tokuly_identity::users::pick(&creators, id));
                let holding = holdings.get(&row.id).copied().unwrap_or_default();
                row.summary(viewer, holding, creator, previews.remove(&row.id).unwrap_or_default())
            })
            .collect())
    }

    pub(crate) async fn summary(&self, viewer: &Viewer, row: &PackRow) -> AppResult<pb::PackSummary> {
        let mut summaries = self.summaries(viewer, std::slice::from_ref(row)).await?;
        Ok(summaries.pop().unwrap_or_default())
    }

    /// The pack page for `viewer`: the card, the stamps they may see and,
    /// for managers and moderators, the review state.
    pub(crate) async fn detail(&self, viewer: &Viewer, row: &PackRow) -> AppResult<pb::Pack> {
        let facts = row.facts();
        let sees_unapproved = viewer.sees_unapproved(&facts);
        let views = stamps::views_of_pack(&self.core, viewer.user_id, row.id).await?;
        let stamps =
            views.iter().filter(|v| sees_unapproved || v.approved_at.is_some()).map(|v| v.to_proto(viewer)).collect();
        let review = if sees_unapproved {
            let trusted = match row.creator_id {
                Some(creator) => creators::is_trusted(&self.core.db, creator).await?,
                None => false,
            };
            Some(row.review(trusted))
        } else {
            None
        };
        Ok(pb::Pack {
            summary: self.summary(viewer, row).await?.into(),
            description: row.description.clone(),
            stamps,
            cover_stamp_id: row.cover_stamp_id.map(|id| id.to_string()).unwrap_or_default(),
            created_at: ts(row.created_at),
            updated_at: ts(row.updated_at),
            review: review.into(),
            ..Default::default()
        })
    }

    /// The pack `pack_ref` names, if `viewer` may see it. Anything else is
    /// `PACK_NOT_FOUND`, so the existence of private packs does not leak.
    pub(crate) async fn visible_pack(&self, viewer: &Viewer, pack_ref: &str) -> AppResult<(PackRow, Holding)> {
        let pack = find(&self.core, pack_ref).await?.ok_or_else(not_found)?;
        let holding = holding(&self.core, viewer.user_id, pack.id).await?;
        ensure!(viewer.can_view(&pack.facts(), holding), not_found, "PACK_NOT_FOUND", "stamp pack not found");
        Ok((pack, holding))
    }

    /// The pack `pack_ref` names, if `viewer` may edit it.
    pub(crate) async fn managed_pack(&self, viewer: &Viewer, pack_ref: &str) -> AppResult<PackRow> {
        let pack = find(&self.core, pack_ref).await?.ok_or_else(not_found)?;
        ensure!(viewer.manages(&pack.facts()), not_found, "PACK_NOT_FOUND", "stamp pack not found");
        Ok(pack)
    }
}

/// Appends the store's price filter to a query on `stamp_packs p`.
pub fn push_price_filter(query: &mut sqlx::QueryBuilder<Postgres>, price: pb::PriceFilter) {
    match price {
        pb::PriceFilter::PRICE_FILTER_FREE => {
            query.push(" AND p.price = 0");
        }
        pb::PriceFilter::PRICE_FILTER_PAID => {
            query.push(" AND p.price > 0");
        }
        pb::PriceFilter::PRICE_FILTER_UNSPECIFIED => {}
    }
}
