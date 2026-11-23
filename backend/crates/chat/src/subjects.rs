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
    Chat, access, content,
    model::{MessageRow, msg, removed, request},
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
/// conversation who can see it, as it is now (a deleted message does not
/// resolve). Messages can be reported, not liked or commented on.
///
/// Its sender editing or unsending it does not take the recipients' way to
/// report what they received: for a report (and nothing else) a message
/// resolves for [`REMOVED_TEXT_RETENTION_DAYS`] after such a change with
/// every version it had, a deleted one included.
struct MessageSubjects {
    core: Core,
}

/// Days the text an edit or an unsend removed is kept for reports, counted
/// from the removal.
pub(crate) const REMOVED_TEXT_RETENTION_DAYS: i32 = 30;

/// Texts edits replaced that are kept per message; later edits keep
/// nothing (what the message says now, or said when it was deleted, is
/// always known).
pub(crate) const MAX_EDITED_TEXTS: i64 = 20;

/// Characters of each version in a subject's title.
const TITLE_CHARS: usize = 80;

/// What a message says, as a report is filed with it: its text, or the line
/// notifications describe it with.
pub(crate) fn reported_text(row: &MessageRow) -> String {
    match row.kind {
        msg::TEXT => row.text.clone(),
        _ => notices::teaser(row).ja,
    }
}

/// The title of a report about a message that had several versions, oldest
/// first: each distinct version, shortened, joined by arrows. `truncated`
/// says that edits after the kept ones were not recorded.
pub(crate) fn report_title(versions: &[String], truncated: bool) -> String {
    let mut distinct: Vec<String> = Vec::with_capacity(versions.len());
    for version in versions {
        let version = content::snippet(version, TITLE_CHARS);
        if distinct.last() != Some(&version) {
            distinct.push(version);
        }
    }
    let count = distinct.len();
    if truncated && count > 1 {
        distinct.insert(count - 1, format!("（{MAX_EDITED_TEXTS}回目より後の編集は記録なし）"));
    }
    if count > 1 { format!("[{count}版] {}", distinct.join(" → ")) } else { distinct.pop().unwrap_or_default() }
}

impl MessageSubjects {
    async fn lookup(&self, id: &str, viewer: Option<Uuid>, for_report: bool) -> AppResult<Option<SubjectInfo>> {
        let (Ok(message_id), Some(viewer)) = (Uuid::parse_str(id), viewer) else { return Ok(None) };
        let Some(row) = access::find_message(&self.core, message_id).await? else { return Ok(None) };
        let member = access::find_member(&self.core, row.conversation_id, viewer).await?;
        let visible =
            member.is_some_and(|member| member.request_state != request::DECLINED && row.seq > member.history_from_seq);
        if !visible || (row.is_deleted() && !for_report) {
            return Ok(None);
        }
        let title = if for_report {
            let earlier: Vec<(String, i16)> = sqlx::query_as(
                "SELECT text, reason FROM chat_removed_texts
                 WHERE message_id = $1 AND removed_at > now() - make_interval(days => $2)
                 ORDER BY removed_at, id LIMIT $3",
            )
            .bind(row.id)
            .bind(REMOVED_TEXT_RETENTION_DAYS)
            .bind(MAX_EDITED_TEXTS + 1)
            .fetch_all(&self.core.db)
            .await?;
            let mut versions: Vec<String> = earlier
                .iter()
                .filter(|(_, reason)| *reason == removed::EDITED)
                .map(|(text, _)| text.clone())
                .take(MAX_EDITED_TEXTS as usize)
                .collect();
            let truncated = versions.len() as i64 >= MAX_EDITED_TEXTS;
            if row.is_deleted() {
                let deleted: Option<String> = sqlx::query_scalar(
                    "SELECT text FROM chat_removed_texts
                     WHERE message_id = $1 AND reason = $2 AND removed_at > now() - make_interval(days => $3)
                     ORDER BY removed_at DESC LIMIT 1",
                )
                .bind(row.id)
                .bind(removed::DELETED)
                .bind(REMOVED_TEXT_RETENTION_DAYS)
                .fetch_optional(&self.core.db)
                .await?;
                // Taken down, or unsent too long ago.
                let Some(deleted) = deleted else { return Ok(None) };
                versions.push(deleted);
            } else {
                versions.push(reported_text(&row));
            }
            report_title(&versions, truncated)
        } else {
            content::snippet(&reported_text(&row), TITLE_CHARS)
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

#[async_trait]
impl SubjectResolver for MessageSubjects {
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        self.lookup(id, viewer, false).await
    }

    async fn resolve_for_report(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        self.lookup(id, viewer, true).await
    }
}

impl Chat {
    /// Forgets the texts edits and unsends removed once they are older than
    /// [`REMOVED_TEXT_RETENTION_DAYS`]. Returns how many went.
    pub async fn purge_removed_texts(&self) -> AppResult<u64> {
        Ok(sqlx::query("DELETE FROM chat_removed_texts WHERE removed_at <= now() - make_interval(days => $1)")
            .bind(REMOVED_TEXT_RETENTION_DAYS)
            .execute(&self.core.db)
            .await?
            .rows_affected())
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
            // What the user's messages said before they edited or unsent
            // them, while it is kept for reports.
            "removed_texts": section(
                "SELECT jsonb_build_object('message_id', r.message_id, 'text', r.text,
                        'reason', CASE r.reason WHEN 1 THEN 'edited' ELSE 'deleted' END, 'removed_at', r.removed_at)
                 FROM chat_removed_texts r JOIN chat_messages x ON x.id = r.message_id
                 WHERE x.sender_id = $1 ORDER BY r.removed_at, r.id"
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

#[cfg(test)]
mod tests {
    use super::*;

    fn versions(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|text| text.to_string()).collect()
    }

    #[test]
    fn report_titles_name_every_distinct_version() {
        assert_eq!(report_title(&versions(&["やっほー"]), false), "やっほー");
        assert_eq!(
            report_title(&versions(&["おはよう", "ばか！", "なんでもない"]), false),
            "[3版] おはよう → ばか！ → なんでもない"
        );
        assert_eq!(report_title(&versions(&["同じ", "同じ"]), false), "同じ");
        let long = "あ".repeat(200);
        assert!(report_title(&versions(&[&long, "短い"]), false).chars().count() < 100);
        assert!(report_title(&versions(&["一", "二", "最後"]), true).contains("記録なし"));
    }
}
