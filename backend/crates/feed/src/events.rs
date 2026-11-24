//! Events the feed domain consumes (see `docs/events/feed.md`).
//!
//! The payloads of identity and social come from those crates
//! (`tokuly_identity::maintenance::{UserCreated, UserRef}`,
//! `tokuly_social::events::{FriendRemoved, BlockChanged}`); the admin domain
//! is not a dependency, so its payload is mirrored here.
//!
//! The feed domain emits no events of its own: it only uses the outbound
//! requests of `tokuly_core::outbound` (`notify.requested` for the recap
//! notice, `search.changed` for news articles).
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use tokuly_core::outbound::ACTIVITY_TOPIC;
pub use tokuly_social::events::{FRIEND_REMOVED, USER_BLOCKED};

pub const USER_CREATED: &str = "identity.user.created";
pub const USER_DELETED: &str = "identity.user.deleted";
pub const CONTENT_REMOVED: &str = "admin.content.removed";

/// Durable consumer name on the event stream.
pub const CONSUMER_NAME: &str = "feed";
/// Topics the consumer subscribes to, besides those of
/// [`crate::withdrawals::RULES`] (content that went away or narrowed).
pub const TOPICS: &[&str] =
    &[ACTIVITY_TOPIC, USER_CREATED, USER_DELETED, FRIEND_REMOVED, USER_BLOCKED, CONTENT_REMOVED];

/// Every topic the consumer subscribes to.
pub fn consumed_topics() -> Vec<&'static str> {
    TOPICS.iter().copied().chain(crate::withdrawals::topics()).collect()
}

/// `admin.content.removed`: a moderator took a piece of content down.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentRemoved {
    /// `tokuly.common.v1.SubjectType`.
    pub subject_type: i32,
    pub subject_id: Uuid,
}

/// Payload of the `feed.news.fetch` job: fetch one news source now.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchSourceJob {
    pub source_id: Uuid,
}

/// Name of the job that fetches one news source.
pub const FETCH_JOB: &str = "feed.news.fetch";
/// Kind of the "your recap is ready" notification.
pub const RECAP_READY_KIND: &str = "feed.recap_ready";
