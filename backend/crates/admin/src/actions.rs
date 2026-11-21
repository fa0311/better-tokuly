//! The moderation log: every action taken against an account or a piece of
//! content. An account's moderation history is the entries targeting it.
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, Postgres, QueryBuilder};
use tokuly_core::prelude::*;
use tokuly_proto::{
    admin::{ModerationAction, ModerationActionKind},
    common::{SubjectRef, UserSummary},
};

use crate::Admin;

macro_rules! action_columns {
    () => {
        "id, kind, moderator_id, target_user_id, subject_type, subject_id, subject_title, case_id, reason, note, expires_at, created_at"
    };
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ActionRow {
    pub id: Uuid,
    pub kind: i32,
    pub moderator_id: Option<Uuid>,
    pub target_user_id: Option<Uuid>,
    pub subject_type: Option<i32>,
    pub subject_id: Option<Uuid>,
    pub subject_title: String,
    pub case_id: Option<Uuid>,
    pub reason: String,
    pub note: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl ActionRow {
    fn people(&self) -> impl Iterator<Item = Uuid> {
        [self.moderator_id, self.target_user_id].into_iter().flatten()
    }

    pub fn to_proto(&self, people: &HashMap<Uuid, UserSummary>) -> ModerationAction {
        let person = |id: Option<Uuid>| id.and_then(|id| people.get(&id).cloned());
        let subject = match (self.subject_type, self.subject_id) {
            (Some(subject_type), Some(id)) => Some(SubjectRef { r#type: subject_type.into(), id: id.to_string(), ..Default::default() }),
            _ => None,
        };
        ModerationAction {
            id: self.id.to_string(),
            kind: self.kind.into(),
            moderator: person(self.moderator_id).into(),
            target_user: person(self.target_user_id).into(),
            subject: subject.into(),
            subject_title: self.subject_title.clone(),
            case_id: self.case_id.map(|id| id.to_string()).unwrap_or_default(),
            reason: self.reason.clone(),
            note: self.note.clone(),
            expires_at: opt_ts(self.expires_at),
            created_at: ts(self.created_at),
            ..Default::default()
        }
    }
}

/// A log entry about to be written.
#[derive(Debug)]
pub(crate) struct NewAction<'a> {
    pub kind: ModerationActionKind,
    /// `None` for automatic actions.
    pub moderator_id: Option<Uuid>,
    pub target_user_id: Option<Uuid>,
    /// `(SubjectType as i32, id)` of the content concerned.
    pub subject: Option<(i32, Uuid)>,
    pub subject_title: &'a str,
    pub case_id: Option<Uuid>,
    pub reason: &'a str,
    pub note: &'a str,
    pub expires_at: Option<DateTime<Utc>>,
}

impl<'a> NewAction<'a> {
    pub fn new(kind: ModerationActionKind, moderator_id: Option<Uuid>) -> Self {
        Self { kind, moderator_id, target_user_id: None, subject: None, subject_title: "", case_id: None, reason: "", note: "", expires_at: None }
    }

    pub fn target(mut self, user_id: Option<Uuid>) -> Self {
        self.target_user_id = user_id;
        self
    }

    pub fn subject(mut self, subject_type: i32, subject_id: Uuid, title: &'a str) -> Self {
        self.subject = Some((subject_type, subject_id));
        self.subject_title = title;
        self
    }

    pub fn case(mut self, case_id: Option<Uuid>) -> Self {
        self.case_id = case_id;
        self
    }

    pub fn texts(mut self, reason: &'a str, note: &'a str) -> Self {
        self.reason = reason;
        self.note = note;
        self
    }

    pub fn expires(mut self, expires_at: Option<DateTime<Utc>>) -> Self {
        self.expires_at = expires_at;
        self
    }

    /// Appends the entry to the moderation log.
    pub async fn insert(self, conn: &mut PgConnection) -> AppResult<ActionRow> {
        Ok(sqlx::query_as(concat!(
            "INSERT INTO moderation_actions (kind, moderator_id, target_user_id, subject_type, subject_id, subject_title, case_id, reason, note, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING ",
            action_columns!()
        ))
        .bind(self.kind as i32)
        .bind(self.moderator_id)
        .bind(self.target_user_id)
        .bind(self.subject.map(|s| s.0))
        .bind(self.subject.map(|s| s.1))
        .bind(self.subject_title)
        .bind(self.case_id)
        .bind(self.reason)
        .bind(self.note)
        .bind(self.expires_at)
        .fetch_one(conn)
        .await?)
    }
}

/// Filters of `ModerationService.ListModerationActions`.
#[derive(Debug, Default)]
pub(crate) struct ActionFilter {
    pub target_user_id: Option<Uuid>,
    pub moderator_id: Option<Uuid>,
    /// `ModerationActionKind as i32`; 0 = any.
    pub kind: i32,
    pub case_id: Option<Uuid>,
}

impl Admin {
    /// One page of the moderation log, newest first.
    pub(crate) async fn list_actions(&self, filter: &ActionFilter, page: &Page) -> AppResult<(Vec<ActionRow>, tokuly_proto::common::PageInfo)> {
        let mut query: QueryBuilder<Postgres> = QueryBuilder::new(concat!("SELECT ", action_columns!(), " FROM moderation_actions WHERE true"));
        if let Some(user_id) = filter.target_user_id {
            query.push(" AND target_user_id = ").push_bind(user_id);
        }
        if let Some(moderator_id) = filter.moderator_id {
            query.push(" AND moderator_id = ").push_bind(moderator_id);
        }
        if filter.kind != 0 {
            query.push(" AND kind = ").push_bind(filter.kind);
        }
        if let Some(case_id) = filter.case_id {
            query.push(" AND case_id = ").push_bind(case_id);
        }
        if let Some(after) = page.decode::<Uuid>()? {
            query.push(" AND id < ").push_bind(after);
        }
        query.push(" ORDER BY id DESC LIMIT ").push_bind(page.fetch());
        let mut rows: Vec<ActionRow> = query.build_query_as().fetch_all(&self.core.db).await?;
        let info = page.finish(&mut rows, |r| r.id);
        Ok((rows, info))
    }

    /// Log entries as API messages, with moderator and target resolved.
    pub(crate) async fn actions_to_proto(&self, rows: &[ActionRow]) -> AppResult<Vec<ModerationAction>> {
        let ids: Vec<Uuid> = rows.iter().flat_map(ActionRow::people).collect();
        let people = tokuly_identity::summaries(&self.core, &ids).await?;
        Ok(rows.iter().map(|row| row.to_proto(&people)).collect())
    }

    pub(crate) async fn action_to_proto(&self, row: &ActionRow) -> AppResult<ModerationAction> {
        Ok(self.actions_to_proto(std::slice::from_ref(row)).await?.remove(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_account_actions_have_no_subject_or_case() {
        let (moderator, user) = (Uuid::now_v7(), Uuid::now_v7());
        let row = ActionRow {
            id: Uuid::now_v7(),
            kind: ModerationActionKind::MODERATION_ACTION_KIND_SUSPEND as i32,
            moderator_id: Some(moderator),
            target_user_id: Some(user),
            subject_type: None,
            subject_id: None,
            subject_title: String::new(),
            case_id: None,
            reason: "spam".into(),
            note: "third strike".into(),
            expires_at: Some(Utc::now()),
            created_at: Utc::now(),
        };
        let people = HashMap::from([(user, UserSummary { id: user.to_string(), ..Default::default() })]);
        let action = row.to_proto(&people);
        assert_eq!(action.kind, ModerationActionKind::MODERATION_ACTION_KIND_SUSPEND);
        assert!(action.subject.is_unset() && action.case_id.is_empty());
        assert!(action.moderator.is_unset(), "unknown moderators stay unset");
        assert_eq!(action.target_user.as_option().unwrap().id, user.to_string());
        assert!(action.expires_at.is_set());
    }
}
