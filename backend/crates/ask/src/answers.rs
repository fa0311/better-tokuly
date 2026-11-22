//! Answers: writing, editing and removing them, and what a publication
//! sets off (search document, activity entry, the event the worker turns
//! into the asker's notification).
//!
//! Nothing in this module reads who sent a question. Telling the asker is
//! the delivery worker's job (`delivery.rs`), started by the
//! `ask.answer.published` event, so the owner's request never touches the
//! sender's ID.
use std::time::Duration;

use buffa::EnumValue;
use sqlx::{Postgres, Transaction};
use tokuly_core::{
    events,
    outbound::{self, ActivityAudience, ActivityEntry, Localized, SearchDoc},
    prelude::*,
};
use tokuly_media::assets::{self, image_from_json};
use tokuly_proto::{
    ask::{AnswerAudience, UpdateAnswerRequest},
    common::SubjectType,
    media::AssetPurpose,
};
use tokuly_stamp::SendContext;

use crate::{
    Ask,
    events::{ANSWER_DELETED, ANSWER_PUBLISHED, AnswerDeleted, AnswerPublished},
    questions::{
        ANSWERED, FRIENDS, PRIVATE, PUBLIC, QuestionRow, REMOVED_BY_OWNER, in_box, question_columns, question_path,
    },
    text,
};

/// Uploads that may be attached to a question or an answer.
pub(crate) const IMAGE_PURPOSES: &[AssetPurpose] =
    &[AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE, AssetPurpose::ASSET_PURPOSE_CHAT_IMAGE];

/// Kind of the search documents of public answers.
pub(crate) const SEARCH_KIND: &str = "answer";
/// Kind of the activity entry of a published answer.
const ACTIVITY_KIND: &str = "ask.answered";
/// Answers one owner may write or edit per hour.
const ANSWERS_PER_HOUR: u32 = 120;

/// What holds the image of a question in the media domain.
pub(crate) fn question_holder(question_id: Uuid) -> String {
    format!("ask:question:{question_id}")
}

/// What holds the image of an answer in the media domain.
pub(crate) fn answer_holder(question_id: Uuid) -> String {
    format!("ask:answer:{question_id}")
}

/// The audience of a request; `fallback` when it is unspecified.
fn audience_of(value: EnumValue<AnswerAudience>, fallback: Option<i16>) -> AppResult<i16> {
    match value.as_known() {
        Some(AnswerAudience::ANSWER_AUDIENCE_PUBLIC) => Ok(PUBLIC),
        Some(AnswerAudience::ANSWER_AUDIENCE_FRIENDS) => Ok(FRIENDS),
        Some(AnswerAudience::ANSWER_AUDIENCE_PRIVATE) => Ok(PRIVATE),
        Some(AnswerAudience::ANSWER_AUDIENCE_UNSPECIFIED) if fallback.is_some() => Ok(fallback.unwrap_or(PUBLIC)),
        _ => Err(AppError::invalid(
            "AUDIENCE_INVALID",
            "choose who may read the answer: everyone, friends, or only the asker",
        )),
    }
}

/// An answer as it is about to be stored.
struct Draft {
    text: String,
    /// The image's asset ID and its `Image` as stored JSON.
    image: Option<(Uuid, serde_json::Value)>,
    stamp_id: Option<Uuid>,
    audience: i16,
}

impl Draft {
    fn check(&self) -> AppResult<()> {
        ensure!(
            !(self.image.is_some() && self.stamp_id.is_some()),
            invalid,
            "ANSWER_ATTACHMENT_CONFLICT",
            "an answer has an image or a stamp, not both"
        );
        ensure!(
            !self.text.is_empty() || self.image.is_some() || self.stamp_id.is_some(),
            invalid,
            "ANSWER_EMPTY",
            "an answer needs text, an image or a stamp"
        );
        Ok(())
    }
}

impl Ask {
    async fn limit_answers(&self, owner: Uuid) -> AppResult<()> {
        self.core.cache.rate_limit(&format!("ask:answer:{owner}"), ANSWERS_PER_HOUR, Duration::from_secs(3600)).await
    }

    /// An uploaded image of `owner`, ready to be stored.
    async fn answer_image(&self, owner: Uuid, asset_id: Uuid) -> AppResult<(Uuid, serde_json::Value)> {
        Ok((asset_id, assets::require_image_json(&self.core, asset_id, owner, IMAGE_PURPOSES).await?))
    }

    /// Answers a question of `owner`'s box.
    pub(crate) async fn write_answer(
        &self,
        owner: Uuid,
        question_id: Uuid,
        text: &str,
        image_asset_id: &str,
        stamp_id: &str,
        audience: EnumValue<AnswerAudience>,
    ) -> AppResult<QuestionRow> {
        let question = self.owned_question(owner, question_id).await?;
        ensure!(
            question.folder != ANSWERED,
            precondition,
            "QUESTION_ALREADY_ANSWERED",
            "this question already has an answer; edit it instead"
        );
        let draft = Draft {
            text: text::optional(text, text::ANSWER_MAX, "ANSWER_TEXT_INVALID", "the answer")?,
            image: match parse_opt_id(image_asset_id)? {
                Some(asset_id) => Some(self.answer_image(owner, asset_id).await?),
                None => None,
            },
            stamp_id: match parse_opt_id(stamp_id)? {
                Some(stamp_id) => {
                    Some(tokuly_stamp::ensure_usable(&self.core, owner, stamp_id).await?).map(|_| stamp_id)
                }
                None => None,
            },
            audience: audience_of(audience, Some(PUBLIC))?,
        };
        draft.check()?;
        self.limit_answers(owner).await?;

        let mut tx = self.core.db.begin().await?;
        let answered: Option<QuestionRow> = sqlx::query_as(concat!(
            "UPDATE ask_questions q
             SET folder = 2, answer_text = $3, answer_image = $4, answer_image_asset_id = $5, answer_stamp_id = $6,
                 answer_audience = $7, answered_at = now(), answer_edited_at = NULL, read_at = COALESCE(q.read_at, now())
             WHERE q.owner_id = $1 AND q.id = $2 AND q.folder <> 2 AND ",
            in_box!(),
            " RETURNING ",
            question_columns!()
        ))
        .bind(owner)
        .bind(question_id)
        .bind(&draft.text)
        .bind(draft.image.as_ref().map(|(_, image)| image))
        .bind(draft.image.as_ref().map(|(asset_id, _)| *asset_id))
        .bind(draft.stamp_id)
        .bind(draft.audience)
        .fetch_optional(&mut *tx)
        .await?;
        // Someone else (another device) answered between the check and here.
        let answered = answered.ok_or_else(|| {
            AppError::precondition("QUESTION_ALREADY_ANSWERED", "this question already has an answer; edit it instead")
        })?;
        if let Some((asset_id, _)) = &draft.image {
            assets::attach(&mut *tx, *asset_id, &answer_holder(question_id)).await?;
        }
        self.announce_answer(&mut tx, &answered, Change::Answered).await?;
        tx.commit().await?;

        if let Some(stamp_id) = draft.stamp_id {
            self.record_answer_stamp(owner, stamp_id).await;
        }
        Ok(answered)
    }

    /// Edits the answer to a question of `owner`'s box; unset fields stay.
    pub(crate) async fn edit_answer(&self, owner: Uuid, request: &UpdateAnswerRequest) -> AppResult<QuestionRow> {
        let question_id = parse_id(&request.question_id)?;
        let current = self.owned_question(owner, question_id).await?;
        ensure!(current.folder == ANSWERED, precondition, "QUESTION_NOT_ANSWERED", "this question has no answer yet");
        let kept_image = current.answer_image_asset_id.zip(current.answer_image.clone());
        let draft = Draft {
            text: match &request.text {
                Some(text) => text::optional(text, text::ANSWER_MAX, "ANSWER_TEXT_INVALID", "the answer")?,
                None => current.answer_text.clone(),
            },
            image: match request.image_asset_id.as_deref().map(parse_opt_id).transpose()? {
                None => kept_image.clone(),
                Some(None) => None,
                Some(Some(asset_id)) if current.answer_image_asset_id == Some(asset_id) => kept_image.clone(),
                Some(Some(asset_id)) => Some(self.answer_image(owner, asset_id).await?),
            },
            stamp_id: match request.stamp_id.as_deref().map(parse_opt_id).transpose()? {
                None => current.answer_stamp_id,
                Some(None) => None,
                Some(Some(stamp_id)) if current.answer_stamp_id == Some(stamp_id) => Some(stamp_id),
                Some(Some(stamp_id)) => {
                    Some(tokuly_stamp::ensure_usable(&self.core, owner, stamp_id).await?).map(|_| stamp_id)
                }
            },
            audience: match request.audience {
                Some(audience) => audience_of(audience, None)?,
                None => current.answer_audience,
            },
        };
        draft.check()?;
        self.limit_answers(owner).await?;
        let new_image_id = draft.image.as_ref().map(|(asset_id, _)| *asset_id);
        let content_changed = draft.text != current.answer_text
            || new_image_id != current.answer_image_asset_id
            || draft.stamp_id != current.answer_stamp_id;

        let mut tx = self.core.db.begin().await?;
        let updated: Option<QuestionRow> = sqlx::query_as(concat!(
            "UPDATE ask_questions q
             SET answer_text = $3, answer_image = $4, answer_image_asset_id = $5, answer_stamp_id = $6, answer_audience = $7,
                 answer_edited_at = CASE WHEN $8 THEN now() ELSE q.answer_edited_at END
             WHERE q.owner_id = $1 AND q.id = $2 AND q.folder = 2 AND ",
            in_box!(),
            " RETURNING ",
            question_columns!()
        ))
        .bind(owner)
        .bind(question_id)
        .bind(&draft.text)
        .bind(draft.image.as_ref().map(|(_, image)| image))
        .bind(new_image_id)
        .bind(draft.stamp_id)
        .bind(draft.audience)
        .bind(content_changed)
        .fetch_optional(&mut *tx)
        .await?;
        let updated = updated
            .ok_or_else(|| AppError::precondition("QUESTION_NOT_ANSWERED", "this question has no answer yet"))?;
        if new_image_id != current.answer_image_asset_id {
            let holder = answer_holder(question_id);
            if let Some(old) = current.answer_image_asset_id {
                assets::detach(&mut *tx, old, &holder).await?;
            }
            if let Some(new) = new_image_id {
                assets::attach(&mut *tx, new, &holder).await?;
            }
        }
        self.announce_answer(&mut tx, &updated, Change::Edited { before: &current }).await?;
        tx.commit().await?;

        if let Some(stamp_id) = draft.stamp_id.filter(|stamp_id| current.answer_stamp_id != Some(*stamp_id)) {
            self.record_answer_stamp(owner, stamp_id).await;
        }
        Ok(updated)
    }

    /// Removes the answer; the question goes back to the inbox.
    pub(crate) async fn clear_answer(&self, owner: Uuid, question_id: Uuid) -> AppResult<QuestionRow> {
        let current = self.owned_question(owner, question_id).await?;
        ensure!(current.folder == ANSWERED, precondition, "QUESTION_NOT_ANSWERED", "this question has no answer");
        let mut tx = self.core.db.begin().await?;
        let cleared: Option<QuestionRow> = sqlx::query_as(concat!(
            "UPDATE ask_questions q
             SET folder = 1, answer_text = '', answer_image = NULL, answer_image_asset_id = NULL, answer_stamp_id = NULL,
                 answer_audience = 0, answered_at = NULL, answer_edited_at = NULL
             WHERE q.owner_id = $1 AND q.id = $2 AND q.folder = 2 AND ",
            in_box!(),
            " RETURNING ",
            question_columns!()
        ))
        .bind(owner)
        .bind(question_id)
        .fetch_optional(&mut *tx)
        .await?;
        let cleared =
            cleared.ok_or_else(|| AppError::precondition("QUESTION_NOT_ANSWERED", "this question has no answer"))?;
        self.retract_answer(&mut tx, &current, Some(owner)).await?;
        tx.commit().await?;
        Ok(cleared)
    }

    /// Deletes a question from `owner`'s box, with its answer. The row
    /// stays (see `removed_by` in the migration); its sender keeps seeing
    /// the question as waiting.
    pub(crate) async fn remove_question(&self, owner: Uuid, question_id: Uuid) -> AppResult<()> {
        let current = self.owned_question(owner, question_id).await?;
        let mut tx = self.core.db.begin().await?;
        let removed = sqlx::query(concat!(
            "UPDATE ask_questions q SET removed_at = now(), removed_by = $3 WHERE q.owner_id = $1 AND q.id = $2 AND ",
            in_box!()
        ))
        .bind(owner)
        .bind(question_id)
        .bind(REMOVED_BY_OWNER)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        ensure!(removed == 1, not_found, "QUESTION_NOT_FOUND", "question not found");
        if current.is_answered() {
            self.retract_answer(&mut tx, &current, Some(owner)).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Undoes what publishing `answered` set off: the hold on its image,
    /// its search document, and an `ask.answer.deleted` event.
    pub(crate) async fn retract_answer(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        answered: &QuestionRow,
        actor: Option<Uuid>,
    ) -> AppResult<()> {
        assets::detach_holder(&mut **tx, &answer_holder(answered.id)).await?;
        if answered.answer_audience == PUBLIC {
            outbound::unindex(&mut **tx, SEARCH_KIND, answered.id).await?;
        }
        events::emit(
            &mut **tx,
            ANSWER_DELETED,
            actor,
            &AnswerDeleted { question_id: answered.id, owner_id: answered.owner_id },
        )
        .await
    }

    /// Tells the generic domains about a new or edited answer.
    async fn announce_answer(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        answer: &QuestionRow,
        change: Change<'_>,
    ) -> AppResult<()> {
        let (first, was_public, was_published, audience_changed) = match change {
            Change::Answered => (true, false, false, true),
            Change::Edited { before } => {
                (false, before.is_public(), before.is_published(), before.answer_audience != answer.answer_audience)
            }
        };
        if audience_changed {
            let event = AnswerPublished {
                question_id: answer.id,
                owner_id: answer.owner_id,
                audience: i32::from(answer.answer_audience),
                first,
            };
            events::emit(&mut **tx, ANSWER_PUBLISHED, Some(answer.owner_id), &event).await?;
        }
        if answer.is_public() {
            outbound::index(&mut **tx, search_doc(answer)).await?;
        } else if was_public {
            outbound::unindex(&mut **tx, SEARCH_KIND, answer.id).await?;
        }
        // Once per publication: when the answer first appears on the page.
        if answer.is_published() && !was_published {
            let audience = if answer.is_public() { ActivityAudience::Public } else { ActivityAudience::Friends };
            let entry = ActivityEntry::new(
                answer.owner_id,
                ACTIVITY_KIND,
                audience,
                Localized::new("質問に答えたよ!", "Answered a question"),
            )
            .subject(SubjectType::SUBJECT_TYPE_QUESTION, answer.id)
            .path(question_path(answer.id))
            .data(serde_json::json!({ "question": text::excerpt(&answer.text, text::PREVIEW_CHARS) }));
            let entry = match answer_image_url(answer) {
                Some(url) => entry.image(url),
                None => entry,
            };
            outbound::activity(&mut **tx, &entry).await?;
        }
        Ok(())
    }

    /// Feeds the stamp domain's usage log; never fails the answer.
    async fn record_answer_stamp(&self, owner: Uuid, stamp_id: Uuid) {
        // An answer is a post on the owner's page: the closest context the
        // stamp domain knows is a note.
        if let Err(e) = tokuly_stamp::record_send(&self.core, owner, stamp_id, SendContext::Note).await {
            tracing::warn!(%owner, %stamp_id, error = %e, "recording the stamp of an answer failed");
        }
    }
}

/// What happened to an answer, for [`Ask::announce_answer`].
enum Change<'a> {
    Answered,
    Edited { before: &'a QuestionRow },
}

fn answer_image_url(answer: &QuestionRow) -> Option<String> {
    image_from_json(answer.answer_image.as_ref()).map(|image| image.url).filter(|url| !url.is_empty())
}

/// The search document of a public answer: found by the question or by the
/// answer.
pub(crate) fn search_doc(answer: &QuestionRow) -> SearchDoc {
    SearchDoc {
        kind: SEARCH_KIND.to_owned(),
        id: answer.id,
        title: text::excerpt(&answer.text, 120),
        text: answer.answer_text.clone(),
        tags: Vec::new(),
        owner_id: Some(answer.owner_id),
        public: answer.is_public(),
        image_url: answer_image_url(answer),
        path: question_path(answer.id),
        created_at: answer.answered_at.unwrap_or(answer.created_at).timestamp(),
        score: f64::from(answer.like_count),
        extra: serde_json::json!({ "anonymous": !answer.signed, "comment_count": answer.comment_count }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(text: &str, image: bool, stamp: bool) -> Draft {
        Draft {
            text: text.to_owned(),
            image: image.then(|| (new_id(), serde_json::json!({}))),
            stamp_id: stamp.then(new_id),
            audience: PUBLIC,
        }
    }

    #[test]
    fn an_answer_needs_content_and_at_most_one_attachment() {
        assert!(draft("はい!", false, false).check().is_ok());
        assert!(draft("", true, false).check().is_ok());
        assert!(draft("", false, true).check().is_ok());
        assert_eq!(draft("", false, false).check().unwrap_err().reason, "ANSWER_EMPTY");
        assert_eq!(draft("両方", true, true).check().unwrap_err().reason, "ANSWER_ATTACHMENT_CONFLICT");
    }

    #[test]
    fn unspecified_audience_falls_back_only_where_allowed() {
        let unspecified: EnumValue<AnswerAudience> = AnswerAudience::ANSWER_AUDIENCE_UNSPECIFIED.into();
        assert_eq!(audience_of(unspecified, Some(PUBLIC)).unwrap(), PUBLIC);
        assert_eq!(audience_of(unspecified, None).unwrap_err().reason, "AUDIENCE_INVALID");
        assert_eq!(audience_of(AnswerAudience::ANSWER_AUDIENCE_PRIVATE.into(), None).unwrap(), PRIVATE);
        assert_eq!(audience_of(EnumValue::from(99), Some(PUBLIC)).unwrap_err().reason, "AUDIENCE_INVALID");
    }

    #[test]
    fn holders_name_the_question() {
        let id = new_id();
        assert_eq!(question_holder(id), format!("ask:question:{id}"));
        assert_eq!(answer_holder(id), format!("ask:answer:{id}"));
    }
}
