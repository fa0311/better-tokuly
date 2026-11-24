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
        "id, kind, moderator_id, target_user_id, target_hidden, subject_type, subject_id, subject_title, case_id, reason, note, expires_at, created_at"
    };
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ActionRow {
    pub id: Uuid,
    pub kind: i32,
    pub moderator_id: Option<Uuid>,
    pub target_user_id: Option<Uuid>,
    /// The target is the hidden author of anonymous content (`owners.rs`).
    pub target_hidden: bool,
    pub subject_type: Option<i32>,
    pub subject_id: Option<Uuid>,
    pub subject_title: String,
    pub case_id: Option<Uuid>,
    pub reason: String,
    pub note: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// How much of a log entry about anonymous content a view may show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shown {
    /// Everything, the sanctioned account included.
    All,
    /// The entry without the account it was taken against, and without
    /// anything that could be matched against that account's own record:
    /// no expiry, no reason or note, only the day.
    WithoutTarget,
}

impl ActionRow {
    fn people(&self, shown: Shown) -> impl Iterator<Item = Uuid> {
        [self.moderator_id, self.target_user_id.filter(|_| shown == Shown::All)].into_iter().flatten()
    }

    pub fn to_proto(&self, people: &HashMap<Uuid, UserSummary>, shown: Shown) -> ModerationAction {
        let person = |id: Option<Uuid>| id.and_then(|id| people.get(&id).cloned());
        let subject = match (self.subject_type, self.subject_id) {
            (Some(subject_type), Some(id)) => Some(SubjectRef { r#type: subject_type.into(), id: id.to_string(), ..Default::default() }),
            _ => None,
        };
        let opaque = shown == Shown::WithoutTarget;
        let text = |text: &str| if opaque { String::new() } else { text.to_owned() };
        ModerationAction {
            id: self.id.to_string(),
            kind: self.kind.into(),
            moderator: person(self.moderator_id).into(),
            target_user: person(self.target_user_id.filter(|_| shown == Shown::All)).into(),
            target_withheld: shown == Shown::WithoutTarget,
            subject: subject.into(),
            subject_title: self.subject_title.clone(),
            case_id: self.case_id.map(|id| id.to_string()).unwrap_or_default(),
            reason: text(&self.reason),
            note: text(&self.note),
            expires_at: opt_ts(self.expires_at.filter(|_| !opaque)),
            created_at: ts(if opaque { crate::day_of(self.created_at) } else { self.created_at }),
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
    /// The target is the hidden author of anonymous content.
    pub target_hidden: bool,
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
        Self {
            kind,
            moderator_id,
            target_user_id: None,
            target_hidden: false,
            subject: None,
            subject_title: "",
            case_id: None,
            reason: "",
            note: "",
            expires_at: None,
        }
    }

    /// The target is the author of content that hides its author.
    pub fn hidden_target(mut self, hidden: bool) -> Self {
        self.target_hidden = hidden;
        self
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
            "INSERT INTO moderation_actions (kind, moderator_id, target_user_id, subject_type, subject_id, subject_title, case_id, reason, note, expires_at, target_hidden)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING ",
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
        .bind(self.target_hidden)
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

    /// Log entries as the answer to the change that wrote them, with
    /// moderator and target resolved. The author of anonymous content is
    /// not named (`owners.rs`): the moderator acted on the post, not on a
    /// person they were shown.
    pub(crate) async fn actions_to_proto(&self, rows: &[ActionRow]) -> AppResult<Vec<ModerationAction>> {
        let shown: Vec<Shown> = rows.iter().map(|row| if row.target_hidden { Shown::WithoutTarget } else { Shown::All }).collect();
        self.actions_shown(rows, &shown).await
    }

    pub(crate) async fn action_to_proto(&self, row: &ActionRow) -> AppResult<ModerationAction> {
        Ok(self.actions_to_proto(std::slice::from_ref(row)).await?.remove(0))
    }

    /// Log entries as a list or a history shows them. An entry about
    /// anonymous content never names the account it hit, to anyone:
    /// `GetCase` is the one place a hidden author is named (counted and on
    /// record, `owners.rs`). In the history of one account (`about` is that
    /// account) such an entry does not come at all: there the entry itself
    /// would say whose post it was.
    pub(crate) async fn actions_listed(&self, rows: &[ActionRow], about: Option<Uuid>) -> AppResult<Vec<ModerationAction>> {
        let kept: Vec<ActionRow> = rows.iter().filter(|row| !(row.target_hidden && about.is_some())).cloned().collect();
        self.actions_to_proto(&kept).await
    }

    /// The log entries of one case in `GetCase`: an anonymous post's author
    /// is named only when the case view named them (`revealed`).
    pub(crate) async fn case_actions(&self, rows: &[ActionRow], revealed: bool) -> AppResult<Vec<ModerationAction>> {
        let shown: Vec<Shown> = rows.iter().map(|row| if row.target_hidden && !revealed { Shown::WithoutTarget } else { Shown::All }).collect();
        self.actions_shown(rows, &shown).await
    }

    async fn actions_shown(&self, rows: &[ActionRow], shown: &[Shown]) -> AppResult<Vec<ModerationAction>> {
        let ids: Vec<Uuid> = rows.iter().zip(shown).flat_map(|(row, shown)| row.people(*shown)).collect();
        let people = tokuly_identity::summaries(&self.core, &ids).await?;
        Ok(rows.iter().zip(shown).map(|(row, shown)| row.to_proto(&people, *shown)).collect())
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
            target_hidden: false,
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
        let action = row.to_proto(&people, Shown::All);
        assert_eq!(action.kind, ModerationActionKind::MODERATION_ACTION_KIND_SUSPEND);
        assert!(action.subject.is_unset() && action.case_id.is_empty());
        assert!(action.moderator.is_unset(), "unknown moderators stay unset");
        assert_eq!(action.target_user.as_option().unwrap().id, user.to_string());
        assert!(action.expires_at.is_set());
        assert!(!action.target_withheld);

        // The same entry without the account it hit.
        let withheld = row.to_proto(&people, Shown::WithoutTarget);
        assert!(withheld.target_withheld && withheld.target_user.is_unset());
        assert!(!row.people(Shown::WithoutTarget).any(|id| id == user), "a withheld target is not even looked up");
    }
}
