//! A user's subjects: one list, reused by terms, study sessions, exams and
//! assignments.
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use tokuly_core::prelude::*;
use tokuly_proto::{
    common::Timestamps,
    school::{CreateSubjectRequest, Subject, SubjectLabel, UpdateSubjectRequest},
};

use crate::{School, text};

pub(crate) const MAX_SUBJECTS: i64 = 200;
const MAX_SHORT_NAME: usize = 8;

/// Colours handed out to subjects created without one, in turn.
const PALETTE: [&str; 12] = [
    "#ef5350", "#ff8a65", "#ffb74d", "#ffd54f", "#aed581", "#4db6ac", "#4fc3f7", "#7986cb", "#9575cd", "#f06292",
    "#a1887f", "#90a4ae",
];

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct SubjectRow {
    pub id: Uuid,
    pub name: String,
    pub short_name: String,
    pub color: String,
    pub emoji: String,
    pub teacher: String,
    pub room: String,
    pub archived: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub(crate) const COLUMNS: &str = "id, name, short_name, color, emoji, teacher, room, archived, created_at, updated_at";

impl SubjectRow {
    pub fn to_proto(&self) -> Subject {
        Subject {
            id: self.id.to_string(),
            name: self.name.clone(),
            short_name: self.short_name.clone(),
            color: self.color.clone(),
            emoji: self.emoji.clone(),
            teacher: self.teacher.clone(),
            room: self.room.clone(),
            archived: self.archived,
            timestamps: Timestamps {
                created_at: ts(self.created_at),
                updated_at: ts(self.updated_at),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        }
    }

    /// What other people see of the subject.
    pub fn label(&self) -> SubjectLabel {
        SubjectLabel {
            name: self.name.clone(),
            emoji: self.emoji.clone(),
            color: self.color.clone(),
            ..Default::default()
        }
    }
}

fn not_found() -> AppError {
    AppError::not_found("SUBJECT_NOT_FOUND", "subject not found")
}

/// Every subject of a user, by ID.
pub(crate) async fn map_of(core: &Core, user_id: Uuid) -> AppResult<HashMap<Uuid, SubjectRow>> {
    let rows: Vec<SubjectRow> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM school_subjects WHERE user_id = $1")))
            .bind(user_id)
            .fetch_all(&core.db)
            .await?;
    Ok(rows.into_iter().map(|row| (row.id, row)).collect())
}

/// Subjects by ID, whoever owns them (for lists that mix several users).
pub(crate) async fn by_ids(core: &Core, ids: &[Uuid]) -> AppResult<HashMap<Uuid, SubjectRow>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<SubjectRow> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM school_subjects WHERE id = ANY($1)")))
            .bind(ids)
            .fetch_all(&core.db)
            .await?;
    Ok(rows.into_iter().map(|row| (row.id, row)).collect())
}

/// One of `user_id`'s subjects, or `SUBJECT_NOT_FOUND`.
pub(crate) async fn require(conn: &mut PgConnection, user_id: Uuid, subject_id: Uuid) -> AppResult<SubjectRow> {
    let row: Option<SubjectRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM school_subjects WHERE id = $1 AND user_id = $2"
    )))
    .bind(subject_id)
    .bind(user_id)
    .fetch_optional(conn)
    .await?;
    row.ok_or_else(not_found)
}

/// The subject a request names with a possibly empty ID.
pub(crate) async fn require_opt(core: &Core, user_id: Uuid, subject_id: &str) -> AppResult<Option<SubjectRow>> {
    match parse_opt_id(subject_id)? {
        None => Ok(None),
        Some(id) => Ok(Some(require(&mut *core.db.acquire().await?, user_id, id).await?)),
    }
}

/// Turns a unique violation on the name index into the documented reason.
fn name_taken(error: sqlx::Error) -> AppError {
    match &error {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            AppError::already_exists("SUBJECT_NAME_TAKEN", "you already have a subject with this name")
        }
        _ => error.into(),
    }
}

impl School {
    pub(crate) async fn list_subjects(&self, user_id: Uuid, include_archived: bool) -> AppResult<Vec<Subject>> {
        let rows: Vec<SubjectRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM school_subjects WHERE user_id = $1 AND ($2 OR NOT archived) ORDER BY lower(name), id"
        )))
        .bind(user_id)
        .bind(include_archived)
        .fetch_all(&self.core.db)
        .await?;
        Ok(rows.iter().map(SubjectRow::to_proto).collect())
    }

    pub(crate) async fn create_subject(&self, user_id: Uuid, request: &CreateSubjectRequest) -> AppResult<SubjectRow> {
        let name = text::name(&request.name, text::MAX_NAME, "SUBJECT_NAME_INVALID", "subject name")?;
        let short_name = text::line(&request.short_name, MAX_SHORT_NAME, "short name")?;
        let color = text::color(&request.color)?;
        let emoji = text::emoji(&request.emoji)?;
        let teacher = text::line(&request.teacher, text::MAX_NAME, "teacher")?;
        let room = text::line(&request.room, text::MAX_NAME, "room")?;

        let mut tx = self.core.db.begin().await?;
        crate::lock_user(&mut tx, "subjects", user_id).await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM school_subjects WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(
            count < MAX_SUBJECTS,
            precondition,
            "SUBJECT_LIMIT_REACHED",
            "an account has at most {MAX_SUBJECTS} subjects"
        );
        let color = if color.is_empty() { PALETTE[count as usize % PALETTE.len()].to_owned() } else { color };
        let row = insert(&mut tx, user_id, &name, &short_name, &color, &emoji, &teacher, &room).await?;
        tx.commit().await?;
        Ok(row)
    }

    pub(crate) async fn update_subject(&self, user_id: Uuid, request: &UpdateSubjectRequest) -> AppResult<SubjectRow> {
        let subject_id = parse_id(&request.subject_id)?;
        let mut tx = self.core.db.begin().await?;
        let mut subject = require(&mut tx, user_id, subject_id).await?;
        if let Some(name) = &request.name {
            subject.name = text::name(name, text::MAX_NAME, "SUBJECT_NAME_INVALID", "subject name")?;
        }
        if let Some(short_name) = &request.short_name {
            subject.short_name = text::line(short_name, MAX_SHORT_NAME, "short name")?;
        }
        if let Some(color) = &request.color {
            let color = text::color(color)?;
            ensure!(!color.is_empty(), invalid, "COLOR_INVALID", "a subject needs a colour");
            subject.color = color;
        }
        if let Some(emoji) = &request.emoji {
            subject.emoji = text::emoji(emoji)?;
        }
        if let Some(teacher) = &request.teacher {
            subject.teacher = text::line(teacher, text::MAX_NAME, "teacher")?;
        }
        if let Some(room) = &request.room {
            subject.room = text::line(room, text::MAX_NAME, "room")?;
        }
        if let Some(archived) = request.archived {
            subject.archived = archived;
        }
        let row: SubjectRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE school_subjects SET name = $2, short_name = $3, color = $4, emoji = $5, teacher = $6, room = $7, archived = $8
             WHERE id = $1 RETURNING {COLUMNS}"
        )))
        .bind(subject_id)
        .bind(&subject.name)
        .bind(&subject.short_name)
        .bind(&subject.color)
        .bind(&subject.emoji)
        .bind(&subject.teacher)
        .bind(&subject.room)
        .bind(subject.archived)
        .fetch_one(&mut *tx)
        .await
        .map_err(name_taken)?;
        tx.commit().await?;
        Ok(row)
    }

    /// Takes a subject off the list and out of every timetable. Returns
    /// whether it was archived instead of erased (history still uses it) and
    /// how many grid classes went with it.
    pub(crate) async fn delete_subject(&self, user_id: Uuid, subject_id: Uuid) -> AppResult<(bool, i32)> {
        let mut tx = self.core.db.begin().await?;
        require(&mut tx, user_id, subject_id).await?;
        let removed = sqlx::query("DELETE FROM school_entries WHERE subject_id = $1")
            .bind(subject_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        sqlx::query("DELETE FROM school_changes WHERE subject_id = $1").bind(subject_id).execute(&mut *tx).await?;
        let in_history: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM school_study_sessions WHERE subject_id = $1)
                 OR EXISTS(SELECT 1 FROM school_exam_subjects WHERE subject_id = $1)
                 OR EXISTS(SELECT 1 FROM school_assignments WHERE subject_id = $1)",
        )
        .bind(subject_id)
        .fetch_one(&mut *tx)
        .await?;
        if in_history {
            sqlx::query("UPDATE school_subjects SET archived = true WHERE id = $1")
                .bind(subject_id)
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query("DELETE FROM school_subjects WHERE id = $1").bind(subject_id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok((in_history, removed as i32))
    }
}

/// Inserts a subject (the caller checked the fields and the limit).
pub(crate) async fn insert(
    conn: &mut PgConnection,
    user_id: Uuid,
    name: &str,
    short_name: &str,
    color: &str,
    emoji: &str,
    teacher: &str,
    room: &str,
) -> AppResult<SubjectRow> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "INSERT INTO school_subjects (user_id, name, short_name, color, emoji, teacher, room)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING {COLUMNS}"
    )))
    .bind(user_id)
    .bind(name)
    .bind(short_name)
    .bind(color)
    .bind(emoji)
    .bind(teacher)
    .bind(room)
    .fetch_one(conn)
    .await
    .map_err(name_taken)
}
