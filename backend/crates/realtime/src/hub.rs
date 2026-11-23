//! Fan-out from the bus to local connections.
//!
//! Every user and topic with at least one connection on this gateway has one
//! *route*: a single bus subscription whose frames are copied to each member
//! connection. Routes are created by the first member and torn down when the
//! last one leaves, so the number of bus subscriptions follows the number of
//! distinct users and topics, not the number of connections.
//!
//! Control messages (`tokuly_core::realtime::CONTROL_HEADER`) on the same
//! subjects are not forwarded: they become instructions in each member's
//! session queue (re-check or drop a subscription).
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use dashmap::{DashMap, Entry};
use futures::StreamExt as _;
use parking_lot::RwLock;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tokuly_core::{
    bus::Bus,
    realtime::{control_of, except_of, topic_subject, user_subject},
};
use uuid::Uuid;

use crate::{
    client::{Client, SessionControl},
    frames::delivery_of,
    stats::Counters,
};

/// How long a joining connection waits for the bus subscription.
const SUBSCRIBE_TIMEOUT: Duration = Duration::from_secs(5);
const RESUBSCRIBE_DELAY_MIN: Duration = Duration::from_millis(200);
const RESUBSCRIBE_DELAY_MAX: Duration = Duration::from_secs(5);

/// What a route carries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum RouteKey {
    /// Everything addressed to one user.
    User(Uuid),
    /// Everything published on one topic.
    Topic(String),
}

impl RouteKey {
    fn subject(&self) -> String {
        match self {
            RouteKey::User(user_id) => user_subject(*user_id),
            RouteKey::Topic(topic) => topic_subject(topic),
        }
    }

    fn topic(&self) -> Option<&str> {
        match self {
            RouteKey::User(_) => None,
            RouteKey::Topic(topic) => Some(topic),
        }
    }
}

/// The bus could not be subscribed to in time.
#[derive(Debug)]
pub(crate) struct BusUnavailable;

struct Route {
    members: RwLock<HashMap<u64, Arc<Client>>>,
    /// True while the bus subscription is in place.
    subscribed: watch::Sender<bool>,
    /// Cancelled when the last member left.
    stop: CancellationToken,
}

impl Route {
    fn with_member(client: &Arc<Client>) -> Arc<Self> {
        Arc::new(Self {
            members: RwLock::new(HashMap::from([(client.key, client.clone())])),
            subscribed: watch::Sender::new(false),
            stop: CancellationToken::new(),
        })
    }

    /// Copies one frame to every member but those signed in as one of
    /// `except` (the frame's exception list, see
    /// `tokuly_core::realtime::Realtime::to_topic_except`). Never blocks on
    /// a client: each copy is a push into that client's bounded outbox.
    fn deliver(&self, frame: Bytes, except: &HashSet<Uuid>) {
        let delivery = delivery_of(&frame);
        for client in self.members.read().values() {
            if client.user.get().is_some_and(|user| except.contains(user)) {
                continue;
            }
            client.send(delivery, frame.clone());
        }
    }

    /// Hands an instruction to every member's session.
    fn control(&self, control: &SessionControl) {
        for client in self.members.read().values() {
            client.control(control.clone());
        }
    }

    async fn wait_subscribed(&self) -> bool {
        self.subscribed.subscribe().wait_for(|subscribed| *subscribed).await.is_ok()
    }
}

pub(crate) struct Hub {
    bus: Bus,
    routes: DashMap<RouteKey, Arc<Route>>,
    counters: Arc<Counters>,
    /// Goes up with every control message any route receives. A session
    /// that checked a guard and then joined a route compares it before and
    /// after, and asks the guard again if a revocation went by in between
    /// (it might have been about the subscription just made).
    controls_seen: AtomicU64,
    /// Cancelled when the gateway stops: every route ends.
    shutdown: CancellationToken,
}

/// How a connection got onto a route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Joined {
    /// The route was there already, with its bus subscription.
    Existing,
    /// This connection created the route: until its bus subscription was in
    /// place, control messages for it could not be heard on this gateway.
    Created,
}

impl Hub {
    pub(crate) fn new(bus: Bus, counters: Arc<Counters>) -> Arc<Self> {
        Arc::new(Self {
            bus,
            routes: DashMap::new(),
            counters,
            controls_seen: AtomicU64::new(0),
            shutdown: CancellationToken::new(),
        })
    }

    /// Adds a connection to a route, creating the route (and its bus
    /// subscription) if this is the first member. Returns once frames
    /// published from now on will reach the connection.
    pub(crate) async fn join(self: &Arc<Self>, key: RouteKey, client: &Arc<Client>) -> Result<Joined, BusUnavailable> {
        // Membership changes under the map entry's lock, so a concurrent
        // `leave` can never tear down a route that just gained a member.
        let (route, joined) = match self.routes.entry(key.clone()) {
            Entry::Occupied(entry) => {
                entry.get().members.write().insert(client.key, client.clone());
                (entry.get().clone(), Joined::Existing)
            }
            Entry::Vacant(entry) => {
                let route = Route::with_member(client);
                entry.insert(route.clone());
                self.gauge(&key).incr();
                tokio::spawn(self.clone().forward(key.clone(), route.clone()));
                (route, Joined::Created)
            }
        };
        // A route whose subscription was lost and re-made also missed what
        // was sent in between.
        let joined = if *route.subscribed.borrow() { joined } else { Joined::Created };
        match tokio::time::timeout(SUBSCRIBE_TIMEOUT, route.wait_subscribed()).await {
            Ok(true) => Ok(joined),
            _ => {
                self.leave(&key, client);
                Err(BusUnavailable)
            }
        }
    }

    /// Removes a connection from a route; the last one out ends the route.
    pub(crate) fn leave(&self, key: &RouteKey, client: &Client) {
        let emptied = self.routes.remove_if(key, |_, route| {
            let mut members = route.members.write();
            members.remove(&client.key);
            members.is_empty()
        });
        if let Some((_, route)) = emptied {
            route.stop.cancel();
            self.gauge(key).decr();
        }
    }

    /// How many control messages this gateway's routes received so far.
    pub(crate) fn controls_seen(&self) -> u64 {
        self.controls_seen.load(Ordering::SeqCst)
    }

    /// Connections on this gateway that are members of a route.
    pub(crate) fn member_count(&self, key: &RouteKey) -> usize {
        self.routes.get(key).map(|route| route.members.read().len()).unwrap_or(0)
    }

    /// Ends every route (gateway shutdown).
    pub(crate) fn shutdown(&self) {
        self.shutdown.cancel();
    }

    fn gauge(&self, key: &RouteKey) -> &crate::stats::Gauge {
        match key {
            RouteKey::User(_) => &self.counters.user_subscriptions,
            RouteKey::Topic(_) => &self.counters.topic_subscriptions,
        }
    }

    /// The task behind one route: holds the bus subscription and forwards
    /// its frames until the route is stopped. A lost subscription is
    /// re-established (frames published in between are lost; clients recover
    /// by refetching, as after any reconnect).
    async fn forward(self: Arc<Self>, key: RouteKey, route: Arc<Route>) {
        let subject = key.subject();
        let mut delay = RESUBSCRIBE_DELAY_MIN;
        loop {
            let subscribing = async {
                let subscriber = self.bus.subscribe(subject.clone()).await.map_err(|e| e.to_string())?;
                // The subscription only counts once the server has it.
                self.bus.client.flush().await.map_err(|e| e.to_string())?;
                Ok::<_, String>(subscriber)
            };
            let subscribed = tokio::select! {
                _ = route.stop.cancelled() => return,
                _ = self.shutdown.cancelled() => return,
                result = subscribing => result,
            };
            match subscribed {
                Ok(mut subscriber) => {
                    route.subscribed.send_replace(true);
                    delay = RESUBSCRIBE_DELAY_MIN;
                    loop {
                        tokio::select! {
                            _ = route.stop.cancelled() => return,
                            _ = self.shutdown.cancelled() => return,
                            message = subscriber.next() => match message {
                                Some(message) => match control_of(message.headers.as_ref()) {
                                    None => route.deliver(message.payload, &except_of(message.headers.as_ref())),
                                    Some(control) => {
                                        self.controls_seen.fetch_add(1, Ordering::SeqCst);
                                        if let Some(control) = control.and_then(|control| SessionControl::from_bus(control, key.topic())) {
                                            route.control(&control);
                                        }
                                    }
                                },
                                None => break,
                            },
                        }
                    }
                    route.subscribed.send_replace(false);
                    tracing::warn!(subject, "realtime bus subscription ended; subscribing again");
                }
                Err(error) => tracing::warn!(subject, error, "realtime bus subscription failed; retrying"),
            }
            tokio::select! {
                _ = route.stop.cancelled() => return,
                _ = self.shutdown.cancelled() => return,
                _ = tokio::time::sleep(delay) => {}
            }
            delay = (delay * 2).min(RESUBSCRIBE_DELAY_MAX);
        }
    }
}
