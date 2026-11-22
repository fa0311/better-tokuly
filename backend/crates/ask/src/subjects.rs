//! The subject resolver: how questions, prompts and prompt responses look
//! to the generic domains (comments, likes, bookmarks, reports, takedowns).
//!
//! All three resolve under `SUBJECT_TYPE_QUESTION`: there is no subject type
//! for prompts, and anything anonymous must be reportable.
//!
//! # Who the "owner" is
//!
//! The generic domains use a subject's owner for three things: who is told
//! about likes and comments, who may moderate the comments, and who a
//! report is about. They also hide a subject from anyone in a block with
//! its owner. That last rule is why the owner must be chosen with care: if
//! the recipient of an anonymous question were told (even only internally)
//! that its owner is the sender, they could find the sender out by blocking
//! people one at a time and watching the question disappear.
//!
//! So the owner depends on who is looking — see [`decide_question`]:
//!
//! - **published** (an answer on the Q&A page): the box owner, for everyone.
//!   They published it; likes and comments are theirs.
//! - **not published, seen by its recipient**: nobody (the nil UUID). The
//!   recipient can report it, and learns nothing.
//! - **not published, seen by staff**: the sender. This is how a takedown
//!   reaches the account that wrote an anonymous question. Every time it
//!   happens it is recorded in `ask_inspections`, like a lookup through
//!   `AskModerationService`.
//!
//! A moderator who is the recipient or the sender of a question is treated
//! as that, not as staff: nobody moderates their own box.
use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use tokuly_core::{
    prelude::*,
    subject::{SubjectInfo, SubjectResolver},
};
use tokuly_media::assets::image_from_json;
use tokuly_proto::common::SubjectType;
use tokuly_social::graph;

use crate::{
    prompts::{prompt_path, response_in_box},
    questions::{ANSWERED, FRIENDS, PUBLIC, REMOVED_BY_MODERATION, in_box, question_path},
    text,
};

/// `SubjectInfo::owner_id` of something that, for this viewer, belongs to
/// nobody: no notifications, no moderation rights, no block check.
const NO_OWNER: Uuid = Uuid::nil();
/// Characters of a text used as a subject's title in reports and notices.
const TITLE_CHARS: usize = 80;
/// How often one staff member's view of one item is recorded.
const INSPECTION_TRACE_EVERY: Duration = Duration::from_secs(3600);

/// Registers the resolver. Called once from `Ask::new`.
pub(crate) fn register(core: &Core) {
    core.subjects.register(SubjectType::SUBJECT_TYPE_QUESTION, Arc::new(AskSubjects { core: core.clone() }));
}

struct AskSubjects {
    core: Core,
}

/// Who is looking at a subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Viewer {
    /// The person it was sent to (the box or prompt owner).
    Recipient,
    /// The person who sent it.
    Sender,
    /// A moderator or admin who is neither of the two.
    Staff,
    /// A friend of the recipient.
    Friend,
    /// Anyone else, signed in or not.
    Other,
}

/// What the resolver answers for one viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Decision {
    pub owner: Owner,
    pub interactions: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Owner {
    /// The box or prompt owner.
    Recipient,
    /// Whoever sent it; only ever answered to staff.
    Sender,
    Nobody,
}

/// The two kinds of anonymous items the resolver can name a sender of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Question,
    PromptResponse,
}

impl Item {
    fn sender_query(self) -> &'static str {
        match self {
            Self::Question => "SELECT sender_id FROM ask_questions WHERE id = $1",
            Self::PromptResponse => "SELECT sender_id FROM ask_prompt_responses WHERE id = $1",
        }
    }

    /// `ask_inspections.kind`, the same names `AskModerationService` writes.
    fn kind(self) -> &'static str {
        match self {
            Self::Question => "question",
            Self::PromptResponse => "prompt_response",
        }
    }
}

/// The facts about a question the decision needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct QuestionFacts {
    /// In the recipient's box: not removed, not dropped, sender not banned.
    pub in_box: bool,
    /// Removed by moderation: gone for everyone.
    pub taken_down: bool,
    pub answered: bool,
    /// `tokuly.ask.v1.AnswerAudience` of the answer.
    pub audience: i16,
}

impl QuestionFacts {
    /// On the Q&A page, for whoever its audience includes.
    fn published(&self) -> bool {
        self.in_box && self.answered && (self.audience == PUBLIC || self.audience == FRIENDS)
    }
}

/// Whether, and as what, `viewer` gets to see a question.
pub(crate) fn decide_question(viewer: Viewer, facts: QuestionFacts) -> Option<Decision> {
    if facts.taken_down {
        return None;
    }
    let published = facts.published();
    let on_page = Decision { owner: Owner::Recipient, interactions: true };
    match viewer {
        Viewer::Recipient if !facts.in_box => None,
        Viewer::Recipient if published => Some(on_page),
        // An incoming question, or one answered privately: reportable, and
        // owned by nobody as far as its recipient can tell.
        Viewer::Recipient => Some(Decision { owner: Owner::Nobody, interactions: false }),
        // The sender sees an answer to their question, never the bare
        // question (there is nothing to like or report in their own words).
        // On a private answer nobody else can interact, so an interaction
        // would name the sender: none are allowed.
        Viewer::Sender if facts.in_box && facts.answered => {
            Some(Decision { owner: Owner::Recipient, interactions: published })
        }
        Viewer::Sender => None,
        Viewer::Staff if published => Some(on_page),
        // Also when its recipient deleted it or its sender withdrew it: a
        // report filed before must still reach the account that wrote it.
        Viewer::Staff => Some(Decision { owner: Owner::Sender, interactions: false }),
        Viewer::Friend if published => Some(on_page),
        Viewer::Other if published && facts.audience == PUBLIC => Some(on_page),
        Viewer::Friend | Viewer::Other => None,
    }
}

/// The facts about a prompt response the decision needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResponseFacts {
    /// In the prompt owner's view: not removed, not dropped, sender not
    /// banned, and its prompt still exists.
    pub in_box: bool,
    pub taken_down: bool,
    /// On the prompt's page (published and not held).
    pub published: bool,
}

/// Whether, and as what, `viewer` gets to see a prompt response. Responses
/// can be reported, never liked or commented on.
pub(crate) fn decide_response(viewer: Viewer, facts: ResponseFacts) -> Option<Decision> {
    if facts.taken_down {
        return None;
    }
    let none = |owner| Some(Decision { owner, interactions: false });
    match viewer {
        Viewer::Recipient if !facts.in_box => None,
        // The prompt owner put it on the page: it is theirs now, and they
        // cannot report what they published.
        Viewer::Recipient if facts.published => none(Owner::Recipient),
        Viewer::Recipient => none(Owner::Nobody),
        Viewer::Staff if facts.in_box && facts.published => none(Owner::Recipient),
        Viewer::Staff => none(Owner::Sender),
        Viewer::Friend if facts.in_box && facts.published => none(Owner::Recipient),
        // Their own words: nothing to report.
        Viewer::Sender | Viewer::Friend | Viewer::Other => None,
    }
}

#[derive(sqlx::FromRow)]
struct QuestionSubject {
    owner_id: Uuid,
    text: String,
    answer_image: Option<serde_json::Value>,
    in_box: bool,
    taken_down: bool,
    answered: bool,
    audience: i16,
    sent_by_viewer: bool,
}

#[derive(sqlx::FromRow)]
struct PromptSubject {
    owner_id: Uuid,
    text: String,
}

#[derive(sqlx::FromRow)]
struct ResponseSubject {
    owner_id: Uuid,
    prompt_id: Uuid,
    text: String,
    in_box: bool,
    taken_down: bool,
    published: bool,
    sent_by_viewer: bool,
}

impl AskSubjects {
    /// Whether `user_id` is an active moderator or admin.
    async fn is_staff(&self, user_id: Uuid) -> AppResult<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL AND banned_at IS NULL
                           AND roles && ARRAY['admin', 'moderator']::text[])",
        )
        .bind(user_id)
        .fetch_one(&self.core.db)
        .await?)
    }

    /// What `viewer` is to something sent to `recipient`.
    async fn viewer_of(&self, viewer: Option<Uuid>, recipient: Uuid, sent_by_viewer: bool) -> AppResult<Viewer> {
        let Some(viewer) = viewer else { return Ok(Viewer::Other) };
        if viewer == recipient {
            return Ok(Viewer::Recipient);
        }
        if sent_by_viewer {
            return Ok(Viewer::Sender);
        }
        if self.is_staff(viewer).await? {
            return Ok(Viewer::Staff);
        }
        Ok(if graph::are_friends(&self.core, recipient, viewer).await? { Viewer::Friend } else { Viewer::Other })
    }

    /// The owner to report for a decision. Handing the sender to staff
    /// leaves a trace first; if the trace cannot be written the subject
    /// does not resolve.
    async fn owner_id(
        &self,
        owner: Owner,
        recipient: Uuid,
        staff: Option<Uuid>,
        item: Item,
        id: Uuid,
    ) -> AppResult<Uuid> {
        match owner {
            Owner::Recipient => Ok(recipient),
            Owner::Nobody => Ok(NO_OWNER),
            Owner::Sender => {
                let Some(staff) = staff else { return Ok(NO_OWNER) };
                // The one place outside `moderation_svc.rs` and `hints.rs`
                // that reads the sender of an anonymous item.
                let sender: Option<Uuid> =
                    sqlx::query_scalar(item.sender_query()).bind(id).fetch_optional(&self.core.db).await?.flatten();
                let Some(sender) = sender else { return Ok(NO_OWNER) };
                // Generic domains resolve a subject on every request: one
                // trace per staff member, item and hour says enough. The
                // row is written before the marker that lets later calls
                // skip it, so a failed write is never mistaken for a trace.
                let marker = format!("ask:inspected:{staff}:{id}");
                if !self.core.cache.exists(&marker).await? {
                    sqlx::query("INSERT INTO ask_inspections (staff_id, kind, target_id) VALUES ($1, $2, $3)")
                        .bind(staff)
                        .bind(item.kind())
                        .bind(id)
                        .execute(&self.core.db)
                        .await?;
                    self.core.cache.set_string(&marker, "1", Some(INSPECTION_TRACE_EVERY)).await?;
                }
                Ok(sender)
            }
        }
    }

    async fn resolve_question(&self, id: Uuid, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        // "In the box" is the same condition the inbox itself uses.
        let row: Option<QuestionSubject> = sqlx::query_as(concat!(
            "SELECT q.owner_id, q.text, q.answer_image, (",
            in_box!(),
            ") AS in_box,
                    (q.removed_by IS NOT DISTINCT FROM $3) AS taken_down,
                    (q.folder = $4) AS answered, q.answer_audience AS audience,
                    COALESCE(q.sender_id = $2, false) AS sent_by_viewer
             FROM ask_questions q WHERE q.id = $1"
        ))
        .bind(id)
        .bind(viewer)
        .bind(REMOVED_BY_MODERATION)
        .bind(ANSWERED)
        .fetch_optional(&self.core.db)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let who = self.viewer_of(viewer, row.owner_id, row.sent_by_viewer).await?;
        let facts = QuestionFacts {
            in_box: row.in_box,
            taken_down: row.taken_down,
            answered: row.answered,
            audience: row.audience,
        };
        let Some(decision) = decide_question(who, facts) else { return Ok(None) };
        let staff = viewer.filter(|_| who == Viewer::Staff);
        Ok(Some(SubjectInfo {
            id,
            owner_id: self.owner_id(decision.owner, row.owner_id, staff, Item::Question, id).await?,
            title: text::excerpt(&row.text, TITLE_CHARS),
            path: question_path(id),
            image_url: image_from_json(row.answer_image.as_ref())
                .map(|image| image.url)
                .filter(|url| !url.is_empty() && facts.published()),
            interactions_enabled: decision.interactions,
        }))
    }

    /// A prompt: its owner's own, signed words. Friends and staff can
    /// report it; nobody likes or comments on it (responses are the
    /// interaction).
    async fn resolve_prompt(&self, id: Uuid, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let row: Option<PromptSubject> =
            sqlx::query_as("SELECT owner_id, text FROM ask_prompts WHERE id = $1 AND removed_at IS NULL")
                .bind(id)
                .fetch_optional(&self.core.db)
                .await?;
        let Some(row) = row else { return Ok(None) };
        let who = self.viewer_of(viewer, row.owner_id, false).await?;
        if !matches!(who, Viewer::Recipient | Viewer::Staff | Viewer::Friend) {
            return Ok(None);
        }
        Ok(Some(SubjectInfo {
            id,
            owner_id: row.owner_id,
            title: text::excerpt(&row.text, TITLE_CHARS),
            path: prompt_path(id),
            image_url: None,
            interactions_enabled: false,
        }))
    }

    async fn resolve_response(&self, id: Uuid, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        // The owner's view of responses, and a prompt that still exists.
        let row: Option<ResponseSubject> = sqlx::query_as(concat!(
            "SELECT r.owner_id, r.prompt_id, r.text, (",
            response_in_box!(),
            " AND EXISTS (SELECT 1 FROM ask_prompts p WHERE p.id = r.prompt_id AND p.removed_at IS NULL)) AS in_box,
                    (r.removed_by IS NOT DISTINCT FROM $3) AS taken_down,
                    (r.published_at IS NOT NULL AND NOT r.held) AS published,
                    COALESCE(r.sender_id = $2, false) AS sent_by_viewer
             FROM ask_prompt_responses r WHERE r.id = $1"
        ))
        .bind(id)
        .bind(viewer)
        .bind(REMOVED_BY_MODERATION)
        .fetch_optional(&self.core.db)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let who = self.viewer_of(viewer, row.owner_id, row.sent_by_viewer).await?;
        let facts = ResponseFacts { in_box: row.in_box, taken_down: row.taken_down, published: row.published };
        let Some(decision) = decide_response(who, facts) else { return Ok(None) };
        let staff = viewer.filter(|_| who == Viewer::Staff);
        Ok(Some(SubjectInfo {
            id,
            owner_id: self.owner_id(decision.owner, row.owner_id, staff, Item::PromptResponse, id).await?,
            title: text::excerpt(&row.text, TITLE_CHARS),
            path: prompt_path(row.prompt_id),
            image_url: None,
            interactions_enabled: false,
        }))
    }
}

#[async_trait]
impl SubjectResolver for AskSubjects {
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        // A malformed ID names nothing; that is "not found", not a client error.
        let Ok(id) = Uuid::parse_str(id.trim()) else { return Ok(None) };
        // IDs are UUIDv7 from three tables; an ID is in at most one of them.
        if let Some(question) = self.resolve_question(id, viewer).await? {
            return Ok(Some(question));
        }
        if let Some(prompt) = self.resolve_prompt(id, viewer).await? {
            return Ok(Some(prompt));
        }
        self.resolve_response(id, viewer).await
    }

    async fn counter_changed(&self, id: Uuid, counter: &str, delta: i64) -> AppResult<()> {
        let sql = match counter {
            "like_count" => {
                "UPDATE ask_questions SET like_count = LEAST(GREATEST(like_count::bigint + $2, 0), 2147483647)::int WHERE id = $1"
            }
            "comment_count" => {
                "UPDATE ask_questions SET comment_count = LEAST(GREATEST(comment_count::bigint + $2, 0), 2147483647)::int WHERE id = $1"
            }
            _ => return Ok(()),
        };
        sqlx::query(sql).bind(id).bind(delta).execute(&self.core.db).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::questions::PRIVATE;

    const VIEWERS: [Viewer; 5] = [Viewer::Recipient, Viewer::Sender, Viewer::Staff, Viewer::Friend, Viewer::Other];

    fn incoming() -> QuestionFacts {
        QuestionFacts { in_box: true, taken_down: false, answered: false, audience: 0 }
    }

    fn answered(audience: i16) -> QuestionFacts {
        QuestionFacts { answered: true, audience, ..incoming() }
    }

    #[test]
    fn the_recipient_is_never_told_that_the_sender_owns_a_question() {
        // Whatever the state: the recipient's answer never names the sender.
        for in_box in [true, false] {
            for is_answered in [true, false] {
                for audience in [0, PUBLIC, FRIENDS, PRIVATE] {
                    let facts = QuestionFacts { in_box, taken_down: false, answered: is_answered, audience };
                    if let Some(decision) = decide_question(Viewer::Recipient, facts) {
                        assert_ne!(decision.owner, Owner::Sender, "{facts:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn only_staff_are_ever_told_the_sender() {
        for viewer in VIEWERS {
            for facts in [
                incoming(),
                answered(PUBLIC),
                answered(FRIENDS),
                answered(PRIVATE),
                QuestionFacts { in_box: false, ..incoming() },
            ] {
                if let Some(decision) = decide_question(viewer, facts)
                    && decision.owner == Owner::Sender
                {
                    assert_eq!(viewer, Viewer::Staff, "{viewer:?} was told the sender of {facts:?}");
                }
            }
            for facts in [
                ResponseFacts { in_box: true, taken_down: false, published: false },
                ResponseFacts { in_box: true, taken_down: false, published: true },
                ResponseFacts { in_box: false, taken_down: false, published: false },
            ] {
                if let Some(decision) = decide_response(viewer, facts)
                    && decision.owner == Owner::Sender
                {
                    assert_eq!(viewer, Viewer::Staff, "{viewer:?} was told the sender of {facts:?}");
                }
            }
        }
    }

    #[test]
    fn an_incoming_question_can_be_reported_by_its_recipient_and_seen_by_staff_only() {
        assert_eq!(
            decide_question(Viewer::Recipient, incoming()),
            Some(Decision { owner: Owner::Nobody, interactions: false })
        );
        assert_eq!(
            decide_question(Viewer::Staff, incoming()),
            Some(Decision { owner: Owner::Sender, interactions: false })
        );
        for stranger in [Viewer::Sender, Viewer::Friend, Viewer::Other] {
            assert_eq!(decide_question(stranger, incoming()), None, "{stranger:?}");
        }
    }

    #[test]
    fn a_published_answer_belongs_to_the_box_owner_for_everyone_who_sees_it() {
        let on_page = Some(Decision { owner: Owner::Recipient, interactions: true });
        for viewer in VIEWERS {
            assert_eq!(decide_question(viewer, answered(PUBLIC)), on_page, "{viewer:?}");
        }
        for viewer in [Viewer::Recipient, Viewer::Sender, Viewer::Staff, Viewer::Friend] {
            assert_eq!(decide_question(viewer, answered(FRIENDS)), on_page, "{viewer:?}");
        }
        assert_eq!(
            decide_question(Viewer::Other, answered(FRIENDS)),
            None,
            "a friends-only answer is not for strangers"
        );
    }

    #[test]
    fn a_private_answer_allows_no_interactions_because_they_would_name_the_asker() {
        assert_eq!(
            decide_question(Viewer::Sender, answered(PRIVATE)),
            Some(Decision { owner: Owner::Recipient, interactions: false })
        );
        assert_eq!(
            decide_question(Viewer::Recipient, answered(PRIVATE)),
            Some(Decision { owner: Owner::Nobody, interactions: false })
        );
        assert_eq!(decide_question(Viewer::Friend, answered(PRIVATE)), None);
        assert_eq!(decide_question(Viewer::Other, answered(PRIVATE)), None);
    }

    #[test]
    fn a_question_its_recipient_deleted_still_resolves_for_staff_and_for_nobody_else() {
        let deleted = QuestionFacts { in_box: false, ..incoming() };
        assert_eq!(
            decide_question(Viewer::Staff, deleted),
            Some(Decision { owner: Owner::Sender, interactions: false })
        );
        for viewer in [Viewer::Recipient, Viewer::Sender, Viewer::Friend, Viewer::Other] {
            assert_eq!(decide_question(viewer, deleted), None, "{viewer:?}");
        }
    }

    #[test]
    fn what_moderation_took_down_resolves_for_nobody() {
        for viewer in VIEWERS {
            assert_eq!(
                decide_question(viewer, QuestionFacts { taken_down: true, ..answered(PUBLIC) }),
                None,
                "{viewer:?}"
            );
            assert_eq!(
                decide_response(viewer, ResponseFacts { in_box: false, taken_down: true, published: true }),
                None,
                "{viewer:?}"
            );
        }
    }

    #[test]
    fn the_sender_cannot_tell_a_dropped_question_from_an_ignored_one() {
        // Not answered: nothing for the sender, whatever happened to it.
        let ignored = incoming();
        let dropped = QuestionFacts { in_box: false, ..incoming() };
        assert_eq!(decide_question(Viewer::Sender, ignored), decide_question(Viewer::Sender, dropped));
        assert_eq!(decide_question(Viewer::Sender, ignored), None);
    }

    #[test]
    fn responses_are_reportable_and_never_open_to_interactions() {
        let received = ResponseFacts { in_box: true, taken_down: false, published: false };
        let published = ResponseFacts { published: true, ..received };
        assert_eq!(
            decide_response(Viewer::Recipient, received),
            Some(Decision { owner: Owner::Nobody, interactions: false })
        );
        assert_eq!(
            decide_response(Viewer::Staff, received),
            Some(Decision { owner: Owner::Sender, interactions: false })
        );
        assert_eq!(decide_response(Viewer::Friend, received), None, "friends only see what is on the page");
        assert_eq!(
            decide_response(Viewer::Friend, published),
            Some(Decision { owner: Owner::Recipient, interactions: false })
        );
        assert_eq!(decide_response(Viewer::Other, published), None);
        assert_eq!(decide_response(Viewer::Sender, published), None);
        for viewer in VIEWERS {
            for facts in [received, published] {
                assert!(
                    !decide_response(viewer, facts).is_some_and(|decision| decision.interactions),
                    "{viewer:?} {facts:?}"
                );
            }
        }
    }
}
