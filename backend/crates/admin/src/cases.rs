//! Moderation cases: rows, queue queries and conversion to API messages.
//!
//! A case groups every report about one subject. At most one case per subject
//! is open at a time (enforced by a partial unique index), so reports arriving
//! while a case is open join it and reports arriving after its resolution open
//! a fresh one.
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, Postgres, QueryBuilder};
use tokuly_core::prelude::*;
use tokuly_proto::{
    admin::{AssigneeFilter, CaseNote, CaseReport, CaseSort, CaseStatus, ModerationCase, ReportCategory, UserModerationSummary},
    common::{SubjectRef, UserSummary},
};

use crate::{Admin, audit::Audit, staff::Staff};

pub(crate) const STATUS_OPEN: &str = "open";
pub(crate) const STATUS_RESOLVED: &str = "resolved";

macro_rules! case_columns {
    () => {
        "id, subject_type, subject_id, subject_owner_id, owner_hidden, subject_title, subject_path, status, report_count, categories, \
         assignee_id, assigned_at, last_reported_at, resolution, content_removed, resolution_reason, resolved_by, \
         resolved_at, created_at"
    };
}
pub(crate) use case_columns;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct CaseRow {
    pub id: Uuid,
    pub subject_type: i32,
    pub subject_id: Uuid,
    pub subject_owner_id: Option<Uuid>,
    /// The content hides its author; see `owners.rs` for who may see
    /// `subject_owner_id` then.
    pub owner_hidden: bool,
    pub subject_title: String,
    pub subject_path: String,
    pub status: String,
    pub report_count: i32,
    pub categories: Vec<String>,
    pub assignee_id: Option<Uuid>,
    pub assigned_at: Option<DateTime<Utc>>,
    pub last_reported_at: DateTime<Utc>,
    pub resolution: Option<i32>,
    pub content_removed: bool,
    pub resolution_reason: String,
    pub resolved_by: Option<Uuid>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl CaseRow {
    pub fn is_open(&self) -> bool {
        self.status == STATUS_OPEN
    }

    /// The owner as far as a view may name them: a hidden owner only when
    /// the view was asked to (`reveal_hidden`).
    fn shown_owner(&self, reveal_hidden: bool) -> Option<Uuid> {
        self.subject_owner_id.filter(|_| reveal_hidden || !self.owner_hidden)
    }

    fn people(&self, reveal_hidden: bool) -> impl Iterator<Item = Uuid> {
        [self.shown_owner(reveal_hidden), self.assignee_id, self.resolved_by].into_iter().flatten()
    }

    pub fn to_proto(&self, people: &HashMap<Uuid, UserSummary>, reveal_hidden: bool) -> ModerationCase {
        let person = |id: Option<Uuid>| id.and_then(|id| people.get(&id).cloned());
        ModerationCase {
            id: self.id.to_string(),
            subject: SubjectRef { r#type: self.subject_type.into(), id: self.subject_id.to_string(), ..Default::default() }.into(),
            subject_title: self.subject_title.clone(),
            subject_path: self.subject_path.clone(),
            subject_owner: person(self.shown_owner(reveal_hidden)).into(),
            owner_withheld: self.owner_hidden && !reveal_hidden,
            status: if self.is_open() { CaseStatus::CASE_STATUS_OPEN } else { CaseStatus::CASE_STATUS_RESOLVED }.into(),
            report_count: self.report_count,
            categories: self.categories.iter().map(|c| category_from_str(c).into()).collect(),
            assignee: person(self.assignee_id).into(),
            assigned_at: opt_ts(self.assigned_at),
            created_at: ts(self.created_at),
            last_reported_at: ts(self.last_reported_at),
            resolution: self.resolution.unwrap_or_default().into(),
            content_removed: self.content_removed,
            resolution_reason: self.resolution_reason.clone(),
            resolved_by: person(self.resolved_by).into(),
            resolved_at: opt_ts(self.resolved_at),
            ..Default::default()
        }
    }
}

/// The category strings used on the wire by `engagement.report.created`.
const CATEGORIES: &[(&str, ReportCategory)] = &[
    ("SPAM", ReportCategory::REPORT_CATEGORY_SPAM),
    ("HARASSMENT", ReportCategory::REPORT_CATEGORY_HARASSMENT),
    ("HATE", ReportCategory::REPORT_CATEGORY_HATE),
    ("SEXUAL", ReportCategory::REPORT_CATEGORY_SEXUAL),
    ("VIOLENCE", ReportCategory::REPORT_CATEGORY_VIOLENCE),
    ("SELF_HARM", ReportCategory::REPORT_CATEGORY_SELF_HARM),
    ("ILLEGAL", ReportCategory::REPORT_CATEGORY_ILLEGAL),
    ("IMPERSONATION", ReportCategory::REPORT_CATEGORY_IMPERSONATION),
    ("COPYRIGHT", ReportCategory::REPORT_CATEGORY_COPYRIGHT),
    ("OTHER", ReportCategory::REPORT_CATEGORY_OTHER),
];

/// Category enum for a stored string; anything unknown reads as OTHER so a
/// category added by engagement never breaks the queue.
pub(crate) fn category_from_str(name: &str) -> ReportCategory {
    CATEGORIES.iter().find(|(n, _)| *n == name).map(|(_, c)| *c).unwrap_or(ReportCategory::REPORT_CATEGORY_OTHER)
}

/// The stored string of a category; `None` for UNSPECIFIED.
pub(crate) fn category_to_str(category: ReportCategory) -> Option<&'static str> {
    CATEGORIES.iter().find(|(_, c)| *c == category).map(|(n, _)| *n)
}

/// Normalizes a category received in an event to one of the known strings.
pub(crate) fn normalize_category(name: &str) -> &'static str {
    let upper = name.trim().to_ascii_uppercase();
    CATEGORIES.iter().find(|(n, _)| *n == upper).map(|(n, _)| *n).unwrap_or("OTHER")
}

/// Filters of `ModerationService.ListCases`.
#[derive(Debug)]
pub(crate) struct CaseFilter {
    pub open: bool,
    pub assignee: AssigneeFilter,
    /// A specific moderator; wins over `assignee`.
    pub assignee_id: Option<Uuid>,
    pub subject_type: i32,
    pub category: Option<&'static str>,
    pub owner_id: Option<Uuid>,
    pub sort: CaseSort,
}

#[derive(Debug, sqlx::FromRow)]
pub(crate) struct ReportRow {
    pub id: Uuid,
    pub reporter_id: Option<Uuid>,
    pub category: String,
    pub text: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
pub(crate) struct NoteRow {
    pub id: Uuid,
    pub author_id: Option<Uuid>,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

impl Admin {
    /// A case by ID, or `CASE_NOT_FOUND`.
    pub(crate) async fn case(&self, case_id: Uuid) -> AppResult<CaseRow> {
        sqlx::query_as(concat!("SELECT ", case_columns!(), " FROM moderation_cases WHERE id = $1"))
            .bind(case_id)
            .fetch_optional(&self.core.db)
            .await?
            .or_not_found("CASE_NOT_FOUND", "moderation case not found")
    }

    /// Locks a case for the rest of the transaction, so two moderators
    /// cannot resolve it at once.
    pub(crate) async fn lock_case(&self, conn: &mut PgConnection, case_id: Uuid) -> AppResult<CaseRow> {
        sqlx::query_as(concat!("SELECT ", case_columns!(), " FROM moderation_cases WHERE id = $1 FOR UPDATE"))
            .bind(case_id)
            .fetch_optional(conn)
            .await?
            .or_not_found("CASE_NOT_FOUND", "moderation case not found")
    }

    /// One page of the queue. Each sort order has its own keyset, so a cursor
    /// from one order is rejected by another.
    pub(crate) async fn find_cases(&self, viewer: Uuid, filter: &CaseFilter, page: &Page) -> AppResult<(Vec<CaseRow>, tokuly_proto::common::PageInfo)> {
        let mut query: QueryBuilder<Postgres> = QueryBuilder::new(concat!("SELECT ", case_columns!(), " FROM moderation_cases WHERE status = "));
        query.push_bind(if filter.open { STATUS_OPEN } else { STATUS_RESOLVED });
        match (filter.assignee_id, filter.assignee) {
            (Some(assignee_id), _) => {
                query.push(" AND assignee_id = ").push_bind(assignee_id);
            }
            (None, AssigneeFilter::ASSIGNEE_FILTER_MINE) => {
                query.push(" AND assignee_id = ").push_bind(viewer);
            }
            (None, AssigneeFilter::ASSIGNEE_FILTER_UNASSIGNED) => {
                query.push(" AND assignee_id IS NULL");
            }
            (None, AssigneeFilter::ASSIGNEE_FILTER_UNSPECIFIED) => {}
        }
        if filter.subject_type != 0 {
            query.push(" AND subject_type = ").push_bind(filter.subject_type);
        }
        if let Some(category) = filter.category {
            query.push(" AND ").push_bind(category).push(" = ANY(categories)");
        }
        if let Some(owner_id) = filter.owner_id {
            // Never the cases whose owner is hidden: a list by owner would name them.
            query.push(" AND NOT owner_hidden AND subject_owner_id = ").push_bind(owner_id);
        }
        match filter.sort {
            CaseSort::CASE_SORT_UNSPECIFIED => {
                if let Some((at, id)) = page.decode::<(DateTime<Utc>, Uuid)>()? {
                    query.push(" AND (last_reported_at, id) < (").push_bind(at).push(", ").push_bind(id).push(")");
                }
                query.push(" ORDER BY last_reported_at DESC, id DESC");
            }
            CaseSort::CASE_SORT_OLDEST => {
                if let Some((at, id)) = page.decode::<(DateTime<Utc>, Uuid)>()? {
                    query.push(" AND (created_at, id) > (").push_bind(at).push(", ").push_bind(id).push(")");
                }
                query.push(" ORDER BY created_at, id");
            }
            CaseSort::CASE_SORT_MOST_REPORTED => {
                if let Some((count, id)) = page.decode::<(i32, Uuid)>()? {
                    query.push(" AND (report_count, id) < (").push_bind(count).push(", ").push_bind(id).push(")");
                }
                query.push(" ORDER BY report_count DESC, id DESC");
            }
        }
        query.push(" LIMIT ").push_bind(page.fetch());
        let mut rows: Vec<CaseRow> = query.build_query_as().fetch_all(&self.core.db).await?;
        let info = match filter.sort {
            CaseSort::CASE_SORT_UNSPECIFIED => page.finish(&mut rows, |r| (r.last_reported_at, r.id)),
            CaseSort::CASE_SORT_OLDEST => page.finish(&mut rows, |r| (r.created_at, r.id)),
            CaseSort::CASE_SORT_MOST_REPORTED => page.finish(&mut rows, |r| (r.report_count, r.id)),
        };
        Ok((rows, info))
    }

    /// Cases as API messages, with owner, assignee and resolver resolved.
    /// A hidden owner is withheld: lists and the answers to changes never
    /// name the author of an anonymous post.
    pub(crate) async fn cases_to_proto(&self, rows: &[CaseRow]) -> AppResult<Vec<ModerationCase>> {
        let ids: Vec<Uuid> = rows.iter().flat_map(|row| row.people(false)).collect();
        let people = tokuly_identity::summaries(&self.core, &ids).await?;
        Ok(rows.iter().map(|row| row.to_proto(&people, false)).collect())
    }

    pub(crate) async fn case_to_proto(&self, row: &CaseRow) -> AppResult<ModerationCase> {
        Ok(self.cases_to_proto(std::slice::from_ref(row)).await?.remove(0))
    }

    /// A case as the moderator who opened it sees it, and whether that view
    /// names its owner. A hidden owner is named to a moderator who is not
    /// involved in the post, which the audit log records; a moderator who is
    /// involved is told that another one has to handle the case.
    pub(crate) async fn opened_case(&self, staff: &Staff, row: &CaseRow) -> AppResult<(ModerationCase, bool)> {
        let involved = row.owner_hidden && self.is_party_to_hidden(staff.id, row.subject_type, row.subject_id).await?;
        let reveal = !involved;
        if row.owner_hidden && reveal && row.subject_owner_id.is_some() {
            Audit::by(staff, "case.author_revealed").case(row.id, row.subject_owner_id).record(&self.core.db).await?;
        }
        let ids: Vec<Uuid> = row.people(reveal).collect();
        let people = tokuly_identity::summaries(&self.core, &ids).await?;
        let mut case = row.to_proto(&people, reveal);
        case.conflict_of_interest = involved;
        Ok((case, reveal && row.subject_owner_id.is_some()))
    }

    /// One page of a case's reports, newest first.
    pub(crate) async fn case_reports(&self, case_id: Uuid, page: &Page) -> AppResult<(Vec<CaseReport>, tokuly_proto::common::PageInfo)> {
        let cursor = page.decode::<(DateTime<Utc>, Uuid)>()?;
        let mut rows: Vec<ReportRow> = sqlx::query_as(
            "SELECT id, reporter_id, category, text, created_at FROM moderation_reports
             WHERE case_id = $1 AND ($2::timestamptz IS NULL OR (created_at, id) < ($2, $3))
             ORDER BY created_at DESC, id DESC LIMIT $4",
        )
        .bind(case_id)
        .bind(cursor.map(|c| c.0))
        .bind(cursor.map(|c| c.1))
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |r| (r.created_at, r.id));
        let reporters: Vec<Uuid> = rows.iter().filter_map(|r| r.reporter_id).collect();
        let people = tokuly_identity::summaries(&self.core, &reporters).await?;
        let reports = rows
            .into_iter()
            .map(|row| CaseReport {
                id: row.id.to_string(),
                reporter: row.reporter_id.and_then(|id| people.get(&id).cloned()).into(),
                category: category_from_str(&row.category).into(),
                text: row.text,
                created_at: ts(row.created_at),
                ..Default::default()
            })
            .collect();
        Ok((reports, info))
    }

    /// A case's internal notes, oldest first.
    pub(crate) async fn case_notes(&self, case_id: Uuid) -> AppResult<Vec<CaseNote>> {
        let rows: Vec<NoteRow> = sqlx::query_as("SELECT id, author_id, body, created_at FROM moderation_case_notes WHERE case_id = $1 ORDER BY id LIMIT 200")
            .bind(case_id)
            .fetch_all(&self.core.db)
            .await?;
        let authors: Vec<Uuid> = rows.iter().filter_map(|r| r.author_id).collect();
        let people = tokuly_identity::summaries(&self.core, &authors).await?;
        Ok(rows.into_iter().map(|row| note_to_proto(row, &people)).collect())
    }

    /// An account's moderation record in numbers.
    pub(crate) async fn moderation_summary(&self, user_id: Uuid) -> AppResult<UserModerationSummary> {
        let (reports_received, reports_filed, open_cases, warnings, content_removals, suspensions): (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT
                    (SELECT COALESCE(sum(report_count), 0) FROM moderation_cases WHERE subject_owner_id = $1),
                    (SELECT count(*) FROM moderation_reports WHERE reporter_id = $1),
                    (SELECT count(*) FROM moderation_cases WHERE subject_owner_id = $1 AND status = 'open'),
                    (SELECT count(*) FROM moderation_actions WHERE target_user_id = $1 AND kind = 3),
                    (SELECT count(*) FROM moderation_actions WHERE target_user_id = $1 AND kind = 2),
                    (SELECT count(*) FROM moderation_actions WHERE target_user_id = $1 AND kind IN (4, 5))",
        )
        .bind(user_id)
        .fetch_one(&self.core.db)
        .await?;
        Ok(UserModerationSummary {
            reports_received: reports_received as i32,
            reports_filed: reports_filed as i32,
            open_cases: open_cases as i32,
            warnings: warnings as i32,
            content_removals: content_removals as i32,
            suspensions: suspensions as i32,
            ..Default::default()
        })
    }
}

pub(crate) fn note_to_proto(row: NoteRow, people: &HashMap<Uuid, UserSummary>) -> CaseNote {
    CaseNote {
        id: row.id.to_string(),
        author: row.author_id.and_then(|id| people.get(&id).cloned()).into(),
        body: row.body,
        created_at: ts(row.created_at),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use tokuly_proto::admin::ModerationActionKind;

    use super::*;

    #[test]
    fn categories_roundtrip_and_unknown_ones_read_as_other() {
        for (name, category) in CATEGORIES {
            assert_eq!(category_from_str(name), *category);
            assert_eq!(category_to_str(*category), Some(*name));
        }
        assert_eq!(category_from_str("SOMETHING_NEW"), ReportCategory::REPORT_CATEGORY_OTHER);
        assert_eq!(category_to_str(ReportCategory::REPORT_CATEGORY_UNSPECIFIED), None);
        assert_eq!(normalize_category(" self_harm "), "SELF_HARM");
        assert_eq!(normalize_category("made up"), "OTHER");
    }

    #[test]
    fn summary_query_kinds_match_the_proto_enum() {
        // `moderation_summary` filters on these numbers in SQL.
        assert_eq!(ModerationActionKind::MODERATION_ACTION_KIND_REMOVE_CONTENT as i32, 2);
        assert_eq!(ModerationActionKind::MODERATION_ACTION_KIND_WARN as i32, 3);
        assert_eq!(ModerationActionKind::MODERATION_ACTION_KIND_SUSPEND as i32, 4);
        assert_eq!(ModerationActionKind::MODERATION_ACTION_KIND_BAN as i32, 5);
    }

    #[test]
    fn resolved_case_converts_with_people() {
        let (owner, moderator) = (Uuid::now_v7(), Uuid::now_v7());
        let row = CaseRow {
            id: Uuid::now_v7(),
            subject_type: 3,
            subject_id: Uuid::now_v7(),
            subject_owner_id: Some(owner),
            owner_hidden: false,
            subject_title: "こんにちは".into(),
            subject_path: "/notes/x".into(),
            status: STATUS_RESOLVED.into(),
            report_count: 2,
            categories: vec!["SPAM".into(), "HATE".into()],
            assignee_id: None,
            assigned_at: None,
            last_reported_at: Utc::now(),
            resolution: Some(ModerationActionKind::MODERATION_ACTION_KIND_WARN as i32),
            content_removed: true,
            resolution_reason: "spam".into(),
            resolved_by: Some(moderator),
            resolved_at: Some(Utc::now()),
            created_at: Utc::now(),
        };
        let people = HashMap::from([(moderator, UserSummary { id: moderator.to_string(), ..Default::default() })]);
        let case = row.to_proto(&people, false);
        assert_eq!(case.status, CaseStatus::CASE_STATUS_RESOLVED);
        assert_eq!(case.resolution, ModerationActionKind::MODERATION_ACTION_KIND_WARN);
        assert_eq!(case.categories.len(), 2);
        assert!(case.subject_owner.is_unset(), "an owner missing from the lookup stays unset");
        assert_eq!(case.resolved_by.as_option().unwrap().id, moderator.to_string());
        assert!(case.assignee.is_unset() && case.resolved_at.is_set());

        // A hidden owner is named only by a view that was asked to.
        let people = HashMap::from([(owner, UserSummary { id: owner.to_string(), ..Default::default() })]);
        let hidden = CaseRow { owner_hidden: true, ..row.clone() };
        let withheld = hidden.to_proto(&people, false);
        assert!(withheld.owner_withheld && withheld.subject_owner.is_unset());
        assert!(!hidden.people(false).any(|id| id == owner), "a withheld owner is not even looked up");
        let named = hidden.to_proto(&people, true);
        assert!(!named.owner_withheld);
        assert_eq!(named.subject_owner.as_option().unwrap().id, owner.to_string());
        assert!(!row.to_proto(&people, false).owner_withheld, "an ordinary owner is always named");
    }
}
