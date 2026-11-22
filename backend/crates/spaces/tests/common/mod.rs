//! Shared fixtures for the spaces integration tests.
//!
//! Every test starts its own app ([`start`]) against the test binary's
//! database and creates its own users. Friendships, blocks and privacy
//! settings are set up through the owning domains' own RPCs. Audio runs on
//! the LiveKit server of the development infrastructure (`TOKULY_LIVEKIT_*`,
//! exported by `tools/dev/infra.sh env`); nobody sends any sound, but a
//! signalling connection ([`Ctx::connect`]) makes someone as "connected" as
//! a real client would be.
#![allow(dead_code)]

use std::time::Duration;

use futures::StreamExt as _;
use serde::{Serialize, de::DeserializeOwned};
use tokuly_core::{
    livekit::{Claims, LiveKit, Participant},
    outbound::{ActivityEntry, NotificationRequest, SearchChange},
};
use tokuly_identity::settings;
use tokuly_proto::{
    identity::{PrivacySettings, UpdateSettingsRequest},
    realtime::{ServerFrame, server_frame::Body},
    social::{BlockUserRequest, SendFriendRequestRequest, send_friend_request_request::Target},
    spaces::{spaces_event::Event as Push, *},
};
use tokuly_social::{Social, SocialConfig};
use tokuly_spaces::{Spaces, SpacesConfig, Venue};
use tokuly_testkit::{FrameListener, RpcError, TestApp, TestUser};
use uuid::Uuid;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0200", "0400", "2200"];

/// How long to listen for the realtime frames an action should have produced.
pub const SOON: Duration = Duration::from_millis(400);

/// Held by a test while it makes something look overdue and runs a worker
/// that deals with everything that is overdue. The tests of one binary
/// share a database and run in parallel, so without it one test's sweep
/// could do another test's work.
pub static SWEEPS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A running app plus the spaces domain behind it, for calling its workers
/// and event handlers directly.
pub struct Ctx {
    pub app: TestApp,
    pub spaces: Spaces,
}

/// The domain on the development LiveKit server, with default settings.
pub async fn start() -> Ctx {
    start_with(SpacesConfig::default()).await
}

/// The domain on the development LiveKit server.
pub async fn start_with(cfg: SpacesConfig) -> Ctx {
    let ctx = build(cfg, None).await;
    assert!(
        ctx.spaces.audio_enabled(),
        "these tests need the LiveKit settings of the dev infrastructure: eval \"$(tools/dev/infra.sh env)\""
    );
    ctx
}

/// The domain on a server that has no LiveKit.
pub async fn start_without_audio() -> Ctx {
    build(SpacesConfig::default(), Some(LiveKit::disabled())).await
}

async fn build(cfg: SpacesConfig, livekit: Option<LiveKit>) -> Ctx {
    let mut domain = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        let social = Social::new(core.clone(), identity.clone(), SocialConfig::default()).expect("social domain");
        let spaces = match livekit {
            Some(livekit) => Spaces::with_livekit(core.clone(), identity.clone(), cfg, livekit),
            None => Spaces::new(core.clone(), identity.clone(), cfg),
        }
        .expect("spaces domain");
        let router = spaces.register(social.register(identity.register(router)));
        domain = Some(spaces);
        router
    })
    .await;
    Ctx { app, spaces: domain.expect("register ran") }
}

/// The spaces events among realtime frames, in order.
pub fn pushes(frames: Vec<ServerFrame>) -> Vec<Push> {
    frames
        .into_iter()
        .filter_map(|frame| match frame.body {
            Some(Body::Spaces(event)) => event.event,
            _ => None,
        })
        .collect()
}

/// A signalling connection to LiveKit: while it is held, its participant
/// is connected to the room.
pub struct Connection {
    reader: tokio::task::JoinHandle<()>,
}

impl Connection {
    /// Drops the connection the way a crashed app does: without a word.
    pub fn drop_dead(self) {
        self.reader.abort();
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

fn space_of<P: buffa::ProtoBox<Space>>(space: &buffa::MessageField<Space, P>) -> Space {
    space.as_option().expect("the response carries the room").clone()
}

impl Ctx {
    /// Calls `tokuly.spaces.v1.<service>Service/<method>`.
    pub async fn call<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        service: &str,
        method: &str,
        user: &TestUser,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.app.call(&format!("tokuly.spaces.v1.{service}Service/{method}"), Some(user), request).await
    }

    /// The same without signing in.
    pub async fn anonymous<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        service: &str,
        method: &str,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.app.call(&format!("tokuly.spaces.v1.{service}Service/{method}"), None, request).await
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
        match self.anonymous::<_, serde_json::Value>(service, method, request).await {
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

    /// Two new users who are friends.
    pub async fn friends(&self, a: &str, b: &str) -> (TestUser, TestUser) {
        let (a, b) = (self.app.user(a).await, self.app.user(b).await);
        self.befriend(&a, &b).await;
        (a, b)
    }

    pub async fn block(&self, blocker: &TestUser, blocked: &TestUser) {
        let request = BlockUserRequest { user_id: blocked.id.to_string(), ..Default::default() };
        let _: serde_json::Value =
            self.app.call("tokuly.social.v1.FriendService/BlockUser", Some(blocker), &request).await.expect("block");
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

    /// Notifications this domain requested for `user` so far, oldest first
    /// (befriending people leaves notifications of the social domain too).
    pub async fn notifications(&self, user: &TestUser) -> Vec<NotificationRequest> {
        self.app
            .events("notify.requested")
            .await
            .into_iter()
            .filter_map(|event| event.data::<NotificationRequest>().ok())
            .filter(|request| request.user_id == user.id && request.kind.starts_with("spaces."))
            .collect()
    }

    /// Activity entries recorded for `user` so far.
    pub async fn activities(&self, user: &TestUser) -> Vec<ActivityEntry> {
        self.app
            .events("activity.recorded")
            .await
            .into_iter()
            .filter_map(|event| event.data::<ActivityEntry>().ok())
            .filter(|entry| entry.user_id == user.id)
            .collect()
    }

    /// Search index changes about one room so far, oldest first.
    pub async fn search_changes(&self, space_id: &str) -> Vec<SearchChange> {
        self.app
            .events("search.changed")
            .await
            .into_iter()
            .filter_map(|event| event.data::<SearchChange>().ok())
            .filter(|change| change.id.to_string() == space_id)
            .collect()
    }

    // ───────────────────────────── realtime ─────────────────────────────

    pub async fn listen(&self, user: &TestUser) -> FrameListener {
        self.app.listen_user(user.id).await
    }

    pub async fn listen_room(&self, space_id: &str) -> FrameListener {
        self.app.listen_topic(&format!("space:{space_id}")).await
    }

    /// The spaces events that arrive within [`SOON`].
    pub async fn events(&self, listener: &mut FrameListener) -> Vec<Push> {
        pushes(listener.drain(SOON).await)
    }

    // ───────────────────────────── rooms ─────────────────────────────

    /// Starts a room on the spot; returns it and the host's credentials.
    pub async fn open_room(&self, host: &TestUser, title: &str, audience: SpaceAudience) -> (Space, AudioCredentials) {
        let request = CreateSpaceRequest { title: title.to_owned(), audience: audience.into(), ..Default::default() };
        let response: CreateSpaceResponse =
            self.call("Space", "CreateSpace", host, &request).await.expect("create room");
        (space_of(&response.space), response.credentials.as_option().expect("credentials").clone())
    }

    /// A public room that is live.
    pub async fn public_room(&self, host: &TestUser, title: &str) -> Space {
        self.open_room(host, title, SpaceAudience::SPACE_AUDIENCE_PUBLIC).await.0
    }

    /// Schedules a room an hour from now.
    pub async fn schedule(&self, host: &TestUser, title: &str, audience: SpaceAudience) -> Space {
        let request = CreateSpaceRequest {
            title: title.to_owned(),
            audience: audience.into(),
            scheduled_at: tokuly_core::time::ts(chrono::Utc::now() + chrono::Duration::hours(1)),
            ..Default::default()
        };
        let response: CreateSpaceResponse =
            self.call("Space", "CreateSpace", host, &request).await.expect("schedule room");
        assert!(response.credentials.is_unset(), "nobody enters a room that has not started");
        space_of(&response.space)
    }

    pub async fn join(&self, user: &TestUser, space_id: &str) -> JoinSpaceResponse {
        self.call("Space", "JoinSpace", user, &JoinSpaceRequest { space_id: space_id.to_owned(), ..Default::default() })
            .await
            .expect("join room")
    }

    pub async fn leave(&self, user: &TestUser, space_id: &str) {
        let _: LeaveSpaceResponse = self
            .call(
                "Space",
                "LeaveSpace",
                user,
                &LeaveSpaceRequest { space_id: space_id.to_owned(), ..Default::default() },
            )
            .await
            .expect("leave room");
    }

    pub async fn get(&self, user: &TestUser, space_id: &str) -> Space {
        let response: GetSpaceResponse = self
            .call("Space", "GetSpace", user, &GetSpaceRequest { space_id: space_id.to_owned(), ..Default::default() })
            .await
            .expect("get room");
        space_of(&response.space)
    }

    /// Raises `user`'s hand and has `moderator` bring them on stage.
    pub async fn bring_on_stage(&self, moderator: &TestUser, user: &TestUser, space_id: &str) {
        let raise = SetHandRaisedRequest { space_id: space_id.to_owned(), raised: true, ..Default::default() };
        let _: SetHandRaisedResponse = self.call("Space", "SetHandRaised", user, &raise).await.expect("raise hand");
        let invite =
            InviteToStageRequest { space_id: space_id.to_owned(), user_id: user.id.to_string(), ..Default::default() };
        let response: InviteToStageResponse =
            self.call("Space", "InviteToStage", moderator, &invite).await.expect("invite to stage");
        assert!(response.on_stage, "a raised hand is answered at once");
    }

    pub async fn make_cohost(&self, host: &TestUser, user: &TestUser, space_id: &str) {
        let request = SetCohostRequest {
            space_id: space_id.to_owned(),
            user_id: user.id.to_string(),
            cohost: true,
            ..Default::default()
        };
        let _: SetCohostResponse = self.call("Space", "SetCohost", host, &request).await.expect("make co-host");
    }

    /// The role `user` has in the room's list of participants, if inside.
    pub async fn role_of(&self, viewer: &TestUser, user: &TestUser, space_id: &str) -> Option<SpaceRole> {
        let request = ListParticipantsRequest { space_id: space_id.to_owned(), ..Default::default() };
        let response: ListParticipantsResponse =
            self.call("Space", "ListParticipants", viewer, &request).await.expect("list participants");
        response
            .participants
            .iter()
            .find(|participant| participant.user.id == user.id.to_string())
            .and_then(|participant| participant.role.as_known())
    }

    // ───────────────────────────── LiveKit ─────────────────────────────

    pub fn livekit(&self) -> &LiveKit {
        &self.spaces.livekit
    }

    /// The claims of a token this server minted.
    pub fn claims(&self, credentials: &AudioCredentials) -> Claims {
        self.livekit().decode_token(&credentials.token).expect("a token of this server")
    }

    /// Connects to LiveKit with `credentials`, the way a client does, and
    /// waits until LiveKit lists the participant.
    pub async fn connect(&self, credentials: &AudioCredentials) -> Connection {
        let url = format!(
            "{}/rtc?access_token={}&auto_subscribe=1&protocol=16&sdk=rust&version=0.1",
            credentials.url.trim_end_matches('/'),
            credentials.token
        );
        let (socket, _) = tokio_tungstenite::connect_async(url).await.expect("LiveKit accepts the token");
        // Reading keeps the connection answered (pings) until it is dropped.
        let reader = tokio::spawn(async move {
            let (_sink, mut stream) = socket.split();
            while let Some(Ok(_)) = stream.next().await {}
        });
        let identity = self.claims(credentials).sub;
        self.wait_until_connected(&credentials.room, &identity, true).await;
        Connection { reader }
    }

    /// The participant LiveKit lists under `identity` in `room`.
    pub async fn connected(&self, room: &str, identity: &str) -> Option<Participant> {
        self.livekit().get_participant(room, identity).await.expect("LiveKit answers")
    }

    /// Waits until LiveKit lists (or no longer lists) `identity` in `room`.
    pub async fn wait_until_connected(&self, room: &str, identity: &str, connected: bool) {
        for _ in 0..300 {
            if self.connected(room, identity).await.is_some() == connected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("LiveKit never showed {identity} as {}", if connected { "connected" } else { "gone" });
    }

    /// The venue of a room.
    pub fn venue(space_id: &str) -> Venue {
        Venue::Space(Uuid::parse_str(space_id).expect("room ID"))
    }

    // ───────────────────────────── hangouts ─────────────────────────────

    /// Creates a hangout and has every invitee accept.
    pub async fn hangout(&self, owner: &TestUser, name: &str, members: &[&TestUser]) -> Hangout {
        let request = CreateHangoutRequest {
            name: name.to_owned(),
            members_can_invite: true,
            invitee_ids: members.iter().map(|member| member.id.to_string()).collect(),
            ..Default::default()
        };
        let response: CreateHangoutResponse =
            self.call("Hangout", "CreateHangout", owner, &request).await.expect("create hangout");
        let hangout = response.hangout.as_option().expect("hangout").clone();
        for member in members {
            let accept =
                RespondToHangoutInviteRequest { hangout_id: hangout.id.clone(), accept: true, ..Default::default() };
            let _: RespondToHangoutInviteResponse =
                self.call("Hangout", "RespondToHangoutInvite", member, &accept).await.expect("accept invitation");
        }
        self.get_hangout(owner, &hangout.id).await
    }

    pub async fn get_hangout(&self, user: &TestUser, hangout_id: &str) -> Hangout {
        let request = GetHangoutRequest { hangout_id: hangout_id.to_owned(), ..Default::default() };
        let response: GetHangoutResponse =
            self.call("Hangout", "GetHangout", user, &request).await.expect("get hangout");
        response.hangout.as_option().expect("hangout").clone()
    }

    pub async fn join_hangout(&self, user: &TestUser, hangout_id: &str) -> JoinHangoutResponse {
        let request = JoinHangoutRequest { hangout_id: hangout_id.to_owned(), ..Default::default() };
        self.call("Hangout", "JoinHangout", user, &request).await.expect("join hangout")
    }
}
