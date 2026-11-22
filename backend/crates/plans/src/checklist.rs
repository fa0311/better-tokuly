//! The checklist of a plan: things to bring (持ち物) and things to do
//! (やること) that anyone going can add, claim and tick off.
use std::time::Duration;

use buffa::Enumeration as _;
use sqlx::PgConnection;
use tokuly_core::prelude::*;
use tokuly_proto::plans::{plans_event::Event as Push, *};

use crate::{
    Plans, input,
    plan::{Access, load_access, participant_ids},
};

const MAX_TEXT: usize = 100;
const MAX_ITEMS: i64 = 100;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ChecklistRow {
    pub id: Uuid,
    pub plan_id: Uuid,
    pub kind: i16,
    pub text: String,
    pub created_by: Uuid,
    pub claimed_by: Option<Uuid>,
    pub done: bool,
    pub done_by: Option<Uuid>,
    pub done_at: Option<Time>,
    pub created_at: Time,
}

macro_rules! checklist_columns {
    () => {
        "id, plan_id, kind, text, created_by, claimed_by, done, done_by, done_at, created_at"
    };
}

impl ChecklistRow {
    pub(crate) fn to_proto(&self) -> ChecklistItem {
        let id_or_empty = |id: Option<Uuid>| id.map(|id| id.to_string()).unwrap_or_default();
        ChecklistItem {
            id: self.id.to_string(),
            plan_id: self.plan_id.to_string(),
            kind: ChecklistKind::from_i32(self.kind as i32).unwrap_or(ChecklistKind::CHECKLIST_KIND_UNSPECIFIED).into(),
            text: self.text.clone(),
            created_by_user_id: self.created_by.to_string(),
            claimed_by_user_id: id_or_empty(self.claimed_by),
            done: self.done,
            done_by_user_id: id_or_empty(self.done_by),
            done_at: opt_ts(self.done_at),
            created_at: ts(self.created_at),
            ..Default::default()
        }
    }
}

/// The checklists of several plans, each in the order the items were added.
pub(crate) async fn items_of(core: &Core, plan_ids: &[Uuid]) -> AppResult<Vec<ChecklistRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        checklist_columns!(),
        " FROM plan_checklist_items WHERE plan_id = ANY($1) ORDER BY plan_id, id"
    ))
    .bind(plan_ids)
    .fetch_all(&core.db)
    .await?)
}

fn item_text(input: &str) -> AppResult<String> {
    input::required_line(input, MAX_TEXT, "TEXT_REQUIRED", "TEXT_TOO_LONG")
}

/// Bring or to-do; anything else is not a kind of item.
fn item_kind(kind: ChecklistKind) -> i16 {
    match kind {
        ChecklistKind::CHECKLIST_KIND_TODO => ChecklistKind::CHECKLIST_KIND_TODO as i32 as i16,
        _ => ChecklistKind::CHECKLIST_KIND_BRING as i32 as i16,
    }
}

/// What a caller wants changed about an item.
pub(crate) struct ItemChange<'a> {
    pub text: Option<&'a str>,
    pub kind: Option<ChecklistKind>,
    pub claimed: Option<bool>,
    pub done: Option<bool>,
}

impl Plans {
    /// An item together with the caller's access to its plan. Someone who is
    /// not in the plan gets `ITEM_NOT_FOUND`, like for an item that does not exist.
    async fn checklist_item(
        &self,
        conn: &mut PgConnection,
        item_id: Uuid,
        user_id: Uuid,
    ) -> AppResult<(ChecklistRow, Access)> {
        let not_found = || AppError::not_found("ITEM_NOT_FOUND", "checklist item not found");
        let plan_id: Uuid = sqlx::query_scalar("SELECT plan_id FROM plan_checklist_items WHERE id = $1")
            .bind(item_id)
            .fetch_optional(&mut *conn)
            .await?
            .ok_or_else(not_found)?;
        // The plan's lock also serializes changes to its checklist.
        let access = load_access(conn, plan_id, user_id, true)
            .await
            .map_err(|e| if e.is_not_found() { not_found() } else { e })?;
        let item = sqlx::query_as(concat!("SELECT ", checklist_columns!(), " FROM plan_checklist_items WHERE id = $1"))
            .bind(item_id)
            .fetch_optional(&mut *conn)
            .await?
            .ok_or_else(not_found)?;
        Ok((item, access))
    }

    async fn push_checklist(
        &self,
        plan_id: Uuid,
        recipients: Vec<Uuid>,
        item: Option<&ChecklistRow>,
        removed: Option<Uuid>,
    ) {
        let event = ChecklistChanged {
            plan_id: plan_id.to_string(),
            item: item.map(ChecklistRow::to_proto).into(),
            removed_item_id: removed.map(|id| id.to_string()).unwrap_or_default(),
            ..Default::default()
        };
        self.push(recipients, Push::ChecklistChanged(Box::new(event))).await;
    }

    /// Adds something to bring or to do.
    pub(crate) async fn add_checklist_item(
        &self,
        user_id: Uuid,
        plan_id: Uuid,
        kind: ChecklistKind,
        text: &str,
    ) -> AppResult<ChecklistItem> {
        let text = item_text(text)?;
        self.core.cache.rate_limit(&format!("plans:checklist:{user_id}"), 300, Duration::from_secs(3600)).await?;
        let mut tx = self.core.db.begin().await?;
        let access = load_access(&mut tx, plan_id, user_id, true).await?;
        access.ensure_going()?;
        access.ensure_active()?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM plan_checklist_items WHERE plan_id = $1")
            .bind(plan_id)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(count < MAX_ITEMS, precondition, "TOO_MANY_ITEMS", "a checklist holds at most {MAX_ITEMS} items");
        let item: ChecklistRow = sqlx::query_as(concat!(
            "INSERT INTO plan_checklist_items (plan_id, kind, text, created_by) VALUES ($1, $2, $3, $4) RETURNING ",
            checklist_columns!()
        ))
        .bind(plan_id)
        .bind(item_kind(kind))
        .bind(&text)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        let everyone = participant_ids(&mut tx, plan_id).await?;
        tx.commit().await?;
        self.push_checklist(plan_id, everyone, Some(&item), None).await;
        Ok(item.to_proto())
    }

    /// Rewrites, claims or ticks off an item.
    pub(crate) async fn update_checklist_item(
        &self,
        user_id: Uuid,
        item_id: Uuid,
        change: ItemChange<'_>,
    ) -> AppResult<ChecklistItem> {
        let mut tx = self.core.db.begin().await?;
        let (mut item, access) = self.checklist_item(&mut tx, item_id, user_id).await?;
        access.ensure_going()?;
        let may_manage = item.created_by == user_id || access.is_organizer();

        if change.text.is_some() || change.kind.is_some() {
            ensure!(may_manage, denied, "NOT_ALLOWED", "only the author of an item or a host can rewrite it");
            if let Some(text) = change.text {
                item.text = item_text(text)?;
            }
            if let Some(kind) = change.kind {
                item.kind = item_kind(kind);
            }
        }
        match change.claimed {
            Some(true) => {
                ensure!(
                    item.claimed_by.is_none_or(|by| by == user_id),
                    precondition,
                    "ALREADY_CLAIMED",
                    "someone else already takes care of this"
                );
                item.claimed_by = Some(user_id);
            }
            Some(false) => {
                let may_release = item.claimed_by.is_none_or(|by| by == user_id) || access.is_organizer();
                ensure!(
                    may_release,
                    denied,
                    "NOT_ALLOWED",
                    "only the person who claimed an item or a host can release it"
                );
                item.claimed_by = None;
            }
            None => {}
        }
        match change.done {
            Some(true) if !item.done => {
                item.done = true;
                item.done_by = Some(user_id);
                item.done_at = Some(now());
            }
            Some(false) => {
                item.done = false;
                item.done_by = None;
                item.done_at = None;
            }
            _ => {}
        }
        sqlx::query("UPDATE plan_checklist_items SET kind = $2, text = $3, claimed_by = $4, done = $5, done_by = $6, done_at = $7 WHERE id = $1")
            .bind(item.id)
            .bind(item.kind)
            .bind(&item.text)
            .bind(item.claimed_by)
            .bind(item.done)
            .bind(item.done_by)
            .bind(item.done_at)
            .execute(&mut *tx)
            .await?;
        let everyone = participant_ids(&mut tx, item.plan_id).await?;
        tx.commit().await?;
        self.push_checklist(item.plan_id, everyone, Some(&item), None).await;
        Ok(item.to_proto())
    }

    /// Deletes an item (its author or a host).
    pub(crate) async fn remove_checklist_item(&self, user_id: Uuid, item_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let (item, access) = self.checklist_item(&mut tx, item_id, user_id).await?;
        ensure!(
            item.created_by == user_id || access.is_organizer(),
            denied,
            "NOT_ALLOWED",
            "only the author of an item or a host can delete it"
        );
        sqlx::query("DELETE FROM plan_checklist_items WHERE id = $1").bind(item_id).execute(&mut *tx).await?;
        let everyone = participant_ids(&mut tx, item.plan_id).await?;
        tx.commit().await?;
        self.push_checklist(item.plan_id, everyone, None, Some(item_id)).await;
        Ok(())
    }
}
