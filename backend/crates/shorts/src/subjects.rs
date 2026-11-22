//! What the shorts domain plugs into the generic domains: the subject
//! resolver (so shorts can be commented on, liked, bookmarked and reported
//! through the engagement domain) and the personal data export.
//!
//! Counters: `like_count` and `comment_count` on a short are copies of the
//! engagement domain's counts, moved by its `counter_changed` callback,
//! which also feeds the creator analytics. The callback says how much a
//! count moved but not who moved it, so it is *not* what the ranking
//! reads: likes and comments reach the ranking through the engagement
//! domain's events, once per account (`workers`). Saves and shares are
//! this domain's own ([`crate::library`]); engagement's bookmark and share
//! counters for shorts are ignored.
use std::sync::Arc;

use async_trait::async_trait;
use tokuly_core::{
    export::DataExporter,
    prelude::*,
    subject::{SubjectInfo, SubjectResolver},
};
use tokuly_proto::common::SubjectType;

use crate::{
    access::{self, Viewer},
    shorts::{self, ShortRow, short_columns},
    signals,
};

/// Registers the resolver and the exporter. Called once from `Shorts::new`.
pub fn register(core: &Core) {
    core.subjects.register(SubjectType::SUBJECT_TYPE_SHORT, Arc::new(ShortSubjects { core: core.clone() }));
    core.exporters.register(Arc::new(ShortsExporter { core: core.clone() }));
}

struct ShortSubjects {
    core: Core,
}

#[async_trait]
impl SubjectResolver for ShortSubjects {
    /// `id` is the short's UUID or short ID.
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let Some(short) = shorts::find_ref(&self.core, id).await? else { return Ok(None) };
        let viewer = Viewer::load(&self.core, viewer).await?;
        if !access::can_watch(&viewer, &short.facts()) {
            return Ok(None);
        }
        Ok(Some(SubjectInfo {
            id: short.id,
            owner_id: short.owner_id,
            title: short.title(),
            path: short.path(),
            image_url: short.cover_url(),
            // Drafts have no audience to hear from. The engagement contract
            // has one switch for comments and reactions alike, so a short
            // with comments off takes no likes either.
            interactions_enabled: short.comments_enabled && short.is_published(),
        }))
    }

    async fn counter_changed(&self, id: Uuid, counter: &str, delta: i64) -> AppResult<()> {
        let (sql, event) = match counter {
            "like_count" => (
                concat!(
                    "UPDATE shorts s SET like_count = GREATEST(s.like_count + $2, 0) WHERE s.id = $1 RETURNING ",
                    short_columns!()
                ),
                "short.like",
            ),
            "comment_count" => (
                concat!(
                    "UPDATE shorts s SET comment_count = GREATEST(s.comment_count + $2, 0) WHERE s.id = $1 RETURNING ",
                    short_columns!()
                ),
                "short.comment",
            ),
            _ => return Ok(()),
        };
        if delta == 0 {
            return Ok(());
        }
        let short: Option<ShortRow> = sqlx::query_as(sql).bind(id).bind(delta).fetch_optional(&self.core.db).await?;
        if let Some(short) = short {
            signals::track(&self.core, event, &short, None, delta as f64, serde_json::json!({})).await;
        }
        Ok(())
    }
}

/// The `shorts` section of a personal data export.
struct ShortsExporter {
    core: Core,
}

#[async_trait]
impl DataExporter for ShortsExporter {
    fn name(&self) -> &'static str {
        "shorts"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        let db = &self.core.db;
        let section = |sql: &'static str| async move {
            let rows: Vec<serde_json::Value> = sqlx::query_scalar(sql).bind(user_id).fetch_all(db).await?;
            AppResult::Ok(serde_json::Value::Array(rows))
        };
        // Lists of other people's shorts name only what the user may still
        // open: an export is not a way to learn that a short one can no
        // longer see (private now, or its owner blocked them) still exists.
        let viewer = Viewer::load(&self.core, Some(user_id)).await?;
        let kept = |sql: &'static str| {
            let viewer = &viewer;
            async move {
                let rows: Vec<serde_json::Value> = sqlx::query_scalar(sql)
                    .bind(user_id)
                    .bind(&viewer.friends)
                    .bind(&viewer.blocked)
                    .fetch_all(db)
                    .await?;
                AppResult::Ok(serde_json::Value::Array(rows))
            }
        };
        Ok(serde_json::json!({
            "shorts": section(
                "SELECT jsonb_build_object('id', s.id, 'short_id', s.short_id, 'channel_id', s.channel_id,
                        'caption', s.caption, 'hashtags', s.hashtags,
                        'mentions', (SELECT COALESCE(jsonb_agg(m -> 'handle'), '[]') FROM jsonb_array_elements(s.mentions) m),
                        'visibility', s.visibility, 'status', s.status, 'comments_enabled', s.comments_enabled,
                        'allow_remix', s.allow_remix, 'allow_duet', s.allow_duet, 'cover', s.cover,
                        'duration_seconds', s.duration_seconds, 'width', s.width, 'height', s.height,
                        'sound', (SELECT jsonb_build_object('id', d.id, 'kind', d.kind, 'title', d.title, 'artist', d.artist)
                                  FROM short_sounds d WHERE d.id = s.sound_id),
                        'remix_of_id', s.remix_of_id, 'remix_kind', s.remix_kind,
                        'source_type', s.source_type, 'source_id', s.source_id,
                        'view_count', s.view_count, 'like_count', s.like_count, 'comment_count', s.comment_count,
                        'share_count', s.share_count, 'save_count', s.save_count, 'loop_count', s.loop_count,
                        'pinned', s.pinned_at IS NOT NULL, 'created_at', s.created_at, 'published_at', s.published_at,
                        'withdrawn_at', s.withdrawn_at)
                 FROM shorts s WHERE s.owner_id = $1 ORDER BY s.id"
            ).await?,
            "saved": kept(concat!(
                "SELECT jsonb_build_object('short_id', v.short_id, 'saved_at', v.created_at)
                 FROM short_saves v JOIN shorts s ON s.id = v.short_id
                 WHERE v.user_id = $1 AND ", access::watchable!(1, 2, 3), " ORDER BY v.created_at"
            )).await?,
            "liked": kept(concat!(
                "SELECT jsonb_build_object('short_id', l.short_id, 'liked_at', l.created_at)
                 FROM short_likes l JOIN shorts s ON s.id = l.short_id
                 WHERE l.user_id = $1 AND l.liked AND ", access::watchable!(1, 2, 3), " ORDER BY l.created_at"
            )).await?,
            "not_interested": section(
                "SELECT jsonb_build_object('short_id', short_id, 'channel_id', channel_id, 'hashtags', hashtags,
                        'marked_at', created_at)
                 FROM short_hidden WHERE user_id = $1 ORDER BY created_at"
            ).await?,
            "followed_channels": section(
                "SELECT jsonb_build_object('channel_id', channel_id, 'since', created_at)
                 FROM short_follows WHERE user_id = $1 ORDER BY created_at"
            ).await?,
        }))
    }
}
