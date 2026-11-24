//! Drafts (下書き) kept on the server, and notes scheduled for later.
//!
//! A draft is the content of a note not posted yet (`DraftContent`, the
//! fields of `CreateNoteRequest`), with the uploads it names held as
//! `notes:draft:<id>` so they are not cleaned up meanwhile. Nothing about
//! the content is checked until it is posted, except its size and that the
//! uploads are the author's: a draft is allowed to be unfinished.
//!
//! Scheduled drafts are posted by the scheduler ([`Notes::publish_due_drafts`])
//! as if their author had pressed the button then. The note gets an ID of
//! its own (it sorts and trends as posted now) and records the draft in the
//! unique `notes.draft_id`: a draft is posted at most once, and whoever
//! comes second (the scheduler, `PublishDraft`, a retry) finds that note.
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use tokuly_core::{
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_media::assets;
use tokuly_proto::{
    common::PageInfo,
    media::{AssetPurpose, Attachment},
    notes::{Draft, DraftContent},
};

use crate::{
    Notes, content,
    model::{NOTE_COLUMNS, NoteRow, draft_holder},
    posting::{MAX_ATTACHMENTS, NewNote, requested_audience},
};

/// Drafts one person may keep, and how many of them may be scheduled.
pub const MAX_DRAFTS: i64 = 100;
pub const MAX_SCHEDULED: i64 = 30;
/// How far ahead a note can be scheduled.
const MIN_SCHEDULE_AHEAD: chrono::Duration = chrono::Duration::minutes(5);
const MAX_SCHEDULE_AHEAD: chrono::Duration = chrono::Duration::days(90);
/// Draft saves one user may make per minute (the composer saves as people type).
const SAVES_PER_MINUTE: u32 = 60;
/// Drafts posted per pass of the scheduler.
const PUBLISH_BATCH: i64 = 50;
/// The failure a draft carries while the scheduler is posting it: what it
/// keeps if the pass stops before it finished.
const INTERRUPTED: &str = "INTERRUPTED";

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct DraftRow {
    pub id: Uuid,
    pub author_id: Uuid,
    pub content: serde_json::Value,
    pub asset_ids: Vec<Uuid>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub failure_reason: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const DRAFT_COLUMNS: &str = "id, author_id, content, asset_ids, scheduled_at, failure_reason, created_at, updated_at";

impl DraftRow {
    pub fn content(&self) -> DraftContent {
        serde_json::from_value(self.content.clone()).unwrap_or_default()
    }

    pub fn to_proto(&self, attachments: Vec<Attachment>) -> Draft {
        Draft {
            id: self.id.to_string(),
            content: self.content().into(),
            attachments,
            scheduled_at: opt_ts(self.scheduled_at),
            failure_reason: self.failure_reason.clone(),
            created_at: ts(self.created_at),
            updated_at: ts(self.updated_at),
            ..Default::default()
        }
    }
}

/// The uploads a draft names, as IDs (unparseable ones are refused).
fn asset_ids_of(content: &DraftContent) -> AppResult<Vec<Uuid>> {
    let mut ids = Vec::new();
    for id in &content.asset_ids {
        let id = Uuid::parse_str(id).map_err(|_| AppError::not_found("ASSET_NOT_FOUND", "upload not found"))?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ensure!(
        ids.len() <= MAX_ATTACHMENTS,
        invalid,
        "TOO_MANY_ATTACHMENTS",
        "a note carries at most {MAX_ATTACHMENTS} images"
    );
    Ok(ids)
}

/// Takes the per-user lock under which drafts are counted.
async fn lock_drafts(tx: &mut Transaction<'_, Postgres>, author: Uuid) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("notes:drafts:{author}"))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Stores a new draft inside `tx` (the draft limit applies) and holds its uploads.
pub(crate) async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    author: Uuid,
    content: &DraftContent,
    asset_ids: &[Uuid],
) -> AppResult<DraftRow> {
    lock_drafts(tx, author).await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM note_drafts WHERE author_id = $1")
        .bind(author)
        .fetch_one(&mut **tx)
        .await?;
    ensure!(count < MAX_DRAFTS, precondition, "TOO_MANY_DRAFTS", "you can keep at most {MAX_DRAFTS} drafts");
    let sql = format!(
        "INSERT INTO note_drafts (author_id, content, asset_ids) VALUES ($1, $2, $3) RETURNING {DRAFT_COLUMNS}"
    );
    let row: DraftRow = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(author)
        .bind(crate::model::to_json(content)?)
        .bind(asset_ids)
        .fetch_one(&mut **tx)
        .await?;
    for asset_id in asset_ids {
        assets::attach(&mut **tx, *asset_id, &draft_holder(row.id)).await?;
    }
    Ok(row)
}

impl Notes {
    /// The uploads of drafts, for previews (missing ones are left out).
    pub(crate) async fn drafts_to_proto(&self, rows: &[DraftRow]) -> AppResult<Vec<Draft>> {
        let mut ids: Vec<Uuid> = rows.iter().flat_map(|row| row.asset_ids.iter().copied()).collect();
        ids.sort();
        ids.dedup();
        let found = assets::find_many(&self.core, &ids).await?;
        Ok(rows
            .iter()
            .map(|row| {
                let attachments = row
                    .asset_ids
                    .iter()
                    .filter_map(|id| found.iter().find(|asset| asset.id == *id).map(|asset| asset.to_attachment()))
                    .collect();
                row.to_proto(attachments)
            })
            .collect())
    }

    /// Creates (`draft_id` = `None`) or replaces one of `author`'s drafts.
    pub(crate) async fn save_draft(
        &self,
        author: Uuid,
        draft_id: Option<Uuid>,
        content: DraftContent,
        scheduled_at: Option<DateTime<Utc>>,
    ) -> AppResult<DraftRow> {
        let core = &self.core;
        core.cache.rate_limit(&format!("notes:draft:{author}"), SAVES_PER_MINUTE, Duration::from_secs(60)).await?;
        ensure!(
            crate::text::fits(&crate::text::clean(&content.text), content::MAX_NOTE_CHARS),
            invalid,
            "NOTE_TOO_LONG",
            "a note has at most {} characters",
            content::MAX_NOTE_CHARS
        );
        if let Some(at) = scheduled_at {
            let now = Utc::now();
            ensure!(
                at >= now + MIN_SCHEDULE_AHEAD && at <= now + MAX_SCHEDULE_AHEAD,
                invalid,
                "INVALID_SCHEDULE",
                "a note can be scheduled 5 minutes to 90 days ahead"
            );
        }
        let asset_ids = asset_ids_of(&content)?;
        let purposes = [AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE, AssetPurpose::ASSET_PURPOSE_NOTE_VIDEO];
        for asset_id in &asset_ids {
            assets::require(core, *asset_id, author, &purposes, false).await?;
        }
        let mut tx = core.db.begin().await?;
        let row = match draft_id {
            None => insert(&mut tx, author, &content, &asset_ids).await?,
            Some(draft_id) => {
                lock_drafts(&mut tx, author).await?;
                let sql =
                    format!("SELECT {DRAFT_COLUMNS} FROM note_drafts WHERE id = $1 AND author_id = $2 FOR UPDATE");
                let before: DraftRow = sqlx::query_as(sqlx::AssertSqlSafe(sql))
                    .bind(draft_id)
                    .bind(author)
                    .fetch_optional(&mut *tx)
                    .await?
                    .or_not_found("DRAFT_NOT_FOUND", "draft not found")?;
                for gone in before.asset_ids.iter().filter(|id| !asset_ids.contains(id)) {
                    assets::detach(&mut *tx, *gone, &draft_holder(draft_id)).await?;
                }
                for added in asset_ids.iter().filter(|id| !before.asset_ids.contains(id)) {
                    assets::attach(&mut *tx, *added, &draft_holder(draft_id)).await?;
                }
                let sql = format!(
                    "UPDATE note_drafts SET content = $2, asset_ids = $3, updated_at = now(), failure_reason = '', scheduled_at = NULL
                     WHERE id = $1 RETURNING {DRAFT_COLUMNS}"
                );
                sqlx::query_as(sqlx::AssertSqlSafe(sql))
                    .bind(draft_id)
                    .bind(crate::model::to_json(&content)?)
                    .bind(&asset_ids)
                    .fetch_one(&mut *tx)
                    .await?
            }
        };
        let row = match scheduled_at {
            None => row,
            Some(at) => {
                let scheduled: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM note_drafts WHERE author_id = $1 AND scheduled_at IS NOT NULL AND id <> $2",
                )
                .bind(author)
                .bind(row.id)
                .fetch_one(&mut *tx)
                .await?;
                ensure!(
                    scheduled < MAX_SCHEDULED,
                    precondition,
                    "TOO_MANY_SCHEDULED",
                    "at most {MAX_SCHEDULED} notes can be scheduled"
                );
                let sql = format!("UPDATE note_drafts SET scheduled_at = $2 WHERE id = $1 RETURNING {DRAFT_COLUMNS}");
                sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(row.id).bind(at).fetch_one(&mut *tx).await?
            }
        };
        tx.commit().await?;
        Ok(row)
    }

    pub(crate) async fn draft(&self, author: Uuid, draft_id: Uuid) -> AppResult<DraftRow> {
        let sql = format!("SELECT {DRAFT_COLUMNS} FROM note_drafts WHERE id = $1 AND author_id = $2");
        sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(draft_id)
            .bind(author)
            .fetch_optional(&self.core.db)
            .await?
            .or_not_found("DRAFT_NOT_FOUND", "draft not found")
    }

    /// `author`'s drafts, most recently changed first, or the scheduled ones, soonest first.
    pub(crate) async fn list_drafts(
        &self,
        author: Uuid,
        scheduled_only: bool,
        page: &Page,
    ) -> AppResult<(Vec<DraftRow>, PageInfo)> {
        let mut rows: Vec<DraftRow> = if scheduled_only {
            let cursor: Option<(DateTime<Utc>, Uuid)> = page.decode()?;
            let (at, id) = cursor.unzip();
            let sql = format!(
                "SELECT {DRAFT_COLUMNS} FROM note_drafts WHERE author_id = $1 AND scheduled_at IS NOT NULL
                   AND ($2::timestamptz IS NULL OR (scheduled_at, id) > ($2, $3))
                 ORDER BY scheduled_at, id LIMIT $4"
            );
            sqlx::query_as(sqlx::AssertSqlSafe(sql))
                .bind(author)
                .bind(at)
                .bind(id)
                .bind(page.fetch())
                .fetch_all(&self.core.db)
                .await?
        } else {
            let cursor: Option<(DateTime<Utc>, Uuid)> = page.decode()?;
            let (at, id) = cursor.unzip();
            let sql = format!(
                "SELECT {DRAFT_COLUMNS} FROM note_drafts WHERE author_id = $1
                   AND ($2::timestamptz IS NULL OR (updated_at, id) < ($2, $3))
                 ORDER BY updated_at DESC, id DESC LIMIT $4"
            );
            sqlx::query_as(sqlx::AssertSqlSafe(sql))
                .bind(author)
                .bind(at)
                .bind(id)
                .bind(page.fetch())
                .fetch_all(&self.core.db)
                .await?
        };
        let info = if scheduled_only {
            page.finish(&mut rows, |row| (row.scheduled_at.unwrap_or(row.updated_at), row.id))
        } else {
            page.finish(&mut rows, |row| (row.updated_at, row.id))
        };
        Ok((rows, info))
    }

    pub(crate) async fn delete_draft(&self, author: Uuid, draft_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let deleted = sqlx::query("DELETE FROM note_drafts WHERE id = $1 AND author_id = $2")
            .bind(draft_id)
            .bind(author)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        ensure!(deleted == 1, not_found, "DRAFT_NOT_FOUND", "draft not found");
        assets::detach_holder(&mut *tx, &draft_holder(draft_id)).await?;
        tx.commit().await?;
        Ok(())
    }

    /// What a draft posts.
    fn draft_note(content: &DraftContent) -> AppResult<NewNote> {
        Ok(NewNote {
            text: content.text.clone(),
            asset_ids: parse_ids(&content.asset_ids)?,
            stamp_id: parse_opt_id(&content.stamp_id)?,
            place: content.place.as_option().cloned(),
            audience: requested_audience(content.visibility, &content.circle_id, &content.direct_user_ids)?,
            reply_to_id: parse_opt_id(&content.reply_to_id)?,
            quote_of_id: parse_opt_id(&content.quote_of_id)?,
            content_warning: content.content_warning,
            content_warning_text: content.content_warning_text.clone(),
            attachment_details: content.attachment_details.clone(),
            poll: content.poll.as_option().cloned(),
            channel_id: parse_opt_id(&content.channel_id)?,
            draft_id: None,
        })
    }

    /// Posts one of `author`'s drafts now and deletes it. A draft is posted
    /// once (`notes.draft_id` is unique): whoever comes second — the
    /// scheduler, a retry, a second device — gets the note posted first.
    pub(crate) async fn publish_draft(&self, author: Uuid, draft_id: Uuid) -> AppResult<NoteRow> {
        let row = match self.draft(author, draft_id).await {
            Ok(row) => row,
            // Posted (and deleted) a moment ago by someone else.
            Err(error) if error.is_not_found() => {
                return self.note_of_draft(draft_id).await?.filter(|note| note.author_id == author).ok_or(error);
            }
            Err(error) => return Err(error),
        };
        let note = self.post_draft(&row).await?;
        self.forget_draft(row.id).await?;
        Ok(note)
    }

    /// Posts a draft as its author would, once: the note gets an ID of its
    /// own (so it sorts and trends as posted now) and remembers the draft.
    async fn post_draft(&self, row: &DraftRow) -> AppResult<NoteRow> {
        if let Some(note) = self.note_of_draft(row.id).await? {
            return Ok(note);
        }
        let mut input = Self::draft_note(&row.content())?;
        input.draft_id = Some(row.id);
        match self.create_note(row.author_id, input).await {
            Ok(note) => Ok(note),
            // Lost a race with another poster of the same draft, or was
            // refused (a rate limit) after it was posted: that note counts.
            Err(error) => match self.note_of_draft(row.id).await? {
                Some(note) => Ok(note),
                None => Err(error),
            },
        }
    }

    async fn forget_draft(&self, draft_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        sqlx::query("DELETE FROM note_drafts WHERE id = $1").bind(draft_id).execute(&mut *tx).await?;
        assets::detach_holder(&mut *tx, &draft_holder(draft_id)).await?;
        tx.commit().await?;
        Ok(())
    }

    /// The note posted from a draft, if any.
    async fn note_of_draft(&self, draft_id: Uuid) -> AppResult<Option<NoteRow>> {
        let sql = format!("SELECT {NOTE_COLUMNS} FROM notes n WHERE n.draft_id = $1");
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(draft_id).fetch_optional(&self.core.db).await?)
    }

    /// Posts the drafts whose time has come (the scheduler; public for tools
    /// and tests). A draft that cannot be posted keeps the reason and is no
    /// longer scheduled; its author is told. Returns how many were posted.
    pub async fn publish_due_drafts(&self) -> AppResult<usize> {
        let core = &self.core;
        // Claimed one batch at a time: no other pass takes them meanwhile.
        let sql = format!(
            "UPDATE note_drafts SET scheduled_at = NULL, failure_reason = '{INTERRUPTED}'
             WHERE id IN (SELECT id FROM note_drafts WHERE scheduled_at <= now() ORDER BY scheduled_at LIMIT $1 FOR UPDATE SKIP LOCKED)
             RETURNING {DRAFT_COLUMNS}"
        );
        let due: Vec<DraftRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(PUBLISH_BATCH).fetch_all(&core.db).await?;
        let mut posted = 0;
        for row in due {
            match self.post_draft(&row).await {
                Ok(_) => {
                    self.forget_draft(row.id).await?;
                    posted += 1;
                }
                Err(error) => {
                    let reason = error.reason.to_string();
                    let mut tx = core.db.begin().await?;
                    sqlx::query("UPDATE note_drafts SET failure_reason = $2 WHERE id = $1")
                        .bind(row.id)
                        .bind(&reason)
                        .execute(&mut *tx)
                        .await?;
                    outbound::notify(&mut *tx, &schedule_failed(row.author_id, row.id, &reason)).await?;
                    tx.commit().await?;
                }
            }
        }
        Ok(posted)
    }
}

/// `notes.schedule_failed`: a scheduled note could not be posted.
fn schedule_failed(author: Uuid, draft_id: Uuid, reason: &str) -> NotificationRequest {
    NotificationRequest::new(
        author,
        "notes.schedule_failed",
        Localized::new("予約したノートを投稿できなかったよ", "A scheduled note couldn't be posted"),
        Localized::new("下書きに残してあるから、見直してね", "It's kept in your drafts: have a look"),
        "/notes/drafts",
    )
    .collapse(format!("notes.schedule_failed:{draft_id}"))
    .data(serde_json::json!({ "draft_id": draft_id, "reason": reason }))
}
