//! The `community` section of the personal data export: everything this
//! domain stores about one user — including their posts in anonymous
//! channels, which are theirs even though nobody else sees that.
use async_trait::async_trait;
use tokuly_core::{export::DataExporter, prelude::*};

use crate::Community;

pub(crate) struct CommunityExporter {
    community: Community,
}

impl CommunityExporter {
    pub(crate) fn new(community: Community) -> Self {
        Self { community }
    }
}

/// Runs a query that returns one JSON array (`[]` when there are no rows).
async fn rows(core: &Core, query: &'static str, user_id: Uuid) -> AppResult<serde_json::Value> {
    let value: Option<serde_json::Value> = sqlx::query_scalar(query).bind(user_id).fetch_one(&core.db).await?;
    Ok(value.unwrap_or_else(|| serde_json::json!([])))
}

#[async_trait]
impl DataExporter for CommunityExporter {
    fn name(&self) -> &'static str {
        "community"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        let core = &self.community.core;
        let memberships = rows(
            core,
            "SELECT jsonb_agg(jsonb_build_object(
                 'community_id', c.id, 'handle', c.handle::text, 'name', c.name, 'role', m.role,
                 'notification_level', m.notification_level, 'show_on_profile', m.show_on_profile,
                 'timeout_until', m.timeout_until, 'joined_at', m.joined_at, 'owner', c.owner_id = m.user_id)
                 ORDER BY m.joined_at)
             FROM community_members m JOIN communities c ON c.id = m.community_id
             WHERE m.user_id = $1 AND c.deleted_at IS NULL",
            user_id,
        )
        .await?;
        let join_requests = rows(
            core,
            "SELECT jsonb_agg(jsonb_build_object('community_id', community_id, 'answers', answers, 'created_at', created_at)
                 ORDER BY created_at)
             FROM community_join_requests WHERE user_id = $1",
            user_id,
        )
        .await?;
        let invites = rows(
            core,
            "SELECT jsonb_agg(jsonb_build_object(
                 'community_id', community_id, 'code', code, 'note', note, 'max_uses', max_uses, 'use_count', use_count,
                 'expires_at', expires_at, 'revoked_at', revoked_at, 'created_at', created_at) ORDER BY created_at)
             FROM community_invites WHERE created_by = $1",
            user_id,
        )
        .await?;
        let bans = rows(
            core,
            "SELECT jsonb_agg(jsonb_build_object(
                 'community_id', community_id, 'reason', reason, 'expires_at', expires_at, 'created_at', created_at)
                 ORDER BY created_at)
             FROM community_bans WHERE user_id = $1",
            user_id,
        )
        .await?;
        let threads = rows(
            core,
            "SELECT jsonb_agg(jsonb_build_object(
                 'id', id, 'community_id', community_id, 'channel_id', channel_id, 'anonymous', anonymous,
                 'title', title, 'body', body, 'images', images, 'stamp_id', stamp_id, 'tags', tags,
                 'created_at', created_at, 'edited_at', edited_at) ORDER BY id)
             FROM community_threads WHERE author_id = $1 AND deleted_at IS NULL",
            user_id,
        )
        .await?;
        let replies = rows(
            core,
            "SELECT jsonb_agg(jsonb_build_object(
                 'id', id, 'thread_id', thread_id, 'community_id', community_id, 'anonymous', anonymous, 'body', body,
                 'images', images, 'stamp_id', stamp_id, 'reply_to_id', reply_to_id, 'created_at', created_at,
                 'edited_at', edited_at) ORDER BY id)
             FROM community_replies WHERE author_id = $1 AND deleted_at IS NULL",
            user_id,
        )
        .await?;
        let poll_votes = rows(
            core,
            "SELECT jsonb_agg(jsonb_build_object('thread_id', v.thread_id, 'option', o.text, 'voted_at', v.voted_at)
                 ORDER BY v.voted_at)
             FROM community_poll_votes v JOIN community_poll_options o ON o.id = v.option_id WHERE v.user_id = $1",
            user_id,
        )
        .await?;
        let followed_threads = rows(
            core,
            "SELECT jsonb_agg(jsonb_build_object('thread_id', thread_id, 'since', created_at) ORDER BY created_at)
             FROM community_thread_follows WHERE user_id = $1 AND following",
            user_id,
        )
        .await?;
        let events_organized = rows(
            core,
            "SELECT jsonb_agg(jsonb_build_object(
                 'id', id, 'community_id', community_id, 'title', title, 'description', description,
                 'starts_at', starts_at, 'ends_at', ends_at, 'online', online, 'place_name', place_name,
                 'cancelled_at', cancelled_at, 'created_at', created_at) ORDER BY id)
             FROM community_events WHERE organizer_id = $1 AND deleted_at IS NULL",
            user_id,
        )
        .await?;
        let event_answers = rows(
            core,
            "SELECT jsonb_agg(jsonb_build_object(
                 'event_id', a.event_id, 'title', e.title, 'status', a.status, 'responded_at', a.responded_at,
                 'checked_in_at', a.checked_in_at) ORDER BY a.responded_at)
             FROM community_event_attendees a JOIN community_events e ON e.id = a.event_id WHERE a.user_id = $1",
            user_id,
        )
        .await?;
        let moderation_actions = rows(
            core,
            "SELECT jsonb_agg(jsonb_build_object(
                 'community_id', community_id, 'action', action, 'reason', reason, 'created_at', created_at)
                 ORDER BY id)
             FROM community_moderation_log WHERE actor_id = $1",
            user_id,
        )
        .await?;
        Ok(serde_json::json!({
            "memberships": memberships,
            "join_requests": join_requests,
            "invites": invites,
            "bans": bans,
            "threads": threads,
            "replies": replies,
            "poll_votes": poll_votes,
            "followed_threads": followed_threads,
            "events_organized": events_organized,
            "event_answers": event_answers,
            "moderation_actions": moderation_actions,
        }))
    }
}
