//! FCM and APNs against local stand-ins: token minting and the wire format.
//!
//! These prove that the requests match the documented formats; they cannot
//! prove that Google and Apple accept them. Neither channel has been run
//! against the real service.
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use tokuly_notify::push::{
    PushChannel as _, PushMessage, PushOutcome, PushPriority, PushTarget,
    apns::{Apns, ProviderClaims},
    collapse_id,
    ece::public_key_bytes,
    fcm::{AssertionClaims, Fcm},
    http_client,
};
use tokuly_proto::notify::PushChannelKind;
use uuid::Uuid;

use crate::common::*;

fn message() -> PushMessage {
    PushMessage {
        notification_id: Some(Uuid::now_v7()),
        kind: "chat.message".into(),
        category: "chat".into(),
        title: "ありす".into(),
        body: "やっほー！".into(),
        image_url: None,
        path: "/chat/42".into(),
        url: "https://tokuly.example/chat/42".into(),
        locale: "ja".into(),
        collapse_key: Some("chat:42".into()),
        badge: 2,
        data: serde_json::json!({ "conversation_id": "42" }),
        priority: PushPriority::High,
        ttl_seconds: 3600,
        sent_at: chrono::Utc::now(),
    }
}

fn target(token: &str, apns_sandbox: bool) -> PushTarget {
    PushTarget {
        token: token.to_owned(),
        p256dh: None,
        auth: None,
        apns_sandbox,
    }
}

fn form_field(body: &[u8], name: &str) -> String {
    url::form_urlencoded::parse(body)
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .unwrap_or_else(|| panic!("form field {name}"))
}

// ───────────────────────────── FCM ─────────────────────────────

/// A stand-in for Google: the OAuth token endpoint and `messages:send`.
async fn google(
    send_response: impl Fn(&Captured) -> (u16, String) + Send + Sync + 'static,
) -> Stub {
    Stub::start(move |request| match request.path.as_str() {
        "/token" => (200, serde_json::json!({ "access_token": "ya29.test-token", "expires_in": 3599, "token_type": "Bearer" }).to_string()),
        _ => send_response(request),
    })
    .await
}

fn fcm(stub: &Stub) -> Fcm {
    Fcm::from_service_account(
        &fcm_service_account(&format!("{}/token", stub.url)),
        http_client().unwrap(),
    )
    .unwrap()
    .with_endpoint(&stub.url)
}

#[tokio::test]
async fn fcm_exchanges_a_signed_assertion_for_an_access_token_and_sends() {
    let stub = google(|_| (200, r#"{"name":"projects/tokuly-test/messages/1"}"#.into())).await;
    let fcm = fcm(&stub);
    assert_eq!(fcm.kind(), PushChannelKind::PUSH_CHANNEL_KIND_FCM);
    let message = message();

    assert_eq!(
        fcm.send(&target("device-token-1", false), &message).await,
        PushOutcome::Delivered
    );

    // 1. The OAuth exchange (RFC 7523): a JWT signed with the service account's key.
    let token_requests = stub.requests_to("/token");
    assert_eq!(token_requests.len(), 1);
    assert!(
        token_requests[0]
            .header("content-type")
            .starts_with("application/x-www-form-urlencoded")
    );
    assert_eq!(
        form_field(&token_requests[0].body, "grant_type"),
        "urn:ietf:params:oauth:grant-type:jwt-bearer"
    );
    let assertion = form_field(&token_requests[0].body, "assertion");
    let header = jsonwebtoken::decode_header(&assertion).unwrap();
    assert_eq!(
        (header.alg, header.kid.as_deref()),
        (Algorithm::RS256, Some("key-1"))
    );
    let token_uri = format!("{}/token", stub.url);
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[token_uri.as_str()]);
    let key = DecodingKey::from_rsa_pem(RSA_KEY.public_pem.as_bytes()).unwrap();
    let claims = jsonwebtoken::decode::<AssertionClaims>(&assertion, &key, &validation)
        .expect("verifies with the account's public key")
        .claims;
    assert_eq!(claims.iss, "push@tokuly-test.iam.gserviceaccount.com");
    assert_eq!(
        claims.scope,
        "https://www.googleapis.com/auth/firebase.messaging"
    );
    assert_eq!(claims.exp - claims.iat, 3600);

    // 2. The send request (HTTP v1).
    let sends = stub.requests_to("/v1/projects/tokuly-test/messages:send");
    assert_eq!(sends.len(), 1);
    assert_eq!(sends[0].header("authorization"), "Bearer ya29.test-token");
    assert_eq!(sends[0].header("content-type"), "application/json");
    let body: serde_json::Value = serde_json::from_slice(&sends[0].body).unwrap();
    let sent = &body["message"];
    assert_eq!(sent["token"], "device-token-1");
    assert_eq!(
        sent["notification"],
        serde_json::json!({ "title": "ありす", "body": "やっほー！" })
    );
    assert_eq!(sent["android"]["collapse_key"], collapse_id("chat:42"));
    assert_eq!(sent["android"]["priority"], "HIGH");
    assert_eq!(sent["android"]["ttl"], "3600s");
    assert_eq!(sent["android"]["notification"]["notification_count"], 2);
    assert_eq!(sent["data"]["path"], "/chat/42");
    assert_eq!(
        sent["data"]["notification_id"],
        message.notification_id.unwrap().to_string()
    );

    assert!(
        sent["android"]["notification"]
            .get("notification_priority")
            .is_none()
    );

    // The access token is reused for the next message: an urgent one, which
    // asks for the highest attention on Android and (through Firebase) iOS.
    let urgent = PushMessage {
        priority: PushPriority::Urgent,
        ..message
    };
    assert_eq!(
        fcm.send(&target("device-token-2", false), &urgent).await,
        PushOutcome::Delivered
    );
    assert_eq!(stub.requests_to("/token").len(), 1);
    let sends = stub.requests_to("/v1/projects/tokuly-test/messages:send");
    assert_eq!(sends.len(), 2);
    let body: serde_json::Value = serde_json::from_slice(&sends[1].body).unwrap();
    let sent = &body["message"];
    assert_eq!(sent["android"]["priority"], "HIGH");
    assert_eq!(
        sent["android"]["notification"]["notification_priority"],
        "PRIORITY_MAX"
    );
    assert_eq!(
        sent["apns"]["payload"]["aps"]["interruption-level"],
        "time-sensitive"
    );
    assert_eq!(sent["apns"]["headers"]["apns-priority"], "10");
    assert_eq!(sent["data"]["urgent"], "true");
}

#[tokio::test]
async fn fcm_errors_become_outcomes() {
    let error = |status: &str, code: &str| {
        serde_json::json!({ "error": { "status": status, "details": [{ "@type": "type.googleapis.com/google.firebase.fcm.v1.FcmError", "errorCode": code }] } }).to_string()
    };
    let stub = google(move |request| {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap_or_default();
        match body["message"]["token"].as_str().unwrap_or_default() {
            "uninstalled" => (404, error("NOT_FOUND", "UNREGISTERED")),
            "busy" => (503, error("UNAVAILABLE", "UNAVAILABLE")),
            "malformed" => (400, error("INVALID_ARGUMENT", "INVALID_ARGUMENT")),
            _ => (200, "{}".into()),
        }
    })
    .await;
    let fcm = fcm(&stub);
    let message = message();
    assert!(matches!(
        fcm.send(&target("uninstalled", false), &message).await,
        PushOutcome::Gone(_)
    ));
    assert!(matches!(
        fcm.send(&target("busy", false), &message).await,
        PushOutcome::Retry(_)
    ));
    assert!(matches!(
        fcm.send(&target("malformed", false), &message).await,
        PushOutcome::Rejected(_)
    ));
    assert_eq!(
        fcm.send(&target("fine", false), &message).await,
        PushOutcome::Delivered
    );
}

#[tokio::test]
async fn fcm_refetches_the_access_token_after_a_401_and_reports_auth_failures() {
    // The send endpoint refuses the token once.
    let refused = std::sync::atomic::AtomicBool::new(false);
    let stub = google(move |_| {
        if refused.swap(true, std::sync::atomic::Ordering::SeqCst) {
            (200, "{}".into())
        } else {
            (401, r#"{"error":{"status":"UNAUTHENTICATED"}}"#.into())
        }
    })
    .await;
    let fcm = fcm(&stub);
    assert!(matches!(
        fcm.send(&target("t", false), &message()).await,
        PushOutcome::Retry(_)
    ));
    assert_eq!(
        fcm.send(&target("t", false), &message()).await,
        PushOutcome::Delivered
    );
    assert_eq!(
        stub.requests_to("/token").len(),
        2,
        "the refused token was thrown away"
    );

    // Google rejecting our credentials is not something a retry fixes; an outage is.
    let denied = Stub::start(|_| (400, r#"{"error":"invalid_grant"}"#.into())).await;
    let outcome = Fcm::from_service_account(
        &fcm_service_account(&format!("{}/token", denied.url)),
        http_client().unwrap(),
    )
    .unwrap()
    .with_endpoint(&denied.url)
    .send(&target("t", false), &message())
    .await;
    assert!(matches!(outcome, PushOutcome::Rejected(_)), "{outcome:?}");
    let down = Stub::start(|_| (503, String::new())).await;
    let outcome = Fcm::from_service_account(
        &fcm_service_account(&format!("{}/token", down.url)),
        http_client().unwrap(),
    )
    .unwrap()
    .with_endpoint(&down.url)
    .send(&target("t", false), &message())
    .await;
    assert!(matches!(outcome, PushOutcome::Retry(_)), "{outcome:?}");
    assert!(
        denied
            .requests()
            .iter()
            .chain(down.requests().iter())
            .all(|r| r.path == "/token"),
        "nothing is sent without a token"
    );
}

// ───────────────────────────── APNs ─────────────────────────────

const KEY_ID: &str = "KEY1234567";
const TEAM_ID: &str = "TEAM123456";
const TOPIC: &str = "app.tokuly.ios";

fn apns(production: &Stub, sandbox: &Stub) -> Apns {
    Apns::new(&APNS_KEY.p8, KEY_ID, TEAM_ID, TOPIC, http_client().unwrap())
        .unwrap()
        .with_endpoints(&production.url, &sandbox.url)
}

#[tokio::test]
async fn apns_sends_with_a_provider_token_and_delivery_headers() {
    let production = Stub::start(|_| (200, String::new())).await;
    let sandbox = Stub::start(|_| (200, String::new())).await;
    let apns = apns(&production, &sandbox);
    assert_eq!(apns.kind(), PushChannelKind::PUSH_CHANNEL_KIND_APNS);
    let message = message();
    let device_token = "ab".repeat(32);

    assert_eq!(
        apns.send(&target(&device_token, false), &message).await,
        PushOutcome::Delivered
    );
    assert!(
        sandbox.requests().is_empty(),
        "production tokens go to the production host"
    );
    let requests = production.requests();
    assert_eq!(requests.len(), 1);
    let sent = &requests[0];
    assert_eq!(sent.path, format!("/3/device/{device_token}"));

    // The provider token: ES256, `kid` = key ID, `iss` = team ID, issued just now.
    let token = sent
        .header("authorization")
        .strip_prefix("bearer ")
        .expect("bearer scheme");
    let header = jsonwebtoken::decode_header(token).unwrap();
    assert_eq!(
        (header.alg, header.kid.as_deref(), header.typ),
        (Algorithm::ES256, Some(KEY_ID), None)
    );
    let mut validation = Validation::new(Algorithm::ES256);
    validation.set_required_spec_claims::<&str>(&[]);
    validation.validate_exp = false; // provider tokens carry `iat` only
    let claims = jsonwebtoken::decode::<ProviderClaims>(
        token,
        &es256_decoding_key(&public_key_bytes(&APNS_KEY.secret)),
        &validation,
    )
    .expect("verifies with the .p8 key")
    .claims;
    assert_eq!(claims.iss, TEAM_ID);
    assert!((chrono::Utc::now().timestamp() - claims.iat).abs() <= 5);

    assert_eq!(sent.header("apns-topic"), TOPIC);
    assert_eq!(sent.header("apns-push-type"), "alert");
    assert_eq!(sent.header("apns-priority"), "10");
    assert_eq!(sent.header("apns-collapse-id"), collapse_id("chat:42"));
    assert_eq!(
        sent.header("apns-expiration"),
        (message.sent_at.timestamp() + 3600).to_string()
    );
    let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
    assert_eq!(
        body["aps"]["alert"],
        serde_json::json!({ "title": "ありす", "body": "やっほー！" })
    );
    assert_eq!(
        (body["aps"]["badge"].as_i64(), body["aps"]["sound"].as_str()),
        (Some(2), Some("default"))
    );
    assert_eq!(body["path"], "/chat/42");

    // Development builds are reached through the sandbox, with the same (reused) provider token.
    assert_eq!(
        apns.send(&target(&device_token, true), &message).await,
        PushOutcome::Delivered
    );
    let sandboxed = sandbox.requests();
    assert_eq!(sandboxed.len(), 1);
    assert_eq!(
        sandboxed[0].header("authorization"),
        sent.header("authorization"),
        "Apple throttles servers that mint a token per request"
    );
    assert_eq!(apns.provider_token().unwrap(), token);

    // A notification that can wait, without a collapse key.
    let relaxed = PushMessage {
        priority: PushPriority::Normal,
        collapse_key: None,
        ..message.clone()
    };
    apns.send(&target(&device_token, false), &relaxed).await;
    let last = production.requests().pop().unwrap();
    assert_eq!(last.header("apns-priority"), "5");
    assert!(last.headers.get("apns-collapse-id").is_none());
    let body: serde_json::Value = serde_json::from_slice(&last.body).unwrap();
    assert!(body["aps"].get("interruption-level").is_none());

    // An urgent one (incoming call, SOS): immediate and time-sensitive.
    let urgent = PushMessage {
        priority: PushPriority::Urgent,
        ..message
    };
    apns.send(&target(&device_token, false), &urgent).await;
    let last = production.requests().pop().unwrap();
    assert_eq!(last.header("apns-priority"), "10");
    let body: serde_json::Value = serde_json::from_slice(&last.body).unwrap();
    assert_eq!(body["aps"]["interruption-level"], "time-sensitive");
    assert_eq!(body["urgent"], true);
}

#[tokio::test]
async fn apns_errors_become_outcomes() {
    let stub = Stub::start(
        |request| match request.path.rsplit('/').next().unwrap_or_default() {
            "removed" => (
                410,
                r#"{"reason":"Unregistered","timestamp":1700000000000}"#.into(),
            ),
            "foreign" => (400, r#"{"reason":"BadDeviceToken"}"#.into()),
            "busy" => (503, r#"{"reason":"ServiceUnavailable"}"#.into()),
            "stale-auth" => (403, r#"{"reason":"ExpiredProviderToken"}"#.into()),
            "bad-auth" => (403, r#"{"reason":"InvalidProviderToken"}"#.into()),
            _ => (200, String::new()),
        },
    )
    .await;
    let apns = apns(&stub, &stub);
    let message = message();
    assert!(matches!(
        apns.send(&target("removed", false), &message).await,
        PushOutcome::Gone(_)
    ));
    assert!(matches!(
        apns.send(&target("foreign", false), &message).await,
        PushOutcome::Gone(_)
    ));
    assert!(matches!(
        apns.send(&target("busy", false), &message).await,
        PushOutcome::Retry(_)
    ));
    assert!(matches!(
        apns.send(&target("bad-auth", false), &message).await,
        PushOutcome::Rejected(_)
    ));

    // An expired provider token is replaced before the retry.
    let before = apns.provider_token().unwrap();
    assert_eq!(apns.provider_token().unwrap(), before, "cached while valid");
    assert!(matches!(
        apns.send(&target("stale-auth", false), &message).await,
        PushOutcome::Retry(_)
    ));
    assert_ne!(
        apns.provider_token().unwrap(),
        before,
        "a 403 discards the cached provider token"
    );
}

#[tokio::test]
async fn a_dead_host_is_a_retry_for_both_channels() {
    let closed = {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    };
    let message = message();
    let apns = Apns::new(&APNS_KEY.p8, KEY_ID, TEAM_ID, TOPIC, http_client().unwrap())
        .unwrap()
        .with_endpoints(&closed, &closed);
    assert!(matches!(
        apns.send(&target(&"ab".repeat(32), false), &message).await,
        PushOutcome::Retry(_)
    ));
    let fcm = Fcm::from_service_account(
        &fcm_service_account(&format!("{closed}/token")),
        http_client().unwrap(),
    )
    .unwrap()
    .with_endpoint(&closed);
    assert!(matches!(
        fcm.send(&target("t", false), &message).await,
        PushOutcome::Retry(_)
    ));
}
