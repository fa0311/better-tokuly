//! Threads: writing, editing and deleting them, the three list orders,
//! pins, locks, announcements, follows and read markers.
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::{Postgres, QueryBuilder, Transaction};
use tokuly_core::{
    analytics::AnalyticsEvent,
    events as core_events,
    ids::{Ref, new_short_id},
    outbound::{self, SearchDoc},
    prelude::*,
};
use tokuly_media::assets;
use tokuly_proto::{
    common::PageInfo,
    community::{self as pb, ModerationAction, Permission, ThreadSort, TopWindow},
};

use crate::{
    Community,
    access::{Access, Caller, permission_denied},
    events::{self, PostCreated, PostDeleted},
    model::{ChannelRow, CommunityRow, MAX_EDITS, ThreadRow, hidden, revised, state, thread_columns, to_json},
    modlog::{self, LogEntry},
    notices::{self, PostNotice, Speaker},
    posting::{self, thread_holder},
    render::View,
    validate,
    workers::AnnounceJob,
};

/// Pinned threads one channel may have.
const MAX_PINNED: i64 = 5;
/// Threads a community may turn into announcements per hour.
const ANNOUNCEMENTS_PER_HOUR: u32 = 10;

pub fn thread_not_found() -> AppError {
    AppError::not_found("THREAD_NOT_FOUND", "thread not found")
}

/// The search document of a thread. Only visible threads of public
/// communities are public; an anonymous thread has no owner in the index.
pub fn search_doc(community: &CommunityRow, thread: &ThreadRow) -> SearchDoc {
    SearchDoc {
        kind: "thread".into(),
        id: thread.id,
        title: thread.title.clone(),
        text: thread.body.clone(),
        tags: thread.tags.clone(),
        owner_id: (!thread.anonymous).then_some(thread.author_id),
        public: community.is_public() && community.deleted_at.is_none() && thread.is_visible(),
        image_url: thread.image_url(),
        path: thread.path(community),
        created_at: thread.created_at.timestamp(),
        score: thread.like_count as f64 + f64::from(thread.reply_count),
        extra: serde_json::json!({
            "community_id": community.id,
            "community_handle": community.handle,
            "community_name": community.name,
            "channel_id": thread.channel_id,
            "anonymous": thread.anonymous,
            "reply_count": thread.reply_count,
        }),
    }
}

/// The event payload of a thread entering public view. Anonymous threads
/// carry no author and the event no actor.
pub(crate) fn created_event(community: &CommunityRow, thread: &ThreadRow) -> (Option<Uuid>, PostCreated) {
    let author = (!thread.anonymous).then_some(thread.author_id);
    (
        author,
        PostCreated {
            community_id: community.id,
            channel_id: thread.channel_id,
            thread_id: thread.id,
            reply_id: None,
            author_id: author,
            anonymous: thread.anonymous,
            public: community.is_public(),
        },
    )
}

/// Keyset of the three list orders.
#[derive(Debug, Serialize, Deserialize)]
enum ThreadCursor {
    Hot(f64, Uuid),
    New(Uuid),
    Top(i64, i32, Uuid),
}

/// What a thread list is narrowed to.
#[derive(Debug)]
pub(crate) struct ThreadFilter {
    pub channel_id: Option<Uuid>,
    pub sort: ThreadSort,
    pub window: TopWindow,
    pub tag: Option<String>,
}

/// Adds or removes a thread from the counters of its channel and community;
/// `touch` also marks both as just active.
pub(crate) async fn count_thread(
    tx: &mut Transaction<'_, Postgres>,
    thread: &ThreadRow,
    delta: i32,
    touch: bool,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE community_channels SET thread_count = greatest(thread_count + $2, 0),
                last_activity_at = CASE WHEN $3 THEN now() ELSE last_activity_at END
         WHERE id = $1",
    )
    .bind(thread.channel_id)
    .bind(delta)
    .bind(touch)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE communities SET thread_count = greatest(thread_count + $2, 0),
                last_activity_at = CASE WHEN $3 THEN now() ELSE last_activity_at END
         WHERE id = $1",
    )
    .bind(thread.community_id)
    .bind(delta)
    .bind(touch)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Keeps what a thread, a reply or an event said before an edit.
pub(crate) async fn record_revision(
    tx: &mut Transaction<'_, Postgres>,
    kind: i32,
    content_id: Uuid,
    title: &str,
    body: &str,
) -> AppResult<()> {
    sqlx::query("INSERT INTO community_revisions (kind, content_id, title, body) VALUES ($1, $2, $3, $4)")
        .bind(kind)
        .bind(content_id)
        .bind(title)
        .bind(body)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

impl Community {
    /// The thread named by a UUID or a short ID; deleted ones do not exist.
    pub(crate) async fn thread_by_ref(&self, reference: &str) -> AppResult<Option<ThreadRow>> {
        match Ref::parse(reference.trim()) {
            Ok(Ref::Id(id)) => self.thread_by_id(id).await,
            Ok(Ref::Short(short_id)) => Ok(sqlx::query_as(concat!(
                "SELECT ",
                thread_columns!(),
                " FROM community_threads t WHERE t.short_id = $1 AND t.deleted_at IS NULL"
            ))
            .bind(short_id)
            .fetch_optional(&self.core.db)
            .await?),
            Err(_) => Ok(None),
        }
    }

    pub(crate) async fn thread_by_id(&self, thread_id: Uuid) -> AppResult<Option<ThreadRow>> {
        Ok(sqlx::query_as(concat!(
            "SELECT ",
            thread_columns!(),
            " FROM community_threads t WHERE t.id = $1 AND t.deleted_at IS NULL"
        ))
        .bind(thread_id)
        .fetch_optional(&self.core.db)
        .await?)
    }

    /// A thread by UUID or short ID, a removed one included for as long as
    /// it is retained. For moderation and for reports only: everything
    /// members read goes through [`Self::thread_by_ref`].
    pub(crate) async fn thread_retained(&self, reference: &str) -> AppResult<Option<ThreadRow>> {
        let (id, short_id) = match Ref::parse(reference.trim()) {
            Ok(Ref::Id(id)) => (Some(id), None),
            Ok(Ref::Short(short_id)) => (None, Some(short_id)),
            Err(_) => return Ok(None),
        };
        Ok(sqlx::query_as(concat!(
            "SELECT ",
            thread_columns!(),
            " FROM community_threads t WHERE (t.id = $1 OR t.short_id = $2) AND t.purged_at IS NULL"
        ))
        .bind(id)
        .bind(short_id)
        .fetch_optional(&self.core.db)
        .await?)
    }

    /// The thread named by `reference` as `caller` may see it, with the
    /// caller's standing in its community and their blocks. A thread the
    /// caller may not know about does not exist.
    pub(crate) async fn open_thread(
        &self,
        caller: &Caller,
        reference: &str,
    ) -> AppResult<(ThreadRow, Access, Vec<Uuid>)> {
        let thread = self.thread_by_ref(reference).await?.ok_or_else(thread_not_found)?;
        let access = match self.access_by_id(caller, thread.community_id).await {
            Ok(access) => access,
            Err(error) if error.is_not_found() => return Err(thread_not_found()),
            Err(error) => return Err(error),
        };
        access.require_read()?;
        let blocked = self.blocked(caller.user_id).await?;
        if !(View { access: &access, blocked: &blocked }).sees_thread(&thread) {
            return Err(thread_not_found());
        }
        Ok((thread, access, blocked))
    }

    /// The hourly allowance of staff actions that write to the log and
    /// push to everyone (pins, locks, announcements, channel and safety
    /// settings): generous for moderating, too small for flooding.
    pub(crate) async fn staff_limit(&self, actor: Uuid) -> AppResult<()> {
        self.limit("staff", actor, 240, Duration::from_secs(3600)).await
    }

    /// Who wrote a thread, as notifications may name them.
    pub(crate) async fn speaker(&self, thread_id: Uuid, author_id: Uuid, anonymous: bool) -> AppResult<Speaker> {
        if anonymous {
            return Ok(Speaker::anonymous(&self.core.keys, thread_id, author_id));
        }
        let user = tokuly_identity::summary(&self.core, author_id).await?;
        Ok(Speaker::user(&user, author_id))
    }

    /// Starts a thread in `channel`.
    pub(crate) async fn create_thread(
        &self,
        access: &Access,
        channel: &ChannelRow,
        req: &pb::CreateThreadRequest,
    ) -> AppResult<ThreadRow> {
        let core = &self.core;
        let community = &access.community;
        let member = access.require_active()?;
        let author = member.user_id;
        ensure!(channel.open_to(member.role()), denied, "POSTING_RESTRICTED", "only moderators post in this channel");
        self.ensure_anonymity(community, channel).await?;
        let title = validate::title(&req.title)?;
        let body = validate::body(&req.body, validate::MAX_THREAD_BODY_CHARS)?;
        let tags = validate::tags(&req.tags, validate::MAX_THREAD_TAGS)?;
        let poll = req.poll.as_option().map(|poll| validate::poll(poll, now())).transpose()?;
        let images = self.resolve_images(author, &req.asset_ids, channel.anonymous).await?;
        let stamp_id = self.resolve_stamp(author, &req.stamp_id, channel.anonymous).await?;
        ensure!(
            !body.is_empty() || !images.asset_ids.is_empty() || stamp_id.is_some() || poll.is_some(),
            invalid,
            "THREAD_EMPTY",
            "a thread needs text, images, a stamp or a poll"
        );
        let mentions = self.resolve_mentions(community.id, author, &format!("{title}\n{body}")).await?;
        let mut screened: Vec<&str> = vec![&title, &body];
        if let Some(poll) = &poll {
            screened.extend(poll.options.iter().map(String::as_str));
        }
        let (post_state, hidden_reason) = posting::screening(community, access, channel.anonymous, &screened);
        let visible = post_state == state::VISIBLE;

        let slot = self.admit_post(access, channel).await?;
        let stored: AppResult<ThreadRow> = async {
            // Everything read from elsewhere is read before the transaction
            // opens: while it holds its connection, nothing here waits for
            // a second one.
            let thread_id = new_id();
            let recipients = if visible {
                self.mention_recipients(community.id, &mentions, author, channel.anonymous, &[]).await?
            } else {
                Vec::new()
            };
            let staff = if visible { Vec::new() } else { self.staff_ids(community.id).await? };
            let speaker = self.speaker(thread_id, author, channel.anonymous).await?;
            let mut tx = core.db.begin().await?;
            let thread: ThreadRow = sqlx::query_as(concat!(
                "INSERT INTO community_threads AS t
                     (short_id, community_id, channel_id, author_id, anonymous, title, body, mentions, images,
                      asset_ids, stamp_id, tags, state, hidden_reason, hidden_at, hot_score, id)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                         CASE WHEN $13 = 2 THEN now() END, community_hot_score(0, 0, 0, now(), now(), now()), $15)
                 RETURNING ",
                thread_columns!()
            ))
            .bind(new_short_id())
            .bind(community.id)
            .bind(channel.id)
            .bind(author)
            .bind(channel.anonymous)
            .bind(&title)
            .bind(&body)
            .bind(to_json(&mentions)?)
            .bind(to_json(&images.images)?)
            .bind(&images.asset_ids)
            .bind(stamp_id)
            .bind(&tags)
            .bind(post_state)
            .bind(hidden_reason)
            .bind(thread_id)
            .fetch_one(&mut *tx)
            .await?;
            for asset_id in &images.asset_ids {
                assets::attach(&mut *tx, *asset_id, &thread_holder(thread.id)).await?;
            }
            if let Some(poll) = &poll {
                self.insert_poll(&mut tx, thread.id, poll).await?;
            }
            // The author follows their thread and has, of course, read it.
            sqlx::query("INSERT INTO community_thread_follows (thread_id, user_id) VALUES ($1, $2)")
                .bind(thread.id)
                .bind(author)
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO community_thread_reads (thread_id, user_id) VALUES ($1, $2)")
                .bind(thread.id)
                .bind(author)
                .execute(&mut *tx)
                .await?;
            if visible {
                count_thread(&mut tx, &thread, 1, true).await?;
                let (actor, event) = created_event(community, &thread);
                core_events::emit(&mut *tx, events::THREAD_CREATED, actor, &event).await?;
                outbound::index(&mut *tx, search_doc(community, &thread)).await?;
                for recipient in recipients {
                    outbound::notify(&mut *tx, &notices::mention(recipient, &speaker, community, &thread, None))
                        .await?;
                }
            } else {
                self.record_held(&mut tx, community, &thread, None, &staff).await?;
            }
            tx.commit().await?;
            Ok(thread)
        }
        .await;
        let thread = match stored {
            Ok(thread) => thread,
            Err(error) => {
                self.release_slot(slot).await;
                return Err(error);
            }
        };

        // From here on nothing can fail the post.
        self.record_stamp(author, thread.stamp_id).await;
        if visible {
            self.push_thread_created(community, &thread).await;
        } else {
            self.push_staff_queues(community.id).await;
        }
        core.analytics
            .track(
                AnalyticsEvent::new("community.thread")
                    .user((!thread.anonymous).then_some(author))
                    .subject("community_thread", thread.id)
                    .props(serde_json::json!({
                        "community_id": community.id,
                        "anonymous": thread.anonymous,
                        "poll": poll.is_some(),
                        "images": thread.asset_ids.len(),
                        "held": !visible,
                    })),
            )
            .await;
        Ok(thread)
    }

    /// What follows a post being held by the word filter, inside its
    /// transaction: the log entry, a word to the author and to the staff.
    pub(crate) async fn record_held(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        community: &CommunityRow,
        thread: &ThreadRow,
        reply: Option<(Uuid, Uuid, bool)>,
        staff: &[Uuid],
    ) -> AppResult<()> {
        let (entry, author, path) = match reply {
            Some((reply_id, author_id, anonymous)) => (
                LogEntry::new(community.id, ModerationAction::MODERATION_ACTION_POST_HIDDEN)
                    .reply(thread.id, reply_id)
                    .author(author_id, anonymous),
                author_id,
                format!("{}?reply={reply_id}", thread.path(community)),
            ),
            None => (
                LogEntry::new(community.id, ModerationAction::MODERATION_ACTION_POST_HIDDEN)
                    .thread(thread.id)
                    .author(thread.author_id, thread.anonymous),
                thread.author_id,
                thread.path(community),
            ),
        };
        modlog::record(&mut **tx, &entry.channel(thread.channel_id).detail("cause", "muted_word")).await?;
        outbound::notify(&mut **tx, &notices::moderation(author, community, &thread.title, path, &PostNotice::Held))
            .await?;
        for staff_id in staff.iter().filter(|id| **id != author) {
            outbound::notify(&mut **tx, &notices::review(*staff_id, community)).await?;
        }
        Ok(())
    }

    /// Announces a thread on the community's topic. The thread itself is
    /// left out when its author has a block with anyone: followers of the
    /// topic then fetch it, and the fetch applies their blocks.
    pub(crate) async fn push_thread_created(&self, community: &CommunityRow, thread: &ThreadRow) {
        let payload = match self.pushable(thread.author_id, thread.anonymous).await {
            true => self.render_thread(community, None, thread).await.ok(),
            false => None,
        };
        let event = pb::ThreadCreated {
            community_id: community.id.to_string(),
            channel_id: thread.channel_id.to_string(),
            thread_id: thread.id.to_string(),
            thread: payload.into(),
            ..Default::default()
        };
        self.push_topic(community.id, event).await;
    }

    /// Whether a post may travel on the topic as it is: anonymous posts
    /// always (nobody's blocks apply to them), others unless their author
    /// has a block with anyone.
    pub(crate) async fn pushable(&self, author_id: Uuid, anonymous: bool) -> bool {
        anonymous || self.blocked(Some(author_id)).await.is_ok_and(|blocked| blocked.is_empty())
    }

    pub(crate) async fn push_thread_updated(&self, community: &CommunityRow, thread: &ThreadRow) {
        if !thread.is_visible() {
            return;
        }
        let payload = match self.pushable(thread.author_id, thread.anonymous).await {
            true => self.render_thread(community, None, thread).await.ok(),
            false => None,
        };
        let event = pb::ThreadUpdated {
            community_id: community.id.to_string(),
            thread_id: thread.id.to_string(),
            thread: payload.into(),
            ..Default::default()
        };
        self.push_topic(community.id, event).await;
    }

    pub(crate) async fn push_thread_removed(&self, thread: &ThreadRow) {
        let event = pb::ThreadRemoved {
            community_id: thread.community_id.to_string(),
            channel_id: thread.channel_id.to_string(),
            thread_id: thread.id.to_string(),
            ..Default::default()
        };
        self.push_topic(thread.community_id, event).await;
    }

    /// Edits title, body and tags of the caller's own thread.
    pub(crate) async fn update_thread(
        &self,
        access: &Access,
        thread: &ThreadRow,
        title: Option<String>,
        body: Option<String>,
        tags: Option<Vec<String>>,
    ) -> AppResult<ThreadRow> {
        let core = &self.core;
        let community = &access.community;
        let member = access.require_active()?;
        if thread.author_id != member.user_id {
            return Err(permission_denied());
        }
        ensure!(!thread.is_locked(), precondition, "THREAD_LOCKED", "this thread is locked");
        let title = title.unwrap_or_else(|| thread.title.clone());
        let body = body.unwrap_or_else(|| thread.body.clone());
        let tags = tags.unwrap_or_else(|| thread.tags.clone());
        // What the thread said before stays on file for the staff; the cap
        // keeps that history bounded without ever dropping the original.
        let reworded = title != thread.title || body != thread.body;
        ensure!(
            !reworded || thread.edit_count < MAX_EDITS,
            precondition,
            "TOO_MANY_EDITS",
            "a thread can be edited {MAX_EDITS} times"
        );
        self.limit("edit", member.user_id, 30, Duration::from_secs(60)).await?;
        let has_poll: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM community_polls WHERE thread_id = $1)")
            .bind(thread.id)
            .fetch_one(&core.db)
            .await?;
        ensure!(
            !body.is_empty() || !thread.asset_ids.is_empty() || thread.stamp_id.is_some() || has_poll,
            invalid,
            "THREAD_EMPTY",
            "a thread needs text, images, a stamp or a poll"
        );
        let mentions = self.resolve_mentions(community.id, member.user_id, &format!("{title}\n{body}")).await?;
        // An edit passes the word filter again. A thread the staff already
        // restored stays visible: they decide once, by hand, after that.
        let (screened, _) = posting::screening(community, access, thread.anonymous, &[&title, &body]);
        let hold = thread.is_visible() && screened == state::HIDDEN && thread.reviewed_at.is_none();
        let already_told: Vec<Uuid> =
            thread.mentions().iter().filter_map(|mention| Uuid::parse_str(&mention.user_id).ok()).collect();
        let recipients = if thread.is_visible() && !hold {
            self.mention_recipients(community.id, &mentions, member.user_id, thread.anonymous, &already_told).await?
        } else {
            Vec::new()
        };
        let staff = if hold { self.staff_ids(community.id).await? } else { Vec::new() };
        let speaker = self.speaker(thread.id, thread.author_id, thread.anonymous).await?;

        let mut tx = core.db.begin().await?;
        let updated: Option<ThreadRow> = sqlx::query_as(concat!(
            "UPDATE community_threads AS t SET
                 title = $2, body = $3, tags = $4, mentions = $5, edited_at = now(),
                 edit_count = edit_count + $8::int,
                 state = CASE WHEN $6 THEN 2 ELSE state END,
                 hidden_reason = CASE WHEN $6 THEN $7 ELSE hidden_reason END,
                 hidden_at = CASE WHEN $6 THEN now() ELSE hidden_at END
             WHERE t.id = $1 AND t.deleted_at IS NULL AND (NOT $8 OR t.edit_count < $9) RETURNING ",
            thread_columns!()
        ))
        .bind(thread.id)
        .bind(&title)
        .bind(&body)
        .bind(&tags)
        .bind(to_json(&mentions)?)
        .bind(hold)
        .bind(hidden::MUTED_WORD)
        .bind(reworded)
        .bind(MAX_EDITS)
        .fetch_optional(&mut *tx)
        .await?;
        // Two edits at once may both have passed the check above; the
        // condition in the statement is what holds the cap.
        let updated = updated.ok_or_else(|| {
            if reworded {
                AppError::precondition("TOO_MANY_EDITS", "this thread cannot be edited any more")
            } else {
                thread_not_found()
            }
        })?;
        if reworded {
            record_revision(&mut tx, revised::THREAD, thread.id, &thread.title, &thread.body).await?;
        }
        if hold {
            count_thread(&mut tx, thread, -1, false).await?;
            self.record_held(&mut tx, community, &updated, None, &staff).await?;
        }
        outbound::index(&mut *tx, search_doc(community, &updated)).await?;
        for recipient in recipients {
            outbound::notify(&mut *tx, &notices::mention(recipient, &speaker, community, &updated, None)).await?;
        }
        tx.commit().await?;

        if hold {
            self.push_thread_removed(&updated).await;
            self.push_staff_queues(community.id).await;
        } else {
            self.push_thread_updated(community, &updated).await;
        }
        Ok(updated)
    }

    /// Takes a thread out of view inside `tx`: its counters and its search
    /// document. False when it was gone already.
    ///
    /// The row keeps its content and its uploads for the retention period
    /// (see `Community::purge_removed`): what was posted can still be
    /// reported, read by the staff and traced to its author after its
    /// author — or anyone — removed it.
    pub(crate) async fn remove_thread(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        thread: &ThreadRow,
        actor: Option<Uuid>,
    ) -> AppResult<bool> {
        // The state on file decides the counters, not the row read earlier.
        let removed: Option<i32> = sqlx::query_scalar(
            "UPDATE community_threads SET deleted_at = now() WHERE id = $1 AND deleted_at IS NULL RETURNING state",
        )
        .bind(thread.id)
        .fetch_optional(&mut **tx)
        .await?;
        let Some(was) = removed else { return Ok(false) };
        if was == state::VISIBLE {
            count_thread(tx, thread, -1, false).await?;
        }
        outbound::unindex(&mut **tx, "thread", thread.id).await?;
        // The author deleting their own anonymous thread is not named on the bus.
        let actor = actor.filter(|actor| !(thread.anonymous && *actor == thread.author_id));
        core_events::emit(
            &mut **tx,
            events::THREAD_DELETED,
            actor,
            &PostDeleted { community_id: thread.community_id, thread_id: thread.id },
        )
        .await?;
        Ok(true)
    }

    /// Deletes a thread: its author, or a moderator with a reason.
    pub(crate) async fn delete_thread(&self, access: &Access, thread: &ThreadRow, reason: &str) -> AppResult<()> {
        let community = &access.community;
        let actor = access.require_member().map_err(|_| permission_denied())?.user_id;
        let own = thread.author_id == actor;
        if !own {
            access.require(Permission::PERMISSION_MODERATE_CONTENT)?;
        }
        let reason = validate::reason(reason, !own)?;
        if !own {
            self.staff_limit(actor).await?;
        }
        let mut tx = self.core.db.begin().await?;
        let removed = self.remove_thread(&mut tx, thread, own.then_some(actor)).await?;
        if removed && !own {
            let entry = LogEntry::new(community.id, ModerationAction::MODERATION_ACTION_THREAD_DELETED)
                .by(actor)
                .thread(thread.id)
                .channel(thread.channel_id)
                .author(thread.author_id, thread.anonymous)
                .reason(&reason)
                .detail("title", &thread.title);
            modlog::record(&mut *tx, &entry).await?;
            let notice = PostNotice::Removed { reason: reason.clone() };
            outbound::notify(
                &mut *tx,
                &notices::moderation(thread.author_id, community, &thread.title, community.path(), &notice),
            )
            .await?;
        }
        tx.commit().await?;
        if removed {
            self.push_thread_removed(thread).await;
            if thread.state == state::HIDDEN {
                self.push_staff_queues(community.id).await;
            }
        }
        Ok(())
    }

    /// Threads of a community (or one channel of it) that are not pinned,
    /// in the chosen order.
    pub(crate) async fn list_threads(
        &self,
        view: &View<'_>,
        filter: &ThreadFilter,
        page: &Page,
    ) -> AppResult<(Vec<ThreadRow>, PageInfo)> {
        let cursor: Option<ThreadCursor> = page.decode()?;
        let mut query = self.thread_query(view, filter);
        query.push(" AND t.pinned_at IS NULL");
        match filter.sort {
            ThreadSort::THREAD_SORT_NEW => {
                if let Some(ThreadCursor::New(id)) = cursor {
                    query.push(" AND t.id < ").push_bind(id);
                }
                query.push(" ORDER BY t.id DESC");
            }
            ThreadSort::THREAD_SORT_TOP => {
                let since = match filter.window {
                    TopWindow::TOP_WINDOW_DAY => Some(chrono::Duration::days(1)),
                    TopWindow::TOP_WINDOW_WEEK => Some(chrono::Duration::days(7)),
                    TopWindow::TOP_WINDOW_MONTH => Some(chrono::Duration::days(30)),
                    TopWindow::TOP_WINDOW_YEAR => Some(chrono::Duration::days(365)),
                    _ => None,
                };
                if let Some(since) = since {
                    query.push(" AND t.created_at > ").push_bind(now() - since);
                }
                if let Some(ThreadCursor::Top(likes, replies, id)) = cursor {
                    query
                        .push(" AND (t.like_count, t.reply_count, t.id) < (")
                        .push_bind(likes)
                        .push(", ")
                        .push_bind(replies)
                        .push(", ")
                        .push_bind(id)
                        .push(")");
                }
                query.push(" ORDER BY t.like_count DESC, t.reply_count DESC, t.id DESC");
            }
            _ => {
                if let Some(ThreadCursor::Hot(score, id)) = cursor {
                    query.push(" AND (t.hot_score, t.id) < (").push_bind(score).push(", ").push_bind(id).push(")");
                }
                query.push(" ORDER BY t.hot_score DESC, t.id DESC");
            }
        }
        query.push(" LIMIT ").push_bind(page.fetch());
        let mut rows: Vec<ThreadRow> = query.build_query_as().fetch_all(&self.core.db).await?;
        let info = page.finish(&mut rows, |row| match filter.sort {
            ThreadSort::THREAD_SORT_NEW => ThreadCursor::New(row.id),
            ThreadSort::THREAD_SORT_TOP => ThreadCursor::Top(row.like_count, row.reply_count, row.id),
            _ => ThreadCursor::Hot(row.hot_score, row.id),
        });
        Ok((rows, info))
    }

    /// The pinned threads matching `filter`, newest pin first.
    pub(crate) async fn pinned_threads(&self, view: &View<'_>, filter: &ThreadFilter) -> AppResult<Vec<ThreadRow>> {
        let mut query = self.thread_query(view, filter);
        query.push(" AND t.pinned_at IS NOT NULL ORDER BY t.pinned_at DESC, t.id DESC LIMIT 50");
        Ok(query.build_query_as().fetch_all(&self.core.db).await?)
    }

    /// The part every thread list shares: the community, what the viewer
    /// may see of it, and the filter.
    fn thread_query(&self, view: &View<'_>, filter: &ThreadFilter) -> QueryBuilder<Postgres> {
        let mut query: QueryBuilder<Postgres> = QueryBuilder::new(concat!(
            "SELECT ",
            thread_columns!(),
            " FROM community_threads t WHERE t.community_id = "
        ));
        query.push_bind(view.access.community.id);
        query.push(" AND t.deleted_at IS NULL");
        // Hidden threads are listed for their author only (the staff has the review queue).
        query.push(" AND (t.state = 1 OR t.author_id = ").push_bind(view.user_id().unwrap_or_default()).push(")");
        // Blocks hide named posts; anonymous ones are exempt (see `View::hides`).
        query.push(" AND (t.anonymous OR NOT (t.author_id = ANY(").push_bind(view.blocked.to_vec()).push(")))");
        if let Some(channel_id) = filter.channel_id {
            query.push(" AND t.channel_id = ").push_bind(channel_id);
        }
        if let Some(tag) = &filter.tag {
            query.push(" AND t.tags @> ARRAY[").push_bind(tag.clone()).push("]::text[]");
        }
        query
    }

    /// The community's announcements, newest first.
    pub(crate) async fn list_announcements(
        &self,
        view: &View<'_>,
        page: &Page,
    ) -> AppResult<(Vec<ThreadRow>, PageInfo)> {
        let after: Option<(Time, Uuid)> = page.decode()?;
        let mut rows: Vec<ThreadRow> = sqlx::query_as(concat!(
            "SELECT ",
            thread_columns!(),
            " FROM community_threads t
              WHERE t.community_id = $1 AND t.announcement AND t.deleted_at IS NULL AND t.state = 1
                AND (t.anonymous OR NOT (t.author_id = ANY($2)))
                AND ($3::timestamptz IS NULL OR (t.announced_at, t.id) < ($3, $4))
              ORDER BY t.announced_at DESC, t.id DESC LIMIT $5"
        ))
        .bind(view.access.community.id)
        .bind(view.blocked)
        .bind(after.map(|(at, _)| at))
        .bind(after.map(|(_, id)| id))
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.announced_at.unwrap_or(row.created_at), row.id));
        Ok((rows, info))
    }

    /// Threads `user_id` follows in communities they are a member of, most
    /// recently active first.
    pub(crate) async fn followed_threads(
        &self,
        user_id: Uuid,
        community_id: Option<Uuid>,
        unread_only: bool,
        page: &Page,
    ) -> AppResult<(Vec<ThreadRow>, PageInfo)> {
        let blocked = self.blocked(Some(user_id)).await?;
        let after: Option<(Time, Uuid)> = page.decode()?;
        let mut rows: Vec<ThreadRow> = sqlx::query_as(concat!(
            "SELECT ",
            thread_columns!(),
            " FROM community_thread_follows f
              JOIN community_threads t ON t.id = f.thread_id
              JOIN communities c ON c.id = t.community_id AND c.deleted_at IS NULL
              JOIN community_members m ON m.community_id = t.community_id AND m.user_id = f.user_id
              LEFT JOIN community_thread_reads tr ON tr.thread_id = t.id AND tr.user_id = f.user_id
              LEFT JOIN community_channel_reads cr ON cr.channel_id = t.channel_id AND cr.user_id = f.user_id
              WHERE f.user_id = $1 AND f.following AND t.deleted_at IS NULL AND t.state = 1
                AND ($2::uuid IS NULL OR t.community_id = $2)
                AND (t.anonymous OR NOT (t.author_id = ANY($3)))
                AND (NOT $4 OR (t.last_activity_at > greatest(m.joined_at, coalesce(m.read_all_at, m.joined_at))
                                AND (tr.read_at IS NULL OR t.last_activity_at > tr.read_at)
                                AND (cr.read_at IS NULL OR t.last_activity_at > cr.read_at)))
                AND ($5::timestamptz IS NULL OR (t.last_activity_at, t.id) < ($5, $6))
              ORDER BY t.last_activity_at DESC, t.id DESC LIMIT $7"
        ))
        .bind(user_id)
        .bind(community_id)
        .bind(&blocked)
        .bind(unread_only)
        .bind(after.map(|(at, _)| at))
        .bind(after.map(|(_, id)| id))
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.last_activity_at, row.id));
        Ok((rows, info))
    }

    /// Pins or unpins a thread. Needs MODERATE_CONTENT.
    pub(crate) async fn pin_thread(&self, access: &Access, thread: &ThreadRow, pinned: bool) -> AppResult<ThreadRow> {
        access.require(Permission::PERMISSION_MODERATE_CONTENT)?;
        let actor = access.user_id.ok_or_else(permission_denied)?;
        self.staff_limit(actor).await?;
        let mut tx = self.core.db.begin().await?;
        if pinned && thread.pinned_at.is_none() {
            // Locking the channel row serializes pins: the limit holds.
            sqlx::query("SELECT 1 FROM community_channels WHERE id = $1 FOR UPDATE")
                .bind(thread.channel_id)
                .execute(&mut *tx)
                .await?;
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM community_threads
                 WHERE channel_id = $1 AND pinned_at IS NOT NULL AND deleted_at IS NULL",
            )
            .bind(thread.channel_id)
            .fetch_one(&mut *tx)
            .await?;
            ensure!(
                count < MAX_PINNED,
                precondition,
                "TOO_MANY_PINNED",
                "a channel has at most {MAX_PINNED} pinned threads"
            );
        }
        let updated: ThreadRow = sqlx::query_as(concat!(
            "UPDATE community_threads AS t
             SET pinned_at = CASE WHEN $2 THEN coalesce(pinned_at, now()) END
             WHERE t.id = $1 AND t.deleted_at IS NULL RETURNING ",
            thread_columns!()
        ))
        .bind(thread.id)
        .bind(pinned)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(thread_not_found)?;
        if pinned != thread.pinned_at.is_some() {
            let action = if pinned {
                ModerationAction::MODERATION_ACTION_THREAD_PINNED
            } else {
                ModerationAction::MODERATION_ACTION_THREAD_UNPINNED
            };
            self.log_thread_action(&mut tx, action, actor, thread).await?;
        }
        tx.commit().await?;
        self.push_thread_updated(&access.community, &updated).await;
        Ok(updated)
    }

    /// Locks or unlocks a thread. Needs MODERATE_CONTENT.
    pub(crate) async fn lock_thread(&self, access: &Access, thread: &ThreadRow, locked: bool) -> AppResult<ThreadRow> {
        access.require(Permission::PERMISSION_MODERATE_CONTENT)?;
        let actor = access.user_id.ok_or_else(permission_denied)?;
        self.staff_limit(actor).await?;
        let mut tx = self.core.db.begin().await?;
        let updated: ThreadRow = sqlx::query_as(concat!(
            "UPDATE community_threads AS t
             SET locked_at = CASE WHEN $2 THEN coalesce(locked_at, now()) END
             WHERE t.id = $1 AND t.deleted_at IS NULL RETURNING ",
            thread_columns!()
        ))
        .bind(thread.id)
        .bind(locked)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(thread_not_found)?;
        if locked != thread.is_locked() {
            let action = if locked {
                ModerationAction::MODERATION_ACTION_THREAD_LOCKED
            } else {
                ModerationAction::MODERATION_ACTION_THREAD_UNLOCKED
            };
            self.log_thread_action(&mut tx, action, actor, thread).await?;
        }
        tx.commit().await?;
        self.push_thread_updated(&access.community, &updated).await;
        Ok(updated)
    }

    async fn log_thread_action(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        action: ModerationAction,
        actor: Uuid,
        thread: &ThreadRow,
    ) -> AppResult<()> {
        let entry = LogEntry::new(thread.community_id, action)
            .by(actor)
            .thread(thread.id)
            .channel(thread.channel_id)
            .author(thread.author_id, thread.anonymous)
            .detail("title", &thread.title);
        modlog::record(&mut **tx, &entry).await
    }

    /// Marks a thread as an announcement or takes the mark back. The first
    /// time, the members are notified (in the background). Needs ANNOUNCE.
    pub(crate) async fn set_announcement(
        &self,
        access: &Access,
        thread: &ThreadRow,
        announcement: bool,
    ) -> AppResult<ThreadRow> {
        access.require(Permission::PERMISSION_ANNOUNCE)?;
        let actor = access.user_id.ok_or_else(permission_denied)?;
        self.staff_limit(actor).await?;
        ensure!(thread.is_visible(), precondition, "THREAD_HIDDEN", "a hidden thread cannot be an announcement");
        let first_time = announcement && thread.announced_at.is_none();
        if first_time {
            self.core
                .cache
                .rate_limit(
                    &format!("community:announce:{}", thread.community_id),
                    ANNOUNCEMENTS_PER_HOUR,
                    Duration::from_secs(3600),
                )
                .await?;
        }
        let mut tx = self.core.db.begin().await?;
        let updated: ThreadRow = sqlx::query_as(concat!(
            "UPDATE community_threads AS t
             SET announcement = $2, announced_at = CASE WHEN $2 THEN coalesce(announced_at, now()) ELSE announced_at END
             WHERE t.id = $1 AND t.deleted_at IS NULL RETURNING ",
            thread_columns!()
        ))
        .bind(thread.id)
        .bind(announcement)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(thread_not_found)?;
        if announcement != thread.announcement {
            let action = if announcement {
                ModerationAction::MODERATION_ACTION_ANNOUNCEMENT_SET
            } else {
                ModerationAction::MODERATION_ACTION_ANNOUNCEMENT_CLEARED
            };
            self.log_thread_action(&mut tx, action, actor, thread).await?;
        }
        tx.commit().await?;
        if first_time {
            self.queue_announcement(AnnounceJob { thread_id: thread.id }).await;
        }
        self.push_thread_updated(&access.community, &updated).await;
        Ok(updated)
    }

    /// Follows or unfollows a thread. Members only.
    pub(crate) async fn set_thread_follow(
        &self,
        access: &Access,
        thread: &ThreadRow,
        following: bool,
    ) -> AppResult<()> {
        let member = access.require_member()?;
        sqlx::query(
            "INSERT INTO community_thread_follows (thread_id, user_id, following) VALUES ($1, $2, $3)
             ON CONFLICT (thread_id, user_id) DO UPDATE SET following = EXCLUDED.following, notified_at = NULL",
        )
        .bind(thread.id)
        .bind(member.user_id)
        .bind(following)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    /// Marks one thread as read by `user_id` (a member).
    pub(crate) async fn mark_thread_read(&self, thread_id: Uuid, user_id: Uuid) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO community_thread_reads (thread_id, user_id) VALUES ($1, $2)
             ON CONFLICT (thread_id, user_id) DO UPDATE SET read_at = now()",
        )
        .bind(thread_id)
        .bind(user_id)
        .execute(&self.core.db)
        .await?;
        // Having read it, the member is told about the next reply again.
        sqlx::query(
            "UPDATE community_thread_follows SET notified_at = NULL
             WHERE thread_id = $1 AND user_id = $2 AND notified_at IS NOT NULL",
        )
        .bind(thread_id)
        .bind(user_id)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    /// Marks every thread of a channel as read by `user_id` (a member).
    pub(crate) async fn mark_channel_read(&self, channel_id: Uuid, user_id: Uuid) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO community_channel_reads (channel_id, user_id) VALUES ($1, $2)
             ON CONFLICT (channel_id, user_id) DO UPDATE SET read_at = now()",
        )
        .bind(channel_id)
        .bind(user_id)
        .execute(&self.core.db)
        .await?;
        sqlx::query(
            "UPDATE community_thread_follows f SET notified_at = NULL FROM community_threads t
             WHERE t.id = f.thread_id AND t.channel_id = $1 AND f.user_id = $2 AND f.notified_at IS NOT NULL",
        )
        .bind(channel_id)
        .bind(user_id)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    /// Marks everything in a community as read by `user_id` (a member).
    pub(crate) async fn mark_community_read(&self, community_id: Uuid, user_id: Uuid) -> AppResult<()> {
        sqlx::query("UPDATE community_members SET read_all_at = now() WHERE community_id = $1 AND user_id = $2")
            .bind(community_id)
            .bind(user_id)
            .execute(&self.core.db)
            .await?;
        sqlx::query(
            "UPDATE community_thread_follows f SET notified_at = NULL FROM community_threads t
             WHERE t.id = f.thread_id AND t.community_id = $1 AND f.user_id = $2 AND f.notified_at IS NOT NULL",
        )
        .bind(community_id)
        .bind(user_id)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_round_trip_per_order() {
        let id = Uuid::now_v7();
        for cursor in [ThreadCursor::Hot(0.35, id), ThreadCursor::New(id), ThreadCursor::Top(12, 3, id)] {
            let encoded = tokuly_core::page::encode_cursor(&cursor);
            let decoded: ThreadCursor = tokuly_core::page::decode_cursor(&encoded).unwrap();
            assert_eq!(format!("{cursor:?}"), format!("{decoded:?}"));
        }
    }
}
