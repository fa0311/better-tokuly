//! Shared fixtures for the shorts integration tests.
//!
//! Every test starts its own app ([`start`]) against the test binary's
//! database and creates its own users, channels and shorts. Lists that are
//! global (the for-you feed, trending hashtags, sound suggestions) are
//! asserted on through things only the test itself made.
//!
//! Valkey is shared by every test, so the buffer of playback counters is
//! too: tests that report playback and then flush hold [`playback_lock`]
//! for the whole sequence.
#![allow(dead_code)] // each test binary uses its own subset

use std::time::Duration;

use serde::{Serialize, de::DeserializeOwned};
use tokuly_core::events::Event;
use tokuly_media::{Media, MediaConfig};
use tokuly_proto::{
    common::{PageRequest, Visibility},
    media::{AssetKind, AssetPurpose},
    shorts::*,
    social::{
        BlockUserRequest, BlockUserResponse, FriendSource, RespondFriendRequestRequest, RespondFriendRequestResponse,
        SendFriendRequestRequest, SendFriendRequestResponse, send_friend_request_request::Target,
    },
    video::{
        Channel, CreateChannelRequest, CreateChannelResponse, NotificationLevel, SubscribeRequest, SubscribeResponse,
    },
};
use tokuly_shorts::{
    Shorts, ShortsConfig,
    shorts::{self, ShortRow},
};
use tokuly_social::{Social, SocialConfig};
use tokuly_testkit::{RpcError, TestApp, TestUser};
use tokuly_video::{Video as VideoDomain, VideoConfig};
use uuid::Uuid;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0200", "0400", "0800", "1000"];

pub const SHORTS: &str = "tokuly.shorts.v1.ShortService";
pub const FEEDS: &str = "tokuly.shorts.v1.ShortFeedService";
pub const DISCOVERY: &str = "tokuly.shorts.v1.ShortDiscoveryService";
pub const ANALYTICS: &str = "tokuly.shorts.v1.ShortAnalyticsService";
pub const CHANNELS: &str = "tokuly.video.v1.ChannelService";

/// A running app plus the domains behind it, for calling the public Rust
/// API, running workers and delivering events by hand.
pub struct Ctx {
    pub app: TestApp,
    pub shorts: Shorts,
    pub media: Media,
    pub video: VideoDomain,
}

pub async fn start() -> Ctx {
    start_with(ShortsConfig::default()).await
}

pub async fn start_with(cfg: ShortsConfig) -> Ctx {
    // Show the cause of internal errors (logged, never sent to clients) in
    // the output of a failing test.
    let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::WARN).with_test_writer().try_init();
    let mut domains = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        // Media, social and video are here for their RPCs: tests upload
        // files, make friends, create channels and subscribe the way
        // clients do.
        let media = Media::new(core.clone(), MediaConfig::default()).expect("media domain");
        let social = Social::new(core.clone(), identity.clone(), SocialConfig::default()).expect("social domain");
        let video = VideoDomain::new(core.clone(), identity.clone(), VideoConfig::default()).expect("video domain");
        let shorts = Shorts::new(core.clone(), identity.clone(), cfg).expect("shorts domain");
        let router = shorts.register(video.register(social.register(media.register(identity.register(router)))));
        domains = Some((shorts, media, video));
        router
    })
    .await;
    let (shorts, media, video) = domains.expect("register ran");
    Ctx { app, shorts, media, video }
}

/// Serializes "report playback, flush, look at the counters" across the
/// tests of a binary: the buffer in Valkey is one for all of them.
pub async fn playback_lock() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

/// A lowercase string no other test uses, e.g. for handles and hashtags.
pub fn unique(prefix: &str) -> String {
    format!("{prefix}{}", tokuly_core::ids::random_base62(10).to_ascii_lowercase())
}

pub fn page<P: buffa::ProtoBox<PageRequest>>(limit: i32, cursor: &str) -> buffa::MessageField<PageRequest, P> {
    buffa::MessageField::some(PageRequest { limit, cursor: cursor.to_owned(), ..Default::default() })
}

pub fn id(value: &str) -> Uuid {
    value.parse().expect("an ID")
}

/// The captions of a list of shorts, in order.
pub fn captions(shorts: &[Short]) -> Vec<&str> {
    shorts.iter().map(|short| short.caption.as_str()).collect()
}

/// The short inside a feed item.
pub fn item_short(item: &FeedItem) -> &Short {
    item.short.as_option().expect("a feed item has a short")
}

pub fn viewer_state(short: &Short) -> &ShortViewerState {
    short.viewer.as_option().expect("signed-in callers get a viewer state")
}

/// One stay on a short, as a client reports it.
pub fn stay(short_id: &str, source: PlaybackSource, watched_ms: i64, completions: i32) -> PlaybackReport {
    PlaybackReport {
        short_id: short_id.to_owned(),
        source: source.into(),
        watched_ms,
        completions,
        ..Default::default()
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

    // ── people ──

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

    // ── events ──

    /// Delivers the events `actor` caused so far (topic prefix `prefix`) to
    /// the shorts domain's consumer, as the relay would. Handlers are
    /// idempotent, so delivering an event again is harmless.
    pub async fn deliver(&self, prefix: &str, actor: Uuid) -> usize {
        let events = self.app.events_by(prefix, actor).await;
        let count = events.len();
        for event in events {
            self.shorts.handle_event(event).await.expect("event handler");
        }
        count
    }

    /// Delivers a hand-made event to the shorts domain's consumer.
    pub async fn deliver_event(&self, topic: &str, actor: Option<Uuid>, data: serde_json::Value) {
        let event = Event {
            id: Uuid::now_v7(),
            topic: topic.to_owned(),
            actor_id: actor,
            occurred_at: chrono::Utc::now(),
            data,
        };
        self.shorts.handle_event(event).await.expect("event handler");
    }

    /// Notifications this domain requested for `user_id`, as `(kind,
    /// request)`. (Making friends and subscribing notify people too.)
    pub async fn notifications(&self, user_id: Uuid) -> Vec<(String, serde_json::Value)> {
        self.app
            .events("notify.requested")
            .await
            .into_iter()
            .filter(|event| event.data["user_id"] == user_id.to_string())
            .map(|event| (event.data["kind"].as_str().unwrap_or_default().to_owned(), event.data))
            .filter(|(kind, _)| kind.starts_with("shorts."))
            .collect()
    }

    /// The kinds of the notifications requested for `user_id`.
    pub async fn notified(&self, user_id: Uuid) -> Vec<String> {
        self.notifications(user_id).await.into_iter().map(|(kind, _)| kind).collect()
    }

    // ── channels ──

    /// A new channel of `owner` with a unique handle.
    pub async fn channel(&self, owner: &TestUser, name: &str) -> Channel {
        let request = CreateChannelRequest { handle: unique("ch"), name: name.to_owned(), ..Default::default() };
        let response: CreateChannelResponse = self
            .call(CHANNELS, "CreateChannel", Some(owner), &request)
            .await
            .unwrap_or_else(|e| panic!("creating channel {name:?} failed: {e}"));
        response.channel.as_option().cloned().expect("the created channel")
    }

    /// `user` subscribes to a channel through the video API, and the shorts
    /// domain hears of it.
    pub async fn subscribe(&self, user: &TestUser, channel: &Channel, level: NotificationLevel) {
        let request = SubscribeRequest {
            channel_id: channel.summary.id.clone(),
            notification_level: level.into(),
            ..Default::default()
        };
        let _: SubscribeResponse = self.call(CHANNELS, "Subscribe", Some(user), &request).await.expect("subscribe");
        self.deliver("video.channel.subscribed", user.id).await;
    }

    // ── uploads ──

    /// A processed upload made for a short (12 seconds, 1280×720, with audio).
    pub async fn upload(&self, owner: &TestUser) -> String {
        tokuly_media::testing::ready_video(&self.app.core, owner.id, AssetPurpose::ASSET_PURPOSE_SHORT)
            .await
            .expect("video fixture")
            .id
            .to_string()
    }

    /// A finished upload of `owner` of any purpose and kind.
    pub async fn asset(&self, owner: &TestUser, purpose: AssetPurpose, kind: AssetKind) -> String {
        tokuly_media::testing::ready_asset(&self.app.core, owner.id, purpose, kind)
            .await
            .expect("asset fixture")
            .id
            .to_string()
    }

    /// Rewrites what the media domain says about a processed video, so a
    /// fixture can be longer, wider or silent.
    pub async fn reshape(&self, asset_id: &str, seconds: f64, width: i32, height: i32, has_audio: bool) {
        let asset = tokuly_media::assets::find(&self.app.core, id(asset_id)).await.expect("lookup").expect("the asset");
        let mut info = asset.video().expect("a video asset");
        (info.duration_seconds, info.width, info.height, info.has_audio) = (seconds, width, height, has_audio);
        sqlx::query("UPDATE media_assets SET video = $2 WHERE id = $1")
            .bind(asset.id)
            .bind(serde_json::to_value(&info).expect("video info"))
            .execute(&self.app.core.db)
            .await
            .expect("reshape fixture");
    }

    /// Puts an asset back into (or takes it out of) the PROCESSING state.
    pub async fn set_asset_status(&self, asset_id: &str, status: tokuly_proto::media::AssetStatus, error: &str) {
        sqlx::query("UPDATE media_assets SET status = $2, error = $3 WHERE id = $1")
            .bind(id(asset_id))
            .bind(status as i32)
            .bind(error)
            .execute(&self.app.core.db)
            .await
            .expect("asset status");
    }

    // ── shorts ──

    pub async fn create(&self, owner: &TestUser, request: &CreateShortRequest) -> Result<Short, RpcError> {
        let response: CreateShortResponse = self.call(SHORTS, "CreateShort", Some(owner), request).await?;
        Ok(response.short.as_option().cloned().expect("the created short"))
    }

    /// A published short on `channel` from a processed upload, built by `fill`.
    pub async fn short_with(
        &self,
        owner: &TestUser,
        channel: &Channel,
        caption: &str,
        fill: impl FnOnce(&mut CreateShortRequest),
    ) -> Short {
        let mut request = CreateShortRequest {
            channel_id: channel.summary.id.clone(),
            asset_id: self.upload(owner).await,
            caption: caption.to_owned(),
            visibility: Visibility::VISIBILITY_PUBLIC.into(),
            ..Default::default()
        };
        fill(&mut request);
        self.create(owner, &request).await.unwrap_or_else(|e| panic!("creating short {caption:?} failed: {e}"))
    }

    /// A published public short.
    pub async fn short(&self, owner: &TestUser, channel: &Channel, caption: &str) -> Short {
        self.short_with(owner, channel, caption, |_| {}).await
    }

    pub async fn get(&self, viewer: Option<&TestUser>, short_id: &str) -> Result<Short, RpcError> {
        let request = GetShortRequest { short_id: short_id.to_owned(), ..Default::default() };
        let response: GetShortResponse = self.call(SHORTS, "GetShort", viewer, &request).await?;
        Ok(response.short.as_option().cloned().expect("the short"))
    }

    pub async fn update(&self, owner: &TestUser, request: &UpdateShortRequest) -> Result<Short, RpcError> {
        let response: UpdateShortResponse = self.call(SHORTS, "UpdateShort", Some(owner), request).await?;
        Ok(response.short.as_option().cloned().expect("the updated short"))
    }

    pub async fn set_visibility(&self, owner: &TestUser, short_id: &str, visibility: Visibility) -> Short {
        let request = UpdateShortRequest {
            short_id: short_id.to_owned(),
            visibility: Some(visibility.into()),
            ..Default::default()
        };
        self.update(owner, &request).await.expect("change visibility")
    }

    pub async fn delete(&self, owner: &TestUser, short_id: &str) {
        let request = DeleteShortRequest { short_id: short_id.to_owned(), ..Default::default() };
        let _: DeleteShortResponse =
            self.call(SHORTS, "DeleteShort", Some(owner), &request).await.expect("delete short");
    }

    /// The stored row of a short.
    pub async fn row(&self, short_id: &str) -> ShortRow {
        shorts::find_by_id(&self.app.core, id(short_id)).await.expect("lookup").expect("the short exists")
    }

    // ── playback ──

    pub async fn report(&self, viewer: Option<&TestUser>, reports: Vec<PlaybackReport>) -> ReportPlaybackResponse {
        let request = ReportPlaybackRequest { reports, ..Default::default() };
        self.call(FEEDS, "ReportPlayback", viewer, &request).await.expect("report playback")
    }

    /// `viewer` watches a short to the end once, in the for-you feed.
    pub async fn watch(&self, viewer: &TestUser, short_id: &str) -> ReportPlaybackResponse {
        self.report(Some(viewer), vec![stay(short_id, PlaybackSource::PLAYBACK_SOURCE_FOR_YOU, 12_000, 1)]).await
    }

    /// Moves the buffered playback counters into PostgreSQL.
    pub async fn flush(&self) {
        self.shorts.flush_all_signals().await.expect("flush");
    }

    // ── feeds ──

    pub async fn for_you(&self, viewer: Option<&TestUser>, limit: i32, cursor: &str) -> GetForYouFeedResponse {
        let request = GetForYouFeedRequest { page: page(limit, cursor), ..Default::default() };
        self.call(FEEDS, "GetForYouFeed", viewer, &request).await.expect("for-you feed")
    }

    /// Pages through one batch of the for-you feed, at most `pages` pages
    /// of 30, and returns its items in order.
    pub async fn for_you_all(&self, viewer: Option<&TestUser>, pages: usize) -> Vec<FeedItem> {
        let mut items = Vec::new();
        let mut cursor = String::new();
        for _ in 0..pages {
            let response = self.for_you(viewer, 30, &cursor).await;
            let got = response.items.len();
            items.extend(response.items);
            let info = response.page.as_option().cloned().unwrap_or_default();
            cursor = info.next_cursor;
            if got < 30 || !info.has_more {
                break;
            }
        }
        items
    }

    pub async fn mark_not_interested(&self, viewer: &TestUser, short_id: &str) {
        let request = MarkNotInterestedRequest { short_id: short_id.to_owned(), ..Default::default() };
        let _: MarkNotInterestedResponse =
            self.call(FEEDS, "MarkNotInterested", Some(viewer), &request).await.expect("not interested");
    }

    /// What the engagement domain does when someone likes a short: the
    /// counter callback, then the event.
    pub async fn like(&self, user: &TestUser, short_id: &str) {
        self.set_like(user, short_id, true).await;
    }

    pub async fn set_like(&self, user: &TestUser, short_id: &str, liked: bool) {
        use tokuly_proto::common::SubjectType;
        let short = id(short_id);
        let delta = if liked { 1 } else { -1 };
        self.app
            .core
            .subjects
            .counter_changed(SubjectType::SUBJECT_TYPE_SHORT, short, "like_count", delta)
            .await
            .expect("counter callback");
        let (kind, previous) = if liked { (1, 0) } else { (0, 1) };
        self.deliver_event(
            "engagement.reaction.changed",
            Some(user.id),
            serde_json::json!({
                "user_id": user.id,
                "subject_type": SubjectType::SUBJECT_TYPE_SHORT as i32,
                "subject_id": short,
                "subject_owner_id": null,
                "kind": kind,
                "emoji": "",
                "previous_kind": previous,
                "previous_emoji": "",
            }),
        )
        .await;
    }
}

/// How long to wait for something that happens in the background.
pub const SOON: Duration = Duration::from_secs(10);

/// What the hourly statistics of the current hour say about a short:
/// `(views, loops, likes, comments, shares, saves)`; zeros when there are none.
pub async fn hourly(ctx: &Ctx, short_id: &str) -> (i64, i64, i64, i64, i64, i64) {
    sqlx::query_as(
        "SELECT COALESCE(sum(views), 0)::bigint, COALESCE(sum(loops), 0)::bigint, COALESCE(sum(likes), 0)::bigint,
                COALESCE(sum(comments), 0)::bigint, COALESCE(sum(shares), 0)::bigint, COALESCE(sum(saves), 0)::bigint
         FROM short_hourly_stats WHERE short_id = $1",
    )
    .bind(id(short_id))
    .fetch_one(&ctx.app.core.db)
    .await
    .expect("hourly statistics")
}
