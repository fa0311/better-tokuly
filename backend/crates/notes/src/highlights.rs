//! Highlights: titled groups of the owner's stories that stay on their
//! profile after the stories expired.
//!
//! A highlight is only as visible as the stories in it: what a viewer gets
//! (the stories, their number, the cover) is computed from the stories that
//! viewer may see, and a highlight with none of those does not exist for
//! them.
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use tokuly_core::prelude::*;
use tokuly_proto::notes::{Highlight, Story};

use crate::{
    Notes,
    model::{STORY_COLUMNS, StoryRow},
    text,
    visibility::{Viewer, sees_story},
};

pub const MAX_TITLE_CHARS: usize = 30;
/// Most stories in one highlight.
pub const MAX_STORIES: usize = 100;
/// Most highlights on one profile.
pub const MAX_HIGHLIGHTS: i64 = 50;

/// A highlight with what one viewer sees of it.
#[derive(sqlx::FromRow)]
struct HighlightView {
    id: Uuid,
    owner_id: Uuid,
    title: String,
    updated_at: DateTime<Utc>,
    story_count: i32,
    cover_story_id: Uuid,
}

fn validate_title(input: &str) -> AppResult<String> {
    let title = text::clean(input).split_whitespace().collect::<Vec<_>>().join(" ");
    ensure!(
        !title.is_empty() && text::fits(&title, MAX_TITLE_CHARS),
        invalid,
        "TITLE_INVALID",
        "a highlight title has 1-{MAX_TITLE_CHARS} characters"
    );
    Ok(title)
}

/// The stories of a highlight in playing order, each once.
fn validate_stories(story_ids: &[Uuid]) -> AppResult<Vec<Uuid>> {
    let mut unique = Vec::with_capacity(story_ids.len());
    for id in story_ids {
        if !unique.contains(id) {
            unique.push(*id);
        }
    }
    ensure!(!unique.is_empty(), invalid, "HIGHLIGHT_EMPTY", "a highlight needs at least one story");
    ensure!(
        unique.len() <= MAX_STORIES,
        invalid,
        "TOO_MANY_STORIES",
        "a highlight holds at most {MAX_STORIES} stories"
    );
    Ok(unique)
}

/// Serializes changes to one user's set of highlights (their number and order).
async fn lock_highlights(tx: &mut Transaction<'_, Postgres>, owner: Uuid) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("notes:highlights:{owner}"))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Replaces the stories of a highlight. Every story must be `owner`'s own.
async fn set_stories(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
    highlight_id: Uuid,
    story_ids: &[Uuid],
) -> AppResult<()> {
    let owned: i64 = sqlx::query_scalar("SELECT count(*) FROM stories WHERE id = ANY($1) AND owner_id = $2")
        .bind(story_ids)
        .bind(owner)
        .fetch_one(&mut **tx)
        .await?;
    ensure!(owned == story_ids.len() as i64, not_found, "STORY_NOT_FOUND", "a highlight is made of your own stories");
    sqlx::query("DELETE FROM story_highlight_items WHERE highlight_id = $1")
        .bind(highlight_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "INSERT INTO story_highlight_items (highlight_id, story_id, position)
         SELECT $1, t.story_id, t.position::int FROM unnest($2::uuid[]) WITH ORDINALITY AS t(story_id, position)",
    )
    .bind(highlight_id)
    .bind(story_ids)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

impl Notes {
    /// Highlights matching `scope` (a condition on `h` using `$7`) that hold
    /// at least one story the viewer may see, in profile order.
    async fn highlight_views(&self, viewer: &Viewer, scope: &str, about: Uuid) -> AppResult<Vec<Highlight>> {
        let sql = format!(
            "SELECT h.id, h.owner_id, h.title, h.updated_at, count(*)::int AS story_count,
                    (array_agg(s.id ORDER BY i.position, s.id))[1] AS cover_story_id
             FROM story_highlights h
             JOIN story_highlight_items i ON i.highlight_id = h.id
             JOIN stories s ON s.id = i.story_id
             WHERE {scope} AND {visible}
             GROUP BY h.id
             ORDER BY h.position, h.id",
            visible = sees_story("s"),
        );
        let views: Vec<HighlightView> = viewer.query_as(sql).bind(about).fetch_all(&self.core.db).await?;
        let cover_ids: Vec<Uuid> = views.iter().map(|view| view.cover_story_id).collect();
        let sql = format!("SELECT {STORY_COLUMNS} FROM stories s WHERE s.id = ANY($1)");
        let covers: Vec<StoryRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(&cover_ids).fetch_all(&self.core.db).await?;
        let covers: HashMap<Uuid, StoryRow> = covers.into_iter().map(|row| (row.id, row)).collect();
        Ok(views
            .into_iter()
            .map(|view| Highlight {
                id: view.id.to_string(),
                owner_id: view.owner_id.to_string(),
                title: view.title,
                cover: covers.get(&view.cover_story_id).and_then(StoryRow::cover).into(),
                story_count: view.story_count,
                updated_at: ts(view.updated_at),
                ..Default::default()
            })
            .collect())
    }

    /// The highlight as the viewer sees it, or `HIGHLIGHT_NOT_FOUND` when
    /// it does not exist or shows them nothing.
    async fn require_highlight(&self, viewer: &Viewer, highlight_id: Uuid) -> AppResult<Highlight> {
        self.highlight_views(viewer, "h.id = $7", highlight_id)
            .await?
            .pop()
            .or_not_found("HIGHLIGHT_NOT_FOUND", "highlight not found")
    }

    /// The highlights on a user's profile that the viewer can see into.
    pub(crate) async fn list_highlights(&self, viewer: &Viewer, user_id: Uuid) -> AppResult<Vec<Highlight>> {
        self.highlight_views(viewer, "h.owner_id = $7", user_id).await
    }

    /// One highlight and the stories in it that the viewer may see, in playing order.
    pub(crate) async fn get_highlight(
        &self,
        viewer: &Viewer,
        highlight_id: Uuid,
    ) -> AppResult<(Highlight, Vec<Story>)> {
        let highlight = self.require_highlight(viewer, highlight_id).await?;
        let sql = format!(
            "SELECT {STORY_COLUMNS} FROM story_highlight_items i JOIN stories s ON s.id = i.story_id
             WHERE i.highlight_id = $7 AND {}
             ORDER BY i.position, s.id",
            sees_story("s")
        );
        let rows: Vec<StoryRow> = viewer.query_as(sql).bind(highlight_id).fetch_all(&self.core.db).await?;
        Ok((highlight, self.stories_to_proto(viewer, &rows).await?))
    }

    /// Creates a highlight at the end of `owner`'s profile row.
    pub(crate) async fn create_highlight(
        &self,
        viewer: &Viewer,
        owner: Uuid,
        title: &str,
        story_ids: &[Uuid],
    ) -> AppResult<Highlight> {
        let title = validate_title(title)?;
        let story_ids = validate_stories(story_ids)?;
        let mut tx = self.core.db.begin().await?;
        lock_highlights(&mut tx, owner).await?;
        let (count, last): (i64, Option<i32>) =
            sqlx::query_as("SELECT count(*), max(position) FROM story_highlights WHERE owner_id = $1")
                .bind(owner)
                .fetch_one(&mut *tx)
                .await?;
        ensure!(
            count < MAX_HIGHLIGHTS,
            precondition,
            "TOO_MANY_HIGHLIGHTS",
            "a profile holds at most {MAX_HIGHLIGHTS} highlights"
        );
        let highlight_id: Uuid = sqlx::query_scalar(
            "INSERT INTO story_highlights (owner_id, title, position) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(owner)
        .bind(&title)
        .bind(last.map_or(0, |last| last + 1))
        .fetch_one(&mut *tx)
        .await?;
        set_stories(&mut tx, owner, highlight_id, &story_ids).await?;
        tx.commit().await?;
        self.require_highlight(viewer, highlight_id).await
    }

    /// Replaces the title and the stories of one of `owner`'s highlights.
    pub(crate) async fn update_highlight(
        &self,
        viewer: &Viewer,
        owner: Uuid,
        highlight_id: Uuid,
        title: &str,
        story_ids: &[Uuid],
    ) -> AppResult<Highlight> {
        let title = validate_title(title)?;
        let story_ids = validate_stories(story_ids)?;
        let mut tx = self.core.db.begin().await?;
        // Writing the title also stamps `updated_at` (see the migration).
        let updated = sqlx::query("UPDATE story_highlights SET title = $3 WHERE id = $1 AND owner_id = $2")
            .bind(highlight_id)
            .bind(owner)
            .bind(&title)
            .execute(&mut *tx)
            .await?;
        ensure!(updated.rows_affected() > 0, not_found, "HIGHLIGHT_NOT_FOUND", "highlight not found");
        set_stories(&mut tx, owner, highlight_id, &story_ids).await?;
        tx.commit().await?;
        self.require_highlight(viewer, highlight_id).await
    }

    /// Removes one of `owner`'s highlights. Its stories stay in the archive.
    pub(crate) async fn delete_highlight(&self, owner: Uuid, highlight_id: Uuid) -> AppResult<()> {
        let deleted = sqlx::query("DELETE FROM story_highlights WHERE id = $1 AND owner_id = $2")
            .bind(highlight_id)
            .bind(owner)
            .execute(&self.core.db)
            .await?;
        ensure!(deleted.rows_affected() > 0, not_found, "HIGHLIGHT_NOT_FOUND", "highlight not found");
        Ok(())
    }

    /// Sets the order of `owner`'s highlights; `ordered` must name each of
    /// them exactly once.
    pub(crate) async fn reorder_highlights(&self, owner: Uuid, ordered: &[Uuid]) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        lock_highlights(&mut tx, owner).await?;
        let mut current: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM story_highlights WHERE owner_id = $1")
            .bind(owner)
            .fetch_all(&mut *tx)
            .await?;
        let mut requested = ordered.to_vec();
        current.sort();
        requested.sort();
        ensure!(current == requested, invalid, "HIGHLIGHTS_MISMATCH", "name each of your highlights exactly once");
        sqlx::query(
            "UPDATE story_highlights h SET position = t.position::int
             FROM unnest($1::uuid[]) WITH ORDINALITY AS t(id, position) WHERE h.id = t.id AND h.owner_id = $2",
        )
        .bind(ordered)
        .bind(owner)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_are_one_to_thirty_characters() {
        assert_eq!(validate_title("  夏の\n思い出 ").unwrap(), "夏の 思い出");
        assert_eq!(validate_title(&"あ".repeat(MAX_TITLE_CHARS)).unwrap().chars().count(), MAX_TITLE_CHARS);
        for bad in ["", "   ", &"あ".repeat(MAX_TITLE_CHARS + 1)] {
            assert_eq!(validate_title(bad).unwrap_err().reason, "TITLE_INVALID");
        }
    }

    #[test]
    fn stories_are_listed_once_and_bounded() {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        assert_eq!(validate_stories(&[b, a, b]).unwrap(), vec![b, a]);
        assert_eq!(validate_stories(&[]).unwrap_err().reason, "HIGHLIGHT_EMPTY");
        let many: Vec<Uuid> = (0..=MAX_STORIES).map(|_| Uuid::now_v7()).collect();
        assert_eq!(validate_stories(&many).unwrap_err().reason, "TOO_MANY_STORIES");
    }
}
