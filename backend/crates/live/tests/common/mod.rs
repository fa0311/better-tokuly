//! Shared fixtures for the live integration tests.
//!
//! Every test starts its own app ([`start`]) against the test binary's
//! database and creates its own users, channels and streams. Friends,
//! blocks, channels, subscriptions and stamps are set up through the owning
//! domains' own RPCs, the way clients do it. Lists that are global ("live
//! now", upcoming) are asserted on through things only the test itself made.
//!
//! No media server runs here: streams go on air with `Live::go_live` or by
//! handing the reconciler the paths a media server would report
//! (`Live::apply_media_state`). `tests/mediamtx.rs` does it with the real
//! thing.
#![allow(dead_code)] // each test binary uses its own subset

use std::time::Duration;

use serde::{Serialize, de::DeserializeOwned};
use tokuly_core::events::Event;
use tokuly_live::{Live, LiveConfig, media_server::MediaPath, streams::StreamRow};
use tokuly_media::{Media, MediaConfig};
use tokuly_proto::{
    common::PageRequest,
    live::*,
    media::{AssetKind, AssetPurpose},
    social::{
        AddCircleMembersRequest, AddCircleMembersResponse, BlockUserRequest, BlockUserResponse, CircleKind,
        CreateCircleRequest, CreateCircleResponse, FollowRequest, FollowResponse, FollowSettings, FriendSource,
        ListCirclesRequest, ListCirclesResponse, RemoveFriendRequest, RemoveFriendResponse,
        RespondFriendRequestRequest, RespondFriendRequestResponse, SendFriendRequestRequest, SendFriendRequestResponse,
        UpdateFollowSettingsRequest, UpdateFollowSettingsResponse, follow_request, send_friend_request_request::Target,
    },
    stamp::{CreatePersonalStampRequest, CreatePersonalStampResponse},
    video::{
        Channel, CreateChannelRequest, CreateChannelResponse, NotificationLevel, SubscribeRequest, SubscribeResponse,
    },
};
use tokuly_social::{Social, SocialConfig};
use tokuly_stamp::{Stamp, StampConfig};
use tokuly_testkit::{FrameListener, RpcError, TestApp, TestUser};
use tokuly_video::{Video, VideoConfig};
use tokuly_wallet::{Wallet, WalletConfig};
use uuid::Uuid;

/// A `LiveEvent` as pushed over the realtime gateway.
pub use tokuly_proto::live::live_event::Event as Push;

/// core, identity, social, media, stamp, video, live, wallet.
pub const MIGRATIONS: &[&str] = &["0001", "0100", "0200", "0400", "0500", "0800", "0900", "1400"];

pub const STREAMS: &str = "tokuly.live.v1.LiveStreamService";
pub const CHAT: &str = "tokuly.live.v1.LiveChatService";
pub const POLLS: &str = "tokuly.live.v1.LivePollService";
pub const GIFTS: &str = "tokuly.live.v1.LiveGiftService";
pub const CLIPS: &str = "tokuly.live.v1.ClipService";
pub const OVERLAYS: &str = "tokuly.live.v1.OverlayService";
pub const ANALYTICS: &str = "tokuly.live.v1.LiveAnalyticsService";

/// How long to wait for a realtime frame that should arrive (generous: the
/// machine may be busy compiling something else).
pub const SOON: Duration = Duration::from_secs(5);
/// How long to listen when asserting that nothing arrives.
pub const QUIET: Duration = Duration::from_millis(300);

/// A running app plus the domains behind it, for calling the public Rust
/// API, running workers and delivering events by hand.
pub struct Ctx {
    pub app: TestApp,
    pub live: Live,
    pub media: Media,
    pub wallet: Wallet,
}

/// The defaults: no media server configured.
pub fn test_config() -> LiveConfig {
    LiveConfig::default()
}

pub async fn start() -> Ctx {
    start_with(test_config()).await
}

pub async fn start_with(cfg: LiveConfig) -> Ctx {
    // Show the cause of internal errors (logged, never sent to clients) in
    // the output of a failing test.
    let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::WARN).with_test_writer().try_init();
    let mut domains = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        let media = Media::new(core.clone(), MediaConfig::default()).expect("media domain");
        let social = Social::new(core.clone(), identity.clone(), SocialConfig::default()).expect("social domain");
        let stamp =
            Stamp::new(core.clone(), identity.clone(), StampConfig { seed_official: false, ..Default::default() })
                .expect("stamp domain");
        let wallet = Wallet::new(core.clone(), identity.clone(), WalletConfig::default()).expect("wallet domain");
        let video = Video::new(core.clone(), identity.clone(), VideoConfig::default()).expect("video domain");
        let live = Live::new(core.clone(), identity.clone(), cfg, wallet.clone()).expect("live domain");
        let router = live.register(
            video.register(wallet.register(stamp.register(social.register(media.register(identity.register(router)))))),
        );
        domains = Some((live, media, wallet));
        router
    })
    .await;
    let (live, media, wallet) = domains.expect("register ran");
    Ctx { app, live, media, wallet }
}

/// A lowercase string no other test uses, e.g. for handles and tags.
pub fn unique(prefix: &str) -> String {
    format!("{prefix}{}", tokuly_core::ids::random_base62(10).to_ascii_lowercase())
}

pub fn page<P: buffa::ProtoBox<PageRequest>>(limit: i32, cursor: &str) -> buffa::MessageField<PageRequest, P> {
    buffa::MessageField::some(PageRequest { limit, cursor: cursor.to_owned(), ..Default::default() })
}

pub fn id(value: &str) -> Uuid {
    value.parse().expect("an ID")
}

/// The card inside a `Stream`.
pub fn card(stream: &Stream) -> &StreamSummary {
    stream.summary.as_option().expect("a stream has a summary")
}

/// The UUID of a stream.
pub fn sid(stream: &Stream) -> Uuid {
    id(&card(stream).id)
}

pub fn titles(streams: &[StreamSummary]) -> Vec<&str> {
    streams.iter().map(|stream| stream.title.as_str()).collect()
}

/// The `LiveEvent`s among realtime frames, in order.
pub fn pushes(frames: Vec<tokuly_proto::realtime::ServerFrame>) -> Vec<Push> {
    frames.into_iter().filter_map(|frame| frame.event::<LiveEvent>().and_then(|event| event.event)).collect()
}

/// Waits for the next `LiveEvent` on a listener.
pub async fn next_push(listener: &mut FrameListener) -> Push {
    loop {
        let event = listener.next_event::<LiveEvent>(SOON).await.expect("a realtime event should arrive");
        if let Some(push) = event.event {
            return push;
        }
    }
}

/// Waits for the next `LiveEvent` that `pick` accepts, skipping the others.
pub async fn wait_for<T>(listener: &mut FrameListener, mut pick: impl FnMut(Push) -> Option<T>) -> T {
    loop {
        if let Some(found) = pick(next_push(listener).await) {
            return found;
        }
    }
}

/// A path as the media server would report it for a stream with an encoder.
pub fn online_path(short_id: &str, readers: i64) -> MediaPath {
    MediaPath {
        name: format!("live/{short_id}"),
        online: true,
        source_type: "rtmpConn".to_owned(),
        source_id: Uuid::now_v7().to_string(),
        readers,
        reader_ids: (0..readers).map(|i| ("hlsSession".to_owned(), format!("reader-{i}"))).collect(),
    }
}

impl Ctx {
    /// Calls `<service>/<method>`.
    pub async fn call<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        service: &str,
        method: &str,
        user: Option<&TestUser>,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.app.call(&format!("{service}/{method}"), user, request).await
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

    /// `a` ends the friendship with `b` through the social API, and the live
    /// domain hears of it as the relay would tell it.
    pub async fn unfriend(&self, a: &TestUser, b: &TestUser) {
        let request = RemoveFriendRequest { user_id: b.id.to_string(), ..Default::default() };
        let _: RemoveFriendResponse =
            self.app.call("tokuly.social.v1.FriendService/RemoveFriend", Some(a), &request).await.expect("unfriend");
        assert_eq!(self.deliver("social.friend.removed", a.id).await, 1, "the end of a friendship is announced");
    }

    /// `follower` follows `followee` (an account that is not locked) through the social API.
    pub async fn follow(&self, follower: &TestUser, followee: &TestUser) {
        let request = FollowRequest {
            target: Some(follow_request::Target::UserId(followee.id.to_string())),
            ..Default::default()
        };
        let _: FollowResponse =
            self.app.call("tokuly.social.v1.FollowService/Follow", Some(follower), &request).await.expect("follow");
    }

    /// Locks or unlocks `user`'s account through the social API. The
    /// `social.follow.settings_changed` event is not delivered here: call
    /// [`Self::deliver_lock_change`] for that.
    pub async fn set_locked(&self, user: &TestUser, locked: bool) {
        let request = UpdateFollowSettingsRequest {
            settings: FollowSettings { locked, ..Default::default() }.into(),
            ..Default::default()
        };
        let _: UpdateFollowSettingsResponse = self
            .app
            .call("tokuly.social.v1.FollowService/UpdateFollowSettings", Some(user), &request)
            .await
            .expect("update follow settings");
    }

    /// Delivers the latest `social.follow.settings_changed` of `user` to the live domain.
    pub async fn deliver_lock_change(&self, user: &TestUser) {
        let event =
            self.app.events_by("social.follow.settings_changed", user.id).await.pop().expect("a settings change");
        self.live.handle_event(event).await.expect("handled");
    }

    /// Creates a custom circle of `owner` with `members` and returns its ID.
    pub async fn circle(&self, owner: &TestUser, name: &str, members: &[&TestUser]) -> String {
        let created: CreateCircleResponse = self
            .app
            .call(
                "tokuly.social.v1.CircleService/CreateCircle",
                Some(owner),
                &CreateCircleRequest { name: name.to_owned(), ..Default::default() },
            )
            .await
            .expect("create circle");
        let circle_id = created.circle.id.clone();
        self.add_to_circle(owner, &circle_id, members).await;
        circle_id
    }

    /// The ID of `owner`'s close-friends circle.
    pub async fn close_friends_circle(&self, owner: &TestUser) -> String {
        let circles: ListCirclesResponse = self
            .app
            .call("tokuly.social.v1.CircleService/ListCircles", Some(owner), &ListCirclesRequest::default())
            .await
            .expect("list circles");
        circles
            .circles
            .iter()
            .find(|circle| circle.kind == buffa::EnumValue::from(CircleKind::CIRCLE_KIND_CLOSE_FRIENDS))
            .map(|circle| circle.id.clone())
            .expect("a close-friends circle")
    }

    pub async fn add_to_circle(&self, owner: &TestUser, circle_id: &str, members: &[&TestUser]) {
        if members.is_empty() {
            return;
        }
        let request = AddCircleMembersRequest {
            circle_id: circle_id.to_owned(),
            user_ids: members.iter().map(|member| member.id.to_string()).collect(),
            ..Default::default()
        };
        let _: AddCircleMembersResponse = self
            .app
            .call("tokuly.social.v1.CircleService/AddCircleMembers", Some(owner), &request)
            .await
            .expect("add circle members");
    }

    /// `blocker` blocks `blocked` through the social API.
    pub async fn block(&self, blocker: &TestUser, blocked: &TestUser) {
        let request = BlockUserRequest { user_id: blocked.id.to_string(), ..Default::default() };
        let _: BlockUserResponse =
            self.app.call("tokuly.social.v1.FriendService/BlockUser", Some(blocker), &request).await.expect("block");
    }

    /// Delivers the events `actor` caused so far (topic prefix `prefix`) to
    /// the live domain's consumer, as the relay would. Handlers are
    /// idempotent, so delivering an event again is harmless.
    pub async fn deliver(&self, prefix: &str, actor: Uuid) -> usize {
        let events = self.app.events_by(prefix, actor).await;
        let count = events.len();
        for event in events {
            self.live.handle_event(event).await.expect("event handler");
        }
        count
    }

    /// Delivers a hand-made event to the live domain's consumer.
    pub async fn deliver_event(&self, topic: &str, actor: Option<Uuid>, data: serde_json::Value) {
        let event = Event {
            id: Uuid::now_v7(),
            topic: topic.to_owned(),
            actor_id: actor,
            occurred_at: chrono::Utc::now(),
            data,
        };
        self.live.handle_event(event).await.expect("event handler");
    }

    /// How many times `subject.deleted` is in the outbox for this subject.
    pub async fn announced(&self, subject_type: tokuly_proto::common::SubjectType, id: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM outbox WHERE topic = $1 AND data = jsonb_build_object('subject_type', $2::integer, 'subject_id', $3::text)",
        )
        .bind(tokuly_core::outbound::SUBJECT_DELETED_TOPIC)
        .bind(subject_type as i32)
        .bind(id)
        .fetch_one(&self.app.core.db)
        .await
        .expect("count announcements")
    }

    /// Notifications of `kind` requested for `user` so far.
    pub async fn notifications(&self, user: &TestUser, kind: &str) -> Vec<Event> {
        self.app
            .events("notify.requested")
            .await
            .into_iter()
            .filter(|event| event.data["user_id"] == user.id.to_string() && event.data["kind"] == kind)
            .collect()
    }

    /// Search documents written for `id`, oldest first (`None` = removed).
    pub async fn search_docs(&self, id: &str) -> Vec<Option<serde_json::Value>> {
        self.app
            .events("search.changed")
            .await
            .into_iter()
            .filter(|event| event.data["id"] == id)
            .map(|event| Some(event.data["doc"].clone()).filter(|doc| !doc.is_null()))
            .collect()
    }

    // ── channels, uploads, coins ──

    /// A new channel of `owner` with a unique handle.
    pub async fn channel(&self, owner: &TestUser, name: &str) -> Channel {
        let request = CreateChannelRequest { handle: unique("ch"), name: name.to_owned(), ..Default::default() };
        let response: CreateChannelResponse = self
            .app
            .call("tokuly.video.v1.ChannelService/CreateChannel", Some(owner), &request)
            .await
            .unwrap_or_else(|e| panic!("creating channel {name:?} failed: {e}"));
        response.channel.as_option().cloned().expect("the created channel")
    }

    pub async fn subscribe(&self, user: &TestUser, channel: &Channel, level: NotificationLevel) {
        let request = SubscribeRequest {
            channel_id: channel.summary.id.clone(),
            notification_level: level.into(),
            ..Default::default()
        };
        let _: SubscribeResponse =
            self.app.call("tokuly.video.v1.ChannelService/Subscribe", Some(user), &request).await.expect("subscribe");
    }

    /// A finished upload of `owner`, as the media domain leaves it.
    pub async fn upload(&self, owner: &TestUser, purpose: AssetPurpose, kind: AssetKind) -> String {
        tokuly_media::testing::ready_asset(&self.app.core, owner.id, purpose, kind)
            .await
            .expect("asset fixture")
            .id
            .to_string()
    }

    pub async fn thumbnail(&self, owner: &TestUser) -> String {
        self.upload(owner, AssetPurpose::ASSET_PURPOSE_THUMBNAIL, AssetKind::ASSET_KIND_IMAGE).await
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

    /// Puts `amount` coins into a user's wallet.
    pub async fn coins(&self, user: &TestUser, amount: i64) {
        tokuly_wallet::ledger::grant(
            &self.app.core,
            user.id,
            amount,
            "test.grant",
            &unique("grant-"),
            serde_json::json!({}),
        )
        .await
        .expect("grant coins");
    }

    /// A user's spendable coins and earnings.
    pub async fn balance(&self, user: &TestUser) -> (i64, i64) {
        let balance = tokuly_wallet::ledger::balance(&self.app.core, user.id).await.expect("balance");
        (balance.coins, balance.earnings)
    }

    // ── streams ──

    pub async fn create_stream(&self, owner: &TestUser, request: &CreateStreamRequest) -> Result<Stream, RpcError> {
        let response: CreateStreamResponse = self.call(STREAMS, "CreateStream", Some(owner), request).await?;
        Ok(response.stream.as_option().cloned().expect("the created stream"))
    }

    /// An upcoming stream on `channel`, built by `fill` (public by default).
    pub async fn stream_with(
        &self,
        owner: &TestUser,
        channel: &Channel,
        title: &str,
        fill: impl FnOnce(&mut CreateStreamRequest),
    ) -> Stream {
        let mut request = CreateStreamRequest {
            channel_id: channel.summary.id.clone(),
            title: title.to_owned(),
            ..Default::default()
        };
        fill(&mut request);
        self.create_stream(owner, &request).await.unwrap_or_else(|e| panic!("creating stream {title:?} failed: {e}"))
    }

    /// An upcoming public stream.
    pub async fn stream(&self, owner: &TestUser, channel: &Channel, title: &str) -> Stream {
        self.stream_with(owner, channel, title, |_| {}).await
    }

    /// Puts a stream on air, as its encoder connecting would.
    pub async fn go_live(&self, owner: &TestUser, stream: &Stream) -> Stream {
        self.live.go_live(sid(stream)).await.expect("go live").expect("the stream was upcoming");
        self.get_stream(Some(owner), &card(stream).id).await.expect("the stream on air")
    }

    /// A public stream that is on air.
    pub async fn on_air(&self, owner: &TestUser, channel: &Channel, title: &str) -> Stream {
        let stream = self.stream(owner, channel, title).await;
        self.go_live(owner, &stream).await
    }

    /// A stream on air built by `fill`.
    pub async fn on_air_with(
        &self,
        owner: &TestUser,
        channel: &Channel,
        title: &str,
        fill: impl FnOnce(&mut CreateStreamRequest),
    ) -> Stream {
        let stream = self.stream_with(owner, channel, title, fill).await;
        self.go_live(owner, &stream).await
    }

    pub async fn get_stream(&self, viewer: Option<&TestUser>, stream_id: &str) -> Result<Stream, RpcError> {
        let request = GetStreamRequest { stream_id: stream_id.to_owned(), ..Default::default() };
        let response: GetStreamResponse = self.call(STREAMS, "GetStream", viewer, &request).await?;
        Ok(response.stream.as_option().cloned().expect("the stream"))
    }

    /// The stream as stored (deleted ones included).
    pub async fn row(&self, stream: &Stream) -> StreamRow {
        tokuly_live::streams::find_with_deleted(&self.app.core, &tokuly_core::ids::Ref::Id(sid(stream)))
            .await
            .expect("lookup")
            .expect("the stream row")
    }

    pub async fn end(&self, owner: &TestUser, stream: &Stream) -> Stream {
        let request = EndStreamRequest { stream_id: card(stream).id.clone(), ..Default::default() };
        let response: EndStreamResponse =
            self.call(STREAMS, "EndStream", Some(owner), &request).await.expect("end stream");
        response.stream.as_option().cloned().expect("the ended stream")
    }

    /// Gives a stream `viewers` signed-in viewers (their heartbeats) and samples the audience.
    pub async fn audience(&self, stream: &Stream, viewers: usize) {
        for _ in 0..viewers {
            let key = tokuly_live::audience::ViewerKey::User(Uuid::now_v7());
            tokuly_live::audience::heartbeat(&self.app.core, sid(stream), &key, 15).await.expect("heartbeat");
        }
        self.live.sample_viewers(&Default::default()).await.expect("viewer sample");
    }

    /// Adds `guests` anonymous viewers to a stream's audience, as if they came from as many addresses.
    pub async fn guests(&self, stream: &Stream, guests: usize) {
        for i in 0..guests {
            let key = tokuly_live::audience::ViewerKey::Guest(format!("fixture-{i}"));
            tokuly_live::audience::heartbeat(&self.app.core, sid(stream), &key, 15).await.expect("heartbeat");
        }
        self.live.sample_viewers(&Default::default()).await.expect("viewer sample");
    }

    // ── chat ──

    pub async fn say(&self, user: &TestUser, stream: &Stream, text: &str) -> Result<ChatMessage, RpcError> {
        let request =
            SendChatMessageRequest { stream_id: card(stream).id.clone(), text: text.to_owned(), ..Default::default() };
        let response: SendChatMessageResponse = self.call(CHAT, "SendChatMessage", Some(user), &request).await?;
        Ok(response.message.as_option().cloned().expect("the message"))
    }

    /// The chat as `viewer` reads it, oldest first.
    pub async fn chat(&self, viewer: Option<&TestUser>, stream: &Stream) -> Vec<ChatMessage> {
        let request =
            ListChatMessagesRequest { stream_id: card(stream).id.clone(), page: page(200, ""), ..Default::default() };
        let response: ListChatMessagesResponse =
            self.call(CHAT, "ListChatMessages", viewer, &request).await.expect("chat history");
        response.messages.into_iter().rev().collect()
    }

    pub async fn restrict(
        &self,
        moderator: &TestUser,
        channel: &Channel,
        target: &TestUser,
        seconds: i32,
        delete_messages: bool,
    ) -> Result<ChatRestriction, RpcError> {
        let request = RestrictChatUserRequest {
            channel_id: channel.summary.id.clone(),
            user_id: target.id.to_string(),
            duration_seconds: seconds,
            delete_messages,
            ..Default::default()
        };
        let response: RestrictChatUserResponse = self.call(CHAT, "RestrictChatUser", Some(moderator), &request).await?;
        Ok(response.restriction.as_option().cloned().expect("the restriction"))
    }

    // ── gifts ──

    pub async fn gift(
        &self,
        sender: &TestUser,
        stream: &Stream,
        coins: i64,
        message: &str,
        key: &str,
    ) -> Result<SendGiftResponse, RpcError> {
        let request = SendGiftRequest {
            stream_id: card(stream).id.clone(),
            coins,
            message: message.to_owned(),
            idempotency_key: key.to_owned(),
            ..Default::default()
        };
        self.call(GIFTS, "SendGift", Some(sender), &request).await
    }
}
