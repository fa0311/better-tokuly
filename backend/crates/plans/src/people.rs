//! Other people as this domain needs them: who is a friend, how someone is
//! named in a notification, how often one person may
//! invite another, and the two facts about a friendship that only the social
//! domain knows.
//!
//! Everything here that asks the social or identity domain goes through the
//! connection pool, so it is called before a transaction is opened, never
//! inside one: a handler that holds a connection (and a lock) while waiting
//! for a second connection can starve the pool. [`locals`] is the exception
//! made for that case: it reads on the connection the caller already holds.
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use chrono_tz::Tz;
use sqlx::PgConnection;
use tokuly_core::prelude::*;
use tokuly_identity::ProfileExtension;
use tokuly_proto::{common::UserSummary, identity::Profile};
use tokuly_social::graph;

use crate::{Plans, dates};

/// The name to use for a user in notifications and activity entries.
pub(crate) fn name_of(user: &UserSummary) -> String {
    if user.display_name.is_empty() { format!("@{}", user.handle) } else { user.display_name.clone() }
}

/// What a transaction needs to know about a user to write to or about them.
#[derive(Debug, Clone)]
pub(crate) struct Local {
    /// As [`name_of`] would give it.
    pub name: String,
    pub zone: Tz,
}

/// Name and time zone of users, read on a connection that is already in use
/// (inside a transaction, where asking the pool for a second connection is
/// not an option). Unknown users are absent.
pub(crate) async fn locals(conn: &mut PgConnection, user_ids: &[Uuid]) -> AppResult<HashMap<Uuid, Local>> {
    let rows: Vec<(Uuid, String, String, String, bool)> = sqlx::query_as(
        "SELECT id, display_name, handle::text, time_zone, deleted_at IS NOT NULL FROM users WHERE id = ANY($1)",
    )
    .bind(user_ids)
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, display_name, handle, time_zone, deleted)| {
            // An erased account is named the way identity's placeholder summary names it.
            let name = match (deleted, display_name.is_empty()) {
                (true, _) => "@deleted".to_owned(),
                (false, true) => format!("@{handle}"),
                (false, false) => display_name,
            };
            (id, Local { name, zone: dates::zone(&time_zone) })
        })
        .collect())
}

/// [`locals`] for one user's name.
pub(crate) async fn name_on(conn: &mut PgConnection, user_id: Uuid) -> AppResult<String> {
    Ok(locals(conn, &[user_id])
        .await?
        .remove(&user_id)
        .map(|local| local.name)
        .unwrap_or_else(|| "@deleted".to_owned()))
}

/// The error for anything aimed at "a friend" who is not one. It also covers
/// unknown, deleted and blocked users, none of whom can be a friend.
pub(crate) fn not_friends() -> AppError {
    AppError::precondition("NOT_FRIENDS", "you are not friends with this user")
}

/// Fails with `NOT_FRIENDS` unless every one of `others` is a friend of `user_id`.
pub(crate) async fn require_friends(core: &Core, user_id: Uuid, others: &[Uuid]) -> AppResult<()> {
    match others {
        [] => Ok(()),
        [other] => {
            if graph::are_friends(core, user_id, *other).await? {
                Ok(())
            } else {
                Err(not_friends())
            }
        }
        _ => {
            let friends: HashSet<Uuid> = graph::friend_ids(core, user_id).await?.into_iter().collect();
            if others.iter().all(|other| friends.contains(other)) { Ok(()) } else { Err(not_friends()) }
        }
    }
}

/// Invitations one person may send the same friend in a day, plans and date
/// polls together. Every invitation notifies, and inviting, removing and
/// inviting again would otherwise be a way to pester someone.
const INVITATIONS_PER_FRIEND: u32 = 10;
/// Invitations one person may send in an hour, to anyone: every invitee of
/// `CreatePlan`, `InviteToPlan`, `CreatePoll` and `InviteToPoll` counts.
const INVITATIONS_PER_HOUR: u32 = 300;

/// Counts one invitation from `from` to each of `to` against the sender's
/// hourly limit and the daily limit per pair (`RATE_LIMITED` when either is
/// used up).
pub(crate) async fn limit_invitations(core: &Core, from: Uuid, to: &[Uuid]) -> AppResult<()> {
    for friend in to {
        core.cache
            .rate_limit(&format!("plans:invitations:{from}"), INVITATIONS_PER_HOUR, Duration::from_secs(3600))
            .await?;
        core.cache
            .rate_limit(
                &format!("plans:invite:{from}:{friend}"),
                INVITATIONS_PER_FRIEND,
                Duration::from_secs(24 * 3600),
            )
            .await?;
    }
    Ok(())
}

/// What the social domain knows about a friendship, as one side sees it.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Friendship {
    /// When the two became friends; `None` when they are not friends.
    pub since: Option<Time>,
    /// The friend is one of the user's best friends (ranked or starred).
    pub best_friend: bool,
}

/// How long an answer to "is X one of Y's best friends" is reused.
const BEST_FRIEND_MEMORY: Duration = Duration::from_secs(300);

fn best_friend_key(owner: Uuid, friend: Uuid) -> String {
    format!("plans:best-friend:{}:{}", owner.simple(), friend.simple())
}

impl Plans {
    /// `user`'s friendship with `friend`: since when, and whether `friend` is
    /// one of `user`'s best friends.
    ///
    /// `tokuly_social::graph` has no function for either fact. What the
    /// social domain does offer is its part of a profile: shown `friend`'s
    /// profile, `user` gets `friends_since` and `best_friend` filled in. So
    /// that part is asked for, on an empty profile. It takes a handful of
    /// queries per question, which is why callers ask about one pair at a
    /// time and [`Self::is_best_friend`] remembers the answer for a while.
    pub(crate) async fn friendship(&self, user: Uuid, friend: Uuid) -> AppResult<Friendship> {
        let mut profile = Profile::default();
        ProfileExtension::enrich_profile(&self.social, Some(user), friend, &mut profile).await?;
        Ok(Friendship { since: profile.friends_since.as_option().map(from_ts), best_friend: profile.best_friend })
    }

    /// Whether `friend` is one of `owner`'s best friends. The two must be
    /// friends right now (checked every time); the ranking itself is
    /// remembered for a few minutes.
    pub(crate) async fn is_best_friend(&self, owner: Uuid, friend: Uuid) -> AppResult<bool> {
        if !graph::are_friends(&self.core, owner, friend).await? {
            return Ok(false);
        }
        let key = best_friend_key(owner, friend);
        if let Some(known) = self.core.cache.get_string(&key).await? {
            return Ok(known == "1");
        }
        let best_friend = self.friendship(owner, friend).await?.best_friend;
        self.core.cache.set_string(&key, if best_friend { "1" } else { "0" }, Some(BEST_FRIEND_MEMORY)).await?;
        Ok(best_friend)
    }

    /// Forgets what was remembered about two people's best-friend ranking
    /// (when their friendship ends).
    pub(crate) async fn forget_best_friend(&self, a: Uuid, b: Uuid) {
        for key in [best_friend_key(a, b), best_friend_key(b, a)] {
            if let Err(e) = self.core.cache.del(&key).await {
                tracing::warn!(error = %e, "could not drop a remembered best-friend answer");
            }
        }
    }
}
