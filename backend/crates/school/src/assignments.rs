//! Assignments with due times and reminders, and `AssignmentService`.
//!
//! Each open assignment carries the moments its two reminders are due (the
//! evening before, and an hour before); the sweep sends the ones whose
//! moment has come and clears them. A moment that had passed already when
//! the assignment was saved is never sent late.
use std::collections::HashMap;

use chrono::{DateTime, Duration, NaiveTime, Utc};
use chrono_tz::Tz;
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use serde::{Deserialize, Serialize};
use tokuly_core::{
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::{
    common::{PageInfo, Timestamps},
    school::*,
};

use crate::{
    School, local, settings,
    subjects::{self, SubjectRow},
    text,
};

pub(crate) const KIND_ASSIGNMENT_DUE: &str = "school.assignment_due";
const MAX_TITLE: usize = 80;
const MAX_OPEN: i64 = 500;
const MAX_DUE_DAYS: i64 = 730;
/// A reminder this late says nothing useful anymore and is dropped.
const MAX_LATENESS_HOURS: i64 = 6;

#[derive(Debug, Clone, sqlx::FromRow)]
struct AssignmentRow {
    id: Uuid,
    user_id: Uuid,
    subject_id: Option<Uuid>,
    title: String,
    memo: String,
    due_at: DateTime<Utc>,
    done_at: Option<DateTime<Utc>>,
    remind: bool,
    remind_evening_at: Option<DateTime<Utc>>,
    remind_hour_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

const COLUMNS: &str = "id, user_id, subject_id, title, memo, due_at, done_at, remind, remind_evening_at, remind_hour_at, created_at, updated_at";

/// When the reminders of an assignment due at `due_at` go out: at
/// `evening_hour` o'clock on the day before, and one hour before. A moment
/// that is not in the future is skipped, and so is an evening reminder that
/// would come after the one-hour one.
pub(crate) fn reminder_times(due_at: Time, tz: Tz, evening_hour: u32, at: Time) -> (Option<Time>, Option<Time>) {
    let day_before = local::local_date(due_at, tz) - Duration::days(1);
    let evening = local::local_instant(
        day_before.and_time(NaiveTime::from_hms_opt(evening_hour, 0, 0).unwrap_or(NaiveTime::MIN)),
        tz,
    );
    let hour = due_at - Duration::hours(1);
    (Some(evening).filter(|moment| *moment > at && *moment < hour), Some(hour).filter(|moment| *moment > at))
}

/// How pressing an assignment is on the caller's calendar.
pub(crate) fn urgency(due_at: Time, done: bool, at: Time, tz: Tz) -> AssignmentUrgency {
    if done {
        return AssignmentUrgency::ASSIGNMENT_URGENCY_DONE;
    }
    if due_at < at {
        return AssignmentUrgency::ASSIGNMENT_URGENCY_OVERDUE;
    }
    match (local::local_date(due_at, tz) - local::local_date(at, tz)).num_days() {
        0 => AssignmentUrgency::ASSIGNMENT_URGENCY_TODAY,
        1 => AssignmentUrgency::ASSIGNMENT_URGENCY_TOMORROW,
        2..=7 => AssignmentUrgency::ASSIGNMENT_URGENCY_THIS_WEEK,
        _ => AssignmentUrgency::ASSIGNMENT_URGENCY_LATER,
    }
}

/// Where a page of the list ended: among the open assignments (by due time)
/// or among the finished ones (by when they were finished).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct Cursor {
    done: bool,
    at: DateTime<Utc>,
    id: Uuid,
}

fn not_found() -> AppError {
    AppError::not_found("ASSIGNMENT_NOT_FOUND", "assignment not found")
}

fn check_due(due_at: Time, at: Time) -> AppResult<()> {
    let limit = Duration::days(MAX_DUE_DAYS);
    ensure!(
        due_at <= at + limit && due_at >= at - limit,
        invalid,
        "DUE_INVALID",
        "the due time must be within two years of now"
    );
    Ok(())
}

impl AssignmentRow {
    fn to_proto(&self, subjects: &HashMap<Uuid, SubjectRow>, at: Time, tz: Tz) -> Assignment {
        Assignment {
            id: self.id.to_string(),
            title: self.title.clone(),
            subject: self.subject_id.and_then(|id| subjects.get(&id)).map(SubjectRow::to_proto).into(),
            due_at: ts(self.due_at),
            done: self.done_at.is_some(),
            done_at: opt_ts(self.done_at),
            memo: self.memo.clone(),
            urgency: urgency(self.due_at, self.done_at.is_some(), at, tz).into(),
            remind: self.remind,
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

impl School {
    /// The reminder moments an assignment should carry right now.
    fn reminders_for(&self, row: &AssignmentRow, reminders_on: bool, tz: Tz, at: Time) -> (Option<Time>, Option<Time>) {
        if !reminders_on || !row.remind || row.done_at.is_some() {
            return (None, None);
        }
        reminder_times(row.due_at, tz, self.cfg.assignment_evening_hour, at)
    }

    async fn assignment_proto(&self, row: &AssignmentRow, at: Time) -> AppResult<Assignment> {
        let core = &self.core;
        let subjects = subjects::by_ids(core, &row.subject_id.into_iter().collect::<Vec<_>>()).await?;
        Ok(row.to_proto(&subjects, at, local::time_zone(core, row.user_id).await?))
    }

    async fn create_assignment(&self, user_id: Uuid, request: &CreateAssignmentRequest) -> AppResult<AssignmentRow> {
        let core = &self.core;
        let at = self.now();
        let title = text::name(&request.title, MAX_TITLE, "ASSIGNMENT_TITLE_INVALID", "assignment title")?;
        let due_at = request
            .due_at
            .as_option()
            .map(from_ts)
            .ok_or_else(|| AppError::invalid("DUE_REQUIRED", "say when the assignment is due"))?;
        check_due(due_at, at)?;
        let memo = text::memo(&request.memo, "memo")?;
        let subject = subjects::require_opt(core, user_id, &request.subject_id).await?;
        let reminders_on = settings::of(core, user_id).await?.assignment_reminders;
        let tz = local::time_zone(core, user_id).await?;

        let mut tx = core.db.begin().await?;
        crate::lock_user(&mut tx, "assignments", user_id).await?;
        let open: i64 =
            sqlx::query_scalar("SELECT count(*) FROM school_assignments WHERE user_id = $1 AND done_at IS NULL")
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await?;
        ensure!(
            open < MAX_OPEN,
            precondition,
            "ASSIGNMENT_LIMIT_REACHED",
            "an account has at most {MAX_OPEN} open assignments"
        );
        let remind = request.remind.unwrap_or(true);
        let (evening, hour) = if reminders_on && remind {
            reminder_times(due_at, tz, self.cfg.assignment_evening_hour, at)
        } else {
            (None, None)
        };
        let row: AssignmentRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO school_assignments (user_id, subject_id, title, memo, due_at, remind, remind_evening_at, remind_hour_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING {COLUMNS}"
        )))
        .bind(user_id)
        .bind(subject.as_ref().map(|s| s.id))
        .bind(&title)
        .bind(&memo)
        .bind(due_at)
        .bind(remind)
        .bind(evening)
        .bind(hour)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row)
    }

    async fn update_assignment(&self, user_id: Uuid, request: &UpdateAssignmentRequest) -> AppResult<AssignmentRow> {
        let core = &self.core;
        let at = self.now();
        let assignment_id = parse_id(&request.assignment_id)?;
        let title = request
            .title
            .as_deref()
            .map(|title| text::name(title, MAX_TITLE, "ASSIGNMENT_TITLE_INVALID", "assignment title"))
            .transpose()?;
        let memo = request.memo.as_deref().map(|memo| text::memo(memo, "memo")).transpose()?;
        let subject = match &request.subject_id {
            Some(id) => Some(subjects::require_opt(core, user_id, id).await?),
            None => None,
        };
        let due_at = request.due_at.as_option().map(from_ts);
        if let Some(due_at) = due_at {
            check_due(due_at, at)?;
        }
        let reminders_on = settings::of(core, user_id).await?.assignment_reminders;
        let tz = local::time_zone(core, user_id).await?;

        let mut tx = core.db.begin().await?;
        let row: Option<AssignmentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM school_assignments WHERE id = $1 AND user_id = $2 FOR UPDATE"
        )))
        .bind(assignment_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
        let mut row = row.ok_or_else(not_found)?;
        if let Some(title) = title {
            row.title = title;
        }
        if let Some(memo) = memo {
            row.memo = memo;
        }
        if let Some(subject) = subject {
            row.subject_id = subject.map(|s| s.id);
        }
        if let Some(due_at) = due_at {
            row.due_at = due_at;
        }
        if let Some(remind) = request.remind {
            row.remind = remind;
        }
        match request.done {
            Some(true) if row.done_at.is_none() => row.done_at = Some(at),
            Some(false) => row.done_at = None,
            _ => {}
        }
        let (evening, hour) = self.reminders_for(&row, reminders_on, tz, at);
        let row: AssignmentRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE school_assignments SET subject_id = $2, title = $3, memo = $4, due_at = $5, done_at = $6, remind = $7,
                 remind_evening_at = $8, remind_hour_at = $9
             WHERE id = $1 RETURNING {COLUMNS}"
        )))
        .bind(assignment_id)
        .bind(row.subject_id)
        .bind(&row.title)
        .bind(&row.memo)
        .bind(row.due_at)
        .bind(row.done_at)
        .bind(row.remind)
        .bind(evening)
        .bind(hour)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row)
    }

    async fn list_assignments(
        &self,
        user_id: Uuid,
        request: &ListAssignmentsRequest,
        page: &Page,
    ) -> AppResult<ListAssignmentsResponse> {
        let core = &self.core;
        let at = self.now();
        let filter = request.filter.as_known().unwrap_or(AssignmentFilter::ASSIGNMENT_FILTER_UNSPECIFIED);
        let subject_id = parse_opt_id(&request.subject_id)?;
        let cursor: Option<Cursor> = page.decode()?;
        let with_open = filter != AssignmentFilter::ASSIGNMENT_FILTER_DONE && !cursor.is_some_and(|c| c.done);
        let with_done =
            matches!(filter, AssignmentFilter::ASSIGNMENT_FILTER_DONE | AssignmentFilter::ASSIGNMENT_FILTER_ALL);

        let mut rows: Vec<AssignmentRow> = Vec::new();
        if with_open {
            // Soonest (and overdue) first.
            let after = cursor.filter(|c| !c.done);
            rows = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {COLUMNS} FROM school_assignments
                 WHERE user_id = $1 AND done_at IS NULL AND ($2::uuid IS NULL OR subject_id = $2)
                   AND ($3::timestamptz IS NULL OR (due_at, id) > ($3, $4))
                 ORDER BY due_at, id LIMIT $5"
            )))
            .bind(user_id)
            .bind(subject_id)
            .bind(after.map(|c| c.at))
            .bind(after.map(|c| c.id))
            .bind(page.fetch())
            .fetch_all(&core.db)
            .await?;
        }
        if with_done && (rows.len() as i64) < page.fetch() {
            // Then the finished ones, most recently finished first.
            let after = cursor.filter(|c| c.done);
            let done: Vec<AssignmentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {COLUMNS} FROM school_assignments
                 WHERE user_id = $1 AND done_at IS NOT NULL AND ($2::uuid IS NULL OR subject_id = $2)
                   AND ($3::timestamptz IS NULL OR (done_at, id) < ($3, $4))
                 ORDER BY done_at DESC, id DESC LIMIT $5"
            )))
            .bind(user_id)
            .bind(subject_id)
            .bind(after.map(|c| c.at))
            .bind(after.map(|c| c.id))
            .bind(page.fetch() - rows.len() as i64)
            .fetch_all(&core.db)
            .await?;
            rows.extend(done);
        }
        let info: PageInfo = page.finish(&mut rows, |row| Cursor {
            done: row.done_at.is_some(),
            at: row.done_at.unwrap_or(row.due_at),
            id: row.id,
        });

        let (open_count, overdue_count): (i64, i64) = sqlx::query_as(
            "SELECT count(*), count(*) FILTER (WHERE due_at < $2) FROM school_assignments WHERE user_id = $1 AND done_at IS NULL",
        )
        .bind(user_id)
        .bind(at)
        .fetch_one(&core.db)
        .await?;
        let subjects =
            subjects::by_ids(core, &rows.iter().filter_map(|row| row.subject_id).collect::<Vec<_>>()).await?;
        let tz = local::time_zone(core, user_id).await?;
        Ok(ListAssignmentsResponse {
            assignments: rows.iter().map(|row| row.to_proto(&subjects, at, tz)).collect(),
            page: info.into(),
            open_count: open_count as i32,
            overdue_count: overdue_count as i32,
            ..Default::default()
        })
    }

    /// Recomputes the reminder moments of a user's open assignments, after
    /// they switched assignment reminders on or off.
    pub(crate) async fn reschedule_assignment_reminders(&self, user_id: Uuid, at: Time) -> AppResult<()> {
        let core = &self.core;
        let reminders_on = settings::of(core, user_id).await?.assignment_reminders;
        let tz = local::time_zone(core, user_id).await?;
        let rows: Vec<AssignmentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM school_assignments WHERE user_id = $1 AND done_at IS NULL"
        )))
        .bind(user_id)
        .fetch_all(&core.db)
        .await?;
        for row in rows {
            let (evening, hour) = self.reminders_for(&row, reminders_on, tz, at);
            if (evening, hour) != (row.remind_evening_at, row.remind_hour_at) {
                sqlx::query("UPDATE school_assignments SET remind_evening_at = $2, remind_hour_at = $3 WHERE id = $1 AND done_at IS NULL")
                    .bind(row.id)
                    .bind(evening)
                    .bind(hour)
                    .execute(&core.db)
                    .await?;
            }
        }
        Ok(())
    }

    /// Sends the assignment reminders whose moment has come. Returns how
    /// many were sent. Safe to run in several processes.
    pub async fn send_assignment_reminders(&self, at: Time) -> AppResult<usize> {
        let due: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM school_assignments
             WHERE done_at IS NULL AND (remind_evening_at <= $1 OR remind_hour_at <= $1)
             ORDER BY LEAST(remind_evening_at, remind_hour_at) LIMIT 500",
        )
        .bind(at)
        .fetch_all(&self.core.db)
        .await?;
        let mut sent = 0;
        for assignment_id in due {
            match self.assignment_reminder(assignment_id, at).await {
                Ok(true) => sent += 1,
                Ok(false) => {}
                Err(e) => tracing::error!(%assignment_id, error = %e, "assignment reminder failed"),
            }
        }
        Ok(sent)
    }

    async fn assignment_reminder(&self, assignment_id: Uuid, at: Time) -> AppResult<bool> {
        let mut tx = self.core.db.begin().await?;
        let row: Option<AssignmentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM school_assignments
             WHERE id = $1 AND done_at IS NULL AND (remind_evening_at <= $2 OR remind_hour_at <= $2) FOR UPDATE SKIP LOCKED"
        )))
        .bind(assignment_id)
        .bind(at)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else { return Ok(false) };
        // When both have come due (the worker was away), the later one says it all.
        let hour_due = row.remind_hour_at.is_some_and(|moment| moment <= at);
        let moment = if hour_due { row.remind_hour_at } else { row.remind_evening_at }.unwrap_or(at);
        let timely = at <= row.due_at && at - moment <= Duration::hours(MAX_LATENESS_HOURS);
        if timely {
            let tz = local::time_zone_on(&mut tx, row.user_id).await?;
            let due = row.due_at.with_timezone(&tz).format("%-H:%M").to_string();
            let (body, when) = if hour_due {
                (Localized::new(format!("あと1時間: {}", row.title), format!("Due in an hour: {}", row.title)), "hour")
            } else {
                (
                    Localized::new(
                        format!("明日{due}まで: {}", row.title),
                        format!("Due tomorrow at {due}: {}", row.title),
                    ),
                    "evening",
                )
            };
            let request = NotificationRequest::new(
                row.user_id,
                KIND_ASSIGNMENT_DUE,
                Localized::new("課題のしめきり", "Assignment due"),
                body,
                "/school/assignments",
            )
            .collapse(format!("{KIND_ASSIGNMENT_DUE}:{}", row.id))
            .data(serde_json::json!({ "assignment_id": row.id, "due_at": row.due_at, "reminder": when }));
            outbound::notify(&mut *tx, &request).await?;
        }
        // The one-hour reminder makes the evening one moot.
        sqlx::query(
            "UPDATE school_assignments SET remind_evening_at = NULL, remind_hour_at = CASE WHEN $2 THEN NULL ELSE remind_hour_at END WHERE id = $1",
        )
        .bind(row.id)
        .bind(hour_due)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(timely)
    }
}

impl AssignmentService for School {
    async fn list_assignments(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListAssignmentsRequest>,
    ) -> ServiceResult<ListAssignmentsResponse> {
        handle(async {
            let request = request.to_owned_message();
            let page = Page::from_request(request.page.as_option(), 100);
            ok(self.list_assignments(ctx.user_id()?, &request, &page).await?)
        })
        .await
    }

    async fn create_assignment(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateAssignmentRequest>,
    ) -> ServiceResult<CreateAssignmentResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_writes(user_id).await?;
            let row = self.create_assignment(user_id, &request.to_owned_message()).await?;
            ok(CreateAssignmentResponse {
                assignment: self.assignment_proto(&row, self.now()).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_assignment(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateAssignmentRequest>,
    ) -> ServiceResult<UpdateAssignmentResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_writes(user_id).await?;
            let row = self.update_assignment(user_id, &request.to_owned_message()).await?;
            ok(UpdateAssignmentResponse {
                assignment: self.assignment_proto(&row, self.now()).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_assignment(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteAssignmentRequest>,
    ) -> ServiceResult<DeleteAssignmentResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let deleted = sqlx::query("DELETE FROM school_assignments WHERE id = $1 AND user_id = $2")
                .bind(parse_id(request.assignment_id)?)
                .bind(user_id)
                .execute(&self.core.db)
                .await?;
            ensure!(deleted.rows_affected() > 0, not_found, "ASSIGNMENT_NOT_FOUND", "assignment not found");
            ok(DeleteAssignmentResponse::default())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    const TOKYO: Tz = chrono_tz::Asia::Tokyo;

    fn tokyo(day: u32, hour: u32, minute: u32) -> Time {
        TOKYO.with_ymd_and_hms(2026, 6, day, hour, minute, 0).unwrap().to_utc()
    }

    #[test]
    fn reminders_come_the_evening_before_and_an_hour_before() {
        // Due Wednesday the 10th at 9:00, saved on Monday.
        let (evening, hour) = reminder_times(tokyo(10, 9, 0), TOKYO, 19, tokyo(8, 12, 0));
        assert_eq!(evening, Some(tokyo(9, 19, 0)));
        assert_eq!(hour, Some(tokyo(10, 8, 0)));

        // Saved after the evening before: only the one-hour reminder is left.
        let (evening, hour) = reminder_times(tokyo(10, 9, 0), TOKYO, 19, tokyo(9, 22, 0));
        assert_eq!((evening, hour), (None, Some(tokyo(10, 8, 0))));

        // Saved within the last hour: nothing to remind of.
        assert_eq!(reminder_times(tokyo(10, 9, 0), TOKYO, 19, tokyo(10, 8, 30)), (None, None));

        // Due just after midnight: the evening before is the evening of the previous day.
        let (evening, hour) = reminder_times(tokyo(10, 0, 30), TOKYO, 19, tokyo(8, 12, 0));
        assert_eq!((evening, hour), (Some(tokyo(9, 19, 0)), Some(tokyo(9, 23, 30))));

        // The day before is the user's, not UTC's: due at 8:00 in Tokyo is the day before in UTC.
        let (evening, _) = reminder_times(tokyo(10, 8, 0), chrono_tz::America::Los_Angeles, 19, tokyo(8, 12, 0));
        // 8:00 JST on the 10th is 16:00 PDT on the 9th; the evening before is 19:00 PDT on the 8th.
        assert_eq!(
            evening,
            Some(chrono_tz::America::Los_Angeles.with_ymd_and_hms(2026, 6, 8, 19, 0, 0).unwrap().to_utc())
        );
    }

    #[test]
    fn urgency_follows_the_callers_calendar() {
        use AssignmentUrgency::*;
        let at = tokyo(8, 22, 0);
        assert_eq!(urgency(tokyo(8, 21, 0), false, at, TOKYO), ASSIGNMENT_URGENCY_OVERDUE);
        assert_eq!(urgency(tokyo(8, 23, 30), false, at, TOKYO), ASSIGNMENT_URGENCY_TODAY);
        // Two and a half hours later, but tomorrow.
        assert_eq!(urgency(tokyo(9, 0, 30), false, at, TOKYO), ASSIGNMENT_URGENCY_TOMORROW);
        assert_eq!(urgency(tokyo(15, 9, 0), false, at, TOKYO), ASSIGNMENT_URGENCY_THIS_WEEK);
        assert_eq!(urgency(tokyo(16, 9, 0), false, at, TOKYO), ASSIGNMENT_URGENCY_LATER);
        assert_eq!(urgency(tokyo(8, 21, 0), true, at, TOKYO), ASSIGNMENT_URGENCY_DONE);
        // The same instant is still "today" for someone in Honolulu.
        assert_eq!(urgency(tokyo(9, 0, 30), false, at, chrono_tz::Pacific::Honolulu), ASSIGNMENT_URGENCY_TODAY);
    }
}
