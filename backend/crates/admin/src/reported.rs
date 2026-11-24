//! Reported content as the people who reported it can still see it.
//!
//! A case quotes its subject as it was reported, but a moderator who opens
//! it also needs to look at the content itself, and the owner may have
//! hidden it since: a video made private, a comment hidden after the
//! report. The normal lookup (`core.subjects.resolve` with the moderator as
//! viewer) refuses those, so the server hands in a lookup that answers the
//! way the owning domain answers the reporters ([`ReportedSubjects`]).
//!
//! It is used in the case view (`GetCase`) and when a takedown has to find
//! its subject, and only for staff who are not involved: a case about the
//! caller and a case whose hidden author the caller must not learn get
//! nothing from it, and the lookup itself refuses the subject's owner and
//! its reporters. What it answers never names an owner in the case view.
use std::sync::Arc;

use async_trait::async_trait;
use tokuly_core::{prelude::*, subject::SubjectInfo};
use tokuly_proto::common::SubjectType;

use crate::{Admin, staff::Staff};

/// Resolves reported content for the moderation team.
#[async_trait]
pub trait ReportedSubjects: Send + Sync + 'static {
    /// The subject as its reporters can still see it, for `staff_id`
    /// working its case. `None` when nobody reported it, when it resolves
    /// for none of its reporters, or when `staff_id` is a party to it (its
    /// owner or a reporter).
    async fn reported_subject(
        &self,
        staff_id: Uuid,
        subject_type: SubjectType,
        subject_id: Uuid,
    ) -> AppResult<Option<SubjectInfo>>;
}

impl Admin {
    /// Hands in the lookup of reported content (see [`ReportedSubjects`]).
    /// Call once at startup; a second call is ignored.
    pub fn set_reported_subjects(&self, subjects: Arc<dyn ReportedSubjects>) {
        let _ = self.reported.set(subjects);
    }

    /// The subject of a case as its reporters can still see it, for `staff`.
    /// `None` without a lookup and for subject types this build does not know.
    pub(crate) async fn reported_subject(
        &self,
        staff: &Staff,
        subject_type: i32,
        subject_id: Uuid,
    ) -> AppResult<Option<SubjectInfo>> {
        let (Some(subjects), Some(subject_type)) =
            (self.reported.get(), buffa::EnumValue::<SubjectType>::from(subject_type).as_known())
        else {
            return Ok(None);
        };
        subjects.reported_subject(staff.id, subject_type, subject_id).await
    }
}
