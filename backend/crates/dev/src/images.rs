//! Uploaded images, for application icons and bot avatars.
//!
//! Images live in the media domain, which this crate does not depend on.
//! The server bridges this trait to `tokuly_media::assets`
//! (`crates/server/src/bridges.rs`); tests use a stand-in.
//!
//! Looking an image up and holding it are separate steps on purpose: the
//! lookup reads through the connection pool, so it has to happen *before*
//! the caller opens its transaction. A handler that held a transaction
//! while waiting for a second connection could be starved of it by a few
//! concurrent requests, and would stall every other request with it.
use async_trait::async_trait;
use sqlx::PgConnection;
use tokuly_core::prelude::*;

#[async_trait]
pub trait ImageStore: Send + Sync + 'static {
    /// The image `uploader` uploaded as `asset_id`, as
    /// `tokuly.common.v1.Image` proto JSON. Fails with the media domain's
    /// reasons when the asset does not exist, is someone else's, is not an
    /// image or is not ready. Call it before opening a transaction.
    async fn image(&self, uploader: Uuid, asset_id: Uuid) -> AppResult<serde_json::Value>;

    /// Makes `holder` (e.g. `dev:app-icon:<app id>`) the thing that keeps
    /// the asset alive, releasing whatever it held before. Runs on the
    /// caller's transaction only, so the hold is stored or rolled back with
    /// the reference.
    async fn hold(&self, tx: &mut PgConnection, asset_id: Uuid, holder: &str) -> AppResult<()>;

    /// Releases everything `holder` holds (the image was removed, or the
    /// application or bot is gone). Runs on the caller's transaction only.
    async fn release(&self, tx: &mut PgConnection, holder: &str) -> AppResult<()>;
}

/// A change to an image slot, resolved before the transaction that stores it.
pub(crate) enum ImageChange {
    Keep,
    Remove,
    Set { asset_id: Uuid, image: serde_json::Value },
}

impl ImageChange {
    /// Resolves what a request asks for: `None` keeps the image, an empty
    /// ID removes it, anything else must be an image `uploader` uploaded.
    pub(crate) async fn resolve(dev: &crate::Dev, uploader: Uuid, asset_id: Option<&str>) -> AppResult<Self> {
        match asset_id {
            None => Ok(Self::Keep),
            Some("") => Ok(Self::Remove),
            Some(id) => {
                let asset_id = parse_id(id)?;
                Ok(Self::Set { asset_id, image: dev.images()?.image(uploader, asset_id).await? })
            }
        }
    }

    /// Applies the hold on the caller's transaction. Returns the new value
    /// of the image column when it changes (`Some(None)` clears it).
    pub(crate) async fn apply(
        self,
        dev: &crate::Dev,
        tx: &mut PgConnection,
        holder: &str,
    ) -> AppResult<Option<Option<serde_json::Value>>> {
        match self {
            Self::Keep => Ok(None),
            Self::Remove => {
                // Where no image store is connected nothing was ever held.
                if let Ok(images) = dev.images() {
                    images.release(tx, holder).await?;
                }
                Ok(Some(None))
            }
            Self::Set { asset_id, image } => {
                dev.images()?.hold(tx, asset_id, holder).await?;
                Ok(Some(Some(image)))
            }
        }
    }
}

pub(crate) fn app_icon_holder(app_id: Uuid) -> String {
    format!("dev:app-icon:{app_id}")
}

pub(crate) fn bot_avatar_holder(bot_id: Uuid) -> String {
    format!("dev:bot-avatar:{bot_id}")
}
