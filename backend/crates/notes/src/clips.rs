//! Clips (クリップ): curated collections of notes, public or private.
//!
//! A clip shows each reader only the notes they may see, and counts only
//! those: its notes go through [`crate::visibility::sees_note`] like every
//! other list, so a clip never tells a stranger that a followers-only note
//! is in it. A clip itself exists for its owner, and for everyone else while
//! it is public and its owner is an account in good standing that no block
//! hides from them.
//!
//! `notes.clip_count` is the number of public clips holding a note. It moves
//! when a public clip gains or loses a note, when a clip turns public or
//! private, and when a public clip is deleted. Every such change locks the
//! clip's row first and then the note rows, in ID order, before touching the
//! items — so two changes of one clip apply one after the other and a note
//! being deleted at the same moment cannot wait on them in a circle.
use std::{collections::HashSet, time::Duration};

use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use tokuly_core::prelude::*;
use tokuly_proto::{
    common::PageInfo,
    notes::{Clip, Note, NoteCounters},
};

use crate::{
    Notes,
    model::{NOTE_COLUMNS, NoteRow},
    text,
    visibility::{Viewer, sees_note},
};

pub const MAX_NAME_CHARS: usize = 50;
pub const MAX_DESCRIPTION_CHARS: usize = 500;
/// Clips one person may have.
pub const MAX_CLIPS: i64 = 100;
/// Notes in one clip.
pub const MAX_NOTES_PER_CLIP: i64 = 1000;
/// Largest page of a clip list or of a clip's notes.
pub const MAX_PAGE: i64 = 50;

const CREATES_PER_HOUR: u32 = 30;
const CHANGES_PER_MINUTE: u32 = 60;
const ITEM_CHANGES_PER_MINUTE: u32 = 120;

/// The columns of a clip as `viewer` gets it (alias `c`): `note_count`
/// counts the notes in it the viewer may see.
fn clip_columns() -> String {
    format!(
        "c.id, c.owner_id, c.name, c.description, c.public, c.created_at, c.updated_at,
         (SELECT count(*)::int FROM note_clip_items ci JOIN notes n ON n.id = ci.note_id
          WHERE ci.clip_id = c.id AND {}) AS note_count",
        sees_note("n")
    )
}

/// SQL condition: the clip in alias `c` exists for the viewer.
fn shown(c: &str) -> String {
    format!(
        "({c}.owner_id = $1 OR ({c}.public AND {c}.owner_id <> ALL($3) AND EXISTS (SELECT 1 FROM users cu_ \
           WHERE cu_.id = {c}.owner_id AND cu_.banned_at IS NULL AND cu_.deleted_at IS NULL)))"
    )
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ClipRow {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub name: String,
    pub description: String,
    pub public: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub note_count: i32,
}

/// The name, description and audience of a clip, as asked for and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClipInput {
    pub name: String,
    pub description: String,
    pub public: bool,
}

impl ClipInput {
    pub(crate) fn validate(name: &str, description: &str, public: bool) -> AppResult<Self> {
        let name = text::clean(name).split_whitespace().collect::<Vec<_>>().join(" ");
        ensure!(
            !name.is_empty() && text::fits(&name, MAX_NAME_CHARS),
            invalid,
            "INVALID_NAME",
            "a clip name has 1-{MAX_NAME_CHARS} characters"
        );
        let description = text::clean(description);
        ensure!(
            text::fits(&description, MAX_DESCRIPTION_CHARS),
            invalid,
            "DESCRIPTION_TOO_LONG",
            "a clip description has at most {MAX_DESCRIPTION_CHARS} characters"
        );
        Ok(Self { name, description, public })
    }
}

/// A clip's note with the item it is there through (the keyset).
#[derive(sqlx::FromRow)]
struct ClippedNote {
    item_id: Uuid,
    #[sqlx(flatten)]
    note: NoteRow,
}

/// The keyset of clip lists: latest changed first.
type ClipCursor = (DateTime<Utc>, Uuid);

/// Locks one of `owner`'s clips, or `CLIP_NOT_FOUND`. Returns whether it is public.
async fn lock_own_clip(tx: &mut Transaction<'_, Postgres>, owner: Uuid, clip_id: Uuid) -> AppResult<bool> {
    let public: Option<bool> =
        sqlx::query_scalar("SELECT public FROM note_clips WHERE id = $1 AND owner_id = $2 FOR UPDATE")
            .bind(clip_id)
            .bind(owner)
            .fetch_optional(&mut **tx)
            .await?;
    public.or_not_found("CLIP_NOT_FOUND", "clip not found")
}

/// Locks notes in ID order and returns the ones that still exist.
async fn lock_notes(tx: &mut Transaction<'_, Postgres>, note_ids: &[Uuid]) -> AppResult<Vec<Uuid>> {
    if note_ids.is_empty() {
        return Ok(Vec::new());
    }
    Ok(sqlx::query_scalar("SELECT id FROM notes WHERE id = ANY($1) ORDER BY id FOR UPDATE")
        .bind(note_ids)
        .fetch_all(&mut **tx)
        .await?)
}

/// Moves the public clip count of notes (already locked) by `delta` and
/// returns their fresh counters.
async fn adjust_clip_counts(
    tx: &mut Transaction<'_, Postgres>,
    note_ids: &[Uuid],
    delta: i64,
) -> AppResult<Vec<NoteCounters>> {
    if note_ids.is_empty() || delta == 0 {
        return Ok(Vec::new());
    }
    let sql = format!(
        "UPDATE notes AS n SET clip_count = GREATEST(n.clip_count + $2, 0) WHERE n.id = ANY($1) RETURNING {NOTE_COLUMNS}"
    );
    let rows: Vec<NoteRow> =
        sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(note_ids).bind(delta).fetch_all(&mut **tx).await?;
    Ok(rows.iter().map(NoteRow::counters).collect())
}

/// The notes of a clip, in ID order.
async fn clip_note_ids(tx: &mut Transaction<'_, Postgres>, clip_id: Uuid) -> AppResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar("SELECT note_id FROM note_clip_items WHERE clip_id = $1 ORDER BY note_id")
        .bind(clip_id)
        .fetch_all(&mut **tx)
        .await?)
}

/// Deletes one of `owner`'s clips inside `tx`, taking it off the clip counts
/// of its notes if it was public. Returns the counters that moved.
async fn delete_clip_in(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
    clip_id: Uuid,
) -> AppResult<Vec<NoteCounters>> {
    let public = lock_own_clip(tx, owner, clip_id).await?;
    let touched = if public {
        let notes = clip_note_ids(tx, clip_id).await?;
        let locked = lock_notes(tx, &notes).await?;
        adjust_clip_counts(tx, &locked, -1).await?
    } else {
        Vec::new()
    };
    sqlx::query("DELETE FROM note_clips WHERE id = $1").bind(clip_id).execute(&mut **tx).await?;
    Ok(touched)
}

impl Notes {
    async fn clip_rate_limit(&self, user_id: Uuid, what: &str, max: u32, window: Duration) -> AppResult<()> {
        self.core.cache.rate_limit(&format!("notes:clip-{what}:{user_id}"), max, window).await
    }

    /// The clip as the viewer may see it, or `CLIP_NOT_FOUND`.
    pub(crate) async fn clip_for(&self, viewer: &Viewer, clip_id: Uuid) -> AppResult<ClipRow> {
        let sql = format!("SELECT {} FROM note_clips c WHERE c.id = $7 AND {}", clip_columns(), shown("c"));
        let row: Option<ClipRow> = viewer.query_as(sql).bind(clip_id).fetch_optional(&self.core.db).await?;
        row.or_not_found("CLIP_NOT_FOUND", "clip not found")
    }

    /// Clips as API messages; `holding` names the clips that hold the note
    /// asked about.
    pub(crate) async fn clip_views(&self, rows: &[ClipRow], holding: &HashSet<Uuid>) -> AppResult<Vec<Clip>> {
        let mut owners: Vec<Uuid> = rows.iter().map(|row| row.owner_id).collect();
        owners.sort_unstable();
        owners.dedup();
        let users = tokuly_identity::summaries(&self.core, &owners).await?;
        Ok(rows
            .iter()
            .map(|row| Clip {
                id: row.id.to_string(),
                owner: tokuly_identity::users::pick(&users, row.owner_id).into(),
                name: row.name.clone(),
                description: row.description.clone(),
                public: row.public,
                note_count: row.note_count,
                created_at: ts(row.created_at),
                updated_at: ts(row.updated_at),
                contains_note: holding.contains(&row.id),
                ..Default::default()
            })
            .collect())
    }

    async fn clip_view(&self, viewer: &Viewer, clip_id: Uuid, contains_note: bool) -> AppResult<Clip> {
        let row = self.clip_for(viewer, clip_id).await?;
        let holding = if contains_note { HashSet::from([row.id]) } else { HashSet::new() };
        Ok(self.clip_views(std::slice::from_ref(&row), &holding).await?.pop().expect("one clip in, one out"))
    }

    /// Creates a clip of `owner` (at most [`MAX_CLIPS`]).
    pub(crate) async fn create_clip(&self, owner: Uuid, input: ClipInput) -> AppResult<Clip> {
        self.clip_rate_limit(owner, "create", CREATES_PER_HOUR, Duration::from_secs(3600)).await?;
        let mut tx = self.core.db.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("notes:clips:{owner}"))
            .execute(&mut *tx)
            .await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM note_clips WHERE owner_id = $1")
            .bind(owner)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(count < MAX_CLIPS, precondition, "TOO_MANY_CLIPS", "you can have at most {MAX_CLIPS} clips");
        let clip_id: Uuid = sqlx::query_scalar(
            "INSERT INTO note_clips (owner_id, name, description, public) VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(owner)
        .bind(&input.name)
        .bind(&input.description)
        .bind(input.public)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        self.clip_view(&Viewer::load(&self.core, Some(owner)).await?, clip_id, false).await
    }

    /// Changes one of `owner`'s clips. Turning it public or private moves
    /// the clip count of every note in it.
    pub(crate) async fn update_clip(&self, owner: Uuid, clip_id: Uuid, input: ClipInput) -> AppResult<Clip> {
        self.clip_rate_limit(owner, "change", CHANGES_PER_MINUTE, Duration::from_secs(60)).await?;
        let mut tx = self.core.db.begin().await?;
        let was_public = lock_own_clip(&mut tx, owner, clip_id).await?;
        let touched = if was_public != input.public {
            let notes = clip_note_ids(&mut tx, clip_id).await?;
            let locked = lock_notes(&mut tx, &notes).await?;
            adjust_clip_counts(&mut tx, &locked, if input.public { 1 } else { -1 }).await?
        } else {
            Vec::new()
        };
        sqlx::query("UPDATE note_clips SET name = $2, description = $3, public = $4, updated_at = now() WHERE id = $1")
            .bind(clip_id)
            .bind(&input.name)
            .bind(&input.description)
            .bind(input.public)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.push_counters(touched).await;
        self.clip_view(&Viewer::load(&self.core, Some(owner)).await?, clip_id, false).await
    }

    /// Deletes one of `owner`'s clips.
    pub(crate) async fn delete_clip(&self, owner: Uuid, clip_id: Uuid) -> AppResult<()> {
        self.clip_rate_limit(owner, "change", CHANGES_PER_MINUTE, Duration::from_secs(60)).await?;
        let mut tx = self.core.db.begin().await?;
        let touched = delete_clip_in(&mut tx, owner, clip_id).await?;
        tx.commit().await?;
        self.push_counters(touched).await;
        Ok(())
    }

    /// Puts a note the owner may see into one of their clips (at most
    /// [`MAX_NOTES_PER_CLIP`]). Adding a note twice changes nothing.
    pub(crate) async fn add_note_to_clip(&self, owner: Uuid, clip_id: Uuid, note_id: Uuid) -> AppResult<Clip> {
        self.clip_rate_limit(owner, "items", ITEM_CHANGES_PER_MINUTE, Duration::from_secs(60)).await?;
        let viewer = Viewer::load(&self.core, Some(owner)).await?;
        // Whether the clip is the caller's comes first: a hidden note must
        // not be told apart from a missing one through someone else's clip.
        let clip = self.clip_for(&viewer, clip_id).await?;
        ensure!(clip.owner_id == owner, not_found, "CLIP_NOT_FOUND", "clip not found");
        let note = self.require_note(&viewer, note_id).await?;

        let mut tx = self.core.db.begin().await?;
        let public = lock_own_clip(&mut tx, owner, clip.id).await?;
        let locked = lock_notes(&mut tx, &[note.id]).await?;
        ensure!(!locked.is_empty(), not_found, "NOTE_NOT_FOUND", "note not found");
        let (count, already): (i64, bool) = sqlx::query_as(
            "SELECT count(*), coalesce(bool_or(note_id = $2), false) FROM note_clip_items WHERE clip_id = $1",
        )
        .bind(clip.id)
        .bind(note.id)
        .fetch_one(&mut *tx)
        .await?;
        let mut touched = Vec::new();
        if !already {
            ensure!(
                count < MAX_NOTES_PER_CLIP,
                precondition,
                "TOO_MANY_CLIPPED_NOTES",
                "a clip holds at most {MAX_NOTES_PER_CLIP} notes"
            );
            sqlx::query("INSERT INTO note_clip_items (clip_id, note_id) VALUES ($1, $2)")
                .bind(clip.id)
                .bind(note.id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE note_clips SET updated_at = now() WHERE id = $1")
                .bind(clip.id)
                .execute(&mut *tx)
                .await?;
            if public {
                touched = adjust_clip_counts(&mut tx, &locked, 1).await?;
            }
        }
        tx.commit().await?;
        self.push_counters(touched).await;
        self.clip_view(&viewer, clip.id, true).await
    }

    /// Takes a note out of one of `owner`'s clips, if it is there (the
    /// note need not be visible any more).
    pub(crate) async fn remove_note_from_clip(&self, owner: Uuid, clip_id: Uuid, note_id: Uuid) -> AppResult<Clip> {
        self.clip_rate_limit(owner, "items", ITEM_CHANGES_PER_MINUTE, Duration::from_secs(60)).await?;
        let mut tx = self.core.db.begin().await?;
        let public = lock_own_clip(&mut tx, owner, clip_id).await?;
        let locked = lock_notes(&mut tx, &[note_id]).await?;
        let removed = sqlx::query("DELETE FROM note_clip_items WHERE clip_id = $1 AND note_id = $2")
            .bind(clip_id)
            .bind(note_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        let mut touched = Vec::new();
        if removed > 0 {
            sqlx::query("UPDATE note_clips SET updated_at = now() WHERE id = $1")
                .bind(clip_id)
                .execute(&mut *tx)
                .await?;
            if public {
                touched = adjust_clip_counts(&mut tx, &locked, -1).await?;
            }
        }
        tx.commit().await?;
        self.push_counters(touched).await;
        self.clip_view(&Viewer::load(&self.core, Some(owner)).await?, clip_id, false).await
    }

    /// `owner`'s clips as the viewer may see them (all of them for the owner,
    /// public ones for others), latest changed first. With `note_id`, each
    /// says whether it holds that note — only if the viewer may see the note.
    pub(crate) async fn list_clips(
        &self,
        viewer: &Viewer,
        owner: Uuid,
        note_id: Option<Uuid>,
        page: &Page,
    ) -> AppResult<(Vec<Clip>, PageInfo)> {
        self.require_user(viewer, owner).await?;
        let cursor: Option<ClipCursor> = page.decode()?;
        let sql = format!(
            "SELECT {} FROM note_clips c
             WHERE c.owner_id = $7 AND {} AND ($8::timestamptz IS NULL OR (c.updated_at, c.id) < ($8, $9))
             ORDER BY c.updated_at DESC, c.id DESC LIMIT $10",
            clip_columns(),
            shown("c"),
        );
        let mut rows: Vec<ClipRow> = viewer
            .query_as(sql)
            .bind(owner)
            .bind(cursor.map(|(at, _)| at))
            .bind(cursor.map(|(_, id)| id))
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
        let info = page.finish(&mut rows, |row| (row.updated_at, row.id));
        let holding = match note_id {
            Some(note_id) if viewer.can_see_note(&self.core.db, note_id).await? => {
                let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
                let held: Vec<Uuid> =
                    sqlx::query_scalar("SELECT clip_id FROM note_clip_items WHERE note_id = $1 AND clip_id = ANY($2)")
                        .bind(note_id)
                        .bind(&ids)
                        .fetch_all(&self.core.db)
                        .await?;
                held.into_iter().collect()
            }
            _ => HashSet::new(),
        };
        Ok((self.clip_views(&rows, &holding).await?, info))
    }

    /// The notes of a clip the viewer may see, most recently added first.
    pub(crate) async fn clip_notes(
        &self,
        viewer: &Viewer,
        clip_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<Note>, PageInfo)> {
        let clip = self.clip_for(viewer, clip_id).await?;
        let cursor: Option<Uuid> = page.decode()?;
        let sql = format!(
            "SELECT i.id AS item_id, {NOTE_COLUMNS} FROM note_clip_items i JOIN notes n ON n.id = i.note_id
             WHERE i.clip_id = $7 AND ($8::uuid IS NULL OR i.id < $8) AND {}
             ORDER BY i.id DESC LIMIT $9",
            sees_note("n")
        );
        let mut rows: Vec<ClippedNote> =
            viewer.query_as(sql).bind(clip.id).bind(cursor).bind(page.fetch()).fetch_all(&self.core.db).await?;
        let info = page.finish(&mut rows, |row| row.item_id);
        viewer.drop_newly_locked(&self.core, &mut rows, |row| &row.note).await?;
        let notes: Vec<NoteRow> = rows.into_iter().map(|row| row.note).collect();
        Ok((self.hydrate(viewer, &notes).await?, info))
    }

    /// The public clips holding a note the viewer may see, and the viewer's
    /// own, latest changed first.
    pub(crate) async fn note_clips(
        &self,
        viewer: &Viewer,
        note_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<Clip>, PageInfo)> {
        let note = self.require_note(viewer, note_id).await?;
        let cursor: Option<ClipCursor> = page.decode()?;
        let sql = format!(
            "SELECT {} FROM note_clips c JOIN note_clip_items held ON held.clip_id = c.id AND held.note_id = $7
             WHERE {} AND ($8::timestamptz IS NULL OR (c.updated_at, c.id) < ($8, $9))
             ORDER BY c.updated_at DESC, c.id DESC LIMIT $10",
            clip_columns(),
            shown("c"),
        );
        let mut rows: Vec<ClipRow> = viewer
            .query_as(sql)
            .bind(note.id)
            .bind(cursor.map(|(at, _)| at))
            .bind(cursor.map(|(_, id)| id))
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
        let info = page.finish(&mut rows, |row| (row.updated_at, row.id));
        let holding: HashSet<Uuid> = rows.iter().map(|row| row.id).collect();
        Ok((self.clip_views(&rows, &holding).await?, info))
    }

    /// `identity.user.deleted`: the account's clips go, each taken off the
    /// clip counts of its notes.
    pub(crate) async fn erase_clips_of(&self, user_id: Uuid) -> AppResult<()> {
        let clips: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM note_clips WHERE owner_id = $1 ORDER BY id")
            .bind(user_id)
            .fetch_all(&self.core.db)
            .await?;
        for clip_id in clips {
            let mut tx = self.core.db.begin().await?;
            let touched = match delete_clip_in(&mut tx, user_id, clip_id).await {
                Ok(touched) => touched,
                // Gone in the meantime: done.
                Err(error) if error.is_not_found() => continue,
                Err(error) => return Err(error),
            };
            tx.commit().await?;
            self.push_counters(touched).await;
        }
        Ok(())
    }
}

/// The `clips` section of the personal data export.
pub(crate) async fn export(db: &sqlx::PgPool, user_id: Uuid) -> AppResult<serde_json::Value> {
    let clips: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', c.id, 'name', c.name, 'description', c.description, 'public', c.public,
                'created_at', c.created_at, 'updated_at', c.updated_at,
                'note_ids', (SELECT COALESCE(jsonb_agg(i.note_id ORDER BY i.id), '[]') FROM note_clip_items i WHERE i.clip_id = c.id))
         FROM note_clips c WHERE c.owner_id = $1 ORDER BY c.id",
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;
    Ok(serde_json::Value::Array(clips))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_input_is_checked() {
        let input = ClipInput::validate("  あとで   読む ", " メモ ", true).unwrap();
        assert_eq!((input.name.as_str(), input.description.as_str(), input.public), ("あとで 読む", "メモ", true));
        assert_eq!(ClipInput::validate("", "", false).unwrap_err().reason, "INVALID_NAME");
        assert_eq!(ClipInput::validate(&"a".repeat(51), "", false).unwrap_err().reason, "INVALID_NAME");
        assert_eq!(ClipInput::validate("ok", &"a".repeat(501), false).unwrap_err().reason, "DESCRIPTION_TOO_LONG");
    }

    #[test]
    fn clip_conditions_use_the_viewer_parameters() {
        for sql in [shown("c"), clip_columns()] {
            assert!(!sql.contains("$7"), "{sql}");
            assert_eq!(sql.matches('(').count(), sql.matches(')').count(), "{sql}");
        }
    }
}
