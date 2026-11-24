//! Every list of notes: the timelines, a note's thread and its quotes.
//!
//! All of them are keyset-paginated on a time-ordered ID (the note's own,
//! or the renote's / like's where a row exists because of one) and embed
//! the visibility condition, so a page never contains a note its viewer may
//! not see — whatever list it came from.
//!
//! Parameters of the queries here: `$1`–`$6` the viewer (see
//! [`crate::visibility`]), then, for a list about something (a user, a
//! hashtag, a note), `$7` that something, `$8` the cursor and `$9` the
//! number of rows to fetch.
//!
//! On top of what SQL decides, every page passes
//! [`Viewer::drop_newly_locked`]: an account locked a moment ago keeps its
//! public notes from people who do not follow it even before this domain's
//! copy of the lock is updated.
use sqlx::Postgres;
use tokuly_core::prelude::*;
use tokuly_proto::{
    common::PageInfo,
    notes::{Hashtag, Note, TimelineEntry, UserTimelineFilter},
};

use crate::{
    Notes, hashtags,
    hydrate::EntryRow,
    model::{NOTE_COLUMNS, NoteRow},
    text,
    visibility::{Viewer, listed, sees_note},
};

/// Largest page of any note list.
pub const MAX_PAGE: i64 = 100;
/// How far up a thread is followed.
const MAX_ANCESTORS: i32 = 50;

/// Columns of a timeline row that is a note in its own right.
fn plain() -> String {
    format!("n.id AS entry_id, NULL::uuid AS renoter_id, NULL::timestamptz AS renoted_at, {NOTE_COLUMNS}")
}

/// A timeline made of notes selected by `filter` plus the notes renoted by
/// the users in `renoters` (a comparison completing `user_id …`, e.g.
/// `= ANY($7)`), merged newest first.
///
/// Each note appears once: where its latest renote by one of those users
/// puts it, or at its own position when none of them renoted it. Without
/// that, a note posted by one friend and renoted by another would show up
/// twice, pages apart.
fn with_renotes(filter: &str, renoters: &str) -> String {
    let visible = sees_note("n");
    format!(
        "SELECT e.entry_id, e.renoter_id, e.renoted_at, {NOTE_COLUMNS}
         FROM (
             (SELECT n.id AS entry_id, n.id AS note_id, NULL::uuid AS renoter_id, NULL::timestamptz AS renoted_at
              FROM notes n
              WHERE {filter} AND ($8::uuid IS NULL OR n.id < $8) AND {visible}
                AND NOT EXISTS (SELECT 1 FROM note_renotes later WHERE later.note_id = n.id AND later.user_id {renoters})
              ORDER BY n.id DESC LIMIT $9)
             UNION ALL
             (SELECT r.id, r.note_id, r.user_id, r.created_at
              FROM note_renotes r JOIN notes n ON n.id = r.note_id
              WHERE r.user_id {renoters} AND ($8::uuid IS NULL OR r.id < $8) AND {visible}
                AND NOT EXISTS (SELECT 1 FROM note_renotes later WHERE later.note_id = r.note_id AND later.user_id {renoters} AND later.id > r.id)
              ORDER BY r.id DESC LIMIT $9)
         ) e
         JOIN notes n ON n.id = e.note_id
         ORDER BY e.entry_id DESC LIMIT $9"
    )
}

impl Notes {
    /// Runs a timeline query and hydrates its page.
    async fn entries<P>(
        &self,
        viewer: &Viewer,
        page: &Page,
        sql: String,
        about: P,
    ) -> AppResult<(Vec<TimelineEntry>, PageInfo)>
    where
        P: 'static + Send + for<'q> sqlx::Encode<'q, Postgres> + sqlx::Type<Postgres>,
    {
        let cursor: Option<Uuid> = page.decode()?;
        let rows: Vec<EntryRow> =
            viewer.query_as(sql).bind(about).bind(cursor).bind(page.fetch()).fetch_all(&self.core.db).await?;
        self.page_of(viewer, page, rows).await
    }

    /// Trims fetched timeline rows to the page asked for and hydrates them.
    async fn page_of(
        &self,
        viewer: &Viewer,
        page: &Page,
        mut rows: Vec<EntryRow>,
    ) -> AppResult<(Vec<TimelineEntry>, PageInfo)> {
        let info = page.finish(&mut rows, |row| row.entry_id);
        // After the page is cut, so the cursor stays where SQL left it.
        viewer.drop_newly_locked(&self.core, &mut rows, |row| &row.note).await?;
        Ok((self.hydrate_entries(viewer, rows).await?, info))
    }

    /// Notes, quotes and renotes by the viewer and their friends, plus the
    /// replies those people wrote to each other.
    pub(crate) async fn home_timeline(
        &self,
        viewer: &Viewer,
        page: &Page,
    ) -> AppResult<(Vec<TimelineEntry>, PageInfo)> {
        let sql = with_renotes(
            "n.author_id = ANY($7) AND (n.reply_to_id IS NULL OR n.reply_to_author_id = ANY($7))",
            "= ANY($7)",
        );
        self.entries(viewer, page, sql, viewer.me_and_friends()).await
    }

    /// Notes, quotes and renotes by the viewer and the accounts they follow
    /// (leaving out those they muted), plus the replies those people wrote
    /// to each other.
    pub(crate) async fn following_timeline(
        &self,
        viewer: &Viewer,
        user_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<TimelineEntry>, PageInfo)> {
        let (following, muted) = tokio::try_join!(
            tokuly_social::graph::following_ids(&self.core, user_id),
            tokuly_social::graph::muted_followee_ids(&self.core, user_id),
        )?;
        let people: Vec<Uuid> =
            std::iter::once(user_id).chain(following.into_iter().filter(|id| !muted.contains(id))).collect();
        let sql = with_renotes(
            "n.author_id = ANY($7) AND (n.reply_to_id IS NULL OR n.reply_to_author_id = ANY($7))",
            "= ANY($7)",
        );
        self.entries(viewer, page, sql, people).await
    }

    /// Every note that is not a reply and that everyone may see.
    pub(crate) async fn global_timeline(
        &self,
        viewer: &Viewer,
        page: &Page,
    ) -> AppResult<(Vec<TimelineEntry>, PageInfo)> {
        // A list about nothing in particular: `$7` is the cursor, `$8` the
        // number of rows. (The visibility is spelled out, not a parameter,
        // so the partial index on public top-level notes applies.)
        let cursor: Option<Uuid> = page.decode()?;
        let sql = format!(
            "SELECT {columns} FROM notes n
             WHERE {listed} AND n.reply_to_id IS NULL
               AND ($7::uuid IS NULL OR n.id < $7) AND {visible}
             ORDER BY n.id DESC LIMIT $8",
            columns = plain(),
            listed = listed("n"),
            visible = sees_note("n"),
        );
        let rows: Vec<EntryRow> = viewer.query_as(sql).bind(cursor).bind(page.fetch()).fetch_all(&self.core.db).await?;
        self.page_of(viewer, page, rows).await
    }

    /// One user's notes as the viewer may see them, and — on the first page
    /// of the notes tab — the user's pinned note.
    pub(crate) async fn user_timeline(
        &self,
        viewer: &Viewer,
        user_id: Uuid,
        filter: UserTimelineFilter,
        page: &Page,
    ) -> AppResult<(Vec<TimelineEntry>, PageInfo, Option<Note>)> {
        self.require_user(viewer, user_id).await?;
        let visible = sees_note("n");
        let sql = match filter {
            UserTimelineFilter::USER_TIMELINE_FILTER_UNSPECIFIED | UserTimelineFilter::USER_TIMELINE_FILTER_NOTES => {
                with_renotes("n.author_id = $7 AND n.reply_to_id IS NULL", "= $7")
            }
            UserTimelineFilter::USER_TIMELINE_FILTER_WITH_REPLIES => with_renotes("n.author_id = $7", "= $7"),
            UserTimelineFilter::USER_TIMELINE_FILTER_MEDIA => format!(
                "SELECT {columns} FROM notes n
                 WHERE n.author_id = $7 AND n.asset_ids <> '{{}}' AND ($8::uuid IS NULL OR n.id < $8) AND {visible}
                 ORDER BY n.id DESC LIMIT $9",
                columns = plain(),
            ),
            UserTimelineFilter::USER_TIMELINE_FILTER_LIKES => format!(
                "SELECT l.id AS entry_id, NULL::uuid AS renoter_id, NULL::timestamptz AS renoted_at, {NOTE_COLUMNS}
                 FROM note_likes l JOIN notes n ON n.id = l.note_id
                 WHERE l.user_id = $7 AND ($8::uuid IS NULL OR l.id < $8) AND {visible}
                 ORDER BY l.id DESC LIMIT $9"
            ),
        };
        let first_page = page.cursor.is_none();
        let (entries, info) = self.entries(viewer, page, sql, user_id).await?;
        let notes_tab = matches!(
            filter,
            UserTimelineFilter::USER_TIMELINE_FILTER_UNSPECIFIED | UserTimelineFilter::USER_TIMELINE_FILTER_NOTES
        );
        let pinned = if first_page && notes_tab {
            let sql = format!(
                "SELECT {NOTE_COLUMNS} FROM notes n WHERE n.author_id = $7 AND n.pinned_at IS NOT NULL AND {visible}"
            );
            let mut row: Vec<NoteRow> = viewer.query_as(sql).bind(user_id).fetch_all(&self.core.db).await?;
            viewer.drop_newly_locked(&self.core, &mut row, |row| row).await?;
            let row = row.into_iter().next();
            match row {
                Some(row) => Some(self.hydrate_one(viewer, &row).await?),
                None => None,
            }
        } else {
            None
        };
        Ok((entries, info, pinned))
    }

    /// Fails with `USER_NOT_FOUND` unless `user_id` is an account in good
    /// standing that no block hides from the viewer.
    pub(crate) async fn require_user(&self, viewer: &Viewer, user_id: Uuid) -> AppResult<()> {
        let active: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND banned_at IS NULL AND deleted_at IS NULL)",
        )
        .bind(user_id)
        .fetch_one(&self.core.db)
        .await?;
        ensure!(
            viewer.is(user_id) || (active && !viewer.hides(user_id)),
            not_found,
            "USER_NOT_FOUND",
            "user not found"
        );
        Ok(())
    }

    /// Notes carrying a hashtag, and the hashtag itself (with a count of
    /// zero when nobody used it yet).
    pub(crate) async fn hashtag_timeline(
        &self,
        viewer: &Viewer,
        tag: &str,
        page: &Page,
    ) -> AppResult<(Hashtag, Vec<TimelineEntry>, PageInfo)> {
        let tag = text::normalize_hashtag(tag).ok_or_else(|| AppError::invalid("INVALID_HASHTAG", "not a hashtag"))?;
        let Some(hashtag) = hashtags::find(&self.core, &tag).await? else {
            page.decode::<Uuid>()?;
            return Ok((Hashtag { tag, ..Default::default() }, Vec::new(), PageInfo::default()));
        };
        let sql = format!(
            "SELECT {columns} FROM note_hashtags nh JOIN notes n ON n.id = nh.note_id
             WHERE nh.hashtag_id = $7 AND ($8::uuid IS NULL OR nh.note_id < $8) AND {visible}
             ORDER BY nh.note_id DESC LIMIT $9",
            columns = plain(),
            visible = sees_note("n"),
        );
        let (entries, info) = self.entries(viewer, page, sql, hashtag.id).await?;
        Ok((hashtag.to_proto(), entries, info))
    }

    /// Notes by other people that mention `user_id`, answer one of their
    /// notes or are addressed to them.
    pub(crate) async fn mentions_timeline(
        &self,
        viewer: &Viewer,
        user_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<TimelineEntry>, PageInfo)> {
        let visible = sees_note("n");
        // Three index scans merged, rather than one scan over an OR.
        let sql = format!(
            "SELECT {columns}
             FROM (
                 (SELECT n.id FROM notes n
                  WHERE n.reply_to_author_id = $7 AND n.author_id <> $7 AND ($8::uuid IS NULL OR n.id < $8) AND {visible}
                  ORDER BY n.id DESC LIMIT $9)
                 UNION
                 (SELECT m.note_id FROM note_mentions m JOIN notes n ON n.id = m.note_id
                  WHERE m.user_id = $7 AND n.author_id <> $7 AND ($8::uuid IS NULL OR m.note_id < $8) AND {visible}
                  ORDER BY m.note_id DESC LIMIT $9)
                 UNION
                 (SELECT n.id FROM notes n
                  WHERE n.direct_user_ids @> ARRAY[$7::uuid] AND n.direct_user_ids <> '{{}}' AND n.author_id <> $7
                    AND ($8::uuid IS NULL OR n.id < $8) AND {visible}
                  ORDER BY n.id DESC LIMIT $9)
             ) found
             JOIN notes n ON n.id = found.id
             ORDER BY n.id DESC LIMIT $9",
            columns = plain(),
        );
        self.entries(viewer, page, sql, user_id).await
    }

    /// Quote notes of a note, newest first.
    pub(crate) async fn quotes(&self, viewer: &Viewer, note_id: Uuid, page: &Page) -> AppResult<(Vec<Note>, PageInfo)> {
        self.require_note(viewer, note_id).await?;
        let cursor: Option<Uuid> = page.decode()?;
        let sql = format!(
            "SELECT {NOTE_COLUMNS} FROM notes n
             WHERE n.quote_of_id = $7 AND ($8::uuid IS NULL OR n.id < $8) AND {}
             ORDER BY n.id DESC LIMIT $9",
            sees_note("n")
        );
        let mut rows: Vec<NoteRow> =
            viewer.query_as(sql).bind(note_id).bind(cursor).bind(page.fetch()).fetch_all(&self.core.db).await?;
        let info = page.finish(&mut rows, |row| row.id);
        viewer.drop_newly_locked(&self.core, &mut rows, |row| row).await?;
        Ok((self.hydrate(viewer, &rows).await?, info))
    }

    /// A note in its conversation: the notes it answers (the start of the
    /// conversation first; only on the first page), the note, and a page of
    /// its direct replies, oldest first.
    pub(crate) async fn thread(
        &self,
        viewer: &Viewer,
        note_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<Note>, Note, Vec<Note>, PageInfo)> {
        let note = self.require_note(viewer, note_id).await?;
        let cursor: Option<Uuid> = page.decode()?;
        let visible = sees_note("n");

        let ancestors: Vec<NoteRow> = match note.reply_to_id.filter(|_| cursor.is_none()) {
            Some(parent) => {
                // The chain stops at the first note that is gone or hidden.
                let sql = format!(
                    "WITH RECURSIVE chain AS (
                         SELECT n.id, n.reply_to_id, 1 AS depth FROM notes n WHERE n.id = $7 AND {visible}
                         UNION ALL
                         SELECT n.id, n.reply_to_id, chain.depth + 1
                         FROM chain JOIN notes n ON n.id = chain.reply_to_id
                         WHERE chain.depth < {MAX_ANCESTORS} AND {visible}
                     )
                     SELECT {NOTE_COLUMNS} FROM chain JOIN notes n ON n.id = chain.id ORDER BY chain.depth DESC"
                );
                viewer.query_as(sql).bind(parent).fetch_all(&self.core.db).await?
            }
            None => Vec::new(),
        };
        let sql = format!(
            "SELECT {NOTE_COLUMNS} FROM notes n
             WHERE n.reply_to_id = $7 AND ($8::uuid IS NULL OR n.id > $8) AND {visible}
             ORDER BY n.id LIMIT $9"
        );
        let mut replies: Vec<NoteRow> =
            viewer.query_as(sql).bind(note.id).bind(cursor).bind(page.fetch()).fetch_all(&self.core.db).await?;
        let info = page.finish(&mut replies, |row| row.id);
        viewer.drop_newly_locked(&self.core, &mut replies, |row| row).await?;
        // The chain above the note stops at a newly locked author too.
        let mut kept = Vec::with_capacity(ancestors.len());
        let locked = viewer.newly_locked(&self.core, &ancestors.iter().collect::<Vec<_>>()).await?;
        for row in ancestors.into_iter().rev() {
            if row.listed && locked.contains(&row.author_id) && !viewer.is_tied_to(row.author_id) {
                break;
            }
            kept.push(row);
        }
        kept.reverse();
        let ancestors = kept;

        // One hydration for the whole thread: its notes share authors.
        let mut all = ancestors;
        let ancestor_count = all.len();
        all.push(note);
        all.extend(replies);
        let mut notes = self.hydrate(viewer, &all).await?;
        let replies = notes.split_off(ancestor_count + 1);
        let note = notes.pop().expect("the thread's own note was hydrated");
        Ok((notes, note, replies, info))
    }

    /// The notes among `note_ids` the viewer may see, in the order asked for.
    pub(crate) async fn batch(&self, viewer: &Viewer, note_ids: &[Uuid]) -> AppResult<Vec<Note>> {
        let mut found = self.visible_notes(viewer, note_ids).await?;
        // A note asked for twice is returned once, at its first position.
        let rows: Vec<NoteRow> = note_ids.iter().filter_map(|id| found.remove(id)).collect();
        self.hydrate(viewer, &rows).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_queries_number_their_parameters_consistently() {
        for sql in [with_renotes("n.author_id = $7", "= $7"), plain()] {
            assert!(!sql.contains("$10"), "{sql}");
            assert_eq!(sql.matches('(').count(), sql.matches(')').count(), "{sql}");
        }
    }
}
