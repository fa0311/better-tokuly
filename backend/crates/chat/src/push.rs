//! Realtime pushes: every change is sent as a `ChatEvent` to the user
//! subjects of the members concerned (all their devices, the actor's other
//! devices included). Delivery is best-effort; the database is the truth.
use tokuly_core::prelude::*;
use tokuly_proto::{
    chat::{self as pb, ChatEvent, chat_event::Event as Push},
    realtime::server_frame::Body,
};

use crate::{
    Chat,
    access::{self, Audience},
    model::{MessageRow, request},
    render::View,
};

pub(crate) fn frame(event: Push) -> Body {
    Body::from(ChatEvent { event: Some(event), ..Default::default() })
}

impl Chat {
    /// Pushes one event to a set of users.
    pub(crate) async fn push_to(&self, user_ids: impl IntoIterator<Item = Uuid>, event: impl Into<Push>) {
        self.core.rt.notify_users(user_ids, frame(event.into())).await;
    }

    /// Pushes one event to the members of a conversation.
    pub(crate) async fn push_members(&self, conversation_id: Uuid, audience: Audience, event: impl Into<Push>) {
        match access::member_ids(&self.core, conversation_id, audience).await {
            Ok(user_ids) => self.push_to(user_ids, event).await,
            Err(e) => tracing::warn!(%conversation_id, error = %e, "chat push skipped: members could not be loaded"),
        }
    }

    /// Sends a message (new or changed) to the members who may see it.
    ///
    /// The payload is the same for everyone with one exception: the quote
    /// above a reply is withheld from members who cannot see the answered
    /// message (they joined later or cleared their history), so those get
    /// their own variant.
    pub(crate) async fn fan_out_message(&self, row: &MessageRow, wrap: impl Fn(pb::Message) -> Push) -> AppResult<()> {
        let members: Vec<(Uuid, i64)> = sqlx::query_as(
            "SELECT user_id, history_from_seq FROM chat_members
             WHERE conversation_id = $1 AND request_state <> $2 AND history_from_seq < $3",
        )
        .bind(row.conversation_id)
        .bind(request::DECLINED)
        .bind(row.seq)
        .fetch_all(&self.core.db)
        .await?;
        let answered_seq: Option<i64> = match row.reply_to_id.filter(|_| !row.is_deleted()) {
            Some(answered) => {
                sqlx::query_scalar("SELECT seq FROM chat_messages WHERE id = $1")
                    .bind(answered)
                    .fetch_optional(&self.core.db)
                    .await?
            }
            None => None,
        };
        let (blind, sighted): (Vec<_>, Vec<_>) =
            members.into_iter().partition(|(_, floor)| answered_seq.is_some_and(|seq| *floor >= seq));
        if !sighted.is_empty() {
            let message = self.render_message(row, &View::event()).await?;
            self.push_to(sighted.into_iter().map(|(user_id, _)| user_id), wrap(message)).await;
        }
        if !blind.is_empty() {
            let message = self.render_message(row, &View::event().with_floor(row.conversation_id, i64::MAX)).await?;
            self.push_to(blind.into_iter().map(|(user_id, _)| user_id), wrap(message)).await;
        }
        Ok(())
    }

    /// Announces a new message.
    pub(crate) async fn push_message_created(&self, row: &MessageRow) {
        let pushed = self
            .fan_out_message(row, |message| pb::MessageCreated { message: message.into(), ..Default::default() }.into())
            .await;
        if let Err(e) = pushed {
            tracing::warn!(message_id = %row.id, error = %e, "chat message push failed");
        }
    }

    /// Announces a changed message.
    pub(crate) async fn push_message_updated(&self, row: &MessageRow, reason: pb::MessageUpdateReason) {
        let pushed = self
            .fan_out_message(row, |message| {
                pb::MessageUpdated { message: message.into(), reason: reason.into(), ..Default::default() }.into()
            })
            .await;
        if let Err(e) = pushed {
            tracing::warn!(message_id = %row.id, error = %e, "chat message update push failed");
        }
    }

    /// Reloads a message and announces it as changed. Does nothing when the
    /// message is gone.
    pub(crate) async fn push_message_updated_by_id(&self, message_id: Uuid, reason: pb::MessageUpdateReason) {
        match access::find_message(&self.core, message_id).await {
            Ok(Some(row)) => self.push_message_updated(&row, reason).await,
            Ok(None) => {}
            Err(e) => tracing::warn!(%message_id, error = %e, "chat message update push failed"),
        }
    }

    /// Tells every member that the shared part of a conversation changed
    /// (name, icon, members, read-only state, live call).
    pub(crate) async fn push_conversation_shared(&self, conversation_id: Uuid) {
        let pushed: AppResult<()> = async {
            let Some(conv) = access::find_conversation(&self.core, conversation_id).await? else { return Ok(()) };
            let conversation = self.shared_conversation(&conv).await?;
            let event = pb::ConversationUpdated { conversation: conversation.into(), ..Default::default() };
            self.push_members(conversation_id, Audience::Reachable, event).await;
            Ok(())
        }
        .await;
        if let Err(e) = pushed {
            tracing::warn!(%conversation_id, error = %e, "chat conversation push failed");
        }
    }

    /// Tells one member that their own view of a conversation changed
    /// (joined, role, request state, preferences set on another device).
    pub(crate) async fn push_conversation_to(&self, conversation_id: Uuid, user_id: Uuid) {
        let pushed: AppResult<()> = async {
            let Ok(access) = self.access(user_id, conversation_id).await else { return Ok(()) };
            let conversation = self.conversation_for(&access).await?;
            self.push_to(
                [user_id],
                pb::ConversationUpdated { conversation: conversation.into(), ..Default::default() },
            )
            .await;
            Ok(())
        }
        .await;
        if let Err(e) = pushed {
            tracing::warn!(%conversation_id, %user_id, error = %e, "chat conversation push failed");
        }
    }

    /// Tells a user that a conversation left their list.
    pub(crate) async fn push_conversation_removed(
        &self,
        conversation_id: Uuid,
        user_id: Uuid,
        reason: pb::ConversationRemovalReason,
    ) {
        let event = pb::ConversationRemoved {
            conversation_id: conversation_id.to_string(),
            reason: reason.into(),
            ..Default::default()
        };
        self.push_to([user_id], event).await;
    }
}
