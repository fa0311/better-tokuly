//! What viewers keep and say about shorts: the private saved list, the
//! copy of their likes, "not interested" marks, and shares.
//!
//! Likes themselves belong to the engagement domain. This domain keeps a
//! copy of who liked which short (`short_likes`, from
//! `engagement.reaction.changed`) because three things need it without a
//! call per short: the `liked` flag on every card, the "liked" list, and
//! the hashtag affinity of the for-you feed. The like *count* does not come
//! from the copy but from engagement's `counter_changed` callback.
//!
//! The copy is kept from events, which arrive at least once and not always
//! in order. Each row remembers the time of the latest change it has seen,
//! and a change older than that is ignored; a like taken back stays as a
//! row (`liked = false`) for a week so that the like it undid cannot
//! arrive late and resurrect it.
//!
//! What a save or a share adds to a short's ranking is counted once per
//! account ([`Shorts::count_signal`]): saving and unsaving in a loop, or
//! sharing a hundred times, moves nothing.
use tokuly_core::prelude::*;
use tokuly_proto::common::PageInfo;

use crate::{
    Shorts,
    access::{self, Viewer},
    shorts::{ShortRow, short_columns},
    signals::{self, Engagement},
};

/// A short with the moment the viewer saved or liked it.
#[derive(sqlx::FromRow)]
struct Kept {
    #[sqlx(flatten)]
    short: ShortRow,
    kept_at: Time,
}

impl Shorts {
    /// Adds `short` to the saved list of `user_id` and returns the short's
    /// save count. Saving a saved short changes nothing.
    pub async fn save(&self, user_id: Uuid, short: &ShortRow) -> AppResult<i64> {
        let core = &self.core;
        // Read before the transaction: nothing below may wait for a second
        // connection while it holds the lock on the saved list.
        let weighs = user_id != short.owner_id && self.carries_weight(user_id).await?;
        let mut tx = core.db.begin().await?;
        // One change to a user's saved list at a time, so the limit holds
        // when several saves arrive together. Everything up to the commit
        // runs on this transaction.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("shorts:saves:{}", user_id.simple()))
            .execute(&mut *tx)
            .await?;
        let (kept, already): (i64, bool) = sqlx::query_as(
            "SELECT count(*), COALESCE(bool_or(short_id = $2), false) FROM short_saves WHERE user_id = $1",
        )
        .bind(user_id)
        .bind(short.id)
        .fetch_one(&mut *tx)
        .await?;
        if already {
            return Ok(short.save_count);
        }
        ensure!(
            kept < self.cfg.max_saves,
            precondition,
            "SAVE_LIMIT_REACHED",
            "you can keep at most {} shorts saved",
            self.cfg.max_saves
        );
        sqlx::query("INSERT INTO short_saves (user_id, short_id) VALUES ($1, $2)")
            .bind(user_id)
            .bind(short.id)
            .execute(&mut *tx)
            .await?;
        let save_count: i64 =
            sqlx::query_scalar("UPDATE shorts SET save_count = save_count + 1 WHERE id = $1 RETURNING save_count")
                .bind(short.id)
                .fetch_one(&mut *tx)
                .await?;
        // The signal is claimed before the write it stands for; if that
        // write does not happen, the claim is given back.
        let counted = weighs && self.claim_signal(short, Engagement::Save, user_id).await?;
        let written = async {
            if counted {
                signals::bump_hourly(&mut *tx, short.id, Engagement::Save, 1).await?;
            }
            tx.commit().await?;
            AppResult::Ok(())
        }
        .await;
        if let Err(error) = written {
            if counted {
                self.release_signal(short, Engagement::Save, user_id).await;
            }
            return Err(error);
        }
        signals::track(core, "short.save", short, Some(user_id), 1.0, serde_json::json!({})).await;
        Ok(save_count)
    }

    /// Removes a short from the saved list of `user_id` and returns the
    /// short as it is afterwards (`None` when it is gone). Whether the
    /// caller may know its numbers is for the caller of this to decide.
    pub async fn unsave(&self, user_id: Uuid, short_id: Uuid) -> AppResult<Option<ShortRow>> {
        let core = &self.core;
        let mut tx = core.db.begin().await?;
        let removed = sqlx::query("DELETE FROM short_saves WHERE user_id = $1 AND short_id = $2")
            .bind(user_id)
            .bind(short_id)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            == 1;
        let short: Option<ShortRow> = if removed {
            sqlx::query_as(concat!(
                "UPDATE shorts s SET save_count = GREATEST(s.save_count - 1, 0) WHERE s.id = $1 RETURNING ",
                short_columns!()
            ))
            .bind(short_id)
            .fetch_optional(&mut *tx)
            .await?
        } else {
            sqlx::query_as(concat!("SELECT ", short_columns!(), " FROM shorts s WHERE s.id = $1"))
                .bind(short_id)
                .fetch_optional(&mut *tx)
                .await?
        };
        tx.commit().await?;
        if let (true, Some(short)) = (removed, &short) {
            signals::track(core, "short.save", short, Some(user_id), -1.0, serde_json::json!({})).await;
        }
        Ok(short)
    }

    /// The viewer's saved shorts, most recently saved first, as far as they
    /// may still see them.
    pub async fn saved_page(&self, viewer: &Viewer, page: &Page) -> AppResult<(Vec<ShortRow>, PageInfo)> {
        let before: Option<(Time, Uuid)> = page.decode()?;
        let mut rows: Vec<Kept> = viewer
            .query_as(concat!(
                "SELECT ",
                short_columns!(),
                ", v.created_at AS kept_at FROM short_saves v JOIN shorts s ON s.id = v.short_id
              WHERE v.user_id = $1 AND ",
                access::watchable!(),
                " AND ($7::timestamptz IS NULL OR (v.created_at, v.short_id) < ($7, $8))
              ORDER BY v.created_at DESC, v.short_id DESC LIMIT $9"
            ))
            .bind(before.map(|(at, _)| at))
            .bind(before.map(|(_, id)| id))
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
        let info = page.finish(&mut rows, |kept| (kept.kept_at, kept.short.id));
        viewer.drop_newly_locked(&self.core, &mut rows, |kept| kept.short.lock_facts()).await?;
        Ok((rows.into_iter().map(|kept| kept.short).collect(), info))
    }

    /// The shorts the viewer liked, most recently liked first, as far as
    /// they may still see them.
    pub async fn liked_page(&self, viewer: &Viewer, page: &Page) -> AppResult<(Vec<ShortRow>, PageInfo)> {
        let before: Option<(Time, Uuid)> = page.decode()?;
        let mut rows: Vec<Kept> = viewer
            .query_as(concat!(
                "SELECT ",
                short_columns!(),
                ", l.created_at AS kept_at FROM short_likes l JOIN shorts s ON s.id = l.short_id
              WHERE l.user_id = $1 AND l.liked AND ",
                access::watchable!(),
                " AND ($7::timestamptz IS NULL OR (l.created_at, l.short_id) < ($7, $8))
              ORDER BY l.created_at DESC, l.short_id DESC LIMIT $9"
            ))
            .bind(before.map(|(at, _)| at))
            .bind(before.map(|(_, id)| id))
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
        let info = page.finish(&mut rows, |kept| (kept.kept_at, kept.short.id));
        viewer.drop_newly_locked(&self.core, &mut rows, |kept| kept.short.lock_facts()).await?;
        Ok((rows.into_iter().map(|kept| kept.short).collect(), info))
    }

    /// Follows a like given or taken back in the engagement domain at `at`
    /// (the event's time). A change older than the latest one already seen
    /// for this user and short is ignored, so events may arrive twice or
    /// out of order. A like of a short that no longer exists is ignored.
    /// Returns whether the copy changed.
    pub async fn mirror_like(&self, user_id: Uuid, short_id: Uuid, liked: bool, at: Time) -> AppResult<bool> {
        let changed = sqlx::query(
            "INSERT INTO short_likes (user_id, short_id, liked, created_at, changed_at)
             SELECT $1, s.id, $3, $4, $4 FROM shorts s
             WHERE s.id = $2 AND EXISTS (SELECT 1 FROM users u WHERE u.id = $1)
             ON CONFLICT (user_id, short_id) DO UPDATE
                 SET liked = EXCLUDED.liked, changed_at = EXCLUDED.changed_at,
                     created_at = CASE WHEN EXCLUDED.liked AND NOT short_likes.liked
                                       THEN EXCLUDED.changed_at ELSE short_likes.created_at END
                 WHERE short_likes.changed_at < EXCLUDED.changed_at",
        )
        .bind(user_id)
        .bind(short_id)
        .bind(liked)
        .bind(at)
        .execute(&self.core.db)
        .await?
        .rows_affected()
            == 1;
        Ok(changed)
    }

    /// Marks a short "not interested" for `user_id`: it leaves their feeds,
    /// and its channel and hashtags (as they are now) rank lower in their
    /// for-you feed for a while.
    pub async fn hide(&self, user_id: Uuid, short: &ShortRow) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO short_hidden (user_id, short_id, channel_id, hashtags) VALUES ($1, $2, $3, $4)
             ON CONFLICT (user_id, short_id) DO NOTHING",
        )
        .bind(user_id)
        .bind(short.id)
        .bind(short.channel_id)
        .bind(&short.hashtags)
        .execute(&self.core.db)
        .await?;
        signals::track(&self.core, "short.not_interested", short, Some(user_id), 1.0, serde_json::json!({})).await;
        Ok(())
    }

    /// Takes a "not interested" back, with the down-weighting that came from it.
    pub async fn unhide(&self, user_id: Uuid, short_id: Uuid) -> AppResult<()> {
        sqlx::query("DELETE FROM short_hidden WHERE user_id = $1 AND short_id = $2")
            .bind(user_id)
            .bind(short_id)
            .execute(&self.core.db)
            .await?;
        Ok(())
    }

    /// Records that `user_id` shared `short` towards `target` (a
    /// `ShareTarget` number) and returns the share count. An account counts
    /// once a week per short. A visitor's share (`None`) counts for
    /// nothing, neither in the number shown nor in the ranking: without an
    /// account there is no telling one sharer from a thousand.
    pub async fn share(&self, user_id: Option<Uuid>, short: &ShortRow, target: i32) -> AppResult<i64> {
        let core = &self.core;
        let Some(sharer) = user_id else { return Ok(short.share_count) };
        if !self.count_signal(short, Engagement::Share, sharer).await? {
            return Ok(short.share_count);
        }
        let written = async {
            let mut tx = core.db.begin().await?;
            let share_count: Option<i64> = sqlx::query_scalar(
                "UPDATE shorts SET share_count = share_count + 1 WHERE id = $1 RETURNING share_count",
            )
            .bind(short.id)
            .fetch_optional(&mut *tx)
            .await?;
            let share_count = share_count.ok_or_else(crate::shorts::not_found)?;
            signals::bump_hourly(&mut *tx, short.id, Engagement::Share, 1).await?;
            tx.commit().await?;
            AppResult::Ok(share_count)
        }
        .await;
        let share_count = match written {
            Ok(share_count) => share_count,
            Err(error) => {
                self.release_signal(short, Engagement::Share, sharer).await;
                return Err(error);
            }
        };
        signals::track(core, "short.share", short, user_id, 1.0, serde_json::json!({ "target": target })).await;
        Ok(share_count)
    }
}
