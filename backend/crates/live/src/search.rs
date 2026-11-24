//! Search documents: `live` for streams that are upcoming or on air, `clip`
//! for clips. Only what everyone may find is indexed; everything else is
//! removed from the index.
use tokuly_core::{
    outbound::{self, SearchDoc},
    prelude::*,
};
use tokuly_video::channels::ChannelRow;

use crate::{
    clips::ClipRow,
    streams::{StreamRow, Tx},
};

/// Writes (or removes) the search document of a stream after a change.
/// `channel` is the stream's channel, read by the caller before it opened
/// the transaction (nothing here asks the pool for another connection).
pub async fn sync_stream(tx: &mut Tx<'_>, stream: &StreamRow, channel: Option<&ChannelRow>) -> AppResult<()> {
    if !stream.is_discoverable() {
        return outbound::unindex(&mut **tx, "live", stream.id).await;
    }
    let mut tags = stream.tags.clone();
    if !stream.game.is_empty() {
        tags.push(stream.game.clone());
    }
    let doc = SearchDoc {
        kind: "live".to_owned(),
        id: stream.id,
        title: stream.title.clone(),
        text: stream.description.clone(),
        tags,
        owner_id: Some(stream.owner_id),
        public: true,
        image_url: stream.thumbnail_url(),
        path: stream.path(),
        created_at: stream.started_at.or(stream.scheduled_at).unwrap_or(stream.created_at).timestamp(),
        // Signed-in viewers: the number that cannot be made up (see `audience`).
        score: stream.ranked_viewer_count as f64,
        extra: serde_json::json!({
            "short_id": stream.short_id,
            "channel_id": stream.channel_id,
            "channel_handle": channel.map(|c| c.handle.clone()),
            "channel_name": channel.map(|c| c.name.clone()),
            "status": stream.status,
            "category": stream.category,
            "game": stream.game,
            "scheduled_at": stream.scheduled_at.map(|at| at.timestamp()),
            "started_at": stream.started_at.map(|at| at.timestamp()),
        }),
    };
    outbound::index(&mut **tx, doc).await
}

/// Writes (or removes) the search document of a clip: indexed while it is
/// READY and its source is public.
pub async fn sync_clip(tx: &mut Tx<'_>, clip: &ClipRow, source_public: bool) -> AppResult<()> {
    if !clip.is_ready() || !source_public {
        return outbound::unindex(&mut **tx, "clip", clip.id).await;
    }
    let doc = SearchDoc {
        kind: "clip".to_owned(),
        id: clip.id,
        title: clip.title.clone(),
        text: String::new(),
        tags: Vec::new(),
        owner_id: Some(clip.creator_id),
        public: true,
        image_url: clip.thumbnail_url(),
        path: clip.path(),
        created_at: clip.created_at.timestamp(),
        score: clip.like_count as f64,
        extra: serde_json::json!({
            "short_id": clip.short_id,
            "channel_id": clip.channel_id,
            "duration_seconds": clip.duration_seconds,
            // Cut by the channel's owner from their own channel: the channel's
            // content, which a hit must not name the owner of.
            "channel_clip": clip.is_channel_clip(),
        }),
    };
    outbound::index(&mut **tx, doc).await
}
