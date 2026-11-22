//! Blocking. A block is silent and total: the friendship ends, pending
//! requests in both directions disappear, and from then on each of the two
//! is hidden from the other everywhere (other domains ask
//! [`crate::graph::is_blocked_either`] / [`crate::graph::blocked_ids_either`]).
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokuly_core::{events, prelude::*};
use tokuly_proto::common::{PageInfo, UserSummary};

use crate::{
    Social,
    events::{BlockChanged, FRIEND_REMOVED, FriendRemoved, USER_BLOCKED, USER_UNBLOCKED},
    friends::end_friendship,
    graph, idcache,
};

const MAX_BLOCKS_PAGE: i64 = 100;

impl Social {
    /// Blocks `target` for `user_id`. Blocking someone already blocked is a no-op.
    pub(crate) async fn block_user(&self, user_id: Uuid, target: Uuid) -> AppResult<()> {
        ensure!(user_id != target, invalid, "CANNOT_BLOCK_SELF", "you cannot block yourself");
        let core = &self.core;
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL)")
                .bind(target)
                .fetch_one(&core.db)
                .await?;
        ensure!(exists, not_found, "USER_NOT_FOUND", "user not found");
        core.cache.rate_limit(&format!("block:{user_id}"), 100, Duration::from_secs(3600)).await?;

        let mut tx = core.db.begin().await?;
        graph::lock_pair(&mut tx, user_id, target).await?;
        let newly_blocked =
            sqlx::query("INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(user_id)
                .bind(target)
                .execute(&mut *tx)
                .await?
                .rows_affected()
                > 0;
        let were_friends = end_friendship(&mut tx, user_id, target).await?;
        sqlx::query("DELETE FROM friend_requests WHERE (from_user_id = $1 AND to_user_id = $2) OR (from_user_id = $2 AND to_user_id = $1)")
            .bind(user_id)
            .bind(target)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM friend_suggestions WHERE (user_id = $1 AND suggested_id = $2) OR (user_id = $2 AND suggested_id = $1)")
            .bind(user_id)
            .bind(target)
            .execute(&mut *tx)
            .await?;
        if were_friends {
            events::emit(
                &mut *tx,
                FRIEND_REMOVED,
                Some(user_id),
                &FriendRemoved { user_id, friend_id: target, blocked: true },
            )
            .await?;
        }
        if newly_blocked {
            events::emit(&mut *tx, USER_BLOCKED, Some(user_id), &BlockChanged { user_id, blocked_id: target }).await?;
        }
        tx.commit().await?;

        idcache::HIDDEN.invalidate(core, &[user_id, target]).await;
        if were_friends {
            // To the blocked user this is indistinguishable from being
            // unfriended. Without a friendship nothing is pushed at all.
            self.after_friendship_ended(user_id, target).await;
        }
        Ok(())
    }

    /// Lifts `user_id`'s block on `target`. The friendship is not restored.
    pub(crate) async fn unblock_user(&self, user_id: Uuid, target: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let unblocked = sqlx::query("DELETE FROM user_blocks WHERE blocker_id = $1 AND blocked_id = $2")
            .bind(user_id)
            .bind(target)
            .execute(&mut *tx)
            .await?;
        if unblocked.rows_affected() > 0 {
            events::emit(&mut *tx, USER_UNBLOCKED, Some(user_id), &BlockChanged { user_id, blocked_id: target })
                .await?;
        }
        tx.commit().await?;
        idcache::HIDDEN.invalidate(&self.core, &[user_id, target]).await;
        Ok(())
    }

    /// One page of the users `user_id` blocked, most recent first.
    pub(crate) async fn list_blocked(&self, user_id: Uuid, page: &Page) -> AppResult<(Vec<UserSummary>, PageInfo)> {
        let cursor: Option<(DateTime<Utc>, Uuid)> = page.decode()?;
        let (after_time, after_id) = cursor.unzip();
        let mut rows: Vec<(Uuid, DateTime<Utc>)> = sqlx::query_as(
            "SELECT blocked_id, created_at FROM user_blocks
             WHERE blocker_id = $1 AND ($2::timestamptz IS NULL OR (created_at, blocked_id) < ($2, $3))
             ORDER BY created_at DESC, blocked_id DESC
             LIMIT $4",
        )
        .bind(user_id)
        .bind(after_time)
        .bind(after_id)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |(id, at)| (*at, *id));
        let ids: Vec<Uuid> = rows.iter().map(|(id, _)| *id).collect();
        Ok((tokuly_identity::summaries_ordered(&self.core, &ids).await?, info))
    }

    pub(crate) fn blocks_page(page: Option<&tokuly_proto::common::PageRequestView<'_>>) -> Page {
        Page::from_view(page, MAX_BLOCKS_PAGE)
    }
}
