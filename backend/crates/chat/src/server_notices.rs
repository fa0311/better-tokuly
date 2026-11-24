//! Notices other domains post into conversations: a line from the server
//! between the bubbles ("this bot has a new owner"), not anybody's message.
//!
//! A notice is a system message (`SYSTEM_EVENT_TYPE_NOTICE`) with no sender:
//! it moves no unread counter, brings no hidden conversation back, requests
//! no notification and cannot be reported (there is nobody behind it). It is
//! pushed to the members' devices like any other message. Each one is posted
//! once per `key`, so a caller that repeats itself (an event delivered twice)
//! does not repeat the notice, and a conversation takes at most
//! [`NOTICES_PER_CONVERSATION_PER_DAY`]: whatever another domain does over
//! and over (a bot handed back and forth) cannot fill a conversation.
use std::time::Duration;

use tokuly_core::{outbound::Localized, prelude::*};
use tokuly_proto::chat as pb;

use crate::{
    Chat,
    model::{SystemPayload, kind},
    send::Draft,
};

/// Notices one conversation takes per day; more are dropped.
pub const NOTICES_PER_CONVERSATION_PER_DAY: u32 = 5;

/// Longest notice key.
const MAX_KEY_CHARS: usize = 100;

/// Longest notice text, per language.
const MAX_NOTICE_CHARS: usize = 300;

/// The direct conversations `user_id` is a member of, oldest first.
pub async fn direct_conversations_of(core: &Core, user_id: Uuid) -> AppResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT c.id FROM chat_members m JOIN chat_conversations c ON c.id = m.conversation_id
         WHERE m.user_id = $1 AND c.kind = $2 ORDER BY c.id",
    )
    .bind(user_id)
    .bind(kind::DIRECT)
    .fetch_all(&core.db)
    .await?)
}

impl Chat {
    /// Posts `notice` into a conversation as a notice from the server (see
    /// the module documentation). `key` names the notice for its caller
    /// (`dev.bot.transferred:<event id>`): a second call with the same key
    /// posts nothing. Returns whether the notice was posted: `false` when
    /// it was there already or the conversation has had its share today.
    /// Fails with `CONVERSATION_NOT_FOUND` when the conversation does not
    /// exist.
    pub async fn post_notice(&self, conversation_id: Uuid, key: &str, notice: Localized) -> AppResult<bool> {
        ensure!(
            !key.is_empty() && key.chars().count() <= MAX_KEY_CHARS,
            invalid,
            "NOTICE_KEY_INVALID",
            "a notice needs a key of at most {MAX_KEY_CHARS} characters"
        );
        let too_long = |text: &str| text.chars().count() > MAX_NOTICE_CHARS;
        ensure!(
            !notice.ja.trim().is_empty() && !too_long(&notice.ja) && !too_long(&notice.en),
            invalid,
            "NOTICE_INVALID",
            "a notice needs a Japanese text of at most {MAX_NOTICE_CHARS} characters"
        );
        // Asked before the rate limit, so a repeated call does not use up
        // the conversation's share.
        if self.has_notice(conversation_id, key).await? {
            return Ok(false);
        }
        let limit = self
            .core
            .cache
            .rate_limit(
                &format!("chat-notice:{conversation_id}"),
                NOTICES_PER_CONVERSATION_PER_DAY,
                Duration::from_secs(86_400),
            )
            .await;
        match limit {
            Ok(()) => {}
            Err(e) if e.reason == "RATE_LIMITED" => {
                tracing::info!(%conversation_id, key, "a conversation had its share of notices today; one was dropped");
                return Ok(false);
            }
            Err(e) => return Err(e),
        }
        let payload = SystemPayload::new(pb::SystemEventType::SYSTEM_EVENT_TYPE_NOTICE, None).notice(notice);
        let draft = Draft { client_id: Some(key.to_owned()), ..Draft::system(payload) };
        Ok(!self.post_and_announce(conversation_id, draft).await?.duplicate)
    }

    async fn has_notice(&self, conversation_id: Uuid, key: &str) -> AppResult<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM chat_messages WHERE conversation_id = $1 AND sender_id IS NULL AND client_id = $2)",
        )
        .bind(conversation_id)
        .bind(key)
        .fetch_one(&self.core.db)
        .await?)
    }
}
