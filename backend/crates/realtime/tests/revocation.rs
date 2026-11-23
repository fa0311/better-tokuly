//! Subscriptions the server ends: revocations published by the domains
//! (`Realtime::revoke`, `revoke_prefix`, `recheck_topic`, `close_topic`) and
//! the periodic re-check that heals a revocation that got lost.
use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use parking_lot::Mutex;
use tokio::sync::Notify;
use tokuly_core::{AppResult, realtime::TopicGuard};
use tokuly_proto::{
    geo::{GeoEvent, geo_event::Event as GeoEventKind},
    realtime::{ServerFrame, server_frame::Body},
};
use tokuly_realtime::RealtimeConfig;
use uuid::Uuid;

mod support;
use support::*;

/// How soon a revocation must take effect.
const PROMPTLY: Duration = Duration::from_secs(1);

fn marker(text: &str) -> GeoEvent {
    GeoEvent { event: Some(GeoEventKind::LocationRemovedUserId(text.to_owned())), ..Default::default() }
}

fn marker_text(frame: &ServerFrame) -> Option<String> {
    match frame.event::<GeoEvent>()?.event {
        Some(GeoEventKind::LocationRemovedUserId(text)) => Some(text),
        _ => None,
    }
}

/// Admits the users in its set (and nobody anonymous); the set can change
/// as a domain's state would. Optionally holds the first answer until
/// released, to stage a revocation in the middle of a subscription.
#[derive(Default)]
struct Members {
    admitted: Mutex<HashSet<Uuid>>,
    calls: AtomicUsize,
    hold_first: Option<(Notify, Notify)>,
}

impl Members {
    fn of(users: &[Uuid]) -> Arc<Self> {
        Arc::new(Self { admitted: Mutex::new(users.iter().copied().collect()), ..Default::default() })
    }

    fn holding_first(users: &[Uuid]) -> Arc<Self> {
        Arc::new(Self {
            admitted: Mutex::new(users.iter().copied().collect()),
            hold_first: Some((Notify::new(), Notify::new())),
            ..Default::default()
        })
    }

    fn remove(&self, user: Uuid) {
        self.admitted.lock().remove(&user);
    }
}

#[async_trait::async_trait]
impl TopicGuard for Members {
    async fn may_subscribe(&self, subscriber: Option<Uuid>, _topic: &str) -> AppResult<bool> {
        let verdict = subscriber.is_some_and(|user| self.admitted.lock().contains(&user));
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0
            && let Some((entered, release)) = &self.hold_first
        {
            entered.notify_one();
            release.notified().await;
        }
        Ok(verdict)
    }
}

/// The next `Unsubscribed` within [`PROMPTLY`]: its topic and reason.
async fn unsubscribed(client: &mut WsClient) -> Option<(String, String)> {
    let deadline = tokio::time::Instant::now() + PROMPTLY;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let frame = client.next_within(remaining).await?;
        if let Some(Body::Unsubscribed(unsubscribed)) = &frame.body {
            assert_eq!(frame.topic, unsubscribed.topic, "the frame names its topic");
            return Some((unsubscribed.topic.clone(), unsubscribed.reason.clone()));
        }
    }
}

async fn receives(client: &mut WsClient, text: &str) -> bool {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match client.next_within(remaining).await {
            Some(frame) if marker_text(&frame).as_deref() == Some(text) => return true,
            Some(_) => {}
            None => return false,
        }
    }
}

fn denied(reply: &ServerFrame) -> bool {
    let error = error_of(reply);
    (error.code.as_str(), error.message.as_str(), error.fatal) == ("permission_denied", "TOPIC_FORBIDDEN", false)
}

#[tokio::test]
async fn a_revoked_subscription_ends_on_every_connection_of_the_user() {
    let harness = Harness::start().await;
    let (member, other) = (harness.app.user("member").await, harness.app.user("other").await);
    let guard = Members::of(&[member.id, other.id]);
    harness.app.core.topic_guards.register("room", guard.clone());
    let topic = unique_topic("room");
    let rt = &harness.app.core.rt;

    let mut phone = harness.ws_as(Some(&member), true).await;
    let mut laptop = harness.ws_as(Some(&member), true).await;
    let mut bystander = harness.ws_as(Some(&other), true).await;
    for client in [&mut phone, &mut laptop, &mut bystander] {
        assert!(is_ack(&client.request(subscribe_body(&[&topic])).await));
    }
    rt.to_topic(&topic, &marker("before")).await.unwrap();
    for client in [&mut phone, &mut laptop, &mut bystander] {
        assert!(receives(client, "before").await);
    }

    // The domain bans the member, then says so.
    guard.remove(member.id);
    rt.revoke(member.id, &topic).await;
    for client in [&mut phone, &mut laptop] {
        assert_eq!(unsubscribed(client).await, Some((topic.clone(), "TOPIC_FORBIDDEN".into())));
    }
    rt.to_topic(&topic, &marker("after")).await.unwrap();
    assert!(receives(&mut bystander, "after").await, "others keep the topic");
    for client in [&mut phone, &mut laptop] {
        assert!(client.stays_quiet().await, "nothing more of the topic, and no raw control message");
    }
    // Coming back is refused while the ban stands.
    assert!(denied(&phone.request(subscribe_body(&[&topic])).await));
    // Revoking again is harmless.
    rt.revoke(member.id, &topic).await;
    assert!(phone.stays_quiet().await);
    assert!(bystander.stays_quiet().await);
}

#[tokio::test]
async fn a_revocation_the_guard_disagrees_with_drops_nothing() {
    let harness = Harness::start().await;
    let member = harness.app.user("member").await;
    harness.app.core.topic_guards.register("room", Members::of(&[member.id]));
    let topic = unique_topic("room");
    let mut client = harness.ws_as(Some(&member), true).await;
    assert!(is_ack(&client.request(subscribe_body(&[&topic])).await));

    // Leaving a room one may still see, say: the domain revokes anyway.
    harness.app.core.rt.revoke(member.id, &topic).await;
    assert!(client.stays_quiet().await);
    harness.app.core.rt.to_topic(&topic, &marker("still here")).await.unwrap();
    assert!(receives(&mut client, "still here").await);
}

#[tokio::test]
async fn a_prefix_revocation_covers_the_topics_below_it_only() {
    let harness = Harness::start().await;
    let member = harness.app.user("member").await;
    let guard = Members::of(&[member.id]);
    harness.app.core.topic_guards.register("party", guard.clone());
    let party = unique_topic("party");
    let (own, below, sibling) = (party.clone(), format!("{party}:seat"), format!("{party}0"));
    let mut client = harness.ws_as(Some(&member), true).await;
    assert!(is_ack(&client.request(subscribe_body(&[&own, &below, &sibling])).await));

    guard.remove(member.id);
    harness.app.core.rt.revoke_prefix(member.id, &party).await;
    let mut dropped = HashSet::new();
    for _ in 0..2 {
        let (topic, reason) = unsubscribed(&mut client).await.expect("an Unsubscribed frame");
        assert_eq!(reason, "TOPIC_FORBIDDEN");
        dropped.insert(topic);
    }
    assert_eq!(dropped, HashSet::from([own, below]));
    harness.app.core.rt.to_topic(&sibling, &marker("sibling")).await.unwrap();
    assert!(receives(&mut client, "sibling").await, "a topic that merely starts with the same letters stays");
}

#[tokio::test]
async fn rechecking_a_topic_drops_whoever_the_guard_now_refuses_anonymous_ones_too() {
    let harness = Harness::start().await;
    let (member, outsider) = (harness.app.user("member").await, harness.app.user("outsider").await);
    // Open to everyone at first, as a public room is.
    let open = Arc::new(Mutex::new(true));
    struct Narrowing(Arc<Mutex<bool>>, Uuid);
    #[async_trait::async_trait]
    impl TopicGuard for Narrowing {
        async fn may_subscribe(&self, subscriber: Option<Uuid>, _topic: &str) -> AppResult<bool> {
            Ok(*self.0.lock() || subscriber == Some(self.1))
        }
    }
    harness.app.core.topic_guards.register("room", Arc::new(Narrowing(open.clone(), member.id)));
    let topic = unique_topic("room");
    let mut members = harness.ws_as(Some(&member), true).await;
    let mut outsiders = harness.ws_as(Some(&outsider), true).await;
    let mut guest = harness.ws_as(None, true).await;
    for client in [&mut members, &mut outsiders, &mut guest] {
        assert!(is_ack(&client.request(subscribe_body(&[&topic])).await));
    }

    // Narrowed to members.
    *open.lock() = false;
    harness.app.core.rt.recheck_topic(&topic).await;
    for client in [&mut outsiders, &mut guest] {
        assert_eq!(unsubscribed(client).await, Some((topic.clone(), "TOPIC_FORBIDDEN".into())));
    }
    harness.app.core.rt.to_topic(&topic, &marker("members only")).await.unwrap();
    assert!(receives(&mut members, "members only").await);
    assert!(outsiders.stays_quiet().await && guest.stays_quiet().await);
    assert!(denied(&guest.request(subscribe_body(&[&topic])).await));
}

#[tokio::test]
async fn a_closed_topic_lets_everyone_go_without_asking_the_guard() {
    let harness = Harness::start().await;
    let member = harness.app.user("member").await;
    // `share:` topics have no guard: the token is the key.
    let topic = unique_topic("share");
    let mut owner = harness.ws_as(Some(&member), true).await;
    let mut guest = harness.ws_as(None, true).await;
    for client in [&mut owner, &mut guest] {
        assert!(is_ack(&client.request(subscribe_body(&[&topic])).await));
    }
    harness.app.core.rt.to_topic(&topic, &marker("last words")).await.unwrap();
    harness.app.core.rt.close_topic(&topic).await;
    for client in [&mut owner, &mut guest] {
        // What was published before the close still arrives, then the close.
        assert!(receives(client, "last words").await);
        assert_eq!(unsubscribed(client).await, Some((topic.clone(), "TOPIC_CLOSED".into())));
    }
    harness.app.core.rt.to_topic(&topic, &marker("too late")).await.unwrap();
    assert!(owner.stays_quiet().await && guest.stays_quiet().await);
}

#[tokio::test]
async fn revocations_reach_every_gateway() {
    let harness = Harness::start().await;
    let member = harness.app.user("member").await;
    let guard = Members::of(&[member.id]);
    harness.app.core.topic_guards.register("room", guard.clone());
    let (_second, addrs) = harness.another_gateway(test_config()).await;
    let topic = unique_topic("room");
    let mut here = harness.ws_as(Some(&member), true).await;
    let mut there = WsClient::connect(addrs.websocket).await;
    there.hello(&member.token, true).await;
    for client in [&mut here, &mut there] {
        assert!(is_ack(&client.request(subscribe_body(&[&topic])).await));
    }
    guard.remove(member.id);
    harness.app.core.rt.revoke(member.id, &topic).await;
    for client in [&mut here, &mut there] {
        assert_eq!(unsubscribed(client).await, Some((topic.clone(), "TOPIC_FORBIDDEN".into())));
    }
}

#[tokio::test]
async fn a_revocation_during_a_subscribe_is_not_lost() {
    let harness = Harness::start().await;
    let member = harness.app.user("member").await;
    let guard = Members::holding_first(&[member.id]);
    harness.app.core.topic_guards.register("room", guard.clone());
    let topic = unique_topic("room");
    // Someone else holds the route already, so the subscriber below joins an
    // existing one: only the revocation counter can tell it something went by.
    let watcher = harness.app.user("watcher").await;
    guard.admitted.lock().insert(watcher.id);
    let mut other = harness.ws_as(Some(&watcher), true).await;
    let (entered, release) = guard.hold_first.as_ref().unwrap();
    let id = other.send(subscribe_body(&[&topic])).await;
    entered.notified().await;
    release.notify_one();
    assert!(is_ack(&other.reply_to(id).await));

    let mut client = harness.ws_as(Some(&member), true).await;
    // The guard answers "yes" for the member, but holds the answer...
    guard.calls.store(0, Ordering::SeqCst);
    let id = client.send(subscribe_body(&[&topic])).await;
    entered.notified().await;
    // ...while the domain bans them and publishes the revocation.
    guard.remove(member.id);
    harness.app.core.rt.revoke(member.id, &topic).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    release.notify_one();
    let reply = client.reply_to(id).await;
    match &reply.body {
        // The gateway noticed and asked again before answering...
        Some(Body::Error(_)) => assert!(denied(&reply)),
        // ...or let it through and drops it at once.
        Some(Body::Ack(_)) => {
            assert_eq!(unsubscribed(&mut client).await, Some((topic.clone(), "TOPIC_FORBIDDEN".into())))
        }
        other => panic!("unexpected answer {other:?}"),
    }
    harness.app.core.rt.to_topic(&topic, &marker("banned")).await.unwrap();
    assert!(client.stays_quiet().await);
}

#[tokio::test]
async fn the_periodic_recheck_heals_a_lost_revocation() {
    let cfg = RealtimeConfig { recheck_interval: Duration::from_millis(300), ..test_config() };
    let harness = Harness::start_with(cfg).await;
    let (member, other) = (harness.app.user("member").await, harness.app.user("other").await);
    let guard = Members::of(&[member.id, other.id]);
    harness.app.core.topic_guards.register("room", guard.clone());
    let topic = unique_topic("room");
    let mut client = harness.ws_as(Some(&member), true).await;
    let mut stays = harness.ws_as(Some(&other), true).await;
    for c in [&mut client, &mut stays] {
        assert!(is_ack(&c.request(subscribe_body(&[&topic])).await));
    }

    // Access ends, but the revocation never makes it to the gateway.
    guard.remove(member.id);
    assert_eq!(unsubscribed(&mut client).await, Some((topic.clone(), "TOPIC_FORBIDDEN".into())));
    // The other subscriber is checked too, and kept.
    tokio::time::sleep(Duration::from_millis(700)).await;
    harness.app.core.rt.to_topic(&topic, &marker("kept")).await.unwrap();
    assert!(receives(&mut stays, "kept").await);
    let stats = harness.gateway.stats();
    assert!(stats.topic_rechecks >= 3, "{stats:?}");
    assert_eq!(stats.topics_revoked, 1);
}

#[tokio::test]
async fn the_periodic_recheck_keeps_within_the_gateway_budget() {
    // One check per second for the whole gateway, however much is due.
    let cfg = RealtimeConfig { recheck_interval: Duration::from_millis(100), recheck_rate: 1.0, ..test_config() };
    let harness = Harness::start_with(cfg).await;
    let member = harness.app.user("member").await;
    harness.app.core.topic_guards.register("room", Members::of(&[member.id]));
    let topics: Vec<String> = (0..5).map(|_| unique_topic("room")).collect();
    let mut client = harness.ws_as(Some(&member), true).await;
    let names: Vec<&str> = topics.iter().map(String::as_str).collect();
    assert!(is_ack(&client.request(subscribe_body(&names)).await));
    tokio::time::sleep(Duration::from_millis(2100)).await;
    let checks = harness.gateway.stats().topic_rechecks;
    assert!((1..=4).contains(&checks), "about one check per second, not {checks}");
}
