//! The API as the simulator calls it: Connect RPC with JSON bodies, the way
//! any client of the public API does. There is no other door: every rule,
//! audience and rate limit of the server applies to the bots.
use std::{fmt, time::Duration};

use serde::{Serialize, de::DeserializeOwned};

/// A refused or failed call.
#[derive(Debug, Clone)]
pub struct ApiError {
    /// Connect code, e.g. `not_found`; `unavailable` when the server was not reached.
    pub code: String,
    /// The server's machine-readable reason (`x-tokuly-error`), e.g. `ALREADY_FRIENDS`.
    pub reason: String,
    pub message: String,
}

impl ApiError {
    fn unreachable(error: impl fmt::Display) -> Self {
        Self { code: "unavailable".into(), reason: "UNREACHABLE".into(), message: error.to_string() }
    }

    pub fn is(&self, reason: &str) -> bool {
        self.reason == reason
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} [{}] {}", self.code, self.reason, self.message)
    }
}

impl std::error::Error for ApiError {}

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Clone)]
pub struct Api {
    http: reqwest::Client,
    base: String,
    /// See `DemoConfig::storage_url`.
    storage: Option<String>,
}

impl Api {
    pub fn new(base_url: &str, storage_url: Option<&str>) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder().timeout(Duration::from_secs(30)).user_agent("tokuly-demo").build()?;
        let trimmed = |url: &str| url.trim_end_matches('/').to_owned();
        Ok(Self { http, base: trimmed(base_url), storage: storage_url.map(trimmed).filter(|url| !url.is_empty()) })
    }

    /// Calls a unary RPC, e.g. `"tokuly.geo.v1.LocationService/ReportLocations"`,
    /// with a session's access token or an API token (or anonymously).
    pub async fn call<Request: Serialize, Response: DeserializeOwned>(
        &self,
        token: Option<&str>,
        procedure: &str,
        request: &Request,
    ) -> ApiResult<Response> {
        let mut call =
            self.http.post(format!("{}/{procedure}", self.base)).header("connect-protocol-version", "1").json(request);
        if let Some(token) = token {
            call = call.bearer_auth(token);
        }
        let response = call.send().await.map_err(ApiError::unreachable)?;
        let status = response.status();
        let reason = response
            .headers()
            .get("x-tokuly-error")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let body = response.bytes().await.map_err(ApiError::unreachable)?;
        if !status.is_success() {
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
            let text = |key: &str| json.get(key).and_then(|value| value.as_str()).map(str::to_owned);
            return Err(ApiError {
                code: text("code").unwrap_or_else(|| status.as_u16().to_string()),
                reason,
                message: text("message").unwrap_or_default(),
            });
        }
        serde_json::from_slice(&body).map_err(|error| ApiError {
            code: "internal".into(),
            reason: "UNREADABLE_RESPONSE".into(),
            message: format!("{procedure}: {error}"),
        })
    }

    /// Sends the bytes of an upload to the signed URL the media service gave.
    pub async fn put<'a>(
        &self,
        url: &str,
        headers: impl IntoIterator<Item = (&'a String, &'a String)>,
        bytes: Vec<u8>,
    ) -> ApiResult<()> {
        let mut put = match &self.storage {
            None => self.http.put(url),
            // The signature covers the public host name, so that stays in the
            // Host header; only the connection goes somewhere else.
            Some(storage) => {
                let parsed = reqwest::Url::parse(url).map_err(ApiError::unreachable)?;
                let authority = match (parsed.host_str(), parsed.port()) {
                    (Some(host), Some(port)) => format!("{host}:{port}"),
                    (Some(host), None) => host.to_owned(),
                    (None, _) => return Err(ApiError::unreachable("the upload URL has no host")),
                };
                let query = parsed.query().map(|query| format!("?{query}")).unwrap_or_default();
                self.http.put(format!("{storage}{}{query}", parsed.path())).header(reqwest::header::HOST, authority)
            }
        };
        for (name, value) in headers {
            put = put.header(name, value);
        }
        let response = put.body(bytes).send().await.map_err(ApiError::unreachable)?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(ApiError {
                code: response.status().as_u16().to_string(),
                reason: "UPLOAD_REFUSED".into(),
                message: response.text().await.unwrap_or_default(),
            })
        }
    }
}
