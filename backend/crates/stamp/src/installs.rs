//! Installed packs: what is in a user's picker, and in which order.
use std::collections::HashSet;

use sqlx::{PgExecutor, Postgres, Transaction};
use tokuly_core::{events, prelude::*};
use tokuly_proto::common::PageInfo;

use crate::{
    events::{PACK_INSTALLED, PACK_UNINSTALLED, PackInstall},
    model::{PackRow, pack_columns},
    stats,
};

/// Packs one user may have installed. Far beyond what a picker can show;
/// the cap only keeps a library from growing without bound.
pub const MAX_INSTALLED: i64 = 300;

/// Fails with `INSTALL_LIMIT_REACHED` when the user's library is full and
/// the pack is not already in it.
pub async fn ensure_room<'e>(executor: impl PgExecutor<'e>, user_id: Uuid, pack_id: Uuid) -> AppResult<()> {
    let (already_installed, installed_count): (bool, i64) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM stamp_pack_installs WHERE user_id = $1 AND pack_id = $2),
                (SELECT count(*) FROM stamp_pack_installs WHERE user_id = $1)",
    )
    .bind(user_id)
    .bind(pack_id)
    .fetch_one(executor)
    .await?;
    ensure!(
        already_installed || installed_count < MAX_INSTALLED,
        precondition,
        "INSTALL_LIMIT_REACHED",
        "you can install at most {MAX_INSTALLED} packs; remove one first"
    );
    Ok(())
}

/// Adds a pack to the front of the user's library and returns whether it
/// was newly added. The caller has already decided the user may have it and
/// (see [`ensure_room`]) has room for it.
///
/// `counted` says whether the install shows up in the pack's numbers: the
/// people who manage a pack are not its audience.
pub async fn install(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    pack: &PackRow,
    counted: bool,
) -> AppResult<bool> {
    // New packs go first: one position before the current front.
    let inserted = sqlx::query(
        "INSERT INTO stamp_pack_installs (user_id, pack_id, position)
         VALUES ($1, $2, (SELECT COALESCE(min(position), 0) - 1 FROM stamp_pack_installs WHERE user_id = $1))
         ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(pack.id)
    .execute(&mut **tx)
    .await?
    .rows_affected()
        == 1;
    if inserted && counted {
        sqlx::query("UPDATE stamp_packs SET install_count = install_count + 1 WHERE id = $1")
            .bind(pack.id)
            .execute(&mut **tx)
            .await?;
        stats::bump_daily(tx, pack.id, 1, 0, 0).await?;
        events::emit(&mut **tx, PACK_INSTALLED, Some(user_id), &PackInstall { pack_id: pack.id, user_id }).await?;
    }
    Ok(inserted)
}

/// Removes a pack from the user's library; returns whether it was there.
pub async fn uninstall(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    pack: &PackRow,
    counted: bool,
) -> AppResult<bool> {
    let removed = sqlx::query("DELETE FROM stamp_pack_installs WHERE user_id = $1 AND pack_id = $2")
        .bind(user_id)
        .bind(pack.id)
        .execute(&mut **tx)
        .await?
        .rows_affected()
        == 1;
    if removed && counted {
        sqlx::query("UPDATE stamp_packs SET install_count = GREATEST(install_count - 1, 0) WHERE id = $1")
            .bind(pack.id)
            .execute(&mut **tx)
            .await?;
        // An install undone the same day does not count for that day, so
        // installing and uninstalling in a loop cannot push a pack up the ranking.
        stats::retract_daily_install(tx, pack.id).await?;
        events::emit(&mut **tx, PACK_UNINSTALLED, Some(user_id), &PackInstall { pack_id: pack.id, user_id }).await?;
    }
    Ok(removed)
}

#[derive(sqlx::FromRow)]
struct InstalledRow {
    #[sqlx(flatten)]
    pack: PackRow,
    install_position: i32,
}

/// One page of the user's installed packs in their order. Packs that were
/// taken down are left out (except for their own creator, who has to see
/// what happened).
pub async fn list(core: &Core, user_id: Uuid, page: &Page) -> AppResult<(Vec<PackRow>, PageInfo)> {
    let cursor: Option<(i32, Uuid)> = page.decode()?;
    let (after_position, after_id) = cursor.unzip();
    let mut rows: Vec<InstalledRow> = sqlx::query_as(concat!(
        "SELECT ",
        pack_columns!(),
        ", i.position AS install_position
         FROM stamp_pack_installs i JOIN stamp_packs p ON p.id = i.pack_id
         WHERE i.user_id = $1 AND (p.status <> 5 OR p.creator_id = $1)
           AND ($2::int IS NULL OR (i.position, i.pack_id) > ($2, $3))
         ORDER BY i.position, i.pack_id
         LIMIT $4"
    ))
    .bind(user_id)
    .bind(after_position)
    .bind(after_id)
    .bind(page.fetch())
    .fetch_all(&core.db)
    .await?;
    let info = page.finish(&mut rows, |row| (row.install_position, row.pack.id));
    Ok((rows.into_iter().map(|row| row.pack).collect(), info))
}

/// The order after moving `first` to the front: the listed packs in the
/// given order, then every other installed pack in its current order.
fn reordered(current: &[Uuid], first: &[Uuid]) -> AppResult<Vec<Uuid>> {
    let installed: HashSet<Uuid> = current.iter().copied().collect();
    let listed: HashSet<Uuid> = first.iter().copied().collect();
    ensure!(
        listed.len() == first.len() && listed.is_subset(&installed),
        invalid,
        "ORDER_MISMATCH",
        "the new order may only list installed packs, each once"
    );
    Ok(first.iter().copied().chain(current.iter().copied().filter(|id| !listed.contains(id))).collect())
}

/// Moves the listed packs to the front of the user's library, in that order.
pub async fn reorder(core: &Core, user_id: Uuid, first: &[Uuid]) -> AppResult<()> {
    let mut tx = core.db.begin().await?;
    let current: Vec<Uuid> = sqlx::query_scalar(
        "SELECT pack_id FROM stamp_pack_installs WHERE user_id = $1 ORDER BY position, pack_id FOR UPDATE",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    let order = reordered(&current, first)?;
    sqlx::query(
        "UPDATE stamp_pack_installs i SET position = (o.ordinal - 1)::int
         FROM unnest($2::uuid[]) WITH ORDINALITY AS o(pack_id, ordinal)
         WHERE i.user_id = $1 AND i.pack_id = o.pack_id",
    )
    .bind(user_id)
    .bind(&order)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listed_packs_move_to_the_front_and_the_rest_keep_their_order() {
        let ids: Vec<Uuid> = (0..5).map(|_| Uuid::now_v7()).collect();
        let order = reordered(&ids, &[ids[3], ids[1]]).unwrap();
        assert_eq!(order, vec![ids[3], ids[1], ids[0], ids[2], ids[4]]);
        assert_eq!(reordered(&ids, &[]).unwrap(), ids, "an empty list changes nothing");
        assert_eq!(
            reordered(&ids, &ids.iter().rev().copied().collect::<Vec<_>>()).unwrap(),
            ids.iter().rev().copied().collect::<Vec<_>>()
        );
    }

    #[test]
    fn unknown_or_repeated_packs_are_rejected() {
        let ids: Vec<Uuid> = (0..3).map(|_| Uuid::now_v7()).collect();
        assert_eq!(reordered(&ids, &[Uuid::now_v7()]).unwrap_err().reason, "ORDER_MISMATCH");
        assert_eq!(reordered(&ids, &[ids[0], ids[0]]).unwrap_err().reason, "ORDER_MISMATCH");
    }
}
