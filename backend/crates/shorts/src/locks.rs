//! Locked accounts, as this domain keeps track of them.
//!
//! The open shorts (PUBLIC, NO_FEED, UNLISTED) of a locked account reach
//! only its followers and friends. Whether an account is locked is the
//! social domain's; this domain copies it onto its rows
//! (`shorts.owner_locked`) so SQL can decide, and a public short of a
//! locked owner is not `listed`: it leaves hashtag pages, sound pages,
//! remix lists, the for-you feed and search, and their counts.
//!
//! [`Shorts::sync_owner_lock`] (on `social.follow.settings_changed`, and
//! after a short was created with a value that was out of date by the time
//! it was stored) reads the social domain and brings the rows in line, one
//! short at a time through [`listing::after_change_in`], with one
//! `shorts.short.visibility_changed` per published open short. Reads do not
//! wait for it: `Viewer::drop_newly_locked` covers the time until then.
use tokuly_core::prelude::*;
use tokuly_social::graph::ContentAudience;

use crate::{Shorts, listing, shorts};

/// Shorts re-checked per transaction.
const BATCH: i64 = 100;

impl Shorts {
    /// Re-reads whether `owner_id` is locked and applies it to their
    /// shorts. Idempotent; returns how many published open shorts changed
    /// reach.
    pub async fn sync_owner_lock(&self, owner_id: Uuid) -> AppResult<u64> {
        let core = &self.core;
        let mut changed = 0;
        loop {
            // Everything from elsewhere is read before a transaction opens;
            // the context carries the lock and the owner's audiences.
            let pending: Vec<(Uuid, Uuid, Option<Uuid>)> = sqlx::query_as(
                "SELECT id, channel_id, circle_id FROM shorts
                 WHERE owner_id = $1 AND owner_locked <> $2 ORDER BY id LIMIT $3",
            )
            .bind(owner_id)
            .bind(tokuly_video::access::is_locked_now(core, owner_id).await?)
            .bind(BATCH)
            .fetch_all(&core.db)
            .await?;
            if pending.is_empty() {
                return Ok(changed);
            }
            for (short_id, channel_id, circle_id) in pending {
                let context = listing::Context::of(core, channel_id, owner_id, circle_id).await?;
                let mut tx = core.db.begin().await?;
                let Some(before) = shorts::find_in(&mut tx, short_id).await? else { continue };
                if before.owner_locked == context.locked {
                    continue; // another pass got here first
                }
                sqlx::query("UPDATE shorts SET owner_locked = $2 WHERE id = $1")
                    .bind(short_id)
                    .bind(context.locked)
                    .execute(&mut *tx)
                    .await?;
                let after = listing::after_change_in(&mut tx, short_id, false, &context).await?;
                if after.is_published() && tokuly_video::access::is_open(after.visibility) {
                    let audience = after.audience(context.locked);
                    let visible_to = match audience {
                        ContentAudience::Public => None,
                        audience => context
                            .members(audience)
                            .map(|members| tokuly_core::events::bounded_audience(members, None)),
                    };
                    listing::emit_visibility_changed_in(&mut tx, &after, visible_to).await?;
                    changed += 1;
                }
                tx.commit().await?;
            }
        }
    }
}
