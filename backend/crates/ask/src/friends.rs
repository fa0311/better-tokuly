//! Friends, as far as this domain needs more than who is friends with whom.
//!
//! Everything is asked of the social domain (`tokuly_social::graph`):
//!
//! - **mutes** ([`graph::muted_friend_ids`], [`graph::muted_by`]): a muted
//!   friend is not offered as a choice in the muter's compliment polls, the
//!   muter is not told about that friend's prompts, and does not see them in
//!   the list of friends' prompts;
//! - **since when** two people are friends ([`graph::friends_since`]), for
//!   the "friends for N months" hint.
//!
//! Mutes are applied only to what the *muter* is shown. They are never
//! applied to anonymous things arriving from the muted person: if muting
//! someone made their anonymous questions or flames stop, muting would be a
//! way to find out who sent them.
use chrono::{DateTime, Utc};
use tokuly_core::prelude::*;
use tokuly_social::{events::FriendRemoved, graph};

use crate::Ask;

impl Ask {
    /// `social.friend.removed`: the friendship is over; forget how often
    /// each was shown to the other in polls.
    pub(crate) async fn on_friend_removed(&self, event: FriendRemoved) -> AppResult<()> {
        self.forget_pair(event.user_id, event.friend_id).await
    }

    /// Drops how often two people were shown to each other in polls.
    pub(crate) async fn forget_pair(&self, a: Uuid, b: Uuid) -> AppResult<()> {
        sqlx::query("DELETE FROM ask_poll_exposure WHERE (user_id = $1 AND friend_id = $2) OR (user_id = $2 AND friend_id = $1)")
            .bind(a)
            .bind(b)
            .execute(&self.core.db)
            .await?;
        Ok(())
    }

    /// The friends `user_id` muted.
    pub(crate) async fn muted_friends(&self, user_id: Uuid) -> AppResult<Vec<Uuid>> {
        graph::muted_friend_ids(&self.core, user_id).await
    }

    /// Of `user_ids`, those who muted `friend_id`.
    pub(crate) async fn muted_by(&self, friend_id: Uuid, user_ids: &[Uuid]) -> AppResult<Vec<Uuid>> {
        graph::muted_by(&self.core, friend_id, user_ids).await
    }
}

/// Whole months between `since` and `now` (30-day months; never negative).
pub(crate) fn months_between(since: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
    ((now - since).num_days() / 30).max(0)
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    #[test]
    fn months_are_whole_and_never_negative() {
        let now = Utc::now();
        assert_eq!(months_between(now, now), 0);
        assert_eq!(months_between(now - Duration::days(29), now), 0);
        assert_eq!(months_between(now - Duration::days(30), now), 1);
        assert_eq!(months_between(now - Duration::days(400), now), 13);
        assert_eq!(
            months_between(now + Duration::days(5), now),
            0,
            "a clock that runs ahead does not produce negative months"
        );
    }
}
