//! The file recipe: files are stored untouched, so there is little to do.
//!
//! - Chat files and subtitles stay in the private bucket and are served
//!   through presigned URLs issued on read (see `assets`).
//! - dotLottie stamps are shown to everyone, so they are validated and
//!   copied to the media bucket under a content-addressed key.
use tokuly_core::storage::Bucket;

use super::{ProcessError, ProcessResult};
use crate::{Media, assets::AssetRow, lifecycle::Processed, recipe, store, upload::checksum_deferred};

impl Media {
    pub(crate) async fn process_file(&self, asset: &AssetRow) -> ProcessResult<Processed> {
        let core = &self.core;
        // Small files had their checksum verified when the upload completed.
        if checksum_deferred(asset) && self.originals.sha256(&asset.original_key).await? != asset.sha256 {
            return Err(ProcessError::rejected("CHECKSUM_MISMATCH", "sha256 differs from the one announced"));
        }
        if !recipe::publishes_files(asset.asset_purpose()) {
            return Ok(Processed::default());
        }

        let bytes = core.storage.get(Bucket::Private, &asset.original_key).await?;
        if !recipe::is_dot_lottie(&bytes) {
            return Err(ProcessError::rejected("UNSUPPORTED_CONTENT", "the archive is not a dotLottie animation"));
        }
        let key = store::public_file_key(asset.id, &store::content_hash(&bytes), "lottie");
        core.storage.put_with_cache(Bucket::Media, &key, bytes, recipe::LOTTIE_CONTENT_TYPE, Some(store::IMMUTABLE)).await?;
        Ok(Processed { url: core.storage.media_url(&key), content_type: Some(recipe::LOTTIE_CONTENT_TYPE.to_owned()), ..Default::default() })
    }
}
