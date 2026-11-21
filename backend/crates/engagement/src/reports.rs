//! Reports: a user tells the moderation team that something is wrong.
//!
//! This domain takes the report, keeps the reporter's copy with its status,
//! and hands it to the admin domain as `engagement.report.created`. A user
//! has one open report per subject (a partial unique index says so); an open
//! report that has gone unanswered for `report_repeat_days` is superseded by
//! the next one instead of blocking it.
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokuly_core::{analytics::AnalyticsEvent, events, prelude::*};
use tokuly_proto::{
    common::PageInfo,
    engagement::{Report, ReportCategory, ReportStatus},
};

use crate::{
    Engagement,
    events::{REPORT_CATEGORIES, REPORT_CREATED, ReportCreated},
    subjects::{Subject, SubjectKey},
    text,
};

const SUBMITTED: i32 = ReportStatus::REPORT_STATUS_SUBMITTED as i32;
const ACTION_TAKEN: i32 = ReportStatus::REPORT_STATUS_ACTION_TAKEN as i32;
const NO_ACTION: i32 = ReportStatus::REPORT_STATUS_NO_ACTION as i32;
const SUPERSEDED: i32 = ReportStatus::REPORT_STATUS_SUPERSEDED as i32;

const _: () = {
    // The migration's partial indexes are on `status = 1`.
    assert!(SUBMITTED == 1 && SUPERSEDED == 4);
    assert!(REPORT_CATEGORIES.len() == ReportCategory::REPORT_CATEGORY_OTHER as usize);
};

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ReportRow {
    pub id: Uuid,
    pub subject_type: i32,
    pub subject_id: Uuid,
    pub subject_title: String,
    pub subject_path: String,
    pub category: i32,
    pub text: String,
    pub status: i32,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

const REPORT_COLUMNS: &str =
    "id, subject_type, subject_id, subject_title, subject_path, category, text, status, created_at, resolved_at";

impl ReportRow {
    pub(crate) fn to_proto(&self) -> Report {
        Report {
            id: self.id.to_string(),
            subject: SubjectKey { kind: self.subject_type, id: self.subject_id }.to_ref().into(),
            subject_title: self.subject_title.clone(),
            subject_path: self.subject_path.clone(),
            category: self.category.into(),
            text: self.text.clone(),
            status: self.status.into(),
            created_at: ts(self.created_at),
            resolved_at: opt_ts(self.resolved_at),
            ..Default::default()
        }
    }
}

/// The category of a request; `REPORT_CATEGORY_INVALID` when unset or unknown.
pub(crate) fn requested_category(value: buffa::EnumValue<ReportCategory>) -> AppResult<ReportCategory> {
    match value.as_known() {
        Some(category) if category != ReportCategory::REPORT_CATEGORY_UNSPECIFIED => Ok(category),
        _ => Err(AppError::invalid("REPORT_CATEGORY_INVALID", "choose what is wrong with it")),
    }
}

/// The category as the admin domain reads it: `SPAM`, `HARASSMENT`, ...
pub(crate) fn category_name(category: ReportCategory) -> &'static str {
    REPORT_CATEGORIES.get((category as usize).wrapping_sub(1)).copied().unwrap_or("OTHER")
}

impl Engagement {
    /// Files a report (`ReportService.CreateReport`). `subject` was
    /// resolved without regard to blocks.
    pub(crate) async fn create_report(
        &self,
        reporter_id: Uuid,
        subject: &Subject,
        category: ReportCategory,
        text: &str,
    ) -> AppResult<ReportRow> {
        let text = text::report_text(text)?;
        ensure!(
            !subject.is_owned_by(reporter_id),
            precondition,
            "CANNOT_REPORT_OWN",
            "you cannot report yourself or your own content"
        );
        self.core
            .cache
            .rate_limit(
                &format!("engagement:report:{reporter_id}"),
                self.cfg.reports_per_hour,
                Duration::from_secs(3600),
            )
            .await?;

        let key = subject.key;
        let mut tx = self.core.db.begin().await?;
        // An open report nobody answered for long enough gives way.
        sqlx::query(
            "UPDATE reports SET status = $4, resolved_at = now()
             WHERE reporter_id = $1 AND subject_type = $2 AND subject_id = $3 AND status = $5
               AND created_at < now() - make_interval(days => $6)",
        )
        .bind(reporter_id)
        .bind(key.kind)
        .bind(key.id)
        .bind(SUPERSEDED)
        .bind(SUBMITTED)
        .bind(self.cfg.report_repeat_days.max(0))
        .execute(&mut *tx)
        .await?;
        let report: Option<ReportRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO reports (reporter_id, subject_type, subject_id, subject_owner_id, subject_title, subject_path, category, text)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (reporter_id, subject_type, subject_id) WHERE status = 1 DO NOTHING
             RETURNING {REPORT_COLUMNS}"
        )))
        .bind(reporter_id)
        .bind(key.kind)
        .bind(key.id)
        .bind(subject.owner())
        .bind(&subject.info.title)
        .bind(&subject.info.path)
        .bind(category as i32)
        .bind(&text)
        .fetch_optional(&mut *tx)
        .await?;
        let report = report.ok_or_else(|| {
            AppError::already_exists("ALREADY_REPORTED", "you already reported this; the team is looking into it")
        })?;
        let event = ReportCreated {
            report_id: report.id,
            reporter_id,
            subject_type: key.kind,
            subject_id: key.id,
            subject_owner_id: subject.owner(),
            category: category_name(category).to_owned(),
            text,
            subject_title: subject.info.title.clone(),
            subject_path: subject.info.path.clone(),
        };
        events::emit(&mut *tx, REPORT_CREATED, Some(reporter_id), &event).await?;
        tx.commit().await?;

        let mut tracked = AnalyticsEvent::new("engagement.report")
            .user(Some(reporter_id))
            .subject("subject", key.id)
            .props(serde_json::json!({ "subject_type": key.kind, "category": event.category }));
        if let Some(owner_id) = subject.owner() {
            tracked = tracked.owner(owner_id);
        }
        self.core.analytics.track(tracked).await;
        Ok(report)
    }

    /// The caller's reports, newest first (`ReportService.ListMyReports`).
    pub(crate) async fn reports_by(&self, reporter_id: Uuid, page: &Page) -> AppResult<(Vec<ReportRow>, PageInfo)> {
        let after: Option<Uuid> = page.decode()?;
        let mut rows: Vec<ReportRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {REPORT_COLUMNS} FROM reports WHERE reporter_id = $1 AND ($2::uuid IS NULL OR id < $2) ORDER BY id DESC LIMIT $3"
        )))
        .bind(reporter_id)
        .bind(after)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| row.id);
        Ok((rows, info))
    }

    /// The moderation team decided about these reports.
    pub(crate) async fn resolve_reports(&self, report_ids: &[Uuid], action_taken: bool) -> AppResult<u64> {
        let resolved =
            sqlx::query("UPDATE reports SET status = $2, resolved_at = now() WHERE id = ANY($1) AND status = $3")
                .bind(report_ids)
                .bind(if action_taken { ACTION_TAKEN } else { NO_ACTION })
                .bind(SUBMITTED)
                .execute(&self.core.db)
                .await?;
        Ok(resolved.rows_affected())
    }
}

/// The content was taken down: every open report about it led to action.
pub(crate) async fn resolve_for_subject(conn: &mut sqlx::PgConnection, key: SubjectKey) -> AppResult<u64> {
    let resolved = sqlx::query(
        "UPDATE reports SET status = $3, resolved_at = now() WHERE subject_type = $1 AND subject_id = $2 AND status = $4",
    )
    .bind(key.kind)
    .bind(key.id)
    .bind(ACTION_TAKEN)
    .bind(SUBMITTED)
    .execute(conn)
    .await?;
    Ok(resolved.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_travel_under_the_names_the_admin_domain_knows() {
        let names: Vec<&str> = [
            ReportCategory::REPORT_CATEGORY_SPAM,
            ReportCategory::REPORT_CATEGORY_HARASSMENT,
            ReportCategory::REPORT_CATEGORY_HATE,
            ReportCategory::REPORT_CATEGORY_SEXUAL,
            ReportCategory::REPORT_CATEGORY_VIOLENCE,
            ReportCategory::REPORT_CATEGORY_SELF_HARM,
            ReportCategory::REPORT_CATEGORY_ILLEGAL,
            ReportCategory::REPORT_CATEGORY_IMPERSONATION,
            ReportCategory::REPORT_CATEGORY_COPYRIGHT,
            ReportCategory::REPORT_CATEGORY_OTHER,
        ]
        .into_iter()
        .map(category_name)
        .collect();
        assert_eq!(names, REPORT_CATEGORIES);
        assert_eq!(category_name(ReportCategory::REPORT_CATEGORY_UNSPECIFIED), "OTHER");
    }

    #[test]
    fn a_report_needs_a_known_category() {
        assert_eq!(
            requested_category(ReportCategory::REPORT_CATEGORY_HATE.into()).unwrap(),
            ReportCategory::REPORT_CATEGORY_HATE
        );
        assert_eq!(
            requested_category(ReportCategory::REPORT_CATEGORY_UNSPECIFIED.into()).unwrap_err().reason,
            "REPORT_CATEGORY_INVALID"
        );
        assert_eq!(requested_category(buffa::EnumValue::from(77)).unwrap_err().reason, "REPORT_CATEGORY_INVALID");
    }
}
