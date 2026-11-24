//! Turning stored messages into API messages.
//!
//! A message row holds IDs (sender, stamp, answered message, call, POV
//! request…); [`Chat::render_messages`] resolves them for a whole page of
//! messages with one query per kind of reference. What a message looks like
//! depends on who looks ([`View`]): reactions "by me", read counts, and
//! whether the quote above a reply may be shown.
use std::collections::{HashMap, HashSet};

use tokuly_core::prelude::*;
use tokuly_identity::users;
use tokuly_media::assets;
use tokuly_proto::{
    chat as pb,
    common::{LocalizedText, StampRef, UserSummary},
    media::{AssetKind, Attachment, LinkPreview},
};

use crate::{
    Chat, content,
    model::{CALL, CallRow, MSG, MemberRow, MessageRow, POV, PovRow, msg, request},
};

/// Who a message is rendered for.
#[derive(Debug, Clone, Default)]
pub(crate) struct View {
    /// The viewing member; `None` for realtime events, which go to every
    /// member alike and therefore carry no per-recipient state.
    pub user_id: Option<Uuid>,
    /// Per conversation, the `seq` at or below which messages are invisible
    /// to the viewer (they joined later, or cleared the history).
    floors: HashMap<Uuid, i64>,
    /// Fill in `Message.read_count`.
    pub read_counts: bool,
}

impl View {
    /// For realtime events.
    pub fn event() -> Self {
        Self::default()
    }

    /// For an RPC answered to `me`, about `me`'s conversation.
    pub fn member(me: &MemberRow) -> Self {
        Self {
            user_id: Some(me.user_id),
            floors: HashMap::from([(me.conversation_id, me.history_from_seq)]),
            read_counts: true,
        }
    }

    /// For an RPC answered to `user_id` spanning several conversations.
    pub fn user(user_id: Uuid, members: impl IntoIterator<Item = (Uuid, i64)>) -> Self {
        Self { user_id: Some(user_id), floors: members.into_iter().collect(), read_counts: true }
    }

    /// Hides everything at or below `seq` in one conversation.
    pub fn with_floor(mut self, conversation_id: Uuid, seq: i64) -> Self {
        self.floors.insert(conversation_id, seq);
        self
    }

    fn sees(&self, conversation_id: Uuid, seq: i64) -> bool {
        seq > self.floors.get(&conversation_id).copied().unwrap_or(0)
    }
}

#[derive(sqlx::FromRow)]
struct ReactionRow {
    message_id: Uuid,
    emoji: String,
    count: i64,
    mine: bool,
}

/// The image that stands for an attachment in a small quote: a small
/// variant of a photo, or the poster of a video.
fn attachment_thumbnail(attachment: &Attachment) -> Option<String> {
    let image = attachment
        .image
        .as_option()
        .or_else(|| attachment.video.as_option().and_then(|video| video.poster.as_option()))?;
    let small = image.variants.iter().filter(|variant| variant.size >= 256).min_by_key(|variant| variant.size);
    Some(small.map(|variant| variant.url.clone()).unwrap_or_else(|| image.url.clone())).filter(|url| !url.is_empty())
}

/// The first image that can stand for a message (reply quotes, notifications).
pub(crate) fn thumbnail_of(row: &MessageRow, stamps: &HashMap<Uuid, StampRef>) -> Option<String> {
    if let Some(stamp) = row.stamp_id.and_then(|id| stamps.get(&id)) {
        return Some(stamp.image_url.clone()).filter(|url| !url.is_empty());
    }
    row.attachments().iter().find_map(attachment_thumbnail)
}

impl Chat {
    pub(crate) async fn render_message(&self, row: &MessageRow, view: &View) -> AppResult<pb::Message> {
        let mut rendered = self.render_messages(std::slice::from_ref(row), view).await?;
        Ok(rendered.pop().unwrap_or_default())
    }

    /// Renders messages in the order given.
    pub(crate) async fn render_messages(&self, rows: &[MessageRow], view: &View) -> AppResult<Vec<pb::Message>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let core = &self.core;
        let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();

        // The messages answered by replies on this page.
        let reply_ids: Vec<Uuid> = rows
            .iter()
            .filter(|row| !row.is_deleted())
            .filter_map(|row| row.reply_to_id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let replied: HashMap<Uuid, MessageRow> = if reply_ids.is_empty() {
            HashMap::new()
        } else {
            sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!(
                "SELECT {MSG} FROM chat_messages x WHERE x.id = ANY($1)"
            )))
            .bind(&reply_ids)
            .fetch_all(&core.db)
            .await?
            .into_iter()
            .map(|row| (row.id, row))
            .collect()
        };

        let mut user_ids: HashSet<Uuid> = HashSet::new();
        let mut stamp_ids: HashSet<Uuid> = HashSet::new();
        let mut call_ids: HashSet<Uuid> = HashSet::new();
        let mut pov_ids: HashSet<Uuid> = HashSet::new();
        for row in rows.iter().chain(replied.values()) {
            user_ids.extend(row.sender_id);
            if row.is_deleted() {
                continue;
            }
            stamp_ids.extend(row.stamp_id);
        }
        for row in rows.iter().filter(|row| !row.is_deleted()) {
            user_ids.extend(row.mentions().iter().map(|mention| mention.user_id));
            if let Some(system) = row.system() {
                user_ids.extend(system.actor_id);
                user_ids.extend(system.target_ids);
            }
            call_ids.extend(row.call_id);
            pov_ids.extend(row.pov_request_id);
        }

        let users = tokuly_identity::summaries(core, &user_ids.into_iter().collect::<Vec<_>>()).await?;
        let stamps = tokuly_stamp::stamp_refs(core, &stamp_ids.into_iter().collect::<Vec<_>>()).await?;
        let calls: HashMap<Uuid, CallRow> = if call_ids.is_empty() {
            HashMap::new()
        } else {
            sqlx::query_as::<_, CallRow>(sqlx::AssertSqlSafe(format!(
                "SELECT {CALL} FROM chat_calls k WHERE k.id = ANY($1)"
            )))
            .bind(call_ids.into_iter().collect::<Vec<_>>())
            .fetch_all(&core.db)
            .await?
            .into_iter()
            .map(|call| (call.id, call))
            .collect()
        };
        let povs: HashMap<Uuid, PovRow> = if pov_ids.is_empty() {
            HashMap::new()
        } else {
            sqlx::query_as::<_, PovRow>(sqlx::AssertSqlSafe(format!(
                "SELECT {POV} FROM chat_pov_requests p WHERE p.id = ANY($1)"
            )))
            .bind(pov_ids.into_iter().collect::<Vec<_>>())
            .fetch_all(&core.db)
            .await?
            .into_iter()
            .map(|pov| (pov.id, pov))
            .collect()
        };

        let mut reactions: HashMap<Uuid, Vec<pb::ReactionCount>> = HashMap::new();
        let reaction_rows: Vec<ReactionRow> = sqlx::query_as(
            "SELECT message_id, emoji, count(*) AS count, COALESCE(bool_or(user_id = $2), false) AS mine
             FROM chat_reactions WHERE message_id = ANY($1)
             GROUP BY message_id, emoji ORDER BY min(created_at), emoji",
        )
        .bind(&ids)
        .bind(view.user_id)
        .fetch_all(&core.db)
        .await?;
        for row in reaction_rows {
            reactions.entry(row.message_id).or_default().push(pb::ReactionCount {
                emoji: row.emoji,
                count: row.count as i32,
                reacted_by_me: row.mine,
                ..Default::default()
            });
        }

        let pinned: HashSet<Uuid> = sqlx::query_scalar("SELECT message_id FROM chat_pins WHERE message_id = ANY($1)")
            .bind(&ids)
            .fetch_all(&core.db)
            .await?
            .into_iter()
            .collect();

        // Read cursors of the members who accepted, per conversation.
        let mut cursors: HashMap<Uuid, Vec<(Uuid, i64)>> = HashMap::new();
        if view.read_counts {
            let conversation_ids: Vec<Uuid> =
                rows.iter().map(|row| row.conversation_id).collect::<HashSet<_>>().into_iter().collect();
            let rows: Vec<(Uuid, Uuid, i64)> = sqlx::query_as(
                "SELECT conversation_id, user_id, read_seq FROM chat_members WHERE conversation_id = ANY($1) AND request_state = $2",
            )
            .bind(&conversation_ids)
            .bind(request::ACCEPTED)
            .fetch_all(&core.db)
            .await?;
            for (conversation_id, user_id, read_seq) in rows {
                cursors.entry(conversation_id).or_default().push((user_id, read_seq));
            }
        }

        let mut messages = Vec::with_capacity(rows.len());
        for row in rows {
            let sender = row.sender_id.map(|id| users::pick(&users, id));
            let mut message = pb::Message {
                id: row.id.to_string(),
                conversation_id: row.conversation_id.to_string(),
                seq: row.seq,
                sender: sender.into(),
                kind: i32::from(row.kind).into(),
                created_at: ts(row.created_at),
                ..Default::default()
            };
            if row.is_deleted() {
                message.deleted = true;
                messages.push(message);
                continue;
            }
            message.client_id = row.client_id.clone().unwrap_or_default();
            message.text = row.text.clone();
            message.mentions = row
                .mentions()
                .into_iter()
                .map(|mention| pb::Mention {
                    user: users::pick(&users, mention.user_id).into(),
                    handle: mention.handle,
                    ..Default::default()
                })
                .collect();
            message.mentions_everyone = row.mention_all;
            message.big_emoji_count = if row.kind == msg::TEXT { content::big_emoji_count(&row.text) } else { 0 };
            message.stamp = row.stamp_id.and_then(|id| stamps.get(&id).cloned()).into();
            let mut attachments = row.attachments();
            if attachments.iter().any(|attachment| attachment.kind == AssetKind::ASSET_KIND_FILE) {
                // Private files are served through short-lived URLs.
                assets::refresh_attachments(core, &mut attachments).await?;
            }
            message.attachments = attachments;
            message.location =
                row.location.clone().and_then(|value| serde_json::from_value::<pb::LocationShare>(value).ok()).into();
            message.link_preview =
                row.link_preview.clone().and_then(|value| serde_json::from_value::<LinkPreview>(value).ok()).into();
            message.reply_to =
                row.reply_to_id.map(|id| reply_reference(id, replied.get(&id), view, &users, &stamps)).into();
            message.forwarded = row.forwarded;
            message.system = row
                .system()
                .map(|system| pb::SystemEvent {
                    r#type: system.r#type.into(),
                    actor: system.actor_id.map(|id| users::pick(&users, id)).into(),
                    targets: system.target_ids.iter().map(|id| users::pick(&users, *id)).collect(),
                    text: system.text,
                    message_id: system.message_id.map(|id| id.to_string()).unwrap_or_default(),
                    notice: system
                        .notice
                        .map(|notice| LocalizedText { ja: notice.ja, en: notice.en, ..Default::default() })
                        .into(),
                    ..Default::default()
                })
                .into();
            message.call = row.call_id.and_then(|id| calls.get(&id)).map(CallRow::summary).into();
            if let Some(pov) = row.pov_request_id.and_then(|id| povs.get(&id)) {
                if row.kind == msg::POV_REQUEST {
                    message.pov_request = pov.to_proto().into();
                } else {
                    message.pov_reply = pov.reply_proto().into();
                }
            }
            message.reactions = reactions.remove(&row.id).unwrap_or_default();
            message.pinned = pinned.contains(&row.id);
            message.read_count = cursors
                .get(&row.conversation_id)
                .map(|cursors| {
                    cursors
                        .iter()
                        .filter(|(user_id, read_seq)| Some(*user_id) != row.sender_id && *read_seq >= row.seq)
                        .count() as i32
                })
                .unwrap_or(0);
            message.edited_at = opt_ts(row.edited_at);
            messages.push(message);
        }
        Ok(messages)
    }
}

/// The quote above a reply. `answered` is `None` when the answered message
/// no longer exists.
fn reply_reference(
    message_id: Uuid,
    answered: Option<&MessageRow>,
    view: &View,
    users: &HashMap<Uuid, UserSummary>,
    stamps: &HashMap<Uuid, StampRef>,
) -> pb::ReplyReference {
    let unavailable =
        pb::ReplyReference { message_id: message_id.to_string(), unavailable: true, ..Default::default() };
    let Some(answered) = answered else { return unavailable };
    if answered.is_deleted() || !view.sees(answered.conversation_id, answered.seq) {
        return pb::ReplyReference { seq: answered.seq, ..unavailable };
    }
    pb::ReplyReference {
        message_id: message_id.to_string(),
        seq: answered.seq,
        sender: answered.sender_id.map(|id| users::pick(users, id)).into(),
        kind: i32::from(answered.kind).into(),
        text: content::snippet(&answered.text, content::REPLY_SNIPPET_CHARS),
        thumbnail_url: thumbnail_of(answered, stamps).unwrap_or_default(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use tokuly_proto::common::{Image, ImageVariant};

    use super::*;

    #[test]
    fn views_hide_messages_at_or_below_their_floor() {
        let conversation = new_id();
        let view = View::event().with_floor(conversation, 5);
        assert!(!view.sees(conversation, 5));
        assert!(view.sees(conversation, 6));
        assert!(view.sees(new_id(), 1), "other conversations have no floor");
    }

    #[test]
    fn thumbnails_prefer_a_small_variant() {
        let variant =
            |size: i32| ImageVariant { size, url: format!("https://media.test/{size}.webp"), ..Default::default() };
        let image = Image {
            url: "https://media.test/full.webp".into(),
            variants: vec![variant(1024), variant(256), variant(96)],
            ..Default::default()
        };
        let attachment = Attachment { image: image.into(), ..Default::default() };
        assert_eq!(attachment_thumbnail(&attachment).as_deref(), Some("https://media.test/256.webp"));

        let only_tiny =
            Image { url: "https://media.test/full.webp".into(), variants: vec![variant(96)], ..Default::default() };
        let attachment = Attachment { image: only_tiny.into(), ..Default::default() };
        assert_eq!(attachment_thumbnail(&attachment).as_deref(), Some("https://media.test/full.webp"));
        assert_eq!(attachment_thumbnail(&Attachment::default()), None);
    }
}
