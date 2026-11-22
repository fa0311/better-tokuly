//! Shared fixtures for the chat integration tests.
//!
//! Every test starts its own app ([`start`]) against the test binary's
//! database and creates its own users. Friendships, blocks, privacy
//! settings and stamps are set up through the owning domains' own RPCs, so
//! the chat domain is tested against what those domains really store.
#![allow(dead_code)]

use std::time::Duration;

use serde::{Serialize, de::DeserializeOwned};
use tokuly_chat::{Chat, ChatConfig};
use tokuly_core::outbound::NotificationRequest;
use tokuly_identity::settings;
use tokuly_proto::{
    chat::{chat_event::Event as Push, *},
    identity::{Audience, PrivacySettings, UpdateSettingsRequest},
    media::{AssetKind, AssetPurpose},
    social::{BlockUserRequest, SendFriendRequestRequest, UnblockUserRequest, send_friend_request_request::Target},
    stamp::{CreatePersonalStampRequest, CreatePersonalStampResponse},
};
use tokuly_social::{Social, SocialConfig};
use tokuly_stamp::{Stamp, StampConfig};
use tokuly_testkit::{FrameListener, RpcError, ServerFrame, TestApp, TestUser};
use uuid::Uuid;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0200", "0400", "0500", "0600", "1400"];

/// How long to listen for the realtime frames an action should have produced.
pub const SOON: Duration = Duration::from_millis(400);

/// Held by a test while it makes something look overdue and runs the worker
/// that deals with it. The tests of one binary share a database and run in
/// parallel, so without it one test's sweep could do another test's work
/// (and leave that test waiting for something that already happened).
pub static SWEEPS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A running app plus the chat domain behind it, for calling its
/// cross-domain API, its workers and its event handlers directly.
pub struct Ctx {
    pub app: TestApp,
    pub chat: Chat,
}

/// Chat without calls and without the AI assistant, whatever the environment says.
pub fn test_config() -> ChatConfig {
    ChatConfig::default()
}

pub async fn start() -> Ctx {
    start_with(test_config()).await
}

pub async fn start_with(cfg: ChatConfig) -> Ctx {
    let mut domain = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        let social = Social::new(core.clone(), identity.clone(), SocialConfig::default()).expect("social domain");
        let stamp =
            Stamp::new(core.clone(), identity.clone(), StampConfig { seed_official: false, ..Default::default() })
                .expect("stamp domain");
        let chat = Chat::new(core.clone(), identity.clone(), cfg).expect("chat domain");
        let router = chat.register(stamp.register(social.register(identity.register(router))));
        domain = Some(chat);
        router
    })
    .await;
    Ctx { app, chat: domain.expect("register ran") }
}

/// A fresh idempotency key.
pub fn client_id() -> String {
    Uuid::now_v7().to_string()
}

/// The chat events among realtime frames, in order.
pub fn chat_events(frames: Vec<ServerFrame>) -> Vec<Push> {
    frames.iter().filter_map(|frame| frame.event::<ChatEvent>()?.event).collect()
}

/// The messages announced as new among chat events.
pub fn created_messages(events: &[Push]) -> Vec<Message> {
    events
        .iter()
        .filter_map(|event| match event {
            Push::MessageCreated(created) => created.message.as_option().cloned(),
            _ => None,
        })
        .collect()
}

/// The messages announced as changed among chat events, with the reason.
pub fn updated_messages(events: &[Push]) -> Vec<(Message, MessageUpdateReason)> {
    events
        .iter()
        .filter_map(|event| match event {
            Push::MessageUpdated(updated) => Some((updated.message.as_option().cloned()?, updated.reason.as_known()?)),
            _ => None,
        })
        .collect()
}

impl Ctx {
    /// Calls `tokuly.chat.v1.<service>Service/<method>`.
    pub async fn call<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        service: &str,
        method: &str,
        user: &TestUser,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.app.call(&format!("tokuly.chat.v1.{service}Service/{method}"), Some(user), request).await
    }

    /// The reason an RPC is rejected with.
    pub async fn reason<Req: Serialize>(&self, service: &str, method: &str, user: &TestUser, request: &Req) -> String {
        match self.call::<_, serde_json::Value>(service, method, user, request).await {
            Ok(response) => panic!("{service}/{method} should fail, got {response}"),
            Err(error) => error.reason,
        }
    }

    /// The reason an anonymous call is rejected with.
    pub async fn anonymous_reason<Req: Serialize>(&self, service: &str, method: &str, request: &Req) -> String {
        match self
            .app
            .call::<_, serde_json::Value>(&format!("tokuly.chat.v1.{service}Service/{method}"), None, request)
            .await
        {
            Ok(response) => panic!("{service}/{method} should need a signed-in caller, got {response}"),
            Err(error) => error.reason,
        }
    }

    // ───────────────────────────── other domains ─────────────────────────────

    /// Makes two users friends through the social API (crossed requests).
    pub async fn befriend(&self, a: &TestUser, b: &TestUser) {
        for (from, to) in [(a, b), (b, a)] {
            let request =
                SendFriendRequestRequest { target: Some(Target::UserId(to.id.to_string())), ..Default::default() };
            let _: serde_json::Value = self
                .app
                .call("tokuly.social.v1.FriendService/SendFriendRequest", Some(from), &request)
                .await
                .expect("friend request");
        }
    }

    pub async fn block(&self, blocker: &TestUser, blocked: &TestUser) {
        let request = BlockUserRequest { user_id: blocked.id.to_string(), ..Default::default() };
        let _: serde_json::Value =
            self.app.call("tokuly.social.v1.FriendService/BlockUser", Some(blocker), &request).await.expect("block");
    }

    pub async fn unblock(&self, blocker: &TestUser, blocked: &TestUser) {
        let request = UnblockUserRequest { user_id: blocked.id.to_string(), ..Default::default() };
        let _: serde_json::Value = self
            .app
            .call("tokuly.social.v1.FriendService/UnblockUser", Some(blocker), &request)
            .await
            .expect("unblock");
    }

    /// Changes a user's privacy settings through the identity API.
    pub async fn set_privacy(&self, user: &TestUser, change: impl FnOnce(&mut PrivacySettings)) {
        let mut privacy = settings::default_privacy();
        change(&mut privacy);
        let request = UpdateSettingsRequest { privacy: privacy.into(), ..Default::default() };
        let _: serde_json::Value = self
            .app
            .call("tokuly.identity.v1.AccountService/UpdateSettings", Some(user), &request)
            .await
            .expect("update settings");
    }

    /// Lets everyone (not only friends) message and call `user`.
    pub async fn open_to_everyone(&self, user: &TestUser) {
        self.set_privacy(user, |privacy| {
            privacy.direct_messages_from = Audience::AUDIENCE_EVERYONE.into();
            privacy.calls_from = Audience::AUDIENCE_EVERYONE.into();
        })
        .await;
    }

    /// A finished upload of `owner`; returns its asset ID.
    pub async fn upload(&self, owner: &TestUser, purpose: AssetPurpose, kind: AssetKind) -> String {
        tokuly_media::testing::ready_asset(&self.app.core, owner.id, purpose, kind)
            .await
            .expect("asset fixture")
            .id
            .to_string()
    }

    pub async fn photo(&self, owner: &TestUser) -> String {
        self.upload(owner, AssetPurpose::ASSET_PURPOSE_CHAT_IMAGE, AssetKind::ASSET_KIND_IMAGE).await
    }

    /// A personal stamp `owner` may send; returns its ID.
    pub async fn stamp(&self, owner: &TestUser) -> String {
        let asset_id = self.upload(owner, AssetPurpose::ASSET_PURPOSE_STAMP, AssetKind::ASSET_KIND_IMAGE).await;
        let request = CreatePersonalStampRequest { asset_id, name: "にこ".into(), ..Default::default() };
        let response: CreatePersonalStampResponse = self
            .app
            .call("tokuly.stamp.v1.StampLibraryService/CreatePersonalStamp", Some(owner), &request)
            .await
            .expect("personal stamp");
        response.stamp.as_option().expect("stamp").stamp_ref.id.clone()
    }

    // ───────────────────────────── chat shortcuts ─────────────────────────────

    /// Two fresh users who are friends.
    pub async fn friends(&self, a: &str, b: &str) -> (TestUser, TestUser) {
        let (a, b) = (self.app.user(a).await, self.app.user(b).await);
        self.befriend(&a, &b).await;
        (a, b)
    }

    /// Opens (or returns) the direct conversation of `me` with `other`.
    pub async fn direct(&self, me: &TestUser, other: &TestUser) -> Conversation {
        let response: OpenDirectResponse = self
            .call(
                "Conversation",
                "OpenDirect",
                me,
                &OpenDirectRequest { user_id: other.id.to_string(), ..Default::default() },
            )
            .await
            .expect("open direct");
        response.conversation.as_option().expect("conversation").clone()
    }

    /// Creates a group owned by `owner` with `members` (who must be the owner's friends).
    pub async fn group(&self, owner: &TestUser, name: &str, members: &[&TestUser]) -> Conversation {
        let request = CreateGroupRequest {
            name: name.to_owned(),
            member_ids: members.iter().map(|member| member.id.to_string()).collect(),
            ..Default::default()
        };
        let response: CreateGroupResponse =
            self.call("Conversation", "CreateGroup", owner, &request).await.expect("create group");
        response.conversation.as_option().expect("conversation").clone()
    }

    /// A group of an owner and `others` freshly created friends of theirs.
    pub async fn group_of(&self, name: &str, others: usize) -> (Conversation, TestUser, Vec<TestUser>) {
        let owner = self.app.user("owner").await;
        let mut members = Vec::with_capacity(others);
        for index in 0..others {
            let member = self.app.user(&format!("member{index}")).await;
            self.befriend(&owner, &member).await;
            members.push(member);
        }
        let conversation = self.group(&owner, name, &members.iter().collect::<Vec<_>>()).await;
        (conversation, owner, members)
    }

    pub async fn conversation(&self, viewer: &TestUser, conversation_id: &str) -> GetConversationResponse {
        self.call(
            "Conversation",
            "GetConversation",
            viewer,
            &GetConversationRequest { conversation_id: conversation_id.to_owned(), ..Default::default() },
        )
        .await
        .expect("get conversation")
    }

    pub async fn list(&self, viewer: &TestUser, filter: ConversationFilter) -> ListConversationsResponse {
        self.call(
            "Conversation",
            "ListConversations",
            viewer,
            &ListConversationsRequest { filter: filter.into(), ..Default::default() },
        )
        .await
        .expect("list conversations")
    }

    pub async fn send(&self, sender: &TestUser, request: &SendMessageRequest) -> Result<SendMessageResponse, RpcError> {
        self.call("Message", "SendMessage", sender, request).await
    }

    /// Sends a text message and returns it.
    pub async fn text(&self, sender: &TestUser, conversation_id: &str, text: &str) -> Message {
        let request = SendMessageRequest {
            conversation_id: conversation_id.to_owned(),
            client_id: client_id(),
            text: text.to_owned(),
            ..Default::default()
        };
        self.send(sender, &request).await.expect("send text").message.as_option().expect("message").clone()
    }

    pub async fn message(&self, viewer: &TestUser, message_id: &str) -> Message {
        let response: GetMessageResponse = self
            .call(
                "Message",
                "GetMessage",
                viewer,
                &GetMessageRequest { message_id: message_id.to_owned(), ..Default::default() },
            )
            .await
            .expect("get message");
        response.message.as_option().expect("message").clone()
    }

    pub async fn messages(&self, viewer: &TestUser, request: &ListMessagesRequest) -> ListMessagesResponse {
        self.call("Message", "ListMessages", viewer, request).await.expect("list messages")
    }

    /// The newest messages of a conversation, oldest first.
    pub async fn latest(&self, viewer: &TestUser, conversation_id: &str) -> Vec<Message> {
        self.messages(
            viewer,
            &ListMessagesRequest { conversation_id: conversation_id.to_owned(), ..Default::default() },
        )
        .await
        .messages
    }

    pub async fn mark_read(&self, reader: &TestUser, conversation_id: &str) -> MarkReadResponse {
        self.call(
            "Conversation",
            "MarkRead",
            reader,
            &MarkReadRequest { conversation_id: conversation_id.to_owned(), ..Default::default() },
        )
        .await
        .expect("mark read")
    }

    pub async fn unread(&self, viewer: &TestUser) -> UnreadCounts {
        let response: GetUnreadCountsResponse = self
            .call("Conversation", "GetUnreadCounts", viewer, &GetUnreadCountsRequest::default())
            .await
            .expect("unread counts");
        response.unread.as_option().expect("unread").clone()
    }

    // ───────────────────────────── observing ─────────────────────────────

    pub async fn listen(&self, user: &TestUser) -> FrameListener {
        self.app.listen_user(user.id).await
    }

    /// The chat events `listener` received since it was created.
    pub async fn events(&self, listener: &mut FrameListener) -> Vec<Push> {
        chat_events(listener.drain(SOON).await)
    }

    /// Every chat notification requested for `user` so far, oldest first.
    /// (Setting up friendships produces notifications of the social domain,
    /// which are not what these tests are about.)
    pub async fn notifications(&self, user: &TestUser) -> Vec<NotificationRequest> {
        self.app
            .events("notify.requested")
            .await
            .into_iter()
            .filter_map(|event| event.data::<NotificationRequest>().ok())
            .filter(|notification| notification.user_id == user.id && notification.kind.starts_with("chat."))
            .collect()
    }

    /// The kinds of the notifications requested for `user` so far.
    pub async fn notification_kinds(&self, user: &TestUser) -> Vec<String> {
        self.notifications(user).await.into_iter().map(|notification| notification.kind).collect()
    }
}
