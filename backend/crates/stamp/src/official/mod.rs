//! The official packs that ship with Tokuly: drawn in code ([`art`]),
//! rendered to WebP ([`canvas`]) and published by [`Stamp::seed_official_packs`].
use base64::Engine as _;
use image::{ExtendedColorType, codecs::webp::WebPEncoder};
use tokuly_core::{prelude::*, storage::Bucket};
use tokuly_proto::common::{Image, ImageVariant};

use crate::{
    Stamp,
    lifecycle::{self, Approval},
    model::{PackRow, pack_columns},
    packs,
    stamps::{self, FORMAT_STATIC, StampHome, StampSource},
    validate,
};

pub mod art;
pub mod canvas;

use art::{PackDesign, StampDesign};

/// Part of every storage key. Bump it when the drawings change: the files
/// are served as immutable, so new art needs new URLs.
const ART_VERSION: u32 = 1;
/// Pixel sizes of the stamp itself and of the variant used in picker grids.
const STAMP_SIZE: u32 = 512;
const THUMB_SIZE: u32 = 160;
/// Size of the render a thumbhash is computed from (the format allows up to 100).
const HASH_SIZE: u32 = 64;
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// A stamp rendered and encoded, ready to upload.
struct RenderedStamp {
    design: StampDesign,
    stamp_webp: Vec<u8>,
    thumb_webp: Vec<u8>,
    /// Base64, as in `Image.thumbhash`.
    thumbhash: String,
}

fn encode_webp(rgba: &[u8], size: u32) -> AppResult<Vec<u8>> {
    let mut out = Vec::new();
    WebPEncoder::new_lossless(&mut out)
        .encode(rgba, size, size, ExtendedColorType::Rgba8)
        .map_err(|e| AppError::internal(anyhow::anyhow!("webp encode: {e}")))?;
    Ok(out)
}

/// Renders one design at every size it is served in. CPU-bound.
fn render(design: StampDesign) -> AppResult<RenderedStamp> {
    let stamp_webp = encode_webp(&design.scene.render(STAMP_SIZE), STAMP_SIZE)?;
    let thumb_webp = encode_webp(&design.scene.render(THUMB_SIZE), THUMB_SIZE)?;
    let hash = thumbhash::rgba_to_thumb_hash(HASH_SIZE as usize, HASH_SIZE as usize, &design.scene.render(HASH_SIZE));
    let thumbhash = base64::engine::general_purpose::STANDARD.encode(hash);
    Ok(RenderedStamp { design, stamp_webp, thumb_webp, thumbhash })
}

impl Stamp {
    /// Creates the built-in official packs that do not exist yet.
    ///
    /// Idempotent and safe to run from several processes at once: a pack is
    /// identified by its `official_key`, and whoever inserts it first wins.
    /// Existing packs are left alone, so what the team changed in the studio
    /// afterwards is never overwritten.
    pub async fn seed_official_packs(&self) -> AppResult<()> {
        for design in art::packs() {
            let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM stamp_packs WHERE official_key = $1)")
                .bind(design.key)
                .fetch_one(&self.core.db)
                .await?;
            if exists {
                continue;
            }
            let key = design.key;
            if self.seed_pack(design).await? {
                tracing::info!(pack = key, "seeded official stamp pack");
            }
        }
        Ok(())
    }

    /// Renders, uploads and publishes one pack. Returns false when another
    /// process created it in the meantime.
    async fn seed_pack(&self, design: PackDesign) -> AppResult<bool> {
        let PackDesign { key, name, description, tags, stamps: designs } = design;
        let name = validate::pack_name(name)?;
        let description = validate::description(description)?;
        let tags = validate::tags(tags)?;

        // Drawing is CPU work: keep it off the async workers.
        let rendered =
            tokio::task::spawn_blocking(move || designs.into_iter().map(render).collect::<AppResult<Vec<_>>>())
                .await??;
        let mut prepared = Vec::with_capacity(rendered.len());
        for stamp in &rendered {
            let text = validate::stamp_text(stamp.design.name, stamp.design.emoji, stamp.design.keywords)?;
            prepared.push((self.upload(key, stamp).await?, text));
        }

        let mut tx = self.core.db.begin().await?;
        let pack: Option<PackRow> = sqlx::query_as(concat!(
            "INSERT INTO stamp_packs AS p (short_id, official, official_key, name, description, tags, featured_at)
             VALUES ($1, true, $2, $3, $4, $5, now())
             ON CONFLICT (official_key) DO NOTHING RETURNING ",
            pack_columns!()
        ))
        .bind(tokuly_core::ids::new_short_id())
        .bind(key)
        .bind(&name)
        .bind(&description)
        .bind(&tags)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(pack) = pack else { return Ok(false) };
        for (position, (source, text)) in prepared.iter().enumerate() {
            stamps::insert(
                &mut tx,
                StampHome::Pack { pack_id: pack.id, approved: true },
                source,
                text,
                position as i32,
            )
            .await?;
        }
        let pack = packs::refresh_derived(&mut tx, pack.id).await?;
        lifecycle::publish(&mut tx, &pack, Approval::Automatic, None).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Stores a rendered stamp in the media bucket and describes it as a
    /// stamp picture.
    async fn upload(&self, pack_key: &str, stamp: &RenderedStamp) -> AppResult<StampSource> {
        let storage = &self.core.storage;
        let base = format!("stamps/official/{pack_key}/{}-v{ART_VERSION}", stamp.design.key);
        let (stamp_key, thumb_key) = (format!("{base}.webp"), format!("{base}-{THUMB_SIZE}.webp"));
        storage
            .put_with_cache(Bucket::Media, &stamp_key, stamp.stamp_webp.clone(), "image/webp", Some(IMMUTABLE))
            .await?;
        storage
            .put_with_cache(Bucket::Media, &thumb_key, stamp.thumb_webp.clone(), "image/webp", Some(IMMUTABLE))
            .await?;
        let image = Image {
            url: storage.media_url(&stamp_key),
            width: STAMP_SIZE as i32,
            height: STAMP_SIZE as i32,
            thumbhash: stamp.thumbhash.clone(),
            dominant_color: stamp.design.color.hex(),
            variants: vec![ImageVariant {
                size: THUMB_SIZE as i32,
                url: storage.media_url(&thumb_key),
                ..Default::default()
            }],
            ..Default::default()
        };
        Ok(StampSource {
            format: FORMAT_STATIC,
            // Built-in art is not an upload: the files are ours, nothing to hold.
            asset_id: None,
            image_url: image.url.clone(),
            animation_url: String::new(),
            width: image.width,
            height: image.height,
            image: serde_json::to_value(&image)
                .map_err(|e| AppError::internal(anyhow::anyhow!("image encode: {e}")))?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendered_stamps_are_valid_webp_in_both_sizes() {
        let design = art::packs().remove(0).stamps.remove(0);
        let rendered = render(design).unwrap();
        for (bytes, size) in [(&rendered.stamp_webp, STAMP_SIZE), (&rendered.thumb_webp, THUMB_SIZE)] {
            assert_eq!(&bytes[..4], b"RIFF");
            assert_eq!(&bytes[8..12], b"WEBP");
            let decoded = image::load_from_memory_with_format(bytes, image::ImageFormat::WebP).unwrap().to_rgba8();
            assert_eq!((decoded.width(), decoded.height()), (size, size));
            assert_eq!(decoded.get_pixel(0, 0).0[3], 0, "transparent corner survives encoding");
        }
        let hash = base64::engine::general_purpose::STANDARD.decode(&rendered.thumbhash).unwrap();
        assert!(thumbhash::thumb_hash_to_rgba(&hash).is_ok());
    }
}
