//! Terms: rows, who may see them, and creating, changing and deleting them.
use std::collections::HashSet;

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::{PgConnection, Postgres, Transaction};
use tokuly_core::prelude::*;
use tokuly_proto::{
    common::{Timestamps, UserSummary},
    school::{CreateTermRequest, DayOverrideKind, Term, TimetableAudience, UpdateTermRequest},
};
use tokuly_social::graph;

use super::{
    layout::{self, Layout},
    resolve::{EVERY, TermInfo, WEEK_A, WEEK_B},
};
use crate::{School, local, text};

pub(crate) const MAX_TERMS: i64 = 30;
const MAX_TERM_DAYS: i64 = 400;
const MAX_SHARES: usize = 200;

pub(crate) const AUDIENCE_NOBODY: i16 = TimetableAudience::TIMETABLE_AUDIENCE_NOBODY as i16;
pub(crate) const AUDIENCE_CHOSEN: i16 = TimetableAudience::TIMETABLE_AUDIENCE_CHOSEN as i16;
pub(crate) const AUDIENCE_FRIENDS: i16 = TimetableAudience::TIMETABLE_AUDIENCE_FRIENDS as i16;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct TermRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub is_current: bool,
    pub layout: serde_json::Value,
    pub alternating_weeks: bool,
    pub week_a_date: Option<NaiveDate>,
    pub observe_public_holidays: bool,
    pub audience: i16,
    pub copied_from_user_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub entry_count: i64,
}

pub(crate) const COLUMNS: &str = "t.id, t.user_id, t.name, t.start_date, t.end_date, t.is_current, t.layout, t.alternating_weeks, t.week_a_date, \
     t.observe_public_holidays, t.audience, t.copied_from_user_id, t.created_at, t.updated_at, \
     (SELECT count(*) FROM school_entries e WHERE e.term_id = t.id) AS entry_count";

impl TermRow {
    pub fn info(&self) -> TermInfo {
        TermInfo {
            id: self.id,
            name: self.name.clone(),
            start_date: self.start_date,
            end_date: self.end_date,
            is_current: self.is_current,
            layout: Layout::from_json(&self.layout),
            alternating_weeks: self.alternating_weeks,
            week_a_date: self.week_a_date,
            observe_public_holidays: self.observe_public_holidays,
        }
    }

    /// The term as friends see it: nothing about who else it is shared with
    /// or where it was copied from.
    pub fn to_proto(&self) -> Term {
        Term {
            id: self.id.to_string(),
            owner_id: self.user_id.to_string(),
            name: self.name.clone(),
            start_date: local::fmt_date(self.start_date),
            end_date: local::fmt_date(self.end_date),
            current: self.is_current,
            layout: Layout::from_json(&self.layout).to_proto().into(),
            alternating_weeks: self.alternating_weeks,
            week_a_date: local::fmt_opt_date(self.week_a_date),
            observe_public_holidays: self.observe_public_holidays,
            audience: (self.audience as i32).into(),
            entry_count: self.entry_count as i32,
            timestamps: Timestamps {
                created_at: ts(self.created_at),
                updated_at: ts(self.updated_at),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        }
    }
}

fn not_found() -> AppError {
    AppError::not_found("TERM_NOT_FOUND", "term not found")
}

/// A user's terms, newest first.
pub(crate) async fn of_user(core: &Core, user_id: Uuid) -> AppResult<Vec<TermRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM school_terms t WHERE t.user_id = $1 ORDER BY t.start_date DESC, t.id DESC"
    )))
    .bind(user_id)
    .fetch_all(&core.db)
    .await?)
}

/// The terms of `owner` that `viewer` may see: all of their own; of someone
/// else's, the ones shared with them — which takes a friendship, so a block
/// or an unfriending hides everything at once.
pub(crate) async fn visible_to(core: &Core, owner: Uuid, viewer: Uuid) -> AppResult<Vec<TermRow>> {
    if owner == viewer {
        return of_user(core, owner).await;
    }
    if !graph::are_friends(core, owner, viewer).await? {
        return Ok(Vec::new());
    }
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM school_terms t
         WHERE t.user_id = $1
           AND (t.audience = {AUDIENCE_FRIENDS}
                OR (t.audience = {AUDIENCE_CHOSEN} AND EXISTS (SELECT 1 FROM school_term_shares s WHERE s.term_id = t.id AND s.user_id = $2)))
         ORDER BY t.start_date DESC, t.id DESC"
    )))
    .bind(owner)
    .bind(viewer)
    .fetch_all(&core.db)
    .await?)
}

/// One of the caller's own terms, or `TERM_NOT_FOUND`.
pub(crate) async fn own(conn: &mut PgConnection, user_id: Uuid, term_id: Uuid, for_update: bool) -> AppResult<TermRow> {
    let lock = if for_update { " FOR UPDATE OF t" } else { "" };
    let row: Option<TermRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM school_terms t WHERE t.id = $1 AND t.user_id = $2{lock}"
    )))
    .bind(term_id)
    .bind(user_id)
    .fetch_optional(conn)
    .await?;
    row.ok_or_else(not_found)
}

/// The fields of a term a request may set, checked.
struct TermFields {
    name: String,
    start_date: NaiveDate,
    end_date: NaiveDate,
    layout: Layout,
    alternating_weeks: bool,
    week_a_date: Option<NaiveDate>,
    observe_public_holidays: bool,
    audience: i16,
}

fn check_dates(start_date: NaiveDate, end_date: NaiveDate) -> AppResult<()> {
    ensure!(end_date >= start_date, invalid, "DATE_RANGE_INVALID", "a term cannot end before it starts");
    ensure!(
        (end_date - start_date).num_days() < MAX_TERM_DAYS,
        invalid,
        "DATE_RANGE_INVALID",
        "a term is at most {MAX_TERM_DAYS} days long"
    );
    Ok(())
}

fn check_audience(audience: TimetableAudience) -> AppResult<i16> {
    match audience {
        TimetableAudience::TIMETABLE_AUDIENCE_NOBODY
        | TimetableAudience::TIMETABLE_AUDIENCE_CHOSEN
        | TimetableAudience::TIMETABLE_AUDIENCE_FRIENDS => Ok(audience as i16),
        TimetableAudience::TIMETABLE_AUDIENCE_UNSPECIFIED => {
            Err(AppError::invalid("AUDIENCE_INVALID", "unknown audience"))
        }
    }
}

fn check_week_a(alternating_weeks: bool, week_a_date: Option<NaiveDate>) -> AppResult<Option<NaiveDate>> {
    if !alternating_weeks {
        return Ok(None);
    }
    ensure!(week_a_date.is_some(), invalid, "WEEK_A_DATE_REQUIRED", "a term with A/B weeks needs a date in a week A");
    Ok(week_a_date)
}

impl School {
    /// The chosen friends to share a term with: friends of the owner only.
    async fn check_shares(&self, user_id: Uuid, ids: &[String]) -> AppResult<Vec<Uuid>> {
        let mut seen = HashSet::new();
        let ids: Vec<Uuid> = parse_ids(ids)?.into_iter().filter(|id| seen.insert(*id)).collect();
        ensure!(
            ids.len() <= MAX_SHARES,
            invalid,
            "TOO_MANY_SHARES",
            "a term is shared with at most {MAX_SHARES} chosen friends"
        );
        let friends: HashSet<Uuid> = graph::friend_ids(&self.core, user_id).await?.into_iter().collect();
        ensure!(
            ids.iter().all(|id| friends.contains(id)),
            precondition,
            "NOT_FRIENDS",
            "a timetable can only be shared with friends"
        );
        Ok(ids)
    }

    pub(crate) async fn create_term(&self, user_id: Uuid, request: &CreateTermRequest) -> AppResult<TermRow> {
        let start_date = local::parse_date(&request.start_date)?;
        let end_date = local::parse_date(&request.end_date)?;
        check_dates(start_date, end_date)?;
        let audience =
            request.audience.as_known().ok_or_else(|| AppError::invalid("AUDIENCE_INVALID", "unknown audience"))?;
        let fields = TermFields {
            name: text::name(&request.name, text::MAX_NAME, "TERM_NAME_INVALID", "term name")?,
            start_date,
            end_date,
            layout: layout::validate(request.layout.as_option())?,
            alternating_weeks: request.alternating_weeks,
            week_a_date: check_week_a(request.alternating_weeks, local::parse_opt_date(&request.week_a_date)?)?,
            observe_public_holidays: request.observe_public_holidays.unwrap_or(true),
            audience: match audience {
                TimetableAudience::TIMETABLE_AUDIENCE_UNSPECIFIED => AUDIENCE_NOBODY,
                audience => check_audience(audience)?,
            },
        };
        let shares = self.check_shares(user_id, &request.shared_with_user_ids).await?;

        let mut tx = self.core.db.begin().await?;
        crate::lock_user(&mut tx, "terms", user_id).await?;
        let (count, has_current): (i64, bool) = sqlx::query_as(
            "SELECT count(*), COALESCE(bool_or(is_current), false) FROM school_terms WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(count < MAX_TERMS, precondition, "TERM_LIMIT_REACHED", "an account has at most {MAX_TERMS} terms");
        let current = request.current || !has_current;
        let term_id = insert(&mut tx, user_id, &fields, current, None).await?;
        replace_shares(&mut tx, term_id, &shares).await?;
        let row = own(&mut tx, user_id, term_id, false).await?;
        tx.commit().await?;
        Ok(row)
    }

    /// Changes a term; returns it with the number of grid classes that the
    /// new layout or week scheme had no place for.
    pub(crate) async fn update_term(&self, user_id: Uuid, request: &UpdateTermRequest) -> AppResult<(TermRow, i32)> {
        let term_id = parse_id(&request.term_id)?;
        let shares = match request.shared_with.as_option() {
            Some(list) => Some(self.check_shares(user_id, &list.user_ids).await?),
            None => None,
        };

        let mut tx = self.core.db.begin().await?;
        crate::lock_user(&mut tx, "terms", user_id).await?;
        let term = own(&mut tx, user_id, term_id, true).await?;
        let start_date = match &request.start_date {
            Some(date) => local::parse_date(date)?,
            None => term.start_date,
        };
        let end_date = match &request.end_date {
            Some(date) => local::parse_date(date)?,
            None => term.end_date,
        };
        check_dates(start_date, end_date)?;
        let alternating_weeks = request.alternating_weeks.unwrap_or(term.alternating_weeks);
        let week_a_date = match &request.week_a_date {
            Some(date) => local::parse_opt_date(date)?,
            None => term.week_a_date,
        };
        let fields = TermFields {
            name: match &request.name {
                Some(name) => text::name(name, text::MAX_NAME, "TERM_NAME_INVALID", "term name")?,
                None => term.name.clone(),
            },
            start_date,
            end_date,
            layout: match request.layout.as_option() {
                Some(layout) => layout::validate(Some(layout))?,
                None => Layout::from_json(&term.layout),
            },
            alternating_weeks,
            week_a_date: check_week_a(alternating_weeks, week_a_date)?,
            observe_public_holidays: request.observe_public_holidays.unwrap_or(term.observe_public_holidays),
            audience: match request.audience.as_known() {
                Some(TimetableAudience::TIMETABLE_AUDIENCE_UNSPECIFIED) => term.audience,
                Some(audience) => check_audience(audience)?,
                None => return Err(AppError::invalid("AUDIENCE_INVALID", "unknown audience")),
            },
        };

        sqlx::query(
            "UPDATE school_terms SET name = $2, start_date = $3, end_date = $4, layout = $5, alternating_weeks = $6, week_a_date = $7,
                 observe_public_holidays = $8, audience = $9
             WHERE id = $1",
        )
        .bind(term_id)
        .bind(&fields.name)
        .bind(fields.start_date)
        .bind(fields.end_date)
        .bind(fields.layout.to_json())
        .bind(fields.alternating_weeks)
        .bind(fields.week_a_date)
        .bind(fields.observe_public_holidays)
        .bind(fields.audience)
        .execute(&mut *tx)
        .await?;
        if let Some(shares) = &shares {
            replace_shares(&mut tx, term_id, shares).await?;
        }
        let removed = fit_contents(&mut tx, term_id, &term, &fields).await?;
        if request.current == Some(true) && !term.is_current {
            sqlx::query("UPDATE school_terms SET is_current = false WHERE user_id = $1 AND is_current")
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE school_terms SET is_current = true WHERE id = $1")
                .bind(term_id)
                .execute(&mut *tx)
                .await?;
        }
        let row = own(&mut tx, user_id, term_id, false).await?;
        tx.commit().await?;
        Ok((row, removed))
    }

    pub(crate) async fn delete_term(&self, user_id: Uuid, term_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        crate::lock_user(&mut tx, "terms", user_id).await?;
        let term = own(&mut tx, user_id, term_id, true).await?;
        sqlx::query("DELETE FROM school_terms WHERE id = $1").bind(term_id).execute(&mut *tx).await?;
        if term.is_current {
            // Someone with terms always has a current one.
            sqlx::query(
                "UPDATE school_terms SET is_current = true
                 WHERE id = (SELECT id FROM school_terms WHERE user_id = $1 ORDER BY start_date DESC, id DESC LIMIT 1)",
            )
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// A term as its owner sees it, with the chosen friends and the classmate
    /// it was copied from.
    pub(crate) async fn term_for_owner(&self, term: &TermRow) -> AppResult<Term> {
        let core = &self.core;
        let shared_with: Vec<Uuid> = sqlx::query_scalar(
            "SELECT user_id FROM school_term_shares WHERE term_id = $1 ORDER BY created_at, user_id",
        )
        .bind(term.id)
        .fetch_all(&core.db)
        .await?;
        let mut ids = shared_with.clone();
        ids.extend(term.copied_from_user_id);
        let users = tokuly_identity::summaries(core, &ids).await?;
        let known = |id: &Uuid| -> Option<UserSummary> { users.get(id).filter(|user| !user.deleted).cloned() };
        Ok(Term {
            shared_with: shared_with.iter().filter_map(known).collect(),
            copied_from: term.copied_from_user_id.as_ref().and_then(known).into(),
            ..term.to_proto()
        })
    }
}

/// Inserts a term, making it the current one when asked.
async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    fields: &TermFields,
    current: bool,
    copied_from: Option<Uuid>,
) -> AppResult<Uuid> {
    if current {
        sqlx::query("UPDATE school_terms SET is_current = false WHERE user_id = $1 AND is_current")
            .bind(user_id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(sqlx::query_scalar(
        "INSERT INTO school_terms (user_id, name, start_date, end_date, is_current, layout, alternating_weeks, week_a_date,
             observe_public_holidays, audience, copied_from_user_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING id",
    )
    .bind(user_id)
    .bind(&fields.name)
    .bind(fields.start_date)
    .bind(fields.end_date)
    .bind(current)
    .bind(fields.layout.to_json())
    .bind(fields.alternating_weeks)
    .bind(fields.week_a_date)
    .bind(fields.observe_public_holidays)
    .bind(fields.audience)
    .bind(copied_from)
    .fetch_one(&mut **tx)
    .await?)
}

/// Creates the caller's copy of someone else's term: same name (unless
/// another is given), dates, layout and week scheme; private, and current
/// when the caller has no current term yet.
pub(crate) async fn insert_copy(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    source: &TermRow,
    name: Option<String>,
) -> AppResult<Uuid> {
    let (count, has_current): (i64, bool) =
        sqlx::query_as("SELECT count(*), COALESCE(bool_or(is_current), false) FROM school_terms WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&mut **tx)
            .await?;
    ensure!(count < MAX_TERMS, precondition, "TERM_LIMIT_REACHED", "an account has at most {MAX_TERMS} terms");
    let fields = TermFields {
        name: name.unwrap_or_else(|| source.name.clone()),
        start_date: source.start_date,
        end_date: source.end_date,
        layout: Layout::from_json(&source.layout),
        alternating_weeks: source.alternating_weeks,
        week_a_date: source.week_a_date,
        observe_public_holidays: source.observe_public_holidays,
        audience: AUDIENCE_NOBODY,
    };
    insert(tx, user_id, &fields, !has_current, Some(source.user_id)).await
}

async fn replace_shares(tx: &mut Transaction<'_, Postgres>, term_id: Uuid, user_ids: &[Uuid]) -> AppResult<()> {
    sqlx::query("DELETE FROM school_term_shares WHERE term_id = $1 AND user_id <> ALL($2)")
        .bind(term_id)
        .bind(user_ids)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "INSERT INTO school_term_shares (term_id, user_id) SELECT $1, unnest($2::uuid[]) ON CONFLICT DO NOTHING",
    )
    .bind(term_id)
    .bind(user_ids)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Makes the grid, changes and day overrides of a term fit its new shape.
/// Returns how many grid classes were removed.
async fn fit_contents(
    tx: &mut Transaction<'_, Postgres>,
    term_id: Uuid,
    before: &TermRow,
    after: &TermFields,
) -> AppResult<i32> {
    let mut removed = 0;
    // Classes in periods or on weekdays the layout no longer has.
    let periods = after.layout.periods.len() as i16;
    let weekdays: Vec<i16> = after.layout.weekdays.iter().map(|weekday| *weekday as i16).collect();
    removed += sqlx::query("DELETE FROM school_entries WHERE term_id = $1 AND (period > $2 OR weekday <> ALL($3))")
        .bind(term_id)
        .bind(periods)
        .bind(&weekdays)
        .execute(&mut **tx)
        .await?
        .rows_affected();
    sqlx::query("DELETE FROM school_changes WHERE term_id = $1 AND (period > $2 OR to_period > $2)")
        .bind(term_id)
        .bind(periods)
        .execute(&mut **tx)
        .await?;

    if before.alternating_weeks && !after.alternating_weeks {
        // Back to one timetable for every week: week A's classes stay, week
        // B's only where week A had nothing.
        removed += sqlx::query(
            "DELETE FROM school_entries b WHERE b.term_id = $1 AND b.week_pattern = $2
               AND EXISTS (SELECT 1 FROM school_entries a
                           WHERE a.term_id = b.term_id AND a.weekday = b.weekday AND a.period = b.period AND a.week_pattern = $3)",
        )
        .bind(term_id)
        .bind(WEEK_B)
        .bind(WEEK_A)
        .execute(&mut **tx)
        .await?
        .rows_affected();
        sqlx::query("UPDATE school_entries SET week_pattern = $2 WHERE term_id = $1 AND week_pattern <> $2")
            .bind(term_id)
            .bind(EVERY)
            .execute(&mut **tx)
            .await?;
        sqlx::query("DELETE FROM school_day_overrides WHERE term_id = $1 AND kind = $2")
            .bind(term_id)
            .bind(DayOverrideKind::DAY_OVERRIDE_KIND_WEEK_PATTERN as i16)
            .execute(&mut **tx)
            .await?;
        sqlx::query("UPDATE school_day_overrides SET week_pattern = NULL WHERE term_id = $1")
            .bind(term_id)
            .execute(&mut **tx)
            .await?;
    }

    if after.start_date != before.start_date || after.end_date != before.end_date {
        // Changes and overrides for dates the term no longer covers.
        sqlx::query("DELETE FROM school_changes WHERE term_id = $1 AND (date NOT BETWEEN $2 AND $3 OR to_date NOT BETWEEN $2 AND $3)")
            .bind(term_id)
            .bind(after.start_date)
            .bind(after.end_date)
            .execute(&mut **tx)
            .await?;
        sqlx::query("DELETE FROM school_day_overrides WHERE term_id = $1 AND (end_date < $2 OR start_date > $3)")
            .bind(term_id)
            .bind(after.start_date)
            .bind(after.end_date)
            .execute(&mut **tx)
            .await?;
        sqlx::query("UPDATE school_day_overrides SET start_date = GREATEST(start_date, $2), end_date = LEAST(end_date, $3) WHERE term_id = $1")
            .bind(term_id)
            .bind(after.start_date)
            .bind(after.end_date)
            .execute(&mut **tx)
            .await?;
    }
    Ok(removed as i32)
}
