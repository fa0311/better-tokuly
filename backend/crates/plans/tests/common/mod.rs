//! Shared setup and helpers for the plans integration tests.
#![allow(dead_code)] // each test binary uses its own subset

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Serialize, de::DeserializeOwned};
use tokuly_core::events::Event;
use tokuly_plans::{Plans, PlansConfig, WorkDone};
use tokuly_proto::{
    Timestamp,
    identity::{GetProfileRequest, GetProfileResponse, ProfileCard, get_profile_request::By},
    plans::{plans_event::Event as Push, *},
    social::*,
};
use tokuly_social::{Social, SocialConfig};
use tokuly_testkit::{FrameListener, RpcError, TestApp, TestUser};
use uuid::Uuid;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0200", "0400", "2000"];

pub const FREE: &str = "tokuly.plans.v1.FreeService";
pub const PLANS: &str = "tokuly.plans.v1.PlanService";
pub const POLLS: &str = "tokuly.plans.v1.PollService";
pub const ANNIVERSARIES: &str = "tokuly.plans.v1.AnniversaryService";
pub const CALENDAR: &str = "tokuly.plans.v1.CalendarService";
pub const FRIENDS: &str = "tokuly.social.v1.FriendService";

/// How long to wait for a realtime frame that should arrive (generous: the
/// machine may be busy compiling something else).
pub const SOON: Duration = Duration::from_secs(5);
/// How long to listen when asserting that nothing arrives.
pub const QUIET: Duration = Duration::from_millis(300);

/// The tests of one binary run side by side on one database, and a worker
/// pass handles whatever is due — whoever made it due. Passes are therefore
/// run one at a time, together with the statement that makes something due
/// (see [`Ctx::make_due`]), so every test's pass finds exactly its own work.
static WORKER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A running test app plus the domain under test (for its worker entry points).
pub struct Ctx {
    pub app: TestApp,
    pub plans: Plans,
    pub social: Social,
}

pub async fn start() -> Ctx {
    start_with(PlansConfig::default()).await
}

pub async fn start_with(cfg: PlansConfig) -> Ctx {
    let mut domains = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        let social = Social::new(core.clone(), identity.clone(), SocialConfig::default()).expect("social config");
        let plans = Plans::new(core.clone(), identity.clone(), cfg, social.clone()).expect("plans config");
        let router = plans.register(social.register(identity.register(router)));
        domains = Some((plans, social));
        router
    })
    .await;
    let (plans, social) = domains.expect("register ran");
    Ctx { app, plans, social }
}

impl Ctx {
    /// Calls an RPC of `service` as `user`.
    pub async fn call<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        service: &str,
        method: &str,
        user: &TestUser,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.app.call(&format!("{service}/{method}"), Some(user), request).await
    }

    /// The reason an RPC is refused with.
    pub async fn refusal<Req: Serialize>(&self, service: &str, method: &str, user: &TestUser, request: &Req) -> String {
        self.call::<_, serde_json::Value>(service, method, user, request)
            .await
            .expect_err("the call should be refused")
            .reason
    }

    /// Makes two users friends through the social API.
    pub async fn befriend(&self, a: &TestUser, b: &TestUser) {
        let request = SendFriendRequestRequest {
            target: Some(send_friend_request_request::Target::UserId(b.id.to_string())),
            source: FriendSource::FRIEND_SOURCE_PROFILE.into(),
            ..Default::default()
        };
        let sent: SendFriendRequestResponse =
            self.call(FRIENDS, "SendFriendRequest", a, &request).await.expect("send friend request");
        let request = sent.request.as_option().expect("a pending request");
        let _: RespondFriendRequestResponse = self
            .call(
                FRIENDS,
                "RespondFriendRequest",
                b,
                &RespondFriendRequestRequest { request_id: request.id.clone(), accept: true, ..Default::default() },
            )
            .await
            .expect("accept");
    }

    /// A user with `count` friends, all friends of the first one only.
    pub async fn group(&self, names: &[&str]) -> Vec<TestUser> {
        let mut users = Vec::new();
        for name in names {
            users.push(self.app.user(name).await);
        }
        for friend in &users[1..] {
            self.befriend(&users[0], friend).await;
        }
        users
    }

    /// `user` stars `friend`, which makes them one of `user`'s best friends.
    pub async fn star(&self, user: &TestUser, friend: &TestUser) {
        let _: UpdateFriendPrefsResponse = self
            .call(
                FRIENDS,
                "UpdateFriendPrefs",
                user,
                &UpdateFriendPrefsRequest {
                    user_id: friend.id.to_string(),
                    favorite: Some(true),
                    ..Default::default()
                },
            )
            .await
            .expect("star a friend");
    }

    pub async fn unfriend(&self, user: &TestUser, friend: &TestUser) {
        let _: RemoveFriendResponse = self
            .call(
                FRIENDS,
                "RemoveFriend",
                user,
                &RemoveFriendRequest { user_id: friend.id.to_string(), ..Default::default() },
            )
            .await
            .expect("remove friend");
    }

    pub async fn block(&self, user: &TestUser, other: &TestUser) {
        let _: BlockUserResponse = self
            .call(FRIENDS, "BlockUser", user, &BlockUserRequest { user_id: other.id.to_string(), ..Default::default() })
            .await
            .expect("block");
    }

    /// The cards this domain put on `owner`'s profile as `viewer` sees it.
    pub async fn cards(&self, viewer: &TestUser, owner: &TestUser) -> Vec<ProfileCard> {
        let request = GetProfileRequest { by: Some(By::UserId(owner.id.to_string())), ..Default::default() };
        let response: GetProfileResponse =
            self.call("tokuly.identity.v1.ProfileService", "GetProfile", viewer, &request).await.expect("get profile");
        response
            .profile
            .as_option()
            .expect("a profile")
            .cards
            .iter()
            .filter(|card| card.kind.starts_with("plans."))
            .cloned()
            .collect()
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
        self.app
            .events_by("activity.recorded", user)
            .await
            .into_iter()
            .filter(|e| e.data["kind"] == serde_json::json!(kind))
            .collect()
    }

    /// Runs a statement straight on the database.
    pub async fn sql(&self, statement: &'static str, id: Uuid) {
        sqlx::query(statement).bind(id).execute(&self.app.core.db).await.expect("test SQL");
    }

    /// Runs one pass of the worker.
    pub async fn work(&self) -> WorkDone {
        let _one_at_a_time = WORKER.lock().await;
        self.plans.run_due_work().await.expect("worker pass")
    }

    /// Moves time forward for the rows `ids` (by running `statement` for
    /// each: a test cannot wait for tomorrow) and runs one pass of the worker.
    pub async fn make_due(&self, statement: &'static str, ids: &[Uuid]) -> WorkDone {
        let _one_at_a_time = WORKER.lock().await;
        for id in ids {
            self.sql(statement, *id).await;
        }
        self.plans.run_due_work().await.expect("worker pass")
    }

    /// [`Ctx::make_due`] with several passes of the worker running at the
    /// same moment, as when several server processes are up.
    pub async fn make_due_for_workers(&self, statement: &'static str, ids: &[Uuid], workers: usize) -> Vec<WorkDone> {
        let _one_at_a_time = WORKER.lock().await;
        for id in ids {
            self.sql(statement, *id).await;
        }
        futures::future::join_all((0..workers).map(|_| self.plans.run_due_work()))
            .await
            .into_iter()
            .map(|pass| pass.expect("worker pass"))
            .collect()
    }

    pub async fn set_time_zone(&self, user: &TestUser, zone: &str) {
        sqlx::query("UPDATE users SET time_zone = $2 WHERE id = $1")
            .bind(user.id)
            .bind(zone)
            .execute(&self.app.core.db)
            .await
            .expect("set time zone");
    }

    pub async fn set_birthdate(&self, user: &TestUser, date: chrono::NaiveDate) {
        sqlx::query("UPDATE users SET birthdate = $2 WHERE id = $1")
            .bind(user.id)
            .bind(date)
            .execute(&self.app.core.db)
            .await
            .expect("set birthdate");
    }

    // ── plans ──

    pub async fn create_plan(&self, host: &TestUser, request: CreatePlanRequest) -> Plan {
        let response: CreatePlanResponse = self.call(PLANS, "CreatePlan", host, &request).await.expect("create plan");
        response.plan.as_option().cloned().expect("a plan")
    }

    pub async fn get_plan(&self, user: &TestUser, plan_id: &str) -> Plan {
        let response: GetPlanResponse = self
            .call(PLANS, "GetPlan", user, &GetPlanRequest { plan_id: plan_id.to_owned(), ..Default::default() })
            .await
            .expect("get plan");
        response.plan.as_option().cloned().expect("a plan")
    }

    /// RSVP; returns what the user ended up as.
    pub async fn rsvp(&self, user: &TestUser, plan_id: &str, rsvp: Rsvp) -> Rsvp {
        let request = RespondToPlanRequest { plan_id: plan_id.to_owned(), rsvp: rsvp.into(), ..Default::default() };
        let response: RespondToPlanResponse =
            self.call(PLANS, "RespondToPlan", user, &request).await.expect("respond to plan");
        response.rsvp.as_known().expect("a known answer")
    }

    // ── polls ──

    pub async fn create_poll(&self, creator: &TestUser, request: CreatePollRequest) -> Poll {
        let response: CreatePollResponse =
            self.call(POLLS, "CreatePoll", creator, &request).await.expect("create poll");
        response.poll.as_option().cloned().expect("a poll")
    }

    pub async fn get_poll(&self, user: &TestUser, poll_id: &str) -> Poll {
        let response: GetPollResponse = self
            .call(POLLS, "GetPoll", user, &GetPollRequest { poll_id: poll_id.to_owned(), ..Default::default() })
            .await
            .expect("get poll");
        response.poll.as_option().cloned().expect("a poll")
    }

    pub async fn answer_poll(&self, user: &TestUser, poll: &Poll, answers: &[PollAnswer]) -> Poll {
        let answers = poll
            .slots
            .iter()
            .zip(answers)
            .map(|(slot, answer)| SlotAnswer {
                slot_id: slot.id.clone(),
                answer: (*answer).into(),
                ..Default::default()
            })
            .collect();
        let response: AnswerPollResponse = self
            .call(
                POLLS,
                "AnswerPoll",
                user,
                &AnswerPollRequest { poll_id: poll.id.clone(), answers, ..Default::default() },
            )
            .await
            .expect("answer poll");
        response.poll.as_option().cloned().expect("a poll")
    }
}

/// A plan request with a title, a start time and invitees.
pub fn plan_at(title: &str, start: DateTime<Utc>, invitees: &[&TestUser]) -> CreatePlanRequest {
    CreatePlanRequest {
        title: title.to_owned(),
        time: PlanTime {
            start: Some(plan_time::Start::StartAt(Box::new(start.into()))),
            time_zone: "Asia/Tokyo".into(),
            ..Default::default()
        }
        .into(),
        invitee_ids: ids(invitees),
        ..Default::default()
    }
}

pub fn ids(users: &[&TestUser]) -> Vec<String> {
    users.iter().map(|user| user.id.to_string()).collect()
}

pub fn timestamp(time: DateTime<Utc>) -> Timestamp {
    time.into()
}

pub fn time_of<P: tokuly_proto::buffa::ProtoBox<Timestamp>>(
    field: &tokuly_proto::buffa::MessageField<Timestamp, P>,
) -> DateTime<Utc> {
    let timestamp = field.as_option().expect("a timestamp");
    DateTime::from_timestamp(timestamp.seconds, timestamp.nanos as u32).expect("a valid time")
}

/// The participant entry of `user` in a plan.
pub fn participant<'a>(plan: &'a Plan, user: &TestUser) -> &'a PlanParticipant {
    plan.participants
        .iter()
        .find(|p| p.user.as_option().is_some_and(|u| u.id == user.id.to_string()))
        .expect("the user is in the plan")
}

/// The next plans event pushed to a listener (other frames are skipped).
pub async fn next_push(listener: &mut FrameListener) -> Option<Push> {
    listener.next_event::<PlansEvent>(SOON).await?.event
}

/// Every plans event pushed to a listener within `window`.
pub async fn pushes(listener: &mut FrameListener, window: Duration) -> Vec<Push> {
    listener.events::<PlansEvent>(window).await.into_iter().filter_map(|event| event.event).collect()
}
