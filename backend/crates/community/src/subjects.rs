//! How the generic domains see this one: subject resolvers for
//! communities, threads, replies and events (likes, comments, bookmarks and
//! reports attach to them through the engagement domain), and the guard of
//! the `community:` realtime topics.
//!
//! A subject is visible to whoever may see it through this domain's own
//! API: a community's front page unless it is hidden, its content for
//! members (and for everyone when it is public), hidden posts for their
//! author and the staff.
//!
//! **A post in an anonymous channel belongs to nobody** as far as these
//! resolvers say (the nil UUID, like official content). The generic domains
//! act on a subject's owner — engagement hides a subject from anyone in a
//! block with its owner and purges it when its owner's account is erased —
//! and any of that would move with the author's account and so say who
//! wrote the post. Site moderation gets the author another way:
//! [`Community::author_for_moderation`], and `RevealAuthor`, which is logged.
//!
//! **Removed content keeps resolving** for whoever could read it, for as
//! long as it is retained (see `retention`), with interactions off and a
//! neutral title: a post can still be reported after its author deleted it.
use std::sync::Arc;

use async_trait::async_trait;
use connectrpc::ErrorCode;
use tokuly_core::{
    prelude::*,
    realtime::TopicGuard,
    subject::{SubjectInfo, SubjectResolver},
};
use tokuly_proto::common::SubjectType;

use crate::{
    Community,
    access::Caller,
    model::state,
    retention::{CLOSED_COMMUNITY_TITLE, REMOVED_EVENT_TITLE, REMOVED_POST_TITLE},
    text,
};

/// Longest title of a reply as notifications and reports show it.
const REPLY_TITLE_CHARS: usize = 60;

/// Registers the resolvers of the four subject types and the topic guard.
pub(crate) fn register(community: &Community) {
    let core = &community.core;
    for kind in [
        SubjectType::SUBJECT_TYPE_COMMUNITY,
        SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD,
        SubjectType::SUBJECT_TYPE_COMMUNITY_REPLY,
        SubjectType::SUBJECT_TYPE_COMMUNITY_EVENT,
    ] {
        core.subjects.register(kind, Arc::new(Resolver { community: community.clone(), kind }));
    }
    core.topic_guards.register("community", Arc::new(Guard(community.clone())));
}

struct Resolver {
    community: Community,
    kind: SubjectType,
}

impl Community {
    async fn community_subject(&self, caller: &Caller, id: &str) -> AppResult<SubjectInfo> {
        let access = self.access(caller, id).await?;
        let community = &access.community;
        Ok(SubjectInfo {
            id: community.id,
            owner_id: community.owner_id,
            title: community.name.clone(),
            path: community.path(),
            image_url: community.icon_url(),
            interactions_enabled: access.ban.is_none(),
        })
    }

    /// Whether the caller could write in the channel right now: a member
    /// who is not timed out, of a role the channel's posting policy
    /// admits. Comments and reactions of the engagement domain are open to
    /// exactly these people — a comment under a thread must not be a way
    /// to write where a reply would be refused. (A banned account is not a
    /// member.)
    async fn may_post_in(&self, access: &crate::access::Access, channel_id: Uuid) -> AppResult<bool> {
        let Some(role) = access.role().filter(|_| access.timeout().is_none()) else { return Ok(false) };
        Ok(self.channel_by_id(channel_id).await?.is_some_and(|channel| channel.open_to(role)))
    }

    async fn thread_subject(&self, caller: &Caller, id: &str) -> AppResult<SubjectInfo> {
        let (thread, access, _) = self.open_thread(caller, id).await?;
        let may_post = self.may_post_in(&access, thread.channel_id).await?;
        Ok(SubjectInfo {
            id: thread.id,
            owner_id: owner_of(thread.author_id, thread.anonymous),
            title: thread.title.clone(),
            path: thread.path(&access.community),
            image_url: thread.image_url(),
            // A locked thread takes no comments; a hidden one nothing at all.
            interactions_enabled: thread.is_visible() && !thread.is_locked() && may_post,
        })
    }

    async fn reply_subject(&self, caller: &Caller, id: &str) -> AppResult<SubjectInfo> {
        let (reply, access, _) = self.open_reply(caller, id).await?;
        let may_post = self.may_post_in(&access, reply.channel_id).await?;
        let excerpt = text::excerpt(&reply.body, REPLY_TITLE_CHARS);
        Ok(SubjectInfo {
            id: reply.id,
            owner_id: owner_of(reply.author_id, reply.anonymous),
            title: if excerpt.is_empty() { reply.thread_title.clone() } else { excerpt },
            path: reply.path(&access.community),
            image_url: reply.images().into_iter().next().map(|image| image.url).filter(|url| !url.is_empty()),
            interactions_enabled: reply.is_visible() && reply.thread_locked_at.is_none() && may_post,
        })
    }

    async fn event_subject(&self, caller: &Caller, id: &str) -> AppResult<SubjectInfo> {
        let (event, access) = self.open_event(caller, id).await?;
        Ok(SubjectInfo {
            id: event.id,
            owner_id: event.organizer_id,
            title: event.title.clone(),
            path: event.path(&access.community),
            image_url: event.cover_url(),
            // Like answering the event: for members who are not timed out.
            interactions_enabled: !event.is_cancelled() && access.is_member() && access.timeout().is_none(),
        })
    }

    /// A removed subject (or one of a closed community) for someone who
    /// could read it while it was there: still reportable, nothing else.
    async fn removed_subject(&self, kind: SubjectType, caller: &Caller, id: &str) -> AppResult<Option<SubjectInfo>> {
        let removed = |id: Uuid, owner_id: Uuid, title: &str, path: String| SubjectInfo {
            id,
            owner_id,
            title: title.to_owned(),
            path,
            image_url: None,
            interactions_enabled: false,
        };
        let uuid = Uuid::parse_str(id.trim()).ok();
        // (community, subject ID, owner, title, was hidden from others,
        //  author for the block rule, removed on its own)
        let (community_id, subject_id, owner_id, title, hidden_from_others, named_author, removed_itself) = match kind {
            SubjectType::SUBJECT_TYPE_COMMUNITY => {
                let Some(community) = self.find_retained_community(id).await? else { return Ok(None) };
                (community.id, community.id, community.owner_id, CLOSED_COMMUNITY_TITLE, false, None, false)
            }
            SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD => {
                let Some(thread) = self.thread_retained(id).await? else { return Ok(None) };
                (
                    thread.community_id,
                    thread.id,
                    owner_of(thread.author_id, thread.anonymous),
                    REMOVED_POST_TITLE,
                    thread.state == state::HIDDEN,
                    (!thread.anonymous).then_some(thread.author_id),
                    thread.deleted_at.is_some(),
                )
            }
            SubjectType::SUBJECT_TYPE_COMMUNITY_REPLY => {
                let Some(reply) = (match uuid {
                    Some(uuid) => self.reply_retained(uuid).await?,
                    None => None,
                }) else {
                    return Ok(None);
                };
                (
                    reply.community_id,
                    reply.id,
                    owner_of(reply.author_id, reply.anonymous),
                    REMOVED_POST_TITLE,
                    reply.state == state::HIDDEN || reply.thread_state == state::HIDDEN,
                    (!reply.anonymous).then_some(reply.author_id),
                    reply.deleted_at.is_some() || reply.thread_deleted_at.is_some(),
                )
            }
            SubjectType::SUBJECT_TYPE_COMMUNITY_EVENT => {
                let Some(event) = (match uuid {
                    Some(uuid) => self.event_retained(uuid).await?,
                    None => None,
                }) else {
                    return Ok(None);
                };
                let removed = event.deleted_at.is_some();
                (
                    event.community_id,
                    event.id,
                    event.organizer_id,
                    REMOVED_EVENT_TITLE,
                    false,
                    Some(event.organizer_id),
                    removed,
                )
            }
            _ => return Ok(None),
        };
        let Some(community) = self.community_retained(community_id).await? else { return Ok(None) };
        // What is still there is for the ordinary resolvers to decide.
        if !removed_itself && community.deleted_at.is_none() {
            return Ok(None);
        }
        let access = self.stance(caller, community).await?;
        // The same people as before the removal: readers of the community
        // (for a community itself: whoever could know it existed)…
        let could_see = match kind {
            SubjectType::SUBJECT_TYPE_COMMUNITY => {
                !access.community.is_hidden() || access.is_member() || access.site_moderator
            }
            _ => access.can_read(),
        };
        // …a post that was hidden then is still only for its author and the
        // staff, and a block still hides named posts.
        let own = access.user_id.is_some_and(|user_id| Some(user_id) == named_author || user_id == owner_id);
        let hidden = hidden_from_others && !own && !access.sees_hidden_posts();
        let blocked = match (access.user_id, named_author) {
            (Some(viewer), Some(author)) if viewer != author && kind != SubjectType::SUBJECT_TYPE_COMMUNITY_EVENT => {
                self.blocked(Some(viewer)).await?.contains(&author)
            }
            _ => false,
        };
        if !could_see || hidden || blocked {
            return Ok(None);
        }
        Ok(Some(removed(subject_id, owner_id, title, access.community.path())))
    }

    /// Applies a like or comment counter of a thread, a reply or an event.
    async fn apply_counter(&self, kind: SubjectType, id: Uuid, counter: &str, delta: i64) -> AppResult<()> {
        // Only these two are denormalized; the column name never comes from outside.
        let column = match counter {
            "like_count" => "like_count",
            "comment_count" => "comment_count",
            _ => return Ok(()),
        };
        let db = &self.core.db;
        match kind {
            SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD => {
                sqlx::query(sqlx::AssertSqlSafe(format!(
                    "UPDATE community_threads SET {column} = greatest({column} + $2, 0)
                     WHERE id = $1 AND deleted_at IS NULL"
                )))
                .bind(id)
                .bind(delta)
                .execute(db)
                .await?;
                // The hot score moves with the counters it is made of.
                sqlx::query(
                    "UPDATE community_threads SET hot_score = community_hot_score(
                         reply_count, like_count, comment_count, created_at, last_activity_at, now())
                     WHERE id = $1 AND deleted_at IS NULL",
                )
                .bind(id)
                .execute(db)
                .await?;
                self.push_thread_counts(id).await;
            }
            SubjectType::SUBJECT_TYPE_COMMUNITY_REPLY => {
                sqlx::query(sqlx::AssertSqlSafe(format!(
                    "UPDATE community_replies SET {column} = greatest({column} + $2, 0) WHERE id = $1"
                )))
                .bind(id)
                .bind(delta)
                .execute(db)
                .await?;
            }
            SubjectType::SUBJECT_TYPE_COMMUNITY_EVENT => {
                sqlx::query(sqlx::AssertSqlSafe(format!(
                    "UPDATE community_events SET {column} = greatest({column} + $2, 0) WHERE id = $1"
                )))
                .bind(id)
                .bind(delta)
                .execute(db)
                .await?;
            }
            _ => {}
        }
        Ok(())
    }
}

#[async_trait]
impl SubjectResolver for Resolver {
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let community = &self.community;
        let caller = Caller::load(&community.core, viewer).await?;
        let resolved = match self.kind {
            SubjectType::SUBJECT_TYPE_COMMUNITY => community.community_subject(&caller, id).await,
            SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD => community.thread_subject(&caller, id).await,
            SubjectType::SUBJECT_TYPE_COMMUNITY_REPLY => community.reply_subject(&caller, id).await,
            SubjectType::SUBJECT_TYPE_COMMUNITY_EVENT => community.event_subject(&caller, id).await,
            _ => return Ok(None),
        };
        match resolved {
            Ok(info) => Ok(Some(info)),
            // "Does not exist" and "not for you" are the same answer here —
            // unless it was removed and is still retained.
            Err(error)
                if matches!(
                    error.code,
                    ErrorCode::NotFound | ErrorCode::PermissionDenied | ErrorCode::InvalidArgument
                ) =>
            {
                community.removed_subject(self.kind, &caller, id).await
            }
            Err(error) => Err(error),
        }
    }

    async fn counter_changed(&self, id: Uuid, counter: &str, delta: i64) -> AppResult<()> {
        self.community.apply_counter(self.kind, id, counter, delta).await
    }
}

/// The owner the generic domains are told: the author, or nobody for a
/// post in an anonymous channel.
fn owner_of(author_id: Uuid, anonymous: bool) -> Uuid {
    if anonymous { Uuid::nil() } else { author_id }
}

/// Who may follow `community:<id>`: everyone for a public community,
/// members (and site moderation) otherwise.
struct Guard(Community);

#[async_trait]
impl TopicGuard for Guard {
    async fn may_subscribe(&self, subscriber: Option<Uuid>, topic: &str) -> AppResult<bool> {
        let community = &self.0;
        let Some(id) = topic.strip_prefix("community:").and_then(|id| Uuid::parse_str(id).ok()) else {
            return Ok(false);
        };
        let Some(row) = community.community_by_id(id).await? else { return Ok(false) };
        if row.is_public() {
            return Ok(true);
        }
        let Some(user_id) = subscriber else { return Ok(false) };
        if community.member(id, user_id).await?.is_some() {
            return Ok(true);
        }
        Ok(Caller::load(&community.core, Some(user_id)).await?.site_moderator)
    }
}
