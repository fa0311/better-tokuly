//! Mutes, as far as this domain applies them: people a user muted are left
//! out of what that user is shown of reactions ("who reacted").
//!
//! Who muted whom is not part of the social or notify domain's public API,
//! and their tables are not ours to read, so `engagement_mutes` keeps a copy
//! from their events: `social.friend.prefs_changed` (a friend muted from
//! the friend list) and `notify.mute.changed` (anyone muted from the
//! notification settings). Each change carries the time it happened, and a
//! pair only takes a change newer than the one it holds, so the two streams
//! — delivered at least once and not always in order — end in the latest
//! state. The copy starts with this domain: a mute older than that applies
//! from its next change.
//!
//! Mutes only change what the muter sees. They never change what the muted
//! person can do or what anyone else sees: muting is silent.
use chrono::{DateTime, Utc};
use tokuly_core::prelude::*;

use crate::Engagement;

impl Engagement {
    /// Applies a mute or unmute of `muted_user_id` by `user_id` that
    /// happened at `at`, unless a newer change of the pair is already known.
    pub(crate) async fn on_mute_changed(
        &self,
        user_id: Uuid,
        muted_user_id: Uuid,
        muted: bool,
        at: DateTime<Utc>,
    ) -> AppResult<()> {
        if user_id == muted_user_id {
            return Ok(());
        }
        // Both accounts must still be there: an event about an erased one is late.
        sqlx::query(
            "INSERT INTO engagement_mutes (user_id, muted_user_id, muted, changed_at)
             SELECT $1, $2, $3, $4 WHERE (SELECT count(*) FROM users WHERE id IN ($1, $2) AND deleted_at IS NULL) = 2
             ON CONFLICT (user_id, muted_user_id) DO UPDATE SET muted = $3, changed_at = $4
                 WHERE engagement_mutes.changed_at < $4",
        )
        .bind(user_id)
        .bind(muted_user_id)
        .bind(muted)
        .bind(at)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    /// The people `viewer` muted.
    pub(crate) async fn muted_by(&self, viewer: Option<Uuid>) -> AppResult<Vec<Uuid>> {
        let Some(viewer) = viewer else { return Ok(Vec::new()) };
        Ok(sqlx::query_scalar("SELECT muted_user_id FROM engagement_mutes WHERE user_id = $1 AND muted")
            .bind(viewer)
            .fetch_all(&self.core.db)
            .await?)
    }
}
