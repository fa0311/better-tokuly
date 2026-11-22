//! Reactions to other domains' events, periodic pruning and the personal
//! data export.
use std::time::Duration;

use async_trait::async_trait;
use tokuly_core::{
    events::{self, ConsumeOptions, Event},
    export::DataExporter,
    prelude::*,
};
use tokuly_identity::maintenance::{UserCreated, UserRef};
use tokuly_proto::social::social_event::Event as Push;

use crate::{Social, idcache, map_reactions, nearby, ranking};

const USER_CREATED: &str = "identity.user.created";
const USER_DELETED: &str = "identity.user.deleted";
const PRIVACY_CHANGED: &str = "identity.user.privacy_changed";
const MUTE_CHANGED: &str = "notify.mute.changed";

/// Payload of `notify.mute.changed`: `user_id` muted or unmuted
/// `muted_user_id` from the notification settings.
#[derive(Debug, serde::Deserialize)]
struct MuteChanged {
    user_id: Uuid,
    muted_user_id: Uuid,
    muted: bool,
}

/// How often [`Social::prune`] runs.
pub(crate) const PRUNE_INTERVAL: Duration = Duration::from_secs(300);
/// Day buckets are kept a little longer than the scoring window needs.
const INTERACTION_RETENTION_DAYS: i64 = ranking::WINDOW_DAYS + 15;
/// A declined request keeps looking pending to its sender for this long;
/// after that it is dropped and they may ask again.
const DECLINED_REQUEST_RETENTION_DAYS: i32 = 90;

impl Social {
    /// Consumes the events this domain reacts to until shutdown.
    pub(crate) async fn run_event_consumer(&self) {
        let social = self.clone();
        events::consume(
            self.core.bus.clone(),
            "social",
            &[USER_CREATED, USER_DELETED, PRIVACY_CHANGED, MUTE_CHANGED],
            ConsumeOptions::default(),
            self.core.shutdown.clone(),
            move |event| {
                let social = social.clone();
                async move { social.handle_event(&event).await }
            },
        )
        .await;
    }

    /// Reacts to one event from another domain. Every reaction is idempotent
    /// (events are delivered at least once). Public so tests can deliver
    /// events directly.
    pub async fn handle_event(&self, event: &Event) -> AppResult<()> {
        match event.topic.as_str() {
            USER_CREATED => {
                let created: UserCreated = event.data()?;
                self.record_intro(created.user_id, &created.invite_code).await
            }
            USER_DELETED => self.forget_user(event.data::<UserRef>()?.user_id).await,
            PRIVACY_CHANGED => self.refresh_presence(event.data::<UserRef>()?.user_id).await,
            MUTE_CHANGED => self.mirror_mute(event.data()?).await,
            _ => Ok(()),
        }
    }

    /// Keeps a friend's `muted` flag in step with a mute made in the
    /// notification settings. Deliberately silent: announcing it as
    /// `social.friend.prefs_changed` would send the change straight back to
    /// the notify domain it came from.
    async fn mirror_mute(&self, change: MuteChanged) -> AppResult<()> {
        sqlx::query("UPDATE friendships SET muted = $3 WHERE user_id = $1 AND friend_id = $2 AND muted <> $3")
            .bind(change.user_id)
            .bind(change.muted_user_id)
            .bind(change.muted)
            .execute(&self.core.db)
            .await?;
        Ok(())
    }

    /// Removes everything this domain holds about a deleted account and
    /// makes the account disappear from its former friends' apps.
    async fn forget_user(&self, user_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        let friends: Vec<Uuid> = sqlx::query_scalar("SELECT friend_id FROM friendships WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(&core.db)
            .await?;
        let in_a_block: Vec<Uuid> = sqlx::query_scalar(
            "SELECT blocked_id FROM user_blocks WHERE blocker_id = $1 UNION SELECT blocker_id FROM user_blocks WHERE blocked_id = $1",
        )
        .bind(user_id)
        .fetch_all(&core.db)
        .await?;

        // Each statement removes the rows in which the user is either party.
        // (`friend_interaction_days` goes with `friend_pairs`.)
        const DELETES: &[&str] = &[
            "DELETE FROM friendships WHERE user_id = $1 OR friend_id = $1",
            "DELETE FROM friend_requests WHERE from_user_id = $1 OR to_user_id = $1",
            "DELETE FROM user_blocks WHERE blocker_id = $1 OR blocked_id = $1",
            "DELETE FROM friend_suggestions WHERE user_id = $1 OR suggested_id = $1",
            "DELETE FROM friend_suggestion_dismissals WHERE user_id = $1 OR dismissed_id = $1",
            "DELETE FROM friend_invites WHERE user_id = $1",
            "DELETE FROM friend_intros WHERE invitee_id = $1 OR inviter_id = $1",
            "DELETE FROM friend_pairs WHERE user_lo = $1 OR user_hi = $1",
            "DELETE FROM bumps WHERE user_lo = $1 OR user_hi = $1",
            "DELETE FROM map_reactions WHERE from_user_id = $1 OR to_user_id = $1",
            "DELETE FROM map_views WHERE viewed_id = $1 OR viewer_id = $1",
        ];
        let mut tx = core.db.begin().await?;
        for delete in DELETES {
            sqlx::query(*delete).bind(user_id).execute(&mut *tx).await?;
        }
        tx.commit().await?;

        let mut stale_friend_lists = friends.clone();
        stale_friend_lists.push(user_id);
        idcache::FRIENDS.invalidate(core, &stale_friend_lists).await;
        let mut stale_hidden_lists = in_a_block;
        stale_hidden_lists.push(user_id);
        idcache::HIDDEN.invalidate(core, &stale_hidden_lists).await;
        // Valkey entries expire by themselves; failing here must not make the
        // whole cleanup retry.
        let _ = nearby::NEARBY.leave(&core.cache, user_id).await;
        let _ = self.bump_attempts().leave(&core.cache, user_id).await;

        self.recompute_ranks(&friends).await?;
        self.push_many(friends, Push::FriendRemovedUserId(user_id.to_string())).await;
        Ok(())
    }

    /// Deletes what has aged out: interaction buckets beyond the scoring
    /// window, map views past their 24 hours, map reactions nobody will
    /// replay anymore, requests declined long ago, and stale members of the
    /// live GEO sets. Run by the maintenance worker; public so tests and
    /// operators can trigger it.
    pub async fn prune(&self) -> AppResult<()> {
        let core = &self.core;
        sqlx::query("DELETE FROM friend_requests WHERE declined_at < now() - make_interval(days => $1)")
            .bind(DECLINED_REQUEST_RETENTION_DAYS)
            .execute(&core.db)
            .await?;
        sqlx::query("DELETE FROM friend_interaction_days WHERE day < current_date - $1::int")
            .bind(INTERACTION_RETENTION_DAYS as i32)
            .execute(&core.db)
            .await?;
        // Views are shown for 24 hours; a second day covers clock edges.
        sqlx::query("DELETE FROM map_views WHERE viewed_at < now() - interval '48 hours'").execute(&core.db).await?;
        sqlx::query("DELETE FROM map_reactions WHERE created_at < now() - make_interval(days => $1)")
            .bind(map_reactions::PENDING_DAYS)
            .execute(&core.db)
            .await?;
        nearby::NEARBY.prune(&core.cache).await?;
        self.bump_attempts().prune(&core.cache).await?;
        Ok(())
    }
}

/// The `social` section of a personal data export.
pub(crate) struct SocialExporter {
    core: Core,
}

impl SocialExporter {
    pub(crate) fn new(core: Core) -> Self {
        Self { core }
    }

    /// Runs a query that aggregates its rows into one JSON array.
    async fn list(&self, sql: &'static str, user_id: Uuid) -> AppResult<serde_json::Value> {
        Ok(sqlx::query_scalar(sql).bind(user_id).fetch_one(&self.core.db).await?)
    }
}

#[async_trait]
impl DataExporter for SocialExporter {
    fn name(&self) -> &'static str {
        "social"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        let invite_code: Option<String> = sqlx::query_scalar("SELECT code FROM friend_invites WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&self.core.db)
            .await?;
        let invited_by: Option<Uuid> = sqlx::query_scalar("SELECT inviter_id FROM friend_intros WHERE invitee_id = $1")
            .bind(user_id)
            .fetch_optional(&self.core.db)
            .await?;
        Ok(serde_json::json!({
            "friends": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object(
                     'user_id', friend_id, 'since', created_at, 'nickname', nickname, 'favorite', favorite, 'muted', muted,
                     'best_friend_rank', best_friend_rank) ORDER BY created_at), '[]'::jsonb)
                 FROM friendships WHERE user_id = $1", user_id).await?,
            "friend_requests_sent": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('to_user_id', to_user_id, 'message', message, 'sent_at', created_at) ORDER BY id), '[]'::jsonb)
                 FROM friend_requests WHERE from_user_id = $1", user_id).await?,
            "friend_requests_received": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('from_user_id', from_user_id, 'message', message, 'sent_at', created_at,
                     'declined_at', declined_at) ORDER BY id), '[]'::jsonb)
                 FROM friend_requests WHERE to_user_id = $1", user_id).await?,
            "blocked_users": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('user_id', blocked_id, 'blocked_at', created_at) ORDER BY created_at), '[]'::jsonb)
                 FROM user_blocks WHERE blocker_id = $1", user_id).await?,
            "dismissed_suggestions": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('user_id', dismissed_id, 'dismissed_at', created_at) ORDER BY created_at), '[]'::jsonb)
                 FROM friend_suggestion_dismissals WHERE user_id = $1", user_id).await?,
            "invite_code": invite_code,
            "invited_by": invited_by,
            "intros": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('user_id', invitee_id, 'joined_at', created_at) ORDER BY created_at), '[]'::jsonb)
                 FROM friend_intros WHERE inviter_id = $1", user_id).await?,
            "shared_history": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object(
                     'with_user_id', CASE WHEN user_lo = $1 THEN user_hi ELSE user_lo END, 'bump_count', bump_count,
                     'last_bump_at', last_bump_at, 'time_together_seconds', time_together_seconds, 'nights_together', nights_together,
                     'streak_weeks', streak_weeks)), '[]'::jsonb)
                 FROM friend_pairs WHERE user_lo = $1 OR user_hi = $1", user_id).await?,
            "bumps": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object(
                     'id', id, 'with_user_id', CASE WHEN user_lo = $1 THEN user_hi ELSE user_lo END, 'lat', lat, 'lng', lng,
                     'place_name', place_name, 'at', created_at) ORDER BY id), '[]'::jsonb)
                 FROM bumps WHERE user_lo = $1 OR user_hi = $1", user_id).await?,
            "map_reactions_sent": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('to_user_id', to_user_id, 'kind', kind, 'emoji', emoji, 'stamp_id', stamp_id,
                     'at', created_at) ORDER BY id), '[]'::jsonb)
                 FROM map_reactions WHERE from_user_id = $1", user_id).await?,
            "map_reactions_received": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('from_user_id', from_user_id, 'kind', kind, 'emoji', emoji, 'stamp_id', stamp_id,
                     'at', created_at) ORDER BY id), '[]'::jsonb)
                 FROM map_reactions WHERE to_user_id = $1", user_id).await?,
            "map_views_made": self.list(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('user_id', viewed_id, 'at', viewed_at) ORDER BY viewed_at), '[]'::jsonb)
                 FROM map_views WHERE viewer_id = $1", user_id).await?,
        }))
    }
}
