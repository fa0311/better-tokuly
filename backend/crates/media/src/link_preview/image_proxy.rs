//! The images of link previews, served by this instance.
//!
//! A preview's picture (`og:image`, an oEmbed thumbnail, an image link) and
//! its site icon live on someone else's server. If viewers' apps loaded them
//! from there, every view of a message or note with a link would tell that
//! server who looked, from where, and when. So a preview never carries those
//! addresses: [`proxied`] replaces them with a URL of this instance,
//!
//! ```text
//! {public_url}{api_path_prefix}/media/link-image/{kind}/{signature}/{target}
//! ```
//!
//! where `kind` is `image` or `icon`, `target` the original URL in
//! base64url and `signature` a MAC of both under the instance secret. The
//! route ([`serve`]) answers only URLs it signed itself, so it cannot be used
//! to fetch arbitrary addresses; what it fetches goes through the egress
//! guard of `tokuly_core::egress` like the preview itself, within a time and
//! size limit, and only a decodable raster image comes back, re-encoded as
//! WebP at a bounded size (nothing of the original file reaches the viewer:
//! no metadata, no SVG scripts, no polyglot files). Results, failures
//! included, are kept in Valkey for a while, so a popular link costs the
//! other site one request, not one per viewer.
use std::{io::Cursor, time::Duration};

use axum::{
    Extension, Router,
    extract::{Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use image::{DynamicImage, ImageReader, Limits};
use sha2::{Digest as _, Sha256};
use tokuly_core::{auth::ClientMeta, keys, prelude::*};
use tokuly_proto::media::LinkPreview;
use url::Url;

use super::{Previews, fetch};
use crate::{Media, process::image as raster};

/// Where the route is mounted, below the API's own root.
pub const ROUTE: &str = "/media/link-image";

/// The largest download accepted for one picture.
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
/// Pictures with more pixels than this are not decoded (the API process
/// does this work, so the bound is far below that of uploads).
pub const MAX_PIXELS: u64 = 16_000_000;
const MAX_DIMENSION: u32 = 8192;
const MAX_DECODE_BYTES: u64 = 128 * 1024 * 1024;
const WEBP_QUALITY: f32 = 80.0;

/// How long a picture (or "there is none") is reused, by us and by caches.
pub const CACHE_SECONDS: u64 = 24 * 3600;
const FAILURE_CACHE_SECONDS: u64 = 30 * 60;
/// Pictures one address may have fetched (not served from the cache) per minute.
const FETCHES_PER_MINUTE_PER_CLIENT: u32 = 120;
/// Decodes running at once in one API process.
pub(crate) const DECODE_SLOTS: usize = 2;

const IMAGE_ACCEPT: &str = "image/webp,image/png,image/jpeg,image/gif,image/x-icon;q=0.9,image/*;q=0.5";

/// Which picture of a preview: it decides the size it is served at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The preview's picture: at most 1200 px on its longest edge.
    Image,
    /// The site icon: at most 128 px.
    Icon,
}

impl Kind {
    fn segment(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Icon => "icon",
        }
    }

    fn parse(segment: &str) -> Option<Self> {
        match segment {
            "image" => Some(Self::Image),
            "icon" => Some(Self::Icon),
            _ => None,
        }
    }

    fn longest_edge(self) -> u32 {
        match self {
            Self::Image => 1200,
            Self::Icon => 128,
        }
    }
}

fn signature(core: &Core, kind: Kind, target: &str) -> String {
    let mac = core.keys.mac("media/link-image", format!("{}\n{target}", kind.segment()).as_bytes());
    keys::b64url(&mac[..18])
}

/// The URL of this instance that serves the picture at `target`. Empty for
/// an empty `target`, and for anything that is not an absolute http(s) URL.
pub fn proxy_url(core: &Core, kind: Kind, target: &str) -> String {
    if !Url::parse(target).is_ok_and(|url| matches!(url.scheme(), "http" | "https")) {
        return String::new();
    }
    format!(
        "{}{}{ROUTE}/{}/{}/{}",
        core.cfg.public_url(),
        core.cfg.api_path_prefix(),
        kind.segment(),
        signature(core, kind, target),
        keys::b64url(target.as_bytes()),
    )
}

/// The preview as clients get it: its pictures come from this instance.
pub fn proxied(core: &Core, mut preview: LinkPreview) -> LinkPreview {
    preview.image_url = proxy_url(core, Kind::Image, &preview.image_url);
    preview.favicon_url = proxy_url(core, Kind::Icon, &preview.favicon_url);
    preview
}

/// The route; mount it on the API's HTTP router.
pub fn routes(media: Media) -> Router {
    Router::new().route(&format!("{ROUTE}/{{kind}}/{{signature}}/{{target}}"), get(serve)).with_state(media)
}

/// Why there is no picture to serve.
#[derive(Debug, PartialEq, Eq)]
enum Missing {
    /// Not a URL we signed: never anything else, whatever the URL says.
    Unknown,
    /// Fetched, and there is nothing to show (remembered for a while).
    Nothing,
    /// The other site gets no more requests from us right now, or this
    /// client asked for too many: try again later (not remembered).
    Busy,
}

impl IntoResponse for Missing {
    fn into_response(self) -> Response {
        let (status, cache) = match self {
            Self::Unknown => (StatusCode::NOT_FOUND, "public, max-age=86400".to_owned()),
            Self::Nothing => (StatusCode::NOT_FOUND, format!("public, max-age={FAILURE_CACHE_SECONDS}")),
            Self::Busy => (StatusCode::SERVICE_UNAVAILABLE, "no-store".to_owned()),
        };
        let mut response = status.into_response();
        let headers = response.headers_mut();
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_str(&cache).expect("ASCII"));
        if self == Self::Busy {
            headers.insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
        }
        response
    }
}

async fn serve(
    State(media): State<Media>,
    Path((kind, signed, target)): Path<(String, String, String)>,
    client: Option<Extension<ClientMeta>>,
) -> Response {
    let client_ip = client.map(|Extension(meta)| meta.ip).unwrap_or_default();
    match media.previews.picture(&media.core, &kind, &signed, &target, &client_ip).await {
        Ok(webp) => {
            let mut response = webp.into_response();
            let headers = response.headers_mut();
            headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/webp"));
            headers.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_str(&format!("public, max-age={CACHE_SECONDS}")).expect("ASCII"),
            );
            headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
            headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; sandbox"));
            response
        }
        Err(missing) => missing.into_response(),
    }
}

impl Previews {
    /// The WebP to serve for a signed picture URL.
    async fn picture(
        &self,
        core: &Core,
        kind: &str,
        signed: &str,
        target: &str,
        client_ip: &str,
    ) -> Result<Vec<u8>, Missing> {
        let kind = Kind::parse(kind).ok_or(Missing::Unknown)?;
        let target =
            keys::b64url_decode(target).and_then(|bytes| String::from_utf8(bytes).ok()).ok_or(Missing::Unknown)?;
        if !keys::secure_eq(signature(core, kind, &target).as_bytes(), signed.as_bytes()) {
            return Err(Missing::Unknown);
        }
        let url = Url::parse(&target).map_err(|_| Missing::Unknown)?;

        let key = format!("media:link-image:{}:{}", kind.segment(), hex::encode(Sha256::digest(target.as_bytes())));
        let mut redis = core.cache.conn();
        match redis::cmd("GET").arg(&key).query_async::<Option<Vec<u8>>>(&mut redis).await {
            Ok(Some(cached)) if cached.is_empty() => return Err(Missing::Nothing),
            Ok(Some(cached)) => return Ok(cached),
            Ok(None) => {}
            Err(e) => tracing::warn!(error = %e, "link image cache unavailable"),
        }

        // A miss costs a fetch and a decode: bounded per client, and per
        // site across everyone (shared with the preview fetches).
        if !client_ip.is_empty()
            && core
                .cache
                .rate_limit(
                    &format!("media:link-image-client:{client_ip}"),
                    FETCHES_PER_MINUTE_PER_CLIENT,
                    Duration::from_secs(60),
                )
                .await
                .is_err()
        {
            return Err(Missing::Busy);
        }
        if !self.may_fetch_from(core, &url).await {
            return Err(Missing::Busy);
        }

        let rendered = self.fetch_picture(&url, kind).await;
        let (value, ttl) = match &rendered {
            Some(webp) => (webp.as_slice(), CACHE_SECONDS),
            None => (&[][..], FAILURE_CACHE_SECONDS),
        };
        if let Err(e) = redis::cmd("SET").arg(&key).arg(value).arg("EX").arg(ttl).query_async::<()>(&mut redis).await {
            tracing::warn!(error = %e, "link image not cached");
        }
        rendered.ok_or(Missing::Nothing)
    }

    async fn fetch_picture(&self, url: &Url, kind: Kind) -> Option<Vec<u8>> {
        let page =
            match fetch::fetch_limited(&self.client, self.policy, url.clone(), IMAGE_ACCEPT, MAX_IMAGE_BYTES).await {
                Ok(page) => page,
                Err(error) => {
                    super::log_failure(url, &error);
                    return None;
                }
            };
        if page.truncated || !is_raster_type(&page.content_type) {
            tracing::debug!(host = url.host_str().unwrap_or_default(), content_type = %page.content_type, truncated = page.truncated, "link image refused");
            return None;
        }
        let _slot = self.decode_slots.acquire().await.ok()?;
        let body = page.body;
        match tokio::task::spawn_blocking(move || render(&body, kind)).await {
            Ok(rendered) => rendered,
            Err(e) => {
                tracing::warn!(error = %e, "link image decoder failed");
                None
            }
        }
    }
}

/// Whether a `Content-Type` announces a raster image. SVG is not one (it is
/// a document that can carry scripts, and would need a renderer).
fn is_raster_type(content_type: &str) -> bool {
    let essence = content_type.split(';').next().unwrap_or_default().trim();
    essence.starts_with("image/") && !essence.starts_with("image/svg")
}

/// Decodes a picture and re-encodes it as WebP, no larger than `kind` allows.
/// `None` when it is not an image we decode, or too large to.
pub(crate) fn render(bytes: &[u8], kind: Kind) -> Option<Vec<u8>> {
    let image = decode(bytes)?;
    let image = raster::normalize(image);
    let (width, height) = raster::scaled_dimensions(image.width(), image.height(), kind.longest_edge());
    let image = raster::resize(&image, width, height);
    raster::encode_webp(&image, WEBP_QUALITY).ok()
}

fn decode(bytes: &[u8]) -> Option<DynamicImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?;
    reader.format()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let decoder = reader.into_decoder().ok()?;
    let (width, height) = image::ImageDecoder::dimensions(&decoder);
    if u64::from(width) * u64::from(height) > MAX_PIXELS || width == 0 || height == 0 {
        return None;
    }
    DynamicImage::from_decoder(decoder).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_raster_types_are_accepted() {
        for accepted in
            ["image/png", "image/jpeg; charset=binary", "image/x-icon", "image/vnd.microsoft.icon", "image/webp"]
        {
            assert!(is_raster_type(accepted), "{accepted}");
        }
        for refused in
            ["image/svg+xml", "text/html", "application/octet-stream", "", "application/xhtml+xml; image/png"]
        {
            assert!(!is_raster_type(refused), "{refused}");
        }
    }

    #[test]
    fn pictures_are_re_encoded_within_bounds() {
        let mut png = Vec::new();
        DynamicImage::ImageRgb8(image::RgbImage::from_pixel(3000, 1500, image::Rgb([200, 30, 30])))
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let webp = render(&png, Kind::Image).expect("rendered");
        assert_eq!(image::guess_format(&webp).unwrap(), image::ImageFormat::WebP);
        let decoded = image::load_from_memory(&webp).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1200, 600));
        let icon = image::load_from_memory(&render(&png, Kind::Icon).unwrap()).unwrap();
        assert_eq!((icon.width(), icon.height()), (128, 64));

        // `/favicon.ico`, the icon most sites have.
        let mut ico = Vec::new();
        DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(48, 48, image::Rgba([0, 90, 200, 255])))
            .write_to(&mut Cursor::new(&mut ico), image::ImageFormat::Ico)
            .unwrap();
        let favicon = image::load_from_memory(&render(&ico, Kind::Icon).expect("ICO decodes")).unwrap();
        assert_eq!((favicon.width(), favicon.height()), (48, 48));

        // Not an image, an SVG, a truncated file: nothing.
        assert!(render(b"<html>hi</html>", Kind::Image).is_none());
        assert!(
            render(br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#, Kind::Icon).is_none()
        );
        assert!(render(&png[..png.len() / 3], Kind::Image).is_none());
    }
}
