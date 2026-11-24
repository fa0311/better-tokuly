//! What the video domain plugs into the generic domains: subject resolvers
//! (so channels, videos and playlists can be commented on, reacted to and
//! reported) and the personal data export.
//!
//! Counters: comments on a video are counted in `videos.comment_count`;
//! likes of a playlist in `playlists.like_count`. Likes of a *video* are not
//! taken from here: they have their own RPC (`VideoService.RateVideo`),
//! which is the only source of `videos.like_count`.
use std::sync::Arc;

use async_trait::async_trait;
use tokuly_core::{
    export::DataExporter,
    ids::Ref,
    prelude::*,
    subject::{SubjectInfo, SubjectResolver},
};
use tokuly_proto::common::SubjectType;

use crate::{access::Viewer, channels, playlists, videos};

/// Registers the resolvers and the exporter. Called once from `Video::new`.
pub fn register(core: &Core) {
    core.subjects.register(SubjectType::SUBJECT_TYPE_CHANNEL, Arc::new(ChannelSubjects { core: core.clone() }));
    core.subjects.register(SubjectType::SUBJECT_TYPE_VIDEO, Arc::new(VideoSubjects { core: core.clone() }));
    core.subjects.register(SubjectType::SUBJECT_TYPE_PLAYLIST, Arc::new(PlaylistSubjects { core: core.clone() }));
    core.exporters.register(Arc::new(VideoExporter { core: core.clone() }));
}

struct ChannelSubjects {
    core: Core,
}

#[async_trait]
impl SubjectResolver for ChannelSubjects {
    /// `id` is the channel's UUID or handle.
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let Some(channel) = channels::find(&self.core, id).await? else { return Ok(None) };
        let viewer = Viewer::load(&self.core, viewer).await?;
        if !viewer.may_see_owner(channel.owner_id, channel.suspended) {
            return Ok(None);
        }
        Ok(Some(SubjectInfo {
            id: channel.id,
            owner_id: channel.owner_id,
            title: channel.name.clone(),
            path: channel.path(),
            image_url: channel.icon_url(),
            interactions_enabled: true,
        }))
    }

    /// A channel stays reportable when it hides itself from the reporter by
    /// a block: what it showed them before is what they report.
    async fn resolve_for_report(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        if viewer.is_none() {
            return Ok(None);
        }
        let Some(channel) = channels::find(&self.core, id).await? else { return Ok(None) };
        Ok(Some(SubjectInfo {
            id: channel.id,
            owner_id: channel.owner_id,
            title: channel.name.clone(),
            path: channel.path(),
            image_url: channel.icon_url(),
            interactions_enabled: true,
        }))
    }
}

struct VideoSubjects {
    core: Core,
}

#[async_trait]
impl SubjectResolver for VideoSubjects {
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let Some(video) = videos::find_ref(&self.core, id).await? else { return Ok(None) };
        let viewer = Viewer::load(&self.core, viewer).await?;
        if !viewer.can_watch(&video.facts()) {
            return Ok(None);
        }
        Ok(Some(SubjectInfo {
            id: video.id,
            owner_id: video.owner_id,
            title: video.title.clone(),
            path: video.path(),
            image_url: video.thumbnail_url(),
            // Drafts have no audience to hear from.
            interactions_enabled: video.comments_enabled && video.is_published(),
        }))
    }

    /// A video the reporter watched stays reportable after its uploader made
    /// it private, unpublished it or blocked them: hiding what they already
    /// saw must not take away the report. Whoever never watched it learns
    /// nothing more than through [`Self::resolve`].
    async fn resolve_for_report(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        if let Some(info) = self.resolve(id, viewer).await? {
            return Ok(Some(info));
        }
        let (Some(user_id), Some(video)) = (viewer, videos::find_ref(&self.core, id).await?) else { return Ok(None) };
        let watched: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM watch_history WHERE user_id = $1 AND video_id = $2)")
                .bind(user_id)
                .bind(video.id)
                .fetch_one(&self.core.db)
                .await?;
        Ok(watched.then(|| SubjectInfo {
            id: video.id,
            owner_id: video.owner_id,
            title: video.title.clone(),
            path: video.path(),
            image_url: None,
            interactions_enabled: false,
        }))
    }

    async fn counter_changed(&self, id: Uuid, counter: &str, delta: i64) -> AppResult<()> {
        if counter == "comment_count" {
            sqlx::query("UPDATE videos SET comment_count = GREATEST(comment_count + $2, 0) WHERE id = $1")
                .bind(id)
                .bind(delta)
                .execute(&self.core.db)
                .await?;
        }
        Ok(())
    }
}

struct PlaylistSubjects {
    core: Core,
}

#[async_trait]
impl SubjectResolver for PlaylistSubjects {
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let Ok(reference) = Ref::parse(id) else { return Ok(None) };
        let Some(playlist) = playlists::find(&self.core, &reference).await? else { return Ok(None) };
        let viewer = Viewer::load(&self.core, viewer).await?;
        // Watch later and liked videos are private tools, not content.
        if playlist.is_builtin() || !playlist.visible_to(&viewer) {
            return Ok(None);
        }
        Ok(Some(SubjectInfo {
            id: playlist.id,
            owner_id: playlist.owner_id,
            title: playlist.title.clone(),
            path: playlist.path(),
            image_url: None,
            interactions_enabled: playlist.visibility != crate::access::PRIVATE,
        }))
    }

    async fn counter_changed(&self, id: Uuid, counter: &str, delta: i64) -> AppResult<()> {
        if counter == "like_count" {
            sqlx::query("UPDATE playlists SET like_count = GREATEST(like_count + $2, 0) WHERE id = $1")
                .bind(id)
                .bind(delta)
                .execute(&self.core.db)
                .await?;
        }
        Ok(())
    }
}

/// The `video` section of a personal data export.
struct VideoExporter {
    core: Core,
}

#[async_trait]
impl DataExporter for VideoExporter {
    fn name(&self) -> &'static str {
        "video"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        let db = &self.core.db;
        let section = |sql: &'static str| async move {
            let rows: Vec<serde_json::Value> = sqlx::query_scalar(sql).bind(user_id).fetch_all(db).await?;
            AppResult::Ok(serde_json::Value::Array(rows))
        };
        Ok(serde_json::json!({
            "channels": section(
                "SELECT jsonb_build_object('id', id, 'handle', handle::text, 'name', name, 'description', description,
                        'links', links, 'icon', icon, 'banner', banner, 'is_default', is_default,
                        'subscriber_count', subscriber_count, 'view_count', view_count, 'created_at', created_at)
                 FROM channels WHERE owner_id = $1 ORDER BY id"
            ).await?,
            "videos": section(
                "SELECT jsonb_build_object('id', v.id, 'short_id', v.short_id, 'channel_id', v.channel_id, 'title', v.title,
                        'description', v.description, 'tags', v.tags, 'category', v.category, 'game', v.game,
                        'chapters', v.chapters, 'visibility', v.visibility, 'status', v.status,
                        'age_restricted', v.age_restricted, 'comments_enabled', v.comments_enabled,
                        'duration_seconds', v.duration_seconds, 'view_count', v.view_count, 'like_count', v.like_count,
                        'dislike_count', v.dislike_count, 'comment_count', v.comment_count, 'thumbnail', v.thumbnail,
                        'created_at', v.created_at, 'published_at', v.published_at,
                        'subtitles', (SELECT COALESCE(jsonb_agg(jsonb_build_object('language', s.language, 'label', s.label,
                                             'is_default', s.is_default) ORDER BY s.language), '[]')
                                      FROM video_subtitles s WHERE s.video_id = v.id))
                 FROM videos v WHERE v.owner_id = $1 ORDER BY v.id"
            ).await?,
            "playlists": section(
                "SELECT jsonb_build_object('id', p.id, 'short_id', p.short_id, 'kind', p.kind, 'title', p.title,
                        'description', p.description, 'visibility', p.visibility, 'channel_id', p.channel_id,
                        'created_at', p.created_at,
                        'video_ids', (SELECT COALESCE(jsonb_agg(i.video_id ORDER BY i.position, i.video_id), '[]')
                                      FROM playlist_items i WHERE i.playlist_id = p.id))
                 FROM playlists p WHERE p.owner_id = $1 ORDER BY p.kind DESC, p.id"
            ).await?,
            "subscriptions": section(
                "SELECT jsonb_build_object('channel_id', s.channel_id, 'channel_handle', c.handle::text,
                        'notification_level', s.notify_level, 'subscribed_at', s.created_at)
                 FROM channel_subscriptions s JOIN channels c ON c.id = s.channel_id WHERE s.user_id = $1 ORDER BY s.created_at"
            ).await?,
            "ratings": section(
                "SELECT jsonb_build_object('video_id', video_id, 'rating', CASE rating WHEN 2 THEN 'like' ELSE 'dislike' END,
                        'rated_at', created_at)
                 FROM video_reactions WHERE user_id = $1 ORDER BY created_at"
            ).await?,
            "watch_history": section(
                "SELECT jsonb_build_object('video_id', video_id, 'position_seconds', position_seconds,
                        'watched_seconds', watched_seconds, 'completed', completed, 'last_watched_at', last_watched_at)
                 FROM watch_history WHERE user_id = $1 ORDER BY last_watched_at"
            ).await?,
            "not_interested": section(
                "SELECT jsonb_build_object('video_id', video_id, 'channel_id', channel_id, 'marked_at', created_at)
                 FROM video_feed_hidden WHERE user_id = $1 ORDER BY created_at"
            ).await?,
            "settings": section(
                "SELECT jsonb_build_object('watch_history_paused', history_paused) FROM video_user_settings WHERE user_id = $1"
            ).await?,
        }))
    }
}
