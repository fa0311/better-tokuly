//! Turning note rows into the `Note` messages a particular viewer gets:
//! authors, stamps, the quoted note, what the note answers, and what the
//! viewer did with it.
//!
//! Everything a note refers to is loaded in bulk, so a page of thirty notes
//! costs the same handful of queries as one note.
use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use tokuly_core::prelude::*;
use tokuly_identity::users::pick;
use tokuly_proto::{
    common::{StampRef, UserSummary},
    notes::{Note, NoteViewerState, QuotedNote, RenoteInfo, ReplyTarget, TimelineEntry},
};

use crate::{
    Notes,
    model::{NOTE_COLUMNS, NoteRow},
    visibility::{Viewer, sees_note},
};

/// One row of a timeline before hydration: a note, possibly there because
/// someone renoted it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct EntryRow {
    /// Sort key and cursor of the row: the note's ID, or the renote's (or
    /// like's) own ID where the row exists because of one.
    pub entry_id: Uuid,
    pub renoter_id: Option<Uuid>,
    pub renoted_at: Option<DateTime<Utc>>,
    #[sqlx(flatten)]
    pub note: NoteRow,
}

/// Everything the notes of one response refer to.
struct Context<'a> {
    viewer: &'a Viewer,
    users: HashMap<Uuid, UserSummary>,
    stamps: HashMap<Uuid, StampRef>,
    /// Referenced notes (quoted, or replied to) the viewer may see.
    referenced: HashMap<Uuid, NoteRow>,
    liked: HashSet<Uuid>,
    renoted: HashSet<Uuid>,
}

impl Context<'_> {
    /// `row` as a `Note`. `with_quote` is false for a note shown inside
    /// another one: a quote is rendered one level deep.
    fn note(&self, row: &NoteRow, with_quote: bool) -> Note {
        let reply_to = row.reply_to_id.map(|note_id| ReplyTarget {
            note_id: note_id.to_string(),
            // Whom a reply answers is shown even when the answered note is
            // gone — but not across a block, where the two do not exist for
            // each other.
            author: row
                .reply_to_author_id
                .filter(|author| !self.viewer.hides(*author))
                .map(|author| pick(&self.users, author))
                .into(),
            unavailable: !self.referenced.contains_key(&note_id),
            ..Default::default()
        });
        let quote = row.quote_of_id.filter(|_| with_quote).map(|note_id| {
            let quoted = self.referenced.get(&note_id);
            QuotedNote {
                note_id: note_id.to_string(),
                note: quoted.map(|quoted| self.note(quoted, false)).into(),
                unavailable: quoted.is_none(),
                ..Default::default()
            }
        });
        Note {
            id: row.id.to_string(),
            author: pick(&self.users, row.author_id).into(),
            text: row.text.clone(),
            attachments: row.attachments(),
            stamp: row.stamp_id.and_then(|id| self.stamps.get(&id).cloned()).into(),
            place: row.place().into(),
            visibility: row.visibility.into(),
            content_warning: row.content_warning,
            reply_to: reply_to.into(),
            quote: quote.into(),
            hashtags: row.hashtags.clone(),
            mentions: row.mentions(),
            link_preview: row.link_preview().into(),
            counters: row.counters().into(),
            viewer: self
                .viewer
                .id
                .map(|_| NoteViewerState {
                    liked: self.liked.contains(&row.id),
                    renoted: self.renoted.contains(&row.id),
                    ..Default::default()
                })
                .into(),
            pinned: row.pinned_at.is_some(),
            created_at: ts(row.created_at),
            ..Default::default()
        }
    }
}

impl Notes {
    /// Loads the notes among `ids` that the viewer may see, keyed by ID.
    pub(crate) async fn visible_notes(&self, viewer: &Viewer, ids: &[Uuid]) -> AppResult<HashMap<Uuid, NoteRow>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let sql = format!("SELECT {NOTE_COLUMNS} FROM notes n WHERE n.id = ANY($4) AND {}", sees_note("n"));
        let rows: Vec<NoteRow> = viewer.query_as(sql).bind(ids).fetch_all(&self.core.db).await?;
        Ok(rows.into_iter().map(|row| (row.id, row)).collect())
    }

    /// The note, if it exists and the viewer may see it.
    pub(crate) async fn find_note(&self, viewer: &Viewer, note_id: Uuid) -> AppResult<Option<NoteRow>> {
        Ok(self.visible_notes(viewer, &[note_id]).await?.remove(&note_id))
    }

    /// The note, or `NOTE_NOT_FOUND` — also when the viewer may not see it,
    /// so a hidden note cannot be told apart from a missing one.
    pub(crate) async fn require_note(&self, viewer: &Viewer, note_id: Uuid) -> AppResult<NoteRow> {
        self.find_note(viewer, note_id).await?.or_not_found("NOTE_NOT_FOUND", "note not found")
    }

    async fn context<'a>(&self, viewer: &'a Viewer, rows: &[&NoteRow], extra_users: &[Uuid]) -> AppResult<Context<'a>> {
        let core = &self.core;
        // Notes shown inside the rows (quotes) and notes the rows answer.
        let mut wanted: Vec<Uuid> = rows.iter().flat_map(|row| [row.quote_of_id, row.reply_to_id]).flatten().collect();
        wanted.sort();
        wanted.dedup();
        let mut referenced = self.visible_notes(viewer, &wanted).await?;
        // A quoted note may itself be a reply; whether what it answers is
        // still there is one more lookup (existence only).
        let mut second: Vec<Uuid> = rows
            .iter()
            .filter_map(|row| row.quote_of_id.and_then(|id| referenced.get(&id)))
            .filter_map(|quoted| quoted.reply_to_id)
            .filter(|id| !referenced.contains_key(id))
            .collect();
        second.sort();
        second.dedup();
        referenced.extend(self.visible_notes(viewer, &second).await?);

        let shown: Vec<&NoteRow> = rows
            .iter()
            .copied()
            .chain(rows.iter().filter_map(|row| row.quote_of_id.and_then(|id| referenced.get(&id))))
            .collect();
        let mut user_ids: Vec<Uuid> = shown
            .iter()
            .flat_map(|row| [Some(row.author_id), row.reply_to_author_id])
            .flatten()
            .chain(extra_users.iter().copied())
            .collect();
        user_ids.sort();
        user_ids.dedup();
        let mut stamp_ids: Vec<Uuid> = shown.iter().filter_map(|row| row.stamp_id).collect();
        stamp_ids.sort();
        stamp_ids.dedup();
        let mut note_ids: Vec<Uuid> = shown.iter().map(|row| row.id).collect();
        note_ids.sort();
        note_ids.dedup();

        let (users, stamps) =
            tokio::try_join!(tokuly_identity::summaries(core, &user_ids), tokuly_stamp::stamp_refs(core, &stamp_ids))?;
        let (liked, renoted) = match viewer.id {
            Some(user_id) => {
                let reactions: Vec<(Uuid, bool)> = sqlx::query_as(
                    "SELECT note_id, true FROM note_likes WHERE user_id = $1 AND note_id = ANY($2)
                     UNION ALL
                     SELECT note_id, false FROM note_renotes WHERE user_id = $1 AND note_id = ANY($2)",
                )
                .bind(user_id)
                .bind(&note_ids)
                .fetch_all(&core.db)
                .await?;
                let liked = reactions.iter().filter(|(_, like)| *like).map(|(id, _)| *id).collect();
                let renoted = reactions.iter().filter(|(_, like)| !*like).map(|(id, _)| *id).collect();
                (liked, renoted)
            }
            None => (HashSet::new(), HashSet::new()),
        };
        Ok(Context { viewer, users, stamps, referenced, liked, renoted })
    }

    /// Notes as `viewer` gets them, in the order of `rows`. The rows must
    /// already be ones the viewer may see.
    pub(crate) async fn hydrate(&self, viewer: &Viewer, rows: &[NoteRow]) -> AppResult<Vec<Note>> {
        let refs: Vec<&NoteRow> = rows.iter().collect();
        let context = self.context(viewer, &refs, &[]).await?;
        Ok(rows.iter().map(|row| context.note(row, true)).collect())
    }

    pub(crate) async fn hydrate_one(&self, viewer: &Viewer, row: &NoteRow) -> AppResult<Note> {
        let context = self.context(viewer, &[row], &[]).await?;
        Ok(context.note(row, true))
    }

    /// Timeline rows as `viewer` gets them, in the same order.
    pub(crate) async fn hydrate_entries(&self, viewer: &Viewer, rows: Vec<EntryRow>) -> AppResult<Vec<TimelineEntry>> {
        let refs: Vec<&NoteRow> = rows.iter().map(|row| &row.note).collect();
        let renoters: Vec<Uuid> = rows.iter().filter_map(|row| row.renoter_id).collect();
        let context = self.context(viewer, &refs, &renoters).await?;
        Ok(rows
            .iter()
            .map(|row| {
                let renote = row.renoter_id.map(|user_id| RenoteInfo {
                    user: pick(&context.users, user_id).into(),
                    renoted_at: opt_ts(row.renoted_at),
                    ..Default::default()
                });
                TimelineEntry {
                    note: context.note(&row.note, true).into(),
                    renote: renote.into(),
                    ..Default::default()
                }
            })
            .collect())
    }
}
