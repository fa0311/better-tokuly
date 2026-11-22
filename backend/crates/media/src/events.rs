//! Events the media domain emits, and the realtime pushes that mirror them.
//!
//! | Topic | When |
//! |---|---|
//! | `media.asset.ready` | processing finished; the asset can be attached |
//! | `media.asset.failed` | the upload was rejected or could not be processed |
//! | `media.asset.deleted` | the owner (or account deletion) removed the asset |
//!
//! All three carry an [`AssetEvent`]. Realtime: every state or progress change
//! is pushed as `MediaEvent.asset_updated` to the owner and to the topic
//! [`asset_topic`].
use serde::{Deserialize, Serialize};
use tokuly_core::prelude::*;
use tokuly_proto::media::MediaEvent;

use crate::assets::AssetRow;

pub const ASSET_READY: &str = "media.asset.ready";
pub const ASSET_FAILED: &str = "media.asset.failed";
pub const ASSET_DELETED: &str = "media.asset.deleted";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetEvent {
    pub asset_id: Uuid,
    pub owner_id: Uuid,
    /// `tokuly.media.v1.AssetPurpose` as its number.
    pub purpose: i32,
    /// `tokuly.media.v1.AssetKind` as its number.
    pub kind: i32,
    /// Failure reason; only set on `media.asset.failed`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

impl AssetEvent {
    pub fn of(row: &AssetRow) -> Self {
        Self { asset_id: row.id, owner_id: row.owner_id, purpose: row.purpose, kind: row.kind, error: row.error.clone() }
    }
}

/// Realtime topic carrying the processing progress of one asset.
pub fn asset_topic(asset_id: Uuid) -> String {
    format!("asset:{asset_id}")
}

/// Pushes the asset's current state to its owner and to the asset topic.
/// Topic subscribers may be anyone who knows the ID (e.g. the recipient of a
/// message whose attachment is still processing), so they get the public view.
pub(crate) async fn push_update(core: &Core, row: &AssetRow) {
    let owner_view = MediaEvent { event: Some(row.to_proto().into()), ..Default::default() };
    core.rt.notify_user(row.owner_id, &owner_view).await;
    let public_view = MediaEvent { event: Some(row.to_public_proto().into()), ..Default::default() };
    core.rt.notify_topic(&asset_topic(row.id), &public_view).await;
}
