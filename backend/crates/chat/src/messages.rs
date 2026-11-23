//! Messages: composing and sending, editing, deleting for everyone,
//! paging through a conversation in both directions, searching, forwarding
//! and the shared-media gallery.
use std::time::Duration;

use sqlx::QueryBuilder;
use tokuly_core::{events, page, prelude::*};
use tokuly_media::assets::{self, AssetRow};
use tokuly_proto::{
    chat::{self as pb, list_messages_request::Position},
    common::PageInfo,
    media::{AssetKind, AssetPurpose, AssetStatus},
};

use crate::{
    Chat,
    access::{self, Access, Content, message_not_found},
    content,
    events::{MESSAGE_DELETED, MessageDeleted},
    model::{ConversationRow, MSG, MentionRef, MessageRow, msg, removed, request, role},
    render::View,
    send::{Draft, JOB_LINK_PREVIEW, LinkPreviewJob, message_holder},
    subjects,
};

/// Uploads a chat message may carry.
pub(crate) const CHAT_PURPOSES: &[AssetPurpose] = &[
    AssetPurpose::ASSET_PURPOSE_CHAT_IMAGE,
    AssetPurpose::ASSET_PURPOSE_CHAT_VIDEO,
    AssetPurpose::ASSET_PURPOSE_CHAT_AUDIO,
    AssetPurpose::ASSET_PURPOSE_CHAT_FILE,
];
/// Messages forwarded by one request, and conversations they can go to.
const MAX_FORWARD_MESSAGES: usize = 10;
const MAX_FORWARD_TARGETS: usize = 10;
/// Longest idempotency key of a forward (a per-message suffix is added).
const MAX_FORWARD_CLIENT_ID_LEN: usize = 48;
/// Largest page of messages.
const MAX_MESSAGES_PAGE: i64 = 100;

#[derive(sqlx::FromRow)]
struct MediaRow {
    message_id: Uuid,
    asset_id: Uuid,
    seq: i64,
    position: i16,
    sender_id: Option<Uuid>,
    created_at: Time,
    attachments: serde_json::Value,
}

/// A window of messages with what lies beyond it.
pub(crate) struct MessageWindow {
    /// Oldest first.
    pub rows: Vec<MessageRow>,
    pub has_older: bool,
    pub has_newer: bool,
    /// Where to continue: messages before `older` / after `newer` (both
    /// are sequence numbers, and both exist for an empty window too, so a
    /// client can keep asking for what arrives later).
    pub older: i64,
    pub newer: i64,
}

impl MessageWindow {
    /// `edges` are the cursors of a window without messages.
    fn new(rows: Vec<MessageRow>, has_older: bool, has_newer: bool, edges: (i64, i64)) -> Self {
        let older = rows.first().map_or(edges.0, |row| row.seq);
        let newer = rows.last().map_or(edges.1, |row| row.seq);
        Self { rows, has_older, has_newer, older, newer }
    }
}

impl Chat {
    // ───────────────────────────── composing ─────────────────────────────

    /// Resolves the uploads of a message. Images, voice messages and files
    /// must be processed; a video may still be transcoding (the message is
    /// updated when it finishes).
    pub(crate) async fn resolve_attachments(
        &self,
        user_id: Uuid,
        asset_ids: &[String],
        purposes: &[AssetPurpose],
        max: usize,
    ) -> AppResult<Vec<AssetRow>> {
        ensure!(asset_ids.len() <= max, invalid, "TOO_MANY_ATTACHMENTS", "a message can carry at most {max} uploads");
        let mut resolved: Vec<AssetRow> = Vec::with_capacity(asset_ids.len());
        for asset_id in asset_ids {
            let asset_id = parse_id(asset_id)?;
            if resolved.iter().any(|asset| asset.id == asset_id) {
                continue;
            }
            let asset = assets::require(&self.core, asset_id, user_id, purposes, false).await?;
            let transcoding = asset.has_status(AssetStatus::ASSET_STATUS_PROCESSING)
                && asset.asset_purpose() == AssetPurpose::ASSET_PURPOSE_CHAT_VIDEO;
            ensure!(
                asset.is_ready() || transcoding,
                precondition,
                "ASSET_NOT_READY",
                "the upload is still being processed"
            );
            resolved.push(asset);
        }
        Ok(resolved)
    }

    /// Members mentioned by `@handle` in `text`, in order of appearance, and
    /// whether `@all` addresses everyone (groups only).
    pub(crate) async fn resolve_mentions(
        &self,
        conv: &ConversationRow,
        text: &str,
    ) -> AppResult<(Vec<MentionRef>, bool)> {
        let found = content::parse_mentions(text);
        let everyone = found.everyone && conv.is_group();
        if found.handles.is_empty() {
            return Ok((Vec::new(), everyone));
        }
        let members: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT m.user_id, lower(u.handle::text)
             FROM chat_members m JOIN users u ON u.id = m.user_id
             WHERE m.conversation_id = $1 AND m.request_state = $2 AND u.deleted_at IS NULL AND lower(u.handle::text) = ANY($3)",
        )
        .bind(conv.id)
        .bind(request::ACCEPTED)
        .bind(&found.handles)
        .fetch_all(&self.core.db)
        .await?;
        let mentions = found
            .handles
            .iter()
            .filter_map(|handle| {
                members
                    .iter()
                    .find(|(_, member)| member == handle)
                    .map(|(user_id, _)| MentionRef { user_id: *user_id, handle: handle.clone() })
            })
            .collect();
        Ok((mentions, everyone))
    }

    /// The message a reply answers: it must be a visible, undeleted user
    /// message of the same conversation.
    async fn resolve_reply(&self, access: &Access, message_id: &str) -> AppResult<(Uuid, Option<Uuid>)> {
        let not_found = || AppError::not_found("REPLY_NOT_FOUND", "the message you are replying to does not exist");
        let message_id = parse_id(message_id)?;
        let answered = access::find_message(&self.core, message_id).await?.ok_or_else(not_found)?;
        let usable = answered.conversation_id == access.conv.id
            && access.sees(answered.seq)
            && !answered.is_deleted()
            && answered.kind != msg::SYSTEM;
        if !usable {
            return Err(not_found());
        }
        Ok((answered.id, answered.sender_id))
    }

    /// Validates the content of a `SendMessage` request and turns it into a draft.
    async fn compose(&self, access: &Access, req: &pb::SendMessageRequest) -> AppResult<Draft> {
        let user_id = access.me.user_id;
        let text = content::normalize_text(&req.text)?;
        let has_stamp = !req.stamp_id.is_empty();
        let has_media = !req.attachment_asset_ids.is_empty();
        let location = req.location.as_option();
        let extras = usize::from(has_stamp) + usize::from(has_media) + usize::from(location.is_some());
        ensure!(
            extras <= 1 && (text.is_empty() || !(has_stamp || location.is_some())),
            invalid,
            "MESSAGE_CONTENT_CONFLICT",
            "a message is text, a stamp, media with an optional caption, or a location"
        );
        ensure!(extras == 1 || !text.is_empty(), invalid, "MESSAGE_EMPTY", "the message is empty");

        let mut draft = if has_stamp {
            let stamp_id = parse_id(&req.stamp_id)?;
            tokuly_stamp::ensure_usable(&self.core, user_id, stamp_id).await?;
            Draft { stamp_id: Some(stamp_id), ..Draft::from_user(msg::STAMP, user_id) }
        } else if let Some(location) = location {
            Draft { location: Some(validate_location(location)?), ..Draft::from_user(msg::LOCATION, user_id) }
        } else {
            let attachments = self
                .resolve_attachments(user_id, &req.attachment_asset_ids, CHAT_PURPOSES, content::MAX_ATTACHMENTS)
                .await?;
            let message_kind = if attachments.is_empty() { msg::TEXT } else { msg::MEDIA };
            let (mentions, mention_all) = self.resolve_mentions(&access.conv, &text).await?;
            let link_url = content::first_url(&text);
            Draft { text, mentions, mention_all, link_url, attachments, ..Draft::from_user(message_kind, user_id) }
        };
        if !req.reply_to_message_id.is_empty() {
            draft.reply_to = Some(self.resolve_reply(access, &req.reply_to_message_id).await?);
        }
        Ok(draft)
    }

    async fn find_by_client_id(
        &self,
        conversation_id: Uuid,
        sender_id: Uuid,
        client_id: &str,
    ) -> AppResult<Option<MessageRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {MSG} FROM chat_messages x WHERE x.conversation_id = $1 AND x.sender_id = $2 AND x.client_id = $3"
        )))
        .bind(conversation_id)
        .bind(sender_id)
        .bind(client_id)
        .fetch_optional(&self.core.db)
        .await?)
    }

    async fn limit_sending(&self, user_id: Uuid) -> AppResult<()> {
        self.core
            .cache
            .rate_limit(&format!("chat-send:{user_id}"), self.cfg.messages_per_minute, Duration::from_secs(60))
            .await
    }

    // ───────────────────────────── sending ─────────────────────────────

    /// Sends a message. Returns the stored message and whether it had been
    /// sent before under the same `client_id`.
    pub(crate) async fn send_message(
        &self,
        user_id: Uuid,
        req: &pb::SendMessageRequest,
    ) -> AppResult<(Access, MessageRow, bool)> {
        let conversation_id = parse_id(&req.conversation_id)?;
        content::validate_client_id(&req.client_id, content::MAX_CLIENT_ID_LEN)?;
        let access = self.access(user_id, conversation_id).await?;
        // A retry gets the stored message back without being judged again.
        if let Some(existing) = self.find_by_client_id(conversation_id, user_id, &req.client_id).await? {
            return Ok((access, existing, true));
        }
        let mut draft = self.compose(&access, req).await?;
        let plain = matches!(draft.kind, msg::TEXT | msg::STAMP);
        let permit = self.ensure_can_send(&access, if plain { Content::Plain } else { Content::Rich }).await?;
        self.limit_sending(user_id).await?;
        draft.client_id = Some(req.client_id.clone());
        draft.is_request = permit.is_request;
        let draft = self.with_notifications(&access.conv, draft).await?;
        let posted = self.post_and_announce(conversation_id, draft).await?;
        Ok((access, posted.row, posted.duplicate))
    }

    /// "First hello": waves at a user the caller never exchanged a message with.
    pub(crate) async fn send_hello(&self, user_id: Uuid, other: Uuid) -> AppResult<(Access, MessageRow)> {
        let access = self.open_direct_as(user_id, other).await?;
        let not_first = || AppError::precondition("HELLO_NOT_FIRST", "you two have already talked");
        if access.conv.last_seq > 0 {
            return Err(not_first());
        }
        let permit = self.ensure_can_send(&access, Content::Plain).await?;
        self.limit_sending(user_id).await?;
        let draft = Draft { is_request: permit.is_request, ..Draft::from_user(msg::HELLO, user_id) };
        let draft = self.with_notifications(&access.conv, draft).await?;
        let mut tx = self.core.db.begin().await?;
        let mut conv = access::lock_conversation(&mut tx, access.conv.id).await?;
        if conv.last_seq > 0 {
            return Err(not_first());
        }
        let posted = self.post(&mut tx, &mut conv, draft).await?;
        tx.commit().await?;
        self.announce(&posted).await;
        Ok((self.access(user_id, access.conv.id).await?, posted.row))
    }

    /// Posts a text message as `sender` on the server's own initiative (a
    /// thank-you after a gift, the demo simulator). The sender must be a
    /// member; message requests and rate limits do not apply, but nobody can
    /// post into a read-only conversation. Returns the message ID.
    pub async fn post_text(&self, conversation_id: Uuid, sender: Uuid, text: &str) -> AppResult<Uuid> {
        let access = self.access(sender, conversation_id).await?;
        let text = content::normalize_text(text)?;
        ensure!(!text.is_empty(), invalid, "MESSAGE_EMPTY", "the message is empty");
        self.ensure_writable(&access).await?;
        let (mentions, mention_all) = self.resolve_mentions(&access.conv, &text).await?;
        let link_url = content::first_url(&text);
        let draft = Draft { text, mentions, mention_all, link_url, ..Draft::from_user(msg::TEXT, sender) };
        let draft = self.with_notifications(&access.conv, draft).await?;
        Ok(self.post_and_announce(conversation_id, draft).await?.row.id)
    }

    // ───────────────────────────── editing and deleting ─────────────────────────────

    pub(crate) async fn edit_message(
        &self,
        user_id: Uuid,
        message_id: Uuid,
        text: &str,
    ) -> AppResult<(Access, MessageRow)> {
        let (access, row) = self.message_access(user_id, message_id).await?;
        ensure!(row.sender_id == Some(user_id), denied, "NOT_ALLOWED", "you can only edit your own messages");
        let editable =
            !row.is_deleted() && !row.forwarded && matches!(row.kind, msg::TEXT | msg::MEDIA) && !row.is_pov_reply();
        ensure!(editable, precondition, "MESSAGE_NOT_EDITABLE", "this message cannot be edited");
        let age = (now() - row.created_at).num_seconds();
        ensure!(
            age <= self.cfg.edit_window_seconds,
            precondition,
            "EDIT_WINDOW_CLOSED",
            "this message is too old to be edited"
        );
        let text = content::normalize_text(text)?;
        ensure!(!text.is_empty() || row.kind == msg::MEDIA, invalid, "MESSAGE_EMPTY", "the message is empty");
        self.ensure_writable(&access).await?;
        if text == row.text {
            return Ok((access, row));
        }
        // An edit reaches every member like a message does.
        self.limit_sending(user_id).await?;
        let (mentions, mention_all) = self.resolve_mentions(&access.conv, &text).await?;
        let link_url = content::first_url(&text);
        let mentions =
            serde_json::to_value(&mentions).map_err(|e| AppError::internal(anyhow::anyhow!("mentions encode: {e}")))?;
        let mut tx = self.core.db.begin().await?;
        // What it said before stays reportable (see `subjects`), for a
        // bounded number of edits. The row lock orders concurrent edits.
        sqlx::query("SELECT 1 FROM chat_messages WHERE id = $1 FOR UPDATE")
            .bind(message_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO chat_removed_texts (message_id, text, reason)
             SELECT id, text, $2 FROM chat_messages
             WHERE id = $1 AND deleted_at IS NULL AND text <> ''
               AND (SELECT count(*) FROM chat_removed_texts WHERE message_id = $1 AND reason = $2) < $3",
        )
        .bind(message_id)
        .bind(removed::EDITED)
        .bind(subjects::MAX_EDITED_TEXTS)
        .execute(&mut *tx)
        .await?;
        // The preview is kept when the link is the same, and fetched again otherwise.
        let updated: MessageRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE chat_messages x SET
                 text = $2, mentions = $3, mention_all = $4, edited_at = now(), link_url = $5,
                 link_preview = CASE WHEN $5 IS NOT DISTINCT FROM x.link_url THEN x.link_preview END,
                 link_checked_at = CASE WHEN $5 IS NOT DISTINCT FROM x.link_url THEN x.link_checked_at END
             WHERE x.id = $1 AND x.deleted_at IS NULL
             RETURNING {MSG}"
        )))
        .bind(message_id)
        .bind(&text)
        .bind(mentions)
        .bind(mention_all)
        .bind(&link_url)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(message_not_found)?;
        tx.commit().await?;
        self.push_message_updated(&updated, pb::MessageUpdateReason::MESSAGE_UPDATE_REASON_EDITED).await;
        if updated.link_url.is_some() && updated.link_checked_at.is_none() {
            let job = LinkPreviewJob { message_id };
            // Edits of one message may each need a fetch: no deduplication by message.
            if let Err(e) = self.core.jobs.enqueue(JOB_LINK_PREVIEW, &job).await {
                tracing::warn!(%message_id, error = %e, "link preview job could not be queued");
            }
        }
        Ok((access, updated))
    }

    pub(crate) async fn delete_message(&self, user_id: Uuid, message_id: Uuid) -> AppResult<(Access, MessageRow)> {
        let (access, row) = self.message_access(user_id, message_id).await?;
        let own = row.sender_id == Some(user_id);
        // In a group, managers delete the messages of those below them, as
        // with removing members: the owner anyone's, admins plain members'
        // (and of people no longer in the group).
        let outranks = access.conv.is_group() && access.me.manages() && {
            let author_role = match row.sender_id {
                Some(author) => access::find_member(&self.core, row.conversation_id, author).await?.map(|m| m.role),
                None => None,
            };
            access.me.role > author_role.unwrap_or(role::MEMBER)
        };
        ensure!(own || outranks, denied, "NOT_ALLOWED", "you can only delete your own messages");
        ensure!(
            !matches!(row.kind, msg::SYSTEM | msg::CALL | msg::POV_REQUEST),
            precondition,
            "MESSAGE_NOT_DELETABLE",
            "this message cannot be deleted"
        );
        if row.is_deleted() {
            return Ok((access, row));
        }
        let tombstone = self.tombstone(message_id, Some(user_id), false).await?.unwrap_or(row);
        Ok((access, tombstone))
    }

    /// Deletes a message for everyone: its content is erased, its uploads
    /// released, its reactions and pin removed, and members who had not read
    /// it yet lose it from their unread count. Returns the tombstone, or
    /// `None` when the message does not exist or was deleted before.
    pub(crate) async fn tombstone(
        &self,
        message_id: Uuid,
        deleted_by: Option<Uuid>,
        moderated: bool,
    ) -> AppResult<Option<MessageRow>> {
        let mut tx = self.core.db.begin().await?;
        let row: Option<MessageRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {MSG} FROM chat_messages x WHERE x.id = $1 FOR UPDATE"
        )))
        .bind(message_id)
        .fetch_optional(&mut *tx)
        .await?;
        // Unsending must not take the recipients' way to report what they
        // received: what it said is kept for that (see `subjects`). A
        // takedown (moderation, a removed upload) keeps nothing, and forgets
        // what was kept before: the report it acted on has the text.
        let takedown = moderated || deleted_by.is_none();
        if takedown {
            sqlx::query("DELETE FROM chat_removed_texts WHERE message_id = $1")
                .bind(message_id)
                .execute(&mut *tx)
                .await?;
        }
        let Some(row) = row.filter(|row| !row.is_deleted()) else {
            tx.commit().await?;
            return Ok(None);
        };
        if !takedown && row.sender_id.is_some() {
            sqlx::query("INSERT INTO chat_removed_texts (message_id, text, reason) VALUES ($1, $2, $3)")
                .bind(message_id)
                .bind(subjects::reported_text(&row))
                .bind(removed::DELETED)
                .execute(&mut *tx)
                .await?;
        }
        if row.counts_as_unread() {
            let mentioned: Vec<Uuid> = row.mentions().iter().map(|mention| mention.user_id).collect();
            sqlx::query(
                "UPDATE chat_members m SET
                     unread_count = GREATEST(m.unread_count - 1, 0),
                     unread_mention_count = GREATEST(m.unread_mention_count - CASE WHEN $4 OR m.user_id = ANY($5) THEN 1 ELSE 0 END, 0)
                 WHERE m.conversation_id = $1 AND m.user_id IS DISTINCT FROM $2 AND m.read_seq < $3 AND m.history_from_seq < $3",
            )
            .bind(row.conversation_id)
            .bind(row.sender_id)
            .bind(row.seq)
            .bind(row.mention_all)
            .bind(&mentioned)
            .execute(&mut *tx)
            .await?;
        }
        let tombstone: MessageRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE chat_messages x SET
                 deleted_at = now(), deleted_by = $2, text = '', mentions = '[]', mention_all = false, stamp_id = NULL,
                 attachments = '[]', location = NULL, link_url = NULL, link_preview = NULL, link_checked_at = NULL, reply_to_id = NULL
             WHERE x.id = $1
             RETURNING {MSG}"
        )))
        .bind(message_id)
        .bind(deleted_by)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM chat_message_assets WHERE message_id = $1").bind(message_id).execute(&mut *tx).await?;
        assets::detach_holder(&mut *tx, &message_holder(message_id)).await?;
        sqlx::query("DELETE FROM chat_reactions WHERE message_id = $1").bind(message_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM chat_pins WHERE message_id = $1").bind(message_id).execute(&mut *tx).await?;
        // A deleted POV answer is gone like an expired one.
        let pov_request_message: Option<Uuid> = if row.is_pov_reply() {
            sqlx::query_scalar(
                "UPDATE chat_pov_requests SET reply_expired_at = COALESCE(reply_expired_at, now())
                 WHERE id = $1 AND reply_message_id = $2 RETURNING request_message_id",
            )
            .bind(row.pov_request_id)
            .bind(message_id)
            .fetch_optional(&mut *tx)
            .await?
            .flatten()
        } else {
            None
        };
        let deleted = MessageDeleted {
            message_id,
            conversation_id: row.conversation_id,
            sender_id: row.sender_id,
            deleted_by,
            moderated,
        };
        events::emit(&mut *tx, MESSAGE_DELETED, deleted_by, &deleted).await?;
        tx.commit().await?;

        self.push_message_updated(&tombstone, pb::MessageUpdateReason::MESSAGE_UPDATE_REASON_DELETED).await;
        if let Some(request_message_id) = pov_request_message {
            self.push_message_updated_by_id(request_message_id, pb::MessageUpdateReason::MESSAGE_UPDATE_REASON_POV)
                .await;
        }
        Ok(Some(tombstone))
    }

    // ───────────────────────────── reading ─────────────────────────────

    async fn messages_before(&self, access: &Access, before: i64, limit: i64) -> AppResult<Vec<MessageRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {MSG} FROM chat_messages x WHERE x.conversation_id = $1 AND x.seq > $2 AND x.seq < $3 ORDER BY x.seq DESC LIMIT $4"
        )))
        .bind(access.conv.id)
        .bind(access.me.history_from_seq)
        .bind(before)
        .bind(limit)
        .fetch_all(&self.core.db)
        .await?)
    }

    async fn messages_from(&self, access: &Access, from: i64, limit: i64) -> AppResult<Vec<MessageRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {MSG} FROM chat_messages x WHERE x.conversation_id = $1 AND x.seq > $2 AND x.seq >= $3 ORDER BY x.seq LIMIT $4"
        )))
        .bind(access.conv.id)
        .bind(access.me.history_from_seq)
        .bind(from)
        .bind(limit)
        .fetch_all(&self.core.db)
        .await?)
    }

    /// A window of the conversation at `position`; without one, the newest messages.
    pub(crate) async fn message_window(
        &self,
        access: &Access,
        position: Option<&Position>,
        limit: i32,
    ) -> AppResult<MessageWindow> {
        let limit = if limit <= 0 { Page::DEFAULT_LIMIT } else { i64::from(limit).min(MAX_MESSAGES_PAGE) };
        // A window centred on a message: `limit` messages in all, the centre
        // sitting just past the middle.
        let around = |centre: i64| async move {
            let older_limit = limit / 2;
            let mut older = self.messages_before(access, centre, older_limit + 1).await?;
            let has_older = older.len() as i64 > older_limit;
            older.truncate(older_limit as usize);
            older.reverse();
            let newer_limit = limit - older_limit;
            let mut newer = self.messages_from(access, centre, newer_limit + 1).await?;
            let has_newer = newer.len() as i64 > newer_limit;
            newer.truncate(newer_limit as usize);
            older.extend(newer);
            AppResult::Ok(MessageWindow::new(older, has_older, has_newer, (centre, centre.saturating_sub(1))))
        };
        let first_unread = access.me.read_seq.max(access.me.history_from_seq) + 1;
        match position {
            Some(Position::Before(cursor)) => {
                let before: i64 = page::decode_cursor(cursor)?;
                let mut rows = self.messages_before(access, before, limit + 1).await?;
                let has_older = rows.len() as i64 > limit;
                rows.truncate(limit as usize);
                rows.reverse();
                Ok(MessageWindow::new(
                    rows,
                    has_older,
                    before <= access.conv.last_seq,
                    (before, before.saturating_sub(1)),
                ))
            }
            Some(Position::After(cursor)) => {
                let after: i64 = page::decode_cursor(cursor)?;
                let mut rows = self.messages_from(access, after.saturating_add(1), limit + 1).await?;
                let has_newer = rows.len() as i64 > limit;
                rows.truncate(limit as usize);
                Ok(MessageWindow::new(
                    rows,
                    after > access.me.history_from_seq,
                    has_newer,
                    (after.saturating_add(1), after),
                ))
            }
            Some(Position::AroundMessageId(message_id)) => {
                let centre = access::find_message(&self.core, parse_id(message_id)?)
                    .await?
                    .filter(|message| message.conversation_id == access.conv.id && access.sees(message.seq))
                    .ok_or_else(message_not_found)?;
                around(centre.seq).await
            }
            Some(Position::AroundUnread(true)) if first_unread <= access.conv.last_seq => around(first_unread).await,
            _ => {
                let mut rows = self.messages_before(access, i64::MAX, limit + 1).await?;
                let has_older = rows.len() as i64 > limit;
                rows.truncate(limit as usize);
                rows.reverse();
                let floor = access.me.history_from_seq.max(access.conv.last_seq);
                Ok(MessageWindow::new(rows, has_older, false, (floor.saturating_add(1), floor)))
            }
        }
    }

    /// Messages of a conversation whose text contains `query`, newest first.
    pub(crate) async fn search_messages(
        &self,
        access: &Access,
        query: &str,
        page: &Page,
    ) -> AppResult<(Vec<MessageRow>, PageInfo)> {
        let pattern = content::search_pattern(query)?;
        self.core.cache.rate_limit(&format!("chat-search:{}", access.me.user_id), 60, Duration::from_secs(60)).await?;
        let before: Option<i64> = page.decode()?;
        let mut rows: Vec<MessageRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {MSG} FROM chat_messages x
             WHERE x.conversation_id = $1 AND x.seq > $2 AND ($3::bigint IS NULL OR x.seq < $3)
               AND x.deleted_at IS NULL AND x.text <> '' AND x.text ILIKE $4
             ORDER BY x.seq DESC LIMIT $5"
        )))
        .bind(access.conv.id)
        .bind(access.me.history_from_seq)
        .bind(before)
        .bind(pattern)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| row.seq);
        Ok((rows, info))
    }

    /// The uploads shared in a conversation, newest first.
    pub(crate) async fn shared_media(
        &self,
        access: &Access,
        filter: pb::SharedMediaFilter,
        page: &Page,
    ) -> AppResult<(Vec<pb::SharedMedia>, PageInfo)> {
        let kinds: &[AssetKind] = match filter {
            pb::SharedMediaFilter::SHARED_MEDIA_FILTER_VISUAL => {
                &[AssetKind::ASSET_KIND_IMAGE, AssetKind::ASSET_KIND_VIDEO, AssetKind::ASSET_KIND_ANIMATION]
            }
            pb::SharedMediaFilter::SHARED_MEDIA_FILTER_AUDIO => &[AssetKind::ASSET_KIND_AUDIO],
            pb::SharedMediaFilter::SHARED_MEDIA_FILTER_FILES => &[AssetKind::ASSET_KIND_FILE],
            _ => &[],
        };
        let cursor: Option<(i64, i16)> = page.decode()?;
        let mut query = QueryBuilder::new(
            "SELECT a.message_id, a.asset_id, a.seq, a.position, x.sender_id, x.created_at, x.attachments
             FROM chat_message_assets a JOIN chat_messages x ON x.id = a.message_id
             WHERE x.deleted_at IS NULL AND a.conversation_id = ",
        );
        query.push_bind(access.conv.id).push(" AND a.seq > ").push_bind(access.me.history_from_seq);
        if !kinds.is_empty() {
            query
                .push(" AND a.kind = ANY(")
                .push_bind(kinds.iter().map(|kind| *kind as i16).collect::<Vec<i16>>())
                .push(")");
        }
        if let Some((seq, position)) = cursor {
            query.push(" AND (a.seq, a.position) < (").push_bind(seq).push(", ").push_bind(position).push(")");
        }
        query.push(" ORDER BY a.seq DESC, a.position DESC LIMIT ").push_bind(page.fetch());
        let mut rows: Vec<MediaRow> = query.build_query_as().fetch_all(&self.core.db).await?;
        let info = page.finish(&mut rows, |row| (row.seq, row.position));

        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let attachments: Vec<tokuly_proto::media::Attachment> =
                serde_json::from_value(row.attachments).unwrap_or_default();
            let asset_id = row.asset_id.to_string();
            let Some(attachment) = attachments.into_iter().find(|attachment| attachment.asset_id == asset_id) else {
                continue;
            };
            let mut attachment = [attachment];
            assets::refresh_attachments(&self.core, &mut attachment).await?;
            let [attachment] = attachment;
            items.push(pb::SharedMedia {
                message_id: row.message_id.to_string(),
                seq: row.seq,
                sender_id: row.sender_id.map(|id| id.to_string()).unwrap_or_default(),
                attachment: attachment.into(),
                created_at: ts(row.created_at),
                ..Default::default()
            });
        }
        Ok((items, info))
    }

    // ───────────────────────────── forwarding ─────────────────────────────

    /// Copies messages the caller can see into conversations the caller
    /// can send to. Returns the new messages with their conversations.
    pub(crate) async fn forward_messages(
        &self,
        user_id: Uuid,
        message_ids: &[Uuid],
        conversation_ids: &[Uuid],
        client_id: &str,
    ) -> AppResult<Vec<(Access, MessageRow)>> {
        let core = &self.core;
        content::validate_client_id(client_id, MAX_FORWARD_CLIENT_ID_LEN)?;
        let mut sources: Vec<Uuid> = Vec::new();
        for id in message_ids {
            if !sources.contains(id) {
                sources.push(*id);
            }
        }
        let mut targets: Vec<Uuid> = Vec::new();
        for id in conversation_ids {
            if !targets.contains(id) {
                targets.push(*id);
            }
        }
        ensure!(
            !sources.is_empty() && !targets.is_empty(),
            invalid,
            "NOTHING_TO_FORWARD",
            "name the messages to forward and where to"
        );
        ensure!(
            sources.len() <= MAX_FORWARD_MESSAGES,
            invalid,
            "TOO_MANY_MESSAGES",
            "at most {MAX_FORWARD_MESSAGES} messages can be forwarded at once"
        );
        ensure!(
            targets.len() <= MAX_FORWARD_TARGETS,
            invalid,
            "TOO_MANY_CONVERSATIONS",
            "messages can be forwarded to at most {MAX_FORWARD_TARGETS} conversations at once"
        );

        // The copies, in the order the originals were sent.
        let mut originals: Vec<MessageRow> = Vec::with_capacity(sources.len());
        for message_id in &sources {
            originals.push(self.message_access(user_id, *message_id).await?.1);
        }
        originals.sort_by_key(|row| (row.created_at, row.seq));
        let mut drafts: Vec<Draft> = Vec::with_capacity(originals.len());
        let mut rich = false;
        for (index, original) in originals.iter().enumerate() {
            let forwardable = !original.is_deleted()
                && !original.is_pov_reply()
                && matches!(original.kind, msg::TEXT | msg::STAMP | msg::MEDIA | msg::LOCATION);
            ensure!(forwardable, precondition, "MESSAGE_NOT_FORWARDABLE", "this message cannot be forwarded");
            let mut draft = Draft {
                client_id: Some(format!("fwd:{client_id}:{index}")),
                text: original.text.clone(),
                forwarded: true,
                link_url: original.link_url.clone(),
                link_preview: original.link_preview.clone(),
                location: original.location.clone().and_then(|value| serde_json::from_value(value).ok()),
                ..Draft::from_user(original.kind, user_id)
            };
            if let Some(stamp_id) = original.stamp_id {
                tokuly_stamp::ensure_usable(core, user_id, stamp_id).await?;
                draft.stamp_id = Some(stamp_id);
            }
            if original.kind == msg::MEDIA {
                // The uploads as they are now; ones deleted since are left out.
                let asset_ids: Vec<Uuid> = original
                    .attachments()
                    .iter()
                    .filter_map(|attachment| Uuid::parse_str(&attachment.asset_id).ok())
                    .collect();
                draft.attachments = assets::find_many(core, &asset_ids)
                    .await?
                    .into_iter()
                    .filter(|asset| !asset.has_status(AssetStatus::ASSET_STATUS_FAILED))
                    .collect();
                ensure!(
                    !draft.attachments.is_empty() || !draft.text.is_empty(),
                    precondition,
                    "MESSAGE_NOT_FORWARDABLE",
                    "nothing of this message is left to forward"
                );
                if draft.attachments.is_empty() {
                    draft.kind = msg::TEXT;
                }
            }
            rich |= !matches!(draft.kind, msg::TEXT | msg::STAMP);
            drafts.push(draft);
        }

        let mut destinations: Vec<(Access, bool)> = Vec::with_capacity(targets.len());
        for conversation_id in &targets {
            let access = self.access(user_id, *conversation_id).await?;
            let permit = self.ensure_can_send(&access, if rich { Content::Rich } else { Content::Plain }).await?;
            // A request allowance has to cover every message of the batch.
            let allowance = self.cfg.request_message_limit - access.me.request_messages;
            if permit.is_request && (drafts.len() as i32) > allowance {
                return Err(access::request_pending());
            }
            destinations.push((access, permit.is_request));
        }
        for _ in 0..drafts.len() * destinations.len() {
            self.limit_sending(user_id).await?;
        }

        let mut forwarded = Vec::with_capacity(drafts.len() * destinations.len());
        for (access, is_request) in destinations {
            // One look at who is to be notified in this conversation serves every copy.
            let mut copies: Vec<Draft> = Vec::with_capacity(drafts.len());
            for draft in &drafts {
                let copy = Draft { is_request, ..draft.clone() };
                copies.push(match copies.first() {
                    Some(first) => Draft { notify: first.notify.clone(), ..copy },
                    None => self.with_notifications(&access.conv, copy).await?,
                });
            }
            let mut tx = core.db.begin().await?;
            let mut conv = access::lock_conversation(&mut tx, access.conv.id).await?;
            let mut posted = Vec::with_capacity(copies.len());
            for copy in copies {
                posted.push(self.post(&mut tx, &mut conv, copy).await?);
            }
            tx.commit().await?;
            for posted in posted {
                self.announce(&posted).await;
                forwarded.push((access.clone(), posted.row));
            }
        }
        Ok(forwarded)
    }

    /// Renders messages of one conversation for its member.
    pub(crate) async fn render_for(&self, access: &Access, rows: &[MessageRow]) -> AppResult<Vec<pb::Message>> {
        self.render_messages(rows, &View::member(&access.me)).await
    }
}

/// Validates a location share and trims its labels.
fn validate_location(location: &pb::LocationShare) -> AppResult<pb::LocationShare> {
    let invalid = |message: &'static str| AppError::invalid("LOCATION_INVALID", message);
    let position = location.position.as_option().ok_or_else(|| invalid("a location needs a position"))?;
    let on_earth = position.lat.is_finite()
        && position.lng.is_finite()
        && (-90.0..=90.0).contains(&position.lat)
        && (-180.0..=180.0).contains(&position.lng);
    if !on_earth {
        return Err(invalid("the position is not a valid coordinate"));
    }
    let name = location.name.trim();
    let address = location.address.trim();
    let clean = |text: &str, max: usize| text.chars().count() <= max && !text.chars().any(char::is_control);
    if !clean(name, 100) || !clean(address, 200) {
        return Err(invalid("the place name or address is too long"));
    }
    Ok(pb::LocationShare {
        position: position.clone().into(),
        name: name.to_owned(),
        address: address.to_owned(),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use tokuly_proto::common::LatLng;

    use super::*;

    fn location(lat: f64, lng: f64, name: &str) -> pb::LocationShare {
        pb::LocationShare {
            position: LatLng { lat, lng, ..Default::default() }.into(),
            name: name.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn locations_must_be_on_earth_and_briefly_labelled() {
        let shibuya = validate_location(&location(35.658, 139.7016, "  渋谷駅 ")).unwrap();
        assert_eq!(shibuya.name, "渋谷駅");
        assert!(validate_location(&location(91.0, 0.0, "")).is_err());
        assert!(validate_location(&location(0.0, -181.0, "")).is_err());
        assert!(validate_location(&location(f64::NAN, 0.0, "")).is_err());
        assert!(validate_location(&location(0.0, 0.0, &"あ".repeat(101))).is_err());
        assert_eq!(validate_location(&pb::LocationShare::default()).unwrap_err().reason, "LOCATION_INVALID");
    }
}
