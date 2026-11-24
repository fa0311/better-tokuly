//! Fetching a page for its preview metadata, within strict limits.
//!
//! One preview costs at most [`TOTAL_TIMEOUT`] of wall-clock time,
//! [`MAX_BODY_BYTES`] of downloaded (decompressed) body and
//! [`MAX_REDIRECTS`] redirect hops. Redirects are followed by hand so that
//! every hop passes the SSRF checks of `tokuly_core::egress`.
use std::time::Duration;

use reqwest::{StatusCode, header};
use tokuly_core::egress::{self, Policy, Refusal, check_url};
use url::Url;

pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
pub const MAX_REDIRECTS: usize = 3;

/// A fetched document.
#[derive(Debug)]
pub struct Page {
    /// Where the content finally came from (after redirects).
    pub url: Url,
    /// The `Content-Type` header, lowercased.
    pub content_type: String,
    /// At most the limit of the fetch ([`MAX_BODY_BYTES`] for pages);
    /// longer bodies are cut off.
    pub body: bytes::Bytes,
    /// Whether the body was longer than the limit (and was cut off).
    pub truncated: bool,
}

#[derive(Debug)]
pub enum FetchError {
    /// The URL, a redirect target or a resolved address is off limits.
    Refused(Refusal),
    /// The server answered, but not with a usable document.
    Status(StatusCode),
    TooManyRedirects,
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
            Self::Timeout => write!(f, "no answer within {TOTAL_TIMEOUT:?}"),
            Self::Network(detail) => write!(f, "network: {detail}"),
        }
    }
}

/// An HTTP client that can only talk to what `policy` permits.
pub fn guarded_client(policy: Policy, user_agent: &str) -> anyhow::Result<reqwest::Client> {
    // Hops are followed (and checked) by `fetch`.
    egress::client_builder(Some(policy), TOTAL_TIMEOUT)
        .user_agent(user_agent)
        // Every preview goes to a different site; keeping connections around
        // buys nothing.
        .pool_max_idle_per_host(0)
        .connect_timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| anyhow::anyhow!("link preview http client: {e}"))
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

/// GETs `url`, following up to [`MAX_REDIRECTS`] checked redirects, and
/// reads at most [`MAX_BODY_BYTES`] of the answer.
pub async fn fetch(client: &reqwest::Client, policy: Policy, url: Url, accept: &str) -> Result<Page, FetchError> {
    fetch_limited(client, policy, url, accept, MAX_BODY_BYTES).await
}

/// [`fetch`] with another body limit: at most `max_bytes` are kept, and
/// [`Page::truncated`] says whether there was more.
pub async fn fetch_limited(client: &reqwest::Client, policy: Policy, url: Url, accept: &str, max_bytes: usize) -> Result<Page, FetchError> {
    match tokio::time::timeout(TOTAL_TIMEOUT, fetch_unbounded(client, policy, url, accept, max_bytes)).await {
        Ok(result) => result,
        Err(_) => Err(FetchError::Timeout),
    }
}

async fn fetch_unbounded(client: &reqwest::Client, policy: Policy, mut url: Url, accept: &str, max_bytes: usize) -> Result<Page, FetchError> {
    for _hop in 0..=MAX_REDIRECTS {
        check_url(&url, policy).map_err(FetchError::Refused)?;
        let mut response = client
            .get(url.clone())
            .header(header::ACCEPT, accept)
            .header(header::ACCEPT_LANGUAGE, "ja,en;q=0.8")
            .send()
            .await
            .map_err(network_error)?;
        // Belt and braces: whatever we ended up connected to must be permitted.
        egress::check_peer(&response, policy).map_err(FetchError::Refused)?;

        let status = response.status();
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

        let content_type =
            response.headers().get(header::CONTENT_TYPE).and_then(|value| value.to_str().ok()).unwrap_or_default().to_ascii_lowercase();
        // One byte more than the limit tells a body of exactly the limit
        // from a longer one.
        let mut body = egress::read_capped(&mut response, max_bytes.saturating_add(1)).await.map_err(network_error)?;
        let truncated = body.len() > max_bytes;
        body.truncate(max_bytes);
        return Ok(Page { url, content_type, body, truncated });
    }
    Err(FetchError::TooManyRedirects)
}
