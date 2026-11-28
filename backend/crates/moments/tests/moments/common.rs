//! Shared fixtures for the moments integration tests.
//!
//! All tests of the binary share a database and run at once, and a prompt
//! goes to a whole region. So every test puts its people into a region of
//! its own ([`Ctx::region`]: a region outside the bands, with a time zone of
//! its own mapped to it) and sends the prompts of that region only
//! ([`Ctx::prompt`]). The bands themselves are only drawn, for dates far
//! ahead, never prompted.
#![allow(dead_code)]

use std::time::Duration;

use serde::{Serialize, de::DeserializeOwned};
use tokuly_core::events::Event;
use tokuly_media::testing::ready_image;
use tokuly_moments::{Moments, MomentsConfig, Sent};
use tokuly_proto::{
    common::LatLng,
    media::AssetPurpose,
    moments::*,
    social::{
        AddCircleMembersRequest, AddCircleMembersResponse, BlockUserRequest, CircleKind, ListCirclesRequest,
        ListCirclesResponse, SendFriendRequestRequest, send_friend_request_request::Target,
    },
};
use tokuly_social::{Social, SocialConfig};
use tokuly_testkit::{RpcError, TestApp, TestUser};
use uuid::Uuid;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0200", "0400", "0500", "1400", "2500"];

/// How long to wait for a realtime frame that should arrive.
pub const SOON: Duration = Duration::from_secs(5);
/// How long to listen when asserting that nothing arrives.
pub const QUIET: Duration = Duration::from_millis(300);

pub struct Ctx {
    pub app: TestApp,
    pub moments: Moments,
}

pub async fn start() -> Ctx {
    start_with(MomentsConfig::default()).await
}

pub async fn start_with(cfg: MomentsConfig) -> Ctx {
    let mut domain = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        // Social is here for its RPCs: tests make friends and block people
        // the way clients do.
        let social = Social::new(core.clone(), identity.clone(), SocialConfig::default()).expect("social domain");
        let moments = Moments::new(core.clone(), identity.clone(), cfg).expect("moments domain");
        let router = moments.register(social.register(identity.register(router)));
        domain = Some(moments);
        router
    })
    .await;
    Ctx { app, moments: domain.expect("register ran") }
}

pub fn id(value: &str) -> Uuid {
    value.parse().expect("an ID")
}

impl Ctx {
    pub async fn call<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        method: &str,
        user: &TestUser,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        let service = if matches!(
            method,
            "React" | "RemoveReaction" | "ListReactions" | "ListRealmojis" | "SetRealmoji" | "DeleteRealmoji"
        ) {
            "MomentReactionService"
        } else {
            "MomentService"
        };
        self.app.call(&format!("tokuly.moments.v1.{service}/{method}"), Some(user), request).await
    }

    /// A region of the test's own, with `users` living in it. The region
    /// and its one time zone share the name.
    pub async fn region(&self, users: &[&TestUser]) -> String {
        let region = format!("test/{}", Uuid::now_v7().simple());
        let db = &self.app.core.db;
        sqlx::query("INSERT INTO moment_regions (region) VALUES ($1)").bind(&region).execute(db).await.expect("region");
        sqlx::query("INSERT INTO moment_zones (time_zone, region) VALUES ($1, $1)")
            .bind(&region)
            .execute(db)
            .await
            .expect("zone");
        self.move_to(&region, users).await;
        region
    }

    /// Moves `users` to the time zone of a region made by [`Self::region`].
    pub async fn move_to(&self, region: &str, users: &[&TestUser]) {
        let ids: Vec<Uuid> = users.iter().map(|user| user.id).collect();
        sqlx::query("UPDATE users SET time_zone = $2 WHERE id = ANY($1)")
            .bind(&ids)
            .bind(region)
            .execute(&self.app.core.db)
            .await
            .expect("move users");
    }

    /// A day of `region` due now (the date after its last one).
    pub async fn due_day(&self, region: &str) -> Uuid {
        let db = &self.app.core.db;
        sqlx::query_scalar(
            "INSERT INTO moment_days (region, date, prompt_at)
             SELECT $1, coalesce(max(date) + 1, DATE '2026-01-01'), now() - interval '1 second'
             FROM moment_days WHERE region = $1
             RETURNING id",
        )
        .bind(region)
        .fetch_one(db)
        .await
        .expect("day")
    }

    /// Sends `region` its next prompt now; returns the day.
    pub async fn prompt(&self, region: &str) -> Uuid {
        let day = self.due_day(region).await;
        assert!(matches!(self.moments.send_prompt(day).await.unwrap(), Sent::Told(_)));
        day
    }

    /// Moves a day's prompt into the past, as if it had gone out `seconds` ago.
    pub async fn age_day(&self, day: Uuid, seconds: i64) {
        sqlx::query(
            "UPDATE moment_days SET announced_at = now() - make_interval(secs => $2),
                                    prompt_at = now() - make_interval(secs => $2)
             WHERE id = $1",
        )
        .bind(day)
        .bind(seconds as f64)
        .execute(&self.app.core.db)
        .await
        .expect("age day");
    }

    pub async fn picture(&self, user: &TestUser, purpose: AssetPurpose) -> String {
        ready_image(&self.app.core, user.id, purpose).await.expect("picture").id.to_string()
    }

    pub async fn pair(&self, user: &TestUser) -> PostMomentRequest {
        PostMomentRequest {
            back_asset_id: self.picture(user, AssetPurpose::ASSET_PURPOSE_MOMENT).await,
            front_asset_id: self.picture(user, AssetPurpose::ASSET_PURPOSE_MOMENT).await,
            caption: "放課後🍦".into(),
            location: Some(LatLng { lat: 35.6595, lng: 139.7005, ..Default::default() }).into(),
            retakes: 2,
            ..Default::default()
        }
    }

    pub async fn try_post(&self, user: &TestUser) -> Result<Moment, RpcError> {
        let request = self.pair(user).await;
        let posted: PostMomentResponse = self.call("PostMoment", user, &request).await?;
        Ok(posted.moment.as_option().expect("a moment").clone())
    }

    pub async fn post(&self, user: &TestUser) -> Moment {
        self.try_post(user).await.expect("post moment")
    }

    pub async fn friend_moments(&self, user: &TestUser) -> ListFriendMomentsResponse {
        self.call("ListFriendMoments", user, &ListFriendMomentsRequest::default()).await.expect("friend moments")
    }

    pub async fn get(&self, user: &TestUser, moment_id: &str) -> Result<Moment, RpcError> {
        let request = GetMomentRequest { moment_id: moment_id.to_owned(), ..Default::default() };
        let got: GetMomentResponse = self.call("GetMoment", user, &request).await?;
        Ok(got.moment.as_option().expect("a moment").clone())
    }

    pub async fn today(&self, user: &TestUser) -> GetTodayResponse {
        self.call("GetToday", user, &GetTodayRequest::default()).await.expect("today")
    }

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

    /// New users who are all friends of the first.
    pub async fn circle_of(&self, names: &[&str]) -> Vec<TestUser> {
        let mut users = Vec::new();
        for name in names {
            users.push(self.app.user(name).await);
        }
        for other in &users[1..] {
            self.befriend(&users[0], other).await;
        }
        users
    }

    pub async fn block(&self, blocker: &TestUser, blocked: &TestUser) {
        let request = BlockUserRequest { user_id: blocked.id.to_string(), ..Default::default() };
        let _: serde_json::Value =
            self.app.call("tokuly.social.v1.FriendService/BlockUser", Some(blocker), &request).await.expect("block");
    }

    pub async fn add_close_friends(&self, owner: &TestUser, members: &[&TestUser]) {
        let circles: ListCirclesResponse = self
            .app
            .call("tokuly.social.v1.CircleService/ListCircles", Some(owner), &ListCirclesRequest::default())
            .await
            .expect("list circles");
        let circle_id = circles
            .circles
            .iter()
            .find(|circle| circle.kind == buffa::EnumValue::from(CircleKind::CIRCLE_KIND_CLOSE_FRIENDS))
            .map(|circle| circle.id.clone())
            .expect("a close-friends circle");
        let request = AddCircleMembersRequest {
            circle_id,
            user_ids: members.iter().map(|member| member.id.to_string()).collect(),
            ..Default::default()
        };
        let _: AddCircleMembersResponse = self
            .app
            .call("tokuly.social.v1.CircleService/AddCircleMembers", Some(owner), &request)
            .await
            .expect("add close friends");
    }

    /// The notifications of `kind` requested for `user`.
    pub async fn notifications(&self, user: &TestUser, kind: &str) -> Vec<serde_json::Value> {
        self.app
            .events("notify.requested")
            .await
            .into_iter()
            .map(|event| event.data)
            .filter(|data| data["user_id"] == user.id.to_string() && data["kind"] == kind)
            .collect()
    }

    /// Delivers an event to the domain's consumer by hand.
    pub async fn deliver(&self, topic: &str, data: serde_json::Value) {
        let event =
            Event { id: Uuid::now_v7(), topic: topic.into(), actor_id: None, occurred_at: chrono::Utc::now(), data };
        self.moments.handle_event(&event).await.expect("handle event");
    }

    pub async fn holders(&self, asset_id: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM media_asset_holders WHERE asset_id = $1")
            .bind(id(asset_id))
            .fetch_one(&self.app.core.db)
            .await
            .expect("holders")
    }
}
