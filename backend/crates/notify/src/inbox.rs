//! The notification inbox: storage and conversion to API messages.
//!
//! An entry keeps its title and body in both languages; which one a reader
//! sees is decided when the entry is read ([`NotificationRow::to_proto`]).
//!
//! Entries of one user that share a collapse key are one row: a newer event
//! overwrites the content, moves the entry to the top and marks it unread
//! again ([`upsert`]).
use sqlx::{PgExecutor, Postgres, Transaction};
use tokuly_core::{outbound::Localized, prelude::*};
use tokuly_proto::{
    common::{SubjectRef, UserSummary},
    notify::{Notification, UnreadCount},
};

/// Columns of [`NotificationRow`], for `SELECT` and `RETURNING`.
macro_rules! notification_columns {
    () => {
        "id, user_id, kind, category, actor_id, actor_unnamed, title_ja, title_en, body_ja, body_en, image_url, path, \
         subject_type, subject_id, collapse_key, data, urgent, group_count, read_at, occurred_at"
    };
}
pub(crate) use notification_columns;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct NotificationRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub kind: String,
    pub category: String,
    pub actor_id: Option<Uuid>,
    /// `actor_id` must not be shown to the recipient (see the column).
    pub actor_unnamed: bool,
    pub title_ja: String,
    pub title_en: String,
    pub body_ja: String,
    pub body_en: String,
    pub image_url: Option<String>,
    pub path: String,
    pub subject_type: Option<i32>,
    pub subject_id: Option<Uuid>,
    pub collapse_key: Option<String>,
    pub data: serde_json::Value,
    pub urgent: bool,
    pub group_count: i32,
    pub read_at: Option<Time>,
    pub occurred_at: Time,
}

impl NotificationRow {
    pub fn title(&self) -> Localized {
        Localized::new(&self.title_ja, &self.title_en)
    }

    pub fn body(&self) -> Localized {
        Localized::new(&self.body_ja, &self.body_en)
    }

    /// Who the recipient may be shown as having caused the entry: nobody
    /// when the producer leaves the actor unnamed.
    pub fn shown_actor_id(&self) -> Option<Uuid> {
        self.actor_id.filter(|_| !self.actor_unnamed)
    }

    /// The entry as its recipient sees it, in their language. `actor` is
    /// dropped for an unnamed actor, whatever the caller passes.
    pub fn to_proto(&self, locale: &str, actor: Option<UserSummary>) -> Notification {
        let actor = actor.filter(|_| !self.actor_unnamed);
        let subject = match (self.subject_type, self.subject_id) {
            (Some(subject_type), Some(id)) => {
                Some(SubjectRef { r#type: subject_type.into(), id: id.to_string(), ..Default::default() })
            }
            _ => None,
        };
        Notification {
            id: self.id.to_string(),
            kind: self.kind.clone(),
            category: self.category.clone(),
            actor: actor.into(),
            title: self.title().pick(locale).to_owned(),
            body: self.body().pick(locale).to_owned(),
            image_url: self.image_url.clone().unwrap_or_default(),
            path: self.path.clone(),
            subject: subject.into(),
            collapse_key: self.collapse_key.clone().unwrap_or_default(),
            group_count: self.group_count,
            urgent: self.urgent,
            actor_unnamed: self.actor_unnamed && self.actor_id.is_some(),
            read: self.read_at.is_some(),
            read_at: opt_ts(self.read_at),
            occurred_at: ts(self.occurred_at),
            data_json: if self.data.is_null() { String::new() } else { self.data.to_string() },
            ..Default::default()
        }
    }
}

/// A notification about to enter an inbox.
pub struct NewNotification<'a> {
    pub user_id: Uuid,
    pub kind: &'a str,
    pub category: &'a str,
    pub actor_id: Option<Uuid>,
    pub actor_unnamed: bool,
    pub title: &'a Localized,
    pub body: &'a Localized,
    pub image_url: Option<&'a str>,
    pub path: &'a str,
    pub subject: Option<(i32, Uuid)>,
    pub collapse_key: Option<&'a str>,
    pub data: &'a serde_json::Value,
    /// What the entry refers to ([`crate::withdraw::refs_of`]).
    pub refs: &'a [Uuid],
    pub urgent: bool,
    pub occurred_at: Time,
}

/// Stores a notification, replacing the user's entry with the same collapse
/// key if there is one.
///
/// Returns `None` when the entry already reflects a *newer* event: events of
/// one key can be handled out of order, and an older one must not overwrite
/// what the user already saw.
pub async fn upsert(
    tx: &mut Transaction<'_, Postgres>,
    new: &NewNotification<'_>,
) -> AppResult<Option<NotificationRow>> {
    let row = sqlx::query_as(concat!(
        "INSERT INTO notifications
            (user_id, kind, category, actor_id, title_ja, title_en, body_ja, body_en, image_url, path,
             subject_type, subject_id, collapse_key, data, refs, urgent, occurred_at, actor_unnamed)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
         ON CONFLICT (user_id, collapse_key) WHERE collapse_key IS NOT NULL DO UPDATE SET
            kind = EXCLUDED.kind, category = EXCLUDED.category, actor_id = EXCLUDED.actor_id,
            actor_unnamed = EXCLUDED.actor_unnamed,
            title_ja = EXCLUDED.title_ja, title_en = EXCLUDED.title_en,
            body_ja = EXCLUDED.body_ja, body_en = EXCLUDED.body_en,
            image_url = EXCLUDED.image_url, path = EXCLUDED.path,
            subject_type = EXCLUDED.subject_type, subject_id = EXCLUDED.subject_id,
            data = EXCLUDED.data,
            -- Unread, the entry stands for every event folded in since it was
            -- read, so it refers to what all of them refer to.
            refs = CASE WHEN notifications.read_at IS NULL
                        THEN ARRAY(SELECT DISTINCT r FROM unnest(notifications.refs || EXCLUDED.refs) AS r ORDER BY r)
                        ELSE EXCLUDED.refs END,
            urgent = EXCLUDED.urgent, occurred_at = EXCLUDED.occurred_at,
            -- The count restarts once the user has seen the entry.
            group_count = CASE WHEN notifications.read_at IS NULL THEN notifications.group_count + 1 ELSE 1 END,
            read_at = NULL
         WHERE notifications.occurred_at <= EXCLUDED.occurred_at
         RETURNING ",
        notification_columns!()
    ))
    .bind(new.user_id)
    .bind(new.kind)
    .bind(new.category)
    .bind(new.actor_id)
    .bind(&new.title.ja)
    .bind(&new.title.en)
    .bind(&new.body.ja)
    .bind(&new.body.en)
    .bind(new.image_url)
    .bind(new.path)
    .bind(new.subject.map(|(subject_type, _)| subject_type))
    .bind(new.subject.map(|(_, id)| id))
    .bind(new.collapse_key)
    .bind(new.data)
    .bind(new.refs)
    .bind(new.urgent)
    .bind(new.occurred_at)
    .bind(new.actor_unnamed)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row)
}

/// Unread entries of a user, in total and per category.
pub async fn unread_count<'e, E: PgExecutor<'e>>(executor: E, user_id: Uuid) -> AppResult<UnreadCount> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT category, count(*) FROM notifications WHERE user_id = $1 AND read_at IS NULL GROUP BY category",
    )
    .bind(user_id)
    .fetch_all(executor)
    .await?;
    let mut unread = UnreadCount::default();
    for (category, count) in rows {
        let count = count.clamp(0, i32::MAX as i64) as i32;
        unread.total = unread.total.saturating_add(count);
        unread.by_category.insert(category, count);
    }
    Ok(unread)
}

/// Rows as API messages for a reader with `locale`, with their actors' summaries.
pub async fn to_protos(core: &Core, rows: &[NotificationRow], locale: &str) -> AppResult<Vec<Notification>> {
    // An unnamed actor is not even looked up.
    let mut actor_ids: Vec<Uuid> = rows.iter().filter_map(NotificationRow::shown_actor_id).collect();
    actor_ids.sort_unstable();
    actor_ids.dedup();
    let actors = tokuly_identity::summaries(core, &actor_ids).await?;
    Ok(rows.iter().map(|row| row.to_proto(locale, row.shown_actor_id().and_then(|id| actors.get(&id).cloned()))).collect())
}

/// The language a user reads their notifications in.
pub async fn locale_of(core: &Core, user_id: Uuid) -> AppResult<String> {
    let locale: Option<String> =
        sqlx::query_scalar("SELECT locale FROM users WHERE id = $1").bind(user_id).fetch_optional(&core.db).await?;
    Ok(locale.unwrap_or_else(|| "ja".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> NotificationRow {
        NotificationRow {
            id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
            kind: "chat.message".into(),
            category: "chat".into(),
            actor_id: None,
            actor_unnamed: false,
            title_ja: "新着メッセージ".into(),
            title_en: "New message".into(),
            body_ja: "やっほー".into(),
            body_en: "Hi there".into(),
            image_url: None,
            path: "/chat/1".into(),
            subject_type: Some(11),
            subject_id: Some(Uuid::nil()),
            collapse_key: Some("chat:1".into()),
            data: serde_json::json!({ "conversation_id": "1" }),
            urgent: false,
            group_count: 3,
            read_at: None,
            occurred_at: now(),
        }
    }

    #[test]
    fn an_unnamed_actor_is_never_shown() {
        let actor_id = Uuid::now_v7();
        let summary = || UserSummary { id: actor_id.to_string(), handle: "buyer".into(), ..Default::default() };
        let named = NotificationRow { actor_id: Some(actor_id), ..row() };
        assert_eq!(named.shown_actor_id(), Some(actor_id));
        assert_eq!(named.to_proto("ja", Some(summary())).actor.as_option().map(|a| a.handle.as_str()), Some("buyer"));
        let unnamed = NotificationRow { actor_unnamed: true, ..named };
        assert_eq!(unnamed.shown_actor_id(), None);
        let proto = unnamed.to_proto("ja", Some(summary()));
        assert!(proto.actor.as_option().is_none(), "even when a caller hands one in");
        assert!(proto.actor_unnamed, "clients are told someone did it");
    }

    #[test]
    fn text_follows_the_reader_locale() {
        let row = row();
        let ja = row.to_proto("ja-JP", None);
        assert_eq!((ja.title.as_str(), ja.body.as_str()), ("新着メッセージ", "やっほー"));
        let en = row.to_proto("en", None);
        assert_eq!((en.title.as_str(), en.body.as_str()), ("New message", "Hi there"));
    }

    #[test]
    fn optional_parts_map_to_empty_fields() {
        let mut row = row();
        let full = row.to_proto("ja", None);
        assert!(!full.read && full.read_at.as_option().is_none());
        assert_eq!(full.group_count, 3);
        assert_eq!(full.collapse_key, "chat:1");
        assert_eq!(full.subject.as_option().unwrap().id, Uuid::nil().to_string());
        assert_eq!(full.data_json, r#"{"conversation_id":"1"}"#);

        row.data = serde_json::Value::Null;
        row.subject_id = None;
        row.collapse_key = None;
        row.read_at = Some(now());
        let bare = row.to_proto("ja", None);
        assert!(bare.read && bare.read_at.as_option().is_some());
        assert!(bare.subject.as_option().is_none());
        assert!(bare.data_json.is_empty() && bare.collapse_key.is_empty());
    }
}
