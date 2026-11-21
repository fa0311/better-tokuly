//! Feature flags.
//!
//! Flags live in the `feature_flags` table and are evaluated per user: a flag
//! is on when the user is explicitly allowed, or when the flag is enabled and
//! the user falls inside its rollout percentage (stable per user and flag).
//! The table is cached in memory for a few seconds.
use std::{collections::HashMap, sync::Arc, time::Duration};

use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Flag {
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub rollout_percent: i32,
    pub allow_user_ids: Vec<Uuid>,
}

impl Flag {
    pub fn is_on(&self, user_id: Option<Uuid>) -> bool {
        if let Some(user_id) = user_id {
            if self.allow_user_ids.contains(&user_id) {
                return true;
            }
        }
        if !self.enabled {
            return false;
        }
        if self.rollout_percent >= 100 {
            return true;
        }
        match user_id {
            Some(user_id) => bucket(&self.name, user_id) < self.rollout_percent as u32,
            None => false,
        }
    }
}

/// Stable bucket 0..100 for a (flag, user) pair.
fn bucket(name: &str, user_id: Uuid) -> u32 {
    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    hasher.update(user_id.as_bytes());
    let digest = hasher.finalize();
    u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 100
}

#[derive(Clone)]
pub struct Flags {
    db: PgPool,
    cache: moka::future::Cache<(), Arc<Vec<Flag>>>,
}

impl Flags {
    pub fn new(db: PgPool) -> Self {
        Self {
            db,
            cache: moka::future::Cache::builder()
                .time_to_live(Duration::from_secs(10))
                .max_capacity(1)
                .build(),
        }
    }

    async fn all(&self) -> AppResult<Arc<Vec<Flag>>> {
        if let Some(flags) = self.cache.get(&()).await {
            return Ok(flags);
        }
        let flags: Vec<Flag> =
            sqlx::query_as("SELECT name, description, enabled, rollout_percent, allow_user_ids FROM feature_flags ORDER BY name")
                .fetch_all(&self.db)
                .await?;
        let flags = Arc::new(flags);
        self.cache.insert((), flags.clone()).await;
        Ok(flags)
    }

    /// Whether `name` is on for the user. Unknown flags are off.
    pub async fn enabled(&self, name: &str, user_id: Option<Uuid>) -> bool {
        match self.all().await {
            Ok(flags) => flags
                .iter()
                .find(|f| f.name == name)
                .is_some_and(|f| f.is_on(user_id)),
            Err(e) => {
                tracing::warn!(error = %e, "feature flag lookup failed");
                false
            }
        }
    }

    /// Every flag evaluated for one user (sent to clients with their account).
    pub async fn for_user(&self, user_id: Uuid) -> HashMap<String, bool> {
        match self.all().await {
            Ok(flags) => flags
                .iter()
                .map(|f| (f.name.clone(), f.is_on(Some(user_id))))
                .collect(),
            Err(_) => HashMap::new(),
        }
    }

    /// Drops the cached table (call after editing flags).
    pub async fn invalidate(&self) {
        self.cache.invalidate(&()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flag(enabled: bool, percent: i32, allow: Vec<Uuid>) -> Flag {
        Flag {
            name: "x".into(),
            description: String::new(),
            enabled,
            rollout_percent: percent,
            allow_user_ids: allow,
        }
    }

    #[test]
    fn allow_list_wins_over_disabled() {
        let user = Uuid::now_v7();
        assert!(flag(false, 0, vec![user]).is_on(Some(user)));
        assert!(!flag(false, 100, vec![]).is_on(Some(user)));
    }

    #[test]
    fn rollout_is_stable_and_roughly_proportional() {
        let f = flag(true, 30, vec![]);
        let users: Vec<Uuid> = (0..2000).map(|_| Uuid::new_v4()).collect();
        let on = users.iter().filter(|u| f.is_on(Some(**u))).count();
        assert!((450..750).contains(&on), "expected ~600 of 2000, got {on}");
        assert_eq!(f.is_on(Some(users[0])), f.is_on(Some(users[0])));
        assert!(!f.is_on(None), "partial rollouts exclude anonymous callers");
        assert!(flag(true, 100, vec![]).is_on(None));
    }
}
