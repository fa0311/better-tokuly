//! What fills a term: the weekly grid, one-off changes and day overrides.
use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use sqlx::PgConnection;
use tokuly_core::prelude::*;
use tokuly_proto::school::{
    ChangeKind, CreateDayOverrideRequest, CreateTimetableChangeRequest, DayOverride, DayOverrideKind, TimetableChange,
    TimetableEntry, TimetableEntryInput, WeekPattern, Weekday,
};

use super::{
    resolve::{self, ChangeData, EVERY, EntryData, OverrideData, TermData, WEEK_A, WEEK_B},
    terms::{self, TermRow},
};
use crate::{
    School, local,
    subjects::{self, SubjectRow},
    text,
};

const MAX_ENTRIES_PER_CALL: usize = 100;
const MAX_CHANGES: i64 = 500;
const MAX_OVERRIDES: i64 = 200;

#[derive(sqlx::FromRow)]
struct EntryRecord {
    id: Uuid,
    term_id: Uuid,
    weekday: i16,
    period: i16,
    week_pattern: i16,
    subject_id: Uuid,
    teacher: String,
    room: String,
    color: String,
    memo: String,
}

#[derive(sqlx::FromRow)]
struct ChangeRecord {
    id: Uuid,
    term_id: Uuid,
    date: NaiveDate,
    period: i16,
    kind: i16,
    subject_id: Option<Uuid>,
    teacher: String,
    room: String,
    memo: String,
    to_date: Option<NaiveDate>,
    to_period: Option<i16>,
}

#[derive(sqlx::FromRow)]
struct OverrideRecord {
    id: Uuid,
    term_id: Uuid,
    start_date: NaiveDate,
    end_date: NaiveDate,
    kind: i16,
    name: String,
    follows_weekday: Option<i16>,
    week_pattern: Option<i16>,
}

const ENTRY_COLUMNS: &str = "id, term_id, weekday, period, week_pattern, subject_id, teacher, room, color, memo";
const CHANGE_COLUMNS: &str = "id, term_id, date, period, kind, subject_id, teacher, room, memo, to_date, to_period";
const OVERRIDE_COLUMNS: &str = "id, term_id, start_date, end_date, kind, name, follows_weekday, week_pattern";

/// Loads the contents of several terms in three queries.
pub(crate) async fn load(conn: &mut PgConnection, terms: &[TermRow]) -> AppResult<Vec<TermData>> {
    let ids: Vec<Uuid> = terms.iter().map(|term| term.id).collect();
    let mut data: HashMap<Uuid, TermData> = terms
        .iter()
        .map(|term| {
            (term.id, TermData { info: term.info(), entries: Vec::new(), changes: Vec::new(), overrides: Vec::new() })
        })
        .collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let entries: Vec<EntryRecord> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ENTRY_COLUMNS} FROM school_entries WHERE term_id = ANY($1) ORDER BY weekday, period, week_pattern"
    )))
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    for e in entries {
        if let Some(term) = data.get_mut(&e.term_id) {
            term.entries.push(EntryData {
                id: e.id,
                weekday: e.weekday,
                period: e.period,
                week_pattern: e.week_pattern,
                subject_id: e.subject_id,
                teacher: e.teacher,
                room: e.room,
                color: e.color,
                memo: e.memo,
            });
        }
    }
    let changes: Vec<ChangeRecord> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {CHANGE_COLUMNS} FROM school_changes WHERE term_id = ANY($1) ORDER BY date, period"
    )))
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    for c in changes {
        if let Some(term) = data.get_mut(&c.term_id) {
            term.changes.push(ChangeData {
                id: c.id,
                date: c.date,
                period: c.period,
                kind: c.kind,
                subject_id: c.subject_id,
                teacher: c.teacher,
                room: c.room,
                memo: c.memo,
                to_date: c.to_date,
                to_period: c.to_period,
            });
        }
    }
    let overrides: Vec<OverrideRecord> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {OVERRIDE_COLUMNS} FROM school_day_overrides WHERE term_id = ANY($1) ORDER BY start_date, id"
    )))
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    for o in overrides {
        if let Some(term) = data.get_mut(&o.term_id) {
            term.overrides.push(OverrideData {
                id: o.id,
                start_date: o.start_date,
                end_date: o.end_date,
                kind: o.kind,
                name: o.name,
                follows_weekday: o.follows_weekday,
                week_pattern: o.week_pattern,
            });
        }
    }
    // In the order the terms were given.
    Ok(terms.iter().filter_map(|term| data.remove(&term.id)).collect())
}

/// Loads one term's contents.
pub(crate) async fn load_one(conn: &mut PgConnection, term: &TermRow) -> AppResult<TermData> {
    Ok(load(conn, std::slice::from_ref(term)).await?.pop().expect("one term in, one term out"))
}

pub(crate) fn entry_proto(
    term_id: Uuid,
    entry: &EntryData,
    subjects: &HashMap<Uuid, SubjectRow>,
    with_memo: bool,
) -> TimetableEntry {
    TimetableEntry {
        id: entry.id.to_string(),
        term_id: term_id.to_string(),
        weekday: (entry.weekday as i32).into(),
        period: entry.period as i32,
        week_pattern: (entry.week_pattern as i32).into(),
        subject: subjects.get(&entry.subject_id).map(SubjectRow::to_proto).into(),
        teacher: entry.teacher.clone(),
        room: entry.room.clone(),
        color: entry.color.clone(),
        memo: if with_memo { entry.memo.clone() } else { String::new() },
        ..Default::default()
    }
}

pub(crate) fn change_proto(
    term_id: Uuid,
    change: &ChangeData,
    subjects: &HashMap<Uuid, SubjectRow>,
) -> TimetableChange {
    TimetableChange {
        id: change.id.to_string(),
        term_id: term_id.to_string(),
        date: local::fmt_date(change.date),
        period: change.period as i32,
        kind: (change.kind as i32).into(),
        subject: change.subject_id.and_then(|id| subjects.get(&id)).map(SubjectRow::to_proto).into(),
        teacher: change.teacher.clone(),
        room: change.room.clone(),
        memo: change.memo.clone(),
        to_date: local::fmt_opt_date(change.to_date),
        to_period: change.to_period.unwrap_or(0) as i32,
        ..Default::default()
    }
}

pub(crate) fn override_proto(term_id: Uuid, day_override: &OverrideData) -> DayOverride {
    DayOverride {
        id: day_override.id.to_string(),
        term_id: term_id.to_string(),
        start_date: local::fmt_date(day_override.start_date),
        end_date: local::fmt_date(day_override.end_date),
        kind: (day_override.kind as i32).into(),
        name: day_override.name.clone(),
        follows_weekday: (day_override.follows_weekday.unwrap_or(0) as i32).into(),
        week_pattern: (day_override.week_pattern.unwrap_or(0) as i32).into(),
        ..Default::default()
    }
}

/// A class to store, checked against the term.
struct NewEntry {
    weekday: i16,
    period: i16,
    week_pattern: i16,
    subject_id: Uuid,
    teacher: String,
    room: String,
    color: String,
    memo: String,
}

fn slot_invalid() -> AppError {
    AppError::invalid("SLOT_INVALID", "the term has no such weekday or period")
}

/// A/B only makes sense in a term whose weeks alternate.
fn check_pattern(pattern: WeekPattern, alternating: bool) -> AppResult<i16> {
    match pattern {
        WeekPattern::WEEK_PATTERN_UNSPECIFIED | WeekPattern::WEEK_PATTERN_EVERY => Ok(EVERY),
        WeekPattern::WEEK_PATTERN_A | WeekPattern::WEEK_PATTERN_B if alternating => Ok(pattern as i16),
        _ => Err(AppError::invalid("WEEK_PATTERN_INVALID", "this term does not alternate between week A and week B")),
    }
}

impl School {
    /// Puts classes into the grid of a term; returns the stored classes and
    /// the IDs of the ones they replaced.
    pub(crate) async fn put_entries(
        &self,
        user_id: Uuid,
        term_id: Uuid,
        inputs: &[TimetableEntryInput],
    ) -> AppResult<(Vec<TimetableEntry>, Vec<String>)> {
        ensure!(!inputs.is_empty(), invalid, "NO_ENTRIES", "no classes given");
        ensure!(
            inputs.len() <= MAX_ENTRIES_PER_CALL,
            invalid,
            "TOO_MANY_ENTRIES",
            "at most {MAX_ENTRIES_PER_CALL} classes per call"
        );
        let own_subjects = subjects::map_of(&self.core, user_id).await?;
        let mut tx = self.core.db.begin().await?;
        let term = terms::own(&mut tx, user_id, term_id, true).await?;
        let info = term.info();

        let mut entries = Vec::with_capacity(inputs.len());
        let mut taken: HashSet<(i16, i16, i16)> = HashSet::new();
        for input in inputs {
            let weekday = input.weekday.as_known().map(|weekday| weekday as i32).unwrap_or(0);
            ensure!(
                info.layout.has_weekday(weekday) && info.layout.period(input.period).is_some(),
                invalid,
                "SLOT_INVALID",
                "the term has no such weekday or period"
            );
            let pattern =
                check_pattern(input.week_pattern.as_known().ok_or_else(slot_invalid)?, info.alternating_weeks)?;
            let slot = (weekday as i16, input.period as i16);
            // Within one call, a slot is filled once for every week, or once per week A / B.
            let collides = taken.contains(&(slot.0, slot.1, pattern))
                || if pattern == EVERY {
                    [WEEK_A, WEEK_B].iter().any(|p| taken.contains(&(slot.0, slot.1, *p)))
                } else {
                    taken.contains(&(slot.0, slot.1, EVERY))
                };
            ensure!(!collides, invalid, "SLOT_INVALID", "two classes for the same slot and week");
            taken.insert((slot.0, slot.1, pattern));
            let subject_id = parse_id(&input.subject_id)?;
            ensure!(own_subjects.contains_key(&subject_id), not_found, "SUBJECT_NOT_FOUND", "subject not found");
            entries.push(NewEntry {
                weekday: slot.0,
                period: slot.1,
                week_pattern: pattern,
                subject_id,
                teacher: text::line(&input.teacher, text::MAX_NAME, "teacher")?,
                room: text::line(&input.room, text::MAX_NAME, "room")?,
                color: text::color(&input.color)?,
                memo: text::memo(&input.memo, "memo")?,
            });
        }

        let mut replaced = Vec::new();
        let mut stored = Vec::with_capacity(entries.len());
        for entry in &entries {
            // An every-week class displaces whatever is in the slot; a week
            // A/B class displaces the every-week class and its own week's.
            let gone: Vec<Uuid> = sqlx::query_scalar(
                "DELETE FROM school_entries
                 WHERE term_id = $1 AND weekday = $2 AND period = $3 AND ($4 = 1 OR week_pattern = 1 OR week_pattern = $4)
                 RETURNING id",
            )
            .bind(term_id)
            .bind(entry.weekday)
            .bind(entry.period)
            .bind(entry.week_pattern)
            .fetch_all(&mut *tx)
            .await?;
            replaced.extend(gone);
            let id: Uuid = sqlx::query_scalar(
                "INSERT INTO school_entries (term_id, user_id, weekday, period, week_pattern, subject_id, teacher, room, color, memo)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id",
            )
            .bind(term_id)
            .bind(user_id)
            .bind(entry.weekday)
            .bind(entry.period)
            .bind(entry.week_pattern)
            .bind(entry.subject_id)
            .bind(&entry.teacher)
            .bind(&entry.room)
            .bind(&entry.color)
            .bind(&entry.memo)
            .fetch_one(&mut *tx)
            .await?;
            stored.push(EntryData {
                id,
                weekday: entry.weekday,
                period: entry.period,
                week_pattern: entry.week_pattern,
                subject_id: entry.subject_id,
                teacher: entry.teacher.clone(),
                room: entry.room.clone(),
                color: entry.color.clone(),
                memo: entry.memo.clone(),
            });
        }
        tx.commit().await?;
        let stored_ids: HashSet<Uuid> = stored.iter().map(|entry| entry.id).collect();
        replaced.retain(|id| !stored_ids.contains(id));
        Ok((
            stored.iter().map(|entry| entry_proto(term_id, entry, &own_subjects, true)).collect(),
            replaced.iter().map(Uuid::to_string).collect(),
        ))
    }

    pub(crate) async fn delete_entries(&self, user_id: Uuid, term_id: Uuid, entry_ids: &[Uuid]) -> AppResult<i32> {
        let mut conn = self.core.db.acquire().await?;
        terms::own(&mut conn, user_id, term_id, false).await?;
        let deleted = sqlx::query("DELETE FROM school_entries WHERE term_id = $1 AND id = ANY($2)")
            .bind(term_id)
            .bind(entry_ids)
            .execute(&mut *conn)
            .await?;
        Ok(deleted.rows_affected() as i32)
    }

    pub(crate) async fn create_change(
        &self,
        user_id: Uuid,
        request: &CreateTimetableChangeRequest,
    ) -> AppResult<TimetableChange> {
        let term_id = parse_id(&request.term_id)?;
        let kind = request.kind.as_known().filter(|kind| *kind != ChangeKind::CHANGE_KIND_UNSPECIFIED);
        let kind = kind.ok_or_else(|| {
            AppError::invalid("CHANGE_KIND_REQUIRED", "say what changes: cancelled, moved, replaced or extra")
        })?;
        let date = local::parse_date(&request.date)?;
        let teacher = text::line(&request.teacher, text::MAX_NAME, "teacher")?;
        let room = text::line(&request.room, text::MAX_NAME, "room")?;
        let memo = text::memo(&request.memo, "memo")?;

        let mut tx = self.core.db.begin().await?;
        let term = terms::own(&mut tx, user_id, term_id, true).await?;
        let data = load_one(&mut tx, &term).await?;
        ensure!(data.info.covers(date), invalid, "DATE_OUTSIDE_TERM", "the date is outside the term");
        ensure!(
            data.info.layout.period(request.period).is_some(),
            invalid,
            "SLOT_INVALID",
            "the term has no such period"
        );
        let subject = match parse_opt_id(&request.subject_id)? {
            Some(id) => Some(subjects::require(&mut tx, user_id, id).await?),
            None => None,
        };

        let acts_on_a_class = kind != ChangeKind::CHANGE_KIND_EXTRA;
        ensure!(
            !acts_on_a_class || resolve::grid_class(&data, date, request.period).is_some(),
            precondition,
            "NO_CLASS_IN_SLOT",
            "no class is scheduled in that slot on that date"
        );
        let mut target: Option<(NaiveDate, i16)> = None;
        let (subject_id, teacher, room) = match kind {
            ChangeKind::CHANGE_KIND_CANCELLED => (None, String::new(), String::new()),
            ChangeKind::CHANGE_KIND_MOVED => {
                ensure!(
                    !request.to_date.trim().is_empty() && request.to_period > 0,
                    invalid,
                    "MOVE_TARGET_REQUIRED",
                    "say where the class moves to"
                );
                let to_date = local::parse_date(&request.to_date)?;
                ensure!(data.info.covers(to_date), invalid, "DATE_OUTSIDE_TERM", "the new date is outside the term");
                ensure!(
                    data.info.layout.period(request.to_period).is_some(),
                    invalid,
                    "SLOT_INVALID",
                    "the term has no such period"
                );
                ensure!(
                    (to_date, request.to_period) != (date, request.period),
                    invalid,
                    "MOVE_TARGET_REQUIRED",
                    "the class would move to where it already is"
                );
                target = Some((to_date, request.to_period as i16));
                (None, String::new(), String::new())
            }
            ChangeKind::CHANGE_KIND_REPLACED => {
                ensure!(
                    subject.is_some() || !teacher.is_empty() || !room.is_empty(),
                    invalid,
                    "SUBJECT_REQUIRED",
                    "a replacement needs another subject, teacher or room"
                );
                (subject.as_ref().map(|s| s.id), teacher, room)
            }
            ChangeKind::CHANGE_KIND_EXTRA => {
                let subject = subject
                    .as_ref()
                    .ok_or_else(|| AppError::invalid("SUBJECT_REQUIRED", "an extra class needs a subject"))?;
                (Some(subject.id), teacher, room)
            }
            ChangeKind::CHANGE_KIND_UNSPECIFIED => unreachable!("filtered above"),
        };

        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM school_changes WHERE term_id = $1")
            .bind(term_id)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(
            count < MAX_CHANGES,
            precondition,
            "CHANGE_LIMIT_REACHED",
            "a term has at most {MAX_CHANGES} one-off changes"
        );
        let record: ChangeRecord = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO school_changes (term_id, user_id, date, period, kind, subject_id, teacher, room, memo, to_date, to_period)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
             ON CONFLICT (term_id, date, period) DO UPDATE SET
                 kind = EXCLUDED.kind, subject_id = EXCLUDED.subject_id, teacher = EXCLUDED.teacher, room = EXCLUDED.room,
                 memo = EXCLUDED.memo, to_date = EXCLUDED.to_date, to_period = EXCLUDED.to_period
             RETURNING {CHANGE_COLUMNS}"
        )))
        .bind(term_id)
        .bind(user_id)
        .bind(date)
        .bind(request.period as i16)
        .bind(kind as i16)
        .bind(subject_id)
        .bind(&teacher)
        .bind(&room)
        .bind(&memo)
        .bind(target.map(|(to_date, _)| to_date))
        .bind(target.map(|(_, to_period)| to_period))
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(TimetableChange {
            id: record.id.to_string(),
            term_id: record.term_id.to_string(),
            date: local::fmt_date(record.date),
            period: record.period as i32,
            kind: (record.kind as i32).into(),
            subject: subject.filter(|s| Some(s.id) == record.subject_id).map(|s| s.to_proto()).into(),
            teacher: record.teacher,
            room: record.room,
            memo: record.memo,
            to_date: local::fmt_opt_date(record.to_date),
            to_period: record.to_period.unwrap_or(0) as i32,
            ..Default::default()
        })
    }

    pub(crate) async fn delete_change(&self, user_id: Uuid, change_id: Uuid) -> AppResult<()> {
        let deleted = sqlx::query("DELETE FROM school_changes WHERE id = $1 AND user_id = $2")
            .bind(change_id)
            .bind(user_id)
            .execute(&self.core.db)
            .await?;
        ensure!(deleted.rows_affected() > 0, not_found, "CHANGE_NOT_FOUND", "change not found");
        Ok(())
    }

    pub(crate) async fn create_day_override(
        &self,
        user_id: Uuid,
        request: &CreateDayOverrideRequest,
    ) -> AppResult<DayOverride> {
        let term_id = parse_id(&request.term_id)?;
        let kind = request.kind.as_known().filter(|kind| *kind != DayOverrideKind::DAY_OVERRIDE_KIND_UNSPECIFIED);
        let kind = kind.ok_or_else(|| {
            AppError::invalid(
                "OVERRIDE_KIND_REQUIRED",
                "say what the dates are: a holiday, a class day or a pinned week",
            )
        })?;
        let start_date = local::parse_date(&request.start_date)?;
        let end_date = local::parse_opt_date(&request.end_date)?.unwrap_or(start_date);
        ensure!(end_date >= start_date, invalid, "DATE_RANGE_INVALID", "the range cannot end before it starts");
        let name = text::line(&request.name, text::MAX_NAME, "name")?;

        let mut tx = self.core.db.begin().await?;
        let term = terms::own(&mut tx, user_id, term_id, true).await?;
        ensure!(
            term.start_date <= start_date && end_date <= term.end_date,
            invalid,
            "DATE_OUTSIDE_TERM",
            "the dates are outside the term"
        );
        let pinned = match request.week_pattern.as_known() {
            Some(WeekPattern::WEEK_PATTERN_UNSPECIFIED | WeekPattern::WEEK_PATTERN_EVERY) => None,
            Some(pattern) => Some(check_pattern(pattern, term.alternating_weeks)?),
            None => return Err(AppError::invalid("WEEK_PATTERN_INVALID", "unknown week pattern")),
        };
        let (follows_weekday, week_pattern) = match kind {
            DayOverrideKind::DAY_OVERRIDE_KIND_HOLIDAY => (None, None),
            DayOverrideKind::DAY_OVERRIDE_KIND_CLASS_DAY => {
                let follows =
                    request.follows_weekday.as_known().filter(|weekday| *weekday != Weekday::WEEKDAY_UNSPECIFIED);
                (follows.map(|weekday| weekday as i16), pinned)
            }
            DayOverrideKind::DAY_OVERRIDE_KIND_WEEK_PATTERN => {
                ensure!(pinned.is_some(), invalid, "WEEK_PATTERN_INVALID", "say which week the dates run as: A or B");
                (None, pinned)
            }
            DayOverrideKind::DAY_OVERRIDE_KIND_UNSPECIFIED => unreachable!("filtered above"),
        };
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM school_day_overrides WHERE term_id = $1")
            .bind(term_id)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(
            count < MAX_OVERRIDES,
            precondition,
            "OVERRIDE_LIMIT_REACHED",
            "a term has at most {MAX_OVERRIDES} day overrides"
        );
        let record: OverrideRecord = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO school_day_overrides (term_id, user_id, start_date, end_date, kind, name, follows_weekday, week_pattern)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING {OVERRIDE_COLUMNS}"
        )))
        .bind(term_id)
        .bind(user_id)
        .bind(start_date)
        .bind(end_date)
        .bind(kind as i16)
        .bind(&name)
        .bind(follows_weekday)
        .bind(week_pattern)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(override_proto(
            term_id,
            &OverrideData {
                id: record.id,
                start_date: record.start_date,
                end_date: record.end_date,
                kind: record.kind,
                name: record.name,
                follows_weekday: record.follows_weekday,
                week_pattern: record.week_pattern,
            },
        ))
    }

    pub(crate) async fn delete_day_override(&self, user_id: Uuid, override_id: Uuid) -> AppResult<()> {
        let deleted = sqlx::query("DELETE FROM school_day_overrides WHERE id = $1 AND user_id = $2")
            .bind(override_id)
            .bind(user_id)
            .execute(&self.core.db)
            .await?;
        ensure!(deleted.rows_affected() > 0, not_found, "OVERRIDE_NOT_FOUND", "day override not found");
        Ok(())
    }
}
