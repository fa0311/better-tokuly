//! Turning thread and reply rows into what one viewer sees.
//!
//! This is the one place that decides whether a post names its author. A
//! post from an anonymous channel never does: `author` stays unset and the
//! pseudonym stands in, whoever is looking — a member, the staff, site
//! moderation. The author alone learns it is theirs (`viewer.mine`).
use std::collections::HashMap;

use tokuly_core::prelude::*;
use tokuly_proto::{
    common::{StampRef, UserSummary},
    community as pb,
};

use crate::{
    Community,
    access::Access,
    model::{CommunityRow, ReplyRow, ThreadRow, posting, state},
    pseudonym::pseudonym,
    text,
};

/// Longest excerpt of a quoted reply.
const QUOTE_CHARS: usize = 120;

/// A caller looking at posts of one community.
#[derive(Clone, Copy)]
pub(crate) struct View<'a> {
    pub access: &'a Access,
    /// Everyone in a block with the caller, either direction.
    pub blocked: &'a [Uuid],
}

impl View<'_> {
    pub fn user_id(&self) -> Option<Uuid> {
        self.access.user_id
    }

    /// Whether a block hides a post by `author_id` from the caller.
    /// Anonymous posts are exempt: hiding them would tell who wrote them.
    pub fn hides(&self, author_id: Uuid, anonymous: bool) -> bool {
        !anonymous && self.blocked.contains(&author_id)
    }

    /// Whether the caller may see the thread at all (reading rights on the
    /// community are checked by the caller of this).
    pub fn sees_thread(&self, thread: &ThreadRow) -> bool {
        let mine = self.user_id() == Some(thread.author_id);
        thread.deleted_at.is_none()
            && (thread.state == state::VISIBLE || mine || self.access.sees_hidden_posts())
            && !self.hides(thread.author_id, thread.anonymous)
    }

    pub fn sees_reply(&self, reply: &ReplyRow) -> bool {
        let mine = self.user_id() == Some(reply.author_id);
        let thread_mine = self.user_id() == Some(reply.thread_author_id);
        let sees_hidden = self.access.sees_hidden_posts();
        reply.deleted_at.is_none()
            && reply.thread_deleted_at.is_none()
            && (reply.state == state::VISIBLE || mine || sees_hidden)
            && (reply.thread_state == state::VISIBLE || thread_mine || sees_hidden)
            && !self.hides(reply.author_id, reply.anonymous)
    }
}

/// What the viewer did with a thread, and what its channel allows.
#[derive(sqlx::FromRow)]
struct ThreadState {
    id: Uuid,
    following: Option<bool>,
    thread_read_at: Option<Time>,
    channel_read_at: Option<Time>,
    posting: i32,
}

impl Community {
    /// One thread as `view` sees it; see [`Self::render_threads`].
    pub(crate) async fn render_thread(
        &self,
        community: &CommunityRow,
        view: Option<&View<'_>>,
        row: &ThreadRow,
    ) -> AppResult<pb::Thread> {
        let mut threads = self.render_threads(community, view, std::slice::from_ref(row)).await?;
        threads.pop().ok_or_else(|| AppError::not_found("THREAD_NOT_FOUND", "thread not found"))
    }

    /// Threads of `community` as `view` sees them, in the order given.
    /// `view` of `None` renders for a realtime push: no viewer state, and
    /// poll results as someone who has not voted sees them.
    pub(crate) async fn render_threads(
        &self,
        community: &CommunityRow,
        view: Option<&View<'_>>,
        rows: &[ThreadRow],
    ) -> AppResult<Vec<pb::Thread>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let core = &self.core;
        let viewer_id = view.and_then(|view| view.user_id());
        let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        let named: Vec<Uuid> = rows.iter().filter(|row| !row.anonymous).map(|row| row.author_id).collect();
        let authors = tokuly_identity::summaries(core, &named).await?;
        let stamp_ids: Vec<Uuid> = rows.iter().filter_map(|row| row.stamp_id).collect();
        let stamps = tokuly_stamp::stamp_refs(core, &stamp_ids).await?;
        let polls = self.load_polls(&ids, viewer_id).await?;
        let viewers = match view {
            Some(view) => self.thread_viewers(view, rows).await?,
            None => HashMap::new(),
        };

        Ok(rows
            .iter()
            .map(|row| {
                let author = (!row.anonymous).then(|| tokuly_identity::users::pick(&authors, row.author_id));
                let stamp = row.stamp_id.and_then(|id| stamps.get(&id).cloned());
                let poll = polls.get(&row.id).map(|poll| poll.to_proto(viewer_id == Some(row.author_id)));
                self.thread_proto(community, row, author, stamp, poll, viewers.get(&row.id).cloned())
            })
            .collect())
    }

    /// The `viewer` block of each thread, for a signed-in caller.
    async fn thread_viewers(&self, view: &View<'_>, rows: &[ThreadRow]) -> AppResult<HashMap<Uuid, pb::ThreadViewer>> {
        let Some(user_id) = view.user_id() else { return Ok(HashMap::new()) };
        let access = view.access;
        let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        let states: Vec<ThreadState> = sqlx::query_as(
            "SELECT t.id, f.following, tr.read_at AS thread_read_at, cr.read_at AS channel_read_at, ch.posting
             FROM community_threads t
             JOIN community_channels ch ON ch.id = t.channel_id
             LEFT JOIN community_thread_follows f ON f.thread_id = t.id AND f.user_id = $2
             LEFT JOIN community_thread_reads tr ON tr.thread_id = t.id AND tr.user_id = $2
             LEFT JOIN community_channel_reads cr ON cr.channel_id = t.channel_id AND cr.user_id = $2
             WHERE t.id = ANY($1)",
        )
        .bind(&ids)
        .bind(user_id)
        .fetch_all(&self.core.db)
        .await?;
        let states: HashMap<Uuid, ThreadState> = states.into_iter().map(|state| (state.id, state)).collect();

        let member = access.member.as_ref();
        let active = member.is_some() && access.timeout().is_none();
        let moderates = access.moderates();
        // Unread: something happened after everything the member marked as
        // read (the thread, its channel, the whole community) and after they joined.
        let mut unread_floors: Vec<(Uuid, Time)> = Vec::new();
        let mut out: HashMap<Uuid, pb::ThreadViewer> = HashMap::with_capacity(rows.len());
        for row in rows {
            let state = states.get(&row.id);
            let mine = row.author_id == user_id;
            let open = row.is_visible() && !row.is_locked();
            let channel_open = state.is_some_and(|state| {
                state.posting != posting::MODERATORS || access.role().is_some_and(|role| role.is_staff())
            });
            let mut viewer = pb::ThreadViewer {
                mine,
                following: state.and_then(|state| state.following).unwrap_or(false),
                can_edit: mine && active && !row.is_locked(),
                can_delete: mine || moderates,
                can_moderate: moderates,
                can_reply: active && open && channel_open,
                ..Default::default()
            };
            if let Some(member) = member {
                let floor = [state.and_then(|s| s.thread_read_at), state.and_then(|s| s.channel_read_at)]
                    .into_iter()
                    .flatten()
                    .fold(member.read_floor(), Time::max);
                if row.is_visible() && row.last_activity_at > floor {
                    viewer.unread = true;
                    unread_floors.push((row.id, floor));
                }
            }
            out.insert(row.id, viewer);
        }
        if !unread_floors.is_empty() {
            let (thread_ids, floors): (Vec<Uuid>, Vec<Time>) = unread_floors.into_iter().unzip();
            let counts: Vec<(Uuid, i64)> = sqlx::query_as(
                "SELECT r.thread_id, count(*) FROM community_replies r
                 JOIN unnest($1::uuid[], $2::timestamptz[]) AS f(thread_id, floor) ON f.thread_id = r.thread_id
                 WHERE r.created_at > f.floor AND r.deleted_at IS NULL AND r.state = 1 AND r.author_id <> $3
                 GROUP BY r.thread_id",
            )
            .bind(&thread_ids)
            .bind(&floors)
            .bind(user_id)
            .fetch_all(&self.core.db)
            .await?;
            for (thread_id, count) in counts {
                if let Some(viewer) = out.get_mut(&thread_id) {
                    viewer.unread_reply_count = count as i32;
                }
            }
        }
        Ok(out)
    }

    fn thread_proto(
        &self,
        community: &CommunityRow,
        row: &ThreadRow,
        author: Option<UserSummary>,
        stamp: Option<StampRef>,
        poll: Option<pb::Poll>,
        viewer: Option<pb::ThreadViewer>,
    ) -> pb::Thread {
        let pseudonym = row.anonymous.then(|| pseudonym(&self.core.keys, row.id, row.author_id));
        pb::Thread {
            id: row.id.to_string(),
            short_id: row.short_id.clone(),
            community_id: row.community_id.to_string(),
            channel_id: row.channel_id.to_string(),
            author: author.into(),
            pseudonym: pseudonym.into(),
            title: row.title.clone(),
            body: row.body.clone(),
            mentions: row.mentions(),
            images: row.images(),
            stamp: stamp.into(),
            poll: poll.into(),
            tags: row.tags.clone(),
            pinned: row.pinned_at.is_some(),
            locked: row.is_locked(),
            announcement: row.announcement,
            state: row.state.into(),
            hidden_reason: if row.state == state::HIDDEN { row.hidden_reason } else { 0 }.into(),
            reply_count: row.reply_count,
            like_count: row.like_count,
            comment_count: row.comment_count,
            created_at: ts(row.created_at),
            edited_at: opt_ts(row.edited_at),
            last_activity_at: ts(row.last_activity_at),
            path: row.path(community),
            viewer: viewer.into(),
            ..Default::default()
        }
    }

    /// One reply as `view` sees it; see [`Self::render_replies`].
    pub(crate) async fn render_reply(&self, view: Option<&View<'_>>, row: &ReplyRow) -> AppResult<pb::Reply> {
        let mut replies = self.render_replies(view, std::slice::from_ref(row)).await?;
        replies.pop().ok_or_else(|| AppError::not_found("REPLY_NOT_FOUND", "reply not found"))
    }

    /// Replies as `view` sees them, in the order given. `view` of `None`
    /// renders for a realtime push.
    pub(crate) async fn render_replies(&self, view: Option<&View<'_>>, rows: &[ReplyRow]) -> AppResult<Vec<pb::Reply>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let core = &self.core;
        // Accounts are only ever looked up for posts that are not anonymous.
        let named: Vec<Uuid> = rows
            .iter()
            .filter(|row| !row.anonymous)
            .flat_map(|row| [Some(row.author_id), row.quoted_author_id])
            .flatten()
            .collect();
        let authors = tokuly_identity::summaries(core, &named).await?;
        let stamp_ids: Vec<Uuid> = rows.iter().filter_map(|row| row.stamp_id).collect();
        let stamps = tokuly_stamp::stamp_refs(core, &stamp_ids).await?;

        let viewer_id = view.and_then(|view| view.user_id());
        let active = view.is_some_and(|view| view.access.is_member() && view.access.timeout().is_none());
        let moderates = view.is_some_and(|view| view.access.moderates());
        Ok(rows
            .iter()
            .map(|row| {
                let author = (!row.anonymous).then(|| tokuly_identity::users::pick(&authors, row.author_id));
                let pseudonym = row.anonymous.then(|| pseudonym(&core.keys, row.thread_id, row.author_id));
                let reply_to = row.reply_to_id.map(|quoted_id| {
                    let available = row.quoted_available.unwrap_or(false)
                        && !row
                            .quoted_author_id
                            .is_some_and(|author| view.is_some_and(|v| v.hides(author, row.anonymous)));
                    let quoted_author = row.quoted_author_id.filter(|_| available);
                    pb::QuotedReply {
                        reply_id: quoted_id.to_string(),
                        author: quoted_author
                            .filter(|_| !row.anonymous)
                            .map(|id| tokuly_identity::users::pick(&authors, id))
                            .into(),
                        pseudonym: quoted_author
                            .filter(|_| row.anonymous)
                            .map(|id| pseudonym_of(core, row.thread_id, id))
                            .into(),
                        excerpt: if available {
                            text::excerpt(row.quoted_body.as_deref().unwrap_or_default(), QUOTE_CHARS)
                        } else {
                            String::new()
                        },
                        unavailable: !available,
                        ..Default::default()
                    }
                });
                let viewer = viewer_id.map(|user_id| {
                    let mine = row.author_id == user_id;
                    pb::ReplyViewer {
                        mine,
                        can_edit: mine && active && row.thread_locked_at.is_none(),
                        can_delete: mine || moderates,
                        ..Default::default()
                    }
                });
                pb::Reply {
                    id: row.id.to_string(),
                    thread_id: row.thread_id.to_string(),
                    community_id: row.community_id.to_string(),
                    author: author.into(),
                    pseudonym: pseudonym.into(),
                    body: row.body.clone(),
                    mentions: row.mentions(),
                    images: row.images(),
                    stamp: row.stamp_id.and_then(|id| stamps.get(&id).cloned()).into(),
                    reply_to: reply_to.into(),
                    state: row.state.into(),
                    hidden_reason: if row.state == state::HIDDEN { row.hidden_reason } else { 0 }.into(),
                    like_count: row.like_count,
                    comment_count: row.comment_count,
                    number: row.seq,
                    by_thread_author: row.author_id == row.thread_author_id,
                    created_at: ts(row.created_at),
                    edited_at: opt_ts(row.edited_at),
                    viewer: viewer.into(),
                    ..Default::default()
                }
            })
            .collect())
    }
}

fn pseudonym_of(core: &Core, thread_id: Uuid, author_id: Uuid) -> pb::Pseudonym {
    pseudonym(&core.keys, thread_id, author_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MemberRow, level, visibility};

    fn community() -> CommunityRow {
        CommunityRow {
            id: Uuid::now_v7(),
            handle: "club".into(),
            name: "Club".into(),
            description: String::new(),
            icon: None,
            icon_asset_id: None,
            banner: None,
            banner_asset_id: None,
            rules: serde_json::json!([]),
            tags: Vec::new(),
            category: 2,
            language: "ja".into(),
            visibility: visibility::PUBLIC,
            school_name: String::new(),
            join_questions: Vec::new(),
            members_can_invite: false,
            members_can_create_events: false,
            muted_words: Vec::new(),
            auto_hide_reports: 0,
            owner_id: Uuid::now_v7(),
            member_count: 1,
            thread_count: 0,
            last_activity_at: now(),
            deleted_at: None,
            created_at: now(),
        }
    }

    fn access(role: Option<i32>) -> Access {
        let community = community();
        let user_id = Uuid::now_v7();
        Access {
            member: role.map(|role| MemberRow {
                community_id: community.id,
                user_id,
                role,
                notification_level: level::ALL,
                show_on_profile: false,
                timeout_until: None,
                timeout_reason: String::new(),
                read_all_at: None,
                joined_at: now(),
            }),
            community,
            user_id: Some(user_id),
            site_moderator: false,
            ban: None,
            requested: false,
        }
    }

    fn thread(author_id: Uuid, anonymous: bool, state: i32) -> ThreadRow {
        ThreadRow {
            id: Uuid::now_v7(),
            short_id: "abcdefghijk".into(),
            community_id: Uuid::now_v7(),
            channel_id: Uuid::now_v7(),
            author_id,
            anonymous,
            title: "t".into(),
            body: String::new(),
            mentions: serde_json::json!([]),
            images: serde_json::json!([]),
            asset_ids: Vec::new(),
            stamp_id: None,
            tags: Vec::new(),
            pinned_at: None,
            locked_at: None,
            announcement: false,
            announced_at: None,
            state,
            hidden_reason: 0,
            reviewed_at: None,
            reply_count: 0,
            like_count: 0,
            comment_count: 0,
            hot_score: 0.0,
            last_activity_at: now(),
            edited_at: None,
            edit_count: 0,
            deleted_at: None,
            created_at: now(),
        }
    }

    #[test]
    fn a_block_hides_named_posts_but_never_anonymous_ones() {
        let access = access(Some(1));
        let blocked_author = Uuid::now_v7();
        let blocked = [blocked_author];
        let view = View { access: &access, blocked: &blocked };
        assert!(!view.sees_thread(&thread(blocked_author, false, state::VISIBLE)));
        // Hiding it would tell the viewer who wrote the anonymous post.
        assert!(view.sees_thread(&thread(blocked_author, true, state::VISIBLE)));
        assert!(view.sees_thread(&thread(Uuid::now_v7(), false, state::VISIBLE)));
    }

    #[test]
    fn hidden_posts_are_for_their_author_and_the_staff() {
        let member = access(Some(1));
        let view = View { access: &member, blocked: &[] };
        assert!(!view.sees_thread(&thread(Uuid::now_v7(), false, state::HIDDEN)));
        assert!(view.sees_thread(&thread(member.user_id.unwrap(), false, state::HIDDEN)));

        let moderator = access(Some(2));
        let view = View { access: &moderator, blocked: &[] };
        assert!(view.sees_thread(&thread(Uuid::now_v7(), false, state::HIDDEN)));

        let mut deleted = thread(member.user_id.unwrap(), false, state::VISIBLE);
        deleted.deleted_at = Some(now());
        assert!(!View { access: &member, blocked: &[] }.sees_thread(&deleted));
    }
}
