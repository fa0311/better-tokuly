//! Webhooks: subscriptions, fan-out from domain events, signed deliveries
//! to a local endpoint, retries, the log, and where deliveries may not go.
use std::time::Duration;

use tokuly_core::outbound::{Localized, NotificationRequest};
use tokuly_dev::{DevConfig, delivery, secrets};
use tokuly_proto::dev::*;
use uuid::Uuid;

use crate::common::*;

type User = tokuly_testkit::TestUser;

fn friend_request(to: &User, from: &User) -> tokuly_core::events::Event {
    event(
        "social.friend.requested",
        serde_json::json!({ "request_id": Uuid::now_v7(), "from_user_id": from.id, "to_user_id": to.id, "source": 1 }),
    )
}

async fn get(ctx: &Ctx, caller: &User, webhook_id: &str) -> Result<Webhook, tokuly_testkit::RpcError> {
    let got: GetWebhookResponse = ctx
        .app
        .call(
            &format!("{WEBHOOK_SERVICE}/GetWebhook"),
            Some(caller),
            &GetWebhookRequest { webhook_id: webhook_id.into(), ..Default::default() },
        )
        .await?;
    Ok(got.webhook.into_option().expect("webhook"))
}

async fn deliveries(ctx: &Ctx, caller: &User, webhook_id: &str) -> Vec<Delivery> {
    let request = ListDeliveriesRequest { webhook_id: webhook_id.into(), ..Default::default() };
    let listed: ListDeliveriesResponse =
        ctx.app.call(&format!("{WEBHOOK_SERVICE}/ListDeliveries"), Some(caller), &request).await.unwrap();
    listed.deliveries
}

/// Makes a waiting retry due now, as if its pause had passed.
async fn make_due(ctx: &Ctx, webhook_id: &str) {
    sqlx::query("UPDATE dev_webhook_deliveries SET next_attempt_at = now() - interval '1 second' WHERE webhook_id = $1 AND next_attempt_at IS NOT NULL")
        .bind(Uuid::parse_str(webhook_id).unwrap())
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
}

#[tokio::test]
async fn the_event_catalogue_says_what_each_type_needs() {
    let ctx = start().await;
    let listed: ListEventTypesResponse = ctx
        .app
        .call(&format!("{WEBHOOK_SERVICE}/ListEventTypes"), None, &ListEventTypesRequest::default())
        .await
        .unwrap();
    let find = |name: &str| {
        listed.event_types.iter().find(|t| t.r#type == name).unwrap_or_else(|| panic!("{name} is in the catalogue"))
    };
    assert_eq!(
        (find("friend.request_received").scope.as_str(), find("friend.request_received").payload.as_str()),
        ("social:read", "social.friend.requested")
    );
    assert_eq!(
        (find("chat.message_received").scope.as_str(), find("chat.message_received").payload.as_str()),
        ("chat:read", "notification")
    );
    assert_eq!(find("place.alert").scope, "geo:read");
    assert!(!find("live.started").description.as_option().unwrap().ja.is_empty());
}

#[tokio::test]
async fn an_event_is_delivered_signed_to_the_account_it_concerns() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);
    let receiver = Receiver::start().await;
    let (webhook, secret) =
        create_webhook(&ctx, &alice, &receiver.url, &["friend.request_received", "friend.added"]).await;
    assert!(secrets::is(&secret, secrets::SecretKind::Webhook));
    assert_eq!(
        (webhook.status.as_known(), webhook.account_id.as_str()),
        (Some(WebhookStatus::WEBHOOK_STATUS_ACTIVE), alice.id.to_string().as_str())
    );
    assert_eq!(webhook.secret_hint, format!("tkl_whs_…{}", &secret[secret.len() - 4..]));

    // Bob asks Alice: hers to hear about. Bob asks Carol: not.
    let event = friend_request(&alice, &bob);
    assert_eq!(ctx.dev.fan_out(&event).await.unwrap(), 1);
    assert_eq!(ctx.dev.fan_out(&friend_request(&carol, &bob)).await.unwrap(), 0);
    // An event of a type she did not subscribe to, and one nobody subscribes to.
    assert_eq!(
        ctx.dev
            .fan_out(&crate::common::event(
                "wallet.balance.changed",
                serde_json::json!({ "user_id": alice.id, "coins": 5 })
            ))
            .await
            .unwrap(),
        0
    );
    // The bus may deliver an event twice; the webhook gets it once.
    assert_eq!(ctx.dev.fan_out(&event).await.unwrap(), 0);

    let ids = delivery_ids(&ctx, &webhook.id).await;
    assert_eq!(ids.len(), 1);
    let pending = &deliveries(&ctx, &alice, &webhook.id).await[0];
    assert_eq!((pending.status.as_known(), pending.attempt_count), (Some(DeliveryStatus::DELIVERY_STATUS_PENDING), 0));

    let delivered = ctx.dev.deliver(ids[0]).await.unwrap().expect("the delivery was due");
    assert_eq!(
        (delivered.status.as_known(), delivered.attempt_count),
        (Some(DeliveryStatus::DELIVERY_STATUS_SUCCEEDED), 1)
    );
    assert_eq!(
        (delivered.attempts[0].status_code, delivered.attempts[0].response_snippet.as_str()),
        (200, "received by the test endpoint")
    );
    // Nothing is sent twice.
    assert!(ctx.dev.deliver(ids[0]).await.unwrap().is_none());

    let hits = receiver.hits();
    assert_eq!(hits.len(), 1);
    let hit = &hits[0];
    // The envelope: id, type, time, data.
    let body = hit.json();
    assert_eq!(
        (body["id"].as_str(), body["type"].as_str()),
        (Some(event.id.to_string().as_str()), Some("friend.request_received"))
    );
    assert_eq!(body["data"]["from_user_id"], bob.id.to_string());
    assert!(chrono::DateTime::parse_from_rfc3339(body["time"].as_str().unwrap()).is_ok());
    assert_eq!(
        (hit.header("content-type"), hit.header("tokuly-event"), hit.header("tokuly-delivery")),
        ("application/json", "friend.request_received", ids[0].to_string().as_str())
    );
    assert!(hit.header("user-agent").starts_with("Tokuly-Webhooks/"));
    // The signature is over this exact body, fresh, and only verifiable with the secret.
    let signature = hit.header(delivery::SIGNATURE_HEADER);
    let tolerance = Duration::from_secs(300);
    assert!(delivery::verify_signature(&secret, signature, &hit.body, tolerance, chrono::Utc::now()));
    assert!(!delivery::verify_signature(
        &secret,
        signature,
        &hit.body.replace("friend", "enemy"),
        tolerance,
        chrono::Utc::now()
    ));
    assert!(
        !delivery::verify_signature(
            &secret,
            signature,
            &hit.body,
            tolerance,
            chrono::Utc::now() + chrono::Duration::hours(1)
        ),
        "a replay is too old"
    );
    assert!(!delivery::verify_signature(
        &secrets::generate(secrets::SecretKind::Webhook),
        signature,
        &hit.body,
        tolerance,
        chrono::Utc::now()
    ));

    // The log: the list is brief, one delivery has the payload and every attempt.
    let logged = &deliveries(&ctx, &alice, &webhook.id).await[0];
    assert!(logged.payload.is_empty() && logged.completed_at.as_option().is_some());
    let one: GetDeliveryResponse = ctx
        .app
        .call(
            &format!("{WEBHOOK_SERVICE}/GetDelivery"),
            Some(&alice),
            &GetDeliveryRequest { delivery_id: ids[0].to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    let full = one.delivery.as_option().unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&full.payload).unwrap(), body);
    assert!(full.attempts[0].latency_ms >= 0 && full.event_id == event.id.to_string());
    let after = get(&ctx, &alice, &webhook.id).await.unwrap();
    assert!(after.last_success_at.as_option().is_some() && after.consecutive_failures == 0);

    // Somebody else sees neither the webhook nor its log.
    assert_eq!(get(&ctx, &bob, &webhook.id).await.unwrap_err().reason, "WEBHOOK_NOT_FOUND");
    let theirs = ctx
        .app
        .call::<_, GetDeliveryResponse>(
            &format!("{WEBHOOK_SERVICE}/GetDelivery"),
            Some(&bob),
            &GetDeliveryRequest { delivery_id: ids[0].to_string(), ..Default::default() },
        )
        .await;
    assert_eq!(
        (theirs.as_ref().unwrap_err().code.as_str(), theirs.as_ref().unwrap_err().reason.as_str()),
        ("not_found", "DELIVERY_NOT_FOUND")
    );
    let log = ctx
        .app
        .call::<_, ListDeliveriesResponse>(
            &format!("{WEBHOOK_SERVICE}/ListDeliveries"),
            Some(&bob),
            &ListDeliveriesRequest { webhook_id: webhook.id.clone(), ..Default::default() },
        )
        .await;
    assert_eq!(log.unwrap_err().reason, "WEBHOOK_NOT_FOUND");
}

#[tokio::test]
async fn notifications_reach_webhooks_by_kind() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let receiver = Receiver::start().await;
    let (webhook, _) =
        create_webhook(&ctx, &alice, &receiver.url, &["notification.created", "chat.message_received", "place.alert"])
            .await;
    let notify = |kind: &str, to: &User| {
        let request = NotificationRequest::new(
            to.id,
            kind,
            Localized::new("ボブ", "Bob"),
            Localized::new("やっほー", "Hey"),
            "/chat/1",
        )
        .actor(bob.id);
        event("notify.requested", &request)
    };
    // A chat message is both "a notification" and "a message received".
    assert_eq!(ctx.dev.fan_out(&notify("chat.message", &alice)).await.unwrap(), 2);
    // A buzz is only a notification; a place alert is its own type too.
    assert_eq!(ctx.dev.fan_out(&notify("social.buzz", &alice)).await.unwrap(), 1);
    assert_eq!(ctx.dev.fan_out(&notify("geo.place_alert", &alice)).await.unwrap(), 2);
    // Somebody else's notification is nobody's business here.
    assert_eq!(ctx.dev.fan_out(&notify("chat.message", &bob)).await.unwrap(), 0);

    for id in delivery_ids(&ctx, &webhook.id).await {
        ctx.dev.deliver(id).await.unwrap();
    }
    let hits = receiver.hits();
    let mut types: Vec<&str> = hits.iter().map(|hit| hit.header("tokuly-event")).collect();
    types.sort();
    assert_eq!(
        types,
        [
            "chat.message_received",
            "notification.created",
            "notification.created",
            "notification.created",
            "place.alert"
        ]
    );
    let message = hits.iter().find(|hit| hit.header("tokuly-event") == "chat.message_received").unwrap().json();
    assert_eq!(
        (message["data"]["kind"].as_str(), message["data"]["actor_id"].as_str()),
        (Some("chat.message"), Some(bob.id.to_string().as_str()))
    );
    assert_eq!(
        (
            message["data"]["body"]["ja"].as_str(),
            message["data"]["body"]["en"].as_str(),
            message["data"]["path"].as_str()
        ),
        (Some("やっほー"), Some("Hey"), Some("/chat/1"))
    );
}

#[tokio::test]
async fn webhook_requests_are_validated() {
    // The production rules for where a webhook may point.
    let ctx = start_with(DevConfig { max_webhooks: 2, ..Default::default() }).await;
    let alice = ctx.app.user("alice").await;
    let create = |url: &str, types: &[&str], description: &str| {
        let request = CreateWebhookRequest {
            url: url.into(),
            event_types: types.iter().map(|t| t.to_string()).collect(),
            description: description.into(),
            ..Default::default()
        };
        let (ctx, alice) = (&ctx, &alice);
        async move {
            ctx.app
                .call::<_, CreateWebhookResponse>(&format!("{WEBHOOK_SERVICE}/CreateWebhook"), Some(alice), &request)
                .await
                .map_err(|e| e.reason)
        }
    };
    for url in [
        "http://hooks.example.com/tokuly",
        "https://localhost/hook",
        "https://127.0.0.1/hook",
        "https://10.0.0.8/hook",
        "https://169.254.169.254/latest/meta-data/",
        "https://[::1]/hook",
        "https://printer.local/hook",
        "https://user:secret@hooks.example.com/",
        "ftp://hooks.example.com/",
        "nonsense",
        "",
    ] {
        assert_eq!(create(url, &["friend.added"], "").await.unwrap_err(), "WEBHOOK_URL_INVALID", "{url}");
    }
    assert_eq!(create("https://hooks.example.com/t", &[], "").await.unwrap_err(), "EVENT_TYPES_REQUIRED");
    assert_eq!(
        create("https://hooks.example.com/t", &["friend.exploded"], "").await.unwrap_err(),
        "EVENT_TYPE_UNKNOWN"
    );
    assert_eq!(create("https://hooks.example.com/t", &["ping"], "").await.unwrap_err(), "EVENT_TYPE_UNKNOWN");
    assert_eq!(
        create("https://hooks.example.com/t", &["friend.added"], &"d".repeat(201)).await.unwrap_err(),
        "DESCRIPTION_TOO_LONG"
    );
    let anonymous = CreateWebhookRequest {
        url: "https://hooks.example.com/t".into(),
        event_types: vec!["friend.added".into()],
        ..Default::default()
    };
    assert_eq!(
        ctx.app
            .call::<_, CreateWebhookResponse>(&format!("{WEBHOOK_SERVICE}/CreateWebhook"), None, &anonymous)
            .await
            .unwrap_err()
            .reason,
        "SIGN_IN_REQUIRED"
    );

    assert!(create("https://hooks.example.com/one", &["friend.added", "friend.added"], "first").await.is_ok());
    assert!(create("https://hooks.example.com/two", &["friend.added"], "").await.is_ok());
    assert_eq!(
        create("https://hooks.example.com/three", &["friend.added"], "").await.unwrap_err(),
        "TOO_MANY_WEBHOOKS"
    );
    let listed: ListWebhooksResponse = ctx
        .app
        .call(&format!("{WEBHOOK_SERVICE}/ListWebhooks"), Some(&alice), &ListWebhooksRequest::default())
        .await
        .unwrap();
    assert_eq!(
        listed.webhooks.iter().map(|w| (w.url.as_str(), w.event_types.len())).collect::<Vec<_>>(),
        [("https://hooks.example.com/one", 1), ("https://hooks.example.com/two", 1)]
    );
}

#[tokio::test]
async fn a_delivery_never_goes_into_a_private_network() {
    // Production rules again; the receiver is on loopback, where no delivery may go.
    let ctx = start_with(DevConfig::default()).await;
    let alice = ctx.app.user("alice").await;
    let receiver = Receiver::start().await;
    let (webhook, _) = create_webhook(&ctx, &alice, "https://hooks.example.com/tokuly", &["friend.added"]).await;
    let webhook_id = Uuid::parse_str(&webhook.id).unwrap();
    let attempt = |url: String| {
        let (ctx, alice) = (&ctx, &alice);
        async move {
            // As if the address had been acceptable when it was registered.
            sqlx::query("UPDATE dev_webhooks SET url = $2 WHERE id = $1")
                .bind(webhook_id)
                .bind(&url)
                .execute(&ctx.app.core.db)
                .await
                .unwrap();
            let ping = PingWebhookRequest { webhook_id: webhook_id.to_string(), ..Default::default() };
            let pinged: PingWebhookResponse =
                ctx.app.call(&format!("{WEBHOOK_SERVICE}/PingWebhook"), Some(alice), &ping).await.unwrap();
            let delivery = pinged.delivery.into_option().unwrap();
            (delivery.status.as_known(), delivery.attempts[0].status_code, delivery.attempts[0].error.clone())
        }
    };
    let failed = Some(DeliveryStatus::DELIVERY_STATUS_FAILED);
    // An address literal, plain http, and names that only exist inside a network.
    assert_eq!(attempt(receiver.url.clone()).await, (failed, 0, "address not allowed".to_owned()));
    assert_eq!(
        attempt(receiver.url.replace("http://", "https://")).await,
        (failed, 0, "address not allowed".to_owned())
    );
    assert_eq!(attempt("https://localhost/hook".to_owned()).await, (failed, 0, "address not allowed".to_owned()));
    assert!(receiver.hits().is_empty(), "nothing reached the loopback endpoint");
}

#[tokio::test]
async fn failed_deliveries_are_retried_with_growing_pauses_then_given_up() {
    let ctx = start_with(DevConfig { webhook_max_attempts: 3, ..config() }).await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let receiver = Receiver::start().await;
    let (webhook, _) = create_webhook(&ctx, &alice, &receiver.url, &["friend.request_received"]).await;
    receiver.answer_with(503);
    assert_eq!(ctx.dev.fan_out(&friend_request(&alice, &bob)).await.unwrap(), 1);
    let id = delivery_ids(&ctx, &webhook.id).await[0];

    let first = ctx.dev.deliver(id).await.unwrap().unwrap();
    assert_eq!(
        (first.status.as_known(), first.attempt_count, first.attempts[0].status_code),
        (Some(DeliveryStatus::DELIVERY_STATUS_RETRYING), 1, 503)
    );
    let wait = |delivery: &Delivery| {
        (tokuly_core::time::from_ts(delivery.next_attempt_at.as_option().unwrap()) - chrono::Utc::now()).num_seconds()
    };
    assert!((50..=60).contains(&wait(&first)), "the first retry waits a minute");
    // Not before its time.
    assert_eq!(ctx.dev.queue_due_deliveries().await.unwrap(), 0);
    assert_eq!(get(&ctx, &alice, &webhook.id).await.unwrap().consecutive_failures, 1);

    // The pause passes; the sweep queues the retry (the job runs it).
    make_due(&ctx, &webhook.id).await;
    assert_eq!(ctx.dev.queue_due_deliveries().await.unwrap(), 1);
    assert_eq!(ctx.dev.queue_due_deliveries().await.unwrap(), 0, "queued once");
    let second = ctx.dev.deliver(id).await.unwrap().unwrap();
    assert_eq!((second.status.as_known(), second.attempt_count), (Some(DeliveryStatus::DELIVERY_STATUS_RETRYING), 2));
    assert!((230..=240).contains(&wait(&second)), "the second waits four times as long");

    // The third attempt is the last.
    let third = ctx.dev.deliver(id).await.unwrap().unwrap();
    assert_eq!(
        (third.status.as_known(), third.attempt_count, third.attempts.len()),
        (Some(DeliveryStatus::DELIVERY_STATUS_FAILED), 3, 3)
    );
    assert!(third.next_attempt_at.as_option().is_none() && third.completed_at.as_option().is_some());
    assert!(ctx.dev.deliver(id).await.unwrap().is_none());
    assert_eq!(receiver.hits().len(), 3);
    let failing = get(&ctx, &alice, &webhook.id).await.unwrap();
    assert_eq!(
        (failing.consecutive_failures, failing.status.as_known()),
        (3, Some(WebhookStatus::WEBHOOK_STATUS_ACTIVE))
    );

    // The log can be filtered by outcome.
    let failed_only = ListDeliveriesRequest {
        webhook_id: webhook.id.clone(),
        status: DeliveryStatus::DELIVERY_STATUS_FAILED.into(),
        ..Default::default()
    };
    let listed: ListDeliveriesResponse =
        ctx.app.call(&format!("{WEBHOOK_SERVICE}/ListDeliveries"), Some(&alice), &failed_only).await.unwrap();
    assert_eq!(listed.deliveries.len(), 1);
    let succeeded_only =
        ListDeliveriesRequest { status: DeliveryStatus::DELIVERY_STATUS_SUCCEEDED.into(), ..failed_only };
    let listed: ListDeliveriesResponse =
        ctx.app.call(&format!("{WEBHOOK_SERVICE}/ListDeliveries"), Some(&alice), &succeeded_only).await.unwrap();
    assert!(listed.deliveries.is_empty());

    // The endpoint is fixed: the event is replayed as a new delivery of the same event.
    receiver.answer_with(204);
    let replayed: ReplayDeliveryResponse = ctx
        .app
        .call(
            &format!("{WEBHOOK_SERVICE}/ReplayDelivery"),
            Some(&alice),
            &ReplayDeliveryRequest { delivery_id: id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    let replay = replayed.delivery.as_option().unwrap();
    assert_eq!(
        (replay.replay_of.as_str(), replay.event_id.as_str(), replay.status.as_known()),
        (id.to_string().as_str(), third.event_id.as_str(), Some(DeliveryStatus::DELIVERY_STATUS_PENDING))
    );
    let done = ctx.dev.deliver(Uuid::parse_str(&replay.id).unwrap()).await.unwrap().unwrap();
    assert_eq!(
        (done.status.as_known(), done.attempts[0].status_code),
        (Some(DeliveryStatus::DELIVERY_STATUS_SUCCEEDED), 204)
    );
    assert_eq!(receiver.hits().last().unwrap().json()["id"], third.event_id);
    // One success ends the run of failures.
    assert_eq!(get(&ctx, &alice, &webhook.id).await.unwrap().consecutive_failures, 0);
    let not_theirs = ctx
        .app
        .call::<_, ReplayDeliveryResponse>(
            &format!("{WEBHOOK_SERVICE}/ReplayDelivery"),
            Some(&bob),
            &ReplayDeliveryRequest { delivery_id: id.to_string(), ..Default::default() },
        )
        .await;
    assert_eq!(not_theirs.unwrap_err().reason, "DELIVERY_NOT_FOUND");
}

#[tokio::test]
async fn redirects_are_failures_and_are_not_followed() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let receiver = Receiver::start().await;
    let (webhook, _) = create_webhook(&ctx, &alice, &receiver.url, &["friend.request_received"]).await;
    receiver.answer_with(302);
    ctx.dev.fan_out(&friend_request(&alice, &bob)).await.unwrap();
    let delivered = ctx.dev.deliver(delivery_ids(&ctx, &webhook.id).await[0]).await.unwrap().unwrap();
    assert_eq!(delivered.status.as_known(), Some(DeliveryStatus::DELIVERY_STATUS_RETRYING));
    assert_eq!(
        (delivered.attempts[0].status_code, delivered.attempts[0].error.as_str()),
        (302, "redirects are not followed")
    );
    let paths: Vec<String> = receiver.hits().into_iter().map(|hit| hit.path).collect();
    assert_eq!(paths, ["/hook"], "the redirect target was never requested");
}

#[tokio::test]
async fn an_endpoint_that_keeps_failing_is_disabled_and_its_owner_told() {
    let ctx =
        start_with(DevConfig { webhook_disable_after_failures: 2, webhook_disable_after_hours: 0, ..config() }).await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let receiver = Receiver::start().await;
    let (webhook, _) = create_webhook(&ctx, &alice, &receiver.url, &["friend.request_received"]).await;
    receiver.answer_with(500);
    for _ in 0..3 {
        ctx.dev.fan_out(&friend_request(&alice, &bob)).await.unwrap();
    }
    let ids = delivery_ids(&ctx, &webhook.id).await;
    ctx.dev.deliver(ids[0]).await.unwrap();
    assert_eq!(
        get(&ctx, &alice, &webhook.id).await.unwrap().status.as_known(),
        Some(WebhookStatus::WEBHOOK_STATUS_ACTIVE)
    );
    ctx.dev.deliver(ids[1]).await.unwrap();

    let disabled = get(&ctx, &alice, &webhook.id).await.unwrap();
    assert_eq!(disabled.status.as_known(), Some(WebhookStatus::WEBHOOK_STATUS_DISABLED));
    assert!(disabled.disabled_reason.contains("HTTP 500"), "{}", disabled.disabled_reason);
    let notices = ctx.notifications(alice.id, "dev.webhook_disabled").await;
    assert_eq!(notices.len(), 1);
    assert!(notices[0]["body"]["en"].as_str().unwrap().contains("127.0.0.1"));
    assert_eq!(
        ctx.app.events("dev.webhook.disabled").await.iter().filter(|e| e.data["webhook_id"] == webhook.id).count(),
        1
    );

    // What was waiting is settled as failed, and nothing more is sent or queued.
    let statuses: Vec<_> =
        deliveries(&ctx, &alice, &webhook.id).await.iter().map(|d| d.status.as_known().unwrap()).collect();
    assert_eq!(statuses, [DeliveryStatus::DELIVERY_STATUS_FAILED; 3]);
    assert!(ctx.dev.deliver(ids[2]).await.unwrap().is_none());
    assert_eq!(ctx.dev.fan_out(&friend_request(&alice, &bob)).await.unwrap(), 0);
    assert_eq!(receiver.hits().len(), 2);
    let replay = ctx
        .app
        .call::<_, ReplayDeliveryResponse>(
            &format!("{WEBHOOK_SERVICE}/ReplayDelivery"),
            Some(&alice),
            &ReplayDeliveryRequest { delivery_id: ids[0].to_string(), ..Default::default() },
        )
        .await;
    assert_eq!(
        (replay.as_ref().unwrap_err().code.as_str(), replay.as_ref().unwrap_err().reason.as_str()),
        ("failed_precondition", "WEBHOOK_NOT_ACTIVE")
    );

    // Its owner fixes the endpoint and resumes it: a clean record.
    receiver.answer_with(200);
    let resume = UpdateWebhookRequest {
        webhook_id: webhook.id.clone(),
        url: receiver.url.clone(),
        event_types: vec!["friend.request_received".into()],
        paused: false,
        ..Default::default()
    };
    let resumed: UpdateWebhookResponse =
        ctx.app.call(&format!("{WEBHOOK_SERVICE}/UpdateWebhook"), Some(&alice), &resume).await.unwrap();
    let resumed = resumed.webhook.as_option().unwrap();
    assert_eq!(
        (resumed.status.as_known(), resumed.consecutive_failures, resumed.disabled_reason.as_str()),
        (Some(WebhookStatus::WEBHOOK_STATUS_ACTIVE), 0, "")
    );
    assert_eq!(ctx.dev.fan_out(&friend_request(&alice, &bob)).await.unwrap(), 1);
}

#[tokio::test]
async fn a_webhook_is_edited_paused_pinged_and_deleted() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let receiver = Receiver::start().await;
    let (webhook, secret) = create_webhook(&ctx, &alice, &receiver.url, &["friend.request_received"]).await;
    let ping = |caller: &User| {
        let (ctx, caller, request) =
            (&ctx, caller.clone(), PingWebhookRequest { webhook_id: webhook.id.clone(), ..Default::default() });
        async move {
            ctx.app
                .call::<_, PingWebhookResponse>(&format!("{WEBHOOK_SERVICE}/PingWebhook"), Some(&caller), &request)
                .await
        }
    };

    // A ping says how the endpoint answers, right away.
    let pinged = ping(&alice).await.unwrap().delivery.into_option().unwrap();
    assert_eq!(
        (pinged.event_type.as_str(), pinged.status.as_known(), pinged.attempts[0].status_code),
        ("ping", Some(DeliveryStatus::DELIVERY_STATUS_SUCCEEDED), 200)
    );
    let body: serde_json::Value = serde_json::from_str(&pinged.payload).unwrap();
    assert_eq!((body["type"].as_str(), body["data"]["webhook_id"].as_str()), (Some("ping"), Some(webhook.id.as_str())));
    assert!(delivery::verify_signature(
        &secret,
        receiver.hits()[0].header(delivery::SIGNATURE_HEADER),
        &receiver.hits()[0].body,
        Duration::from_secs(60),
        chrono::Utc::now()
    ));
    assert_eq!(ping(&bob).await.unwrap_err().reason, "WEBHOOK_NOT_FOUND");
    // A failing ping is one failed try: no retry, and no mark against the endpoint.
    receiver.answer_with(500);
    let failed = ping(&alice).await.unwrap().delivery.into_option().unwrap();
    assert_eq!((failed.status.as_known(), failed.attempt_count), (Some(DeliveryStatus::DELIVERY_STATUS_FAILED), 1));
    assert_eq!(get(&ctx, &alice, &webhook.id).await.unwrap().consecutive_failures, 0);
    receiver.answer_with(200);

    // Editing replaces URL, description and types; pausing stops deliveries but not pings.
    let update = UpdateWebhookRequest {
        webhook_id: webhook.id.clone(),
        url: format!("{}/v2", receiver.url),
        description: "second version".into(),
        event_types: vec!["friend.added".into(), "friend.removed".into()],
        paused: true,
        ..Default::default()
    };
    assert_eq!(
        ctx.app
            .call::<_, UpdateWebhookResponse>(&format!("{WEBHOOK_SERVICE}/UpdateWebhook"), Some(&bob), &update)
            .await
            .unwrap_err()
            .reason,
        "WEBHOOK_NOT_FOUND"
    );
    let invalid = UpdateWebhookRequest { event_types: vec!["nope".into()], ..update.clone() };
    assert_eq!(
        ctx.app
            .call::<_, UpdateWebhookResponse>(&format!("{WEBHOOK_SERVICE}/UpdateWebhook"), Some(&alice), &invalid)
            .await
            .unwrap_err()
            .reason,
        "EVENT_TYPE_UNKNOWN"
    );
    let updated: UpdateWebhookResponse =
        ctx.app.call(&format!("{WEBHOOK_SERVICE}/UpdateWebhook"), Some(&alice), &update).await.unwrap();
    let paused = updated.webhook.as_option().unwrap();
    assert_eq!(
        (paused.status.as_known(), paused.description.as_str(), paused.event_types.len()),
        (Some(WebhookStatus::WEBHOOK_STATUS_PAUSED), "second version", 2)
    );
    let accepted =
        event("social.friend.accepted", serde_json::json!({ "user_id": bob.id, "friend_id": alice.id, "source": 1 }));
    assert_eq!(ctx.dev.fan_out(&accepted).await.unwrap(), 0, "paused");
    assert!(ping(&alice).await.is_ok());
    assert_eq!(receiver.hits().last().unwrap().path, "/hook/v2");

    // Resumed, it hears the next event (either side of a new friendship is "the account").
    let resumed: UpdateWebhookResponse = ctx
        .app
        .call(
            &format!("{WEBHOOK_SERVICE}/UpdateWebhook"),
            Some(&alice),
            &UpdateWebhookRequest { paused: false, ..update.clone() },
        )
        .await
        .unwrap();
    assert_eq!(resumed.webhook.as_option().unwrap().status.as_known(), Some(WebhookStatus::WEBHOOK_STATUS_ACTIVE));
    let accepted =
        event("social.friend.accepted", serde_json::json!({ "user_id": bob.id, "friend_id": alice.id, "source": 1 }));
    assert_eq!(ctx.dev.fan_out(&accepted).await.unwrap(), 1);

    // Deleting takes the log with it.
    let delete = DeleteWebhookRequest { webhook_id: webhook.id.clone(), ..Default::default() };
    assert_eq!(
        ctx.app
            .call::<_, DeleteWebhookResponse>(&format!("{WEBHOOK_SERVICE}/DeleteWebhook"), Some(&bob), &delete)
            .await
            .unwrap_err()
            .reason,
        "WEBHOOK_NOT_FOUND"
    );
    ctx.app
        .call::<_, DeleteWebhookResponse>(&format!("{WEBHOOK_SERVICE}/DeleteWebhook"), Some(&alice), &delete)
        .await
        .unwrap();
    assert_eq!(get(&ctx, &alice, &webhook.id).await.unwrap_err().reason, "WEBHOOK_NOT_FOUND");
    assert!(delivery_ids(&ctx, &webhook.id).await.is_empty());
    assert_eq!(
        ctx.dev
            .fan_out(&event("social.friend.accepted", serde_json::json!({ "user_id": bob.id, "friend_id": alice.id })))
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn a_rotated_secret_overlaps_with_the_one_it_replaces() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let receiver = Receiver::start().await;
    let (webhook, old_secret) = create_webhook(&ctx, &alice, &receiver.url, &["friend.request_received"]).await;
    let rotate = RotateWebhookSecretRequest { webhook_id: webhook.id.clone(), ..Default::default() };
    assert_eq!(
        ctx.app
            .call::<_, RotateWebhookSecretResponse>(
                &format!("{WEBHOOK_SERVICE}/RotateWebhookSecret"),
                Some(&bob),
                &rotate
            )
            .await
            .unwrap_err()
            .reason,
        "WEBHOOK_NOT_FOUND"
    );
    let rotated: RotateWebhookSecretResponse =
        ctx.app.call(&format!("{WEBHOOK_SERVICE}/RotateWebhookSecret"), Some(&alice), &rotate).await.unwrap();
    assert!(
        rotated.secret != old_secret
            && rotated.webhook.as_option().unwrap().previous_secret_expires_at.as_option().is_some()
    );
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM dev_webhooks w WHERE position(convert_to($1, 'UTF8') in secret_sealed) > 0 OR to_jsonb(w)::text LIKE '%' || $1 || '%'")
        .bind(&rotated.secret)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(stored, 0, "the signing secret is encrypted at rest");

    ctx.dev.fan_out(&friend_request(&alice, &bob)).await.unwrap();
    ctx.dev.deliver(delivery_ids(&ctx, &webhook.id).await[0]).await.unwrap();
    let hit = &receiver.hits()[0];
    let signature = hit.header(delivery::SIGNATURE_HEADER);
    assert_eq!(signature.matches("v1=").count(), 2);
    // A receiver that has not switched yet still verifies; so does one that has.
    for secret in [&old_secret, &rotated.secret] {
        assert!(delivery::verify_signature(secret, signature, &hit.body, Duration::from_secs(60), chrono::Utc::now()));
    }
    // After the overlap only the new secret signs.
    sqlx::query("UPDATE dev_webhooks SET previous_secret_expires_at = now() - interval '1 second' WHERE id = $1")
        .bind(Uuid::parse_str(&webhook.id).unwrap())
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.dev.fan_out(&friend_request(&alice, &bob)).await.unwrap();
    ctx.dev.deliver(delivery_ids(&ctx, &webhook.id).await[1]).await.unwrap();
    let hit = &receiver.hits()[1];
    assert_eq!(hit.header(delivery::SIGNATURE_HEADER).matches("v1=").count(), 1);
    assert!(!delivery::verify_signature(
        &old_secret,
        hit.header(delivery::SIGNATURE_HEADER),
        &hit.body,
        Duration::from_secs(60),
        chrono::Utc::now()
    ));
}

#[tokio::test]
async fn a_token_subscribes_only_to_what_it_may_read() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let (_, limited) = personal_token(&ctx, &alice, &["dev:write", "social:read"]).await;
    let limited = with_token(&alice, &limited);
    let create = |caller: &User, types: &[&str]| {
        let request = CreateWebhookRequest {
            url: "http://127.0.0.1:9/hook".into(),
            event_types: types.iter().map(|t| t.to_string()).collect(),
            ..Default::default()
        };
        let (ctx, caller) = (&ctx, caller.clone());
        async move {
            ctx.app
                .call::<_, CreateWebhookResponse>(&format!("{WEBHOOK_SERVICE}/CreateWebhook"), Some(&caller), &request)
                .await
        }
    };
    // Friend events are within its scopes; private messages are not.
    let created = create(&limited, &["friend.request_received"]).await.unwrap().webhook.into_option().unwrap();
    let escalation = create(&limited, &["friend.request_received", "chat.message_received"]).await.unwrap_err();
    assert_eq!((escalation.code.as_str(), escalation.reason.as_str()), ("permission_denied", "INSUFFICIENT_SCOPE"));
    assert!(escalation.message.contains("chat:read"));
    // Nor by editing an existing webhook.
    let widen = UpdateWebhookRequest {
        webhook_id: created.id.clone(),
        url: created.url.clone(),
        event_types: vec!["place.alert".into()],
        ..Default::default()
    };
    assert_eq!(
        ctx.app
            .call::<_, UpdateWebhookResponse>(&format!("{WEBHOOK_SERVICE}/UpdateWebhook"), Some(&limited), &widen)
            .await
            .unwrap_err()
            .reason,
        "INSUFFICIENT_SCOPE"
    );
    // Without dev:write a token manages no webhooks at all; with dev:read it may look.
    let (_, reader) = personal_token(&ctx, &alice, &["dev:read", "chat:read"]).await;
    let reader = with_token(&alice, &reader);
    assert_eq!(create(&reader, &["chat.message_received"]).await.unwrap_err().reason, "INSUFFICIENT_SCOPE");
    let listed: ListWebhooksResponse = ctx
        .app
        .call(&format!("{WEBHOOK_SERVICE}/ListWebhooks"), Some(&reader), &ListWebhooksRequest::default())
        .await
        .unwrap();
    assert_eq!(listed.webhooks.len(), 1);
    // The session of the same person is not limited.
    assert!(create(&alice, &["chat.message_received", "place.alert"]).await.is_ok());
}

#[tokio::test]
async fn a_bot_has_webhooks_of_its_own() {
    let ctx = start().await;
    let (owen, mallory, friend) =
        (ctx.app.user("owen").await, ctx.app.user("mallory").await, ctx.app.user("friend").await);
    let handle = format!("hooked_{}", tokuly_core::ids::random_base62(6).to_lowercase());
    let created: CreateBotResponse = ctx
        .app
        .call(
            &format!("{BOT_SERVICE}/CreateBot"),
            Some(&owen),
            &CreateBotRequest { handle, display_name: "Hooked".into(), ..Default::default() },
        )
        .await
        .unwrap();
    let bot_id = created.bot.as_option().unwrap().user.as_option().unwrap().id.clone();
    let bot = User { id: Uuid::parse_str(&bot_id).unwrap(), ..owen.clone() };
    let receiver = Receiver::start().await;

    // The owner subscribes the bot; a stranger cannot, and a token cannot name a bot.
    let request = CreateWebhookRequest {
        url: receiver.url.clone(),
        event_types: vec!["friend.request_received".into()],
        bot_id: bot_id.clone(),
        ..Default::default()
    };
    assert_eq!(
        ctx.app
            .call::<_, CreateWebhookResponse>(&format!("{WEBHOOK_SERVICE}/CreateWebhook"), Some(&mallory), &request)
            .await
            .unwrap_err()
            .reason,
        "BOT_NOT_FOUND"
    );
    let (_, token) = personal_token(&ctx, &owen, &["dev:write", "social:read"]).await;
    assert_eq!(
        ctx.app
            .call::<_, CreateWebhookResponse>(
                &format!("{WEBHOOK_SERVICE}/CreateWebhook"),
                Some(&with_token(&owen, &token)),
                &request
            )
            .await
            .unwrap_err()
            .reason,
        "SESSION_REQUIRED"
    );
    let created: CreateWebhookResponse =
        ctx.app.call(&format!("{WEBHOOK_SERVICE}/CreateWebhook"), Some(&owen), &request).await.unwrap();
    let webhook = created.webhook.into_option().unwrap();
    assert_eq!(webhook.account_id, bot_id);

    // Listed under the bot, managed by the owner, invisible to others.
    let of_bot: ListWebhooksResponse = ctx
        .app
        .call(
            &format!("{WEBHOOK_SERVICE}/ListWebhooks"),
            Some(&owen),
            &ListWebhooksRequest { bot_id: bot_id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(of_bot.webhooks.len(), 1);
    let own: ListWebhooksResponse = ctx
        .app
        .call(&format!("{WEBHOOK_SERVICE}/ListWebhooks"), Some(&owen), &ListWebhooksRequest::default())
        .await
        .unwrap();
    assert!(own.webhooks.is_empty());
    assert!(get(&ctx, &owen, &webhook.id).await.is_ok());
    assert_eq!(get(&ctx, &mallory, &webhook.id).await.unwrap_err().reason, "WEBHOOK_NOT_FOUND");

    // The bot's events arrive; the owner's own do not.
    assert_eq!(ctx.dev.fan_out(&friend_request(&bot, &friend)).await.unwrap(), 1);
    assert_eq!(ctx.dev.fan_out(&friend_request(&owen, &friend)).await.unwrap(), 0);
    // While its owner is suspended a bot hears nothing, and neither does a suspended account itself.
    let (own, _) = create_webhook(&ctx, &owen, &receiver.url, &["friend.request_received"]).await;
    assert_eq!(ctx.dev.fan_out(&friend_request(&owen, &friend)).await.unwrap(), 1);
    ctx.app.identity.set_banned(owen.id, Some("spam")).await.unwrap();
    assert_eq!(ctx.dev.fan_out(&friend_request(&bot, &friend)).await.unwrap(), 0);
    assert_eq!(ctx.dev.fan_out(&friend_request(&owen, &friend)).await.unwrap(), 0);
    ctx.app.identity.set_banned(owen.id, None).await.unwrap();
    let owen = ctx.app.sign_in(owen.id).await;
    assert_eq!(ctx.dev.fan_out(&friend_request(&bot, &friend)).await.unwrap(), 1);
    ctx.app
        .call::<_, DeleteWebhookResponse>(
            &format!("{WEBHOOK_SERVICE}/DeleteWebhook"),
            Some(&owen),
            &DeleteWebhookRequest { webhook_id: own.id, ..Default::default() },
        )
        .await
        .unwrap();
    // A bot that is switched off hears nothing.
    ctx.app
        .call::<_, SetBotDisabledResponse>(
            &format!("{BOT_SERVICE}/SetBotDisabled"),
            Some(&owen),
            &SetBotDisabledRequest { bot_id: bot_id.clone(), disabled: true, ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(ctx.dev.fan_out(&friend_request(&bot, &friend)).await.unwrap(), 0);
}
