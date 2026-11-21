//! Shared fixtures for the notes integration tests.
//!
//! Every test starts its own app ([`start`]) against the test binary's
//! database and creates its own users. Lists that are global (the global
//! timeline, trending hashtags) are asserted on through things only the
//! test itself made: its own notes, or hashtags from [`unique`].
#![allow(dead_code)] // each test binary uses its own subset

use std::time::Duration;

use serde::{Serialize, de::DeserializeOwned};
use tokuly_core::events::Event;
use tokuly_notes::{Notes, NotesConfig};
use tokuly_proto::{
    common::{PageRequest, Visibility},
    media::{AssetKind, AssetPurpose},
    notes::{notes_event::Event as Push, *},
    realtime::server_frame::Body,
    social::{
        BlockUserRequest, BlockUserResponse, FriendSource, RespondFriendRequestRequest, RespondFriendRequestResponse,
        SendFriendRequestRequest, SendFriendRequestResponse, send_friend_request_request::Target,
    },
};
use tokuly_social::{Social, SocialConfig};
use tokuly_testkit::{FrameListener, RpcError, TestApp, TestUser};
use uuid::Uuid;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0200", "0400", "0500", "0700", "1400"];

/// How long to wait for a realtime frame that should arrive (generous: the
/// machine may be busy compiling something else).
pub const SOON: Duration = Duration::from_secs(5);
/// How long to listen when asserting that nothing arrives.
pub const QUIET: Duration = Duration::from_millis(300);

/// A running app plus the domain behind it, for calling its public Rust
/// API, running its workers and delivering events by hand.
pub struct Ctx {
    pub app: TestApp,
    pub notes: Notes,
}

pub async fn start() -> Ctx {
    start_with(NotesConfig::default()).await
}

pub async fn start_with(cfg: NotesConfig) -> Ctx {
    let mut domain = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        // The social domain is here for its RPCs: tests make friends and
        // block people the way clients do.
        let social = Social::new(core.clone(), identity.clone(), SocialConfig::default()).expect("social domain");
        let notes = Notes::new(core.clone(), identity.clone(), cfg).expect("notes domain");
        let router = notes.register(social.register(identity.register(router)));
        domain = Some(notes);
        router
    })
    .await;
    Ctx { app, notes: domain.expect("register ran") }
}

/// A lowercase string no other test uses, e.g. for hashtags.
pub fn unique(prefix: &str) -> String {
    format!("{prefix}{}", tokuly_core::ids::random_base62(10).to_ascii_lowercase())
}

pub fn page<P: buffa::ProtoBox<PageRequest>>(limit: i32, cursor: &str) -> buffa::MessageField<PageRequest, P> {
    buffa::MessageField::some(PageRequest { limit, cursor: cursor.to_owned(), ..Default::default() })
}

pub fn id(value: &str) -> Uuid {
    value.parse().expect("an ID")
}

/// The texts of the notes of a timeline page, in order.
pub fn texts(entries: &[TimelineEntry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.note.text.as_str()).collect()
}

pub fn note_texts(notes: &[Note]) -> Vec<&str> {
    notes.iter().map(|note| note.text.as_str()).collect()
}

impl Ctx {
    /// Calls `tokuly.notes.v1.NoteService/<method>`.
    pub async fn note<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        method: &str,
        user: Option<&TestUser>,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.app.call(&format!("tokuly.notes.v1.NoteService/{method}"), user, request).await
    }

    /// Calls `tokuly.notes.v1.StoryService/<method>`.
    pub async fn story<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        method: &str,
        user: Option<&TestUser>,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.app.call(&format!("tokuly.notes.v1.StoryService/{method}"), user, request).await
    }

    /// The reason a `NoteService` RPC is rejected with.
    pub async fn note_reason<Req: Serialize>(&self, method: &str, user: Option<&TestUser>, request: &Req) -> String {
        match self.note::<_, serde_json::Value>(method, user, request).await {
            Ok(response) => panic!("NoteService/{method} should fail, got {response}"),
            Err(error) => error.reason,
        }
    }

    /// The reason a `StoryService` RPC is rejected with.
    pub async fn story_reason<Req: Serialize>(&self, method: &str, user: Option<&TestUser>, request: &Req) -> String {
        match self.story::<_, serde_json::Value>(method, user, request).await {
            Ok(response) => panic!("StoryService/{method} should fail, got {response}"),
            Err(error) => error.reason,
        }
    }

    /// A new user who has agreed to the note rules, so they can post.
    pub async fn member(&self, name: &str) -> TestUser {
        let user = self.app.user(name).await;
        let rules: GetNoteRulesResponse =
            self.note("GetNoteRules", Some(&user), &GetNoteRulesRequest::default()).await.unwrap();
        let _: AgreeToNoteRulesResponse = self
            .note(
                "AgreeToNoteRules",
                Some(&user),
                &AgreeToNoteRulesRequest { version: rules.version, ..Default::default() },
            )
            .await
            .unwrap();
        user
    }

    /// Makes two users friends through the social API.
    pub async fn befriend(&self, a: &TestUser, b: &TestUser) {
        let request = SendFriendRequestRequest {
            target: Some(Target::UserId(b.id.to_string())),
            source: FriendSource::FRIEND_SOURCE_PROFILE.into(),
            ..Default::default()
        };
        let sent: SendFriendRequestResponse = self
            .app
            .call("tokuly.social.v1.FriendService/SendFriendRequest", Some(a), &request)
            .await
            .expect("send friend request");
        let Some(request) = sent.request.as_option() else { return };
        let accept = RespondFriendRequestRequest { request_id: request.id.clone(), accept: true, ..Default::default() };
        let _: RespondFriendRequestResponse = self
            .app
            .call("tokuly.social.v1.FriendService/RespondFriendRequest", Some(b), &accept)
            .await
            .expect("accept friend request");
    }

    /// `blocker` blocks `blocked` through the social API.
    pub async fn block(&self, blocker: &TestUser, blocked: &TestUser) {
        let request = BlockUserRequest { user_id: blocked.id.to_string(), ..Default::default() };
        let _: BlockUserResponse =
            self.app.call("tokuly.social.v1.FriendService/BlockUser", Some(blocker), &request).await.expect("block");
    }

    /// Posts a note built by `fill` and returns it.
    pub async fn post_with(&self, user: &TestUser, text: &str, fill: impl FnOnce(&mut CreateNoteRequest)) -> Note {
        let mut request = CreateNoteRequest { text: text.to_owned(), ..Default::default() };
        fill(&mut request);
        let response: CreateNoteResponse = self
            .note("CreateNote", Some(user), &request)
            .await
            .unwrap_or_else(|e| panic!("posting {text:?} failed: {e}"));
        response.note.as_option().cloned().expect("the posted note")
    }

    /// Posts a plain note with the author's default visibility (public).
    pub async fn post(&self, user: &TestUser, text: &str) -> Note {
        self.post_with(user, text, |_| {}).await
    }

    pub async fn post_visible_to(&self, user: &TestUser, text: &str, visibility: Visibility) -> Note {
        self.post_with(user, text, |request| request.visibility = visibility.into()).await
    }

    pub async fn reply(&self, user: &TestUser, to: &Note, text: &str) -> Note {
        self.post_with(user, text, |request| request.reply_to_id = to.id.clone()).await
    }

    pub async fn quote(&self, user: &TestUser, of: &Note, text: &str) -> Note {
        self.post_with(user, text, |request| request.quote_of_id = of.id.clone()).await
    }

    /// The reason posting is rejected with.
    pub async fn post_reason(&self, user: &TestUser, text: &str, fill: impl FnOnce(&mut CreateNoteRequest)) -> String {
        let mut request = CreateNoteRequest { text: text.to_owned(), ..Default::default() };
        fill(&mut request);
        self.note_reason("CreateNote", Some(user), &request).await
    }

    pub async fn get_note(&self, viewer: Option<&TestUser>, note_id: &str) -> Result<Note, RpcError> {
        let response: GetNoteResponse =
            self.note("GetNote", viewer, &GetNoteRequest { note_id: note_id.to_owned(), ..Default::default() }).await?;
        Ok(response.note.as_option().cloned().expect("the note"))
    }

    pub async fn like(&self, user: &TestUser, note: &Note) -> NoteCounters {
        let response: LikeNoteResponse = self
            .note("LikeNote", Some(user), &LikeNoteRequest { note_id: note.id.clone(), ..Default::default() })
            .await
            .expect("like");
        response.counters.as_option().cloned().expect("counters")
    }

    pub async fn renote(&self, user: &TestUser, note: &Note) -> NoteCounters {
        let response: RenoteNoteResponse = self
            .note("RenoteNote", Some(user), &RenoteNoteRequest { note_id: note.id.clone(), ..Default::default() })
            .await
            .expect("renote");
        response.counters.as_option().cloned().expect("counters")
    }

    pub async fn delete_note(&self, user: &TestUser, note: &Note) {
        let _: DeleteNoteResponse = self
            .note("DeleteNote", Some(user), &DeleteNoteRequest { note_id: note.id.clone(), ..Default::default() })
            .await
            .expect("delete note");
    }

    /// A finished upload of `owner`, as the media domain leaves it.
    pub async fn upload(&self, owner: &TestUser, purpose: AssetPurpose, kind: AssetKind) -> String {
        tokuly_media::testing::ready_asset(&self.app.core, owner.id, purpose, kind)
            .await
            .expect("asset fixture")
            .id
            .to_string()
    }

    pub async fn note_image(&self, owner: &TestUser) -> String {
        self.upload(owner, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE, AssetKind::ASSET_KIND_IMAGE).await
    }

    pub async fn note_video(&self, owner: &TestUser) -> String {
        self.upload(owner, AssetPurpose::ASSET_PURPOSE_NOTE_VIDEO, AssetKind::ASSET_KIND_VIDEO).await
    }

    pub async fn story_image(&self, owner: &TestUser) -> String {
        self.upload(owner, AssetPurpose::ASSET_PURPOSE_STORY, AssetKind::ASSET_KIND_IMAGE).await
    }

    /// Posts a story of `photos` photos built by `fill` and returns it.
    pub async fn story_with(
        &self,
        owner: &TestUser,
        title: &str,
        photos: usize,
        fill: impl FnOnce(&mut CreateStoryRequest),
    ) -> Story {
        let mut items = Vec::new();
        for index in 0..photos {
            items.push(StoryItemInput {
                asset_id: self.story_image(owner).await,
                caption: format!("photo {}", index + 1),
                ..Default::default()
            });
        }
        let mut request = CreateStoryRequest { title: title.to_owned(), items, ..Default::default() };
        fill(&mut request);
        let response: CreateStoryResponse = self
            .story("CreateStory", Some(owner), &request)
            .await
            .unwrap_or_else(|e| panic!("posting story {title:?} failed: {e}"));
        response.story.as_option().cloned().expect("the posted story")
    }

    /// Posts a one-photo story for all friends.
    pub async fn post_story(&self, owner: &TestUser, title: &str) -> Story {
        self.story_with(owner, title, 1, |_| {}).await
    }

    pub async fn get_story(&self, viewer: &TestUser, story_id: &str) -> Result<Story, RpcError> {
        let response: GetStoryResponse = self
            .story("GetStory", Some(viewer), &GetStoryRequest { story_id: story_id.to_owned(), ..Default::default() })
            .await?;
        Ok(response.story.as_option().cloned().expect("the story"))
    }

    pub async fn watch(&self, viewer: &TestUser, story: &Story) {
        let _: MarkStoryViewedResponse = self
            .story(
                "MarkStoryViewed",
                Some(viewer),
                &MarkStoryViewedRequest { story_id: story.id.clone(), ..Default::default() },
            )
            .await
            .expect("mark viewed");
    }

    /// Makes a story as old as if it had been posted `hours` ago (and
    /// expired, if that is more than its lifetime).
    pub async fn age_story(&self, story: &Story, hours: i64) {
        sqlx::query("UPDATE stories SET created_at = created_at - make_interval(hours => $2::int), expires_at = expires_at - make_interval(hours => $2::int) WHERE id = $1")
            .bind(id(&story.id))
            .bind(hours as i32)
            .execute(&self.app.core.db)
            .await
            .expect("age story");
    }

    /// Notification requests (`notify.requested`) addressed to `user`, by kind.
    pub async fn notifications_for(&self, user: Uuid, kind: &str) -> Vec<Event> {
        self.app
            .events("notify.requested")
            .await
            .into_iter()
            .filter(|event| {
                event.data["user_id"] == serde_json::json!(user) && event.data["kind"] == serde_json::json!(kind)
            })
            .collect()
    }

    /// Activity entries (`activity.recorded`) of `user`, by kind.
    pub async fn activities_of(&self, user: Uuid, kind: &str) -> Vec<Event> {
        self.app
            .events_by("activity.recorded", user)
            .await
            .into_iter()
            .filter(|event| event.data["kind"] == serde_json::json!(kind))
            .collect()
    }

    /// Search index changes (`search.changed`) about one document, oldest first.
    pub async fn search_changes(&self, kind: &str, doc_id: Uuid) -> Vec<Event> {
        self.app
            .events("search.changed")
            .await
            .into_iter()
            .filter(|event| {
                event.data["kind"] == serde_json::json!(kind) && event.data["id"] == serde_json::json!(doc_id)
            })
            .collect()
    }

    /// Whether the last search change about a document left it in the index.
    pub async fn is_indexed(&self, kind: &str, doc_id: Uuid) -> bool {
        self.search_changes(kind, doc_id).await.last().is_some_and(|event| !event.data["doc"].is_null())
    }
}

/// The next notes event pushed to a listener (other frames are skipped).
pub async fn next_push(listener: &mut FrameListener) -> Option<Push> {
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let left = deadline.checked_duration_since(tokio::time::Instant::now())?;
        match listener.next(left).await?.body {
            Some(Body::Notes(event)) => return event.event,
            _ => continue,
        }
    }
}

/// Every notes event pushed to a listener within `window`.
pub async fn pushes(listener: &mut FrameListener, window: Duration) -> Vec<Push> {
    listener
        .drain(window)
        .await
        .into_iter()
        .filter_map(|frame| match frame.body {
            Some(Body::Notes(event)) => event.event,
            _ => None,
        })
        .collect()
}
