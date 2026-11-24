//! What a comment said before it was edited or deleted, kept for reports.
//!
//! An author who edits or deletes a comment takes it out of sight, but not
//! out of the hands of the people who already read it: for
//! [`RETENTION_DAYS`] after the change, a report about the comment resolves
//! with every version it had ([`Engagement::comment_for_report`]). Only the
//! moderation team gets those versions (the report's `reported_content`);
//! the reporter's own copy of the report shows what they may see now.
//! Nothing else reads these texts. A takedown by moderation, the purge of
//! the subject and the erasure of the author's account forget them at once.
use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use tokuly_core::{prelude::*, subject::SubjectInfo};
use tokuly_social::graph;

use super::{CommentRow, find};
use crate::{
    Engagement,
    subjects::{SubjectKey, comment_path, is_absent},
    text,
};

/// Days a removed text is kept, counted from its removal.
pub(crate) const RETENTION_DAYS: i32 = 30;
/// Texts replaced by the first edits of a comment that are always kept.
const KEEP_FIRST: i64 = 10;
/// Texts replaced by the latest edits that are kept on top of those: each
/// further edit drops the oldest of these, so what a comment said just
/// before its last edits is always known.
const KEEP_LATEST: i64 = 10;
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
const REPORTED_TEXT_SQL: &str = "CASE WHEN c.text <> '' THEN c.text WHEN c.stamp_id IS NOT NULL THEN '（スタンプ）' \
                                 WHEN c.image_asset_id IS NOT NULL THEN '（画像）' ELSE 'コメント' END";

/// One entry of a report title: a version, or edits that were not kept.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Entry {
    Version(String),
    Skipped(i32),
}

/// The versions of a comment, oldest first, from the kept edits (numbered)
/// and what it says now (or said when it was deleted).
fn entries(edits: Vec<(i32, String)>, last: String) -> Vec<Entry> {
    let mut entries = Vec::with_capacity(edits.len() + 2);
    let mut previous = 0;
    for (version, text) in edits {
        if version > previous + 1 {
            entries.push(Entry::Skipped(version - previous - 1));
        }
        entries.push(Entry::Version(text));
        previous = version;
    }
    entries.push(Entry::Version(last));
    entries
}

/// The moderators' title of a comment: each distinct version, shortened,
/// oldest first, joined by arrows, with a note where edits were not kept.
pub(crate) fn report_title(entries: &[Entry]) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(entries.len());
    let mut last_version: Option<String> = None;
    let mut count = 0;
    for entry in entries {
        match entry {
            Entry::Version(version) => {
                let version = text::excerpt(version, TITLE_CHARS);
                if last_version.as_ref() != Some(&version) {
                    parts.push(version.clone());
                    last_version = Some(version);
                    count += 1;
                }
            }
            Entry::Skipped(skipped) => parts.push(format!("（{skipped}件の編集は記録なし）")),
        }
    }
    if count > 1 { format!("[{count}版] {}", parts.join(" → ")) } else { parts.join(" → ") }
}

/// Keeps the text an edit is about to replace. Runs under the comment's
/// row lock ([`super::lock_thread`]), which makes the numbering and the cap
/// exact: past [`KEEP_FIRST`] + [`KEEP_LATEST`], the oldest of the latest
/// ones goes.
pub(crate) async fn keep_edited(conn: &mut PgConnection, row: &CommentRow) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO comment_removed_texts (comment_id, parent_id, subject_type, subject_id, author_id, reason, version, text, written_at)
         SELECT $1, $2, $3, $4, $5, $6,
                COALESCE((SELECT max(version) FROM comment_removed_texts WHERE comment_id = $1 AND reason = $6), 0) + 1,
                $7, $8",
    )
    .bind(row.id)
    .bind(row.parent_id)
    .bind(row.subject_type)
    .bind(row.subject_id)
    .bind(row.author_id)
    .bind(EDITED)
    .bind(reported_text(row))
    .bind(row.created_at)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "DELETE FROM comment_removed_texts
         WHERE id = (SELECT id FROM comment_removed_texts WHERE comment_id = $1 AND reason = $2
                     ORDER BY version OFFSET $3 LIMIT 1)
           AND (SELECT count(*) FROM comment_removed_texts WHERE comment_id = $1 AND reason = $2) > $3 + $4",
    )
    .bind(row.id)
    .bind(EDITED)
    .bind(KEEP_FIRST)
    .bind(KEEP_LATEST)
    .execute(conn)
    .await?;
    Ok(())
}

/// Keeps what a comment about to be deleted said — and, for a top-level
/// comment, what the replies going with it said. Runs on the locked thread,
/// before [`super::lifecycle::delete`]. A reply counts as having been in
/// public view only below a published top-level comment.
pub(crate) async fn keep_deleted(conn: &mut PgConnection, row: &CommentRow) -> AppResult<()> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO comment_removed_texts
             (comment_id, parent_id, subject_type, subject_id, author_id, reason, text, was_public, written_at)
         SELECT c.id, c.parent_id, c.subject_type, c.subject_id, c.author_id, $2, {REPORTED_TEXT_SQL},
                (c.status = 1 AND (p.id IS NULL OR p.status = 1)) OR c.notified_at IS NOT NULL, c.created_at
         FROM comments c LEFT JOIN comments p ON p.id = c.parent_id
         WHERE c.id = $1 OR c.parent_id = $1"
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
    written_at: DateTime<Utc>,
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
    /// `resolve_for_report` of `SUBJECT_TYPE_COMMENT`): titled with every
    /// version it had, also after its deletion, for [`RETENTION_DAYS`]. The
    /// title is for the moderation team only; the reporter's copy of the
    /// report is titled with what they may see now.
    ///
    /// The comment must have been in front of the viewer: it is or was in
    /// public view (a reply only below a published top-level comment), or
    /// the viewer owns the subject (and saw it in the review queue). Blocks
    /// do not take that away, but a comment written after a block began —
    /// with its author or with the subject's owner — never reached the
    /// other side of it. The subject is asked for what the viewer received
    /// of it.
    pub(crate) async fn comment_for_report(&self, comment_id: Uuid, viewer: Uuid) -> AppResult<Option<SubjectInfo>> {
        let db = &self.core.db;
        let (key, author_id, shown, written_at, now) = match find(db, comment_id).await? {
            Some(row) => {
                let parent = match row.parent_id {
                    Some(parent_id) => find(db, parent_id).await?,
                    None => None,
                };
                let shown = row.in_public_view(parent.as_ref()) || row.notified_at.is_some();
                (row.key(), row.author_id, shown, row.created_at, reported_text(&row))
            }
            None => {
                let deleted: Option<Deleted> = sqlx::query_as(
                    "SELECT subject_type, subject_id, author_id, text, was_public, written_at FROM comment_removed_texts
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
                (key, deleted.author_id, deleted.was_public, deleted.written_at, deleted.text)
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
        for party in [author_id, subject.owner_id] {
            if party == viewer || party.is_nil() {
                continue;
            }
            if let Some(since) = graph::blocked_since(&self.core, viewer, party).await?
                && written_at >= since
            {
                return Ok(None);
            }
        }
        let edits: Vec<(i32, String)> = sqlx::query_as(
            "SELECT version, text FROM comment_removed_texts
             WHERE comment_id = $1 AND reason = $2 AND removed_at > now() - make_interval(days => $3)
             ORDER BY version LIMIT $4",
        )
        .bind(comment_id)
        .bind(EDITED)
        .bind(RETENTION_DAYS)
        .bind(KEEP_FIRST + KEEP_LATEST)
        .fetch_all(db)
        .await?;
        Ok(Some(SubjectInfo {
            id: comment_id,
            owner_id: author_id,
            title: report_title(&entries(edits, now)),
            path: comment_path(&subject.path, comment_id),
            image_url: None,
            interactions_enabled: false,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::{super::testing::comment, *};

    fn versions(texts: &[&str]) -> Vec<Entry> {
        texts.iter().map(|text| Entry::Version(text.to_string())).collect()
    }

    #[test]
    fn titles_list_each_distinct_version() {
        assert_eq!(report_title(&versions(&["ひどい"])), "ひどい");
        assert_eq!(report_title(&versions(&["ひどい", "ごめん"])), "[2版] ひどい → ごめん");
        assert_eq!(report_title(&versions(&["a", "a", "b"])), "[2版] a → b");
        assert_eq!(report_title(&versions(&["x".repeat(100).as_str()])).chars().count(), 80);
    }

    #[test]
    fn edits_that_were_not_kept_are_marked_where_they_were() {
        let kept = vec![(1, "a".to_owned()), (2, "b".to_owned()), (5, "e".to_owned())];
        let listed = entries(kept, "f".to_owned());
        assert_eq!(listed[2], Entry::Skipped(2));
        assert_eq!(report_title(&listed), "[4版] a → b → （2件の編集は記録なし） → e → f");
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
