//! The protocol of one connection (`tokuly.realtime.v1`), independent of
//! the transport it runs over.
//!
//! A session starts anonymous and unwelcomed. `Hello` signs it in (or keeps
//! it anonymous) and is answered by `Welcome`; from then on the client may
//! subscribe to topics, report locations and send chat signals, and must
//! ping at least every `ping_interval`. `Hello` may be repeated on the same
//! connection to present a fresh access token before (or shortly after) the
//! old one expires.
//!
//! The server may end a topic subscription on its own: a domain revoked it
//! (`tokuly_core::realtime::Realtime::revoke`) or the periodic re-check found
//! that the topic's guard no longer admits the subscriber. The client then
//! gets `Unsubscribed`. Revocations wait in the client's control queue and
//! are applied between two client frames, never in the middle of one.
use std::{collections::HashMap, sync::Arc, time::Duration};

use buffa::Message as _;
use chrono::Utc;
use serde::Serialize;
use tokio::time::Instant;
use tokuly_core::{
    presence::{
        self, CONVERSATION_PRESENCE_SUBJECT, ConversationPresenceSignal, LocationReport, TYPING_SUBJECT, TypingSignal,
    },
    time::{now, ts},
};
use tokuly_proto::realtime::{ClientFrame, Hello, Welcome, client_frame::Body};
use uuid::Uuid;

use crate::{
    GatewayInner,
    client::{Client, SessionControl},
    connection::{FrameSource, Inbound, RecvError},
    frames::{self, Code, Delivery, Rejection},
    hub::{ControlsMark, Joined, RouteKey},
    limits::TokenBucket,
    location,
    outbox::CloseReason,
    topics,
};

const FRAME_INVALID: Rejection = Rejection::new(Code::InvalidArgument, "FRAME_INVALID");
const FRAME_TOO_LARGE: Rejection = Rejection::new(Code::InvalidArgument, "FRAME_TOO_LARGE");
const BINARY_FRAMES_ONLY: Rejection = Rejection::new(Code::InvalidArgument, "BINARY_FRAMES_ONLY");
const DATAGRAM_NOT_ALLOWED: Rejection = Rejection::new(Code::InvalidArgument, "DATAGRAM_NOT_ALLOWED");
const CONVERSATION_ID_INVALID: Rejection = Rejection::new(Code::InvalidArgument, "CONVERSATION_ID_INVALID");
const HELLO_REQUIRED: Rejection = Rejection::new(Code::FailedPrecondition, "HELLO_REQUIRED");
const HELLO_TIMEOUT: Rejection = Rejection::new(Code::FailedPrecondition, "HELLO_TIMEOUT");
const IDLE_TIMEOUT: Rejection = Rejection::new(Code::FailedPrecondition, "IDLE_TIMEOUT");
const SIGN_IN_REQUIRED: Rejection = Rejection::new(Code::Unauthenticated, "SIGN_IN_REQUIRED");
const TOKEN_EXPIRED: Rejection = Rejection::new(Code::Unauthenticated, "TOKEN_EXPIRED");
const SESSION_REVOKED: Rejection = Rejection::new(Code::Unauthenticated, "SESSION_REVOKED");
const USER_MISMATCH: Rejection = Rejection::new(Code::PermissionDenied, "USER_MISMATCH");
const RATE_LIMITED: Rejection = Rejection::new(Code::ResourceExhausted, "RATE_LIMITED");
const TOO_MANY_TOPICS: Rejection = Rejection::new(Code::ResourceExhausted, "TOO_MANY_TOPICS");
const BUS_UNAVAILABLE: Rejection = Rejection::new(Code::Unavailable, "BUS_UNAVAILABLE");
const SHUTTING_DOWN: Rejection = Rejection::new(Code::Unavailable, "SHUTTING_DOWN");

/// `Unsubscribed.reason` when the guard refused on a re-check.
const REVOKED_FORBIDDEN: &str = "TOPIC_FORBIDDEN";
/// `Unsubscribed.reason` when the topic ended for everyone.
const REVOKED_CLOSED: &str = "TOPIC_CLOSED";
/// When a re-check could not be decided (the guard failed), the next try.
const RECHECK_RETRY: Duration = Duration::from_secs(10);
/// When the gateway's re-check budget is spent, the next try: on average
/// this long, spread by ±50 % so the sessions that were turned away do not
/// all come back in the same instant.
const RECHECK_POSTPONE: Duration = Duration::from_secs(1);

/// Frames of any kind per connection: far above what an honest client
/// sends, low enough that a flood costs the gateway next to nothing.
const FRAME_BURST: u32 = 200;
const FRAME_RATE: f64 = 50.0;
/// Typing and conversation presence signals.
const SIGNAL_BURST: u32 = 20;
const SIGNAL_RATE: f64 = 5.0;

/// The signed-in state of a connection.
struct Auth {
    user_id: Uuid,
    session_id: Uuid,
    expires_at: Instant,
    /// Set once the token expired: until when a fresh `Hello` is awaited.
    grace_until: Option<Instant>,
    next_revocation_check: Instant,
}

pub(crate) struct Session<'g> {
    gateway: &'g GatewayInner,
    client: Arc<Client>,
    connected_at: Instant,
    last_inbound: Instant,
    /// A `Hello` was accepted.
    welcomed: bool,
    auth: Option<Auth>,
    /// The app is visible to the user (from `Hello` / `AppState`).
    foreground: bool,
    /// This connection currently counts towards its user being online.
    counted_online: bool,
    /// Subscribed topics and when each is due for its periodic re-check.
    topics: HashMap<String, Instant>,
    frame_limit: TokenBucket,
    /// State of the cheap generator that spreads postponed re-checks.
    jitter: u64,
    location_limit: TokenBucket,
    signal_limit: TokenBucket,
}

impl<'g> Session<'g> {
    pub(crate) fn new(gateway: &'g GatewayInner, client: Arc<Client>) -> Self {
        let jitter = client.key.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let now = Instant::now();
        Self {
            gateway,
            client,
            connected_at: now,
            last_inbound: now,
            welcomed: false,
            auth: None,
            foreground: false,
            counted_online: false,
            topics: HashMap::new(),
            frame_limit: TokenBucket::new(FRAME_BURST, FRAME_RATE),
            jitter,
            location_limit: TokenBucket::new(gateway.cfg.location_burst, gateway.cfg.location_rate),
            signal_limit: TokenBucket::new(SIGNAL_BURST, SIGNAL_RATE),
        }
    }

    /// Serves the connection until it has to end; says why.
    pub(crate) async fn run(&mut self, source: &mut impl FrameSource) -> CloseReason {
        loop {
            let deadline = self.next_deadline();
            tokio::select! {
                _ = self.gateway.closing.cancelled() => {
                    self.client.reply(frames::error(0, SHUTTING_DOWN, true));
                    return CloseReason::ShuttingDown;
                }
                // The writer gave up (peer gone) or the outbox overflowed.
                _ = self.client.outbox.closed() => return self.client.outbox.close_reason().unwrap_or(CloseReason::PeerClosed),
                _ = self.client.controls_ready() => self.apply_controls().await,
                _ = tokio::time::sleep_until(deadline) => {
                    let now = Instant::now();
                    if let Some(reason) = self.on_deadline(now) {
                        return reason;
                    }
                    self.recheck_due(now).await;
                }
                received = source.recv() => match received {
                    Ok(Some(inbound)) => {
                        if let Some(reason) = self.on_inbound(inbound).await {
                            return reason;
                        }
                    }
                    Ok(None) | Err(RecvError::Lost) => return CloseReason::PeerClosed,
                    Err(RecvError::FrameTooLarge) => return self.fatal(0, FRAME_TOO_LARGE),
                    Err(RecvError::NotBinary) => return self.fatal(0, BINARY_FRAMES_ONLY),
                },
            }
        }
    }

    /// Releases everything the connection held. Call once, after [`Self::run`].
    pub(crate) fn finish(&mut self) {
        self.set_foreground(false);
        for (topic, _) in self.topics.drain() {
            self.gateway.hub.leave(&RouteKey::Topic(topic.clone()), &self.client);
            self.gateway.audience.touch(&topic);
        }
        if let Some(auth) = self.auth.take() {
            self.gateway.hub.leave(&RouteKey::User(auth.user_id), &self.client);
            self.gateway.counters.authenticated_connections.decr();
        }
    }

    // ── Timers ──

    /// The next moment something has to happen without a frame arriving.
    fn next_deadline(&self) -> Instant {
        let cfg = &self.gateway.cfg;
        let mut deadline =
            if self.welcomed { self.last_inbound + cfg.idle_timeout } else { self.connected_at + cfg.hello_timeout };
        if let Some(auth) = &self.auth {
            deadline = deadline.min(auth.next_revocation_check).min(auth.grace_until.unwrap_or(auth.expires_at));
        }
        if let Some(due) = self.topics.values().min() {
            deadline = deadline.min(*due);
        }
        deadline
    }

    fn on_deadline(&mut self, now: Instant) -> Option<CloseReason> {
        let cfg = &self.gateway.cfg;
        if !self.welcomed {
            if now >= self.connected_at + cfg.hello_timeout {
                return Some(self.fatal(0, HELLO_TIMEOUT));
            }
        } else if now >= self.last_inbound + cfg.idle_timeout {
            self.client.reply(frames::error(0, IDLE_TIMEOUT, true));
            return Some(CloseReason::Idle);
        }

        let auth = self.auth.as_mut()?;
        let mut verdict = None;
        if now >= auth.next_revocation_check {
            auth.next_revocation_check = now + cfg.revocation_check;
            if self.gateway.core.tokens.is_revoked(auth.session_id) {
                verdict = Some((SESSION_REVOKED, true));
            }
        }
        if verdict.is_none() {
            match auth.grace_until {
                // The token just expired: ask for a new one, keep serving meanwhile.
                None if now >= auth.expires_at => {
                    auth.grace_until = Some(now + cfg.auth_grace);
                    verdict = Some((TOKEN_EXPIRED, false));
                }
                Some(until) if now >= until => verdict = Some((TOKEN_EXPIRED, true)),
                _ => {}
            }
        }
        match verdict {
            Some((rejection, true)) => Some(self.fatal(0, rejection)),
            Some((rejection, false)) => {
                self.client.reply(frames::error(0, rejection, false));
                None
            }
            None => None,
        }
    }

    // ── Inbound frames ──

    async fn on_inbound(&mut self, inbound: Inbound) -> Option<CloseReason> {
        let counters = &self.gateway.counters;
        self.last_inbound = Instant::now();
        counters.frames_in.incr();
        if inbound.datagram {
            counters.datagrams_in.incr();
        }
        let Ok(frame) = ClientFrame::decode_from_slice(&inbound.frame) else {
            counters.frames_rejected.incr();
            // The ID is unreadable too, so this error answers "frame 0".
            // A broken datagram gets no answer: datagrams are best effort.
            if !inbound.datagram {
                self.client.reply(frames::error(0, FRAME_INVALID, false));
            }
            return None;
        };
        let id = frame.id;
        if !self.frame_limit.allow() {
            self.answer(id, Err(RATE_LIMITED));
            return None;
        }
        let Some(body) = frame.body else {
            self.answer(id, Err(FRAME_INVALID));
            return None;
        };
        if inbound.datagram && !matches!(body, Body::Location(_) | Body::Ping(_)) {
            self.answer(id, Err(DATAGRAM_NOT_ALLOWED));
            return None;
        }
        match body {
            Body::Hello(hello) => return self.on_hello(id, *hello).await,
            Body::Ping(ping) => {
                // Answered the way it came, so clients can measure both paths.
                let delivery = if inbound.datagram { Delivery::Droppable } else { Delivery::Reliable };
                self.client.send(delivery, frames::pong(id, ping.nonce));
            }
            _ if !self.welcomed => self.answer(id, Err(HELLO_REQUIRED)),
            Body::Location(sample) => {
                let result = self.on_location(&sample).await;
                self.answer(id, result);
            }
            Body::Subscribe(subscribe) => {
                let result = self.subscribe(&subscribe.topics).await;
                self.answer(id, result);
            }
            Body::Unsubscribe(unsubscribe) => {
                self.unsubscribe(&unsubscribe.topics);
                self.answer(id, Ok(()));
            }
            Body::AppState(state) => {
                self.set_foreground(state.foreground);
                self.answer(id, Ok(()));
            }
            Body::Typing(typing) => {
                let result =
                    self.forward_signal(TYPING_SUBJECT, &typing.conversation_id, |user_id, conversation_id| {
                        TypingSignal { user_id, conversation_id, typing: typing.typing }
                    })
                    .await;
                self.answer(id, result);
            }
            Body::ConversationPresence(presence) => {
                let result = self
                    .forward_signal(
                        CONVERSATION_PRESENCE_SUBJECT,
                        &presence.conversation_id,
                        |user_id, conversation_id| ConversationPresenceSignal {
                            user_id,
                            conversation_id,
                            present: presence.present,
                        },
                    )
                    .await;
                self.answer(id, result);
            }
        }
        None
    }

    /// Answers a client frame: `Ack` or `Error` echoing its ID. Frames sent
    /// with ID 0 asked for no answer and get none. A `Subscribe` is answered
    /// only once it is in effect (its routes joined, the bus subscriptions
    /// in place) or refused as a whole: a client that reads a snapshot after
    /// the `Ack` misses no frame of those topics.
    fn answer(&self, id: u64, result: Result<(), Rejection>) {
        if result.is_err() {
            self.gateway.counters.frames_rejected.incr();
        }
        if id == 0 {
            return;
        }
        match result {
            Ok(()) => self.client.reply(frames::ack(id)),
            Err(rejection) => self.client.reply(frames::error(id, rejection, false)),
        }
    }

    /// Queues a fatal `Error`; the caller ends the session with the result.
    fn fatal(&self, reply_to: u64, rejection: Rejection) -> CloseReason {
        self.gateway.counters.frames_rejected.incr();
        self.client.reply(frames::error(reply_to, rejection, true));
        CloseReason::Fatal
    }

    // ── Hello ──

    async fn on_hello(&mut self, id: u64, hello: Hello) -> Option<CloseReason> {
        let claimed = match self.authenticate(&hello.access_token).await {
            Ok(claimed) => claimed,
            // The reason (TOKEN_EXPIRED, TOKEN_INVALID, SESSION_REVOKED)
            // tells the client whether refreshing its token will help.
            // Not a token a refresh would mend.
            Err(reason @ tokuly_core::auth::ACCOUNT_TOO_YOUNG) => {
                return Some(self.fatal(id, Rejection::new(Code::PermissionDenied, reason)));
            }
            Err(reason) => return Some(self.fatal(id, Rejection::new(Code::Unauthenticated, reason))),
        };
        // A connection may sign in, and refresh its token, but never become
        // someone else: routes and presence are bound to the user.
        if let Some(current) = &self.auth
            && claimed.as_ref().map(|claimed| claimed.user_id) != Some(current.user_id)
        {
            return Some(self.fatal(id, USER_MISMATCH));
        }
        let subscriber = claimed.as_ref().map(|claimed| claimed.user_id);
        // The user's route comes first: before `Welcome` goes out, so that a
        // client which then fetches a snapshot cannot miss an event in
        // between, and before the guards of the Hello's topics are asked, so
        // that a revocation published meanwhile reaches this connection.
        let new_user = claimed.as_ref().filter(|_| self.auth.is_none()).map(|claimed| claimed.user_id);
        if let Some(user_id) = new_user
            && self.gateway.hub.join(RouteKey::User(user_id), &self.client).await.is_err()
        {
            return Some(self.fatal(id, BUS_UNAVAILABLE));
        }
        let marks = self.controls_marks(&hello.topics);
        // A topic the guard refuses does not refuse the Hello: a client that
        // reconnects with a topic it lost access to meanwhile would never be
        // welcomed again. It hears `Unsubscribed` for that topic instead.
        let (new_topics, forbidden) = match self.plan_subscriptions(&hello.topics, subscriber, false).await {
            Ok(planned) => planned,
            Err(rejection) => {
                // Nothing was applied; the client may correct its Hello.
                if let Some(user_id) = new_user {
                    self.gateway.hub.leave(&RouteKey::User(user_id), &self.client);
                }
                self.gateway.counters.frames_rejected.incr();
                self.client.reply(frames::error(id, rejection, false));
                return None;
            }
        };

        if let Some(claimed) = claimed {
            // Before any topic is joined: topic frames that leave this user out must skip it.
            let _ = self.client.user.set(claimed.user_id);
            if new_user.is_some() {
                self.gateway.counters.authenticated_connections.incr();
            }
            self.auth = Some(claimed);
        }
        let mut refused = match self.admit(new_topics, subscriber, &marks).await {
            Ok(refused) => refused,
            Err(rejection) => return Some(self.fatal(id, rejection)),
        };
        refused.extend(forbidden.into_iter().map(|topic| (topic, REVOKED_FORBIDDEN)));
        // The refused topics are told before `Welcome`, so that `Welcome`
        // answers the rest: every topic of the Hello that was not refused
        // first is subscribed when it arrives.
        for (topic, reason) in refused {
            self.gateway.counters.topics_revoked.incr();
            self.client.reply(frames::unsubscribed(&topic, reason));
        }
        self.welcomed = true;
        self.set_foreground(hello.foreground);
        self.client.reply(frames::welcome(id, self.welcome()));
        tracing::debug!(
            connection = %self.client.id,
            user = subscriber.map(|user_id| user_id.to_string()),
            platform = ?hello.platform,
            app_version = hello.app_version,
            foreground = hello.foreground,
            "realtime hello accepted"
        );
        None
    }

    /// Verifies a Hello's token, asking Valkey whether its session was
    /// revoked elsewhere (one lookup per Hello; the periodic re-check stays
    /// in memory).
    async fn authenticate(&self, access_token: &str) -> Result<Option<Auth>, &'static str> {
        if access_token.is_empty() {
            return Ok(None);
        }
        let verified = match self.gateway.core.tokens.authenticate_shared(access_token).await {
            // An account under the minimum age has nothing to listen to.
            tokuly_core::auth::AuthState::Authenticated(verified) if verified.too_young => {
                return Err(tokuly_core::auth::ACCOUNT_TOO_YOUNG);
            }
            tokuly_core::auth::AuthState::Authenticated(verified) => verified,
            tokuly_core::auth::AuthState::Invalid(reason) => return Err(reason),
            tokuly_core::auth::AuthState::Anonymous => return Err("TOKEN_INVALID"),
        };
        let lifetime = (verified.expires_at - Utc::now()).to_std().unwrap_or_default();
        let now = Instant::now();
        Ok(Some(Auth {
            user_id: verified.user_id,
            session_id: verified.session_id,
            expires_at: now + lifetime,
            grace_until: None,
            next_revocation_check: now + self.gateway.cfg.revocation_check,
        }))
    }

    fn welcome(&self) -> Welcome {
        let cfg = &self.gateway.cfg;
        let seconds = |duration: std::time::Duration| duration.as_secs().clamp(1, i32::MAX as u64) as i32;
        Welcome {
            connection_id: self.client.id.to_string(),
            user_id: self.auth.as_ref().map(|auth| auth.user_id.to_string()).unwrap_or_default(),
            server_time: ts(now()),
            ping_interval_seconds: seconds(cfg.ping_interval),
            transport: self.client.transport.as_str().to_owned(),
            moving_interval_seconds: seconds(cfg.moving_interval),
            stationary_interval_seconds: seconds(cfg.stationary_interval),
            protocol_version: frames::PROTOCOL_VERSION,
            ..Default::default()
        }
    }

    // ── Presence ──

    fn set_foreground(&mut self, foreground: bool) {
        self.foreground = foreground;
        let Some(auth) = &self.auth else { return };
        match (foreground, self.counted_online) {
            (true, false) => self.gateway.presence.foreground_started(auth.user_id),
            (false, true) => self.gateway.presence.foreground_ended(auth.user_id),
            _ => return,
        }
        self.counted_online = foreground;
    }

    // ── Topics ──

    async fn subscribe(&mut self, requested: &[String]) -> Result<(), Rejection> {
        let subscriber = self.auth.as_ref().map(|auth| auth.user_id);
        let marks = self.controls_marks(requested);
        let (new_topics, _) = self.plan_subscriptions(requested, subscriber, true).await?;
        let added = new_topics.clone();
        let refused = self.admit(new_topics, subscriber, &marks).await?;
        if !refused.is_empty() {
            // All or nothing, as when the first check refuses.
            self.unsubscribe(&added);
            return Err(topics::FORBIDDEN);
        }
        Ok(())
    }

    /// Where each topic's route stands with control messages, before its
    /// guard is asked (see [`Self::admit`]).
    fn controls_marks(&self, topics: &[String]) -> HashMap<String, Option<ControlsMark>> {
        topics
            .iter()
            .map(|topic| (topic.clone(), self.gateway.hub.controls_mark(&RouteKey::Topic(topic.clone()))))
            .collect()
    }

    /// Joins topics whose guards admitted the subscriber, `marks` being
    /// where each topic's route stood with control messages before the
    /// guard was asked. A revocation for the topic that went by since — or
    /// one this gateway could not hear because the route was only just made
    /// — may have been about this very subscription: those topics are asked
    /// about again, after the join, so the answer reflects everything
    /// committed before the subscription was in place. (One user's own
    /// revocations need no such care: they wait in the queue and are applied
    /// once the topic is subscribed.) Returns the topics refused then
    /// (already left again). A guard that fails then keeps the topic and is
    /// asked again soon: an outage is not a revocation.
    async fn admit(
        &mut self,
        new_topics: Vec<String>,
        subscriber: Option<Uuid>,
        marks: &HashMap<String, Option<ControlsMark>>,
    ) -> Result<Vec<(String, &'static str)>, Rejection> {
        let joined = self.join_topics(new_topics).await?;
        let mut refused = Vec::new();
        for (topic, how) in joined {
            let before = marks.get(&topic).copied().flatten();
            let after = self.gateway.hub.controls_mark(&RouteKey::Topic(topic.clone()));
            if how == Joined::Existing && before.is_some() && before == after {
                continue;
            }
            match topics::authorize(&self.gateway.core.topic_guards, subscriber, &topic).await {
                Ok(()) => {}
                Err(rejection) if rejection == topics::FORBIDDEN => {
                    self.leave_topic(&topic);
                    refused.push((topic, REVOKED_FORBIDDEN));
                }
                Err(_) => {
                    if let Some(due) = self.topics.get_mut(&topic) {
                        *due = Instant::now() + RECHECK_RETRY;
                    }
                }
            }
        }
        Ok(refused)
    }

    /// Validates a subscription request as a whole and returns the topics it
    /// would add. Nothing is changed yet. With `strict`, a topic the guard
    /// refuses refuses the request; otherwise it is left out and returned
    /// second.
    async fn plan_subscriptions(
        &self,
        requested: &[String],
        subscriber: Option<Uuid>,
        strict: bool,
    ) -> Result<(Vec<String>, Vec<String>), Rejection> {
        let max_topics = self.gateway.cfg.max_topics;
        if requested.len() > max_topics {
            return Err(TOO_MANY_TOPICS);
        }
        let mut new_topics: Vec<String> = Vec::new();
        for topic in requested {
            topics::validate(topic)?;
            if !self.topics.contains_key(topic) && !new_topics.contains(topic) {
                new_topics.push(topic.clone());
            }
        }
        if self.topics.len() + new_topics.len() > max_topics {
            return Err(TOO_MANY_TOPICS);
        }
        let mut admitted = Vec::with_capacity(new_topics.len());
        let mut forbidden = Vec::new();
        for topic in new_topics {
            match topics::authorize(&self.gateway.core.topic_guards, subscriber, &topic).await {
                Ok(()) => admitted.push(topic),
                Err(rejection) if rejection == topics::FORBIDDEN && !strict => forbidden.push(topic),
                Err(rejection) => return Err(rejection),
            }
        }
        Ok((admitted, forbidden))
    }

    /// Joins all the given topics, or none of them; says how each was joined.
    async fn join_topics(&mut self, new_topics: Vec<String>) -> Result<Vec<(String, Joined)>, Rejection> {
        let hub = &self.gateway.hub;
        let mut joined = Vec::with_capacity(new_topics.len());
        for topic in new_topics {
            match hub.join(RouteKey::Topic(topic.clone()), &self.client).await {
                Ok(how) => joined.push((topic, how)),
                Err(_) => {
                    for (topic, _) in &joined {
                        hub.leave(&RouteKey::Topic(topic.clone()), &self.client);
                    }
                    return Err(BUS_UNAVAILABLE);
                }
            }
        }
        let due = Instant::now() + self.gateway.cfg.recheck_interval;
        for (topic, _) in &joined {
            self.gateway.audience.touch(topic);
            self.topics.insert(topic.clone(), due);
        }
        Ok(joined)
    }

    fn unsubscribe(&mut self, topics: &[String]) {
        for topic in topics {
            self.leave_topic(topic);
        }
    }

    /// Leaves a topic; whether it was subscribed.
    fn leave_topic(&mut self, topic: &str) -> bool {
        if self.topics.remove(topic).is_none() {
            return false;
        }
        self.gateway.hub.leave(&RouteKey::Topic(topic.to_owned()), &self.client);
        self.gateway.audience.touch(topic);
        true
    }

    /// Ends a subscription on the server's initiative and tells the client.
    fn revoke(&mut self, topic: &str, reason: &str) {
        if self.leave_topic(topic) {
            self.gateway.counters.topics_revoked.incr();
            self.client.reply(frames::unsubscribed(topic, reason));
            tracing::debug!(connection = %self.client.id, topic, reason, "topic subscription revoked");
        }
    }

    // ── Revocations and re-checks ──

    /// Applies what the domains asked for since the last time.
    async fn apply_controls(&mut self) {
        for control in self.client.take_controls() {
            match control {
                SessionControl::Close(topic) => self.revoke(&topic, REVOKED_CLOSED),
                SessionControl::RecheckSoon(topic) => {
                    // Due now: the periodic re-check takes it within the budget.
                    let now = Instant::now();
                    if let Some(due) = self.topics.get_mut(&topic) {
                        *due = (*due).min(now);
                    }
                }
                SessionControl::Recheck(pattern) => {
                    let matching: Vec<String> =
                        self.topics.keys().filter(|topic| pattern.matches(topic)).cloned().collect();
                    for topic in matching {
                        self.recheck(&topic).await;
                    }
                }
            }
        }
    }

    /// Asks the guard again whether this connection may keep `topic`.
    async fn recheck(&mut self, topic: &str) {
        let subscriber = self.auth.as_ref().map(|auth| auth.user_id);
        self.gateway.counters.topic_rechecks.incr();
        let verdict = topics::authorize(&self.gateway.core.topic_guards, subscriber, topic).await;
        let now = Instant::now();
        match verdict {
            Ok(()) => {
                if let Some(due) = self.topics.get_mut(topic) {
                    *due = now + self.gateway.cfg.recheck_interval;
                }
            }
            Err(rejection) if rejection == topics::FORBIDDEN => self.revoke(topic, REVOKED_FORBIDDEN),
            // Undecided (the guard failed): keep it and ask again soon.
            Err(_) => {
                if let Some(due) = self.topics.get_mut(topic) {
                    *due = now + RECHECK_RETRY;
                }
            }
        }
    }

    /// [`RECHECK_POSTPONE`] spread by ±50 % (xorshift; nothing here needs
    /// more than an even spread).
    fn postponement(&mut self) -> Duration {
        self.jitter ^= self.jitter << 13;
        self.jitter ^= self.jitter >> 7;
        self.jitter ^= self.jitter << 17;
        let fraction = (self.jitter % 1000) as f64 / 1000.0;
        RECHECK_POSTPONE.mul_f64(0.5 + fraction)
    }

    /// The periodic safety net: re-checks the subscriptions that are due, at
    /// most `recheck_batch` at once and within the gateway's budget.
    async fn recheck_due(&mut self, now: Instant) {
        let mut due: Vec<(String, Instant)> =
            self.topics.iter().filter(|(_, at)| **at <= now).map(|(topic, at)| (topic.clone(), *at)).collect();
        if due.is_empty() {
            return;
        }
        due.sort_by_key(|(_, at)| *at);
        let batch = self.gateway.cfg.recheck_batch;
        for (index, (topic, _)) in due.iter().enumerate() {
            if index >= batch || !self.gateway.recheck_limit.lock().allow() {
                // The rest wait for a later round, each at its own moment.
                for (topic, _) in &due[index..] {
                    let wait = self.postponement();
                    if let Some(at) = self.topics.get_mut(topic) {
                        *at = now + wait;
                    }
                }
                return;
            }
            self.recheck(topic).await;
        }
    }

    // ── Signals passed on to the domains ──

    fn user_id(&self) -> Result<Uuid, Rejection> {
        self.auth.as_ref().map(|auth| auth.user_id).ok_or(SIGN_IN_REQUIRED)
    }

    /// Passes a location sample (an encoded `LocationSample`) on to the geo
    /// pipeline.
    async fn on_location(&mut self, sample: &[u8]) -> Result<(), Rejection> {
        let auth = self.auth.as_ref().ok_or(SIGN_IN_REQUIRED)?;
        if !self.location_limit.allow() {
            return Err(RATE_LIMITED);
        }
        let received_at = Utc::now();
        let sample = location::decode(sample, received_at)?;
        let report =
            LocationReport { user_id: auth.user_id, session_id: Some(auth.session_id), received_at, sample };
        presence::submit_location(&self.gateway.core.bus, &report).await.map_err(|_| BUS_UNAVAILABLE)?;
        self.gateway.counters.locations_forwarded.incr();
        Ok(())
    }

    /// Publishes a typing / conversation presence signal for the chat domain,
    /// which knows whether the user is in that conversation at all.
    async fn forward_signal<T: Serialize>(
        &mut self,
        subject: &'static str,
        conversation_id: &str,
        signal: impl FnOnce(Uuid, Uuid) -> T,
    ) -> Result<(), Rejection> {
        let user_id = self.user_id()?;
        if !self.signal_limit.allow() {
            return Err(RATE_LIMITED);
        }
        let conversation_id = Uuid::parse_str(conversation_id).map_err(|_| CONVERSATION_ID_INVALID)?;
        let payload = serde_json::to_vec(&signal(user_id, conversation_id)).map_err(|_| FRAME_INVALID)?;
        self.gateway.core.bus.publish(subject, payload).await.map_err(|_| BUS_UNAVAILABLE)
    }
}
