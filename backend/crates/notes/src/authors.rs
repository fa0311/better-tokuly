//! Locked accounts, as this domain keeps track of them.
//!
//! The public notes of a locked account reach only its followers and
//! friends. Whether an account is locked is the social domain's; this
//! domain keeps a copy (`note_authors`) so SQL can decide, and marks every
//! public note of an unlocked author as `listed` — in search, in the hashtag
//! counts and trends, in the global timeline, renotable and quotable by
//! anyone.
//!
//! Who writes what, and why it stays consistent:
//! - [`Notes::sync_author`] (on `social.follow.settings_changed`, and after a
//!   post that found the copy out of date) reads the social domain, stores
//!   the copy and re-lists the author's public notes in batches, each under
//!   the author's lock taken exclusively;
//! - posting and visibility changes take the same lock shared, so they
//!   decide `listed` from a copy that no re-listing pass is halfway through;
//! - reads double-check the social domain for rows that are listed and
//!   reach someone who is not tied to the author
//!   ([`crate::visibility::Viewer::drop_newly_locked`]), which covers the
//!   moment between a lock and its event.
use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use tokuly_core::{events, outbound, prelude::*};
use tokuly_proto::common::UserSummary;
use tokuly_social::graph::{self, ContentAudience};

use crate::{
    Notes,
    events::{NOTE_VISIBILITY_CHANGED, NoteVisibilityChanged},
    hashtags,
    model::{NOTE_COLUMNS, NoteRow},
    posting::search_doc,
    visibility,
};

/// Public notes re-listed per transaction when an account is locked or unlocked.
const RELIST_BATCH: i64 = 200;

/// Takes the author's lock for the rest of `tx`: shared by everything that
/// decides whether one note is listed, exclusive for a re-listing pass.
pub(crate) async fn lock_author(tx: &mut Transaction<'_, Postgres>, author: Uuid, exclusive: bool) -> AppResult<()> {
    let sql = if exclusive {
        "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))"
    } else {
        "SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))"
    };
    sqlx::query(sql).bind(format!("notes:author:{author}")).execute(&mut **tx).await?;
    Ok(())
}

/// Whether the author is locked as this domain's copy knows it (`None`:
/// never heard of them).
pub(crate) async fn copy_says_locked(tx: &mut Transaction<'_, Postgres>, author: Uuid) -> AppResult<Option<bool>> {
    Ok(sqlx::query_scalar("SELECT locked FROM note_authors WHERE user_id = $1")
        .bind(author)
        .fetch_optional(&mut **tx)
        .await?)
}

/// Records an author's state the first time this domain hears of them.
/// Never overwrites: only [`Notes::sync_author`] changes the copy.
pub(crate) async fn record_first(tx: &mut Transaction<'_, Postgres>, author: Uuid, locked: bool) -> AppResult<()> {
    sqlx::query("INSERT INTO note_authors (user_id, locked) VALUES ($1, $2) ON CONFLICT (user_id) DO NOTHING")
        .bind(author)
        .bind(locked)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Brings the row's `listed` mark (and what follows from it: the hashtag
/// counts, the search document) to `listed`. Returns the row as it is now.
/// The row must be locked by `tx`.
pub(crate) async fn relist(
    tx: &mut Transaction<'_, Postgres>,
    note: NoteRow,
    listed: bool,
    author: &UserSummary,
) -> AppResult<NoteRow> {
    if note.listed == listed {
        return Ok(note);
    }
    let sql = format!("UPDATE notes AS n SET listed = $2 WHERE n.id = $1 RETURNING {NOTE_COLUMNS}");
    let updated: NoteRow =
        sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(note.id).bind(listed).fetch_one(&mut **tx).await?;
    hashtags::adjust(tx, &updated.hashtags, if listed { 1 } else { -1 }).await?;
    outbound::index(&mut **tx, search_doc(&updated, author)).await?;
    Ok(updated)
}

impl Notes {
    /// Whether the social domain says `author` is locked, right now.
    pub(crate) async fn is_locked_now(&self, author: Uuid) -> AppResult<bool> {
        Ok(graph::locked_among(&self.core, &[author]).await?.contains(&author))
    }

    /// Re-reads whether `author` is locked, stores it, and re-lists their
    /// public notes to match: a lock takes them out of search, hashtag
    /// counts and the global timeline, an unlock puts them back. For each
    /// note that changed, `notes.note.visibility_changed` tells the feed and
    /// notification domains (see `docs/events/notes.md` for what `visible_to`
    /// holds in this case). Idempotent; returns how many notes changed.
    pub async fn sync_author(&self, author: Uuid) -> AppResult<u64> {
        let core = &self.core;
        let active: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL)")
                .bind(author)
                .fetch_one(&core.db)
                .await?;
        if !active {
            return Ok(0); // erased: the account's notes go with it
        }
        // Everything from elsewhere is read before a transaction opens.
        let locked = self.is_locked_now(author).await?;
        let summary = tokuly_identity::summary(core, author).await?;
        let tied: HashSet<Uuid> = if locked {
            graph::audience_member_ids(core, author, ContentAudience::Followers).await?.into_iter().collect()
        } else {
            HashSet::new()
        };
        let mut changed = 0;
        loop {
            let mut tx = core.db.begin().await?;
            lock_author(&mut tx, author, true).await?;
            sqlx::query(
                "INSERT INTO note_authors (user_id, locked) VALUES ($1, $2)
                 ON CONFLICT (user_id) DO UPDATE SET locked = EXCLUDED.locked, updated_at = now()
                 WHERE note_authors.locked IS DISTINCT FROM EXCLUDED.locked",
            )
            .bind(author)
            .bind(locked)
            .execute(&mut *tx)
            .await?;
            // Public notes whose mark is the wrong way round: listed although
            // the author is locked, or not listed although they are not.
            let sql = format!(
                "SELECT {NOTE_COLUMNS} FROM notes n
                 WHERE n.author_id = $1 AND n.visibility = {public} AND n.listed = $2
                 ORDER BY n.id LIMIT $3 FOR UPDATE",
                public = visibility::PUBLIC,
            );
            let rows: Vec<NoteRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
                .bind(author)
                .bind(locked)
                .bind(RELIST_BATCH)
                .fetch_all(&mut *tx)
                .await?;
            if rows.is_empty() {
                tx.commit().await?;
                return Ok(changed);
            }
            let changed_at: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()").fetch_one(&mut *tx).await?;
            let told = if locked { told_about(&mut tx, &rows).await? } else { HashMap::new() };
            for row in rows {
                let row = relist(&mut tx, row, !locked, &summary).await?;
                let visible_to = locked.then(|| {
                    let mut people: Vec<Uuid> =
                        told.get(&row.id).into_iter().flatten().copied().filter(|id| tied.contains(id)).collect();
                    people.sort_unstable();
                    events::bounded_audience(people, None)
                });
                events::emit(
                    &mut *tx,
                    NOTE_VISIBILITY_CHANGED,
                    Some(author),
                    &NoteVisibilityChanged {
                        note_id: row.id,
                        author_id: author,
                        // What the note's audience is now in effect.
                        visibility: if locked { visibility::FOLLOWERS } else { visibility::PUBLIC },
                        visible_to,
                        changed_at,
                    },
                )
                .await?;
                changed += 1;
            }
            tx.commit().await?;
        }
    }
}

/// The people this domain told about each note, or who reacted to it: the
/// ones who may have something about the note in their inbox.
async fn told_about(tx: &mut Transaction<'_, Postgres>, rows: &[NoteRow]) -> AppResult<HashMap<Uuid, Vec<Uuid>>> {
    let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    let pairs: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT note_id, user_id FROM note_recipients WHERE note_id = ANY($1)
         UNION SELECT note_id, user_id FROM note_likes WHERE note_id = ANY($1)
         UNION SELECT note_id, user_id FROM note_renotes WHERE note_id = ANY($1)",
    )
    .bind(&ids)
    .fetch_all(&mut **tx)
    .await?;
    let mut told: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for (note_id, user_id) in pairs {
        told.entry(note_id).or_default().push(user_id);
    }
    Ok(told)
}
