//! Questions: the row, who sees what of it, and the lists.
//!
//! A question is shown to three kinds of people, and [`Renderer::render`]
//! is the one place that decides what each of them gets:
//!
//! - the **owner** of the box: text, folder, read state, the answer — and
//!   the asker only if the asker signed;
//! - the **sender**: their own question, and whether it is `WAITING` or
//!   `ANSWERED`. Nothing else about its fate (read, archived, filtered,
//!   deleted, dropped) reaches them;
//! - a **reader** of the Q&A page: the question with its answer.
//!
//! The row type cannot leak an anonymous sender by accident: the queries
//! here select the sender's ID only for signed questions (`asker_id`), and
//! compare it with the viewer inside the database (`sent_by_viewer`).
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use tokuly_core::prelude::*;
use tokuly_media::assets::image_from_json;
use tokuly_proto::{
    ask::{Answer, AnswerAudience, InboxCounts, Question, QuestionFolder, SentState},
    common::{PageInfo, StampRef, UserSummary},
};
use tokuly_social::graph;

use crate::Ask;

pub(crate) const INBOX: i16 = QuestionFolder::QUESTION_FOLDER_INBOX as i16;
pub(crate) const ANSWERED: i16 = QuestionFolder::QUESTION_FOLDER_ANSWERED as i16;
pub(crate) const ARCHIVED: i16 = QuestionFolder::QUESTION_FOLDER_ARCHIVED as i16;
pub(crate) const FILTERED: i16 = QuestionFolder::QUESTION_FOLDER_FILTERED as i16;

pub(crate) const PUBLIC: i16 = AnswerAudience::ANSWER_AUDIENCE_PUBLIC as i16;
pub(crate) const FRIENDS: i16 = AnswerAudience::ANSWER_AUDIENCE_FRIENDS as i16;
pub(crate) const PRIVATE: i16 = AnswerAudience::ANSWER_AUDIENCE_PRIVATE as i16;

/// Who removed a question from the box (`ask_questions.removed_by`).
pub(crate) const REMOVED_BY_OWNER: i16 = 1;
pub(crate) const REMOVED_BY_SENDER: i16 = 2;
pub(crate) const REMOVED_BY_MODERATION: i16 = 3;

const _: () = {
    // The migration's partial indexes and CHECKs use these numbers.
    assert!(INBOX == 1 && ANSWERED == 2 && ARCHIVED == 3 && FILTERED == 4);
    assert!(PUBLIC == 1 && FRIENDS == 2 && PRIVATE == 3);
};

/// Audiences of the answers on a Q&A page, for the owner and their friends
/// and for everyone else.
const ON_PAGE_FOR_FRIENDS: &[i16] = &[PUBLIC, FRIENDS];
const ON_PAGE_FOR_ANYONE: &[i16] = &[PUBLIC];

/// Largest page of any list in this domain.
pub(crate) const MAX_PAGE: i64 = 50;

/// Path of a question's page in the web app.
pub(crate) fn question_path(id: Uuid) -> String {
    format!("/ask/q/{id}")
}

/// Columns of [`QuestionRow`], from `ask_questions q`. `$1` must be bound to
/// the viewer (NULL for guests).
macro_rules! question_columns {
    () => {
        "q.id, q.owner_id, CASE WHEN q.signed THEN q.sender_id END AS asker_id, q.signed, q.text, \
         q.image, q.folder, q.read_at, q.created_at, \
         (q.removed_at IS NULL AND NOT q.dropped) AS live, \
         q.answer_text, q.answer_image, q.answer_image_asset_id, q.answer_stamp_id, q.answer_audience, q.answered_at, \
         q.answer_edited_at, \
         q.like_count, q.comment_count, COALESCE(q.sender_id = $1, false) AS sent_by_viewer"
    };
}
pub(crate) use question_columns;

/// SQL condition for "this question is in its owner's box": delivered, not
/// deleted, withdrawn or taken down, not dropped, and not a signed question
/// of a suspended account. (An anonymous question stays whatever becomes of
/// its sender's account: see `senders.rs`.)
macro_rules! in_box {
    () => {
        concat!(
            "q.delivered_at IS NOT NULL AND q.removed_at IS NULL AND NOT q.dropped AND NOT ",
            $crate::senders::signed_by_suspended!("q")
        )
    };
}
pub(crate) use in_box;

/// SQL condition for "the sender still sees this question in their list";
/// `$2` must be bound to the retention in days. Deletion by the owner does
/// not count (the sender must not notice it), and unanswered questions age
/// out after the retention, whatever became of them.
macro_rules! in_sent_list {
    () => {
        "(q.removed_by IS NULL OR q.removed_by = 1) \
         AND ((q.folder = 2 AND q.removed_at IS NULL AND NOT q.dropped) OR q.created_at > now() - make_interval(days => $2))"
    };
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct QuestionRow {
    pub id: Uuid,
    pub owner_id: Uuid,
    /// The sender, on signed questions only.
    pub asker_id: Option<Uuid>,
    pub signed: bool,
    pub text: String,
    pub image: Option<serde_json::Value>,
    pub folder: i16,
    pub read_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    /// In the box: not removed and not dropped.
    pub live: bool,
    pub answer_text: String,
    pub answer_image: Option<serde_json::Value>,
    pub answer_image_asset_id: Option<Uuid>,
    pub answer_stamp_id: Option<Uuid>,
    pub answer_audience: i16,
    pub answered_at: Option<DateTime<Utc>>,
    pub answer_edited_at: Option<DateTime<Utc>>,
    pub like_count: i32,
    pub comment_count: i32,
    /// Whether the viewer the query was made for sent this question.
    pub sent_by_viewer: bool,
}

impl QuestionRow {
    /// Answered, and the answer still stands.
    pub(crate) fn is_answered(&self) -> bool {
        self.live && self.folder == ANSWERED
    }

    /// On the owner's Q&A page (for the audience that may see it).
    pub(crate) fn is_published(&self) -> bool {
        self.is_answered() && (self.answer_audience == PUBLIC || self.answer_audience == FRIENDS)
    }

    pub(crate) fn is_public(&self) -> bool {
        self.is_answered() && self.answer_audience == PUBLIC
    }

    fn answer(&self) -> Option<Answer> {
        let answered_at = self.answered_at.filter(|_| self.is_answered())?;
        Some(Answer {
            text: self.answer_text.clone(),
            image: image_from_json(self.answer_image.as_ref()).into(),
            audience: i32::from(self.answer_audience).into(),
            answered_at: ts(answered_at),
            edited_at: opt_ts(self.answer_edited_at),
            ..Default::default()
        })
    }
}

/// Everything needed to turn rows into `Question` messages for one viewer.
pub(crate) struct Renderer {
    viewer: Option<Uuid>,
    users: HashMap<Uuid, UserSummary>,
    stamps: HashMap<Uuid, StampRef>,
    /// Everyone in a block with the viewer: their names are not shown.
    hidden: Vec<Uuid>,
}

impl Renderer {
    pub(crate) fn render(&self, row: &QuestionRow) -> Question {
        let is_owner = self.viewer == Some(row.owner_id);
        let is_sender = row.sent_by_viewer && !is_owner;
        let mut answer = row.answer();
        if let (Some(answer), Some(stamp_id)) = (answer.as_mut(), row.answer_stamp_id) {
            answer.stamp = self.stamps.get(&stamp_id).cloned().into();
        }
        let answered = answer.is_some();
        let asker = row
            .asker_id
            .filter(|id| row.signed && !self.hidden.contains(id))
            .and_then(|id| self.users.get(&id).cloned());
        Question {
            id: row.id.to_string(),
            owner: tokuly_identity::users::pick(&self.users, row.owner_id).into(),
            text: row.text.clone(),
            image: image_from_json(row.image.as_ref()).into(),
            anonymous: !row.signed,
            asker: asker.into(),
            created_at: ts(row.created_at),
            answer: answer.into(),
            folder: if is_owner {
                i32::from(row.folder).into()
            } else {
                QuestionFolder::QUESTION_FOLDER_UNSPECIFIED.into()
            },
            read: is_owner && row.read_at.is_some(),
            sent_by_me: is_sender,
            sent_state: match (is_sender, answered) {
                (true, true) => SentState::SENT_STATE_ANSWERED,
                (true, false) => SentState::SENT_STATE_WAITING,
                (false, _) => SentState::SENT_STATE_UNSPECIFIED,
            }
            .into(),
            like_count: if answered { row.like_count } else { 0 },
            comment_count: if answered { row.comment_count } else { 0 },
            path: question_path(row.id),
            ..Default::default()
        }
    }

    pub(crate) fn render_all(&self, rows: &[QuestionRow]) -> Vec<Question> {
        rows.iter().map(|row| self.render(row)).collect()
    }
}

impl Ask {
    /// Loads the names and stamps `rows` need when shown to `viewer`.
    pub(crate) async fn renderer(&self, rows: &[QuestionRow], viewer: Option<Uuid>) -> AppResult<Renderer> {
        let mut user_ids: Vec<Uuid> =
            rows.iter().flat_map(|row| [Some(row.owner_id), row.asker_id]).flatten().collect();
        user_ids.sort_unstable();
        user_ids.dedup();
        let stamp_ids: Vec<Uuid> =
            rows.iter().filter(|row| row.is_answered()).filter_map(|row| row.answer_stamp_id).collect();
        let hidden = match viewer {
            Some(viewer) => graph::blocked_ids_either(&self.core, viewer).await?,
            None => Vec::new(),
        };
        Ok(Renderer {
            viewer,
            users: tokuly_identity::summaries(&self.core, &user_ids).await?,
            stamps: tokuly_stamp::stamp_refs(&self.core, &stamp_ids).await?,
            hidden,
        })
    }

    /// One row as `viewer` sees it.
    pub(crate) async fn render_one(&self, row: &QuestionRow, viewer: Option<Uuid>) -> AppResult<Question> {
        Ok(self.renderer(std::slice::from_ref(row), viewer).await?.render(row))
    }

    /// A question in `owner`'s box, or `QUESTION_NOT_FOUND`.
    pub(crate) async fn owned_question(&self, owner: Uuid, question_id: Uuid) -> AppResult<QuestionRow> {
        let row: Option<QuestionRow> = sqlx::query_as(concat!(
            "SELECT ",
            question_columns!(),
            " FROM ask_questions q WHERE q.owner_id = $1 AND q.id = $2 AND ",
            in_box!()
        ))
        .bind(owner)
        .bind(question_id)
        .fetch_optional(&self.core.db)
        .await?;
        row.or_not_found("QUESTION_NOT_FOUND", "question not found")
    }

    /// A question `sender` sent that is still in their list.
    pub(crate) async fn sent_question(&self, sender: Uuid, question_id: Uuid) -> AppResult<QuestionRow> {
        let row: Option<QuestionRow> = sqlx::query_as(concat!(
            "SELECT ",
            question_columns!(),
            " FROM ask_questions q WHERE q.sender_id = $1 AND q.id = $3 AND ",
            in_sent_list!()
        ))
        .bind(sender)
        .bind(self.retention_days())
        .bind(question_id)
        .fetch_optional(&self.core.db)
        .await?;
        row.or_not_found("QUESTION_NOT_FOUND", "question not found")
    }

    /// One folder of `owner`'s inbox. Answered questions come newest answer
    /// first, the other folders newest question first.
    pub(crate) async fn inbox_page(
        &self,
        owner: Uuid,
        folder: i16,
        page: &Page,
    ) -> AppResult<(Vec<QuestionRow>, PageInfo)> {
        if folder == ANSWERED {
            return self.answers_where_owner(owner, Some(owner), &[PUBLIC, FRIENDS, PRIVATE], page).await;
        }
        let cursor: Option<Uuid> = page.decode()?;
        let mut rows: Vec<QuestionRow> = sqlx::query_as(concat!(
            "SELECT ",
            question_columns!(),
            " FROM ask_questions q
              WHERE q.owner_id = $1 AND q.folder = $2 AND ",
            in_box!(),
            " AND ($3::uuid IS NULL OR q.id < $3)
              ORDER BY q.id DESC LIMIT $4"
        ))
        .bind(owner)
        .bind(folder)
        .bind(cursor)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| row.id);
        Ok((rows, info))
    }

    /// Answered questions of `owner` whose audience is one of `audiences`,
    /// newest answer first.
    pub(crate) async fn answers_where_owner(
        &self,
        owner: Uuid,
        viewer: Option<Uuid>,
        audiences: &[i16],
        page: &Page,
    ) -> AppResult<(Vec<QuestionRow>, PageInfo)> {
        let cursor: Option<(DateTime<Utc>, Uuid)> = page.decode()?;
        let (after_time, after_id) = cursor.unzip();
        let mut rows: Vec<QuestionRow> = sqlx::query_as(concat!(
            "SELECT ",
            question_columns!(),
            " FROM ask_questions q
              WHERE q.owner_id = $2 AND q.folder = 2 AND q.answer_audience = ANY($3) AND ",
            in_box!(),
            " AND ($4::timestamptz IS NULL OR (q.answered_at, q.id) < ($4, $5))
              ORDER BY q.answered_at DESC, q.id DESC LIMIT $6"
        ))
        .bind(viewer)
        .bind(owner)
        .bind(audiences)
        .bind(after_time)
        .bind(after_id)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.answered_at.unwrap_or(row.created_at), row.id));
        Ok((rows, info))
    }

    /// How many answers of `owner` have one of `audiences`.
    pub(crate) async fn answer_count(&self, owner: Uuid, audiences: &[i16]) -> AppResult<i64> {
        Ok(sqlx::query_scalar(concat!(
            "SELECT count(*) FROM ask_questions q WHERE q.owner_id = $1 AND q.folder = 2 AND q.answer_audience = ANY($2) AND ",
            in_box!()
        ))
        .bind(owner)
        .bind(audiences)
        .fetch_one(&self.core.db)
        .await?)
    }

    /// The audiences of `owner`'s answers that `viewer` finds on the Q&A
    /// page. Private answers are never on the page, not even for the owner
    /// (they are in the ANSWERED folder).
    pub(crate) async fn page_audiences(&self, owner: Uuid, viewer: Option<Uuid>) -> AppResult<&'static [i16]> {
        let close = match viewer {
            Some(viewer) if viewer == owner => true,
            Some(viewer) => graph::are_friends(&self.core, owner, viewer).await?,
            None => false,
        };
        Ok(if close { ON_PAGE_FOR_FRIENDS } else { ON_PAGE_FOR_ANYONE })
    }

    /// Questions `sender` sent, newest first.
    pub(crate) async fn sent_page(&self, sender: Uuid, page: &Page) -> AppResult<(Vec<QuestionRow>, PageInfo)> {
        let cursor: Option<Uuid> = page.decode()?;
        // Boxes of people in a block with the sender are gone for them, like
        // the rest of those people's content.
        let hidden = graph::blocked_ids_either(&self.core, sender).await?;
        let mut rows: Vec<QuestionRow> = sqlx::query_as(concat!(
            "SELECT ",
            question_columns!(),
            " FROM ask_questions q
              WHERE q.sender_id = $1 AND ",
            in_sent_list!(),
            " AND NOT (q.owner_id = ANY($3)) AND ($4::uuid IS NULL OR q.id < $4)
              ORDER BY q.id DESC LIMIT $5"
        ))
        .bind(sender)
        .bind(self.retention_days())
        .bind(hidden)
        .bind(cursor)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| row.id);
        Ok((rows, info))
    }

    /// The question `viewer` asked to see by ID, if they may: the owner of
    /// its box, its sender, or a reader inside the answer's audience.
    pub(crate) async fn readable_question(&self, question_id: Uuid, viewer: Option<Uuid>) -> AppResult<QuestionRow> {
        let not_found = || AppError::not_found("QUESTION_NOT_FOUND", "question not found");
        // The sender is answered from their own list and from nothing else:
        // a question that left the list must not be findable for them by ID
        // either, or looking it up would tell whether it is still in the box.
        if let Some(viewer) = viewer {
            match self.sent_question(viewer, question_id).await {
                Ok(row) => {
                    let blocked = graph::is_blocked_either(&self.core, viewer, row.owner_id).await?;
                    return if blocked { Err(not_found()) } else { Ok(row) };
                }
                Err(e) if e.is_not_found() => {}
                Err(e) => return Err(e),
            }
        }
        let row: Option<QuestionRow> = sqlx::query_as(concat!(
            "SELECT ",
            question_columns!(),
            " FROM ask_questions q
              WHERE q.id = $2 AND COALESCE(q.sender_id <> $1, true) AND ",
            in_box!(),
            " AND EXISTS (SELECT 1 FROM users o WHERE o.id = q.owner_id AND o.deleted_at IS NULL AND o.banned_at IS NULL)"
        ))
        .bind(viewer)
        .bind(question_id)
        .fetch_optional(&self.core.db)
        .await?;
        let row = row.ok_or_else(not_found)?;
        if viewer == Some(row.owner_id) {
            return Ok(row);
        }
        if let Some(viewer) = viewer
            && graph::is_blocked_either(&self.core, viewer, row.owner_id).await?
        {
            return Err(not_found());
        }
        let readable =
            row.is_published() && self.page_audiences(row.owner_id, viewer).await?.contains(&row.answer_audience);
        if readable { Ok(row) } else { Err(not_found()) }
    }

    /// The numbers on the owner's inbox tabs.
    pub(crate) async fn inbox_counts(&self, owner: Uuid) -> AppResult<InboxCounts> {
        let (unread, inbox, answered, archived, filtered): (i64, i64, i64, i64, i64) = sqlx::query_as(concat!(
            "SELECT count(*) FILTER (WHERE q.folder = 1 AND q.read_at IS NULL),
                    count(*) FILTER (WHERE q.folder = 1),
                    count(*) FILTER (WHERE q.folder = 2),
                    count(*) FILTER (WHERE q.folder = 3),
                    count(*) FILTER (WHERE q.folder = 4)
             FROM ask_questions q WHERE q.owner_id = $1 AND ",
            in_box!()
        ))
        .bind(owner)
        .fetch_one(&self.core.db)
        .await?;
        Ok(InboxCounts {
            unread: unread as i32,
            inbox: inbox as i32,
            answered: answered as i32,
            archived: archived as i32,
            filtered: filtered as i32,
            ..Default::default()
        })
    }

    /// Marks questions of `owner`'s box as read: `ids`, or all of them.
    pub(crate) async fn mark_read(&self, owner: Uuid, ids: &[Uuid], all: bool) -> AppResult<()> {
        sqlx::query(concat!(
            "UPDATE ask_questions q SET read_at = now()
             WHERE q.owner_id = $1 AND q.read_at IS NULL AND ($2 OR q.id = ANY($3)) AND ",
            in_box!()
        ))
        .bind(owner)
        .bind(all)
        .bind(ids)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    /// Moves an unanswered question of `owner`'s box to `folder`.
    pub(crate) async fn refile_question(&self, owner: Uuid, question_id: Uuid, folder: i16) -> AppResult<QuestionRow> {
        let current = self.owned_question(owner, question_id).await?;
        ensure!(
            current.folder != ANSWERED,
            precondition,
            "QUESTION_ALREADY_ANSWERED",
            "delete the answer before moving the question"
        );
        let moved: Option<QuestionRow> = sqlx::query_as(concat!(
            "UPDATE ask_questions q SET folder = $3
             WHERE q.owner_id = $1 AND q.id = $2 AND q.folder <> 2 AND ",
            in_box!(),
            " RETURNING ",
            question_columns!()
        ))
        .bind(owner)
        .bind(question_id)
        .bind(folder)
        .fetch_optional(&self.core.db)
        .await?;
        moved.or_not_found("QUESTION_NOT_FOUND", "question not found")
    }

    /// Takes back a question `sender` sent, unless it was answered.
    pub(crate) async fn take_back_question(&self, sender: Uuid, question_id: Uuid) -> AppResult<()> {
        let question = self.sent_question(sender, question_id).await?;
        ensure!(
            !question.is_answered(),
            precondition,
            "QUESTION_ALREADY_ANSWERED",
            "an answered question cannot be withdrawn"
        );
        let mut tx = self.core.db.begin().await?;
        let withdrawn = sqlx::query(
            "UPDATE ask_questions q SET removed_at = now(), removed_by = $3
             WHERE q.id = $1 AND q.sender_id = $2 AND (q.removed_by IS NULL OR q.removed_by = 1)
               AND NOT (q.folder = 2 AND q.removed_at IS NULL AND NOT q.dropped)",
        )
        .bind(question_id)
        .bind(sender)
        .bind(REMOVED_BY_SENDER)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        // Answered in the meantime.
        ensure!(withdrawn == 1, precondition, "QUESTION_ALREADY_ANSWERED", "an answered question cannot be withdrawn");
        tx.commit().await?;
        Ok(())
    }

    pub(crate) fn retention_days(&self) -> i32 {
        self.cfg.retention_days.min(i32::MAX as u32) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(owner: Uuid) -> QuestionRow {
        QuestionRow {
            id: new_id(),
            owner_id: owner,
            asker_id: None,
            signed: false,
            text: "好きな食べ物は?".into(),
            image: None,
            folder: INBOX,
            read_at: None,
            created_at: now(),
            live: true,
            answer_text: String::new(),
            answer_image: None,
            answer_image_asset_id: None,
            answer_stamp_id: None,
            answer_audience: 0,
            answered_at: None,
            answer_edited_at: None,
            like_count: 0,
            comment_count: 0,
            sent_by_viewer: false,
        }
    }

    fn renderer(viewer: Option<Uuid>) -> Renderer {
        Renderer { viewer, users: HashMap::new(), stamps: HashMap::new(), hidden: Vec::new() }
    }

    fn answered(mut question: QuestionRow, audience: i16) -> QuestionRow {
        question.folder = ANSWERED;
        question.answer_text = "ラーメン!".into();
        question.answer_audience = audience;
        question.answered_at = Some(now());
        question.like_count = 3;
        question
    }

    #[test]
    fn the_owner_sees_folder_and_read_state_but_no_asker_on_anonymous_questions() {
        let owner = new_id();
        let mut question = row(owner);
        question.read_at = Some(now());
        let seen = renderer(Some(owner)).render(&question);
        assert_eq!(seen.folder, QuestionFolder::QUESTION_FOLDER_INBOX);
        assert!(seen.read && seen.anonymous);
        assert!(seen.asker.is_unset() && !seen.sent_by_me);
        assert_eq!(seen.sent_state, SentState::SENT_STATE_UNSPECIFIED);
    }

    #[test]
    fn the_sender_sees_only_waiting_or_answered() {
        let (owner, sender) = (new_id(), new_id());
        for (folder, live) in [(INBOX, true), (ARCHIVED, true), (FILTERED, true), (INBOX, false), (ANSWERED, false)] {
            let mut question = row(owner);
            question.sent_by_viewer = true;
            question.folder = folder;
            question.live = live;
            question.read_at = Some(now());
            if folder == ANSWERED {
                question = answered(question, PUBLIC);
            }
            let seen = renderer(Some(sender)).render(&question);
            assert_eq!(seen.sent_state, SentState::SENT_STATE_WAITING, "folder {folder}, live {live}");
            assert_eq!(seen.folder, QuestionFolder::QUESTION_FOLDER_UNSPECIFIED);
            assert!(!seen.read && seen.sent_by_me && seen.answer.is_unset());
            assert_eq!(seen.like_count, 0);
        }
        let mut question = answered(row(owner), PRIVATE);
        question.sent_by_viewer = true;
        let seen = renderer(Some(sender)).render(&question);
        assert_eq!(seen.sent_state, SentState::SENT_STATE_ANSWERED);
        assert_eq!(seen.answer.as_option().unwrap().text, "ラーメン!");
    }

    #[test]
    fn readers_get_the_answer_without_inbox_details() {
        let owner = new_id();
        let question = answered(row(owner), PUBLIC);
        let seen = renderer(None).render(&question);
        assert!(seen.answer.is_set() && !seen.read && !seen.sent_by_me);
        assert_eq!(seen.folder, QuestionFolder::QUESTION_FOLDER_UNSPECIFIED);
        assert_eq!(seen.like_count, 3);
        assert_eq!(seen.path, format!("/ask/q/{}", question.id));
    }

    #[test]
    fn a_signed_asker_is_named_unless_blocked_with_the_viewer() {
        let (owner, asker) = (new_id(), new_id());
        let mut question = row(owner);
        question.signed = true;
        question.asker_id = Some(asker);
        let mut names = renderer(Some(owner));
        names.users.insert(asker, UserSummary { id: asker.to_string(), handle: "hana".into(), ..Default::default() });
        assert_eq!(names.render(&question).asker.as_option().unwrap().handle, "hana");
        assert!(!names.render(&question).anonymous);
        names.hidden.push(asker);
        assert!(names.render(&question).asker.is_unset());
    }

    #[test]
    fn publication_follows_the_audience() {
        let owner = new_id();
        assert!(answered(row(owner), PUBLIC).is_public());
        assert!(answered(row(owner), FRIENDS).is_published() && !answered(row(owner), FRIENDS).is_public());
        assert!(!answered(row(owner), PRIVATE).is_published());
        let mut removed = answered(row(owner), PUBLIC);
        removed.live = false;
        assert!(!removed.is_answered() && !removed.is_published());
    }
}
