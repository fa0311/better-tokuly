//! Comments: the row, who may see it, and how one is loaded for a viewer.
//!
//! | module | what lives there |
//! |---|---|
//! | [`lifecycle`] | every change of a comment's place in public view: publish, hold, hide, delete, pin, heart — and the counters that follow |
//! | [`write`] | creating and editing comments |
//! | [`screening`] | the owner's rules: review-all, blocked words, links, blocked commenters |
//! | [`moderate`] | what the owner does to single comments and commenters |
//! | [`list`] | the queries behind the lists |
//! | [`render`] | rows → `tokuly.engagement.v1.Comment` for a viewer |
//! | [`announce`] | notifications about comments |
//!
//! # Public view
//!
//! A comment is in public view when it is published and, for a reply, its
//! top-level comment is published too ([`CommentRow::in_public_view`]).
//! `subject_stats.comment_count`, realtime events and notifications all
//! follow that one definition.
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use tokuly_core::prelude::*;
use tokuly_proto::engagement::{CommentStatus, HoldReason};

use crate::{
    Engagement,
    subjects::{Subject, SubjectKey, is_absent},
};

pub(crate) mod announce;
pub(crate) mod lifecycle;
pub(crate) mod list;
pub(crate) mod moderate;
pub(crate) mod render;
pub(crate) mod screening;
pub(crate) mod write;

/// `comments.status` values (`tokuly.engagement.v1.CommentStatus`).
pub(crate) mod status {
    use tokuly_proto::engagement::CommentStatus;

    pub(crate) const PUBLISHED: i32 = CommentStatus::COMMENT_STATUS_PUBLISHED as i32;
    pub(crate) const HELD: i32 = CommentStatus::COMMENT_STATUS_HELD as i32;
    pub(crate) const HIDDEN: i32 = CommentStatus::COMMENT_STATUS_HIDDEN as i32;
}

const _: () = {
    // The migration's CHECK constraint and partial indexes use these numbers.
    assert!(status::PUBLISHED == 1 && status::HELD == 2 && status::HIDDEN == 3);
};

/// The media holder of a comment's image.
pub(crate) fn image_holder(comment_id: Uuid) -> String {
    format!("engagement:comment:{comment_id}")
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct CommentRow {
    pub id: Uuid,
    pub subject_type: i32,
    pub subject_id: Uuid,
    /// The subject's owner when the comment was written.
    pub subject_owner_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub reply_to_id: Option<Uuid>,
    pub author_id: Uuid,
    pub channel_id: Option<Uuid>,
    pub text: String,
    pub stamp_id: Option<Uuid>,
    pub image: Option<serde_json::Value>,
    pub image_asset_id: Option<Uuid>,
    pub timestamp_seconds: Option<i32>,
    pub status: i32,
    pub hold_reason: i32,
    pub like_count: i64,
    pub dislike_count: i64,
    pub reply_count: i32,
    pub pinned_at: Option<DateTime<Utc>>,
    pub hearted_at: Option<DateTime<Utc>>,
    pub edited_at: Option<DateTime<Utc>>,
    pub notified_at: Option<DateTime<Utc>>,
    pub score: f64,
    pub created_at: DateTime<Utc>,
}

macro_rules! comment_columns {
    () => {
        "id, subject_type, subject_id, subject_owner_id, parent_id, reply_to_id, author_id, channel_id, text, stamp_id, \
         image, image_asset_id, timestamp_seconds, status, hold_reason, like_count, dislike_count, reply_count, \
         pinned_at, hearted_at, edited_at, notified_at, score, created_at"
    };
}
pub(crate) use comment_columns;

impl CommentRow {
    pub(crate) fn key(&self) -> SubjectKey {
        SubjectKey { kind: self.subject_type, id: self.subject_id }
    }

    pub(crate) fn is_reply(&self) -> bool {
        self.parent_id.is_some()
    }

    pub(crate) fn is_published(&self) -> bool {
        self.status == status::PUBLISHED
    }

    pub(crate) fn is_held(&self) -> bool {
        self.status == status::HELD
    }

    /// Hidden because the subject owner blocked its author. The author is
    /// not told: to them the comment reads as published.
    pub(crate) fn is_shadowed(&self) -> bool {
        self.status == status::HIDDEN && self.hold_reason == HoldReason::HOLD_REASON_BLOCKED_COMMENTER as i32
    }

    /// Whether everyone who sees the subject sees this comment. `parent` is
    /// the top-level comment of a reply.
    pub(crate) fn in_public_view(&self, parent: Option<&CommentRow>) -> bool {
        self.is_published() && parent.is_none_or(CommentRow::is_published)
    }

    /// Whether the comment reads as published to `user_id`: it is, or it is
    /// their own comment hidden behind a block they are not told about.
    /// Decides what a user may reply to.
    pub(crate) fn reads_published_to(&self, user_id: Uuid) -> bool {
        self.is_published() || (self.author_id == user_id && self.is_shadowed())
    }

    /// Whether `viewer` may see this comment, taken by itself: its author
    /// always; nobody in a block with its author (`hidden`); everyone else
    /// when it is published; the subject owner also while it is held or
    /// hidden.
    pub(crate) fn visible_to(&self, viewer: Option<Uuid>, subject_owner: Uuid, hidden: &[Uuid]) -> bool {
        if viewer == Some(self.author_id) {
            return true;
        }
        if hidden.contains(&self.author_id) {
            return false;
        }
        self.is_published() || (viewer == Some(subject_owner) && !subject_owner.is_nil())
    }

    /// The status as `viewer` is shown it.
    pub(crate) fn status_for(&self, viewer: Option<Uuid>) -> CommentStatus {
        if self.is_published() || (viewer == Some(self.author_id) && self.is_shadowed()) {
            CommentStatus::COMMENT_STATUS_PUBLISHED
        } else if self.is_held() {
            CommentStatus::COMMENT_STATUS_HELD
        } else {
            CommentStatus::COMMENT_STATUS_HIDDEN
        }
    }
}

pub(crate) async fn find<'e, E: PgExecutor<'e>>(executor: E, comment_id: Uuid) -> AppResult<Option<CommentRow>> {
    Ok(sqlx::query_as(concat!("SELECT ", comment_columns!(), " FROM comments WHERE id = $1"))
        .bind(comment_id)
        .fetch_optional(executor)
        .await?)
}

async fn lock(conn: &mut PgConnection, comment_id: Uuid) -> AppResult<Option<CommentRow>> {
    Ok(sqlx::query_as(concat!("SELECT ", comment_columns!(), " FROM comments WHERE id = $1 FOR UPDATE"))
        .bind(comment_id)
        .fetch_optional(conn)
        .await?)
}

/// Locks a comment for the rest of the transaction and returns it as it is
/// now — together with its top-level comment when it is a reply, which is
/// locked first. Every change to a thread takes its locks through here, in
/// that order, so two of them queue up instead of deadlocking.
pub(crate) async fn lock_thread(conn: &mut PgConnection, comment_id: Uuid) -> AppResult<Option<(CommentRow, Option<CommentRow>)>> {
    let parent_id: Option<Option<Uuid>> =
        sqlx::query_scalar("SELECT parent_id FROM comments WHERE id = $1").bind(comment_id).fetch_optional(&mut *conn).await?;
    let Some(parent_id) = parent_id else { return Ok(None) };
    let parent = match parent_id {
        // A reply cannot outlive its thread, so a missing parent means the
        // reply is gone as well.
        Some(parent_id) => match lock(conn, parent_id).await? {
            Some(parent) => Some(parent),
            None => return Ok(None),
        },
        None => None,
    };
    Ok(lock(conn, comment_id).await?.map(|row| (row, parent)))
}

/// Comments waiting for `owner_id`'s review, across all their subjects.
pub(crate) async fn held_count<'e, E: PgExecutor<'e>>(executor: E, owner_id: Uuid) -> AppResult<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM comments WHERE subject_owner_id = $1 AND status = $2")
        .bind(owner_id)
        .bind(status::HELD)
        .fetch_one(executor)
        .await?)
}

/// A comment as loaded for a viewer who may see it.
#[derive(Debug, Clone)]
pub(crate) struct Found {
    pub row: CommentRow,
    /// The top-level comment, when `row` is a reply.
    pub parent: Option<CommentRow>,
    /// The subject, as resolved for the viewer.
    pub subject: Subject,
    /// Everyone in a block with the viewer.
    pub hidden: Vec<Uuid>,
}

impl Found {
    /// The subject's owner; the nil UUID when it belongs to nobody.
    pub(crate) fn owner_id(&self) -> Uuid {
        self.subject.info.owner_id
    }

    /// The top-level comment, if the viewer may see it. The author of a
    /// reply keeps seeing the reply after its thread was hidden, but not
    /// the comment above it.
    pub(crate) fn visible_parent(&self, viewer: Option<Uuid>) -> Option<&CommentRow> {
        self.parent.as_ref().filter(|parent| parent.visible_to(viewer, self.owner_id(), &self.hidden))
    }
}

pub(crate) fn comment_not_found() -> AppError {
    AppError::not_found("COMMENT_NOT_FOUND", "comment not found")
}

impl Engagement {
    /// Loads a comment for `viewer`. `None` when it does not exist or the
    /// viewer may not see it: they cannot see its subject, there is a block
    /// between them and its author, it is held or hidden and they are
    /// neither its author nor the subject owner, or it is a reply to a
    /// comment they cannot see (unless the reply is their own).
    pub(crate) async fn comment_for(&self, comment_id: Uuid, viewer: Option<Uuid>) -> AppResult<Option<Found>> {
        let Some(row) = find(&self.core.db, comment_id).await? else { return Ok(None) };
        let subject = match self.subject(row.key().subject_type(), &row.subject_id.to_string(), viewer).await {
            Ok(subject) => subject,
            Err(error) if is_absent(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
        let hidden = self.hidden_from(viewer).await?;
        let owner_id = subject.info.owner_id;
        if !row.visible_to(viewer, owner_id, &hidden) {
            return Ok(None);
        }
        let parent = match row.parent_id {
            Some(parent_id) => find(&self.core.db, parent_id).await?,
            None => None,
        };
        if row.is_reply() && viewer != Some(row.author_id) {
            match &parent {
                Some(parent) if parent.visible_to(viewer, owner_id, &hidden) => {}
                _ => return Ok(None),
            }
        }
        Ok(Some(Found { row, parent, subject, hidden }))
    }

    /// [`Self::comment_for`] for the comment ID of a request;
    /// `COMMENT_NOT_FOUND` when there is nothing for the viewer to see.
    pub(crate) async fn require_comment(&self, comment_id: &str, viewer: Option<Uuid>) -> AppResult<Found> {
        let comment_id = parse_id(comment_id)?;
        self.comment_for(comment_id, viewer).await?.ok_or_else(comment_not_found)
    }

    /// The comment of a request, for an action only the owner of its subject
    /// may take: `COMMENT_NOT_FOUND` when the caller cannot see it,
    /// `NOT_SUBJECT_OWNER` when they can but do not own the subject.
    pub(crate) async fn require_moderated_comment(&self, comment_id: &str, user_id: Uuid) -> AppResult<Found> {
        let found = self.require_comment(comment_id, Some(user_id)).await?;
        ensure!(found.subject.is_owned_by(user_id), denied, "NOT_SUBJECT_OWNER", "only the owner of the content can do this");
        Ok(found)
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// A published top-level comment for unit tests.
    pub(crate) fn comment(author_id: Uuid) -> CommentRow {
        CommentRow {
            id: Uuid::now_v7(),
            subject_type: 5,
            subject_id: Uuid::now_v7(),
            subject_owner_id: Uuid::now_v7(),
            parent_id: None,
            reply_to_id: None,
            author_id,
            channel_id: None,
            text: "こんにちは".into(),
            stamp_id: None,
            image: None,
            image_asset_id: None,
            timestamp_seconds: None,
            status: status::PUBLISHED,
            hold_reason: 0,
            like_count: 0,
            dislike_count: 0,
            reply_count: 0,
            pinned_at: None,
            hearted_at: None,
            edited_at: None,
            notified_at: None,
            score: 0.0,
            created_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{testing::comment, *};

    #[test]
    fn published_comments_are_visible_to_everyone_but_blocked_users() {
        let (author, owner, stranger, blocked) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let row = comment(author);
        assert!(row.visible_to(None, owner, &[]));
        assert!(row.visible_to(Some(stranger), owner, &[]));
        assert!(row.visible_to(Some(owner), owner, &[]));
        // `hidden` is the viewer's block list: the author is on it.
        assert!(!row.visible_to(Some(blocked), owner, &[author]));
        assert!(row.visible_to(Some(author), owner, &[author]));
    }

    #[test]
    fn held_and_hidden_comments_are_for_the_author_and_the_owner() {
        let (author, owner, stranger) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        for state in [status::HELD, status::HIDDEN] {
            let row = CommentRow { status: state, ..comment(author) };
            assert!(row.visible_to(Some(author), owner, &[]));
            assert!(row.visible_to(Some(owner), owner, &[]));
            assert!(!row.visible_to(Some(stranger), owner, &[]));
            assert!(!row.visible_to(None, owner, &[]));
            // A subject that belongs to nobody has no owner to show them to.
            assert!(!row.visible_to(Some(Uuid::nil()), Uuid::nil(), &[]));
        }
    }

    #[test]
    fn a_shadowed_comment_reads_published_to_its_author_only() {
        let (author, owner) = (Uuid::now_v7(), Uuid::now_v7());
        let shadowed = CommentRow {
            status: status::HIDDEN,
            hold_reason: HoldReason::HOLD_REASON_BLOCKED_COMMENTER as i32,
            ..comment(author)
        };
        assert!(shadowed.is_shadowed());
        assert_eq!(shadowed.status_for(Some(author)), CommentStatus::COMMENT_STATUS_PUBLISHED);
        assert_eq!(shadowed.status_for(Some(owner)), CommentStatus::COMMENT_STATUS_HIDDEN);
        assert!(shadowed.reads_published_to(author) && !shadowed.reads_published_to(owner));

        let hidden_by_hand =
            CommentRow { status: status::HIDDEN, hold_reason: HoldReason::HOLD_REASON_HIDDEN_BY_OWNER as i32, ..comment(author) };
        assert_eq!(hidden_by_hand.status_for(Some(author)), CommentStatus::COMMENT_STATUS_HIDDEN);
        assert!(!hidden_by_hand.reads_published_to(author));
        let held = CommentRow { status: status::HELD, ..comment(author) };
        assert_eq!(held.status_for(Some(author)), CommentStatus::COMMENT_STATUS_HELD);
    }

    #[test]
    fn a_reply_is_in_public_view_only_below_a_published_comment() {
        let author = Uuid::now_v7();
        let top = comment(author);
        let reply = CommentRow { parent_id: Some(top.id), ..comment(author) };
        assert!(top.in_public_view(None));
        assert!(reply.in_public_view(Some(&top)));
        let hidden_top = CommentRow { status: status::HIDDEN, ..top.clone() };
        assert!(!reply.in_public_view(Some(&hidden_top)));
        let held_reply = CommentRow { status: status::HELD, ..reply };
        assert!(!held_reply.in_public_view(Some(&top)));
    }
}
