//! How the generic domains see chat: messages as reportable subjects, and
//! the chat section of a personal data export.
use std::sync::Arc;

use async_trait::async_trait;
use tokuly_core::{
    export::DataExporter,
    prelude::*,
    subject::{SubjectInfo, SubjectResolver},
};
use tokuly_proto::common::SubjectType;

use crate::{
    access, content,
    model::{msg, request},
    notices, render,
};

/// Owner reported for messages nobody sent (system messages).
const NO_OWNER: Uuid = Uuid::nil();

/// Registers the chat subject resolver and data exporter.
pub(crate) fn register(core: &Core) {
    core.subjects.register(SubjectType::SUBJECT_TYPE_CHAT_MESSAGE, Arc::new(MessageSubjects { core: core.clone() }));
    core.exporters.register(Arc::new(ChatExporter { core: core.clone() }));
}

/// `SUBJECT_TYPE_CHAT_MESSAGE`: a message resolves only for members of its
/// conversation who can see it. Messages can be reported, not liked or
/// commented on.
struct MessageSubjects {
    core: Core,
}

#[async_trait]
impl SubjectResolver for MessageSubjects {
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let (Ok(message_id), Some(viewer)) = (Uuid::parse_str(id), viewer) else { return Ok(None) };
        let Some(row) = access::find_message(&self.core, message_id).await?.filter(|row| !row.is_deleted()) else {
            return Ok(None);
        };
        let member = access::find_member(&self.core, row.conversation_id, viewer).await?;
        let visible =
            member.is_some_and(|member| member.request_state != request::DECLINED && row.seq > member.history_from_seq);
        if !visible {
            return Ok(None);
        }
        let title = match row.kind {
            msg::TEXT => content::snippet(&row.text, 80),
            _ => notices::teaser(&row).ja,
        };
        Ok(Some(SubjectInfo {
            id: row.id,
            owner_id: row.sender_id.unwrap_or(NO_OWNER),
            title,
            path: format!("{}?message={}", notices::conversation_path(row.conversation_id), row.id),
            image_url: render::thumbnail_of(&row, &Default::default()),
            interactions_enabled: false,
        }))
    }
}

struct ChatExporter {
    core: Core,
}

#[async_trait]
impl DataExporter for ChatExporter {
    fn name(&self) -> &'static str {
        "chat"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        let db = &self.core.db;
        let section = |sql: &'static str| async move {
            let rows: Vec<serde_json::Value> = sqlx::query_scalar(sql).bind(user_id).fetch_all(db).await?;
            AppResult::Ok(serde_json::Value::Array(rows))
        };
        Ok(serde_json::json!({
            "conversations": section(
                "SELECT jsonb_build_object('id', c.id, 'kind', c.kind, 'name', c.name, 'role', m.role, 'request_state', m.request_state,
                        'joined_at', m.joined_at, 'pinned', m.pinned_at IS NOT NULL, 'muted', m.muted, 'archived', m.archived_at IS NOT NULL)
                 FROM chat_members m JOIN chat_conversations c ON c.id = m.conversation_id
                 WHERE m.user_id = $1 ORDER BY m.joined_at, c.id"
            ).await?,
            "messages": section(
                "SELECT jsonb_build_object('id', id, 'conversation_id', conversation_id, 'kind', kind, 'text', text, 'stamp_id', stamp_id,
                        'attachments', attachments, 'location', location, 'link_url', link_url, 'forwarded', forwarded,
                        'sent_at', created_at, 'edited_at', edited_at)
                 FROM chat_messages WHERE sender_id = $1 AND deleted_at IS NULL ORDER BY id"
            ).await?,
            "reactions": section(
                "SELECT jsonb_build_object('message_id', message_id, 'emoji', emoji, 'reacted_at', created_at)
                 FROM chat_reactions WHERE user_id = $1 ORDER BY created_at, message_id"
            ).await?,
            "calls": section(
                "SELECT jsonb_build_object('call_id', k.id, 'conversation_id', k.conversation_id, 'video', k.video, 'started_by_me', k.started_by = $1,
                        'started_at', k.created_at, 'ended_at', k.ended_at, 'my_state', p.state, 'seconds_connected', p.seconds)
                 FROM chat_call_participants p JOIN chat_calls k ON k.id = p.call_id WHERE p.user_id = $1 ORDER BY k.id"
            ).await?,
            "pov_requests": section(
                "SELECT jsonb_build_object('id', id, 'conversation_id', conversation_id, 'requested_by_me', requester_id = $1, 'note', note,
                        'state', state, 'requested_at', created_at, 'answered_at', answered_at, 'saved', saved_at IS NOT NULL)
                 FROM chat_pov_requests WHERE requester_id = $1 OR target_id = $1 ORDER BY id"
            ).await?,
        }))
    }
}
