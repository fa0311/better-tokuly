//! Locked accounts, as this domain keeps track of them.
//!
//! The open videos (PUBLIC, NO_FEED, UNLISTED, FRIENDS with a link) and
//! the public or unlisted playlists of a locked account reach only its
//! followers and friends. Whether an account is locked is the social
//! domain's; this domain copies it onto its rows (`videos.owner_locked`,
//! `playlists.owner_locked`) so SQL can decide.
//!
//! [`sync_owner`] (on `social.follow.settings_changed`, and after a video
//! was created with a value that was out of date by the time it was
//! stored) reads the social domain and brings the rows in line, in batches:
//! search documents follow, and every published open video whose reach
//! changed gets a `video.video.visibility_changed`, so notifications and
//! feed entries of people outside the new audience are withdrawn. Reads do
//! not wait for this: [`crate::access::Viewer::newly_locked`] covers the
//! time until the copy catches up.
//!
//! Channels keep no copy: a locked account's channel stays visible as a
//! channel (it never reveals its owner), and only its page asks whether
//! its views and trailer may be shown (see `channels::to_proto`).
use std::collections::{HashMap, HashSet};

use sqlx::{Postgres, Transaction};
use tokuly_core::prelude::*;
use tokuly_social::graph::{self, ContentAudience};

use crate::{
    access, channels, playlists, search,
    videos::{self, VideoRow, video_columns},
};

/// Videos re-checked per transaction when an account is locked or unlocked.
const BATCH: i64 = 200;

/// Re-reads whether `owner_id` is locked and brings this domain's copy, the
/// search index and the audiences other domains know in line. Idempotent
/// (late or repeated events give the same result); returns how many videos
/// changed.
pub async fn sync_owner(core: &Core, owner_id: Uuid) -> AppResult<u64> {
    let mut changed = 0;
    // A lock and an unlock in quick succession can have their passes cross:
    // the last word is whatever the social domain says once a pass is done.
    loop {
        let locked = access::is_locked_now(core, owner_id).await?;
        changed += sync_to(core, owner_id, locked).await?;
        if access::is_locked_now(core, owner_id).await? == locked {
            return Ok(changed);
        }
    }
}

/// One pass of [`sync_owner`]: brings every row to `locked`.
async fn sync_to(core: &Core, owner_id: Uuid, locked: bool) -> AppResult<u64> {
    // Everything from elsewhere is read before a transaction opens.
    let tied: HashSet<Uuid> = if locked {
        graph::audience_member_ids(core, owner_id, ContentAudience::Followers).await?.into_iter().collect()
    } else {
        HashSet::new()
    };
    let mut changed = 0;
    loop {
        let mut tx = core.db.begin().await?;
        let rows: Vec<VideoRow> = sqlx::query_as(concat!(
            "SELECT ",
            video_columns!(),
            " FROM videos v WHERE v.owner_id = $1 AND v.owner_locked <> $2 ORDER BY v.id LIMIT $3 FOR UPDATE"
        ))
        .bind(owner_id)
        .bind(locked)
        .bind(BATCH)
        .fetch_all(&mut *tx)
        .await?;
        if rows.is_empty() {
            sync_playlists_in(&mut tx, owner_id, locked).await?;
            tx.commit().await?;
            return Ok(changed);
        }
        let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        sqlx::query("UPDATE videos SET owner_locked = $2 WHERE id = ANY($1)")
            .bind(&ids)
            .bind(locked)
            .execute(&mut *tx)
            .await?;
        let told = if locked { told_about_in(&mut tx, &rows).await? } else { HashMap::new() };
        let mut narrowed = Vec::new();
        for before in rows {
            let after = VideoRow { owner_locked: locked, ..before.clone() };
            let channel = channels::find_in(&mut tx, after.channel_id).await?;
            if let Some(channel) = &channel {
                search::sync_video(&mut tx, before.is_public(), &after, channel).await?;
            }
            if before.is_public() != after.is_public() {
                playlists::sync_search_of_video_in(&mut tx, after.id).await?;
            }
            if after.is_published() && after.is_open() {
                let visible_to = locked.then(|| {
                    let mut people: Vec<Uuid> =
                        told.get(&after.id).into_iter().flatten().copied().filter(|id| tied.contains(id)).collect();
                    people.sort_unstable();
                    people.dedup();
                    people
                });
                videos::emit_visibility_changed_in(&mut tx, &after, visible_to).await?;
                if locked {
                    narrowed.push(after.id);
                }
                changed += 1;
            }
        }
        tx.commit().await?;
        // Parties of a video that strangers may no longer watch end.
        for video_id in narrowed {
            crate::party::end_for_video(core, video_id).await?;
        }
    }
}

/// The owner's playlists follow the lock: the copy, and the search
/// documents of the public ones.
async fn sync_playlists_in(tx: &mut Transaction<'_, Postgres>, owner_id: Uuid, locked: bool) -> AppResult<()> {
    let changed: Vec<(Uuid, i32)> = sqlx::query_as(
        "UPDATE playlists SET owner_locked = $2 WHERE owner_id = $1 AND owner_locked <> $2 RETURNING id, visibility",
    )
    .bind(owner_id)
    .bind(locked)
    .fetch_all(&mut **tx)
    .await?;
    for (playlist_id, visibility) in changed {
        if visibility == access::PUBLIC {
            // Forced: the document is written (unlocked) or removed (locked).
            playlists::sync_search_in(tx, true, playlist_id).await?;
        }
    }
    Ok(())
}

/// The people who may have something about each video in their inbox:
/// the channel's subscribers (told when it was published) and everyone who
/// rated it.
async fn told_about_in(tx: &mut Transaction<'_, Postgres>, rows: &[VideoRow]) -> AppResult<HashMap<Uuid, Vec<Uuid>>> {
    let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    let pairs: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT v.id, s.user_id FROM videos v JOIN channel_subscriptions s ON s.channel_id = v.channel_id
         WHERE v.id = ANY($1)
         UNION SELECT video_id, user_id FROM video_reactions WHERE video_id = ANY($1)",
    )
    .bind(&ids)
    .fetch_all(&mut **tx)
    .await?;
    let mut told: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for (video_id, user_id) in pairs {
        told.entry(video_id).or_default().push(user_id);
    }
    Ok(told)
}
