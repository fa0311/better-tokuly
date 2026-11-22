//! What the live domain plugs into the generic domains: subject resolvers
//! (so streams, clips and chat messages can be liked and reported) and the
//! personal data export.
//!
//! Counters: likes of a stream are counted in `live_streams.like_count`
//! (and pushed to the stream's viewers while it is on air), likes of a clip
//! in `live_clips.like_count`. Chat messages take no likes or comments;
//! they resolve so that they can be reported.
use std::sync::Arc;

use async_trait::async_trait;
use tokuly_core::{
    export::DataExporter,
    ids::Ref,
    prelude::*,
    subject::{SubjectInfo, SubjectResolver},
};
use tokuly_proto::{
    common::SubjectType,
    live::{LiveEvent, StreamStats, live_event::Event},
};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::{
    access::{self, Viewer},
    chat, clips, streams,
};

/// Registers the resolvers and the exporter. Called once from `Live::new`.
pub fn register(core: &Core) {
    core.subjects.register(SubjectType::SUBJECT_TYPE_LIVE_STREAM, Arc::new(StreamSubjects { core: core.clone() }));
    core.subjects.register(SubjectType::SUBJECT_TYPE_CLIP, Arc::new(ClipSubjects { core: core.clone() }));
    core.subjects.register(SubjectType::SUBJECT_TYPE_LIVE_CHAT_MESSAGE, Arc::new(ChatSubjects { core: core.clone() }));
    core.exporters.register(Arc::new(LiveExporter { core: core.clone() }));
}

struct StreamSubjects {
    core: Core,
}

#[async_trait]
impl SubjectResolver for StreamSubjects {
    /// A stream that was deleted still resolves while it is kept (for
    /// whoever could see it before): what was broadcast can be reported
    /// after its owner removed it. It takes no likes or comments any more.
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let Ok(reference) = Ref::parse(id.trim()) else { return Ok(None) };
        let Some(stream) = streams::find_with_deleted(&self.core, &reference).await? else { return Ok(None) };
        let viewer = Viewer::load(&self.core, viewer).await?;
        if !access::can_watch(&viewer, &stream.facts()) {
            return Ok(None);
        }
        Ok(Some(SubjectInfo {
            id: stream.id,
            owner_id: stream.owner_id,
            title: stream.title.clone(),
            path: stream.path(),
            image_url: stream.thumbnail_url(),
            // A stream that was cancelled before it went on air has no audience to hear from.
            interactions_enabled: !stream.is_deleted() && !(stream.is_ended() && stream.started_at.is_none()),
        }))
    }

    async fn counter_changed(&self, id: Uuid, counter: &str, delta: i64) -> AppResult<()> {
        if counter != "like_count" {
            return Ok(());
        }
        let updated: Option<(i32, i64, i64, i64)> = sqlx::query_as(
            "UPDATE live_streams SET like_count = GREATEST(like_count + $2, 0) WHERE id = $1 AND deleted_at IS NULL
             RETURNING status, viewer_count, peak_viewer_count, like_count",
        )
        .bind(id)
        .bind(delta)
        .fetch_optional(&self.core.db)
        .await?;
        if let Some((status, viewer_count, peak_viewer_count, like_count)) = updated
            && status == access::LIVE
        {
            let stats = StreamStats {
                stream_id: id.to_string(),
                viewer_count,
                peak_viewer_count,
                like_count,
                ..Default::default()
            };
            let event = LiveEvent { event: Some(Event::Stats(Box::new(stats))), ..Default::default() };
            self.core.rt.notify_topic(&streams::topic(id), &event).await;
        }
        Ok(())
    }
}

struct ClipSubjects {
    core: Core,
}

#[async_trait]
impl SubjectResolver for ClipSubjects {
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let Some(clip) = clips::find_ref(&self.core, id).await? else { return Ok(None) };
        let viewer = Viewer::load(&self.core, viewer).await?;
        let Some(source) = clips::source_of(&self.core, &viewer, &clip).await? else { return Ok(None) };
        let own = viewer.is(clip.creator_id) || viewer.is(clip.owner_id);
        if !own && (!source.watchable || !clip.is_ready() || viewer.hides(clip.creator_id)) {
            return Ok(None);
        }
        Ok(Some(SubjectInfo {
            id: clip.id,
            owner_id: clip.creator_id,
            title: clip.title.clone(),
            path: clip.path(),
            image_url: clip.thumbnail_url(),
            interactions_enabled: clip.is_ready(),
        }))
    }

    async fn counter_changed(&self, id: Uuid, counter: &str, delta: i64) -> AppResult<()> {
        if counter == "like_count" {
            sqlx::query("UPDATE live_clips SET like_count = GREATEST(like_count + $2, 0) WHERE id = $1")
                .bind(id)
                .bind(delta)
                .execute(&self.core.db)
                .await?;
        }
        Ok(())
    }
}

struct ChatSubjects {
    core: Core,
}

#[async_trait]
impl SubjectResolver for ChatSubjects {
    /// `id` is the message's UUID. Notices written by the server have no
    /// author and do not resolve.
    ///
    /// A message resolves after it was removed from the chat, and after its
    /// stream was deleted, for as long as the row is kept: its author
    /// deleting it, a moderator clearing it or a timeout does not take away
    /// the handle someone needs to report what they were sent.
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let Ok(message_id) = Uuid::parse_str(id) else { return Ok(None) };
        let Some(message) = chat::find_with_deleted(&self.core, message_id).await? else { return Ok(None) };
        let Some(author_id) = message.user_id else { return Ok(None) };
        let stream = streams::find_with_deleted(&self.core, &Ref::Id(message.stream_id)).await?;
        let Some(stream) = stream else { return Ok(None) };
        let viewer = Viewer::load(&self.core, viewer).await?;
        if !access::can_watch(&viewer, &stream.facts()) {
            return Ok(None);
        }
        let excerpt: String = message.text.graphemes(true).take(80).collect();
        Ok(Some(SubjectInfo {
            id: message.id,
            owner_id: author_id,
            title: if excerpt.is_empty() { format!("ライブチャット（{}）", stream.title) } else { excerpt },
            path: stream.path(),
            image_url: None,
            // Chat lines are reported, not liked or commented on.
            interactions_enabled: false,
        }))
    }
}

/// The `live` section of a personal data export.
struct LiveExporter {
    core: Core,
}

#[async_trait]
impl DataExporter for LiveExporter {
    fn name(&self) -> &'static str {
        "live"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        let db = &self.core.db;
        let section = |sql: &'static str| async move {
            let rows: Vec<serde_json::Value> = sqlx::query_scalar(sql).bind(user_id).fetch_all(db).await?;
            AppResult::Ok(serde_json::Value::Array(rows))
        };
        Ok(serde_json::json!({
            "streams": section(
                "SELECT jsonb_build_object('id', id, 'short_id', short_id, 'channel_id', channel_id, 'title', title,
                        'description', description, 'tags', tags, 'category', category, 'game', game, 'visibility', visibility,
                        'status', status, 'scheduled_at', scheduled_at, 'started_at', started_at, 'ended_at', ended_at,
                        'low_latency', low_latency, 'dvr_enabled', dvr_enabled, 'auto_archive', auto_archive, 'chat', chat,
                        'peak_viewer_count', peak_viewer_count, 'like_count', like_count, 'deleted_at', deleted_at,
                        'chat_message_count', chat_message_count, 'gift_coins', gift_coins, 'gift_net_coins', gift_net_coins,
                        'thumbnail', thumbnail, 'created_at', created_at)
                 FROM live_streams WHERE owner_id = $1 ORDER BY id"
            ).await?,
            "chat_messages": section(
                "SELECT jsonb_build_object('id', id, 'stream_id', stream_id, 'kind', kind, 'text', text, 'stamp_id', stamp_id,
                        'gift_coins', gift_coins, 'offset_ms', offset_ms, 'created_at', created_at, 'deleted_at', deleted_at)
                 FROM live_chat_messages WHERE user_id = $1 ORDER BY id"
            ).await?,
            "gifts_sent": section(
                "SELECT jsonb_build_object('id', id, 'stream_id', stream_id, 'channel_id', channel_id, 'coins', coins,
                        'tier', tier, 'message', message, 'created_at', created_at)
                 FROM live_gifts WHERE sender_id = $1 AND status = 2 ORDER BY id"
            ).await?,
            "gifts_received": section(
                "SELECT jsonb_build_object('id', id, 'stream_id', stream_id, 'channel_id', channel_id, 'coins', coins,
                        'net_coins', coins - fee, 'tier', tier, 'message', message, 'created_at', created_at)
                 FROM live_gifts WHERE recipient_id = $1 AND status = 2 ORDER BY id"
            ).await?,
            "clips": section(
                "SELECT jsonb_build_object('id', id, 'short_id', short_id, 'title', title, 'channel_id', channel_id,
                        'stream_id', stream_id, 'video_id', video_id, 'start_seconds', start_seconds,
                        'duration_seconds', duration_seconds, 'status', status, 'created_at', created_at)
                 FROM live_clips WHERE creator_id = $1 ORDER BY id"
            ).await?,
            "overlays": section(
                "SELECT jsonb_build_object('id', id, 'channel_id', channel_id, 'kind', kind, 'name', name, 'preset', preset,
                        'custom_css', custom_css, 'settings', settings, 'created_at', created_at)
                 FROM live_overlays WHERE owner_id = $1 ORDER BY id"
            ).await?,
            "banned_words": section(
                "SELECT jsonb_build_object('channel_id', channel_id, 'words', banned_words)
                 FROM live_channel_settings WHERE owner_id = $1 ORDER BY channel_id"
            ).await?,
            "moderator_of": section(
                "SELECT jsonb_build_object('channel_id', channel_id, 'appointed_at', created_at)
                 FROM live_moderators WHERE user_id = $1 ORDER BY created_at"
            ).await?,
            "chat_restrictions": section(
                "SELECT jsonb_build_object('channel_id', channel_id, 'expires_at', expires_at, 'created_at', created_at)
                 FROM live_chat_restrictions WHERE user_id = $1 ORDER BY created_at"
            ).await?,
            "reminders": section(
                "SELECT jsonb_build_object('stream_id', stream_id, 'created_at', created_at)
                 FROM live_reminders WHERE user_id = $1 ORDER BY created_at"
            ).await?,
            "poll_votes": section(
                "SELECT jsonb_build_object('poll_id', v.poll_id, 'question', p.question, 'option', p.options[v.option],
                        'voted_at', v.created_at)
                 FROM live_poll_votes v JOIN live_polls p ON p.id = v.poll_id WHERE v.user_id = $1 ORDER BY v.created_at"
            ).await?,
        }))
    }
}
