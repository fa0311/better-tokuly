//! Firebase Cloud Messaging through the HTTP v1 API (Android devices).
//!
//! Authentication is OAuth 2.0 with a service account: the server signs a
//! short assertion with the account's RSA key, exchanges it for an access
//! token at Google's token endpoint, and sends
//! `POST /v1/projects/{project}/messages:send` with that token. Access tokens
//! live an hour and are cached until shortly before they expire.
//!
//! Only enabled when `TOKULY_FCM_SERVICE_ACCOUNT_JSON` is set.
use async_trait::async_trait;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokuly_core::prelude::*;
use tokuly_proto::notify::PushChannelKind;

use super::{
    PushChannel, PushMessage, PushOutcome, PushTarget, Trim, collapse_id, fit_json, network_failure, read_response,
    truncate_chars,
};

const DEFAULT_ENDPOINT: &str = "https://fcm.googleapis.com";
const DEFAULT_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
const JWT_BEARER_GRANT: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
/// Google caps assertions at one hour.
const ASSERTION_LIFETIME_SECONDS: i64 = 3600;
/// An access token is replaced once it has less than this left.
const REFRESH_MARGIN_SECONDS: i64 = 120;
/// FCM's limit for one message.
const MAX_MESSAGE_BYTES: usize = 4096;

/// The fields of a service-account key file this channel uses.
#[derive(Deserialize)]
struct ServiceAccount {
    project_id: String,
    client_email: String,
    /// PKCS#8 PEM.
    private_key: String,
    #[serde(default)]
    private_key_id: String,
    #[serde(default = "default_token_uri")]
    token_uri: String,
}

fn default_token_uri() -> String {
    DEFAULT_TOKEN_URI.to_owned()
}

/// Claims of the assertion exchanged for an access token.
#[derive(Debug, Serialize, Deserialize)]
pub struct AssertionClaims {
    /// The service account's email.
    pub iss: String,
    pub scope: String,
    /// The token endpoint.
    pub aud: String,
    pub iat: i64,
    pub exp: i64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: i64,
}

struct AccessToken {
    value: String,
    expires_at: i64,
}

pub struct Fcm {
    http: reqwest::Client,
    project_id: String,
    client_email: String,
    key_id: String,
    signing_key: EncodingKey,
    token_uri: String,
    endpoint: String,
    /// Held across the refresh so concurrent sends share one token request.
    access_token: tokio::sync::Mutex<Option<AccessToken>>,
}

impl Fcm {
    /// Builds the channel from the content of a service-account JSON file.
    pub fn from_service_account(json: &str, http: reqwest::Client) -> anyhow::Result<Self> {
        let account: ServiceAccount = serde_json::from_str(json)
            .map_err(|e| anyhow::anyhow!("TOKULY_FCM_SERVICE_ACCOUNT_JSON is not a service-account key file: {e}"))?;
        anyhow::ensure!(
            !account.project_id.is_empty() && !account.client_email.is_empty(),
            "the FCM service account lacks project_id or client_email"
        );
        let signing_key = EncodingKey::from_rsa_pem(account.private_key.as_bytes())
            .map_err(|e| anyhow::anyhow!("the FCM service account's private_key is not an RSA key in PEM form: {e}"))?;
        Ok(Self {
            http,
            project_id: account.project_id,
            client_email: account.client_email,
            key_id: account.private_key_id,
            signing_key,
            token_uri: account.token_uri,
            endpoint: DEFAULT_ENDPOINT.to_owned(),
            access_token: tokio::sync::Mutex::new(None),
        })
    }

    /// Points the channel at another FCM host (a local stand-in under test).
    pub fn with_endpoint(mut self, endpoint: &str) -> Self {
        self.endpoint = endpoint.trim_end_matches('/').to_owned();
        self
    }

    pub fn send_url(&self) -> String {
        format!("{}/v1/projects/{}/messages:send", self.endpoint, self.project_id)
    }

    /// The signed assertion (RFC 7523) that proves we hold the account's key.
    pub fn assertion(&self, now: i64) -> AppResult<String> {
        let claims = AssertionClaims {
            iss: self.client_email.clone(),
            scope: SCOPE.to_owned(),
            aud: self.token_uri.clone(),
            iat: now,
            exp: now + ASSERTION_LIFETIME_SECONDS,
        };
        let mut header = Header::new(Algorithm::RS256);
        if !self.key_id.is_empty() {
            header.kid = Some(self.key_id.clone());
        }
        jsonwebtoken::encode(&header, &claims, &self.signing_key)
            .map_err(|e| AppError::internal(anyhow::anyhow!("FCM assertion signing failed: {e}")))
    }

    /// A valid access token, fetched or refreshed when needed. An `Err` is
    /// the outcome to report for the message that needed it.
    async fn access_token(&self) -> Result<String, PushOutcome> {
        let mut cached = self.access_token.lock().await;
        let now = chrono::Utc::now().timestamp();
        if let Some(token) = cached.as_ref()
            && token.expires_at - now > REFRESH_MARGIN_SECONDS
        {
            return Ok(token.value.clone());
        }
        let assertion = self.assertion(now).map_err(|e| PushOutcome::Rejected(e.to_string()))?;
        let response = self
            .http
            .post(&self.token_uri)
            .form(&[("grant_type", JWT_BEARER_GRANT), ("assertion", assertion.as_str())])
            .send()
            .await
            .map_err(|e| network_failure(&e))?;
        let (status, body) = read_response(response).await;
        if status != 200 {
            let detail = format!("FCM token endpoint answered {status}: {}", truncate_chars(body.trim(), 200));
            // 4xx means Google refused our credentials; no retry fixes that.
            return Err(if status == 429 || status >= 500 {
                PushOutcome::Retry(detail)
            } else {
                PushOutcome::Rejected(detail)
            });
        }
        let token: TokenResponse = serde_json::from_str(&body)
            .map_err(|e| PushOutcome::Retry(format!("FCM token endpoint sent an unreadable answer: {e}")))?;
        let lifetime = if token.expires_in > 0 { token.expires_in } else { ASSERTION_LIFETIME_SECONDS };
        let value = token.access_token.clone();
        *cached = Some(AccessToken { value: token.access_token, expires_at: now + lifetime });
        Ok(value)
    }

    async fn forget_access_token(&self) {
        *self.access_token.lock().await = None;
    }
}

#[async_trait]
impl PushChannel for Fcm {
    fn kind(&self) -> PushChannelKind {
        PushChannelKind::PUSH_CHANNEL_KIND_FCM
    }

    async fn send(&self, target: &PushTarget, message: &PushMessage) -> PushOutcome {
        let access_token = match self.access_token().await {
            Ok(token) => token,
            Err(outcome) => return outcome,
        };
        let body = fit_json(MAX_MESSAGE_BYTES, |trim| request_body(&target.token, message, trim));
        let response = self
            .http
            .post(self.send_url())
            .bearer_auth(access_token)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await;
        match response {
            Ok(response) => {
                let (status, detail) = read_response(response).await;
                if status == 401 {
                    // The token was revoked early; the retry fetches a new one.
                    self.forget_access_token().await;
                }
                classify(status, &detail)
            }
            Err(e) => network_failure(&e),
        }
    }
}

/// The `messages:send` request body for one device.
///
/// It is a notification message (the system tray shows it while the app is in
/// the background) with a `data` block carrying what the app needs to route a
/// tap. `data` values must be strings.
pub fn request_body(token: &str, message: &PushMessage, trim: Trim) -> serde_json::Value {
    let badge = message.badge.max(0);
    let mut data = json!({
        "kind": message.kind,
        "category": message.category,
        "path": message.path,
        "url": message.url,
        "badge": badge.to_string(),
    });
    if let Some(id) = message.notification_id {
        data["notification_id"] = json!(id.to_string());
    }
    if trim.keeps_extra_data() && !message.data.is_null() {
        data["data"] = json!(message.data.to_string());
    }
    if message.priority.is_urgent() {
        data["urgent"] = json!("true");
    }
    let mut notification = json!({ "title": trim.title(&message.title), "body": trim.body(&message.body) });
    if trim.keeps_image()
        && let Some(image) = &message.image_url
    {
        notification["image"] = json!(image);
    }
    let mut android = json!({
        // HIGH is delivered at once, even to a device in Doze.
        "priority": if message.priority.is_high() { "HIGH" } else { "NORMAL" },
        "ttl": format!("{}s", message.ttl_seconds),
        // One Android notification channel per preference category.
        "notification": { "channel_id": message.category, "notification_count": badge },
    });
    // Used when the token belongs to an iOS app that registered through Firebase.
    let mut apns_headers = json!({ "apns-expiration": message.expires_at().to_string() });
    let mut aps = json!({ "badge": badge, "sound": "default" });
    if message.priority.is_urgent() {
        // Heads-up on Android; time-sensitive on iOS, which lets the
        // notification through Focus and the scheduled summary.
        android["notification"]["notification_priority"] = json!("PRIORITY_MAX");
        apns_headers["apns-priority"] = json!("10");
        aps["interruption-level"] = json!("time-sensitive");
    }
    if let Some(key) = &message.collapse_key {
        let id = collapse_id(key);
        android["collapse_key"] = json!(id);
        android["notification"]["tag"] = json!(id);
        apns_headers["apns-collapse-id"] = json!(id);
        aps["thread-id"] = json!(id);
    }
    json!({
        "message": {
            "token": token,
            "notification": notification,
            "data": data,
            "android": android,
            "apns": { "headers": apns_headers, "payload": { "aps": aps } },
        }
    })
}

/// What an FCM answer means. Errors look like
/// `{"error": {"status": "NOT_FOUND", "details": [{"errorCode": "UNREGISTERED"}]}}`.
pub fn classify(status: u16, body: &str) -> PushOutcome {
    if status == 200 {
        return PushOutcome::Delivered;
    }
    let error: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let error_code = error
        .pointer("/error/details")
        .and_then(|d| d.as_array())
        .and_then(|details| details.iter().find_map(|d| d.get("errorCode").and_then(|c| c.as_str())))
        .or_else(|| error.pointer("/error/status").and_then(|s| s.as_str()))
        .unwrap_or_default();
    let detail = format!("FCM answered {status} {error_code}");
    match error_code {
        // The app was uninstalled, or the token expired or belongs to another sender.
        "UNREGISTERED" | "SENDER_ID_MISMATCH" => PushOutcome::Gone(detail),
        "QUOTA_EXCEEDED" | "UNAVAILABLE" | "INTERNAL" => PushOutcome::Retry(detail),
        // A malformed message or broken APNs credentials on the Firebase project.
        "INVALID_ARGUMENT" | "THIRD_PARTY_AUTH_ERROR" => PushOutcome::Rejected(detail),
        _ => match status {
            404 => PushOutcome::Gone(detail),
            // 401: our access token was not accepted; a fresh one is fetched.
            401 | 429 | 500..=599 => PushOutcome::Retry(detail),
            _ => PushOutcome::Rejected(detail),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::push::{PushPriority, sample_message};

    #[test]
    fn bad_credentials_are_reported_at_startup() {
        assert!(Fcm::from_service_account("not json", reqwest::Client::new()).is_err());
        let no_key = json!({ "project_id": "p", "client_email": "e", "private_key": "nope" }).to_string();
        assert!(Fcm::from_service_account(&no_key, reqwest::Client::new()).is_err());
    }

    #[test]
    fn request_body_follows_the_v1_message_format() {
        let message = sample_message();
        let body = request_body("device-token", &message, Trim::NONE);
        let m = &body["message"];
        assert_eq!(m["token"], "device-token");
        assert_eq!(
            m["notification"],
            json!({ "title": "ありす", "body": "やっほー！", "image": "https://cdn.example/a.webp" })
        );
        let collapse = collapse_id("chat:42");
        assert_eq!(m["android"]["collapse_key"], collapse);
        assert_eq!(m["android"]["priority"], "HIGH");
        assert_eq!(m["android"]["ttl"], "3600s");
        assert_eq!(
            m["android"]["notification"],
            json!({ "channel_id": "chat", "notification_count": 3, "tag": collapse })
        );
        assert_eq!(m["apns"]["headers"]["apns-collapse-id"], collapse);
        assert_eq!(m["apns"]["headers"]["apns-expiration"], "1800003600");
        assert_eq!(m["apns"]["payload"]["aps"]["badge"], 3);
        let data = m["data"].as_object().unwrap();
        assert!(data.values().all(|v| v.is_string()), "FCM data values must be strings");
        assert_eq!(data["path"], "/chat/42");
        assert_eq!(data["url"], "https://tokuly.example/chat/42");
        assert_eq!(data["badge"], "3");
        assert_eq!(data["notification_id"], Uuid::nil().to_string());
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(data["data"].as_str().unwrap()).unwrap(),
            json!({ "conversation_id": "42" })
        );
    }

    #[test]
    fn request_body_without_collapse_key_or_extras() {
        let mut message = sample_message();
        message.collapse_key = None;
        message.image_url = None;
        message.data = serde_json::Value::Null;
        message.notification_id = None;
        message.priority = PushPriority::Normal;
        let body = request_body("t", &message, Trim::NONE);
        let m = &body["message"];
        assert!(m["android"].get("collapse_key").is_none());
        assert_eq!(m["android"]["priority"], "NORMAL");
        assert!(m["notification"].get("image").is_none());
        assert!(m["data"].get("data").is_none() && m["data"].get("notification_id").is_none());
    }

    #[test]
    fn urgent_messages_ask_for_the_highest_attention_on_both_platforms() {
        let mut message = sample_message();
        let high = request_body("t", &message, Trim::NONE);
        let m = &high["message"];
        assert_eq!(m["android"]["priority"], "HIGH");
        assert!(m["android"]["notification"].get("notification_priority").is_none());
        assert!(m["apns"]["payload"]["aps"].get("interruption-level").is_none());
        assert!(m["data"].get("urgent").is_none());

        message.priority = PushPriority::Urgent;
        let urgent = request_body("t", &message, Trim::NONE);
        let m = &urgent["message"];
        assert_eq!(m["android"]["priority"], "HIGH");
        assert_eq!(m["android"]["notification"]["notification_priority"], "PRIORITY_MAX");
        assert_eq!(m["apns"]["headers"]["apns-priority"], "10");
        assert_eq!(m["apns"]["payload"]["aps"]["interruption-level"], "time-sensitive");
        assert_eq!(m["data"]["urgent"], "true");
    }

    #[test]
    fn oversized_messages_are_trimmed_to_the_limit() {
        let mut message = sample_message();
        message.data = json!({ "blob": "x".repeat(6000) });
        let bytes = fit_json(MAX_MESSAGE_BYTES, |trim| request_body("t", &message, trim));
        assert!(bytes.len() <= MAX_MESSAGE_BYTES);
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(body["message"]["data"].get("data").is_none());
        assert_eq!(body["message"]["notification"]["body"], "やっほー！");
    }

    #[test]
    fn responses_map_to_outcomes() {
        let error = |status: &str, code: &str| {
            json!({ "error": { "code": 0, "status": status, "details": [{ "@type": "type.googleapis.com/google.firebase.fcm.v1.FcmError", "errorCode": code }] } }).to_string()
        };
        assert_eq!(classify(200, r#"{"name":"projects/p/messages/1"}"#), PushOutcome::Delivered);
        assert!(matches!(classify(404, &error("NOT_FOUND", "UNREGISTERED")), PushOutcome::Gone(_)));
        assert!(matches!(classify(403, &error("PERMISSION_DENIED", "SENDER_ID_MISMATCH")), PushOutcome::Gone(_)));
        assert!(matches!(classify(429, &error("RESOURCE_EXHAUSTED", "QUOTA_EXCEEDED")), PushOutcome::Retry(_)));
        assert!(matches!(classify(503, &error("UNAVAILABLE", "UNAVAILABLE")), PushOutcome::Retry(_)));
        assert!(matches!(classify(500, "<html>oops</html>"), PushOutcome::Retry(_)));
        assert!(
            matches!(classify(401, r#"{"error":{"status":"UNAUTHENTICATED"}}"#), PushOutcome::Retry(_)),
            "a fresh access token is fetched"
        );
        assert!(matches!(classify(400, &error("INVALID_ARGUMENT", "INVALID_ARGUMENT")), PushOutcome::Rejected(_)));
        assert!(matches!(classify(401, &error("UNAUTHENTICATED", "THIRD_PARTY_AUTH_ERROR")), PushOutcome::Rejected(_)));
        assert!(matches!(classify(403, r#"{"error":{"status":"PERMISSION_DENIED"}}"#), PushOutcome::Rejected(_)));
    }
}
