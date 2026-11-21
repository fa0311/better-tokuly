//! Polymorphic subjects.
//!
//! Comments, reactions, bookmarks, reports and notifications attach to "a
//! subject" — a note, a video, a short, a live stream... The owning domain
//! registers a [`SubjectResolver`] for its subject types at startup, and the
//! generic domains (engagement, notify, search) look subjects up through
//! [`Subjects`] without depending on the owning crates. A new kind of content
//! gets comments, likes and reports by registering one resolver.
use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use parking_lot::RwLock;
use tokuly_proto::common::SubjectType;
use uuid::Uuid;

use crate::error::{AppError, AppResult};

/// What generic domains need to know about a subject.
#[derive(Debug, Clone)]
pub struct SubjectInfo {
    pub id: Uuid,
    /// The user who owns the subject (receives notifications about it).
    pub owner_id: Uuid,
    /// Short human description for notifications and reports, e.g. a video title.
    pub title: String,
    /// Path of the subject in the web app, e.g. `/watch/dQw4w9WgXcQ`.
    pub path: String,
    pub image_url: Option<String>,
    /// Whether comments/reactions are currently accepted.
    pub interactions_enabled: bool,
}

#[async_trait]
pub trait SubjectResolver: Send + Sync + 'static {
    /// Looks up a subject as seen by `viewer`. Returns `None` when it does not
    /// exist or the viewer may not see it. `id` is a UUID or a short ID.
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>>;

    /// A denormalized counter on the subject changed, e.g. `"comment_count"`
    /// or `"like_count"`. Domains that keep such counters apply the delta.
    async fn counter_changed(&self, _id: Uuid, _counter: &str, _delta: i64) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct Subjects {
    resolvers: RwLock<HashMap<i32, Arc<dyn SubjectResolver>>>,
}

impl Subjects {
    pub fn register(&self, subject_type: SubjectType, resolver: Arc<dyn SubjectResolver>) {
        self.resolvers.write().insert(subject_type as i32, resolver);
    }

    fn resolver(&self, subject_type: SubjectType) -> AppResult<Arc<dyn SubjectResolver>> {
        self.resolvers
            .read()
            .get(&(subject_type as i32))
            .cloned()
            .ok_or_else(|| {
                AppError::invalid(
                    "UNSUPPORTED_SUBJECT",
                    format!("unsupported subject type {subject_type:?}"),
                )
            })
    }

    /// Resolves a subject or fails with `not_found`.
    pub async fn resolve(
        &self,
        subject_type: SubjectType,
        id: &str,
        viewer: Option<Uuid>,
    ) -> AppResult<SubjectInfo> {
        self.resolver(subject_type)?
            .resolve(id, viewer)
            .await?
            .ok_or_else(|| {
                AppError::not_found(
                    "SUBJECT_NOT_FOUND",
                    "the item does not exist or is not visible to you",
                )
            })
    }

    pub async fn counter_changed(
        &self,
        subject_type: SubjectType,
        id: Uuid,
        counter: &str,
        delta: i64,
    ) -> AppResult<()> {
        // Take the resolver out first: the lock guard must not live across the
        // await (it would make this future !Send, unusable in spawned tasks).
        let resolver = self.resolvers.read().get(&(subject_type as i32)).cloned();
        match resolver {
            Some(resolver) => resolver.counter_changed(id, counter, delta).await,
            None => Ok(()),
        }
    }

    pub fn is_registered(&self, subject_type: SubjectType) -> bool {
        self.resolvers.read().contains_key(&(subject_type as i32))
    }
}
