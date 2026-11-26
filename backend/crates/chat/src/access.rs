//! Who may see and do what: every RPC starts by resolving the caller's
//! membership here.
//!
//! A conversation the caller is not a member of does not exist as far as
//! they can tell (`CONVERSATION_NOT_FOUND`), and neither do its messages
//! (`MESSAGE_NOT_FOUND`) — including messages from before the caller joined
//! or cleared their history.
use sqlx::{Postgres, Transaction};
use tokuly_core::prelude::*;
use tokuly_proto::common::UserSummary;
use tokuly_social::graph;

use crate::{
    Chat,
    model::{CONV, ConversationRow, MEMBER, MSG, MemberRow, MessageRow, request},
};

pub(crate) fn conversation_not_found() -> AppError {
    AppError::not_found("CONVERSATION_NOT_FOUND", "conversation not found")
}

pub(crate) fn message_not_found() -> AppError {
    AppError::not_found("MESSAGE_NOT_FOUND", "message not found")
}

pub(crate) fn read_only() -> AppError {
    AppError::precondition("CONVERSATION_READ_ONLY", "nobody can send in this conversation any more")
}

pub(crate) fn request_pending() -> AppError {
    AppError::precondition("REQUEST_PENDING", "the message request has not been accepted yet")
}

/// A conversation together with the caller's membership in it.
#[derive(Debug, Clone)]
pub(crate) struct Access {
    pub conv: ConversationRow,
    pub me: MemberRow,
}

impl Access {
    /// Direct conversations: the user on the other side.
    pub fn other_user(&self) -> Option<Uuid> {
        self.conv.other_of(self.me.user_id)
    }

    /// Whether the caller may see the message at `seq`.
    pub fn sees(&self, seq: i64) -> bool {
        seq > self.me.history_from_seq
    }
}

/// What a message is made of, as far as the request rules care: while the
/// other side has not accepted a message request only plain content
/// (text, stamps, a hello) may be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Content {
    Plain,
    Rich,
}

/// The outcome of [`Chat::ensure_can_send`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SendPermit {
    /// The message goes to someone who has not accepted the conversation:
    /// it counts against the sender's request allowance.
    pub is_request: bool,
}

pub(crate) async fn find_conversation(core: &Core, conversation_id: Uuid) -> AppResult<Option<ConversationRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {CONV} FROM chat_conversations c WHERE c.id = $1")))
        .bind(conversation_id)
        .fetch_optional(&core.db)
        .await?)
}

/// Locks a conversation for the rest of the transaction. Everything that
/// adds a message, changes the member list or starts a call takes this lock
/// first, which is what orders those changes.
pub(crate) async fn lock_conversation(
    tx: &mut Transaction<'_, Postgres>,
    conversation_id: Uuid,
) -> AppResult<ConversationRow> {
    lock_conversation_if_present(tx, conversation_id).await?.ok_or_else(conversation_not_found)
}

/// [`lock_conversation`] for work that has nothing left to do once the
/// conversation was deleted meanwhile (event consumers).
pub(crate) async fn lock_conversation_if_present(
    tx: &mut Transaction<'_, Postgres>,
    conversation_id: Uuid,
) -> AppResult<Option<ConversationRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {CONV} FROM chat_conversations c WHERE c.id = $1 FOR UPDATE"
    )))
    .bind(conversation_id)
    .fetch_optional(&mut **tx)
    .await?)
}

pub(crate) async fn find_member(core: &Core, conversation_id: Uuid, user_id: Uuid) -> AppResult<Option<MemberRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {MEMBER} FROM chat_members m WHERE m.conversation_id = $1 AND m.user_id = $2"
    )))
    .bind(conversation_id)
    .bind(user_id)
    .fetch_optional(&core.db)
    .await?)
}

/// The member row inside a transaction (after the conversation was locked).
pub(crate) async fn member_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    conversation_id: Uuid,
    user_id: Uuid,
) -> AppResult<Option<MemberRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {MEMBER} FROM chat_members m WHERE m.conversation_id = $1 AND m.user_id = $2"
    )))
    .bind(conversation_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?)
}

/// Who a user is, as far as the wording of a notification needs it, read
/// through the open transaction (which must not wait for a second
/// connection). Unknown and deleted users come back as a placeholder.
pub(crate) async fn summary_in_tx(tx: &mut Transaction<'_, Postgres>, user_id: Uuid) -> AppResult<UserSummary> {
    let row: Option<(String, String, bool)> =
        sqlx::query_as("SELECT handle::text, display_name, deleted_at IS NOT NULL FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&mut **tx)
            .await?;
    Ok(match row {
        Some((handle, display_name, false)) => {
            UserSummary { id: user_id.to_string(), handle, display_name, ..Default::default() }
        }
        _ => UserSummary { id: user_id.to_string(), handle: "deleted".into(), deleted: true, ..Default::default() },
    })
}

pub(crate) async fn find_message(core: &Core, message_id: Uuid) -> AppResult<Option<MessageRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {MSG} FROM chat_messages x WHERE x.id = $1")))
        .bind(message_id)
        .fetch_optional(&core.db)
        .await?)
}

/// IDs of the members who should hear about what happens in a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Audience {
    /// Everyone except members who declined the conversation.
    Reachable,
    /// Members who accepted the conversation (typing, presence, read receipts).
    Accepted,
}

pub(crate) async fn member_ids(core: &Core, conversation_id: Uuid, audience: Audience) -> AppResult<Vec<Uuid>> {
    let states: &[i16] = match audience {
        Audience::Reachable => &[request::ACCEPTED, request::PENDING],
        Audience::Accepted => &[request::ACCEPTED],
    };
    Ok(sqlx::query_scalar("SELECT user_id FROM chat_members WHERE conversation_id = $1 AND request_state = ANY($2)")
        .bind(conversation_id)
        .bind(states)
        .fetch_all(&core.db)
        .await?)
}

impl Chat {
    /// The conversation as a member sees it, or `CONVERSATION_NOT_FOUND`.
    pub(crate) async fn access(&self, user_id: Uuid, conversation_id: Uuid) -> AppResult<Access> {
        let me = find_member(&self.core, conversation_id, user_id).await?.ok_or_else(conversation_not_found)?;
        let conv = find_conversation(&self.core, conversation_id).await?.ok_or_else(conversation_not_found)?;
        Ok(Access { conv, me })
    }

    /// A message the caller may see, with its conversation.
    pub(crate) async fn message_access(&self, user_id: Uuid, message_id: Uuid) -> AppResult<(Access, MessageRow)> {
        let message = find_message(&self.core, message_id).await?.ok_or_else(message_not_found)?;
        let access = self.access(user_id, message.conversation_id).await.map_err(|_| message_not_found())?;
        if !access.sees(message.seq) {
            return Err(message_not_found());
        }
        Ok((access, message))
    }

    /// Fails with `CONVERSATION_READ_ONLY` when nobody can send in the
    /// conversation: a direct conversation whose other side is gone or in a
    /// block with the caller. Blocks are checked live, so a block takes
    /// effect before its event has been processed.
    pub(crate) async fn ensure_writable(&self, access: &Access) -> AppResult<()> {
        if !access.conv.is_direct() {
            return Ok(());
        }
        if access.conv.read_only {
            return Err(read_only());
        }
        let other = access.other_user().ok_or_else(read_only)?;
        if graph::is_blocked_either(&self.core, access.me.user_id, other).await? {
            return Err(read_only());
        }
        Ok(())
    }

    /// Checks that the caller may send `content` into the conversation.
    ///
    /// In a direct conversation the other side may still have to accept the
    /// caller's message request: until then only plain content goes through,
    /// and only `request_message_limit` messages of it (`REQUEST_PENDING`
    /// otherwise — also when the request was declined, which the sender must
    /// not be able to tell apart).
    pub(crate) async fn ensure_can_send(&self, access: &Access, content: Content) -> AppResult<SendPermit> {
        self.ensure_writable(access).await?;
        if !access.conv.is_direct() {
            return Ok(SendPermit::default());
        }
        let other = access.other_user().ok_or_else(read_only)?;
        let Some(other_state) = self.request_state_of(access.conv.id, other).await? else {
            return Err(read_only());
        };
        if other_state == request::ACCEPTED {
            return Ok(SendPermit::default());
        }
        // Friends need no request, whatever was stored before they became friends.
        if other_state == request::PENDING && graph::are_friends(&self.core, access.me.user_id, other).await? {
            self.accept_request_of(access.conv.id, other).await?;
            return Ok(SendPermit::default());
        }
        if content != Content::Plain || access.me.request_messages >= self.cfg.request_message_limit {
            return Err(request_pending());
        }
        Ok(SendPermit { is_request: true })
    }

    /// The caller must have accepted the conversation to do things that tell
    /// the other side they looked at it (reactions, pins, calls, POV).
    pub(crate) fn ensure_accepted(&self, access: &Access) -> AppResult<()> {
        if access.me.is_accepted() { Ok(()) } else { Err(request_pending()) }
    }

    pub(crate) async fn request_state_of(&self, conversation_id: Uuid, user_id: Uuid) -> AppResult<Option<i16>> {
        Ok(sqlx::query_scalar("SELECT request_state FROM chat_members WHERE conversation_id = $1 AND user_id = $2")
            .bind(conversation_id)
            .bind(user_id)
            .fetch_optional(&self.core.db)
            .await?)
    }
}
