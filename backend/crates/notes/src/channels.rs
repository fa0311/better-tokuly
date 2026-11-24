//! Channels (チャンネル): topic timelines anyone can post into.
//!
//! This part is what the rest of the crate needs from channels: whether a
//! note may be posted into one, the reference shown on a note, and the SQL
//! that keeps channel notes out of the home, following and global timelines
//! unless the reader follows the channel.
use std::collections::HashMap;

use sqlx::{Postgres, Transaction};
use tokuly_core::prelude::*;
use tokuly_proto::notes::ChannelRef;

/// SQL condition: the note in alias `n` belongs in a general timeline for
/// the viewer (`$1`): it is in no channel, a moderator took it out of its
/// channel, or the viewer follows the channel.
pub fn outside_unfollowed_channels(n: &str) -> String {
    format!(
        "({n}.channel_id IS NULL OR {n}.channel_removed_at IS NOT NULL \
          OR EXISTS (SELECT 1 FROM note_channel_follows cf_ WHERE cf_.channel_id = {n}.channel_id AND cf_.user_id = $1))"
    )
}

/// SQL condition: the note in alias `n` is a top-level note in a channel the
/// viewer (`$1`) follows (the home timeline shows those).
pub fn in_followed_channel(n: &str) -> String {
    format!(
        "({n}.channel_id IS NOT NULL AND {n}.channel_removed_at IS NULL AND {n}.reply_to_id IS NULL \
          AND EXISTS (SELECT 1 FROM note_channel_follows cf_ WHERE cf_.channel_id = {n}.channel_id AND cf_.user_id = $1))"
    )
}

/// A channel a note is about to be posted into: it must exist. Whether it
/// is archived is checked again when the note is counted
/// ([`count_new_note`]), inside the posting transaction.
pub(crate) async fn require_open(core: &Core, channel_id: Uuid) -> AppResult<()> {
    let archived: Option<bool> = sqlx::query_scalar("SELECT archived_at IS NOT NULL FROM note_channels WHERE id = $1")
        .bind(channel_id)
        .fetch_optional(&core.db)
        .await?;
    match archived {
        None => Err(AppError::not_found("CHANNEL_NOT_FOUND", "channel not found")),
        Some(true) => Err(AppError::precondition("CHANNEL_ARCHIVED", "this channel is archived")),
        Some(false) => Ok(()),
    }
}

/// Counts a new note towards its channel, refusing (`CHANNEL_ARCHIVED`) a
/// channel archived since it was checked.
pub(crate) async fn count_new_note(tx: &mut Transaction<'_, Postgres>, channel_id: Uuid) -> AppResult<()> {
    let counted = sqlx::query(
        "UPDATE note_channels SET note_count = note_count + 1, last_note_at = now() WHERE id = $1 AND archived_at IS NULL",
    )
    .bind(channel_id)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    ensure!(counted == 1, precondition, "CHANNEL_ARCHIVED", "this channel is archived");
    Ok(())
}

/// Takes notes that left their channels (deleted, or taken out) off the
/// channels' counts: `(channel, how many)`.
pub(crate) async fn uncount_notes(tx: &mut Transaction<'_, Postgres>, gone: &HashMap<Uuid, i64>) -> AppResult<()> {
    if gone.is_empty() {
        return Ok(());
    }
    let mut ids: Vec<Uuid> = gone.keys().copied().collect();
    ids.sort();
    let counts: Vec<i64> = ids.iter().map(|id| gone[id]).collect();
    sqlx::query(
        "UPDATE note_channels c SET note_count = GREATEST(c.note_count - d.n, 0)
         FROM unnest($1::uuid[], $2::bigint[]) AS d(id, n) WHERE c.id = d.id",
    )
    .bind(&ids)
    .bind(&counts)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The references shown on notes for these channels.
pub(crate) async fn refs(core: &Core, channel_ids: &[Uuid]) -> AppResult<HashMap<Uuid, ChannelRef>> {
    if channel_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(Uuid, String, String)> =
        sqlx::query_as("SELECT id, name, color FROM note_channels WHERE id = ANY($1)")
            .bind(channel_ids)
            .fetch_all(&core.db)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name, color)| (id, ChannelRef { id: id.to_string(), name, color, ..Default::default() }))
        .collect())
}
