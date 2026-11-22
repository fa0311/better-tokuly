//! Exam periods with their subjects, countdowns and private results, and
//! `ExamService`.
use std::collections::HashMap;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use sqlx::PgConnection;
use tokuly_core::prelude::*;
use tokuly_proto::{common::Timestamps, school::*};

use crate::{
    School, local,
    subjects::{self, SubjectRow},
    text,
};

const MAX_EXAMS: i64 = 100;
const MAX_EXAM_DAYS: i64 = 60;
const MAX_EXAM_SUBJECTS: i64 = 40;
const MAX_SCORE: i32 = 10_000;
/// How long before its first day an exam shows on the profile.
pub(crate) const PROFILE_LEAD_DAYS: i64 = 14;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ExamRow {
    pub id: Uuid,
    pub name: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub show_on_profile: bool,
    pub memo: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const EXAM_COLUMNS: &str = "id, name, start_date, end_date, show_on_profile, memo, created_at, updated_at";

#[derive(Debug, Clone, sqlx::FromRow)]
struct ExamSubjectRow {
    id: Uuid,
    exam_id: Uuid,
    subject_id: Uuid,
    date: Option<NaiveDate>,
    period: i16,
    scope: String,
    has_result: bool,
    score: Option<i32>,
    max_score: i32,
    average: Option<f64>,
    rank: Option<i32>,
    result_memo: String,
}

const EXAM_SUBJECT_COLUMNS: &str =
    "id, exam_id, subject_id, date, period, scope, has_result, score, max_score, average, rank, result_memo";

/// Where an exam stands on `today`: its status, the days until it starts and
/// the days left until it is over (today included).
pub(crate) fn countdown(start: NaiveDate, end: NaiveDate, today: NaiveDate) -> (ExamStatus, i32, i32) {
    let left = ((end - today).num_days() + 1) as i32;
    if today < start {
        (ExamStatus::EXAM_STATUS_UPCOMING, (start - today).num_days() as i32, left)
    } else if today <= end {
        (ExamStatus::EXAM_STATUS_ONGOING, 0, left)
    } else {
        (ExamStatus::EXAM_STATUS_FINISHED, 0, 0)
    }
}

fn exam_not_found() -> AppError {
    AppError::not_found("EXAM_NOT_FOUND", "exam not found")
}

fn exam_subject_not_found() -> AppError {
    AppError::not_found("EXAM_SUBJECT_NOT_FOUND", "exam subject not found")
}

fn check_dates(start: NaiveDate, end: NaiveDate) -> AppResult<()> {
    ensure!(end >= start, invalid, "DATE_RANGE_INVALID", "an exam cannot end before it starts");
    ensure!(
        (end - start).num_days() < MAX_EXAM_DAYS,
        invalid,
        "DATE_RANGE_INVALID",
        "an exam period is at most {MAX_EXAM_DAYS} days long"
    );
    Ok(())
}

impl ExamSubjectRow {
    fn to_proto(&self, subjects: &HashMap<Uuid, SubjectRow>) -> ExamSubject {
        let result = self.has_result.then(|| ExamResult {
            score: self.score,
            max_score: self.max_score,
            average: self.average,
            rank: self.rank,
            memo: self.result_memo.clone(),
            ..Default::default()
        });
        ExamSubject {
            id: self.id.to_string(),
            exam_id: self.exam_id.to_string(),
            subject: subjects.get(&self.subject_id).map(SubjectRow::to_proto).into(),
            date: local::fmt_opt_date(self.date),
            period: self.period as i32,
            scope: self.scope.clone(),
            result: result.into(),
            ..Default::default()
        }
    }
}

impl ExamRow {
    fn to_proto(&self, subjects: Vec<ExamSubject>, today: NaiveDate) -> Exam {
        let (status, days_until_start, days_left) = countdown(self.start_date, self.end_date, today);
        Exam {
            id: self.id.to_string(),
            name: self.name.clone(),
            start_date: local::fmt_date(self.start_date),
            end_date: local::fmt_date(self.end_date),
            status: status.into(),
            days_until_start,
            days_left,
            show_on_profile: self.show_on_profile,
            memo: self.memo.clone(),
            subjects,
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

async fn own_exam(conn: &mut PgConnection, user_id: Uuid, exam_id: Uuid) -> AppResult<ExamRow> {
    let row: Option<ExamRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {EXAM_COLUMNS} FROM school_exams WHERE id = $1 AND user_id = $2 FOR UPDATE"
    )))
    .bind(exam_id)
    .bind(user_id)
    .fetch_optional(conn)
    .await?;
    row.ok_or_else(exam_not_found)
}

/// Turns a unique violation on (exam, subject) into the documented reason.
fn subject_exists(error: sqlx::Error) -> AppError {
    match &error {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            AppError::already_exists("EXAM_SUBJECT_EXISTS", "the exam already has this subject")
        }
        _ => error.into(),
    }
}

impl School {
    async fn today_of(&self, user_id: Uuid) -> AppResult<NaiveDate> {
        Ok(local::local_date(self.now(), local::time_zone(&self.core, user_id).await?))
    }

    /// Exams as the API returns them, each with its subjects.
    async fn exam_protos(&self, user_id: Uuid, exams: &[ExamRow]) -> AppResult<Vec<Exam>> {
        let core = &self.core;
        let today = self.today_of(user_id).await?;
        let ids: Vec<Uuid> = exams.iter().map(|exam| exam.id).collect();
        let rows: Vec<ExamSubjectRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {EXAM_SUBJECT_COLUMNS} FROM school_exam_subjects WHERE exam_id = ANY($1) ORDER BY date NULLS LAST, period, id"
        )))
        .bind(&ids)
        .fetch_all(&core.db)
        .await?;
        let subjects = if rows.is_empty() { HashMap::new() } else { subjects::map_of(core, user_id).await? };
        Ok(exams
            .iter()
            .map(|exam| {
                exam.to_proto(
                    rows.iter().filter(|row| row.exam_id == exam.id).map(|row| row.to_proto(&subjects)).collect(),
                    today,
                )
            })
            .collect())
    }

    async fn exam_proto(&self, user_id: Uuid, exam: &ExamRow) -> AppResult<Exam> {
        Ok(self.exam_protos(user_id, std::slice::from_ref(exam)).await?.pop().expect("one exam in, one exam out"))
    }

    async fn exam_subject_proto(&self, user_id: Uuid, row: &ExamSubjectRow) -> AppResult<ExamSubject> {
        Ok(row.to_proto(&subjects::map_of(&self.core, user_id).await?))
    }

    async fn list_exams(&self, user_id: Uuid) -> AppResult<Vec<Exam>> {
        let today = self.today_of(user_id).await?;
        // Upcoming and ongoing ones first, soonest first; then the finished
        // ones, the latest first.
        let rows: Vec<ExamRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {EXAM_COLUMNS} FROM school_exams WHERE user_id = $1
             ORDER BY (end_date < $2), CASE WHEN end_date < $2 THEN $2 - end_date ELSE start_date - $2 END, id"
        )))
        .bind(user_id)
        .bind(today)
        .fetch_all(&self.core.db)
        .await?;
        self.exam_protos(user_id, &rows).await
    }

    async fn create_exam(&self, user_id: Uuid, request: &CreateExamRequest) -> AppResult<ExamRow> {
        let name = text::name(&request.name, text::MAX_NAME, "EXAM_NAME_INVALID", "exam name")?;
        let start_date = local::parse_date(&request.start_date)?;
        let end_date = local::parse_opt_date(&request.end_date)?.unwrap_or(start_date);
        check_dates(start_date, end_date)?;
        let memo = text::memo(&request.memo, "memo")?;
        let mut tx = self.core.db.begin().await?;
        crate::lock_user(&mut tx, "exams", user_id).await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM school_exams WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(count < MAX_EXAMS, precondition, "EXAM_LIMIT_REACHED", "an account has at most {MAX_EXAMS} exams");
        let row: ExamRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO school_exams (user_id, name, start_date, end_date, show_on_profile, memo) VALUES ($1, $2, $3, $4, $5, $6) RETURNING {EXAM_COLUMNS}"
        )))
        .bind(user_id)
        .bind(&name)
        .bind(start_date)
        .bind(end_date)
        .bind(request.show_on_profile)
        .bind(&memo)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row)
    }

    async fn update_exam(&self, user_id: Uuid, request: &UpdateExamRequest) -> AppResult<ExamRow> {
        let exam_id = parse_id(&request.exam_id)?;
        let mut tx = self.core.db.begin().await?;
        let mut exam = own_exam(&mut tx, user_id, exam_id).await?;
        if let Some(name) = &request.name {
            exam.name = text::name(name, text::MAX_NAME, "EXAM_NAME_INVALID", "exam name")?;
        }
        if let Some(date) = &request.start_date {
            exam.start_date = local::parse_date(date)?;
        }
        if let Some(date) = &request.end_date {
            exam.end_date = local::parse_date(date)?;
        }
        check_dates(exam.start_date, exam.end_date)?;
        if let Some(show) = request.show_on_profile {
            exam.show_on_profile = show;
        }
        if let Some(memo) = &request.memo {
            exam.memo = text::memo(memo, "memo")?;
        }
        // Tests keep their day only while the exam still covers it.
        let outside: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM school_exam_subjects WHERE exam_id = $1 AND date NOT BETWEEN $2 AND $3)",
        )
        .bind(exam_id)
        .bind(exam.start_date)
        .bind(exam.end_date)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(!outside, invalid, "DATE_OUTSIDE_EXAM", "a subject of this exam has its test outside the new dates");
        let row: ExamRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE school_exams SET name = $2, start_date = $3, end_date = $4, show_on_profile = $5, memo = $6 WHERE id = $1 RETURNING {EXAM_COLUMNS}"
        )))
        .bind(exam_id)
        .bind(&exam.name)
        .bind(exam.start_date)
        .bind(exam.end_date)
        .bind(exam.show_on_profile)
        .bind(&exam.memo)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row)
    }

    async fn put_exam_subject(&self, user_id: Uuid, request: &PutExamSubjectRequest) -> AppResult<ExamSubjectRow> {
        let exam_id = parse_id(&request.exam_id)?;
        let exam_subject_id = parse_opt_id(&request.exam_subject_id)?;
        let subject_id = parse_opt_id(&request.subject_id)?;
        let date = local::parse_opt_date(&request.date)?;
        ensure!(
            (0..=12).contains(&request.period),
            invalid,
            "PERIOD_INVALID",
            "period must be 1-12, or 0 when it is not known"
        );
        let scope = text::memo(&request.scope, "scope")?;

        let mut tx = self.core.db.begin().await?;
        let exam = own_exam(&mut tx, user_id, exam_id).await?;
        ensure!(
            date.is_none_or(|date| exam.start_date <= date && date <= exam.end_date),
            invalid,
            "DATE_OUTSIDE_EXAM",
            "the test is outside the exam's dates"
        );
        if let Some(subject_id) = subject_id {
            subjects::require(&mut tx, user_id, subject_id).await?;
        }
        let row: ExamSubjectRow = match exam_subject_id {
            Some(id) => {
                let updated: Option<ExamSubjectRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                    "UPDATE school_exam_subjects SET subject_id = COALESCE($3, subject_id), date = $4, period = $5, scope = $6
                     WHERE id = $1 AND exam_id = $2 RETURNING {EXAM_SUBJECT_COLUMNS}"
                )))
                .bind(id)
                .bind(exam_id)
                .bind(subject_id)
                .bind(date)
                .bind(request.period as i16)
                .bind(&scope)
                .fetch_optional(&mut *tx)
                .await
                .map_err(subject_exists)?;
                updated.ok_or_else(exam_subject_not_found)?
            }
            None => {
                let subject_id = subject_id
                    .ok_or_else(|| AppError::not_found("SUBJECT_NOT_FOUND", "say which subject the test is in"))?;
                let count: i64 = sqlx::query_scalar("SELECT count(*) FROM school_exam_subjects WHERE exam_id = $1")
                    .bind(exam_id)
                    .fetch_one(&mut *tx)
                    .await?;
                ensure!(
                    count < MAX_EXAM_SUBJECTS,
                    precondition,
                    "EXAM_SUBJECT_LIMIT_REACHED",
                    "an exam has at most {MAX_EXAM_SUBJECTS} subjects"
                );
                sqlx::query_as(sqlx::AssertSqlSafe(format!(
                    "INSERT INTO school_exam_subjects (exam_id, user_id, subject_id, date, period, scope)
                     VALUES ($1, $2, $3, $4, $5, $6) RETURNING {EXAM_SUBJECT_COLUMNS}"
                )))
                .bind(exam_id)
                .bind(user_id)
                .bind(subject_id)
                .bind(date)
                .bind(request.period as i16)
                .bind(&scope)
                .fetch_one(&mut *tx)
                .await
                .map_err(subject_exists)?
            }
        };
        tx.commit().await?;
        Ok(row)
    }

    async fn set_exam_result(
        &self,
        user_id: Uuid,
        exam_subject_id: Uuid,
        result: Option<&ExamResult>,
    ) -> AppResult<ExamSubjectRow> {
        let invalid = |message: &str| AppError::invalid("RESULT_INVALID", message.to_owned());
        let (has_result, score, max_score, average, rank, memo) = match result {
            None => (false, None, 100, None, None, String::new()),
            Some(result) => {
                let max_score = if result.max_score == 0 { 100 } else { result.max_score };
                if !(1..=MAX_SCORE).contains(&max_score) {
                    return Err(invalid("max_score must be between 1 and 10000"));
                }
                if result.score.is_some_and(|score| !(0..=max_score).contains(&score)) {
                    return Err(invalid("the score must be between 0 and the maximum"));
                }
                if result
                    .average
                    .is_some_and(|average| !average.is_finite() || average < 0.0 || average > max_score as f64)
                {
                    return Err(invalid("the average must be between 0 and the maximum"));
                }
                if result.rank.is_some_and(|rank| !(1..=1_000_000).contains(&rank)) {
                    return Err(invalid("the rank must be positive"));
                }
                (true, result.score, max_score, result.average, result.rank, text::memo(&result.memo, "memo")?)
            }
        };
        let row: Option<ExamSubjectRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE school_exam_subjects SET has_result = $3, score = $4, max_score = $5, average = $6, rank = $7, result_memo = $8
             WHERE id = $1 AND user_id = $2 RETURNING {EXAM_SUBJECT_COLUMNS}"
        )))
        .bind(exam_subject_id)
        .bind(user_id)
        .bind(has_result)
        .bind(score)
        .bind(max_score)
        .bind(average)
        .bind(rank)
        .bind(&memo)
        .fetch_optional(&self.core.db)
        .await?;
        row.ok_or_else(exam_subject_not_found)
    }

    /// The exam `owner` shows on their profile around `today`: one that is
    /// on, or starts within two weeks — the soonest of them.
    pub(crate) async fn profile_exam(&self, owner: Uuid, today: NaiveDate) -> AppResult<Option<ExamRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {EXAM_COLUMNS} FROM school_exams
             WHERE user_id = $1 AND show_on_profile AND end_date >= $2 AND start_date <= $3
             ORDER BY start_date, id LIMIT 1"
        )))
        .bind(owner)
        .bind(today)
        .bind(today + Duration::days(PROFILE_LEAD_DAYS))
        .fetch_optional(&self.core.db)
        .await?)
    }
}

impl ExamService for School {
    async fn list_exams(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListExamsRequest>,
    ) -> ServiceResult<ListExamsResponse> {
        handle(async {
            let exams = self.list_exams(ctx.user_id()?).await?;
            ok(ListExamsResponse { exams, ..Default::default() })
        })
        .await
    }

    async fn get_exam(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetExamRequest>,
    ) -> ServiceResult<GetExamResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let exam: Option<ExamRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {EXAM_COLUMNS} FROM school_exams WHERE id = $1 AND user_id = $2"
            )))
            .bind(parse_id(request.exam_id)?)
            .bind(user_id)
            .fetch_optional(&self.core.db)
            .await?;
            let exam = exam.ok_or_else(exam_not_found)?;
            ok(GetExamResponse { exam: self.exam_proto(user_id, &exam).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn create_exam(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateExamRequest>,
    ) -> ServiceResult<CreateExamResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_writes(user_id).await?;
            let exam = self.create_exam(user_id, &request.to_owned_message()).await?;
            ok(CreateExamResponse { exam: self.exam_proto(user_id, &exam).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn update_exam(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateExamRequest>,
    ) -> ServiceResult<UpdateExamResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_writes(user_id).await?;
            let exam = self.update_exam(user_id, &request.to_owned_message()).await?;
            ok(UpdateExamResponse { exam: self.exam_proto(user_id, &exam).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_exam(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteExamRequest>,
    ) -> ServiceResult<DeleteExamResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let deleted = sqlx::query("DELETE FROM school_exams WHERE id = $1 AND user_id = $2")
                .bind(parse_id(request.exam_id)?)
                .bind(user_id)
                .execute(&self.core.db)
                .await?;
            ensure!(deleted.rows_affected() > 0, not_found, "EXAM_NOT_FOUND", "exam not found");
            ok(DeleteExamResponse::default())
        })
        .await
    }

    async fn put_exam_subject(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, PutExamSubjectRequest>,
    ) -> ServiceResult<PutExamSubjectResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_writes(user_id).await?;
            let row = self.put_exam_subject(user_id, &request.to_owned_message()).await?;
            ok(PutExamSubjectResponse {
                exam_subject: self.exam_subject_proto(user_id, &row).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_exam_subject(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteExamSubjectRequest>,
    ) -> ServiceResult<DeleteExamSubjectResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let deleted = sqlx::query("DELETE FROM school_exam_subjects WHERE id = $1 AND user_id = $2")
                .bind(parse_id(request.exam_subject_id)?)
                .bind(user_id)
                .execute(&self.core.db)
                .await?;
            ensure!(deleted.rows_affected() > 0, not_found, "EXAM_SUBJECT_NOT_FOUND", "exam subject not found");
            ok(DeleteExamSubjectResponse::default())
        })
        .await
    }

    async fn set_exam_result(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetExamResultRequest>,
    ) -> ServiceResult<SetExamResultResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_writes(user_id).await?;
            let request = request.to_owned_message();
            let row =
                self.set_exam_result(user_id, parse_id(&request.exam_subject_id)?, request.result.as_option()).await?;
            ok(SetExamResultResponse {
                exam_subject: self.exam_subject_proto(user_id, &row).await?.into(),
                ..Default::default()
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }

    #[test]
    fn countdowns_count_days_before_during_and_after() {
        let (start, end) = (date(7, 1), date(7, 3));
        assert_eq!(countdown(start, end, date(6, 28)), (ExamStatus::EXAM_STATUS_UPCOMING, 3, 6));
        assert_eq!(countdown(start, end, date(6, 30)), (ExamStatus::EXAM_STATUS_UPCOMING, 1, 4));
        assert_eq!(countdown(start, end, date(7, 1)), (ExamStatus::EXAM_STATUS_ONGOING, 0, 3));
        assert_eq!(countdown(start, end, date(7, 3)), (ExamStatus::EXAM_STATUS_ONGOING, 0, 1));
        assert_eq!(countdown(start, end, date(7, 4)), (ExamStatus::EXAM_STATUS_FINISHED, 0, 0));
        assert!(check_dates(date(7, 1), date(8, 29)).is_ok());
        assert_eq!(check_dates(date(7, 1), date(8, 30)).unwrap_err().reason, "DATE_RANGE_INVALID");
        assert_eq!(check_dates(date(7, 2), date(7, 1)).unwrap_err().reason, "DATE_RANGE_INVALID");
    }
}
