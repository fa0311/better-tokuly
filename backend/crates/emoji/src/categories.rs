//! The managed list of categories. Moderators create, rename, order and
//! delete them; an emoji names one of them or none. Every change to the
//! list is a change to the catalogue (clients show the categories of their
//! copy), and so is every emoji that changes category along the way.
use tokuly_core::prelude::*;

use crate::{
    emojis::{self, Effects, Tx},
    validate::{self, MAX_CATEGORIES},
};

fn not_found() -> AppError {
    AppError::not_found("CATEGORY_NOT_FOUND", "there is no such category")
}

/// Makes changes to the list queue up for the rest of the transaction:
/// positions and the size limit are computed from the whole list.
async fn serialize(tx: &mut Tx<'_>) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('emoji-categories', 0))").execute(&mut **tx).await?;
    Ok(())
}

/// The stored name of the category a client named (matched without regard
/// to case), locked against deletion until the transaction ends.
async fn find(tx: &mut Tx<'_>, name: &str) -> AppResult<Option<String>> {
    Ok(sqlx::query_scalar("SELECT name FROM emoji_categories WHERE lower(name) = lower($1) FOR KEY SHARE")
        .bind(name.trim())
        .fetch_optional(&mut **tx)
        .await?)
}

/// Resolves the category of an emoji being stored: empty means none
/// (`None`), anything else must be on the list (`CATEGORY_NOT_FOUND`).
pub async fn resolve(tx: &mut Tx<'_>, name: &str) -> AppResult<Option<String>> {
    if name.trim().is_empty() {
        return Ok(None);
    }
    find(tx, name).await?.ok_or_else(not_found).map(Some)
}

/// Approved emoji of a category change with it: they get the next
/// catalogue version and a fresh search document.
async fn touch_emoji(tx: &mut Tx<'_>, fx: &mut Effects, emoji_ids: &[Uuid]) -> AppResult<()> {
    for emoji_id in emoji_ids {
        fx.catalog.upsert(*emoji_id);
    }
    emojis::reindex(tx, emoji_ids).await
}

async fn approved_in(tx: &mut Tx<'_>, category: &str) -> AppResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar("SELECT id FROM emojis WHERE category = $1 AND state = 2")
        .bind(category)
        .fetch_all(&mut **tx)
        .await?)
}

async fn insert(tx: &mut Tx<'_>, fx: &mut Effects, name: &str) -> AppResult<Option<String>> {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM emoji_categories").fetch_one(&mut **tx).await?;
    ensure!(
        count < MAX_CATEGORIES,
        rate_limited,
        "TOO_MANY_CATEGORIES",
        "there can be at most {MAX_CATEGORIES} categories"
    );
    let created: Option<String> = sqlx::query_scalar(
        "INSERT INTO emoji_categories (name, position)
         SELECT $1, COALESCE(max(position), 0) + 1 FROM emoji_categories
         ON CONFLICT DO NOTHING
         RETURNING name",
    )
    .bind(name)
    .fetch_optional(&mut **tx)
    .await?;
    if created.is_some() {
        fx.catalog.categories_changed();
    }
    Ok(created)
}

/// Adds a category at the end of the list.
/// Errors: `CATEGORY_NAME_INVALID`, `CATEGORY_EXISTS`, `TOO_MANY_CATEGORIES`.
pub async fn create(tx: &mut Tx<'_>, fx: &mut Effects, name: &str) -> AppResult<String> {
    let name = validate::category_name(name)?;
    serialize(tx).await?;
    insert(tx, fx, &name)
        .await?
        .ok_or_else(|| AppError::already_exists("CATEGORY_EXISTS", "there already is a category with this name"))
}

/// The stored name of a category, creating it when it is not on the list
/// (imports bring their categories along). `None` when the name cannot be
/// a category or the list is full: the emoji then goes without one.
pub async fn ensure(tx: &mut Tx<'_>, fx: &mut Effects, name: &str) -> AppResult<Option<String>> {
    let Some(name) = validate::normalize_category(name) else { return Ok(None) };
    serialize(tx).await?;
    if let Some(existing) = find(tx, &name).await? {
        return Ok(Some(existing));
    }
    match insert(tx, fx, &name).await {
        Ok(created) => Ok(created),
        Err(e) if e.reason == "TOO_MANY_CATEGORIES" => Ok(None),
        Err(e) => Err(e),
    }
}

/// Renames a category; its emoji follow (the foreign key cascades).
/// Errors: `CATEGORY_NOT_FOUND`, `CATEGORY_NAME_INVALID`, `CATEGORY_EXISTS`.
pub async fn rename(tx: &mut Tx<'_>, fx: &mut Effects, name: &str, new_name: &str) -> AppResult<()> {
    let new_name = validate::category_name(new_name)?;
    serialize(tx).await?;
    let current = find(tx, name).await?.ok_or_else(not_found)?;
    if current == new_name {
        return Ok(());
    }
    // Changing only the case of a name is a rename of the same category.
    if current.to_lowercase() != new_name.to_lowercase() {
        ensure!(
            find(tx, &new_name).await?.is_none(),
            already_exists,
            "CATEGORY_EXISTS",
            "there already is a category with this name"
        );
    }
    sqlx::query("UPDATE emoji_categories SET name = $2 WHERE name = $1")
        .bind(&current)
        .bind(&new_name)
        .execute(&mut **tx)
        .await?;
    let affected = approved_in(tx, &new_name).await?;
    touch_emoji(tx, fx, &affected).await?;
    fx.catalog.categories_changed();
    Ok(())
}

/// Sets the display order. `names` must be exactly the existing categories
/// (`ORDER_MISMATCH`).
pub async fn reorder(tx: &mut Tx<'_>, fx: &mut Effects, names: &[String]) -> AppResult<()> {
    serialize(tx).await?;
    let existing: Vec<String> = sqlx::query_scalar("SELECT name FROM emoji_categories").fetch_all(&mut **tx).await?;
    let mut ordered: Vec<String> = Vec::with_capacity(names.len());
    for name in names {
        let wanted = name.trim().to_lowercase();
        let stored = existing.iter().find(|stored| stored.to_lowercase() == wanted);
        match stored {
            Some(stored) if !ordered.contains(stored) => ordered.push(stored.clone()),
            _ => bail!(invalid, "ORDER_MISMATCH", "the list must name every category exactly once"),
        }
    }
    ensure!(
        ordered.len() == existing.len(),
        invalid,
        "ORDER_MISMATCH",
        "the list must name every category exactly once"
    );
    sqlx::query(
        "UPDATE emoji_categories c SET position = listed.ord
         FROM unnest($1::text[]) WITH ORDINALITY AS listed (name, ord)
         WHERE c.name = listed.name",
    )
    .bind(&ordered)
    .execute(&mut **tx)
    .await?;
    fx.catalog.categories_changed();
    Ok(())
}

/// Deletes a category. Its emoji move to `move_to`, or lose their category
/// when that is empty. Errors: `CATEGORY_NOT_FOUND` (also for `move_to`,
/// and when it names the category being deleted).
pub async fn delete(tx: &mut Tx<'_>, fx: &mut Effects, name: &str, move_to: &str) -> AppResult<()> {
    serialize(tx).await?;
    let current = find(tx, name).await?.ok_or_else(not_found)?;
    let target = resolve(tx, move_to).await?;
    ensure!(
        target.as_ref() != Some(&current),
        not_found,
        "CATEGORY_NOT_FOUND",
        "the emoji cannot move to the category being deleted"
    );
    let affected = approved_in(tx, &current).await?;
    sqlx::query("UPDATE emojis SET category = $2 WHERE category = $1")
        .bind(&current)
        .bind(&target)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM emoji_categories WHERE name = $1").bind(&current).execute(&mut **tx).await?;
    touch_emoji(tx, fx, &affected).await?;
    fx.catalog.categories_changed();
    Ok(())
}
