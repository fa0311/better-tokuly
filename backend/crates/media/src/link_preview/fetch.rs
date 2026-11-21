//! Fetching a page for its preview metadata, within strict limits.
//!
//! One preview costs at most [`TOTAL_TIMEOUT`] of wall-clock time,
//! [`MAX_BODY_BYTES`] of downloaded (decompressed) body and
//! [`MAX_REDIRECTS`] redirect hops. Redirects are followed by hand so that
//! every hop passes the SSRF checks in [`super::guard`].
use std::{sync::Arc, time::Duration};

use bytes::BytesMut;
use reqwest::{StatusCode, header};
use url::Url;

use super::guard::{GuardedResolver, Policy, Refusal, check_url};

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
    /// At most [`MAX_BODY_BYTES`]; longer bodies are cut off.
    pub body: bytes::Bytes,
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
    reqwest::Client::builder()
        .user_agent(user_agent)
        .dns_resolver(Arc::new(GuardedResolver { policy }))
        // Hops are followed (and checked) by `fetch`.
        .redirect(reqwest::redirect::Policy::none())
        // A proxy would resolve host names for us, bypassing the resolver.
        .no_proxy()
        // Every preview goes to a different site; keeping connections around
        // buys nothing.
        .pool_max_idle_per_host(0)
        .connect_timeout(Duration::from_secs(3))
        .timeout(TOTAL_TIMEOUT)
        .build()
        .map_err(|e| anyhow::anyhow!("link preview http client: {e}"))
}

fn network_error(error: reqwest::Error) -> FetchError {
    if error.is_timeout() {
        return FetchError::Timeout;
    }
    // The guarded resolver's refusal travels inside the connect error.
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(&error);
    while let Some(cause) = source {
        if let Some(refusal) = cause.downcast_ref::<Refusal>() {
            return FetchError::Refused(refusal.clone());
        }
        source = cause.source();
    }
    FetchError::Network(format!("{error:#}"))
}

/// GETs `url`, following up to [`MAX_REDIRECTS`] checked redirects.
pub async fn fetch(client: &reqwest::Client, policy: Policy, url: Url, accept: &str) -> Result<Page, FetchError> {
    match tokio::time::timeout(TOTAL_TIMEOUT, fetch_unbounded(client, policy, url, accept)).await {
        Ok(result) => result,
        Err(_) => Err(FetchError::Timeout),
    }
}

async fn fetch_unbounded(client: &reqwest::Client, policy: Policy, mut url: Url, accept: &str) -> Result<Page, FetchError> {
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
        if let Some(peer) = response.remote_addr()
            && !policy.permits(peer.ip())
        {
            return Err(FetchError::Refused(Refusal::Address(peer.ip())));
        }

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
        let mut body = BytesMut::new();
        while body.len() < MAX_BODY_BYTES {
            let Some(chunk) = response.chunk().await.map_err(network_error)? else { break };
            let room = MAX_BODY_BYTES - body.len();
            body.extend_from_slice(&chunk[..chunk.len().min(room)]);
        }
        return Ok(Page { url, content_type, body: body.freeze() });
    }
    Err(FetchError::TooManyRedirects)
}
