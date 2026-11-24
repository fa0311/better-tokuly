//! The image recipe: original → WebP variants (and an animated WebP when the
//! upload is animated and the purpose keeps animations) in the media bucket.
use tokuly_core::{prelude::*, storage::Bucket};
use tokuly_proto::{
    common::{Image, ImageVariant},
    media::AssetKind,
};

use super::{ProcessError, ProcessResult, Progress, animation, blocking, image};
use crate::{Media, assets::AssetRow, lifecycle::Processed, recipe, store, upload::checksum_deferred};

const WEBP: &str = "image/webp";

impl Media {
    pub(crate) async fn process_image(&self, asset: &AssetRow, progress: &Progress) -> ProcessResult<Processed> {
        let core = &self.core;
        let bytes = core.storage.get(Bucket::Private, &asset.original_key).await?;
        if checksum_deferred(asset) && store::sha256_hex(&bytes) != asset.sha256 {
            return Err(ProcessError::rejected("CHECKSUM_MISMATCH", "sha256 differs from the one announced"));
        }
        progress.set(10).await?;

        let animated = recipe::keeps_animation(asset.asset_purpose()) && {
            let bytes = bytes.clone();
            blocking(move || Ok(animation::is_animated(&bytes))).await?
        };
        if animated {
            // One animation at a time: its frames are all held in memory.
            let _slot = self.animation_slot.acquire().await.map_err(ProcessError::transient)?;
            let frames = bytes.clone();
            if let Some(rendered) = blocking(move || animation::render(&frames)).await? {
                progress.set(60).await?;
                tracing::debug!(asset_id = %asset.id, width = rendered.width, height = rendered.height, frames = rendered.frame_count, "animation rendered");
                let poster = self.publish_image(asset.id, rendered.poster).await?;
                let key = store::animation_key(asset.id, &store::content_hash(&rendered.webp));
                core.storage.put_with_cache(Bucket::Media, &key, rendered.webp, WEBP, Some(store::PUBLIC_CACHE)).await?;
                return Ok(Processed {
                    kind: Some(AssetKind::ASSET_KIND_ANIMATION),
                    url: core.storage.media_url(&key),
                    image: Some(poster),
                    ..Default::default()
                });
            }
        }

        let decoded = blocking(move || image::decode(&bytes)).await?;
        progress.set(40).await?;
        let image = self.publish_image(asset.id, decoded).await?;
        Ok(Processed { kind: Some(AssetKind::ASSET_KIND_IMAGE), url: image.url.clone(), image: Some(image), ..Default::default() })
    }

    /// Renders a decoded image and stores its variants under
    /// content-addressed keys. Also used for video posters.
    pub(crate) async fn publish_image(&self, asset_id: Uuid, decoded: ::image::DynamicImage) -> ProcessResult<Image> {
        let storage = &self.core.storage;
        let rendered = blocking(move || image::render(decoded)).await?;

        let mut variants = Vec::with_capacity(rendered.variants.len());
        let (mut width, mut height) = (0, 0);
        for (index, variant) in rendered.variants.into_iter().enumerate() {
            if index == 0 {
                (width, height) = (variant.width, variant.height);
            }
            let key = store::image_key(asset_id, variant.size, &store::content_hash(&variant.webp));
            storage.put_with_cache(Bucket::Media, &key, variant.webp, WEBP, Some(store::PUBLIC_CACHE)).await?;
            variants.push(ImageVariant { size: variant.size as i32, url: storage.media_url(&key), ..Default::default() });
        }
        Ok(Image {
            asset_id: asset_id.to_string(),
            // Variants are ordered largest first; the largest is the canonical one.
            url: variants.first().map(|v| v.url.clone()).unwrap_or_default(),
            width: width as i32,
            height: height as i32,
            thumbhash: rendered.thumbhash,
            dominant_color: rendered.dominant_color,
            variants,
            ..Default::default()
        })
    }
}
