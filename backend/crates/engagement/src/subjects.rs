//! Subjects: what comments, reactions, bookmarks, shares and reports hang
//! off.
//!
//! A subject is a `(SubjectType, UUID)` pair owned by another domain. This
//! module turns the `SubjectRef` of a request into a [`Subject`] through
//! `core.subjects` — the owning domain decides whether the viewer may see it
//! — and adds the one rule that is this domain's own: two users with a block
//! between them do not reach each other's subjects.
//!
//! It also holds what the engagement domain registers itself: the resolver
//! that makes comments subjects (so they can be liked and reported), a
//! fallback resolver for users (so accounts can be reported), and the guard
//! of the `subject:` realtime topics.
use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use buffa::EnumValue;
use connectrpc::ErrorCode;
use futures::StreamExt as _;
use tokuly_core::{
    prelude::*,
    realtime::TopicGuard,
    subject::{SubjectInfo, SubjectResolver},
};
use tokuly_identity::users;
use tokuly_media::assets::image_from_json;
use tokuly_proto::{
    common::{SubjectRef, SubjectType},
    engagement::SubjectPreview,
};
use tokuly_social::graph;

use crate::{Engagement, text};

/// Kind of the realtime topics engagement events travel on.
pub const TOPIC_KIND: &str = "subject";

/// Subjects resolved at once when a list needs previews.
const RESOLVE_CONCURRENCY: usize = 8;

/// A subject's identity as stored: the type number and the UUID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SubjectKey {
    /// `tokuly.common.v1.SubjectType` as its number.
    pub kind: i32,
    pub id: Uuid,
}

impl SubjectKey {
    pub fn new(kind: SubjectType, id: Uuid) -> Self {
        Self { kind: kind as i32, id }
    }

    pub fn of_comment(comment_id: Uuid) -> Self {
        Self::new(SubjectType::SUBJECT_TYPE_COMMENT, comment_id)
    }

    /// The type as an enum; `UNSPECIFIED` for numbers this build does not know.
    pub fn subject_type(&self) -> SubjectType {
        EnumValue::<SubjectType>::from(self.kind).as_known().unwrap_or(SubjectType::SUBJECT_TYPE_UNSPECIFIED)
    }

    pub fn is_comment(&self) -> bool {
        self.kind == SubjectType::SUBJECT_TYPE_COMMENT as i32
    }

    pub fn to_ref(&self) -> SubjectRef {
        SubjectRef { r#type: self.kind.into(), id: self.id.to_string(), ..Default::default() }
    }

    /// The realtime topic of the subject: `subject:<type number>:<uuid>`.
    pub fn topic(&self) -> String {
        format!("{TOPIC_KIND}:{}:{}", self.kind, self.id)
    }

    /// Parses a topic made by [`Self::topic`].
    pub fn from_topic(topic: &str) -> Option<Self> {
        let mut parts = topic.split(':');
        let (kind, number, id) = (parts.next()?, parts.next()?, parts.next()?);
        if kind != TOPIC_KIND || parts.next().is_some() {
            return None;
        }
        Some(Self { kind: number.parse().ok()?, id: Uuid::parse_str(id).ok()? })
    }
}

/// A subject as resolved for one viewer.
#[derive(Debug, Clone)]
pub struct Subject {
    pub key: SubjectKey,
    pub info: SubjectInfo,
}

impl Subject {
    /// The owner, unless the subject belongs to nobody (official content
    /// has the nil UUID as its owner).
    pub fn owner(&self) -> Option<Uuid> {
        Some(self.info.owner_id).filter(|id| !id.is_nil())
    }

    pub fn is_owned_by(&self, user_id: Uuid) -> bool {
        self.owner() == Some(user_id)
    }

    pub fn preview(&self) -> SubjectPreview {
        SubjectPreview {
            subject: self.key.to_ref().into(),
            title: self.info.title.clone(),
            path: self.info.path.clone(),
            image_url: self.info.image_url.clone().unwrap_or_default(),
            available: true,
            ..Default::default()
        }
    }
}

/// A subject named by a client that may have stopped being visible: taking
/// one's own reaction or bookmark back must keep working then.
#[derive(Debug, Clone)]
pub struct LooseSubject {
    pub key: SubjectKey,
    /// `None` when the subject could not be resolved for the viewer.
    pub subject: Option<Subject>,
}

/// The preview of a subject in a list: what it is when it resolved, a
/// "no longer available" entry otherwise.
pub fn preview_of(key: SubjectKey, subject: Option<&Subject>) -> SubjectPreview {
    match subject {
        Some(subject) => subject.preview(),
        None => SubjectPreview { subject: key.to_ref().into(), available: false, ..Default::default() },
    }
}

/// The subject type of a request, or `UNSUPPORTED_SUBJECT` when it is unset
/// or unknown to this build.
pub fn requested_type(value: EnumValue<SubjectType>) -> AppResult<SubjectType> {
    match value.as_known() {
        Some(kind) if kind != SubjectType::SUBJECT_TYPE_UNSPECIFIED => Ok(kind),
        _ => Err(AppError::invalid("UNSUPPORTED_SUBJECT", "unsupported subject type")),
    }
}

fn not_found() -> AppError {
    AppError::not_found("SUBJECT_NOT_FOUND", "the item does not exist or is not visible to you")
}

/// Whether an error from resolving a subject means "there is no such
/// subject for this viewer" rather than a failure.
pub fn is_absent(error: &AppError) -> bool {
    error.is_not_found() || error.code == ErrorCode::InvalidArgument
}

/// The path of a comment in the web app: its subject's page, pointing at it.
pub fn comment_path(subject_path: &str, comment_id: Uuid) -> String {
    let separator = if subject_path.contains('?') { '&' } else { '?' };
    format!("{subject_path}{separator}comment={comment_id}")
}

impl Engagement {
    /// Resolves a subject for `viewer` without looking at blocks. Reports
    /// use this: a block must not stop anyone from reporting what they can
    /// still see.
    pub(crate) async fn subject_unguarded(&self, kind: SubjectType, id: &str, viewer: Option<Uuid>) -> AppResult<Subject> {
        let id = id.trim();
        // No resolver takes anything but a UUID, a short ID or a handle.
        if id.is_empty() || id.len() > 64 {
            return Err(not_found());
        }
        let info = self.core.subjects.resolve(kind, id, viewer).await?;
        Ok(Subject { key: SubjectKey::new(kind, info.id), info })
    }

    /// Resolves a subject for `viewer`: `SUBJECT_NOT_FOUND` when it does not
    /// exist, the viewer may not see it, or there is a block between the
    /// viewer and its owner; `UNSUPPORTED_SUBJECT` when no domain serves the
    /// type.
    pub(crate) async fn subject(&self, kind: SubjectType, id: &str, viewer: Option<Uuid>) -> AppResult<Subject> {
        let subject = self.subject_unguarded(kind, id, viewer).await?;
        if let (Some(viewer), Some(owner)) = (viewer, subject.owner())
            && viewer != owner
            && graph::is_blocked_either(&self.core, viewer, owner).await?
        {
            return Err(not_found());
        }
        Ok(subject)
    }

    /// [`Self::subject_unguarded`] for the `SubjectRef` of a request.
    pub(crate) async fn requested_subject_unguarded(&self, subject: Option<&SubjectRef>, viewer: Option<Uuid>) -> AppResult<Subject> {
        let (kind, id) = requested(subject)?;
        self.subject_unguarded(kind, id, viewer).await
    }

    /// [`Self::subject`] for the `SubjectRef` of a request.
    pub(crate) async fn requested_subject(&self, subject: Option<&SubjectRef>, viewer: Option<Uuid>) -> AppResult<Subject> {
        let (kind, id) = requested(subject)?;
        self.subject(kind, id, viewer).await
    }

    /// Like [`Self::requested_subject`], but a subject named by UUID that no
    /// longer resolves is still returned, as its bare key.
    pub(crate) async fn requested_subject_loose(&self, subject: Option<&SubjectRef>, viewer: Uuid) -> AppResult<LooseSubject> {
        let (kind, id) = requested(subject)?;
        match self.subject(kind, id, Some(viewer)).await {
            Ok(subject) => Ok(LooseSubject { key: subject.key, subject: Some(subject) }),
            Err(error) if error.is_not_found() => match Uuid::parse_str(id.trim()) {
                Ok(uuid) => Ok(LooseSubject { key: SubjectKey::new(kind, uuid), subject: None }),
                Err(_) => Err(error),
            },
            Err(error) => Err(error),
        }
    }

    /// Resolves many stored subjects for the rows of a list. Subjects that
    /// are gone, hidden from the viewer or of a type nobody serves any more
    /// are simply absent from the result.
    pub(crate) async fn subjects(&self, keys: &[SubjectKey], viewer: Option<Uuid>) -> AppResult<HashMap<SubjectKey, Subject>> {
        let mut distinct = keys.to_vec();
        distinct.sort();
        distinct.dedup();
        let hidden = self.hidden_from(viewer).await?;
        let mut resolving = futures::stream::iter(distinct)
            .map(|key| async move {
                let resolved = self.subject_unguarded(key.subject_type(), &key.id.to_string(), viewer).await;
                (key, resolved)
            })
            .buffer_unordered(RESOLVE_CONCURRENCY);
        let mut subjects = HashMap::new();
        while let Some((key, resolved)) = resolving.next().await {
            match resolved {
                Ok(subject) if !hidden.contains(&subject.info.owner_id) => {
                    subjects.insert(key, subject);
                }
                Ok(_) => {}
                Err(error) if is_absent(&error) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(subjects)
    }

    /// Everyone in a block with `viewer`, in either direction.
    pub(crate) async fn hidden_from(&self, viewer: Option<Uuid>) -> AppResult<Vec<Uuid>> {
        match viewer {
            Some(viewer) => graph::blocked_ids_either(&self.core, viewer).await,
            None => Ok(Vec::new()),
        }
    }
}

fn requested(subject: Option<&SubjectRef>) -> AppResult<(SubjectType, &str)> {
    match subject {
        Some(subject) => Ok((requested_type(subject.r#type)?, subject.id.as_str())),
        None => Err(AppError::invalid("UNSUPPORTED_SUBJECT", "a subject is required")),
    }
}

/// Registers what this domain contributes to the shared registries. Called
/// once from `Engagement::new`.
pub(crate) fn register(engagement: &Engagement) {
    let core = &engagement.core;
    core.subjects.register(SubjectType::SUBJECT_TYPE_COMMENT, Arc::new(CommentSubjects(engagement.clone())));
    // Accounts must be reportable even when no other domain serves them as
    // subjects; a domain that registers users later replaces this.
    if !core.subjects.is_registered(SubjectType::SUBJECT_TYPE_USER) {
        core.subjects.register(SubjectType::SUBJECT_TYPE_USER, Arc::new(UserSubjects { core: core.clone() }));
    }
    core.topic_guards.register(TOPIC_KIND, Arc::new(SubjectTopics(engagement.clone())));
}

/// Comments as subjects: they take likes and dislikes, and can be reported.
struct CommentSubjects(Engagement);

#[async_trait]
impl SubjectResolver for CommentSubjects {
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let Ok(comment_id) = Uuid::parse_str(id) else { return Ok(None) };
        let Some(found) = self.0.comment_for(comment_id, viewer).await? else { return Ok(None) };
        let comment = &found.row;
        let title = match text::excerpt(&comment.text, 60) {
            excerpt if excerpt.is_empty() => "コメント".to_owned(),
            excerpt => excerpt,
        };
        Ok(Some(SubjectInfo {
            id: comment.id,
            owner_id: comment.author_id,
            title,
            path: comment_path(&found.subject.info.path, comment.id),
            image_url: None,
            // Only what is in public view can be liked; a hidden comment
            // can still be reported by those who see it.
            interactions_enabled: comment.is_published() && found.subject.info.interactions_enabled,
        }))
    }

    // Like and dislike counts of comments are kept by the reaction's own
    // transaction (see `reactions`), so there is nothing to apply here.
}

/// Users as subjects, for reports: any account that has not been erased,
/// by UUID or handle.
struct UserSubjects {
    core: Core,
}

#[async_trait]
impl SubjectResolver for UserSubjects {
    async fn resolve(&self, id: &str, _viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let user = match Uuid::parse_str(id) {
            Ok(user_id) => users::find_by_id(&self.core, user_id).await?,
            Err(_) => match users::normalize_handle(id) {
                Ok(handle) => users::find_by_handle(&self.core, &handle).await?,
                Err(_) => None,
            },
        };
        let Some(user) = user.filter(|user| user.deleted_at.is_none()) else { return Ok(None) };
        Ok(Some(SubjectInfo {
            id: user.id,
            owner_id: user.id,
            title: if user.display_name.is_empty() { format!("@{}", user.handle) } else { user.display_name.clone() },
            path: format!("/u/{}", user.handle),
            image_url: image_from_json(user.avatar.as_ref()).map(|image| image.url).filter(|url| !url.is_empty()),
            // An account is reported, saved or shared — not liked or commented on.
            interactions_enabled: false,
        }))
    }
}

/// Only people who may see a subject may follow its topic: comments on a
/// friends-only note must not reach everyone who knows the note's ID.
struct SubjectTopics(Engagement);

#[async_trait]
impl TopicGuard for SubjectTopics {
    async fn may_subscribe(&self, subscriber: Option<Uuid>, topic: &str) -> AppResult<bool> {
        let Some(key) = SubjectKey::from_topic(topic) else { return Ok(false) };
        match self.0.subject(key.subject_type(), &key.id.to_string(), subscriber).await {
            Ok(_) => Ok(true),
            Err(error) if is_absent(&error) => Ok(false),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topics_round_trip() {
        let key = SubjectKey::new(SubjectType::SUBJECT_TYPE_VIDEO, Uuid::now_v7());
        assert_eq!(key.topic(), format!("subject:5:{}", key.id));
        assert_eq!(SubjectKey::from_topic(&key.topic()), Some(key));
        for topic in ["subject:5", "live:5:x", "subject:x:y", &format!("subject:5:{}:extra", key.id), ""] {
            assert_eq!(SubjectKey::from_topic(topic), None, "{topic}");
        }
    }

    #[test]
    fn unknown_type_numbers_read_as_unspecified() {
        let key = SubjectKey { kind: 9999, id: Uuid::nil() };
        assert_eq!(key.subject_type(), SubjectType::SUBJECT_TYPE_UNSPECIFIED);
        assert!(SubjectKey::of_comment(Uuid::nil()).is_comment());
    }

    #[test]
    fn requested_types_must_be_known_and_set() {
        assert_eq!(requested_type(SubjectType::SUBJECT_TYPE_NOTE.into()).unwrap(), SubjectType::SUBJECT_TYPE_NOTE);
        assert_eq!(requested_type(SubjectType::SUBJECT_TYPE_UNSPECIFIED.into()).unwrap_err().reason, "UNSUPPORTED_SUBJECT");
        assert_eq!(requested_type(EnumValue::from(9999)).unwrap_err().reason, "UNSUPPORTED_SUBJECT");
        assert_eq!(requested(None).unwrap_err().reason, "UNSUPPORTED_SUBJECT");
    }

    #[test]
    fn comment_paths_extend_the_subject_path() {
        let id = Uuid::nil();
        assert_eq!(comment_path("/watch/abc", id), format!("/watch/abc?comment={id}"));
        assert_eq!(comment_path("/watch/abc?t=10", id), format!("/watch/abc?t=10&comment={id}"));
    }

    #[test]
    fn ownerless_subjects_have_no_owner() {
        let subject = |owner_id| Subject {
            key: SubjectKey::new(SubjectType::SUBJECT_TYPE_STAMP_PACK, Uuid::now_v7()),
            info: SubjectInfo {
                id: Uuid::now_v7(),
                owner_id,
                title: "t".into(),
                path: "/p".into(),
                image_url: None,
                interactions_enabled: true,
            },
        };
        let user = Uuid::now_v7();
        assert!(subject(user).is_owned_by(user));
        assert_eq!(subject(Uuid::nil()).owner(), None);
        assert!(!subject(Uuid::nil()).is_owned_by(Uuid::nil()));
        assert!(!preview_of(subject(user).key, None).available);
        assert!(preview_of(subject(user).key, Some(&subject(user))).available);
    }
}
