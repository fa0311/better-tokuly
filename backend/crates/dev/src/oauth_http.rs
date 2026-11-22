//! The plain HTTP endpoints of the developer platform: what OAuth
//! libraries, CLIs and code generators call without speaking Connect.
//!
//! - `POST /oauth/token` — RFC 6749 section 3.2 (form in, JSON out)
//! - `POST /oauth/revoke` — RFC 7009
//! - `POST /oauth/device_authorization` — RFC 8628
//! - `GET /.well-known/oauth-authorization-server` — RFC 8414
//! - `GET /reference/descriptor.binpb` — the API's descriptor set
use std::{collections::HashMap, time::Duration};

use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::Engine as _;
use tokuly_core::auth::ClientMeta;

use crate::{
    Dev,
    oauth::{ClientCredentials, GRANT_DEVICE_CODE, OAuthError, OAuthResult},
    scopes,
};

/// Requests one address may make to the OAuth endpoints per minute.
const REQUESTS_PER_MINUTE: u32 = 120;
/// Device flows one address may start per minute: each parks a code that
/// waits a quarter of an hour for a person.
const DEVICE_STARTS_PER_MINUTE: u32 = 20;
const MAX_FORM_BYTES: usize = 16 * 1024;

pub(crate) fn routes(dev: Dev) -> Router {
    Router::new()
        .route("/oauth/token", post(token))
        .route("/oauth/revoke", post(revoke))
        .route("/oauth/device_authorization", post(device_authorization))
        .route("/.well-known/oauth-authorization-server", get(metadata))
        .route("/reference/descriptor.binpb", get(descriptor))
        .with_state(dev)
}

/// `Authorization: Basic` credentials; both halves are form-urlencoded
/// (RFC 6749 section 2.3.1).
fn basic_credentials(headers: &HeaderMap) -> OAuthResult<Option<(String, String)>> {
    let Some(value) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) else { return Ok(None) };
    let Some(encoded) = value.strip_prefix("Basic ").or_else(|| value.strip_prefix("basic ")) else { return Ok(None) };
    let malformed = || OAuthError::new("invalid_client", "malformed Basic credentials");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or_else(malformed)?;
    let (id, secret) = decoded.split_once(':').ok_or_else(malformed)?;
    let decode = |part: &str| {
        url::form_urlencoded::parse(part.as_bytes()).next().map(|(key, _)| key.into_owned()).unwrap_or_default()
    };
    Ok(Some((decode(id), decode(secret))))
}

/// A parsed OAuth request: the form and the client's credentials.
struct OAuthRequest {
    form: HashMap<String, String>,
    credentials: ClientCredentials,
    used_basic: bool,
}

impl OAuthRequest {
    async fn read(dev: &Dev, client: &ClientMeta, headers: &HeaderMap, body: &Bytes) -> OAuthResult<Self> {
        dev.core
            .cache
            .rate_limit(&format!("dev-oauth:{}", client.ip), REQUESTS_PER_MINUTE, Duration::from_secs(60))
            .await?;
        let content_type = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or_default();
        if !content_type.to_ascii_lowercase().starts_with("application/x-www-form-urlencoded")
            || body.len() > MAX_FORM_BYTES
        {
            return Err(OAuthError::new("invalid_request", "the body must be application/x-www-form-urlencoded"));
        }
        let mut form = HashMap::new();
        for (key, value) in url::form_urlencoded::parse(body) {
            // RFC 6749 section 3.2: parameters must not be repeated.
            if form.insert(key.into_owned(), value.into_owned()).is_some() {
                return Err(OAuthError::new("invalid_request", "a parameter is repeated"));
            }
        }
        let basic = basic_credentials(headers)?;
        let used_basic = basic.is_some();
        let credentials = ClientCredentials {
            basic,
            body_id: form.get("client_id").filter(|v| !v.is_empty()).cloned(),
            body_secret: form.get("client_secret").filter(|v| !v.is_empty()).cloned(),
        };
        Ok(Self { form, credentials, used_basic })
    }
}

fn no_store(mut response: Response) -> Response {
    // RFC 6749 section 5.1: token responses are never cached.
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

fn error_response(error: &OAuthError, used_basic: bool) -> Response {
    let status = StatusCode::from_u16(error.status).unwrap_or(StatusCode::BAD_REQUEST);
    let mut response =
        (status, Json(serde_json::json!({ "error": error.error, "error_description": error.description })))
            .into_response();
    if status == StatusCode::UNAUTHORIZED && used_basic {
        // RFC 6749 section 5.2: answer the scheme the client used.
        response.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static(r#"Basic realm="tokuly""#));
    }
    no_store(response)
}

fn respond<T: serde::Serialize>(result: OAuthResult<T>, used_basic: bool) -> Response {
    match result {
        Ok(body) => no_store(Json(body).into_response()),
        Err(error) => error_response(&error, used_basic),
    }
}

async fn token(
    State(dev): State<Dev>,
    client: Option<Extension<ClientMeta>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let client = client.map(|Extension(meta)| meta).unwrap_or_default();
    match OAuthRequest::read(&dev, &client, &headers, &body).await {
        Ok(request) => respond(dev.token_request(&request.credentials, &request.form).await, request.used_basic),
        Err(error) => error_response(&error, false),
    }
}

async fn revoke(
    State(dev): State<Dev>,
    client: Option<Extension<ClientMeta>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let client = client.map(|Extension(meta)| meta).unwrap_or_default();
    match OAuthRequest::read(&dev, &client, &headers, &body).await {
        // RFC 7009 section 2.2: an empty 200.
        Ok(request) => match dev.revoke_request(&request.credentials, &request.form).await {
            Ok(()) => no_store(StatusCode::OK.into_response()),
            Err(error) => error_response(&error, request.used_basic),
        },
        Err(error) => error_response(&error, false),
    }
}

async fn device_authorization(
    State(dev): State<Dev>,
    client: Option<Extension<ClientMeta>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let client = client.map(|Extension(meta)| meta).unwrap_or_default();
    if let Err(limited) = dev
        .core
        .cache
        .rate_limit(&format!("dev-device-start:{}", client.ip), DEVICE_STARTS_PER_MINUTE, Duration::from_secs(60))
        .await
    {
        return error_response(&OAuthError::from(limited), false);
    }
    match OAuthRequest::read(&dev, &client, &headers, &body).await {
        Ok(request) => respond(dev.device_authorization(&request.credentials, &request.form).await, request.used_basic),
        Err(error) => error_response(&error, false),
    }
}

/// Authorization server metadata (RFC 8414), so clients configure
/// themselves from one URL.
async fn metadata(State(dev): State<Dev>) -> Json<serde_json::Value> {
    let issuer = dev.issuer();
    Json(serde_json::json!({
        "issuer": issuer,
        "authorization_endpoint": dev.core.web_url("/oauth/authorize"),
        "token_endpoint": format!("{issuer}/oauth/token"),
        "revocation_endpoint": format!("{issuer}/oauth/revoke"),
        "device_authorization_endpoint": format!("{issuer}/oauth/device_authorization"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token", GRANT_DEVICE_CODE],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post", "none"],
        "revocation_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post", "none"],
        "scopes_supported": scopes::catalogue().into_iter().map(|scope| scope.scope).collect::<Vec<_>>(),
        "authorization_response_iss_parameter_supported": true,
        "service_documentation": dev.core.web_url("/developers"),
    }))
}

/// The `google.protobuf.FileDescriptorSet` of the whole API, for code
/// generators and reflection-based tools. A large download open to anyone,
/// so it is counted per address.
async fn descriptor(State(dev): State<Dev>, client: Option<Extension<ClientMeta>>) -> Response {
    let client = client.map(|Extension(meta)| meta).unwrap_or_default();
    if dev.core.cache.rate_limit(&format!("dev-descriptor:{}", client.ip), 30, Duration::from_secs(60)).await.is_err() {
        return (StatusCode::TOO_MANY_REQUESTS, "too many requests").into_response();
    }
    let headers = [(header::CONTENT_TYPE, "application/x-protobuf"), (header::CACHE_CONTROL, "public, max-age=300")];
    (headers, tokuly_proto::FILE_DESCRIPTOR_SET).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn basic(value: &str) -> OAuthResult<Option<(String, String)>> {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, HeaderValue::from_str(value).unwrap());
        basic_credentials(&headers)
    }

    #[test]
    fn basic_credentials_are_form_decoded() {
        let encode = |raw: &str| format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(raw));
        assert_eq!(basic(&encode("client:secret")).unwrap(), Some(("client".to_owned(), "secret".to_owned())));
        assert_eq!(
            basic(&encode("my%20app:p%3Ass+word")).unwrap(),
            Some(("my app".to_owned(), "p:ss word".to_owned()))
        );
        assert_eq!(basic(&encode("no-colon")).unwrap_err().error, "invalid_client");
        assert_eq!(basic("Basic !!!").unwrap_err().error, "invalid_client");
        // Another scheme is simply not client authentication.
        assert_eq!(basic("Bearer abc").unwrap(), None);
        assert_eq!(basic_credentials(&HeaderMap::new()).unwrap(), None);
    }
}
