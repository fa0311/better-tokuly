//! What the owner of a subject does to the comments on it: approve, hide,
//! pin, heart, and stop people from commenting.
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokuly_core::{events, prelude::*};
use tokuly_identity::users;
use tokuly_proto::engagement::HoldReason;

use super::{CommentRow, Found, comment_not_found, lifecycle, lock_thread, status, write::comment_event};
use crate::{Engagement, effects::Effects, events::COMMENT_PUBLISHED};

/// Blocks one owner may place per hour.
const BLOCKS_PER_HOUR: u32 = 60;
/// How long a second heart on the same comment sends no second notification.
const HEART_NOTICE_WINDOW: Duration = Duration::from_secs(24 * 3600);

/// A row of `comment_blocks`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct BlockRow {
    pub id: Uuid,
    pub blocked_user_id: Uuid,
    /// The identity the owner blocked them under; `None` = as the user.
    pub channel_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// Whom `BlockCommenter` names.
pub(crate) enum BlockTarget<'a> {
    User(&'a str),
    /// Whoever wrote this comment, under the identity they wrote it with.
    AuthorOf(&'a str),
}

impl Engagement {
    /// Publishes a held or hidden comment (`ApproveComment`). The people
    /// it answers and mentions are told now, unless they were before.
    pub(crate) async fn approve_comment(&self, user_id: Uuid, comment_id: &str) -> AppResult<Found> {
        let mut found = self.require_moderated_comment(comment_id, user_id).await?;
        if found.row.is_published() {
            return Ok(found);
        }
        // Worked out before the transaction, which then only writes.
        let enters_public_view = found.parent.as_ref().is_none_or(CommentRow::is_published);
        let audience = if enters_public_view && found.row.notified_at.is_none() {
            let concerned = self.concerned_by(&found.row, found.parent.as_ref(), &found.subject).await?;
            self.comment_audience(found.row.author_id, &concerned, &found.subject).await?
        } else {
            Vec::new()
        };
        let sender = self.sender_of(found.row.author_id, found.row.channel_id).await?;

        let mut tx = self.core.db.begin().await?;
        let mut effects = Effects::default();
        let (mut row, mut parent) = lock_thread(&mut tx, found.row.id).await?.ok_or_else(comment_not_found)?;
        if !row.is_published() {
            lifecycle::set_status(&mut tx, &mut effects, &mut row, parent.as_mut(), status::PUBLISHED, HoldReason::HOLD_REASON_UNSPECIFIED)
                .await?;
            if row.in_public_view(parent.as_ref()) {
                if row.notified_at.is_none() {
                    self.announce_published(&mut tx, &row, &found.subject, &audience, &sender).await?;
                }
                events::emit(&mut *tx, COMMENT_PUBLISHED, Some(user_id), &comment_event(&row)).await?;
            }
        }
        tx.commit().await?;
        self.apply(effects).await;
        found.row = row;
        found.parent = parent;
        Ok(found)
    }

    /// Takes a comment out of public view without deleting it (`HideComment`).
    pub(crate) async fn hide_comment(&self, user_id: Uuid, comment_id: &str) -> AppResult<Found> {
        let mut found = self.require_moderated_comment(comment_id, user_id).await?;
        let mut tx = self.core.db.begin().await?;
        let mut effects = Effects::default();
        let (mut row, mut parent) = lock_thread(&mut tx, found.row.id).await?.ok_or_else(comment_not_found)?;
        // A comment already hidden keeps its reason (it may be hidden behind a block).
        if row.status != status::HIDDEN {
            lifecycle::set_status(&mut tx, &mut effects, &mut row, parent.as_mut(), status::HIDDEN, HoldReason::HOLD_REASON_HIDDEN_BY_OWNER)
                .await?;
        }
        tx.commit().await?;
        self.apply(effects).await;
        found.row = row;
        found.parent = parent;
        Ok(found)
    }

    /// Pins a comment to the top of its subject, or takes the pin off
    /// (`PinComment` / `UnpinComment`).
    pub(crate) async fn pin_comment(&self, user_id: Uuid, comment_id: &str, pinned: bool) -> AppResult<Found> {
        let mut found = self.require_moderated_comment(comment_id, user_id).await?;
        let mut tx = self.core.db.begin().await?;
        let mut effects = Effects::default();
        let (mut row, parent) = lock_thread(&mut tx, found.row.id).await?.ok_or_else(comment_not_found)?;
        if pinned {
            ensure!(
                !row.is_reply() && row.is_published(),
                precondition,
                "COMMENT_NOT_PINNABLE",
                "only a published top-level comment can be pinned"
            );
            lifecycle::pin(&mut tx, &mut effects, &mut row).await?;
        } else {
            lifecycle::unpin(&mut tx, &mut effects, &mut row).await?;
        }
        tx.commit().await?;
        self.apply(effects).await;
        found.row = row;
        found.parent = parent;
        Ok(found)
    }

    /// Gives a comment the creator's heart or takes it back (`SetCommentHeart`).
    pub(crate) async fn heart_comment(&self, user_id: Uuid, comment_id: &str, hearted: bool) -> AppResult<Found> {
        let mut found = self.require_moderated_comment(comment_id, user_id).await?;
        let mut tx = self.core.db.begin().await?;
        let mut effects = Effects::default();
        let (mut row, parent) = lock_thread(&mut tx, found.row.id).await?.ok_or_else(comment_not_found)?;
        ensure!(!hearted || row.is_published(), precondition, "COMMENT_NOT_PUBLISHED", "only a published comment can get a heart");
        let gained = lifecycle::set_heart(&mut tx, &mut effects, &mut row, parent.as_ref(), hearted).await?;
        // Taking a heart back and giving it again is not news twice.
        if gained
            && self.core.cache.set_nx(&format!("engagement:heart:{}", row.id), "1", HEART_NOTICE_WINDOW).await.unwrap_or(true)
        {
            self.announce_heart(&mut tx, &row, &found.subject).await?;
        }
        tx.commit().await?;
        self.apply(effects).await;
        found.row = row;
        found.parent = parent;
        Ok(found)
    }

    /// Stops someone from commenting on the caller's subjects
    /// (`BlockCommenter`) and hides what they wrote there under the blocked
    /// identity.
    pub(crate) async fn block_commenter(&self, owner_id: Uuid, target: BlockTarget<'_>) -> AppResult<BlockRow> {
        let (blocked_user_id, channel_id) = match target {
            BlockTarget::User(user_id) => {
                let user_id = parse_id(user_id)?;
                ensure!(user_id != owner_id, invalid, "CANNOT_BLOCK_SELF", "you cannot block yourself");
                let user = users::find_by_id(&self.core, user_id).await?.filter(|user| user.deleted_at.is_none());
                ensure!(user.is_some(), not_found, "USER_NOT_FOUND", "user not found");
                (user_id, None)
            }
            BlockTarget::AuthorOf(comment_id) => {
                let found = self.require_moderated_comment(comment_id, owner_id).await?;
                ensure!(found.row.author_id != owner_id, invalid, "CANNOT_BLOCK_SELF", "you cannot block yourself");
                (found.row.author_id, found.row.channel_id)
            }
        };
        self.core.cache.rate_limit(&format!("engagement:comment:block:{owner_id}"), BLOCKS_PER_HOUR, Duration::from_secs(3600)).await?;

        let mut tx = self.core.db.begin().await?;
        let mut effects = Effects::default();
        // Blocking the same identity again returns the block that exists.
        let block: BlockRow = sqlx::query_as(
            "INSERT INTO comment_blocks (owner_id, blocked_user_id, channel_id) VALUES ($1, $2, $3)
             ON CONFLICT (owner_id, blocked_user_id, channel_id) DO UPDATE SET owner_id = EXCLUDED.owner_id
             RETURNING id, blocked_user_id, channel_id, created_at",
        )
        .bind(owner_id)
        .bind(blocked_user_id)
        .bind(channel_id)
        .fetch_one(&mut *tx)
        .await?;
        lifecycle::hide_comments_of(&mut tx, &mut effects, owner_id, blocked_user_id, Some(channel_id)).await?;
        tx.commit().await?;
        self.apply(effects).await;
        Ok(block)
    }

    /// Lifts a block (`UnblockCommenter`). What it hid stays hidden.
    pub(crate) async fn unblock_commenter(&self, owner_id: Uuid, block_id: &str) -> AppResult<()> {
        let block_id = parse_id(block_id)?;
        let lifted = sqlx::query("DELETE FROM comment_blocks WHERE id = $1 AND owner_id = $2")
            .bind(block_id)
            .bind(owner_id)
            .execute(&self.core.db)
            .await?;
        ensure!(lifted.rows_affected() == 1, not_found, "BLOCK_NOT_FOUND", "block not found");
        Ok(())
    }

    /// The people `owner_id` blocked from commenting, most recent first.
    pub(crate) async fn blocked_commenters(&self, owner_id: Uuid, page: &Page) -> AppResult<(Vec<BlockRow>, tokuly_proto::common::PageInfo)> {
        let after: Option<Uuid> = page.decode()?;
        let mut rows: Vec<BlockRow> = sqlx::query_as(
            "SELECT id, blocked_user_id, channel_id, created_at FROM comment_blocks
             WHERE owner_id = $1 AND ($2::uuid IS NULL OR id < $2)
             ORDER BY id DESC LIMIT $3",
        )
        .bind(owner_id)
        .bind(after)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| row.id);
        Ok((rows, info))
    }
}
