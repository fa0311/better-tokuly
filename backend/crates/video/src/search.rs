//! Search documents: public channels, public videos and public playlists
//! (`tokuly_core::outbound::index`). Only what everyone may find is indexed;
//! anything that stops being public is removed.
use tokuly_core::{
    outbound::{self, SearchDoc},
    prelude::*,
};

use crate::{channels::ChannelRow, playlists::PlaylistRow, videos::VideoRow};

type Tx<'a> = sqlx::Transaction<'a, sqlx::Postgres>;

fn channel_doc(channel: &ChannelRow) -> SearchDoc {
    SearchDoc {
        kind: "channel".to_owned(),
        id: channel.id,
        title: channel.name.clone(),
        text: channel.description.clone(),
        tags: vec![channel.handle.clone()],
        owner_id: Some(channel.owner_id),
        public: !channel.suspended,
        image_url: channel.icon_url(),
        path: channel.path(),
        created_at: channel.created_at.timestamp(),
        score: channel.subscriber_count as f64,
        extra: serde_json::json!({
            "handle": channel.handle,
            "verified": channel.verified,
            "subscriber_count": channel.subscriber_count,
        }),
    }
}

/// Writes (or, for a suspended owner, removes) a channel's search document.
pub async fn index_channel(tx: &mut Tx<'_>, channel: &ChannelRow) -> AppResult<()> {
    outbound::index(&mut **tx, channel_doc(channel)).await
}

fn video_doc(video: &VideoRow, channel: &ChannelRow) -> SearchDoc {
    let mut tags = video.tags.clone();
    if !video.game.is_empty() {
        tags.push(video.game.clone());
    }
    SearchDoc {
        kind: "video".to_owned(),
        id: video.id,
        title: video.title.clone(),
        text: video.description.clone(),
        tags,
        owner_id: Some(video.owner_id),
        public: video.is_public(),
        image_url: video.thumbnail_url(),
        path: video.path(),
        created_at: video.published_at.unwrap_or(video.created_at).timestamp(),
        score: video.view_count as f64,
        extra: serde_json::json!({
            "short_id": video.short_id,
            "channel_id": channel.id,
            "channel_handle": channel.handle,
            "channel_name": channel.name,
            "duration_seconds": video.duration_seconds,
            "category": video.category,
            "game": video.game,
            "age_restricted": video.age_restricted,
            "origin": video.origin,
        }),
    }
}

/// Keeps a video's search document in step with a change: written while the
/// video is public, removed when it stops being public, untouched when it
/// was not and is not (drafts being edited).
pub async fn sync_video(tx: &mut Tx<'_>, was_public: bool, video: &VideoRow, channel: &ChannelRow) -> AppResult<()> {
    if video.is_public() || was_public {
        outbound::index(&mut **tx, video_doc(video, channel)).await?;
    }
    Ok(())
}

/// Like [`sync_video`], for a playlist. `image_url` is the thumbnail of its
/// first public video, if the caller has it at hand.
pub async fn sync_playlist(
    tx: &mut Tx<'_>,
    was_public: bool,
    playlist: &PlaylistRow,
    image_url: Option<String>,
) -> AppResult<()> {
    if !(playlist.is_public() || was_public) {
        return Ok(());
    }
    let doc = SearchDoc {
        kind: "playlist".to_owned(),
        id: playlist.id,
        title: playlist.title.clone(),
        text: playlist.description.clone(),
        tags: Vec::new(),
        owner_id: Some(playlist.owner_id),
        public: playlist.is_public(),
        image_url,
        path: playlist.path(),
        created_at: playlist.created_at.timestamp(),
        score: playlist.like_count as f64,
        extra: serde_json::json!({
            "short_id": playlist.short_id,
            "item_count": playlist.item_count,
            "channel_id": playlist.channel_id,
        }),
    };
    outbound::index(&mut **tx, doc).await
}
