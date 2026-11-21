//! Typing indicators and co-presence ("who has the conversation open").
//!
//! The realtime gateway forwards what clients say without checking
//! anything (see `tokuly_core::presence`). This module checks membership and
//! fans the signals out as `ChatEvent`s.
//!
//! Co-presence lives in Valkey, never in PostgreSQL:
//!
//! - `chat:present:<conversation>` — a sorted set of user IDs scored with
//!   the time their presence runs out. Clients repeat the signal every 30
//!   seconds; an entry not refreshed for [`PRESENCE_TTL_SECONDS`] no longer
//!   counts, so a crashed client disappears by itself.
//! - `chat:present:due:<namespace>` — every entry again, by expiry, so a
//!   sweeper can tell the other members when someone silently went away.
//!
//! Presence serves two purposes: the "is here" indicator (only accepted
//! members who share their online status are shown) and not notifying
//! people about messages they are watching arrive (everyone counts).
use std::{sync::Arc, time::Duration};

use chrono::Utc;
use futures::StreamExt as _;
use tokio::sync::Semaphore;
use tokuly_core::{
    prelude::*,
    presence::{CONVERSATION_PRESENCE_SUBJECT, ConversationPresenceSignal, TYPING_SUBJECT, TypingSignal},
};
use tokuly_identity::settings;
use tokuly_proto::chat as pb;
use tokuly_social::graph;

use crate::{
    Chat,
    access::{self, Audience},
    model::request,
};

/// Seconds a presence signal lasts without being repeated.
pub const PRESENCE_TTL_SECONDS: i64 = 75;
/// Typing signals one user may send per minute; more are dropped.
const TYPING_SIGNALS_PER_MINUTE: u32 = 120;
/// Presence signals one user may send per minute; more are dropped.
const PRESENCE_SIGNALS_PER_MINUTE: u32 = 120;
/// Queue groups: worker processes share the signals, each is handled once.
const TYPING_QUEUE: &str = "chat-typing";
const PRESENCE_QUEUE: &str = "chat-presence";
/// Signals handled concurrently by one process.
const CONCURRENCY: usize = 32;

/// Atomically takes the entries whose presence ran out, so two sweepers
/// never announce the same departure and a refresh racing with the sweep is
/// not lost.
const POP_DUE: &str = r"
local due = redis.call('ZRANGEBYSCORE', KEYS[1], '-inf', ARGV[1], 'LIMIT', 0, ARGV[2])
if #due > 0 then redis.call('ZREM', KEYS[1], unpack(due)) end
return due
";

fn presence_key(conversation_id: Uuid) -> String {
    format!("chat:present:{}", conversation_id.simple())
}

fn due_member(conversation_id: Uuid, user_id: Uuid) -> String {
    format!("{}:{}", conversation_id.simple(), user_id.simple())
}

fn parse_due_member(member: &str) -> Option<(Uuid, Uuid)> {
    let (conversation_id, user_id) = member.split_once(':')?;
    Some((Uuid::parse_str(conversation_id).ok()?, Uuid::parse_str(user_id).ok()?))
}

impl Chat {
    /// Deployments sharing one Valkey (and test runs) keep separate expiry lists.
    fn due_key(&self) -> String {
        format!("chat:present:due:{}", self.core.cfg.nats_prefix)
    }

    /// Everyone who has the conversation open right now, shown or not.
    pub(crate) async fn present_users(&self, conversation_id: Uuid) -> AppResult<Vec<Uuid>> {
        let members: Vec<String> = redis::cmd("ZRANGEBYSCORE")
            .arg(presence_key(conversation_id))
            .arg(format!("({}", Utc::now().timestamp()))
            .arg("+inf")
            .query_async(&mut self.core.cache.conn())
            .await?;
        Ok(members.iter().filter_map(|member| Uuid::parse_str(member).ok()).collect())
    }

    /// The members shown as present: those who accepted the conversation
    /// and share their online status.
    pub(crate) async fn visible_present_users(&self, conversation_id: Uuid) -> AppResult<Vec<Uuid>> {
        let present = self.present_users(conversation_id).await?;
        if present.is_empty() {
            return Ok(Vec::new());
        }
        let mut accepted: Vec<Uuid> = sqlx::query_scalar(
            "SELECT user_id FROM chat_members WHERE conversation_id = $1 AND request_state = $2 AND user_id = ANY($3)",
        )
        .bind(conversation_id)
        .bind(request::ACCEPTED)
        .bind(&present)
        .fetch_all(&self.core.db)
        .await?;
        let privacy = settings::privacy_of_many(&self.core, &accepted).await?;
        accepted.retain(|user_id| privacy.get(user_id).is_none_or(|privacy| privacy.share_online_status));
        accepted.sort_unstable();
        Ok(accepted)
    }

    async fn push_presence(&self, conversation_id: Uuid) -> AppResult<()> {
        let present = self.visible_present_users(conversation_id).await?;
        let event = pb::PresenceChanged {
            conversation_id: conversation_id.to_string(),
            present_user_ids: present.iter().map(Uuid::to_string).collect(),
            ..Default::default()
        };
        self.push_members(conversation_id, Audience::Accepted, event).await;
        Ok(())
    }

    /// Whether `user_id`'s presence in a conversation is shown to the others.
    async fn presence_is_shown(&self, accepted: bool, user_id: Uuid) -> AppResult<bool> {
        Ok(accepted && settings::privacy_of(&self.core, user_id).await?.share_online_status)
    }

    /// Handles "opened / left the conversation screen" from a client.
    /// Signals about conversations the user is not a member of are ignored.
    pub async fn handle_conversation_presence(&self, signal: ConversationPresenceSignal) -> AppResult<()> {
        let core = &self.core;
        let ConversationPresenceSignal { user_id, conversation_id, present } = signal;
        if core
            .cache
            .rate_limit(&format!("chat-presence:{user_id}"), PRESENCE_SIGNALS_PER_MINUTE, Duration::from_secs(60))
            .await
            .is_err()
        {
            return Ok(());
        }
        let Some(member) = access::find_member(core, conversation_id, user_id).await? else { return Ok(()) };
        if member.request_state == request::DECLINED {
            return Ok(());
        }
        let key = presence_key(conversation_id);
        let now = Utc::now().timestamp();
        let mut conn = core.cache.conn();
        let previous: Option<i64> =
            redis::cmd("ZSCORE").arg(&key).arg(user_id.to_string()).query_async(&mut conn).await?;
        let was_present = previous.is_some_and(|expires| expires > now);
        if present {
            let expires = now + PRESENCE_TTL_SECONDS;
            redis::pipe()
                .cmd("ZADD")
                .arg(&key)
                .arg(expires)
                .arg(user_id.to_string())
                .ignore()
                .cmd("EXPIRE")
                .arg(&key)
                .arg(PRESENCE_TTL_SECONDS * 2)
                .ignore()
                .cmd("ZADD")
                .arg(self.due_key())
                .arg(expires)
                .arg(due_member(conversation_id, user_id))
                .ignore()
                .query_async::<()>(&mut conn)
                .await?;
        } else {
            redis::pipe()
                .cmd("ZREM")
                .arg(&key)
                .arg(user_id.to_string())
                .ignore()
                .cmd("ZREM")
                .arg(self.due_key())
                .arg(due_member(conversation_id, user_id))
                .ignore()
                .query_async::<()>(&mut conn)
                .await?;
        }
        if was_present != present && self.presence_is_shown(member.is_accepted(), user_id).await? {
            self.push_presence(conversation_id).await?;
        }
        Ok(())
    }

    /// Drops a user's presence in a conversation they are no longer part of.
    pub(crate) async fn forget_presence(&self, conversation_id: Uuid, user_id: Uuid) {
        let forgotten: AppResult<()> = async {
            let mut conn = self.core.cache.conn();
            let (removed,): (i64,) = redis::pipe()
                .cmd("ZREM")
                .arg(presence_key(conversation_id))
                .arg(user_id.to_string())
                .cmd("ZREM")
                .arg(self.due_key())
                .arg(due_member(conversation_id, user_id))
                .ignore()
                .query_async(&mut conn)
                .await?;
            if removed > 0 {
                self.push_presence(conversation_id).await?;
            }
            Ok(())
        }
        .await;
        if let Err(e) = forgotten {
            tracing::warn!(%conversation_id, %user_id, error = %e, "co-presence cleanup failed");
        }
    }

    /// Announces members whose presence ran out without a goodbye (closed
    /// laptop, lost connection). Returns how many conversations changed.
    pub async fn sweep_presence(&self) -> AppResult<usize> {
        let core = &self.core;
        let now = Utc::now().timestamp();
        let mut conn = core.cache.conn();
        let due: Vec<String> =
            redis::Script::new(POP_DUE).key(self.due_key()).arg(now).arg(500).invoke_async(&mut conn).await?;
        let mut conversations: Vec<Uuid> = due
            .iter()
            .filter_map(|member| parse_due_member(member))
            .map(|(conversation_id, _)| conversation_id)
            .collect();
        conversations.sort_unstable();
        conversations.dedup();
        for conversation_id in &conversations {
            // Only what really ran out: a signal repeated since has a later score.
            redis::cmd("ZREMRANGEBYSCORE")
                .arg(presence_key(*conversation_id))
                .arg("-inf")
                .arg(now)
                .query_async::<()>(&mut conn)
                .await?;
            self.push_presence(*conversation_id).await?;
        }
        Ok(conversations.len())
    }

    /// Handles "started / stopped typing" from a client: accepted members
    /// of a conversation people can still write in tell the other accepted
    /// members.
    pub async fn handle_typing(&self, signal: TypingSignal) -> AppResult<()> {
        let core = &self.core;
        let TypingSignal { user_id, conversation_id, typing } = signal;
        if core
            .cache
            .rate_limit(&format!("chat-typing:{user_id}"), TYPING_SIGNALS_PER_MINUTE, Duration::from_secs(60))
            .await
            .is_err()
        {
            return Ok(());
        }
        let Ok(access) = self.access(user_id, conversation_id).await else { return Ok(()) };
        if !access.me.is_accepted() || access.conv.read_only {
            return Ok(());
        }
        let mut recipients = access::member_ids(core, conversation_id, Audience::Accepted).await?;
        // People in a block with the typist hear nothing from them.
        let hidden = graph::blocked_ids_either(core, user_id).await?;
        recipients.retain(|recipient| *recipient != user_id && !hidden.contains(recipient));
        let event = pb::TypingChanged {
            conversation_id: conversation_id.to_string(),
            user_id: user_id.to_string(),
            typing,
            ..Default::default()
        };
        self.push_to(recipients, event).await;
        Ok(())
    }

    /// Consumes typing signals until shutdown.
    pub(crate) async fn run_typing_fanout(&self) {
        self.run_signal_consumer(TYPING_SUBJECT, TYPING_QUEUE, |chat, signal: TypingSignal| async move {
            chat.handle_typing(signal).await
        })
        .await;
    }

    /// Consumes co-presence signals until shutdown.
    pub(crate) async fn run_presence_fanout(&self) {
        self.run_signal_consumer(
            CONVERSATION_PRESENCE_SUBJECT,
            PRESENCE_QUEUE,
            |chat, signal: ConversationPresenceSignal| async move { chat.handle_conversation_presence(signal).await },
        )
        .await;
    }

    /// Subscribes to a gateway subject (as a queue group) and runs `handler`
    /// for every signal, resubscribing if the subscription ends.
    async fn run_signal_consumer<T, F, Fut>(&self, subject: &'static str, queue: &'static str, handler: F)
    where
        T: serde::de::DeserializeOwned + Send + 'static,
        F: Fn(Chat, T) -> Fut + Send + Sync + Copy + 'static,
        Fut: Future<Output = AppResult<()>> + Send + 'static,
    {
        let shutdown = self.core.shutdown.clone();
        let limiter = Arc::new(Semaphore::new(CONCURRENCY));
        while !shutdown.is_cancelled() {
            let mut signals = match self.core.bus.queue_subscribe(subject, queue).await {
                Ok(subscription) => subscription,
                Err(e) => {
                    tracing::error!(subject, error = %e, "chat signal subscription failed; retrying");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };
            tracing::info!(subject, "chat signal fan-out started");
            loop {
                let message = tokio::select! {
                    _ = shutdown.cancelled() => return,
                    message = signals.next() => message,
                };
                let Some(message) = message else { break };
                let Ok(signal) = serde_json::from_slice::<T>(&message.payload) else {
                    tracing::warn!(subject, "undecodable chat signal dropped");
                    continue;
                };
                let permit = limiter.clone().acquire_owned().await.expect("semaphore is never closed");
                let chat = self.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(e) = handler(chat, signal).await {
                        tracing::warn!(subject, error = %e, "chat signal fan-out failed");
                    }
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_members_roundtrip() {
        let (conversation_id, user_id) = (Uuid::now_v7(), Uuid::now_v7());
        assert_eq!(parse_due_member(&due_member(conversation_id, user_id)), Some((conversation_id, user_id)));
        assert_eq!(parse_due_member("garbage"), None);
        assert_eq!(parse_due_member("a:b"), None);
    }
}
