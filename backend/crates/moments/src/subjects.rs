//! What the moments domain plugs into the rest of the backend: subject
//! resolvers, so that moments and reactions can be reported, and the
//! personal data export.
//!
//! Neither takes comments or engagement reactions (`interactions_enabled`
//! is false): a moment has reactions of its own. For a report, a moment
//! resolves also after it was deleted or its day is over — for its owner
//! and for everyone it was shown to unlocked — and a reaction also after it
//! was replaced or taken back, for the moment's owner, the reactor and
//! everyone the moment was shown to. Until the purge.
use std::sync::Arc;

use async_trait::async_trait;
use tokuly_core::{
    export::DataExporter,
    prelude::*,
    subject::{SubjectInfo, SubjectResolver},
};
use tokuly_proto::common::SubjectType;

use crate::{
    Moments,
    model::{MomentRow, moment_path},
};

/// Registers the resolvers and the exporter. Called once from `Moments::new`.
pub(crate) fn register(moments: &Moments) {
    let core = &moments.core;
    core.subjects.register(SubjectType::SUBJECT_TYPE_MOMENT, Arc::new(MomentSubjects(moments.clone())));
    core.subjects.register(SubjectType::SUBJECT_TYPE_MOMENT_REACTION, Arc::new(ReactionSubjects(moments.clone())));
    core.exporters.register(Arc::new(MomentsExporter { core: core.clone() }));
}

fn moment_info(moment: &MomentRow) -> SubjectInfo {
    SubjectInfo {
        id: moment.id,
        owner_id: moment.user_id,
        title: if moment.caption.is_empty() {
            format!("いまの瞬間 {}", moment.date)
        } else {
            moment.caption.clone()
        },
        path: moment_path(moment.id),
        image_url: None,
        interactions_enabled: false,
    }
}

impl Moments {
    /// Whether the moment was shown to `viewer` unlocked, or is theirs.
    async fn was_shown(&self, moment: &MomentRow, viewer: Uuid) -> AppResult<bool> {
        if moment.user_id == viewer {
            return Ok(true);
        }
        Ok(sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM moment_deliveries WHERE moment_id = $1 AND user_id = $2)")
            .bind(moment.id)
            .bind(viewer)
            .fetch_one(&self.core.db)
            .await?)
    }
}

struct MomentSubjects(Moments);

#[async_trait]
impl SubjectResolver for MomentSubjects {
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let (Ok(id), Some(viewer)) = (Uuid::parse_str(id), viewer) else { return Ok(None) };
        Ok(self.0.visible_moment(viewer, id).await?.as_ref().map(moment_info))
    }

    async fn resolve_for_report(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let (Ok(moment_id), Some(viewer)) = (Uuid::parse_str(id), viewer) else { return Ok(None) };
        if let Some(found) = self.resolve(id, Some(viewer)).await? {
            return Ok(Some(found));
        }
        let Some(moment) = self.0.find_moment(moment_id).await? else { return Ok(None) };
        Ok(self.0.was_shown(&moment, viewer).await?.then(|| moment_info(&moment)))
    }
}

struct ReactionSubjects(Moments);

impl ReactionSubjects {
    /// The reaction (live or not) with its reactor and moment.
    async fn find(&self, id: &str) -> AppResult<Option<(Uuid, Uuid, bool, MomentRow)>> {
        let Ok(reaction_id) = Uuid::parse_str(id) else { return Ok(None) };
        let found: Option<(Uuid, Uuid, bool)> =
            sqlx::query_as("SELECT user_id, moment_id, removed_at IS NULL FROM moment_reactions WHERE id = $1")
                .bind(reaction_id)
                .fetch_optional(&self.0.core.db)
                .await?;
        let Some((reactor, moment_id, live)) = found else { return Ok(None) };
        let Some(moment) = self.0.find_moment(moment_id).await? else { return Ok(None) };
        Ok(Some((reaction_id, reactor, live, moment)))
    }

    fn info(reaction_id: Uuid, reactor: Uuid, moment: &MomentRow) -> SubjectInfo {
        SubjectInfo {
            id: reaction_id,
            owner_id: reactor,
            title: "いまの瞬間へのリアクション".into(),
            path: moment_path(moment.id),
            image_url: None,
            interactions_enabled: false,
        }
    }
}

#[async_trait]
impl SubjectResolver for ReactionSubjects {
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let Some(viewer) = viewer else { return Ok(None) };
        let Some((reaction_id, reactor, live, moment)) = self.find(id).await? else { return Ok(None) };
        let sees = live
            && (reactor == viewer
                || (self.0.visible_moment(viewer, moment.id).await?.is_some()
                    && (moment.user_id == viewer || self.0.unlocked(viewer).await?)));
        Ok(sees.then(|| Self::info(reaction_id, reactor, &moment)))
    }

    async fn resolve_for_report(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let Some(viewer) = viewer else { return Ok(None) };
        let Some((reaction_id, reactor, _, moment)) = self.find(id).await? else { return Ok(None) };
        let may = reactor == viewer || self.0.was_shown(&moment, viewer).await?;
        Ok(may.then(|| Self::info(reaction_id, reactor, &moment)))
    }
}

struct MomentsExporter {
    core: Core,
}

#[async_trait]
impl DataExporter for MomentsExporter {
    fn name(&self) -> &'static str {
        "moments"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        let db = &self.core.db;
        let section = |sql: &'static str| async move {
            let rows: Vec<serde_json::Value> = sqlx::query_scalar(sql).bind(user_id).fetch_all(db).await?;
            AppResult::Ok(serde_json::Value::Array(rows))
        };
        Ok(serde_json::json!({
            "moments": section(
                "SELECT jsonb_build_object('id', m.id, 'date', d.date, 'region', d.region, 'posted_at', m.posted_at,
                        'late_seconds', m.late_seconds, 'retakes', m.retakes, 'caption', m.caption,
                        'lat', m.lat, 'lng', m.lng, 'audience', m.audience, 'back', m.back, 'front', m.front,
                        'deleted_at', m.deleted_at)
                 FROM moments m JOIN moment_days d ON d.id = m.day_id
                 WHERE m.user_id = $1 ORDER BY m.posted_at"
            ).await?,
            "reactions": section(
                "SELECT jsonb_build_object('id', id, 'moment_id', moment_id, 'realmoji', realmoji, 'image', image,
                        'stamp_id', stamp_id, 'created_at', created_at, 'removed_at', removed_at)
                 FROM moment_reactions WHERE user_id = $1 ORDER BY created_at"
            ).await?,
            "realmojis": section(
                "SELECT jsonb_build_object('emoji', emoji, 'image', image, 'updated_at', updated_at)
                 FROM moment_realmojis WHERE user_id = $1 ORDER BY emoji"
            ).await?,
            "settings": section(
                "SELECT jsonb_build_object('share_location', share_location, 'audience', audience, 'prompts', prompts)
                 FROM moment_settings WHERE user_id = $1"
            ).await?,
        }))
    }
}
