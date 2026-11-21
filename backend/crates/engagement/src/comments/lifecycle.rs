//! Every change of a comment's place in public view, and the bookkeeping
//! that follows from it.
//!
//! All functions here run inside the caller's transaction, on rows locked by
//! [`lock_thread`](super::lock_thread), and record what the outside world
//! must hear in an [`Effects`] value. They keep three numbers exact:
//!
//! - `comments.reply_count` of a top-level comment: its published replies;
//! - `subject_stats.comment_count`: the comments in public view;
//! - the owner's review queue (comments with status `HELD`).
//!
//! Locks are always taken in the same order — the top-level comment, the
//! reply, then the subject's statistics row — so concurrent changes to one
//! thread wait for each other instead of deadlocking.
use sqlx::PgConnection;
use tokuly_core::prelude::*;
use tokuly_media::assets;
use tokuly_proto::{common::SubjectType, engagement::HoldReason};

use super::{CommentRow, image_holder, lock_thread, status};
use crate::{
    cleanup,
    effects::{Effects, Push},
    stats::{self, Counter},
    subjects::SubjectKey,
};

/// The comments in public view on the comment's subject changed by `delta`.
async fn count_comments(conn: &mut PgConnection, effects: &mut Effects, row: &CommentRow, delta: i64) -> AppResult<()> {
    stats::add(conn, row.key(), Some(row.subject_owner_id), Counter::Comment, delta).await?;
    effects.count(row.key(), Counter::Comment, delta);
    Ok(())
}

async fn add_replies(conn: &mut PgConnection, parent: &mut CommentRow, delta: i32) -> AppResult<()> {
    sqlx::query("UPDATE comments SET reply_count = GREATEST(reply_count + $2, 0) WHERE id = $1")
        .bind(parent.id)
        .bind(delta)
        .execute(conn)
        .await?;
    parent.reply_count = (parent.reply_count + delta).max(0);
    Ok(())
}

/// A comment that was just inserted takes its place: a published one enters
/// public view, a held one the owner's review queue.
pub(crate) async fn inserted(
    conn: &mut PgConnection,
    effects: &mut Effects,
    row: &CommentRow,
    parent: Option<&mut CommentRow>,
) -> AppResult<()> {
    if row.is_held() {
        effects.review_queue(row.subject_owner_id);
    }
    if row.is_published() {
        entered(conn, effects, row, parent).await?;
    }
    Ok(())
}

/// `row` became published: counts and announces what entered public view
/// with it.
async fn entered(conn: &mut PgConnection, effects: &mut Effects, row: &CommentRow, parent: Option<&mut CommentRow>) -> AppResult<()> {
    match parent {
        None => {
            count_comments(conn, effects, row, 1 + i64::from(row.reply_count)).await?;
            effects.push(Push::Added { key: row.key(), comment_id: row.id });
        }
        Some(parent) => {
            add_replies(conn, parent, 1).await?;
            if parent.is_published() {
                count_comments(conn, effects, row, 1).await?;
                effects.push(Push::Added { key: row.key(), comment_id: row.id });
                // The thread's reply count changed for everyone watching.
                effects.push(Push::Updated { key: row.key(), comment_id: parent.id });
            }
        }
    }
    Ok(())
}

/// `row` stopped being published (or is being deleted while published):
/// counts and announces what left public view with it.
async fn left(conn: &mut PgConnection, effects: &mut Effects, row: &CommentRow, parent: Option<&mut CommentRow>) -> AppResult<()> {
    match parent {
        None => {
            let gone = 1 + row.reply_count;
            count_comments(conn, effects, row, -i64::from(gone)).await?;
            effects.push(Push::Removed { key: row.key(), comment_id: row.id, parent_id: None, removed_count: gone });
        }
        Some(parent) => {
            add_replies(conn, parent, -1).await?;
            if parent.is_published() {
                count_comments(conn, effects, row, -1).await?;
                effects.push(Push::Removed { key: row.key(), comment_id: row.id, parent_id: Some(parent.id), removed_count: 1 });
                effects.push(Push::Updated { key: row.key(), comment_id: parent.id });
            }
        }
    }
    Ok(())
}

/// Moves a comment to another status. Leaving `PUBLISHED` also takes the
/// pin off. `row` and `parent` are updated to match the database.
pub(crate) async fn set_status(
    conn: &mut PgConnection,
    effects: &mut Effects,
    row: &mut CommentRow,
    parent: Option<&mut CommentRow>,
    new_status: i32,
    hold_reason: HoldReason,
) -> AppResult<()> {
    let hold_reason = hold_reason as i32;
    if row.status == new_status && row.hold_reason == hold_reason {
        return Ok(());
    }
    sqlx::query(
        "UPDATE comments SET status = $2, hold_reason = $3, pinned_at = CASE WHEN $2 = 1 THEN pinned_at END WHERE id = $1",
    )
    .bind(row.id)
    .bind(new_status)
    .bind(hold_reason)
    .execute(&mut *conn)
    .await?;
    let (was_published, was_held) = (row.is_published(), row.is_held());
    row.status = new_status;
    row.hold_reason = hold_reason;
    if was_held != row.is_held() {
        effects.review_queue(row.subject_owner_id);
    }
    if was_published == row.is_published() {
        return Ok(());
    }
    if row.is_published() {
        entered(conn, effects, row, parent).await
    } else {
        row.pinned_at = None;
        // `left` reads the reply count of the row as it was in public view.
        left(conn, effects, row, parent).await
    }
}

/// Deletes a comment and, for a top-level comment, its replies: releases
/// their uploads, drops the reactions and other engagement data hanging off
/// them, and settles the counters. Returns how many comments are gone.
pub(crate) async fn delete(conn: &mut PgConnection, effects: &mut Effects, row: &CommentRow, parent: Option<&mut CommentRow>) -> AppResult<i32> {
    let mut comment_ids: Vec<Uuid> = vec![row.id];
    if !row.is_reply() {
        let replies: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM comments WHERE parent_id = $1").bind(row.id).fetch_all(&mut *conn).await?;
        comment_ids.extend(replies);
    }
    let held: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM comments WHERE id = ANY($1) AND status = $2)")
        .bind(&comment_ids)
        .bind(status::HELD)
        .fetch_one(&mut *conn)
        .await?;
    if held {
        effects.review_queue(row.subject_owner_id);
    }
    release(conn, &comment_ids).await?;
    // Replies go with their thread (ON DELETE CASCADE).
    sqlx::query("DELETE FROM comments WHERE id = $1").bind(row.id).execute(&mut *conn).await?;
    if row.is_published() {
        left(conn, effects, row, parent).await?;
    }
    Ok(comment_ids.len() as i32)
}

/// Lets go of what comments about to be deleted hold elsewhere: their
/// uploads, and the engagement data that has them as its subject (likes,
/// bookmarks, counters).
pub(crate) async fn release(conn: &mut PgConnection, comment_ids: &[Uuid]) -> AppResult<()> {
    let with_image: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM comments WHERE id = ANY($1) AND image_asset_id IS NOT NULL ORDER BY id")
        .bind(comment_ids)
        .fetch_all(&mut *conn)
        .await?;
    for comment_id in with_image {
        assets::detach_holder(&mut *conn, &image_holder(comment_id)).await?;
    }
    cleanup::forget_subjects(conn, SubjectType::SUBJECT_TYPE_COMMENT as i32, comment_ids).await
}

/// Serializes changes to a subject's pin for the rest of the transaction;
/// without it two owners' devices pinning at once would both pass the
/// "unpin the other" step.
async fn lock_pin(conn: &mut PgConnection, key: SubjectKey) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("engagement:pin:{}:{}", key.kind, key.id))
        .execute(conn)
        .await?;
    Ok(())
}

/// Pins a published top-level comment, replacing the subject's current pin.
pub(crate) async fn pin(conn: &mut PgConnection, effects: &mut Effects, row: &mut CommentRow) -> AppResult<()> {
    lock_pin(conn, row.key()).await?;
    let unpinned: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE comments SET pinned_at = NULL
         WHERE subject_type = $1 AND subject_id = $2 AND pinned_at IS NOT NULL AND id <> $3
         RETURNING id",
    )
    .bind(row.subject_type)
    .bind(row.subject_id)
    .bind(row.id)
    .fetch_all(&mut *conn)
    .await?;
    for comment_id in unpinned {
        effects.push(Push::Pinned { key: row.key(), comment_id, pinned: false });
    }
    if row.pinned_at.is_none() {
        let pinned_at: Time =
            sqlx::query_scalar("UPDATE comments SET pinned_at = now() WHERE id = $1 RETURNING pinned_at").bind(row.id).fetch_one(conn).await?;
        row.pinned_at = Some(pinned_at);
        effects.push(Push::Pinned { key: row.key(), comment_id: row.id, pinned: true });
    }
    Ok(())
}

pub(crate) async fn unpin(conn: &mut PgConnection, effects: &mut Effects, row: &mut CommentRow) -> AppResult<()> {
    if row.pinned_at.is_some() {
        sqlx::query("UPDATE comments SET pinned_at = NULL WHERE id = $1").bind(row.id).execute(conn).await?;
        row.pinned_at = None;
        effects.push(Push::Pinned { key: row.key(), comment_id: row.id, pinned: false });
    }
    Ok(())
}

/// Gives or takes the creator's heart. Returns whether the comment gained
/// one it did not have.
pub(crate) async fn set_heart(
    conn: &mut PgConnection,
    effects: &mut Effects,
    row: &mut CommentRow,
    parent: Option<&CommentRow>,
    hearted: bool,
) -> AppResult<bool> {
    if row.hearted_at.is_some() == hearted {
        return Ok(false);
    }
    let hearted_at: Option<Time> =
        sqlx::query_scalar("UPDATE comments SET hearted_at = CASE WHEN $2 THEN now() END WHERE id = $1 RETURNING hearted_at")
            .bind(row.id)
            .bind(hearted)
            .fetch_one(conn)
            .await?;
    row.hearted_at = hearted_at;
    if row.in_public_view(parent) {
        effects.push(Push::Updated { key: row.key(), comment_id: row.id });
    }
    Ok(hearted)
}

/// Hides what `author_id` wrote on the subjects of `owner_id` — under one
/// identity (`Some(None)`: as the user, `Some(Some(id))`: as that channel)
/// or under any (`None`). Held comments leave the review queue the same
/// way. Used when an owner blocks a commenter or the whole account.
pub(crate) async fn hide_comments_of(
    conn: &mut PgConnection,
    effects: &mut Effects,
    owner_id: Uuid,
    author_id: Uuid,
    identity: Option<Option<Uuid>>,
) -> AppResult<u64> {
    let (any_identity, channel_id) = match identity {
        Some(channel_id) => (false, channel_id),
        None => (true, None),
    };
    // Oldest first: a top-level comment always precedes its replies, which
    // is the order `lock_thread` takes its locks in.
    let comment_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM comments
         WHERE subject_owner_id = $1 AND author_id = $2 AND status <> $3
           AND ($4 OR channel_id IS NOT DISTINCT FROM $5)
         ORDER BY id",
    )
    .bind(owner_id)
    .bind(author_id)
    .bind(status::HIDDEN)
    .bind(any_identity)
    .bind(channel_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut hidden = 0;
    for comment_id in comment_ids {
        let Some((mut row, mut parent)) = lock_thread(conn, comment_id).await? else { continue };
        if row.status == status::HIDDEN {
            continue;
        }
        set_status(conn, effects, &mut row, parent.as_mut(), status::HIDDEN, HoldReason::HOLD_REASON_BLOCKED_COMMENTER).await?;
        hidden += 1;
    }
    Ok(hidden)
}
