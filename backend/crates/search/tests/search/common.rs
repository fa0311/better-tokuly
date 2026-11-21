//! Shared setup and helpers for the search integration tests.
//!
//! Every test binary gets its own index prefix (`tokuly_test_<unix>_<random>`),
//! which also namespaces its Valkey keys. All tests of the binary share the
//! twelve indexes under it, so each test works with its own users and with
//! [`unique`] words nobody else searches for. The indexes are deleted when
//! the last test holding them finishes ([`IndexLease`]); what a crashed run
//! leaves behind is removed by the next run.
#![allow(dead_code)] // each test binary uses its own subset

use std::{
    sync::LazyLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use meilisearch_sdk::client::Client;
use serde::{Serialize, de::DeserializeOwned};
use tokuly_core::{
    events::Event,
    outbound::{SEARCH_TOPIC, SearchChange, SearchDoc},
};
use tokuly_proto::search::*;
use tokuly_search::{Search, SearchConfig, kinds::KINDS};
use tokuly_social::{Social, SocialConfig};
use tokuly_testkit::{RpcError, TestApp, TestUser};
use uuid::Uuid;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0200", "0400", "1500"];

pub const SEARCH: &str = "tokuly.search.v1.SearchService";
pub const ADMIN: &str = "tokuly.search.v1.SearchAdminService";
pub const FRIENDS: &str = "tokuly.social.v1.FriendService";

const TEST_PREFIX: &str = "tokuly_test_";
/// Test indexes older than this belong to a run that crashed.
const STALE_AFTER: Duration = Duration::from_secs(20 * 60);
/// How long the last test to finish waits for another to start before it
/// deletes the indexes (so they are not rebuilt between two tests).
const TEARDOWN_GRACE: Duration = Duration::from_millis(250);

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_secs()
}

/// Lowercase letters only, so the result is a single word for the engine.
fn random_letters(count: usize) -> String {
    Uuid::new_v4().as_bytes().iter().cycle().take(count).map(|b| (b'a' + b % 26) as char).collect()
}

/// The index prefix of this test binary.
pub static PREFIX: LazyLock<String> = LazyLock::new(|| format!("{TEST_PREFIX}{}_{}", unix_now(), random_letters(6)));

struct SharedIndexes {
    /// Tests currently using the indexes.
    users: usize,
    /// The indexes exist with their settings.
    ready: bool,
    /// Leftovers of crashed runs were looked for.
    swept: bool,
}

static SHARED: tokio::sync::Mutex<SharedIndexes> =
    tokio::sync::Mutex::const_new(SharedIndexes { users: 0, ready: false, swept: false });

/// Keeps the binary's indexes alive while a test uses them.
pub struct IndexLease {
    engine_url: String,
    engine_key: String,
}

impl IndexLease {
    async fn acquire(search: &Search) -> Self {
        let mut shared = SHARED.lock().await;
        if !shared.swept {
            delete_stale_indexes(&search.core.search).await;
            shared.swept = true;
        }
        if !shared.ready {
            search.ensure_indexes().await.expect("test indexes are created");
            shared.ready = true;
        }
        shared.users += 1;
        Self { engine_url: search.core.cfg.meili_url.clone(), engine_key: search.core.cfg.meili_key.clone() }
    }
}

impl Drop for IndexLease {
    fn drop(&mut self) {
        let (url, key) = (self.engine_url.clone(), self.engine_key.clone());
        // Async work from a destructor: its own thread and runtime, and the
        // test waits for it so the process cannot exit mid-cleanup.
        let cleanup = std::thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
            runtime.block_on(async {
                {
                    let mut shared = SHARED.lock().await;
                    shared.users = shared.users.saturating_sub(1);
                    if shared.users > 0 {
                        return;
                    }
                }
                tokio::time::sleep(TEARDOWN_GRACE).await;
                let mut shared = SHARED.lock().await;
                if shared.users == 0 && shared.ready {
                    delete_indexes_with_prefix(&url, &key, &PREFIX).await;
                    shared.ready = false;
                }
            });
        });
        let _ = cleanup.join();
    }
}

/// Deletes the twelve indexes of a prefix and waits for the engine.
pub async fn delete_indexes_with_prefix(url: &str, key: &str, prefix: &str) {
    let Ok(client) = Client::new(url, Some(key)) else { return };
    let mut tasks = Vec::new();
    for kind in KINDS {
        if let Ok(task) = client.delete_index(format!("{prefix}_{}", kind.name)).await {
            tasks.push(task);
        }
    }
    for task in tasks {
        let _ = task.wait_for_completion(&client, Some(Duration::from_millis(25)), Some(Duration::from_secs(60))).await;
    }
}

/// Removes test indexes a crashed run left behind.
async fn delete_stale_indexes(client: &Client) {
    let mut query = meilisearch_sdk::indexes::IndexesQuery::new(client);
    query.with_limit(1000);
    let Ok(indexes) = query.execute().await else { return };
    let now = unix_now();
    for index in indexes.results {
        let created = index.uid.strip_prefix(TEST_PREFIX).and_then(|rest| rest.split('_').next()).and_then(|t| t.parse::<u64>().ok());
        if created.is_some_and(|created| now.saturating_sub(created) > STALE_AFTER.as_secs()) {
            let _ = client.delete_index(&index.uid).await;
        }
    }
}

/// A running test app plus the domain under test.
pub struct Ctx {
    pub app: TestApp,
    pub search: Search,
    _indexes: Option<IndexLease>,
}

pub async fn start() -> Ctx {
    start_with(|_| {}).await
}

/// Like [`start`], with settings changed. Tests share the binary's prefix
/// unless they set another one (and then look after those indexes
/// themselves).
pub async fn start_with(configure: impl FnOnce(&mut SearchConfig)) -> Ctx {
    let mut cfg = SearchConfig { index_prefix: PREFIX.clone(), trending_min_users: 3, ..Default::default() };
    configure(&mut cfg);
    let shared_prefix = cfg.index_prefix == *PREFIX;
    let mut holder = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        let social = Social::new(core.clone(), identity.clone(), SocialConfig::default()).expect("social starts");
        let search = Search::new(core.clone(), identity.clone(), cfg).expect("search starts");
        let router = search.register(social.register(router));
        holder = Some(search);
        router
    })
    .await;
    let search = holder.expect("register ran");
    let lease = if shared_prefix { Some(IndexLease::acquire(&search).await) } else { None };
    Ctx { app, search, _indexes: lease }
}

// ───────────────────────────── documents and events ─────────────────────────────

/// A word no other test uses: `<label>` followed by random letters.
pub fn unique(label: &str) -> String {
    format!("{label}{}", random_letters(12))
}

/// A public document with sensible defaults.
pub fn doc(kind: &str, owner: Option<Uuid>, title: &str) -> SearchDoc {
    SearchDoc {
        kind: kind.to_owned(),
        id: Uuid::now_v7(),
        title: title.to_owned(),
        text: String::new(),
        tags: Vec::new(),
        owner_id: owner,
        public: true,
        image_url: None,
        path: format!("/{kind}/test"),
        created_at: unix_now() as i64,
        score: 0.0,
        extra: serde_json::json!({}),
    }
}

/// An event as the outbox relay would deliver it, with a given ID.
pub fn event_with_id(id: Uuid, topic: &str, data: impl Serialize) -> Event {
    Event {
        id,
        topic: topic.to_owned(),
        actor_id: None,
        occurred_at: chrono::Utc::now(),
        data: serde_json::to_value(data).expect("event payload"),
    }
}

/// An event with a fresh (newest) ID.
pub fn event(topic: &str, data: impl Serialize) -> Event {
    event_with_id(Uuid::now_v7(), topic, data)
}

/// What `outbound::index` emits for a document.
pub fn upsert_of(doc: &SearchDoc) -> SearchChange {
    SearchChange { kind: doc.kind.clone(), id: doc.id, doc: doc.public.then(|| doc.clone()) }
}

/// What `outbound::unindex` emits.
pub fn removal_of(kind: &str, id: Uuid) -> SearchChange {
    SearchChange { kind: kind.to_owned(), id, doc: None }
}

pub fn user_ref(user_id: Uuid) -> serde_json::Value {
    serde_json::json!({ "user_id": user_id })
}

// ───────────────────────────── responses ─────────────────────────────

pub fn hit_ids(hits: &[SearchHit]) -> Vec<String> {
    hits.iter().map(|hit| hit.id.clone()).collect()
}

/// The matching runs of a segmented text.
pub fn highlighted(segments: &[TextSegment]) -> Vec<&str> {
    segments.iter().filter(|s| s.highlighted).map(|s| s.text.as_str()).collect()
}

/// A segmented text put back together.
pub fn joined(segments: &[TextSegment]) -> String {
    segments.iter().map(|s| s.text.as_str()).collect()
}

pub fn kind_of(hit: &SearchHit) -> SearchKind {
    hit.kind.as_known().expect("known kind")
}

/// The count of one kind in a response's tab counts (0 when absent).
pub fn count_of(response: &SearchResponse, kind: SearchKind) -> i64 {
    response.counts.iter().find(|c| c.kind.as_known() == Some(kind)).map(|c| c.count).unwrap_or(0)
}

impl Ctx {
    /// Delivers a document to the consumer as a `search.changed` event.
    pub async fn publish(&self, doc: &SearchDoc) {
        self.search.handle_event(event(SEARCH_TOPIC, upsert_of(doc))).await.expect("search.changed is handled");
    }

    /// Delivers an unindex request to the consumer.
    pub async fn unpublish(&self, kind: &str, id: Uuid) {
        self.search.handle_event(event(SEARCH_TOPIC, removal_of(kind, id))).await.expect("search.changed is handled");
    }

    /// Delivers an event to the consumer.
    pub async fn deliver(&self, event: Event) {
        self.search.handle_event(event).await.expect("event is handled");
    }

    /// Writes everything pending to the engine and waits for it.
    pub async fn flush(&self) {
        self.search.flush().await.expect("indexer pass");
    }

    /// Publishes documents and waits until they are searchable.
    pub async fn index(&self, docs: &[SearchDoc]) {
        for doc in docs {
            self.publish(doc).await;
        }
        self.flush().await;
    }

    /// Indexes a test user (created without an event) as identity's
    /// `identity.user.created` would.
    pub async fn index_user(&self, user: &TestUser) {
        self.deliver(event("identity.user.created", user_ref(user.id))).await;
    }

    pub async fn call<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        service: &str,
        method: &str,
        user: Option<&TestUser>,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.app.call(&format!("{service}/{method}"), user, request).await
    }

    pub async fn try_search(&self, user: Option<&TestUser>, request: &SearchRequest) -> Result<SearchResponse, RpcError> {
        self.call(SEARCH, "Search", user, request).await
    }

    pub async fn search(&self, user: Option<&TestUser>, request: &SearchRequest) -> SearchResponse {
        self.try_search(user, request).await.expect("Search succeeds")
    }

    /// Searches every kind for `query`.
    pub async fn find(&self, user: Option<&TestUser>, query: &str) -> SearchResponse {
        self.search(user, &SearchRequest { query: query.to_owned(), ..Default::default() }).await
    }

    /// Searches every kind for `query` among one owner's documents: how tests
    /// with everyday words (渋谷) keep other tests' documents out.
    pub async fn find_owned(&self, user: Option<&TestUser>, owner: Uuid, query: &str) -> SearchResponse {
        self.search(user, &SearchRequest { query: query.to_owned(), owner_id: owner.to_string(), ..Default::default() }).await
    }

    pub async fn suggest(&self, user: Option<&TestUser>, query: &str) -> SuggestResponse {
        self.call(SEARCH, "Suggest", user, &SuggestRequest { query: query.to_owned(), ..Default::default() })
            .await
            .expect("Suggest succeeds")
    }

    pub async fn trending(&self, user: Option<&TestUser>) -> Vec<String> {
        let response: GetTrendingResponse = self
            .call(SEARCH, "GetTrending", user, &GetTrendingRequest { limit: 20, ..Default::default() })
            .await
            .expect("GetTrending succeeds");
        response.queries.into_iter().map(|q| q.query).collect()
    }

    pub async fn recent(&self, user: &TestUser) -> ListRecentSearchesResponse {
        self.call(SEARCH, "ListRecentSearches", Some(user), &ListRecentSearchesRequest::default())
            .await
            .expect("ListRecentSearches succeeds")
    }

    /// `blocker` blocks `blocked` through the social domain.
    pub async fn block(&self, blocker: &TestUser, blocked: &TestUser) {
        let _: tokuly_proto::social::BlockUserResponse = self
            .call(
                FRIENDS,
                "BlockUser",
                Some(blocker),
                &tokuly_proto::social::BlockUserRequest { user_id: blocked.id.to_string(), ..Default::default() },
            )
            .await
            .expect("BlockUser succeeds");
    }
}
