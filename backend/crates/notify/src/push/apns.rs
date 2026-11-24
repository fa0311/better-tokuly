//! Apple Push Notification service (iOS devices), token-based authentication.
//!
//! One HTTP/2 request per notification: `POST /3/device/{token}` with the
//! notification as JSON and its delivery options as `apns-*` headers. The
//! server proves who it is with a provider token: an ES256 JWT signed with
//! the team's `.p8` key, reused for 40 minutes (Apple refuses tokens older
//! than an hour and throttles servers that mint one more often than every 20
//! minutes).
//!
//! Only enabled when `TOKULY_APNS_KEY_P8`, `_KEY_ID`, `_TEAM_ID` and `_TOPIC`
//! are set.
use async_trait::async_trait;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokuly_core::prelude::*;
use tokuly_proto::notify::PushChannelKind;

use super::{
    PushChannel, PushMessage, PushOutcome, PushTarget, Trim, collapse_id, fit_json, network_failure, read_response,
};

const PRODUCTION_ENDPOINT: &str = "https://api.push.apple.com";
const SANDBOX_ENDPOINT: &str = "https://api.sandbox.push.apple.com";
const PROVIDER_TOKEN_LIFETIME_SECONDS: i64 = 40 * 60;
/// APNs' limit for a notification payload.
const MAX_PAYLOAD_BYTES: usize = 4096;

/// Claims of a provider token.
#[derive(Debug, Serialize, Deserialize)]
pub struct ProviderClaims {
    /// The Apple developer team ID.
    pub iss: String,
    pub iat: i64,
}

struct ProviderToken {
    value: String,
    issued_at: i64,
}

pub struct Apns {
    http: reqwest::Client,
    key_id: String,
    team_id: String,
    /// The app's bundle ID.
    topic: String,
    signing_key: EncodingKey,
    production_endpoint: String,
    sandbox_endpoint: String,
    provider_token: Mutex<Option<ProviderToken>>,
}

impl Apns {
    /// `key_p8` is the content of the `.p8` file (PKCS#8 PEM). Environment
    /// variables often carry it on one line, so literal `\n` is accepted.
    pub fn new(key_p8: &str, key_id: &str, team_id: &str, topic: &str, http: reqwest::Client) -> anyhow::Result<Self> {
        let pem = key_p8.trim().replace("\\n", "\n");
        let signing_key = EncodingKey::from_ec_pem(pem.as_bytes())
            .map_err(|e| anyhow::anyhow!("TOKULY_APNS_KEY_P8 is not a PKCS#8 EC private key in PEM form: {e}"))?;
        Ok(Self {
            http,
            key_id: key_id.trim().to_owned(),
            team_id: team_id.trim().to_owned(),
            topic: topic.trim().to_owned(),
            signing_key,
            production_endpoint: PRODUCTION_ENDPOINT.to_owned(),
            sandbox_endpoint: SANDBOX_ENDPOINT.to_owned(),
            provider_token: Mutex::new(None),
        })
    }

    /// Points the channel at other hosts (a local stand-in under test).
    pub fn with_endpoints(mut self, production: &str, sandbox: &str) -> Self {
        self.production_endpoint = production.trim_end_matches('/').to_owned();
        self.sandbox_endpoint = sandbox.trim_end_matches('/').to_owned();
        self
    }

    /// The current provider token, minting a new one when the cached one is
    /// due for replacement.
    pub fn provider_token(&self) -> AppResult<String> {
        let now = chrono::Utc::now().timestamp();
        let mut cached = self.provider_token.lock();
        if let Some(token) = cached.as_ref()
            && now - token.issued_at < PROVIDER_TOKEN_LIFETIME_SECONDS
        {
            return Ok(token.value.clone());
        }
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.key_id.clone());
        // Apple documents exactly two header fields: `alg` and `kid`.
        header.typ = None;
        let claims = ProviderClaims { iss: self.team_id.clone(), iat: now };
        let value = jsonwebtoken::encode(&header, &claims, &self.signing_key)
            .map_err(|e| AppError::internal(anyhow::anyhow!("APNs provider token signing failed: {e}")))?;
        *cached = Some(ProviderToken { value: value.clone(), issued_at: now });
        Ok(value)
    }

    fn forget_provider_token(&self) {
        *self.provider_token.lock() = None;
    }

    /// Where the notification for a device goes. Tokens issued to development
    /// builds only exist in the sandbox environment.
    pub fn device_url(&self, target: &PushTarget) -> String {
        let endpoint = if target.apns_sandbox { &self.sandbox_endpoint } else { &self.production_endpoint };
        format!("{endpoint}/3/device/{}", target.token)
    }

    /// The `apns-*` request headers (everything but `authorization`).
    pub fn headers(&self, message: &PushMessage) -> Vec<(&'static str, String)> {
        let mut headers = vec![
            ("apns-topic", self.topic.clone()),
            ("apns-push-type", "alert".to_owned()),
            // 10 delivers immediately; 5 lets the device pick a power-friendly moment.
            ("apns-priority", if message.priority.is_high() { "10" } else { "5" }.to_owned()),
            ("apns-expiration", message.expires_at().to_string()),
        ];
        if let Some(key) = &message.collapse_key {
            headers.push(("apns-collapse-id", collapse_id(key)));
        }
        headers
    }
}

#[async_trait]
impl PushChannel for Apns {
    fn kind(&self) -> PushChannelKind {
        PushChannelKind::PUSH_CHANNEL_KIND_APNS
    }

    async fn send(&self, target: &PushTarget, message: &PushMessage) -> PushOutcome {
        let provider_token = match self.provider_token() {
            Ok(token) => token,
            Err(e) => return PushOutcome::Rejected(e.to_string()),
        };
        let mut request = self
            .http
            .post(self.device_url(target))
            .header("authorization", format!("bearer {provider_token}"))
            .header("content-type", "application/json");
        for (name, value) in self.headers(message) {
            request = request.header(name, value);
        }
        let body = fit_json(MAX_PAYLOAD_BYTES, |trim| payload(message, trim));
        match request.body(body).send().await {
            Ok(response) => {
                let (status, detail) = read_response(response).await;
                if status == 403 {
                    // Whatever was wrong with the provider token, the next
                    // attempt should not reuse it.
                    self.forget_provider_token();
                }
                classify(status, &detail)
            }
            Err(e) => network_failure(&e),
        }
    }
}

/// The notification payload: the `aps` dictionary iOS understands plus our
/// own keys for routing a tap.
pub fn payload(message: &PushMessage, trim: Trim) -> serde_json::Value {
    // Notifications of one conversation (or category) stack together.
    let thread = message.collapse_key.as_deref().map(collapse_id).unwrap_or_else(|| message.category.clone());
    let mut aps = json!({
        "alert": { "title": trim.title(&message.title), "body": trim.body(&message.body) },
        "badge": message.badge.max(0),
        "sound": "default",
        "thread-id": thread,
    });
    let mut payload = json!({
        "kind": message.kind,
        "category": message.category,
        "path": message.path,
        "url": message.url,
    });
    if let Some(id) = message.notification_id {
        payload["notification_id"] = json!(id);
    }
    if trim.keeps_image()
        && let Some(image) = &message.image_url
    {
        payload["image_url"] = json!(image);
        // Lets the app's notification service extension download and attach it.
        aps["mutable-content"] = json!(1);
    }
    if trim.keeps_extra_data() && !message.data.is_null() {
        payload["data"] = message.data.clone();
    }
    if message.priority.is_urgent() {
        // Time-sensitive notifications are shown immediately even when a
        // Focus is on or the user gets a scheduled summary.
        aps["interruption-level"] = json!("time-sensitive");
        payload["urgent"] = json!(true);
    }
    payload["aps"] = aps;
    payload
}

/// What an APNs answer means. Failures carry `{"reason": "BadDeviceToken"}`.
pub fn classify(status: u16, body: &str) -> PushOutcome {
    if status == 200 {
        return PushOutcome::Delivered;
    }
    let reason = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("reason").and_then(|r| r.as_str()).map(str::to_owned))
        .unwrap_or_default();
    let detail = format!("APNs answered {status} {reason}");
    match (status, reason.as_str()) {
        // The app was removed, or the token is not one of this app's
        // (wrong environment, another bundle).
        (410, _) | (_, "BadDeviceToken") | (_, "DeviceTokenNotForTopic") => PushOutcome::Gone(detail),
        (_, "ExpiredProviderToken") | (429, _) | (500..=599, _) => PushOutcome::Retry(detail),
        _ => PushOutcome::Rejected(detail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::push::{PushPriority, sample_message};

    #[test]
    fn payload_has_the_aps_dictionary_and_routing_keys() {
        let message = sample_message();
        let value = payload(&message, Trim::NONE);
        assert_eq!(value["aps"]["alert"], json!({ "title": "ありす", "body": "やっほー！" }));
        assert_eq!(value["aps"]["badge"], 3);
        assert_eq!(value["aps"]["sound"], "default");
        assert_eq!(value["aps"]["thread-id"], collapse_id("chat:42"));
        assert_eq!(value["aps"]["mutable-content"], 1);
        assert_eq!(value["image_url"], "https://cdn.example/a.webp");
        assert_eq!(value["kind"], "chat.message");
        assert_eq!(value["path"], "/chat/42");
        assert_eq!(value["url"], "https://tokuly.example/chat/42");
        assert_eq!(value["notification_id"], Uuid::nil().to_string());
        assert_eq!(value["data"], json!({ "conversation_id": "42" }));
    }

    #[test]
    fn payload_without_image_or_collapse_key() {
        let mut message = sample_message();
        message.image_url = None;
        message.collapse_key = None;
        message.badge = -4;
        let value = payload(&message, Trim::NONE);
        assert!(value["aps"].get("mutable-content").is_none());
        assert_eq!(value["aps"]["thread-id"], "chat", "falls back to the category");
        assert_eq!(value["aps"]["badge"], 0);
    }

    #[test]
    fn urgent_notifications_are_time_sensitive() {
        let mut message = sample_message();
        let value = payload(&message, Trim::NONE);
        assert!(value["aps"].get("interruption-level").is_none());
        assert!(value.get("urgent").is_none());

        message.priority = PushPriority::Urgent;
        let value = payload(&message, Trim::NONE);
        assert_eq!(value["aps"]["interruption-level"], "time-sensitive");
        assert_eq!(value["urgent"], true);
    }

    #[test]
    fn oversized_payloads_are_trimmed_to_the_limit() {
        let mut message = sample_message();
        message.data = json!({ "blob": "x".repeat(6000) });
        message.body = "あ".repeat(3000);
        let bytes = fit_json(MAX_PAYLOAD_BYTES, |trim| payload(&message, trim));
        assert!(bytes.len() <= MAX_PAYLOAD_BYTES);
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value.get("data").is_none());
        assert_eq!(value["aps"]["alert"]["title"], "ありす");
    }

    #[test]
    fn responses_map_to_outcomes() {
        let reason = |r: &str| json!({ "reason": r }).to_string();
        assert_eq!(classify(200, ""), PushOutcome::Delivered);
        assert!(matches!(classify(410, &reason("Unregistered")), PushOutcome::Gone(_)));
        assert!(matches!(classify(410, &reason("ExpiredToken")), PushOutcome::Gone(_)));
        assert!(matches!(classify(400, &reason("BadDeviceToken")), PushOutcome::Gone(_)));
        assert!(matches!(classify(400, &reason("DeviceTokenNotForTopic")), PushOutcome::Gone(_)));
        assert!(matches!(classify(403, &reason("ExpiredProviderToken")), PushOutcome::Retry(_)));
        assert!(matches!(classify(429, &reason("TooManyRequests")), PushOutcome::Retry(_)));
        assert!(matches!(classify(503, &reason("ServiceUnavailable")), PushOutcome::Retry(_)));
        assert!(matches!(classify(403, &reason("InvalidProviderToken")), PushOutcome::Rejected(_)));
        assert!(matches!(classify(413, &reason("PayloadTooLarge")), PushOutcome::Rejected(_)));
        assert!(matches!(classify(400, &reason("BadTopic")), PushOutcome::Rejected(_)));
        assert!(matches!(classify(400, "not json"), PushOutcome::Rejected(_)));
    }

    #[test]
    fn a_key_that_is_not_pem_is_reported_at_startup() {
        assert!(Apns::new("garbage", "KEYID", "TEAMID", "app.tokuly", reqwest::Client::new()).is_err());
    }
}
