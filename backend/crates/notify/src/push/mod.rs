//! Push channels: the ways a notification reaches a device that is not
//! connected to the realtime gateway.
//!
//! Each channel ([`webpush`], [`fcm`], [`apns`]) implements [`PushChannel`]:
//! it takes one device and one already-localized message and reports what the
//! push service said. Everything about *whether* to push (preferences, quiet
//! hours, mutes) and about retries lives in `delivery.rs`, not here.
use async_trait::async_trait;
use sha2::{Digest as _, Sha256};
use tokuly_core::{egress, keys::b64url, prelude::*};
use tokuly_proto::notify::PushChannelKind;

pub mod apns;
pub mod ece;
pub mod fcm;
pub mod vapid;
pub mod webpush;

/// A registered device, as much as a channel needs to address it.
#[derive(Debug, Clone)]
pub struct PushTarget {
    /// Web Push endpoint URL, FCM registration token or APNs device token.
    pub token: String,
    /// Web Push only: the subscription's public key and auth secret.
    pub p256dh: Option<Vec<u8>>,
    pub auth: Option<Vec<u8>>,
    /// APNs only: deliver through the sandbox environment.
    pub apns_sandbox: bool,
}

/// How hard a push tries to get the user's attention. Every channel maps the
/// three levels onto what its push service offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushPriority {
    /// The push service may wait for a moment that suits the device's
    /// battery (a new video, a like).
    Normal,
    /// Delivered right away, waking a sleeping device (a message, a buzz, a
    /// stream going live).
    High,
    /// The producing domain marked the notification urgent (an incoming
    /// call, an SOS). Delivered like `High` and, where the platform has such
    /// a level, allowed to break through the device's own focus modes.
    Urgent,
}

impl PushPriority {
    /// Whether the device should be woken for the message.
    pub fn is_high(self) -> bool {
        self != Self::Normal
    }

    pub fn is_urgent(self) -> bool {
        self == Self::Urgent
    }
}

/// One notification, localized for one device.
#[derive(Debug, Clone)]
pub struct PushMessage {
    /// Inbox entry this push belongs to; `None` for push-only notifications.
    pub notification_id: Option<Uuid>,
    /// Dotted kind, e.g. `chat.message`.
    pub kind: String,
    /// Preference category of the kind; doubles as the Android channel ID.
    pub category: String,
    pub title: String,
    pub body: String,
    pub image_url: Option<String>,
    /// Path in the app, e.g. `/chat/<id>`.
    pub path: String,
    /// The same destination as an absolute web URL (what a browser opens).
    pub url: String,
    /// Language the title and body are written in.
    pub locale: String,
    /// A newer push with the same key replaces this one on the device.
    pub collapse_key: Option<String>,
    /// The user's unread inbox entries, shown on the app icon.
    pub badge: i64,
    /// Extra structured data from the producing domain.
    pub data: serde_json::Value,
    pub priority: PushPriority,
    /// How long the push service keeps the message for an offline device.
    pub ttl_seconds: u32,
    pub sent_at: Time,
}

impl PushMessage {
    /// Seconds since the epoch after which the message is not worth delivering.
    pub fn expires_at(&self) -> i64 {
        self.sent_at.timestamp() + i64::from(self.ttl_seconds)
    }
}

/// What a push service did with a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushOutcome {
    /// Accepted for delivery.
    Delivered,
    /// The token will never work again (unsubscribed, uninstalled, expired):
    /// the registration must be dropped.
    Gone(String),
    /// A temporary failure (rate limit, outage, network): try again later.
    Retry(String),
    /// The push service refused this message for a reason a retry will not
    /// fix (bad credentials, payload too large).
    Rejected(String),
}

#[async_trait]
pub trait PushChannel: Send + Sync + 'static {
    fn kind(&self) -> PushChannelKind;

    /// Sends one message to one device. Never fails: every failure is an
    /// outcome the caller acts on.
    async fn send(&self, target: &PushTarget, message: &PushMessage) -> PushOutcome;
}

/// The HTTP client push channels share. Redirects are never followed: a push
/// service has no reason to redirect, and following one would let a crafted
/// Web Push endpoint steer requests somewhere else.
pub fn http_client() -> anyhow::Result<reqwest::Client> {
    builder()
        .build()
        .map_err(|e| anyhow::anyhow!("push http client: {e}"))
}

/// The HTTP client for Web Push: endpoints are chosen by clients, so it only
/// ever connects to public addresses (`tokuly_core::egress`, checked when
/// the name resolves and again on the connected peer). `allow_insecure`
/// (development and tests, never production) lets it reach local stand-ins.
pub fn web_push_client(allow_insecure: bool) -> anyhow::Result<reqwest::Client> {
    let guard = (!allow_insecure).then(egress::Policy::default);
    egress::client_builder(guard, std::time::Duration::from_secs(15))
        .user_agent("Tokuly/1.0")
        .build()
        .map_err(|e| anyhow::anyhow!("web push http client: {e}"))
}

fn builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent("Tokuly/1.0")
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(15))
}

/// Reads a push service's answer: status and the start of the body. The
/// body is only ever used for a small JSON document or an error message, and
/// the server on the other end is not ours, so it is read up to a bound.
pub(crate) async fn read_response(mut response: reqwest::Response) -> (u16, String) {
    const MAX_BODY_BYTES: usize = 64 * 1024;
    let status = response.status().as_u16();
    let body = egress::read_capped(&mut response, MAX_BODY_BYTES)
        .await
        .unwrap_or_default();
    (status, String::from_utf8_lossy(&body).into_owned())
}

/// Maps a transport-level failure to an outcome: the request may not have
/// reached the push service, so it is worth another attempt.
pub(crate) fn network_failure(error: &reqwest::Error) -> PushOutcome {
    // Deliberately not `error.to_string()`: it embeds the request URL, and Web
    // Push endpoints and device tokens are capabilities that stay out of logs.
    let what = if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect failed"
    } else {
        "request failed"
    };
    PushOutcome::Retry(format!("network: {what}"))
}

/// The token sent to push services for a collapse key (Web Push `Topic`, FCM
/// `collapse_key`, APNs `apns-collapse-id`). Services restrict its alphabet
/// and length (32 base64url characters is the tightest), and a digest keeps
/// our internal IDs out of third-party systems.
pub fn collapse_id(collapse_key: &str) -> String {
    let digest = Sha256::digest(collapse_key.as_bytes());
    b64url(digest.as_slice())[..32].to_owned()
}

/// How much of a message to leave out so that it fits a push service's size
/// limit (4096 bytes everywhere). A channel builds its payload at each level
/// in turn and sends the first one that fits: the producer's extra data goes
/// first, then the image, then the text gets shorter.
#[derive(Debug, Clone, Copy)]
pub struct Trim(u8);

impl Trim {
    const LAST_LEVEL: u8 = 4;

    /// Nothing left out.
    pub const NONE: Trim = Trim(0);

    pub fn keeps_extra_data(self) -> bool {
        self.0 < 1
    }

    pub fn keeps_image(self) -> bool {
        self.0 < 2
    }

    pub fn title(self, title: &str) -> String {
        truncate_chars(title, if self.0 < Self::LAST_LEVEL { 120 } else { 60 })
    }

    pub fn body(self, body: &str) -> String {
        let max_chars = match self.0 {
            0..=2 => 500,
            3 => 150,
            _ => 60,
        };
        truncate_chars(body, max_chars)
    }
}

/// Serializes the least-trimmed payload that fits `max_bytes`. If even the
/// most trimmed one is too large (absurdly long paths), it is returned anyway
/// and the push service's refusal is reported like any other.
pub fn fit_json(max_bytes: usize, build: impl Fn(Trim) -> serde_json::Value) -> Vec<u8> {
    let mut bytes = Vec::new();
    for level in 0..=Trim::LAST_LEVEL {
        bytes = serde_json::to_vec(&build(Trim(level))).expect("JSON values serialize");
        if bytes.len() <= max_bytes {
            break;
        }
    }
    bytes
}

/// Shortens `text` to at most `max_chars` characters, ending with an ellipsis
/// when something was cut.
pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
pub(crate) fn sample_message() -> PushMessage {
    PushMessage {
        notification_id: Some(Uuid::nil()),
        kind: "chat.message".into(),
        category: "chat".into(),
        title: "ありす".into(),
        body: "やっほー！".into(),
        image_url: Some("https://cdn.example/a.webp".into()),
        path: "/chat/42".into(),
        url: "https://tokuly.example/chat/42".into(),
        locale: "ja".into(),
        collapse_key: Some("chat:42".into()),
        badge: 3,
        data: serde_json::json!({ "conversation_id": "42" }),
        priority: PushPriority::High,
        ttl_seconds: 3600,
        sent_at: chrono::DateTime::from_timestamp(1_800_000_000, 0).expect("valid timestamp"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapse_ids_are_stable_short_and_url_safe() {
        let id = collapse_id("chat:0191f3a0-7c1e-7cc3-9d2b-6f1f0e2a9b11");
        assert_eq!(id, collapse_id("chat:0191f3a0-7c1e-7cc3-9d2b-6f1f0e2a9b11"));
        assert_ne!(id, collapse_id("chat:other"));
        assert_eq!(id.len(), 32);
        assert!(
            id.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
    }

    #[test]
    fn truncation_counts_characters_not_bytes() {
        assert_eq!(truncate_chars("こんにちは", 5), "こんにちは");
        assert_eq!(truncate_chars("こんにちは", 3), "こん…");
        assert_eq!(truncate_chars("", 3), "");
    }
}
