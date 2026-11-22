//! This domain's section of a personal data export.
//!
//! The export gives a user what they can see in the app and nothing more.
//! That matters in both directions of anonymity:
//!
//! - what they **received** comes without its sender, unless the sender
//!   signed, and without anything that was dropped or is hidden from them;
//! - what they **sent** comes as "answered" or not, exactly as their list
//!   shows it: an export that told a dropped question from an ignored one
//!   would tell the sender they were blocked.
use async_trait::async_trait;
use sqlx::PgPool;
use tokuly_core::{export::DataExporter, prelude::*};

/// Holds a database handle rather than the domain: the exporter is stored
/// inside `Core`, and pointing back at it would form a reference cycle.
pub(crate) struct AskExporter {
    db: PgPool,
    /// The same retention the "asked" list uses.
    retention_days: i32,
}

impl AskExporter {
    pub(crate) fn new(core: &Core, retention_days: i32) -> Self {
        Self { db: core.db.clone(), retention_days }
    }

    async fn section(&self, sql: &'static str, user_id: Uuid) -> AppResult<serde_json::Value> {
        let rows: Vec<serde_json::Value> =
            sqlx::query_scalar(sql).bind(user_id).bind(self.retention_days).fetch_all(&self.db).await?;
        Ok(serde_json::Value::Array(rows))
    }
}

/// "Not sent by a banned account", for a table aliased `x`.
macro_rules! sender_not_banned {
    ($alias:literal) => {
        concat!(
            "NOT EXISTS (SELECT 1 FROM users banned WHERE banned.id = ",
            $alias,
            ".sender_id AND banned.banned_at IS NOT NULL)"
        )
    };
}

#[async_trait]
impl DataExporter for AskExporter {
    fn name(&self) -> &'static str {
        "ask"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        // Every statement takes the user as $1 and the retention as $2
        // (`$2::int IS NOT NULL` where a statement has no other use for it).
        Ok(serde_json::json!({
            "settings": self.section(
                "SELECT jsonb_build_object('box_open', box_open, 'audience', audience, 'box_prompt', box_prompt, 'allow_signed', allow_signed,
                        'allow_images', allow_images, 'muted_words', muted_words, 'in_polls', in_polls, 'allow_hints', allow_hints)
                 FROM ask_settings WHERE user_id = $1 AND $2::int IS NOT NULL",
                user_id,
            ).await?,
            "questions_received": self.section(
                concat!(
                    "SELECT jsonb_build_object('id', q.id, 'text', q.text, 'anonymous', NOT q.signed,
                            'asker_id', CASE WHEN q.signed THEN q.sender_id END, 'folder', q.folder, 'received_at', q.created_at,
                            'answer', CASE WHEN q.folder = 2 THEN jsonb_build_object('text', q.answer_text, 'audience', q.answer_audience,
                                           'answered_at', q.answered_at, 'edited_at', q.answer_edited_at, 'image', q.answer_image) END)
                     FROM ask_questions q
                     WHERE q.owner_id = $1 AND $2::int IS NOT NULL AND q.removed_at IS NULL AND NOT q.dropped AND ",
                    sender_not_banned!("q"),
                    " ORDER BY q.id"
                ),
                user_id,
            ).await?,
            "questions_sent": self.section(
                "SELECT jsonb_build_object('id', q.id, 'to_user_id', q.owner_id, 'text', q.text, 'signed', q.signed, 'sent_at', q.created_at,
                        'answered', (q.folder = 2 AND q.removed_at IS NULL AND NOT q.dropped),
                        'answer', CASE WHEN q.folder = 2 AND q.removed_at IS NULL AND NOT q.dropped THEN q.answer_text END)
                 FROM ask_questions q
                 WHERE q.sender_id = $1 AND (q.removed_by IS NULL OR q.removed_by = 1)
                   AND ((q.folder = 2 AND q.removed_at IS NULL AND NOT q.dropped) OR q.created_at > now() - make_interval(days => $2))
                 ORDER BY q.id",
                user_id,
            ).await?,
            "sender_blocks": self.section(
                "SELECT jsonb_build_object('id', id, 'source', source, 'preview', preview, 'blocked_at', created_at)
                 FROM ask_sender_blocks WHERE owner_id = $1 AND $2::int IS NOT NULL ORDER BY id",
                user_id,
            ).await?,
            "prompts": self.section(
                concat!(
                    "SELECT jsonb_build_object('id', p.id, 'text', p.text, 'emoji', p.emoji, 'posted_at', p.created_at, 'closes_at', p.closes_at,
                            'closed_at', p.closed_at,
                            'responses', (SELECT COALESCE(jsonb_agg(jsonb_build_object('id', r.id, 'text', r.text, 'anonymous', NOT r.signed,
                                                  'responder_id', CASE WHEN r.signed THEN r.sender_id END, 'held', r.held,
                                                  'published_at', r.published_at, 'received_at', r.created_at) ORDER BY r.id), '[]')
                                          FROM ask_prompt_responses r
                                          WHERE r.prompt_id = p.id AND r.removed_at IS NULL AND NOT r.dropped AND ",
                    sender_not_banned!("r"),
                    "))
                     FROM ask_prompts p WHERE p.owner_id = $1 AND $2::int IS NOT NULL AND p.removed_at IS NULL ORDER BY p.id"
                ),
                user_id,
            ).await?,
            "prompt_responses_sent": self.section(
                "SELECT jsonb_build_object('id', r.id, 'prompt_id', r.prompt_id, 'to_user_id', r.owner_id, 'text', r.text, 'signed', r.signed,
                        'sent_at', r.created_at)
                 FROM ask_prompt_responses r
                 WHERE r.sender_id = $1 AND $2::int IS NOT NULL AND (r.removed_by IS NULL OR r.removed_by = 1) ORDER BY r.id",
                user_id,
            ).await?,
            "flames_received": self.section(
                concat!(
                    "SELECT jsonb_build_object('id', f.id, 'question_ja', p.text_ja, 'question_en', p.text_en, 'received_at', f.created_at,
                            'seen_at', f.seen_at,
                            'hints', (SELECT COALESCE(jsonb_agg(jsonb_build_object('kind', h.kind, 'value', h.value, 'price', h.price,
                                              'bought_at', h.created_at) ORDER BY h.position), '[]')
                                      FROM ask_flame_hints h WHERE h.flame_id = f.id AND h.entry_id IS NOT NULL))
                     FROM ask_flames f JOIN ask_poll_questions p ON p.id = f.question_id
                     WHERE f.receiver_id = $1 AND $2::int IS NOT NULL AND NOT f.dropped AND ",
                    sender_not_banned!("f"),
                    " ORDER BY f.id"
                ),
                user_id,
            ).await?,
            "votes_cast": self.section(
                "SELECT jsonb_build_object('for_user_id', f.receiver_id, 'question_ja', p.text_ja, 'question_en', p.text_en, 'voted_at', f.created_at)
                 FROM ask_flames f JOIN ask_poll_questions p ON p.id = f.question_id
                 WHERE f.sender_id = $1 AND $2::int IS NOT NULL ORDER BY f.id",
                user_id,
            ).await?,
            "rounds": self.section(
                "SELECT jsonb_build_object('id', id, 'started_at', created_at, 'completed_at', completed_at, 'polls', cardinality(question_ids),
                        'answered', answered, 'votes', votes, 'reward_coins', reward_coins)
                 FROM ask_rounds WHERE user_id = $1 AND $2::int IS NOT NULL ORDER BY id",
                user_id,
            ).await?,
        }))
    }
}
