//! Downloading a feed document, within strict limits.
//!
//! One fetch costs at most `limits.timeout` of wall-clock time (redirects
//! included), `limits.max_bytes` of decompressed body and [`MAX_REDIRECTS`]
//! redirect hops. Redirects are followed by hand so that every hop passes the
//! SSRF checks of `tokuly_core::egress`. Requests are conditional: the validators
//! of the last successful response are sent back, and a `304 Not Modified`
//! ends the fetch without a body.
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use reqwest::{StatusCode, header};
use tokuly_core::egress::{self, Policy, Refusal, check_url};
use url::Url;

pub const MAX_REDIRECTS: usize = 5;
/// Longest validator (`ETag` / `Last-Modified`) kept and sent back.
const MAX_VALIDATOR_LEN: usize = 512;

const ACCEPT: &str = "application/rss+xml, application/atom+xml, application/feed+json, application/xml;q=0.9, text/xml;q=0.8, */*;q=0.5";

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub timeout: Duration,
    pub max_bytes: usize,
}

/// Validators of the last successful response, for a conditional request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Validators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

/// A downloaded feed document.
#[derive(Debug)]
pub struct Document {
    /// Where the document finally came from (after redirects).
    pub url: Url,
    pub body: Bytes,
    /// Validators to send with the next request.
    pub validators: Validators,
}

#[derive(Debug)]
pub enum Fetched {
    /// The server says nothing changed since the validators were issued.
    NotModified,
    Document(Document),
}

#[derive(Debug)]
pub enum FetchError {
    /// The URL, a redirect target or a resolved address is off limits.
    Refused(Refusal),
    /// The server answered, but not with a document.
    Status(StatusCode),
    TooManyRedirects,
    /// The document is larger than the limit (in bytes).
    TooLarge(usize),
    Timeout,
    /// Connection, TLS or protocol failure (also: a host name that resolves
    /// to nothing we may reach).
    Network(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(refusal) => write!(f, "refused: {refusal}"),
            Self::Status(status) => write!(f, "HTTP {status}"),
            Self::TooManyRedirects => write!(f, "more than {MAX_REDIRECTS} redirects"),
            Self::TooLarge(limit) => write!(f, "the feed is larger than {limit} bytes"),
            Self::Timeout => write!(f, "the server took too long to answer"),
            Self::Network(detail) => write!(f, "network: {detail}"),
        }
    }
}

/// An HTTP client that can only talk to what `policy` permits.
pub fn guarded_client(policy: Policy, timeout: Duration) -> anyhow::Result<reqwest::Client> {
    // Hops are followed (and checked) by `fetch`.
    egress::client_builder(Some(policy), timeout)
        .user_agent("Tokuly-News/1.0 (feed reader)")
        .build()
        .map_err(|e| anyhow::anyhow!("news http client: {e}"))
}

fn network_error(error: reqwest::Error) -> FetchError {
    if error.is_timeout() {
        return FetchError::Timeout;
    }
    match egress::refusal_of(&error) {
        Some(refusal) => FetchError::Refused(refusal),
        None => FetchError::Network(format!("{error:#}")),
    }
}

/// A response header as a validator worth keeping.
fn validator(headers: &header::HeaderMap, name: header::HeaderName) -> Option<String> {
    let value = headers.get(name)?.to_str().ok()?.trim();
    (!value.is_empty() && value.len() <= MAX_VALIDATOR_LEN).then(|| value.to_owned())
}

/// GETs `url`, following up to [`MAX_REDIRECTS`] checked redirects.
pub async fn fetch(
    client: &reqwest::Client,
    policy: Policy,
    limits: Limits,
    url: Url,
    validators: &Validators,
) -> Result<Fetched, FetchError> {
    match tokio::time::timeout(limits.timeout, fetch_unbounded(client, policy, limits, url, validators)).await {
        Ok(result) => result,
        Err(_) => Err(FetchError::Timeout),
    }
}

async fn fetch_unbounded(
    client: &reqwest::Client,
    policy: Policy,
    limits: Limits,
    mut url: Url,
    validators: &Validators,
) -> Result<Fetched, FetchError> {
    for _hop in 0..=MAX_REDIRECTS {
        check_url(&url, policy).map_err(FetchError::Refused)?;
        let mut request = client.get(url.clone()).header(header::ACCEPT, ACCEPT);
        if let Some(etag) = &validators.etag {
            request = request.header(header::IF_NONE_MATCH, etag);
        }
        if let Some(last_modified) = &validators.last_modified {
            request = request.header(header::IF_MODIFIED_SINCE, last_modified);
        }
        let mut response = request.send().await.map_err(network_error)?;
        // Belt and braces: whatever we ended up connected to must be permitted.
        egress::check_peer(&response, policy).map_err(FetchError::Refused)?;

        let status = response.status();
        if status == StatusCode::NOT_MODIFIED {
            return Ok(Fetched::NotModified);
        }
        if status.is_redirection() {
            let location = response.headers().get(header::LOCATION).and_then(|value| value.to_str().ok());
            let Some(next) = location.and_then(|location| url.join(location).ok()) else {
                return Err(FetchError::Status(status));
            };
            url = next;
            continue;
        }
        if !status.is_success() {
            return Err(FetchError::Status(status));
        }

        // An honest Content-Length saves downloading what would be thrown away.
        if response.content_length().is_some_and(|length| length > limits.max_bytes as u64) {
            return Err(FetchError::TooLarge(limits.max_bytes));
        }
        let validators = Validators {
            etag: validator(response.headers(), header::ETAG),
            last_modified: validator(response.headers(), header::LAST_MODIFIED),
        };
        let mut body = BytesMut::new();
        while let Some(chunk) = response.chunk().await.map_err(network_error)? {
            // A truncated feed would parse as garbage: over the limit is an error.
            if body.len() + chunk.len() > limits.max_bytes {
                return Err(FetchError::TooLarge(limits.max_bytes));
            }
            body.extend_from_slice(&chunk);
        }
        return Ok(Fetched::Document(Document { url, body: body.freeze(), validators }));
    }
    Err(FetchError::TooManyRedirects)
}
