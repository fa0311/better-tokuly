//! Owners the subject resolvers do not name.
//!
//! A report says who owns what was reported, and the owner is who a warning,
//! a suspension or a ban is for. Some content is reported without one: a
//! post in an anonymous channel resolves with nobody as its owner, on
//! purpose, so that nothing a member can do tells them who wrote it. The
//! domain that keeps such content knows the author, and this domain must not
//! depend on it — so the server hands in a lookup ([`HiddenOwners`]).
//!
//! The answer goes where a case's owner goes: the moderation queue, which
//! only moderators read, and the moderation log. It is asked when a report
//! arrives without an owner and when a moderator takes content down by
//! reference; nothing a member can call reaches it.
use std::sync::Arc;

use async_trait::async_trait;
use tokuly_core::prelude::*;
use tokuly_proto::common::SubjectType;

use crate::Admin;

/// Who answers for content whose resolver names no owner.
#[async_trait]
pub trait HiddenOwners: Send + Sync + 'static {
    /// The account behind `subject_id`, or `None` when the subject type is
    /// not one the implementation knows, or there is no such subject.
    async fn owner_for_moderation(&self, subject_type: SubjectType, subject_id: Uuid) -> AppResult<Option<Uuid>>;
}

impl Admin {
    /// Hands in the lookup for content reported without an owner (see
    /// [`HiddenOwners`]). Call once at startup; a second call is ignored.
    pub fn set_hidden_owners(&self, owners: Arc<dyn HiddenOwners>) {
        let _ = self.hidden_owners.set(owners);
    }

    /// The owner of a subject nothing else names an owner for. `None`
    /// without a lookup, for subject types this build does not know, and
    /// when the lookup has no answer.
    pub(crate) async fn hidden_owner(&self, subject_type: i32, subject_id: Uuid) -> AppResult<Option<Uuid>> {
        let (Some(owners), Some(subject_type)) =
            (self.hidden_owners.get(), buffa::EnumValue::<SubjectType>::from(subject_type).as_known())
        else {
            return Ok(None);
        };
        owners.owner_for_moderation(subject_type, subject_id).await
    }
}
