//! Push delivery end to end: registration → consumer → job → a local HTTP
//! server standing in for the browser vendor's push service.
use std::sync::atomic::{AtomicUsize, Ordering};

use jsonwebtoken::{Algorithm, Validation};
use tokuly_core::outbound::{Localized, NotificationRequest};
use tokuly_notify::{
    consumer::{PushDecision, RequestOutcome},
    delivery::DeliveryReport,
    push::vapid::VapidClaims,
};
use tokuly_proto::notify::*;
use tokuly_testkit::TestUser;

use crate::common::*;

/// Registers a browser of `user` whose push endpoint is `path` on the stub.
async fn subscribe(
    ctx: &Ctx,
    user: &TestUser,
    stub: &Stub,
    path: &str,
    locale: &str,
) -> (Subscription, PushDevice) {
    let subscription = Subscription::new(&format!("{}{path}", stub.url));
    let device = ctx
        .register(
            user,
            &RegisterDeviceRequest {
                locale: locale.into(),
                ..subscription.request()
            },
        )
        .await;
    (subscription, device)
}

async fn vapid_public_key(ctx: &Ctx) -> String {
    let config: GetPushConfigResponse = ctx
        .app
        .call(
            "tokuly.notify.v1.PushService/GetPushConfig",
            None,
            &GetPushConfigRequest::default(),
        )
        .await
        .unwrap();
    config.vapid_public_key
}

#[tokio::test]
async fn a_notification_reaches_the_push_service_encrypted_for_the_browser() {
    let ctx = start_insecure().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let stub = Stub::accepting().await;
    // The account reads Japanese; this browser is set to English.
    let (subscription, device) = subscribe(&ctx, &alice, &stub, "/push/alice-laptop", "en").await;

    let request = chat_message(&alice, &bob)
        .collapse("chat:42")
        .image("https://cdn.example/preview.webp")
        .data(serde_json::json!({ "conversation_id": "42" }));
    let job = ctx.deliver_for_push(&request).await;
    let report = ctx.notify.deliver_push(&job).await.unwrap();
    assert_eq!(
        report,
        DeliveryReport {
            delivered: 1,
            ..Default::default()
        }
    );

    let requests = stub.requests_to("/push/alice-laptop");
    assert_eq!(requests.len(), 1);
    let sent = &requests[0];

    // RFC 8030 delivery options.
    assert_eq!(sent.header("ttl"), "86400");
    assert_eq!(
        sent.header("urgency"),
        "high",
        "chat is worth waking the device for"
    );
    assert_eq!(sent.header("content-encoding"), "aes128gcm");
    let topic = sent.header("topic");
    assert!(
        topic.len() == 32
            && topic
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "{topic}"
    );

    // RFC 8292: a token signed by the key that GetPushConfig advertises, for this push service.
    let public_key = vapid_public_key(&ctx).await;
    let authorization = sent.header("authorization");
    let (token, key) = authorization
        .strip_prefix("vapid t=")
        .and_then(|rest| rest.split_once(", k="))
        .expect("vapid scheme");
    assert_eq!(key, public_key);
    let mut validation = Validation::new(Algorithm::ES256);
    validation.set_audience(&[stub.url.as_str()]);
    let decoding_key = es256_decoding_key(&tokuly_core::keys::b64url_decode(&public_key).unwrap());
    let claims = jsonwebtoken::decode::<VapidClaims>(token, &decoding_key, &validation)
        .expect("the VAPID token verifies")
        .claims;
    assert!(claims.sub.starts_with("mailto:"));

    // RFC 8291: only the subscription's private key opens the body.
    assert!(
        !String::from_utf8_lossy(&sent.body).contains("Hi there"),
        "the push service sees ciphertext only"
    );
    let message = subscription.decrypt(&sent.body);
    assert_eq!(message["web_push"], 8030);
    let notification = &message["notification"];
    assert_eq!(
        (
            notification["title"].as_str(),
            notification["body"].as_str()
        ),
        (Some("New message"), Some("Hi there!")),
        "written in the device's language"
    );
    assert_eq!(notification["lang"], "en");
    assert_eq!(
        notification["navigate"], "http://localhost:8080/chat/42",
        "the click target is the web URL of the path"
    );
    assert_eq!(notification["tag"], "chat:42");
    assert_eq!(notification["image"], "https://cdn.example/preview.webp");
    assert_eq!(notification["data"]["kind"], "chat.message");
    assert_eq!(notification["data"]["path"], "/chat/42");
    assert_eq!(
        notification["data"]["id"],
        job.notification_id.unwrap().to_string()
    );
    assert_eq!(notification["data"]["extra"]["conversation_id"], "42");
    assert_eq!(message["app_badge"], 1, "the badge is the unread count");

    // Someone else's key does not open it.
    let stranger = Subscription::new(&subscription.endpoint);
    assert!(
        tokuly_notify::push::ece::decrypt(&sent.body, &stranger.secret, &subscription.auth)
            .is_err()
    );

    let listed = ctx.devices(&alice).await;
    assert_eq!(listed[0].id, device.id);
    assert!(listed[0].last_delivered_at.as_option().is_some());
}

#[tokio::test]
async fn each_device_gets_its_own_language_and_a_collapse_topic_only_when_keyed() {
    let ctx = start_insecure().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let stub = Stub::accepting().await;
    let (laptop, _) = subscribe(&ctx, &alice, &stub, "/push/laptop", "").await; // no device language: the account's
    let phone_session = ctx.app.sign_in(alice.id).await;
    let (phone, _) = subscribe(&ctx, &phone_session, &stub, "/push/phone", "en-GB").await;

    // No collapse key, not an urgent category.
    let request = NotificationRequest::new(
        alice.id,
        "video.uploaded",
        Localized::new("新しい動画", "New video"),
        Localized::new("見てみてね", "Have a look"),
        "/watch/abc",
    )
    .actor(bob.id);
    let job = ctx.deliver_for_push(&request).await;
    assert_eq!(ctx.notify.deliver_push(&job).await.unwrap().delivered, 2);

    let to_laptop = &stub.requests_to("/push/laptop")[0];
    let to_phone = &stub.requests_to("/push/phone")[0];
    assert_eq!(
        laptop.decrypt(&to_laptop.body)["notification"]["title"],
        "新しい動画"
    );
    assert_eq!(
        phone.decrypt(&to_phone.body)["notification"]["title"],
        "New video"
    );
    assert_eq!(to_laptop.header("urgency"), "normal");
    assert!(
        to_laptop.headers.get("topic").is_none(),
        "nothing to replace without a collapse key"
    );
    // Without a key the tag is the entry's ID, so distinct notifications do not replace each other.
    assert_eq!(
        laptop.decrypt(&to_laptop.body)["notification"]["tag"],
        job.notification_id.unwrap().to_string()
    );
}

#[tokio::test]
async fn subscriptions_the_push_service_reports_gone_are_dropped() {
    let ctx = start_insecure().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let stub = Stub::start(|request| match request.path.as_str() {
        "/push/expired" => (410, "subscription expired".into()),
        "/push/unknown" => (404, String::new()),
        _ => (201, String::new()),
    })
    .await;
    let (_, expired) = subscribe(&ctx, &alice, &stub, "/push/expired", "").await;
    let (_, unknown) = subscribe(
        &ctx,
        &ctx.app.sign_in(alice.id).await,
        &stub,
        "/push/unknown",
        "",
    )
    .await;
    let (_, healthy) = subscribe(
        &ctx,
        &ctx.app.sign_in(alice.id).await,
        &stub,
        "/push/healthy",
        "",
    )
    .await;

    let job = ctx.deliver_for_push(&chat_message(&alice, &bob)).await;
    let report = ctx.notify.deliver_push(&job).await.unwrap();
    assert_eq!(
        report,
        DeliveryReport {
            delivered: 1,
            removed: 2,
            ..Default::default()
        }
    );
    ctx.notify
        .run_push_job(job)
        .await
        .expect("gone devices do not fail the job");

    let left: Vec<String> = ctx
        .devices(&alice)
        .await
        .into_iter()
        .map(|d| d.id)
        .collect();
    assert_eq!(left, [healthy.id]);
    assert!(!left.contains(&expired.id) && !left.contains(&unknown.id));
}

#[tokio::test]
async fn a_temporary_failure_is_retried_for_the_missing_device_only() {
    let ctx = start_insecure().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let flaky_calls = AtomicUsize::new(0);
    let stub = Stub::start(move |request| match request.path.as_str() {
        // Down for the first two attempts, then back.
        "/push/flaky" if flaky_calls.fetch_add(1, Ordering::SeqCst) < 2 => {
            (503, "try later".into())
        }
        _ => (201, String::new()),
    })
    .await;
    subscribe(&ctx, &alice, &stub, "/push/steady", "").await;
    subscribe(
        &ctx,
        &ctx.app.sign_in(alice.id).await,
        &stub,
        "/push/flaky",
        "",
    )
    .await;
    let job = ctx.deliver_for_push(&chat_message(&alice, &bob)).await;

    // Attempt 1: one delivered, one to retry. The job fails so the queue runs it again.
    let err = ctx.notify.run_push_job(job.clone()).await.unwrap_err();
    assert_eq!(err.reason, "PUSH_RETRY");
    // Attempt 2: still down. The steady device is not bothered again.
    let report = ctx.notify.deliver_push(&job).await.unwrap();
    assert_eq!(
        report,
        DeliveryReport {
            retry: 1,
            skipped: 1,
            ..Default::default()
        }
    );
    // Attempt 3: back up.
    ctx.notify
        .run_push_job(job.clone())
        .await
        .expect("the retry succeeds");

    assert_eq!(
        stub.requests_to("/push/steady").len(),
        1,
        "delivered once, never repeated"
    );
    assert_eq!(stub.requests_to("/push/flaky").len(), 3);
    // A redelivered job (the queue is at-least-once) sends nothing more.
    let report = ctx.notify.deliver_push(&job).await.unwrap();
    assert_eq!(
        report,
        DeliveryReport {
            skipped: 2,
            ..Default::default()
        }
    );
    assert_eq!(stub.requests().len(), 4);
    assert_eq!(
        ctx.devices(&alice).await.len(),
        2,
        "a temporary failure never drops a registration"
    );
}

#[tokio::test]
async fn an_unreachable_push_service_is_a_retry_not_a_loss() {
    let ctx = start_insecure().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    // A port nothing listens on.
    let closed = {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        format!("http://{}/push/nobody-home", listener.local_addr().unwrap())
    };
    ctx.register(&alice, &Subscription::new(&closed).request())
        .await;
    let job = ctx.deliver_for_push(&chat_message(&alice, &bob)).await;
    let report = ctx.notify.deliver_push(&job).await.unwrap();
    assert_eq!(
        report,
        DeliveryReport {
            retry: 1,
            ..Default::default()
        }
    );
    assert_eq!(ctx.devices(&alice).await.len(), 1);
}

#[tokio::test]
async fn a_refused_message_is_neither_retried_nor_held_against_the_device() {
    let ctx = start_insecure().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let stub = Stub::start(|_| (403, "vapid key mismatch".into())).await;
    let (_, device) = subscribe(&ctx, &alice, &stub, "/push/picky", "").await;
    let job = ctx.deliver_for_push(&chat_message(&alice, &bob)).await;

    let report = ctx.notify.deliver_push(&job).await.unwrap();
    assert_eq!(
        report,
        DeliveryReport {
            rejected: 1,
            ..Default::default()
        }
    );
    ctx.notify
        .run_push_job(job)
        .await
        .expect("a refusal does not fail the job");
    assert_eq!(stub.requests().len(), 1, "not sent again");

    let registered: Vec<String> = ctx
        .devices(&alice)
        .await
        .into_iter()
        .map(|d| d.id)
        .collect();
    assert_eq!(
        registered,
        std::slice::from_ref(&device.id),
        "the registration stays"
    );
    let (rejections, last_error): (i32, String) =
        sqlx::query_as("SELECT rejections, last_error FROM push_devices WHERE id = $1::uuid")
            .bind(&device.id)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(rejections, 1);
    assert!(last_error.contains("403"), "{last_error}");
}

#[tokio::test]
async fn a_signed_out_device_stays_silent() {
    let ctx = start_insecure().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let stub = Stub::accepting().await;
    let phone = ctx.app.sign_in(alice.id).await;
    subscribe(&ctx, &alice, &stub, "/push/laptop", "").await;
    subscribe(&ctx, &phone, &stub, "/push/phone", "").await;
    let job = ctx.deliver_for_push(&chat_message(&alice, &bob)).await;

    // The phone's session is revoked between queuing and sending.
    ctx.app
        .identity
        .revoke_session(alice.id, phone.session_id)
        .await
        .unwrap();
    let report = ctx.notify.deliver_push(&job).await.unwrap();
    assert_eq!(
        report,
        DeliveryReport {
            delivered: 1,
            removed: 1,
            ..Default::default()
        }
    );
    assert!(
        stub.requests_to("/push/phone").is_empty(),
        "nothing is sent to a revoked session's device"
    );
    assert_eq!(stub.requests_to("/push/laptop").len(), 1);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM push_devices WHERE session_id = $1")
        .bind(phone.session_id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(rows, 0, "and its registration is removed");
}

#[tokio::test]
async fn stale_jobs_and_users_without_devices_send_nothing() {
    let ctx = start_insecure().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let stub = Stub::accepting().await;
    subscribe(&ctx, &alice, &stub, "/push/laptop", "").await;
    let mut job = ctx.deliver_for_push(&chat_message(&alice, &bob)).await;

    // Stuck in the queue for longer than a push service would have kept it.
    job.created_at -= chrono::Duration::days(2);
    assert_eq!(
        ctx.notify.deliver_push(&job).await.unwrap(),
        DeliveryReport::default()
    );
    // The user unregistered in the meantime.
    job.created_at = chrono::Utc::now();
    let _: UnregisterDeviceResponse = ctx
        .app
        .call(
            "tokuly.notify.v1.PushService/UnregisterDevice",
            Some(&alice),
            &UnregisterDeviceRequest::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        ctx.notify.deliver_push(&job).await.unwrap(),
        DeliveryReport::default()
    );
    assert!(stub.requests().is_empty());
}

#[tokio::test]
async fn a_test_push_reaches_the_device_even_during_quiet_hours() {
    let ctx = start_insecure().await;
    let alice = ctx.app.user("alice").await;
    let stub = Stub::accepting().await;
    let (subscription, _) = subscribe(&ctx, &alice, &stub, "/push/laptop", "").await;
    let _: PausePushResponse = ctx
        .app
        .call(
            "tokuly.notify.v1.PreferencesService/PausePush",
            Some(&alice),
            &PausePushRequest::default(),
        )
        .await
        .unwrap();
    let _: SendTestPushResponse = ctx
        .app
        .call(
            "tokuly.notify.v1.PushService/SendTestPush",
            Some(&alice),
            &SendTestPushRequest::default(),
        )
        .await
        .unwrap();

    // No worker runs in tests: take the job off the queue by hand, as the worker would.
    let stream = ctx
        .app
        .core
        .bus
        .js
        .get_stream(ctx.app.core.bus.stream_name(tokuly_core::bus::STREAM_JOBS))
        .await
        .unwrap();
    let subject = ctx.app.core.bus.subject("job.notify.push");
    let mut found = None;
    let last = stream.get_info().await.unwrap().state.last_sequence;
    for sequence in (1..=last).rev() {
        let Ok(raw) = stream.get_raw_message(sequence).await else {
            continue;
        };
        if raw.subject.as_str() != subject {
            continue;
        }
        let job: tokuly_notify::delivery::PushJob = serde_json::from_slice(&raw.payload).unwrap();
        if job.user_id == alice.id && job.kind == tokuly_notify::TEST_KIND {
            found = Some(job);
            break;
        }
    }
    let job = found.expect("the test push was queued as a notify.push job");
    ctx.notify.run_push_job(job).await.unwrap();

    let message = subscription.decrypt(&stub.requests_to("/push/laptop")[0].body);
    assert_eq!(message["notification"]["title"], "テスト通知だよ！");
    assert_eq!(
        message["notification"]["navigate"],
        "http://localhost:8080/settings/notifications"
    );
}

#[tokio::test]
async fn an_urgent_notification_is_pushed_at_high_urgency_even_while_paused() {
    let ctx = start_insecure().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let stub = Stub::accepting().await;
    let (subscription, _) = subscribe(&ctx, &alice, &stub, "/push/laptop", "").await;
    let _: PausePushResponse = ctx
        .app
        .call(
            "tokuly.notify.v1.PreferencesService/PausePush",
            Some(&alice),
            &PausePushRequest::default(),
        )
        .await
        .unwrap();
    // A category that is otherwise delivered at normal urgency.
    let payout = |title: &str| {
        NotificationRequest::new(
            alice.id,
            "wallet.payout",
            Localized::same(title),
            Localized::same("…"),
            "/wallet",
        )
        .actor(bob.id)
    };

    // The ordinary one is held back by the pause; the urgent one goes out.
    assert!(matches!(
        ctx.deliver(&payout("ordinary")).await,
        RequestOutcome::Handled {
            push: PushDecision::Skipped(_),
            ..
        }
    ));
    let job = ctx.deliver_for_push(&payout("urgent").urgent()).await;
    assert_eq!(ctx.notify.deliver_push(&job).await.unwrap().delivered, 1);

    let requests = stub.requests_to("/push/laptop");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].header("urgency"), "high");
    let message = subscription.decrypt(&requests[0].body);
    assert_eq!(message["notification"]["title"], "urgent");
    assert_eq!(message["notification"]["requireInteraction"], true);
    assert_eq!(message["notification"]["data"]["urgent"], true);

    // The same kind without the flag, once pushes are resumed: normal urgency.
    let _: ResumePushResponse = ctx
        .app
        .call(
            "tokuly.notify.v1.PreferencesService/ResumePush",
            Some(&alice),
            &ResumePushRequest::default(),
        )
        .await
        .unwrap();
    let job = ctx.deliver_for_push(&payout("later")).await;
    ctx.notify.deliver_push(&job).await.unwrap();
    let last = stub.requests_to("/push/laptop").pop().unwrap();
    assert_eq!(last.header("urgency"), "normal");
    let message = subscription.decrypt(&last.body);
    assert!(message["notification"].get("requireInteraction").is_none());
}
