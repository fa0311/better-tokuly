//! The message pipeline: how a message of any kind enters a conversation.
//!
//! Everything that adds a message — user messages, forwards, system
//! messages, call and POV entries — builds a [`Draft`] and goes through
//! [`Chat::post`] inside a transaction that holds the conversation lock:
//!
//! 1. a retried send (same `client_id`) returns the stored message;
//! 2. the message gets the next sequence number and is stored, with its
//!    uploads indexed and held;
//! 3. the other members' unread counters move, the sender's read cursor
//!    jumps to their own message;
//! 4. notifications are requested (respecting mutes, blocks, message
//!    requests and who has the conversation open right now).
//!
//! Nothing in there waits for a second database connection: what the
//! notification rules need to know from elsewhere is gathered before the
//! transaction starts ([`Chat::with_notifications`]).
//!
//! After the commit, [`Chat::announce`] pushes the message to the members'
//! devices and reports it to the other domains (best-friend ranking, stamp
//! statistics, the link preview job, analytics).
use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use tokuly_core::{analytics::AnalyticsEvent, outbound, prelude::*};
use tokuly_media::assets::{self, AssetRow};
use tokuly_proto::{chat as pb, common::UserSummary};
use tokuly_social::{InteractionKind, graph};
use tokuly_stamp::SendContext;

use crate::{
    Chat,
    access::{self, Audience},
    model::{ConversationRow, MSG, MentionRef, MessageRow, SystemPayload, msg, muted_now, request},
    notices, render,
};

/// Name of the job that fetches the preview of a message's first link.
pub const JOB_LINK_PREVIEW: &str = "chat.link_preview";

/// Payload of [`JOB_LINK_PREVIEW`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkPreviewJob {
    pub message_id: Uuid,
}

/// Members a group message is reported for in the best-friend ranking: the
/// people it addresses directly (the answered sender, the mentioned).
const MAX_GROUP_INTERACTIONS: usize = 10;

/// The asset holder of a message's uploads (see `tokuly_media::assets`).
pub(crate) fn message_holder(message_id: Uuid) -> String {
    format!("chat:message:{message_id}")
}

/// What the notification rules need to know about the sender and the
/// moment a message is sent.
#[derive(Debug, Clone)]
pub(crate) struct NotifyContext {
    sender: UserSummary,
    /// Members looking at the conversation right now: they see the message arrive.
    present: HashSet<Uuid>,
    /// In a group: people in a block with the sender, who hear nothing from them.
    hidden: HashSet<Uuid>,
}

/// Whether a message asks for the ordinary message notifications.
#[derive(Debug, Clone)]
pub(crate) enum Notify {
    /// `chat.message` / `chat.mention` / `chat.request`, as the rules allow.
    Standard(Box<NotifyContext>),
    /// Nothing: system messages, and flows that notify in their own words
    /// (calls, POV requests).
    Silent,
}

/// A message about to be stored.
#[derive(Debug, Clone)]
pub(crate) struct Draft {
    pub kind: i16,
    /// `None` for system messages.
    pub sender_id: Option<Uuid>,
    pub client_id: Option<String>,
    pub text: String,
    pub mentions: Vec<MentionRef>,
    pub mention_all: bool,
    pub stamp_id: Option<Uuid>,
    pub attachments: Vec<AssetRow>,
    pub location: Option<pb::LocationShare>,
    pub link_url: Option<String>,
    /// An already known preview of `link_url` (forwards); otherwise a job fetches it.
    pub link_preview: Option<serde_json::Value>,
    /// The answered message and its sender.
    pub reply_to: Option<(Uuid, Option<Uuid>)>,
    pub forwarded: bool,
    pub system: Option<SystemPayload>,
    pub call_id: Option<Uuid>,
    pub pov_request_id: Option<Uuid>,
    pub notify: Notify,
    /// The message goes to someone who has not accepted the conversation yet.
    pub is_request: bool,
}

impl Draft {
    /// A message of `kind` from a user, without content yet. It notifies
    /// nobody unless [`Chat::with_notifications`] says so.
    pub fn from_user(kind: i16, sender_id: Uuid) -> Self {
        Self {
            kind,
            sender_id: Some(sender_id),
            client_id: None,
            text: String::new(),
            mentions: Vec::new(),
            mention_all: false,
            stamp_id: None,
            attachments: Vec::new(),
            location: None,
            link_url: None,
            link_preview: None,
            reply_to: None,
            forwarded: false,
            system: None,
            call_id: None,
            pov_request_id: None,
            notify: Notify::Silent,
            is_request: false,
        }
    }

    /// A system message: something that happened to the conversation.
    pub fn system(payload: SystemPayload) -> Self {
        Self { sender_id: None, system: Some(payload), ..Self::from_user(msg::SYSTEM, Uuid::nil()) }
    }
}

/// The result of [`Chat::post`].
#[derive(Debug, Clone)]
pub(crate) struct Posted {
    pub row: MessageRow,
    /// The message had been stored before under the same `client_id`;
    /// nothing was written this time.
    pub duplicate: bool,
    conv: ConversationRow,
    /// The sender's read cursor jumped over messages they had not read.
    read_skipped: bool,
    /// Sending accepted a message request the sender had not answered.
    accepted_now: bool,
    reply_sender: Option<Uuid>,
}

#[derive(sqlx::FromRow)]
struct Recipient {
    user_id: Uuid,
    request_state: i16,
    muted: bool,
    muted_until: Option<Time>,
}

#[derive(sqlx::FromRow)]
struct SenderBefore {
    read_seq: i64,
    request_state: i16,
    request_messages: i32,
}

impl Chat {
    /// Gives a user's draft the ordinary message notifications. Call it
    /// before the transaction that posts the draft begins: it looks up the
    /// sender, who has the conversation open and (for groups) who is in a
    /// block with the sender.
    pub(crate) async fn with_notifications(&self, conv: &ConversationRow, mut draft: Draft) -> AppResult<Draft> {
        let Some(sender_id) = draft.sender_id else { return Ok(draft) };
        let core = &self.core;
        let sender = tokuly_identity::summary(core, sender_id).await?;
        let present = self.present_users(conv.id).await.unwrap_or_default().into_iter().collect();
        let hidden = if conv.is_group() {
            graph::blocked_ids_either(core, sender_id).await?.into_iter().collect()
        } else {
            HashSet::new()
        };
        draft.notify = Notify::Standard(Box::new(NotifyContext { sender, present, hidden }));
        Ok(draft)
    }

    /// Stores a message in `conv`, which the caller locked in `tx` (see
    /// [`access::lock_conversation`]); `conv` is advanced to the new message,
    /// so several messages can be posted in a row. Call [`Chat::announce`]
    /// with the result once the transaction has committed.
    pub(crate) async fn post(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        conv: &mut ConversationRow,
        draft: Draft,
    ) -> AppResult<Posted> {
        if let Some(client_id) = &draft.client_id {
            // System messages with a `client_id` (server notices) are unique
            // the same way; the conversation lock serializes the check.
            let existing: Option<MessageRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {MSG} FROM chat_messages x
                 WHERE x.conversation_id = $1 AND x.sender_id IS NOT DISTINCT FROM $2 AND x.client_id = $3"
            )))
            .bind(conv.id)
            .bind(draft.sender_id)
            .bind(client_id)
            .fetch_optional(&mut **tx)
            .await?;
            if let Some(row) = existing {
                return Ok(Posted {
                    row,
                    duplicate: true,
                    conv: conv.clone(),
                    read_skipped: false,
                    accepted_now: false,
                    reply_sender: None,
                });
            }
        }

        let message_id = new_id();
        let seq = conv.last_seq + 1;
        let json = |value: Result<serde_json::Value, serde_json::Error>| {
            value.map_err(|e| AppError::internal(anyhow::anyhow!("message encode: {e}")))
        };
        let attachments =
            json(serde_json::to_value(draft.attachments.iter().map(AssetRow::to_attachment).collect::<Vec<_>>()))?;
        let mentions = json(serde_json::to_value(&draft.mentions))?;
        let location = draft.location.as_ref().map(|location| json(serde_json::to_value(location))).transpose()?;
        let system = draft.system.as_ref().map(|system| json(serde_json::to_value(system))).transpose()?;
        let row: MessageRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO chat_messages AS x
                 (id, conversation_id, seq, sender_id, kind, client_id, text, mentions, mention_all, stamp_id, attachments,
                  location, link_url, link_preview, link_checked_at, reply_to_id, forwarded, system, call_id, pov_request_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, CASE WHEN $14::jsonb IS NULL THEN NULL ELSE now() END,
                     $15, $16, $17, $18, $19)
             RETURNING {MSG}"
        )))
        .bind(message_id)
        .bind(conv.id)
        .bind(seq)
        .bind(draft.sender_id)
        .bind(draft.kind)
        .bind(&draft.client_id)
        .bind(&draft.text)
        .bind(mentions)
        .bind(draft.mention_all)
        .bind(draft.stamp_id)
        .bind(attachments)
        .bind(location)
        .bind(&draft.link_url)
        .bind(&draft.link_preview)
        .bind(draft.reply_to.map(|(id, _)| id))
        .bind(draft.forwarded)
        .bind(system)
        .bind(draft.call_id)
        .bind(draft.pov_request_id)
        .fetch_one(&mut **tx)
        .await?;

        let holder = message_holder(message_id);
        for (position, asset) in draft.attachments.iter().enumerate() {
            sqlx::query(
                "INSERT INTO chat_message_assets (message_id, asset_id, conversation_id, seq, position, kind) VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (message_id, asset_id) DO NOTHING",
            )
            .bind(message_id)
            .bind(asset.id)
            .bind(conv.id)
            .bind(seq)
            .bind(position as i16)
            .bind(asset.kind as i16)
            .execute(&mut **tx)
            .await?;
            assets::attach(&mut **tx, asset.id, &holder).await?;
        }

        sqlx::query(
            "UPDATE chat_conversations SET last_seq = $2, last_message_id = $3, last_activity_at = now() WHERE id = $1",
        )
        .bind(conv.id)
        .bind(seq)
        .bind(message_id)
        .execute(&mut **tx)
        .await?;

        // The other members: unread counters move, a hidden conversation comes back.
        let counts = i32::from(row.counts_as_unread());
        let mentioned: Vec<Uuid> = draft.mentions.iter().map(|mention| mention.user_id).collect();
        let recipients: Vec<Recipient> = sqlx::query_as(
            "UPDATE chat_members m SET
                 unread_count = m.unread_count + $3,
                 unread_mention_count = m.unread_mention_count + CASE WHEN $4 OR m.user_id = ANY($5) THEN $3 ELSE 0 END,
                 hidden_at = CASE WHEN $3 = 1 THEN NULL ELSE m.hidden_at END
             WHERE m.conversation_id = $1 AND m.user_id IS DISTINCT FROM $2 AND m.request_state <> $6
             RETURNING m.user_id, m.request_state, m.muted, m.muted_until",
        )
        .bind(conv.id)
        .bind(draft.sender_id)
        .bind(counts)
        .bind(draft.mention_all)
        .bind(&mentioned)
        .bind(request::DECLINED)
        .fetch_all(&mut **tx)
        .await?;

        // The sender has seen everything up to their own message, and
        // answering a message request accepts it.
        let before: Option<SenderBefore> = match draft.sender_id {
            Some(sender_id) => {
                sqlx::query_as(
                    "WITH old AS (
                         SELECT read_seq, request_state, request_messages FROM chat_members
                         WHERE conversation_id = $1 AND user_id = $2 FOR UPDATE
                     )
                     UPDATE chat_members m SET
                         read_seq = $3, read_at = now(), unread_count = 0, unread_mention_count = 0, hidden_at = NULL,
                         request_state = $4, request_messages = m.request_messages + $5
                     FROM old WHERE m.conversation_id = $1 AND m.user_id = $2
                     RETURNING old.read_seq, old.request_state, old.request_messages",
                )
                .bind(conv.id)
                .bind(sender_id)
                .bind(seq)
                .bind(request::ACCEPTED)
                .bind(i32::from(draft.is_request))
                .fetch_optional(&mut **tx)
                .await?
            }
            None => None,
        };
        // The allowance was judged before the lock was taken; concurrent
        // sends all passed that check, so it is settled here, on the
        // sender's locked row (failing rolls the message back).
        if draft.is_request
            && before.as_ref().is_some_and(|before| before.request_messages >= self.cfg.request_message_limit)
        {
            return Err(access::request_pending());
        }

        if let (Notify::Standard(context), Some(sender_id)) = (&draft.notify, draft.sender_id) {
            let first_request = draft.is_request && before.as_ref().is_some_and(|before| before.request_messages == 0);
            request_message_notifications(tx, context, conv, &row, sender_id, &recipients, &mentioned, first_request)
                .await?;
        }

        let read_skipped = before.as_ref().is_some_and(|before| before.read_seq < seq - 1);
        let accepted_now = before.as_ref().is_some_and(|before| before.request_state != request::ACCEPTED);
        conv.last_seq = seq;
        conv.last_message_id = Some(message_id);
        let reply_sender = draft.reply_to.and_then(|(_, sender)| sender);
        Ok(Posted { row, duplicate: false, conv: conv.clone(), read_skipped, accepted_now, reply_sender })
    }

    /// Pushes a stored message to the members and reports it to the other
    /// domains. Call after the transaction of [`Chat::post`] committed.
    pub(crate) async fn announce(&self, posted: &Posted) {
        if posted.duplicate {
            return;
        }
        let row = &posted.row;
        let conv = &posted.conv;
        self.push_message_created(row).await;
        let Some(sender_id) = row.sender_id else { return };
        if posted.read_skipped {
            let event = pb::ReadReceiptChanged {
                conversation_id: conv.id.to_string(),
                user_id: sender_id.to_string(),
                read_seq: row.seq,
                read_at: ts(row.created_at),
                ..Default::default()
            };
            self.push_members(conv.id, Audience::Accepted, event).await;
        }
        if posted.accepted_now {
            self.push_conversation_to(conv.id, sender_id).await;
            if let Some(other) = conv.other_of(sender_id) {
                self.push_conversation_to(conv.id, other).await;
            }
        }
        if let Err(e) = self.report_message(posted, sender_id).await {
            tracing::warn!(message_id = %row.id, error = %e, "reporting a chat message to other domains failed");
        }
    }

    /// The side effects of a message outside the chat domain.
    async fn report_message(&self, posted: &Posted, sender_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        let row = &posted.row;
        let conv = &posted.conv;
        let interaction = match row.kind {
            msg::TEXT | msg::MEDIA | msg::LOCATION | msg::HELLO => Some(InteractionKind::Message),
            msg::STAMP => Some(InteractionKind::Stamp),
            _ => None,
        };
        if let Some(interaction) = interaction {
            let targets: Vec<Uuid> = match conv.other_of(sender_id) {
                Some(other) => vec![other],
                None => {
                    let mut addressed: Vec<Uuid> =
                        posted.reply_sender.into_iter().chain(row.mentions().into_iter().map(|m| m.user_id)).collect();
                    addressed.retain(|user_id| *user_id != sender_id);
                    addressed.sort_unstable();
                    addressed.dedup();
                    addressed.truncate(MAX_GROUP_INTERACTIONS);
                    addressed
                }
            };
            for target in targets {
                graph::record_interaction(core, sender_id, target, interaction, 1).await?;
            }
        }
        if let Some(stamp_id) = row.stamp_id {
            tokuly_stamp::record_send(core, sender_id, stamp_id, SendContext::Chat).await?;
        }
        if row.link_url.is_some() && row.link_checked_at.is_none() {
            core.jobs
                .enqueue_unique(JOB_LINK_PREVIEW, &row.id.to_string(), &LinkPreviewJob { message_id: row.id })
                .await?;
        }
        if interaction.is_some() {
            let event = AnalyticsEvent::new("chat.message.send")
                .user(Some(sender_id))
                .subject("conversation", conv.id)
                .props(serde_json::json!({ "kind": row.kind, "group": conv.is_group(), "forwarded": row.forwarded }));
            core.analytics.track(event).await;
        }
        Ok(())
    }

    /// Locks the conversation, stores one message and announces it: the
    /// whole pipeline for flows that change nothing else.
    pub(crate) async fn post_and_announce(&self, conversation_id: Uuid, draft: Draft) -> AppResult<Posted> {
        let mut tx = self.core.db.begin().await?;
        let mut conv = access::lock_conversation(&mut tx, conversation_id).await?;
        let posted = self.post(&mut tx, &mut conv, draft).await?;
        tx.commit().await?;
        self.announce(&posted).await;
        Ok(posted)
    }
}

/// Decides, member by member, who is told about a new message, and requests
/// those notifications in the message's transaction.
async fn request_message_notifications(
    tx: &mut Transaction<'_, Postgres>,
    context: &NotifyContext,
    conv: &ConversationRow,
    row: &MessageRow,
    sender_id: Uuid,
    recipients: &[Recipient],
    mentioned: &[Uuid],
    first_request: bool,
) -> AppResult<()> {
    let image = render::thumbnail_of(row, &Default::default());
    for recipient in recipients {
        let kind = if recipient.request_state == request::PENDING {
            // A message request announces itself once, not with every message.
            if !first_request {
                continue;
            }
            notices::KIND_REQUEST
        } else {
            if context.present.contains(&recipient.user_id) || context.hidden.contains(&recipient.user_id) {
                continue;
            }
            let by_name = mentioned.contains(&recipient.user_id);
            let muted = muted_now(recipient.muted, recipient.muted_until);
            if muted && !by_name {
                continue;
            }
            if by_name || row.mention_all { notices::KIND_MENTION } else { notices::KIND_MESSAGE }
        };
        let request = notices::message(recipient.user_id, kind, &context.sender, sender_id, conv, row, image.clone());
        outbound::notify(&mut **tx, &request).await?;
    }
    Ok(())
}
