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
//! included, are kept for a day (the WebP in the private bucket under
//! `link-images/`, a row in `link_preview_images`; the janitor expires
//! both), so a popular link costs the other site one request, not one per
//! viewer.
//!
//! Pictures on this instance's own media origin (`TOKULY_MEDIA_URL`) are
//! passed through unchanged: proxying them would keep a copy alive after
//! the original was deleted or taken down. Anything else on this instance's
//! own origin is dropped.
use std::{collections::HashMap, io::Cursor, net::IpAddr, sync::Mutex, time::Duration};

use axum::{
    Extension, Router,
    extract::{Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use image::{DynamicImage, ImageReader, Limits};
use sha2::{Digest as _, Sha256};
use tokuly_core::{auth::ClientMeta, keys, prelude::*, storage::Bucket};
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
/// Pictures not in the cache one address may ask for per minute.
const FETCHES_PER_MINUTE_PER_CLIENT: u32 = 120;
/// Pictures fetched (and then decoded) at once in one API process, so
/// parallel requests cannot pile up downloads in memory.
pub(crate) const WORK_SLOTS: usize = 4;
/// Of those, decodes running at once.
pub(crate) const DECODE_SLOTS: usize = 2;
/// How long a miss that finds every slot taken waits for one (a news list
/// asks for a dozen new pictures at once) before it is answered `503`.
const SLOT_WAIT: Duration = Duration::from_secs(5);
/// Misses waiting for a slot in one API process, and of those, from one
/// client: beyond them a miss is answered `503` at once, so nobody can park
/// hundreds of requests here.
const WAITING: usize = 64;
const WAITING_PER_CLIENT: usize = 16;
/// Picture fetches towards one site per minute, across all viewers; a
/// budget of its own so viewers' misses do not starve page previews.
const HOST_FETCHES_PER_MINUTE: u32 = 60;

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

/// Where a picture lives, seen from this instance.
enum Origin {
    /// This instance's public media: served as it is (and gone when it is).
    OwnMedia,
    /// Anything else of this instance: never shown, never fetched.
    Own,
    Elsewhere,
}

fn origin_of(core: &Core, url: &Url) -> Origin {
    let media = format!("{}/", core.cfg.media_url());
    if url.as_str().starts_with(&media) {
        return Origin::OwnMedia;
    }
    let same_origin = |base: &str| Url::parse(base).is_ok_and(|base| base.origin() == url.origin());
    if same_origin(core.cfg.public_url()) || same_origin(&media) { Origin::Own } else { Origin::Elsewhere }
}

/// The URL clients get for the picture at `target`: a URL of this
/// instance that serves it, the URL itself when it is this instance's own
/// public media, empty for anything else (empty, not http(s), another URL
/// of this instance).
pub fn proxy_url(core: &Core, kind: Kind, target: &str) -> String {
    let Some(url) = Url::parse(target).ok().filter(|url| matches!(url.scheme(), "http" | "https")) else {
        return String::new();
    };
    match origin_of(core, &url) {
        Origin::OwnMedia => return target.to_owned(),
        Origin::Own => return String::new(),
        Origin::Elsewhere => {}
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
    /// The other site gets no more requests from us right now, this client
    /// asked for too many, or no slot came free in time: try again later
    /// (not remembered).
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
        // Signed before the rule existed, or by a configuration with another
        // media URL: this instance's own files are never copied.
        if !matches!(origin_of(core, &url), Origin::Elsewhere) {
            return Err(Missing::Unknown);
        }

        let cache_key = format!("{}/{}", kind.segment(), hex::encode(Sha256::digest(target.as_bytes())));
        match cached(core, &cache_key).await {
            Ok(Some(Some(webp))) => return Ok(webp),
            Ok(Some(None)) => return Err(Missing::Nothing),
            Ok(None) => {}
            Err(e) => tracing::warn!(error = %e, "link image cache unavailable"),
        }

        // A miss costs a download and a decode: a bounded number per client,
        // at once (a slot taken before anything is downloaded), and per site
        // across everyone.
        let client = client_key(client_ip);
        if let Some(client) = &client
            && core
                .cache
                .rate_limit(
                    &format!("media:link-image-client:{client}"),
                    FETCHES_PER_MINUTE_PER_CLIENT,
                    Duration::from_secs(60),
                )
                .await
                .is_err()
        {
            return Err(Missing::Busy);
        }
        let _slot = self.work_slot(client).await.ok_or(Missing::Busy)?;
        let server = format!("{}:{}", url.host_str().unwrap_or_default(), url.port_or_known_default().unwrap_or(0));
        if core
            .cache
            .rate_limit(&format!("media:link-image-host:{server}"), HOST_FETCHES_PER_MINUTE, Duration::from_secs(60))
            .await
            .is_err()
        {
            return Err(Missing::Busy);
        }

        let rendered = self.fetch_picture(&url, kind).await;
        if let Err(e) = remember(core, &cache_key, rendered.as_deref()).await {
            tracing::warn!(error = %e, "link image not cached");
        }
        rendered.ok_or(Missing::Nothing)
    }

    /// A work slot: free now, or within [`SLOT_WAIT`] if there is room to
    /// wait for one.
    async fn work_slot(&self, client: Option<String>) -> Option<tokio::sync::SemaphorePermit<'_>> {
        if let Ok(slot) = self.work_slots.try_acquire() {
            return Some(slot);
        }
        let _place = self.waiting.enter(client)?;
        tokio::time::timeout(SLOT_WAIT, self.work_slots.acquire()).await.ok()?.ok()
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

/// The misses waiting for a work slot: at most [`WAITING`], and at most
/// [`WAITING_PER_CLIENT`] of one client.
pub(super) struct Waiting {
    places: tokio::sync::Semaphore,
    per_client: Mutex<HashMap<String, usize>>,
}

/// A place in the queue, given up when dropped.
struct Place<'a> {
    _place: tokio::sync::SemaphorePermit<'a>,
    client: Option<(&'a Waiting, String)>,
}

impl Waiting {
    pub(super) fn new() -> Self {
        Self { places: tokio::sync::Semaphore::new(WAITING), per_client: Mutex::default() }
    }

    /// A place for a miss of `client`; `None` when the queue, or the
    /// client's share of it, is full.
    fn enter(&self, client: Option<String>) -> Option<Place<'_>> {
        let place = self.places.try_acquire().ok()?;
        if let Some(client) = &client {
            let mut counts = self.per_client.lock().expect("not poisoned");
            let count = counts.entry(client.clone()).or_default();
            if *count >= WAITING_PER_CLIENT {
                return None;
            }
            *count += 1;
        }
        Some(Place { _place: place, client: client.map(|client| (self, client)) })
    }
}

impl Drop for Place<'_> {
    fn drop(&mut self) {
        let Some((waiting, client)) = &self.client else { return };
        let mut counts = waiting.per_client.lock().expect("not poisoned");
        if let Some(count) = counts.get_mut(client) {
            *count -= 1;
            if *count == 0 {
                counts.remove(client);
            }
        }
    }
}

/// The object holding a cached picture.
fn object_key(cache_key: &str) -> String {
    format!("{OBJECT_PREFIX}{cache_key}.webp")
}

/// Prefix of the cached pictures in the private bucket.
pub(crate) const OBJECT_PREFIX: &str = "link-images/";

/// `Some(Some(webp))`: a fresh picture; `Some(None)`: fresh "nothing to
/// show"; `None`: not cached (or its object is missing).
async fn cached(core: &Core, cache_key: &str) -> AppResult<Option<Option<Vec<u8>>>> {
    let found: Option<bool> = sqlx::query_scalar(
        "SELECT found FROM link_preview_images
         WHERE cache_key = $1 AND fetched_at > now() - make_interval(secs => CASE WHEN found THEN $2 ELSE $3 END)",
    )
    .bind(cache_key)
    .bind(CACHE_SECONDS as f64)
    .bind(FAILURE_CACHE_SECONDS as f64)
    .fetch_optional(&core.db)
    .await?;
    match found {
        None => Ok(None),
        Some(false) => Ok(Some(None)),
        Some(true) => match core.storage.get(Bucket::Private, &object_key(cache_key)).await {
            Ok(bytes) => Ok(Some(Some(bytes.to_vec()))),
            Err(_) => Ok(None),
        },
    }
}

async fn remember(core: &Core, cache_key: &str, webp: Option<&[u8]>) -> AppResult<()> {
    if let Some(webp) = webp {
        core.storage.put(Bucket::Private, &object_key(cache_key), webp.to_vec(), "image/webp").await?;
    }
    sqlx::query(
        "INSERT INTO link_preview_images (cache_key, found, size_bytes, fetched_at) VALUES ($1, $2, $3, now())
         ON CONFLICT (cache_key) DO UPDATE SET found = EXCLUDED.found, size_bytes = EXCLUDED.size_bytes, fetched_at = now()",
    )
    .bind(cache_key)
    .bind(webp.is_some())
    .bind(webp.map_or(0, |webp| webp.len() as i32))
    .execute(&core.db)
    .await?;
    Ok(())
}

/// Removes cached pictures older than their lifetime, rows and objects.
/// Returns how many entries went.
pub(crate) async fn expire(core: &Core, limit: i64) -> AppResult<u64> {
    let rows: Vec<(String, bool)> = sqlx::query_as(
        "SELECT cache_key, found FROM link_preview_images
         WHERE fetched_at < now() - make_interval(secs => $1) ORDER BY fetched_at LIMIT $2",
    )
    .bind(CACHE_SECONDS as f64)
    .bind(limit)
    .fetch_all(&core.db)
    .await?;
    let mut expired = 0;
    for (cache_key, found) in rows {
        if found {
            core.storage.delete(Bucket::Private, &object_key(&cache_key)).await?;
        }
        // Only if nobody fetched it again meanwhile (its object is then new).
        let done = sqlx::query(
            "DELETE FROM link_preview_images WHERE cache_key = $1 AND fetched_at < now() - make_interval(secs => $2)",
        )
        .bind(&cache_key)
        .bind(CACHE_SECONDS as f64)
        .execute(&core.db)
        .await?;
        expired += done.rows_affected();
    }
    Ok(expired)
}

/// What the per-client limit counts: the address, or its /64 for IPv6
/// (one subscriber usually has a whole /64). `None` when unknown.
fn client_key(ip: &str) -> Option<String> {
    match ip.parse::<IpAddr>().ok()? {
        IpAddr::V4(v4) => Some(v4.to_string()),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => Some(v4.to_string()),
            None => {
                let s = v6.segments();
                Some(format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3]))
            }
        },
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
    fn clients_are_counted_by_address_or_ipv6_64() {
        assert_eq!(client_key("203.0.113.7").as_deref(), Some("203.0.113.7"));
        assert_eq!(client_key("2001:db8:1:2:aaaa::1"), client_key("2001:db8:1:2:ffff:ffff:ffff:ffff"));
        assert_eq!(client_key("2001:db8:1:2::1").as_deref(), Some("2001:db8:1:2::/64"));
        assert_ne!(client_key("2001:db8:1:2::1"), client_key("2001:db8:1:3::1"));
        assert_eq!(client_key("::ffff:203.0.113.7").as_deref(), Some("203.0.113.7"));
        assert_eq!(client_key(""), None);
    }

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
