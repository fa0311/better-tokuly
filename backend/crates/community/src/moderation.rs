//! The staff's tools: removing, banning and timing out members, the word
//! filter and auto-hide settings, slow mode, the review queue of hidden
//! posts, and looking up the author of an anonymous post.
use std::{collections::HashMap, time::Duration};

use sqlx::{Postgres, Transaction};
use tokuly_core::{events as core_events, outbound, prelude::*};
use tokuly_proto::{
    common::{PageInfo, SubjectType, UserSummary},
    community::{self as pb, MemberRole, ModerationAction, Permission, ReviewDecision},
};

use crate::{
    Community,
    access::{Access, permission_denied},
    events,
    model::{
        BanRow, ChannelRow, CommunityRow, MemberRow, ReplyRow, ThreadRow, channel_columns, hidden, member_columns,
        reply_select, state, thread_columns,
    },
    modlog::{self, LogEntry},
    notices::{self, MembershipNotice, PostNotice},
    render::View,
    replies::{self, count_reply},
    threads::{self, count_thread},
    validate,
};

/// Author lookups one staff member may make per hour.
const REVEALS_PER_HOUR: u32 = 30;

/// A thread or a reply the staff acts on.
pub(crate) enum Post {
    Thread(ThreadRow),
    Reply(ReplyRow),
}

impl Post {
    pub(crate) fn author(&self) -> (Uuid, bool) {
        match self {
            Self::Thread(thread) => (thread.author_id, thread.anonymous),
            Self::Reply(reply) => (reply.author_id, reply.anonymous),
        }
    }

    fn state(&self) -> i32 {
        match self {
            Self::Thread(thread) => thread.state,
            Self::Reply(reply) => reply.state,
        }
    }

    fn hidden_reason(&self) -> i32 {
        match self {
            Self::Thread(thread) => thread.hidden_reason,
            Self::Reply(reply) => reply.hidden_reason,
        }
    }

    /// The title of the thread the post is, or belongs to.
    fn thread_title(&self) -> &str {
        match self {
            Self::Thread(thread) => &thread.title,
            Self::Reply(reply) => &reply.thread_title,
        }
    }

    fn thread_id(&self) -> Uuid {
        match self {
            Self::Thread(thread) => thread.id,
            Self::Reply(reply) => reply.thread_id,
        }
    }

    fn path(&self, community: &CommunityRow) -> String {
        match self {
            Self::Thread(thread) => thread.path(community),
            Self::Reply(reply) => reply.path(community),
        }
    }

    /// A log entry about this post, naming its author (or, in the log's
    /// rendering, their pseudonym).
    fn log(&self, community_id: Uuid, action: ModerationAction) -> LogEntry {
        let (author, anonymous) = self.author();
        let entry = LogEntry::new(community_id, action).author(author, anonymous);
        match self {
            Self::Thread(thread) => entry.thread(thread.id).channel(thread.channel_id),
            Self::Reply(reply) => entry.reply(reply.thread_id, reply.id).channel(reply.channel_id),
        }
    }
}

impl Community {
    fn ban_proto(&self, ban: &BanRow, users: &HashMap<Uuid, UserSummary>) -> pb::Ban {
        pb::Ban {
            user: tokuly_identity::users::pick(users, ban.user_id).into(),
            reason: ban.reason.clone(),
            banned_by: ban.banned_by.map(|id| tokuly_identity::users::pick(users, id)).into(),
            expires_at: opt_ts(ban.expires_at),
            created_at: ts(ban.created_at),
            ..Default::default()
        }
    }

    /// Removes a member; they may join again. Needs MANAGE_MEMBERS and a
    /// higher role than the member's.
    pub(crate) async fn kick_member(&self, access: &Access, user_id: Uuid, reason: &str) -> AppResult<()> {
        access.require(Permission::PERMISSION_MANAGE_MEMBERS)?;
        let actor = access.user_id.ok_or_else(permission_denied)?;
        let reason = validate::reason(reason, false)?;
        self.require_lower_member(access, user_id).await?;
        self.sanction_limit(actor).await?;
        let community = &access.community;
        let below = access.role();
        let mut tx = self.core.db.begin().await?;
        // The role on file decides: a member promoted a moment ago stays.
        let departure = self.remove_member(&mut tx, community, user_id, "removed", below).await?;
        if departure.is_some() {
            let entry = LogEntry::new(community.id, ModerationAction::MODERATION_ACTION_MEMBER_REMOVED)
                .by(actor)
                .target(user_id)
                .reason(&reason);
            modlog::record(&mut *tx, &entry).await?;
            let notice = MembershipNotice::Removed { reason: reason.clone() };
            outbound::notify(&mut *tx, &notices::membership(user_id, community, &notice)).await?;
        }
        tx.commit().await?;
        match departure {
            Some(departure) => self.after_departure(community, user_id, &departure).await,
            // Gone or promoted between the check and the removal.
            None => self.require_lower_member(access, user_id).await.map(|_| ())?,
        }
        Ok(())
    }

    /// The hourly allowance of removals, bans and timeouts of one staff
    /// member: each of them notifies its target.
    async fn sanction_limit(&self, actor: Uuid) -> AppResult<()> {
        self.limit("sanction", actor, 120, Duration::from_secs(3600)).await
    }

    /// Bans a user, removing them if they are a member. Needs
    /// MANAGE_MEMBERS and, for a member, a higher role than theirs.
    pub(crate) async fn ban_member(
        &self,
        access: &Access,
        user_id: Uuid,
        reason: &str,
        expires_at: Option<Time>,
    ) -> AppResult<pb::Ban> {
        access.require(Permission::PERMISSION_MANAGE_MEMBERS)?;
        let actor = access.require_member()?;
        ensure!(actor.user_id != user_id, invalid, "CANNOT_TARGET_SELF", "you cannot ban yourself");
        let reason = validate::reason(reason, true)?;
        ensure!(
            expires_at.is_none_or(|expires| expires > now()),
            invalid,
            "EXPIRY_INVALID",
            "a ban ends in the future, or never"
        );
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL)")
                .bind(user_id)
                .fetch_one(&self.core.db)
                .await?;
        ensure!(exists, not_found, "USER_NOT_FOUND", "user not found");
        self.sanction_limit(actor.user_id).await?;
        let community = &access.community;

        let mut tx = self.core.db.begin().await?;
        // Joining takes the same lock on the community row, so a join and a
        // ban of one person happen one after the other: they end up banned
        // and out, never banned and in.
        sqlx::query("SELECT 1 FROM communities WHERE id = $1 FOR UPDATE").bind(community.id).execute(&mut *tx).await?;
        // The role on file decides whether the actor outranks the target.
        let target_role: Option<i32> = sqlx::query_scalar(
            "SELECT role FROM community_members WHERE community_id = $1 AND user_id = $2 FOR UPDATE",
        )
        .bind(community.id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(role) = target_role
            && !actor
                .role()
                .outranks(crate::permissions::Role::from_code(role).unwrap_or(crate::permissions::Role::Member))
        {
            return Err(permission_denied());
        }
        let departure = self.remove_member(&mut tx, community, user_id, "banned", None).await?;
        let ban: BanRow = sqlx::query_as(
            "INSERT INTO community_bans (community_id, user_id, reason, banned_by, expires_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (community_id, user_id) DO UPDATE
                 SET reason = EXCLUDED.reason, banned_by = EXCLUDED.banned_by, expires_at = EXCLUDED.expires_at,
                     created_at = now()
             RETURNING community_id, user_id, reason, banned_by, expires_at, created_at",
        )
        .bind(community.id)
        .bind(user_id)
        .bind(&reason)
        .bind(actor.user_id)
        .bind(expires_at)
        .fetch_one(&mut *tx)
        .await?;
        // What a banned user could still get in with goes too.
        sqlx::query("DELETE FROM community_join_requests WHERE community_id = $1 AND user_id = $2")
            .bind(community.id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE community_invites SET revoked_at = now()
             WHERE community_id = $1 AND created_by = $2 AND revoked_at IS NULL",
        )
        .bind(community.id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        let mut entry = LogEntry::new(community.id, ModerationAction::MODERATION_ACTION_MEMBER_BANNED)
            .by(actor.user_id)
            .target(user_id)
            .reason(&reason);
        if let Some(expires) = expires_at {
            entry = entry.detail("expires_at", expires.to_rfc3339());
        }
        modlog::record(&mut *tx, &entry).await?;
        let notice = MembershipNotice::Banned { reason: reason.clone(), expires_at };
        outbound::notify(&mut *tx, &notices::membership(user_id, community, &notice)).await?;
        tx.commit().await?;

        match &departure {
            Some(departure) => self.after_departure(community, user_id, departure).await,
            None => self.push_membership(community, user_id).await,
        }
        let users = tokuly_identity::summaries(&self.core, &[user_id, actor.user_id]).await?;
        Ok(self.ban_proto(&ban, &users))
    }

    /// Lifts a ban. Needs MANAGE_MEMBERS.
    pub(crate) async fn unban_member(&self, access: &Access, user_id: Uuid) -> AppResult<()> {
        access.require(Permission::PERMISSION_MANAGE_MEMBERS)?;
        let actor = access.user_id.ok_or_else(permission_denied)?;
        let community = &access.community;
        let mut tx = self.core.db.begin().await?;
        let lifted = sqlx::query(
            "DELETE FROM community_bans
             WHERE community_id = $1 AND user_id = $2 AND (expires_at IS NULL OR expires_at > now())",
        )
        .bind(community.id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        ensure!(lifted.rows_affected() > 0, not_found, "BAN_NOT_FOUND", "this user is not banned");
        let entry =
            LogEntry::new(community.id, ModerationAction::MODERATION_ACTION_MEMBER_UNBANNED).by(actor).target(user_id);
        modlog::record(&mut *tx, &entry).await?;
        tx.commit().await?;
        self.push_membership(community, user_id).await;
        Ok(())
    }

    /// Bans in force, newest first. Needs MANAGE_MEMBERS.
    pub(crate) async fn list_bans(&self, access: &Access, page: &Page) -> AppResult<(Vec<pb::Ban>, PageInfo)> {
        access.require(Permission::PERMISSION_MANAGE_MEMBERS)?;
        let after: Option<(Time, Uuid)> = page.decode()?;
        let mut rows: Vec<BanRow> = sqlx::query_as(
            "SELECT community_id, user_id, reason, banned_by, expires_at, created_at FROM community_bans
             WHERE community_id = $1 AND (expires_at IS NULL OR expires_at > now())
               AND ($2::timestamptz IS NULL OR (created_at, user_id) < ($2, $3))
             ORDER BY created_at DESC, user_id DESC LIMIT $4",
        )
        .bind(access.community.id)
        .bind(after.map(|(at, _)| at))
        .bind(after.map(|(_, id)| id))
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.created_at, row.user_id));
        let ids: Vec<Uuid> = rows.iter().flat_map(|row| [Some(row.user_id), row.banned_by]).flatten().collect();
        let users = tokuly_identity::summaries(&self.core, &ids).await?;
        Ok((rows.iter().map(|row| self.ban_proto(row, &users)).collect(), info))
    }

    /// Times a member out, or (with `seconds` of `None`) ends their
    /// timeout. Needs MANAGE_MEMBERS and a higher role than the member's.
    pub(crate) async fn set_timeout(
        &self,
        access: &Access,
        user_id: Uuid,
        seconds: Option<i32>,
        reason: &str,
    ) -> AppResult<pb::Member> {
        access.require(Permission::PERMISSION_MANAGE_MEMBERS)?;
        let actor = access.user_id.ok_or_else(permission_denied)?;
        if let Some(seconds) = seconds {
            ensure!(
                (validate::MIN_TIMEOUT_SECONDS..=validate::MAX_TIMEOUT_SECONDS).contains(&seconds),
                invalid,
                "DURATION_INVALID",
                "a timeout lasts between a minute and 28 days"
            );
        }
        let reason = validate::reason(reason, false)?;
        self.require_lower_member(access, user_id).await?;
        if seconds.is_some() {
            self.sanction_limit(actor).await?;
        }
        let community = &access.community;
        let until = seconds.map(|seconds| now() + chrono::Duration::seconds(i64::from(seconds)));

        let mut tx = self.core.db.begin().await?;
        // The role on file decides: nobody times out a peer promoted a moment ago.
        let updated: MemberRow = sqlx::query_as(concat!(
            "UPDATE community_members AS m SET timeout_until = $3, timeout_reason = $4
             WHERE m.community_id = $1 AND m.user_id = $2 AND m.role < $5 RETURNING ",
            member_columns!()
        ))
        .bind(community.id)
        .bind(user_id)
        .bind(until)
        .bind(if until.is_some() { reason.as_str() } else { "" })
        .bind(access.role().map_or(0, crate::permissions::Role::code))
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(permission_denied)?;
        match until {
            Some(until) => {
                let entry = LogEntry::new(community.id, ModerationAction::MODERATION_ACTION_MEMBER_TIMED_OUT)
                    .by(actor)
                    .target(user_id)
                    .reason(&reason)
                    .detail("until", until.to_rfc3339());
                modlog::record(&mut *tx, &entry).await?;
                let notice = MembershipNotice::TimedOut { reason: reason.clone(), until };
                outbound::notify(&mut *tx, &notices::membership(user_id, community, &notice)).await?;
            }
            None => {
                let entry = LogEntry::new(community.id, ModerationAction::MODERATION_ACTION_TIMEOUT_CLEARED)
                    .by(actor)
                    .target(user_id);
                modlog::record(&mut *tx, &entry).await?;
            }
        }
        tx.commit().await?;
        self.push_membership(community, user_id).await;
        let user = tokuly_identity::summary(&self.core, user_id).await?;
        Ok(updated.to_proto(user, true))
    }

    // ───────────────────────────── settings ─────────────────────────────

    pub(crate) fn safety_proto(&self, community: &CommunityRow) -> pb::SafetySettings {
        pb::SafetySettings {
            muted_words: community.muted_words.clone(),
            auto_hide_reports: community.auto_hide_reports,
            effective_auto_hide_reports: self.auto_hide_threshold(community),
            ..Default::default()
        }
    }

    /// Reports by different members that hide a post in this community.
    pub(crate) fn auto_hide_threshold(&self, community: &CommunityRow) -> i32 {
        if community.auto_hide_reports > 0 { community.auto_hide_reports } else { self.cfg.auto_hide_reports as i32 }
    }

    /// Changes muted words and the auto-hide threshold. Needs MANAGE_SAFETY.
    pub(crate) async fn update_safety(
        &self,
        access: &Access,
        muted_words: Option<Vec<String>>,
        auto_hide_reports: Option<i32>,
    ) -> AppResult<pb::SafetySettings> {
        access.require(Permission::PERMISSION_MANAGE_SAFETY)?;
        let actor = access.user_id.ok_or_else(permission_denied)?;
        self.staff_limit(actor).await?;
        let community = &access.community;
        let mut tx = self.core.db.begin().await?;
        let before: Vec<String> = sqlx::query_scalar("SELECT muted_words FROM communities WHERE id = $1 FOR UPDATE")
            .bind(community.id)
            .fetch_one(&mut *tx)
            .await?;
        let (words, threshold): (Vec<String>, i32) = sqlx::query_as(
            "UPDATE communities SET muted_words = coalesce($2, muted_words),
                    auto_hide_reports = coalesce($3, auto_hide_reports)
             WHERE id = $1 RETURNING muted_words, auto_hide_reports",
        )
        .bind(community.id)
        .bind(&muted_words)
        .bind(auto_hide_reports)
        .fetch_one(&mut *tx)
        .await?;
        // Which words, not only how many: a filter that silently starts
        // holding a member's name is something the rest of the staff must
        // be able to see and trace.
        let added: Vec<&str> = words.iter().filter(|word| !before.contains(word)).map(String::as_str).collect();
        let removed: Vec<&str> = before.iter().filter(|word| !words.contains(word)).map(String::as_str).collect();
        let entry = LogEntry::new(community.id, ModerationAction::MODERATION_ACTION_SAFETY_SETTINGS_CHANGED)
            .by(actor)
            .detail("muted_words", words.len())
            .detail("muted_words_added", added.join(", "))
            .detail("muted_words_removed", removed.join(", "))
            .detail("auto_hide_reports", threshold);
        modlog::record(&mut *tx, &entry).await?;
        tx.commit().await?;
        let updated = CommunityRow { muted_words: words, auto_hide_reports: threshold, ..community.clone() };
        Ok(self.safety_proto(&updated))
    }

    /// Sets a channel's slow mode. Needs MANAGE_SAFETY.
    pub(crate) async fn set_slow_mode(
        &self,
        access: &Access,
        channel: &ChannelRow,
        seconds: i32,
    ) -> AppResult<pb::Channel> {
        access.require(Permission::PERMISSION_MANAGE_SAFETY)?;
        let actor = access.user_id.ok_or_else(permission_denied)?;
        ensure!(
            (0..=validate::MAX_SLOW_MODE_SECONDS).contains(&seconds),
            invalid,
            "DURATION_INVALID",
            "slow mode is between 0 (off) and 6 hours"
        );
        self.staff_limit(actor).await?;
        let mut tx = self.core.db.begin().await?;
        let updated: ChannelRow = sqlx::query_as(concat!(
            "UPDATE community_channels AS ch SET slow_mode_seconds = $2 WHERE ch.id = $1 AND ch.deleted_at IS NULL
             RETURNING ",
            channel_columns!()
        ))
        .bind(channel.id)
        .bind(seconds)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(crate::channels::channel_not_found)?;
        let entry = LogEntry::new(channel.community_id, ModerationAction::MODERATION_ACTION_SLOW_MODE_CHANGED)
            .by(actor)
            .channel(channel.id)
            .detail("seconds", seconds);
        modlog::record(&mut *tx, &entry).await?;
        tx.commit().await?;
        self.push_channels_changed(channel.community_id).await;
        Ok(self.channel_proto(access, &updated, false))
    }

    // ───────────────────────────── review ─────────────────────────────

    /// Hides a thread that enough members reported, inside `tx`. False
    /// when it is not in public view or the staff already cleared it.
    pub(crate) async fn hide_thread(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        community: &CommunityRow,
        thread: &ThreadRow,
        reports: i64,
    ) -> AppResult<bool> {
        let hidden = sqlx::query(
            "UPDATE community_threads SET state = 2, hidden_reason = $2, hidden_at = now()
             WHERE id = $1 AND state = 1 AND deleted_at IS NULL AND reviewed_at IS NULL",
        )
        .bind(thread.id)
        .bind(hidden::REPORTS)
        .execute(&mut **tx)
        .await?;
        if hidden.rows_affected() == 0 {
            return Ok(false);
        }
        count_thread(tx, thread, -1, false).await?;
        outbound::unindex(&mut **tx, "thread", thread.id).await?;
        self.record_auto_hidden(tx, community, &Post::Thread(thread.clone()), reports).await?;
        Ok(true)
    }

    /// [`Self::hide_thread`] for a reply.
    pub(crate) async fn hide_reply(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        community: &CommunityRow,
        reply: &ReplyRow,
        reports: i64,
    ) -> AppResult<bool> {
        let hidden = sqlx::query(
            "UPDATE community_replies SET state = 2, hidden_reason = $2, hidden_at = now()
             WHERE id = $1 AND state = 1 AND deleted_at IS NULL AND reviewed_at IS NULL",
        )
        .bind(reply.id)
        .bind(hidden::REPORTS)
        .execute(&mut **tx)
        .await?;
        if hidden.rows_affected() == 0 {
            return Ok(false);
        }
        count_reply(tx, reply.thread_id, -1).await?;
        self.record_auto_hidden(tx, community, &Post::Reply(reply.clone()), reports).await?;
        Ok(true)
    }

    /// The log entry and the notices of a post hidden after reports.
    async fn record_auto_hidden(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        community: &CommunityRow,
        post: &Post,
        reports: i64,
    ) -> AppResult<()> {
        let entry = post
            .log(community.id, ModerationAction::MODERATION_ACTION_POST_HIDDEN)
            .detail("cause", "reports")
            .detail("reports", reports);
        modlog::record(&mut **tx, &entry).await?;
        let (author, _) = post.author();
        let notice =
            notices::moderation(author, community, post.thread_title(), post.path(community), &PostNotice::Held);
        outbound::notify(&mut **tx, &notice).await?;
        let staff: Vec<Uuid> =
            sqlx::query_scalar("SELECT user_id FROM community_members WHERE community_id = $1 AND role >= 2")
                .bind(community.id)
                .fetch_all(&mut **tx)
                .await?;
        for staff_id in staff.into_iter().filter(|id| *id != author) {
            outbound::notify(&mut **tx, &notices::review(staff_id, community)).await?;
        }
        Ok(())
    }

    /// Hidden posts waiting for a decision, oldest first. Needs MODERATE_CONTENT.
    pub(crate) async fn review_queue(
        &self,
        access: &Access,
        page: &Page,
    ) -> AppResult<(Vec<pb::ReviewItem>, PageInfo)> {
        access.require(Permission::PERMISSION_MODERATE_CONTENT)?;
        let community = &access.community;
        let db = &self.core.db;
        let after: Option<(Time, Uuid)> = page.decode()?;
        let mut queue: Vec<(bool, Uuid, Time)> = sqlx::query_as(
            "SELECT is_thread, id, hidden_at FROM (
                 SELECT true AS is_thread, t.id, t.hidden_at FROM community_threads t
                 WHERE t.community_id = $1 AND t.state = 2 AND t.deleted_at IS NULL
                 UNION ALL
                 SELECT false, r.id, r.hidden_at FROM community_replies r
                 JOIN community_threads t ON t.id = r.thread_id
                 WHERE r.community_id = $1 AND r.state = 2 AND r.deleted_at IS NULL AND t.deleted_at IS NULL
             ) AS held
             WHERE hidden_at IS NOT NULL AND ($2::timestamptz IS NULL OR (hidden_at, id) > ($2, $3))
             ORDER BY hidden_at, id LIMIT $4",
        )
        .bind(community.id)
        .bind(after.map(|(at, _)| at))
        .bind(after.map(|(_, id)| id))
        .bind(page.fetch())
        .fetch_all(db)
        .await?;
        let info = page.finish(&mut queue, |(_, id, hidden_at)| (*hidden_at, *id));

        let thread_ids: Vec<Uuid> = queue.iter().filter(|(is_thread, ..)| *is_thread).map(|(_, id, _)| *id).collect();
        let reply_ids: Vec<Uuid> = queue.iter().filter(|(is_thread, ..)| !*is_thread).map(|(_, id, _)| *id).collect();
        let threads: Vec<ThreadRow> =
            sqlx::query_as(concat!("SELECT ", thread_columns!(), " FROM community_threads t WHERE t.id = ANY($1)"))
                .bind(&thread_ids)
                .fetch_all(db)
                .await?;
        let replies: Vec<ReplyRow> =
            sqlx::query_as(concat!(reply_select!(), "WHERE r.id = ANY($1)")).bind(&reply_ids).fetch_all(db).await?;
        let all_ids: Vec<Uuid> = queue.iter().map(|(_, id, _)| *id).collect();
        let reports: Vec<(Uuid, i64)> = sqlx::query_as(
            "SELECT subject_id, count(*) FROM community_reports WHERE subject_id = ANY($1) GROUP BY subject_id",
        )
        .bind(&all_ids)
        .fetch_all(db)
        .await?;
        let reports: HashMap<Uuid, i64> = reports.into_iter().collect();

        // Rendered for the staff member: authors of anonymous posts stay pseudonyms.
        let view = View { access, blocked: &[] };
        let rendered_threads = self.render_threads(community, Some(&view), &threads).await?;
        let rendered_replies = self.render_replies(Some(&view), &replies).await?;
        let mut thread_items: HashMap<Uuid, (pb::Thread, &ThreadRow)> =
            threads.iter().zip(rendered_threads).map(|(row, proto)| (row.id, (proto, row))).collect();
        let mut reply_items: HashMap<Uuid, (pb::Reply, &ReplyRow)> =
            replies.iter().zip(rendered_replies).map(|(row, proto)| (row.id, (proto, row))).collect();
        let items = queue
            .into_iter()
            .filter_map(|(is_thread, id, hidden_at)| {
                let report_count = reports.get(&id).copied().unwrap_or(0) as i32;
                let (post, reason, title) = if is_thread {
                    let (proto, row) = thread_items.remove(&id)?;
                    (pb::review_item::Post::Thread(Box::new(proto)), row.hidden_reason, row.title.clone())
                } else {
                    let (proto, row) = reply_items.remove(&id)?;
                    (pb::review_item::Post::Reply(Box::new(proto)), row.hidden_reason, row.thread_title.clone())
                };
                Some(pb::ReviewItem {
                    post: Some(post),
                    reason: reason.into(),
                    report_count,
                    hidden_at: ts(hidden_at),
                    thread_title: title,
                    ..Default::default()
                })
            })
            .collect();
        Ok((items, info))
    }

    /// Decides about a hidden post: back into public view, or gone.
    /// Needs MODERATE_CONTENT.
    pub(crate) async fn review_post(
        &self,
        access: &Access,
        post: &Post,
        decision: ReviewDecision,
        reason: &str,
    ) -> AppResult<()> {
        access.require(Permission::PERMISSION_MODERATE_CONTENT)?;
        let actor = access.user_id.ok_or_else(permission_denied)?;
        let restore = match decision {
            ReviewDecision::REVIEW_DECISION_RESTORE => true,
            ReviewDecision::REVIEW_DECISION_REMOVE => false,
            _ => bail!(invalid, "DECISION_INVALID", "choose restore or remove"),
        };
        let reason = validate::reason(reason, false)?;
        let community = &access.community;
        let (author, _) = post.author();
        // Nobody judges their own case: a hidden post of a staff member is
        // for another staff member to decide.
        ensure!(
            author != actor,
            denied,
            "CONFLICT_OF_INTEREST",
            "this is your own post; another moderator has to review it"
        );
        ensure!(post.state() == state::HIDDEN, precondition, "POST_NOT_HIDDEN", "this post is not waiting for review");
        self.staff_limit(actor).await?;
        // A post held by the word filter was never public: restoring it is
        // its first appearance. One hidden after reports was announced before.
        let first_appearance = post.hidden_reason() == hidden::MUTED_WORD;
        // Read before the transaction opens: inside it nothing waits for a
        // second connection.
        let reply_thread = match post {
            Post::Reply(reply) if restore && first_appearance => self.thread_by_id(reply.thread_id).await?,
            _ => None,
        };

        let mut tx = self.core.db.begin().await?;
        let done = if restore {
            let restored = match post {
                Post::Thread(thread) => {
                    let restored = sqlx::query(
                        "UPDATE community_threads SET state = 1, hidden_reason = 0, hidden_at = NULL, reviewed_at = now()
                         WHERE id = $1 AND state = 2 AND deleted_at IS NULL",
                    )
                    .bind(thread.id)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected()
                        > 0;
                    if restored {
                        count_thread(&mut tx, thread, 1, first_appearance).await?;
                        let visible = ThreadRow { state: state::VISIBLE, ..thread.clone() };
                        outbound::index(&mut *tx, threads::search_doc(community, &visible)).await?;
                        if first_appearance {
                            let (event_actor, event) = threads::created_event(community, thread);
                            core_events::emit(&mut *tx, events::THREAD_CREATED, event_actor, &event).await?;
                        }
                    }
                    restored
                }
                Post::Reply(reply) => {
                    let restored = sqlx::query(
                        "UPDATE community_replies SET state = 1, hidden_reason = 0, hidden_at = NULL, reviewed_at = now()
                         WHERE id = $1 AND state = 2 AND deleted_at IS NULL",
                    )
                    .bind(reply.id)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected()
                        > 0;
                    if restored {
                        count_reply(&mut tx, reply.thread_id, 1).await?;
                        if let Some(thread) = &reply_thread {
                            let (event_actor, event) = replies::created_event(community, thread, reply);
                            core_events::emit(&mut *tx, events::REPLY_CREATED, event_actor, &event).await?;
                        }
                    }
                    restored
                }
            };
            if restored {
                let subject_type = match post {
                    Post::Thread(_) => SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD,
                    Post::Reply(_) => SubjectType::SUBJECT_TYPE_COMMUNITY_REPLY,
                };
                let subject_id = match post {
                    Post::Thread(thread) => thread.id,
                    Post::Reply(reply) => reply.id,
                };
                // The staff looked: the reports that hid it are answered.
                sqlx::query("DELETE FROM community_reports WHERE subject_type = $1 AND subject_id = $2")
                    .bind(subject_type as i32)
                    .bind(subject_id)
                    .execute(&mut *tx)
                    .await?;
                let entry = post.log(community.id, ModerationAction::MODERATION_ACTION_POST_RESTORED).by(actor);
                modlog::record(&mut *tx, &entry).await?;
                let notice = notices::moderation(
                    author,
                    community,
                    post.thread_title(),
                    post.path(community),
                    &PostNotice::Restored,
                );
                outbound::notify(&mut *tx, &notice).await?;
            }
            restored
        } else {
            let removed = match post {
                Post::Thread(thread) => self.remove_thread(&mut tx, thread, Some(actor)).await?,
                Post::Reply(reply) => self.remove_reply(&mut tx, reply).await?,
            };
            if removed {
                let entry =
                    post.log(community.id, ModerationAction::MODERATION_ACTION_POST_REMOVED).by(actor).reason(&reason);
                modlog::record(&mut *tx, &entry).await?;
                let notice = PostNotice::Removed { reason: reason.clone() };
                outbound::notify(
                    &mut *tx,
                    &notices::moderation(author, community, post.thread_title(), community.path(), &notice),
                )
                .await?;
            }
            removed
        };
        tx.commit().await?;
        ensure!(done, precondition, "POST_NOT_HIDDEN", "this post is not waiting for review");

        if restore {
            match post {
                Post::Thread(thread) => {
                    if let Some(thread) = self.thread_by_id(thread.id).await? {
                        self.push_thread_created(community, &thread).await;
                    }
                }
                Post::Reply(reply) => {
                    if let Some(reply) = self.reply_by_id(reply.id).await? {
                        self.push_reply_created(community, &reply).await;
                    }
                }
            }
        }
        self.push_staff_queues(community.id).await;
        Ok(())
    }

    /// Whether `user_id` is involved in a post: wrote it, is mentioned in
    /// it, is answered by it (the reply it quotes, the thread it replies
    /// in), or reported it.
    async fn is_party(&self, user_id: Uuid, post: &Post) -> AppResult<bool> {
        let (author_id, _) = post.author();
        if author_id == user_id {
            return Ok(true);
        }
        // Within a thread a pseudonym is one person, so the question is
        // not about this post alone: looking up any post of that author
        // unmasks all of them. The actor is a party when they took part in
        // the thread themselves, or when anything that author wrote in it
        // mentions them, answers them, or was reported by them.
        let mentions_actor = serde_json::json!([{ "userId": user_id.to_string() }]);
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM community_threads t WHERE t.id = $1 AND t.author_id = $3)
                 OR EXISTS(SELECT 1 FROM community_replies r WHERE r.thread_id = $1 AND r.author_id = $3)
                 OR EXISTS(SELECT 1 FROM community_threads t
                           WHERE t.id = $1 AND t.author_id = $2
                             AND (t.mentions @> $4
                                  OR EXISTS(SELECT 1 FROM community_reports p
                                            WHERE p.subject_type = $5 AND p.subject_id = t.id AND p.reporter_id = $3)))
                 OR EXISTS(SELECT 1 FROM community_replies r
                           LEFT JOIN community_replies q ON q.id = r.reply_to_id
                           WHERE r.thread_id = $1 AND r.author_id = $2
                             AND (r.mentions @> $4 OR q.author_id = $3
                                  OR EXISTS(SELECT 1 FROM community_reports p
                                            WHERE p.subject_type = $6 AND p.subject_id = r.id AND p.reporter_id = $3)))",
        )
        .bind(post.thread_id())
        .bind(author_id)
        .bind(user_id)
        .bind(mentions_actor)
        .bind(SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD as i32)
        .bind(SubjectType::SUBJECT_TYPE_COMMUNITY_REPLY as i32)
        .fetch_one(&self.core.db)
        .await?)
    }

    /// Names the author of an anonymous post to a staff member (or to site
    /// moderation), leaving a trace: a log entry and a word to the rest of
    /// the staff, neither of which names the author.
    pub(crate) async fn reveal_author(
        &self,
        access: &Access,
        post: &Post,
        reason: &str,
    ) -> AppResult<(UserSummary, MemberRole)> {
        let as_staff = access.has(Permission::PERMISSION_REVEAL_AUTHOR);
        if !as_staff && !access.site_moderator {
            return Err(permission_denied());
        }
        let actor = access.user_id.ok_or_else(permission_denied)?;
        let (author, anonymous) = post.author();
        ensure!(anonymous, precondition, "POST_NOT_ANONYMOUS", "this post already names its author");
        let reason = validate::reason(reason, true)?;
        // Nobody unmasks someone they have a quarrel with: whoever is
        // involved in the post leaves the lookup to another staff member,
        // or to site moderation.
        ensure!(
            !self.is_party(actor, post).await?,
            denied,
            "CONFLICT_OF_INTEREST",
            "you are involved in this post; another moderator has to look"
        );
        self.limit("reveal", actor, REVEALS_PER_HOUR, Duration::from_secs(3600)).await?;
        let community = &access.community;
        let actor_summary = tokuly_identity::summary(&self.core, actor).await?;
        let others: Vec<Uuid> =
            self.staff_ids(community.id).await?.into_iter().filter(|staff_id| *staff_id != actor).collect();

        let mut tx = self.core.db.begin().await?;
        let entry = post
            .log(community.id, ModerationAction::MODERATION_ACTION_AUTHOR_REVEALED)
            .by(actor)
            .site(!as_staff)
            .reason(&reason);
        modlog::record(&mut *tx, &entry).await?;
        for staff_id in &others {
            let notice =
                notices::author_revealed(*staff_id, &actor_summary, actor, community, post.thread_title(), &reason);
            outbound::notify(&mut *tx, &notice).await?;
        }
        tx.commit().await?;

        let event = pb::AuthorRevealed {
            community_id: community.id.to_string(),
            actor: actor_summary.into(),
            thread_id: post.thread_id().to_string(),
            reply_id: match post {
                Post::Reply(reply) => reply.id.to_string(),
                Post::Thread(_) => String::new(),
            },
            reason,
            ..Default::default()
        };
        self.push_users(others, event).await;

        let summary = tokuly_identity::summary(&self.core, author).await?;
        let role = self.member(community.id, author).await?.map(|member| member.role().to_proto());
        Ok((summary, role.unwrap_or(MemberRole::MEMBER_ROLE_UNSPECIFIED)))
    }
}
