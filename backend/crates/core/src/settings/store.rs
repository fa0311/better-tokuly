//! The console layer: `server_settings` and its history `server_setting_changes`.
//!
//! Secrets are sealed with the core keys and only opened into the
//! process's settings; what leaves this module about them is whether one
//! is set and its last four characters.
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::{Def, Registry};
use crate::{error::AppResult, keys::Keys};

/// The console's view of a stored secret.
#[derive(Debug, Clone, Default)]
pub struct SecretInfo {
    pub last4: String,
}

#[derive(sqlx::FromRow)]
struct Row {
    section: String,
    key: String,
    value: Option<String>,
    sealed: Option<Vec<u8>>,
}

fn seal_label(def: &Def) -> String {
    format!("settings/{}.{}", def.section, def.key)
}

/// The console layer by argument ID. Rows of settings this build does not
/// know, or whose value it does not accept, are left out with a warning
/// (they may come from another version); a missing table (before the first
/// migration) is an empty layer.
pub async fn load(db: &PgPool, registry: &Registry, keys: &Keys) -> anyhow::Result<HashMap<String, String>> {
    let rows = match sqlx::query_as::<_, Row>("SELECT section, key, value, sealed FROM server_settings").fetch_all(db).await {
        Ok(rows) => rows,
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("42P01") => return Ok(HashMap::new()),
        Err(e) => return Err(anyhow::anyhow!("cannot read the console settings: {e}")),
    };
    let mut values = HashMap::new();
    for row in rows {
        let Some(def) = registry.get(&row.section, &row.key) else {
            tracing::warn!(section = row.section, key = row.key, "settings: the console has a value for an unknown setting; ignored");
            continue;
        };
        let value = match (row.value, row.sealed) {
            (Some(value), _) => value,
            (None, Some(sealed)) => match keys.open(&seal_label(def), &sealed).and_then(|v| String::from_utf8(v).ok()) {
                Some(value) => value,
                None => {
                    tracing::warn!(setting = def.name(), "settings: a console secret does not open with this TOKULY_SECRET; ignored");
                    continue;
                }
            },
            (None, None) => continue,
        };
        match def.check_value(&value) {
            Ok(value) => {
                values.insert(def.id.clone(), value);
            }
            Err(e) => tracing::warn!(setting = def.name(), error = e, "settings: a console value is not accepted; ignored"),
        }
    }
    Ok(values)
}

/// The last four characters of the secrets the console stores, by argument ID.
pub async fn secrets(db: &PgPool, registry: &Registry) -> AppResult<HashMap<String, SecretInfo>> {
    let rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT section, key, last4 FROM server_settings WHERE sealed IS NOT NULL").fetch_all(db).await?;
    Ok(rows
        .into_iter()
        .filter_map(|(section, key, last4)| registry.get(&section, &key).map(|d| (d.id.clone(), SecretInfo { last4 })))
        .collect())
}

/// A recorded change: the history row's values (secrets as a placeholder).
#[derive(Debug, Clone)]
pub struct Change {
    pub old: Option<String>,
    pub new: Option<String>,
}

/// How the history writes a secret's value.
const SECRET_MARK: &str = "••••";

/// Sets (`Some`) or removes (`None`) the console value of `def` and records
/// the change, on the caller's transaction. `value` has passed
/// [`Def::check_value`]. Returns `None` when nothing changed.
pub async fn write(tx: &mut PgConnection, keys: &Keys, def: &Def, value: Option<&str>, actor: Uuid) -> AppResult<Option<Change>> {
    // One writer per setting at a time, so the history's "old" is the real one.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('server_settings:' || $1 || '.' || $2))")
        .bind(def.section)
        .bind(&def.key)
        .execute(&mut *tx)
        .await?;
    let old: Option<(Option<String>, Option<Vec<u8>>)> =
        sqlx::query_as("SELECT value, sealed FROM server_settings WHERE section = $1 AND key = $2")
            .bind(def.section)
            .bind(&def.key)
            .fetch_optional(&mut *tx)
            .await?;
    let old_value: Option<String> = match &old {
        None => None,
        Some((Some(value), _)) => Some(value.clone()),
        Some((None, Some(sealed))) => Some(keys.open(&seal_label(def), sealed).and_then(|v| String::from_utf8(v).ok()).unwrap_or_default()),
        Some((None, None)) => None,
    };
    if old_value.as_deref() == value {
        return Ok(None);
    }
    match value {
        Some(value) if def.secret => {
            let last4: String = value.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
            sqlx::query(
                "INSERT INTO server_settings (section, key, value, sealed, last4, updated_by) VALUES ($1, $2, NULL, $3, $4, $5)
                 ON CONFLICT (section, key) DO UPDATE SET value = NULL, sealed = EXCLUDED.sealed, last4 = EXCLUDED.last4,
                     updated_by = EXCLUDED.updated_by, updated_at = now()",
            )
            .bind(def.section)
            .bind(&def.key)
            .bind(keys.seal(&seal_label(def), value.as_bytes()))
            .bind(last4)
            .bind(actor)
            .execute(&mut *tx)
            .await?;
        }
        Some(value) => {
            sqlx::query(
                "INSERT INTO server_settings (section, key, value, sealed, last4, updated_by) VALUES ($1, $2, $3, NULL, '', $4)
                 ON CONFLICT (section, key) DO UPDATE SET value = EXCLUDED.value, sealed = NULL, last4 = '',
                     updated_by = EXCLUDED.updated_by, updated_at = now()",
            )
            .bind(def.section)
            .bind(&def.key)
            .bind(value)
            .bind(actor)
            .execute(&mut *tx)
            .await?;
        }
        None => {
            sqlx::query("DELETE FROM server_settings WHERE section = $1 AND key = $2").bind(def.section).bind(&def.key).execute(&mut *tx).await?;
        }
    }
    let shown = |v: Option<&str>| v.map(|v| if def.secret { SECRET_MARK.to_owned() } else { v.to_owned() });
    let change = Change { old: shown(old_value.as_deref()), new: shown(value) };
    sqlx::query("INSERT INTO server_setting_changes (section, key, old_value, new_value, secret, actor_id) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(def.section)
        .bind(&def.key)
        .bind(&change.old)
        .bind(&change.new)
        .bind(def.secret)
        .bind(actor)
        .execute(&mut *tx)
        .await?;
    Ok(Some(change))
}

/// One entry of the history.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ChangeRow {
    pub id: Uuid,
    pub section: String,
    pub key: String,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub secret: bool,
    pub actor_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// The history, newest first, of one setting (`Some`) or of all of them;
/// `before` continues after the given entry.
pub async fn changes(db: &PgPool, setting: Option<(&str, &str)>, before: Option<Uuid>, limit: i64) -> AppResult<Vec<ChangeRow>> {
    let (section, key) = setting.unzip();
    Ok(sqlx::query_as::<_, ChangeRow>(
        "SELECT id, section, key, old_value, new_value, secret, actor_id, created_at FROM server_setting_changes
         WHERE ($1::text IS NULL OR (section = $1 AND key = $2)) AND ($3::uuid IS NULL OR id < $3)
         ORDER BY id DESC LIMIT $4",
    )
    .bind(section)
    .bind(key)
    .bind(before)
    .bind(limit)
    .fetch_all(db)
    .await?)
}
