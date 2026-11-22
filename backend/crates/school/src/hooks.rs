//! What the school domain plugs into the generic parts of the backend: the
//! subject resolver that makes study rooms reportable, the guard of the
//! `study:{room_id}` realtime topic, and the personal data export.
use std::sync::Arc;

use async_trait::async_trait;
use tokuly_core::{
    export::DataExporter,
    prelude::*,
    realtime::TopicGuard,
    subject::{SubjectInfo, SubjectResolver},
};
use tokuly_proto::common::SubjectType;

use crate::{School, rooms};

/// Registers the hooks. Called once from `School::new`.
pub(crate) fn register(school: &School) {
    let core = &school.core;
    core.subjects.register(SubjectType::SUBJECT_TYPE_STUDY_ROOM, Arc::new(RoomSubjects { school: school.clone() }));
    core.topic_guards.register("study", Arc::new(RoomTopics { school: school.clone() }));
    core.exporters.register(Arc::new(SchoolExporter { core: core.clone() }));
}

struct RoomSubjects {
    school: School,
}

#[async_trait]
impl SubjectResolver for RoomSubjects {
    /// A room is a subject for the people who may enter it — which is what
    /// reporting a room needs. Rooms take no comments or reactions.
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        // A malformed ID names no room; that is "not found", not a client error.
        let (Ok(room_id), Some(viewer)) = (Uuid::parse_str(id), viewer) else { return Ok(None) };
        let Some(room) = rooms::find(&self.school.core, room_id).await? else { return Ok(None) };
        if !self.school.can_enter(&room, viewer).await? {
            return Ok(None);
        }
        Ok(Some(SubjectInfo {
            id: room.id,
            owner_id: room.host_id,
            title: room.title.clone(),
            path: rooms::path(room.id),
            image_url: None,
            interactions_enabled: false,
        }))
    }
}

struct RoomTopics {
    school: School,
}

#[async_trait]
impl TopicGuard for RoomTopics {
    /// `study:{room_id}` is for signed-in people who may enter the room:
    /// the events name who is inside and what they study.
    async fn may_subscribe(&self, subscriber: Option<Uuid>, topic: &str) -> AppResult<bool> {
        let room_id = topic.strip_prefix("study:").and_then(|id| Uuid::parse_str(id).ok());
        let (Some(room_id), Some(subscriber)) = (room_id, subscriber) else { return Ok(false) };
        match rooms::find(&self.school.core, room_id).await? {
            Some(room) => self.school.can_enter(&room, subscriber).await,
            None => Ok(false),
        }
    }
}

/// The `school` section of a personal data export.
struct SchoolExporter {
    core: Core,
}

impl SchoolExporter {
    /// Runs a query that aggregates its rows into one JSON value.
    async fn json(&self, sql: &'static str, user_id: Uuid) -> AppResult<serde_json::Value> {
        Ok(sqlx::query_scalar(sql).bind(user_id).fetch_one(&self.core.db).await?)
    }
}

#[async_trait]
impl DataExporter for SchoolExporter {
    fn name(&self) -> &'static str {
        "school"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        Ok(serde_json::json!({
            "settings": self.json(
                "SELECT COALESCE((SELECT to_jsonb(s) - 'user_id' - 'attrs' - 'class_soon_due_at' FROM school_settings s WHERE s.user_id = $1), 'null'::jsonb)",
                user_id).await?,
            "subjects": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object(
                     'id', id, 'name', name, 'short_name', short_name, 'color', color, 'emoji', emoji, 'teacher', teacher, 'room', room,
                     'archived', archived, 'created_at', created_at) ORDER BY id), '[]'::jsonb)
                 FROM school_subjects WHERE user_id = $1", user_id).await?,
            "terms": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object(
                     'id', t.id, 'name', t.name, 'start_date', t.start_date, 'end_date', t.end_date, 'current', t.is_current,
                     'layout', t.layout, 'alternating_weeks', t.alternating_weeks, 'week_a_date', t.week_a_date,
                     'observe_public_holidays', t.observe_public_holidays, 'audience', t.audience,
                     'copied_from_user_id', t.copied_from_user_id, 'created_at', t.created_at,
                     'shared_with', (SELECT COALESCE(jsonb_agg(s.user_id ORDER BY s.created_at), '[]'::jsonb) FROM school_term_shares s WHERE s.term_id = t.id),
                     'entries', (SELECT COALESCE(jsonb_agg(jsonb_build_object(
                          'weekday', e.weekday, 'period', e.period, 'week_pattern', e.week_pattern, 'subject_id', e.subject_id,
                          'teacher', e.teacher, 'room', e.room, 'color', e.color, 'memo', e.memo) ORDER BY e.weekday, e.period, e.week_pattern), '[]'::jsonb)
                          FROM school_entries e WHERE e.term_id = t.id),
                     'changes', (SELECT COALESCE(jsonb_agg(jsonb_build_object(
                          'date', c.date, 'period', c.period, 'kind', c.kind, 'subject_id', c.subject_id, 'teacher', c.teacher,
                          'room', c.room, 'memo', c.memo, 'to_date', c.to_date, 'to_period', c.to_period) ORDER BY c.date, c.period), '[]'::jsonb)
                          FROM school_changes c WHERE c.term_id = t.id),
                     'day_overrides', (SELECT COALESCE(jsonb_agg(jsonb_build_object(
                          'start_date', o.start_date, 'end_date', o.end_date, 'kind', o.kind, 'name', o.name,
                          'follows_weekday', o.follows_weekday, 'week_pattern', o.week_pattern) ORDER BY o.start_date, o.id), '[]'::jsonb)
                          FROM school_day_overrides o WHERE o.term_id = t.id)
                 ) ORDER BY t.start_date, t.id), '[]'::jsonb)
                 FROM school_terms t WHERE t.user_id = $1", user_id).await?,
            "timetables_shared_with_me": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('term_id', s.term_id, 'owner_id', t.user_id, 'since', s.created_at) ORDER BY s.created_at), '[]'::jsonb)
                 FROM school_term_shares s JOIN school_terms t ON t.id = s.term_id WHERE s.user_id = $1", user_id).await?,
            "study_sessions": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object(
                     'id', id, 'subject_id', subject_id, 'memo', memo, 'source', source, 'state', state, 'started_at', started_at,
                     'ended_at', ended_at, 'studied_seconds', counted_seconds, 'auto_stopped', auto_stopped, 'cheer_count', cheer_count,
                     'room_id', room_id) ORDER BY started_at, id), '[]'::jsonb)
                 FROM school_study_sessions WHERE user_id = $1", user_id).await?,
            "study_segments": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('session_id', session_id, 'started_at', started_at, 'ended_at', ended_at)
                     ORDER BY started_at), '[]'::jsonb)
                 FROM school_study_segments WHERE user_id = $1", user_id).await?,
            "cheers_sent": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('to_user_id', to_user_id, 'at', created_at) ORDER BY id), '[]'::jsonb)
                 FROM school_study_cheers WHERE from_user_id = $1", user_id).await?,
            "cheers_received": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('from_user_id', from_user_id, 'session_id', session_id, 'at', created_at) ORDER BY id), '[]'::jsonb)
                 FROM school_study_cheers WHERE to_user_id = $1", user_id).await?,
            "study_rooms_hosted": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object(
                     'id', r.id, 'title', r.title, 'audience', r.audience, 'member_cap', r.member_cap, 'created_at', r.created_at,
                     'closed_at', r.closed_at,
                     'invited', (SELECT COALESCE(jsonb_agg(i.user_id ORDER BY i.created_at), '[]'::jsonb) FROM school_room_invites i WHERE i.room_id = r.id)
                 ) ORDER BY r.id), '[]'::jsonb)
                 FROM school_rooms r WHERE r.host_id = $1", user_id).await?,
            "study_room_invitations": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('room_id', room_id, 'at', created_at) ORDER BY created_at), '[]'::jsonb)
                 FROM school_room_invites WHERE user_id = $1", user_id).await?,
            "exams": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object(
                     'id', x.id, 'name', x.name, 'start_date', x.start_date, 'end_date', x.end_date, 'show_on_profile', x.show_on_profile,
                     'memo', x.memo, 'created_at', x.created_at,
                     'subjects', (SELECT COALESCE(jsonb_agg(jsonb_build_object(
                          'subject_id', s.subject_id, 'date', s.date, 'period', s.period, 'scope', s.scope,
                          'result', CASE WHEN s.has_result THEN jsonb_build_object(
                              'score', s.score, 'max_score', s.max_score, 'average', s.average, 'rank', s.rank, 'memo', s.result_memo) END
                     ) ORDER BY s.date NULLS LAST, s.period, s.id), '[]'::jsonb) FROM school_exam_subjects s WHERE s.exam_id = x.id)
                 ) ORDER BY x.start_date, x.id), '[]'::jsonb)
                 FROM school_exams x WHERE x.user_id = $1", user_id).await?,
            "assignments": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object(
                     'id', id, 'title', title, 'subject_id', subject_id, 'memo', memo, 'due_at', due_at, 'done_at', done_at,
                     'remind', remind, 'created_at', created_at) ORDER BY due_at, id), '[]'::jsonb)
                 FROM school_assignments WHERE user_id = $1", user_id).await?,
        }))
    }
}
