//! Bookmarks: saved subjects, in named collections.
//!
//! Every user has a default collection ("saved"), created the first time it
//! is needed. It holds everything the user saved; the collections they
//! create hold subsets of it. So a subject is saved exactly when it is in
//! the default collection — which is what `subject_stats.bookmark_count`
//! counts — and deleting a collection never unsaves anything.
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use tokuly_core::{events, prelude::*};
use tokuly_proto::{common::PageInfo, engagement::Collection};

use crate::{
    Engagement,
    effects::Effects,
    events::{BOOKMARK_CHANGED, BookmarkChanged},
    stats::{self, Counter},
    subjects::{LooseSubject, Subject, SubjectKey},
    text,
};

/// The stored name of the default collection; clients show their own.
pub const DEFAULT_COLLECTION_NAME: &str = "saved";

/// Bookmark changes one user may make per minute.
const CHANGES_PER_MINUTE: u32 = 120;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct CollectionRow {
    pub id: Uuid,
    pub name: String,
    pub is_default: bool,
    pub item_count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const COLLECTION_COLUMNS: &str = "id, name, is_default, item_count, created_at, updated_at";

impl CollectionRow {
    pub(crate) fn to_proto(&self, contains_subject: bool) -> Collection {
        Collection {
            id: self.id.to_string(),
            name: self.name.clone(),
            is_default: self.is_default,
            item_count: self.item_count,
            created_at: ts(self.created_at),
            updated_at: ts(self.updated_at),
            contains_subject,
            ..Default::default()
        }
    }
}

/// A saved subject in a collection.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct BookmarkRow {
    pub subject_type: i32,
    pub subject_id: Uuid,
    pub created_at: DateTime<Utc>,
}

impl BookmarkRow {
    pub(crate) fn key(&self) -> SubjectKey {
        SubjectKey { kind: self.subject_type, id: self.subject_id }
    }
}

fn collection_not_found() -> AppError {
    AppError::not_found("COLLECTION_NOT_FOUND", "collection not found")
}

/// The user's default collection, created if they have none yet.
async fn default_collection(conn: &mut PgConnection, user_id: Uuid) -> AppResult<CollectionRow> {
    sqlx::query(
        "INSERT INTO bookmark_collections (user_id, name, name_key, is_default) VALUES ($1, $2, $2, true)
         ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(DEFAULT_COLLECTION_NAME)
    .execute(&mut *conn)
    .await?;
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLLECTION_COLUMNS} FROM bookmark_collections WHERE user_id = $1 AND is_default FOR UPDATE"
    )))
    .bind(user_id)
    .fetch_one(conn)
    .await?)
}

/// One of the user's collections, locked for the rest of the transaction.
async fn lock_collection(conn: &mut PgConnection, user_id: Uuid, collection_id: Uuid) -> AppResult<CollectionRow> {
    let row: Option<CollectionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLLECTION_COLUMNS} FROM bookmark_collections WHERE id = $1 AND user_id = $2 FOR UPDATE"
    )))
    .bind(collection_id)
    .bind(user_id)
    .fetch_optional(conn)
    .await?;
    row.ok_or_else(collection_not_found)
}

/// Puts a subject into a collection. Returns whether it was not in it yet.
async fn put(
    conn: &mut PgConnection,
    user_id: Uuid,
    collection: &CollectionRow,
    key: SubjectKey,
    limit: i32,
) -> AppResult<bool> {
    let present: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM bookmarks WHERE collection_id = $1 AND subject_type = $2 AND subject_id = $3)",
    )
    .bind(collection.id)
    .bind(key.kind)
    .bind(key.id)
    .fetch_one(&mut *conn)
    .await?;
    if present {
        return Ok(false);
    }
    ensure!(collection.item_count < limit, precondition, "BOOKMARK_LIMIT", "this collection is full ({limit} items)");
    sqlx::query("INSERT INTO bookmarks (collection_id, user_id, subject_type, subject_id) VALUES ($1, $2, $3, $4)")
        .bind(collection.id)
        .bind(user_id)
        .bind(key.kind)
        .bind(key.id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("UPDATE bookmark_collections SET item_count = item_count + 1 WHERE id = $1")
        .bind(collection.id)
        .execute(conn)
        .await?;
    Ok(true)
}

/// Takes a subject out of the user's collections — out of one, or out of
/// all of them. Returns how many collections it was in.
async fn take(conn: &mut PgConnection, user_id: Uuid, key: SubjectKey, only: Option<Uuid>) -> AppResult<u64> {
    // The collections are locked first, in ID order, like every other
    // change to them.
    let collection_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT c.id FROM bookmark_collections c
         WHERE c.user_id = $1 AND ($4::uuid IS NULL OR c.id = $4)
           AND EXISTS (SELECT 1 FROM bookmarks b WHERE b.collection_id = c.id AND b.subject_type = $2 AND b.subject_id = $3)
         ORDER BY c.id FOR UPDATE OF c",
    )
    .bind(user_id)
    .bind(key.kind)
    .bind(key.id)
    .bind(only)
    .fetch_all(&mut *conn)
    .await?;
    if collection_ids.is_empty() {
        return Ok(0);
    }
    sqlx::query("DELETE FROM bookmarks WHERE collection_id = ANY($1) AND subject_type = $2 AND subject_id = $3")
        .bind(&collection_ids)
        .bind(key.kind)
        .bind(key.id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("UPDATE bookmark_collections SET item_count = GREATEST(item_count - 1, 0) WHERE id = ANY($1)")
        .bind(&collection_ids)
        .execute(conn)
        .await?;
    Ok(collection_ids.len() as u64)
}

impl Engagement {
    async fn bookmark_rate_limit(&self, user_id: Uuid) -> AppResult<()> {
        self.core
            .cache
            .rate_limit(&format!("engagement:bookmark:{user_id}"), CHANGES_PER_MINUTE, Duration::from_secs(60))
            .await
    }

    /// Saves a subject (`BookmarkService.AddBookmark`): into the default
    /// collection, and into `collection_id` when one is named.
    pub(crate) async fn add_bookmark(
        &self,
        user_id: Uuid,
        subject: &Subject,
        collection_id: Option<Uuid>,
    ) -> AppResult<()> {
        self.bookmark_rate_limit(user_id).await?;
        let limit = self.cfg.max_bookmarks;
        let mut tx = self.core.db.begin().await?;
        let mut effects = Effects::default();
        // The default collection is locked first in every change.
        let default = default_collection(&mut tx, user_id).await?;
        let named = match collection_id.filter(|id| *id != default.id) {
            Some(collection_id) => Some(lock_collection(&mut tx, user_id, collection_id).await?),
            None => None,
        };
        if put(&mut tx, user_id, &default, subject.key, limit).await? {
            stats::add(&mut tx, subject.key, Some(subject.info.owner_id), Counter::Bookmark, 1).await?;
            effects.count(subject.key, Counter::Bookmark, 1);
            let event =
                BookmarkChanged { user_id, subject_type: subject.key.kind, subject_id: subject.key.id, saved: true };
            events::emit(&mut *tx, BOOKMARK_CHANGED, Some(user_id), &event).await?;
        }
        if let Some(named) = &named {
            put(&mut tx, user_id, named, subject.key, limit).await?;
        }
        tx.commit().await?;
        self.apply(effects).await;
        Ok(())
    }

    /// Removes a saved subject (`BookmarkService.RemoveBookmark`): from one
    /// collection, or — without one, or with the default one — from
    /// everything. Returns whether the subject is still saved.
    pub(crate) async fn remove_bookmark(
        &self,
        user_id: Uuid,
        subject: &LooseSubject,
        collection_id: Option<Uuid>,
    ) -> AppResult<bool> {
        self.bookmark_rate_limit(user_id).await?;
        let key = subject.key;
        let mut tx = self.core.db.begin().await?;
        let mut effects = Effects::default();
        let default = default_collection(&mut tx, user_id).await?;
        let still_saved = match collection_id.filter(|id| *id != default.id) {
            Some(collection_id) => {
                lock_collection(&mut tx, user_id, collection_id).await?;
                take(&mut tx, user_id, key, Some(collection_id)).await?;
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM bookmarks WHERE collection_id = $1 AND subject_type = $2 AND subject_id = $3)")
                    .bind(default.id)
                    .bind(key.kind)
                    .bind(key.id)
                    .fetch_one(&mut *tx)
                    .await?
            }
            None => {
                if take(&mut tx, user_id, key, None).await? > 0 {
                    let owner_id = subject.subject.as_ref().map(|subject| subject.info.owner_id);
                    stats::add(&mut tx, key, owner_id, Counter::Bookmark, -1).await?;
                    effects.count(key, Counter::Bookmark, -1);
                    let event = BookmarkChanged { user_id, subject_type: key.kind, subject_id: key.id, saved: false };
                    events::emit(&mut *tx, BOOKMARK_CHANGED, Some(user_id), &event).await?;
                }
                false
            }
        };
        tx.commit().await?;
        self.apply(effects).await;
        Ok(still_saved)
    }

    /// Whether `user_id` saved each of `keys`.
    pub(crate) async fn saved_among(&self, user_id: Uuid, keys: &[SubjectKey]) -> AppResult<Vec<SubjectKey>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let (kinds, ids): (Vec<i32>, Vec<Uuid>) = keys.iter().map(|key| (key.kind, key.id)).unzip();
        let rows: Vec<(i32, Uuid)> = sqlx::query_as(
            "SELECT DISTINCT subject_type, subject_id FROM bookmarks
             WHERE user_id = $1 AND (subject_type, subject_id) IN (SELECT * FROM unnest($2::integer[], $3::uuid[]))",
        )
        .bind(user_id)
        .bind(&kinds)
        .bind(&ids)
        .fetch_all(&self.core.db)
        .await?;
        Ok(rows.into_iter().map(|(kind, id)| SubjectKey { kind, id }).collect())
    }

    /// The saved subjects of a collection (`None`: the default one), most
    /// recently saved first, optionally of one subject type (0 = any).
    pub(crate) async fn bookmarks(
        &self,
        user_id: Uuid,
        collection_id: Option<Uuid>,
        subject_type: i32,
        page: &Page,
    ) -> AppResult<(Vec<BookmarkRow>, PageInfo)> {
        let collection: Option<Uuid> = match collection_id {
            Some(collection_id) => {
                sqlx::query_scalar("SELECT id FROM bookmark_collections WHERE id = $1 AND user_id = $2")
                    .bind(collection_id)
                    .bind(user_id)
                    .fetch_optional(&self.core.db)
                    .await?
            }
            None => {
                let default: Option<Uuid> =
                    sqlx::query_scalar("SELECT id FROM bookmark_collections WHERE user_id = $1 AND is_default")
                        .bind(user_id)
                        .fetch_optional(&self.core.db)
                        .await?;
                // Nothing saved yet: an empty list, not an error.
                match default {
                    Some(default) => Some(default),
                    None => return Ok((Vec::new(), PageInfo::default())),
                }
            }
        };
        let collection = collection.ok_or_else(collection_not_found)?;
        let after: Option<(DateTime<Utc>, Uuid)> = page.decode()?;
        let (after_time, after_subject) = after.unzip();
        let mut rows: Vec<BookmarkRow> = sqlx::query_as(
            "SELECT subject_type, subject_id, created_at FROM bookmarks
             WHERE collection_id = $1 AND ($2 = 0 OR subject_type = $2)
               AND ($3::timestamptz IS NULL OR (created_at, subject_id) < ($3, $4))
             ORDER BY created_at DESC, subject_id DESC LIMIT $5",
        )
        .bind(collection)
        .bind(subject_type)
        .bind(after_time)
        .bind(after_subject)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.created_at, row.subject_id));
        Ok((rows, info))
    }

    /// The user's collections — the default one first, then the others by
    /// name — each with whether it holds `subject`.
    pub(crate) async fn collections(
        &self,
        user_id: Uuid,
        subject: Option<SubjectKey>,
    ) -> AppResult<Vec<(CollectionRow, bool)>> {
        // Listing is how clients learn the default collection's ID, so it
        // exists from the first look.
        let mut tx = self.core.db.begin().await?;
        default_collection(&mut tx, user_id).await?;
        tx.commit().await?;
        let rows: Vec<CollectionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLLECTION_COLUMNS} FROM bookmark_collections WHERE user_id = $1 ORDER BY is_default DESC, name_key, id"
        )))
        .bind(user_id)
        .fetch_all(&self.core.db)
        .await?;
        let holding: Vec<Uuid> =
            match subject {
                Some(key) => sqlx::query_scalar(
                    "SELECT collection_id FROM bookmarks WHERE user_id = $1 AND subject_type = $2 AND subject_id = $3",
                )
                .bind(user_id)
                .bind(key.kind)
                .bind(key.id)
                .fetch_all(&self.core.db)
                .await?,
                None => Vec::new(),
            };
        Ok(rows
            .into_iter()
            .map(|row| {
                let holds = holding.contains(&row.id);
                (row, holds)
            })
            .collect())
    }

    /// Creates an empty collection (`BookmarkService.CreateCollection`).
    pub(crate) async fn create_collection(&self, user_id: Uuid, name: &str) -> AppResult<CollectionRow> {
        let (name, name_key) = text::collection_name(name)?;
        self.bookmark_rate_limit(user_id).await?;
        let mut tx = self.core.db.begin().await?;
        // Locking the default collection serializes the count below.
        default_collection(&mut tx, user_id).await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM bookmark_collections WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(
            count < self.cfg.max_collections,
            precondition,
            "COLLECTION_LIMIT",
            "you can have at most {} collections",
            self.cfg.max_collections
        );
        let created: Option<CollectionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO bookmark_collections (user_id, name, name_key) VALUES ($1, $2, $3)
             ON CONFLICT (user_id, name_key) DO NOTHING
             RETURNING {COLLECTION_COLUMNS}"
        )))
        .bind(user_id)
        .bind(&name)
        .bind(&name_key)
        .fetch_optional(&mut *tx)
        .await?;
        let created = created.ok_or_else(|| {
            AppError::already_exists("COLLECTION_NAME_TAKEN", "you already have a collection with this name")
        })?;
        tx.commit().await?;
        Ok(created)
    }

    /// Renames a collection (`BookmarkService.UpdateCollection`).
    pub(crate) async fn rename_collection(
        &self,
        user_id: Uuid,
        collection_id: Uuid,
        name: &str,
    ) -> AppResult<CollectionRow> {
        let mut tx = self.core.db.begin().await?;
        let collection = lock_collection(&mut tx, user_id, collection_id).await?;
        ensure!(!collection.is_default, precondition, "DEFAULT_COLLECTION", "the default collection cannot be renamed");
        let (name, name_key) = text::collection_name(name)?;
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM bookmark_collections WHERE user_id = $1 AND name_key = $2 AND id <> $3)",
        )
        .bind(user_id)
        .bind(&name_key)
        .bind(collection_id)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(!taken, already_exists, "COLLECTION_NAME_TAKEN", "you already have a collection with this name");
        let renamed: CollectionRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE bookmark_collections SET name = $2, name_key = $3 WHERE id = $1 RETURNING {COLLECTION_COLUMNS}"
        )))
        .bind(collection_id)
        .bind(&name)
        .bind(&name_key)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(renamed)
    }

    /// Deletes a collection (`BookmarkService.DeleteCollection`). What it
    /// held stays saved in the default collection.
    pub(crate) async fn delete_collection(&self, user_id: Uuid, collection_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let collection = lock_collection(&mut tx, user_id, collection_id).await?;
        ensure!(!collection.is_default, precondition, "DEFAULT_COLLECTION", "the default collection cannot be deleted");
        sqlx::query("DELETE FROM bookmark_collections WHERE id = $1").bind(collection_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}
