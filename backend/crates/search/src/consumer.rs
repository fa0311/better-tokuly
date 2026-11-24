//! The event consumer: everything that changes what can be found.
//!
//! | Event | Effect |
//! |---|---|
//! | `search.changed` | the document is stored in or removed from the registry |
//! | `identity.user.created` / `updated` / `privacy_changed` | the user's own document is rebuilt |
//! | `social.follow.settings_changed` | the user's own document is rebuilt (a locked account is not found by its プロフ帳) |
//! | `identity.user.banned` | the user's document is removed and everything they own leaves the indexes |
//! | `identity.user.unbanned` | their document and everything they own come back |
//! | `identity.user.deleted` | their documents are removed for good, their search history is deleted |
//! | `admin.content.removed` | the moderated subject's document is removed |
//!
//! Events arrive at least once and in no guaranteed order. Every effect is
//! versioned by the event's ID (see [`crate::registry`]) or is a statement
//! of current state, so handling an event twice or late changes nothing.
use tokuly_core::{
    events::Event,
    outbound::{SEARCH_TOPIC, SearchChange},
    prelude::*,
};

use crate::{
    Search,
    events::{
        CONTENT_REMOVED_TOPIC, ContentRemoved, FOLLOW_SETTINGS_CHANGED_TOPIC, USER_BANNED_TOPIC, USER_CREATED_TOPIC, USER_DELETED_TOPIC,
        USER_PRIVACY_CHANGED_TOPIC, USER_UNBANNED_TOPIC, USER_UPDATED_TOPIC, UserRef,
    },
    kinds,
};

/// Durable consumer name on the event stream.
pub const CONSUMER_NAME: &str = "search";
/// Topics the consumer subscribes to.
pub const TOPICS: &[&str] = &[
    SEARCH_TOPIC,
    USER_CREATED_TOPIC,
    USER_UPDATED_TOPIC,
    USER_PRIVACY_CHANGED_TOPIC,
    USER_BANNED_TOPIC,
    USER_UNBANNED_TOPIC,
    USER_DELETED_TOPIC,
    CONTENT_REMOVED_TOPIC,
    FOLLOW_SETTINGS_CHANGED_TOPIC,
];

/// Reads an event's payload. A payload that does not parse will not parse on
/// the next delivery either, so it is logged and the event is given up.
fn payload<T: serde::de::DeserializeOwned>(event: &Event) -> Option<T> {
    match serde_json::from_value(event.data.clone()) {
        Ok(data) => Some(data),
        Err(error) => {
            tracing::error!(topic = %event.topic, id = %event.id, %error, "event with an unreadable payload dropped");
            None
        }
    }
}

impl Search {
    /// Handles one event of [`TOPICS`]. Idempotent.
    pub async fn handle_event(&self, event: Event) -> AppResult<()> {
        match event.topic.as_str() {
            SEARCH_TOPIC => {
                if let Some(change) = payload::<SearchChange>(&event) {
                    self.apply_change(event.id, change).await?;
                }
            }
            USER_CREATED_TOPIC | USER_UPDATED_TOPIC | USER_PRIVACY_CHANGED_TOPIC | FOLLOW_SETTINGS_CHANGED_TOPIC => {
                if let Some(user) = payload::<UserRef>(&event) {
                    self.index_user(event.id, user.user_id).await?;
                }
            }
            USER_BANNED_TOPIC | USER_UNBANNED_TOPIC => {
                if let Some(user) = payload::<UserRef>(&event) {
                    // The account row already says whether it is suspended:
                    // the user's document follows it, and the indexer leaves
                    // out or restores everything else they own.
                    self.index_user(event.id, user.user_id).await?;
                    self.requeue_owner(user.user_id).await?;
                }
            }
            USER_DELETED_TOPIC => {
                if let Some(user) = payload::<UserRef>(&event) {
                    self.purge_user(event.id, user.user_id).await?;
                }
            }
            CONTENT_REMOVED_TOPIC => {
                if let Some(removed) = payload::<ContentRemoved>(&event) {
                    // The owning domain unindexes the subject too; this makes
                    // the takedown effective in search without waiting for it.
                    if let Some(kind) = kinds::by_subject_type(removed.subject_type) {
                        self.remove_document(event.id, kind.name, removed.subject_id).await?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
