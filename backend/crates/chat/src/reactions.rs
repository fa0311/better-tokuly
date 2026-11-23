//! Emoji reactions and pinned messages.
use std::time::Duration;

use tokuly_core::{outbound, prelude::*};
use tokuly_identity::users;
use tokuly_proto::{chat as pb, common::PageInfo};
use tokuly_social::graph;

use crate::{
    Chat,
    access::{self, Access, Audience},
    content,
    model::{MSG, MessageRow, SystemPayload, msg},
    notices,
    send::Draft,
};

/// Different emoji one message can collect.
const MAX_REACTION_KINDS: i64 = 20;
/// Messages a conversation can have pinned.
pub(crate) const MAX_PINNED_MESSAGES: i64 = 50;
/// How long the author is not told again that the same person reacted to
/// the same message.
const REACTION_NOTICE_WINDOW: Duration = Duration::from_secs(24 * 3600);

fn check_emoji(emoji: &str) -> AppResult<()> {
    ensure!(content::is_single_emoji(emoji), invalid, "EMOJI_INVALID", "a reaction is exactly one emoji");
    Ok(())
}

impl Chat {
    /// The reactions of a message as `viewer` sees them.
    pub(crate) async fn reaction_counts(&self, message_id: Uuid, viewer: Uuid) -> AppResult<Vec<pb::ReactionCount>> {
        let rows: Vec<(String, i64, bool)> = sqlx::query_as(
            "SELECT emoji, count(*), COALESCE(bool_or(user_id = $2), false)
             FROM chat_reactions WHERE message_id = $1 GROUP BY emoji ORDER BY min(created_at), emoji",
        )
        .bind(message_id)
        .bind(viewer)
        .fetch_all(&self.core.db)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(emoji, count, reacted_by_me)| pb::ReactionCount {
                emoji,
                count: count as i32,
                reacted_by_me,
                ..Default::default()
            })
            .collect())
    }

    async fn push_reaction(&self, row: &MessageRow, user_id: Uuid, emoji: &str, added: bool) -> AppResult<()> {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM chat_reactions WHERE message_id = $1 AND emoji = $2")
            .bind(row.id)
            .bind(emoji)
            .fetch_one(&self.core.db)
            .await?;
        let event = pb::ReactionChanged {
            conversation_id: row.conversation_id.to_string(),
            message_id: row.id.to_string(),
            emoji: emoji.to_owned(),
            user_id: user_id.to_string(),
            added,
            count: count as i32,
            ..Default::default()
        };
        self.push_members(row.conversation_id, Audience::Reachable, event).await;
        Ok(())
    }

    pub(crate) async fn add_reaction(
        &self,
        user_id: Uuid,
        message_id: Uuid,
        emoji: &str,
    ) -> AppResult<Vec<pb::ReactionCount>> {
        let core = &self.core;
        check_emoji(emoji)?;
        let (access, row) = self.message_access(user_id, message_id).await?;
        ensure!(
            !row.is_deleted() && row.kind != msg::SYSTEM,
            precondition,
            "MESSAGE_NOT_REACTABLE",
            "this message cannot be reacted to"
        );
        self.ensure_accepted(&access)?;
        self.ensure_writable(&access).await?;
        core.cache.rate_limit(&format!("chat-react:{user_id}"), 120, Duration::from_secs(60)).await?;

        // The limit on different emoji is checked and the reaction stored in
        // one statement. (Reactions arriving at the same instant may overshoot
        // the limit by one or two; it only exists to keep messages tidy.)
        let outcome: Option<(bool, bool)> = sqlx::query_as(
            "WITH target AS (
                 SELECT id FROM chat_messages WHERE id = $1 AND deleted_at IS NULL FOR UPDATE
             ), kinds AS (
                 SELECT count(DISTINCT emoji) AS total, COALESCE(bool_or(emoji = $3), false) AS known
                 FROM chat_reactions WHERE message_id = $1
             ), stored AS (
                 INSERT INTO chat_reactions (message_id, user_id, emoji)
                 SELECT target.id, $2, $3 FROM target, kinds WHERE kinds.known OR kinds.total < $4
                 ON CONFLICT (message_id, user_id, emoji) DO NOTHING
                 RETURNING 1
             )
             SELECT (kinds.known OR kinds.total < $4) AS allowed, EXISTS (SELECT 1 FROM stored) AS added FROM target, kinds",
        )
        .bind(message_id)
        .bind(user_id)
        .bind(emoji)
        .bind(MAX_REACTION_KINDS)
        .fetch_optional(&core.db)
        .await?;
        let (allowed, added) = outcome.ok_or_else(access::message_not_found)?;
        ensure!(allowed, precondition, "TOO_MANY_REACTIONS", "this message has too many different reactions");
        if added {
            self.push_reaction(&row, user_id, emoji, true).await?;
            if let Err(e) = self.notify_reaction(&access, &row, user_id, emoji).await {
                tracing::warn!(%message_id, error = %e, "reaction notification failed");
            }
        }
        self.reaction_counts(message_id, user_id).await
    }

    /// Tells the author of a message that someone reacted to it, unless they
    /// are looking at the conversation, muted it, are in a block with the
    /// reactor, or were already told about this person's reaction to it.
    async fn notify_reaction(&self, access: &Access, row: &MessageRow, reactor: Uuid, emoji: &str) -> AppResult<()> {
        let core = &self.core;
        let Some(author) = row.sender_id.filter(|author| *author != reactor) else { return Ok(()) };
        let Some(member) = access::find_member(core, row.conversation_id, author).await? else { return Ok(()) };
        if !member.is_accepted() || member.muted_now() {
            return Ok(());
        }
        if self.present_users(row.conversation_id).await.unwrap_or_default().contains(&author) {
            return Ok(());
        }
        if graph::is_blocked_either(core, author, reactor).await? {
            return Ok(());
        }
        // Once per person and message: a reaction switched off and on again
        // (or a second emoji) is no news, and every push would count against
        // the author's per-minute push budget, crowding out everyone else.
        let first = format!("chat:reaction-notified:{}:{}", row.id.simple(), reactor.simple());
        if !core.cache.set_nx(&first, "1", REACTION_NOTICE_WINDOW).await? {
            return Ok(());
        }
        let summary = tokuly_identity::summary(core, reactor).await?;
        outbound::notify(&core.db, &notices::reaction(author, &summary, reactor, &access.conv, row, emoji)).await
    }

    pub(crate) async fn remove_reaction(
        &self,
        user_id: Uuid,
        message_id: Uuid,
        emoji: &str,
    ) -> AppResult<Vec<pb::ReactionCount>> {
        check_emoji(emoji)?;
        let (_, row) = self.message_access(user_id, message_id).await?;
        let removed = sqlx::query("DELETE FROM chat_reactions WHERE message_id = $1 AND user_id = $2 AND emoji = $3")
            .bind(message_id)
            .bind(user_id)
            .bind(emoji)
            .execute(&self.core.db)
            .await?
            .rows_affected()
            > 0;
        if removed {
            self.push_reaction(&row, user_id, emoji, false).await?;
        }
        self.reaction_counts(message_id, user_id).await
    }

    /// Who reacted to a message, newest first; `emoji` narrows it to one.
    pub(crate) async fn list_reactions(
        &self,
        user_id: Uuid,
        message_id: Uuid,
        emoji: &str,
        page: &Page,
    ) -> AppResult<(Vec<pb::Reaction>, PageInfo)> {
        self.message_access(user_id, message_id).await?;
        let cursor: Option<(Time, Uuid, String)> = page.decode()?;
        let (before_at, before_user, before_emoji) = match cursor {
            Some((at, user, emoji)) => (Some(at), Some(user), Some(emoji)),
            None => (None, None, None),
        };
        let mut rows: Vec<(Uuid, String, Time)> = sqlx::query_as(
            "SELECT user_id, emoji, created_at FROM chat_reactions
             WHERE message_id = $1 AND ($2 = '' OR emoji = $2)
               AND ($3::timestamptz IS NULL OR (created_at, user_id, emoji) < ($3, $4, $5))
             ORDER BY created_at DESC, user_id DESC, emoji DESC
             LIMIT $6",
        )
        .bind(message_id)
        .bind(emoji)
        .bind(before_at)
        .bind(before_user)
        .bind(before_emoji)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |(user, emoji, at)| (*at, *user, emoji.clone()));
        let summaries =
            tokuly_identity::summaries(&self.core, &rows.iter().map(|(user, _, _)| *user).collect::<Vec<_>>()).await?;
        let reactions = rows
            .into_iter()
            .map(|(user, emoji, at)| pb::Reaction {
                user: users::pick(&summaries, user).into(),
                emoji,
                created_at: ts(at),
                ..Default::default()
            })
            .collect();
        Ok((reactions, info))
    }

    // ───────────────────────────── pins ─────────────────────────────

    pub(crate) async fn pin_message(&self, user_id: Uuid, message_id: Uuid) -> AppResult<(Access, MessageRow)> {
        let (access, row) = self.message_access(user_id, message_id).await?;
        let pinnable = !row.is_deleted()
            && matches!(row.kind, msg::TEXT | msg::STAMP | msg::MEDIA | msg::LOCATION | msg::HELLO)
            && !row.is_pov_reply();
        ensure!(pinnable, precondition, "MESSAGE_NOT_PINNABLE", "this message cannot be pinned");
        self.ensure_accepted(&access)?;
        self.ensure_writable(&access).await?;
        // Every pin adds a line to the conversation.
        self.core.cache.rate_limit(&format!("chat-pin:{user_id}"), 30, Duration::from_secs(60)).await?;

        let mut tx = self.core.db.begin().await?;
        let mut conv = access::lock_conversation(&mut tx, access.conv.id).await?;
        let already: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM chat_pins WHERE message_id = $1)")
            .bind(message_id)
            .fetch_one(&mut *tx)
            .await?;
        if already {
            return Ok((access, row));
        }
        let pinned: i64 = sqlx::query_scalar("SELECT count(*) FROM chat_pins WHERE conversation_id = $1")
            .bind(conv.id)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(
            pinned < MAX_PINNED_MESSAGES,
            precondition,
            "TOO_MANY_PINNED",
            "at most {MAX_PINNED_MESSAGES} messages can be pinned"
        );
        // The message may have been deleted since it was loaded.
        let stored = sqlx::query(
            "INSERT INTO chat_pins (message_id, conversation_id, pinned_by)
             SELECT id, conversation_id, $2 FROM chat_messages WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(message_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        ensure!(stored.rows_affected() > 0, precondition, "MESSAGE_NOT_PINNABLE", "this message cannot be pinned");
        let payload = SystemPayload::new(pb::SystemEventType::SYSTEM_EVENT_TYPE_MESSAGE_PINNED, Some(user_id))
            .message(message_id);
        let posted = self.post(&mut tx, &mut conv, Draft::system(payload)).await?;
        tx.commit().await?;
        self.push_message_updated(&row, pb::MessageUpdateReason::MESSAGE_UPDATE_REASON_PINNED).await;
        self.announce(&posted).await;
        Ok((access, row))
    }

    pub(crate) async fn unpin_message(&self, user_id: Uuid, message_id: Uuid) -> AppResult<(Access, MessageRow)> {
        let (access, row) = self.message_access(user_id, message_id).await?;
        self.ensure_accepted(&access)?;
        let unpinned = sqlx::query("DELETE FROM chat_pins WHERE message_id = $1")
            .bind(message_id)
            .execute(&self.core.db)
            .await?
            .rows_affected()
            > 0;
        if unpinned {
            self.push_message_updated(&row, pb::MessageUpdateReason::MESSAGE_UPDATE_REASON_UNPINNED).await;
        }
        Ok((access, row))
    }

    /// Pinned messages of a conversation, most recently pinned first.
    pub(crate) async fn pinned_messages(&self, access: &Access, page: &Page) -> AppResult<(Vec<MessageRow>, PageInfo)> {
        #[derive(sqlx::FromRow)]
        struct Pinned {
            pinned_at: Time,
            #[sqlx(flatten)]
            row: MessageRow,
        }
        let cursor: Option<(Time, Uuid)> = page.decode()?;
        let (before_at, before_id) = match cursor {
            Some((at, id)) => (Some(at), Some(id)),
            None => (None, None),
        };
        let mut rows: Vec<Pinned> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT p.created_at AS pinned_at, {MSG}
             FROM chat_pins p JOIN chat_messages x ON x.id = p.message_id
             WHERE p.conversation_id = $1 AND x.seq > $2
               AND ($3::timestamptz IS NULL OR (p.created_at, p.message_id) < ($3, $4))
             ORDER BY p.created_at DESC, p.message_id DESC
             LIMIT $5"
        )))
        .bind(access.conv.id)
        .bind(access.me.history_from_seq)
        .bind(before_at)
        .bind(before_id)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |pinned| (pinned.pinned_at, pinned.row.id));
        Ok((rows.into_iter().map(|pinned| pinned.row).collect(), info))
    }
}
