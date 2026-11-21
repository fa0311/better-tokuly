//! POV requests (Bump): "show me where you are right now".
//!
//! A friend asks in a direct conversation; the request appears there as a
//! `POV_REQUEST` message and can be answered for two hours with photos or
//! videos taken on the spot. One request per conversation is open at a
//! time. The answer is a media message that disappears after 24 hours
//! unless the requester saves it, and the person who answered is told when
//! that happens. Expiry runs in a worker ([`Chat::expire_povs`]).
use std::time::Duration;

use sqlx::{Postgres, Transaction};
use tokuly_core::{outbound, prelude::*};
use tokuly_media::assets;
use tokuly_proto::{chat as pb, media::AssetPurpose};
use tokuly_social::graph;

use crate::{
    Chat,
    access::{self, Access, Content},
    content,
    model::{MessageRow, POV, PovRow, msg, pov},
    notices::{self, PovNotice},
    send::{Draft, message_holder},
};

/// Longest note of a request, in characters.
const MAX_NOTE_CHARS: usize = 200;
/// Photos/videos one answer may carry.
const MAX_POV_ATTACHMENTS: usize = 4;
const POV_PURPOSES: &[AssetPurpose] = &[AssetPurpose::ASSET_PURPOSE_CHAT_IMAGE, AssetPurpose::ASSET_PURPOSE_CHAT_VIDEO];

fn pov_not_found() -> AppError {
    AppError::not_found("POV_NOT_FOUND", "POV request not found")
}

fn pov_not_open() -> AppError {
    AppError::precondition("POV_NOT_OPEN", "this POV request can no longer be answered")
}

fn not_yours() -> AppError {
    AppError::denied("NOT_ALLOWED", "this is for the other side of the POV request to do")
}

async fn lock_pov(tx: &mut Transaction<'_, Postgres>, request_id: Uuid) -> AppResult<PovRow> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {POV} FROM chat_pov_requests p WHERE p.id = $1 FOR UPDATE")))
        .bind(request_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(pov_not_found)
}

impl Chat {
    async fn find_pov(&self, request_id: Uuid) -> AppResult<Option<PovRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {POV} FROM chat_pov_requests p WHERE p.id = $1")))
            .bind(request_id)
            .fetch_optional(&self.core.db)
            .await?)
    }

    /// A POV request of a conversation the caller is a member of.
    async fn pov_access(&self, user_id: Uuid, request_id: Uuid) -> AppResult<(Access, PovRow)> {
        let request = self.find_pov(request_id).await?.ok_or_else(pov_not_found)?;
        let access = self.access(user_id, request.conversation_id).await.map_err(|_| pov_not_found())?;
        Ok((access, request))
    }

    /// The request of a conversation that is waiting for an answer, if any.
    pub(crate) async fn open_pov(&self, conversation_id: Uuid) -> AppResult<Option<PovRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {POV} FROM chat_pov_requests p WHERE p.conversation_id = $1 AND p.state = $2 AND p.expires_at > now()"
        )))
        .bind(conversation_id)
        .bind(pov::OPEN)
        .fetch_optional(&self.core.db)
        .await?)
    }

    /// Tells a member about a POV event unless they are looking at the conversation.
    async fn notify_pov(&self, tx: &mut Transaction<'_, Postgres>, recipient: Uuid, notice: PovNotice, actor: Uuid, request: &PovRow) -> AppResult<()> {
        if self.present_users(request.conversation_id).await.unwrap_or_default().contains(&recipient) {
            return Ok(());
        }
        let muted: Option<(bool, Option<Time>)> = sqlx::query_as("SELECT muted, muted_until FROM chat_members WHERE conversation_id = $1 AND user_id = $2")
            .bind(request.conversation_id)
            .bind(recipient)
            .fetch_optional(&mut **tx)
            .await?;
        if muted.is_none_or(|(muted, until)| crate::model::muted_now(muted, until)) {
            return Ok(());
        }
        let summary = tokuly_identity::summary(&self.core, actor).await?;
        outbound::notify(&mut **tx, &notices::pov(recipient, notice, &summary, actor, request.conversation_id, request.id)).await
    }

    pub(crate) async fn request_pov(&self, user_id: Uuid, conversation_id: Uuid, note: &str) -> AppResult<(Access, PovRow, MessageRow)> {
        let core = &self.core;
        let access = self.access(user_id, conversation_id).await?;
        ensure!(access.conv.is_direct(), precondition, "NOT_A_DIRECT_CONVERSATION", "POV requests are for direct conversations");
        let note = note.trim();
        ensure!(
            note.chars().count() <= MAX_NOTE_CHARS && !note.chars().any(|c| c.is_control() && c != '\n'),
            invalid,
            "NOTE_TOO_LONG",
            "the note can be at most {MAX_NOTE_CHARS} characters"
        );
        let other = access.other_user().ok_or_else(access::read_only)?;
        self.ensure_writable(&access).await?;
        ensure!(graph::are_friends(core, user_id, other).await?, precondition, "NOT_FRIENDS", "you can only ask friends for their POV");
        // Friends never talk through a message request; this settles one left over.
        self.ensure_can_send(&access, Content::Rich).await?;
        core.cache.rate_limit(&format!("chat-pov:{user_id}"), 30, Duration::from_secs(3600)).await?;

        let mut tx = core.db.begin().await?;
        let mut conv = access::lock_conversation(&mut tx, conversation_id).await?;
        // A request whose time ran out but which the worker has not reached yet.
        let stale: Vec<Option<Uuid>> = sqlx::query_scalar(
            "UPDATE chat_pov_requests SET state = $2 WHERE conversation_id = $1 AND state = $3 AND expires_at <= now() RETURNING request_message_id",
        )
        .bind(conversation_id)
        .bind(pov::EXPIRED)
        .bind(pov::OPEN)
        .fetch_all(&mut *tx)
        .await?;
        let open: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM chat_pov_requests WHERE conversation_id = $1 AND state = $2)")
            .bind(conversation_id)
            .bind(pov::OPEN)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(!open, precondition, "POV_ALREADY_OPEN", "there is already a POV request waiting in this conversation");

        let request_id = new_id();
        sqlx::query(
            "INSERT INTO chat_pov_requests (id, conversation_id, requester_id, target_id, note, expires_at)
             VALUES ($1, $2, $3, $4, $5, now() + make_interval(secs => $6))",
        )
        .bind(request_id)
        .bind(conversation_id)
        .bind(user_id)
        .bind(other)
        .bind(note)
        .bind(self.cfg.pov_answer_seconds as f64)
        .execute(&mut *tx)
        .await?;
        let draft = Draft { pov_request_id: Some(request_id), ..Draft::from_user(msg::POV_REQUEST, user_id) }.silent();
        let posted = self.post(&mut tx, &mut conv, draft).await?;
        let request: PovRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE chat_pov_requests p SET request_message_id = $2 WHERE p.id = $1 RETURNING {POV}"
        )))
        .bind(request_id)
        .bind(posted.row.id)
        .fetch_one(&mut *tx)
        .await?;
        self.notify_pov(&mut tx, other, PovNotice::Requested, user_id, &request).await?;
        tx.commit().await?;

        for message_id in stale.into_iter().flatten() {
            self.push_message_updated_by_id(message_id, pb::MessageUpdateReason::MESSAGE_UPDATE_REASON_POV).await;
        }
        self.announce(&posted).await;
        Ok((access, request, posted.row))
    }

    pub(crate) async fn answer_pov(&self, user_id: Uuid, request_id: Uuid, asset_ids: &[String], client_id: &str, text: &str) -> AppResult<(Access, PovRow, MessageRow)> {
        let core = &self.core;
        content::validate_client_id(client_id, content::MAX_CLIENT_ID_LEN)?;
        let (access, request) = self.pov_access(user_id, request_id).await?;
        if request.target_id != user_id {
            return Err(not_yours());
        }
        // A retried answer gets the stored message back.
        if let Some(reply_id) = request.reply_message_id {
            if let Some(reply) = access::find_message(core, reply_id).await?.filter(|reply| reply.client_id.as_deref() == Some(client_id)) {
                return Ok((access, request, reply));
            }
        }
        if request.state != pov::OPEN || request.expires_at <= now() {
            return Err(pov_not_open());
        }
        ensure!(!asset_ids.is_empty(), invalid, "NO_ATTACHMENTS", "a POV is a photo or a video");
        let attachments = self.resolve_attachments(user_id, asset_ids, POV_PURPOSES, MAX_POV_ATTACHMENTS).await?;
        // "Live" is as far as the server can check: uploaded after being asked.
        ensure!(
            attachments.iter().all(|asset| asset.created_at >= request.created_at),
            precondition,
            "POV_NOT_LIVE",
            "a POV must be taken after it was requested"
        );
        let text = content::normalize_text(text)?;
        self.ensure_writable(&access).await?;

        let mut tx = core.db.begin().await?;
        let mut conv = access::lock_conversation(&mut tx, request.conversation_id).await?;
        let locked = lock_pov(&mut tx, request_id).await?;
        if locked.state != pov::OPEN || locked.expires_at <= now() {
            return Err(pov_not_open());
        }
        let draft = Draft { client_id: Some(client_id.to_owned()), text, attachments, pov_request_id: Some(request_id), ..Draft::from_user(msg::MEDIA, user_id) }
            .silent();
        let posted = self.post(&mut tx, &mut conv, draft).await?;
        let answered: PovRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE chat_pov_requests p SET state = $2, answered_at = now(), reply_message_id = $3,
                    reply_expires_at = now() + make_interval(secs => $4)
             WHERE p.id = $1 RETURNING {POV}"
        )))
        .bind(request_id)
        .bind(pov::ANSWERED)
        .bind(posted.row.id)
        .bind(self.cfg.pov_reply_seconds as f64)
        .fetch_one(&mut *tx)
        .await?;
        self.notify_pov(&mut tx, answered.requester_id, PovNotice::Answered, user_id, &answered).await?;
        tx.commit().await?;

        self.announce(&posted).await;
        self.push_pov_messages(&answered, false).await;
        Ok((access, answered, posted.row))
    }

    /// Pushes the messages that show a POV request again after it changed.
    async fn push_pov_messages(&self, request: &PovRow, with_reply: bool) {
        let reason = pb::MessageUpdateReason::MESSAGE_UPDATE_REASON_POV;
        if let Some(message_id) = request.request_message_id {
            self.push_message_updated_by_id(message_id, reason).await;
        }
        if let Some(message_id) = request.reply_message_id.filter(|_| with_reply) {
            self.push_message_updated_by_id(message_id, reason).await;
        }
    }

    /// Closes an open request as declined (by the person asked) or
    /// cancelled (by the requester).
    async fn close_pov(&self, user_id: Uuid, request_id: Uuid, state: i16) -> AppResult<PovRow> {
        let (_, request) = self.pov_access(user_id, request_id).await?;
        let closer = if state == pov::DECLINED { request.target_id } else { request.requester_id };
        if closer != user_id {
            return Err(not_yours());
        }
        let closed: Option<PovRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE chat_pov_requests p SET state = $2 WHERE p.id = $1 AND p.state = $3 AND p.expires_at > now() RETURNING {POV}"
        )))
        .bind(request_id)
        .bind(state)
        .bind(pov::OPEN)
        .fetch_optional(&self.core.db)
        .await?;
        let closed = closed.ok_or_else(pov_not_open)?;
        self.push_pov_messages(&closed, false).await;
        Ok(closed)
    }

    pub(crate) async fn decline_pov(&self, user_id: Uuid, request_id: Uuid) -> AppResult<PovRow> {
        self.close_pov(user_id, request_id, pov::DECLINED).await
    }

    pub(crate) async fn cancel_pov(&self, user_id: Uuid, request_id: Uuid) -> AppResult<PovRow> {
        self.close_pov(user_id, request_id, pov::CANCELLED).await
    }

    /// The requester keeps the answer: it no longer expires.
    pub(crate) async fn save_pov(&self, user_id: Uuid, request_id: Uuid) -> AppResult<PovRow> {
        let (_, request) = self.pov_access(user_id, request_id).await?;
        if request.requester_id != user_id {
            return Err(not_yours());
        }
        ensure!(request.state == pov::ANSWERED, precondition, "POV_NOT_ANSWERED", "this POV request has not been answered");
        if request.saved_at.is_some() {
            return Ok(request);
        }
        let mut tx = self.core.db.begin().await?;
        let saved: Option<PovRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE chat_pov_requests p SET saved_at = now()
             WHERE p.id = $1 AND p.state = $2 AND p.saved_at IS NULL AND p.reply_expired_at IS NULL AND p.reply_expires_at > now()
             RETURNING {POV}"
        )))
        .bind(request_id)
        .bind(pov::ANSWERED)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(saved) = saved else {
            // Saved by another device in the meantime, or gone.
            drop(tx);
            return match self.find_pov(request_id).await?.filter(|request| request.saved_at.is_some()) {
                Some(request) => Ok(request),
                None => Err(AppError::precondition("POV_REPLY_EXPIRED", "the POV has already disappeared")),
            };
        };
        self.notify_pov(&mut tx, saved.target_id, PovNotice::Saved, user_id, &saved).await?;
        tx.commit().await?;
        self.push_pov_messages(&saved, true).await;
        Ok(saved)
    }

    /// Expires requests nobody answered in time and removes the content of
    /// answers nobody saved in time. Returns how many requests changed.
    pub async fn expire_povs(&self) -> AppResult<usize> {
        let core = &self.core;
        let unanswered: Vec<PovRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE chat_pov_requests p SET state = $1 WHERE p.state = $2 AND p.expires_at <= now() RETURNING {POV}"
        )))
        .bind(pov::EXPIRED)
        .bind(pov::OPEN)
        .fetch_all(&core.db)
        .await?;
        for request in &unanswered {
            self.push_pov_messages(request, false).await;
        }

        let due: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM chat_pov_requests
             WHERE state = $1 AND saved_at IS NULL AND reply_expired_at IS NULL AND reply_expires_at <= now()
             ORDER BY reply_expires_at LIMIT 200",
        )
        .bind(pov::ANSWERED)
        .fetch_all(&core.db)
        .await?;
        let mut expired = unanswered.len();
        for request_id in due {
            let mut tx = core.db.begin().await?;
            let gone: Option<PovRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "UPDATE chat_pov_requests p SET reply_expired_at = now()
                 WHERE p.id = $1 AND p.state = $2 AND p.saved_at IS NULL AND p.reply_expired_at IS NULL AND p.reply_expires_at <= now()
                 RETURNING {POV}"
            )))
            .bind(request_id)
            .bind(pov::ANSWERED)
            .fetch_optional(&mut *tx)
            .await?;
            // Saved at the last moment: nothing to do.
            let Some(gone) = gone else { continue };
            if let Some(reply_id) = gone.reply_message_id {
                sqlx::query(
                    "UPDATE chat_messages SET attachments = '[]', text = '', mentions = '[]', mention_all = false, link_url = NULL, link_preview = NULL
                     WHERE id = $1 AND deleted_at IS NULL",
                )
                .bind(reply_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query("DELETE FROM chat_message_assets WHERE message_id = $1").bind(reply_id).execute(&mut *tx).await?;
                assets::detach_holder(&mut *tx, &message_holder(reply_id)).await?;
            }
            tx.commit().await?;
            self.push_pov_messages(&gone, true).await;
            expired += 1;
        }
        Ok(expired)
    }

    /// Closes the open request of a conversation nobody can write in any
    /// more (a block, a deleted account).
    pub(crate) async fn cancel_open_pov(&self, tx: &mut Transaction<'_, Postgres>, conversation_id: Uuid) -> AppResult<Option<PovRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE chat_pov_requests p SET state = $2 WHERE p.conversation_id = $1 AND p.state = $3 RETURNING {POV}"
        )))
        .bind(conversation_id)
        .bind(pov::CANCELLED)
        .bind(pov::OPEN)
        .fetch_optional(&mut **tx)
        .await?)
    }

    /// Announces a request closed by [`Chat::cancel_open_pov`], after the commit.
    pub(crate) async fn after_pov_closed(&self, request: &PovRow) {
        self.push_pov_messages(request, false).await;
    }
}
