//! Shared fixtures for the video integration tests.
//!
//! Every test starts its own app ([`start`]) against the test binary's
//! database and creates its own users, channels and videos. Lists that are
//! global (trending, the home feed, recommended channels) are asserted on
//! through things only the test itself made.
#![allow(dead_code)] // each test binary uses its own subset

use std::time::Duration;

use serde::{Serialize, de::DeserializeOwned};
use tokuly_core::events::Event;
use tokuly_media::{Media, MediaConfig};
use tokuly_proto::{
    common::{PageRequest, Visibility},
    media::{AssetKind, AssetPurpose},
    social::{
        BlockUserRequest, BlockUserResponse, FriendSource, RespondFriendRequestRequest, RespondFriendRequestResponse,
        SendFriendRequestRequest, SendFriendRequestResponse, send_friend_request_request::Target,
    },
    video::*,
};
use tokuly_social::{Social, SocialConfig};
use tokuly_testkit::{FrameListener, RpcError, ServerFrame, TestApp, TestUser};
use tokuly_video::{Video as VideoDomain, VideoConfig};
use uuid::Uuid;

/// A `VideoEvent` as pushed over the realtime gateway.
pub use tokuly_proto::video::video_event::Event as Push;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0200", "0400", "0800"];

pub const CHANNELS: &str = "tokuly.video.v1.ChannelService";
pub const VIDEOS: &str = "tokuly.video.v1.VideoService";
pub const PLAYBACK: &str = "tokuly.video.v1.PlaybackService";
pub const PLAYLISTS: &str = "tokuly.video.v1.PlaylistService";
pub const FEEDS: &str = "tokuly.video.v1.VideoFeedService";
pub const PARTIES: &str = "tokuly.video.v1.WatchPartyService";
pub const ANALYTICS: &str = "tokuly.video.v1.StudioAnalyticsService";

/// How long to wait for a realtime frame that should arrive (generous: the
/// machine may be busy compiling something else).
pub const SOON: Duration = Duration::from_secs(5);
/// How long to listen when asserting that nothing arrives.
pub const QUIET: Duration = Duration::from_millis(300);

/// A running app plus the domains behind it, for calling the public Rust
/// API, running workers and delivering events by hand.
pub struct Ctx {
    pub app: TestApp,
    pub video: VideoDomain,
    pub media: Media,
}

/// Settings that keep tests quick: a view counts after a fifth of a second,
/// and two viewers are an audience analytics break down.
pub fn test_config() -> VideoConfig {
    VideoConfig { min_view_seconds: 0.2, analytics_min_audience: 2, ..Default::default() }
}

pub async fn start() -> Ctx {
    start_with(test_config()).await
}

pub async fn start_with(cfg: VideoConfig) -> Ctx {
    // Show the cause of internal errors (logged, never sent to clients) in
    // the output of a failing test.
    let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::WARN).with_test_writer().try_init();
    let mut domains = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        // Media and social are here for their RPCs: tests upload files, make
        // friends and block people the way clients do.
        let media = Media::new(core.clone(), MediaConfig::default()).expect("media domain");
        let social = Social::new(core.clone(), identity.clone(), SocialConfig::default()).expect("social domain");
        let video = VideoDomain::new(core.clone(), identity.clone(), cfg).expect("video domain");
        let router = video.register(social.register(media.register(identity.register(router))));
        domains = Some((video, media));
        router
    })
    .await;
    let (video, media) = domains.expect("register ran");
    Ctx { app, video, media }
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

/// The card inside a `Video`.
pub fn card(video: &Video) -> &VideoSummary {
    video.summary.as_option().expect("a video has a summary")
}

pub fn titles(videos: &[VideoSummary]) -> Vec<&str> {
    videos.iter().map(|video| video.title.as_str()).collect()
}

/// The `VideoEvent`s among realtime frames, in order.
pub fn pushes(frames: Vec<ServerFrame>) -> Vec<Push> {
    frames.iter().filter_map(|frame| frame.event::<VideoEvent>()?.event).collect()
}

/// Waits for the next `VideoEvent` on a listener.
pub async fn next_push(listener: &mut FrameListener) -> Push {
    loop {
        let frame = listener.next(SOON).await.expect("a realtime frame should arrive");
        if let Some(push) = frame.event::<VideoEvent>().and_then(|event| event.event) {
            return push;
        }
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

    /// `blocker` blocks `blocked` through the social API.
    pub async fn block(&self, blocker: &TestUser, blocked: &TestUser) {
        let request = BlockUserRequest { user_id: blocked.id.to_string(), ..Default::default() };
        let _: BlockUserResponse =
            self.app.call("tokuly.social.v1.FriendService/BlockUser", Some(blocker), &request).await.expect("block");
    }

    /// Delivers the events `actor` caused so far (topic prefix `prefix`) to
    /// the video domain's consumer, as the relay would. Handlers are
    /// idempotent, so delivering an event again is harmless.
    pub async fn deliver(&self, prefix: &str, actor: Uuid) -> usize {
        let events = self.app.events_by(prefix, actor).await;
        let count = events.len();
        for event in events {
            self.video.handle_event(event).await.expect("event handler");
        }
        count
    }

    /// Delivers a hand-made event to the video domain's consumer.
    pub async fn deliver_event(&self, topic: &str, actor: Option<Uuid>, data: serde_json::Value) {
        let event = Event {
            id: Uuid::now_v7(),
            topic: topic.to_owned(),
            actor_id: actor,
            occurred_at: chrono::Utc::now(),
            data,
        };
        self.video.handle_event(event).await.expect("event handler");
    }

    // ── channels ──

    pub async fn create_channel(&self, owner: &TestUser, request: &CreateChannelRequest) -> Result<Channel, RpcError> {
        let response: CreateChannelResponse = self.call(CHANNELS, "CreateChannel", Some(owner), request).await?;
        Ok(response.channel.as_option().cloned().expect("the created channel"))
    }

    /// A new channel of `owner` with a unique handle.
    pub async fn channel(&self, owner: &TestUser, name: &str) -> Channel {
        let request = CreateChannelRequest { handle: unique("ch"), name: name.to_owned(), ..Default::default() };
        self.create_channel(owner, &request).await.unwrap_or_else(|e| panic!("creating channel {name:?} failed: {e}"))
    }

    pub async fn get_channel(&self, viewer: Option<&TestUser>, channel_id: &str) -> Result<Channel, RpcError> {
        let request = GetChannelRequest { channel_id: channel_id.to_owned(), ..Default::default() };
        let response: GetChannelResponse = self.call(CHANNELS, "GetChannel", viewer, &request).await?;
        Ok(response.channel.as_option().cloned().expect("the channel"))
    }

    pub async fn subscribe(&self, user: &TestUser, channel_id: &str, level: NotificationLevel) -> Channel {
        let request = SubscribeRequest {
            channel_id: channel_id.to_owned(),
            notification_level: level.into(),
            ..Default::default()
        };
        let response: SubscribeResponse =
            self.call(CHANNELS, "Subscribe", Some(user), &request).await.expect("subscribe");
        response.channel.as_option().cloned().expect("the channel")
    }

    // ── uploads ──

    /// A finished upload of `owner`, as the media domain leaves it.
    pub async fn upload(&self, owner: &TestUser, purpose: AssetPurpose, kind: AssetKind) -> String {
        tokuly_media::testing::ready_asset(&self.app.core, owner.id, purpose, kind)
            .await
            .expect("asset fixture")
            .id
            .to_string()
    }

    /// A processed video upload (12 seconds long).
    pub async fn video_upload(&self, owner: &TestUser) -> String {
        tokuly_media::testing::ready_video(&self.app.core, owner.id, AssetPurpose::ASSET_PURPOSE_VIDEO)
            .await
            .expect("video fixture")
            .id
            .to_string()
    }

    pub async fn image_upload(&self, owner: &TestUser, purpose: AssetPurpose) -> String {
        tokuly_media::testing::ready_image(&self.app.core, owner.id, purpose)
            .await
            .expect("image fixture")
            .id
            .to_string()
    }

    // ── videos ──

    pub async fn create_video(&self, owner: &TestUser, request: &CreateVideoRequest) -> Result<Video, RpcError> {
        let response: CreateVideoResponse = self.call(VIDEOS, "CreateVideo", Some(owner), request).await?;
        Ok(response.video.as_option().cloned().expect("the created video"))
    }

    /// A draft on `channel` from a processed upload, built by `fill`.
    pub async fn draft_with(
        &self,
        owner: &TestUser,
        channel: &Channel,
        title: &str,
        fill: impl FnOnce(&mut CreateVideoRequest),
    ) -> Video {
        let mut request = CreateVideoRequest {
            channel_id: channel.summary.id.clone(),
            asset_id: self.video_upload(owner).await,
            title: title.to_owned(),
            ..Default::default()
        };
        fill(&mut request);
        self.create_video(owner, &request).await.unwrap_or_else(|e| panic!("creating video {title:?} failed: {e}"))
    }

    pub async fn draft(&self, owner: &TestUser, channel: &Channel, title: &str) -> Video {
        self.draft_with(owner, channel, title, |_| {}).await
    }

    pub async fn publish(&self, owner: &TestUser, video_id: &str) -> Video {
        let request = PublishVideoRequest { video_id: video_id.to_owned(), ..Default::default() };
        let response: PublishVideoResponse =
            self.call(VIDEOS, "PublishVideo", Some(owner), &request).await.expect("publish");
        response.video.as_option().cloned().expect("the published video")
    }

    /// A published video with the given visibility, built by `fill`.
    pub async fn published_with(
        &self,
        owner: &TestUser,
        channel: &Channel,
        title: &str,
        visibility: Visibility,
        fill: impl FnOnce(&mut CreateVideoRequest),
    ) -> Video {
        let draft = self
            .draft_with(owner, channel, title, |request| {
                request.visibility = visibility.into();
                fill(request);
            })
            .await;
        self.publish(owner, &card(&draft).id).await
    }

    pub async fn published(&self, owner: &TestUser, channel: &Channel, title: &str, visibility: Visibility) -> Video {
        self.published_with(owner, channel, title, visibility, |_| {}).await
    }

    /// A published public video.
    pub async fn public_video(&self, owner: &TestUser, channel: &Channel, title: &str) -> Video {
        self.published(owner, channel, title, Visibility::VISIBILITY_PUBLIC).await
    }

    pub async fn get_video(&self, viewer: Option<&TestUser>, video_id: &str) -> Result<Video, RpcError> {
        let request = GetVideoRequest { video_id: video_id.to_owned(), ..Default::default() };
        let response: GetVideoResponse = self.call(VIDEOS, "GetVideo", viewer, &request).await?;
        Ok(response.video.as_option().cloned().expect("the video"))
    }

    pub async fn rate(&self, user: &TestUser, video_id: &str, rating: Rating) -> RateVideoResponse {
        let request = RateVideoRequest { video_id: video_id.to_owned(), rating: rating.into(), ..Default::default() };
        self.call(VIDEOS, "RateVideo", Some(user), &request).await.expect("rate")
    }

    // ── playback ──

    pub async fn start_playback(
        &self,
        viewer: Option<&TestUser>,
        video_id: &str,
        source: TrafficSource,
    ) -> StartPlaybackResponse {
        let request =
            StartPlaybackRequest { video_id: video_id.to_owned(), source: source.into(), ..Default::default() };
        self.call(PLAYBACK, "StartPlayback", viewer, &request).await.expect("start playback")
    }

    pub async fn report(
        &self,
        viewer: Option<&TestUser>,
        session_id: &str,
        position: f64,
        playing: bool,
        ended: bool,
    ) -> ReportPlaybackResponse {
        let request = ReportPlaybackRequest {
            session_id: session_id.to_owned(),
            position_seconds: position,
            playing,
            ended,
            ..Default::default()
        };
        self.call(PLAYBACK, "ReportPlayback", viewer, &request).await.expect("report playback")
    }

    /// Watches a video long enough for the view to count: starts a session,
    /// lets the minimum watch time pass, and reports once at `position`.
    pub async fn watch(
        &self,
        viewer: Option<&TestUser>,
        video_id: &str,
        position: f64,
        source: TrafficSource,
    ) -> ReportPlaybackResponse {
        let started = self.start_playback(viewer, video_id, source).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        self.report(viewer, &started.session_id, position, true, false).await
    }
}
