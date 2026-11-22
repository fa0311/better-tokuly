//! What the gateway owes the domains (`tokuly_core::presence`): online
//! markers and announcements, location samples, chat signals and topic
//! audiences.
use std::time::Duration;

use buffa::Message as _;
use chrono::Utc;
use tokuly_core::{
    presence::{
        self, CONVERSATION_PRESENCE_SUBJECT, ConversationPresenceSignal, LOCATION_SUBJECT_PREFIX, PRESENCE_SUBJECT, PresenceChange, TYPING_SUBJECT,
        TypingSignal,
    },
    time::from_ts,
};
use tokuly_proto::{
    common::LatLng,
    geo::{Activity, LocationSample},
    realtime::{ConversationPresence, ServerFrame, Typing, client_frame::Body as ClientBody, server_frame::Body},
};
use tokuly_realtime::{RealtimeConfig, keys};
use uuid::Uuid;

mod support;
use support::*;

fn location_sample(lat: f64, lng: f64) -> LocationSample {
    LocationSample {
        position: LatLng { lat, lng, ..Default::default() }.into(),
        accuracy_meters: 8.5,
        speed_mps: Some(1.4),
        battery_percent: Some(71),
        activity: Activity::ACTIVITY_WALKING.into(),
        foreground: true,
        ..Default::default()
    }
}

/// A `ClientFrame.location` body: the frame carries the sample encoded.
fn location_body(lat: f64, lng: f64) -> ClientBody {
    ClientBody::Location(location_sample(lat, lng).encode_to_vec())
}

fn typing_body(conversation_id: &str, typing: bool) -> ClientBody {
    ClientBody::Typing(Box::new(Typing { conversation_id: conversation_id.to_owned(), typing, ..Default::default() }))
}

fn subscriber_count(frame: &ServerFrame) -> Option<i64> {
    match &frame.body {
        Some(Body::TopicStats(stats)) => Some(stats.subscriber_count),
        _ => None,
    }
}

/// Waits for the announcement about `user_id`; `None` if none comes.
async fn presence_change(bus: &mut async_nats::Subscriber, user_id: Uuid, wait: Duration) -> Option<PresenceChange> {
    next_on_bus(bus, wait, |change: &PresenceChange| change.user_id == user_id).await
}

// ───────────────────────────── Presence ─────────────────────────────

#[tokio::test]
async fn presence_follows_foreground_connections() {
    let harness = Harness::start().await;
    let core = &harness.app.core;
    let alice = harness.app.user("alice").await;
    let mut announcements = core.bus.subscribe(PRESENCE_SUBJECT).await.unwrap();

    // A background connection does not make anyone online.
    let mut background = harness.ws_as(Some(&alice), false).await;
    assert!(presence_change(&mut announcements, alice.id, QUIET).await.is_none());
    assert!(!presence::is_online(&core.cache, alice.id).await.unwrap());

    // The first foreground connection does, exactly once.
    let before = Utc::now();
    let phone = harness.ws_as(Some(&alice), true).await;
    let change = presence_change(&mut announcements, alice.id, WAIT).await.expect("online announcement");
    assert!(change.online && change.at >= before - chrono::Duration::seconds(1));
    assert!(presence::is_online(&core.cache, alice.id).await.unwrap());
    assert_eq!(presence::online_among(&core.cache, &[alice.id, Uuid::now_v7()]).await.unwrap(), vec![alice.id]);

    // More foreground connections change nothing.
    assert!(is_ack(&background.request(app_state_body(true)).await));
    assert!(presence_change(&mut announcements, alice.id, QUIET).await.is_none());

    // Neither does losing one of several.
    phone.close().await;
    assert!(presence_change(&mut announcements, alice.id, QUIET).await.is_none());
    assert!(presence::is_online(&core.cache, alice.id).await.unwrap());

    // Going to the background on the last one means offline...
    assert!(is_ack(&background.request(app_state_body(false)).await));
    let change = presence_change(&mut announcements, alice.id, WAIT).await.expect("offline announcement");
    assert!(!change.online);
    assert!(!presence::is_online(&core.cache, alice.id).await.unwrap());
    let last_seen = presence::last_seen(&core.cache, alice.id).await.unwrap().expect("last seen is recorded");
    assert!((Utc::now() - last_seen).num_seconds() < 10);

    // ...and coming back means online again, until the connection drops.
    assert!(is_ack(&background.request(app_state_body(true)).await));
    assert!(presence_change(&mut announcements, alice.id, WAIT).await.expect("online again").online);
    background.close().await;
    assert!(!presence_change(&mut announcements, alice.id, WAIT).await.expect("offline on disconnect").online);
}

#[tokio::test]
async fn a_user_stays_online_while_connected_to_any_gateway() {
    let harness = Harness::start().await;
    let core = &harness.app.core;
    let (_second_gateway, second_addrs) = harness.another_gateway(test_config()).await;
    let alice = harness.app.user("alice").await;
    let mut announcements = core.bus.subscribe(PRESENCE_SUBJECT).await.unwrap();

    let on_first = harness.ws_as(Some(&alice), true).await;
    assert!(presence_change(&mut announcements, alice.id, WAIT).await.expect("online").online);
    let mut on_second = WsClient::connect(second_addrs.websocket).await;
    on_second.hello(&alice.token, true).await;
    assert!(presence_change(&mut announcements, alice.id, QUIET).await.is_none(), "already online");

    // Leaving one gateway is not going offline...
    on_first.close().await;
    assert!(presence_change(&mut announcements, alice.id, QUIET).await.is_none());
    assert!(presence::is_online(&core.cache, alice.id).await.unwrap());

    // ...leaving the last one is.
    on_second.close().await;
    assert!(!presence_change(&mut announcements, alice.id, WAIT).await.expect("offline").online);
    assert!(!presence::is_online(&core.cache, alice.id).await.unwrap());
}

#[tokio::test]
async fn heartbeats_keep_a_connected_user_online_past_the_lease() {
    let cfg = RealtimeConfig { presence_lease: Duration::from_millis(900), presence_heartbeat: Duration::from_millis(250), ..test_config() };
    let harness = Harness::start_with(cfg).await;
    let core = &harness.app.core;
    let alice = harness.app.user("alice").await;
    let mut announcements = core.bus.subscribe(PRESENCE_SUBJECT).await.unwrap();

    let _client = harness.ws_as(Some(&alice), true).await;
    assert!(presence_change(&mut announcements, alice.id, WAIT).await.expect("online").online);
    // Three lease periods later nobody reaped her and nothing was announced.
    assert!(presence_change(&mut announcements, alice.id, Duration::from_millis(2700)).await.is_none());
    assert!(presence::is_online(&core.cache, alice.id).await.unwrap());
}

#[tokio::test]
async fn users_of_a_dead_gateway_are_announced_offline() {
    let harness = Harness::start().await;
    let core = &harness.app.core;
    let ghost = harness.app.user("ghost").await;
    let mut announcements = core.bus.subscribe(PRESENCE_SUBJECT).await.unwrap();

    // What a gateway leaves behind when it dies with a user online: the
    // online marker and a lease nobody renews anymore.
    presence::mark_online(&core.cache, ghost.id).await.unwrap();
    let expired = Utc::now().timestamp_millis() - 1;
    let mut conn = core.cache.conn();
    redis::pipe()
        .cmd("ZADD").arg(keys::foreground(ghost.id)).arg(expired).arg("dead-gateway").ignore()
        .cmd("ZADD").arg(keys::FOREGROUND_LEASES).arg(expired).arg(ghost.id.simple().to_string()).ignore()
        .query_async::<()>(&mut conn)
        .await
        .unwrap();

    let change = presence_change(&mut announcements, ghost.id, WAIT).await.expect("a surviving gateway announces the user offline");
    assert!(!change.online);
    assert!(!presence::is_online(&core.cache, ghost.id).await.unwrap());
    // Announced once: the lease is gone from the index.
    assert!(presence_change(&mut announcements, ghost.id, Duration::from_millis(800)).await.is_none());
}

// ───────────────────────────── Locations ─────────────────────────────

#[tokio::test]
async fn location_samples_are_handed_to_the_geo_pipeline() {
    let harness = Harness::start().await;
    let core = &harness.app.core;
    let alice = harness.app.user("alice").await;
    let mut samples = core.bus.subscribe(format!("{LOCATION_SUBJECT_PREFIX}.{}", alice.id.simple())).await.unwrap();
    let mut client = harness.ws_as(Some(&alice), true).await;

    let before = Utc::now();
    assert!(is_ack(&client.request(location_body(35.6595, 139.7005)).await));
    let report: LocationReport = next_on_bus(&mut samples, WAIT, |_| true).await.expect("a location report on the bus");
    assert_eq!(report.user_id, alice.id);
    assert!(report.received_at >= before - chrono::Duration::seconds(1));
    let position = report.sample.position.as_option().unwrap();
    assert_eq!((position.lat, position.lng), (35.6595, 139.7005));
    assert_eq!((report.sample.accuracy_meters, report.sample.speed_mps, report.sample.battery_percent), (8.5, Some(1.4), Some(71)));
    assert_eq!(report.sample.activity.as_known(), Some(Activity::ACTIVITY_WALKING));
    // A sample without a device timestamp is stamped on arrival.
    let recorded_at = from_ts(report.sample.recorded_at.as_option().expect("recorded_at is filled in"));
    assert_eq!(recorded_at.timestamp_millis(), report.received_at.timestamp_millis());

    // Samples sent with ID 0 are forwarded without an answer.
    client.send_unanswered(location_body(35.66, 139.70)).await;
    assert!(next_on_bus::<LocationReport>(&mut samples, WAIT, |_| true).await.is_some());
    let ping = client.send(ping_body(1)).await;
    assert_eq!(client.next().await.reply_to, ping, "the pong is the next frame: the sample got no answer");
    assert_eq!(harness.gateway.stats().locations_forwarded, 2);
}

#[tokio::test]
async fn unusable_or_unauthorized_samples_are_refused() {
    let harness = Harness::start().await;
    let core = &harness.app.core;
    let alice = harness.app.user("alice").await;
    let mut samples = core.bus.subscribe(format!("{LOCATION_SUBJECT_PREFIX}.{}", alice.id.simple())).await.unwrap();
    let mut client = harness.ws_as(Some(&alice), true).await;

    for (lat, lng) in [(91.0, 0.0), (0.0, 181.0), (f64::NAN, 139.0)] {
        let reply = client.request(location_body(lat, lng)).await;
        let error = error_of(&reply);
        assert_eq!((error.code.as_str(), error.message.as_str(), error.fatal), ("invalid_argument", "LOCATION_INVALID", false));
    }
    let mut from_the_future = location_sample(35.0, 139.0);
    from_the_future.recorded_at = tokuly_core::time::ts(Utc::now() + chrono::Duration::hours(1));
    let reply = client.request(ClientBody::Location(from_the_future.encode_to_vec())).await;
    assert_eq!(error_of(&reply).message, "LOCATION_INVALID");
    // Bytes that are no sample at all are refused the same way, and the connection goes on.
    let reply = client.request(ClientBody::Location(vec![0xff; 12])).await;
    assert_eq!(error_of(&reply).message, "LOCATION_INVALID");
    assert!(next_on_bus::<LocationReport>(&mut samples, QUIET, |_| true).await.is_none(), "nothing reaches the pipeline");

    let mut anonymous = harness.ws_as(None, true).await;
    let reply = anonymous.request(location_body(35.0, 139.0)).await;
    let error = error_of(&reply);
    assert_eq!((error.code.as_str(), error.message.as_str(), error.fatal), ("unauthenticated", "SIGN_IN_REQUIRED", false));
}

#[tokio::test]
async fn location_samples_are_rate_limited_per_connection() {
    let harness = Harness::start_with(RealtimeConfig { location_burst: 3, location_rate: 0.5, ..test_config() }).await;
    let alice = harness.app.user("alice").await;
    let mut client = harness.ws_as(Some(&alice), true).await;
    for _ in 0..3 {
        assert!(is_ack(&client.request(location_body(35.0, 139.0)).await));
    }
    let reply = client.request(location_body(35.0, 139.0)).await;
    let error = error_of(&reply);
    assert_eq!((error.code.as_str(), error.message.as_str(), error.fatal), ("resource_exhausted", "RATE_LIMITED", false));

    // The limit is per connection, not per user.
    let mut other_device = harness.ws_as(Some(&alice), true).await;
    assert!(is_ack(&other_device.request(location_body(35.0, 139.0)).await));
    assert_eq!(harness.gateway.stats().locations_forwarded, 4);
}

// ───────────────────────────── Chat signals ─────────────────────────────

#[tokio::test]
async fn typing_and_conversation_presence_are_forwarded_to_chat() {
    let harness = Harness::start().await;
    let core = &harness.app.core;
    let alice = harness.app.user("alice").await;
    let conversation = Uuid::now_v7();
    let mut typing = core.bus.subscribe(TYPING_SUBJECT).await.unwrap();
    let mut co_presence = core.bus.subscribe(CONVERSATION_PRESENCE_SUBJECT).await.unwrap();
    let mut client = harness.ws_as(Some(&alice), true).await;

    assert!(is_ack(&client.request(typing_body(&conversation.to_string(), true)).await));
    let signal: TypingSignal = next_on_bus(&mut typing, WAIT, |signal: &TypingSignal| signal.user_id == alice.id).await.expect("typing signal");
    assert_eq!((signal.conversation_id, signal.typing), (conversation, true));
    assert!(is_ack(&client.request(typing_body(&conversation.to_string(), false)).await));
    let signal: TypingSignal = next_on_bus(&mut typing, WAIT, |signal: &TypingSignal| signal.user_id == alice.id).await.expect("stopped typing");
    assert!(!signal.typing);

    let opened = ClientBody::ConversationPresence(Box::new(ConversationPresence {
        conversation_id: conversation.to_string(),
        present: true,
        ..Default::default()
    }));
    assert!(is_ack(&client.request(opened).await));
    let signal: ConversationPresenceSignal =
        next_on_bus(&mut co_presence, WAIT, |signal: &ConversationPresenceSignal| signal.user_id == alice.id).await.expect("presence signal");
    assert_eq!((signal.conversation_id, signal.present), (conversation, true));

    // Malformed IDs and anonymous senders are refused.
    let reply = client.request(typing_body("not-a-uuid", true)).await;
    let error = error_of(&reply);
    assert_eq!((error.code.as_str(), error.message.as_str(), error.fatal), ("invalid_argument", "CONVERSATION_ID_INVALID", false));
    let mut anonymous = harness.ws_as(None, true).await;
    let reply = anonymous.request(typing_body(&conversation.to_string(), true)).await;
    assert_eq!(error_of(&reply).message, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn chat_signals_are_rate_limited() {
    let harness = Harness::start().await;
    let alice = harness.app.user("alice").await;
    let conversation = Uuid::now_v7().to_string();
    let mut client = harness.ws_as(Some(&alice), true).await;
    let mut refused = 0;
    for _ in 0..40 {
        let reply = client.request(typing_body(&conversation, true)).await;
        if !is_ack(&reply) {
            assert_eq!(error_of(&reply).message, "RATE_LIMITED");
            refused += 1;
        }
    }
    assert!((10..=20).contains(&refused), "a burst of 20 is allowed, the rest refused (refused {refused})");
}

// ───────────────────────────── Topic audiences ─────────────────────────────

#[tokio::test]
async fn audiences_are_counted_and_pushed_to_subscribers() {
    let harness = Harness::start().await;
    let cache = &harness.app.core.cache;
    let topic = unique_topic("live");
    let audience = async || presence::topic_audience(cache, &topic).await.unwrap();

    let mut first = harness.ws_as(None, true).await;
    assert!(is_ack(&first.request(subscribe_body(&[&topic])).await));
    let frame = first.next_where(is_topic_stats).await;
    assert_eq!((subscriber_count(&frame), frame.topic.as_str()), (Some(1), topic.as_str()));
    assert_eq!(audience().await, 1);

    // A second viewer: both learn the new count.
    let alice = harness.app.user("alice").await;
    let mut second = harness.ws_as(Some(&alice), true).await;
    assert!(is_ack(&second.request(subscribe_body(&[&topic])).await));
    assert_eq!(subscriber_count(&second.next_where(is_topic_stats).await), Some(2));
    assert_eq!(subscriber_count(&first.next_where(|frame| subscriber_count(frame) == Some(2)).await), Some(2));
    assert_eq!(audience().await, 2);

    // One leaves by disconnecting, the other by unsubscribing.
    second.close().await;
    assert_eq!(subscriber_count(&first.next_where(is_topic_stats).await), Some(1));
    assert_eq!(audience().await, 1);
    assert!(is_ack(&first.request(unsubscribe_body(&[&topic])).await));
    eventually("the audience is empty", async || { audience().await == 0 }).await;
}

#[tokio::test]
async fn audience_pushes_are_throttled() {
    let cfg = RealtimeConfig { topic_stats_interval: Duration::from_millis(600), ..test_config() };
    let harness = Harness::start_with(cfg).await;
    let topic = unique_topic("live");
    let mut watcher = harness.ws_as(None, true).await;
    assert!(is_ack(&watcher.request(subscribe_body(&[&topic])).await));
    assert_eq!(subscriber_count(&watcher.next_where(is_topic_stats).await), Some(1));

    // Five viewers arrive in a burst...
    let _viewers = futures::future::join_all((0..5).map(async |_| {
        let mut viewer = harness.ws_as(None, true).await;
        assert!(is_ack(&viewer.request(subscribe_body(&[&topic])).await));
        viewer
    }))
    .await;
    // ...and the watcher is not told five times: the burst collapses into
    // one push with the final count (two if the burst straddled an interval).
    let mut pushes = 0;
    loop {
        pushes += 1;
        if subscriber_count(&watcher.next_where(is_topic_stats).await) == Some(6) {
            break;
        }
    }
    assert!(pushes <= 2, "{pushes} pushes for one burst");
    assert!(watcher.next_within(QUIET).await.is_none());
}

#[tokio::test]
async fn audiences_add_up_across_gateways() {
    let harness = Harness::start().await;
    let cache = &harness.app.core.cache;
    let (_second_gateway, second_addrs) = harness.another_gateway(test_config()).await;
    let topic = unique_topic("live");

    let mut here = harness.ws_as(None, true).await;
    assert!(is_ack(&here.request(subscribe_body(&[&topic])).await));
    assert_eq!(subscriber_count(&here.next_where(is_topic_stats).await), Some(1));

    let mut there = WsClient::connect(second_addrs.websocket).await;
    there.hello("", true).await;
    assert!(is_ack(&there.request(subscribe_body(&[&topic])).await));
    // The count published by the other gateway reaches subscribers on both.
    assert_eq!(subscriber_count(&there.next_where(is_topic_stats).await), Some(2));
    assert_eq!(subscriber_count(&here.next_where(is_topic_stats).await), Some(2));
    assert_eq!(presence::topic_audience(cache, &topic).await.unwrap(), 2);

    there.close().await;
    assert_eq!(subscriber_count(&here.next_where(is_topic_stats).await), Some(1));
}

#[tokio::test]
async fn totals_that_change_unannounced_are_pushed_at_the_next_refresh() {
    let harness = Harness::start().await;
    let cache = &harness.app.core.cache;
    let topic = unique_topic("live");
    // Another gateway reports five viewers of its own.
    presence::set_topic_audience(cache, &topic, "another-gateway", 5).await.unwrap();

    let mut viewer = harness.ws_as(None, true).await;
    assert!(is_ack(&viewer.request(subscribe_body(&[&topic])).await));
    assert_eq!(subscriber_count(&viewer.next_where(is_topic_stats).await), Some(6));

    // Its report disappears without a word (it went away; a stale report
    // dropping out of the sum looks the same). Nobody published a new
    // count, yet the viewer learns it when this gateway next refreshes.
    presence::set_topic_audience(cache, &topic, "another-gateway", 0).await.unwrap();
    assert_eq!(subscriber_count(&viewer.next_where(is_topic_stats).await), Some(1));
    assert_eq!(presence::topic_audience(cache, &topic).await.unwrap(), 1);

    // While nothing changes, refreshes are silent.
    assert!(viewer.next_within(Duration::from_millis(1200)).await.is_none());
    assert_eq!(presence::topic_audience(cache, &topic).await.unwrap(), 1);
}
