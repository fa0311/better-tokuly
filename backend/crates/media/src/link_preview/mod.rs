//! Link previews: Open Graph / Twitter-card metadata of a web page, plus
//! recognition of links that embed (YouTube, Spotify, this instance).
//!
//! Other domains call [`link_preview`]; clients call
//! `MediaService.GetLinkPreview`. Both go through [`Previews::preview`]:
//!
//! 1. the URL is normalised and validated;
//! 2. links to this instance are answered without any request (the client
//!    renders them natively, with the viewer's own permissions);
//! 3. the `link_previews` table answers repeat lookups, including "nothing
//!    to show" results, so a URL pasted into a busy chat is fetched once;
//! 4. otherwise the page (or the provider's oEmbed document) is fetched
//!    under the limits in [`fetch`] and the SSRF rules of `tokuly_core::egress`.
//!
//! A failed or refused fetch is not an error for the caller: it simply
//! means there is no preview.
//!
//! The pictures of a preview are never handed out as the other site's
//! addresses: [`image_proxy`] serves them from this instance, so viewing a
//! preview tells the other site nothing about the viewer.
use std::{sync::OnceLock, time::Duration};

use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use tokuly_core::prelude::*;
use tokuly_proto::media::LinkPreview;
use url::Url;

pub mod embed;
pub mod fetch;
pub mod html;
pub mod image_proxy;

use embed::Embed;
use fetch::{FetchError, Page};
use tokuly_core::egress::Policy;

/// How long a found preview / a "nothing to show" result is reused.
const POSITIVE_TTL_SECONDS: i64 = 7 * 24 * 3600;
const NEGATIVE_TTL_SECONDS: i64 = 30 * 60;
/// Cached rows older than this are deleted by the janitor.
pub(crate) const CACHE_RETENTION_DAYS: i32 = 30;
const MAX_URL_LENGTH: usize = 2048;
/// Fetches per minute towards one server, across all users: we must not be
/// usable as a traffic amplifier against someone else's site.
const HOST_FETCHES_PER_MINUTE: u32 = 30;

const HTML_ACCEPT: &str = "text/html,application/xhtml+xml;q=0.9,*/*;q=0.5";
const JSON_ACCEPT: &str = "application/json";

/// The link preview fetcher: a guarded HTTP client and its policy.
pub struct Previews {
    client: reqwest::Client,
    policy: Policy,
    /// Bounds the pictures [`image_proxy`] decodes at once.
    decode_slots: tokio::sync::Semaphore,
}

/// Fetches (or recalls) the preview of `url`. `None` when the page has
/// nothing to show, cannot be reached, or may not be fetched. The only
/// error is `URL_INVALID` (besides infrastructure failures). Its
/// `image_url` and `favicon_url` are URLs of this instance
/// ([`image_proxy`]), whatever the page referred to.
///
/// Callers are expected to rate-limit their own users; the fetch itself is
/// throttled per target host.
pub async fn link_preview(core: &Core, url: &str) -> AppResult<Option<LinkPreview>> {
    static STRICT: OnceLock<Previews> = OnceLock::new();
    let previews = match STRICT.get() {
        Some(previews) => previews,
        None => {
            let built = Previews::new(Policy::default(), core.cfg.public_url()).map_err(AppError::internal)?;
            STRICT.get_or_init(|| built)
        }
    };
    previews.preview(core, url).await
}

/// Parses and normalises a user-supplied URL: http(s), with a host, without
/// fragment, of sane length.
pub fn normalize_url(input: &str) -> AppResult<Url> {
    let input = input.trim();
    ensure!(!input.is_empty() && input.len() <= MAX_URL_LENGTH, invalid, "URL_INVALID", "the URL is empty or too long");
    let mut url = Url::parse(input).map_err(|_| AppError::invalid("URL_INVALID", "not a valid URL"))?;
    ensure!(matches!(url.scheme(), "http" | "https") && url.host_str().is_some_and(|h| !h.is_empty()), invalid, "URL_INVALID", "only http(s) links have previews");
    url.set_fragment(None);
    Ok(url)
}

fn cache_key(url: &Url) -> String {
    hex::encode(Sha256::digest(url.as_str().as_bytes()))
}

/// The fields of an oEmbed response we use.
#[derive(Debug, Default, Deserialize)]
struct OEmbed {
    #[serde(default)]
    title: String,
    #[serde(default)]
    author_name: String,
    #[serde(default)]
    thumbnail_url: String,
    #[serde(default)]
    thumbnail_width: Option<i32>,
    #[serde(default)]
    thumbnail_height: Option<i32>,
}

/// An absolute http(s) URL for `reference` as found in `page`; empty when
/// it is missing or points anywhere else (`data:`, `javascript:`…).
fn resolve_reference(page: &Url, reference: &str) -> String {
    let reference = reference.trim();
    if reference.is_empty() {
        return String::new();
    }
    match page.join(reference) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => url.into(),
        _ => String::new(),
    }
}

/// The preview of an HTML page, or `None` when it offers nothing to show.
fn preview_from_html(requested: &Url, page: &Page) -> Option<LinkPreview> {
    let meta = html::extract(&html::decode(&page.body, &page.content_type));
    if meta.title.is_empty() && meta.description.is_empty() && meta.image.is_empty() {
        return None;
    }
    let favicon = if meta.favicon.is_empty() { "/favicon.ico" } else { &meta.favicon };
    let site_name = if meta.site_name.is_empty() { page.url.host_str().unwrap_or_default().trim_start_matches("www.").to_owned() } else { meta.site_name };
    Some(LinkPreview {
        url: requested.to_string(),
        title: meta.title,
        description: meta.description,
        site_name,
        image_url: resolve_reference(&page.url, &meta.image),
        favicon_url: resolve_reference(&page.url, favicon),
        image_width: meta.image_width,
        image_height: meta.image_height,
        ..Default::default()
    })
}

/// A link straight to an image previews as that image.
fn preview_from_image(requested: &Url, page: &Page) -> LinkPreview {
    let file_name = page.url.path_segments().and_then(|mut segments| segments.next_back()).unwrap_or_default();
    LinkPreview {
        url: requested.to_string(),
        title: file_name.to_owned(),
        site_name: page.url.host_str().unwrap_or_default().to_owned(),
        image_url: page.url.to_string(),
        ..Default::default()
    }
}

/// The preview of a recognised embed, from its oEmbed document when there is one.
fn preview_from_oembed(requested: &Url, embed: &Embed, document: Option<OEmbed>) -> LinkPreview {
    let document = document.unwrap_or_default();
    let image_url = if document.thumbnail_url.starts_with("https://") { document.thumbnail_url } else { embed.fallback_image().unwrap_or_default() };
    LinkPreview {
        url: requested.to_string(),
        title: document.title,
        description: document.author_name,
        site_name: embed.site_name().to_owned(),
        image_url,
        image_width: document.thumbnail_width.unwrap_or(0).max(0),
        image_height: document.thumbnail_height.unwrap_or(0).max(0),
        embed_provider: embed.provider.to_owned(),
        embed_id: embed.id.clone(),
        ..Default::default()
    }
}

impl Previews {
    pub fn new(policy: Policy, public_url: &str) -> anyhow::Result<Self> {
        // Sites vary their markup by client; identify as a link-preview bot.
        let user_agent = format!("Mozilla/5.0 (compatible; TokulyBot/1.0; +{public_url})");
        Ok(Self { client: fetch::guarded_client(policy, &user_agent)?, policy, decode_slots: tokio::sync::Semaphore::new(image_proxy::DECODE_SLOTS) })
    }

    /// See [`link_preview`].
    pub async fn preview(&self, core: &Core, input: &str) -> AppResult<Option<LinkPreview>> {
        // The cache keeps the pictures' own addresses (the proxy needs them
        // to sign); clients only ever get the proxied ones.
        Ok(self.preview_unproxied(core, input).await?.map(|preview| image_proxy::proxied(core, preview)))
    }

    async fn preview_unproxied(&self, core: &Core, input: &str) -> AppResult<Option<LinkPreview>> {
        let url = normalize_url(input)?;
        let embed = embed::recognise(&url, core.cfg.public_url());
        if embed::is_own(&url, core.cfg.public_url()) {
            // Our own pages are never fetched: the client shows them natively.
            return Ok(embed.map(|embed| LinkPreview {
                url: url.to_string(),
                site_name: embed.site_name().to_owned(),
                embed_provider: embed.provider.to_owned(),
                embed_id: embed.id,
                ..Default::default()
            }));
        }

        let key = cache_key(&url);
        if let Some(cached) = self.cached(core, &key).await? {
            return Ok(cached);
        }
        if !self.may_fetch_from(core, &url).await {
            return Ok(None); // not cached: the next minute may succeed
        }

        let preview = match &embed {
            Some(embed) => Some(self.fetch_embed(&url, embed).await),
            None => self.fetch_page(&url).await,
        };
        self.store(core, &key, &url, preview.as_ref()).await?;
        Ok(preview)
    }

    /// Whether the server of `url` may get one more request from us this
    /// minute (pages and pictures together).
    async fn may_fetch_from(&self, core: &Core, url: &Url) -> bool {
        let server = format!("{}:{}", url.host_str().unwrap_or_default(), url.port_or_known_default().unwrap_or(0));
        core.cache.rate_limit(&format!("media:preview-host:{server}"), HOST_FETCHES_PER_MINUTE, Duration::from_secs(60)).await.is_ok()
    }

    async fn fetch_page(&self, url: &Url) -> Option<LinkPreview> {
        match fetch::fetch(&self.client, self.policy, url.clone(), HTML_ACCEPT).await {
            Ok(page) if page.content_type.starts_with("text/html") || page.content_type.starts_with("application/xhtml") => preview_from_html(url, &page),
            Ok(page) if page.content_type.starts_with("image/") => Some(preview_from_image(url, &page)),
            Ok(_) => None,
            Err(error) => {
                log_failure(url, &error);
                None
            }
        }
    }

    /// Embeds always have a preview (the embed itself); the provider's
    /// oEmbed document only adds a title and thumbnail.
    async fn fetch_embed(&self, url: &Url, embed: &Embed) -> LinkPreview {
        let document = match embed.oembed_url(url) {
            Some(endpoint) => match fetch::fetch(&self.client, self.policy, endpoint, JSON_ACCEPT).await {
                Ok(page) => serde_json::from_slice::<OEmbed>(&page.body).ok(),
                Err(error) => {
                    log_failure(url, &error);
                    None
                }
            },
            None => None,
        };
        preview_from_oembed(url, embed, document)
    }

    /// `Some(result)` when the cache holds a fresh answer (which may be
    /// "nothing to show": `Some(None)`).
    async fn cached(&self, core: &Core, key: &str) -> AppResult<Option<Option<LinkPreview>>> {
        let row: Option<(Option<serde_json::Value>,)> = sqlx::query_as(
            "SELECT preview FROM link_previews
             WHERE url_hash = $1 AND fetched_at > now() - make_interval(secs => CASE WHEN preview IS NULL THEN $2 ELSE $3 END)",
        )
        .bind(key)
        .bind(NEGATIVE_TTL_SECONDS as f64)
        .bind(POSITIVE_TTL_SECONDS as f64)
        .fetch_optional(&core.db)
        .await?;
        Ok(row.map(|(preview,)| preview.and_then(|json| serde_json::from_value(json).ok())))
    }

    async fn store(&self, core: &Core, key: &str, url: &Url, preview: Option<&LinkPreview>) -> AppResult<()> {
        let json = preview.map(serde_json::to_value).transpose().map_err(|e| AppError::internal(anyhow::anyhow!("preview encode: {e}")))?;
        sqlx::query(
            "INSERT INTO link_previews (url_hash, url, preview, fetched_at) VALUES ($1, $2, $3, now())
             ON CONFLICT (url_hash) DO UPDATE SET url = EXCLUDED.url, preview = EXCLUDED.preview, fetched_at = now()",
        )
        .bind(key)
        .bind(url.as_str())
        .bind(json)
        .execute(&core.db)
        .await?;
        Ok(())
    }
}

fn log_failure(url: &Url, error: &FetchError) {
    match error {
        // Worth seeing: someone pointed the fetcher at something internal.
        FetchError::Refused(refusal) => tracing::warn!(host = url.host_str().unwrap_or_default(), %refusal, "link preview refused"),
        other => tracing::debug!(host = url.host_str().unwrap_or_default(), error = %other, "link preview fetch failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(url: &str, content_type: &str, body: &str) -> Page {
        Page { url: Url::parse(url).unwrap(), content_type: content_type.to_owned(), body: bytes::Bytes::copy_from_slice(body.as_bytes()), truncated: false }
    }

    #[test]
    fn urls_are_normalised_and_validated() {
        assert_eq!(normalize_url("  https://Example.com/a?b=1#frag ").unwrap().as_str(), "https://example.com/a?b=1");
        for bad in ["", "not a url", "ftp://example.com/x", "javascript:alert(1)", "file:///etc/passwd", "http://", "mailto:a@example.com"] {
            let error = normalize_url(bad).unwrap_err();
            assert_eq!(error.reason, "URL_INVALID", "{bad}");
        }
        assert!(normalize_url(&format!("https://example.com/{}", "a".repeat(3000))).is_err());
    }

    #[test]
    fn html_previews_resolve_relative_urls() {
        let requested = Url::parse("https://short.example/x").unwrap();
        let page = page(
            "https://www.example.com/articles/1",
            "text/html; charset=utf-8",
            r#"<title>記事</title><meta property="og:image" content="/img/cover.jpg"><link rel="icon" href="icon.png">"#,
        );
        let preview = preview_from_html(&requested, &page).unwrap();
        assert_eq!(preview.url, "https://short.example/x", "the preview is keyed by the requested URL");
        assert_eq!(preview.title, "記事");
        assert_eq!(preview.site_name, "example.com");
        assert_eq!(preview.image_url, "https://www.example.com/img/cover.jpg");
        assert_eq!(preview.favicon_url, "https://www.example.com/articles/icon.png");
    }

    #[test]
    fn pages_without_metadata_have_no_preview() {
        let requested = Url::parse("https://example.com/").unwrap();
        assert!(preview_from_html(&requested, &page("https://example.com/", "text/html", "<p>hi</p>")).is_none());
        // Without an explicit icon the conventional location is assumed.
        let titled = preview_from_html(&requested, &page("https://example.com/a/b", "text/html", "<title>T</title>")).unwrap();
        assert_eq!(titled.favicon_url, "https://example.com/favicon.ico");
    }

    #[test]
    fn unsafe_references_are_dropped() {
        let base = Url::parse("https://example.com/a").unwrap();
        assert_eq!(resolve_reference(&base, "javascript:alert(1)"), "");
        assert_eq!(resolve_reference(&base, "data:image/png;base64,AAAA"), "");
        assert_eq!(resolve_reference(&base, "//cdn.example.com/x.png"), "https://cdn.example.com/x.png");
        assert_eq!(resolve_reference(&base, "  "), "");
    }

    #[test]
    fn embeds_use_oembed_with_a_fallback() {
        let requested = Url::parse("https://youtu.be/dQw4w9WgXcQ").unwrap();
        let embed = embed::recognise(&requested, "https://tokuly.example").unwrap();
        let document: OEmbed = serde_json::from_str(
            r#"{"title":"Never Gonna Give You Up","author_name":"Rick Astley","thumbnail_url":"https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg","thumbnail_width":480,"thumbnail_height":360,"html":"<iframe></iframe>"}"#,
        )
        .unwrap();
        let preview = preview_from_oembed(&requested, &embed, Some(document));
        assert_eq!((preview.embed_provider.as_str(), preview.embed_id.as_str()), ("youtube", "dQw4w9WgXcQ"));
        assert_eq!((preview.title.as_str(), preview.description.as_str(), preview.site_name.as_str()), ("Never Gonna Give You Up", "Rick Astley", "YouTube"));
        assert_eq!((preview.image_width, preview.image_height), (480, 360));

        // Provider unreachable: still an embed, with the predictable thumbnail.
        let bare = preview_from_oembed(&requested, &embed, None);
        assert_eq!(bare.image_url, "https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg");
        assert_eq!(bare.embed_id, "dQw4w9WgXcQ");
    }

    #[test]
    fn image_links_preview_as_the_image() {
        let requested = Url::parse("https://example.com/cat.png").unwrap();
        let preview = preview_from_image(&requested, &page("https://example.com/cat.png", "image/png", ""));
        assert_eq!((preview.title.as_str(), preview.image_url.as_str()), ("cat.png", "https://example.com/cat.png"));
    }
}
