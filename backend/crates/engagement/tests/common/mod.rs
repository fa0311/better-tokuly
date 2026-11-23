//! Shared fixtures for the engagement integration tests.
//!
//! Engagement knows no content of its own, so the tests bring two fake
//! content domains ([`FakeDomain`]: "videos" and "notes") that register
//! subject resolvers exactly like a real domain would, and record the
//! counter callbacks they receive. The social and stamp domains run for
//! real: blocks and stamps are part of what is tested.
//!
//! Every test starts its own app ([`start`]) against the test binary's
//! database and creates its own users and subjects.
#![allow(dead_code)]

use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Serialize, de::DeserializeOwned};
use tokuly_core::{
    events::Event,
    prelude::*,
    subject::{SubjectInfo, SubjectResolver},
};
use tokuly_engagement::{ChannelDirectory, Engagement, EngagementConfig, SubjectKey};
use tokuly_proto::{
    common::{ChannelSummary, Image, PageRequest, SubjectRef, SubjectType},
    engagement::{__buffa::oneof::engagement_event::Event as Pushed, *},
    media::AssetPurpose,
};
use tokuly_social::{Social, SocialConfig};
use tokuly_stamp::{Stamp, StampConfig};
use tokuly_testkit::{FrameListener, RpcError, ServerFrame, TestApp, TestUser};

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0200", "0400", "0500", "1100", "1400"];

/// Who may see a fake subject, besides its owner.
#[derive(Debug, Clone)]
pub enum Audience {
    Everyone,
    Only(Vec<Uuid>),
    Nobody,
}

#[derive(Debug, Clone)]
pub struct FakeSubject {
    pub kind: SubjectType,
    pub id: Uuid,
    pub short_id: String,
    pub owner_id: Uuid,
    pub title: String,
    pub path: String,
    pub audience: Audience,
    pub interactions_enabled: bool,
    /// Withdrawn by its owner after it reached people: it resolves only for
    /// a report (`resolve_for_report`), titled with what they received.
    pub withdrawn: bool,
}

impl FakeSubject {
    pub fn key(&self) -> SubjectKey {
        SubjectKey::new(self.kind, self.id)
    }

    /// The subject as a client names it, by UUID.
    pub fn subject_ref(&self) -> SubjectRef {
        SubjectRef { r#type: self.kind.into(), id: self.id.to_string(), ..Default::default() }
    }

    /// The subject as a client names it, by short ID.
    pub fn short_ref(&self) -> SubjectRef {
        SubjectRef { r#type: self.kind.into(), id: self.short_id.clone(), ..Default::default() }
    }

    pub fn topic(&self) -> String {
        self.key().topic()
    }
}

/// A content domain for tests: holds subjects, resolves them for viewers,
/// and records the counter callbacks engagement sends it.
pub struct FakeDomain {
    kind: SubjectType,
    subjects: Mutex<HashMap<Uuid, FakeSubject>>,
    counters: Mutex<Vec<(Uuid, String, i64)>>,
}

impl FakeDomain {
    fn new(kind: SubjectType) -> Arc<Self> {
        Arc::new(Self { kind, subjects: Mutex::default(), counters: Mutex::default() })
    }

    /// Adds a public subject that accepts interactions.
    pub fn add(&self, owner_id: Uuid, title: &str) -> FakeSubject {
        let id = Uuid::now_v7();
        let short_id = tokuly_core::ids::new_short_id();
        let subject = FakeSubject {
            kind: self.kind,
            id,
            path: format!("/fake/{short_id}"),
            short_id,
            owner_id,
            title: title.to_owned(),
            audience: Audience::Everyone,
            interactions_enabled: true,
            withdrawn: false,
        };
        self.subjects.lock().insert(id, subject.clone());
        subject
    }

    /// Changes a stored subject.
    pub fn update(&self, id: Uuid, change: impl FnOnce(&mut FakeSubject)) {
        change(self.subjects.lock().get_mut(&id).expect("fake subject exists"));
    }

    pub fn remove(&self, id: Uuid) {
        self.subjects.lock().remove(&id);
    }

    /// The sum of the deltas engagement reported for a counter of a subject.
    pub fn counter(&self, id: Uuid, name: &str) -> i64 {
        self.counters
            .lock()
            .iter()
            .filter(|(subject, counter, _)| *subject == id && counter == name)
            .map(|(_, _, delta)| delta)
            .sum()
    }
}

#[async_trait]
impl SubjectResolver for FakeDomain {
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let resolved = self.resolve_for_report(id, viewer).await?;
        let withdrawn = |info: &SubjectInfo| self.subjects.lock().get(&info.id).is_some_and(|subject| subject.withdrawn);
        Ok(resolved.filter(|info| !withdrawn(info)))
    }

    async fn resolve_for_report(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let subjects = self.subjects.lock();
        let Some(subject) = subjects.values().find(|subject| subject.id.to_string() == id || subject.short_id == id)
        else {
            return Ok(None);
        };
        let visible = viewer == Some(subject.owner_id)
            || match &subject.audience {
                Audience::Everyone => true,
                Audience::Only(users) => viewer.is_some_and(|viewer| users.contains(&viewer)),
                Audience::Nobody => false,
            };
        Ok(visible.then(|| SubjectInfo {
            id: subject.id,
            owner_id: subject.owner_id,
            title: subject.title.clone(),
            path: subject.path.clone(),
            image_url: Some(format!("https://media.test/{}.webp", subject.short_id)),
            interactions_enabled: subject.interactions_enabled,
        }))
    }

    async fn counter_changed(&self, id: Uuid, counter: &str, delta: i64) -> AppResult<()> {
        self.counters.lock().push((id, counter.to_owned(), delta));
        Ok(())
    }
}

/// The channel identities of the test users.
#[derive(Default)]
pub struct FakeChannels {
    channels: Mutex<HashMap<Uuid, (Uuid, ChannelSummary)>>,
}

impl FakeChannels {
    /// Gives `owner` a channel and returns its ID.
    pub fn add(&self, owner: &TestUser, name: &str) -> Uuid {
        let id = Uuid::now_v7();
        let summary = ChannelSummary {
            id: id.to_string(),
            handle: format!("ch-{}", &owner.handle),
            name: name.to_owned(),
            icon: Image { url: format!("https://media.test/channel/{id}.webp"), ..Default::default() }.into(),
            ..Default::default()
        };
        self.channels.lock().insert(id, (owner.id, summary));
        id
    }
}

#[async_trait]
impl ChannelDirectory for FakeChannels {
    async fn owned_channel(&self, user_id: Uuid, channel_id: Uuid) -> AppResult<Option<ChannelSummary>> {
        Ok(self
            .channels
            .lock()
            .get(&channel_id)
            .filter(|(owner_id, _)| *owner_id == user_id)
            .map(|(_, summary)| summary.clone()))
    }

    async fn summaries(&self, ids: &[Uuid]) -> AppResult<HashMap<Uuid, ChannelSummary>> {
        let channels = self.channels.lock();
        Ok(ids.iter().filter_map(|id| channels.get(id).map(|(_, summary)| (*id, summary.clone()))).collect())
    }
}

/// A running app with the engagement domain and its fake surroundings.
pub struct Ctx {
    pub app: TestApp,
    pub engagement: Engagement,
    pub videos: Arc<FakeDomain>,
    pub notes: Arc<FakeDomain>,
    pub channels: Arc<FakeChannels>,
}

/// Settings for tests: the comment rate limits are out of the way.
pub fn test_config() -> EngagementConfig {
    EngagementConfig {
        comments_per_minute: 1000,
        comments_per_hour: 10_000,
        reports_per_hour: 1000,
        ..Default::default()
    }
}

pub async fn start() -> Ctx {
    start_with(test_config()).await
}

pub async fn start_with(cfg: EngagementConfig) -> Ctx {
    let videos = FakeDomain::new(SubjectType::SUBJECT_TYPE_VIDEO);
    let notes = FakeDomain::new(SubjectType::SUBJECT_TYPE_NOTE);
    let channels = Arc::new(FakeChannels::default());
    let mut domain = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        core.subjects.register(SubjectType::SUBJECT_TYPE_VIDEO, videos.clone());
        core.subjects.register(SubjectType::SUBJECT_TYPE_NOTE, notes.clone());
        let social = Social::new(core.clone(), identity.clone(), SocialConfig::default()).expect("social domain");
        let stamp =
            Stamp::new(core.clone(), identity.clone(), StampConfig { seed_official: false, ..Default::default() })
                .expect("stamp domain");
        let engagement = Engagement::new(core.clone(), identity.clone(), cfg).expect("engagement domain");
        engagement.set_channel_directory(channels.clone());
        let router = engagement.register(stamp.register(social.register(router)));
        domain = Some(engagement);
        router
    })
    .await;
    Ctx { app, engagement: domain.expect("register ran"), videos, notes, channels }
}

pub fn page<P: buffa::ProtoBox<PageRequest>>(limit: i32, cursor: &str) -> buffa::MessageField<PageRequest, P> {
    buffa::MessageField::some(PageRequest { limit, cursor: cursor.to_owned(), ..Default::default() })
}

pub fn like() -> Reaction {
    Reaction { kind: ReactionKind::REACTION_KIND_LIKE.into(), ..Default::default() }
}

pub fn dislike() -> Reaction {
    Reaction { kind: ReactionKind::REACTION_KIND_DISLIKE.into(), ..Default::default() }
}

pub fn emoji(emoji: &str) -> Reaction {
    Reaction { kind: ReactionKind::REACTION_KIND_EMOJI.into(), emoji: emoji.to_owned(), ..Default::default() }
}

/// A comment as a subject, to like or report it.
pub fn comment_ref(comment_id: &str) -> SubjectRef {
    SubjectRef { r#type: SubjectType::SUBJECT_TYPE_COMMENT.into(), id: comment_id.to_owned(), ..Default::default() }
}

impl Ctx {
    /// Calls `tokuly.engagement.v1.<service>/<method>`.
    pub async fn call<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        service: &str,
        method: &str,
        user: Option<&TestUser>,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.app.call(&format!("tokuly.engagement.v1.{service}/{method}"), user, request).await
    }

    /// The reason an RPC is rejected with.
    pub async fn reason<Req: Serialize>(
        &self,
        service: &str,
        method: &str,
        user: Option<&TestUser>,
        request: &Req,
    ) -> String {
        match self.call::<_, serde_json::Value>(service, method, user, request).await {
            Ok(response) => panic!("{service}/{method} should fail, got {response}"),
            Err(error) => error.reason,
        }
    }

    pub fn video(&self, owner: &TestUser, title: &str) -> FakeSubject {
        self.videos.add(owner.id, title)
    }

    pub fn note(&self, owner: &TestUser, title: &str) -> FakeSubject {
        self.notes.add(owner.id, title)
    }

    /// Writes a top-level text comment.
    pub async fn comment(&self, user: &TestUser, subject: &FakeSubject, text: &str) -> Comment {
        let request =
            CreateCommentRequest { subject: subject.subject_ref().into(), text: text.to_owned(), ..Default::default() };
        let response: CreateCommentResponse =
            self.call("CommentService", "CreateComment", Some(user), &request).await.unwrap();
        response.comment.as_option().expect("a comment").clone()
    }

    /// Replies to a comment (or to a reply).
    pub async fn reply(&self, user: &TestUser, parent_id: &str, text: &str) -> Comment {
        let request =
            CreateCommentRequest { parent_id: parent_id.to_owned(), text: text.to_owned(), ..Default::default() };
        let response: CreateCommentResponse =
            self.call("CommentService", "CreateComment", Some(user), &request).await.unwrap();
        response.comment.as_option().expect("a comment").clone()
    }

    /// The top-level comments of a subject as `viewer` sees them (first page, newest first).
    pub async fn comments(&self, viewer: Option<&TestUser>, subject: &FakeSubject) -> ListCommentsResponse {
        let request = ListCommentsRequest {
            subject: subject.subject_ref().into(),
            sort: CommentSort::COMMENT_SORT_NEWEST.into(),
            ..Default::default()
        };
        self.call("CommentService", "ListComments", viewer, &request).await.unwrap()
    }

    pub async fn get_comment(
        &self,
        viewer: Option<&TestUser>,
        comment_id: &str,
    ) -> Result<GetCommentResponse, RpcError> {
        self.call(
            "CommentService",
            "GetComment",
            viewer,
            &GetCommentRequest { comment_id: comment_id.to_owned(), ..Default::default() },
        )
        .await
    }

    pub async fn react(&self, user: &TestUser, subject: SubjectRef, reaction: Reaction) -> ReactionSummary {
        let request = SetReactionRequest { subject: subject.into(), reaction: reaction.into(), ..Default::default() };
        let response: SetReactionResponse =
            self.call("ReactionService", "SetReaction", Some(user), &request).await.unwrap();
        response.reactions.as_option().expect("a summary").clone()
    }

    pub async fn engagement_of(&self, viewer: Option<&TestUser>, subject: &FakeSubject) -> SubjectEngagement {
        let request = GetEngagementRequest { subject: subject.subject_ref().into(), ..Default::default() };
        let response: GetEngagementResponse =
            self.call("EngagementService", "GetEngagement", viewer, &request).await.unwrap();
        response.engagement.as_option().expect("engagement").clone()
    }

    /// Changes the screening rules of `owner`.
    pub async fn set_rules(&self, owner: &TestUser, hold_all: bool, hold_links: bool, blocked_words: &[&str]) {
        let settings = ModerationSettings {
            hold_all,
            hold_links,
            blocked_words: blocked_words.iter().map(|word| word.to_string()).collect(),
            ..Default::default()
        };
        let request = UpdateModerationSettingsRequest { settings: settings.into(), ..Default::default() };
        let _: UpdateModerationSettingsResponse =
            self.call("CommentModerationService", "UpdateModerationSettings", Some(owner), &request).await.unwrap();
    }

    /// `blocker` blocks `blocked` through the social domain, and engagement
    /// hears about it as it would from the event stream.
    pub async fn block(&self, blocker: &TestUser, blocked: &TestUser) {
        let request = tokuly_proto::social::BlockUserRequest { user_id: blocked.id.to_string(), ..Default::default() };
        let _: tokuly_proto::social::BlockUserResponse =
            self.app.call("tokuly.social.v1.FriendService/BlockUser", Some(blocker), &request).await.unwrap();
        self.deliver(
            "social.user.blocked",
            Some(blocker.id),
            serde_json::json!({ "user_id": blocker.id, "blocked_id": blocked.id }),
        )
        .await;
    }

    /// Hands engagement an event as the consumer would.
    pub async fn deliver(&self, topic: &str, actor_id: Option<Uuid>, data: serde_json::Value) {
        let event =
            Event { id: Uuid::now_v7(), topic: topic.to_owned(), actor_id, occurred_at: chrono::Utc::now(), data };
        self.engagement.handle_event(event).await.unwrap_or_else(|error| panic!("{topic} handler failed: {error}"));
    }

    /// A personal stamp `user` may send, by ID.
    pub async fn stamp(&self, user: &TestUser) -> String {
        let asset = tokuly_media::testing::ready_image(&self.app.core, user.id, AssetPurpose::ASSET_PURPOSE_STAMP)
            .await
            .unwrap();
        let request = tokuly_proto::stamp::CreatePersonalStampRequest {
            asset_id: asset.id.to_string(),
            name: "やあ".into(),
            ..Default::default()
        };
        let response: tokuly_proto::stamp::CreatePersonalStampResponse = self
            .app
            .call("tokuly.stamp.v1.StampLibraryService/CreatePersonalStamp", Some(user), &request)
            .await
            .unwrap();
        response.stamp.stamp_ref.id.clone()
    }

    /// A processed image upload of `user`, by asset ID.
    pub async fn image(&self, user: &TestUser, purpose: AssetPurpose) -> Uuid {
        tokuly_media::testing::ready_image(&self.app.core, user.id, purpose).await.unwrap().id
    }

    /// The notifications of `kind` requested for `user_id` so far.
    pub async fn notifications(&self, user_id: Uuid, kind: &str) -> Vec<serde_json::Value> {
        self.app
            .events("notify.requested")
            .await
            .into_iter()
            .map(|event| event.data)
            .filter(|data| data["user_id"] == serde_json::json!(user_id) && data["kind"] == kind)
            .collect()
    }

    /// Every notification requested for `user_id` so far, as its kind.
    pub async fn notification_kinds(&self, user_id: Uuid) -> Vec<String> {
        self.app
            .events("notify.requested")
            .await
            .into_iter()
            .filter(|event| event.data["user_id"] == serde_json::json!(user_id))
            .map(|event| event.data["kind"].as_str().unwrap_or_default().to_owned())
            .collect()
    }

    /// The payloads of the events on `topic` that are about `subject_id`.
    pub async fn events_about(&self, topic: &str, subject_id: Uuid) -> Vec<serde_json::Value> {
        self.app
            .events(topic)
            .await
            .into_iter()
            .map(|event| event.data)
            .filter(|data| data["subject_id"] == serde_json::json!(subject_id))
            .collect()
    }
}

/// The engagement events among realtime frames.
pub fn pushed(frames: Vec<ServerFrame>) -> Vec<Pushed> {
    frames.iter().filter_map(|frame| frame.event::<EngagementEvent>()?.event).collect()
}

/// Everything pushed to a listener by what just happened: waits generously
/// for the first frame (the machine may be busy), then until the stream has
/// been quiet for a moment. Expecting nothing therefore takes the full wait.
pub async fn drain(listener: &mut FrameListener) -> Vec<Pushed> {
    let mut frames = Vec::new();
    let mut patience = Duration::from_secs(3);
    while let Some(frame) = listener.next(patience).await {
        frames.push(frame);
        patience = Duration::from_millis(400);
    }
    pushed(frames)
}

/// IDs of comments, for comparing lists.
pub fn ids(comments: &[Comment]) -> Vec<&str> {
    comments.iter().map(|comment| comment.id.as_str()).collect()
}

/// Texts of comments, for comparing lists.
pub fn texts(comments: &[Comment]) -> Vec<&str> {
    comments.iter().map(|comment| comment.text.as_str()).collect()
}
