//! Owners the subject resolvers do not name.
//!
//! A report says who owns what was reported, and the owner is who a warning,
//! a suspension or a ban is for. Some content is reported without one: a
//! post in an anonymous channel resolves with nobody as its owner, on
//! purpose, so that nothing a member can do tells them who wrote it. The
//! domain that keeps such content knows the author, and this domain must not
//! depend on it — so the server hands in a lookup ([`HiddenOwners`]).
//!
//! The answer is stored as the case's owner, marked as hidden
//! (`moderation_cases.owner_hidden`), and from there it is handled the way
//! the keeping domain handles it itself:
//!
//! - **Nobody unmasks someone they have a quarrel with.** A moderator who
//!   reported the post, or is otherwise involved in it (the lookup knows:
//!   wrote in that thread, is mentioned or answered by its author), is not
//!   shown the author. The case tells them so, and they can neither resolve
//!   it nor take the post down: another moderator has to.
//! - **Every reveal leaves a trace.** Lists and the answers to changes never
//!   name a hidden owner. `GetCase` does, for a moderator who is not
//!   involved, and writes `case.author_revealed` to the audit log each time.
//!   A resolution or takedown that reaches the author is in the audit log
//!   as every resolution is.
//!
//! Nothing a member can call reaches the lookup.
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

    /// Whether `user_id` is involved in the subject beyond having written
    /// it: took part in the same conversation, is mentioned or answered by
    /// its author there, reported it inside the keeping domain. Such a
    /// moderator must not learn who the author is.
    async fn is_party(&self, subject_type: SubjectType, subject_id: Uuid, user_id: Uuid) -> AppResult<bool>;
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

    /// Whether `staff_id` must not learn the hidden owner of a subject:
    /// they reported it to site moderation, or the keeping domain says they
    /// are involved. Without a lookup there is nobody to ask, and the answer
    /// is yes.
    pub(crate) async fn is_party_to_hidden(
        &self,
        staff_id: Uuid,
        subject_type: i32,
        subject_id: Uuid,
    ) -> AppResult<bool> {
        let reported: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM moderation_reports r JOIN moderation_cases c ON c.id = r.case_id
                           WHERE c.subject_type = $1 AND c.subject_id = $2 AND r.reporter_id = $3)",
        )
        .bind(subject_type)
        .bind(subject_id)
        .bind(staff_id)
        .fetch_one(&self.core.db)
        .await?;
        if reported {
            return Ok(true);
        }
        let (Some(owners), Some(subject_type)) =
            (self.hidden_owners.get(), buffa::EnumValue::<SubjectType>::from(subject_type).as_known())
        else {
            return Ok(true);
        };
        owners.is_party(subject_type, subject_id, staff_id).await
    }
}
