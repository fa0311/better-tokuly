//! What happens when things disappear: moderation takes content down, an
//! account is erased, one user blocks another, an upload is deleted by
//! force. These are the reactions to the events this domain consumes; every
//! one is idempotent, so a redelivered event changes nothing.
use sqlx::PgConnection;
use tokuly_core::{events, prelude::*};
use tokuly_media::assets;

use crate::{
    Engagement,
    comments::{image_holder, lifecycle, lock_thread, lock_threads_of, status, write::comment_event},
    effects::{Effects, Push},
    events::{COMMENT_DELETED, ContentRemoved, ReportResolved},
    reactions::{self, Stored},
    reports,
    stats::{self, Counter},
    subjects::SubjectKey,
};

/// Rows handled per transaction when an account's data is erased.
const ERASE_BATCH: i64 = 200;

/// Drops the engagement data that has one of these subjects as its target:
/// bookmarks, reactions, share counts and counters. Comments written on the
/// subjects are the caller's business.
///
/// The order is the one every change follows — bookmark collections (by
/// ID), then the rows counted, then the statistics row — so a purge and a
/// user's tap on the same subject wait for each other instead of
/// deadlocking.
pub(crate) async fn forget_subjects(conn: &mut PgConnection, kind: i32, subject_ids: &[Uuid]) -> AppResult<()> {
    if subject_ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "SELECT id FROM bookmark_collections
         WHERE id IN (SELECT collection_id FROM bookmarks WHERE subject_type = $1 AND subject_id = ANY($2))
         ORDER BY id FOR UPDATE",
    )
    .bind(kind)
    .bind(subject_ids)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "WITH gone AS (
             DELETE FROM bookmarks WHERE subject_type = $1 AND subject_id = ANY($2) RETURNING collection_id
         )
         UPDATE bookmark_collections c SET item_count = GREATEST(c.item_count - g.items, 0)
         FROM (SELECT collection_id, count(*)::integer AS items FROM gone GROUP BY collection_id) g
         WHERE c.id = g.collection_id",
    )
    .bind(kind)
    .bind(subject_ids)
    .execute(&mut *conn)
    .await?;
    for table in ["reactions", "reaction_emoji_counts", "share_counts", "subject_stats"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM {table} WHERE subject_type = $1 AND subject_id = ANY($2)"
        )))
        .bind(kind)
        .bind(subject_ids)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// Removes everything attached to a subject that is gone: the comments on
/// it (with what hangs off them) and its own reactions, bookmarks, shares
/// and counters. Nobody is watching a removed subject, so nothing is pushed
/// except the review-queue count of owners who had comments waiting there.
pub(crate) async fn purge_subject(conn: &mut PgConnection, effects: &mut Effects, key: SubjectKey) -> AppResult<()> {
    let comments: Vec<(Uuid, Uuid, i32)> = sqlx::query_as(
        "SELECT id, subject_owner_id, status FROM comments WHERE subject_type = $1 AND subject_id = $2 ORDER BY id FOR UPDATE",
    )
    .bind(key.kind)
    .bind(key.id)
    .fetch_all(&mut *conn)
    .await?;
    if !comments.is_empty() {
        for (_, owner_id, _) in comments.iter().filter(|(_, _, state)| *state == status::HELD) {
            effects.review_queue(*owner_id);
        }
        let comment_ids: Vec<Uuid> = comments.iter().map(|(id, _, _)| *id).collect();
        lifecycle::release(conn, &comment_ids).await?;
        sqlx::query("DELETE FROM comments WHERE subject_type = $1 AND subject_id = $2")
            .bind(key.kind)
            .bind(key.id)
            .execute(&mut *conn)
            .await?;
    }
    forget_subjects(conn, key.kind, &[key.id]).await
}

impl Engagement {
    /// `admin.content.removed`: a moderator took content down. A comment is
    /// deleted with its replies; for anything else, what this domain kept
    /// about it is purged. Either way the open reports about it are
    /// answered: action was taken.
    pub(crate) async fn on_content_removed(&self, removed: ContentRemoved) -> AppResult<()> {
        let key = SubjectKey { kind: removed.subject_type, id: removed.subject_id };
        let mut tx = self.core.db.begin().await?;
        let mut effects = Effects::default();
        if key.is_comment() {
            // Already gone (or the event came twice): nothing to delete.
            if let Some((row, mut parent)) = lock_thread(&mut tx, key.id).await? {
                lifecycle::delete(&mut tx, &mut effects, &row, parent.as_mut()).await?;
                events::emit(&mut *tx, COMMENT_DELETED, removed.moderator_id, &comment_event(&row)).await?;
            }
        } else {
            purge_subject(&mut tx, &mut effects, key).await?;
        }
        reports::resolve_for_subject(&mut tx, key).await?;
        tx.commit().await?;
        self.apply(effects).await;
        if !key.is_comment() {
            // The subject is gone: whoever followed its counters stops.
            self.core.rt.close_topic(&key.topic()).await;
        }
        Ok(())
    }

    /// `admin.report.resolved`: the moderation team decided about reports.
    pub(crate) async fn on_report_resolved(&self, resolved: ReportResolved) -> AppResult<()> {
        self.resolve_reports(&resolved.report_ids, resolved.action_taken).await?;
        Ok(())
    }

    /// `social.user.blocked`: what the blocked user wrote on the blocker's
    /// subjects leaves public view (and the review queue) for everyone, not
    /// just for the two of them. Lifting the block brings nothing back by
    /// itself; the owner can approve the comments again.
    pub(crate) async fn on_user_blocked(&self, blocker_id: Uuid, blocked_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let mut effects = Effects::default();
        lifecycle::hide_comments_of(&mut tx, &mut effects, blocker_id, blocked_id, None).await?;
        tx.commit().await?;
        self.apply(effects).await;
        self.recheck_subject_topics(blocker_id, blocked_id).await;
        Ok(())
    }

    /// `social.friend.removed`: friends-only subjects of either are no
    /// longer the other's to follow.
    pub(crate) async fn on_friend_removed(&self, user_id: Uuid, friend_id: Uuid) {
        self.recheck_subject_topics(user_id, friend_id).await;
    }

    /// Each of the two may follow `subject:` topics of the other's things;
    /// whether they still may is the subject's owner domain's call, which
    /// the gateway asks through the guard.
    async fn recheck_subject_topics(&self, a: Uuid, b: Uuid) {
        self.core.rt.revoke_prefix(a, crate::subjects::TOPIC_KIND).await;
        self.core.rt.revoke_prefix(b, crate::subjects::TOPIC_KIND).await;
    }

    /// `media.asset.deleted`: the picture of a comment is gone. A comment
    /// that was only the picture goes with it; one with text loses the
    /// picture and stays.
    ///
    /// A safety net: a comment holds its upload, so this only happens when
    /// some domain deleted the upload regardless of holders (a takedown).
    pub(crate) async fn on_asset_deleted(&self, asset_id: Uuid) -> AppResult<()> {
        let comment_ids: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM comments WHERE image_asset_id = $1 ORDER BY id")
                .bind(asset_id)
                .fetch_all(&self.core.db)
                .await?;
        for comment_id in comment_ids {
            let mut tx = self.core.db.begin().await?;
            let mut effects = Effects::default();
            let Some((row, mut parent)) = lock_thread(&mut tx, comment_id).await? else { continue };
            if row.image_asset_id != Some(asset_id) {
                continue;
            }
            if row.text.is_empty() {
                lifecycle::delete(&mut tx, &mut effects, &row, parent.as_mut()).await?;
                events::emit(&mut *tx, COMMENT_DELETED, None, &comment_event(&row)).await?;
            } else {
                sqlx::query("UPDATE comments SET image = NULL, image_asset_id = NULL WHERE id = $1")
                    .bind(row.id)
                    .execute(&mut *tx)
                    .await?;
                assets::detach(&mut *tx, asset_id, &image_holder(row.id)).await?;
                if row.in_public_view(parent.as_ref()) {
                    effects.push(Push::Updated { key: row.key(), comment_id: row.id });
                }
            }
            tx.commit().await?;
            self.apply(effects).await;
        }
        Ok(())
    }

    /// `identity.user.deleted`: erases what the account left in this
    /// domain — its comments (with the replies below them), reactions,
    /// bookmarks, reports, moderation rules and blocks — and everything
    /// other people left on the subjects it owned, which the owning domains
    /// delete at the same moment.
    ///
    /// Works in batches, one transaction each, so a prolific account does
    /// not hold locks for long; a redelivery picks up where the last one
    /// stopped.
    pub(crate) async fn on_user_deleted(&self, user_id: Uuid) -> AppResult<()> {
        self.erase_comments_of(user_id).await?;
        self.erase_reactions_of(user_id).await?;
        self.erase_bookmarks_of(user_id).await?;
        self.erase_subjects_of(user_id).await?;

        let mut tx = self.core.db.begin().await?;
        for sql in [
            "DELETE FROM reports WHERE reporter_id = $1",
            // Reports about them stay with the moderation team, without the name.
            "UPDATE reports SET subject_owner_id = NULL WHERE subject_owner_id = $1",
            "DELETE FROM comment_mentions WHERE user_id = $1",
            "DELETE FROM comment_settings WHERE user_id = $1",
            "DELETE FROM comment_blocks WHERE owner_id = $1 OR blocked_user_id = $1",
        ] {
            sqlx::query(sql).bind(user_id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn erase_comments_of(&self, user_id: Uuid) -> AppResult<()> {
        loop {
            // Oldest first, so threads go before the replies inside them.
            let comment_ids: Vec<Uuid> =
                sqlx::query_scalar("SELECT id FROM comments WHERE author_id = $1 ORDER BY id LIMIT $2")
                    .bind(user_id)
                    .bind(ERASE_BATCH)
                    .fetch_all(&self.core.db)
                    .await?;
            if comment_ids.is_empty() {
                return Ok(());
            }
            let mut tx = self.core.db.begin().await?;
            let mut effects = Effects::default();
            lock_threads_of(&mut tx, &comment_ids).await?;
            for comment_id in comment_ids {
                if let Some((row, mut parent)) = lock_thread(&mut tx, comment_id).await? {
                    lifecycle::delete(&mut tx, &mut effects, &row, parent.as_mut()).await?;
                }
            }
            tx.commit().await?;
            self.apply(effects).await;
        }
    }

    async fn erase_reactions_of(&self, user_id: Uuid) -> AppResult<()> {
        loop {
            let mut tx = self.core.db.begin().await?;
            let mut effects = Effects::default();
            // Subjects are handled in one order by everyone who handles several.
            let batch: Vec<(i32, Uuid)> = sqlx::query_as(
                "SELECT subject_type, subject_id FROM reactions WHERE user_id = $1 ORDER BY subject_type, subject_id LIMIT $2",
            )
            .bind(user_id)
            .bind(ERASE_BATCH)
            .fetch_all(&mut *tx)
            .await?;
            if batch.is_empty() {
                return Ok(());
            }
            // Liked comments are locked before their reactions go, as in every other change to them.
            let comment_ids: Vec<Uuid> = batch
                .iter()
                .map(|(kind, id)| SubjectKey { kind: *kind, id: *id })
                .filter(SubjectKey::is_comment)
                .map(|key| key.id)
                .collect();
            sqlx::query("SELECT id FROM comments WHERE id = ANY($1) ORDER BY id FOR UPDATE")
                .bind(&comment_ids)
                .execute(&mut *tx)
                .await?;
            let (kinds, ids): (Vec<i32>, Vec<Uuid>) = batch.into_iter().unzip();
            let mut removed: Vec<(i32, Uuid, i32, String)> = sqlx::query_as(
                "DELETE FROM reactions
                 WHERE user_id = $1 AND (subject_type, subject_id) IN (SELECT * FROM unnest($2::integer[], $3::uuid[]))
                 RETURNING subject_type, subject_id, kind, emoji",
            )
            .bind(user_id)
            .bind(&kinds)
            .bind(&ids)
            .fetch_all(&mut *tx)
            .await?;
            removed.sort();
            for (kind, id, reaction_kind, emoji) in removed {
                let key = SubjectKey { kind, id };
                reactions::uncount(&mut tx, &mut effects, key, &Stored { kind: reaction_kind, emoji }).await?;
            }
            tx.commit().await?;
            self.apply(effects).await;
        }
    }

    async fn erase_bookmarks_of(&self, user_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let mut effects = Effects::default();
        // Collections first, then the counters of what they held.
        sqlx::query("SELECT id FROM bookmark_collections WHERE user_id = $1 ORDER BY id FOR UPDATE")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        // A subject is saved when it is in the default collection.
        let saved: Vec<(i32, Uuid)> = sqlx::query_as(
            "SELECT b.subject_type, b.subject_id FROM bookmarks b JOIN bookmark_collections c ON c.id = b.collection_id
             WHERE c.user_id = $1 AND c.is_default ORDER BY b.subject_type, b.subject_id",
        )
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?;
        for (kind, id) in saved {
            let key = SubjectKey { kind, id };
            stats::add(&mut tx, key, None, Counter::Bookmark, -1).await?;
            effects.count(key, Counter::Bookmark, -1);
        }
        sqlx::query("DELETE FROM bookmark_collections WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
        tx.commit().await?;
        self.apply(effects).await;
        Ok(())
    }

    /// Purges what was attached to the subjects the account owned.
    async fn erase_subjects_of(&self, user_id: Uuid) -> AppResult<()> {
        loop {
            // Subjects with numbers, and subjects that only have comments
            // waiting for review (those have no numbers yet).
            let subjects: Vec<(i32, Uuid)> = sqlx::query_as(
                "SELECT subject_type, subject_id FROM subject_stats WHERE owner_id = $1
                 UNION
                 SELECT subject_type, subject_id FROM comments WHERE subject_owner_id = $1
                 ORDER BY subject_type, subject_id LIMIT $2",
            )
            .bind(user_id)
            .bind(ERASE_BATCH)
            .fetch_all(&self.core.db)
            .await?;
            if subjects.is_empty() {
                return Ok(());
            }
            let mut tx = self.core.db.begin().await?;
            let mut effects = Effects::default();
            for (kind, id) in subjects {
                purge_subject(&mut tx, &mut effects, SubjectKey { kind, id }).await?;
            }
            tx.commit().await?;
            self.apply(effects).await;
        }
    }
}
