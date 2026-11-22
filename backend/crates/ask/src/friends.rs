//! Friends, as far as this domain needs more than `tokuly_social::graph`
//! offers.
//!
//! Who is friends with whom, and who blocked whom, is always asked of the
//! social domain. Two facts are not part of its public API, and its tables
//! are not ours to read, so we keep our own copy of them from its events
//! (`ask_friend_links`):
//!
//! - **mutes** (`social.friend.prefs_changed`): a muted friend is not
//!   offered as a choice in the muter's compliment polls, the muter is not
//!   told about that friend's prompts, and does not see them in the list of
//!   friends' prompts;
//! - **since when** two people are friends (`social.friend.accepted`), for
//!   the "friends for N months" hint.
//!
//! The copy starts with this domain: a friendship older than that has no
//! row until something about it changes. Missing rows mean "not muted" and
//! "since unknown", and the hint then falls back to the first time the two
//! met in a poll (`ask_poll_exposure.created_at`).
//!
//! Mutes are applied only to what the *muter* is shown. They are never
//! applied to anonymous things arriving from the muted person: if muting
//! someone made their anonymous questions or flames stop, muting would be a
//! way to find out who sent them.
use chrono::{DateTime, Utc};
use tokuly_core::prelude::*;
use tokuly_social::events::{FriendAccepted, FriendPrefsChanged, FriendRemoved};

use crate::Ask;

impl Ask {
    /// `social.friend.accepted`: remembers when the friendship began, for
    /// both directions.
    pub(crate) async fn on_friend_accepted(&self, event: FriendAccepted, at: DateTime<Utc>) -> AppResult<()> {
        if event.user_id == event.friend_id {
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO ask_friend_links (user_id, friend_id, since)
             VALUES ($1, $2, $3), ($2, $1, $3)
             ON CONFLICT (user_id, friend_id) DO UPDATE SET
                 since = COALESCE(ask_friend_links.since, EXCLUDED.since), updated_at = now()",
        )
        .bind(event.user_id)
        .bind(event.friend_id)
        .bind(at)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    /// `social.friend.removed`: the friendship is over; forget it.
    pub(crate) async fn on_friend_removed(&self, event: FriendRemoved) -> AppResult<()> {
        self.forget_pair(event.user_id, event.friend_id).await
    }

    /// `social.friend.prefs_changed`: `user_id` muted or unmuted `friend_id`.
    pub(crate) async fn on_friend_prefs_changed(&self, event: FriendPrefsChanged) -> AppResult<()> {
        if event.muted {
            sqlx::query(
                "INSERT INTO ask_friend_links (user_id, friend_id, muted) VALUES ($1, $2, true)
                 ON CONFLICT (user_id, friend_id) DO UPDATE SET muted = true, updated_at = now()",
            )
            .bind(event.user_id)
            .bind(event.friend_id)
            .execute(&self.core.db)
            .await?;
        } else {
            // Nothing to remember about an unmuted friend we know nothing else of.
            sqlx::query("UPDATE ask_friend_links SET muted = false, updated_at = now() WHERE user_id = $1 AND friend_id = $2 AND muted")
                .bind(event.user_id)
                .bind(event.friend_id)
                .execute(&self.core.db)
                .await?;
        }
        Ok(())
    }

    /// Drops what we remember about two people as friends: the link in both
    /// directions and how often each was shown to the other in polls.
    pub(crate) async fn forget_pair(&self, a: Uuid, b: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        sqlx::query(
            "DELETE FROM ask_friend_links WHERE (user_id = $1 AND friend_id = $2) OR (user_id = $2 AND friend_id = $1)",
        )
        .bind(a)
        .bind(b)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM ask_poll_exposure WHERE (user_id = $1 AND friend_id = $2) OR (user_id = $2 AND friend_id = $1)")
            .bind(a)
            .bind(b)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The friends `user_id` muted.
    pub(crate) async fn muted_friends(&self, user_id: Uuid) -> AppResult<Vec<Uuid>> {
        Ok(sqlx::query_scalar("SELECT friend_id FROM ask_friend_links WHERE user_id = $1 AND muted")
            .bind(user_id)
            .fetch_all(&self.core.db)
            .await?)
    }

    /// Of `user_ids`, those who muted `friend_id`.
    pub(crate) async fn muted_by(&self, friend_id: Uuid, user_ids: &[Uuid]) -> AppResult<Vec<Uuid>> {
        Ok(sqlx::query_scalar(
            "SELECT user_id FROM ask_friend_links WHERE friend_id = $1 AND user_id = ANY($2) AND muted",
        )
        .bind(friend_id)
        .bind(user_ids)
        .fetch_all(&self.core.db)
        .await?)
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
