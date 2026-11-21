//! Shared helpers for the notify integration tests.
use std::sync::{Arc, LazyLock, Mutex};

use base64::Engine as _;
use chrono::{DateTime, Utc};
use serde::Serialize;
use tokuly_core::{
    events::Event,
    outbound::{Localized, NOTIFY_TOPIC, NotificationRequest},
};
use tokuly_notify::{
    Notify, NotifyConfig,
    consumer::{DropReason, PushDecision, PushSkip, RequestOutcome},
    delivery::PushJob,
    push::ece,
};
use tokuly_proto::notify::{mark_read_request::Target, *};
use tokuly_testkit::{TestApp, TestUser};
use uuid::Uuid;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0400", "1200"];

/// A running app plus the domain object, for calling handlers directly.
pub struct Ctx {
    pub app: TestApp,
    pub notify: Notify,
}

pub async fn start() -> Ctx {
    start_with(NotifyConfig::default()).await
}

/// Like [`start`], but Web Push endpoints may be plain `http://` on any host,
/// so a local server can stand in for a push service.
pub async fn start_insecure() -> Ctx {
    start_with(NotifyConfig {
        webpush_allow_insecure: true,
        ..Default::default()
    })
    .await
}

pub async fn start_with(cfg: NotifyConfig) -> Ctx {
    let mut holder = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        let notify = Notify::new(core.clone(), identity.clone(), cfg).expect("notify starts");
        let router = notify.register(router);
        holder = Some(notify);
        router
    })
    .await;
    Ctx {
        app,
        notify: holder.expect("register ran"),
    }
}

// ───────────────────────────── events ─────────────────────────────

pub fn event_at(topic: &str, data: impl Serialize, occurred_at: DateTime<Utc>) -> Event {
    Event {
        id: Uuid::now_v7(),
        topic: topic.to_owned(),
        actor_id: None,
        occurred_at,
        data: serde_json::to_value(data).expect("payload serializes"),
    }
}

pub fn event(topic: &str, data: impl Serialize) -> Event {
    event_at(topic, data, Utc::now())
}

/// A `notify.requested` event as the outbox relay would deliver it.
pub fn request_event(request: &NotificationRequest) -> Event {
    event(NOTIFY_TOPIC, request)
}

/// "`from` sent you a message": the notification most tests use.
pub fn chat_message(to: &TestUser, from: &TestUser) -> NotificationRequest {
    NotificationRequest::new(
        to.id,
        "chat.message",
        Localized::new("新着メッセージ", "New message"),
        Localized::new("やっほー！", "Hi there!"),
        "/chat/42",
    )
    .actor(from.id)
}

impl Ctx {
    /// Runs the consumer on a request and returns what it did.
    pub async fn deliver(&self, request: &NotificationRequest) -> RequestOutcome {
        self.notify
            .handle_request(&request_event(request))
            .await
            .expect("request handled")
    }

    /// Runs the consumer and expects an inbox entry; returns its ID.
    pub async fn deliver_to_inbox(&self, request: &NotificationRequest) -> String {
        match self.deliver(request).await {
            RequestOutcome::Handled {
                notification_id: Some(id),
                ..
            } => id.to_string(),
            other => panic!("expected an inbox entry, got {other:?}"),
        }
    }

    /// Runs the consumer and expects a queued push job.
    pub async fn deliver_for_push(&self, request: &NotificationRequest) -> PushJob {
        match self.deliver(request).await {
            RequestOutcome::Handled {
                push: PushDecision::Queued(job),
                ..
            } => job,
            other => panic!("expected a queued push, got {other:?}"),
        }
    }

    pub async fn inbox(&self, user: &TestUser) -> ListNotificationsResponse {
        self.app
            .call(
                "tokuly.notify.v1.InboxService/ListNotifications",
                Some(user),
                &ListNotificationsRequest::default(),
            )
            .await
            .unwrap()
    }

    pub async fn devices(&self, user: &TestUser) -> Vec<PushDevice> {
        let response: ListDevicesResponse = self
            .app
            .call(
                "tokuly.notify.v1.PushService/ListDevices",
                Some(user),
                &ListDevicesRequest::default(),
            )
            .await
            .unwrap();
        response.devices
    }

    pub async fn register(&self, user: &TestUser, request: &RegisterDeviceRequest) -> PushDevice {
        let response: RegisterDeviceResponse = self
            .app
            .call(
                "tokuly.notify.v1.PushService/RegisterDevice",
                Some(user),
                request,
            )
            .await
            .unwrap();
        response.device.into_option().expect("device returned")
    }

    /// Gives `user` a Web Push registration at a real-looking endpoint
    /// (nothing is ever sent to it) and returns it.
    pub async fn register_browser(&self, user: &TestUser) -> PushDevice {
        let subscription = Subscription::new(&format!(
            "https://fcm.googleapis.com/fcm/send/{}",
            Uuid::now_v7().simple()
        ));
        self.register(user, &subscription.request()).await
    }
}

pub fn unread_of(response: &ListNotificationsResponse) -> i32 {
    response
        .unread
        .as_option()
        .map(|u| u.total)
        .unwrap_or_default()
}

// ───────────────────────────── browser subscription ─────────────────────────────

/// What a browser holds after `PushManager.subscribe`: the endpoint and the
/// private half of the keys it handed to the server.
pub struct Subscription {
    pub endpoint: String,
    pub secret: p256::SecretKey,
    pub auth: [u8; 16],
}

impl Subscription {
    pub fn new(endpoint: &str) -> Self {
        let mut auth = [0u8; 16];
        auth.copy_from_slice(&Uuid::new_v4().into_bytes());
        Self {
            endpoint: endpoint.to_owned(),
            secret: ece::random_secret_key(),
            auth,
        }
    }

    /// The `RegisterDevice` request a browser would send.
    pub fn request(&self) -> RegisterDeviceRequest {
        let subscription = WebPushSubscription {
            endpoint: self.endpoint.clone(),
            p256dh: tokuly_core::keys::b64url(&ece::public_key_bytes(&self.secret)),
            auth: tokuly_core::keys::b64url(&self.auth),
            ..Default::default()
        };
        RegisterDeviceRequest {
            token: Some(register_device_request::Token::from(subscription)),
            ..Default::default()
        }
    }

    /// Decrypts a push message body the way the browser would.
    pub fn decrypt(&self, body: &[u8]) -> serde_json::Value {
        let plaintext = ece::decrypt(body, &self.secret, &self.auth)
            .expect("the message decrypts with the subscription's private key");
        serde_json::from_slice(&plaintext).expect("the plaintext is JSON")
    }
}

// ───────────────────────────── local push service ─────────────────────────────

#[derive(Debug, Clone)]
pub struct Captured {
    pub path: String,
    pub headers: http::HeaderMap,
    pub body: Vec<u8>,
}

impl Captured {
    pub fn header(&self, name: &str) -> &str {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
    }
}

/// A local HTTP server that records every request and answers with whatever
/// the test scripted: a stand-in for a push service.
pub struct Stub {
    pub url: String,
    requests: Arc<Mutex<Vec<Captured>>>,
}

impl Stub {
    /// `respond` maps a request to `(status, body)`.
    pub async fn start(
        respond: impl Fn(&Captured) -> (u16, String) + Send + Sync + 'static,
    ) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a local port");
        let url = format!("http://{}", listener.local_addr().expect("local address"));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let respond = Arc::new(respond);
        let recorded = requests.clone();
        let router = axum::Router::new().fallback(move |request: axum::extract::Request| {
            let (recorded, respond) = (recorded.clone(), respond.clone());
            async move {
                let (parts, body) = request.into_parts();
                let body = axum::body::to_bytes(body, 1 << 20)
                    .await
                    .expect("request body")
                    .to_vec();
                let captured = Captured {
                    path: parts.uri.path().to_owned(),
                    headers: parts.headers,
                    body,
                };
                let (status, text) = respond(&captured);
                recorded.lock().unwrap().push(captured);
                (
                    http::StatusCode::from_u16(status).expect("valid status"),
                    text,
                )
            }
        });
        tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("stub server runs");
        });
        Self { url, requests }
    }

    /// A push service that accepts everything.
    pub async fn accepting() -> Self {
        Self::start(|_| (201, String::new())).await
    }

    pub fn requests(&self) -> Vec<Captured> {
        self.requests.lock().unwrap().clone()
    }

    pub fn requests_to(&self, path: &str) -> Vec<Captured> {
        self.requests()
            .into_iter()
            .filter(|r| r.path == path)
            .collect()
    }
}

// ───────────────────────────── throwaway credentials ─────────────────────────────

fn pem(label: &str, der: &[u8]) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(der);
    let lines: Vec<&str> = encoded
        .as_bytes()
        .chunks(64)
        .map(|chunk| std::str::from_utf8(chunk).expect("base64 is ASCII"))
        .collect();
    format!(
        "-----BEGIN {label}-----\n{}\n-----END {label}-----\n",
        lines.join("\n")
    )
}

pub struct RsaKey {
    /// PKCS#8, as found in a service-account file.
    pub private_pem: String,
    pub public_pem: String,
}

/// An RSA key pair made for this test run (nothing secret is checked in).
pub static RSA_KEY: LazyLock<RsaKey> = LazyLock::new(|| {
    use aws_lc_rs::{
        encoding::{AsDer, Pkcs8V1Der, PublicKeyX509Der},
        rsa::{KeyPair, KeySize},
        signature::KeyPair as _,
    };
    let key = KeyPair::generate(KeySize::Rsa2048).expect("RSA key generation");
    let private: Pkcs8V1Der = key.as_der().expect("PKCS#8 encoding");
    let public: PublicKeyX509Der = key.public_key().as_der().expect("SPKI encoding");
    RsaKey {
        private_pem: pem("PRIVATE KEY", private.as_ref()),
        public_pem: pem("PUBLIC KEY", public.as_ref()),
    }
});

/// The content of a Firebase service-account file for the test key.
pub fn fcm_service_account(token_uri: &str) -> String {
    serde_json::json!({
        "type": "service_account",
        "project_id": "tokuly-test",
        "private_key_id": "key-1",
        "private_key": RSA_KEY.private_pem,
        "client_email": "push@tokuly-test.iam.gserviceaccount.com",
        "token_uri": token_uri,
    })
    .to_string()
}

pub struct EcKey {
    pub secret: p256::SecretKey,
    /// PKCS#8 PEM, the format of an APNs `.p8` file.
    pub p8: String,
}

pub static APNS_KEY: LazyLock<EcKey> = LazyLock::new(|| {
    use p256::pkcs8::EncodePrivateKey as _;
    let secret = ece::random_secret_key();
    let p8 = pem(
        "PRIVATE KEY",
        secret.to_pkcs8_der().expect("PKCS#8 encoding").as_bytes(),
    );
    EcKey { secret, p8 }
});

/// A decoding key for ES256 tokens signed by `secret`.
pub fn es256_decoding_key(public_key: &[u8]) -> jsonwebtoken::DecodingKey {
    assert_eq!(
        (public_key.len(), public_key[0]),
        (65, 0x04),
        "uncompressed P-256 point"
    );
    let b64 = tokuly_core::keys::b64url;
    jsonwebtoken::DecodingKey::from_ec_components(&b64(&public_key[1..33]), &b64(&public_key[33..]))
        .expect("valid EC coordinates")
}

// ───────────────────────────── consumer outcomes and preferences ─────────────────────────────

pub async fn update_preferences(ctx: &Ctx, user: &TestUser, request: UpdatePreferencesRequest) {
    let _: UpdatePreferencesResponse = ctx
        .app
        .call(
            "tokuly.notify.v1.PreferencesService/UpdatePreferences",
            Some(user),
            &request,
        )
        .await
        .unwrap();
}

pub fn toggle(category: &str, inbox: Option<bool>, push: Option<bool>) -> UpdatePreferencesRequest {
    UpdatePreferencesRequest {
        categories: vec![CategoryChange {
            category: category.into(),
            inbox,
            push,
            ..Default::default()
        }],
        ..Default::default()
    }
}

pub fn skipped(outcome: &RequestOutcome) -> Option<PushSkip> {
    match outcome {
        RequestOutcome::Handled {
            push: PushDecision::Skipped(skip),
            ..
        } => Some(*skip),
        _ => None,
    }
}

pub fn dropped(outcome: &RequestOutcome) -> Option<DropReason> {
    match outcome {
        RequestOutcome::Dropped(reason) => Some(*reason),
        _ => None,
    }
}

// ───────────────────────────── inbox ─────────────────────────────

pub const INBOX_LIST: &str = "tokuly.notify.v1.InboxService/ListNotifications";
pub const INBOX_UNREAD: &str = "tokuly.notify.v1.InboxService/GetUnreadCount";
pub const INBOX_MARK_READ: &str = "tokuly.notify.v1.InboxService/MarkRead";
pub const INBOX_DELETE: &str = "tokuly.notify.v1.InboxService/DeleteNotifications";
pub const INBOX_CLEAR: &str = "tokuly.notify.v1.InboxService/ClearNotifications";

/// A notification of `kind` for `to`, without an actor.
pub fn notice(to: &TestUser, kind: &str, title: &str) -> NotificationRequest {
    NotificationRequest::new(
        to.id,
        kind,
        Localized::same(title),
        Localized::same("…"),
        "/somewhere",
    )
}

/// Fills an inbox with `titles`, oldest first, one second apart, and returns the entry IDs.
pub async fn fill(ctx: &Ctx, user: &TestUser, entries: &[(&str, &str)]) -> Vec<String> {
    let start = Utc::now() - chrono::Duration::seconds(entries.len() as i64);
    let mut ids = Vec::new();
    for (index, (kind, title)) in entries.iter().enumerate() {
        let event = event_at(
            NOTIFY_TOPIC,
            notice(user, kind, title),
            start + chrono::Duration::seconds(index as i64),
        );
        match ctx.notify.handle_request(&event).await.unwrap() {
            RequestOutcome::Handled {
                notification_id: Some(id),
                ..
            } => ids.push(id.to_string()),
            other => panic!("{other:?}"),
        }
    }
    ids
}

pub fn titles(response: &ListNotificationsResponse) -> Vec<&str> {
    response
        .notifications
        .iter()
        .map(|n| n.title.as_str())
        .collect()
}

pub async fn unread(ctx: &Ctx, user: &TestUser) -> UnreadCount {
    let response: GetUnreadCountResponse = ctx
        .app
        .call(INBOX_UNREAD, Some(user), &GetUnreadCountRequest::default())
        .await
        .unwrap();
    response.unread.into_option().unwrap()
}

pub async fn mark_read(
    ctx: &Ctx,
    user: &TestUser,
    target: Target,
    category: &str,
) -> MarkReadResponse {
    ctx.app
        .call(
            INBOX_MARK_READ,
            Some(user),
            &MarkReadRequest {
                target: Some(target),
                category: category.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
}

pub fn ids(ids: &[&String]) -> Target {
    Target::from(NotificationIds {
        ids: ids.iter().map(|id| id.to_string()).collect(),
        ..Default::default()
    })
}
