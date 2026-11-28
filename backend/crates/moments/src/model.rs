//! Database rows, the numbers stored in them and the names of the holds on
//! media assets.
use chrono::NaiveDate;
use tokuly_core::prelude::*;
use tokuly_proto::common::Image;

/// `moments.audience` (`tokuly.moments.v1.MomentAudience`).
pub mod audience {
    pub const FRIENDS: i16 = 1;
    pub const CLOSE_FRIENDS: i16 = 2;
}

/// `moments.delete_reason`, also the `reason` of `moments.moment.deleted`.
pub mod reason {
    pub const OWNER: &str = "owner";
    pub const MODERATION: &str = "moderation";
    pub const MEDIA_DELETED: &str = "media_deleted";
    pub const ACCOUNT_DELETED: &str = "account_deleted";
}

pub fn moment_holder(moment_id: Uuid) -> String {
    format!("moments:moment:{moment_id}")
}

pub fn reaction_holder(reaction_id: Uuid) -> String {
    format!("moments:reaction:{reaction_id}")
}

pub fn realmoji_holder(user_id: Uuid, emoji: i16) -> String {
    format!("moments:realmoji:{user_id}:{emoji}")
}

/// Where a moment opens in the apps.
pub fn moment_path(moment_id: Uuid) -> String {
    format!("/moments/{moment_id}")
}

/// A moment with the day it belongs to. Select with [`MOMENT_COLUMNS`] from
/// `moments m JOIN moment_days d ON d.id = m.day_id`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MomentRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub day_id: Uuid,
    pub back: serde_json::Value,
    pub front: serde_json::Value,
    pub caption: String,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
    pub retakes: i16,
    pub late_seconds: i64,
    pub audience: i16,
    pub posted_at: Time,
    pub deleted_at: Option<Time>,
    pub region: String,
    pub date: NaiveDate,
    /// A moment is only ever posted to a day whose prompt went out.
    pub announced_at: Time,
}

pub const MOMENT_COLUMNS: &str = "m.id, m.user_id, m.day_id, m.back, m.front, m.caption, m.lat, m.lng, m.retakes,
     m.late_seconds, m.audience, m.posted_at, m.deleted_at, d.region, d.date, d.announced_at";

impl MomentRow {
    pub fn back(&self) -> Image {
        serde_json::from_value(self.back.clone()).unwrap_or_default()
    }

    pub fn front(&self) -> Image {
        serde_json::from_value(self.front.clone()).unwrap_or_default()
    }

    pub fn location(&self) -> Option<(f64, f64)> {
        self.lat.zip(self.lng)
    }
}

/// A region's day as far as posting is concerned.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DayRow {
    pub id: Uuid,
    pub region: String,
    pub date: NaiveDate,
    pub announced_at: Time,
}

/// A reaction, with the number of reactions of its moment the query
/// counted (see `viewing`).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ReactionRow {
    pub id: Uuid,
    pub moment_id: Uuid,
    pub user_id: Uuid,
    pub realmoji: Option<i16>,
    pub image: Option<serde_json::Value>,
    pub stamp_id: Option<Uuid>,
    pub created_at: Time,
}

pub const REACTION_COLUMNS: &str = "r.id, r.moment_id, r.user_id, r.realmoji, r.image, r.stamp_id, r.created_at";
