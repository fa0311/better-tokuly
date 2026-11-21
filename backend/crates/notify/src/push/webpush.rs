//! Web Push (RFC 8030): notifications for browsers.
//!
//! A browser subscription is an endpoint URL at the browser vendor's push
//! service plus two keys. Sending means POSTing the encrypted message
//! ([`super::ece`]) to that URL with a VAPID token ([`super::vapid`]).
//!
//! The endpoint comes from the client, and this server makes requests to it,
//! so [`EndpointPolicy`] decides which URLs are acceptable — when a device is
//! registered and again every time one is used.
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tokuly_proto::notify::PushChannelKind;

use super::{
    PushChannel, PushMessage, PushOutcome, PushTarget, Trim, collapse_id, ece, fit_json,
    network_failure, read_response, truncate_chars, vapid::Vapid,
};
use crate::config::NotifyConfig;

const MAX_ENDPOINT_LEN: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointError {
    /// Not a URL we could ever send to.
    Invalid,
    /// A well-formed URL that does not belong to an allowed push service.
    NotAllowed,
}

/// Which Web Push endpoints the server is willing to send requests to.
#[derive(Debug, Clone)]
pub struct EndpointPolicy {
    /// Development and tests: `http://` and any host are fine.
    allow_insecure: bool,
    /// `*` was configured: any public-looking https host.
    allow_any_host: bool,
    /// Lowercase host names; subdomains match too.
    allowed_hosts: Vec<String>,
}

impl EndpointPolicy {
    pub fn from_config(cfg: &NotifyConfig) -> Self {
        let hosts: Vec<String> = cfg
            .webpush_allowed_hosts
            .split(',')
            .map(|h| {
                h.trim()
                    .trim_start_matches("*.")
                    .trim_matches('.')
                    .to_ascii_lowercase()
            })
            .filter(|h| !h.is_empty())
            .collect();
        Self {
            allow_insecure: cfg.webpush_allow_insecure,
            allow_any_host: hosts.iter().any(|h| h == "*"),
            allowed_hosts: hosts.into_iter().filter(|h| h != "*").collect(),
        }
    }

    /// Parses and vets a subscription endpoint.
    pub fn check(&self, endpoint: &str) -> Result<url::Url, EndpointError> {
        if endpoint.len() > MAX_ENDPOINT_LEN {
            return Err(EndpointError::Invalid);
        }
        let url = url::Url::parse(endpoint).map_err(|_| EndpointError::Invalid)?;
        let has_credentials = !url.username().is_empty() || url.password().is_some();
        if has_credentials || url.host().is_none() {
            return Err(EndpointError::Invalid);
        }
        match url.scheme() {
            "https" => {}
            "http" if self.allow_insecure => {}
            "http" => return Err(EndpointError::NotAllowed),
            _ => return Err(EndpointError::Invalid),
        }
        if self.allow_insecure {
            return Ok(url);
        }
        // Push services live on the default port of a named host. IP literals
        // and single-label names are how requests get aimed at internal services.
        let Some(url::Host::Domain(host)) = url.host() else {
            return Err(EndpointError::NotAllowed);
        };
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        if url.port().is_some() {
            return Err(EndpointError::NotAllowed);
        }
        let allowed = if self.allow_any_host {
            looks_public(&host)
        } else {
            self.allowed_hosts
                .iter()
                .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
        };
        if allowed {
            Ok(url)
        } else {
            Err(EndpointError::NotAllowed)
        }
    }
}

/// A multi-label name outside the special-use zones that only resolve inside
/// a network.
fn looks_public(host: &str) -> bool {
    const INTERNAL_SUFFIXES: &[&str] = &[
        "localhost",
        "local",
        "internal",
        "lan",
        "home",
        "corp",
        "intranet",
        "test",
        "invalid",
        "example",
    ];
    match host.rsplit_once('.') {
        Some((_, tld)) => !INTERNAL_SUFFIXES.contains(&tld),
        None => false,
    }
}

pub struct WebPush {
    http: reqwest::Client,
    vapid: Arc<Vapid>,
    policy: EndpointPolicy,
}

impl WebPush {
    pub fn new(http: reqwest::Client, vapid: Arc<Vapid>, policy: EndpointPolicy) -> Self {
        Self {
            http,
            vapid,
            policy,
        }
    }
}

#[async_trait]
impl PushChannel for WebPush {
    fn kind(&self) -> PushChannelKind {
        PushChannelKind::PUSH_CHANNEL_KIND_WEB_PUSH
    }

    async fn send(&self, target: &PushTarget, message: &PushMessage) -> PushOutcome {
        // The policy may have been tightened since the device registered.
        let Ok(endpoint) = self.policy.check(&target.token) else {
            return PushOutcome::Gone("endpoint is no longer allowed".into());
        };
        let (Some(p256dh), Some(auth)) = (&target.p256dh, &target.auth) else {
            return PushOutcome::Gone("subscription keys are missing".into());
        };
        let body = match ece::encrypt(&payload(message), p256dh, auth) {
            Ok(body) => body,
            Err(e) => return PushOutcome::Gone(format!("subscription keys are unusable: {e}")),
        };
        let authorization = match self.vapid.authorization(&endpoint) {
            Ok(value) => value,
            Err(e) => return PushOutcome::Rejected(format!("{e}")),
        };
        let mut request = self
            .http
            .post(endpoint)
            .header("ttl", message.ttl_seconds)
            // RFC 8030 section 5.3: "high" may wake a sleeping device.
            .header(
                "urgency",
                if message.priority.is_high() {
                    "high"
                } else {
                    "normal"
                },
            )
            .header("content-encoding", "aes128gcm")
            .header("content-type", "application/octet-stream")
            .header("authorization", authorization);
        if let Some(key) = &message.collapse_key {
            request = request.header("topic", collapse_id(key));
        }
        match request.body(body).send().await {
            Ok(response) => {
                let (status, detail) = read_response(response).await;
                classify(status, &detail)
            }
            Err(e) => network_failure(&e),
        }
    }
}

/// What a push service's answer means (RFC 8030 section 5 and vendor docs).
pub fn classify(status: u16, detail: &str) -> PushOutcome {
    let describe = || {
        format!(
            "push service answered {status}: {}",
            truncate_chars(detail.trim(), 200)
        )
    };
    match status {
        200..=299 => PushOutcome::Delivered,
        // The subscription expired or the user unsubscribed.
        404 | 410 => PushOutcome::Gone(describe()),
        429 | 500..=599 => PushOutcome::Retry(describe()),
        _ => PushOutcome::Rejected(describe()),
    }
}

/// The plaintext a browser receives: a declarative push message (Push API,
/// "declarative push message"). Browsers that support it show the
/// notification without running any script; everywhere else the service
/// worker's `push` handler reads the same JSON.
pub fn payload(message: &PushMessage) -> Vec<u8> {
    fit_json(ece::MAX_PLAINTEXT_LEN, |trim| payload_json(message, trim))
}

fn payload_json(message: &PushMessage, trim: Trim) -> serde_json::Value {
    let mut data = json!({
        "kind": message.kind,
        "category": message.category,
        "path": message.path,
    });
    if let Some(id) = message.notification_id {
        data["id"] = json!(id);
    }
    if trim.keeps_extra_data() && !message.data.is_null() {
        data["extra"] = message.data.clone();
    }
    if message.priority.is_urgent() {
        data["urgent"] = json!(true);
    }
    let mut notification = json!({
        "title": trim.title(&message.title),
        "body": trim.body(&message.body),
        "navigate": message.url,
        "lang": message.locale,
        "timestamp": message.sent_at.timestamp_millis(),
        "data": data,
    });
    // Same tag = the newer notification replaces the older one on screen.
    let tag = message
        .collapse_key
        .clone()
        .or_else(|| message.notification_id.map(|id| id.to_string()));
    if let Some(tag) = tag {
        notification["tag"] = json!(tag);
    }
    if message.priority.is_urgent() {
        // An incoming call or an SOS stays on screen until the user reacts.
        notification["requireInteraction"] = json!(true);
    }
    if trim.keeps_image()
        && let Some(image) = &message.image_url
    {
        notification["image"] = json!(image);
    }
    json!({
        "web_push": 8030,
        "notification": notification,
        // Unread count for the app icon (Badging API; set by the service worker).
        "app_badge": message.badge.max(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::push::{PushPriority, sample_message};

    fn policy(hosts: &str, insecure: bool) -> EndpointPolicy {
        EndpointPolicy::from_config(&NotifyConfig {
            webpush_allowed_hosts: hosts.into(),
            webpush_allow_insecure: insecure,
            ..Default::default()
        })
    }

    #[test]
    fn default_policy_accepts_the_browser_push_services() {
        let policy = EndpointPolicy::from_config(&NotifyConfig::default());
        for endpoint in [
            "https://fcm.googleapis.com/fcm/send/abc:def",
            "https://updates.push.services.mozilla.com/wpush/v2/gAAAA",
            "https://wns2-par02p.notify.windows.com/w/?token=abc",
            "https://web.push.apple.com/QGuOvVmF",
        ] {
            assert!(policy.check(endpoint).is_ok(), "{endpoint}");
        }
    }

    #[test]
    fn default_policy_refuses_everything_else() {
        let policy = EndpointPolicy::from_config(&NotifyConfig::default());
        let cases = [
            (
                "http://fcm.googleapis.com/fcm/send/abc",
                EndpointError::NotAllowed,
            ), // not https
            (
                "https://evil.example/fcm.googleapis.com",
                EndpointError::NotAllowed,
            ),
            (
                "https://fcm.googleapis.com.evil.example/x",
                EndpointError::NotAllowed,
            ),
            ("https://notfcm.googleapis.com/x", EndpointError::NotAllowed), // suffix must be a whole label
            (
                "https://169.254.169.254/latest/meta-data",
                EndpointError::NotAllowed,
            ),
            ("https://[::1]/x", EndpointError::NotAllowed),
            ("https://localhost/x", EndpointError::NotAllowed),
            (
                "https://fcm.googleapis.com:8443/x",
                EndpointError::NotAllowed,
            ),
            (
                "https://user:pw@fcm.googleapis.com/x",
                EndpointError::Invalid,
            ),
            ("ftp://fcm.googleapis.com/x", EndpointError::Invalid),
            ("not a url", EndpointError::Invalid),
            ("", EndpointError::Invalid),
        ];
        for (endpoint, expected) in cases {
            assert_eq!(policy.check(endpoint).unwrap_err(), expected, "{endpoint}");
        }
        let long = format!(
            "https://fcm.googleapis.com/{}",
            "a".repeat(MAX_ENDPOINT_LEN)
        );
        assert_eq!(policy.check(&long).unwrap_err(), EndpointError::Invalid);
    }

    #[test]
    fn wildcard_allows_public_hosts_only() {
        let policy = policy("*", false);
        assert!(policy.check("https://push.vendor.example.org/s/1").is_ok());
        for endpoint in [
            "https://intranet/x",
            "https://db.internal/x",
            "https://printer.local/x",
            "https://10.0.0.5/x",
            "http://push.vendor.example.org/x",
        ] {
            assert_eq!(
                policy.check(endpoint).unwrap_err(),
                EndpointError::NotAllowed,
                "{endpoint}"
            );
        }
    }

    #[test]
    fn insecure_mode_accepts_a_local_stand_in() {
        let policy = policy("fcm.googleapis.com", true);
        assert!(policy.check("http://127.0.0.1:4010/push/abc").is_ok());
        assert_eq!(
            policy.check("file:///etc/passwd").unwrap_err(),
            EndpointError::Invalid
        );
    }

    #[test]
    fn responses_map_to_outcomes() {
        assert_eq!(classify(201, ""), PushOutcome::Delivered);
        assert!(matches!(classify(410, "gone"), PushOutcome::Gone(_)));
        assert!(matches!(classify(404, ""), PushOutcome::Gone(_)));
        assert!(matches!(classify(429, ""), PushOutcome::Retry(_)));
        assert!(matches!(classify(503, ""), PushOutcome::Retry(_)));
        assert!(matches!(
            classify(403, "bad vapid"),
            PushOutcome::Rejected(_)
        ));
        assert!(matches!(classify(413, ""), PushOutcome::Rejected(_)));
        assert!(
            matches!(classify(301, ""), PushOutcome::Rejected(_)),
            "redirects are not followed"
        );
    }

    #[test]
    fn payload_is_a_declarative_push_message() {
        let message = sample_message();
        let value: serde_json::Value = serde_json::from_slice(&payload(&message)).unwrap();
        assert_eq!(value["web_push"], 8030);
        assert_eq!(value["app_badge"], 3);
        let notification = &value["notification"];
        assert_eq!(notification["title"], "ありす");
        assert_eq!(notification["body"], "やっほー！");
        assert_eq!(notification["navigate"], "https://tokuly.example/chat/42");
        assert_eq!(notification["lang"], "ja");
        assert_eq!(notification["tag"], "chat:42");
        assert_eq!(notification["image"], "https://cdn.example/a.webp");
        assert_eq!(notification["data"]["kind"], "chat.message");
        assert_eq!(notification["data"]["path"], "/chat/42");
        assert_eq!(notification["data"]["extra"]["conversation_id"], "42");
    }

    #[test]
    fn only_urgent_notifications_insist_on_being_seen() {
        let mut message = sample_message();
        let value: serde_json::Value = serde_json::from_slice(&payload(&message)).unwrap();
        assert!(value["notification"].get("requireInteraction").is_none());
        assert!(value["notification"]["data"].get("urgent").is_none());

        message.priority = PushPriority::Urgent;
        let value: serde_json::Value = serde_json::from_slice(&payload(&message)).unwrap();
        assert_eq!(value["notification"]["requireInteraction"], true);
        assert_eq!(value["notification"]["data"]["urgent"], true);
    }

    #[test]
    fn oversized_payloads_shed_data_before_text() {
        let mut message = sample_message();
        message.data = json!({ "blob": "x".repeat(5000) });
        message.body = "あ".repeat(400);
        let bytes = payload(&message);
        assert!(bytes.len() <= ece::MAX_PLAINTEXT_LEN);
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            value["notification"]["data"].get("extra").is_none(),
            "extra data went first"
        );
        assert_eq!(
            value["notification"]["body"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            400,
            "the text survived"
        );

        // Japanese text is three bytes per character: a long body alone can overflow.
        message.data = serde_json::Value::Null;
        message.body = "あ".repeat(5000);
        message.title = "い".repeat(500);
        let bytes = payload(&message);
        assert!(bytes.len() <= ece::MAX_PLAINTEXT_LEN);
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            value["notification"]["body"]
                .as_str()
                .unwrap()
                .ends_with('…')
        );
    }
}
