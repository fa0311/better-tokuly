//! Requests to the generic domains: notifications, the activity log and search.
//!
//! Domains do not depend on `tokuly-notify`, `tokuly-feed` or `tokuly-search`.
//! They describe what happened with one of the request structs below, inside
//! their own transaction, and the generic domain picks it up from the event
//! stream:
//!
//! - [`notify`]   → `notify.requested`   → inbox entry + push
//! - [`activity`] → `activity.recorded`  → activity log / friends' feed / map stories
//! - [`index`] / [`unindex`] → `search.changed` → search index
//! - [`subject_deleted`] → `subject.deleted` → whatever other domains attached to the subject
use serde::{Deserialize, Serialize};
use sqlx::PgExecutor;
use tokuly_proto::common::SubjectType;
use uuid::Uuid;

use crate::{error::AppResult, events};

pub const NOTIFY_TOPIC: &str = "notify.requested";
pub const ACTIVITY_TOPIC: &str = "activity.recorded";
pub const SEARCH_TOPIC: &str = "search.changed";
pub const SUBJECT_DELETED_TOPIC: &str = "subject.deleted";

/// Text in both supported languages; the recipient's locale picks one.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Localized {
    pub ja: String,
    pub en: String,
}

impl Localized {
    pub fn new(ja: impl Into<String>, en: impl Into<String>) -> Self {
        Self { ja: ja.into(), en: en.into() }
    }

    /// The same text for every language (names, user-written content).
    pub fn same(text: impl Into<String>) -> Self {
        let text = text.into();
        Self { ja: text.clone(), en: text }
    }

    pub fn pick(&self, locale: &str) -> &str {
        if locale.to_ascii_lowercase().starts_with("ja") || self.en.is_empty() { &self.ja } else { &self.en }
    }
}

/// A notification for one user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationRequest {
    /// Recipient.
    pub user_id: Uuid,
    /// Dotted kind, e.g. `social.friend_request`, `chat.message`,
    /// `live.started`. Users mute notifications per kind prefix.
    pub kind: String,
    /// Who caused it (their avatar is shown); `None` for system notices.
    pub actor_id: Option<Uuid>,
    pub title: Localized,
    pub body: Localized,
    /// Optional preview image.
    pub image_url: Option<String>,
    /// Path in the app to open when tapped, e.g. `/chat/<id>`.
    pub path: String,
    /// What the notification is about, as `(SubjectType as i32, id)`.
    pub subject: Option<(i32, Uuid)>,
    /// Notifications sharing a key replace each other (e.g. one per chat).
    pub collapse_key: Option<String>,
    /// Keep an entry in the in-app inbox.
    pub inbox: bool,
    /// Also deliver as a push notification.
    pub push: bool,
    /// Must reach the user now: bypasses quiet hours and the push pause
    /// (incoming calls, SOS). Use sparingly.
    #[serde(default)]
    pub urgent: bool,
    /// Extra structured data for clients.
    #[serde(default)]
    pub data: serde_json::Value,
    /// The text deliberately does not name the actor ("Someone bought your
    /// pack"), so nothing may reveal them: no push or inbox entry names them,
    /// and the recipient's blocks and mutes do not apply to them (an entry
    /// that vanished or never came would say who it was). The actor is kept
    /// for account deletion.
    #[serde(default)]
    pub actor_unnamed: bool,
}

impl NotificationRequest {
    pub fn new(user_id: Uuid, kind: &str, title: Localized, body: Localized, path: impl Into<String>) -> Self {
        Self {
            user_id,
            kind: kind.to_owned(),
            actor_id: None,
            title,
            body,
            image_url: None,
            path: path.into(),
            subject: None,
            collapse_key: None,
            inbox: true,
            push: true,
            urgent: false,
            data: serde_json::Value::Null,
            actor_unnamed: false,
        }
    }

    pub fn actor(mut self, actor_id: Uuid) -> Self {
        self.actor_id = Some(actor_id);
        self
    }

    /// `actor_id` caused it but must not be named (see `actor_unnamed`).
    pub fn unnamed_actor(mut self, actor_id: Uuid) -> Self {
        self.actor_id = Some(actor_id);
        self.actor_unnamed = true;
        self
    }

    pub fn subject(mut self, subject_type: SubjectType, id: Uuid) -> Self {
        self.subject = Some((subject_type as i32, id));
        self
    }

    pub fn image(mut self, url: impl Into<String>) -> Self {
        self.image_url = Some(url.into());
        self
    }

    pub fn collapse(mut self, key: impl Into<String>) -> Self {
        self.collapse_key = Some(key.into());
        self
    }

    /// Push only (no inbox entry): transient things like buzzes.
    pub fn push_only(mut self) -> Self {
        self.inbox = false;
        self
    }

    /// Inbox only (no push): low-priority things.
    pub fn inbox_only(mut self) -> Self {
        self.push = false;
        self
    }

    pub fn data(mut self, data: serde_json::Value) -> Self {
        self.data = data;
        self
    }

    /// Marks the notification urgent (see [`Self::urgent`]).
    pub fn urgent(mut self) -> Self {
        self.urgent = true;
        self
    }
}

/// Requests a notification. Skips notifying the actor about their own action.
pub async fn notify<'e, E: PgExecutor<'e>>(executor: E, request: &NotificationRequest) -> AppResult<()> {
    if request.actor_id == Some(request.user_id) {
        return Ok(());
    }
    events::emit(executor, NOTIFY_TOPIC, request.actor_id, request).await
}

/// Who may see an activity entry in feeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityAudience {
    /// Only the actor (their own log).
    Private,
    /// The actor's friends.
    Friends,
    /// Anyone.
    Public,
}

/// Something a user did, for their activity log and their friends' feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub user_id: Uuid,
    /// Dotted kind, e.g. `account.joined`, `note.posted`, `live.started`,
    /// `social.bumped`, `geo.new_region`.
    pub kind: String,
    pub audience: ActivityAudience,
    /// What it is about, as `(SubjectType as i32, id)`.
    pub subject: Option<(i32, Uuid)>,
    /// Other users involved (e.g. the friend you bumped).
    #[serde(default)]
    pub with_user_ids: Vec<Uuid>,
    /// Short human summary, shown as-is when a client has no template for `kind`.
    pub summary: Localized,
    /// Path in the app, e.g. `/watch/<id>`.
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub image_url: Option<String>,
    /// Where it happened, when relevant for map stories: `(lat, lng)`.
    #[serde(default)]
    pub location: Option<(f64, f64)>,
    #[serde(default)]
    pub data: serde_json::Value,
}

impl ActivityEntry {
    pub fn new(user_id: Uuid, kind: &str, audience: ActivityAudience, summary: Localized) -> Self {
        Self {
            user_id,
            kind: kind.to_owned(),
            audience,
            subject: None,
            with_user_ids: Vec::new(),
            summary,
            path: String::new(),
            image_url: None,
            location: None,
            data: serde_json::Value::Null,
        }
    }

    pub fn subject(mut self, subject_type: SubjectType, id: Uuid) -> Self {
        self.subject = Some((subject_type as i32, id));
        self
    }

    pub fn with(mut self, user_ids: impl IntoIterator<Item = Uuid>) -> Self {
        self.with_user_ids = user_ids.into_iter().collect();
        self
    }

    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.path = path.into();
        self
    }

    pub fn image(mut self, url: impl Into<String>) -> Self {
        self.image_url = Some(url.into());
        self
    }

    pub fn at(mut self, lat: f64, lng: f64) -> Self {
        self.location = Some((lat, lng));
        self
    }

    pub fn data(mut self, data: serde_json::Value) -> Self {
        self.data = data;
        self
    }
}

/// Records an activity entry.
pub async fn activity<'e, E: PgExecutor<'e>>(executor: E, entry: &ActivityEntry) -> AppResult<()> {
    events::emit(executor, ACTIVITY_TOPIC, Some(entry.user_id), entry).await
}

/// A searchable document. `kind` selects the index: `user`, `channel`, `note`,
/// `video`, `short`, `live`, `clip`, `playlist`, `stamp_pack`, `place`,
/// `hashtag`, `news`, `answer`, `emoji`, `community`, `thread`, `space`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchDoc {
    pub kind: String,
    pub id: Uuid,
    pub title: String,
    /// Body text (description, note text, bio...).
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Owner user ID, for "my content" filters and block lists.
    pub owner_id: Option<Uuid>,
    /// Only documents everyone may find belong in the index; pass `false` to
    /// remove a document that stopped being public.
    pub public: bool,
    #[serde(default)]
    pub image_url: Option<String>,
    /// Path in the app, e.g. `/watch/<short_id>`.
    #[serde(default)]
    pub path: String,
    /// Unix seconds; used for recency ranking.
    pub created_at: i64,
    /// Popularity signal for ranking (views, likes, members...).
    #[serde(default)]
    pub score: f64,
    /// Kind-specific fields returned with results.
    #[serde(default)]
    pub extra: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchChange {
    pub kind: String,
    pub id: Uuid,
    /// `None` removes the document.
    pub doc: Option<SearchDoc>,
}

/// Adds or updates a document in the search index (removes it when `doc.public` is false).
pub async fn index<'e, E: PgExecutor<'e>>(executor: E, doc: SearchDoc) -> AppResult<()> {
    let change = SearchChange { kind: doc.kind.clone(), id: doc.id, doc: doc.public.then_some(doc) };
    events::emit(executor, SEARCH_TOPIC, None, &change).await
}

/// Removes a document from the search index.
pub async fn unindex<'e, E: PgExecutor<'e>>(executor: E, kind: &str, id: Uuid) -> AppResult<()> {
    events::emit(executor, SEARCH_TOPIC, None, &SearchChange { kind: kind.to_owned(), id, doc: None }).await
}

/// `subject.deleted`: a subject is gone for good, whoever deleted it (its
/// owner, a moderator, a retention timer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectDeleted {
    /// `tokuly.common.v1.SubjectType` as its number.
    pub subject_type: i32,
    pub subject_id: Uuid,
}

/// Says that a subject is gone, so the generic domains drop what they
/// attached to it (engagement: comments, reactions, bookmarks, counters).
///
/// Call it in the transaction that deletes the content, and only when the
/// content is gone for good. Content that is merely hidden (made private,
/// unpublished, behind a block) is still there; and content a domain keeps
/// for a while after its removal, so the people it reached can still report
/// it, is announced when that time is over: until then the comments on it
/// are reportable too. The domain's own deletion event, if it has one, is
/// emitted as before: that one carries what its consumers need (owner,
/// audience, reason). Rules: `docs/events.md`.
pub async fn subject_deleted<'e, E: PgExecutor<'e>>(
    executor: E,
    subject_type: SubjectType,
    subject_id: Uuid,
) -> AppResult<()> {
    subjects_deleted(executor, subject_type, &[subject_id]).await
}

/// [`subject_deleted`] for many subjects of one type (a cascade, a retention
/// sweep): one event each, written by one statement.
pub async fn subjects_deleted<'e, E: PgExecutor<'e>>(
    executor: E,
    subject_type: SubjectType,
    subject_ids: &[Uuid],
) -> AppResult<()> {
    if subject_ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO outbox (topic, data)
         SELECT $1, jsonb_build_object('subject_type', $2::integer, 'subject_id', subject_id)
         FROM unnest($3::uuid[]) AS subject_id",
    )
    .bind(SUBJECT_DELETED_TOPIC)
    .bind(subject_type as i32)
    .bind(subject_ids)
    .execute(executor)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_deleted_travels_as_the_type_number_and_the_id() {
        let subject_id = Uuid::now_v7();
        let deleted = SubjectDeleted { subject_type: SubjectType::SUBJECT_TYPE_VIDEO as i32, subject_id };
        let sent = serde_json::json!({ "subject_type": 5, "subject_id": subject_id });
        assert_eq!(serde_json::to_value(deleted).unwrap(), sent);
        assert_eq!(serde_json::from_value::<SubjectDeleted>(sent).unwrap(), deleted);
    }

    #[test]
    fn localized_picks_by_locale_with_japanese_fallback() {
        let text = Localized::new("こんにちは", "Hello");
        assert_eq!(text.pick("ja-JP"), "こんにちは");
        assert_eq!(text.pick("en-US"), "Hello");
        assert_eq!(text.pick("fr"), "Hello");
        assert_eq!(Localized::new("のみ", "").pick("en"), "のみ");
    }

    #[test]
    fn notification_builder_sets_delivery_channels() {
        let user = Uuid::now_v7();
        let n = NotificationRequest::new(user, "social.buzz", Localized::same("t"), Localized::same("b"), "/map").push_only();
        assert!(n.push && !n.inbox);
        let n = NotificationRequest::new(user, "x.y", Localized::same("t"), Localized::same("b"), "/").inbox_only();
        assert!(!n.push && n.inbox);
    }
}
