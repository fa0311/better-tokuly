//! Shared setup and helpers for the social integration tests.
#![allow(dead_code)] // each test binary uses its own subset

use std::time::Duration;

use rand::RngExt as _;
use tokuly_core::events::Event;
use tokuly_identity::settings;
use tokuly_proto::{
    common::LatLng,
    identity::{PrivacySettings, UpdateSettingsRequest, UpdateSettingsResponse},
    realtime::{PresenceEvent, server_frame::Body},
    social::{social_event::Event as Push, *},
};
use tokuly_social::{Social, SocialConfig};
use tokuly_testkit::{FrameListener, TestApp, TestUser};
use uuid::Uuid;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0200", "0400"];

pub const FRIENDS: &str = "tokuly.social.v1.FriendService";
pub const INTERACTIONS: &str = "tokuly.social.v1.InteractionService";

/// How long to wait for a realtime frame that should arrive (generous: the
/// machine may be busy compiling something else).
pub const SOON: Duration = Duration::from_secs(5);
/// How long to listen when asserting that nothing arrives.
pub const QUIET: Duration = Duration::from_millis(300);

/// A running test app plus the domain under test (for its worker entry points).
pub struct Ctx {
    pub app: TestApp,
    pub social: Social,
}

pub async fn start() -> Ctx {
    start_with(SocialConfig::default()).await
}

pub async fn start_with(cfg: SocialConfig) -> Ctx {
    let mut social = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        let domain = Social::new(core.clone(), identity.clone(), cfg).expect("social config");
        let router = domain.register(identity.register(router));
        social = Some(domain);
        router
    })
    .await;
    Ctx { app, social: social.expect("register ran") }
}

impl Ctx {
    /// Calls a `FriendService` RPC.
    pub async fn friends<Req: serde::Serialize, Resp: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        user: &TestUser,
        request: &Req,
    ) -> Result<Resp, tokuly_testkit::RpcError> {
        self.app.call(&format!("{FRIENDS}/{method}"), Some(user), request).await
    }

    /// Calls an `InteractionService` RPC.
    pub async fn interact<Req: serde::Serialize, Resp: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        user: &TestUser,
        request: &Req,
    ) -> Result<Resp, tokuly_testkit::RpcError> {
        self.app.call(&format!("{INTERACTIONS}/{method}"), Some(user), request).await
    }

    /// Sends a friend request from `from` to `to`.
    pub async fn request(&self, from: &TestUser, to: &TestUser) -> SendFriendRequestResponse {
        self.friends("SendFriendRequest", from, &request_to(to.id)).await.expect("send friend request")
    }

    /// Makes two users friends through the API: `a` asks and `b` accepts
    /// (or, if `b` had already asked, `a`'s request completes it).
    pub async fn befriend(&self, a: &TestUser, b: &TestUser) {
        let sent = self.request(a, b).await;
        let Some(request) = sent.request.as_option() else {
            assert_eq!(sent.relationship, tokuly_proto::identity::Relationship::RELATIONSHIP_FRIENDS);
            return;
        };
        let _: RespondFriendRequestResponse = self
            .friends("RespondFriendRequest", b, &RespondFriendRequestRequest { request_id: request.id.clone(), accept: true, ..Default::default() })
            .await
            .expect("accept");
    }

    /// Changes one user's privacy settings through the identity API.
    pub async fn set_privacy(&self, user: &TestUser, change: impl FnOnce(&mut PrivacySettings)) {
        let mut privacy = settings::privacy_of(&self.app.core, user.id).await.expect("privacy");
        change(&mut privacy);
        let _: UpdateSettingsResponse = self
            .app
            .call("tokuly.identity.v1.AccountService/UpdateSettings", Some(user), &UpdateSettingsRequest { privacy: privacy.into(), ..Default::default() })
            .await
            .expect("update settings");
    }

    /// Creates `count` bare accounts straight in the database, for tests
    /// that need a crowd rather than people who do things.
    pub async fn crowd(&self, count: i32) -> Vec<Uuid> {
        let prefix = format!("crowd{}_", tokuly_core::ids::random_base62(8).to_ascii_lowercase());
        sqlx::query_scalar("INSERT INTO users (handle, display_name) SELECT ($1 || n)::citext, 'Crowd' FROM generate_series(1, $2) AS n RETURNING id")
            .bind(prefix)
            .bind(count)
            .fetch_all(&self.app.core.db)
            .await
            .expect("insert crowd")
    }

    /// Notification requests (`notify.requested`) addressed to `user`, by kind.
    pub async fn notifications_for(&self, user: Uuid, kind: &str) -> Vec<Event> {
        self.app
            .events("notify.requested")
            .await
            .into_iter()
            .filter(|e| e.data["user_id"] == serde_json::json!(user) && e.data["kind"] == serde_json::json!(kind))
            .collect()
    }

    /// Activity entries (`activity.recorded`) of `user`, by kind.
    pub async fn activities_of(&self, user: Uuid, kind: &str) -> Vec<Event> {
        self.app.events_by("activity.recorded", user).await.into_iter().filter(|e| e.data["kind"] == serde_json::json!(kind)).collect()
    }
}

/// The pending requests to (or, for `OUTGOING`, from) a user.
pub async fn list_requests(ctx: &Ctx, user: &TestUser, direction: FriendRequestDirection) -> ListFriendRequestsResponse {
    ctx.friends("ListFriendRequests", user, &ListFriendRequestsRequest { direction: direction.into(), ..Default::default() }).await.unwrap()
}

pub fn request_to(user_id: Uuid) -> SendFriendRequestRequest {
    SendFriendRequestRequest {
        target: Some(send_friend_request_request::Target::UserId(user_id.to_string())),
        source: FriendSource::FRIEND_SOURCE_PROFILE.into(),
        ..Default::default()
    }
}

pub fn id_of(user: &tokuly_proto::common::UserSummary) -> Uuid {
    user.id.parse().expect("user id")
}

/// The next social event pushed to a listener (other frames are skipped).
pub async fn next_push(listener: &mut FrameListener) -> Option<Push> {
    listener.next_event::<SocialEvent>(SOON).await?.event
}

/// Every social event pushed to a listener within `window`.
pub async fn pushes(listener: &mut FrameListener, window: Duration) -> Vec<Push> {
    listener.events::<SocialEvent>(window).await.into_iter().filter_map(|event| event.event).collect()
}

/// The next presence event pushed to a listener (other frames are skipped).
pub async fn next_presence(listener: &mut FrameListener) -> Option<PresenceEvent> {
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let left = deadline.checked_duration_since(tokio::time::Instant::now())?;
        match listener.next(left).await?.body {
            Some(Body::Presence(event)) => return Some(*event),
            _ => continue,
        }
    }
}

/// Every presence event pushed to a listener within `window`.
pub async fn presence_events(listener: &mut FrameListener, window: Duration) -> Vec<PresenceEvent> {
    listener
        .drain(window)
        .await
        .into_iter()
        .filter_map(|frame| match frame.body {
            Some(Body::Presence(event)) => Some(*event),
            _ => None,
        })
        .collect()
}

/// A random point on land-ish latitudes. Valkey is shared with other test
/// runs, so every test that uses positions picks its own corner of the world.
pub fn somewhere() -> (f64, f64) {
    let mut rng = rand::rng();
    (rng.random_range(-60.0..60.0), rng.random_range(-170.0..170.0))
}

/// The point `meters` north of `(lat, lng)`.
pub fn north_of((lat, lng): (f64, f64), meters: f64) -> (f64, f64) {
    (lat + meters / 111_195.0, lng)
}

pub fn lat_lng((lat, lng): (f64, f64)) -> LatLng {
    LatLng { lat, lng, ..Default::default() }
}
