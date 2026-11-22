//! What happens to this domain's data when something elsewhere changes: an
//! account is deleted or banned, one user blocks another, moderation takes
//! content down, an upload is deleted. Plus the periodic purge.
//!
//! Every function here is the handler of an event that can arrive twice, so
//! each is safe to repeat.
use tokuly_core::{outbound, prelude::*};
use tokuly_media::assets;
use tokuly_proto::common::SubjectType;
use tokuly_social::events::BlockChanged;

use crate::{
    Ask,
    answers::{SEARCH_KIND, answer_holder, question_holder, search_doc},
    events::ContentRemoved,
    questions::{QuestionRow, REMOVED_BY_MODERATION, in_box, question_columns},
};

/// Rows removed per statement by the purge.
const PURGE_BATCH: i64 = 500;
/// Days a finished round is kept (its questions steer the next rounds).
const ROUND_RETENTION_DAYS: i32 = 30;

/// SQL condition for "this answer has a search document": a public answer
/// in the box.
macro_rules! indexed {
    () => {
        concat!("q.folder = 2 AND q.answer_audience = 1 AND ", in_box!())
    };
}

impl Ask {
    /// `identity.user.deleted`: the user's box, answers, prompts, rounds and
    /// received flames go. What they sent to others stays where it is,
    /// anonymous, with nothing left that points back to them.
    pub(crate) async fn on_user_deleted(&self, user_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;

        // Their box, with its answers.
        let received: Vec<(Uuid, bool, bool, bool)> = sqlx::query_as(concat!(
            "SELECT q.id, (",
            indexed!(),
            "), q.image_asset_id IS NOT NULL, q.answer_image_asset_id IS NOT NULL FROM ask_questions q WHERE q.owner_id = $1"
        ))
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?;
        for (question_id, indexed, has_image, has_answer_image) in received {
            if indexed {
                outbound::unindex(&mut *tx, SEARCH_KIND, question_id).await?;
            }
            if has_image {
                assets::detach_holder(&mut *tx, &question_holder(question_id)).await?;
            }
            if has_answer_image {
                assets::detach_holder(&mut *tx, &answer_holder(question_id)).await?;
            }
        }
        sqlx::query("DELETE FROM ask_questions WHERE owner_id = $1").bind(user_id).execute(&mut *tx).await?;

        // Questions they sent stay, anonymous. A signed one loses its
        // signature and its image, which would both name the sender.
        let sent_with_image: Vec<Uuid> = sqlx::query_scalar(
            "WITH sent AS (SELECT id, image_asset_id IS NOT NULL AS had_image FROM ask_questions WHERE sender_id = $1 FOR UPDATE),
                  cleared AS (UPDATE ask_questions q SET sender_id = NULL, signed = false, image = NULL, image_asset_id = NULL
                              FROM sent WHERE q.id = sent.id)
             SELECT id FROM sent WHERE had_image",
        )
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?;
        for question_id in sent_with_image {
            assets::detach_holder(&mut *tx, &question_holder(question_id)).await?;
        }

        // Blocks they made go; blocks made of them stay, pointing at nobody
        // (a block that vanished would tell its owner that the sender left).
        sqlx::query("DELETE FROM ask_sender_blocks WHERE owner_id = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE ask_sender_blocks SET sender_id = NULL WHERE sender_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;

        // Their prompts go with every response; responses they sent stay, anonymous.
        sqlx::query("DELETE FROM ask_prompts WHERE owner_id = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE ask_prompt_responses SET sender_id = NULL, signed = false WHERE sender_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;

        // Flames they received go; compliments they gave stay with whoever got them.
        sqlx::query("DELETE FROM ask_flames WHERE receiver_id = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE ask_flames SET sender_id = NULL WHERE sender_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query("DELETE FROM ask_rounds WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM ask_poll_exposure WHERE user_id = $1 OR friend_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM ask_friend_links WHERE user_id = $1 OR friend_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM ask_settings WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
        // Staff records keep their rows without the name.
        sqlx::query("UPDATE ask_poll_questions SET created_by = NULL WHERE created_by = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE ask_inspections SET staff_id = NULL WHERE staff_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// `identity.user.banned` / `identity.user.unbanned`: keeps the search
    /// index in step with what the Q&A pages show. (The pages themselves
    /// need nothing: every list leaves out what a banned account sent, for
    /// as long as the ban lasts.)
    pub(crate) async fn on_ban_changed(&self, user_id: Uuid, banned: bool) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        if banned {
            // Public answers in the banned user's box, and public answers to
            // questions they asked.
            let hidden: Vec<Uuid> = sqlx::query_scalar(
                "SELECT id FROM ask_questions WHERE (owner_id = $1 OR sender_id = $1) AND folder = 2 AND answer_audience = 1 AND removed_at IS NULL AND NOT dropped",
            )
            .bind(user_id)
            .fetch_all(&mut *tx)
            .await?;
            for question_id in hidden {
                outbound::unindex(&mut *tx, SEARCH_KIND, question_id).await?;
            }
        } else {
            let shown: Vec<QuestionRow> = sqlx::query_as(concat!(
                "SELECT ",
                question_columns!(),
                " FROM ask_questions q
                  WHERE (q.owner_id = $2 OR q.sender_id = $2) AND ",
                indexed!(),
                " AND EXISTS (SELECT 1 FROM users o WHERE o.id = q.owner_id AND o.banned_at IS NULL AND o.deleted_at IS NULL)"
            ))
            .bind(None::<Uuid>)
            .bind(user_id)
            .fetch_all(&mut *tx)
            .await?;
            for answer in &shown {
                outbound::index(&mut *tx, search_doc(answer)).await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// `social.user.blocked`: what the two sent each other **under their
    /// names** and that is still waiting is withdrawn, and what this domain
    /// remembers about their friendship is forgotten.
    ///
    /// Anonymous items are left exactly where they are, on purpose. If a
    /// block made the blocked person's anonymous questions disappear, the
    /// blocker could find out who wrote a question by blocking suspects one
    /// at a time. From now on the blocked person cannot reach the box at
    /// all (`BOX_NOT_FOUND`), like the rest of the blocker's profile.
    pub(crate) async fn on_user_blocked(&self, event: BlockChanged) -> AppResult<()> {
        let (a, b) = (event.user_id, event.blocked_id);
        sqlx::query(
            "UPDATE ask_questions SET dropped = true
             WHERE signed AND NOT dropped AND folder <> 2 AND removed_at IS NULL
               AND ((owner_id = $1 AND sender_id = $2) OR (owner_id = $2 AND sender_id = $1))",
        )
        .bind(a)
        .bind(b)
        .execute(&self.core.db)
        .await?;
        sqlx::query(
            "UPDATE ask_prompt_responses SET dropped = true, published_at = NULL
             WHERE signed AND NOT dropped AND removed_at IS NULL
               AND ((owner_id = $1 AND sender_id = $2) OR (owner_id = $2 AND sender_id = $1))",
        )
        .bind(a)
        .bind(b)
        .execute(&self.core.db)
        .await?;
        self.forget_pair(a, b).await
    }

    /// `admin.content.removed`: a moderator took a question, a prompt or a
    /// prompt response down. It is gone for everyone, its sender included.
    /// An ID we do not know, or one that was taken down before, is done.
    pub(crate) async fn on_content_removed(&self, event: ContentRemoved) -> AppResult<()> {
        if event.subject_type != SubjectType::SUBJECT_TYPE_QUESTION as i32 {
            return Ok(());
        }
        let mut tx = self.core.db.begin().await?;
        let question: Option<QuestionRow> = sqlx::query_as(concat!(
            "SELECT ",
            question_columns!(),
            " FROM ask_questions q WHERE q.id = $2 AND q.removed_by IS DISTINCT FROM $3 FOR UPDATE"
        ))
        .bind(None::<Uuid>)
        .bind(event.subject_id)
        .bind(REMOVED_BY_MODERATION)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(question) = question {
            sqlx::query(
                "UPDATE ask_questions SET removed_at = COALESCE(removed_at, now()), removed_by = $2 WHERE id = $1",
            )
            .bind(question.id)
            .bind(REMOVED_BY_MODERATION)
            .execute(&mut *tx)
            .await?;
            if question.is_answered() {
                self.retract_answer(&mut tx, &question, None).await?;
            }
            // The upload is no longer shown anywhere; let the media domain collect it.
            assets::detach_holder(&mut *tx, &question_holder(question.id)).await?;
            assets::detach_holder(&mut *tx, &answer_holder(question.id)).await?;
            tx.commit().await?;
            return Ok(());
        }
        drop(tx);
        self.take_down_prompt_content(event.subject_id).await
    }

    /// `media.asset.deleted`: an image we show no longer exists.
    pub(crate) async fn on_asset_deleted(&self, asset_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        sqlx::query("UPDATE ask_questions SET image = NULL, image_asset_id = NULL WHERE image_asset_id = $1")
            .bind(asset_id)
            .execute(&mut *tx)
            .await?;
        let answers: Vec<QuestionRow> = sqlx::query_as(concat!(
            "SELECT ",
            question_columns!(),
            " FROM ask_questions q WHERE q.answer_image_asset_id = $2 FOR UPDATE"
        ))
        .bind(None::<Uuid>)
        .bind(asset_id)
        .fetch_all(&mut *tx)
        .await?;
        for answer in answers {
            let image_was_all = answer.answer_text.is_empty() && answer.answer_stamp_id.is_none();
            if image_was_all {
                // Nothing is left of the answer: the question is unanswered again.
                sqlx::query(
                    "UPDATE ask_questions
                     SET folder = 1, answer_image = NULL, answer_image_asset_id = NULL, answer_audience = 0, answered_at = NULL, answer_edited_at = NULL
                     WHERE id = $1",
                )
                .bind(answer.id)
                .execute(&mut *tx)
                .await?;
                if answer.is_answered() {
                    self.retract_answer(&mut tx, &answer, None).await?;
                }
            } else {
                let updated: QuestionRow = sqlx::query_as(concat!(
                    "UPDATE ask_questions q SET answer_image = NULL, answer_image_asset_id = NULL WHERE q.id = $2 RETURNING ",
                    question_columns!()
                ))
                .bind(None::<Uuid>)
                .bind(answer.id)
                .fetch_one(&mut *tx)
                .await?;
                if updated.is_public() {
                    outbound::index(&mut *tx, search_doc(&updated)).await?;
                }
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// The periodic purge: erases what was only kept for moderators
    /// (deleted, withdrawn, taken-down and dropped questions; deleted
    /// prompts) once it is older than the retention, and old rounds.
    /// Returns how many rows went.
    pub async fn purge(&self) -> AppResult<u64> {
        let retention = self.retention_days();
        let mut purged = 0;
        loop {
            let mut tx = self.core.db.begin().await?;
            let gone: Vec<(Uuid, bool, bool)> = sqlx::query_as(
                "DELETE FROM ask_questions WHERE id IN (
                     SELECT id FROM ask_questions
                     WHERE (removed_at IS NOT NULL OR dropped) AND created_at < now() - make_interval(days => $1)
                     ORDER BY created_at LIMIT $2 FOR UPDATE SKIP LOCKED)
                 RETURNING id, image_asset_id IS NOT NULL, answer_image_asset_id IS NOT NULL",
            )
            .bind(retention)
            .bind(PURGE_BATCH)
            .fetch_all(&mut *tx)
            .await?;
            for (question_id, had_image, had_answer_image) in &gone {
                if *had_image {
                    assets::detach_holder(&mut *tx, &question_holder(*question_id)).await?;
                }
                if *had_answer_image {
                    assets::detach_holder(&mut *tx, &answer_holder(*question_id)).await?;
                }
            }
            tx.commit().await?;
            purged += gone.len() as u64;
            if (gone.len() as i64) < PURGE_BATCH {
                break;
            }
        }
        let prompts = sqlx::query("DELETE FROM ask_prompts WHERE removed_at < now() - make_interval(days => $1)")
            .bind(retention)
            .execute(&self.core.db)
            .await?;
        let rounds = sqlx::query("DELETE FROM ask_rounds WHERE completed_at < now() - make_interval(days => $1)")
            .bind(ROUND_RETENTION_DAYS)
            .execute(&self.core.db)
            .await?;
        Ok(purged + prompts.rows_affected() + rounds.rows_affected())
    }
}
