//! The queries behind the comment lists. Each returns rows the viewer may
//! see, one page at a time.
use chrono::{DateTime, Utc};
use sqlx::{Postgres, QueryBuilder};
use tokuly_core::prelude::*;
use tokuly_proto::{common::PageInfo, engagement::CommentSort};

use super::{CommentRow, comment_columns, status};
use crate::{Engagement, subjects::SubjectKey};

/// What the owner's comment manager narrows its list by.
#[derive(Debug, Default)]
pub(crate) struct ReceivedFilter {
    /// A `comments.status` value.
    pub status: Option<i32>,
    pub subject: Option<SubjectKey>,
    /// An `ILIKE` pattern for the text.
    pub pattern: Option<String>,
    /// Only what this account wrote under this identity (`None` = as the user).
    pub identity: Option<(Uuid, Option<Uuid>)>,
    /// `Some(true)`: written as a channel; `Some(false)`: as a user.
    pub as_channel: Option<bool>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
}

impl Engagement {
    /// The pinned comment of a subject, if the viewer may see it.
    pub(crate) async fn pinned_comment(&self, key: SubjectKey, hidden: &[Uuid]) -> AppResult<Option<CommentRow>> {
        Ok(sqlx::query_as(concat!(
            "SELECT ",
            comment_columns!(),
            " FROM comments
             WHERE subject_type = $1 AND subject_id = $2 AND pinned_at IS NOT NULL AND status = 1 AND author_id <> ALL($3)"
        ))
        .bind(key.kind)
        .bind(key.id)
        .bind(hidden)
        .fetch_optional(&self.core.db)
        .await?)
    }

    /// Top-level comments of a subject, without the pinned one: published
    /// ones, plus the viewer's own whatever their status, minus those of
    /// people in a block with the viewer (`hidden`).
    pub(crate) async fn top_level_comments(
        &self,
        key: SubjectKey,
        viewer: Option<Uuid>,
        hidden: &[Uuid],
        sort: CommentSort,
        page: &Page,
    ) -> AppResult<(Vec<CommentRow>, PageInfo)> {
        // No comment is written by the nil user, so a guest matches nothing "own".
        let own = viewer.unwrap_or(Uuid::nil());
        if sort == CommentSort::COMMENT_SORT_NEWEST {
            let after: Option<Uuid> = page.decode()?;
            let mut rows: Vec<CommentRow> = sqlx::query_as(concat!(
                "SELECT ",
                comment_columns!(),
                " FROM comments
                 WHERE subject_type = $1 AND subject_id = $2 AND parent_id IS NULL AND pinned_at IS NULL
                   AND (status = 1 OR author_id = $3) AND author_id <> ALL($4)
                   AND ($5::uuid IS NULL OR id < $5)
                 ORDER BY id DESC LIMIT $6"
            ))
            .bind(key.kind)
            .bind(key.id)
            .bind(own)
            .bind(hidden)
            .bind(after)
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let info = page.finish(&mut rows, |row| row.id);
            return Ok((rows, info));
        }
        let after: Option<(f64, Uuid)> = page.decode()?;
        let (after_score, after_id) = after.unzip();
        let mut rows: Vec<CommentRow> = sqlx::query_as(concat!(
            "SELECT ",
            comment_columns!(),
            " FROM comments
             WHERE subject_type = $1 AND subject_id = $2 AND parent_id IS NULL AND pinned_at IS NULL
               AND (status = 1 OR author_id = $3) AND author_id <> ALL($4)
               AND ($5::float8 IS NULL OR (score, id) < ($5, $6))
             ORDER BY score DESC, id DESC LIMIT $7"
        ))
        .bind(key.kind)
        .bind(key.id)
        .bind(own)
        .bind(hidden)
        .bind(after_score)
        .bind(after_id)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.score, row.id));
        Ok((rows, info))
    }

    /// Replies of a thread, oldest first, under the same rules as
    /// [`Self::top_level_comments`].
    pub(crate) async fn replies(
        &self,
        thread_id: Uuid,
        viewer: Option<Uuid>,
        hidden: &[Uuid],
        page: &Page,
    ) -> AppResult<(Vec<CommentRow>, PageInfo)> {
        let after: Option<Uuid> = page.decode()?;
        let mut rows: Vec<CommentRow> = sqlx::query_as(concat!(
            "SELECT ",
            comment_columns!(),
            " FROM comments
             WHERE parent_id = $1 AND (status = 1 OR author_id = $2) AND author_id <> ALL($3)
               AND ($4::uuid IS NULL OR id > $4)
             ORDER BY id LIMIT $5"
        ))
        .bind(thread_id)
        .bind(viewer.unwrap_or(Uuid::nil()))
        .bind(hidden)
        .bind(after)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| row.id);
        Ok((rows, info))
    }

    /// Everything `user_id` wrote, newest first.
    pub(crate) async fn comments_by(&self, user_id: Uuid, page: &Page) -> AppResult<(Vec<CommentRow>, PageInfo)> {
        let after: Option<Uuid> = page.decode()?;
        let mut rows: Vec<CommentRow> = sqlx::query_as(concat!(
            "SELECT ",
            comment_columns!(),
            " FROM comments WHERE author_id = $1 AND ($2::uuid IS NULL OR id < $2) ORDER BY id DESC LIMIT $3"
        ))
        .bind(user_id)
        .bind(after)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| row.id);
        Ok((rows, info))
    }

    /// What other people wrote on the subjects of `owner_id`, newest first.
    /// Left out: the comments of people the owner stopped from commenting
    /// and of people in a block with the owner (`hidden`).
    pub(crate) async fn received_comments(
        &self,
        owner_id: Uuid,
        hidden: &[Uuid],
        filter: &ReceivedFilter,
        page: &Page,
    ) -> AppResult<(Vec<CommentRow>, PageInfo)> {
        let after: Option<Uuid> = page.decode()?;
        let mut query: QueryBuilder<Postgres> = QueryBuilder::new(concat!("SELECT ", comment_columns!(), " FROM comments c WHERE c.subject_owner_id = "));
        query.push_bind(owner_id);
        query.push(" AND c.author_id <> ").push_bind(owner_id);
        query.push(" AND c.author_id <> ALL(").push_bind(hidden).push(")");
        query.push(" AND NOT EXISTS (SELECT 1 FROM comment_blocks b WHERE b.owner_id = c.subject_owner_id AND b.blocked_user_id = c.author_id)");
        if let Some(status) = filter.status {
            query.push(" AND c.status = ").push_bind(status);
        }
        if let Some(subject) = filter.subject {
            query.push(" AND c.subject_type = ").push_bind(subject.kind);
            query.push(" AND c.subject_id = ").push_bind(subject.id);
        }
        if let Some(pattern) = &filter.pattern {
            query.push(" AND c.text ILIKE ").push_bind(pattern);
        }
        if let Some((author_id, channel_id)) = filter.identity {
            query.push(" AND c.author_id = ").push_bind(author_id);
            query.push(" AND c.channel_id IS NOT DISTINCT FROM ").push_bind(channel_id);
        }
        match filter.as_channel {
            Some(true) => {
                query.push(" AND c.channel_id IS NOT NULL");
            }
            Some(false) => {
                query.push(" AND c.channel_id IS NULL");
            }
            None => {}
        }
        if let Some(created_after) = filter.created_after {
            query.push(" AND c.created_at >= ").push_bind(created_after);
        }
        if let Some(created_before) = filter.created_before {
            query.push(" AND c.created_at < ").push_bind(created_before);
        }
        if let Some(after) = after {
            query.push(" AND c.id < ").push_bind(after);
        }
        query.push(" ORDER BY c.id DESC LIMIT ").push_bind(page.fetch());
        let mut rows: Vec<CommentRow> = query.build_query_as().fetch_all(&self.core.db).await?;
        let info = page.finish(&mut rows, |row| row.id);
        Ok((rows, info))
    }
}

/// The `comments.status` value a list filter asks for; `None` = any.
pub(crate) fn status_filter(value: tokuly_proto::engagement::CommentStatus) -> Option<i32> {
    use tokuly_proto::engagement::CommentStatus;
    match value {
        CommentStatus::COMMENT_STATUS_UNSPECIFIED => None,
        CommentStatus::COMMENT_STATUS_PUBLISHED => Some(status::PUBLISHED),
        CommentStatus::COMMENT_STATUS_HELD => Some(status::HELD),
        CommentStatus::COMMENT_STATUS_HIDDEN => Some(status::HIDDEN),
    }
}
