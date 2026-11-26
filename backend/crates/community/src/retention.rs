//! What stays of removed content, for how long, and who may still see it.
//!
//! Deleting a post, editing it, deleting the channel it was in or closing
//! the whole community must not be a way to make harassment disappear
//! before anyone can act on it. So:
//!
//! - a removed thread, reply or event **keeps its content and uploads** for
//!   the retention period (`TOKULY_COMMUNITY_REMOVED_RETENTION_DAYS`),
//!   counted from the moment it was removed — never from when it was
//!   written, which would purge an old post the moment it is deleted;
//! - members no longer see it, but whoever could read it can still
//!   **report** it (the subject resolvers keep resolving it, see `subjects`);
//! - the staff and site moderation can still **read** it, with what it said
//!   before each edit ([`Community::content_history`]), and still **look up
//!   its author** when it was anonymous (`RevealAuthor`), and so ban them;
//! - after the retention period [`Community::purge_removed`] blanks the
//!   content, releases the uploads and drops the edit history; a closed
//!   community is then deleted altogether, memberships and log included.
use sqlx::{Postgres, Transaction};
use tokuly_core::{outbound, prelude::*};
use tokuly_media::assets;
use tokuly_proto::{
    common::SubjectType,
    community::{self as pb, content_ref::Content as ContentTarget, post_ref::Post as PostTarget},
};

use crate::{
    Community,
    access::{Access, Caller, permission_denied},
    calendar::{cover_holder, event_not_found},
    communities::{banner_holder, icon_holder},
    model::{EventRow, event_columns, revised},
    moderation::Post,
    posting::{reply_holder, thread_holder},
    render::View,
};

/// Rows purged per kind and pass of the maintenance timer.
const PURGE_BATCH: i64 = 200;

impl Post {
    pub(crate) fn community_id(&self) -> Uuid {
        match self {
            Self::Thread(thread) => thread.community_id,
            Self::Reply(reply) => reply.community_id,
        }
    }

    /// When the post left public view by removal: its own, or its thread's.
    fn removed_at(&self) -> Option<Time> {
        match self {
            Self::Thread(thread) => thread.deleted_at,
            Self::Reply(reply) => reply.deleted_at.or(reply.thread_deleted_at),
        }
    }
}

impl Community {
    /// The standing of `caller` towards the community that retained content
    /// belongs to, when they may act on it as staff there: a member of the
    /// staff of a live community, or site moderation — which is the only
    /// way into a closed one.
    async fn staff_access(&self, caller: &Caller, community_id: Uuid) -> AppResult<Option<Access>> {
        let Some(community) = self.community_retained(community_id).await? else { return Ok(None) };
        let closed = community.deleted_at.is_some();
        let access = self.stance(caller, community).await?;
        let staff = access.site_moderator || (!closed && access.moderates());
        Ok(staff.then_some(access))
    }

    /// The thread or reply a request names, as the caller may see it.
    pub(crate) async fn open_post(&self, caller: &Caller, post: Option<&pb::PostRef>) -> AppResult<(Post, Access)> {
        match post.and_then(|post| post.post.as_ref()) {
            Some(PostTarget::ThreadId(thread_id)) => {
                let (thread, access, _) = self.open_thread(caller, thread_id).await?;
                Ok((Post::Thread(thread), access))
            }
            Some(PostTarget::ReplyId(reply_id)) => {
                let (reply, access, _) = self.open_reply(caller, reply_id).await?;
                Ok((Post::Reply(reply), access))
            }
            None => Err(AppError::invalid("POST_REQUIRED", "name a thread or a reply")),
        }
    }

    /// [`Self::open_post`] for the tools that must keep working on removed
    /// posts: for the staff (and site moderation) a post that was removed
    /// is still there for as long as it is retained.
    pub(crate) async fn open_post_for_staff(
        &self,
        caller: &Caller,
        post: Option<&pb::PostRef>,
    ) -> AppResult<(Post, Access)> {
        let not_found = match self.open_post(caller, post).await {
            Err(error) if error.is_not_found() => error,
            other => return other,
        };
        let retained = match post.and_then(|post| post.post.as_ref()) {
            Some(PostTarget::ThreadId(thread_id)) => self.thread_retained(thread_id).await?.map(Post::Thread),
            Some(PostTarget::ReplyId(reply_id)) => match Uuid::parse_str(reply_id.trim()) {
                Ok(id) => self.reply_retained(id).await?.map(Post::Reply),
                Err(_) => None,
            },
            None => None,
        };
        let Some(post) = retained else { return Err(not_found) };
        match self.staff_access(caller, post.community_id()).await? {
            Some(access) => Ok((post, access)),
            None => Err(not_found),
        }
    }

    /// An event by ID, a removed one included for as long as it is retained.
    pub(crate) async fn event_retained(&self, event_id: Uuid) -> AppResult<Option<EventRow>> {
        Ok(sqlx::query_as(concat!(
            "SELECT ",
            event_columns!(),
            " FROM community_events e WHERE e.id = $1 AND e.purged_at IS NULL"
        ))
        .bind(event_id)
        .fetch_optional(&self.core.db)
        .await?)
    }

    async fn revisions(&self, kind: i32, content_id: Uuid) -> AppResult<Vec<pb::ContentRevision>> {
        let rows: Vec<(String, String, Time)> = sqlx::query_as(
            "SELECT title, body, replaced_at FROM community_revisions WHERE kind = $1 AND content_id = $2 ORDER BY id",
        )
        .bind(kind)
        .bind(content_id)
        .fetch_all(&self.core.db)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(title, body, replaced_at)| pb::ContentRevision {
                title,
                body,
                replaced_at: ts(replaced_at),
                ..Default::default()
            })
            .collect())
    }

    /// A thread, a reply or an event as the staff may read it: what it says
    /// now (also after its removal, while it is retained) and what it said
    /// before each edit. Authors of anonymous posts stay pseudonyms.
    pub(crate) async fn content_history(
        &self,
        caller: &Caller,
        content: Option<&pb::ContentRef>,
    ) -> AppResult<pb::GetContentHistoryResponse> {
        use pb::get_content_history_response::Current;
        let staff_only = |access: &Access| {
            if access.moderates() || access.site_moderator { Ok(()) } else { Err(permission_denied()) }
        };
        match content.and_then(|content| content.content.as_ref()) {
            Some(ContentTarget::ThreadId(_) | ContentTarget::ReplyId(_)) => {
                let post_ref = match content.and_then(|content| content.content.as_ref()) {
                    Some(ContentTarget::ThreadId(id)) => PostTarget::ThreadId(id.clone()),
                    Some(ContentTarget::ReplyId(id)) => PostTarget::ReplyId(id.clone()),
                    _ => unreachable!("matched above"),
                };
                let post_ref = pb::PostRef { post: Some(post_ref), ..Default::default() };
                let (post, access) = self.open_post_for_staff(caller, Some(&post_ref)).await?;
                staff_only(&access)?;
                let removed_at = post.removed_at().or(access.community.deleted_at);
                let view = View { access: &access, blocked: &[] };
                let (current, revisions) = match &post {
                    Post::Thread(thread) => (
                        Current::Thread(Box::new(self.render_thread(&access.community, Some(&view), thread).await?)),
                        self.revisions(revised::THREAD, thread.id).await?,
                    ),
                    Post::Reply(reply) => (
                        Current::Reply(Box::new(self.render_reply(Some(&view), reply).await?)),
                        self.revisions(revised::REPLY, reply.id).await?,
                    ),
                };
                Ok(pb::GetContentHistoryResponse {
                    current: Some(current),
                    removed_at: opt_ts(removed_at),
                    revisions,
                    ..Default::default()
                })
            }
            Some(ContentTarget::EventId(event_id)) => {
                let (event, access) = match self.open_event(caller, event_id).await {
                    Ok(found) => found,
                    Err(error) if error.is_not_found() => {
                        let retained = match Uuid::parse_str(event_id.trim()) {
                            Ok(id) => self.event_retained(id).await?,
                            Err(_) => None,
                        };
                        let Some(event) = retained else { return Err(event_not_found()) };
                        match self.staff_access(caller, event.community_id).await? {
                            Some(access) => (event, access),
                            None => return Err(event_not_found()),
                        }
                    }
                    Err(error) => return Err(error),
                };
                staff_only(&access)?;
                Ok(pb::GetContentHistoryResponse {
                    current: Some(Current::Event(Box::new(self.render_event(&access, &event).await?))),
                    removed_at: opt_ts(event.deleted_at.or(access.community.deleted_at)),
                    revisions: self.revisions(revised::EVENT, event.id).await?,
                    ..Default::default()
                })
            }
            None => Err(AppError::invalid("CONTENT_REQUIRED", "name a thread, a reply or an event")),
        }
    }

    /// Purges what was removed longer ago than the retention period:
    /// content is blanked, uploads are released, edit history, reports and
    /// polls go; a closed community goes entirely once its content has.
    /// The clock starts at the removal — of the post, or of the community
    /// it was in. Returns how many threads, replies and events were
    /// purged. Safe to run on several workers.
    pub async fn purge_removed(&self) -> AppResult<u64> {
        let db = &self.core.db;
        let days = self.cfg.removed_retention_days as i32;
        let mut purged = 0;

        // A closed community's content was never marked removed one by one:
        // once the community's own retention is over, it is — as of the
        // closing — and the passes below pick it up.
        let closed: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM communities
             WHERE deleted_at < now() - make_interval(days => $1) AND purged_at IS NULL ORDER BY deleted_at LIMIT $2",
        )
        .bind(days)
        .bind(PURGE_BATCH)
        .fetch_all(db)
        .await?;
        for community_id in closed {
            let mut tx = db.begin().await?;
            assets::detach_holder(&mut *tx, &icon_holder(community_id)).await?;
            assets::detach_holder(&mut *tx, &banner_holder(community_id)).await?;
            for statement in [
                "UPDATE community_threads t SET deleted_at = c.deleted_at FROM communities c
                 WHERE c.id = $1 AND t.community_id = c.id AND t.deleted_at IS NULL",
                "UPDATE community_replies r SET deleted_at = c.deleted_at FROM communities c
                 WHERE c.id = $1 AND r.community_id = c.id AND r.deleted_at IS NULL",
                "UPDATE community_events e SET deleted_at = c.deleted_at FROM communities c
                 WHERE c.id = $1 AND e.community_id = c.id AND e.deleted_at IS NULL",
                "UPDATE communities SET icon = NULL, icon_asset_id = NULL, banner = NULL, banner_asset_id = NULL,
                        description = '', rules = '[]', purged_at = now()
                 WHERE id = $1",
            ] {
                sqlx::query(statement).bind(community_id).execute(&mut *tx).await?;
            }
            outbound::subject_deleted(&mut *tx, SubjectType::SUBJECT_TYPE_COMMUNITY, community_id).await?;
            tx.commit().await?;
        }

        let threads: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM community_threads
             WHERE deleted_at < now() - make_interval(days => $1) AND purged_at IS NULL ORDER BY deleted_at LIMIT $2",
        )
        .bind(days)
        .bind(PURGE_BATCH)
        .fetch_all(db)
        .await?;
        for thread_id in threads {
            let mut tx = db.begin().await?;
            let replies: Vec<Uuid> =
                sqlx::query_scalar("SELECT id FROM community_replies WHERE thread_id = $1 AND purged_at IS NULL")
                    .bind(thread_id)
                    .fetch_all(&mut *tx)
                    .await?;
            assets::detach_holder(&mut *tx, &thread_holder(thread_id)).await?;
            for reply_id in &replies {
                purge_reply(&mut tx, *reply_id).await?;
            }
            forget(&mut tx, revised::THREAD, SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD, thread_id).await?;
            for statement in [
                "DELETE FROM community_polls WHERE thread_id = $1",
                "DELETE FROM community_thread_follows WHERE thread_id = $1",
                "DELETE FROM community_thread_reads WHERE thread_id = $1",
                "UPDATE community_threads SET title = '', body = '', mentions = '[]', images = '[]', asset_ids = '{}',
                        stamp_id = NULL, tags = '{}', purged_at = now()
                 WHERE id = $1",
            ] {
                sqlx::query(statement).bind(thread_id).execute(&mut *tx).await?;
            }
            tx.commit().await?;
            purged += 1 + replies.len() as u64;
        }

        // Replies removed on their own, in threads that are still there.
        let replies: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM community_replies
             WHERE deleted_at < now() - make_interval(days => $1) AND purged_at IS NULL ORDER BY deleted_at LIMIT $2",
        )
        .bind(days)
        .bind(PURGE_BATCH)
        .fetch_all(db)
        .await?;
        for reply_id in replies {
            let mut tx = db.begin().await?;
            purge_reply(&mut tx, reply_id).await?;
            tx.commit().await?;
            purged += 1;
        }

        let events: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM community_events
             WHERE deleted_at < now() - make_interval(days => $1) AND purged_at IS NULL ORDER BY deleted_at LIMIT $2",
        )
        .bind(days)
        .bind(PURGE_BATCH)
        .fetch_all(db)
        .await?;
        for event_id in events {
            let mut tx = db.begin().await?;
            assets::detach_holder(&mut *tx, &cover_holder(event_id)).await?;
            sqlx::query("DELETE FROM community_revisions WHERE kind = $2 AND content_id = $1")
                .bind(event_id)
                .bind(revised::EVENT)
                .execute(&mut *tx)
                .await?;
            for statement in [
                "DELETE FROM community_event_attendees WHERE event_id = $1",
                "UPDATE community_events SET title = '', description = '', place_name = '', place_address = '',
                        place_lat = NULL, place_lng = NULL, online_url = '', cover = NULL, cover_asset_id = NULL,
                        cancel_reason = '', purged_at = now()
                 WHERE id = $1",
            ] {
                sqlx::query(statement).bind(event_id).execute(&mut *tx).await?;
            }
            outbound::subject_deleted(&mut *tx, SubjectType::SUBJECT_TYPE_COMMUNITY_EVENT, event_id).await?;
            tx.commit().await?;
            purged += 1;
        }

        // A closed community whose content has all been purged goes
        // entirely, and with it everything that still said who had been
        // part of it: memberships, bans, invites, requests, the log.
        sqlx::query(
            "DELETE FROM communities WHERE id IN (
                 SELECT c.id FROM communities c
                 WHERE c.purged_at IS NOT NULL
                   AND NOT EXISTS (SELECT 1 FROM community_threads t WHERE t.community_id = c.id AND t.purged_at IS NULL)
                   AND NOT EXISTS (SELECT 1 FROM community_replies r WHERE r.community_id = c.id AND r.purged_at IS NULL)
                   AND NOT EXISTS (SELECT 1 FROM community_events e WHERE e.community_id = c.id AND e.purged_at IS NULL)
                 ORDER BY c.purged_at LIMIT 20)",
        )
        .execute(db)
        .await?;
        Ok(purged)
    }
}

/// Drops the edit history of one post and the reports that were counted
/// against it, and says that the post is gone for good (`subject.deleted`:
/// until its purge a removed post answers reports, with what hangs off it).
async fn forget(tx: &mut Transaction<'_, Postgres>, kind: i32, subject_type: SubjectType, id: Uuid) -> AppResult<()> {
    sqlx::query("DELETE FROM community_revisions WHERE kind = $2 AND content_id = $1")
        .bind(id)
        .bind(kind)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM community_reports WHERE subject_type = $2 AND subject_id = $1")
        .bind(id)
        .bind(subject_type as i32)
        .execute(&mut **tx)
        .await?;
    outbound::subject_deleted(&mut **tx, subject_type, id).await
}

/// Purges one reply: its upload holds, its history, its content.
async fn purge_reply(tx: &mut Transaction<'_, Postgres>, reply_id: Uuid) -> AppResult<()> {
    assets::detach_holder(&mut **tx, &reply_holder(reply_id)).await?;
    forget(tx, revised::REPLY, SubjectType::SUBJECT_TYPE_COMMUNITY_REPLY, reply_id).await?;
    sqlx::query(
        "UPDATE community_replies SET body = '', mentions = '[]', images = '[]', asset_ids = '{}', stamp_id = NULL,
                deleted_at = coalesce(deleted_at, now()), purged_at = now()
         WHERE id = $1",
    )
    .bind(reply_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Names used for a removed subject where its own title would show what
/// its author took back.
pub(crate) const REMOVED_POST_TITLE: &str = "削除された投稿 (deleted post)";
pub(crate) const REMOVED_EVENT_TITLE: &str = "削除されたイベント (deleted event)";
pub(crate) const CLOSED_COMMUNITY_TITLE: &str = "閉鎖されたコミュニティ (closed community)";
