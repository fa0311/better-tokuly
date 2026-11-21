//! The stamp lists of a user's library: favourites, recently used, the
//! sending history, and search across everything they can send.
//!
//! These lists point at stamps the user may since have lost (a pack was
//! uninstalled, unpublished or taken down). Rows are kept — the stamp may
//! come back — but every read filters to what the user can send right now.
use sqlx::PgExecutor;
use tokuly_core::prelude::*;
use tokuly_proto::{common::PageInfo, stamp as pb};

use crate::{
    access::Viewer,
    model::{StampView, stamp_view_columns, stamp_view_from},
    stamps,
    usage::RECENTS_KEPT,
    validate,
};

/// Favourites one user may keep.
pub const MAX_FAVORITES: i64 = 500;

/// A stamp with the timestamp its list is sorted by.
#[derive(sqlx::FromRow)]
struct Dated {
    #[sqlx(flatten)]
    view: StampView,
    sort_at: Time,
}

/// Marks a stamp as a favourite. Only stamps the user may send qualify:
/// a favourite is a shortcut in the picker.
pub async fn favorite(core: &Core, viewer: &Viewer, user_id: Uuid, stamp_id: Uuid) -> AppResult<()> {
    let view = stamps::view(core, Some(user_id), stamp_id)
        .await?
        .filter(|v| v.visible_to(viewer))
        .ok_or_else(stamps::not_found)?;
    ensure!(
        view.usable_by(viewer),
        precondition,
        "STAMP_NOT_USABLE",
        "install or buy the stamp's pack to favourite it"
    );
    if view.favorited {
        return Ok(());
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM stamp_favorites WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(&core.db)
        .await?;
    ensure!(
        count < MAX_FAVORITES,
        precondition,
        "FAVORITE_LIMIT_REACHED",
        "you can keep at most {MAX_FAVORITES} favourites"
    );
    sqlx::query("INSERT INTO stamp_favorites (user_id, stamp_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(user_id)
        .bind(stamp_id)
        .execute(&core.db)
        .await?;
    Ok(())
}

pub async fn unfavorite(core: &Core, user_id: Uuid, stamp_id: Uuid) -> AppResult<()> {
    sqlx::query("DELETE FROM stamp_favorites WHERE user_id = $1 AND stamp_id = $2")
        .bind(user_id)
        .bind(stamp_id)
        .execute(&core.db)
        .await?;
    Ok(())
}

/// One page of favourites, most recently added first.
pub async fn favorites(
    core: &Core,
    viewer: &Viewer,
    user_id: Uuid,
    page: &Page,
) -> AppResult<(Vec<StampView>, PageInfo)> {
    let cursor: Option<(Time, Uuid)> = page.decode()?;
    let (before_at, before_id) = cursor.unzip();
    let mut rows: Vec<Dated> = sqlx::query_as(concat!(
        "SELECT ",
        stamp_view_columns!(),
        ", fav.created_at AS sort_at",
        stamp_view_from!(),
        "JOIN stamp_favorites fav ON fav.stamp_id = s.id AND fav.user_id = $1
         WHERE ($2::timestamptz IS NULL OR (fav.created_at, s.id) < ($2, $3))
         ORDER BY fav.created_at DESC, s.id DESC
         LIMIT $4"
    ))
    .bind(user_id)
    .bind(before_at)
    .bind(before_id)
    .bind(page.fetch())
    .fetch_all(&core.db)
    .await?;
    let info = page.finish(&mut rows, |row| (row.sort_at, row.view.id));
    Ok((usable(rows.into_iter().map(|row| row.view), viewer), info))
}

/// Keeps the stamps `viewer` can send right now.
fn usable(views: impl IntoIterator<Item = StampView>, viewer: &Viewer) -> Vec<StampView> {
    views.into_iter().filter(|view| view.usable_by(viewer)).collect()
}

/// The user's recently used stamps, by last use or by number of uses.
pub async fn recents(
    core: &Core,
    viewer: &Viewer,
    user_id: Uuid,
    order: pb::RecentOrder,
    limit: usize,
) -> AppResult<Vec<StampView>> {
    // Both variants read the whole (capped) list, so stamps the user lost do
    // not leave holes in a short result.
    let sql = match order {
        pb::RecentOrder::RECENT_ORDER_MOST_USED => concat!(
            "SELECT ",
            stamp_view_columns!(),
            stamp_view_from!(),
            "JOIN stamp_recents r ON r.stamp_id = s.id AND r.user_id = $1
             ORDER BY r.use_count DESC, r.used_at DESC LIMIT $2"
        ),
        pb::RecentOrder::RECENT_ORDER_LAST_USED | pb::RecentOrder::RECENT_ORDER_UNSPECIFIED => concat!(
            "SELECT ",
            stamp_view_columns!(),
            stamp_view_from!(),
            "JOIN stamp_recents r ON r.stamp_id = s.id AND r.user_id = $1
             ORDER BY r.used_at DESC LIMIT $2"
        ),
    };
    let rows: Vec<StampView> = sqlx::query_as(sql).bind(user_id).bind(RECENTS_KEPT).fetch_all(&core.db).await?;
    let mut stamps = usable(rows, viewer);
    stamps.truncate(limit);
    Ok(stamps)
}

/// Forgets one recently used stamp, or all of them.
pub async fn clear_recents<'e>(executor: impl PgExecutor<'e>, user_id: Uuid, stamp_id: Option<Uuid>) -> AppResult<()> {
    sqlx::query("DELETE FROM stamp_recents WHERE user_id = $1 AND ($2::uuid IS NULL OR stamp_id = $2)")
        .bind(user_id)
        .bind(stamp_id)
        .execute(executor)
        .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct UsageRow {
    #[sqlx(flatten)]
    view: StampView,
    usage_id: Uuid,
    usage_context: i32,
    sent_at: Time,
}

/// One page of the user's sending history, newest first. Sends of stamps
/// that no longer exist for the user are left out.
pub async fn usage_log(
    core: &Core,
    viewer: &Viewer,
    user_id: Uuid,
    page: &Page,
) -> AppResult<(Vec<pb::UsageEntry>, PageInfo)> {
    let before: Option<Uuid> = page.decode()?;
    let mut rows: Vec<UsageRow> = sqlx::query_as(concat!(
        "SELECT ",
        stamp_view_columns!(),
        ", u.id AS usage_id, u.context AS usage_context, u.created_at AS sent_at
         FROM stamp_usage u JOIN stamps s ON s.id = u.stamp_id LEFT JOIN stamp_packs p ON p.id = s.pack_id
         WHERE u.user_id = $1 AND ($2::uuid IS NULL OR u.id < $2)
         ORDER BY u.id DESC
         LIMIT $3"
    ))
    .bind(user_id)
    .bind(before)
    .bind(page.fetch())
    .fetch_all(&core.db)
    .await?;
    let info = page.finish(&mut rows, |row| row.usage_id);
    let entries = rows
        .into_iter()
        .filter(|row| row.view.visible_to(viewer))
        .map(|row| pb::UsageEntry {
            id: row.usage_id.to_string(),
            stamp: row.view.to_proto(viewer).into(),
            context: row.usage_context.into(),
            sent_at: ts(row.sent_at),
            ..Default::default()
        })
        .collect();
    Ok((entries, info))
}

/// Stamps the user can send whose name, keywords or emoji match `query`,
/// most recently used first.
///
/// Candidates are the user's personal stamps and the stamps of packs they
/// installed or made. Names match by substring, keywords by prefix (so
/// typing "あり" finds "ありがとう"), the emoji exactly.
pub async fn search(
    core: &Core,
    viewer: &Viewer,
    user_id: Uuid,
    query: &str,
    limit: usize,
) -> AppResult<Vec<StampView>> {
    let name_pattern = format!("%{}%", validate::like_escape(query));
    let word = validate::normalize_word(query);
    let keyword_prefix = word.as_deref().map(|word| format!("{}%", validate::like_escape(word)));
    // Fetch more than asked for: some candidates may not be sendable (e.g. unreviewed stamps).
    let fetch = (limit * 2) as i64;
    let rows: Vec<StampView> = sqlx::query_as(concat!(
        "SELECT ",
        stamp_view_columns!(),
        stamp_view_from!(),
        "LEFT JOIN stamp_recents r ON r.stamp_id = s.id AND r.user_id = $1
         WHERE (s.owner_id = $1
                OR p.creator_id = $1
                OR s.pack_id IN (SELECT pack_id FROM stamp_pack_installs WHERE user_id = $1))
           AND (s.name ILIKE $2
                OR s.emoji = $3
                OR EXISTS (SELECT 1 FROM unnest(s.keywords) AS keyword WHERE keyword LIKE $4))
         ORDER BY r.used_at DESC NULLS LAST, s.send_count DESC, s.id DESC
         LIMIT $5"
    ))
    .bind(user_id)
    .bind(name_pattern)
    .bind(query)
    .bind(keyword_prefix)
    .bind(fetch)
    .fetch_all(&core.db)
    .await?;
    let mut stamps = usable(rows, viewer);
    stamps.truncate(limit);
    Ok(stamps)
}
