//! The moderation log: what the staff (and the automatic safety tools) did
//! in a community. Written in the transaction of the action it records.
//!
//! The log never unmasks the author of an anonymous post. An entry about
//! such a post stores the author (the audit trail behind `RevealAuthor`)
//! but is rendered with the author's pseudonym only.
use sqlx::PgExecutor;
use tokuly_core::prelude::*;
use tokuly_proto::community::{self as pb, ModerationAction};

use crate::{Community, pseudonym::pseudonym};

/// One entry to be written.
#[derive(Debug, Clone)]
pub struct LogEntry {
    community_id: Uuid,
    action: ModerationAction,
    actor_id: Option<Uuid>,
    site_moderation: bool,
    target_user_id: Option<Uuid>,
    target_anonymous: bool,
    thread_id: Option<Uuid>,
    reply_id: Option<Uuid>,
    channel_id: Option<Uuid>,
    event_id: Option<Uuid>,
    reason: String,
    details: serde_json::Map<String, serde_json::Value>,
}

impl LogEntry {
    /// An automatic action; name the actor with [`Self::by`].
    pub fn new(community_id: Uuid, action: ModerationAction) -> Self {
        Self {
            community_id,
            action,
            actor_id: None,
            site_moderation: false,
            target_user_id: None,
            target_anonymous: false,
            thread_id: None,
            reply_id: None,
            channel_id: None,
            event_id: None,
            reason: String::new(),
            details: serde_json::Map::new(),
        }
    }

    pub fn by(mut self, actor_id: Uuid) -> Self {
        self.actor_id = Some(actor_id);
        self
    }

    /// The actor used the site-wide moderator role, not a community role.
    pub fn site(mut self, site_moderation: bool) -> Self {
        self.site_moderation = site_moderation;
        self
    }

    /// The member the action was done to.
    pub fn target(mut self, user_id: Uuid) -> Self {
        self.target_user_id = Some(user_id);
        self
    }

    /// The author of the post the action concerns. For an anonymous post
    /// the log shows the pseudonym instead.
    pub fn author(mut self, author_id: Uuid, anonymous: bool) -> Self {
        self.target_user_id = Some(author_id);
        self.target_anonymous = anonymous;
        self
    }

    pub fn thread(mut self, thread_id: Uuid) -> Self {
        self.thread_id = Some(thread_id);
        self
    }

    pub fn reply(mut self, thread_id: Uuid, reply_id: Uuid) -> Self {
        self.thread_id = Some(thread_id);
        self.reply_id = Some(reply_id);
        self
    }

    pub fn channel(mut self, channel_id: Uuid) -> Self {
        self.channel_id = Some(channel_id);
        self
    }

    pub fn event(mut self, event_id: Uuid) -> Self {
        self.event_id = Some(event_id);
        self
    }

    pub fn reason(mut self, reason: &str) -> Self {
        self.reason = reason.to_owned();
        self
    }

    pub fn detail(mut self, key: &str, value: impl ToString) -> Self {
        self.details.insert(key.to_owned(), serde_json::Value::String(value.to_string()));
        self
    }
}

/// Writes an entry. Pass the open transaction of the action.
pub async fn record<'e, E: PgExecutor<'e>>(executor: E, entry: &LogEntry) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO community_moderation_log
             (community_id, action, actor_id, site_moderation, target_user_id, target_anonymous,
              thread_id, reply_id, channel_id, event_id, reason, details)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(entry.community_id)
    .bind(entry.action as i32)
    .bind(entry.actor_id)
    .bind(entry.site_moderation)
    .bind(entry.target_user_id)
    .bind(entry.target_anonymous)
    .bind(entry.thread_id)
    .bind(entry.reply_id)
    .bind(entry.channel_id)
    .bind(entry.event_id)
    .bind(&entry.reason)
    .bind(serde_json::Value::Object(entry.details.clone()))
    .execute(executor)
    .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct LogRow {
    id: Uuid,
    action: i32,
    actor_id: Option<Uuid>,
    site_moderation: bool,
    target_user_id: Option<Uuid>,
    target_anonymous: bool,
    thread_id: Option<Uuid>,
    reply_id: Option<Uuid>,
    channel_id: Option<Uuid>,
    event_id: Option<Uuid>,
    reason: String,
    details: serde_json::Value,
    created_at: Time,
}

impl Community {
    /// A page of the log, newest first; `action` 0 = every action.
    pub(crate) async fn moderation_log(
        &self,
        community_id: Uuid,
        action: i32,
        page: &Page,
    ) -> AppResult<(Vec<pb::ModerationLogEntry>, tokuly_proto::common::PageInfo)> {
        let after: Option<Uuid> = page.decode()?;
        let mut rows: Vec<LogRow> = sqlx::query_as(
            "SELECT id, action, actor_id, site_moderation, target_user_id, target_anonymous, thread_id, reply_id,
                    channel_id, event_id, reason, details, created_at
             FROM community_moderation_log
             WHERE community_id = $1 AND ($2 = 0 OR action = $2) AND ($3::uuid IS NULL OR id < $3)
             ORDER BY id DESC LIMIT $4",
        )
        .bind(community_id)
        .bind(action)
        .bind(after)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| row.id);

        // Authors of anonymous posts are never looked up: they stay pseudonyms.
        let named: Vec<Uuid> = rows
            .iter()
            .flat_map(|row| [row.actor_id, row.target_user_id.filter(|_| !row.target_anonymous)])
            .flatten()
            .collect();
        let users = tokuly_identity::summaries(&self.core, &named).await?;
        let entries = rows
            .into_iter()
            .map(|row| {
                let target_pseudonym = match (row.target_anonymous, row.thread_id, row.target_user_id) {
                    (true, Some(thread_id), Some(author_id)) => Some(pseudonym(&self.core.keys, thread_id, author_id)),
                    _ => None,
                };
                let details = row
                    .details
                    .as_object()
                    .map(|object| {
                        object
                            .iter()
                            .map(|(key, value)| {
                                (key.clone(), value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string()))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                pb::ModerationLogEntry {
                    id: row.id.to_string(),
                    action: row.action.into(),
                    actor: row.actor_id.map(|id| tokuly_identity::users::pick(&users, id)).into(),
                    site_moderation: row.site_moderation,
                    target_user: row
                        .target_user_id
                        .filter(|_| !row.target_anonymous)
                        .map(|id| tokuly_identity::users::pick(&users, id))
                        .into(),
                    target_pseudonym: target_pseudonym.into(),
                    thread_id: row.thread_id.map(|id| id.to_string()).unwrap_or_default(),
                    reply_id: row.reply_id.map(|id| id.to_string()).unwrap_or_default(),
                    channel_id: row.channel_id.map(|id| id.to_string()).unwrap_or_default(),
                    event_id: row.event_id.map(|id| id.to_string()).unwrap_or_default(),
                    reason: row.reason,
                    details,
                    created_at: ts(row.created_at),
                    ..Default::default()
                }
            })
            .collect();
        Ok((entries, info))
    }
}
