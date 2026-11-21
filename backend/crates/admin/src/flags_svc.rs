//! `FeatureFlagService`: editing the `feature_flags` table that
//! `tokuly_core::flags` evaluates. Every RPC needs the `admin` role.
//!
//! Core caches the table for a few seconds per process; writes here drop
//! this process's cache at once and other instances catch up when theirs
//! expires.
use chrono::{DateTime, Utc};
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use sqlx::PgConnection;
use tokuly_core::{flags::Flag, prelude::*};
use tokuly_proto::admin::*;

use crate::{
    Admin,
    audit::{Audit, escape_like},
    staff::Staff,
};

const DESCRIPTION_MAX_CHARS: usize = 500;
const ALLOW_LIST_MAX: usize = 1000;

#[derive(Debug, Clone, sqlx::FromRow)]
struct FlagRow {
    name: String,
    description: String,
    enabled: bool,
    rollout_percent: i32,
    allow_user_ids: Vec<Uuid>,
    updated_at: DateTime<Utc>,
}

impl FlagRow {
    /// The flag's state as recorded in the audit log.
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "description": self.description,
            "enabled": self.enabled,
            "rollout_percent": self.rollout_percent,
            "allow_user_ids": self.allow_user_ids,
        })
    }

    /// The form core evaluates.
    fn to_core(&self) -> Flag {
        Flag {
            name: self.name.clone(),
            description: self.description.clone(),
            enabled: self.enabled,
            rollout_percent: self.rollout_percent,
            allow_user_ids: self.allow_user_ids.clone(),
        }
    }
}

macro_rules! flag_columns {
    () => {
        "name, description, enabled, rollout_percent, allow_user_ids, updated_at"
    };
}

/// A flag name: 2-64 characters of `a-z 0-9 _ - .`, starting with a letter.
fn validate_flag_name(input: &str) -> AppResult<String> {
    let name = input.trim();
    let valid = (2..=64).contains(&name.len())
        && name.as_bytes()[0].is_ascii_lowercase()
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.'));
    ensure!(valid, invalid, "FLAG_NAME_INVALID", "flag names are 2-64 characters of a-z, 0-9, '_', '-' or '.', starting with a letter");
    Ok(name.to_owned())
}

fn validate_description(input: &str) -> AppResult<String> {
    let description = input.trim();
    ensure!(
        description.chars().count() <= DESCRIPTION_MAX_CHARS,
        invalid,
        "FLAG_DESCRIPTION_TOO_LONG",
        "descriptions are at most {DESCRIPTION_MAX_CHARS} characters"
    );
    Ok(description.to_owned())
}

fn validate_rollout(percent: i32) -> AppResult<i32> {
    ensure!((0..=100).contains(&percent), invalid, "ROLLOUT_INVALID", "rollout percent must be between 0 and 100");
    Ok(percent)
}

/// Why `flag` is on or off for `user_id`.
fn decide(flag: &Flag, user_id: Uuid) -> FlagDecision {
    if flag.allow_user_ids.contains(&user_id) {
        FlagDecision::FLAG_DECISION_ALLOW_LISTED
    } else if !flag.enabled {
        FlagDecision::FLAG_DECISION_DISABLED
    } else if flag.rollout_percent >= 100 {
        FlagDecision::FLAG_DECISION_EVERYONE
    } else if flag.is_on(Some(user_id)) {
        FlagDecision::FLAG_DECISION_IN_ROLLOUT
    } else {
        FlagDecision::FLAG_DECISION_OUT_OF_ROLLOUT
    }
}

/// The allow-list after adding and removing users: order kept, no duplicates.
fn edit_allow_list(current: &[Uuid], add: &[Uuid], remove: &[Uuid]) -> Vec<Uuid> {
    let mut list: Vec<Uuid> = Vec::with_capacity(current.len() + add.len());
    for id in current.iter().chain(add) {
        if !remove.contains(id) && !list.contains(id) {
            list.push(*id);
        }
    }
    list
}

impl Admin {
    /// Rejects allow-lists that are too long or name accounts that do not exist.
    async fn check_allow_list(&self, user_ids: &[Uuid]) -> AppResult<()> {
        ensure!(user_ids.len() <= ALLOW_LIST_MAX, invalid, "ALLOW_LIST_TOO_LONG", "an allow-list holds at most {ALLOW_LIST_MAX} users");
        let existing: i64 =
            sqlx::query_scalar("SELECT count(*) FROM users WHERE id = ANY($1) AND deleted_at IS NULL").bind(user_ids).fetch_one(&self.core.db).await?;
        ensure!(existing as usize == user_ids.len(), not_found, "USER_NOT_FOUND", "an allow-listed user does not exist");
        Ok(())
    }

    async fn lock_flag(&self, conn: &mut PgConnection, name: &str) -> AppResult<FlagRow> {
        sqlx::query_as(concat!("SELECT ", flag_columns!(), " FROM feature_flags WHERE name = $1 FOR UPDATE"))
            .bind(name)
            .fetch_optional(conn)
            .await?
            .or_not_found("FLAG_NOT_FOUND", "feature flag not found")
    }

    async fn flags_to_proto(&self, rows: Vec<FlagRow>) -> AppResult<Vec<FeatureFlag>> {
        let ids: Vec<Uuid> = rows.iter().flat_map(|r| r.allow_user_ids.iter().copied()).collect();
        let people = tokuly_identity::summaries(&self.core, &ids).await?;
        Ok(rows
            .into_iter()
            .map(|row| FeatureFlag {
                allow_users: row.allow_user_ids.iter().filter_map(|id| people.get(id).cloned()).collect(),
                name: row.name,
                description: row.description,
                enabled: row.enabled,
                rollout_percent: row.rollout_percent,
                updated_at: ts(row.updated_at),
                ..Default::default()
            })
            .collect())
    }

    async fn flag_to_proto(&self, row: FlagRow) -> AppResult<FeatureFlag> {
        Ok(self.flags_to_proto(vec![row]).await?.remove(0))
    }
}

impl FeatureFlagService for Admin {
    async fn list_flags(&self, ctx: RequestContext, request: ServiceRequest<'_, ListFlagsRequest>) -> ServiceResult<ListFlagsResponse> {
        handle(async {
            Staff::admin(&ctx)?;
            let page = Page::from_view(request.page.as_option(), 200);
            let pattern = format!("%{}%", escape_like(request.query.trim()));
            let mut rows: Vec<FlagRow> = sqlx::query_as(concat!(
                "SELECT ",
                flag_columns!(),
                " FROM feature_flags WHERE name ILIKE $1 AND ($2::text IS NULL OR name > $2) ORDER BY name LIMIT $3"
            ))
            .bind(pattern)
            .bind(page.decode::<String>()?)
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let info = page.finish(&mut rows, |r| r.name.clone());
            ok(ListFlagsResponse { flags: self.flags_to_proto(rows).await?, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn create_flag(&self, ctx: RequestContext, request: ServiceRequest<'_, CreateFlagRequest>) -> ServiceResult<CreateFlagResponse> {
        handle(async {
            let staff = Staff::admin(&ctx)?;
            let request = request.to_owned_message();
            let name = validate_flag_name(&request.name)?;
            let description = validate_description(&request.description)?;
            let rollout_percent = validate_rollout(request.rollout_percent.unwrap_or(100))?;
            let allow_user_ids = edit_allow_list(&[], &parse_ids(&request.allow_user_ids)?, &[]);
            self.check_allow_list(&allow_user_ids).await?;

            let mut tx = self.core.db.begin().await?;
            let created: Option<FlagRow> = sqlx::query_as(concat!(
                "INSERT INTO feature_flags (name, description, enabled, rollout_percent, allow_user_ids) VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (name) DO NOTHING RETURNING ",
                flag_columns!()
            ))
            .bind(&name)
            .bind(&description)
            .bind(request.enabled)
            .bind(rollout_percent)
            .bind(&allow_user_ids)
            .fetch_optional(&mut *tx)
            .await?;
            let Some(flag) = created else {
                bail!(already_exists, "FLAG_EXISTS", "a flag named {name} already exists");
            };
            Audit::by(&staff, "flag.created").flag(&flag.name).after(flag.snapshot()).record(&mut *tx).await?;
            tx.commit().await?;
            self.core.flags.invalidate().await;
            ok(CreateFlagResponse { flag: self.flag_to_proto(flag).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn update_flag(&self, ctx: RequestContext, request: ServiceRequest<'_, UpdateFlagRequest>) -> ServiceResult<UpdateFlagResponse> {
        handle(async {
            let staff = Staff::admin(&ctx)?;
            let request = request.to_owned_message();
            let description = request.description.as_deref().map(validate_description).transpose()?;
            let rollout_percent = request.rollout_percent.map(validate_rollout).transpose()?;
            let add = parse_ids(&request.add_allow_user_ids)?;
            let remove = parse_ids(&request.remove_allow_user_ids)?;

            // Only newly added users must exist: accounts deleted since they
            // were allow-listed must not block unrelated edits.
            self.check_allow_list(&edit_allow_list(&[], &add, &[])).await?;

            let mut tx = self.core.db.begin().await?;
            let before = self.lock_flag(&mut tx, request.name.trim()).await?;
            let allow_user_ids = edit_allow_list(&before.allow_user_ids, &add, &remove);
            ensure!(allow_user_ids.len() <= ALLOW_LIST_MAX, invalid, "ALLOW_LIST_TOO_LONG", "an allow-list holds at most {ALLOW_LIST_MAX} users");
            let after: FlagRow = sqlx::query_as(concat!(
                "UPDATE feature_flags
                 SET description = COALESCE($2, description), enabled = COALESCE($3, enabled),
                     rollout_percent = COALESCE($4, rollout_percent), allow_user_ids = $5, updated_at = now()
                 WHERE name = $1 RETURNING ",
                flag_columns!()
            ))
            .bind(&before.name)
            .bind(description)
            .bind(request.enabled)
            .bind(rollout_percent)
            .bind(&allow_user_ids)
            .fetch_one(&mut *tx)
            .await?;
            Audit::by(&staff, "flag.updated").flag(&after.name).before(before.snapshot()).after(after.snapshot()).record(&mut *tx).await?;
            tx.commit().await?;
            self.core.flags.invalidate().await;
            ok(UpdateFlagResponse { flag: self.flag_to_proto(after).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_flag(&self, ctx: RequestContext, request: ServiceRequest<'_, DeleteFlagRequest>) -> ServiceResult<DeleteFlagResponse> {
        handle(async {
            let staff = Staff::admin(&ctx)?;
            let mut tx = self.core.db.begin().await?;
            let flag = self.lock_flag(&mut tx, request.name.trim()).await?;
            sqlx::query("DELETE FROM feature_flags WHERE name = $1").bind(&flag.name).execute(&mut *tx).await?;
            Audit::by(&staff, "flag.deleted").flag(&flag.name).before(flag.snapshot()).record(&mut *tx).await?;
            tx.commit().await?;
            self.core.flags.invalidate().await;
            ok(DeleteFlagResponse::default())
        })
        .await
    }

    async fn evaluate_flags(&self, ctx: RequestContext, request: ServiceRequest<'_, EvaluateFlagsRequest>) -> ServiceResult<EvaluateFlagsResponse> {
        handle(async {
            Staff::admin(&ctx)?;
            let user_id = parse_id(request.user_id)?;
            self.existing_user(user_id).await?.or_not_found("USER_NOT_FOUND", "user not found")?;
            // Read straight from the table so a preview right after an edit is never stale.
            let rows: Vec<FlagRow> = sqlx::query_as(concat!("SELECT ", flag_columns!(), " FROM feature_flags ORDER BY name")).fetch_all(&self.core.db).await?;
            let evaluations = rows
                .iter()
                .map(|row| {
                    let flag = row.to_core();
                    FlagEvaluation { name: row.name.clone(), on: flag.is_on(Some(user_id)), decision: decide(&flag, user_id).into(), ..Default::default() }
                })
                .collect();
            ok(EvaluateFlagsResponse { evaluations, ..Default::default() })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_names_follow_the_key_format() {
        for good in ["ab", "new_map", "chat.ai-assist", "v2"] {
            assert_eq!(validate_flag_name(&format!(" {good} ")).unwrap(), good);
        }
        for bad in ["", "a", "1st", "Has_Upper", "with space", "日本語", "_lead", &"x".repeat(65)] {
            assert_eq!(validate_flag_name(bad).unwrap_err().reason, "FLAG_NAME_INVALID", "{bad:?}");
        }
    }

    #[test]
    fn rollout_and_description_limits() {
        assert_eq!(validate_rollout(0).unwrap(), 0);
        assert_eq!(validate_rollout(100).unwrap(), 100);
        assert_eq!(validate_rollout(101).unwrap_err().reason, "ROLLOUT_INVALID");
        assert_eq!(validate_rollout(-1).unwrap_err().reason, "ROLLOUT_INVALID");
        assert_eq!(validate_description(&"あ".repeat(501)).unwrap_err().reason, "FLAG_DESCRIPTION_TOO_LONG");
    }

    #[test]
    fn allow_list_edits_keep_order_and_drop_duplicates() {
        let (a, b, c) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        assert_eq!(edit_allow_list(&[a, b], &[b, c, c], &[]), vec![a, b, c]);
        assert_eq!(edit_allow_list(&[a, b], &[c], &[a]), vec![b, c]);
        // Removing wins when a user is in both lists.
        assert_eq!(edit_allow_list(&[a], &[b], &[b]), vec![a]);
    }

    #[test]
    fn decisions_explain_the_evaluation() {
        let user = Uuid::now_v7();
        let flag = |enabled: bool, percent: i32, allow: Vec<Uuid>| Flag {
            name: "x".into(),
            description: String::new(),
            enabled,
            rollout_percent: percent,
            allow_user_ids: allow,
        };
        assert_eq!(decide(&flag(false, 0, vec![user]), user), FlagDecision::FLAG_DECISION_ALLOW_LISTED);
        assert_eq!(decide(&flag(false, 100, vec![]), user), FlagDecision::FLAG_DECISION_DISABLED);
        assert_eq!(decide(&flag(true, 100, vec![]), user), FlagDecision::FLAG_DECISION_EVERYONE);
        assert_eq!(decide(&flag(true, 0, vec![]), user), FlagDecision::FLAG_DECISION_OUT_OF_ROLLOUT);
        // A partial rollout agrees with core's own evaluation either way.
        let partial = flag(true, 50, vec![]);
        let expected = if partial.is_on(Some(user)) { FlagDecision::FLAG_DECISION_IN_ROLLOUT } else { FlagDecision::FLAG_DECISION_OUT_OF_ROLLOUT };
        assert_eq!(decide(&partial, user), expected);
    }
}
