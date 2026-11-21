//! Link previews against a local HTTP server. Reaching loopback is exactly
//! what the fetcher must refuse, so these tests enable it through the
//! test-only configuration switch — and verify that without the switch the
//! same URLs are refused.
mod support;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use support::*;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
};
use tokuly_media::MediaConfig;
use tokuly_proto::media::*;

const ARTICLE: &str = r#"<!doctype html><html><head>
<meta charset="utf-8">
<title>ignored in favour of og:title</title>
<meta property="og:title" content="今日のおでかけ &amp; ごはん">
<meta property="og:description" content="友達と位置情報共有を楽しもう!">
<meta property="og:site_name" content="Example Times">
<meta property="og:image" content="/images/cover.jpg">
<meta property="og:image:width" content="1200">
<meta property="og:image:height" content="630">
<link rel="icon" href="/favicon-32.png">
</head><body><h1>hello</h1></body></html>"#;

/// A tiny HTTP/1.1 server with a handful of canned pages. Counts requests
/// so tests can tell a cache hit from a fetch.
struct Site {
    origin: String,
    hits: Arc<AtomicUsize>,
}

impl Site {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { break };
                let counter = counter.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buffer = [0u8; 2048];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match socket.read(&mut buffer).await {
                            Ok(0) | Err(_) => return,
                            Ok(read) => request.extend_from_slice(&buffer[..read]),
                        }
                    }
                    counter.fetch_add(1, Ordering::SeqCst);
                    let head = String::from_utf8_lossy(&request);
                    let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    let response = respond(&path);
                    let _ = socket.write_all(&response).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        Self { origin, hits }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.origin)
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

fn http(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut response = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
    for (name, value) in headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str("\r\n");
    let mut bytes = response.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn respond(path: &str) -> Vec<u8> {
    const HTML: (&str, &str) = ("Content-Type", "text/html; charset=utf-8");
    match path {
        "/article" => http("200 OK", &[HTML], ARTICLE.as_bytes()),
        "/moved" => http("301 Moved Permanently", &[("Location", "/hop")], b""),
        "/hop" => http("302 Found", &[("Location", "/article")], b""),
        "/loop" => http("302 Found", &[("Location", "/loop")], b""),
        "/to-metadata" => http("302 Found", &[("Location", "http://169.254.169.254/latest/meta-data/")], b""),
        "/to-private" => http("302 Found", &[("Location", "http://10.0.0.1/admin")], b""),
        "/to-file" => http("302 Found", &[("Location", "file:///etc/passwd")], b""),
        "/empty" => http("200 OK", &[HTML], b"<html><body><p>nothing to see</p></body></html>"),
        "/missing" => http("404 Not Found", &[HTML], b"<title>Not found</title>"),
        "/sjis" => {
            let (body, _, _) = encoding_rs::SHIFT_JIS.encode("<html><head><title>渋谷で集合中!</title></head></html>");
            http("200 OK", &[("Content-Type", "text/html; charset=Shift_JIS")], &body)
        }
        "/huge" => {
            // The title is at the very start; megabytes of filler follow.
            let mut body = b"<title>Huge page</title>".to_vec();
            body.extend(std::iter::repeat_n(b'x', 3 * 1024 * 1024));
            http("200 OK", &[HTML], &body)
        }
        "/late-title" => {
            // Metadata beyond the 1 MB cap is never seen.
            let mut body = vec![b' '; 1024 * 1024 + 10];
            body.extend_from_slice(b"<title>Too late</title>");
            http("200 OK", &[HTML], &body)
        }
        "/photo.png" => http("200 OK", &[("Content-Type", "image/png")], &png(8, 8)),
        "/data.json" => http("200 OK", &[("Content-Type", "application/json")], b"{\"title\":\"not html\"}"),
        _ => http("404 Not Found", &[HTML], b""),
    }
}

async fn loopback_harness() -> Harness {
    start_with(MediaConfig { link_preview_allow_loopback: true, ..Default::default() }).await
}

async fn preview(h: &Harness, user: &tokuly_testkit::TestUser, url: &str) -> Option<LinkPreview> {
    let response: GetLinkPreviewResponse = h.rpc("GetLinkPreview", Some(user), &GetLinkPreviewRequest { url: url.to_owned(), ..Default::default() }).await.expect("GetLinkPreview");
    response.preview.into_option()
}

#[tokio::test]
async fn open_graph_metadata_is_fetched_and_cached() {
    let h = loopback_harness().await;
    let site = Site::start().await;
    let alice = h.app.user("alice").await;

    let url = site.url("/article");
    let found = preview(&h, &alice, &format!("  {url}#comments ")).await.expect("preview");
    assert_eq!(found.url, url, "normalised: trimmed, without fragment");
    assert_eq!(found.title, "今日のおでかけ & ごはん");
    assert_eq!(found.description, "友達と位置情報共有を楽しもう!");
    assert_eq!(found.site_name, "Example Times");
    assert_eq!(found.image_url, site.url("/images/cover.jpg"), "relative URLs are resolved");
    assert_eq!((found.image_width, found.image_height), (1200, 630));
    assert_eq!(found.favicon_url, site.url("/favicon-32.png"));
    assert_eq!((found.embed_provider.as_str(), found.embed_id.as_str()), ("", ""));
    assert_eq!(site.hits(), 1);

    // Asked again (by anyone), the answer comes from the cache.
    let bob = h.app.user("bob").await;
    assert_eq!(preview(&h, &bob, &url).await, Some(found.clone()));
    assert_eq!(preview(&h, &alice, &url).await, Some(found));
    assert_eq!(site.hits(), 1);

    // "Nothing to show" is remembered too.
    for path in ["/empty", "/missing", "/data.json"] {
        let before = site.hits();
        assert_eq!(preview(&h, &alice, &site.url(path)).await, None, "{path}");
        assert_eq!(preview(&h, &alice, &site.url(path)).await, None, "{path}");
        assert_eq!(site.hits(), before + 1, "{path} is fetched once");
    }
}

#[tokio::test]
async fn redirects_charsets_and_size_limits() {
    let h = loopback_harness().await;
    let site = Site::start().await;
    let alice = h.app.user("alice").await;

    // Two hops, then the article; the preview keeps the URL that was asked for.
    let moved = preview(&h, &alice, &site.url("/moved")).await.expect("followed redirects");
    assert_eq!(moved.url, site.url("/moved"));
    assert_eq!(moved.title, "今日のおでかけ & ごはん");
    assert_eq!(moved.image_url, site.url("/images/cover.jpg"), "resolved against the final page");
    assert_eq!(site.hits(), 3);

    // A redirect loop ends after the hop limit (first request + 3 redirects).
    let before = site.hits();
    assert_eq!(preview(&h, &alice, &site.url("/loop")).await, None);
    assert_eq!(site.hits(), before + 4);

    // Pages that are not UTF-8.
    assert_eq!(preview(&h, &alice, &site.url("/sjis")).await.expect("shift_jis page").title, "渋谷で集合中!");

    // Only the first megabyte is read.
    let huge = preview(&h, &alice, &site.url("/huge")).await.expect("title within the cap");
    assert_eq!(huge.title, "Huge page");
    assert_eq!(huge.favicon_url, site.url("/favicon.ico"), "the conventional icon location is assumed");
    assert_eq!(preview(&h, &alice, &site.url("/late-title")).await, None);

    // A link straight to an image previews as that image.
    let photo = preview(&h, &alice, &site.url("/photo.png")).await.expect("image preview");
    assert_eq!((photo.title.as_str(), photo.image_url.as_str()), ("photo.png", site.url("/photo.png").as_str()));
}

#[tokio::test]
async fn requests_that_lead_inward_are_refused() {
    let h = loopback_harness().await;
    let site = Site::start().await;
    let alice = h.app.user("alice").await;

    // Even with loopback allowed for the test, redirects into private
    // ranges, to the metadata service or to other schemes go nowhere.
    for path in ["/to-metadata", "/to-private", "/to-file"] {
        let before = site.hits();
        assert_eq!(preview(&h, &alice, &site.url(path)).await, None, "{path}");
        assert_eq!(site.hits(), before + 1, "only the redirecting page itself was requested");
    }
    for url in ["http://10.0.0.1/", "http://169.254.169.254/latest/meta-data/", "http://192.168.1.1:8080/", "http://[fd00::1]/", "http://[::ffff:10.0.0.1]/"] {
        assert_eq!(preview(&h, &alice, url).await, None, "{url}");
    }

    // The production policy (what chat and notes get) refuses loopback
    // outright: the local site is never contacted.
    let core = &h.app.core;
    let before = site.hits();
    for url in [site.url("/article"), "http://127.0.0.1/".to_owned(), "http://localhost/".to_owned(), "http://[::1]/".to_owned(), "http://0x7f000001/".to_owned()] {
        assert_eq!(tokuly_media::link_preview(core, &url).await.unwrap(), None, "{url}");
    }
    assert_eq!(site.hits(), before);
    assert_eq!(tokuly_media::link_preview(core, "ftp://example.com/x").await.unwrap_err().reason, "URL_INVALID");
}

#[tokio::test]
async fn own_pages_are_embeds_and_input_is_validated() {
    let h = loopback_harness().await;
    let alice = h.app.user("alice").await;

    // The test app's public URL is http://localhost:8080; its pages are
    // described without any request.
    let own = preview(&h, &alice, "http://localhost:8080/video/dQw4w9WgXcQ?t=42").await.expect("own page");
    assert_eq!((own.embed_provider.as_str(), own.embed_id.as_str(), own.site_name.as_str()), ("tokuly", "video/dQw4w9WgXcQ", "Tokuly"));
    assert_eq!(preview(&h, &alice, "http://localhost:8080/settings/privacy").await, None);

    let request = |url: &str| GetLinkPreviewRequest { url: url.to_owned(), ..Default::default() };
    for bad in ["", "not a url", "ftp://example.com/file", "javascript:alert(1)", "file:///etc/passwd"] {
        let error = h.rpc::<_, GetLinkPreviewResponse>("GetLinkPreview", Some(&alice), &request(bad)).await.unwrap_err();
        assert_eq!((error.code.as_str(), error.reason.as_str()), ("invalid_argument", "URL_INVALID"), "{bad}");
    }
    let anonymous = h.rpc::<_, GetLinkPreviewResponse>("GetLinkPreview", None, &request("https://example.com/")).await.unwrap_err();
    assert_eq!(anonymous.status, 401);
}

#[tokio::test]
async fn callers_are_rate_limited() {
    let h = loopback_harness().await;
    let alice = h.app.user("alice").await;
    let request = GetLinkPreviewRequest { url: "http://localhost:8080/@tokuzou".into(), ..Default::default() };
    for _ in 0..60 {
        let _: GetLinkPreviewResponse = h.rpc("GetLinkPreview", Some(&alice), &request).await.unwrap();
    }
    let error = h.rpc::<_, GetLinkPreviewResponse>("GetLinkPreview", Some(&alice), &request).await.unwrap_err();
    assert_eq!((error.code.as_str(), error.reason.as_str()), ("resource_exhausted", "RATE_LIMITED"));
    // Limits are per caller.
    let bob = h.app.user("bob").await;
    let _: GetLinkPreviewResponse = h.rpc("GetLinkPreview", Some(&bob), &request).await.unwrap();
}
