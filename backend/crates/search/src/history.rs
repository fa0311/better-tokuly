//! Per-user search history: recent searches and the pause switch.
use chrono::{DateTime, Utc};
use tokuly_core::prelude::*;

use crate::{Search, text::prefix_upper_bound};

/// Recent searches kept per user.
pub const MAX_RECENT: i64 = 50;

/// One remembered search.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RecentRow {
    pub id: Uuid,
    pub query: String,
    /// Kind name of the tab it was searched in; empty = all kinds.
    pub kind: String,
    pub searched_at: DateTime<Utc>,
}

impl Search {
    /// Whether the user paused their search history.
    pub async fn history_paused(&self, user_id: Uuid) -> AppResult<bool> {
        let paused: Option<bool> = sqlx::query_scalar("SELECT history_paused FROM search_user_settings WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&self.core.db)
            .await?;
        Ok(paused.unwrap_or(false))
    }

    pub(crate) async fn set_history_paused(&self, user_id: Uuid, paused: bool) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO search_user_settings (user_id, history_paused) VALUES ($1, $2)
             ON CONFLICT (user_id) DO UPDATE SET history_paused = EXCLUDED.history_paused",
        )
        .bind(user_id)
        .bind(paused)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    /// Remembers a search unless the user paused their history. Searching
    /// the same words again moves the entry to the top. Returns whether the
    /// history is on, i.e. whether the search was remembered.
    pub(crate) async fn remember_search(&self, user_id: Uuid, query: &str, query_key: &str, kind: &str) -> AppResult<bool> {
        let remembered: i64 = sqlx::query_scalar(
            "WITH remembered AS (
                 INSERT INTO search_recent (user_id, query, query_key, kind)
                 SELECT $1, $2, $3, $4
                 WHERE NOT EXISTS (SELECT 1 FROM search_user_settings WHERE user_id = $1 AND history_paused)
                 ON CONFLICT (user_id, query_key) DO UPDATE SET
                     query = EXCLUDED.query, kind = EXCLUDED.kind, searched_at = now()
                 RETURNING id
             )
             SELECT count(*) FROM remembered",
        )
        .bind(user_id)
        .bind(query)
        .bind(query_key)
        .bind(kind)
        .fetch_one(&self.core.db)
        .await?;
        if remembered == 0 {
            return Ok(false);
        }
        sqlx::query(
            "DELETE FROM search_recent WHERE user_id = $1 AND id IN (
                 SELECT id FROM search_recent WHERE user_id = $1 ORDER BY searched_at DESC, id DESC OFFSET $2
             )",
        )
        .bind(user_id)
        .bind(MAX_RECENT)
        .execute(&self.core.db)
        .await?;
        Ok(true)
    }

    /// The user's recent searches starting with `prefix_key`, newest first.
    pub(crate) async fn recent_completions(&self, user_id: Uuid, prefix_key: &str, limit: i64) -> AppResult<Vec<RecentRow>> {
        if prefix_key.is_empty() {
            return Ok(Vec::new());
        }
        Ok(sqlx::query_as(
            "SELECT id, query, kind, searched_at FROM search_recent
             WHERE user_id = $1 AND query_key >= $2 AND query_key < $3 AND query_key <> $2
             ORDER BY searched_at DESC, id DESC
             LIMIT $4",
        )
        .bind(user_id)
        .bind(prefix_key)
        .bind(prefix_upper_bound(prefix_key))
        .bind(limit)
        .fetch_all(&self.core.db)
        .await?)
    }

    /// One page of the user's recent searches, newest first.
    pub(crate) async fn list_recent(&self, user_id: Uuid, page: &Page) -> AppResult<Vec<RecentRow>> {
        let after: Option<(DateTime<Utc>, Uuid)> = page.decode()?;
        Ok(sqlx::query_as(
            "SELECT id, query, kind, searched_at FROM search_recent
             WHERE user_id = $1 AND ($2::timestamptz IS NULL OR (searched_at, id) < ($2, $3))
             ORDER BY searched_at DESC, id DESC
             LIMIT $4",
        )
        .bind(user_id)
        .bind(after.map(|a| a.0))
        .bind(after.map(|a| a.1))
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?)
    }

    /// Forgets one recent search; `false` when it is not the user's.
    pub(crate) async fn delete_recent(&self, user_id: Uuid, id: Uuid) -> AppResult<bool> {
        let done = sqlx::query("DELETE FROM search_recent WHERE user_id = $1 AND id = $2")
            .bind(user_id)
            .bind(id)
            .execute(&self.core.db)
            .await?;
        Ok(done.rows_affected() > 0)
    }

    /// Forgets every recent search of the user; returns how many there were.
    pub(crate) async fn clear_recent(&self, user_id: Uuid) -> AppResult<u64> {
        let done = sqlx::query("DELETE FROM search_recent WHERE user_id = $1").bind(user_id).execute(&self.core.db).await?;
        Ok(done.rows_affected())
    }
}
