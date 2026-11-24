//! Personal data export: everything this domain stores about a user.
use std::sync::Arc;

use async_trait::async_trait;
use tokuly_core::{export::DataExporter, prelude::*};

/// Registers the exporter. Called once from `Engagement::new`.
pub(crate) fn register(core: &Core) {
    core.exporters.register(Arc::new(EngagementExporter { core: core.clone() }));
}

struct EngagementExporter {
    core: Core,
}

#[async_trait]
impl DataExporter for EngagementExporter {
    fn name(&self) -> &'static str {
        "engagement"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        let db = &self.core.db;
        let section = |sql: &'static str| async move {
            let rows: Vec<serde_json::Value> = sqlx::query_scalar(sql).bind(user_id).fetch_all(db).await?;
            AppResult::Ok(serde_json::Value::Array(rows))
        };
        let settings: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT jsonb_build_object('hold_all', hold_all, 'hold_links', hold_links, 'blocked_words', blocked_words)
             FROM comment_settings WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(db)
        .await?;
        Ok(serde_json::json!({
            "comments": section(
                "SELECT jsonb_build_object('id', id, 'subject_type', subject_type, 'subject_id', subject_id, 'parent_id', parent_id,
                        'channel_id', channel_id, 'text', text, 'stamp_id', stamp_id, 'image', image,
                        'timestamp_seconds', timestamp_seconds, 'status', status, 'created_at', created_at, 'edited_at', edited_at)
                 FROM comments WHERE author_id = $1 ORDER BY id"
            ).await?,
            // What the user's comments said before they edited or deleted
            // them, while it is kept for reports.
            "removed_comment_texts": section(
                "SELECT jsonb_build_object('comment_id', comment_id, 'subject_type', subject_type, 'subject_id', subject_id, 'text', text,
                        'reason', CASE reason WHEN 1 THEN 'edited' ELSE 'deleted' END, 'removed_at', removed_at)
                 FROM comment_removed_texts WHERE author_id = $1 ORDER BY removed_at, id"
            ).await?,
            "reactions": section(
                "SELECT jsonb_build_object('subject_type', subject_type, 'subject_id', subject_id, 'kind', kind, 'emoji', emoji,
                        'reacted_at', created_at)
                 FROM reactions WHERE user_id = $1 ORDER BY created_at, subject_id, emoji"
            ).await?,
            "reaction_settings": section(
                "SELECT jsonb_build_object('custom_emoji', custom_emoji, 'sensitive_emoji', sensitive_emoji, 'updated_at', updated_at)
                 FROM reaction_settings WHERE user_id = $1"
            ).await?,
            "collections": section(
                "SELECT jsonb_build_object('id', c.id, 'name', c.name, 'is_default', c.is_default, 'created_at', c.created_at,
                        'bookmarks', (SELECT COALESCE(jsonb_agg(jsonb_build_object('subject_type', b.subject_type,
                                             'subject_id', b.subject_id, 'saved_at', b.created_at) ORDER BY b.created_at, b.subject_id), '[]')
                                      FROM bookmarks b WHERE b.collection_id = c.id))
                 FROM bookmark_collections c WHERE c.user_id = $1 ORDER BY c.is_default DESC, c.id"
            ).await?,
            "reports": section(
                "SELECT jsonb_build_object('id', id, 'subject_type', subject_type, 'subject_id', subject_id, 'subject_title', subject_title,
                        'category', category, 'text', text, 'status', status, 'created_at', created_at, 'resolved_at', resolved_at)
                 FROM reports WHERE reporter_id = $1 ORDER BY id"
            ).await?,
            "comment_settings": settings,
            // The identity is the one the owner saw: the user, or a channel.
            "blocked_commenters": section(
                "SELECT jsonb_build_object('user_id', CASE WHEN channel_id IS NULL THEN blocked_user_id END, 'channel_id', channel_id,
                        'blocked_at', created_at)
                 FROM comment_blocks WHERE owner_id = $1 ORDER BY id"
            ).await?,
        }))
    }
}
