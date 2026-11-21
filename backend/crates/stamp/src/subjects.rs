//! What the stamp domain plugs into the generic domains: subject resolvers,
//! so packs can be liked, commented on and reported and single stamps
//! reported, and the personal data export.
use std::sync::Arc;

use async_trait::async_trait;
use tokuly_core::{
    export::DataExporter,
    ids::Ref,
    prelude::*,
    subject::{SubjectInfo, SubjectResolver},
};
use tokuly_proto::common::SubjectType;

use crate::{access::Viewer, packs, stamps};

/// Registers the resolvers and the exporter. Called once from `Stamp::new`.
pub fn register(core: &Core) {
    core.subjects.register(SubjectType::SUBJECT_TYPE_STAMP_PACK, Arc::new(PackSubjects { core: core.clone() }));
    core.subjects.register(SubjectType::SUBJECT_TYPE_STAMP, Arc::new(StampSubjects { core: core.clone() }));
    core.exporters.register(Arc::new(StampExporter { core: core.clone() }));
}

/// Generic domains know viewers only by ID, so nobody is staff here.
fn viewer_of(user_id: Option<Uuid>) -> Viewer {
    user_id.map(Viewer::user).unwrap_or_default()
}

/// `SubjectInfo::owner_id` of official packs and their stamps: they belong
/// to the platform, so there is no user to notify.
const NO_OWNER: Uuid = Uuid::nil();

struct PackSubjects {
    core: Core,
}

#[async_trait]
impl SubjectResolver for PackSubjects {
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        // A malformed ID names no pack; that is "not found", not a client error.
        if Ref::parse(id).is_err() {
            return Ok(None);
        }
        let Some(pack) = packs::find(&self.core, id).await? else { return Ok(None) };
        let holding = packs::holding(&self.core, viewer, pack.id).await?;
        if !viewer_of(viewer).can_view(&pack.facts(), holding) {
            return Ok(None);
        }
        Ok(Some(SubjectInfo {
            id: pack.id,
            owner_id: pack.creator_id.unwrap_or(NO_OWNER),
            title: pack.name.clone(),
            path: pack.path(),
            image_url: pack.cover_url(),
            // Likes and comments belong to the store page.
            interactions_enabled: pack.is_published(),
        }))
    }

    async fn counter_changed(&self, id: Uuid, counter: &str, delta: i64) -> AppResult<()> {
        let sql = match counter {
            "like_count" => "UPDATE stamp_packs SET like_count = GREATEST(like_count + $2, 0) WHERE id = $1",
            "comment_count" => "UPDATE stamp_packs SET comment_count = GREATEST(comment_count + $2, 0) WHERE id = $1",
            _ => return Ok(()),
        };
        sqlx::query(sql).bind(id).bind(delta).execute(&self.core.db).await?;
        Ok(())
    }
}

struct StampSubjects {
    core: Core,
}

#[async_trait]
impl SubjectResolver for StampSubjects {
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let Ok(stamp_id) = Uuid::parse_str(id) else { return Ok(None) };
        let Some(view) =
            stamps::view(&self.core, viewer, stamp_id).await?.filter(|view| view.visible_to(&viewer_of(viewer)))
        else {
            return Ok(None);
        };
        let path = match view.pack_id {
            Some(pack_id) => packs::find_by_id(&self.core.db, pack_id).await?.map(|pack| pack.path()),
            None => None,
        };
        Ok(Some(SubjectInfo {
            id: view.id,
            owner_id: view.responsible_user().unwrap_or(NO_OWNER),
            title: if view.name.is_empty() { "スタンプ".to_owned() } else { view.name.clone() },
            path: path.unwrap_or_else(|| "/stamps".to_owned()),
            image_url: Some(view.image_url.clone()).filter(|url| !url.is_empty()),
            // A single stamp can be reported, not liked or commented on.
            interactions_enabled: false,
        }))
    }
}

struct StampExporter {
    core: Core,
}

#[async_trait]
impl DataExporter for StampExporter {
    fn name(&self) -> &'static str {
        "stamp"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        let db = &self.core.db;
        let section = |sql: &'static str| async move {
            let rows: Vec<serde_json::Value> = sqlx::query_scalar(sql).bind(user_id).fetch_all(db).await?;
            AppResult::Ok(serde_json::Value::Array(rows))
        };
        Ok(serde_json::json!({
            "packs": section(
                "SELECT jsonb_build_object('id', p.id, 'name', p.name, 'description', p.description, 'tags', p.tags, 'price', p.price,
                        'status', p.status, 'created_at', p.created_at, 'published_at', p.published_at,
                        'stamps', (SELECT COALESCE(jsonb_agg(jsonb_build_object('id', s.id, 'name', s.name, 'emoji', s.emoji,
                                          'keywords', s.keywords, 'image_url', s.image_url, 'animation_url', s.animation_url)
                                          ORDER BY s.position, s.id), '[]')
                                   FROM stamps s WHERE s.pack_id = p.id))
                 FROM stamp_packs p WHERE p.creator_id = $1 ORDER BY p.id"
            ).await?,
            "personal_stamps": section(
                "SELECT jsonb_build_object('id', id, 'name', name, 'emoji', emoji, 'keywords', keywords, 'image_url', image_url,
                        'animation_url', animation_url, 'created_at', created_at)
                 FROM stamps WHERE owner_id = $1 ORDER BY position, id"
            ).await?,
            "installed_packs": section(
                "SELECT jsonb_build_object('pack_id', i.pack_id, 'name', p.name, 'installed_at', i.installed_at)
                 FROM stamp_pack_installs i JOIN stamp_packs p ON p.id = i.pack_id WHERE i.user_id = $1 ORDER BY i.position"
            ).await?,
            "purchases": section(
                "SELECT jsonb_build_object('pack_id', o.pack_id, 'name', p.name, 'price', o.price, 'purchased_at', o.purchased_at)
                 FROM stamp_pack_purchases o JOIN stamp_packs p ON p.id = o.pack_id WHERE o.user_id = $1 ORDER BY o.purchased_at"
            ).await?,
            "favorite_stamps": section(
                "SELECT jsonb_build_object('stamp_id', stamp_id, 'added_at', created_at) FROM stamp_favorites WHERE user_id = $1 ORDER BY created_at"
            ).await?,
            "usage_log": section(
                "SELECT jsonb_build_object('stamp_id', stamp_id, 'context', context, 'sent_at', created_at)
                 FROM stamp_usage WHERE user_id = $1 ORDER BY id"
            ).await?,
        }))
    }
}
