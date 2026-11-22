//! Shared fixtures for the community integration tests.
//!
//! Every test starts its own app ([`start`]) against the test binary's
//! database and creates its own users and communities (handles get a
//! random suffix). The social and stamp domains run for real: blocks,
//! friendships and stamps are part of what is tested. Events of other
//! domains are handed to the consumer directly ([`Ctx::deliver`]); no relay
//! runs in tests.
#![allow(dead_code)]

use serde::{Serialize, de::DeserializeOwned};
use tokuly_community::{Community, CommunityConfig};
use tokuly_core::{events::Event, prelude::*};
use tokuly_proto::{
    common::{PageRequest, SubjectType},
    community::{community_event::Event as Pushed, *},
    media::AssetPurpose,
};
use tokuly_social::{Social, SocialConfig};
use tokuly_stamp::{Stamp, StampConfig};
use tokuly_testkit::{FrameListener, RpcError, TestApp, TestUser};

pub use tokuly_core::time::{now, ts};
pub use tokuly_proto::community as pb;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0200", "0400", "0500", "1400", "2300"];

/// A running app with the community domain and what it leans on.
pub struct Ctx {
    pub app: TestApp,
    pub community: Community,
}

/// Settings for tests: the posting limits are out of the way, and an
/// anonymous channel works in the small communities tests build.
pub fn test_config() -> CommunityConfig {
    CommunityConfig {
        posts_per_minute: 10_000,
        posts_per_hour: 100_000,
        max_owned: 50,
        anonymous_min_members: 2,
        ..Default::default()
    }
}

pub async fn start() -> Ctx {
    start_with(test_config()).await
}

pub async fn start_with(cfg: CommunityConfig) -> Ctx {
    let mut domain = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        let social = Social::new(core.clone(), identity.clone(), SocialConfig::default()).expect("social domain");
        let stamp =
            Stamp::new(core.clone(), identity.clone(), StampConfig { seed_official: false, ..Default::default() })
                .expect("stamp domain");
        let community = Community::new(core.clone(), identity.clone(), cfg).expect("community domain");
        let router = community.register(stamp.register(social.register(identity.register(router))));
        domain = Some(community);
        router
    })
    .await;
    Ctx { app, community: domain.expect("register ran") }
}

/// A handle no other test uses.
pub fn unique_handle(prefix: &str) -> String {
    format!("{prefix}-{}", tokuly_core::ids::random_base62(8).to_ascii_lowercase())
}

pub fn page<P: buffa::ProtoBox<PageRequest>>(limit: i32, cursor: &str) -> buffa::MessageField<PageRequest, P> {
    buffa::MessageField::some(PageRequest { limit, cursor: cursor.to_owned(), ..Default::default() })
}

pub fn thread_ref<P: buffa::ProtoBox<PostRef>>(thread_id: &str) -> buffa::MessageField<PostRef, P> {
    buffa::MessageField::some(PostRef {
        post: Some(post_ref::Post::ThreadId(thread_id.to_owned())),
        ..Default::default()
    })
}

pub fn reply_ref<P: buffa::ProtoBox<PostRef>>(reply_id: &str) -> buffa::MessageField<PostRef, P> {
    buffa::MessageField::some(PostRef {
        post: Some(post_ref::Post::ReplyId(reply_id.to_owned())),
        ..Default::default()
    })
}

impl Ctx {
    /// Calls `tokuly.community.v1.<service>/<method>`.
    pub async fn call<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        service: &str,
        method: &str,
        user: Option<&TestUser>,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.app.call(&format!("tokuly.community.v1.{service}/{method}"), user, request).await
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

    // ───────────────────────────── communities ─────────────────────────────

    /// What a creation request looks like by default: a game community.
    pub fn new_community(&self, visibility: CommunityVisibility) -> CreateCommunityRequest {
        CreateCommunityRequest {
            name: "Apex部".into(),
            handle: unique_handle("apex"),
            description: "ランクを回そう".into(),
            category: CommunityCategory::COMMUNITY_CATEGORY_GAME.into(),
            visibility: visibility.into(),
            language: "ja".into(),
            tags: vec!["apex".into(), "fps".into()],
            ..Default::default()
        }
    }

    pub async fn try_create(
        &self,
        owner: &TestUser,
        request: &CreateCommunityRequest,
    ) -> Result<pb::Community, RpcError> {
        let response: CreateCommunityResponse =
            self.call("CommunityService", "CreateCommunity", Some(owner), request).await?;
        Ok(response.community.as_option().expect("a community").clone())
    }

    /// Creates a community of `visibility` owned by `owner`.
    pub async fn create(&self, owner: &TestUser, visibility: CommunityVisibility) -> pb::Community {
        self.try_create(owner, &self.new_community(visibility)).await.unwrap()
    }

    pub async fn public(&self, owner: &TestUser) -> pb::Community {
        self.create(owner, CommunityVisibility::COMMUNITY_VISIBILITY_PUBLIC).await
    }

    pub async fn restricted(&self, owner: &TestUser) -> pb::Community {
        self.create(owner, CommunityVisibility::COMMUNITY_VISIBILITY_RESTRICTED).await
    }

    pub async fn hidden(&self, owner: &TestUser) -> pb::Community {
        self.create(owner, CommunityVisibility::COMMUNITY_VISIBILITY_HIDDEN).await
    }

    pub async fn get(&self, viewer: Option<&TestUser>, community_id: &str) -> Result<pb::Community, RpcError> {
        let request = GetCommunityRequest { community_id: community_id.to_owned(), ..Default::default() };
        let response: GetCommunityResponse = self.call("CommunityService", "GetCommunity", viewer, &request).await?;
        Ok(response.community.as_option().expect("a community").clone())
    }

    /// The `viewer` block of a community as `user` sees it.
    pub async fn viewer(&self, user: &TestUser, community_id: &str) -> CommunityViewer {
        self.get(Some(user), community_id).await.unwrap().viewer.as_option().expect("a viewer block").clone()
    }

    pub async fn try_join(
        &self,
        user: &TestUser,
        community_id: &str,
        invite_code: &str,
    ) -> Result<JoinCommunityResponse, RpcError> {
        let request = JoinCommunityRequest {
            community_id: community_id.to_owned(),
            invite_code: invite_code.to_owned(),
            ..Default::default()
        };
        self.call("CommunityService", "JoinCommunity", Some(user), &request).await
    }

    /// Joins a public community.
    pub async fn join(&self, user: &TestUser, community: &pb::Community) {
        let response = self.try_join(user, &community.id, "").await.unwrap();
        assert_eq!(response.status, MembershipStatus::MEMBERSHIP_STATUS_MEMBER, "joining should make a member");
    }

    /// An invite created by `staff`.
    pub async fn invite(&self, staff: &TestUser, community: &pb::Community) -> Invite {
        let request = CreateInviteRequest { community_id: community.id.clone(), ..Default::default() };
        let response: CreateInviteResponse =
            self.call("CommunityService", "CreateInvite", Some(staff), &request).await.unwrap();
        response.invite.as_option().expect("an invite").clone()
    }

    /// A new user who is a member of `community` (any visibility: they come
    /// in through an invite of the owner).
    pub async fn member(&self, owner: &TestUser, community: &pb::Community, name: &str) -> TestUser {
        let user = self.app.user(name).await;
        self.add(owner, community, &user).await;
        user
    }

    /// Makes an existing user a member through an invite of the owner.
    pub async fn add(&self, owner: &TestUser, community: &pb::Community, user: &TestUser) {
        let invite = self.invite(owner, community).await;
        let response = self.try_join(user, "", &invite.code).await.unwrap();
        assert_eq!(response.status, MembershipStatus::MEMBERSHIP_STATUS_MEMBER);
    }

    pub async fn set_role(&self, actor: &TestUser, community: &pb::Community, user: &TestUser, role: MemberRole) {
        let request = SetMemberRoleRequest {
            community_id: community.id.clone(),
            user_id: user.id.to_string(),
            role: role.into(),
            ..Default::default()
        };
        let _: SetMemberRoleResponse =
            self.call("CommunityService", "SetMemberRole", Some(actor), &request).await.unwrap();
    }

    /// A new member holding `role`, appointed by the owner.
    pub async fn staff(&self, owner: &TestUser, community: &pb::Community, name: &str, role: MemberRole) -> TestUser {
        let user = self.member(owner, community, name).await;
        self.set_role(owner, community, &user, role).await;
        user
    }

    pub async fn moderator(&self, owner: &TestUser, community: &pb::Community) -> TestUser {
        self.staff(owner, community, "Moderator", MemberRole::MEMBER_ROLE_MODERATOR).await
    }

    // ───────────────────────────── boards ─────────────────────────────

    pub async fn channels(&self, viewer: Option<&TestUser>, community: &pb::Community) -> Vec<Channel> {
        let request = ListChannelsRequest { community_id: community.id.clone(), ..Default::default() };
        let response: ListChannelsResponse =
            self.call("CommunityBoardService", "ListChannels", viewer, &request).await.unwrap();
        response.channels
    }

    /// The channel every community starts with.
    pub async fn general(&self, viewer: &TestUser, community: &pb::Community) -> Channel {
        self.channels(Some(viewer), community).await.into_iter().next().expect("the first channel")
    }

    /// A new channel created by `admin`.
    pub async fn channel(&self, admin: &TestUser, community: &pb::Community, name: &str, anonymous: bool) -> Channel {
        let request = CreateChannelRequest {
            community_id: community.id.clone(),
            name: name.to_owned(),
            anonymous,
            ..Default::default()
        };
        let response: CreateChannelResponse =
            self.call("CommunityBoardService", "CreateChannel", Some(admin), &request).await.unwrap();
        response.channel.as_option().expect("a channel").clone()
    }

    pub async fn try_thread(&self, author: &TestUser, request: &CreateThreadRequest) -> Result<Thread, RpcError> {
        let response: CreateThreadResponse =
            self.call("CommunityBoardService", "CreateThread", Some(author), request).await?;
        Ok(response.thread.as_option().expect("a thread").clone())
    }

    /// Starts a text thread.
    pub async fn thread(&self, author: &TestUser, channel: &Channel, title: &str, body: &str) -> Thread {
        let request = CreateThreadRequest {
            channel_id: channel.id.clone(),
            title: title.to_owned(),
            body: body.to_owned(),
            ..Default::default()
        };
        self.try_thread(author, &request).await.unwrap()
    }

    pub async fn get_thread(&self, viewer: Option<&TestUser>, thread_id: &str) -> Result<Thread, RpcError> {
        let request = GetThreadRequest { thread_id: thread_id.to_owned(), ..Default::default() };
        let response: GetThreadResponse = self.call("CommunityBoardService", "GetThread", viewer, &request).await?;
        Ok(response.thread.as_option().expect("a thread").clone())
    }

    pub async fn list_threads(&self, viewer: Option<&TestUser>, request: &ListThreadsRequest) -> ListThreadsResponse {
        self.call("CommunityBoardService", "ListThreads", viewer, request).await.unwrap()
    }

    /// The titles of the community's threads in "new" order, pinned ones aside.
    pub async fn thread_titles(&self, viewer: Option<&TestUser>, community: &pb::Community) -> Vec<String> {
        let request = ListThreadsRequest {
            community_id: community.id.clone(),
            sort: ThreadSort::THREAD_SORT_NEW.into(),
            ..Default::default()
        };
        self.list_threads(viewer, &request).await.threads.into_iter().map(|thread| thread.title).collect()
    }

    pub async fn try_reply(&self, author: &TestUser, request: &CreateReplyRequest) -> Result<Reply, RpcError> {
        let response: CreateReplyResponse =
            self.call("CommunityBoardService", "CreateReply", Some(author), request).await?;
        Ok(response.reply.as_option().expect("a reply").clone())
    }

    /// Replies to a thread with text.
    pub async fn reply(&self, author: &TestUser, thread: &Thread, body: &str) -> Reply {
        let request = CreateReplyRequest { thread_id: thread.id.clone(), body: body.to_owned(), ..Default::default() };
        self.try_reply(author, &request).await.unwrap()
    }

    pub async fn replies(&self, viewer: Option<&TestUser>, thread_id: &str) -> Vec<Reply> {
        let request = ListRepliesRequest { thread_id: thread_id.to_owned(), ..Default::default() };
        let response: ListRepliesResponse =
            self.call("CommunityBoardService", "ListReplies", viewer, &request).await.unwrap();
        response.replies
    }

    pub async fn get_reply(&self, viewer: Option<&TestUser>, reply_id: &str) -> Result<Reply, RpcError> {
        let request = GetReplyRequest { reply_id: reply_id.to_owned(), ..Default::default() };
        let response: GetReplyResponse = self.call("CommunityBoardService", "GetReply", viewer, &request).await?;
        Ok(response.reply.as_option().expect("a reply").clone())
    }

    /// Sets the community's muted words.
    pub async fn mute_words(&self, staff: &TestUser, community: &pb::Community, words: &[&str]) {
        let request = UpdateSafetySettingsRequest {
            community_id: community.id.clone(),
            muted_words: StringList {
                values: words.iter().map(|word| word.to_string()).collect(),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        };
        let _: UpdateSafetySettingsResponse =
            self.call("CommunityModerationService", "UpdateSafetySettings", Some(staff), &request).await.unwrap();
    }

    pub async fn review_queue(&self, staff: &TestUser, community: &pb::Community) -> Vec<ReviewItem> {
        let request = ListReviewQueueRequest { community_id: community.id.clone(), ..Default::default() };
        let response: ListReviewQueueResponse =
            self.call("CommunityModerationService", "ListReviewQueue", Some(staff), &request).await.unwrap();
        response.items
    }

    pub async fn moderation_log(&self, staff: &TestUser, community: &pb::Community) -> Vec<ModerationLogEntry> {
        let request = ListModerationLogRequest { community_id: community.id.clone(), ..Default::default() };
        let response: ListModerationLogResponse =
            self.call("CommunityModerationService", "ListModerationLog", Some(staff), &request).await.unwrap();
        response.entries
    }

    // ───────────────────────────── events ─────────────────────────────

    /// What an event request looks like by default: in person, tomorrow.
    pub fn new_event(&self, community: &pb::Community) -> CreateEventRequest {
        CreateEventRequest {
            community_id: community.id.clone(),
            title: "オフ会".into(),
            description: "みんなで集まろう".into(),
            starts_at: ts(now() + chrono::Duration::days(1)),
            place: EventPlace { name: "駅前のカフェ".into(), ..Default::default() }.into(),
            ..Default::default()
        }
    }

    pub async fn try_event(
        &self,
        organizer: &TestUser,
        request: &CreateEventRequest,
    ) -> Result<CalendarEvent, RpcError> {
        let response: CreateEventResponse =
            self.call("CommunityEventService", "CreateEvent", Some(organizer), request).await?;
        Ok(response.event.as_option().expect("an event").clone())
    }

    pub async fn event(&self, organizer: &TestUser, community: &pb::Community) -> CalendarEvent {
        self.try_event(organizer, &self.new_event(community)).await.unwrap()
    }

    pub async fn try_rsvp(
        &self,
        user: &TestUser,
        event_id: &str,
        answer: RsvpAnswer,
    ) -> Result<CalendarEvent, RpcError> {
        let request = RsvpRequest { event_id: event_id.to_owned(), answer: answer.into(), ..Default::default() };
        let response: RsvpResponse = self.call("CommunityEventService", "Rsvp", Some(user), &request).await?;
        Ok(response.event.as_option().expect("an event").clone())
    }

    pub async fn rsvp(&self, user: &TestUser, event: &CalendarEvent, answer: RsvpAnswer) -> CalendarEvent {
        self.try_rsvp(user, &event.id, answer).await.unwrap()
    }

    pub async fn get_event(&self, viewer: Option<&TestUser>, event_id: &str) -> Result<CalendarEvent, RpcError> {
        let request = GetEventRequest { event_id: event_id.to_owned(), ..Default::default() };
        let response: GetEventResponse = self.call("CommunityEventService", "GetEvent", viewer, &request).await?;
        Ok(response.event.as_option().expect("an event").clone())
    }

    /// Moves an event in time, as if it had been created long ago: the
    /// clock cannot be moved, the event can.
    pub async fn move_event(&self, event_id: &str, starts_in: chrono::Duration, length: chrono::Duration) {
        let starts_at = now() + starts_in;
        sqlx::query(
            "UPDATE community_events SET starts_at = $2, ends_at = $3, created_at = least(created_at, $2 - interval '3 days')
             WHERE id = $1",
        )
        .bind(parse_id(event_id).unwrap())
        .bind(starts_at)
        .bind(starts_at + length)
        .execute(&self.app.core.db)
        .await
        .unwrap();
    }

    // ───────────────────────────── surroundings ─────────────────────────────

    /// `blocker` blocks `blocked` through the social domain.
    pub async fn block(&self, blocker: &TestUser, blocked: &TestUser) {
        let request = tokuly_proto::social::BlockUserRequest { user_id: blocked.id.to_string(), ..Default::default() };
        let _: tokuly_proto::social::BlockUserResponse =
            self.app.call("tokuly.social.v1.FriendService/BlockUser", Some(blocker), &request).await.unwrap();
    }

    /// Makes two users friends: crossed requests accept each other.
    pub async fn befriend(&self, a: &TestUser, b: &TestUser) {
        use tokuly_proto::social::{SendFriendRequestRequest, send_friend_request_request::Target};
        for (from, to) in [(a, b), (b, a)] {
            let request =
                SendFriendRequestRequest { target: Some(Target::UserId(to.id.to_string())), ..Default::default() };
            let _: serde_json::Value =
                self.app.call("tokuly.social.v1.FriendService/SendFriendRequest", Some(from), &request).await.unwrap();
        }
    }

    /// Hands the community domain an event as the consumer would.
    pub async fn deliver(&self, topic: &str, actor_id: Option<Uuid>, data: serde_json::Value) {
        let event =
            Event { id: Uuid::now_v7(), topic: topic.to_owned(), actor_id, occurred_at: chrono::Utc::now(), data };
        self.community.handle_event(event).await.unwrap_or_else(|error| panic!("{topic} handler failed: {error}"));
    }

    /// A report as the engagement domain announces it.
    pub async fn report(&self, reporter: &TestUser, subject_type: SubjectType, subject_id: &str) {
        self.deliver(
            "engagement.report.created",
            Some(reporter.id),
            serde_json::json!({
                "report_id": Uuid::now_v7(),
                "reporter_id": reporter.id,
                "subject_type": subject_type as i32,
                "subject_id": subject_id,
                "subject_owner_id": null,
                "category": "HARASSMENT",
                "text": "",
                "subject_title": "",
                "subject_path": "",
            }),
        )
        .await;
    }

    /// A processed image upload of `user`, by asset ID.
    pub async fn image(&self, user: &TestUser, purpose: AssetPurpose) -> String {
        tokuly_media::testing::ready_image(&self.app.core, user.id, purpose).await.unwrap().id.to_string()
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

    /// Whether `holder` holds the upload.
    pub async fn is_held(&self, asset_id: &str, holder: &str) -> bool {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM media_asset_holders WHERE asset_id = $1 AND holder = $2)")
            .bind(parse_id(asset_id).unwrap())
            .bind(holder)
            .fetch_one(&self.app.core.db)
            .await
            .unwrap()
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

    /// The search documents written for `id` so far, oldest first (`doc`
    /// is null where the document was removed).
    pub async fn search_changes(&self, kind: &str, id: &str) -> Vec<serde_json::Value> {
        self.app
            .events("search.changed")
            .await
            .into_iter()
            .map(|event| event.data)
            .filter(|data| data["kind"] == kind && data["id"] == id)
            .collect()
    }

    /// Events of this domain with `topic` about `community_id`.
    pub async fn community_events(&self, topic: &str, community_id: &str) -> Vec<Event> {
        self.app.events(topic).await.into_iter().filter(|event| event.data["community_id"] == community_id).collect()
    }
}

/// The community events among the frames that arrive within a moment.
pub async fn pushes(listener: &mut FrameListener) -> Vec<Pushed> {
    let events = listener.events::<CommunityEvent>(std::time::Duration::from_millis(400)).await;
    events.into_iter().filter_map(|event| event.event).collect()
}

/// Whether a serialized response mentions `needle` anywhere: the blunt
/// instrument behind "no response names the author".
pub fn mentions<T: Serialize>(response: &T, needle: &str) -> bool {
    serde_json::to_string(response).expect("responses serialize").contains(needle)
}
