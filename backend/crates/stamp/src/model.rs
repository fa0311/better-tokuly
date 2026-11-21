//! Database rows of the stamp domain and how they turn into API messages.
use tokuly_core::prelude::*;
use tokuly_media::assets::image_from_json;
use tokuly_proto::{
    common::{StampRef, UserSummary},
    stamp as pb,
};

use crate::access::{Holding, PackFacts, Viewer};

/// `stamp_packs.status` values (`tokuly.stamp.v1.PackStatus`).
pub mod status {
    use tokuly_proto::stamp::PackStatus;

    pub const DRAFT: i32 = PackStatus::PACK_STATUS_DRAFT as i32;
    pub const IN_REVIEW: i32 = PackStatus::PACK_STATUS_IN_REVIEW as i32;
    pub const PUBLISHED: i32 = PackStatus::PACK_STATUS_PUBLISHED as i32;
    pub const REJECTED: i32 = PackStatus::PACK_STATUS_REJECTED as i32;
    pub const REMOVED: i32 = PackStatus::PACK_STATUS_REMOVED as i32;

    // Queries in this crate and the partial indexes of the migration spell
    // some of these out as literals (`p.status = 3`).
    const _: () = assert!(IN_REVIEW == 2 && PUBLISHED == 3 && REMOVED == 5);
}

/// Columns of [`PackRow`]; the table must be aliased `p`.
macro_rules! pack_columns {
    () => {
        "p.id, p.short_id, p.creator_id, p.official, p.name, p.description, p.tags, p.price, p.status, \
         p.needs_review, p.review_note, p.submitted_at, p.reviewed_at, p.published_at, p.featured_at, \
         p.cover_stamp_id, p.cover, p.stamp_count, p.approved_count, p.animated, p.install_count, \
         p.purchase_count, p.like_count, p.comment_count, p.send_count, p.popular_score, p.created_at, p.updated_at"
    };
}
pub(crate) use pack_columns;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PackRow {
    pub id: Uuid,
    pub short_id: String,
    pub creator_id: Option<Uuid>,
    pub official: bool,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub price: i32,
    pub status: i32,
    pub needs_review: bool,
    pub review_note: String,
    pub submitted_at: Option<Time>,
    pub reviewed_at: Option<Time>,
    pub published_at: Option<Time>,
    pub featured_at: Option<Time>,
    pub cover_stamp_id: Option<Uuid>,
    pub cover: Option<serde_json::Value>,
    pub stamp_count: i32,
    pub approved_count: i32,
    pub animated: bool,
    pub install_count: i64,
    pub purchase_count: i64,
    pub like_count: i64,
    pub comment_count: i64,
    pub send_count: i64,
    pub popular_score: f64,
    pub created_at: Time,
    pub updated_at: Time,
}

impl PackRow {
    pub fn facts(&self) -> PackFacts {
        PackFacts { status: self.status, price: self.price, creator_id: self.creator_id, official: self.official }
    }

    pub fn is_published(&self) -> bool {
        self.status == status::PUBLISHED
    }

    /// Path of the pack page in the app.
    pub fn path(&self) -> String {
        format!("/stamps/{}", self.short_id)
    }

    /// Path of the pack in the creator studio.
    pub fn studio_path(&self) -> String {
        format!("/stamps/studio/{}", self.short_id)
    }

    pub fn cover_url(&self) -> Option<String> {
        image_from_json(self.cover.as_ref()).map(|image| image.url).filter(|url| !url.is_empty())
    }

    /// Stamps the viewer gets to see: everything for managers and
    /// moderators, the reviewed ones for everybody else.
    pub fn visible_stamp_count(&self, viewer: &Viewer) -> i32 {
        if viewer.sees_unapproved(&self.facts()) { self.stamp_count } else { self.approved_count }
    }

    pub fn summary(
        &self,
        viewer: &Viewer,
        holding: Holding,
        creator: Option<UserSummary>,
        previews: Vec<StampRef>,
    ) -> pb::PackSummary {
        let facts = self.facts();
        let viewer_state = viewer.user_id.map(|_| pb::PackViewerState {
            installed: holding.installed,
            owned: holding.owned,
            can_manage: viewer.manages(&facts),
            usable: viewer.can_use(&facts, holding, true),
            ..Default::default()
        });
        pb::PackSummary {
            id: self.id.to_string(),
            short_id: self.short_id.clone(),
            name: self.name.clone(),
            cover: image_from_json(self.cover.as_ref()).into(),
            creator: creator.into(),
            official: self.official,
            price: self.price,
            stamp_count: self.visible_stamp_count(viewer),
            animated: self.animated,
            tags: self.tags.clone(),
            status: self.status.into(),
            featured: self.featured_at.is_some(),
            install_count: self.install_count,
            send_count: self.send_count,
            like_count: self.like_count,
            comment_count: self.comment_count,
            published_at: opt_ts(self.published_at),
            previews,
            viewer: viewer_state.into(),
            ..Default::default()
        }
    }

    /// The review state shown to managers and moderators.
    pub fn review(&self, creator_trusted: bool) -> pb::PackReview {
        pb::PackReview {
            submitted_at: opt_ts(self.submitted_at),
            reviewed_at: opt_ts(self.reviewed_at),
            note: self.review_note.clone(),
            pending_stamp_count: self.stamp_count - self.approved_count,
            needs_review: crate::access::submission_needs_moderator(&self.facts(), self.needs_review, creator_trusted),
            ..Default::default()
        }
    }
}

/// Columns of [`StampView`]: the stamp, the facts of its pack and what the
/// viewer (`$1`, a user ID or NULL) holds. The query must name the stamp `s`
/// and its pack `p`, as [`stamp_view_from`] does.
macro_rules! stamp_view_columns {
    () => {
        "s.id, s.pack_id, s.owner_id, s.name, s.emoji, s.keywords, s.format, s.image, s.image_url, \
         s.animation_url, s.width, s.height, s.position, s.approved_at, s.asset_id, s.send_count, s.created_at, \
         p.status AS pack_status, p.price AS pack_price, p.creator_id AS pack_creator_id, \
         COALESCE(p.official, false) AS pack_official, \
         EXISTS (SELECT 1 FROM stamp_pack_installs i WHERE i.pack_id = s.pack_id AND i.user_id = $1) AS installed, \
         EXISTS (SELECT 1 FROM stamp_pack_purchases o WHERE o.pack_id = s.pack_id AND o.user_id = $1) AS owned, \
         EXISTS (SELECT 1 FROM stamp_favorites f WHERE f.stamp_id = s.id AND f.user_id = $1) AS favorited"
    };
}
pub(crate) use stamp_view_columns;

/// The tables [`stamp_view_columns`] reads from.
macro_rules! stamp_view_from {
    () => {
        " FROM stamps s LEFT JOIN stamp_packs p ON p.id = s.pack_id "
    };
}
pub(crate) use stamp_view_from;

/// `SELECT … FROM …` for a [`StampView`]; append `WHERE` / `ORDER BY`.
macro_rules! stamp_view_select {
    () => {
        concat!("SELECT ", $crate::model::stamp_view_columns!(), $crate::model::stamp_view_from!())
    };
}
pub(crate) use stamp_view_select;

/// A stamp together with everything needed to decide what a viewer may do
/// with it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StampView {
    pub id: Uuid,
    pub pack_id: Option<Uuid>,
    /// Set for personal stamps only.
    pub owner_id: Option<Uuid>,
    pub name: String,
    pub emoji: String,
    pub keywords: Vec<String>,
    pub format: i32,
    pub image: serde_json::Value,
    pub image_url: String,
    pub animation_url: String,
    pub width: i32,
    pub height: i32,
    pub position: i32,
    pub approved_at: Option<Time>,
    /// The upload the picture comes from (`None` for built-in art). Shared
    /// by a stamp and every copy saved of it.
    pub asset_id: Option<Uuid>,
    pub send_count: i64,
    pub created_at: Time,
    pub pack_status: Option<i32>,
    pub pack_price: Option<i32>,
    pub pack_creator_id: Option<Uuid>,
    pub pack_official: bool,
    pub installed: bool,
    pub owned: bool,
    pub favorited: bool,
}

impl StampView {
    pub fn is_personal(&self) -> bool {
        self.pack_id.is_none()
    }

    /// Facts of the pack the stamp belongs to; `None` for personal stamps.
    pub fn pack(&self) -> Option<PackFacts> {
        Some(PackFacts {
            status: self.pack_status?,
            price: self.pack_price.unwrap_or(0),
            creator_id: self.pack_creator_id,
            official: self.pack_official,
        })
    }

    pub fn holding(&self) -> Holding {
        Holding { installed: self.installed, owned: self.owned }
    }

    /// The user who answers for the stamp: the owner of a personal stamp or
    /// the creator of its pack (`None` for official packs).
    pub fn responsible_user(&self) -> Option<Uuid> {
        self.owner_id.or(self.pack_creator_id)
    }

    /// Whether `viewer` may send the stamp.
    pub fn usable_by(&self, viewer: &Viewer) -> bool {
        match self.pack() {
            Some(pack) => viewer.can_use(&pack, self.holding(), self.approved_at.is_some()),
            None => viewer.user_id.is_some() && viewer.user_id == self.owner_id,
        }
    }

    /// Whether the stamp exists as far as `viewer` is concerned. A stamp
    /// whose ID someone holds was sent to them, so pictures are not secret;
    /// what is hidden is content that was taken down or never reviewed.
    pub fn visible_to(&self, viewer: &Viewer) -> bool {
        match self.pack() {
            Some(pack) => {
                viewer.sees_unapproved(&pack) || (pack.status != status::REMOVED && self.approved_at.is_some())
            }
            None => true,
        }
    }

    pub fn stamp_ref(&self) -> StampRef {
        StampRef {
            id: self.id.to_string(),
            pack_id: self.pack_id.map(|id| id.to_string()).unwrap_or_default(),
            name: self.name.clone(),
            image_url: self.image_url.clone(),
            animation_url: self.animation_url.clone(),
            format: self.format.into(),
            width: self.width,
            height: self.height,
            ..Default::default()
        }
    }

    pub fn to_proto(&self, viewer: &Viewer) -> pb::Stamp {
        let pending_review =
            self.approved_at.is_none() && self.pack().is_some_and(|pack| viewer.sees_unapproved(&pack));
        pb::Stamp {
            stamp_ref: self.stamp_ref().into(),
            image: image_from_json(Some(&self.image)).into(),
            emoji: self.emoji.clone(),
            keywords: self.keywords.clone(),
            personal: self.is_personal(),
            favorited: self.favorited,
            usable: self.usable_by(viewer),
            pending_review,
            created_at: ts(self.created_at),
            ..Default::default()
        }
    }
}

/// Converts stamp views into API messages, in order.
pub fn stamps_to_proto(views: &[StampView], viewer: &Viewer) -> Vec<pb::Stamp> {
    views.iter().map(|view| view.to_proto(viewer)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn personal_stamp(owner: Uuid) -> StampView {
        StampView {
            id: Uuid::now_v7(),
            pack_id: None,
            owner_id: Some(owner),
            name: "やあ".into(),
            emoji: String::new(),
            keywords: vec![],
            format: 1,
            image: serde_json::json!({ "url": "https://media.test/a.webp", "width": 320, "height": 320 }),
            image_url: "https://media.test/a.webp".into(),
            animation_url: String::new(),
            width: 320,
            height: 320,
            position: 0,
            approved_at: None,
            asset_id: Some(Uuid::now_v7()),
            send_count: 0,
            created_at: now(),
            pack_status: None,
            pack_price: None,
            pack_creator_id: None,
            pack_official: false,
            installed: false,
            owned: false,
            favorited: false,
        }
    }

    fn pack_stamp(creator: Uuid, pack_status: i32, approved: bool) -> StampView {
        StampView {
            pack_id: Some(Uuid::now_v7()),
            owner_id: None,
            approved_at: approved.then(now),
            pack_status: Some(pack_status),
            pack_price: Some(0),
            pack_creator_id: Some(creator),
            ..personal_stamp(creator)
        }
    }

    #[test]
    fn personal_stamps_are_usable_by_their_owner_only() {
        let owner = Uuid::now_v7();
        let stamp = personal_stamp(owner);
        assert!(stamp.usable_by(&Viewer::user(owner)));
        assert!(!stamp.usable_by(&Viewer::user(Uuid::now_v7())));
        assert!(!stamp.usable_by(&Viewer::default()));
        assert!(stamp.visible_to(&Viewer::default()), "whoever holds the ID received the stamp");
        let proto = stamp.to_proto(&Viewer::user(owner));
        assert!(proto.personal && proto.usable && !proto.pending_review);
        assert_eq!(proto.stamp_ref.pack_id, "");
        assert_eq!(proto.image.width, 320);
    }

    #[test]
    fn unreviewed_and_removed_pack_stamps_are_hidden_from_others() {
        let creator = Uuid::now_v7();
        let stranger = Viewer::user(Uuid::now_v7());
        let pending = pack_stamp(creator, status::DRAFT, false);
        assert!(!pending.visible_to(&stranger));
        assert!(pending.visible_to(&Viewer::user(creator)));
        assert!(pending.to_proto(&Viewer::user(creator)).pending_review);

        let removed = pack_stamp(creator, status::REMOVED, true);
        assert!(!removed.visible_to(&stranger));
        assert!(removed.visible_to(&Viewer { moderator: true, ..stranger }));

        let live = pack_stamp(creator, status::PUBLISHED, true);
        assert!(live.visible_to(&Viewer::default()));
        assert!(live.usable_by(&stranger) && !live.to_proto(&stranger).pending_review);
    }
}
