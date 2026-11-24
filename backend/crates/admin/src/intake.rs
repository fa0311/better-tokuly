//! Report intake: turns `engagement.report.created` events into the
//! moderation queue.
use tokuly_core::{events::Event, prelude::*};

use crate::{Admin, cases::normalize_category, events::ReportCreated, notices::truncate_chars};

/// Stored lengths are bounded here because the payload comes from another
/// domain whose own limits may change.
const REPORT_TEXT_MAX_CHARS: usize = 2000;
const SUBJECT_TITLE_MAX_CHARS: usize = 300;
const SUBJECT_PATH_MAX_CHARS: usize = 500;

impl Admin {
    /// Files a report into the open case of its subject, opening one when
    /// there is none. Idempotent on the report ID, so redelivered events do
    /// not inflate the report count.
    pub(crate) async fn on_report_created(&self, event: &Event) -> AppResult<()> {
        let report: ReportCreated = event.data()?;
        let category = normalize_category(&report.category);
        let title = truncate_chars(report.subject_title.trim(), SUBJECT_TITLE_MAX_CHARS);
        let path = truncate_chars(report.subject_path.trim(), SUBJECT_PATH_MAX_CHARS);

        // Content that hides its author is reported without an owner; whoever
        // keeps it says who answers for it. Asked before the transaction, so
        // this handler never waits for a second connection while holding one.
        let (owner_id, owner_hidden) = match report.subject_owner_id.filter(|owner_id| !owner_id.is_nil()) {
            Some(owner_id) => (Some(owner_id), false),
            None => {
                let hidden = self.hidden_owner(report.subject_type, report.subject_id).await?;
                (hidden, hidden.is_some())
            }
        };

        let mut tx = self.core.db.begin().await?;
        let known: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM moderation_reports WHERE id = $1)").bind(report.report_id).fetch_one(&mut *tx).await?;
        if known {
            return Ok(());
        }
        // Owner and reporter are looked up rather than trusted, so an event
        // about an account that has since been erased still files cleanly.
        let case_id: Uuid = sqlx::query_scalar(
            "INSERT INTO moderation_cases (subject_type, subject_id, subject_owner_id, subject_title, subject_path, report_count, categories, last_reported_at, owner_hidden)
             VALUES ($1, $2, (SELECT id FROM users WHERE id = $3 AND deleted_at IS NULL), $4, $5, 1, ARRAY[$6], $7, $8)
             ON CONFLICT (subject_type, subject_id) WHERE status = 'open' DO UPDATE SET
                 report_count     = moderation_cases.report_count + 1,
                 categories       = CASE WHEN $6 = ANY(moderation_cases.categories) THEN moderation_cases.categories
                                         ELSE array_append(moderation_cases.categories, $6) END,
                 last_reported_at = greatest(moderation_cases.last_reported_at, EXCLUDED.last_reported_at),
                 subject_owner_id = COALESCE(EXCLUDED.subject_owner_id, moderation_cases.subject_owner_id),
                 owner_hidden     = moderation_cases.owner_hidden OR EXCLUDED.owner_hidden,
                 subject_title    = CASE WHEN EXCLUDED.subject_title <> '' THEN EXCLUDED.subject_title ELSE moderation_cases.subject_title END,
                 subject_path     = CASE WHEN EXCLUDED.subject_path <> '' THEN EXCLUDED.subject_path ELSE moderation_cases.subject_path END
             RETURNING id",
        )
        .bind(report.subject_type)
        .bind(report.subject_id)
        .bind(owner_id)
        .bind(&title)
        .bind(&path)
        .bind(category)
        .bind(event.occurred_at)
        .bind(owner_hidden)
        .fetch_one(&mut *tx)
        .await?;
        let filed = sqlx::query(
            "INSERT INTO moderation_reports (id, case_id, reporter_id, category, text, created_at)
             VALUES ($1, $2, (SELECT id FROM users WHERE id = $3 AND deleted_at IS NULL), $4, $5, $6)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(report.report_id)
        .bind(case_id)
        .bind(report.reporter_id)
        .bind(category)
        .bind(truncate_chars(report.text.trim(), REPORT_TEXT_MAX_CHARS))
        .bind(event.occurred_at)
        .execute(&mut *tx)
        .await?;
        if filed.rows_affected() == 0 {
            // A concurrent delivery of the same event won the race; dropping
            // the transaction undoes our increment of the report count.
            return Ok(());
        }
        // From now on an unsend or an edit does not take the uploads with it.
        self.hold_evidence(&mut tx, case_id, report.subject_type, report.subject_id).await?;
        tx.commit().await?;
        Ok(())
    }
}
