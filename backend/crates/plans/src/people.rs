//! Other people as this domain needs them: who is a friend, who is hidden by
//! a block, how someone is named in a notification, and the two facts about
//! a friendship that only the social domain knows.
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use tokuly_core::prelude::*;
use tokuly_identity::{ProfileExtension, users};
use tokuly_proto::{common::UserSummary, identity::Profile};
use tokuly_social::graph;

use crate::Plans;

/// The name to use for a user in notifications and activity entries.
pub(crate) fn name_of(user: &UserSummary) -> String {
    if user.display_name.is_empty() { format!("@{}", user.handle) } else { user.display_name.clone() }
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

/// The people in a plan or a poll as one viewer may see them. Someone hidden
/// from the viewer by a block (in either direction) is still there — the
/// plan and its cost split are shared — but only as an ID.
pub(crate) struct People {
    summaries: HashMap<Uuid, UserSummary>,
    hidden: HashSet<Uuid>,
}

impl People {
    pub(crate) async fn load(core: &Core, viewer: Uuid, ids: impl IntoIterator<Item = Uuid>) -> AppResult<Self> {
        let hidden: HashSet<Uuid> = graph::blocked_ids_either(core, viewer).await?.into_iter().collect();
        let visible: Vec<Uuid> =
            ids.into_iter().filter(|id| !hidden.contains(id)).collect::<HashSet<_>>().into_iter().collect();
        let summaries = tokuly_identity::summaries(core, &visible).await?;
        Ok(Self { summaries, hidden })
    }

    pub(crate) fn is_hidden(&self, id: Uuid) -> bool {
        self.hidden.contains(&id)
    }

    /// The person's summary, or a placeholder carrying nothing but the ID
    /// when they are hidden from the viewer.
    pub(crate) fn summary(&self, id: Uuid) -> UserSummary {
        if self.is_hidden(id) {
            UserSummary { id: id.to_string(), ..Default::default() }
        } else {
            users::pick(&self.summaries, id)
        }
    }
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
