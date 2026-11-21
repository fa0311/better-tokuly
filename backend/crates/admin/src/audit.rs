//! The audit log: one row per administrative action.
//!
//! Every state-changing admin RPC writes an [`Audit`] entry in the same
//! transaction as the change, so the log can never disagree with what
//! happened. Entries record who acted, on what, why, and (where it is
//! meaningful) the state before and after as JSON.
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgExecutor, Postgres, QueryBuilder};
use tokuly_core::prelude::*;
use tokuly_proto::admin::AuditEntry;

use crate::{Admin, staff::Staff};

/// An audit entry under construction: `Audit::by(&staff, "user.suspended").user(id).reason(..).record(&mut *tx)`.
#[derive(Debug)]
pub(crate) struct Audit {
    actor_id: Option<Uuid>,
    ip: String,
    action: &'static str,
    target_type: &'static str,
    target_id: String,
    target_user_id: Option<Uuid>,
    reason: String,
    before: Option<Value>,
    after: Option<Value>,
}

impl Audit {
    /// An action performed by a staff member.
    pub fn by(staff: &Staff, action: &'static str) -> Self {
        Self::new(Some(staff.id), staff.ip.clone(), action)
    }

    /// An action the system performed on its own (expired suspensions,
    /// bootstrap configuration).
    pub fn system(action: &'static str) -> Self {
        Self::new(None, String::new(), action)
    }

    fn new(actor_id: Option<Uuid>, ip: String, action: &'static str) -> Self {
        Self { actor_id, ip, action, target_type: "", target_id: String::new(), target_user_id: None, reason: String::new(), before: None, after: None }
    }

    /// The action targets an account.
    pub fn user(mut self, user_id: Uuid) -> Self {
        self.target_type = "user";
        self.target_id = user_id.to_string();
        self.target_user_id = Some(user_id);
        self
    }

    /// The action targets a moderation case about `owner`'s content.
    pub fn case(mut self, case_id: Uuid, owner: Option<Uuid>) -> Self {
        self.target_type = "case";
        self.target_id = case_id.to_string();
        self.target_user_id = owner;
        self
    }

    /// The action targets a piece of content owned by `owner`.
    pub fn subject(mut self, subject_type: i32, subject_id: Uuid, owner: Option<Uuid>) -> Self {
        self.target_type = "subject";
        self.target_id = format!("{subject_type}:{subject_id}");
        self.target_user_id = owner;
        self
    }

    pub fn flag(mut self, name: &str) -> Self {
        self.target_type = "flag";
        self.target_id = name.to_owned();
        self
    }

    pub fn announcement(mut self, id: Uuid) -> Self {
        self.target_type = "announcement";
        self.target_id = id.to_string();
        self
    }

    pub fn reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = reason.into();
        self
    }

    pub fn before(mut self, state: Value) -> Self {
        self.before = Some(state);
        self
    }

    pub fn after(mut self, state: Value) -> Self {
        self.after = Some(state);
        self
    }

    /// Writes the entry. Pass the open transaction of the change it describes.
    pub async fn record<'e, E: PgExecutor<'e>>(self, executor: E) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO admin_audit_log (actor_id, action, target_type, target_id, target_user_id, reason, before, after, ip)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(self.actor_id)
        .bind(self.action)
        .bind(self.target_type)
        .bind(self.target_id)
        .bind(self.target_user_id)
        .bind(self.reason)
        .bind(self.before)
        .bind(self.after)
        .bind(self.ip)
        .execute(executor)
        .await?;
        Ok(())
    }
}

/// Filters of `SystemService.ListAuditLog`.
#[derive(Debug, Default)]
pub(crate) struct AuditFilter {
    pub actor_id: Option<Uuid>,
    /// Exact action, or a prefix when it ends with a dot (`user.`).
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub target_user_id: Option<Uuid>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
}

#[derive(Debug, sqlx::FromRow)]
pub(crate) struct AuditRow {
    pub id: Uuid,
    pub actor_id: Option<Uuid>,
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub target_user_id: Option<Uuid>,
    pub reason: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub ip: String,
    pub created_at: DateTime<Utc>,
}

/// Escapes `%`, `_` and `\` so user input matches literally inside LIKE.
pub(crate) fn escape_like(input: &str) -> String {
    input.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

impl Admin {
    /// One page of the audit log, newest first.
    pub(crate) async fn list_audit(&self, filter: &AuditFilter, page: &Page) -> AppResult<Vec<AuditRow>> {
        let mut query: QueryBuilder<Postgres> = QueryBuilder::new(
            "SELECT id, actor_id, action, target_type, target_id, target_user_id, reason, before, after, ip, created_at
             FROM admin_audit_log WHERE true",
        );
        if let Some(actor_id) = filter.actor_id {
            query.push(" AND actor_id = ").push_bind(actor_id);
        }
        if let Some(prefix) = filter.action.strip_suffix('.') {
            query.push(" AND action LIKE ").push_bind(format!("{}.%", escape_like(prefix)));
        } else if !filter.action.is_empty() {
            query.push(" AND action = ").push_bind(filter.action.clone());
        }
        if !filter.target_type.is_empty() {
            query.push(" AND target_type = ").push_bind(filter.target_type.clone());
        }
        if !filter.target_id.is_empty() {
            query.push(" AND target_id = ").push_bind(filter.target_id.clone());
        }
        if let Some(user_id) = filter.target_user_id {
            query.push(" AND target_user_id = ").push_bind(user_id);
        }
        if let Some(since) = filter.since {
            query.push(" AND created_at >= ").push_bind(since);
        }
        if let Some(until) = filter.until {
            query.push(" AND created_at < ").push_bind(until);
        }
        if let Some(after) = page.decode::<Uuid>()? {
            query.push(" AND id < ").push_bind(after);
        }
        query.push(" ORDER BY id DESC LIMIT ").push_bind(page.fetch());
        Ok(query.build_query_as().fetch_all(&self.core.db).await?)
    }

    /// Audit rows as API messages, with the people involved resolved.
    pub(crate) async fn audit_entries(&self, rows: Vec<AuditRow>) -> AppResult<Vec<AuditEntry>> {
        let ids: Vec<Uuid> = rows.iter().flat_map(|r| [r.actor_id, r.target_user_id]).flatten().collect();
        let people = tokuly_identity::summaries(&self.core, &ids).await?;
        let json = |value: Option<Value>| value.map(|v| v.to_string()).unwrap_or_default();
        Ok(rows
            .into_iter()
            .map(|row| AuditEntry {
                id: row.id.to_string(),
                actor: row.actor_id.and_then(|id| people.get(&id).cloned()).into(),
                action: row.action,
                target_type: row.target_type,
                target_id: row.target_id,
                target_user: row.target_user_id.and_then(|id| people.get(&id).cloned()).into(),
                reason: row.reason,
                before_json: json(row.before),
                after_json: json(row.after),
                ip: row.ip,
                created_at: ts(row.created_at),
                ..Default::default()
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_patterns_are_escaped() {
        assert_eq!(escape_like("50%_off\\"), "50\\%\\_off\\\\");
        assert_eq!(escape_like("plain"), "plain");
    }

    #[test]
    fn targets_fill_the_right_columns() {
        let staff = Staff { id: Uuid::now_v7(), is_admin: true, ip: "203.0.113.9".into() };
        let user = Uuid::now_v7();
        let entry = Audit::by(&staff, "user.suspended").user(user).reason("spam");
        assert_eq!((entry.target_type, entry.target_user_id, entry.actor_id), ("user", Some(user), Some(staff.id)));
        assert_eq!(entry.ip, "203.0.113.9");

        let subject = Uuid::now_v7();
        let entry = Audit::system("content.removed").subject(5, subject, None);
        assert_eq!(entry.target_id, format!("5:{subject}"));
        assert!(entry.actor_id.is_none() && entry.target_user_id.is_none());

        let entry = Audit::by(&staff, "flag.updated").flag("new_map");
        assert_eq!((entry.target_type, entry.target_id.as_str()), ("flag", "new_map"));
    }
}
