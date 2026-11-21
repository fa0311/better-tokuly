//! A local HTTP server that stands in for news sites: it serves feed
//! documents with validators (and answers conditional requests with 304),
//! and has endpoints that misbehave in the ways the fetcher must survive.
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};

#[derive(Clone)]
struct Document {
    body: Vec<u8>,
    etag: String,
    last_modified: String,
}

/// One request the server answered.
#[derive(Debug, Clone)]
pub struct Hit {
    pub path: String,
    pub if_none_match: Option<String>,
    pub if_modified_since: Option<String>,
    pub status: u16,
}

#[derive(Default)]
struct Shared {
    documents: Mutex<HashMap<String, Document>>,
    hits: Mutex<Vec<Hit>>,
    versions: Mutex<u64>,
}

pub struct FeedServer {
    pub addr: SocketAddr,
    shared: Arc<Shared>,
}

impl FeedServer {
    pub async fn start() -> Self {
        let shared = Arc::new(Shared::default());
        let router = Router::new()
            .route("/feeds/{name}", get(feed))
            .route("/redirect/to/{name}", get(redirect_to_feed))
            .route("/redirect/private", get(|| async { redirect("http://10.11.12.13/feed.xml") }))
            .route("/redirect/metadata", get(|| async { redirect("http://169.254.169.254/latest/meta-data/") }))
            .route("/redirect/loop", get(|| async { redirect("/redirect/loop") }))
            .route("/redirect/file", get(|| async { redirect("file:///etc/passwd") }))
            .route("/slow", get(slow))
            .route("/stream-big", get(stream_big))
            .route("/status/{code}", get(status))
            .with_state(shared.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind a local port");
        let addr = listener.local_addr().expect("local address");
        tokio::spawn(async move {
            axum::serve(listener, router).await.expect("feed server");
        });
        Self { addr, shared }
    }

    /// The URL of a path on this server.
    pub fn url(&self, path: &str) -> String {
        format!("http://{}/{}", self.addr, path.trim_start_matches('/'))
    }

    /// The URL under which the document `name` is served.
    pub fn feed_url(&self, name: &str) -> String {
        self.url(&format!("feeds/{name}"))
    }

    /// Publishes (or replaces) the document `name`. Every version gets new validators.
    pub fn publish(&self, name: &str, body: impl Into<Vec<u8>>) {
        let mut versions = self.shared.versions.lock().unwrap();
        *versions += 1;
        let document = Document { body: body.into(), etag: format!("\"v{}\"", *versions), last_modified: format!("Thu, 19 Nov 2026 10:00:{:02} GMT", *versions % 60) };
        self.shared.documents.lock().unwrap().insert(name.to_owned(), document);
    }

    /// Every request answered so far whose path ends with `suffix`.
    pub fn hits(&self, suffix: &str) -> Vec<Hit> {
        self.shared.hits.lock().unwrap().iter().filter(|hit| hit.path.ends_with(suffix)).cloned().collect()
    }
}

fn text(headers: &HeaderMap, name: header::HeaderName) -> Option<String> {
    headers.get(name).and_then(|value| value.to_str().ok()).map(str::to_owned)
}

fn redirect(location: &str) -> Response {
    (StatusCode::FOUND, [(header::LOCATION, location.to_owned())]).into_response()
}

async fn feed(State(shared): State<Arc<Shared>>, Path(name): Path<String>, headers: HeaderMap) -> Response {
    let document = shared.documents.lock().unwrap().get(&name).cloned();
    let (if_none_match, if_modified_since) = (text(&headers, header::IF_NONE_MATCH), text(&headers, header::IF_MODIFIED_SINCE));
    let response = match document {
        None => StatusCode::NOT_FOUND.into_response(),
        Some(document) if if_none_match.as_deref() == Some(document.etag.as_str()) || if_modified_since.as_deref() == Some(document.last_modified.as_str()) => {
            StatusCode::NOT_MODIFIED.into_response()
        }
        Some(document) => {
            (StatusCode::OK, [(header::CONTENT_TYPE, "application/xml".to_owned()), (header::ETAG, document.etag), (header::LAST_MODIFIED, document.last_modified)], document.body)
                .into_response()
        }
    };
    shared.hits.lock().unwrap().push(Hit { path: format!("/feeds/{name}"), if_none_match, if_modified_since, status: response.status().as_u16() });
    response
}

async fn redirect_to_feed(Path(name): Path<String>) -> Response {
    redirect(&format!("/feeds/{name}"))
}

async fn slow() -> Response {
    tokio::time::sleep(Duration::from_secs(20)).await;
    "too late".into_response()
}

/// A megabyte in chunks, without a Content-Length to give it away.
async fn stream_big() -> Response {
    let chunks = futures::stream::iter((0..256).map(|_| Ok::<_, std::io::Error>(vec![b'x'; 4096])));
    Response::builder().status(StatusCode::OK).header(header::CONTENT_TYPE, "application/xml").body(Body::from_stream(chunks)).expect("response")
}

async fn status(Path(code): Path<u16>) -> Response {
    StatusCode::from_u16(code).unwrap_or(StatusCode::IM_A_TEAPOT).into_response()
}

/// One item of a generated feed.
#[derive(Debug, Clone, Default)]
pub struct Item {
    pub title: String,
    pub link: String,
    /// HTML, escaped by the generator.
    pub description: String,
    pub categories: Vec<String>,
    /// RFC 3339; empty for an item without a date.
    pub published: String,
    pub image: String,
}

impl Item {
    pub fn new(title: &str, link: &str) -> Self {
        Self { title: title.to_owned(), link: link.to_owned(), ..Default::default() }
    }

    pub fn description(mut self, html: &str) -> Self {
        self.description = html.to_owned();
        self
    }

    pub fn categories(mut self, categories: &[&str]) -> Self {
        self.categories = categories.iter().map(|c| (*c).to_owned()).collect();
        self
    }

    pub fn published(mut self, at: chrono::DateTime<chrono::Utc>) -> Self {
        self.published = at.to_rfc3339();
        self
    }

    pub fn image(mut self, url: &str) -> Self {
        self.image = url.to_owned();
        self
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// An RSS 2.0 document.
pub fn rss(title: &str, site: &str, items: &[Item]) -> String {
    let items: String = items
        .iter()
        .map(|item| {
            let categories: String = item.categories.iter().map(|c| format!("<category>{}</category>", escape(c))).collect();
            let date = if item.published.is_empty() { String::new() } else { format!("<dc:date>{}</dc:date>", item.published) };
            let image = if item.image.is_empty() { String::new() } else { format!("<media:thumbnail url=\"{}\"/>", escape(&item.image)) };
            format!(
                "<item><title>{}</title><link>{}</link><description>{}</description>{categories}{date}{image}</item>",
                escape(&item.title),
                escape(&item.link),
                escape(&item.description)
            )
        })
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <rss version=\"2.0\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:media=\"http://search.yahoo.com/mrss/\">\
         <channel><title>{}</title><link>{}</link><description>test feed</description>{items}</channel></rss>",
        escape(title),
        escape(site)
    )
}

/// An Atom document.
pub fn atom(title: &str, site: &str, items: &[Item]) -> String {
    let entries: String = items
        .iter()
        .map(|item| {
            let categories: String = item.categories.iter().map(|c| format!("<category term=\"{}\"/>", escape(c))).collect();
            let date = if item.published.is_empty() { String::new() } else { format!("<published>{0}</published><updated>{0}</updated>", item.published) };
            let image = if item.image.is_empty() { String::new() } else { format!("<link rel=\"enclosure\" type=\"image/jpeg\" href=\"{}\"/>", escape(&item.image)) };
            format!(
                "<entry><title>{}</title><link rel=\"alternate\" href=\"{}\"/><id>{}</id><summary type=\"html\">{}</summary>{categories}{date}{image}</entry>",
                escape(&item.title),
                escape(&item.link),
                escape(&item.link),
                escape(&item.description)
            )
        })
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <feed xmlns=\"http://www.w3.org/2005/Atom\"><title>{}</title><link rel=\"alternate\" href=\"{}\"/>\
         <id>{}</id><updated>2026-11-20T00:00:00Z</updated>{entries}</feed>",
        escape(title),
        escape(site),
        escape(site)
    )
}
