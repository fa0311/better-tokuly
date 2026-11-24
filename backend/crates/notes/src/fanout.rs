//! Post notifications: followers who set their notification level for an
//! account to every post (tokuly.social.v1 follow prefs) hear of its new
//! notes — those they may see — through the `notes.post_notify` job.
use tokuly_core::{
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::common::{SubjectType, UserSummary};
use tokuly_social::graph;

use crate::{
    Notes, copy,
    model::{NOTE_COLUMNS, NoteRow, note_path},
};

/// Recipients told per transaction.
const FANOUT_BATCH: usize = 500;

impl Notes {
    /// Tells the followers who asked for it about a new note (the
    /// `notes.post_notify` job). Idempotent: a second run collapses into the
    /// same inbox entries. Returns how many were told.
    pub async fn notify_post(&self, note_id: Uuid) -> AppResult<usize> {
        let core = &self.core;
        let sql = format!("SELECT {NOTE_COLUMNS} FROM notes n WHERE n.id = $1");
        let Some(note): Option<NoteRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(note_id).fetch_optional(&core.db).await?
        else {
            return Ok(0);
        };
        let wanting = graph::post_notification_recipients(core, note.author_id).await?;
        let mut readers = self.still_reaching(&note, &wanting).await?;
        // People already told about the note (mentioned, answered) are not told twice.
        let told: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM note_recipients WHERE note_id = $1")
            .bind(note.id)
            .fetch_all(&core.db)
            .await?;
        readers.retain(|reader| !told.contains(reader));
        if readers.is_empty() {
            return Ok(0);
        }
        let author = tokuly_identity::summary(core, note.author_id).await?;
        for chunk in readers.chunks(FANOUT_BATCH) {
            let mut tx = core.db.begin().await?;
            sqlx::query("INSERT INTO note_recipients (note_id, user_id) SELECT $1, * FROM unnest($2::uuid[]) ON CONFLICT DO NOTHING")
                .bind(note.id)
                .bind(chunk)
                .execute(&mut *tx)
                .await?;
            for reader in chunk {
                outbound::notify(&mut *tx, &new_post(&author, *reader, &note)).await?;
            }
            tx.commit().await?;
        }
        Ok(readers.len())
    }
}

/// `notes.post`: someone the recipient follows posted a note.
fn new_post(author: &UserSummary, recipient: Uuid, note: &NoteRow) -> NotificationRequest {
    let name = copy::display_name(author);
    let mut request = NotificationRequest::new(
        recipient,
        "notes.post",
        Localized::new(format!("{name}さんがノートしたよ!"), format!("{name} posted a note")),
        Localized::same(note.title()),
        note_path(note.id),
    )
    .actor(note.author_id)
    .subject(SubjectType::SUBJECT_TYPE_NOTE, note.id)
    .collapse(format!("notes.post:{}", note.id))
    .data(serde_json::json!({ "note_id": note.id }));
    if let Some(avatar) = author.avatar.as_option().filter(|avatar| !avatar.url.is_empty()) {
        request = request.image(avatar.url.clone());
    }
    request
}
