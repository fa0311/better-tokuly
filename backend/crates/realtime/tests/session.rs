//! The session protocol over WebSocket: handshake, authentication, liveness,
//! delivery, subscriptions, limits and connection hygiene.
use std::{sync::Arc, time::Duration};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_tungstenite::tungstenite::Message;
use tokuly_core::{AppResult, presence, realtime::TopicGuard};
use tokuly_proto::{
    common::LatLng,
    geo::{FriendLocation, GeoEvent, geo_event::Event as GeoEventKind},
    realtime::server_frame::Body,
};
use tokuly_realtime::RealtimeConfig;
use uuid::Uuid;

mod support;
use support::*;

/// A small pushed event that is easy to recognise (and must not be lost).
fn marker(text: &str) -> GeoEvent {
    GeoEvent { event: Some(GeoEventKind::LocationRemovedUserId(text.to_owned())), ..Default::default() }
}

fn marker_text(frame: &tokuly_proto::realtime::ServerFrame) -> Option<String> {
    match frame.event::<GeoEvent>()?.event {
        Some(GeoEventKind::LocationRemovedUserId(text)) => Some(text),
        _ => None,
    }
}

// ───────────────────────────── Hello / Welcome ─────────────────────────────

#[tokio::test]
async fn anonymous_hello_is_welcomed() {
    let harness = Harness::start().await;
    let mut client = harness.ws().await;
    let welcome = client.hello("", true).await;
    assert_eq!(welcome.user_id, "");
    assert_eq!(welcome.transport, "websocket");
    assert!(Uuid::parse_str(&welcome.connection_id).is_ok());
    assert!(welcome.server_time.is_set());
    assert_eq!(welcome.ping_interval_seconds, 25);
    assert_eq!(welcome.protocol_version, 2, "the version clients compare with the one they were built for");
    assert_eq!((welcome.moving_interval_seconds, welcome.stationary_interval_seconds), (5, 60));

    let stats = harness.gateway.stats();
    assert_eq!((stats.websocket_connections, stats.authenticated_connections, stats.user_subscriptions), (1, 0, 0));
}

#[tokio::test]
async fn signed_in_hello_is_welcomed_as_the_user() {
    let harness = Harness::start().await;
    let alice = harness.app.user("alice").await;
    let mut client = harness.ws().await;
    let welcome = client.hello(&alice.token, true).await;
    assert_eq!(welcome.user_id, alice.id.to_string());

    let stats = harness.gateway.stats();
    assert_eq!((stats.websocket_connections, stats.authenticated_connections, stats.user_subscriptions), (1, 1, 1));
    assert!(stats.frames_in >= 1 && stats.frames_out >= 1);
}

#[tokio::test]
async fn invalid_tokens_are_fatal_and_say_why() {
    let harness = Harness::start().await;
    let alice = harness.app.user("alice").await;

    let mut client = harness.ws().await;
    let reply = client.request(hello_body("not-a-token", true, &[])).await;
    let error = error_of(&reply);
    assert_eq!((error.code.as_str(), error.message.as_str(), error.fatal), ("unauthenticated", "TOKEN_INVALID", true));
    assert_eq!(u16::from(client.closed().await.expect("close frame").code), 4002);

    let expired = short_lived_token(&harness.app, &alice, -60);
    let mut client = harness.ws().await;
    let reply = client.request(hello_body(&expired, true, &[])).await;
    assert_eq!(error_of(&reply).message, "TOKEN_EXPIRED");

    harness.app.core.revoke_session(alice.session_id).await;
    let mut client = harness.ws().await;
    let reply = client.request(hello_body(&alice.token, true, &[])).await;
    assert_eq!(error_of(&reply).message, "SESSION_REVOKED");
    assert!(error_of(&reply).fatal);
}

#[tokio::test]
async fn only_ping_works_before_hello() {
    let harness = Harness::start().await;
    let mut client = harness.ws().await;

    let reply = client.request(subscribe_body(&["live:1"])).await;
    let error = error_of(&reply);
    assert_eq!((error.code.as_str(), error.message.as_str(), error.fatal), ("failed_precondition", "HELLO_REQUIRED", false));

    let reply = client.request(ping_body(7)).await;
    assert!(matches!(reply.body, Some(Body::Pong(ref pong)) if pong.nonce == 7));

    // The connection is still usable.
    client.hello("", false).await;
}

#[tokio::test]
async fn a_connection_that_never_says_hello_is_closed() {
    let harness = Harness::start_with(RealtimeConfig { hello_timeout: Duration::from_millis(300), ..test_config() }).await;
    let mut client = harness.ws().await;
    let frame = client.next().await;
    let error = error_of(&frame);
    assert_eq!((error.message.as_str(), error.fatal), ("HELLO_TIMEOUT", true));
    assert_eq!(u16::from(client.closed().await.expect("close frame").code), 4002);
}

#[tokio::test]
async fn a_connection_can_sign_in_later_but_not_change_user() {
    let harness = Harness::start().await;
    let alice = harness.app.user("alice").await;
    let bob = harness.app.user("bob").await;

    let mut client = harness.ws_as(None, true).await;
    harness.app.core.rt.to_user(alice.id, &marker("before sign-in")).await.unwrap();
    assert!(client.stays_quiet().await);

    // Anonymous → signed in, on the same connection.
    let welcome = client.hello(&alice.token, true).await;
    assert_eq!(welcome.user_id, alice.id.to_string());
    harness.app.core.rt.to_user(alice.id, &marker("after sign-in")).await.unwrap();
    assert_eq!(marker_text(&client.next().await).as_deref(), Some("after sign-in"));

    // Presenting the same user again (a refreshed token) is fine.
    let again = harness.app.sign_in(alice.id).await;
    assert_eq!(client.hello(&again.token, true).await.user_id, alice.id.to_string());
    assert_eq!(harness.gateway.stats().authenticated_connections, 1);

    // Becoming someone else is not.
    let reply = client.request(hello_body(&bob.token, true, &[])).await;
    let error = error_of(&reply);
    assert_eq!((error.code.as_str(), error.message.as_str(), error.fatal), ("permission_denied", "USER_MISMATCH", true));
}

// ───────────────────────────── Liveness ─────────────────────────────

#[tokio::test]
async fn pings_keep_a_connection_alive_and_silence_ends_it() {
    let cfg = RealtimeConfig { ping_interval: Duration::from_millis(200), idle_timeout: Duration::from_millis(700), ..test_config() };
    let harness = Harness::start_with(cfg).await;
    let mut client = harness.ws_as(None, true).await;

    // Twice the idle timeout, but pinging: still connected.
    for nonce in 1..=7u64 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let reply = client.request(ping_body(nonce)).await;
        let Some(Body::Pong(pong)) = reply.body else { panic!("expected Pong") };
        assert_eq!(pong.nonce, nonce);
        assert!(pong.server_time.is_set());
    }

    // Silent: told why, then closed.
    let frame = client.next().await;
    let error = error_of(&frame);
    assert_eq!((error.message.as_str(), error.fatal), ("IDLE_TIMEOUT", true));
    assert_eq!(u16::from(client.closed().await.expect("close frame").code), 4003);
    eventually("the connection is gone from the stats", async || { harness.gateway.stats().websocket_connections == 0 }).await;
}

// ───────────────────────────── Token lifetime ─────────────────────────────

#[tokio::test]
async fn an_expired_token_can_be_replaced_on_the_same_connection() {
    let harness = Harness::start_with(RealtimeConfig { auth_grace: Duration::from_secs(2), ..test_config() }).await;
    let alice = harness.app.user("alice").await;
    let mut client = harness.ws().await;
    client.hello(&short_lived_token(&harness.app, &alice, 1), true).await;

    // At expiry the gateway asks for a new Hello without dropping the connection.
    let frame = client.next().await;
    let error = error_of(&frame);
    assert_eq!((error.code.as_str(), error.message.as_str(), error.fatal), ("unauthenticated", "TOKEN_EXPIRED", false));
    assert_eq!(frame.reply_to, 0);

    // During the grace period events still flow.
    harness.app.core.rt.to_user(alice.id, &marker("during grace")).await.unwrap();
    assert_eq!(marker_text(&client.next().await).as_deref(), Some("during grace"));

    // A fresh token settles it: well past the grace period the connection lives.
    client.hello(&alice.token, true).await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    harness.app.core.rt.to_user(alice.id, &marker("after refresh")).await.unwrap();
    assert_eq!(marker_text(&client.next().await).as_deref(), Some("after refresh"));
}

#[tokio::test]
async fn an_expired_token_without_refresh_ends_the_connection() {
    let harness = Harness::start_with(RealtimeConfig { auth_grace: Duration::from_millis(500), ..test_config() }).await;
    let alice = harness.app.user("alice").await;
    let mut client = harness.ws().await;
    client.hello(&short_lived_token(&harness.app, &alice, 1), true).await;

    let warning = client.next().await;
    assert!(!error_of(&warning).fatal);
    let last = client.next().await;
    let error = error_of(&last);
    assert_eq!((error.code.as_str(), error.message.as_str(), error.fatal), ("unauthenticated", "TOKEN_EXPIRED", true));
    assert_eq!(u16::from(client.closed().await.expect("close frame").code), 4002);
}

#[tokio::test]
async fn revoking_a_session_ends_its_connections() {
    let harness = Harness::start().await;
    let alice = harness.app.user("alice").await;
    let other_session = harness.app.sign_in(alice.id).await;
    let mut revoked = harness.ws_as(Some(&alice), true).await;
    let mut unaffected = harness.ws_as(Some(&other_session), true).await;

    harness.app.core.revoke_session(alice.session_id).await;
    let frame = revoked.next().await;
    let error = error_of(&frame);
    assert_eq!((error.code.as_str(), error.message.as_str(), error.fatal), ("unauthenticated", "SESSION_REVOKED", true));
    revoked.closed().await;

    // The user's other session is untouched.
    assert!(matches!(unaffected.request(ping_body(1)).await.body, Some(Body::Pong(_))));
}

// ───────────────────────────── Delivery ─────────────────────────────

#[tokio::test]
async fn user_events_reach_every_connection_of_that_user_only() {
    let harness = Harness::start().await;
    let alice = harness.app.user("alice").await;
    let bob = harness.app.user("bob").await;
    let mut phone = harness.ws_as(Some(&alice), true).await;
    let mut laptop = harness.ws_as(Some(&alice), false).await;
    let mut bobs = harness.ws_as(Some(&bob), true).await;
    let mut anonymous = harness.ws_as(None, true).await;

    harness.app.core.rt.to_user(alice.id, &marker("for alice")).await.unwrap();
    for client in [&mut phone, &mut laptop] {
        let frame = client.next().await;
        assert_eq!(marker_text(&frame).as_deref(), Some("for alice"));
        assert_eq!((frame.reply_to, frame.topic.as_str()), (0, ""));
    }
    assert!(bobs.stays_quiet().await);
    assert!(anonymous.stays_quiet().await);

    // Two connections of one user share a single bus subscription.
    let stats = harness.gateway.stats();
    assert_eq!((stats.authenticated_connections, stats.user_subscriptions), (3, 2));

    // The subscription goes when the last connection of the user does.
    phone.close().await;
    laptop.close().await;
    eventually("alice's bus subscription is dropped", async || { harness.gateway.stats().user_subscriptions == 1 }).await;
}

#[tokio::test]
async fn topic_events_follow_subscriptions() {
    let harness = Harness::start().await;
    let topic = unique_topic("live");
    let mut viewer = harness.ws_as(None, true).await;
    let mut bystander = harness.ws_as(None, true).await;

    assert!(is_ack(&viewer.request(subscribe_body(&[&topic])).await));
    harness.app.core.rt.to_topic(&topic, &marker("on air")).await.unwrap();
    let frame = viewer.next_where(|frame| !is_topic_stats(frame)).await;
    assert_eq!((marker_text(&frame).as_deref(), frame.topic.as_str()), (Some("on air"), topic.as_str()));
    assert!(bystander.stays_quiet().await);

    // Subscribing twice changes nothing: one copy per event.
    assert!(is_ack(&viewer.request(subscribe_body(&[&topic])).await));
    harness.app.core.rt.to_topic(&topic, &marker("once")).await.unwrap();
    assert_eq!(marker_text(&viewer.next_where(|frame| !is_topic_stats(frame)).await).as_deref(), Some("once"));
    assert!(viewer.stays_quiet().await);

    assert!(is_ack(&viewer.request(unsubscribe_body(&[&topic])).await));
    harness.app.core.rt.to_topic(&topic, &marker("missed")).await.unwrap();
    assert!(viewer.stays_quiet().await);
    eventually("the topic's bus subscription is dropped", async || { harness.gateway.stats().topic_subscriptions == 0 }).await;
}

#[tokio::test]
async fn hello_can_subscribe_right_away() {
    let harness = Harness::start().await;
    let (first, second) = (unique_topic("live"), unique_topic("party"));
    let mut client = harness.ws().await;
    let reply = client.request(hello_body("", true, &[&first, &second])).await;
    assert!(matches!(reply.body, Some(Body::Welcome(_))));

    harness.app.core.rt.to_topic(&second, &marker("party time")).await.unwrap();
    assert_eq!(marker_text(&client.next_where(|frame| !is_topic_stats(frame)).await).as_deref(), Some("party time"));

    // A Hello with a bad topic is refused as a whole, but may be corrected.
    let mut client = harness.ws().await;
    let reply = client.request(hello_body("", true, &[&first, "not a topic"])).await;
    let error = error_of(&reply);
    assert_eq!((error.code.as_str(), error.message.as_str(), error.fatal), ("invalid_argument", "TOPIC_INVALID", false));
    let reply = client.request(ping_body(1)).await;
    assert!(matches!(reply.body, Some(Body::Pong(_))));
    let reply = client.request(subscribe_body(&[&first])).await;
    assert_eq!(error_of(&reply).message, "HELLO_REQUIRED");
    client.hello("", true).await;
}

#[tokio::test]
async fn subscriptions_are_validated_and_limited() {
    let harness = Harness::start_with(RealtimeConfig { max_topics: 3, ..test_config() }).await;
    let mut client = harness.ws_as(None, true).await;
    let topics: Vec<String> = (0..4).map(|_| unique_topic("live")).collect();
    let names: Vec<&str> = topics.iter().map(String::as_str).collect();

    for invalid in ["live", "live:", "LIVE:1", "live:a b", "live:*", "live:>", "live:a.b"] {
        let reply = client.request(subscribe_body(&[invalid])).await;
        let error = error_of(&reply);
        assert_eq!((error.code.as_str(), error.message.as_str(), error.fatal), ("invalid_argument", "TOPIC_INVALID", false), "{invalid}");
    }

    // One bad name spoils the request: nothing of it is subscribed.
    let reply = client.request(subscribe_body(&[names[0], "nope"])).await;
    assert_eq!(error_of(&reply).message, "TOPIC_INVALID");
    harness.app.core.rt.to_topic(names[0], &marker("unheard")).await.unwrap();
    assert!(client.stays_quiet().await);

    // More than the limit in one request, or in total.
    let reply = client.request(subscribe_body(&names)).await;
    let error = error_of(&reply);
    assert_eq!((error.code.as_str(), error.message.as_str()), ("resource_exhausted", "TOO_MANY_TOPICS"));
    assert!(is_ack(&client.request(subscribe_body(&names[..3])).await));
    let reply = client.request(subscribe_body(&names[3..])).await;
    assert_eq!(error_of(&reply).message, "TOO_MANY_TOPICS");

    // Unsubscribing makes room again; unknown topics are ignored.
    assert!(is_ack(&client.request(unsubscribe_body(&[names[0], "live:unknown"])).await));
    assert!(is_ack(&client.request(subscribe_body(&names[3..])).await));
    assert_eq!(harness.gateway.stats().topic_subscriptions, 3);
}

struct OnlyFor(Uuid);

#[async_trait::async_trait]
impl TopicGuard for OnlyFor {
    async fn may_subscribe(&self, subscriber: Option<Uuid>, _topic: &str) -> AppResult<bool> {
        Ok(subscriber == Some(self.0))
    }
}

#[tokio::test]
async fn guarded_topics_are_only_open_to_those_the_domain_admits() {
    let harness = Harness::start().await;
    let member = harness.app.user("member").await;
    let stranger = harness.app.user("stranger").await;
    // What the domain owning `party:*` topics does when it is constructed.
    harness.app.core.topic_guards.register("party", Arc::new(OnlyFor(member.id)));
    let topic = unique_topic("party");

    for outsider in [None, Some(&stranger)] {
        let mut client = harness.ws_as(outsider, true).await;
        let reply = client.request(subscribe_body(&[&topic])).await;
        let error = error_of(&reply);
        assert_eq!((error.code.as_str(), error.message.as_str(), error.fatal), ("permission_denied", "TOPIC_FORBIDDEN", false));
        harness.app.core.rt.to_topic(&topic, &marker("members only")).await.unwrap();
        assert!(client.stays_quiet().await);
        // Other kinds of topics are unaffected.
        assert!(is_ack(&client.request(subscribe_body(&[&unique_topic("live")])).await));
    }

    let mut client = harness.ws_as(Some(&member), true).await;
    assert!(is_ack(&client.request(subscribe_body(&[&topic])).await));
    // The guard also covers topics named in Hello.
    let mut anonymous = harness.ws().await;
    let reply = anonymous.request(hello_body("", true, &[&topic])).await;
    assert_eq!(error_of(&reply).message, "TOPIC_FORBIDDEN");
}

// ───────────────────────────── Malformed input ─────────────────────────────

#[tokio::test]
async fn malformed_frames_are_answered_not_crashed_on() {
    let harness = Harness::start().await;

    // Not protobuf: an error for "frame 0", the connection stays.
    let mut client = harness.ws_as(None, true).await;
    client.send_raw(Message::Binary(vec![0xff; 12].into())).await;
    let frame = client.next().await;
    let error = error_of(&frame);
    assert_eq!((frame.reply_to, error.code.as_str(), error.message.as_str(), error.fatal), (0, "invalid_argument", "FRAME_INVALID", false));

    // A frame without a body.
    client.send_frame(tokuly_proto::realtime::ClientFrame { id: 77, ..Default::default() }).await;
    let reply = client.reply_to(77).await;
    assert_eq!(error_of(&reply).message, "FRAME_INVALID");
    assert!(harness.gateway.stats().frames_rejected >= 2);

    // Text messages are a protocol violation.
    client.send_raw(Message::text("hello?")).await;
    let frame = client.next().await;
    let error = error_of(&frame);
    assert_eq!((error.message.as_str(), error.fatal), ("BINARY_FRAMES_ONLY", true));
    client.closed().await;

    // So is a frame over the size limit.
    let mut client = harness.ws_as(None, true).await;
    client.send_raw(Message::Binary(vec![0u8; 70 * 1024].into())).await;
    let frame = client.next().await;
    let error = error_of(&frame);
    assert_eq!((error.message.as_str(), error.fatal), ("FRAME_TOO_LARGE", true));
    client.closed().await;
}

// ───────────────────────────── HTTP surface ─────────────────────────────

#[tokio::test]
async fn foreign_origins_are_refused() {
    let harness = Harness::start().await;
    let addr = harness.addrs.websocket;

    let refused = WsClient::connect_with(addr, WsOptions { origin: Some("https://evil.example"), ..Default::default() }).await.err().expect("refused");
    assert!(refused.contains("403"), "{refused}");
    assert_eq!(harness.gateway.stats().connections_refused, 1);

    // The app's own origin, local dev servers (development mode) and
    // clients without an Origin header (native apps) are fine.
    for origin in [Some("http://localhost:8080"), Some("http://localhost:5173"), Some("tauri://localhost"), None] {
        let mut client = WsClient::connect_with(addr, WsOptions { origin, ..Default::default() }).await.unwrap_or_else(|e| panic!("{origin:?}: {e}"));
        client.hello("", true).await;
    }
    // React Native makes up an Origin from the address it connects to.
    let own = format!("http://{addr}");
    assert!(WsClient::connect_with(addr, WsOptions { origin: Some(&own), ..Default::default() }).await.is_ok());
}

#[tokio::test]
async fn healthz_answers_and_other_paths_do_not_upgrade() {
    let harness = Harness::start().await;
    let get = |path: &'static str| async move {
        let mut tcp = tokio::net::TcpStream::connect(harness.addrs.websocket).await.unwrap();
        tcp.write_all(format!("GET {path} HTTP/1.1\r\nHost: gateway\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        let mut response = String::new();
        tcp.read_to_string(&mut response).await.unwrap();
        response
    };
    let health = get("/healthz").await;
    assert!(health.starts_with("HTTP/1.1 200") && health.ends_with("ok"), "{health}");
    assert!(get("/").await.starts_with("HTTP/1.1 404"));
    // A plain GET on the WebSocket path is not an upgrade.
    assert!(!get("/rt/ws").await.starts_with("HTTP/1.1 200"));
}

#[tokio::test]
async fn connections_beyond_the_limit_are_refused() {
    let harness = Harness::start_with(RealtimeConfig { max_connections: 2, ..test_config() }).await;
    let _first = harness.ws_as(None, true).await;
    let second = harness.ws_as(None, true).await;
    let refused = WsClient::connect_with(harness.addrs.websocket, WsOptions::default()).await.err().expect("refused");
    assert!(refused.contains("503"), "{refused}");

    second.close().await;
    eventually("a slot is free again", async || { harness.gateway.stats().websocket_connections == 1 }).await;
    harness.ws_as(None, true).await;
}

#[tokio::test]
async fn one_address_may_only_hold_so_many_connections() {
    let harness = Harness::start_with(RealtimeConfig { max_connections_per_ip: 2, ..test_config() }).await;
    let addr = harness.addrs.websocket;
    let _first = harness.ws_as(None, true).await;
    let second = harness.ws_as(None, true).await;
    let refused = WsClient::connect_with(addr, WsOptions::default()).await.err().expect("refused");
    assert!(refused.contains("429"), "{refused}");
    // WebTransport draws on the same allowance.
    let hashes = harness.endpoints().await.webtransport_cert_hashes;
    assert!(WtClient::connect(&harness.wt_url(), &hashes).await.is_err());
    assert_eq!(harness.gateway.stats().connections_refused, 2);

    // The test connects from loopback, where the gateway expects its edge
    // proxy: clients the proxy names are counted on their own...
    let via_proxy = |forwarded_for: &'static str| WsClient::connect_with(addr, WsOptions { forwarded_for: Some(forwarded_for), ..Default::default() });
    let _third = via_proxy("203.0.113.7").await.expect("another client address");
    // ...by the address the proxy saw, not by what the client claims to the left of it.
    let _fourth = via_proxy("198.51.100.1, 203.0.113.7").await.expect("second connection of that address");
    let refused = via_proxy("198.51.100.2, 203.0.113.7").await.err().expect("third connection of that address");
    assert!(refused.contains("429"), "{refused}");
    assert!(via_proxy("203.0.113.8").await.is_ok());

    // A closed connection frees its slot.
    second.close().await;
    eventually("the slot is free again", async || WsClient::connect_with(addr, WsOptions::default()).await.is_ok()).await;
}

#[tokio::test]
async fn forwarded_addresses_from_untrusted_peers_are_ignored() {
    // Exposed directly, no proxy in front: nobody may name their own address.
    let cfg = RealtimeConfig { max_connections_per_ip: 1, trusted_proxies: Vec::new(), ..test_config() };
    let harness = Harness::start_with(cfg).await;
    let claiming = |forwarded_for: &'static str| {
        WsClient::connect_with(harness.addrs.websocket, WsOptions { forwarded_for: Some(forwarded_for), ..Default::default() })
    };
    let _held = claiming("203.0.113.7").await.expect("the first connection");
    let refused = claiming("203.0.113.8").await.err().expect("posing as someone else does not get around the limit");
    assert!(refused.contains("429"), "{refused}");
}

#[tokio::test]
async fn the_per_address_limit_can_be_switched_off() {
    let harness = Harness::start_with(RealtimeConfig { max_connections_per_ip: 0, ..test_config() }).await;
    let mut clients = Vec::new();
    for _ in 0..12 {
        clients.push(harness.ws_as(None, true).await);
    }
    assert_eq!(harness.gateway.stats().websocket_connections, 12);
}

// ───────────────────────────── Slow consumers ─────────────────────────────

/// A friend's position, padded so that a few hundred of them fill the
/// socket buffers of a client that is not reading.
fn bulky_location(sequence: u32) -> GeoEvent {
    let location = FriendLocation {
        user_id: sequence.to_string(),
        position: LatLng { lat: 35.6595, lng: 139.7005, ..Default::default() }.into(),
        place_name: "x".repeat(48 * 1024),
        ..Default::default()
    };
    GeoEvent { event: Some(GeoEventKind::Location(Box::new(location))), ..Default::default() }
}

#[tokio::test]
async fn a_stalled_client_loses_positions_then_its_connection_without_slowing_others() {
    let cfg = RealtimeConfig { queue_frames: 8, queue_droppable_frames: 4, ..test_config() };
    let harness = Harness::start_with(cfg).await;
    let alice = harness.app.user("alice").await;

    // Two devices of one user: one reads, the other has a tiny receive
    // buffer and never reads, so the gateway's writes to it stall.
    let mut healthy = harness.ws_as(Some(&alice), true).await;
    let mut stalled = WsClient::connect_with(harness.addrs.websocket, WsOptions { receive_buffer: Some(4096), ..Default::default() }).await.unwrap();
    stalled.hello(&alice.token, true).await;

    // Positions are droppable: once the stalled client's socket is full its
    // queue sheds the oldest ones, while the healthy client gets every one.
    let mut sent = 0u32;
    while harness.gateway.stats().frames_dropped == 0 {
        assert!(sent < 2000, "the stalled client never fell behind");
        harness.app.core.rt.to_user(alice.id, &bulky_location(sent)).await.unwrap();
        let frame = healthy.next().await;
        let Some(event) = frame.event::<GeoEvent>() else { panic!("expected the position") };
        let Some(GeoEventKind::Location(location)) = event.event else { panic!("expected a location") };
        assert_eq!(location.user_id, sent.to_string(), "the healthy client misses nothing");
        sent += 1;
    }
    let stats = harness.gateway.stats();
    assert_eq!((stats.slow_consumer_disconnects, stats.websocket_connections), (0, 2), "dropping positions never disconnects");

    // Reliable frames cannot be shed: a backlog over the limit ends the
    // stalled connection. The healthy one keeps receiving in order.
    for sequence in 0..12 {
        harness.app.core.rt.to_user(alice.id, &marker(&format!("reliable {sequence}"))).await.unwrap();
        let frame = healthy.next_where(|frame| marker_text(frame).is_some()).await;
        assert_eq!(marker_text(&frame), Some(format!("reliable {sequence}")));
    }
    eventually("the stalled client is disconnected", async || {
        let stats = harness.gateway.stats();
        stats.slow_consumer_disconnects == 1 && stats.websocket_connections == 1
    })
    .await;
    assert!(matches!(healthy.request(ping_body(1)).await.body, Some(Body::Pong(_))));
    drop(stalled);
}

// ───────────────────────────── Shutdown ─────────────────────────────

#[tokio::test]
async fn shutdown_says_goodbye_and_cleans_up() {
    let mut harness = Harness::start().await;
    let alice = harness.app.user("alice").await;
    let topic = unique_topic("live");
    let mut client = harness.ws_as(Some(&alice), true).await;
    assert!(is_ack(&client.request(subscribe_body(&[&topic])).await));
    let cache = harness.app.core.cache.clone();
    eventually("alice is online and counted", async || {
        presence::is_online(&cache, alice.id).await.unwrap() && presence::topic_audience(&cache, &topic).await.unwrap() == 1
    })
    .await;

    harness.app.core.shutdown.cancel();
    let frame = client.next_where(|frame| matches!(frame.body, Some(Body::Error(_)))).await;
    let error = error_of(&frame);
    assert_eq!((error.code.as_str(), error.message.as_str(), error.fatal), ("unavailable", "SHUTTING_DOWN", true));
    assert_eq!(u16::from(client.closed().await.expect("close frame").code), 4001);

    harness.stopped().await.expect("clean shutdown");
    assert!(!presence::is_online(&cache, alice.id).await.unwrap());
    assert_eq!(presence::topic_audience(&cache, &topic).await.unwrap(), 0);
    assert_eq!(harness.gateway.stats().websocket_connections, 0);
    // The listener is gone.
    assert!(tokio::net::TcpStream::connect(harness.addrs.websocket).await.is_err());
}
