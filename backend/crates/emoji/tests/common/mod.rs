//! Shared fixtures for the emoji integration tests.
//!
//! Every test starts its own app ([`start`]) against the test binary's
//! database and creates its own users. The catalogue is one for the whole
//! database, so tests name their emoji with [`unique`] and assert on their
//! own emoji only: other tests add, change and remove emoji at the same time.
#![allow(dead_code)]

pub mod pack;

use std::time::Duration;

use serde::{Serialize, de::DeserializeOwned};
use tokuly_core::events::Event;
use tokuly_emoji::{Emoji, EmojiConfig};
use tokuly_media::{Media, MediaConfig, testing::ready_asset};
use tokuly_proto::{
    buffa::{MessageField, ProtoBox},
    common::PageRequest,
    emoji::{__buffa::oneof::emoji_event, *},
    media::{AssetKind, AssetPurpose},
    realtime::{ServerFrame, server_frame::Body},
};
use tokuly_testkit::{FrameListener, RpcError, TestApp, TestUser};
use uuid::Uuid;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0400", "1700"];

/// A running app with the emoji domain behind it (for the cross-domain API
/// and for delivering events and running jobs by hand) and the media domain
/// (for processing uploads in place).
pub struct Ctx {
    pub app: TestApp,
    pub emoji: Emoji,
    pub media: Media,
}

pub async fn start() -> Ctx {
    start_with(EmojiConfig::default()).await
}

pub async fn start_with(cfg: EmojiConfig) -> Ctx {
    // Show the cause of internal errors (logged, never sent to clients) in
    // the output of a failing test.
    let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::WARN).with_test_writer().try_init();
    let mut domains = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        let media = Media::new(core.clone(), MediaConfig::default()).expect("media domain");
        let emoji = Emoji::new(core.clone(), identity.clone(), cfg).expect("emoji domain");
        // Profiles are served by identity; the emoji domain adds a card to them.
        let router = emoji.register(identity.register(router));
        domains = Some((emoji, media));
        router
    })
    .await;
    let (emoji, media) = domains.expect("register ran");
    Ctx { app, emoji, media }
}

/// A valid emoji name no other test uses: `prefix` plus random characters.
pub fn unique(prefix: &str) -> String {
    format!("{prefix}{}", tokuly_core::ids::random_base62(10).to_ascii_lowercase())
}

/// An event as a consumer receives it.
pub fn event(topic: &str, data: serde_json::Value) -> Event {
    Event { id: Uuid::now_v7(), topic: topic.to_owned(), actor_id: None, occurred_at: chrono::Utc::now(), data }
}

/// A page request field, whichever way the message stores it.
pub fn page<P: ProtoBox<PageRequest>>(limit: i32, cursor: &str) -> MessageField<PageRequest, P> {
    MessageField::some(PageRequest { limit, cursor: cursor.to_owned(), ..Default::default() })
}

pub fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

/// The emoji called `name` in a list, if it is there.
pub fn named<'a>(emojis: &'a [CustomEmoji], name: &str) -> Option<&'a CustomEmoji> {
    emojis.iter().find(|emoji| emoji.name == name)
}

/// The positions of `names` in a list of emoji (panics when one is missing).
pub fn positions(emojis: &[CustomEmoji], names: &[&str]) -> Vec<usize> {
    names
        .iter()
        .map(|name| {
            emojis.iter().position(|emoji| emoji.name == *name).unwrap_or_else(|| panic!("{name} is not in the list"))
        })
        .collect()
}

/// What an `EmojiEvent` frame carries.
pub fn emoji_event(frame: &ServerFrame) -> Option<&emoji_event::Event> {
    match frame.body.as_ref()? {
        Body::Emoji(event) => event.event.as_ref(),
        _ => None,
    }
}

/// Waits for a frame that `pick` accepts; other frames are skipped.
pub async fn next_matching<T>(
    listener: &mut FrameListener,
    mut pick: impl FnMut(&ServerFrame) -> Option<T>,
) -> Option<T> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while tokio::time::Instant::now() < deadline {
        let frame = listener.next(Duration::from_secs(2)).await;
        if let Some(found) = frame.as_ref().and_then(&mut pick) {
            return Some(found);
        }
    }
    None
}

impl Ctx {
    /// Calls `tokuly.emoji.v1.Emoji<service>Service/<method>`; `service` is
    /// `""` (the catalogue service), `"Submission"` or `"Admin"`.
    pub async fn call<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        service: &str,
        method: &str,
        user: Option<&TestUser>,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.app.call(&format!("tokuly.emoji.v1.Emoji{service}Service/{method}"), user, request).await
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

    pub async fn moderator(&self) -> TestUser {
        self.app.user_with_roles("moderator", &["moderator"]).await
    }

    /// A processed upload of `owner`, of any purpose and kind.
    pub async fn upload(&self, owner: &TestUser, purpose: AssetPurpose, kind: AssetKind) -> Uuid {
        ready_asset(&self.app.core, owner.id, purpose, kind).await.expect("fixture upload").id
    }

    /// A processed still image `owner` uploaded for an emoji.
    pub async fn picture(&self, owner: &TestUser) -> String {
        self.upload(owner, AssetPurpose::ASSET_PURPOSE_STAMP, AssetKind::ASSET_KIND_IMAGE).await.to_string()
    }

    /// A processed animation `owner` uploaded for an emoji.
    pub async fn animation(&self, owner: &TestUser) -> String {
        self.upload(owner, AssetPurpose::ASSET_PURPOSE_STAMP, AssetKind::ASSET_KIND_ANIMATION).await.to_string()
    }

    /// How many holders keep an upload in use.
    pub async fn holders(&self, asset_id: &str) -> i32 {
        sqlx::query_scalar("SELECT holder_count FROM media_assets WHERE id = $1::uuid")
            .bind(asset_id)
            .fetch_one(&self.app.core.db)
            .await
            .expect("upload exists")
    }

    pub async fn submit_request(&self, user: &TestUser, name: &str) -> SubmitEmojiRequest {
        SubmitEmojiRequest { name: name.to_owned(), asset_id: self.picture(user).await, ..Default::default() }
    }

    /// Submits an emoji with a fresh picture; panics when refused.
    pub async fn submit(&self, user: &TestUser, name: &str) -> EmojiDetail {
        let request = self.submit_request(user, name).await;
        let response: SubmitEmojiResponse = self.call("Submission", "SubmitEmoji", Some(user), &request).await.unwrap();
        response.emoji.into_option().unwrap()
    }

    pub async fn add_request(&self, moderator: &TestUser, name: &str) -> AddEmojiRequest {
        AddEmojiRequest { name: name.to_owned(), asset_id: self.picture(moderator).await, ..Default::default() }
    }

    /// Adds an emoji straight to the catalogue, as `moderator`.
    pub async fn add(&self, moderator: &TestUser, name: &str) -> EmojiDetail {
        let request = self.add_request(moderator, name).await;
        self.add_with(moderator, request).await
    }

    pub async fn add_with(&self, moderator: &TestUser, request: AddEmojiRequest) -> EmojiDetail {
        let response: AddEmojiResponse = self.call("Admin", "AddEmoji", Some(moderator), &request).await.unwrap();
        response.emoji.into_option().unwrap()
    }

    pub async fn approve(&self, moderator: &TestUser, emoji_id: &str) -> EmojiDetail {
        let request = ApproveEmojiRequest { emoji_id: emoji_id.to_owned(), ..Default::default() };
        let response: ApproveEmojiResponse =
            self.call("Admin", "ApproveEmoji", Some(moderator), &request).await.unwrap();
        response.emoji.into_option().unwrap()
    }

    pub async fn reject(&self, moderator: &TestUser, emoji_id: &str, reason: &str) -> EmojiDetail {
        let request =
            RejectEmojiRequest { emoji_id: emoji_id.to_owned(), reason: reason.to_owned(), ..Default::default() };
        let response: RejectEmojiResponse = self.call("Admin", "RejectEmoji", Some(moderator), &request).await.unwrap();
        response.emoji.into_option().unwrap()
    }

    pub async fn disable(&self, moderator: &TestUser, emoji_id: &str, reason: &str) -> EmojiDetail {
        let request =
            DisableEmojiRequest { emoji_id: emoji_id.to_owned(), reason: reason.to_owned(), ..Default::default() };
        let response: DisableEmojiResponse =
            self.call("Admin", "DisableEmoji", Some(moderator), &request).await.unwrap();
        response.emoji.into_option().unwrap()
    }

    pub async fn enable(&self, moderator: &TestUser, emoji_id: &str) -> EmojiDetail {
        let request = EnableEmojiRequest { emoji_id: emoji_id.to_owned(), ..Default::default() };
        let response: EnableEmojiResponse = self.call("Admin", "EnableEmoji", Some(moderator), &request).await.unwrap();
        response.emoji.into_option().unwrap()
    }

    pub async fn delete(&self, moderator: &TestUser, emoji_id: &str) {
        let request = DeleteEmojiRequest { emoji_id: emoji_id.to_owned(), ..Default::default() };
        let _: DeleteEmojiResponse = self.call("Admin", "DeleteEmoji", Some(moderator), &request).await.unwrap();
    }

    pub async fn update(&self, moderator: &TestUser, request: UpdateEmojiRequest) -> EmojiDetail {
        let response: UpdateEmojiResponse = self.call("Admin", "UpdateEmoji", Some(moderator), &request).await.unwrap();
        response.emoji.into_option().unwrap()
    }

    pub async fn create_category(&self, moderator: &TestUser, name: &str) {
        let request = CreateCategoryRequest { name: name.to_owned(), ..Default::default() };
        let _: CreateCategoryResponse = self.call("Admin", "CreateCategory", Some(moderator), &request).await.unwrap();
    }

    /// Flags (or unflags) a user as a trusted submitter.
    pub async fn trust(&self, moderator: &TestUser, user: &TestUser, trusted: bool) {
        let request = SetSubmitterTrustedRequest { user_id: user.id.to_string(), trusted, ..Default::default() };
        let _: SetSubmitterTrustedResponse =
            self.call("Admin", "SetSubmitterTrusted", Some(moderator), &request).await.unwrap();
    }

    pub async fn get(&self, viewer: Option<&TestUser>, emoji: &str) -> Result<EmojiDetail, RpcError> {
        let request = GetEmojiRequest { emoji: emoji.to_owned(), ..Default::default() };
        let response: GetEmojiResponse = self.call("", "GetEmoji", viewer, &request).await?;
        Ok(response.emoji.into_option().unwrap())
    }

    /// The whole catalogue, as an anonymous client gets it.
    pub async fn catalog(&self) -> GetCatalogResponse {
        self.call("", "GetCatalog", None, &GetCatalogRequest::default()).await.unwrap()
    }

    pub async fn changes(&self, since_version: i64) -> GetCatalogChangesResponse {
        let request = GetCatalogChangesRequest { since_version, ..Default::default() };
        self.call("", "GetCatalogChanges", None, &request).await.unwrap()
    }

    pub async fn search(&self, query: &str, category: Option<&str>) -> Vec<CustomEmoji> {
        let request = SearchEmojiRequest {
            query: query.to_owned(),
            category: category.map(str::to_owned),
            limit: 100,
            ..Default::default()
        };
        let response: SearchEmojiResponse = self.call("", "SearchEmoji", None, &request).await.unwrap();
        response.emojis
    }

    pub async fn record_use(&self, user: &TestUser, emojis: &[&str]) {
        let request = RecordUseRequest { emojis: strings(emojis), ..Default::default() };
        let _: RecordUseResponse = self.call("", "RecordUse", Some(user), &request).await.unwrap();
    }

    /// Notification requests addressed to `user`, in the order they were made.
    pub async fn notifications(&self, user: &TestUser) -> Vec<serde_json::Value> {
        self.app
            .events("notify.requested")
            .await
            .into_iter()
            .map(|event| event.data)
            .filter(|data| data["user_id"] == serde_json::json!(user.id))
            .collect()
    }

    /// The kinds of the notifications addressed to `user`.
    pub async fn notification_kinds(&self, user: &TestUser) -> Vec<String> {
        self.notifications(user).await.iter().map(|data| data["kind"].as_str().unwrap_or_default().to_owned()).collect()
    }

    /// Payloads of the emoji domain's own events about one emoji.
    pub async fn emoji_events(&self, topic: &str, emoji_id: &str) -> Vec<serde_json::Value> {
        self.app
            .events(topic)
            .await
            .into_iter()
            .map(|event| event.data)
            .filter(|data| data["emoji_id"] == serde_json::json!(emoji_id))
            .collect()
    }

    /// Search index changes for one emoji: `Some(doc)` for an upsert,
    /// `None` for a removal, oldest first.
    pub async fn search_changes(&self, emoji_id: &str) -> Vec<Option<serde_json::Value>> {
        self.app
            .events("search.changed")
            .await
            .into_iter()
            .map(|event| event.data)
            .filter(|data| data["kind"] == "emoji" && data["id"] == serde_json::json!(emoji_id))
            .map(|data| Some(data["doc"].clone()).filter(|doc| !doc.is_null()))
            .collect()
    }
}
