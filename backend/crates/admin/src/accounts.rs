//! Accounts as staff see them.
//!
//! This is the one place outside identity that reads the `users` and
//! `sessions` tables: staff need to list and search every account, including
//! suspended and deleted ones, which identity's public lookups hide. It is
//! read-only; every change goes through `tokuly_identity::Identity`.
use chrono::{DateTime, Utc};
use sqlx::{Postgres, QueryBuilder};
use tokuly_core::prelude::*;
use tokuly_proto::admin::{AccountState, AdminUser, Suspension};

use crate::{Admin, audit::escape_like};

/// Roles identity accepts.
pub(crate) const ROLES: &[&str] = &["admin", "moderator", "staff"];

/// Longest search query.
const QUERY_MAX_CHARS: usize = 100;

macro_rules! admin_user_select {
    () => {
        "SELECT u.id, u.email::text AS email, u.email_verified_at, u.roles, u.locale, u.banned_at, u.ban_reason,
            u.deletion_scheduled_at, u.deleted_at, u.created_at,
            s.last_active_at, COALESCE(s.active_sessions, 0) AS active_session_count,
            sp.id AS suspension_id, sp.reason AS suspension_reason, sp.moderator_id AS suspension_moderator_id,
            sp.created_at AS suspension_started_at, sp.expires_at AS suspension_expires_at, sp.case_id AS suspension_case_id
     FROM users u
     LEFT JOIN LATERAL (
         SELECT max(last_active_at) AS last_active_at, count(*) AS active_sessions
         FROM sessions WHERE user_id = u.id AND revoked_at IS NULL AND expires_at > now()
     ) s ON true
     LEFT JOIN user_suspensions sp ON sp.user_id = u.id AND sp.ended_at IS NULL"
    };
}

#[derive(Debug, sqlx::FromRow)]
pub(crate) struct AdminUserRow {
    pub id: Uuid,
    pub email: Option<String>,
    pub email_verified_at: Option<DateTime<Utc>>,
    pub roles: Vec<String>,
    pub locale: String,
    pub banned_at: Option<DateTime<Utc>>,
    pub ban_reason: String,
    pub deletion_scheduled_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub last_active_at: Option<DateTime<Utc>>,
    pub active_session_count: i64,
    pub suspension_id: Option<Uuid>,
    pub suspension_reason: Option<String>,
    pub suspension_moderator_id: Option<Uuid>,
    pub suspension_started_at: Option<DateTime<Utc>>,
    pub suspension_expires_at: Option<DateTime<Utc>>,
    pub suspension_case_id: Option<Uuid>,
}

impl AdminUserRow {
    /// Deleted wins over suspended, which wins over a pending deletion.
    pub fn state(&self) -> AccountState {
        if self.deleted_at.is_some() {
            AccountState::ACCOUNT_STATE_DELETED
        } else if self.banned_at.is_some() {
            AccountState::ACCOUNT_STATE_SUSPENDED
        } else if self.deletion_scheduled_at.is_some() {
            AccountState::ACCOUNT_STATE_DELETION_SCHEDULED
        } else {
            AccountState::ACCOUNT_STATE_ACTIVE
        }
    }
}

/// How a search query is matched.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum UserQuery {
    /// Newest accounts.
    All,
    Id(Uuid),
    /// Exact address or its beginning.
    Email(String),
    /// Part of a handle or display name.
    Name(String),
}

impl UserQuery {
    pub fn parse(input: &str) -> AppResult<Self> {
        let query = input.trim();
        ensure!(query.chars().count() <= QUERY_MAX_CHARS, invalid, "QUERY_TOO_LONG", "search query is too long");
        if query.is_empty() {
            return Ok(Self::All);
        }
        if let Ok(id) = Uuid::parse_str(query) {
            return Ok(Self::Id(id));
        }
        // `@handle` is a handle; `name@host` is an email address.
        if query.contains('@') && !query.starts_with('@') {
            return Ok(Self::Email(query.to_owned()));
        }
        let name = query.trim_start_matches('@');
        Ok(if name.is_empty() { Self::All } else { Self::Name(name.to_owned()) })
    }
}

/// Filters of `UserAdminService.SearchUsers`.
#[derive(Debug)]
pub(crate) struct UserFilter {
    pub query: UserQuery,
    pub state: AccountState,
    /// One of [`ROLES`], or empty.
    pub role: String,
    pub verified_only: bool,
}

impl Admin {
    /// One page of accounts matching the filter, newest first.
    pub(crate) async fn search_accounts(&self, filter: &UserFilter, page: &Page) -> AppResult<(Vec<AdminUserRow>, tokuly_proto::common::PageInfo)> {
        let mut query: QueryBuilder<Postgres> = QueryBuilder::new(admin_user_select!());
        query.push(" WHERE true");
        match &filter.query {
            UserQuery::All => {}
            UserQuery::Id(id) => {
                query.push(" AND u.id = ").push_bind(*id);
            }
            UserQuery::Email(email) => {
                query.push(" AND (u.email = ").push_bind(email.clone()).push("::citext OR u.email::text ILIKE ");
                query.push_bind(format!("{}%", escape_like(email))).push(")");
            }
            UserQuery::Name(name) => {
                let pattern = format!("%{}%", escape_like(name));
                query.push(" AND (u.handle::text ILIKE ").push_bind(pattern.clone());
                query.push(" OR u.display_name ILIKE ").push_bind(pattern).push(")");
            }
        }
        match filter.state {
            AccountState::ACCOUNT_STATE_UNSPECIFIED => {}
            AccountState::ACCOUNT_STATE_ACTIVE => {
                query.push(" AND u.deleted_at IS NULL AND u.banned_at IS NULL AND u.deletion_scheduled_at IS NULL");
            }
            AccountState::ACCOUNT_STATE_SUSPENDED => {
                query.push(" AND u.deleted_at IS NULL AND u.banned_at IS NOT NULL");
            }
            AccountState::ACCOUNT_STATE_DELETION_SCHEDULED => {
                query.push(" AND u.deleted_at IS NULL AND u.banned_at IS NULL AND u.deletion_scheduled_at IS NOT NULL");
            }
            AccountState::ACCOUNT_STATE_DELETED => {
                query.push(" AND u.deleted_at IS NOT NULL");
            }
        }
        if !filter.role.is_empty() {
            query.push(" AND ").push_bind(filter.role.clone()).push(" = ANY(u.roles)");
        }
        if filter.verified_only {
            query.push(" AND u.verified");
        }
        if let Some(after) = page.decode::<Uuid>()? {
            query.push(" AND u.id < ").push_bind(after);
        }
        query.push(" ORDER BY u.id DESC LIMIT ").push_bind(page.fetch());
        let mut rows: Vec<AdminUserRow> = query.build_query_as().fetch_all(&self.core.db).await?;
        let info = page.finish(&mut rows, |r| r.id);
        Ok((rows, info))
    }

    /// One account, including deleted ones, or `USER_NOT_FOUND`.
    pub(crate) async fn account(&self, user_id: Uuid) -> AppResult<AdminUserRow> {
        sqlx::query_as(concat!(admin_user_select!(), " WHERE u.id = $1"))
            .bind(user_id)
            .fetch_optional(&self.core.db)
            .await?
            .or_not_found("USER_NOT_FOUND", "user not found")
    }

    /// Accounts as API messages, with summaries and suspending moderators resolved.
    pub(crate) async fn accounts_to_proto(&self, rows: &[AdminUserRow]) -> AppResult<Vec<AdminUser>> {
        let ids: Vec<Uuid> = rows.iter().flat_map(|r| [Some(r.id), r.suspension_moderator_id]).flatten().collect();
        let people = tokuly_identity::summaries(&self.core, &ids).await?;
        Ok(rows
            .iter()
            .map(|row| {
                let state = row.state();
                // An account locked outside the console has no suspension row;
                // identity's own columns still say when and why.
                let suspension = (state == AccountState::ACCOUNT_STATE_SUSPENDED).then(|| Suspension {
                    id: row.suspension_id.map(|id| id.to_string()).unwrap_or_default(),
                    reason: row.suspension_reason.clone().unwrap_or_else(|| row.ban_reason.clone()),
                    moderator: row.suspension_moderator_id.and_then(|id| people.get(&id).cloned()).into(),
                    started_at: opt_ts(row.suspension_started_at.or(row.banned_at)),
                    expires_at: opt_ts(row.suspension_expires_at),
                    permanent: row.suspension_expires_at.is_none(),
                    case_id: row.suspension_case_id.map(|id| id.to_string()).unwrap_or_default(),
                    ..Default::default()
                });
                AdminUser {
                    user: people.get(&row.id).cloned().into(),
                    email: row.email.clone().unwrap_or_default(),
                    email_verified: row.email_verified_at.is_some(),
                    roles: row.roles.clone(),
                    state: state.into(),
                    suspension: suspension.into(),
                    created_at: ts(row.created_at),
                    deletion_scheduled_at: opt_ts(row.deletion_scheduled_at),
                    deleted_at: opt_ts(row.deleted_at),
                    last_active_at: opt_ts(row.last_active_at),
                    active_session_count: row.active_session_count as i32,
                    locale: row.locale.clone(),
                    ..Default::default()
                }
            })
            .collect())
    }

    /// The current staff view of one account.
    pub(crate) async fn admin_user(&self, user_id: Uuid) -> AppResult<AdminUser> {
        let row = self.account(user_id).await?;
        Ok(self.accounts_to_proto(std::slice::from_ref(&row)).await?.remove(0))
    }
}

/// Validates and normalizes a role list: known roles only, sorted, no duplicates.
pub(crate) fn normalize_roles(roles: &[String]) -> AppResult<Vec<String>> {
    let mut normalized = Vec::with_capacity(roles.len());
    for role in roles {
        let role = role.trim();
        ensure!(ROLES.contains(&role), invalid, "ROLE_INVALID", "unknown role {role:?}");
        if !normalized.iter().any(|r| r == role) {
            normalized.push(role.to_owned());
        }
    }
    normalized.sort();
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_are_classified() {
        let id = Uuid::now_v7();
        assert_eq!(UserQuery::parse("  ").unwrap(), UserQuery::All);
        assert_eq!(UserQuery::parse(&id.to_string()).unwrap(), UserQuery::Id(id));
        assert_eq!(UserQuery::parse("alice@example.com").unwrap(), UserQuery::Email("alice@example.com".into()));
        assert_eq!(UserQuery::parse("alice@").unwrap(), UserQuery::Email("alice@".into()));
        assert_eq!(UserQuery::parse("@alice").unwrap(), UserQuery::Name("alice".into()));
        assert_eq!(UserQuery::parse("ありす").unwrap(), UserQuery::Name("ありす".into()));
        assert_eq!(UserQuery::parse("@").unwrap(), UserQuery::All);
        assert_eq!(UserQuery::parse(&"x".repeat(QUERY_MAX_CHARS + 1)).unwrap_err().reason, "QUERY_TOO_LONG");
    }

    #[test]
    fn roles_are_validated_sorted_and_deduplicated() {
        let roles = normalize_roles(&["moderator".into(), " admin ".into(), "moderator".into()]).unwrap();
        assert_eq!(roles, vec!["admin".to_owned(), "moderator".to_owned()]);
        assert!(normalize_roles(&[]).unwrap().is_empty());
        assert_eq!(normalize_roles(&["root".into()]).unwrap_err().reason, "ROLE_INVALID");
    }

    fn row() -> AdminUserRow {
        AdminUserRow {
            id: Uuid::now_v7(),
            email: None,
            email_verified_at: None,
            roles: vec![],
            locale: "ja".into(),
            banned_at: None,
            ban_reason: String::new(),
            deletion_scheduled_at: None,
            deleted_at: None,
            created_at: Utc::now(),
            last_active_at: None,
            active_session_count: 0,
            suspension_id: None,
            suspension_reason: None,
            suspension_moderator_id: None,
            suspension_started_at: None,
            suspension_expires_at: None,
            suspension_case_id: None,
        }
    }

    #[test]
    fn account_state_has_a_precedence() {
        let now = Some(Utc::now());
        assert_eq!(row().state(), AccountState::ACCOUNT_STATE_ACTIVE);
        assert_eq!(AdminUserRow { deletion_scheduled_at: now, ..row() }.state(), AccountState::ACCOUNT_STATE_DELETION_SCHEDULED);
        assert_eq!(AdminUserRow { banned_at: now, deletion_scheduled_at: now, ..row() }.state(), AccountState::ACCOUNT_STATE_SUSPENDED);
        assert_eq!(AdminUserRow { banned_at: now, deleted_at: now, ..row() }.state(), AccountState::ACCOUNT_STATE_DELETED);
    }
}
