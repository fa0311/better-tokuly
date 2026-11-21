//! Shared setup and helpers for the feed integration tests.
#![allow(dead_code)] // each test binary uses its own subset

pub mod server;

use chrono::{DateTime, Duration, Utc};
use serde::{Serialize, de::DeserializeOwned};
use tokuly_core::{
    events::Event,
    outbound::{ActivityAudience, ActivityEntry, Localized},
};
use tokuly_feed::{Feed, FeedConfig, IngestOutcome};
use tokuly_proto::{
    common::{PageRequest, SubjectType},
    feed::*,
    social::{BlockUserRequest, BlockUserResponse, FriendSource, RemoveFriendRequest, RemoveFriendResponse, RespondFriendRequestRequest, RespondFriendRequestResponse, SendFriendRequestRequest, SendFriendRequestResponse, send_friend_request_request},
};
use tokuly_social::{Social, SocialConfig};
use tokuly_testkit::{RpcError, TestApp, TestUser};
use uuid::Uuid;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0200", "0400", "1300"];

pub const ACTIVITY: &str = "tokuly.feed.v1.ActivityService";
pub const STORIES: &str = "tokuly.feed.v1.MapStoryService";
pub const HOME: &str = "tokuly.feed.v1.HomeFeedService";
pub const NEWS: &str = "tokuly.feed.v1.NewsService";
pub const NEWS_ADMIN: &str = "tokuly.feed.v1.NewsAdminService";
pub const RECAP: &str = "tokuly.feed.v1.RecapService";
const FRIENDS: &str = "tokuly.social.v1.FriendService";

/// A running test app plus the domain under test (for its worker entry points).
pub struct Ctx {
    pub app: TestApp,
    pub feed: Feed,
}

/// Settings for tests: the news fetcher may reach the local feed server,
/// and its limits are small enough to hit quickly.
pub fn test_config() -> FeedConfig {
    FeedConfig { news_allow_loopback: true, news_fetch_timeout_seconds: 3, news_max_feed_bytes: 64 * 1024, ..Default::default() }
}

pub async fn start() -> Ctx {
    start_with(test_config()).await
}

pub async fn start_with(cfg: FeedConfig) -> Ctx {
    let mut feed = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        let social = Social::new(core.clone(), identity.clone(), SocialConfig::default()).expect("social config");
        let domain = Feed::new(core.clone(), identity.clone(), cfg).expect("feed config");
        let router = domain.register(social.register(router));
        feed = Some(domain);
        router
    })
    .await;
    Ctx { app, feed: feed.expect("register ran") }
}

impl Ctx {
    /// Calls an RPC of `service` as `user` (`None`: anonymously).
    pub async fn call<Req: Serialize, Resp: DeserializeOwned>(&self, service: &str, method: &str, user: Option<&TestUser>, request: &Req) -> Result<Resp, RpcError> {
        self.app.call(&format!("{service}/{method}"), user, request).await
    }

    /// Makes two users friends through the social API.
    pub async fn befriend(&self, a: &TestUser, b: &TestUser) {
        let request = SendFriendRequestRequest {
            target: Some(send_friend_request_request::Target::UserId(b.id.to_string())),
            source: FriendSource::FRIEND_SOURCE_PROFILE.into(),
            ..Default::default()
        };
        let sent: SendFriendRequestResponse = self.call(FRIENDS, "SendFriendRequest", Some(a), &request).await.expect("send friend request");
        let request_id = sent.request.as_option().expect("a pending request").id.clone();
        let _: RespondFriendRequestResponse =
            self.call(FRIENDS, "RespondFriendRequest", Some(b), &RespondFriendRequestRequest { request_id, accept: true, ..Default::default() }).await.expect("accept");
    }

    pub async fn unfriend(&self, a: &TestUser, b: &TestUser) {
        let _: RemoveFriendResponse = self.call(FRIENDS, "RemoveFriend", Some(a), &RemoveFriendRequest { user_id: b.id.to_string(), ..Default::default() }).await.expect("remove friend");
    }

    pub async fn block(&self, blocker: &TestUser, blocked: &TestUser) {
        let _: BlockUserResponse = self.call(FRIENDS, "BlockUser", Some(blocker), &BlockUserRequest { user_id: blocked.id.to_string(), ..Default::default() }).await.expect("block");
    }

    /// Delivers an `activity.recorded` event that happened at `at`, the way
    /// the event stream would. Returns what the consumer did with it.
    pub async fn deliver(&self, entry: &ActivityEntry, at: DateTime<Utc>) -> IngestOutcome {
        self.feed.handle_activity_recorded(&activity_event(entry, at)).await.expect("ingest")
    }

    /// Records an activity that happened at `at`; returns the entry's ID.
    pub async fn record_at(&self, entry: ActivityEntry, at: DateTime<Utc>) -> Uuid {
        match self.deliver(&entry, at).await {
            IngestOutcome::Stored(id) => id,
            other => panic!("the entry was not stored: {other:?}"),
        }
    }

    /// Records an activity that happened `minutes_ago` minutes ago.
    pub async fn record(&self, entry: ActivityEntry, minutes_ago: i64) -> Uuid {
        self.record_at(entry, Utc::now() - Duration::minutes(minutes_ago)).await
    }

    pub async fn set_locale(&self, user: &TestUser, locale: &str) {
        sqlx::query("UPDATE users SET locale = $2 WHERE id = $1").bind(user.id).bind(locale).execute(&self.app.core.db).await.expect("set locale");
    }

    pub async fn set_time_zone(&self, user: &TestUser, time_zone: &str) {
        sqlx::query("UPDATE users SET time_zone = $2 WHERE id = $1").bind(user.id).bind(time_zone).execute(&self.app.core.db).await.expect("set time zone");
    }

    /// The caller's own log, newest first (first page of up to 100).
    pub async fn my_log(&self, user: &TestUser) -> Vec<Activity> {
        let response: ListMyActivityResponse = self.call(ACTIVITY, "ListMyActivity", Some(user), &ListMyActivityRequest { page: page(100, "").into(), ..Default::default() }).await.expect("my log");
        response.activities
    }

    /// A profile's activity list as `viewer` sees it.
    pub async fn profile(&self, viewer: Option<&TestUser>, owner: Uuid) -> Result<Vec<Activity>, RpcError> {
        let response: ListUserActivityResponse =
            self.call(ACTIVITY, "ListUserActivity", viewer, &ListUserActivityRequest { user_id: owner.to_string(), page: page(100, "").into(), ..Default::default() }).await?;
        Ok(response.activities)
    }

    /// The friends' feed of `user` (first page of up to 100 cards).
    pub async fn friends_feed(&self, user: &TestUser) -> Vec<ActivityCard> {
        let response: ListFriendsActivityResponse =
            self.call(ACTIVITY, "ListFriendsActivity", Some(user), &ListFriendsActivityRequest { page: page(100, "").into(), ..Default::default() }).await.expect("friends feed");
        response.cards
    }

    /// The home feed of `user` (first page of up to 50 cards).
    pub async fn home_feed(&self, user: &TestUser) -> Vec<ActivityCard> {
        let response: ListHomeFeedResponse = self.call(HOME, "ListHomeFeed", Some(user), &ListHomeFeedRequest { page: page(50, "").into(), ..Default::default() }).await.expect("home feed");
        response.cards
    }
}

impl Ctx {
    /// A user holding the `admin` role.
    pub async fn admin(&self) -> TestUser {
        self.app.user_with_roles("admin", &["admin"]).await
    }

    /// Creates a news source through the admin API.
    pub async fn create_source(&self, admin: &TestUser, request: CreateNewsSourceRequest) -> ManagedNewsSource {
        let response: CreateNewsSourceResponse = self.call(NEWS_ADMIN, "CreateNewsSource", Some(admin), &request).await.expect("create news source");
        response.source.into_option().expect("the created source")
    }

    /// A source for `feed_url` with a language of its own, so that lists
    /// filtered by it only hold this test's articles.
    pub async fn source_for(&self, admin: &TestUser, feed_url: &str, language: &str) -> ManagedNewsSource {
        self.create_source(admin, CreateNewsSourceRequest { feed_url: feed_url.to_owned(), language: language.to_owned(), ..Default::default() }).await
    }

    pub async fn managed_source(&self, admin: &TestUser, source_id: &str) -> ManagedNewsSource {
        let response: ListManagedNewsSourcesResponse = self.call(NEWS_ADMIN, "ListManagedNewsSources", Some(admin), &ListManagedNewsSourcesRequest::default()).await.expect("list sources");
        response.sources.into_iter().find(|source| source.id == source_id).expect("the source is listed")
    }

    pub async fn list_news(&self, user: Option<&TestUser>, request: ListNewsRequest) -> ListNewsResponse {
        self.call(NEWS, "ListNews", user, &request).await.expect("list news")
    }

    /// Every article of a source, newest first.
    pub async fn articles_of(&self, source_id: &str) -> Vec<NewsArticle> {
        self.list_news(None, ListNewsRequest { source_id: source_id.to_owned(), page: page(50, "").into(), ..Default::default() }).await.articles
    }

    /// `search.changed` events about one document, oldest first.
    pub async fn search_changes(&self, id: &str) -> Vec<Event> {
        self.app.events("search.changed").await.into_iter().filter(|event| event.data["id"] == serde_json::json!(id)).collect()
    }
}

/// A short random lowercase token, to keep one test's URLs, tags and
/// languages apart from another's (tests share a database).
pub fn token() -> String {
    tokuly_core::ids::random_base62(8).to_ascii_lowercase()
}

/// The event the outbox relay would deliver for an activity entry.
pub fn activity_event(entry: &ActivityEntry, at: DateTime<Utc>) -> Event {
    Event { id: Uuid::now_v7(), topic: "activity.recorded".to_owned(), actor_id: Some(entry.user_id), occurred_at: at, data: serde_json::to_value(entry).expect("entry serializes") }
}

/// Any other event, as the relay would deliver it now.
pub fn event(topic: &str, actor: Option<Uuid>, data: serde_json::Value) -> Event {
    Event { id: Uuid::now_v7(), topic: topic.to_owned(), actor_id: actor, occurred_at: Utc::now(), data }
}

pub fn page(limit: i32, cursor: &str) -> PageRequest {
    PageRequest { limit, cursor: cursor.to_owned(), ..Default::default() }
}

/// An entry of `kind` by `user` with a summary in both languages.
pub fn entry(user: &TestUser, kind: &str, audience: ActivityAudience) -> ActivityEntry {
    ActivityEntry::new(user.id, kind, audience, Localized::new(format!("{kind} しました"), format!("did {kind}")))
}

/// A note posted to friends, pointing at a fresh note.
pub fn note(user: &TestUser) -> ActivityEntry {
    let note_id = Uuid::now_v7();
    entry(user, "note.posted", ActivityAudience::Friends).subject(SubjectType::SUBJECT_TYPE_NOTE, note_id).path(format!("/notes/{note_id}"))
}

/// A bump of `user` with `partner`, as the social domain records it for `user`.
pub fn bump(user: &TestUser, partner: &TestUser, lat: f64, lng: f64) -> ActivityEntry {
    entry(user, "social.bumped", ActivityAudience::Friends)
        .subject(SubjectType::SUBJECT_TYPE_USER, partner.id)
        .with([partner.id])
        .path(format!("/u/{}", partner.handle))
        .at(lat, lng)
        .data(serde_json::json!({ "bump_id": Uuid::now_v7() }))
}

pub fn ids(activities: &[Activity]) -> Vec<String> {
    activities.iter().map(|activity| activity.id.clone()).collect()
}

pub fn kinds_of(activities: &[Activity]) -> Vec<&str> {
    activities.iter().map(|activity| activity.kind.as_str()).collect()
}

/// The actor of a card's newest entry.
pub fn actor_of(card: &ActivityCard) -> Uuid {
    card.activity.as_option().and_then(|activity| activity.actor.as_option()).expect("a card has an actor").id.parse().expect("user id")
}

/// The ID of a card's newest entry.
pub fn head_of(card: &ActivityCard) -> &str {
    &card.activity.as_option().expect("a card has an entry").id
}
