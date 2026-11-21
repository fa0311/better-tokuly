//! Personal ("quick") stamps: single images a user keeps as stamps of their
//! own, and saving a stamp someone else sent.
use sqlx::{Postgres, Transaction};
use tokuly_core::prelude::*;
use tokuly_proto::{common::PageInfo, stamp::SaveOutcome};

use crate::{
    Stamp,
    access::{InstallCheck, Viewer},
    installs,
    model::{PackRow, StampView, stamp_view_select},
    packs,
    stamps::{self, StampHome, StampSource},
    validate::StampText,
};

/// Makes room for one more personal stamp or fails with
/// `PERSONAL_STAMP_LIMIT_REACHED`, and returns the position that puts the
/// new stamp first.
async fn next_personal_position(tx: &mut Transaction<'_, Postgres>, owner_id: Uuid, max: i64) -> AppResult<i32> {
    let (count, front): (i64, Option<i32>) =
        sqlx::query_as("SELECT count(*), min(position) FROM stamps WHERE owner_id = $1")
            .bind(owner_id)
            .fetch_one(&mut **tx)
            .await?;
    ensure!(
        count < max,
        precondition,
        "PERSONAL_STAMP_LIMIT_REACHED",
        "you can keep at most {max} personal stamps; delete one first"
    );
    Ok(front.unwrap_or(0) - 1)
}

/// Creates a personal stamp from a processed upload.
pub async fn create(core: &Core, owner_id: Uuid, source: &StampSource, text: &StampText, max: i64) -> AppResult<Uuid> {
    let mut tx = core.db.begin().await?;
    let position = next_personal_position(&mut tx, owner_id, max).await?;
    let id = stamps::insert(&mut tx, StampHome::Personal { owner_id }, source, text, position).await?;
    tx.commit().await?;
    Ok(id)
}

/// One page of a user's personal stamps in their order.
pub async fn list(core: &Core, owner_id: Uuid, page: &Page) -> AppResult<(Vec<StampView>, PageInfo)> {
    let cursor: Option<(i32, Uuid)> = page.decode()?;
    let (after_position, after_id) = cursor.unzip();
    let mut rows: Vec<StampView> = sqlx::query_as(concat!(
        stamp_view_select!(),
        "WHERE s.owner_id = $1 AND ($2::int IS NULL OR (s.position, s.id) > ($2, $3))
         ORDER BY s.position, s.id
         LIMIT $4"
    ))
    .bind(owner_id)
    .bind(after_position)
    .bind(after_id)
    .bind(page.fetch())
    .fetch_all(&core.db)
    .await?;
    let info = page.finish(&mut rows, |view| (view.position, view.id));
    Ok((rows, info))
}

/// The caller's personal stamp `stamp_id`; `STAMP_NOT_FOUND` for anything else.
pub async fn owned(core: &Core, owner_id: Uuid, stamp_id: Uuid) -> AppResult<StampView> {
    stamps::view(core, Some(owner_id), stamp_id)
        .await?
        .filter(|view| view.owner_id == Some(owner_id))
        .ok_or_else(stamps::not_found)
}

/// Deletes a personal stamp and lets go of its upload. Copies other people
/// saved are theirs and stay: each holds the upload in its own right.
pub async fn delete(core: &Core, owner_id: Uuid, stamp_id: Uuid) -> AppResult<()> {
    let mut tx = core.db.begin().await?;
    let theirs: Option<Uuid> = sqlx::query_scalar("SELECT id FROM stamps WHERE id = $1 AND owner_id = $2 FOR UPDATE")
        .bind(stamp_id)
        .bind(owner_id)
        .fetch_optional(&mut *tx)
        .await?;
    ensure!(theirs.is_some(), not_found, "STAMP_NOT_FOUND", "stamp not found");
    stamps::delete(&mut tx, &[stamp_id]).await?;
    tx.commit().await?;
    Ok(())
}

/// Puts the user's personal stamps in the given order, which must list all of them.
pub async fn reorder(core: &Core, owner_id: Uuid, ordered_ids: &[Uuid]) -> AppResult<()> {
    let mut tx = core.db.begin().await?;
    let current: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM stamps WHERE owner_id = $1 FOR UPDATE")
        .bind(owner_id)
        .fetch_all(&mut *tx)
        .await?;
    stamps::ensure_same_set(&current, ordered_ids)?;
    stamps::apply_order(&mut tx, ordered_ids).await?;
    tx.commit().await?;
    Ok(())
}

/// What saving a received stamp would do for a viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavePlan {
    /// Add the stamp's pack to the library.
    InstallPack,
    /// Copy the (personal) stamp into the viewer's personal stamps.
    CopyToPersonal,
    /// The viewer already has it.
    AlreadyInLibrary,
    /// The stamp's pack is paid and not bought.
    PurchaseRequired,
    /// The stamp's pack is not in the store, so it cannot be had (yet).
    NotSavable,
}

/// Decides what saving `view` means for `viewer`. A stamp from a pack is
/// saved by getting the pack — copying it out would bypass paid packs — and
/// a personal stamp by copying it.
pub fn plan_save(view: &StampView, viewer: &Viewer) -> SavePlan {
    let Some(user_id) = viewer.user_id else { return SavePlan::NotSavable };
    match view.pack() {
        None if view.owner_id == Some(user_id) => SavePlan::AlreadyInLibrary,
        None => SavePlan::CopyToPersonal,
        Some(_) if view.installed => SavePlan::AlreadyInLibrary,
        Some(pack) => match viewer.can_install(&pack, view.holding()) {
            InstallCheck::Allowed => SavePlan::InstallPack,
            InstallCheck::PurchaseRequired => SavePlan::PurchaseRequired,
            InstallCheck::Unavailable => SavePlan::NotSavable,
        },
    }
}

/// The viewer's own personal stamp showing the same picture, if they have
/// one: the original they uploaded, or a copy they saved before. Copies keep
/// the asset ID of the upload they come from, which is what identifies a
/// picture.
async fn existing_copy<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    owner_id: Uuid,
    view: &StampView,
) -> AppResult<Option<Uuid>> {
    let Some(asset_id) = view.asset_id else { return Ok(None) };
    Ok(sqlx::query_scalar("SELECT id FROM stamps WHERE owner_id = $1 AND asset_id = $2 ORDER BY id LIMIT 1")
        .bind(owner_id)
        .bind(asset_id)
        .fetch_optional(executor)
        .await?)
}

/// Whether SaveStamp would add something to the viewer's library.
pub async fn would_save(core: &Core, viewer: &Viewer, view: &StampView) -> AppResult<bool> {
    Ok(match (plan_save(view, viewer), viewer.user_id) {
        (SavePlan::InstallPack, _) => true,
        (SavePlan::CopyToPersonal, Some(user_id)) => existing_copy(&core.db, user_id, view).await?.is_none(),
        _ => false,
    })
}

/// The result of saving a stamp.
pub struct Saved {
    pub outcome: SaveOutcome,
    /// The stamp the caller ends up with: the same one, or their new copy.
    pub stamp_id: Uuid,
    /// The stamp's pack, when it has one.
    pub pack: Option<PackRow>,
}

impl Stamp {
    /// Saves a stamp the caller received (see [`plan_save`]).
    pub(crate) async fn save_received_stamp(&self, viewer: &Viewer, user_id: Uuid, stamp_id: Uuid) -> AppResult<Saved> {
        let core = &self.core;
        let view = stamps::view(core, Some(user_id), stamp_id)
            .await?
            .filter(|v| v.visible_to(viewer))
            .ok_or_else(stamps::not_found)?;
        let pack = match view.pack_id {
            Some(pack_id) => packs::find_by_id(&core.db, pack_id).await?,
            None => None,
        };
        let unchanged = |outcome| Saved { outcome, stamp_id: view.id, pack: pack.clone() };
        match plan_save(&view, viewer) {
            SavePlan::AlreadyInLibrary => Ok(unchanged(SaveOutcome::SAVE_OUTCOME_ALREADY_IN_LIBRARY)),
            SavePlan::PurchaseRequired => Ok(unchanged(SaveOutcome::SAVE_OUTCOME_PURCHASE_REQUIRED)),
            SavePlan::NotSavable => {
                Err(AppError::precondition("STAMP_NOT_SAVABLE", "this stamp's pack is not in the store"))
            }
            SavePlan::InstallPack => {
                let pack = pack.ok_or_else(stamps::not_found)?;
                let mut tx = core.db.begin().await?;
                installs::ensure_room(&mut *tx, user_id, pack.id).await?;
                installs::install(&mut tx, user_id, &pack, !viewer.manages(&pack.facts())).await?;
                tx.commit().await?;
                Ok(Saved { outcome: SaveOutcome::SAVE_OUTCOME_PACK_INSTALLED, stamp_id: view.id, pack: Some(pack) })
            }
            SavePlan::CopyToPersonal => {
                let mut tx = core.db.begin().await?;
                if let Some(existing) = existing_copy(&mut *tx, user_id, &view).await? {
                    return Ok(Saved {
                        outcome: SaveOutcome::SAVE_OUTCOME_ALREADY_IN_LIBRARY,
                        stamp_id: existing,
                        pack: None,
                    });
                }
                let position = next_personal_position(&mut tx, user_id, self.cfg.max_personal_stamps).await?;
                // The copy shows the same picture and holds the same upload in
                // its own right: it outlives the stamp it was copied from.
                let source = StampSource {
                    format: view.format,
                    asset_id: view.asset_id,
                    image: view.image.clone(),
                    image_url: view.image_url.clone(),
                    animation_url: view.animation_url.clone(),
                    width: view.width,
                    height: view.height,
                };
                let text =
                    StampText { name: view.name.clone(), emoji: view.emoji.clone(), keywords: view.keywords.clone() };
                let copy_id =
                    stamps::insert(&mut tx, StampHome::Personal { owner_id: user_id }, &source, &text, position)
                        .await
                        // The picture was taken down while the caller was looking at it.
                        .map_err(|e| if e.reason == "ASSET_NOT_FOUND" { stamps::not_found() } else { e })?;
                tx.commit().await?;
                Ok(Saved { outcome: SaveOutcome::SAVE_OUTCOME_SAVED_AS_PERSONAL, stamp_id: copy_id, pack: None })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::status;

    fn stamp(owner: Option<Uuid>, pack: Option<(i32, i32, Uuid)>) -> StampView {
        StampView {
            id: Uuid::now_v7(),
            pack_id: pack.map(|_| Uuid::now_v7()),
            owner_id: owner,
            name: String::new(),
            emoji: String::new(),
            keywords: vec![],
            format: 1,
            image: serde_json::json!({}),
            image_url: "https://media.test/a.webp".into(),
            animation_url: String::new(),
            width: 0,
            height: 0,
            position: 0,
            approved_at: pack.map(|_| now()),
            asset_id: Some(Uuid::now_v7()),
            send_count: 0,
            created_at: now(),
            pack_status: pack.map(|(pack_status, _, _)| pack_status),
            pack_price: pack.map(|(_, price, _)| price),
            pack_creator_id: pack.map(|(_, _, creator)| creator),
            pack_official: false,
            installed: false,
            owned: false,
            favorited: false,
        }
    }

    #[test]
    fn personal_stamps_are_copied_unless_they_are_your_own() {
        let (alice, bob) = (Uuid::now_v7(), Uuid::now_v7());
        let hers = stamp(Some(alice), None);
        assert_eq!(plan_save(&hers, &Viewer::user(bob)), SavePlan::CopyToPersonal);
        assert_eq!(plan_save(&hers, &Viewer::user(alice)), SavePlan::AlreadyInLibrary);
        assert_eq!(plan_save(&hers, &Viewer::default()), SavePlan::NotSavable);
    }

    #[test]
    fn pack_stamps_are_saved_by_getting_the_pack() {
        let (creator, user) = (Uuid::now_v7(), Viewer::user(Uuid::now_v7()));
        let free = stamp(None, Some((status::PUBLISHED, 0, creator)));
        assert_eq!(plan_save(&free, &user), SavePlan::InstallPack);
        assert_eq!(plan_save(&StampView { installed: true, ..free.clone() }, &user), SavePlan::AlreadyInLibrary);

        let paid = stamp(None, Some((status::PUBLISHED, 120, creator)));
        assert_eq!(plan_save(&paid, &user), SavePlan::PurchaseRequired, "never copied out of a paid pack");
        assert_eq!(
            plan_save(&StampView { owned: true, ..paid }, &user),
            SavePlan::InstallPack,
            "bought but uninstalled"
        );

        let draft = stamp(None, Some((status::DRAFT, 0, creator)));
        assert_eq!(plan_save(&draft, &user), SavePlan::NotSavable);
        assert_eq!(
            plan_save(&draft, &Viewer::user(creator)),
            SavePlan::InstallPack,
            "the creator can put their draft in the picker"
        );
    }
}
