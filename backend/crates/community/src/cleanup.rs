//! Reactions to what other domains announce: erased accounts, reports,
//! site-level takedowns and uploads deleted by force.
//!
//! Every handler is idempotent: events are delivered at least once.
use tokuly_core::{events::Event, outbound, prelude::*};
use tokuly_proto::{common::SubjectType, community::ModerationAction};

use crate::{
    Community,
    events::{AssetDeleted, ContentRemoved, ReportCreated, UserDeleted},
    model::{CommunityRow, community_columns},
    moderation::Post,
    modlog::{self, LogEntry},
    notices::{self, MembershipNotice},
};

/// The event topics this domain consumes.
pub const CONSUMED_TOPICS: [&str; 4] =
    ["identity.user.deleted", "engagement.report.created", "admin.content.removed", "media.asset.deleted"];

const THREAD: i32 = SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD as i32;
const REPLY: i32 = SubjectType::SUBJECT_TYPE_COMMUNITY_REPLY as i32;
const COMMUNITY: i32 = SubjectType::SUBJECT_TYPE_COMMUNITY as i32;
const EVENT: i32 = SubjectType::SUBJECT_TYPE_COMMUNITY_EVENT as i32;

impl Community {
    /// Entry point of the durable consumer `community`.
    pub async fn handle_event(&self, event: Event) -> AppResult<()> {
        match event.topic.as_str() {
            "identity.user.deleted" => self.on_user_deleted(event.data::<UserDeleted>()?.user_id).await,
            "engagement.report.created" => self.on_report_created(event.data()?).await,
            "admin.content.removed" => self.on_content_removed(event.data()?).await,
            "media.asset.deleted" => self.on_asset_deleted(event.data::<AssetDeleted>()?.asset_id).await,
            _ => Ok(()),
        }
    }

    /// An account was erased. Its posts stay (they render as "deleted
    /// user"); everything personal goes: memberships, requests, votes,
    /// answers to events, follows and read markers. A community it owned
    /// passes to the longest-serving admin — failing that the
    /// longest-serving moderator, then member — and closes when nobody is left.
    ///
    /// Nothing that others can see about an anonymous post may change here,
    /// or erasing an account would say which posts were its own: the post
    /// keeps its pseudonym, and so do the log entries about it. For the
    /// same reason counters that others watch (how many voted in a poll,
    /// how many reported a post) stay as they were — the vote and the
    /// report stop naming the account instead.
    async fn on_user_deleted(&self, user_id: Uuid) -> AppResult<()> {
        let db = &self.core.db;
        let owned: Vec<CommunityRow> = sqlx::query_as(concat!(
            "SELECT ",
            community_columns!(),
            " FROM communities c WHERE c.owner_id = $1 AND c.deleted_at IS NULL ORDER BY c.id"
        ))
        .bind(user_id)
        .fetch_all(db)
        .await?;
        for community in owned {
            let mut tx = db.begin().await?;
            let heir: Option<Uuid> = sqlx::query_scalar(
                "SELECT m.user_id FROM community_members m JOIN users u ON u.id = m.user_id
                 WHERE m.community_id = $1 AND m.user_id <> $2 AND u.deleted_at IS NULL AND u.banned_at IS NULL
                 ORDER BY m.role DESC, m.joined_at, m.user_id LIMIT 1 FOR UPDATE OF m",
            )
            .bind(community.id)
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
            match heir {
                Some(heir) => {
                    let updated = self.set_owner(&mut tx, &community, heir).await?;
                    let entry = LogEntry::new(community.id, ModerationAction::MODERATION_ACTION_OWNERSHIP_TRANSFERRED)
                        .target(heir)
                        .detail("cause", "account_deleted");
                    modlog::record(&mut *tx, &entry).await?;
                    outbound::notify(&mut *tx, &notices::membership(heir, &updated, &MembershipNotice::Ownership))
                        .await?;
                    tx.commit().await?;
                    self.push_membership(&updated, heir).await;
                }
                None => {
                    let closed = self.close_community(&mut tx, &community, None).await?;
                    tx.commit().await?;
                    if closed {
                        self.after_close(community.id).await;
                    }
                }
            }
        }

        // Memberships, one community at a time: each takes the member's
        // answers to coming events with it and lets the waitlist move up.
        let joined: Vec<CommunityRow> = sqlx::query_as(concat!(
            "SELECT ",
            community_columns!(),
            " FROM community_members m JOIN communities c ON c.id = m.community_id WHERE m.user_id = $1 ORDER BY c.id"
        ))
        .bind(user_id)
        .fetch_all(db)
        .await?;
        for community in joined {
            let mut tx = db.begin().await?;
            let departure = self.remove_member(&mut tx, &community, user_id, "account_deleted", None).await?;
            tx.commit().await?;
            if let Some(departure) = departure
                && community.deleted_at.is_none()
            {
                self.push_community_changed(community.id).await;
                for event_id in &departure.events {
                    self.push_event_changed(&community, *event_id).await;
                }
            }
        }

        let mut tx = db.begin().await?;
        for statement in [
            "DELETE FROM community_join_requests WHERE user_id = $1",
            "UPDATE community_join_requests SET declined_by = NULL WHERE declined_by = $1",
            "UPDATE community_invites SET revoked_at = now() WHERE created_by = $1 AND revoked_at IS NULL",
            "DELETE FROM community_bans WHERE user_id = $1",
            "UPDATE community_bans SET banned_by = NULL WHERE banned_by = $1",
            "DELETE FROM community_thread_follows WHERE user_id = $1",
            "DELETE FROM community_thread_reads WHERE user_id = $1",
            "DELETE FROM community_channel_reads WHERE user_id = $1",
            // A report still counts; it no longer says who made it.
            "UPDATE community_reports SET reporter_id = uuidv7() WHERE reporter_id = $1",
            // The votes go; what the polls count does not move.
            "DELETE FROM community_poll_votes WHERE user_id = $1",
            // Answers to events that are over; the counters of those events
            // are history and stay as they were.
            "DELETE FROM community_event_attendees WHERE user_id = $1",
            "UPDATE community_event_attendees SET checked_in_by = NULL WHERE checked_in_by = $1",
            "UPDATE community_moderation_log SET actor_id = NULL WHERE actor_id = $1",
            // An entry about an anonymous post keeps the key its pseudonym
            // is made from, exactly like the post itself does.
            "UPDATE community_moderation_log SET target_user_id = NULL WHERE target_user_id = $1 AND NOT target_anonymous",
        ] {
            sqlx::query(statement).bind(user_id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Someone reported a thread or a reply. When enough different members
    /// have, the post is hidden until the staff reviewed it. Reports by
    /// outsiders do not count (a public community must not be something
    /// passers-by can silence), nor do reports against the staff's named
    /// posts, nor against posts the staff already restored.
    async fn on_report_created(&self, report: ReportCreated) -> AppResult<()> {
        // Everything is read before the transaction opens: nothing below
        // asks the pool for a second connection while holding one.
        let post = match report.subject_type {
            THREAD => self.thread_by_id(report.subject_id).await?.map(Post::Thread),
            REPLY => self.reply_by_id(report.subject_id).await?.map(Post::Reply),
            _ => None,
        };
        let Some(post) = post else { return Ok(()) };
        let (author_id, anonymous) = post.author();
        let Some(community) = self.community_by_id(post.community_id()).await? else { return Ok(()) };
        if report.reporter_id == author_id || self.member(community.id, report.reporter_id).await?.is_none() {
            return Ok(());
        }
        // The staff's own posts are not hidden by reports — except in an
        // anonymous channel, where a post that cannot be hidden would be
        // known to be the staff's.
        let exempt =
            !anonymous && self.member(community.id, author_id).await?.is_some_and(|member| member.role().is_staff());
        let threshold = i64::from(self.auto_hide_threshold(&community));

        let mut tx = self.core.db.begin().await?;
        // Reports on one post take turns: each counts the ones before it,
        // so the report that reaches the threshold is the one that hides.
        let lock = match &post {
            Post::Thread(_) => "SELECT 1 FROM community_threads WHERE id = $1 FOR UPDATE",
            Post::Reply(_) => "SELECT 1 FROM community_replies WHERE id = $1 FOR UPDATE",
        };
        sqlx::query(lock).bind(report.subject_id).execute(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO community_reports (subject_type, subject_id, reporter_id, community_id, report_id)
             VALUES ($1, $2, $3, $4, $5) ON CONFLICT (subject_type, subject_id, reporter_id) DO NOTHING",
        )
        .bind(report.subject_type)
        .bind(report.subject_id)
        .bind(report.reporter_id)
        .bind(community.id)
        .bind(report.report_id)
        .execute(&mut *tx)
        .await?;
        let reports: i64 =
            sqlx::query_scalar("SELECT count(*) FROM community_reports WHERE subject_type = $1 AND subject_id = $2")
                .bind(report.subject_type)
                .bind(report.subject_id)
                .fetch_one(&mut *tx)
                .await?;
        let mut hidden_thread = None;
        let mut hidden_reply = None;
        if reports >= threshold && !exempt {
            match post {
                Post::Thread(thread) => {
                    if self.hide_thread(&mut tx, &community, &thread, reports).await? {
                        hidden_thread = Some(thread);
                    }
                }
                Post::Reply(reply) => {
                    if self.hide_reply(&mut tx, &community, &reply, reports).await? {
                        hidden_reply = Some(reply);
                    }
                }
            }
        }
        tx.commit().await?;
        if let Some(thread) = &hidden_thread {
            self.push_thread_removed(thread).await;
        }
        if let Some(reply) = &hidden_reply {
            self.push_reply_removed(reply).await;
        }
        if hidden_thread.is_some() || hidden_reply.is_some() {
            self.push_staff_queues(community.id).await;
        }
        Ok(())
    }

    /// Site moderation took something down. The admin domain tells the
    /// owner; this removes the content and leaves a line in the
    /// community's own log. Unknown or already removed content is done.
    async fn on_content_removed(&self, removed: ContentRemoved) -> AppResult<()> {
        let db = &self.core.db;
        let log = |community_id: Uuid| {
            let entry = LogEntry::new(community_id, ModerationAction::MODERATION_ACTION_CONTENT_TAKEN_DOWN)
                .site(true)
                .reason(&removed.reason);
            match removed.moderator_id {
                Some(moderator_id) => entry.by(moderator_id),
                None => entry,
            }
        };
        match removed.subject_type {
            COMMUNITY => {
                let Some(community) = self.community_by_id(removed.subject_id).await? else { return Ok(()) };
                let mut tx = db.begin().await?;
                let closed = self.close_community(&mut tx, &community, removed.moderator_id).await?;
                tx.commit().await?;
                if closed {
                    self.after_close(community.id).await;
                }
            }
            THREAD => {
                let Some(thread) = self.thread_by_id(removed.subject_id).await? else { return Ok(()) };
                let mut tx = db.begin().await?;
                if self.remove_thread(&mut tx, &thread, removed.moderator_id).await? {
                    let entry = log(thread.community_id)
                        .thread(thread.id)
                        .channel(thread.channel_id)
                        .author(thread.author_id, thread.anonymous)
                        .detail("title", &thread.title);
                    modlog::record(&mut *tx, &entry).await?;
                }
                tx.commit().await?;
                self.push_thread_removed(&thread).await;
            }
            REPLY => {
                let Some(reply) = self.reply_by_id(removed.subject_id).await? else { return Ok(()) };
                let mut tx = db.begin().await?;
                if self.remove_reply(&mut tx, &reply).await? {
                    let entry = log(reply.community_id)
                        .reply(reply.thread_id, reply.id)
                        .channel(reply.channel_id)
                        .author(reply.author_id, reply.anonymous);
                    modlog::record(&mut *tx, &entry).await?;
                }
                tx.commit().await?;
                self.push_reply_removed(&reply).await;
            }
            EVENT => {
                let Some(event) = self.event_by_id(removed.subject_id).await? else { return Ok(()) };
                let mut tx = db.begin().await?;
                if self.remove_event(&mut tx, &event).await? {
                    let entry = log(event.community_id)
                        .event(event.id)
                        .target(event.organizer_id)
                        .detail("title", &event.title);
                    modlog::record(&mut *tx, &entry).await?;
                }
                tx.commit().await?;
                if let Some(community) = self.community_by_id(event.community_id).await? {
                    self.push_event_changed(&community, event.id).await;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// An upload was deleted by force (posts hold their uploads, so owners
    /// and the janitor cannot delete them: this is a moderation takedown
    /// of the file). Whatever showed it stops showing it; a reply that was
    /// nothing but the image goes with it.
    async fn on_asset_deleted(&self, asset_id: Uuid) -> AppResult<()> {
        let db = &self.core.db;
        for statement in [
            "UPDATE communities SET icon = NULL, icon_asset_id = NULL WHERE icon_asset_id = $1",
            "UPDATE communities SET banner = NULL, banner_asset_id = NULL WHERE banner_asset_id = $1",
            "UPDATE community_events SET cover = NULL, cover_asset_id = NULL WHERE cover_asset_id = $1",
        ] {
            sqlx::query(statement).bind(asset_id).execute(db).await?;
        }
        // Stored images are proto JSON: the asset is named by `assetId`.
        sqlx::query(
            "UPDATE community_threads SET
                 asset_ids = array_remove(asset_ids, $1),
                 images = (SELECT coalesce(jsonb_agg(image), '[]'::jsonb) FROM jsonb_array_elements(images) AS image
                           WHERE image->>'assetId' IS DISTINCT FROM $2)
             WHERE asset_ids <> '{}' AND asset_ids @> ARRAY[$1]",
        )
        .bind(asset_id)
        .bind(asset_id.to_string())
        .execute(db)
        .await?;
        let emptied: Vec<Uuid> = sqlx::query_scalar(
            "UPDATE community_replies SET
                 asset_ids = array_remove(asset_ids, $1),
                 images = (SELECT coalesce(jsonb_agg(image), '[]'::jsonb) FROM jsonb_array_elements(images) AS image
                           WHERE image->>'assetId' IS DISTINCT FROM $2)
             WHERE asset_ids <> '{}' AND asset_ids @> ARRAY[$1]
             RETURNING CASE WHEN body = '' AND stamp_id IS NULL AND asset_ids = '{}' AND deleted_at IS NULL
                            THEN id END",
        )
        .bind(asset_id)
        .bind(asset_id.to_string())
        .fetch_all(db)
        .await
        .map(|ids: Vec<Option<Uuid>>| ids.into_iter().flatten().collect())?;
        for reply_id in emptied {
            let Some(reply) = self.reply_by_id(reply_id).await? else { continue };
            let mut tx = db.begin().await?;
            let removed = self.remove_reply(&mut tx, &reply).await?;
            tx.commit().await?;
            if removed {
                self.push_reply_removed(&reply).await;
            }
        }
        Ok(())
    }
}
