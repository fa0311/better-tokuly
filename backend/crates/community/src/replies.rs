//! Replies to threads.
use std::time::Duration;

use sqlx::{Postgres, Transaction};
use tokuly_core::{analytics::AnalyticsEvent, events as core_events, outbound, prelude::*};
use tokuly_media::assets;
use tokuly_proto::{
    common::PageInfo,
    community::{self as pb, ModerationAction, Permission},
};
use tokuly_social::graph;

use crate::{
    Community,
    access::{Access, Caller, permission_denied},
    events::{self, PostCreated},
    model::{
        ChannelRow, CommunityRow, MAX_EDITS, ReplyRow, ThreadRow, hidden, level, reply_select, revised, state, to_json,
    },
    modlog::{self, LogEntry},
    notices::{self, PostNotice},
    posting::{self, reply_holder},
    render::View,
    threads::{record_revision, thread_not_found},
    validate,
};

/// Followers told about one reply at most; the rest hear of the next one.
const MAX_REPLY_NOTICES: i64 = 2000;

pub fn reply_not_found() -> AppError {
    AppError::not_found("REPLY_NOT_FOUND", "reply not found")
}

/// Adjusts a thread's reply count (and with it the hot score) by `delta`.
pub(crate) async fn count_reply(tx: &mut Transaction<'_, Postgres>, thread_id: Uuid, delta: i32) -> AppResult<()> {
    sqlx::query(
        "UPDATE community_threads SET
             reply_count = greatest(reply_count + $2, 0),
             hot_score = community_hot_score(greatest(reply_count + $2, 0), like_count, comment_count, created_at,
                                             last_activity_at, now())
         WHERE id = $1",
    )
    .bind(thread_id)
    .bind(delta)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

impl Community {
    /// A reply that is not deleted, of a thread that is not deleted.
    pub(crate) async fn reply_by_id(&self, reply_id: Uuid) -> AppResult<Option<ReplyRow>> {
        Ok(sqlx::query_as(concat!(
            reply_select!(),
            "WHERE r.id = $1 AND r.deleted_at IS NULL AND t.deleted_at IS NULL"
        ))
        .bind(reply_id)
        .fetch_optional(&self.core.db)
        .await?)
    }

    /// A reply by ID, a removed one (or one of a removed thread) included
    /// for as long as it is retained. For moderation and reports only.
    pub(crate) async fn reply_retained(&self, reply_id: Uuid) -> AppResult<Option<ReplyRow>> {
        Ok(sqlx::query_as(concat!(reply_select!(), "WHERE r.id = $1 AND r.purged_at IS NULL"))
            .bind(reply_id)
            .fetch_optional(&self.core.db)
            .await?)
    }

    /// The reply named by `reply_id` as `caller` may see it, with the
    /// caller's standing in its community and their blocks.
    pub(crate) async fn open_reply(&self, caller: &Caller, reply_id: &str) -> AppResult<(ReplyRow, Access, Vec<Uuid>)> {
        let id = Uuid::parse_str(reply_id.trim()).map_err(|_| reply_not_found())?;
        let reply = self.reply_by_id(id).await?.ok_or_else(reply_not_found)?;
        let access = match self.access_by_id(caller, reply.community_id).await {
            Ok(access) => access,
            Err(error) if error.is_not_found() => return Err(reply_not_found()),
            Err(error) => return Err(error),
        };
        access.require_read()?;
        let blocked = self.blocked(caller.user_id).await?;
        if !(View { access: &access, blocked: &blocked }).sees_reply(&reply) {
            return Err(reply_not_found());
        }
        Ok((reply, access, blocked))
    }

    /// Replies to `thread`, optionally to another reply of it.
    pub(crate) async fn create_reply(
        &self,
        view: &View<'_>,
        thread: &ThreadRow,
        req: &pb::CreateReplyRequest,
    ) -> AppResult<ReplyRow> {
        let core = &self.core;
        let access = view.access;
        let community = &access.community;
        let member = access.require_active()?;
        let author = member.user_id;
        let channel = self.channel_by_id(thread.channel_id).await?.ok_or_else(thread_not_found)?;
        ensure!(channel.open_to(member.role()), denied, "POSTING_RESTRICTED", "only moderators post in this channel");
        ensure!(!thread.is_locked(), precondition, "THREAD_LOCKED", "this thread is locked");
        ensure!(thread.is_visible(), precondition, "THREAD_HIDDEN", "this thread is waiting for review");
        // The thread decides whether its replies are anonymous: a thread
        // keeps the mode it was started in.
        let channel = ChannelRow { anonymous: thread.anonymous, ..channel };
        self.ensure_anonymity(community, &channel).await?;
        let body = validate::body(&req.body, validate::MAX_REPLY_BODY_CHARS)?;
        let images = self.resolve_images(author, &req.asset_ids, thread.anonymous).await?;
        let stamp_id = self.resolve_stamp(author, &req.stamp_id, thread.anonymous).await?;
        ensure!(
            !body.is_empty() || !images.asset_ids.is_empty() || stamp_id.is_some(),
            invalid,
            "REPLY_EMPTY",
            "a reply needs text, images or a stamp"
        );
        // The reply answered must be one the author can see, in this thread.
        let reply_to: Option<(Uuid, Uuid)> = match parse_opt_id(&req.reply_to_id)? {
            Some(id) => {
                let answered: Option<(Uuid, bool)> = sqlx::query_as(
                    "SELECT author_id, anonymous FROM community_replies
                     WHERE id = $1 AND thread_id = $2 AND deleted_at IS NULL AND state = 1",
                )
                .bind(id)
                .bind(thread.id)
                .fetch_optional(&core.db)
                .await?;
                let (answered_author, anonymous) = answered.ok_or_else(reply_not_found)?;
                if view.hides(answered_author, anonymous) {
                    return Err(reply_not_found());
                }
                Some((id, answered_author))
            }
            None => None,
        };
        let mentions = self.resolve_mentions(community.id, author, &body).await?;
        let (post_state, hidden_reason) = posting::screening(community, access, thread.anonymous, &[&body]);
        let visible = post_state == state::VISIBLE;

        let slot = self.admit_post(access, &channel).await?;
        let stored: AppResult<Uuid> = async {
            let mentioned = if visible {
                self.mention_recipients(community.id, &mentions, author, thread.anonymous, &[]).await?
            } else {
                Vec::new()
            };
            // Followers are not told twice (a mention is the stronger notice)
            // and never across a block — except in anonymous threads, where
            // a missing notification would tell who wrote the reply.
            let mut skip = mentioned.clone();
            if !thread.anonymous {
                skip.extend(graph::blocked_ids_either(core, author).await?);
            }
            let staff = if visible { Vec::new() } else { self.staff_ids(community.id).await? };
            let speaker = self.speaker(thread.id, author, thread.anonymous).await?;

            let mut tx = core.db.begin().await?;
            // Numbering the reply locks the thread row: replies to one
            // thread queue up, and a lock or deletion in the meantime wins.
            let seq: Option<i32> = sqlx::query_scalar(
                "UPDATE community_threads SET
                     reply_seq = reply_seq + 1,
                     reply_count = reply_count + $2::int,
                     last_activity_at = CASE WHEN $2 THEN now() ELSE last_activity_at END,
                     hot_score = community_hot_score(reply_count + $2::int, like_count, comment_count, created_at,
                                                     CASE WHEN $2 THEN now() ELSE last_activity_at END, now())
                 WHERE id = $1 AND deleted_at IS NULL AND locked_at IS NULL AND state = 1
                 RETURNING reply_seq",
            )
            .bind(thread.id)
            .bind(visible)
            .fetch_optional(&mut *tx)
            .await?;
            let seq =
                seq.ok_or_else(|| AppError::precondition("THREAD_LOCKED", "this thread no longer takes replies"))?;
            let reply_id: Uuid = sqlx::query_scalar(
                "INSERT INTO community_replies
                     (thread_id, community_id, author_id, anonymous, seq, body, mentions, images, asset_ids, stamp_id,
                      reply_to_id, state, hidden_reason, hidden_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, CASE WHEN $12 = 2 THEN now() END)
                 RETURNING id",
            )
            .bind(thread.id)
            .bind(community.id)
            .bind(author)
            .bind(thread.anonymous)
            .bind(seq)
            .bind(&body)
            .bind(to_json(&mentions)?)
            .bind(to_json(&images.images)?)
            .bind(&images.asset_ids)
            .bind(stamp_id)
            .bind(reply_to.map(|(id, _)| id))
            .bind(post_state)
            .bind(hidden_reason)
            .fetch_one(&mut *tx)
            .await?;
            for asset_id in &images.asset_ids {
                assets::attach(&mut *tx, *asset_id, &reply_holder(reply_id)).await?;
            }
            // Whoever replies follows the thread, unless they chose not to before.
            sqlx::query(
                "INSERT INTO community_thread_follows (thread_id, user_id) VALUES ($1, $2)
                 ON CONFLICT (thread_id, user_id) DO NOTHING",
            )
            .bind(thread.id)
            .bind(author)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO community_thread_reads (thread_id, user_id) VALUES ($1, $2)
                 ON CONFLICT (thread_id, user_id) DO UPDATE SET read_at = now()",
            )
            .bind(thread.id)
            .bind(author)
            .execute(&mut *tx)
            .await?;
            let reply: ReplyRow =
                sqlx::query_as(concat!(reply_select!(), "WHERE r.id = $1")).bind(reply_id).fetch_one(&mut *tx).await?;
            if visible {
                sqlx::query("UPDATE community_channels SET last_activity_at = now() WHERE id = $1")
                    .bind(thread.channel_id)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("UPDATE communities SET last_activity_at = now() WHERE id = $1")
                    .bind(community.id)
                    .execute(&mut *tx)
                    .await?;
                let (actor, event) = created_event(community, thread, &reply);
                core_events::emit(&mut *tx, events::REPLY_CREATED, actor, &event).await?;
                for recipient in &mentioned {
                    let notice = notices::mention(*recipient, &speaker, community, thread, Some(&reply));
                    outbound::notify(&mut *tx, &notice).await?;
                }
                // One notice per follower until they read the thread again;
                // the author of the reply answered is always told.
                let followers: Vec<Uuid> = sqlx::query_scalar(
                    "UPDATE community_thread_follows f SET notified_at = now()
                     WHERE f.thread_id = $1 AND f.user_id IN (
                         SELECT f2.user_id FROM community_thread_follows f2
                         JOIN community_members m
                           ON m.community_id = $2 AND m.user_id = f2.user_id AND m.notification_level = $6
                         WHERE f2.thread_id = $1 AND f2.following AND f2.user_id <> $3
                           AND (f2.notified_at IS NULL OR f2.user_id = $4)
                           AND NOT (f2.user_id = ANY($5))
                         LIMIT $7)
                     RETURNING f.user_id",
                )
                .bind(thread.id)
                .bind(community.id)
                .bind(author)
                .bind(reply_to.map(|(_, answered_author)| answered_author))
                .bind(&skip)
                .bind(level::ALL)
                .bind(MAX_REPLY_NOTICES)
                .fetch_all(&mut *tx)
                .await?;
                for recipient in followers {
                    outbound::notify(&mut *tx, &notices::reply(recipient, &speaker, community, thread, &reply)).await?;
                }
            } else {
                self.record_held(&mut tx, community, thread, Some((reply_id, author, thread.anonymous)), &staff)
                    .await?;
            }
            tx.commit().await?;
            Ok(reply_id)
        }
        .await;
        let reply_id = match stored {
            Ok(reply_id) => reply_id,
            Err(error) => {
                self.release_slot(slot).await;
                return Err(error);
            }
        };

        // From here on nothing can fail the post.
        let reply = self.reply_by_id(reply_id).await?.ok_or_else(reply_not_found)?;
        self.record_stamp(author, reply.stamp_id).await;
        if visible {
            self.push_reply_created(community, &reply).await;
        } else {
            self.push_staff_queues(community.id).await;
        }
        core.analytics
            .track(
                AnalyticsEvent::new("community.reply")
                    .user((!reply.anonymous).then_some(author))
                    .subject("community_reply", reply.id)
                    .props(serde_json::json!({
                        "community_id": community.id,
                        "thread_id": thread.id,
                        "anonymous": reply.anonymous,
                        "images": reply.asset_ids.len(),
                        "held": !visible,
                    })),
            )
            .await;
        Ok(reply)
    }

    /// The thread's current reply count, for pushes.
    async fn reply_count(&self, thread_id: Uuid) -> i32 {
        sqlx::query_scalar("SELECT reply_count FROM community_threads WHERE id = $1")
            .bind(thread_id)
            .fetch_optional(&self.core.db)
            .await
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    /// Announces a reply on the community's topic (see `push_thread_created`
    /// for when the reply itself is left out).
    pub(crate) async fn push_reply_created(&self, community: &CommunityRow, reply: &ReplyRow) {
        let payload = match self.pushable(community, reply.author_id, reply.anonymous).await {
            true => self.render_reply(None, reply).await.ok(),
            false => None,
        };
        let event = pb::ReplyCreated {
            community_id: community.id.to_string(),
            thread_id: reply.thread_id.to_string(),
            reply_id: reply.id.to_string(),
            reply: payload.into(),
            reply_count: self.reply_count(reply.thread_id).await,
            ..Default::default()
        };
        self.push_topic(community.id, event).await;
    }

    pub(crate) async fn push_reply_removed(&self, reply: &ReplyRow) {
        let event = pb::ReplyRemoved {
            community_id: reply.community_id.to_string(),
            thread_id: reply.thread_id.to_string(),
            reply_id: reply.id.to_string(),
            reply_count: self.reply_count(reply.thread_id).await,
            ..Default::default()
        };
        self.push_topic(reply.community_id, event).await;
    }

    /// Edits the body of the caller's own reply.
    pub(crate) async fn update_reply(&self, access: &Access, reply: &ReplyRow, body: &str) -> AppResult<ReplyRow> {
        let core = &self.core;
        let community = &access.community;
        let member = access.require_active()?;
        if reply.author_id != member.user_id {
            return Err(permission_denied());
        }
        ensure!(reply.thread_locked_at.is_none(), precondition, "THREAD_LOCKED", "this thread is locked");
        let body = validate::body(body, validate::MAX_REPLY_BODY_CHARS)?;
        ensure!(
            !body.is_empty() || !reply.asset_ids.is_empty() || reply.stamp_id.is_some(),
            invalid,
            "REPLY_EMPTY",
            "a reply needs text, images or a stamp"
        );
        let reworded = body != reply.body;
        ensure!(
            !reworded || reply.edit_count < MAX_EDITS,
            precondition,
            "TOO_MANY_EDITS",
            "a reply can be edited {MAX_EDITS} times"
        );
        self.limit("edit", member.user_id, 30, Duration::from_secs(60)).await?;
        let thread = self.thread_by_id(reply.thread_id).await?.ok_or_else(reply_not_found)?;
        let mentions = self.resolve_mentions(community.id, member.user_id, &body).await?;
        let (screened, _) = posting::screening(community, access, reply.anonymous, &[&body]);
        let hold = reply.is_visible() && screened == state::HIDDEN && reply.reviewed_at.is_none();
        let already_told: Vec<Uuid> =
            reply.mentions().iter().filter_map(|mention| Uuid::parse_str(&mention.user_id).ok()).collect();
        let recipients = if reply.is_visible() && !hold {
            self.mention_recipients(community.id, &mentions, member.user_id, reply.anonymous, &already_told).await?
        } else {
            Vec::new()
        };
        let staff = if hold { self.staff_ids(community.id).await? } else { Vec::new() };
        // Read before the transaction opens: inside it nothing waits for a
        // second connection.
        let speaker = self.speaker(thread.id, reply.author_id, reply.anonymous).await?;

        let mut tx = core.db.begin().await?;
        let updated = sqlx::query(
            "UPDATE community_replies SET
                 body = $2, mentions = $3, edited_at = now(),
                 edit_count = edit_count + $6::int,
                 state = CASE WHEN $4 THEN 2 ELSE state END,
                 hidden_reason = CASE WHEN $4 THEN $5 ELSE hidden_reason END,
                 hidden_at = CASE WHEN $4 THEN now() ELSE hidden_at END
             WHERE id = $1 AND deleted_at IS NULL AND (NOT $6 OR edit_count < $7)",
        )
        .bind(reply.id)
        .bind(&body)
        .bind(to_json(&mentions)?)
        .bind(hold)
        .bind(hidden::MUTED_WORD)
        .bind(reworded)
        .bind(MAX_EDITS)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() == 0 {
            // Deleted meanwhile, or two edits at once went past the cap.
            return Err(if reworded {
                AppError::precondition("TOO_MANY_EDITS", "this reply cannot be edited any more")
            } else {
                reply_not_found()
            });
        }
        if reworded {
            record_revision(&mut tx, revised::REPLY, reply.id, "", &reply.body).await?;
        }
        let updated: ReplyRow =
            sqlx::query_as(concat!(reply_select!(), "WHERE r.id = $1")).bind(reply.id).fetch_one(&mut *tx).await?;
        if hold {
            count_reply(&mut tx, reply.thread_id, -1).await?;
            self.record_held(&mut tx, community, &thread, Some((reply.id, reply.author_id, reply.anonymous)), &staff)
                .await?;
        }
        for recipient in recipients {
            let notice = notices::mention(recipient, &speaker, community, &thread, Some(&updated));
            outbound::notify(&mut *tx, &notice).await?;
        }
        tx.commit().await?;

        if hold {
            self.push_reply_removed(&updated).await;
            self.push_staff_queues(community.id).await;
        } else if updated.is_visible() {
            let payload = match self.pushable(community, updated.author_id, updated.anonymous).await {
                true => self.render_reply(None, &updated).await.ok(),
                false => None,
            };
            let event = pb::ReplyUpdated {
                community_id: community.id.to_string(),
                thread_id: updated.thread_id.to_string(),
                reply_id: updated.id.to_string(),
                reply: payload.into(),
                ..Default::default()
            };
            self.push_topic(community.id, event).await;
        }
        Ok(updated)
    }

    /// Takes a reply out of view inside `tx`. False when it was gone
    /// already. Like a removed thread it keeps its content and uploads for
    /// the retention period.
    pub(crate) async fn remove_reply(&self, tx: &mut Transaction<'_, Postgres>, reply: &ReplyRow) -> AppResult<bool> {
        let removed: Option<i32> = sqlx::query_scalar(
            "UPDATE community_replies SET deleted_at = now() WHERE id = $1 AND deleted_at IS NULL RETURNING state",
        )
        .bind(reply.id)
        .fetch_optional(&mut **tx)
        .await?;
        let Some(was) = removed else { return Ok(false) };
        if was == state::VISIBLE {
            count_reply(tx, reply.thread_id, -1).await?;
        }
        Ok(true)
    }

    /// Deletes a reply: its author, or a moderator with a reason.
    pub(crate) async fn delete_reply(&self, access: &Access, reply: &ReplyRow, reason: &str) -> AppResult<()> {
        let community = &access.community;
        let actor = access.require_member().map_err(|_| permission_denied())?.user_id;
        let own = reply.author_id == actor;
        if !own {
            access.require(Permission::PERMISSION_MODERATE_CONTENT)?;
        }
        let reason = validate::reason(reason, !own)?;
        if !own {
            self.staff_limit(actor).await?;
        }
        let mut tx = self.core.db.begin().await?;
        let removed = self.remove_reply(&mut tx, reply).await?;
        if removed && !own {
            let entry = LogEntry::new(community.id, ModerationAction::MODERATION_ACTION_REPLY_DELETED)
                .by(actor)
                .reply(reply.thread_id, reply.id)
                .channel(reply.channel_id)
                .author(reply.author_id, reply.anonymous)
                .reason(&reason);
            modlog::record(&mut *tx, &entry).await?;
            let notice = PostNotice::Removed { reason: reason.clone() };
            let path = format!("{}/t/{}", community.path(), reply.thread_short_id);
            outbound::notify(
                &mut *tx,
                &notices::moderation(reply.author_id, community, &reply.thread_title, path, &notice),
            )
            .await?;
        }
        tx.commit().await?;
        if removed {
            self.push_reply_removed(reply).await;
            if reply.state == state::HIDDEN {
                self.push_staff_queues(community.id).await;
            }
        }
        Ok(())
    }

    /// The replies of a thread as `view` may see them, by position.
    pub(crate) async fn list_replies(
        &self,
        view: &View<'_>,
        thread: &ThreadRow,
        newest_first: bool,
        page: &Page,
    ) -> AppResult<(Vec<ReplyRow>, PageInfo)> {
        let after: Option<i32> = page.decode()?;
        let query = if newest_first {
            concat!(
                reply_select!(),
                "WHERE r.thread_id = $1 AND r.deleted_at IS NULL
                   AND (r.state = 1 OR r.author_id = $2 OR $3)
                   AND (r.anonymous OR NOT (r.author_id = ANY($4)))
                   AND ($5::int IS NULL OR r.seq < $5)
                 ORDER BY r.seq DESC LIMIT $6"
            )
        } else {
            concat!(
                reply_select!(),
                "WHERE r.thread_id = $1 AND r.deleted_at IS NULL
                   AND (r.state = 1 OR r.author_id = $2 OR $3)
                   AND (r.anonymous OR NOT (r.author_id = ANY($4)))
                   AND ($5::int IS NULL OR r.seq > $5)
                 ORDER BY r.seq LIMIT $6"
            )
        };
        let mut rows: Vec<ReplyRow> = sqlx::query_as(query)
            .bind(thread.id)
            .bind(view.user_id().unwrap_or_default())
            .bind(view.access.sees_hidden_posts())
            .bind(view.blocked)
            .bind(after)
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
        let info = page.finish(&mut rows, |row| row.seq);
        Ok((rows, info))
    }
}

/// The event payload of a reply entering public view; anonymous replies
/// carry no author and the event no actor.
pub(crate) fn created_event(
    community: &CommunityRow,
    thread: &ThreadRow,
    reply: &ReplyRow,
) -> (Option<Uuid>, PostCreated) {
    let author = (!reply.anonymous).then_some(reply.author_id);
    (
        author,
        PostCreated {
            community_id: community.id,
            channel_id: thread.channel_id,
            thread_id: thread.id,
            reply_id: Some(reply.id),
            author_id: author,
            anonymous: reply.anonymous,
            public: community.is_public(),
        },
    )
}
