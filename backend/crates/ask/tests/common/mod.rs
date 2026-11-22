//! Shared setup and helpers for the ask integration tests.
#![allow(dead_code)] // each test binary uses its own subset

use std::time::Duration;

use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::{Mutex, MutexGuard};
use tokuly_ask::{Ask, AskConfig};
use tokuly_core::events::Event;
use tokuly_proto::{
    ask::{ask_event::Event as Push, *},
    realtime::server_frame::Body,
    social::{
        BlockUserRequest, BlockUserResponse, FriendSource, RespondFriendRequestRequest, RespondFriendRequestResponse,
        SendFriendRequestRequest, SendFriendRequestResponse, UpdateFriendPrefsRequest, UpdateFriendPrefsResponse,
        send_friend_request_request::Target,
    },
};
use tokuly_social::{Social, SocialConfig};
use tokuly_testkit::{FrameListener, RpcError, TestApp, TestUser};
use uuid::Uuid;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0200", "0400", "0500", "1400", "1800"];

pub const BOX: &str = "tokuly.ask.v1.QuestionBoxService";
pub const PROMPTS: &str = "tokuly.ask.v1.PromptService";
pub const POLLS: &str = "tokuly.ask.v1.ComplimentService";
pub const MODERATION: &str = "tokuly.ask.v1.AskModerationService";
const FRIENDS: &str = "tokuly.social.v1.FriendService";

/// How long to wait for a realtime frame that should arrive (generous: the
/// machine may be busy compiling something else).
pub const SOON: Duration = Duration::from_secs(5);
/// How long to listen when asserting that nothing arrives.
pub const QUIET: Duration = Duration::from_millis(300);

/// Topics the `ask` consumer subscribes to; `Ctx::deliver` replays them.
const CONSUMED_PREFIXES: &[&str] =
    &["ask.", "identity.user.", "social.", "admin.content.removed", "media.asset.deleted"];

/// The tests of one binary share a database and its outbox, and `deliver`
/// replays whatever is in it. Running them one at a time keeps one test's
/// replay from delivering another test's questions.
static ONE_AT_A_TIME: Mutex<()> = Mutex::const_new(());

/// A running test app plus the domain under test (for its worker entry points).
pub struct Ctx {
    pub app: TestApp,
    pub ask: Ask,
    /// Outbox rows up to here were replayed (or predate this test).
    replayed: std::sync::Mutex<Uuid>,
    _turn: MutexGuard<'static, ()>,
}

pub async fn start() -> Ctx {
    start_with(AskConfig::default()).await
}

pub async fn start_with(cfg: AskConfig) -> Ctx {
    let turn = ONE_AT_A_TIME.lock().await;
    let mut ask = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        let social = Social::new(core.clone(), identity.clone(), SocialConfig::default()).expect("social config");
        let domain = Ask::new(core.clone(), identity.clone(), cfg).expect("ask config");
        let router = domain.register(social.register(identity.register(router)));
        ask = Some(domain);
        router
    })
    .await;
    let newest: Option<Uuid> = sqlx::query_scalar("SELECT id FROM outbox ORDER BY id DESC LIMIT 1")
        .fetch_optional(&app.core.db)
        .await
        .expect("read outbox");
    Ctx {
        app,
        ask: ask.expect("register ran"),
        replayed: std::sync::Mutex::new(newest.unwrap_or(Uuid::nil())),
        _turn: turn,
    }
}

impl Ctx {
    /// Calls an RPC of `service`.
    pub async fn call<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        service: &str,
        method: &str,
        user: &TestUser,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.app.call(&format!("{service}/{method}"), Some(user), request).await
    }

    /// Calls an RPC of `service` and returns the response as raw JSON, the
    /// way it goes over the wire.
    pub async fn json<Req: Serialize>(
        &self,
        service: &str,
        method: &str,
        user: &TestUser,
        request: &Req,
    ) -> serde_json::Value {
        self.call(service, method, user, request).await.unwrap_or_else(|e| panic!("{service}/{method} failed: {e}"))
    }

    /// Every outbox row written since the last call (or since the test
    /// began), oldest first.
    pub async fn new_events(&self) -> Vec<Event> {
        let after = *self.replayed.lock().unwrap();
        let rows: Vec<(Uuid, String, Option<Uuid>, serde_json::Value, chrono::DateTime<chrono::Utc>)> =
            sqlx::query_as("SELECT id, topic, actor_id, data, occurred_at FROM outbox WHERE id > $1 ORDER BY id")
                .bind(after)
                .fetch_all(&self.app.core.db)
                .await
                .expect("read outbox");
        if let Some(last) = rows.last() {
            *self.replayed.lock().unwrap() = last.0;
        }
        rows.into_iter()
            .map(|(id, topic, actor_id, data, occurred_at)| Event { id, topic, actor_id, occurred_at, data })
            .collect()
    }

    /// Plays the part of the event relay and the `ask` consumer: hands every
    /// new event the consumer subscribes to to `Ask::handle_event`. Returns
    /// all new events, consumed or not, for inspection.
    pub async fn deliver(&self) -> Vec<Event> {
        let mut seen = Vec::new();
        loop {
            let batch = self.new_events().await;
            if batch.is_empty() {
                return seen;
            }
            for event in batch {
                if CONSUMED_PREFIXES.iter().any(|prefix| event.topic.starts_with(prefix)) {
                    self.ask
                        .handle_event(event.clone())
                        .await
                        .unwrap_or_else(|e| panic!("handling {} failed: {e}", event.topic));
                }
                seen.push(event);
            }
        }
    }

    /// Hands events taken with [`Self::new_events`] to the consumer.
    pub async fn handle(&self, events: &[Event]) {
        for event in events {
            if CONSUMED_PREFIXES.iter().any(|prefix| event.topic.starts_with(prefix)) {
                self.ask
                    .handle_event(event.clone())
                    .await
                    .unwrap_or_else(|e| panic!("handling {} failed: {e}", event.topic));
            }
        }
    }

    /// An event as another domain would announce it.
    pub async fn announce(&self, topic: &str, data: serde_json::Value) {
        let event = Event {
            id: Uuid::now_v7(),
            topic: topic.to_owned(),
            actor_id: None,
            occurred_at: chrono::Utc::now(),
            data,
        };
        self.ask.handle_event(event).await.unwrap_or_else(|e| panic!("handling {topic} failed: {e}"));
    }

    /// Makes two users friends through the social API and lets this domain
    /// hear about it.
    pub async fn befriend(&self, a: &TestUser, b: &TestUser) {
        let request = SendFriendRequestRequest {
            target: Some(Target::UserId(b.id.to_string())),
            source: FriendSource::FRIEND_SOURCE_PROFILE.into(),
            ..Default::default()
        };
        let sent: SendFriendRequestResponse =
            self.call(FRIENDS, "SendFriendRequest", a, &request).await.expect("send friend request");
        if let Some(request) = sent.request.as_option() {
            let accept =
                RespondFriendRequestRequest { request_id: request.id.clone(), accept: true, ..Default::default() };
            let _: RespondFriendRequestResponse =
                self.call(FRIENDS, "RespondFriendRequest", b, &accept).await.expect("accept friend request");
        }
        self.deliver().await;
    }

    /// `blocker` blocks `blocked` in the social graph.
    pub async fn social_block(&self, blocker: &TestUser, blocked: &TestUser) {
        let _: BlockUserResponse = self
            .call(
                FRIENDS,
                "BlockUser",
                blocker,
                &BlockUserRequest { user_id: blocked.id.to_string(), ..Default::default() },
            )
            .await
            .expect("block");
        self.deliver().await;
    }

    /// `user` mutes their friend `friend`.
    pub async fn mute(&self, user: &TestUser, friend: &TestUser) {
        let request =
            UpdateFriendPrefsRequest { user_id: friend.id.to_string(), muted: Some(true), ..Default::default() };
        let _: UpdateFriendPrefsResponse = self.call(FRIENDS, "UpdateFriendPrefs", user, &request).await.expect("mute");
        self.deliver().await;
    }

    /// Opens `owner`'s box, for any signed-in user or for friends only.
    pub async fn open_box(&self, owner: &TestUser, anyone: bool) -> Settings {
        let audience =
            if anyone { AskerAudience::ASKER_AUDIENCE_SIGNED_IN } else { AskerAudience::ASKER_AUDIENCE_FRIENDS };
        self.update_settings(
            owner,
            UpdateSettingsRequest { box_open: Some(true), audience: Some(audience.into()), ..Default::default() },
        )
        .await
    }

    pub async fn update_settings(&self, user: &TestUser, change: UpdateSettingsRequest) -> Settings {
        let response: UpdateSettingsResponse =
            self.call(BOX, "UpdateSettings", user, &change).await.expect("update settings");
        response.settings.as_option().cloned().expect("settings")
    }

    /// `sender` asks `owner` an anonymous question; the worker then runs.
    pub async fn ask(&self, sender: &TestUser, owner: &TestUser, text: &str) -> Question {
        let question = self.try_ask(sender, owner, text, false).await.expect("ask");
        self.deliver().await;
        question
    }

    pub async fn try_ask(
        &self,
        sender: &TestUser,
        owner: &TestUser,
        text: &str,
        signed: bool,
    ) -> Result<Question, RpcError> {
        let request =
            AskQuestionRequest { user: owner.id.to_string(), text: text.to_owned(), signed, ..Default::default() };
        let response: AskQuestionResponse = self.call(BOX, "AskQuestion", sender, &request).await?;
        Ok(response.question.as_option().cloned().expect("question"))
    }

    pub async fn inbox(&self, owner: &TestUser, folder: QuestionFolder) -> Vec<Question> {
        let response: ListInboxResponse = self
            .call(BOX, "ListInbox", owner, &ListInboxRequest { folder: folder.into(), ..Default::default() })
            .await
            .expect("list inbox");
        response.questions
    }

    pub async fn counts(&self, owner: &TestUser) -> InboxCounts {
        let response: GetInboxCountsResponse =
            self.call(BOX, "GetInboxCounts", owner, &GetInboxCountsRequest::default()).await.expect("inbox counts");
        response.counts.as_option().cloned().expect("counts")
    }

    pub async fn asked(&self, sender: &TestUser) -> Vec<Question> {
        let response: ListAskedQuestionsResponse = self
            .call(BOX, "ListAskedQuestions", sender, &ListAskedQuestionsRequest::default())
            .await
            .expect("list asked");
        response.questions
    }

    /// `owner` answers a question with text, for `audience`.
    pub async fn answer(&self, owner: &TestUser, question_id: &str, text: &str, audience: AnswerAudience) -> Question {
        let request = AnswerQuestionRequest {
            question_id: question_id.to_owned(),
            text: text.to_owned(),
            audience: audience.into(),
            ..Default::default()
        };
        let response: AnswerQuestionResponse = self.call(BOX, "AnswerQuestion", owner, &request).await.expect("answer");
        self.deliver().await;
        response.question.as_option().cloned().expect("question")
    }

    /// `owner` blocks whoever sent `question_id`.
    pub async fn block_sender_of_question(&self, owner: &TestUser, question_id: &str) -> SenderBlock {
        let request = BlockSenderRequest {
            target: Some(block_sender_request::Target::QuestionId(question_id.to_owned())),
            ..Default::default()
        };
        let response: BlockSenderResponse = self.call(BOX, "BlockSender", owner, &request).await.expect("block sender");
        response.block.as_option().cloned().expect("block")
    }

    /// Notification requests (`notify.requested`) addressed to `user`, by kind.
    pub async fn notifications_for(&self, user: Uuid, kind: &str) -> Vec<Event> {
        self.app
            .events("notify.requested")
            .await
            .into_iter()
            .filter(|e| e.data["user_id"] == serde_json::json!(user) && e.data["kind"] == serde_json::json!(kind))
            .collect()
    }

    /// Every notification request addressed to `user`.
    pub async fn all_notifications_for(&self, user: Uuid) -> Vec<Event> {
        self.app
            .events("notify.requested")
            .await
            .into_iter()
            .filter(|e| e.data["user_id"] == serde_json::json!(user))
            .collect()
    }

    /// A user with a name that cannot turn up in a response by accident.
    pub async fn user(&self, name: &str) -> TestUser {
        self.app.user(name).await
    }

    /// `count` friends of `user`, each befriended through the social API.
    pub async fn friends_of(&self, user: &TestUser, count: usize) -> Vec<TestUser> {
        let mut friends = Vec::with_capacity(count);
        for n in 0..count {
            let friend = self.app.user(&format!("Friend{n}")).await;
            self.befriend(user, &friend).await;
            friends.push(friend);
        }
        friends
    }

    /// Gives `user` coins to spend.
    pub async fn give_coins(&self, user: &TestUser, coins: i64) {
        tokuly_wallet::ledger::grant(
            &self.app.core,
            user.id,
            coins,
            "test.grant",
            &format!("test.grant:{}", Uuid::now_v7()),
            serde_json::json!({}),
        )
        .await
        .expect("grant coins");
    }

    pub async fn coins(&self, user: &TestUser) -> i64 {
        tokuly_wallet::ledger::balance(&self.app.core, user.id).await.expect("balance").coins
    }

    /// The current round of `user`, started if one is due.
    pub async fn round(&self, user: &TestUser) -> Round {
        let response: GetRoundResponse =
            self.call(POLLS, "GetRound", user, &GetRoundRequest::default()).await.expect("get round");
        response.round.as_option().cloned().expect("round")
    }

    /// Votes for `picked` in `user`'s current poll; the worker then runs.
    pub async fn vote(&self, user: &TestUser, poll_id: &str, picked: &str) -> Round {
        let request = VoteRequest { poll_id: poll_id.to_owned(), user_id: picked.to_owned(), ..Default::default() };
        let response: VoteResponse = self.call(POLLS, "Vote", user, &request).await.expect("vote");
        self.deliver().await;
        response.round.as_option().cloned().expect("round")
    }

    pub async fn flames(&self, receiver: &TestUser) -> Vec<Flame> {
        let response: ListFlamesResponse =
            self.call(POLLS, "ListFlames", receiver, &ListFlamesRequest::default()).await.expect("list flames");
        response.flames
    }

    /// Votes through `voter`'s polls until `target` is among the choices,
    /// votes for them, and returns the round after that vote. Skips polls
    /// that do not show the target.
    pub async fn vote_for(&self, voter: &TestUser, target: &TestUser) -> Round {
        let target_id = target.id.to_string();
        for _ in 0..40 {
            let round = self.round(voter).await;
            let poll = round.poll.as_option().cloned().expect("an active poll");
            if poll.choices.iter().any(|choice| choice.id == target_id) {
                return self.vote(voter, &poll.id, &target_id).await;
            }
            let _: SkipPollResponse = self
                .call(POLLS, "SkipPoll", voter, &SkipPollRequest { poll_id: poll.id.clone(), ..Default::default() })
                .await
                .expect("skip");
        }
        panic!("{} never came up in {}'s polls", target.handle, voter.handle);
    }
}

/// Asserts that `value`, as it goes over the wire, says nothing about
/// `person`: not their ID in either spelling, not their handle, not their
/// name.
pub fn assert_no_trace_of<T: Serialize>(person: &TestUser, name: &str, value: &T, what: &str) {
    let wire = serde_json::to_string(value).expect("serialize");
    let lower = wire.to_lowercase();
    for (label, needle) in [
        ("ID", person.id.to_string()),
        ("ID without dashes", person.id.simple().to_string()),
        ("handle", person.handle.to_lowercase()),
        ("name", name.to_lowercase()),
    ] {
        assert!(!lower.contains(&needle), "{what} contains the sender's {label} ({needle}): {wire}");
    }
}

/// The next ask event pushed to a listener (other frames are skipped).
pub async fn next_push(listener: &mut FrameListener) -> Option<Push> {
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let left = deadline.checked_duration_since(tokio::time::Instant::now())?;
        let frame = listener.next(left).await?;
        if let Some(Body::Ask(event)) = frame.body
            && let Some(push) = event.event
        {
            return Some(push);
        }
    }
}

/// Whether no ask event reaches the listener for a short while.
pub async fn stays_quiet(listener: &mut FrameListener) -> bool {
    listener.drain(QUIET).await.into_iter().all(|frame| !matches!(frame.body, Some(Body::Ask(_))))
}

pub fn id_of(user: &tokuly_proto::common::UserSummary) -> Uuid {
    user.id.parse().expect("user id")
}
