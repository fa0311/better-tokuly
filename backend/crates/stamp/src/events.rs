//! Event topics and payloads of the stamp domain (see `docs/events/stamp.md`).
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── emitted ──

/// A creator sent a pack to review. Payload: [`PackEvent`].
pub const PACK_SUBMITTED: &str = "stamp.pack.submitted";
/// A pack went live in the store. Payload: [`PackEvent`].
pub const PACK_PUBLISHED: &str = "stamp.pack.published";
/// A moderator declined a pack. Payload: [`PackDecision`].
pub const PACK_REJECTED: &str = "stamp.pack.rejected";
/// A creator took a pack out of the store or out of review. Payload: [`PackEvent`].
pub const PACK_UNPUBLISHED: &str = "stamp.pack.unpublished";
/// Moderation took a pack down. Payload: [`PackDecision`].
pub const PACK_REMOVED: &str = "stamp.pack.removed";
/// A pack was deleted with its stamps. Payload: [`PackEvent`].
pub const PACK_DELETED: &str = "stamp.pack.deleted";
/// Payload: [`PackInstall`].
pub const PACK_INSTALLED: &str = "stamp.pack.installed";
/// Payload: [`PackInstall`].
pub const PACK_UNINSTALLED: &str = "stamp.pack.uninstalled";
/// A paid pack was bought. Payload: [`PackPurchase`].
pub const PACK_PURCHASED: &str = "stamp.pack.purchased";
/// A moderator changed a creator's trusted flag. Payload: [`CreatorTrust`].
pub const CREATOR_TRUST_CHANGED: &str = "stamp.creator.trust_changed";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackEvent {
    pub pack_id: Uuid,
    /// `None` for official packs.
    pub creator_id: Option<Uuid>,
    pub official: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackDecision {
    pub pack_id: Uuid,
    pub creator_id: Option<Uuid>,
    /// `None` when the moderator is unknown (e.g. an automated takedown).
    pub moderator_id: Option<Uuid>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackInstall {
    pub pack_id: Uuid,
    pub user_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackPurchase {
    pub pack_id: Uuid,
    pub buyer_id: Uuid,
    /// `None` for official packs (the coins went to the platform).
    pub creator_id: Option<Uuid>,
    /// Coins the buyer paid.
    pub price: i64,
    /// Coins the creator received (`price` minus the platform fee).
    pub net: i64,
    /// The wallet ledger entry of the payment.
    pub entry_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatorTrust {
    pub user_id: Uuid,
    pub trusted: bool,
    pub moderator_id: Uuid,
}

// ── consumed ──

pub const USER_DELETED: &str = "identity.user.deleted";
pub const ASSET_DELETED: &str = "media.asset.deleted";
pub const CONTENT_REMOVED: &str = "admin.content.removed";

/// `media.asset.deleted`: an upload and its files are gone. Only the field
/// the stamp domain needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetDeleted {
    pub asset_id: Uuid,
}

/// `admin.content.removed`: a moderator took content down. The stamp domain
/// acts on stamp packs and single stamps and ignores everything else.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentRemoved {
    /// `tokuly.common.v1.SubjectType`.
    pub subject_type: i32,
    pub subject_id: Uuid,
    pub reason: String,
    pub moderator_id: Uuid,
}
