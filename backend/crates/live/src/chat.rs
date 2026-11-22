//! Live chat: messages, who may write, pins, and the channel-wide
//! moderation tools (timeouts and bans, moderators, banned words).
//!
//! Guests read, members write. A message is refused, in this order, when:
//! the stream is over; a block stands between writer and streamer; the
//! writer is timed out or banned on the channel; the chat is closed; the
//! chat's audience excludes them (members-only, subscribers-only); the text
//! is invalid, contains a banned word or a link the chat does not allow;
//! they write too fast (rate limit, slow mode). The streamer and the
//! channel's moderators are exempt from everything but the first.
//!
//! The streamer writes as the channel: their messages carry the channel,
//! never the account, so a channel does not reveal who is behind it.
use std::{collections::HashMap, time::Duration};

use tokuly_core::{outbound, prelude::*};
use tokuly_proto::{
    common::{Author, ChannelSummary, StampRef, UserSummary},
    live::{
        ChatAudience, ChatGift, ChatMessage, ChatMessageDeleted, ChatMessageKind, ChatNotice, ChatPinChanged,
        ChatRestriction, ChatRestrictionChanged, ChatRole, ChatSettings, ChatUserCleared, LinkPolicy, Moderator,
        OverlayKind, live_event::Event,
    },
};
use tokuly_social::graph;
use tokuly_stamp::SendContext;
use tokuly_video::channels::{self, ChannelRow};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::{
    Live,
    access::{self, Standing},
    lifecycle,
    streams::{StreamRow, Tx},
};

/// Longest chat message, in characters as people count them.
pub const MAX_TEXT: usize = 200;
pub const MAX_BANNED_WORDS: usize = 200;
pub const MAX_BANNED_WORD: usize = 30;
pub const MAX_REASON: usize = 200;
pub const MAX_MODERATORS: i64 = 50;
/// Shortest and longest timeout, in seconds (10 s to 14 days).
pub const MIN_TIMEOUT_SECONDS: i32 = 10;
pub const MAX_TIMEOUT_SECONDS: i32 = 14 * 24 * 3600;
/// Messages returned by one replay request.
pub const REPLAY_LIMIT: i64 = 500;
/// Longest stretch one replay request may cover: ten minutes.
pub const MAX_REPLAY_SPAN_MS: i64 = 600_000;

pub const KIND_TEXT: i32 = ChatMessageKind::CHAT_MESSAGE_KIND_TEXT as i32;
pub const KIND_STAMP: i32 = ChatMessageKind::CHAT_MESSAGE_KIND_STAMP as i32;
pub const KIND_GIFT: i32 = ChatMessageKind::CHAT_MESSAGE_KIND_GIFT as i32;
pub const KIND_NOTICE: i32 = ChatMessageKind::CHAT_MESSAGE_KIND_NOTICE as i32;

const ROLE_OWNER: i32 = ChatRole::CHAT_ROLE_OWNER as i32;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ChatRow {
    pub id: Uuid,
    pub stream_id: Uuid,
    /// `None` for notices written by the server.
    pub user_id: Option<Uuid>,
    pub kind: i32,
    pub role: i32,
    pub text: String,
    pub stamp_id: Option<Uuid>,
    pub gift_id: Option<Uuid>,
    pub gift_coins: i64,
    pub gift_tier: i32,
    pub highlighted_until: Option<Time>,
    pub notice: Option<serde_json::Value>,
    pub offset_ms: i64,
    pub created_at: Time,
    pub deleted_at: Option<Time>,
}

macro_rules! chat_columns {
    () => {
        "m.id, m.stream_id, m.user_id, m.kind, m.role, m.text, m.stamp_id, m.gift_id, m.gift_coins, m.gift_tier, \
         m.highlighted_until, m.notice, m.offset_ms, m.created_at, m.deleted_at"
    };
}
pub(crate) use chat_columns;

// ───────────────────────────── text rules ─────────────────────────────

/// A chat message as it is stored: one line, 1 to `max` characters.
pub fn validate_text(input: &str, max: usize) -> AppResult<String> {
    // Chat is one line: line breaks and tabs become spaces.
    let text: String = input.chars().map(|c| if c.is_whitespace() { ' ' } else { c }).collect();
    let text = text.trim();
    ensure!(
        (1..=max).contains(&text.graphemes(true).count()) && !text.chars().any(char::is_control),
        invalid,
        "MESSAGE_INVALID",
        "a message is 1 to {max} characters"
    );
    Ok(text.to_owned())
}

/// Top-level domains that make a bare `name.tld` count as a link.
const LINK_TLDS: [&str; 24] = [
    "com", "net", "org", "jp", "io", "co", "me", "tv", "gg", "ly", "app", "dev", "xyz", "info", "biz", "site", "link",
    "live", "shop", "online", "page", "cc", "to", "be",
];

/// Whether a message contains something that reads as a link.
pub fn contains_link(text: &str) -> bool {
    let lower = text.to_lowercase();
    if lower.contains("http://") || lower.contains("https://") || lower.contains("www.") {
        return true;
    }
    lower.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-')).any(|token| {
        let token = token.trim_matches('.');
        match token.rsplit_once('.') {
            Some((name, tld)) => {
                LINK_TLDS.contains(&tld)
                    && name.rsplit('.').next().is_some_and(|label| label.chars().any(|c| c.is_ascii_alphabetic()))
            }
            None => false,
        }
    })
}

/// The banned word a message contains, if any (case-insensitive).
pub fn banned_word<'a>(words: &'a [String], text: &str) -> Option<&'a str> {
    let lower = text.to_lowercase();
    words.iter().map(String::as_str).find(|word| !word.is_empty() && lower.contains(word))
}

/// A channel's banned words as they are stored: trimmed, lower-cased,
/// without duplicates (`BANNED_WORDS_INVALID`).
pub fn normalize_banned_words<S: AsRef<str>>(inputs: &[S]) -> AppResult<Vec<String>> {
    let bad = |message: String| AppError::invalid("BANNED_WORDS_INVALID", message);
    let mut words: Vec<String> = Vec::new();
    for input in inputs {
        let word = input.as_ref().trim().to_lowercase();
        if word.is_empty() {
            continue;
        }
        if word.graphemes(true).count() > MAX_BANNED_WORD || word.chars().any(char::is_control) {
            return Err(bad(format!("a banned word is at most {MAX_BANNED_WORD} characters")));
        }
        if !words.contains(&word) {
            words.push(word);
        }
    }
    if words.len() > MAX_BANNED_WORDS {
        return Err(bad(format!("at most {MAX_BANNED_WORDS} banned words")));
    }
    Ok(words)
}

// ───────────────────────────── who may write ─────────────────────────────

/// Why someone may not write in a chat right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatBlock {
    pub reason: &'static str,
    /// End of a timeout.
    pub until: Option<Time>,
}

impl ChatBlock {
    fn new(reason: &'static str) -> Self {
        Self { reason, until: None }
    }

    pub fn into_error(self) -> AppError {
        match self.reason {
            "STREAM_ENDED" => AppError::precondition(self.reason, "the stream is over"),
            "CHAT_DISABLED" => AppError::precondition(self.reason, "the chat is closed"),
            "CHAT_TIMED_OUT" => AppError::denied(self.reason, "you are timed out from this chat"),
            "CHAT_BANNED" => AppError::denied(self.reason, "you are banned from this chat"),
            "MEMBERS_ONLY" => AppError::denied(self.reason, "this chat is for established members"),
            "SUBSCRIBERS_ONLY" => AppError::denied(self.reason, "this chat is for subscribers of the channel"),
            _ => AppError::denied(self.reason, "you cannot write in this chat"),
        }
    }
}

/// The restriction in force for a user on a channel: `Some(None)` is a
/// ban, `Some(Some(until))` a timeout.
pub async fn restriction_of(core: &Core, channel_id: Uuid, user_id: Uuid) -> AppResult<Option<Option<Time>>> {
    Ok(sqlx::query_scalar(
        "SELECT expires_at FROM live_chat_restrictions
         WHERE channel_id = $1 AND user_id = $2 AND (expires_at IS NULL OR expires_at > now())",
    )
    .bind(channel_id)
    .bind(user_id)
    .fetch_optional(&core.db)
    .await?)
}

/// Why `user_id` may not write in the chat of `stream`, if so. Message
/// content and pacing are checked separately when a message is sent.
pub async fn chat_block(
    core: &Core,
    stream: &StreamRow,
    settings: &ChatSettings,
    user_id: Uuid,
    standing: Standing,
) -> AppResult<Option<ChatBlock>> {
    if stream.is_ended() {
        return Ok(Some(ChatBlock::new("STREAM_ENDED")));
    }
    if standing.can_moderate() {
        return Ok(None);
    }
    if graph::is_blocked_either(core, stream.owner_id, user_id).await? {
        return Ok(Some(ChatBlock::new("CHAT_NOT_ALLOWED")));
    }
    match restriction_of(core, stream.channel_id, user_id).await? {
        Some(None) => return Ok(Some(ChatBlock::new("CHAT_BANNED"))),
        Some(until) => return Ok(Some(ChatBlock { reason: "CHAT_TIMED_OUT", until })),
        None => {}
    }
    if !settings.enabled {
        return Ok(Some(ChatBlock::new("CHAT_DISABLED")));
    }
    match settings.audience.as_known() {
        Some(ChatAudience::CHAT_AUDIENCE_MEMBERS) => {
            let old_enough: bool = sqlx::query_scalar(
                "SELECT COALESCE((SELECT created_at <= now() - make_interval(mins => $2) FROM users WHERE id = $1), false)",
            )
            .bind(user_id)
            .bind(settings.min_account_age_minutes)
            .fetch_one(&core.db)
            .await?;
            if !old_enough {
                return Ok(Some(ChatBlock::new("MEMBERS_ONLY")));
            }
        }
        Some(ChatAudience::CHAT_AUDIENCE_SUBSCRIBERS) => {
            if !channels::is_subscribed(core, user_id, stream.channel_id).await? {
                return Ok(Some(ChatBlock::new("SUBSCRIBERS_ONLY")));
            }
        }
        _ => {}
    }
    Ok(None)
}

/// The role a message of `user_id` carries.
async fn role_of(core: &Core, stream: &StreamRow, user_id: Uuid, standing: Standing) -> AppResult<ChatRole> {
    Ok(if standing.is_owner {
        ChatRole::CHAT_ROLE_OWNER
    } else if standing.is_appointed {
        ChatRole::CHAT_ROLE_MODERATOR
    } else if channels::is_subscribed(core, user_id, stream.channel_id).await? {
        ChatRole::CHAT_ROLE_SUBSCRIBER
    } else {
        ChatRole::CHAT_ROLE_VIEWER
    })
}

/// A viewer's standing on a stream's channel, as a chat role (for the
/// viewer list).
pub(crate) fn role_from_standing(standing: Standing, subscribed: bool) -> ChatRole {
    if standing.is_owner {
        ChatRole::CHAT_ROLE_OWNER
    } else if standing.is_appointed {
        ChatRole::CHAT_ROLE_MODERATOR
    } else if subscribed {
        ChatRole::CHAT_ROLE_SUBSCRIBER
    } else {
        ChatRole::CHAT_ROLE_VIEWER
    }
}

// ───────────────────────────── API representation ─────────────────────────────

/// Chat lines of one stream as the API returns them, in the order given.
pub async fn to_protos(core: &Core, stream: &StreamRow, rows: &[ChatRow]) -> AppResult<Vec<ChatMessage>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut user_ids: Vec<Uuid> =
        rows.iter().filter(|row| row.role != ROLE_OWNER).filter_map(|row| row.user_id).collect();
    user_ids.sort_unstable();
    user_ids.dedup();
    let users: HashMap<Uuid, UserSummary> = tokuly_identity::summaries(core, &user_ids).await?;
    let stamp_ids: Vec<Uuid> = rows.iter().filter_map(|row| row.stamp_id).collect();
    let stamps: HashMap<Uuid, StampRef> = tokuly_stamp::stamp_refs(core, &stamp_ids).await?;
    let channel: Option<ChannelSummary> = if rows.iter().any(|row| row.role == ROLE_OWNER) {
        channels::summaries(core, &[stream.channel_id]).await?.remove(&stream.channel_id)
    } else {
        None
    };
    Ok(rows.iter().map(|row| to_proto(row, &users, &stamps, channel.as_ref())).collect())
}

pub async fn to_proto_one(core: &Core, stream: &StreamRow, row: &ChatRow) -> AppResult<ChatMessage> {
    Ok(to_protos(core, stream, std::slice::from_ref(row)).await?.pop().unwrap_or_default())
}

fn to_proto(
    row: &ChatRow,
    users: &HashMap<Uuid, UserSummary>,
    stamps: &HashMap<Uuid, StampRef>,
    channel: Option<&ChannelSummary>,
) -> ChatMessage {
    let author = match (row.role == ROLE_OWNER, row.user_id) {
        // The streamer speaks as the channel.
        (true, _) => Some(Author { channel: channel.cloned().into(), ..Default::default() }),
        (false, Some(user_id)) => {
            Some(Author { user: Some(tokuly_identity::users::pick(users, user_id)).into(), ..Default::default() })
        }
        (false, None) => None,
    };
    let gift = row.gift_id.map(|gift_id| ChatGift {
        gift_id: gift_id.to_string(),
        coins: row.gift_coins,
        tier: row.gift_tier,
        highlighted_until: opt_ts(row.highlighted_until),
        ..Default::default()
    });
    let notice: Option<ChatNotice> = row.notice.clone().and_then(|value| serde_json::from_value(value).ok());
    ChatMessage {
        id: row.id.to_string(),
        stream_id: row.stream_id.to_string(),
        kind: row.kind.into(),
        author: author.into(),
        role: row.role.into(),
        text: row.text.clone(),
        stamp: row.stamp_id.and_then(|id| stamps.get(&id).cloned()).into(),
        gift: gift.into(),
        notice: notice.into(),
        offset_ms: row.offset_ms,
        created_at: ts(row.created_at),
        ..Default::default()
    }
}

// ───────────────────────────── lookups ─────────────────────────────

/// A message that is in the chat (deleted ones are not found).
pub async fn find(core: &Core, message_id: Uuid) -> AppResult<Option<ChatRow>> {
    Ok(find_with_deleted(core, message_id).await?.filter(|message| message.deleted_at.is_none()))
}

/// A message whether or not it was removed from the chat since. Removed
/// messages are kept (until their stream is erased) so that what was sent
/// can still be reported: deleting a message does not undo having sent it.
pub async fn find_with_deleted(core: &Core, message_id: Uuid) -> AppResult<Option<ChatRow>> {
    Ok(sqlx::query_as(concat!("SELECT ", chat_columns!(), " FROM live_chat_messages m WHERE m.id = $1"))
        .bind(message_id)
        .fetch_optional(&core.db)
        .await?)
}

/// Messages of a stream before `before`, newest first, without those of
/// the users in `hidden` (people the reader blocked or is blocked by).
pub async fn list(
    core: &Core,
    stream_id: Uuid,
    before: Option<Uuid>,
    hidden: &[Uuid],
    limit: i64,
) -> AppResult<Vec<ChatRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        chat_columns!(),
        " FROM live_chat_messages m
          WHERE m.stream_id = $1 AND m.deleted_at IS NULL AND ($2::uuid IS NULL OR m.id < $2)
            AND (m.user_id IS NULL OR NOT (m.user_id = ANY($3)))
          ORDER BY m.id DESC LIMIT $4"
    ))
    .bind(stream_id)
    .bind(before)
    .bind(hidden)
    .bind(limit)
    .fetch_all(&core.db)
    .await?)
}

/// Messages of a stream whose offset lies in `[from_ms, to_ms)`, in order.
pub async fn replay(
    core: &Core,
    stream_id: Uuid,
    from_ms: i64,
    to_ms: i64,
    hidden: &[Uuid],
    limit: i64,
) -> AppResult<Vec<ChatRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        chat_columns!(),
        " FROM live_chat_messages m
          WHERE m.stream_id = $1 AND m.deleted_at IS NULL AND m.offset_ms >= $2 AND m.offset_ms < $3
            AND (m.user_id IS NULL OR NOT (m.user_id = ANY($4)))
          ORDER BY m.offset_ms, m.id LIMIT $5"
    ))
    .bind(stream_id)
    .bind(from_ms)
    .bind(to_ms)
    .bind(hidden)
    .bind(limit)
    .fetch_all(&core.db)
    .await?)
}

/// The pinned message of a stream, if it still exists.
pub async fn pinned(core: &Core, stream: &StreamRow) -> AppResult<Option<ChatRow>> {
    match stream.pinned_message_id {
        Some(message_id) => find(core, message_id).await,
        None => Ok(None),
    }
}

/// Gift messages whose highlight time has not run out, newest first.
pub async fn highlighted(core: &Core, stream_id: Uuid) -> AppResult<Vec<ChatRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        chat_columns!(),
        " FROM live_chat_messages m
          WHERE m.stream_id = $1 AND m.deleted_at IS NULL AND m.highlighted_until > now()
          ORDER BY m.id DESC LIMIT 30"
    ))
    .bind(stream_id)
    .fetch_all(&core.db)
    .await?)
}

/// The banned words of a channel.
pub async fn banned_words(core: &Core, channel_id: Uuid) -> AppResult<Vec<String>> {
    let words: Option<Vec<String>> =
        sqlx::query_scalar("SELECT banned_words FROM live_channel_settings WHERE channel_id = $1")
            .bind(channel_id)
            .fetch_optional(&core.db)
            .await?;
    Ok(words.unwrap_or_default())
}

// ───────────────────────────── writing ─────────────────────────────

/// A row to insert into the chat.
pub(crate) struct NewMessage<'a> {
    pub stream_id: Uuid,
    pub user_id: Option<Uuid>,
    pub kind: i32,
    pub role: ChatRole,
    pub text: &'a str,
    pub stamp_id: Option<Uuid>,
    pub gift: Option<(Uuid, i64, i32, i64)>,
    pub notice: Option<&'a ChatNotice>,
}

/// Inserts a chat line at the stream's current media offset and counts it.
pub(crate) async fn insert_in(tx: &mut Tx<'_>, new: NewMessage<'_>) -> AppResult<ChatRow> {
    let offset_ms = lifecycle::offset_ms_now(&mut **tx, new.stream_id).await?;
    let (gift_id, gift_coins, gift_tier, highlight_seconds) = match new.gift {
        Some((id, coins, tier, seconds)) => (Some(id), coins, tier, seconds),
        None => (None, 0, 0, 0),
    };
    let notice = new
        .notice
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| AppError::internal(anyhow::anyhow!("chat notice encode: {e}")))?;
    let row: ChatRow = sqlx::query_as(concat!(
        "INSERT INTO live_chat_messages AS m (id, stream_id, user_id, kind, role, text, stamp_id, gift_id, gift_coins, gift_tier,
                                             highlighted_until, notice, offset_ms)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                 CASE WHEN $11 > 0 THEN now() + make_interval(secs => $11) END, $12, $13)
         RETURNING ",
        chat_columns!()
    ))
    .bind(new_id())
    .bind(new.stream_id)
    .bind(new.user_id)
    .bind(new.kind)
    .bind(new.role as i32)
    .bind(new.text)
    .bind(new.stamp_id)
    .bind(gift_id)
    .bind(gift_coins)
    .bind(gift_tier)
    .bind(highlight_seconds as f64)
    .bind(notice)
    .bind(offset_ms)
    .fetch_one(&mut **tx)
    .await?;
    if new.kind != KIND_NOTICE {
        sqlx::query("UPDATE live_streams SET chat_message_count = chat_message_count + 1 WHERE id = $1")
            .bind(new.stream_id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(row)
}

/// Serializes, for the rest of the transaction, what one user does in the
/// chats of one channel against what is done to them there: a message and
/// a ban of its author cannot pass each other. The lock is per user, so
/// nobody waits for anyone else.
async fn lock_chatter(tx: &mut Tx<'_>, channel_id: Uuid, user_id: Uuid) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("live:chatter:{}:{}", channel_id.simple(), user_id.simple()))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

impl Live {
    /// Pushes a new chat line to the stream and the channel's chat boxes.
    pub(crate) async fn push_chat_message(&self, stream: &StreamRow, row: &ChatRow) {
        match to_proto_one(&self.core, stream, row).await {
            Ok(message) => {
                let event = Event::ChatMessage(Box::new(message));
                self.push_to_stream(stream.id, event.clone()).await;
                self.push_to_overlays(stream.channel_id, &[OverlayKind::OVERLAY_KIND_CHAT_BOX], event).await;
            }
            Err(error) => tracing::warn!(message_id = %row.id, %error, "could not push a chat message"),
        }
    }

    /// Posts a text message or a stamp for `user_id`.
    pub async fn post_chat_message(
        &self,
        stream: &StreamRow,
        user_id: Uuid,
        text: &str,
        stamp_id: Option<Uuid>,
    ) -> AppResult<ChatRow> {
        let core = &self.core;
        let settings = stream.chat_settings();
        let standing = access::standing(core, stream.channel_id, stream.owner_id, user_id).await?;
        if let Some(block) = chat_block(core, stream, &settings, user_id, standing).await? {
            return Err(block.into_error());
        }
        let privileged = standing.can_moderate();

        let (kind, text, stamp) = match stamp_id {
            Some(stamp_id) => {
                ensure!(text.trim().is_empty(), invalid, "MESSAGE_INVALID", "send either text or a stamp");
                ensure!(
                    settings.stamps_enabled || privileged,
                    precondition,
                    "STAMPS_DISABLED",
                    "stamps are off in this chat"
                );
                tokuly_stamp::ensure_usable(core, user_id, stamp_id).await?;
                (KIND_STAMP, String::new(), Some(stamp_id))
            }
            None => {
                let text = validate_text(text, MAX_TEXT)?;
                if !privileged {
                    ensure!(
                        banned_word(&banned_words(core, stream.channel_id).await?, &text).is_none(),
                        invalid,
                        "MESSAGE_BLOCKED",
                        "this message contains a word the streamer does not allow"
                    );
                    ensure!(
                        settings.link_policy != LinkPolicy::LINK_POLICY_BLOCK || !contains_link(&text),
                        invalid,
                        "LINKS_NOT_ALLOWED",
                        "links are not allowed in this chat"
                    );
                }
                (KIND_TEXT, text, None)
            }
        };

        core.cache.rate_limit(&format!("live-chat:{}:{user_id}", stream.id), 8, Duration::from_secs(10)).await?;
        if !privileged && settings.slow_mode_seconds > 0 {
            let free = core
                .cache
                .set_nx(
                    &format!("live:slow:{}:{user_id}", stream.id),
                    "1",
                    Duration::from_secs(settings.slow_mode_seconds as u64),
                )
                .await?;
            ensure!(
                free,
                rate_limited,
                "SLOW_MODE",
                "slow mode: wait {} seconds between messages",
                settings.slow_mode_seconds
            );
        }

        let role = role_of(core, stream, user_id, standing).await?;
        let mut tx = core.db.begin().await?;
        if !privileged {
            // A timeout or ban imposed since the check above (or right now)
            // wins: under the lock, either this message is in the chat
            // before the restriction, or it is refused.
            lock_chatter(&mut tx, stream.channel_id, user_id).await?;
            let restricted: Option<Option<Time>> = sqlx::query_scalar(
                "SELECT expires_at FROM live_chat_restrictions
                 WHERE channel_id = $1 AND user_id = $2 AND (expires_at IS NULL OR expires_at > now())",
            )
            .bind(stream.channel_id)
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
            match restricted {
                Some(None) => return Err(ChatBlock::new("CHAT_BANNED").into_error()),
                Some(until) => return Err(ChatBlock { reason: "CHAT_TIMED_OUT", until }.into_error()),
                None => {}
            }
        }
        let row = insert_in(
            &mut tx,
            NewMessage {
                stream_id: stream.id,
                user_id: Some(user_id),
                kind,
                role,
                text: &text,
                stamp_id: stamp,
                gift: None,
                notice: None,
            },
        )
        .await?;
        tx.commit().await?;

        if let Some(stamp_id) = stamp
            && let Err(error) = tokuly_stamp::record_send(core, user_id, stamp_id, SendContext::LiveChat).await
        {
            tracing::warn!(%stamp_id, %error, "could not record a stamp sent in live chat");
        }
        core.analytics
            .track(
                tokuly_core::analytics::AnalyticsEvent::new("live.chat")
                    .user(Some(user_id))
                    .subject("live_stream", stream.id)
                    .owner(stream.owner_id)
                    .value(1.0)
                    .props(serde_json::json!({ "channel_id": stream.channel_id, "kind": kind })),
            )
            .await;
        self.push_chat_message(stream, &row).await;
        Ok(row)
    }

    /// Removes a message from the chat (and the pin, if it was pinned).
    pub async fn remove_chat_message(&self, stream: &StreamRow, message: &ChatRow) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let deleted =
            sqlx::query("UPDATE live_chat_messages SET deleted_at = now() WHERE id = $1 AND deleted_at IS NULL")
                .bind(message.id)
                .execute(&mut *tx)
                .await?
                .rows_affected()
                == 1;
        let unpinned =
            sqlx::query("UPDATE live_streams SET pinned_message_id = NULL WHERE id = $1 AND pinned_message_id = $2")
                .bind(stream.id)
                .bind(message.id)
                .execute(&mut *tx)
                .await?
                .rows_affected()
                == 1;
        tx.commit().await?;
        if deleted {
            let gone = ChatMessageDeleted {
                stream_id: stream.id.to_string(),
                message_id: message.id.to_string(),
                ..Default::default()
            };
            let event = Event::ChatMessageDeleted(Box::new(gone));
            self.push_to_stream(stream.id, event.clone()).await;
            self.push_to_overlays(stream.channel_id, &[OverlayKind::OVERLAY_KIND_CHAT_BOX], event).await;
        }
        if unpinned {
            self.push_pin(stream, None).await;
        }
        Ok(())
    }

    async fn push_pin(&self, stream: &StreamRow, message: Option<ChatMessage>) {
        let change = ChatPinChanged { stream_id: stream.id.to_string(), message: message.into(), ..Default::default() };
        self.push_to_stream(stream.id, Event::ChatPinChanged(Box::new(change))).await;
    }

    /// Pins a message (`Some`) or clears the pin (`None`).
    pub async fn set_pinned_message(
        &self,
        stream: &StreamRow,
        message: Option<&ChatRow>,
    ) -> AppResult<Option<ChatMessage>> {
        sqlx::query("UPDATE live_streams SET pinned_message_id = $2 WHERE id = $1")
            .bind(stream.id)
            .bind(message.map(|m| m.id))
            .execute(&self.core.db)
            .await?;
        let proto = match message {
            Some(message) => Some(to_proto_one(&self.core, stream, message).await?),
            None => None,
        };
        self.push_pin(stream, proto.clone()).await;
        Ok(proto)
    }

    // ───────────────────────────── restrictions ─────────────────────────────

    /// Times out (`duration_seconds > 0`) or bans (`0`) a user on a channel.
    pub async fn restrict_user(
        &self,
        channel: &ChannelRow,
        moderator_id: Uuid,
        standing: Standing,
        target_id: Uuid,
        duration_seconds: i32,
        reason: &str,
        delete_messages: bool,
    ) -> AppResult<ChatRestriction> {
        let core = &self.core;
        ensure!(
            duration_seconds == 0 || (MIN_TIMEOUT_SECONDS..=MAX_TIMEOUT_SECONDS).contains(&duration_seconds),
            invalid,
            "RESTRICTION_INVALID",
            "a timeout lasts 10 seconds to 14 days; 0 bans"
        );
        let reason = reason.trim();
        ensure!(
            reason.graphemes(true).count() <= MAX_REASON && !reason.chars().any(char::is_control),
            invalid,
            "RESTRICTION_INVALID",
            "the reason is at most {MAX_REASON} characters"
        );
        ensure!(
            target_id != channel.owner_id && target_id != moderator_id,
            denied,
            "CANNOT_RESTRICT",
            "this user cannot be restricted"
        );
        let target =
            tokuly_identity::summaries(core, &[target_id]).await?.remove(&target_id).filter(|user| !user.deleted);
        let target = target.or_not_found("USER_NOT_FOUND", "user not found")?;
        let target_standing = access::standing(core, channel.id, channel.owner_id, target_id).await?;
        // Moderators answer to the streamer, not to each other.
        ensure!(
            standing.is_owner || !target_standing.is_appointed,
            denied,
            "CANNOT_RESTRICT",
            "only the streamer can restrict a moderator"
        );

        let mut tx = core.db.begin().await?;
        lock_chatter(&mut tx, channel.id, target_id).await?;
        let (expires_at, created_at): (Option<Time>, Time) = sqlx::query_as(
            "INSERT INTO live_chat_restrictions (channel_id, user_id, expires_at, reason, moderator_id)
             VALUES ($1, $2, CASE WHEN $3 > 0 THEN now() + make_interval(secs => $3) END, $4, $5)
             ON CONFLICT (channel_id, user_id) DO UPDATE
                 SET expires_at = EXCLUDED.expires_at, reason = EXCLUDED.reason, moderator_id = EXCLUDED.moderator_id,
                     created_at = now()
             RETURNING expires_at, created_at",
        )
        .bind(channel.id)
        .bind(target_id)
        .bind(f64::from(duration_seconds))
        .bind(reason)
        .bind(moderator_id)
        .fetch_one(&mut *tx)
        .await?;
        // A restricted moderator is no moderator.
        sqlx::query("DELETE FROM live_moderators WHERE channel_id = $1 AND user_id = $2")
            .bind(channel.id)
            .bind(target_id)
            .execute(&mut *tx)
            .await?;
        let cleared: Vec<(Uuid, Uuid)> = if delete_messages {
            sqlx::query_as(
                "UPDATE live_chat_messages m SET deleted_at = now()
                 FROM live_streams s
                 WHERE s.id = m.stream_id AND s.channel_id = $1 AND s.status < 3 AND m.user_id = $2 AND m.deleted_at IS NULL
                   AND m.kind <> 3
                 RETURNING s.id, s.channel_id",
            )
            .bind(channel.id)
            .bind(target_id)
            .fetch_all(&mut *tx)
            .await?
        } else {
            Vec::new()
        };
        tx.commit().await?;

        let mut cleared_streams: Vec<Uuid> = cleared.into_iter().map(|(stream_id, _)| stream_id).collect();
        cleared_streams.sort_unstable();
        cleared_streams.dedup();
        for stream_id in cleared_streams {
            let event = Event::ChatUserCleared(Box::new(ChatUserCleared {
                stream_id: stream_id.to_string(),
                user_id: target_id.to_string(),
                ..Default::default()
            }));
            self.push_to_stream(stream_id, event.clone()).await;
            self.push_to_overlays(channel.id, &[OverlayKind::OVERLAY_KIND_CHAT_BOX], event).await;
        }
        let change = ChatRestrictionChanged {
            channel_id: channel.id.to_string(),
            restricted: true,
            expires_at: opt_ts(expires_at),
            ..Default::default()
        };
        self.push_to_user(target_id, Event::ChatRestrictionChanged(Box::new(change))).await;
        Ok(ChatRestriction {
            user: Some(target).into(),
            expires_at: opt_ts(expires_at),
            reason: reason.to_owned(),
            moderator: if standing.is_owner { None } else { Some(tokuly_identity::summary(core, moderator_id).await?) }
                .into(),
            created_at: ts(created_at),
            ..Default::default()
        })
    }

    /// Lifts a timeout or ban. Returns whether there was one.
    pub async fn unrestrict_user(&self, channel_id: Uuid, target_id: Uuid) -> AppResult<bool> {
        let lifted = sqlx::query("DELETE FROM live_chat_restrictions WHERE channel_id = $1 AND user_id = $2")
            .bind(channel_id)
            .bind(target_id)
            .execute(&self.core.db)
            .await?
            .rows_affected()
            == 1;
        if lifted {
            let change =
                ChatRestrictionChanged { channel_id: channel_id.to_string(), restricted: false, ..Default::default() };
            self.push_to_user(target_id, Event::ChatRestrictionChanged(Box::new(change))).await;
        }
        Ok(lifted)
    }

    // ───────────────────────────── moderators ─────────────────────────────

    /// Appoints `user_id` moderator of a channel's chats and tells them.
    pub async fn appoint_moderator(&self, channel: &ChannelRow, user_id: Uuid) -> AppResult<Moderator> {
        let core = &self.core;
        ensure!(user_id != channel.owner_id, invalid, "CANNOT_MODERATE_SELF", "the streamer needs no appointment");
        let user = tokuly_identity::summaries(core, &[user_id]).await?.remove(&user_id).filter(|user| !user.deleted);
        let user = user.or_not_found("USER_NOT_FOUND", "user not found")?;
        ensure!(
            !graph::is_blocked_either(core, channel.owner_id, user_id).await?,
            not_found,
            "USER_NOT_FOUND",
            "user not found"
        );
        // An appointment notifies its target in the channel's name, so it
        // is only open towards people who chose a tie to the channel or its
        // owner; strangers cannot be reached this way.
        let eligible = channels::is_subscribed(core, user_id, channel.id).await?
            || graph::are_friends(core, channel.owner_id, user_id).await?;
        ensure!(
            eligible,
            precondition,
            "MODERATOR_NOT_ELIGIBLE",
            "moderators are chosen among the channel's subscribers and the streamer's friends"
        );
        let mut tx = core.db.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("live:moderators:{}", channel.id.simple()))
            .execute(&mut *tx)
            .await?;
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM live_moderators WHERE channel_id = $1 AND user_id <> $2")
                .bind(channel.id)
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await?;
        ensure!(
            count < MAX_MODERATORS,
            precondition,
            "MODERATOR_LIMIT_REACHED",
            "a channel has at most {MAX_MODERATORS} moderators"
        );
        let (appointed_at, created): (Time, bool) = sqlx::query_as(
            "INSERT INTO live_moderators (channel_id, user_id) VALUES ($1, $2)
             ON CONFLICT (channel_id, user_id) DO UPDATE SET channel_id = EXCLUDED.channel_id
             RETURNING created_at, (xmax = 0)",
        )
        .bind(channel.id)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        if created {
            // A moderator is trusted: an old restriction does not survive the appointment.
            sqlx::query("DELETE FROM live_chat_restrictions WHERE channel_id = $1 AND user_id = $2")
                .bind(channel.id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
            // No actor: the channel speaks, not the account behind it.
            let request = outbound::NotificationRequest::new(
                user_id,
                "live.moderator",
                outbound::Localized::same(channel.name.clone()),
                outbound::Localized::new(
                    format!("「{}」のライブチャットのモデレーターになったよ!", channel.name),
                    format!("You are now a chat moderator of {}", channel.name),
                ),
                channel.path(),
            )
            .subject(tokuly_proto::common::SubjectType::SUBJECT_TYPE_CHANNEL, channel.id)
            .collapse(format!("live.moderator:{}", channel.id))
            .data(serde_json::json!({ "channel_id": channel.id, "channel_handle": channel.handle }));
            outbound::notify(&mut *tx, &request).await?;
        }
        tx.commit().await?;
        Ok(Moderator { user: Some(user).into(), appointed_at: ts(appointed_at), ..Default::default() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_text_is_one_trimmed_line_of_limited_length() {
        assert_eq!(validate_text("  こんにちは!\n元気?  ", MAX_TEXT).unwrap(), "こんにちは! 元気?");
        assert_eq!(validate_text(&"あ".repeat(200), MAX_TEXT).unwrap().chars().count(), 200);
        for bad in ["", "   ", "\n\t", &"あ".repeat(201), "bell\u{7}"] {
            assert_eq!(validate_text(bad, MAX_TEXT).unwrap_err().reason, "MESSAGE_INVALID", "{bad:?}");
        }
        // Emoji with modifiers count as one character each.
        assert!(validate_text(&"👨‍👩‍👧‍👦".repeat(200), MAX_TEXT).is_ok());
    }

    #[test]
    fn links_are_recognized_in_their_usual_shapes() {
        for text in [
            "見て https://example.com/x",
            "HTTP://EXAMPLE.COM",
            "www.example.jp だよ",
            "example.com",
            "みて→tokuly.app/live",
            "a.b.example.io!",
        ] {
            assert!(contains_link(text), "{text}");
        }
        for text in ["こんにちは", "3.14", "v1.2.3", "え...なに", "a.b", "end of sentence.next", "10.com"] {
            assert!(!contains_link(text), "{text}");
        }
    }

    #[test]
    fn banned_words_match_inside_messages_whatever_the_case() {
        let words = normalize_banned_words(&[" Spoiler ", "ネタバレ", "spoiler", ""]).unwrap();
        assert_eq!(words, vec!["spoiler".to_owned(), "ネタバレ".to_owned()]);
        assert_eq!(banned_word(&words, "No SPOILERS please"), Some("spoiler"));
        assert_eq!(banned_word(&words, "それネタバレだよ"), Some("ネタバレ"));
        assert_eq!(banned_word(&words, "hello"), None);
        assert_eq!(normalize_banned_words(&["x".repeat(31)]).unwrap_err().reason, "BANNED_WORDS_INVALID");
        let many: Vec<String> = (0..201).map(|i| format!("w{i}")).collect();
        assert_eq!(normalize_banned_words(&many).unwrap_err().reason, "BANNED_WORDS_INVALID");
    }

    #[test]
    fn chat_blocks_become_errors_with_their_reason() {
        for reason in [
            "STREAM_ENDED",
            "CHAT_DISABLED",
            "CHAT_TIMED_OUT",
            "CHAT_BANNED",
            "MEMBERS_ONLY",
            "SUBSCRIBERS_ONLY",
            "CHAT_NOT_ALLOWED",
        ] {
            assert_eq!(ChatBlock::new(reason).into_error().reason, reason);
        }
    }
}
