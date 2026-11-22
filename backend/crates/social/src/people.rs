//! Looking users up on behalf of a viewer: existence checks that respect
//! blocks, display names for server-written text, and online state.
use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use tokuly_core::{prelude::*, presence};
use tokuly_identity::{settings, users};
use tokuly_proto::{common::UserSummary, identity::Audience};

/// The name to use for a user in notifications and activity entries.
pub(crate) fn name_of(user: &UserSummary) -> String {
    if user.display_name.is_empty() { format!("@{}", user.handle) } else { user.display_name.clone() }
}

/// Fails with `USER_NOT_FOUND` unless `user_id` is an active account that has
/// not blocked `viewer`. A block is silent, so to the blocked user the
/// blocker looks exactly like an account that does not exist.
pub(crate) async fn ensure_visible(core: &Core, viewer: Uuid, user_id: Uuid) -> AppResult<()> {
    let visible: bool = sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1 FROM users u
             WHERE u.id = $2 AND u.deleted_at IS NULL AND u.banned_at IS NULL
               AND NOT EXISTS (SELECT 1 FROM user_blocks b WHERE b.blocker_id = $2 AND b.blocked_id = $1))",
    )
    .bind(viewer)
    .bind(user_id)
    .fetch_one(&core.db)
    .await?;
    ensure!(visible, not_found, "USER_NOT_FOUND", "user not found");
    Ok(())
}

/// Resolves a handle (with or without `@`) to a user `viewer` may know about.
pub(crate) async fn visible_by_handle(core: &Core, viewer: Uuid, handle: &str) -> AppResult<Uuid> {
    let user = users::find_by_handle(core, handle).await?.or_not_found("USER_NOT_FOUND", "user not found")?;
    ensure_visible(core, viewer, user.id).await?;
    Ok(user.id)
}

/// An audience stored in privacy settings as the plain enum (unknown numbers
/// read as unspecified, which [`crate::graph::allows`] treats cautiously).
pub(crate) fn audience(value: &buffa::EnumValue<Audience>) -> Audience {
    value.as_known().unwrap_or(Audience::AUDIENCE_UNSPECIFIED)
}

/// Who among a set of users is online, and when the others were last seen.
/// Only users who share their online status appear at all.
#[derive(Debug, Default)]
pub(crate) struct Presence {
    online: HashSet<Uuid>,
    last_seen: HashMap<Uuid, DateTime<Utc>>,
}

impl Presence {
    pub(crate) fn is_online(&self, user_id: Uuid) -> bool {
        self.online.contains(&user_id)
    }

    pub(crate) fn last_seen(&self, user_id: Uuid) -> Option<DateTime<Utc>> {
        self.last_seen.get(&user_id).copied()
    }
}

/// Online state of `user_ids`, honouring each user's `share_online_status`.
/// Presence is decoration on a friend list: when Valkey cannot be read the
/// list is served without it rather than failing.
pub(crate) async fn presence_of(core: &Core, user_ids: &[Uuid]) -> Presence {
    match load_presence(core, user_ids).await {
        Ok(presence) => presence,
        Err(e) => {
            tracing::warn!(error = %e, "presence lookup failed; serving friends without online state");
            Presence::default()
        }
    }
}

async fn load_presence(core: &Core, user_ids: &[Uuid]) -> AppResult<Presence> {
    if user_ids.is_empty() {
        return Ok(Presence::default());
    }
    let privacy = settings::privacy_of_many(core, user_ids).await?;
    let sharing: Vec<Uuid> =
        user_ids.iter().copied().filter(|id| privacy.get(id).is_some_and(|p| p.share_online_status)).collect();
    let online: HashSet<Uuid> = presence::online_among(&core.cache, &sharing).await?.into_iter().collect();
    let offline: Vec<Uuid> = sharing.into_iter().filter(|id| !online.contains(id)).collect();
    let seen = futures::future::join_all(offline.iter().map(|id| presence::last_seen(&core.cache, *id))).await;
    let mut last_seen = HashMap::new();
    for (user_id, seen) in offline.into_iter().zip(seen) {
        if let Some(at) = seen? {
            last_seen.insert(user_id, at);
        }
    }
    Ok(Presence { online, last_seen })
}
