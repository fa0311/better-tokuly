//! What a comment said before it was edited or deleted, kept for reports.
//!
//! An author who edits or deletes a comment takes it out of sight, but not
//! out of the hands of the people who already read it: for
//! [`RETENTION_DAYS`] after the change, a report about the comment resolves
//! with every version it had ([`Engagement::comment_for_report`]). Nothing
//! else reads these texts. A takedown by moderation, the purge of the
//! subject and the erasure of the author's account forget them at once.
use sqlx::PgConnection;
use tokuly_core::{prelude::*, subject::SubjectInfo};

use super::{CommentRow, find};
use crate::{
    Engagement,
    subjects::{SubjectKey, comment_path, is_absent},
    text,
};

/// Days a removed text is kept, counted from its removal.
pub(crate) const RETENTION_DAYS: i32 = 30;
/// Texts replaced by edits that are kept per comment; later edits keep
/// nothing (what the comment says now, or said when it was deleted, is
/// always known).
pub(crate) const MAX_EDITED_TEXTS: i64 = 20;
/// Characters of each version in the title of a report.
const TITLE_CHARS: usize = 80;

/// `comment_removed_texts.reason` values.
const EDITED: i16 = 1;
const DELETED: i16 = 2;

/// What a comment says, as a report is filed with it: its text, or what it
/// carries instead.
fn reported_text(row: &CommentRow) -> String {
    if !row.text.is_empty() {
        row.text.clone()
    } else if row.stamp_id.is_some() {
        "（スタンプ）".to_owned()
    } else if row.image_asset_id.is_some() {
        "（画像）".to_owned()
    } else {
        "コメント".to_owned()
    }
}

/// The SQL form of [`reported_text`], for copying rows that are about to go.
const REPORTED_TEXT_SQL: &str = "CASE WHEN text <> '' THEN text WHEN stamp_id IS NOT NULL THEN '（スタンプ）' \
                                 WHEN image_asset_id IS NOT NULL THEN '（画像）' ELSE 'コメント' END";

/// The title of a report about a comment, from its versions oldest first:
/// each distinct version, shortened, joined by arrows. `truncated` says
/// that edits after the kept ones were not recorded.
pub(crate) fn report_title(versions: &[String], truncated: bool) -> String {
    let mut distinct: Vec<String> = Vec::with_capacity(versions.len());
    for version in versions {
        let version = text::excerpt(version, TITLE_CHARS);
        if distinct.last() != Some(&version) {
            distinct.push(version);
        }
    }
    let count = distinct.len();
    if truncated && count > 1 {
        distinct.insert(count - 1, format!("（{MAX_EDITED_TEXTS}回目より後の編集は記録なし）"));
    }
    if count > 1 { format!("[{count}版] {}", distinct.join(" → ")) } else { distinct.pop().unwrap_or_default() }
}

/// Keeps the text an edit is about to replace. Runs under the comment's
/// row lock ([`super::lock_thread`]), which makes the cap exact.
pub(crate) async fn keep_edited(conn: &mut PgConnection, row: &CommentRow) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO comment_removed_texts (comment_id, parent_id, subject_type, subject_id, author_id, reason, text)
         SELECT $1, $2, $3, $4, $5, $6, $7
         WHERE (SELECT count(*) FROM comment_removed_texts WHERE comment_id = $1 AND reason = $6) < $8",
    )
    .bind(row.id)
    .bind(row.parent_id)
    .bind(row.subject_type)
    .bind(row.subject_id)
    .bind(row.author_id)
    .bind(EDITED)
    .bind(reported_text(row))
    .bind(MAX_EDITED_TEXTS)
    .execute(conn)
    .await?;
    Ok(())
}

/// Keeps what a comment about to be deleted said — and, for a top-level
/// comment, what the replies going with it said. Runs on the locked thread,
/// before [`super::lifecycle::delete`].
pub(crate) async fn keep_deleted(conn: &mut PgConnection, row: &CommentRow) -> AppResult<()> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO comment_removed_texts (comment_id, parent_id, subject_type, subject_id, author_id, reason, text, was_public)
         SELECT id, parent_id, subject_type, subject_id, author_id, $2, {REPORTED_TEXT_SQL}, status = 1 OR notified_at IS NOT NULL
         FROM comments WHERE id = $1 OR parent_id = $1"
    )))
    .bind(row.id)
    .bind(DELETED)
    .execute(conn)
    .await?;
    Ok(())
}

/// Forgets what was kept of these comments and of the replies below them
/// (a takedown).
pub(crate) async fn forget(conn: &mut PgConnection, comment_ids: &[Uuid]) -> AppResult<()> {
    sqlx::query("DELETE FROM comment_removed_texts WHERE comment_id = ANY($1) OR parent_id = ANY($1)")
        .bind(comment_ids)
        .execute(conn)
        .await?;
    Ok(())
}

/// Forgets what was kept of the comments on a subject (its purge).
pub(crate) async fn forget_subject(conn: &mut PgConnection, key: SubjectKey) -> AppResult<()> {
    sqlx::query("DELETE FROM comment_removed_texts WHERE subject_type = $1 AND subject_id = $2")
        .bind(key.kind)
        .bind(key.id)
        .execute(conn)
        .await?;
    Ok(())
}

/// The deletion of a comment as kept.
#[derive(sqlx::FromRow)]
struct Deleted {
    subject_type: i32,
    subject_id: Uuid,
    author_id: Uuid,
    text: String,
    was_public: bool,
}

impl Engagement {
    /// Forgets removed texts older than [`RETENTION_DAYS`]. Returns how many went.
    pub async fn purge_removed_comment_texts(&self) -> AppResult<u64> {
        Ok(sqlx::query("DELETE FROM comment_removed_texts WHERE removed_at <= now() - make_interval(days => $1)")
            .bind(RETENTION_DAYS)
            .execute(&self.core.db)
            .await?
            .rows_affected())
    }

    /// A comment as `viewer` received it, for a report (the
    /// `resolve_for_report` of `SUBJECT_TYPE_COMMENT`): with every version
    /// it had, and also after its deletion, for [`RETENTION_DAYS`].
    ///
    /// Blocks do not count — neither with the author nor with the subject's
    /// owner — and the subject is asked for what the viewer received of it:
    /// a block or a withdrawal must not take away the report. But the
    /// comment must have been in front of the viewer: it is or was in
    /// public view, or the viewer owns the subject (and saw it in the
    /// review queue).
    pub(crate) async fn comment_for_report(&self, comment_id: Uuid, viewer: Uuid) -> AppResult<Option<SubjectInfo>> {
        let db = &self.core.db;
        let (key, author_id, shown, now) = match find(db, comment_id).await? {
            Some(row) => {
                let shown = row.is_published() || row.notified_at.is_some();
                (row.key(), row.author_id, shown, reported_text(&row))
            }
            None => {
                let deleted: Option<Deleted> = sqlx::query_as(
                    "SELECT subject_type, subject_id, author_id, text, was_public FROM comment_removed_texts
                     WHERE comment_id = $1 AND reason = $2 AND removed_at > now() - make_interval(days => $3)
                     ORDER BY removed_at DESC LIMIT 1",
                )
                .bind(comment_id)
                .bind(DELETED)
                .bind(RETENTION_DAYS)
                .fetch_optional(db)
                .await?;
                // Never written, taken down, or deleted too long ago.
                let Some(deleted) = deleted else { return Ok(None) };
                let key = SubjectKey { kind: deleted.subject_type, id: deleted.subject_id };
                (key, deleted.author_id, deleted.was_public, deleted.text)
            }
        };
        let subject =
            match self.core.subjects.resolve_for_report(key.subject_type(), &key.id.to_string(), Some(viewer)).await {
                Ok(subject) => subject,
                Err(error) if is_absent(&error) => return Ok(None),
                Err(error) => return Err(error),
            };
        let owns_subject = subject.owner_id == viewer && !viewer.is_nil();
        if !(shown || owns_subject || author_id == viewer) {
            return Ok(None);
        }
        let mut versions: Vec<String> = sqlx::query_scalar(
            "SELECT text FROM comment_removed_texts
             WHERE comment_id = $1 AND reason = $2 AND removed_at > now() - make_interval(days => $3)
             ORDER BY removed_at, id LIMIT $4",
        )
        .bind(comment_id)
        .bind(EDITED)
        .bind(RETENTION_DAYS)
        .bind(MAX_EDITED_TEXTS)
        .fetch_all(db)
        .await?;
        let truncated = versions.len() as i64 >= MAX_EDITED_TEXTS;
        versions.push(now);
        Ok(Some(SubjectInfo {
            id: comment_id,
            owner_id: author_id,
            title: report_title(&versions, truncated),
            path: comment_path(&subject.path, comment_id),
            image_url: None,
            interactions_enabled: false,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::{super::testing::comment, *};

    #[test]
    fn titles_list_each_distinct_version() {
        let versions = |texts: &[&str]| texts.iter().map(|text| text.to_string()).collect::<Vec<_>>();
        assert_eq!(report_title(&versions(&["ひどい"]), false), "ひどい");
        assert_eq!(report_title(&versions(&["ひどい", "ごめん"]), false), "[2版] ひどい → ごめん");
        assert_eq!(report_title(&versions(&["a", "a", "b"]), false), "[2版] a → b");
        assert_eq!(report_title(&versions(&["a", "b"]), true), "[2版] a → （20回目より後の編集は記録なし） → b");
        assert_eq!(report_title(&versions(&["x".repeat(100).as_str()]), false).chars().count(), 80);
    }

    #[test]
    fn comments_without_text_are_reported_by_what_they_carry() {
        let author = Uuid::now_v7();
        assert_eq!(reported_text(&comment(author)), "こんにちは");
        let stamp = CommentRow { text: String::new(), stamp_id: Some(Uuid::now_v7()), ..comment(author) };
        assert_eq!(reported_text(&stamp), "（スタンプ）");
        let image = CommentRow { text: String::new(), image_asset_id: Some(Uuid::now_v7()), ..comment(author) };
        assert_eq!(reported_text(&image), "（画像）");
    }
}
