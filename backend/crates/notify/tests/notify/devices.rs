//! `PushService` RPCs: push configuration and device registrations.
use tokuly_notify::{NotifyConfig, TEST_KIND, delivery::PUSH_JOB};
use tokuly_proto::{
    common::Platform,
    notify::{register_device_request::Token, *},
};
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

use crate::common::*;

const CONFIG: &str = "tokuly.notify.v1.PushService/GetPushConfig";
const REGISTER: &str = "tokuly.notify.v1.PushService/RegisterDevice";
const UNREGISTER: &str = "tokuly.notify.v1.PushService/UnregisterDevice";
const LIST: &str = "tokuly.notify.v1.PushService/ListDevices";
const TEST_PUSH: &str = "tokuly.notify.v1.PushService/SendTestPush";

fn endpoint() -> String {
    format!("https://updates.push.services.mozilla.com/wpush/v2/{}", Uuid::now_v7().simple())
}

async fn try_register(ctx: &Ctx, user: &TestUser, request: &RegisterDeviceRequest) -> Result<PushDevice, RpcError> {
    ctx.app
        .call::<_, RegisterDeviceResponse>(REGISTER, Some(user), request)
        .await
        .map(|r| r.device.into_option().unwrap())
}

async fn unregister(ctx: &Ctx, user: &TestUser, device_id: &str) -> Result<UnregisterDeviceResponse, RpcError> {
    ctx.app
        .call(
            UNREGISTER,
            Some(user),
            &UnregisterDeviceRequest { device_id: device_id.to_owned(), ..Default::default() },
        )
        .await
}

fn fcm_token(token: &str) -> RegisterDeviceRequest {
    RegisterDeviceRequest { token: Some(Token::FcmToken(token.into())), ..Default::default() }
}

fn apns_token(token: &str, sandbox: bool) -> RegisterDeviceRequest {
    RegisterDeviceRequest {
        token: Some(Token::from(ApnsToken { token: token.into(), sandbox, ..Default::default() })),
        ..Default::default()
    }
}

/// A config with FCM and APNs credentials (throwaway keys).
fn all_channels() -> NotifyConfig {
    NotifyConfig {
        fcm_service_account_json: fcm_service_account("https://oauth2.googleapis.com/token"),
        apns_key_p8: APNS_KEY.p8.replace('\n', "\\n"), // as it would sit in an env file
        apns_key_id: "KEY1234567".into(),
        apns_team_id: "TEAM123456".into(),
        apns_topic: "app.tokuly.ios".into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn push_config_is_public_and_lists_configured_channels() {
    let ctx = start().await;
    let config: GetPushConfigResponse = ctx.app.call(CONFIG, None, &GetPushConfigRequest::default()).await.unwrap();
    let key = tokuly_core::keys::b64url_decode(&config.vapid_public_key).expect("base64url");
    assert_eq!((key.len(), key[0]), (65, 0x04), "an uncompressed P-256 point, as PushManager.subscribe expects");
    assert_eq!(
        config.channels.iter().map(|c| c.as_known().unwrap()).collect::<Vec<_>>(),
        [PushChannelKind::PUSH_CHANNEL_KIND_WEB_PUSH]
    );

    // The key is derived from the master secret: every instance serves the same one.
    let again: GetPushConfigResponse =
        start().await.app.call(CONFIG, None, &GetPushConfigRequest::default()).await.unwrap();
    assert_eq!(again.vapid_public_key, config.vapid_public_key);

    let ctx = start_with(all_channels()).await;
    let config: GetPushConfigResponse = ctx.app.call(CONFIG, None, &GetPushConfigRequest::default()).await.unwrap();
    assert_eq!(
        config.channels.iter().map(|c| c.as_known().unwrap()).collect::<Vec<_>>(),
        [
            PushChannelKind::PUSH_CHANNEL_KIND_WEB_PUSH,
            PushChannelKind::PUSH_CHANNEL_KIND_FCM,
            PushChannelKind::PUSH_CHANNEL_KIND_APNS
        ]
    );
}

#[tokio::test]
async fn a_configured_vapid_key_overrides_the_derived_one() {
    let secret = tokuly_notify::push::ece::random_secret_key();
    let cfg = NotifyConfig { vapid_private_key: tokuly_core::keys::b64url(&secret.to_bytes()), ..Default::default() };
    let ctx = start_with(cfg).await;
    let config: GetPushConfigResponse = ctx.app.call(CONFIG, None, &GetPushConfigRequest::default()).await.unwrap();
    assert_eq!(
        config.vapid_public_key,
        tokuly_core::keys::b64url(&tokuly_notify::push::ece::public_key_bytes(&secret))
    );
}

#[tokio::test]
async fn device_rpcs_require_sign_in() {
    let ctx = start().await;
    let app = &ctx.app;
    let errors = [
        app.call::<_, RegisterDeviceResponse>(REGISTER, None, &Subscription::new(&endpoint()).request())
            .await
            .unwrap_err(),
        app.call::<_, UnregisterDeviceResponse>(UNREGISTER, None, &UnregisterDeviceRequest::default())
            .await
            .unwrap_err(),
        app.call::<_, ListDevicesResponse>(LIST, None, &ListDevicesRequest::default()).await.unwrap_err(),
        app.call::<_, SendTestPushResponse>(TEST_PUSH, None, &SendTestPushRequest::default()).await.unwrap_err(),
    ];
    for err in errors {
        assert_eq!((err.status, err.reason.as_str()), (401, "SIGN_IN_REQUIRED"));
    }
}

#[tokio::test]
async fn a_browser_registers_its_subscription() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let subscription = Subscription::new(&endpoint());
    let request = RegisterDeviceRequest {
        locale: "en-US".into(),
        device_name: "Firefox on Linux".into(),
        ..subscription.request()
    };

    let device = ctx.register(&alice, &request).await;
    assert_eq!(device.channel.as_known(), Some(PushChannelKind::PUSH_CHANNEL_KIND_WEB_PUSH));
    assert_eq!(device.platform.as_known(), Some(Platform::PLATFORM_WEB), "the platform defaults by channel");
    assert_eq!((device.locale.as_str(), device.device_name.as_str()), ("en-US", "Firefox on Linux"));
    assert!(
        device.current && device.created_at.as_option().is_some() && device.last_delivered_at.as_option().is_none()
    );

    // Registering the same subscription again (every page load) refreshes it in place.
    let again = ctx.register(&alice, &RegisterDeviceRequest { locale: "ja".into(), ..subscription.request() }).await;
    assert_eq!(again.id, device.id);
    let devices = ctx.devices(&alice).await;
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].locale, "ja");
}

#[tokio::test]
async fn a_session_holds_one_registration_per_channel() {
    let ctx = start_with(all_channels()).await;
    let alice = ctx.app.user("alice").await;

    // The push service rotated the subscription: the new one replaces the old.
    let first = ctx.register(&alice, &Subscription::new(&endpoint()).request()).await;
    let second = ctx.register(&alice, &Subscription::new(&endpoint()).request()).await;
    assert_ne!(first.id, second.id);
    assert_eq!(ctx.devices(&alice).await.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), [second.id.as_str()]);

    // Another channel on the same session sits next to it.
    let fcm = ctx.register(&alice, &fcm_token("fcm-token-1")).await;
    assert_eq!(
        (fcm.channel.as_known(), fcm.platform.as_known()),
        (Some(PushChannelKind::PUSH_CHANNEL_KIND_FCM), Some(Platform::PLATFORM_ANDROID))
    );
    assert_eq!(ctx.devices(&alice).await.len(), 2);

    // A second session of the same user has its own registrations.
    let phone = ctx.app.sign_in(alice.id).await;
    let apns = ctx.register(&phone, &apns_token(&"AB12".repeat(16), true)).await;
    assert_eq!(
        (apns.channel.as_known(), apns.platform.as_known()),
        (Some(PushChannelKind::PUSH_CHANNEL_KIND_APNS), Some(Platform::PLATFORM_IOS))
    );
    let from_phone = ctx.devices(&phone).await;
    assert_eq!(from_phone.len(), 3);
    assert_eq!(
        from_phone.iter().filter(|d| d.current).map(|d| d.id.as_str()).collect::<Vec<_>>(),
        [apns.id.as_str()],
        "`current` marks the caller's session"
    );
}

#[tokio::test]
async fn a_token_follows_the_account_that_last_registered_it() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    // Alice signs out without unregistering and Bob signs in on the same browser.
    let subscription = Subscription::new(&endpoint());
    ctx.register(&alice, &subscription.request()).await;
    let device = ctx.register(&bob, &subscription.request()).await;

    assert!(ctx.devices(&alice).await.is_empty(), "Alice's notifications must not reach Bob's browser");
    assert_eq!(ctx.devices(&bob).await.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), [device.id.as_str()]);
}

#[tokio::test]
async fn registrations_are_validated() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let valid = Subscription::new(&endpoint());
    let web_push = |endpoint: &str, p256dh: &str, auth: &str| RegisterDeviceRequest {
        token: Some(Token::from(WebPushSubscription {
            endpoint: endpoint.into(),
            p256dh: p256dh.into(),
            auth: auth.into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    let Some(Token::WebPush(keys)) = valid.request().token else { unreachable!() };

    let cases = [
        ("TOKEN_REQUIRED", RegisterDeviceRequest::default()),
        ("ENDPOINT_INVALID", web_push("not a url", &keys.p256dh, &keys.auth)),
        // The server sends requests to this URL: only real push services qualify.
        ("ENDPOINT_NOT_ALLOWED", web_push("https://attacker.example/collect", &keys.p256dh, &keys.auth)),
        ("ENDPOINT_NOT_ALLOWED", web_push("http://127.0.0.1:8080/internal", &keys.p256dh, &keys.auth)),
        ("ENDPOINT_NOT_ALLOWED", web_push("https://169.254.169.254/latest/meta-data", &keys.p256dh, &keys.auth)),
        ("SUBSCRIPTION_KEYS_INVALID", web_push(&valid.endpoint, "AAAA", &keys.auth)),
        ("SUBSCRIPTION_KEYS_INVALID", web_push(&valid.endpoint, &keys.p256dh, "AAAA")),
        ("LOCALE_INVALID", RegisterDeviceRequest { locale: "not a locale".into(), ..valid.request() }),
        ("DEVICE_NAME_TOO_LONG", RegisterDeviceRequest { device_name: "x".repeat(101), ..valid.request() }),
    ];
    for (reason, request) in cases {
        let err = try_register(&ctx, &alice, &request).await.unwrap_err();
        assert_eq!((err.code.as_str(), err.reason.as_str()), ("invalid_argument", reason));
    }
    // Channels without credentials are not offered.
    for request in [fcm_token("fcm-token"), apns_token(&"ab".repeat(32), false)] {
        let err = try_register(&ctx, &alice, &request).await.unwrap_err();
        assert_eq!((err.code.as_str(), err.reason.as_str()), ("unimplemented", "PUSH_CHANNEL_UNAVAILABLE"));
    }
    assert!(ctx.devices(&alice).await.is_empty());

    // With credentials, native tokens are checked for shape.
    let ctx = start_with(all_channels()).await;
    let alice = ctx.app.user("alice").await;
    for request in [fcm_token("has spaces in it"), apns_token("abc", false), apns_token(&"zz".repeat(32), false)] {
        assert_eq!(try_register(&ctx, &alice, &request).await.unwrap_err().reason, "TOKEN_INVALID");
    }
}

#[tokio::test]
async fn unregistering_is_scoped_to_the_caller() {
    let ctx = start_with(all_channels()).await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let laptop = ctx.register_browser(&alice).await;
    let phone_session = ctx.app.sign_in(alice.id).await;
    let phone_fcm = ctx.register(&phone_session, &fcm_token("alice-phone")).await;
    let phone_web = ctx.register_browser(&phone_session).await;
    let bobs = ctx.register_browser(&bob).await;

    // Bob cannot remove Alice's registration, nor learn that it exists.
    let err = unregister(&ctx, &bob, &laptop.id).await.unwrap_err();
    assert_eq!((err.code.as_str(), err.reason.as_str()), ("not_found", "DEVICE_NOT_FOUND"));
    assert_eq!(unregister(&ctx, &alice, &Uuid::now_v7().to_string()).await.unwrap_err().reason, "DEVICE_NOT_FOUND");
    assert_eq!(unregister(&ctx, &alice, "nope").await.unwrap_err().reason, "INVALID_ID");

    // One registration by ID, from any of the owner's sessions.
    unregister(&ctx, &alice, &phone_fcm.id).await.unwrap();
    let left: Vec<String> = ctx.devices(&alice).await.into_iter().map(|d| d.id).collect();
    assert!(left.contains(&laptop.id) && left.contains(&phone_web.id) && left.len() == 2);

    // No ID: everything of the calling session (what a client does before signing out).
    unregister(&ctx, &phone_session, "").await.unwrap();
    assert_eq!(ctx.devices(&alice).await.into_iter().map(|d| d.id).collect::<Vec<_>>(), [laptop.id]);
    assert_eq!(ctx.devices(&bob).await.into_iter().map(|d| d.id).collect::<Vec<_>>(), [bobs.id]);
}

#[tokio::test]
async fn registrations_end_with_their_session() {
    let _sweep = retention_lock().await;
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let phone = ctx.app.sign_in(alice.id).await;
    let kept = ctx.register_browser(&alice).await;
    ctx.register_browser(&phone).await;
    assert_eq!(ctx.devices(&alice).await.len(), 2);

    ctx.app.identity.revoke_session(alice.id, phone.session_id).await.unwrap();
    // Hidden at once...
    let visible: Vec<String> = ctx.devices(&alice).await.into_iter().map(|d| d.id).collect();
    assert_eq!(visible, std::slice::from_ref(&kept.id));
    // ...and a revoked session cannot register again while its access token is still valid.
    let err = try_register(&ctx, &phone, &Subscription::new(&endpoint()).request()).await.unwrap_err();
    assert_eq!(err.status, 401);

    // The sweep removes the rows; expired sessions go the same way.
    let report = ctx.notify.run_retention().await.unwrap();
    assert!(report.devices >= 1);
    sqlx::query("UPDATE sessions SET expires_at = now() - interval '1 minute' WHERE id = $1")
        .bind(alice.session_id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.notify.run_retention().await.unwrap();
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM push_devices WHERE user_id = $1")
        .bind(alice.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn a_test_push_is_queued_for_registered_devices() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let err = ctx
        .app
        .call::<_, SendTestPushResponse>(TEST_PUSH, Some(&alice), &SendTestPushRequest::default())
        .await
        .unwrap_err();
    assert_eq!((err.code.as_str(), err.reason.as_str()), ("failed_precondition", "NO_PUSH_DEVICES"));

    ctx.register_browser(&alice).await;
    ctx.register_browser(&ctx.app.sign_in(alice.id).await).await;
    let response: SendTestPushResponse =
        ctx.app.call(TEST_PUSH, Some(&alice), &SendTestPushRequest::default()).await.unwrap();
    assert_eq!(response.device_count, 2);
    assert!(ctx.inbox(&alice).await.notifications.is_empty(), "a test does not clutter the inbox");
    assert_eq!((TEST_KIND, PUSH_JOB), ("system.test", "notify.push"));

    // Too many tests in a row are refused.
    let mut limited = false;
    for _ in 0..6 {
        if let Err(err) =
            ctx.app.call::<_, SendTestPushResponse>(TEST_PUSH, Some(&alice), &SendTestPushRequest::default()).await
        {
            assert_eq!(err.reason, "RATE_LIMITED");
            limited = true;
        }
    }
    assert!(limited);
}
