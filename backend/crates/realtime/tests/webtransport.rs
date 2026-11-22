//! The WebTransport transport and `GetEndpoints`: pinned certificates,
//! length-prefixed frames on the stream, datagrams, certificate rotation.
use std::time::Duration;

use buffa::Message as _;
use tokuly_core::presence::LOCATION_SUBJECT_PREFIX;
use tokuly_proto::{
    common::LatLng,
    geo::{FriendLocation, GeoEvent, LocationSample, geo_event::Event as GeoEventKind},
    realtime::{ServerFrame, client_frame::Body as ClientBody, server_frame::Body},
};
use tokuly_realtime::RealtimeConfig;

mod support;
use support::*;

/// A small pushed event that is easy to recognise (and must not be lost).
fn marker(text: &str) -> GeoEvent {
    GeoEvent { event: Some(GeoEventKind::LocationRemovedUserId(text.to_owned())), ..Default::default() }
}

fn marker_text(frame: &ServerFrame) -> Option<String> {
    match frame.event::<GeoEvent>()?.event {
        Some(GeoEventKind::LocationRemovedUserId(text)) => Some(text),
        _ => None,
    }
}

fn friend_location(friend: &str, place_name: &str) -> GeoEvent {
    let location = FriendLocation {
        user_id: friend.to_owned(),
        position: LatLng { lat: 35.6595, lng: 139.7005, ..Default::default() }.into(),
        place_name: place_name.to_owned(),
        ..Default::default()
    };
    GeoEvent { event: Some(GeoEventKind::Location(Box::new(location))), ..Default::default() }
}

fn located_friend(frame: &ServerFrame) -> Option<String> {
    match frame.event::<GeoEvent>()?.event {
        Some(GeoEventKind::Location(location)) => Some(location.user_id),
        _ => None,
    }
}

fn location_body(lat: f64, lng: f64) -> ClientBody {
    let sample = LocationSample { position: LatLng { lat, lng, ..Default::default() }.into(), accuracy_meters: 5.0, ..Default::default() };
    ClientBody::Location(sample.encode_to_vec())
}

// ───────────────────────────── The stream ─────────────────────────────

#[tokio::test]
async fn hello_and_delivery_over_the_stream() {
    let harness = Harness::start().await;
    let alice = harness.app.user("alice").await;
    let mut client = harness.wt().await;

    let welcome = client.hello(&alice.token, true).await;
    assert_eq!((welcome.transport.as_str(), welcome.user_id), ("webtransport", alice.id.to_string()));
    let stats = harness.gateway.stats();
    assert_eq!((stats.webtransport_connections, stats.websocket_connections, stats.authenticated_connections), (1, 0, 1));

    // User-scoped events arrive on the stream, in order.
    for text in ["one", "two", "three"] {
        harness.app.core.rt.to_user(alice.id, &marker(text)).await.unwrap();
    }
    for text in ["one", "two", "three"] {
        assert_eq!(marker_text(&client.next().await).as_deref(), Some(text));
    }

    // So do topic events, after subscribing.
    let topic = unique_topic("live");
    assert!(is_ack(&client.request(subscribe_body(&[&topic])).await));
    harness.app.core.rt.to_topic(&topic, &marker("on air")).await.unwrap();
    let frame = loop {
        let frame = client.next().await;
        if !is_topic_stats(&frame) {
            break frame;
        }
    };
    assert_eq!((marker_text(&frame).as_deref(), frame.topic.as_str()), (Some("on air"), topic.as_str()));

    let reply = client.request(ping_body(42)).await;
    assert!(matches!(reply.body, Some(Body::Pong(ref pong)) if pong.nonce == 42));
}

#[tokio::test]
async fn anonymous_and_invalid_hellos_behave_as_on_websocket() {
    let harness = Harness::start().await;
    let mut anonymous = harness.wt().await;
    assert_eq!(anonymous.hello("", false).await.user_id, "");

    let mut intruder = harness.wt().await;
    let reply = intruder.request(hello_body("forged", true, &[])).await;
    let error = error_of(&reply);
    assert_eq!((error.code.as_str(), error.message.as_str(), error.fatal), ("unauthenticated", "TOKEN_INVALID", true));
    // The fatal error was delivered before the session was closed.
    let closed = tokio::time::timeout(WAIT, intruder.connection.closed()).await;
    assert!(closed.is_ok(), "the gateway closes the session");
}

#[tokio::test]
async fn oversized_stream_frames_end_the_session() {
    let harness = Harness::start().await;
    let mut client = harness.wt().await;
    client.hello("", true).await;
    // Only the prefix is needed to know the frame is too large.
    client.send_raw(&vec![0u8; 80 * 1024]).await;
    let frame = client.next().await;
    let error = error_of(&frame);
    assert_eq!((error.message.as_str(), error.fatal), ("FRAME_TOO_LARGE", true));
}

// ───────────────────────────── Datagrams ─────────────────────────────

#[tokio::test]
async fn locations_travel_as_datagrams_in_both_directions() {
    let harness = Harness::start().await;
    let core = &harness.app.core;
    let alice = harness.app.user("alice").await;
    let mut samples = core.bus.subscribe(format!("{LOCATION_SUBJECT_PREFIX}.{}", alice.id.simple())).await.unwrap();
    let mut client = harness.wt().await;
    client.hello(&alice.token, true).await;

    // Up: a location sample in a datagram reaches the geo pipeline.
    client.send_datagram(0, location_body(35.6595, 139.7005));
    let report: LocationReport = next_on_bus(&mut samples, WAIT, |_| true).await.expect("the sample reaches the bus");
    assert_eq!(report.user_id, alice.id);
    assert_eq!(report.sample.position.as_option().map(|p| (p.lat, p.lng)), Some((35.6595, 139.7005)));

    // Down: a friend's position arrives as a datagram, not on the stream.
    core.rt.to_user(alice.id, &friend_location("friend-1", "")).await.unwrap();
    assert_eq!(located_friend(&client.next_datagram().await).as_deref(), Some("friend-1"));
    assert!(client.next_within(QUIET).await.is_none(), "nothing on the stream");

    // Everything else stays on the stream, even other geo events.
    core.rt.to_user(alice.id, &marker("friend-1")).await.unwrap();
    assert_eq!(marker_text(&client.next().await).as_deref(), Some("friend-1"));

    // A position too large for a datagram falls back to the stream.
    core.rt.to_user(alice.id, &friend_location("friend-2", &"x".repeat(4000))).await.unwrap();
    assert_eq!(located_friend(&client.next().await).as_deref(), Some("friend-2"));

    let stats = harness.gateway.stats();
    assert_eq!((stats.datagrams_in, stats.datagrams_out, stats.locations_forwarded), (1, 1, 1));
}

#[tokio::test]
async fn datagrams_carry_pings_and_locations_only() {
    let harness = Harness::start().await;
    let alice = harness.app.user("alice").await;
    let mut client = harness.wt().await;
    client.hello(&alice.token, true).await;

    // A datagram ping is answered by a datagram pong.
    client.send_datagram(9, ping_body(1234));
    let pong = client.next_datagram().await;
    assert_eq!(pong.reply_to, 9);
    assert!(matches!(pong.body, Some(Body::Pong(ref pong)) if pong.nonce == 1234));

    // A sample with an ID is acknowledged, reliably.
    client.send_datagram(10, location_body(35.0, 139.0));
    assert!(is_ack(&client.reply_to(10).await));

    // Anything else has no business in a datagram.
    let topic = unique_topic("live");
    client.send_datagram(11, subscribe_body(&[&topic]));
    let reply = client.reply_to(11).await;
    let error = error_of(&reply);
    assert_eq!((error.code.as_str(), error.message.as_str(), error.fatal), ("invalid_argument", "DATAGRAM_NOT_ALLOWED", false));
    assert_eq!(harness.gateway.stats().topic_subscriptions, 0);

    // Garbage in a datagram is dropped silently; the session carries on.
    client.connection.send_datagram([0xffu8; 8]).unwrap();
    let reply = client.request(ping_body(1)).await;
    assert!(matches!(reply.body, Some(Body::Pong(_))));
}

#[tokio::test]
async fn websocket_clients_get_positions_on_their_only_channel() {
    let harness = Harness::start().await;
    let alice = harness.app.user("alice").await;
    let mut client = harness.ws_as(Some(&alice), true).await;
    harness.app.core.rt.to_user(alice.id, &friend_location("friend-1", "")).await.unwrap();
    assert_eq!(located_friend(&client.next().await).as_deref(), Some("friend-1"));
    assert_eq!(harness.gateway.stats().datagrams_out, 0);
}

// ───────────────────────────── Endpoints and certificates ─────────────────────────────

#[tokio::test]
async fn get_endpoints_returns_configured_urls_and_the_presented_certificate() {
    let cfg = RealtimeConfig {
        public_wt_url: Some("https://rt.tokuly.example:4433/rt".into()),
        public_ws_url: Some("wss://rt.tokuly.example/rt/ws".into()),
        ..test_config()
    };
    let harness = Harness::start_with(cfg).await;
    let endpoints = harness.endpoints().await;
    assert_eq!(endpoints.webtransport_url, "https://rt.tokuly.example:4433/rt");
    assert_eq!(endpoints.websocket_url, "wss://rt.tokuly.example/rt/ws");
    assert!(!endpoints.webtransport_cert_hashes.is_empty());
    assert!(endpoints.webtransport_cert_hashes.iter().all(|hash| hash.len() == 32));

    // A client pinning those hashes connects, and what the listener
    // presented is one of them.
    let client = WtClient::connect(&harness.wt_url(), &endpoints.webtransport_cert_hashes).await.expect("pinned connection");
    assert!(endpoints.webtransport_cert_hashes.contains(&client.presented_cert_hash()));

    // Pinning anything else fails the handshake.
    assert!(WtClient::connect(&harness.wt_url(), &[vec![7u8; 32]]).await.is_err());
}

#[tokio::test]
async fn get_endpoints_defaults_follow_the_public_url() {
    // An API process: registers the service but runs no gateway.
    let app = tokuly_testkit::TestApp::start(&["0001", "0100", "0400"], |core, _identity, router| {
        tokuly_realtime::Gateway::new(core.clone(), RealtimeConfig::default()).unwrap().register(router)
    })
    .await;
    let endpoints: tokuly_proto::realtime::GetEndpointsResponse = app
        .call("tokuly.realtime.v1.RealtimeService/GetEndpoints", None, &tokuly_proto::realtime::GetEndpointsRequest::default())
        .await
        .unwrap();
    // The testkit's public URL is http://localhost:8080, in development mode.
    assert_eq!(endpoints.webtransport_url, "https://localhost:4433/rt");
    assert_eq!(endpoints.websocket_url, "ws://localhost:8082/rt/ws");
}

#[tokio::test]
async fn disabled_webtransport_is_not_advertised() {
    let harness = Harness::start_with(RealtimeConfig { wt_listen: String::new(), ..test_config() }).await;
    assert_eq!(harness.addrs.webtransport, None);
    let endpoints = harness.endpoints().await;
    assert_eq!(endpoints.webtransport_url, "");
    assert!(endpoints.webtransport_cert_hashes.is_empty());
    assert!(endpoints.websocket_url.ends_with("/rt/ws"));
    // WebSocket still works.
    harness.ws_as(None, true).await;
}

#[tokio::test]
async fn certificate_rotation_never_strands_a_pinning_client() {
    let harness = Harness::start().await;
    let url = harness.wt_url();

    let before = harness.endpoints().await.webtransport_cert_hashes;
    let old = WtClient::connect(&url, &before).await.unwrap().presented_cert_hash();

    // Announced: clients asking now learn both hashes, the listener still
    // presents the old certificate.
    harness.gateway.announce_next_certificate().await.unwrap();
    let announced = harness.endpoints().await.webtransport_cert_hashes;
    assert!(announced.contains(&old));
    assert!(announced.len() > before.iter().filter(|hash| announced.contains(hash)).count(), "a new hash was added");
    let mut long_lived = WtClient::connect(&url, &announced).await.unwrap();
    assert_eq!(long_lived.presented_cert_hash(), old);
    long_lived.hello("", true).await;

    // Switched: the client that fetched hashes before the switch still gets in...
    harness.gateway.activate_next_certificate().await.unwrap();
    let new = WtClient::connect(&url, &announced).await.expect("hashes fetched before the switch still work").presented_cert_hash();
    assert_ne!(new, old);
    // ...the retired hash is no longer handed out, the new one is...
    let after = harness.endpoints().await.webtransport_cert_hashes;
    assert!(after.contains(&new) && !after.contains(&old));
    assert!(WtClient::connect(&url, &[old]).await.is_err(), "the old certificate is no longer presented");
    // ...and connections opened before the switch are not disturbed.
    let reply = long_lived.request(ping_body(5)).await;
    assert!(matches!(reply.body, Some(Body::Pong(_))));
}

#[tokio::test]
async fn certificates_rotate_on_schedule() {
    // Eight seconds of validity: announced after 4.8 s, switched after 6 s.
    let harness = Harness::start_with(RealtimeConfig { cert_validity: Duration::from_secs(8), ..test_config() }).await;
    let url = harness.wt_url();
    let first = harness.wt().await.presented_cert_hash();

    // Keep asking for hashes and connecting, like clients arriving over time.
    // A client whose hashes went stale between asking and connecting asks
    // again, as real clients do; that must always work.
    let connect = async || {
        for _attempt in 0..2 {
            let hashes = harness.endpoints().await.webtransport_cert_hashes;
            if let Ok(client) = WtClient::connect(&url, &hashes).await {
                return (hashes, client.presented_cert_hash());
            }
        }
        panic!("a client with fresh hashes could not connect");
    };
    let mut hash_lists = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    let second = loop {
        assert!(tokio::time::Instant::now() < deadline, "the certificate was never replaced");
        let (hashes, presented) = connect().await;
        hash_lists.push(hashes);
        if presented != first {
            break presented;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    // The successor was handed out while the first certificate was still in use.
    assert!(hash_lists.iter().any(|hashes| hashes.contains(&first) && hashes.contains(&second)));
    eventually("the retired hash is no longer handed out", async || !harness.endpoints().await.webtransport_cert_hashes.contains(&first)).await;
}

#[tokio::test]
async fn an_operator_certificate_is_presented_and_not_pinned() {
    let dir = tempfile::tempdir().unwrap();
    let (cert, key) = (dir.path().join("cert.pem"), dir.path().join("key.pem"));
    let identity = wtransport::Identity::self_signed(["localhost", "127.0.0.1"]).unwrap();
    identity.certificate_chain().store_pemfile(&cert).await.unwrap();
    identity.private_key().store_secret_pemfile(&key).await.unwrap();
    let harness = Harness::start_with(RealtimeConfig { tls_cert: Some(cert), tls_key: Some(key), ..test_config() }).await;

    // A publicly trusted certificate needs no pinning: no hashes are handed out.
    let endpoints = harness.endpoints().await;
    assert!(endpoints.webtransport_cert_hashes.is_empty());
    assert!(!endpoints.webtransport_url.is_empty());

    // The test certificate happens to be pinnable, which lets us check that
    // it is the one presented.
    let hash = identity.certificate_chain().as_slice()[0].hash().as_ref().to_vec();
    let mut client = WtClient::connect(&harness.wt_url(), std::slice::from_ref(&hash)).await.expect("connection with the operator's certificate");
    assert_eq!(client.presented_cert_hash(), hash);
    assert_eq!(client.hello("", true).await.transport, "webtransport");
}

#[tokio::test]
async fn a_missing_certificate_file_fails_startup_with_a_clear_error() {
    let cfg = RealtimeConfig { tls_cert: Some("/nonexistent/cert.pem".into()), tls_key: Some("/nonexistent/key.pem".into()), ..test_config() };
    let app = tokuly_testkit::TestApp::start(&["0001", "0100", "0400"], |_, _, router| router).await;
    let gateway = tokuly_realtime::Gateway::new(app.core.clone(), cfg).unwrap();
    let error = gateway.run().await.expect_err("run() refuses to start");
    assert!(error.to_string().contains("/nonexistent/cert.pem"), "{error}");
}

#[tokio::test]
async fn certificate_hashes_are_withdrawn_on_shutdown() {
    let mut harness = Harness::start().await;
    let mut client = harness.wt().await;
    let presented = client.presented_cert_hash();
    client.hello("", true).await;

    harness.app.core.shutdown.cancel();
    // The goodbye arrives before the session is closed.
    let frame = client.next().await;
    let error = error_of(&frame);
    assert_eq!((error.code.as_str(), error.message.as_str(), error.fatal), ("unavailable", "SHUTTING_DOWN", true));
    tokio::time::timeout(WAIT, client.connection.closed()).await.expect("the session is closed");

    harness.stopped().await.expect("clean shutdown");
    assert!(!harness.endpoints().await.webtransport_cert_hashes.contains(&presented));
    assert_eq!(harness.gateway.stats().webtransport_connections, 0);
}

// ───────────────────────────── Session requests ─────────────────────────────

#[tokio::test]
async fn sessions_on_other_paths_or_from_foreign_origins_are_refused() {
    let harness = Harness::start().await;
    let hashes = harness.endpoints().await.webtransport_cert_hashes;
    let port = harness.addrs.webtransport.unwrap().port();

    assert!(WtClient::connect(&format!("https://127.0.0.1:{port}/elsewhere"), &hashes).await.is_err());
    assert_eq!(harness.gateway.stats().connections_refused, 1);
    // A query string does not change the path.
    assert!(WtClient::connect(&format!("https://127.0.0.1:{port}/rt?v=1"), &hashes).await.is_ok());

    let connect_from = |origin: &'static str| {
        let (hashes, url) = (hashes.clone(), harness.wt_url());
        async move {
            let digests = hashes.iter().map(|hash| wtransport::tls::Sha256Digest::new(hash.as_slice().try_into().unwrap()));
            let config = wtransport::ClientConfig::builder().with_bind_address("127.0.0.1:0".parse().unwrap()).with_server_certificate_hashes(digests).build();
            let options = wtransport::endpoint::ConnectOptions::builder(url).add_header("origin", origin).build();
            tokio::time::timeout(Duration::from_secs(5), wtransport::Endpoint::client(config).unwrap().connect(options)).await.unwrap()
        }
    };
    assert!(connect_from("https://evil.example").await.is_err());
    assert!(connect_from("http://localhost:5173").await.is_ok());
    assert_eq!(harness.gateway.stats().connections_refused, 2);
}

#[tokio::test]
async fn sessions_over_the_per_address_limit_are_refused() {
    let harness = Harness::start_with(RealtimeConfig { max_connections_per_ip: 1, ..test_config() }).await;
    let (url, hashes) = (harness.wt_url(), harness.endpoints().await.webtransport_cert_hashes);
    let mut first = WtClient::connect(&url, &hashes).await.unwrap();
    first.hello("", true).await;

    assert!(WtClient::connect(&url, &hashes).await.is_err());
    // WebSocket connections count towards the same allowance.
    let refused = WsClient::connect_with(harness.addrs.websocket, WsOptions::default()).await.err().expect("refused");
    assert!(refused.contains("429"), "{refused}");
    assert_eq!(harness.gateway.stats().connections_refused, 2);

    // The slot comes back when the session ends.
    first.connection.close(wtransport::VarInt::from_u32(0), b"done");
    eventually("the slot is free again", async || WtClient::connect(&url, &hashes).await.is_ok()).await;
}

#[tokio::test]
async fn a_session_that_never_opens_its_stream_is_dropped() {
    let harness = Harness::start_with(RealtimeConfig { hello_timeout: Duration::from_millis(400), ..test_config() }).await;
    let hashes = harness.endpoints().await.webtransport_cert_hashes;
    let digests = hashes.iter().map(|hash| wtransport::tls::Sha256Digest::new(hash.as_slice().try_into().unwrap()));
    let config = wtransport::ClientConfig::builder().with_bind_address("127.0.0.1:0".parse().unwrap()).with_server_certificate_hashes(digests).build();
    let connection = wtransport::Endpoint::client(config).unwrap().connect(harness.wt_url()).await.unwrap();
    tokio::time::timeout(WAIT, connection.closed()).await.expect("the gateway gives up on the session");
    assert_eq!(harness.gateway.stats().webtransport_connections, 0);
}
