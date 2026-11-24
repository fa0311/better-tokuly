//! Changing a note after it was posted: editing its words within an hour
//! (the earlier versions stay readable by everyone who may see the note),
//! and "delete and redraft" (削除して再編集), which turns a note back into a
//! draft with its uploads.
use std::{collections::HashMap, time::Duration};

use chrono::{DateTime, Utc};
use tokuly_core::{events, outbound, prelude::*};
use tokuly_proto::notes::{
    AttachmentDetailsInput, DraftContent, NoteEdit, NoteEdited, PollInput, RichText, notes_event::Event as Push,
};

use crate::{
    Notes, authors, content, copy, drafts,
    events::RemovalReason,
    hashtags,
    model::{NOTE_COLUMNS, NoteRow, note_topic, to_json},
    posting::{JOB_LINK_PREVIEW, LinkPreviewJob, search_doc},
    visibility::Viewer,
};

/// How long after posting a note can be edited.
pub const EDIT_WINDOW_MINUTES: i64 = 60;
/// How often one note can be edited.
pub const MAX_EDITS: i32 = 10;
/// Edits one user may make per minute.
const EDITS_PER_MINUTE: u32 = 20;

/// Whether the note's author may still edit it.
pub(crate) fn may_still_edit(note: &NoteRow) -> bool {
    note.edit_count < MAX_EDITS && Utc::now() < note.created_at + chrono::Duration::minutes(EDIT_WINDOW_MINUTES)
}

/// What an edit changes.
#[derive(Debug, Clone, Default)]
pub(crate) struct Edit {
    pub text: String,
    pub content_warning: bool,
    pub content_warning_text: String,
    pub attachment_details: Vec<AttachmentDetailsInput>,
}

#[derive(sqlx::FromRow)]
struct EditRow {
    version: i32,
    text: String,
    body: serde_json::Value,
    content_warning: bool,
    content_warning_text: String,
    replaced_at: DateTime<Utc>,
}

impl Notes {
    /// Edits one of `user_id`'s notes and returns it as it is now.
    pub(crate) async fn edit_note(&self, user_id: Uuid, note_id: Uuid, edit: Edit) -> AppResult<NoteRow> {
        let core = &self.core;
        core.cache.rate_limit(&format!("notes:edit:{user_id}"), EDITS_PER_MINUTE, Duration::from_secs(60)).await?;
        let note = self.own_note(user_id, note_id).await?;
        ensure!(
            Utc::now() < note.created_at + chrono::Duration::minutes(EDIT_WINDOW_MINUTES),
            precondition,
            "EDIT_WINDOW_CLOSED",
            "a note can be edited for {EDIT_WINDOW_MINUTES} minutes after posting"
        );
        let said = content::prepare(&edit.text, edit.content_warning, &edit.content_warning_text)?;
        let details = content::attachment_details(&note.attachments(), &edit.attachment_details)?;
        let has_poll: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM note_polls WHERE note_id = $1)")
            .bind(note.id)
            .fetch_one(&core.db)
            .await?;
        ensure!(
            !said.text.is_empty() || !note.asset_ids.is_empty() || note.stamp_id.is_some() || has_poll,
            invalid,
            "NOTE_EMPTY",
            "a note needs text, media, a stamp or a poll"
        );
        // Everything from elsewhere before the transaction: who is
        // mentioned now, and as whom they would see the note.
        let author = Viewer::load(core, Some(user_id)).await?;
        let mentioned = self.resolve_mentions(&author, &said.facts.handles).await?;
        let before: Vec<Uuid> = note.mentions().iter().filter_map(|m| Uuid::parse_str(&m.user_id).ok()).collect();
        let mut newly: HashMap<Uuid, Viewer> = HashMap::new();
        for (person, _) in mentioned.iter().filter(|(person, _)| !before.contains(person) && *person != user_id) {
            if !newly.contains_key(person) {
                newly.insert(*person, Viewer::load(core, Some(*person)).await?);
            }
        }
        let muted = self.thread_muters(&note, &newly.keys().copied().collect::<Vec<_>>()).await?;
        let actor = tokuly_identity::summary(core, user_id).await?;
        let link_url = said
            .facts
            .first_url
            .as_deref()
            .and_then(|url| tokuly_media::link_preview::normalize_url(url).ok())
            .map(String::from);

        let mut tx = core.db.begin().await?;
        authors::lock_author(&mut tx, user_id, false).await?;
        let sql = format!("SELECT {NOTE_COLUMNS} FROM notes n WHERE n.id = $1 FOR UPDATE");
        let current: NoteRow = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(note.id)
            .fetch_optional(&mut *tx)
            .await?
            .or_not_found("NOTE_NOT_FOUND", "note not found")?;
        ensure!(
            current.edit_count < MAX_EDITS,
            precondition,
            "TOO_MANY_EDITS",
            "a note can be edited {MAX_EDITS} times"
        );
        sqlx::query(
            "INSERT INTO note_edits (note_id, version, text, body, content_warning, content_warning_text)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(current.id)
        .bind(current.edit_count + 1)
        .bind(&current.text)
        .bind(&current.body)
        .bind(current.content_warning)
        .bind(&current.content_warning_text)
        .execute(&mut *tx)
        .await?;
        let mentions: Vec<tokuly_proto::notes::Mention> = mentioned
            .iter()
            .map(|(person, handle)| tokuly_proto::notes::Mention {
                handle: handle.clone(),
                user_id: person.to_string(),
                ..Default::default()
            })
            .collect();
        let link_changed = link_url != current.link_url;
        let sql = format!(
            "UPDATE notes AS n SET text = $2, body = $3, content_warning = $4, content_warning_text = $5,
                    attachment_details = $6, hashtags = $7, mentions = $8, language = $9,
                    link_url = $10, link_preview = CASE WHEN $11 THEN NULL ELSE n.link_preview END,
                    edited_at = now(), edit_count = n.edit_count + 1
             WHERE n.id = $1 RETURNING {NOTE_COLUMNS}"
        );
        let updated: NoteRow = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(current.id)
            .bind(&said.text)
            .bind(to_json(&said.body)?)
            .bind(said.content_warning)
            .bind(&said.content_warning_text)
            .bind(to_json(&details)?)
            .bind(&said.facts.hashtags)
            .bind(to_json(&mentions)?)
            .bind(said.language)
            .bind(&link_url)
            .bind(link_changed)
            .fetch_one(&mut *tx)
            .await?;
        // Hashtags: links and listed counts follow the new text.
        let removed: Vec<String> =
            current.hashtags.iter().filter(|tag| !updated.hashtags.contains(tag)).cloned().collect();
        let added: Vec<String> =
            updated.hashtags.iter().filter(|tag| !current.hashtags.contains(tag)).cloned().collect();
        hashtags::unlink(&mut tx, current.id, &removed, current.listed).await?;
        hashtags::link(&mut tx, current.id, &added, current.listed).await?;
        sqlx::query("DELETE FROM note_mentions WHERE note_id = $1").bind(current.id).execute(&mut *tx).await?;
        if !mentioned.is_empty() {
            let (people, handles): (Vec<Uuid>, Vec<String>) = mentioned.iter().cloned().unzip();
            sqlx::query("INSERT INTO note_mentions (note_id, user_id, handle) SELECT $1, * FROM unnest($2::uuid[], $3::text[]) ON CONFLICT DO NOTHING")
                .bind(current.id)
                .bind(&people)
                .bind(&handles)
                .execute(&mut *tx)
                .await?;
        }
        if updated.is_listed() {
            outbound::index(&mut *tx, search_doc(&updated, &actor)).await?;
        }
        // People mentioned for the first time are told, like on a new note.
        let mut told = Vec::new();
        for (person, viewer) in &newly {
            if !muted.contains(person) && viewer.can_see_note(&mut *tx, updated.id).await? {
                told.push(*person);
            }
        }
        if !told.is_empty() {
            sqlx::query("INSERT INTO note_recipients (note_id, user_id) SELECT $1, * FROM unnest($2::uuid[]) ON CONFLICT DO NOTHING")
                .bind(updated.id)
                .bind(&told)
                .execute(&mut *tx)
                .await?;
        }
        for person in &told {
            outbound::notify(&mut *tx, &copy::mention(&actor, *person, &updated)).await?;
        }
        events::emit(
            &mut *tx,
            crate::events::NOTE_EDITED,
            Some(user_id),
            &crate::events::NoteEditedEvent { note_id: updated.id, author_id: user_id, version: updated.edit_count },
        )
        .await?;
        tx.commit().await?;

        let edited =
            NoteEdited { note_id: updated.id.to_string(), edited_at: opt_ts(updated.edited_at), ..Default::default() };
        self.push_topic(&note_topic(updated.id), Push::NoteEdited(Box::new(edited))).await;
        if link_changed
            && updated.link_url.is_some()
            && let Err(e) = core
                .jobs
                .enqueue_unique(
                    JOB_LINK_PREVIEW,
                    &format!("{}:{}", updated.id, updated.edit_count),
                    &LinkPreviewJob { note_id: updated.id },
                )
                .await
        {
            tracing::warn!(error = %e, "queueing the link preview of an edited note failed");
        }
        Ok(updated)
    }

    /// The earlier versions of a note the viewer may see, newest first.
    pub(crate) async fn note_edits(&self, viewer: &Viewer, note_id: Uuid) -> AppResult<Vec<NoteEdit>> {
        let note = self.require_note(viewer, note_id).await?;
        let rows: Vec<EditRow> = sqlx::query_as(
            "SELECT version, text, body, content_warning, content_warning_text, replaced_at
             FROM note_edits WHERE note_id = $1 ORDER BY version DESC",
        )
        .bind(note.id)
        .fetch_all(&self.core.db)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| NoteEdit {
                version: row.version,
                text: row.text,
                body: serde_json::from_value::<RichText>(row.body).unwrap_or_default().into(),
                content_warning: row.content_warning,
                content_warning_text: row.content_warning_text,
                replaced_at: ts(row.replaced_at),
                ..Default::default()
            })
            .collect())
    }

    /// Deletes one of `user_id`'s notes and returns a draft with its content.
    pub(crate) async fn redraft(&self, user_id: Uuid, note_id: Uuid) -> AppResult<drafts::DraftRow> {
        let note = self.own_note(user_id, note_id).await?;
        let poll: Option<(Vec<String>, bool, i32, i64)> = sqlx::query_as(
            "SELECT p.choices, p.multiple, p.results_visibility,
                    GREATEST(extract(epoch FROM p.ends_at - n.created_at)::bigint, $2)
             FROM note_polls p JOIN notes n ON n.id = p.note_id WHERE p.note_id = $1",
        )
        .bind(note.id)
        .bind(content::MIN_POLL_SECONDS)
        .fetch_optional(&self.core.db)
        .await?;
        let draft = DraftContent {
            text: note.text.clone(),
            asset_ids: note.asset_ids.iter().map(Uuid::to_string).collect(),
            stamp_id: note.stamp_id.map(|id| id.to_string()).unwrap_or_default(),
            place: note.place().into(),
            visibility: note.visibility.into(),
            reply_to_id: note.reply_to_id.map(|id| id.to_string()).unwrap_or_default(),
            quote_of_id: note.quote_of_id.map(|id| id.to_string()).unwrap_or_default(),
            content_warning: note.content_warning,
            circle_id: note.circle_id.map(|id| id.to_string()).unwrap_or_default(),
            direct_user_ids: note.direct_user_ids.iter().map(Uuid::to_string).collect(),
            content_warning_text: note.content_warning_text.clone(),
            attachment_details: note
                .attachment_details()
                .into_iter()
                .map(|details| AttachmentDetailsInput {
                    asset_id: details.asset_id,
                    alt_text: details.alt_text,
                    sensitive: details.sensitive,
                    ..Default::default()
                })
                .collect(),
            poll: poll
                .map(|(choices, multiple, results, seconds)| PollInput {
                    choices,
                    multiple,
                    duration_seconds: seconds.min(content::MAX_POLL_SECONDS),
                    results_visibility: results.into(),
                    ..Default::default()
                })
                .into(),
            channel_id: note.shown_channel().map(|id| id.to_string()).unwrap_or_default(),
            ..Default::default()
        };
        let mut tx = self.core.db.begin().await?;
        // The draft holds the uploads before the note lets go of them, so
        // they are never without a holder.
        let row = drafts::insert(&mut tx, user_id, &draft, &note.asset_ids).await?;
        let removed = self.delete_notes(&mut tx, &[note.id], RemovalReason::Author, Some(user_id)).await?;
        tx.commit().await?;
        self.announce_removed(removed).await;
        Ok(row)
    }
}
