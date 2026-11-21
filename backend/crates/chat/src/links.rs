//! Link previews: the first link of a message gets its preview shortly
//! after sending, from a background job, and the message is pushed again
//! once it is there.
use std::time::Duration;

use tokuly_core::prelude::*;
use tokuly_proto::chat as pb;

use crate::{
    Chat, access,
    model::{MSG, MessageRow},
    send::{JOB_LINK_PREVIEW, LinkPreviewJob},
};

/// Link previews one user's messages may trigger per minute; links beyond
/// that simply get none.
const PREVIEWS_PER_MINUTE: u32 = 30;

impl Chat {
    /// Fetches the preview of a message's first link and stores it. The
    /// handler of the `chat.link_preview` job; safe to run more than once.
    pub async fn resolve_link_preview(&self, message_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        let Some(row) = access::find_message(core, message_id).await? else { return Ok(()) };
        let Some(url) = row.link_url.clone().filter(|_| !row.is_deleted() && row.link_checked_at.is_none()) else {
            return Ok(());
        };
        let within_limit = match row.sender_id {
            Some(sender) => core
                .cache
                .rate_limit(&format!("chat-link:{sender}"), PREVIEWS_PER_MINUTE, Duration::from_secs(60))
                .await
                .is_ok(),
            None => true,
        };
        let preview = if within_limit {
            match tokuly_media::link_preview(core, &url).await {
                Ok(preview) => preview,
                // A link the fetcher will not touch has no preview; anything
                // else is worth a retry.
                Err(e) if e.reason == "URL_INVALID" => None,
                Err(e) => return Err(e),
            }
        } else {
            None
        };
        let preview = preview
            .map(|preview| serde_json::to_value(&preview))
            .transpose()
            .map_err(|e| AppError::internal(anyhow::anyhow!("preview encode: {e}")))?;
        // Only if the link is still the one that was looked up (the message
        // may have been edited meanwhile).
        let updated: Option<MessageRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE chat_messages x SET link_preview = $2, link_checked_at = now()
             WHERE x.id = $1 AND x.link_url = $3 AND x.link_checked_at IS NULL AND x.deleted_at IS NULL
             RETURNING {MSG}"
        )))
        .bind(message_id)
        .bind(preview)
        .bind(&url)
        .fetch_optional(&core.db)
        .await?;
        if let Some(updated) = updated.filter(|updated| updated.link_preview.is_some()) {
            self.push_message_updated(&updated, pb::MessageUpdateReason::MESSAGE_UPDATE_REASON_LINK_PREVIEW).await;
        }
        Ok(())
    }

    /// Queues the preview job again for links that were never looked up (a
    /// job lost with a crashed worker), and gives up on those older than a
    /// day. Returns how many jobs were queued.
    pub async fn sweep_link_previews(&self) -> AppResult<usize> {
        let core = &self.core;
        sqlx::query(
            "UPDATE chat_messages SET link_checked_at = now()
             WHERE link_url IS NOT NULL AND link_checked_at IS NULL AND created_at <= now() - interval '1 day'",
        )
        .execute(&core.db)
        .await?;
        let pending: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM chat_messages
             WHERE link_url IS NOT NULL AND link_checked_at IS NULL AND deleted_at IS NULL AND created_at <= now() - interval '2 minutes'
             ORDER BY created_at LIMIT 200",
        )
        .fetch_all(&core.db)
        .await?;
        for message_id in &pending {
            core.jobs
                .enqueue_unique(JOB_LINK_PREVIEW, &message_id.to_string(), &LinkPreviewJob { message_id: *message_id })
                .await?;
        }
        Ok(pending.len())
    }
}
