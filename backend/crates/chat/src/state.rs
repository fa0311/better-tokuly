//! Each member's private state of a conversation: preferences (pin, mute,
//! archive), hiding, message requests, the read cursor and unread totals.
use tokuly_core::prelude::*;
use tokuly_identity::users;
use tokuly_proto::chat as pb;

use crate::{
    Chat,
    access::{Access, Audience, message_not_found},
    conversations::MAX_PINNED_CONVERSATIONS,
    model::{MEMBER, MemberRow, request},
};

/// What `UpdatePreferences` may change; `None` leaves a setting alone.
#[derive(Debug, Default)]
pub(crate) struct PreferenceChange {
    pub pinned: Option<bool>,
    pub muted: Option<bool>,
    /// With `muted = Some(true)`: when the mute ends by itself.
    pub muted_until: Option<Time>,
    pub archived: Option<bool>,
}

#[derive(sqlx::FromRow)]
struct UnreadRow {
    badge: i64,
    unread_conversations: i64,
    pending_requests: i64,
    archived_unread: i64,
}

#[derive(sqlx::FromRow)]
struct ReadRow {
    read_seq: i64,
    read_at: Option<Time>,
    unread_count: i32,
    previous_read_seq: i64,
}

impl Chat {
    /// Unread totals of one user (the app badge).
    pub(crate) async fn unread_counts(&self, user_id: Uuid) -> AppResult<pb::UnreadCounts> {
        let row: UnreadRow = sqlx::query_as(
            "SELECT
                 COALESCE(sum(unread_count) FILTER (
                     WHERE request_state = 1 AND archived_at IS NULL AND NOT (muted AND (muted_until IS NULL OR muted_until > now()))), 0)::bigint AS badge,
                 count(*) FILTER (WHERE request_state = 1 AND archived_at IS NULL AND unread_count > 0) AS unread_conversations,
                 count(*) FILTER (WHERE request_state = 2 AND hidden_at IS NULL) AS pending_requests,
                 COALESCE(sum(unread_count) FILTER (WHERE request_state = 1 AND archived_at IS NOT NULL), 0)::bigint AS archived_unread
             FROM chat_members WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_one(&self.core.db)
        .await?;
        let clamp = |value: i64| value.clamp(0, i64::from(i32::MAX)) as i32;
        Ok(pb::UnreadCounts {
            badge: clamp(row.badge),
            unread_conversations: clamp(row.unread_conversations),
            pending_requests: clamp(row.pending_requests),
            archived_unread: clamp(row.archived_unread),
            ..Default::default()
        })
    }

    /// Applies preference changes of the caller to one conversation.
    pub(crate) async fn update_preferences(
        &self,
        user_id: Uuid,
        conversation_id: Uuid,
        change: PreferenceChange,
    ) -> AppResult<Access> {
        let access = self.access(user_id, conversation_id).await?;
        let me = &access.me;
        let mut pinned_at = me.pinned_at;
        let mut muted = me.muted;
        let mut muted_until = me.muted_until;
        let mut archived_at = me.archived_at;
        match change.muted {
            Some(true) => {
                if let Some(until) = change.muted_until {
                    ensure!(until > now(), invalid, "MUTE_UNTIL_INVALID", "the mute must end in the future");
                }
                muted = true;
                muted_until = change.muted_until;
            }
            Some(false) => {
                muted = false;
                muted_until = None;
            }
            None => {}
        }
        match change.archived {
            Some(true) => {
                archived_at = archived_at.or_else(|| Some(now()));
                // An archived conversation is out of the list it could be pinned in.
                pinned_at = None;
            }
            Some(false) => archived_at = None,
            None => {}
        }
        match change.pinned {
            Some(true) if pinned_at.is_none() => {
                let pinned: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM chat_members WHERE user_id = $1 AND pinned_at IS NOT NULL",
                )
                .bind(user_id)
                .fetch_one(&self.core.db)
                .await?;
                ensure!(
                    pinned < MAX_PINNED_CONVERSATIONS,
                    precondition,
                    "TOO_MANY_PINNED",
                    "at most {MAX_PINNED_CONVERSATIONS} conversations can be pinned"
                );
                pinned_at = Some(now());
                archived_at = None;
            }
            Some(false) => pinned_at = None,
            _ => {}
        }
        sqlx::query("UPDATE chat_members SET pinned_at = $3, muted = $4, muted_until = $5, archived_at = $6 WHERE conversation_id = $1 AND user_id = $2")
            .bind(conversation_id)
            .bind(user_id)
            .bind(pinned_at)
            .bind(muted)
            .bind(muted_until)
            .bind(archived_at)
            .execute(&self.core.db)
            .await?;
        // The caller's other devices follow.
        self.push_conversation_to(conversation_id, user_id).await;
        self.access(user_id, conversation_id).await
    }

    /// Takes a conversation out of the caller's list until the next message
    /// arrives; with `clear_history` everything so far is gone for them.
    pub(crate) async fn hide_conversation(
        &self,
        user_id: Uuid,
        conversation_id: Uuid,
        clear_history: bool,
    ) -> AppResult<()> {
        let access = self.access(user_id, conversation_id).await?;
        // The read cursor stays where it is: hiding must not tell the
        // others that the caller read anything.
        sqlx::query(
            "UPDATE chat_members SET hidden_at = now(), unread_count = 0, unread_mention_count = 0, pinned_at = NULL,
                    history_from_seq = CASE WHEN $3 THEN GREATEST(history_from_seq, $4) ELSE history_from_seq END
             WHERE conversation_id = $1 AND user_id = $2",
        )
        .bind(conversation_id)
        .bind(user_id)
        .bind(clear_history)
        .bind(access.conv.last_seq)
        .execute(&self.core.db)
        .await?;
        self.push_conversation_removed(
            conversation_id,
            user_id,
            pb::ConversationRemovalReason::CONVERSATION_REMOVAL_REASON_HIDDEN,
        )
        .await;
        Ok(())
    }

    /// Answers a message request. Returns the conversation when accepted.
    pub(crate) async fn respond_to_request(
        &self,
        user_id: Uuid,
        conversation_id: Uuid,
        accept: bool,
    ) -> AppResult<Option<Access>> {
        let access = self.access(user_id, conversation_id).await?;
        let no_request = || AppError::precondition("NO_PENDING_REQUEST", "there is no message request to answer");
        if access.me.request_state != request::PENDING {
            return Err(no_request());
        }
        if accept {
            if !self.accept_request_of(conversation_id, user_id).await? {
                return Err(no_request());
            }
            // The sender's allowance is lifted: their view changes too.
            if let Some(other) = access.other_user() {
                self.push_conversation_to(conversation_id, other).await;
            }
            return Ok(Some(self.access(user_id, conversation_id).await?));
        }
        // Declining is silent: nothing reaches the sender, now or later.
        let declined = sqlx::query(
            "UPDATE chat_members SET request_state = $3, hidden_at = now(), unread_count = 0, unread_mention_count = 0
             WHERE conversation_id = $1 AND user_id = $2 AND request_state = $4",
        )
        .bind(conversation_id)
        .bind(user_id)
        .bind(request::DECLINED)
        .bind(request::PENDING)
        .execute(&self.core.db)
        .await?;
        if declined.rows_affected() == 0 {
            return Err(no_request());
        }
        self.forget_presence(conversation_id, user_id).await;
        self.push_conversation_removed(
            conversation_id,
            user_id,
            pb::ConversationRemovalReason::CONVERSATION_REMOVAL_REASON_DECLINED,
        )
        .await;
        Ok(None)
    }

    /// Turns a waiting message request of `user_id` into an ordinary
    /// conversation. Returns whether there was one; the user's devices are
    /// told when so.
    pub(crate) async fn accept_request_of(&self, conversation_id: Uuid, user_id: Uuid) -> AppResult<bool> {
        let accepted = sqlx::query("UPDATE chat_members SET request_state = $3 WHERE conversation_id = $1 AND user_id = $2 AND request_state = $4")
            .bind(conversation_id)
            .bind(user_id)
            .bind(request::ACCEPTED)
            .bind(request::PENDING)
            .execute(&self.core.db)
            .await?
            .rows_affected()
            > 0;
        if accepted {
            self.push_conversation_to(conversation_id, user_id).await;
        }
        Ok(accepted)
    }

    /// Moves the caller's read cursor forward to `message_id` (or to the
    /// latest message) and recounts what is still unread.
    pub(crate) async fn mark_read(
        &self,
        user_id: Uuid,
        conversation_id: Uuid,
        message_id: Option<Uuid>,
    ) -> AppResult<pb::MarkReadResponse> {
        let access = self.access(user_id, conversation_id).await?;
        let target = match message_id {
            Some(message_id) => {
                let seq: Option<i64> =
                    sqlx::query_scalar("SELECT seq FROM chat_messages WHERE id = $1 AND conversation_id = $2")
                        .bind(message_id)
                        .bind(conversation_id)
                        .fetch_optional(&self.core.db)
                        .await?;
                seq.filter(|seq| access.sees(*seq)).ok_or_else(message_not_found)?
            }
            None => access.conv.last_seq,
        };
        // Counting from the messages themselves (rather than subtracting)
        // also repairs a counter that drifted.
        let row: ReadRow = sqlx::query_as(
            "WITH old AS (
                 SELECT read_seq, GREATEST(read_seq, history_from_seq, $3) AS horizon
                 FROM chat_members WHERE conversation_id = $1 AND user_id = $2 FOR UPDATE
             ), unread AS (
                 SELECT count(*) AS total,
                        count(*) FILTER (WHERE x.mention_all OR x.mentions @> jsonb_build_array(jsonb_build_object('user_id', $2::text))) AS mentions
                 FROM chat_messages x, old
                 WHERE x.conversation_id = $1 AND x.seq > old.horizon AND x.sender_id IS NOT NULL AND x.sender_id <> $2
                   AND x.kind <> 6 AND x.deleted_at IS NULL
             )
             UPDATE chat_members m SET
                 read_seq = GREATEST(m.read_seq, $3),
                 read_at = CASE WHEN $3 > m.read_seq THEN now() ELSE m.read_at END,
                 unread_count = unread.total, unread_mention_count = unread.mentions
             FROM old, unread
             WHERE m.conversation_id = $1 AND m.user_id = $2
             RETURNING m.read_seq, m.read_at, m.unread_count, old.read_seq AS previous_read_seq",
        )
        .bind(conversation_id)
        .bind(user_id)
        .bind(target)
        .fetch_one(&self.core.db)
        .await?;

        if row.read_seq > row.previous_read_seq {
            let event = pb::ReadReceiptChanged {
                conversation_id: conversation_id.to_string(),
                user_id: user_id.to_string(),
                read_seq: row.read_seq,
                read_at: opt_ts(row.read_at),
                ..Default::default()
            };
            if access.me.is_accepted() {
                self.push_members(conversation_id, Audience::Accepted, event).await;
            } else {
                // Looking at a message request must not tell its sender.
                self.push_to([user_id], event).await;
            }
        }
        Ok(pb::MarkReadResponse {
            read_seq: row.read_seq,
            unread_count: row.unread_count,
            unread: self.unread_counts(user_id).await?.into(),
            ..Default::default()
        })
    }

    /// Who has read a message: every accepted member other than its sender
    /// whose cursor is at or past it, most recent reader first, and how many
    /// have not.
    pub(crate) async fn read_receipts(
        &self,
        conversation_id: Uuid,
        seq: i64,
        sender_id: Option<Uuid>,
    ) -> AppResult<(Vec<pb::ReadReceipt>, i32)> {
        let members: Vec<MemberRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {MEMBER} FROM chat_members m
             WHERE m.conversation_id = $1 AND m.request_state = $2 AND m.user_id IS DISTINCT FROM $3
             ORDER BY m.read_at DESC NULLS LAST, m.user_id"
        )))
        .bind(conversation_id)
        .bind(request::ACCEPTED)
        .bind(sender_id)
        .fetch_all(&self.core.db)
        .await?;
        let (readers, others): (Vec<&MemberRow>, Vec<&MemberRow>) =
            members.iter().partition(|member| member.read_seq >= seq);
        let summaries =
            tokuly_identity::summaries(&self.core, &readers.iter().map(|member| member.user_id).collect::<Vec<_>>())
                .await?;
        let receipts = readers
            .into_iter()
            .map(|member| pb::ReadReceipt {
                user: users::pick(&summaries, member.user_id).into(),
                read_at: opt_ts(member.read_at),
                ..Default::default()
            })
            .collect();
        Ok((receipts, others.len() as i32))
    }
}
