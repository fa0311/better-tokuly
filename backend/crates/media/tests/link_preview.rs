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
use tokuly_media::{
    MediaConfig,
    link_preview::image_proxy::{Kind, proxy_url},
};
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
                    if path.starts_with("/slow") {
                        // Holds a download slot of the proxy for a while.
                        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                    } else if path.starts_with("/brief") {
                        // A picture of an ordinary site.
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    }
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
        "/images/cover.jpg" => http("200 OK", &[("Content-Type", "image/png")], &png(1600, 800)),
        "/favicon-32.png" => http("200 OK", &[("Content-Type", "image/png")], &png(32, 32)),
        "/vector.svg" => http("200 OK", &[("Content-Type", "image/svg+xml")], br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#),
        "/fake.png" => http("200 OK", &[("Content-Type", "image/png")], b"<html><script>alert(1)</script></html>"),
        "/page.png" => http("200 OK", &[HTML], &png(8, 8)),
        "/heavy.png" => {
            // A valid PNG header, then more than the 8 MB a picture may weigh.
            let mut body = png(8, 8);
            body.extend(std::iter::repeat_n(0u8, 9 * 1024 * 1024));
            http("200 OK", &[("Content-Type", "image/png")], &body)
        }
        path if path.starts_with("/slow") || path.starts_with("/brief") => http("200 OK", &[("Content-Type", "image/png")], &png(16, 16)),
        "/own-media" => http(
            "200 OK",
            &[HTML],
            br#"<title>Reposted</title>
<meta property="og:image" content="http://localhost:8080/media/img/0195f0aa-0000-7000-8000-000000000001/1024-0123456789abcdef.webp">
<link rel="icon" href="http://localhost:8080/api/tokuly.identity.v1.AccountService/GetMe">"#,
        ),
        "/to-private.png" => http("302 Found", &[("Location", "http://10.0.0.1/cat.png")], b""),
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
    let core = &h.app.core;
    assert_eq!(found.image_url, proxy_url(core, Kind::Image, &site.url("/images/cover.jpg")), "relative URLs are resolved, and served by us");
    assert!(found.image_url.starts_with("http://localhost:8080/api/media/link-image/image/"), "{}", found.image_url);
    assert!(!found.image_url.contains("127.0.0.1"), "the other site's address is not handed out");
    assert_eq!((found.image_width, found.image_height), (1200, 630));
    assert_eq!(found.favicon_url, proxy_url(core, Kind::Icon, &site.url("/favicon-32.png")));
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
    assert_eq!(moved.image_url, proxy_url(&h.app.core, Kind::Image, &site.url("/images/cover.jpg")), "resolved against the final page");
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
    assert_eq!(huge.favicon_url, proxy_url(&h.app.core, Kind::Icon, &site.url("/favicon.ico")), "the conventional icon location is assumed");
    assert_eq!(preview(&h, &alice, &site.url("/late-title")).await, None);

    // A link straight to an image previews as that image.
    let photo = preview(&h, &alice, &site.url("/photo.png")).await.expect("image preview");
    assert_eq!((photo.title.as_str(), photo.image_url.as_str()), ("photo.png", proxy_url(&h.app.core, Kind::Image, &site.url("/photo.png")).as_str()));
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

/// Serves the media domain's HTTP routes on a local port, the way the API
/// does (without the auth layer, so without a client address).
async fn serve_routes(h: &Harness) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let router = h.media.http_routes();
    tokio::spawn(async move { axum::serve(listener, router).await });
    origin
}

/// The proxy URL, pointed at the local server instead of the public URL.
fn local(origin: &str, proxied: &str) -> String {
    let at = proxied.find("/media/link-image/").expect("a proxy URL");
    format!("{origin}{}", &proxied[at..])
}

#[tokio::test]
async fn preview_pictures_are_served_by_this_instance() {
    let h = loopback_harness().await;
    let site = Site::start().await;
    let origin = serve_routes(&h).await;
    let alice = h.app.user("alice").await;
    let core = &h.app.core;

    let found = preview(&h, &alice, &site.url("/article")).await.expect("preview");
    let hits = site.hits();
    let response = h.http.get(local(&origin, &found.image_url)).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "image/webp");
    assert_eq!(response.headers()["cache-control"], "public, max-age=86400");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    let picture = image::load_from_memory(&response.bytes().await.unwrap()).expect("a WebP");
    assert_eq!((picture.width(), picture.height()), (1200, 600), "re-encoded at a bounded size");
    assert_eq!(site.hits(), hits + 1);

    // Seen again (by anyone): from our cache, the other site is not asked.
    let again = h.http.get(local(&origin, &found.image_url)).send().await.unwrap();
    assert_eq!(again.status(), 200);
    assert_eq!(site.hits(), hits + 1);

    // Kept in the object store, not in Valkey, and expired by the janitor.
    let cache_key = format!("image/{}", sha256_hex(site.url("/images/cover.jpg").as_bytes()));
    let object = format!("link-images/{cache_key}.webp");
    assert!(core.storage.head(tokuly_core::storage::Bucket::Private, &object).await.unwrap().is_some());
    sqlx::query("UPDATE link_preview_images SET fetched_at = now() - interval '2 days' WHERE cache_key = $1")
        .bind(&cache_key)
        .execute(&core.db)
        .await
        .unwrap();
    assert!(h.media.sweep().await.unwrap().expired_link_images >= 1);
    assert!(core.storage.head(tokuly_core::storage::Bucket::Private, &object).await.unwrap().is_none());
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM link_preview_images WHERE cache_key = $1").bind(&cache_key).fetch_one(&core.db).await.unwrap();
    assert_eq!(rows, 0);
    // Asked for again, it is fetched again.
    assert_eq!(h.http.get(local(&origin, &found.image_url)).send().await.unwrap().status(), 200);
    assert_eq!(site.hits(), hits + 2);

    let icon = h.http.get(local(&origin, &found.favicon_url)).send().await.unwrap();
    assert_eq!(icon.status(), 200);
    let icon = image::load_from_memory(&icon.bytes().await.unwrap()).unwrap();
    assert_eq!((icon.width(), icon.height()), (32, 32), "small icons are not enlarged");

    // Only URLs we signed are served: not a changed signature, not another
    // kind under the same signature, not another target.
    let hits = site.hits();
    let image_path = local(&origin, &found.image_url);
    let tampered = format!("{}x", &image_path[..image_path.len() - 1]);
    let other_kind = image_path.replace("/link-image/image/", "/link-image/icon/");
    let signature = image_path.rsplit('/').nth(1).unwrap();
    let other_target = format!("{origin}/media/link-image/image/{signature}/{}", tokuly_core::keys::b64url(site.url("/photo.png").as_bytes()));
    for url in [tampered, other_kind, other_target, format!("{origin}/media/link-image/image/x/y"), format!("{origin}/media/link-image/file/x/y")] {
        assert_eq!(h.http.get(&url).send().await.unwrap().status(), 404, "{url}");
    }
    assert_eq!(site.hits(), hits, "nothing was fetched for them");

    // What is not a raster image, too heavy, or leads inward is not served;
    // the failure is remembered too.
    for path in ["/vector.svg", "/fake.png", "/page.png", "/heavy.png", "/to-private.png", "/missing"] {
        let url = local(&origin, &proxy_url(core, Kind::Image, &site.url(path)));
        let hits = site.hits();
        assert_eq!(h.http.get(&url).send().await.unwrap().status(), 404, "{path}");
        assert_eq!(h.http.get(&url).send().await.unwrap().status(), 404, "{path}");
        assert_eq!(site.hits(), hits + 1, "{path} is fetched once");
    }

    // Pictures on addresses the egress guard refuses are never requested.
    let refused = local(&origin, &proxy_url(core, Kind::Image, "http://169.254.169.254/latest/meta-data/x.png"));
    assert_eq!(h.http.get(&refused).send().await.unwrap().status(), 404);
}

#[tokio::test]
async fn this_instances_own_media_is_never_copied() {
    let h = loopback_harness().await;
    let site = Site::start().await;
    let alice = h.app.user("alice").await;
    let core = &h.app.core;

    // A page whose og:image is one of our own public files: the preview
    // points at the file itself, so a takedown or deletion ends it at once
    // (a proxied copy would live on for a day). Other URLs of this instance
    // are dropped.
    let found = preview(&h, &alice, &site.url("/own-media")).await.expect("preview");
    let own = "http://localhost:8080/media/img/0195f0aa-0000-7000-8000-000000000001/1024-0123456789abcdef.webp";
    assert_eq!(found.image_url, own);
    assert_eq!(found.favicon_url, "");
    assert_eq!(proxy_url(core, Kind::Icon, "http://localhost:8080/api/x.png"), "");
    assert_eq!(proxy_url(core, Kind::Image, "http://localhost:8080/media/../api/x.png"), "", "normalised before it is judged");

    // A URL signed for an own file anyway (another media URL configured
    // back then) is refused without a request.
    let signature = tokuly_core::keys::b64url(&core.keys.mac("media/link-image", format!("image\n{own}").as_bytes())[..18]);
    let origin = serve_routes(&h).await;
    let forged = format!("{origin}/media/link-image/image/{signature}/{}", tokuly_core::keys::b64url(own.as_bytes()));
    assert_eq!(h.http.get(&forged).send().await.unwrap().status(), 404);
}

/// The media routes behind the auth layer, as the API serves them: requests
/// come from a client address.
struct Proxy {
    router: axum::Router,
    origin: String,
}

impl Proxy {
    fn new(h: &Harness) -> Self {
        let core = &h.app.core;
        let router = h
            .media
            .http_routes()
            .layer(axum::middleware::from_fn_with_state(core.auth_layer.clone(), tokuly_core::auth::auth_middleware));
        Self { router, origin: core.cfg.public_url().to_owned() }
    }

    /// The path of the proxy URL of `target`.
    fn path(&self, core: &tokuly_core::Core, target: &str) -> String {
        let proxied = proxy_url(core, Kind::Image, target);
        assert!(proxied.starts_with(&self.origin));
        proxied[proxied.find("/media/link-image/").unwrap()..].to_owned()
    }

    /// Status and `Cache-Control` of a request from `ip`.
    fn get(&self, path: String, ip: std::net::IpAddr) -> impl Future<Output = (u16, Option<String>)> + Send + 'static {
        use tower::ServiceExt as _;
        let router = self.router.clone();
        async move {
            let request = axum::http::Request::get(path)
                .extension(axum::extract::ConnectInfo(std::net::SocketAddr::new(ip, 4000)))
                .body(axum::body::Body::empty())
                .unwrap();
            let response = router.oneshot(request).await.unwrap();
            (
                response.status().as_u16(),
                response.headers().get("cache-control").map(|v| v.to_str().unwrap().to_owned()),
            )
        }
    }
}

/// An address of a fresh /64 of the documentation range: its own client.
fn client_network() -> impl Fn(u16) -> std::net::IpAddr {
    let net = rand_u16();
    move |host| std::net::IpAddr::from([0x2001, 0xdb8, net, 7, 0, 0, 0, host])
}

#[tokio::test]
async fn a_list_of_new_pictures_loads_at_once() {
    let h = loopback_harness().await;
    let site = Site::start().await;
    let proxy = Proxy::new(&h);
    let client = client_network()(1);

    // A news list shows a dozen pictures nobody has asked for yet: more than
    // there are download slots, all requested together. Each waits its turn.
    let started = std::time::Instant::now();
    let answers = futures::future::join_all(
        (0..12).map(|n| proxy.get(proxy.path(&h.app.core, &site.url(&format!("/brief-{n}.png"))), client)),
    )
    .await;
    for (n, (status, _)) in answers.iter().enumerate() {
        assert_eq!(*status, 200, "picture {n}");
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
}

#[tokio::test]
async fn one_client_cannot_park_requests_beyond_its_share() {
    let h = loopback_harness().await;
    let site = Site::start().await;
    let proxy = Proxy::new(&h);
    let core = &h.app.core;
    let address = client_network();
    let picture = |name: String| proxy.path(core, &site.url(&name));

    // Slow downloads take every slot; sixteen more misses of the same client
    // wait for one.
    let holding: Vec<_> =
        (0..4).map(|n| tokio::spawn(proxy.get(picture(format!("/slow-{n}.png")), address(1)))).collect();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let waiting: Vec<_> =
        (0..16).map(|n| tokio::spawn(proxy.get(picture(format!("/brief-{n}.png")), address(n + 2)))).collect();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // One more of that client is turned away at once, and not remembered.
    let started = std::time::Instant::now();
    assert_eq!(proxy.get(picture("/brief-extra.png".into()), address(99)).await, (503, Some("no-store".to_owned())));
    assert!(started.elapsed() < std::time::Duration::from_millis(500), "refused without waiting");
    // Another client still gets a place in the queue.
    let elsewhere = std::net::IpAddr::from([0x2001, 0xdb8, rand_u16(), 8, 0, 0, 0, 1]);
    let other = tokio::spawn(proxy.get(picture("/brief-other.png".into()), elsewhere));

    for task in holding.into_iter().chain(waiting) {
        assert_eq!(task.await.unwrap().0, 200);
    }
    assert_eq!(other.await.unwrap().0, 200);
    assert_eq!(proxy.get(picture("/brief-extra.png".into()), address(99)).await.0, 200);
}

#[tokio::test]
async fn misses_are_limited_per_client_through_the_auth_layer() {
    let h = loopback_harness().await;
    let site = Site::start().await;
    let proxy = Proxy::new(&h);
    let core = &h.app.core;
    let net: u16 = rand_u16();
    let address = |host: u16| std::net::IpAddr::from([0x2001, 0xdb8, net, 7, 0, 0, 0, host]);
    let path = |name: &str| proxy.path(core, &site.url(name));

    // One picture fetched while the client is within its limit.
    assert_eq!(proxy.get(path("/photo.png"), address(1)).await.0, 200);
    // Use up the rest of the minute's 120 misses of this /64.
    for _ in 0..119 {
        let _ = core
            .cache
            .rate_limit(
                &format!("media:link-image-client:2001:db8:{net:x}:7::/64"),
                120,
                std::time::Duration::from_secs(60),
            )
            .await;
    }
    let hits = site.hits();
    // Another address of the same /64 is the same client.
    assert_eq!(proxy.get(path("/favicon-32.png"), address(2)).await, (503, Some("no-store".to_owned())));
    assert_eq!(site.hits(), hits, "nothing was fetched");
    // What is cached is still served; other networks are not affected.
    assert_eq!(proxy.get(path("/photo.png"), address(3)).await.0, 200);
    let elsewhere = std::net::IpAddr::from([0x2001, 0xdb8, net, 8, 0, 0, 0, 1]);
    assert_eq!(proxy.get(path("/favicon-32.png"), elsewhere).await.0, 200);
}

fn rand_u16() -> u16 {
    use std::hash::{BuildHasher as _, Hasher as _};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
    hasher.finish() as u16
}
