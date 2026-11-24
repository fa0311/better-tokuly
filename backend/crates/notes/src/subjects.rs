//! What the notes domain plugs into the rest of the backend: subject
//! resolvers (so notes and stories can be reported, bookmarked, shared and
//! reacted to through the generic domains), the personal data export, and
//! the note count on profiles.
use std::sync::Arc;

use async_trait::async_trait;
use tokuly_core::{
    export::DataExporter,
    prelude::*,
    subject::{SubjectInfo, SubjectResolver},
};
use tokuly_identity::ProfileExtension;
use tokuly_proto::{common::SubjectType, identity::Profile};

use crate::{
    Notes,
    model::{NOTE_COLUMNS, NoteRow, STORY_COLUMNS, StoryRow, note_path, story_path},
    visibility::{PUBLIC, Viewer, sees_note},
};

/// Registers the resolvers, the exporter and the profile extension. Called
/// once from `Notes::new`.
pub(crate) fn register(notes: &Notes) {
    let core = &notes.core;
    core.subjects.register(SubjectType::SUBJECT_TYPE_NOTE, Arc::new(NoteSubjects(notes.clone())));
    core.subjects.register(SubjectType::SUBJECT_TYPE_STORY, Arc::new(StorySubjects(notes.clone())));
    core.exporters.register(Arc::new(NotesExporter { core: core.clone() }));
    notes.identity.add_profile_extension(Arc::new(NoteCount(notes.clone())));
}

/// Notes as subjects. Visibility is the same as everywhere else: a note
/// the viewer may not see does not resolve.
struct NoteSubjects(Notes);

#[async_trait]
impl SubjectResolver for NoteSubjects {
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        // A malformed ID names no note; that is "not found", not a client error.
        let Ok(note_id) = Uuid::parse_str(id) else { return Ok(None) };
        let viewer = Viewer::load(&self.0.core, viewer).await?;
        let Some(note) = self.0.find_note(&viewer, note_id).await? else { return Ok(None) };
        Ok(Some(note_info(&note)))
    }

    /// A note stays reportable by someone it reached after it was hidden
    /// from them — made private, or behind a block — so withdrawing what
    /// was already delivered does not take away the report. It reached
    /// them when it is public and its author's account is in good standing
    /// (anyone can read it signed out), when they
    /// were told about it (a reply, quote or mention notification), or when
    /// they liked or renoted it.
    async fn resolve_for_report(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        if let Some(info) = self.resolve(id, viewer).await? {
            return Ok(Some(info));
        }
        let (Ok(note_id), Some(viewer)) = (Uuid::parse_str(id), viewer) else { return Ok(None) };
        let sql = format!(
            "SELECT {NOTE_COLUMNS} FROM notes n WHERE n.id = $1 AND n.author_id <> $2 AND (
                 (n.visibility = {PUBLIC}
                  AND EXISTS (SELECT 1 FROM users u WHERE u.id = n.author_id AND u.banned_at IS NULL AND u.deleted_at IS NULL))
                 OR EXISTS (SELECT 1 FROM note_recipients r WHERE r.note_id = n.id AND r.user_id = $2)
                 OR EXISTS (SELECT 1 FROM note_likes l WHERE l.note_id = n.id AND l.user_id = $2)
                 OR EXISTS (SELECT 1 FROM note_renotes r WHERE r.note_id = n.id AND r.user_id = $2))"
        );
        let note: Option<NoteRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(note_id).bind(viewer).fetch_optional(&self.0.core.db).await?;
        Ok(note.as_ref().map(note_info))
    }
    // No `counter_changed`: a note's own counters (replies, likes, renotes,
    // quotes, views) are all kept by this domain.
}

fn note_info(note: &NoteRow) -> SubjectInfo {
    SubjectInfo {
        id: note.id,
        owner_id: note.author_id,
        title: note.title(),
        path: note_path(note.id),
        image_url: note.image_url().filter(|_| !note.content_warning),
        // Likes and replies are the notes domain's own; what the generic
        // domains add on top (emoji reactions, bookmarks, shares) is welcome.
        interactions_enabled: true,
    }
}

/// Stories as subjects, for reports and for linking notifications. A story
/// has its own reactions, and replies to it are direct messages.
struct StorySubjects(Notes);

#[async_trait]
impl SubjectResolver for StorySubjects {
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let Ok(story_id) = Uuid::parse_str(id) else { return Ok(None) };
        let viewer = Viewer::load(&self.0.core, viewer).await?;
        let story = match self.0.require_story(&viewer, story_id).await {
            Ok(story) => story,
            Err(error) if error.is_not_found() => return Ok(None),
            Err(error) => return Err(error),
        };
        Ok(Some(story_info(&story)))
    }

    /// A story someone watched stays reportable by them after its day is
    /// over, after a block, or after they were taken off its audience.
    async fn resolve_for_report(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        if let Some(info) = self.resolve(id, viewer).await? {
            return Ok(Some(info));
        }
        let (Ok(story_id), Some(viewer)) = (Uuid::parse_str(id), viewer) else { return Ok(None) };
        let sql = format!(
            "SELECT {STORY_COLUMNS} FROM stories s WHERE s.id = $1 AND s.owner_id <> $2
               AND EXISTS (SELECT 1 FROM story_views v WHERE v.story_id = s.id AND v.viewer_id = $2)"
        );
        let story: Option<StoryRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(story_id)
            .bind(viewer)
            .fetch_optional(&self.0.core.db)
            .await?;
        Ok(story.as_ref().map(story_info))
    }
}

fn story_info(story: &StoryRow) -> SubjectInfo {
    SubjectInfo {
        id: story.id,
        owner_id: story.owner_id,
        title: story.title_or_default(),
        path: story_path(story.id),
        image_url: story.cover().map(|cover| cover.url).filter(|url| !url.is_empty()),
        interactions_enabled: false,
    }
}

/// Fills in `ProfileStats.note_count`: the notes (replies aside) of the
/// profile's owner that the viewer may see.
struct NoteCount(Notes);

#[async_trait]
impl ProfileExtension for NoteCount {
    async fn enrich_profile(&self, viewer: Option<Uuid>, owner: Uuid, profile: &mut Profile) -> AppResult<()> {
        let viewer = Viewer::load(&self.0.core, viewer).await?;
        let sql = format!(
            "SELECT count(*) FROM notes n WHERE n.author_id = $4 AND n.reply_to_id IS NULL AND {}",
            sees_note("n")
        );
        let count: i64 = viewer.query_scalar(sql).bind(owner).fetch_one(&self.0.core.db).await?;
        profile.stats.get_or_insert_default().note_count = count.min(i64::from(i32::MAX)) as i32;
        Ok(())
    }
}

/// The `notes` section of a personal data export: everything the user
/// wrote, liked, renoted and posted as stories.
struct NotesExporter {
    core: Core,
}

#[async_trait]
impl DataExporter for NotesExporter {
    fn name(&self) -> &'static str {
        "notes"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        let db = &self.core.db;
        let section = |sql: &'static str| async move {
            let rows: Vec<serde_json::Value> = sqlx::query_scalar(sql).bind(user_id).fetch_all(db).await?;
            AppResult::Ok(serde_json::Value::Array(rows))
        };
        let rules: Option<serde_json::Value> =
            sqlx::query_scalar("SELECT jsonb_build_object('version', version, 'agreed_at', agreed_at) FROM note_rule_agreements WHERE user_id = $1")
                .bind(user_id)
                .fetch_optional(db)
                .await?;
        Ok(serde_json::json!({
            "rules_agreement": rules,
            "notes": section(
                "SELECT jsonb_build_object('id', id, 'text', text, 'attachments', attachments, 'stamp_id', stamp_id,
                        'place', CASE WHEN place_name IS NULL THEN NULL
                                      ELSE jsonb_build_object('name', place_name, 'lat', place_lat, 'lng', place_lng, 'place_id', place_id) END,
                        'visibility', visibility, 'content_warning', content_warning, 'reply_to_id', reply_to_id,
                        'quote_of_id', quote_of_id, 'hashtags', hashtags, 'mentions', mentions, 'link_url', link_url,
                        'reply_count', reply_count, 'renote_count', renote_count, 'quote_count', quote_count,
                        'like_count', like_count, 'view_count', view_count, 'pinned', pinned_at IS NOT NULL, 'created_at', created_at)
                 FROM notes WHERE author_id = $1 ORDER BY id"
            ).await?,
            "likes": section("SELECT jsonb_build_object('note_id', note_id, 'liked_at', created_at) FROM note_likes WHERE user_id = $1 ORDER BY id").await?,
            "renotes": section("SELECT jsonb_build_object('note_id', note_id, 'renoted_at', created_at) FROM note_renotes WHERE user_id = $1 ORDER BY id").await?,
            // Notes of other people announced to the user (reply, quote, mention).
            "received_notes": section("SELECT jsonb_build_object('note_id', note_id) FROM note_recipients WHERE user_id = $1 ORDER BY note_id").await?,
            "stories": section(
                "SELECT jsonb_build_object('id', s.id, 'title', s.title, 'items', s.items, 'audience', s.audience,
                        'audience_user_ids', (SELECT COALESCE(jsonb_agg(a.user_id ORDER BY a.user_id), '[]') FROM story_audience a WHERE a.story_id = s.id),
                        'view_count', s.view_count, 'reaction_count', s.reaction_count,
                        'created_at', s.created_at, 'expires_at', s.expires_at)
                 FROM stories s WHERE s.owner_id = $1 ORDER BY s.id"
            ).await?,
            "story_views": section(
                "SELECT jsonb_build_object('story_id', story_id, 'viewed_at', viewed_at, 'reaction', reaction, 'reacted_at', reacted_at)
                 FROM story_views WHERE viewer_id = $1 ORDER BY viewed_at"
            ).await?,
            "highlights": section(
                "SELECT jsonb_build_object('id', h.id, 'title', h.title, 'created_at', h.created_at,
                        'story_ids', (SELECT COALESCE(jsonb_agg(i.story_id ORDER BY i.position), '[]') FROM story_highlight_items i WHERE i.highlight_id = h.id))
                 FROM story_highlights h WHERE h.owner_id = $1 ORDER BY h.position, h.id"
            ).await?,
        }))
    }
}
