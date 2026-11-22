//! Shared fixtures for the school integration tests.
//!
//! Every test starts its own app ([`start`]) against the test binary's
//! database and creates its own users. Each app has its own school domain
//! and therefore its own clock: tests stop the clock where the story needs
//! it ([`Ctx::at`]) and move it forward ([`Ctx::pass`]) instead of waiting.
//!
//! The sweeps (`School::sweep` and its steps) act on the whole database, so
//! only `tests/sweep.rs` runs them, one test at a time.
#![allow(dead_code)] // each test binary uses its own subset

use std::time::Duration;

use chrono::{DateTime, TimeZone as _, Utc};
use serde::{Serialize, de::DeserializeOwned};
use tokuly_core::events::Event;
use tokuly_proto::{
    identity::{GetProfileRequest, GetProfileResponse, ProfileCard, get_profile_request},
    realtime::{ServerFrame, server_frame::Body},
    school::{school_event::Event as Push, *},
    social::{
        BlockUserRequest, BlockUserResponse, FriendSource, RemoveFriendRequest, RemoveFriendResponse,
        RespondFriendRequestRequest, RespondFriendRequestResponse, SendFriendRequestRequest, SendFriendRequestResponse,
        send_friend_request_request::Target,
    },
};
use tokuly_school::{School, SchoolConfig};
use tokuly_social::{Social, SocialConfig};
use tokuly_testkit::{FrameListener, RpcError, TestApp, TestUser};
use uuid::Uuid;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0200", "0400", "2100"];

pub const SETTINGS: &str = "tokuly.school.v1.SchoolService";
pub const TIMETABLE: &str = "tokuly.school.v1.TimetableService";
pub const STUDY: &str = "tokuly.school.v1.StudyService";
pub const ROOMS: &str = "tokuly.school.v1.StudyRoomService";
pub const EXAMS: &str = "tokuly.school.v1.ExamService";
pub const ASSIGNMENTS: &str = "tokuly.school.v1.AssignmentService";

/// How long to wait for a realtime frame that should arrive (generous: the
/// machine may be busy compiling something else).
pub const SOON: Duration = Duration::from_secs(5);
/// How long to listen when collecting the frames an action produced, or
/// when asserting that nothing arrives.
pub const QUIET: Duration = Duration::from_millis(400);

/// A running app plus the domain behind it, for its clock, its sweeps and
/// delivering events by hand.
pub struct Ctx {
    pub app: TestApp,
    pub school: School,
}

pub async fn start() -> Ctx {
    start_with(SchoolConfig::default()).await
}

pub async fn start_with(cfg: SchoolConfig) -> Ctx {
    let mut domain = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        // The social domain is here for its RPCs: tests make friends and
        // block people the way clients do. It goes first, as in the server,
        // so profiles carry the relationship before the school cards.
        let social = Social::new(core.clone(), identity.clone(), SocialConfig::default()).expect("social domain");
        let school = School::new(core.clone(), identity.clone(), cfg).expect("school domain");
        let router = school.register(social.register(identity.register(router)));
        domain = Some(school);
        router
    })
    .await;
    Ctx { app, school: domain.expect("register ran") }
}

/// An instant given as Tokyo wall-clock time (the default time zone of accounts).
pub fn tokyo(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    chrono_tz::Asia::Tokyo.with_ymd_and_hms(year, month, day, hour, minute, 0).unwrap().to_utc()
}

pub fn id(value: &str) -> Uuid {
    value.parse().expect("an ID")
}

/// The school events among some realtime frames, in order.
pub fn pushes(frames: Vec<ServerFrame>) -> Vec<Push> {
    frames
        .into_iter()
        .filter_map(|frame| match frame.body {
            Some(Body::School(event)) => event.event,
            _ => None,
        })
        .collect()
}

/// The school events a listener receives within [`QUIET`].
pub async fn heard(listener: &mut FrameListener) -> Vec<Push> {
    pushes(listener.drain(QUIET).await)
}

/// What kind of school event a push is, for asserting on what was heard.
pub fn kind(push: &Push) -> &'static str {
    match push {
        Push::Session(_) => "session",
        Push::FriendStudying(_) => "friend_studying",
        Push::Cheer(_) => "cheer",
        Push::RoomInvitation(_) => "room_invitation",
        Push::RoomLeft(_) => "room_left",
        Push::RoomTimer(_) => "room_timer",
        Push::RoomMemberJoined(_) => "room_member_joined",
        Push::RoomMemberLeft(_) => "room_member_left",
        Push::RoomMemberUpdated(_) => "room_member_updated",
        Push::RoomReaction(_) => "room_reaction",
        Push::RoomUpdated(_) => "room_updated",
        Push::RoomClosed(_) => "room_closed",
    }
}

pub fn kinds(pushes: &[Push]) -> Vec<&'static str> {
    pushes.iter().map(kind).collect()
}

/// A class for `PutTimetableEntries`.
pub fn class(weekday: Weekday, period: i32, subject: &Subject) -> TimetableEntryInput {
    TimetableEntryInput { weekday: weekday.into(), period, subject_id: subject.id.clone(), ..Default::default() }
}

/// What a resolved day holds, in short: `(period, subject name, status)`.
pub fn classes(day: &TimetableDay) -> Vec<(i32, String, ClassStatus)> {
    day.classes
        .iter()
        .map(|class| {
            (
                class.period,
                class.subject.as_option().map(|s| s.name.clone()).unwrap_or_default(),
                class.status.as_known().unwrap(),
            )
        })
        .collect()
}

impl Ctx {
    /// Calls `<service>/<method>` as `user`.
    pub async fn call<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        service: &str,
        method: &str,
        user: &TestUser,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.app.call(&format!("{service}/{method}"), Some(user), request).await
    }

    /// Calls an RPC that must succeed.
    pub async fn ok<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        service: &str,
        method: &str,
        user: &TestUser,
        request: &Req,
    ) -> Resp {
        self.call(service, method, user, request).await.unwrap_or_else(|e| panic!("{service}/{method} failed: {e}"))
    }

    /// The reason an RPC is rejected with.
    pub async fn reason<Req: Serialize>(&self, service: &str, method: &str, user: &TestUser, request: &Req) -> String {
        match self.call::<_, serde_json::Value>(service, method, user, request).await {
            Ok(response) => panic!("{service}/{method} should fail, got {response}"),
            Err(error) => error.reason,
        }
    }

    /// Stops this app's clock at a Tokyo wall-clock time.
    pub fn at(&self, year: i32, month: u32, day: u32, hour: u32, minute: u32) {
        self.school.set_clock(tokyo(year, month, day, hour, minute));
    }

    /// Lets time pass on this app's clock.
    pub fn pass(&self, minutes: i64) {
        self.school.shift_clock(chrono::Duration::minutes(minutes));
    }

    pub fn pass_seconds(&self, seconds: i64) {
        self.school.shift_clock(chrono::Duration::seconds(seconds));
    }

    pub async fn set_time_zone(&self, user: &TestUser, time_zone: &str) {
        sqlx::query("UPDATE users SET time_zone = $2 WHERE id = $1")
            .bind(user.id)
            .bind(time_zone)
            .execute(&self.app.core.db)
            .await
            .expect("set time zone");
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

    /// `a` ends the friendship with `b`, and the school domain hears of it.
    pub async fn unfriend(&self, a: &TestUser, b: &TestUser) {
        let request = RemoveFriendRequest { user_id: b.id.to_string(), ..Default::default() };
        let _: RemoveFriendResponse = self
            .app
            .call("tokuly.social.v1.FriendService/RemoveFriend", Some(a), &request)
            .await
            .expect("remove friend");
        self.deliver("social.friend.removed", a.id).await;
    }

    /// `blocker` blocks `blocked`, and the school domain hears of it.
    pub async fn block(&self, blocker: &TestUser, blocked: &TestUser) {
        let request = BlockUserRequest { user_id: blocked.id.to_string(), ..Default::default() };
        let _: BlockUserResponse =
            self.app.call("tokuly.social.v1.FriendService/BlockUser", Some(blocker), &request).await.expect("block");
        self.deliver("social.friend.removed", blocker.id).await;
        self.deliver("social.user.blocked", blocker.id).await;
    }

    /// Hands the events `actor` caused under a topic to the school domain,
    /// as the event consumer would (no relay runs in tests).
    pub async fn deliver(&self, topic: &str, actor: Uuid) {
        for event in self.app.events_by(topic, actor).await {
            self.school.handle_event(&event).await.unwrap_or_else(|e| panic!("handling {topic} failed: {e}"));
        }
    }

    /// Hands a made-up event to the school domain.
    pub async fn deliver_event(&self, topic: &str, data: serde_json::Value) {
        let event =
            Event { id: Uuid::now_v7(), topic: topic.to_owned(), actor_id: None, occurred_at: Utc::now(), data };
        self.school.handle_event(&event).await.unwrap_or_else(|e| panic!("handling {topic} failed: {e}"));
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

    /// The cards of `owner`'s profile as `viewer` sees it, by kind.
    pub async fn profile_cards(&self, viewer: &TestUser, owner: &TestUser) -> Vec<ProfileCard> {
        let request =
            GetProfileRequest { by: Some(get_profile_request::By::UserId(owner.id.to_string())), ..Default::default() };
        let response: GetProfileResponse = self
            .app
            .call("tokuly.identity.v1.ProfileService/GetProfile", Some(viewer), &request)
            .await
            .expect("get profile");
        response.profile.as_option().expect("a profile").cards.clone()
    }

    pub async fn profile_card(&self, viewer: &TestUser, owner: &TestUser, kind: &str) -> Option<ProfileCard> {
        self.profile_cards(viewer, owner).await.into_iter().find(|card| card.kind == kind)
    }

    // ── timetable ──

    pub async fn subject(&self, user: &TestUser, name: &str) -> Subject {
        let response: CreateSubjectResponse = self
            .ok(TIMETABLE, "CreateSubject", user, &CreateSubjectRequest { name: name.to_owned(), ..Default::default() })
            .await;
        response.subject.as_option().cloned().expect("the subject")
    }

    /// Creates a term: by default the first term of 2026 (April 8 to July
    /// 20), six 50-minute periods, Monday to Friday, private.
    pub async fn term(&self, user: &TestUser, fill: impl FnOnce(&mut CreateTermRequest)) -> Term {
        let mut request = CreateTermRequest {
            name: "1学期".into(),
            start_date: "2026-04-08".into(),
            end_date: "2026-07-20".into(),
            ..Default::default()
        };
        fill(&mut request);
        let response: CreateTermResponse = self.ok(TIMETABLE, "CreateTerm", user, &request).await;
        response.term.as_option().cloned().expect("the term")
    }

    pub async fn put(
        &self,
        user: &TestUser,
        term: &Term,
        entries: Vec<TimetableEntryInput>,
    ) -> PutTimetableEntriesResponse {
        self.ok(
            TIMETABLE,
            "PutTimetableEntries",
            user,
            &PutTimetableEntriesRequest { term_id: term.id.clone(), entries, ..Default::default() },
        )
        .await
    }

    /// `owner`'s day as `viewer` sees it.
    pub async fn day_of(&self, viewer: &TestUser, owner: &TestUser, date: &str) -> Result<TimetableDay, RpcError> {
        let request = GetDayRequest {
            date: date.to_owned(),
            user_id: if viewer.id == owner.id { String::new() } else { owner.id.to_string() },
            ..Default::default()
        };
        let response: GetDayResponse = self.call(TIMETABLE, "GetDay", viewer, &request).await?;
        Ok(response.day.as_option().cloned().expect("the day"))
    }

    pub async fn day(&self, user: &TestUser, date: &str) -> TimetableDay {
        self.day_of(user, user, date).await.expect("own day")
    }

    // ── study ──

    pub async fn start_stopwatch(&self, user: &TestUser) -> StudySession {
        let request = StartStudySessionRequest { mode: StudyMode::STUDY_MODE_STOPWATCH.into(), ..Default::default() };
        let response: StartStudySessionResponse = self.ok(STUDY, "StartStudySession", user, &request).await;
        response.session.as_option().cloned().expect("the session")
    }

    pub async fn heartbeat(&self, user: &TestUser) -> StudySession {
        let response: HeartbeatStudySessionResponse =
            self.ok(STUDY, "HeartbeatStudySession", user, &HeartbeatStudySessionRequest::default()).await;
        response.session.as_option().cloned().expect("the session")
    }

    pub async fn stop(&self, user: &TestUser) -> StopStudySessionResponse {
        self.ok(STUDY, "StopStudySession", user, &StopStudySessionRequest::default()).await
    }

    /// Studies with the stopwatch for `minutes`, heartbeating every five.
    pub async fn study_for(&self, user: &TestUser, minutes: i64) -> StudySession {
        self.start_stopwatch(user).await;
        let mut left = minutes;
        while left > 5 {
            self.pass(5);
            self.heartbeat(user).await;
            left -= 5;
        }
        self.pass(left);
        self.stop(user).await.session.as_option().cloned().expect("the session")
    }

    /// Logs a finished session by hand between two instants.
    pub async fn log(
        &self,
        user: &TestUser,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        subject: Option<&Subject>,
    ) -> Result<StudySession, RpcError> {
        let request = LogStudySessionRequest {
            started_at: Some(tokuly_proto::Timestamp::from(from)).into(),
            ended_at: Some(tokuly_proto::Timestamp::from(to)).into(),
            subject_id: subject.map(|s| s.id.clone()).unwrap_or_default(),
            ..Default::default()
        };
        let response: LogStudySessionResponse = self.call(STUDY, "LogStudySession", user, &request).await?;
        Ok(response.session.as_option().cloned().expect("the session"))
    }

    pub async fn update_settings(&self, user: &TestUser, request: UpdateSchoolSettingsRequest) -> SchoolSettings {
        let response: UpdateSchoolSettingsResponse = self.ok(SETTINGS, "UpdateSchoolSettings", user, &request).await;
        response.settings.as_option().cloned().expect("the settings")
    }

    // ── rooms ──

    pub async fn create_room(
        &self,
        host: &TestUser,
        fill: impl FnOnce(&mut CreateStudyRoomRequest),
    ) -> CreateStudyRoomResponse {
        let mut request = CreateStudyRoomRequest { title: "もくもく".into(), ..Default::default() };
        fill(&mut request);
        self.ok(ROOMS, "CreateStudyRoom", host, &request).await
    }

    pub async fn join(&self, user: &TestUser, room_id: &str) -> Result<JoinStudyRoomResponse, RpcError> {
        self.call(
            ROOMS,
            "JoinStudyRoom",
            user,
            &JoinStudyRoomRequest { room_id: room_id.to_owned(), ..Default::default() },
        )
        .await
    }

    pub async fn room(&self, viewer: &TestUser, room_id: &str) -> Result<StudyRoom, RpcError> {
        let response: GetStudyRoomResponse = self
            .call(
                ROOMS,
                "GetStudyRoom",
                viewer,
                &GetStudyRoomRequest { room_id: room_id.to_owned(), ..Default::default() },
            )
            .await?;
        Ok(response.room.as_option().cloned().expect("the room"))
    }

    pub async fn control(
        &self,
        host: &TestUser,
        room_id: &str,
        action: TimerAction,
    ) -> Result<TimerSnapshot, RpcError> {
        let request =
            ControlStudyRoomTimerRequest { room_id: room_id.to_owned(), action: action.into(), ..Default::default() };
        let response: ControlStudyRoomTimerResponse = self.call(ROOMS, "ControlStudyRoomTimer", host, &request).await?;
        Ok(response.timer.as_option().cloned().expect("the timer"))
    }

    /// Heartbeats for several users at once (everyone in a room, say).
    pub async fn beat(&self, users: &[&TestUser]) {
        for user in users {
            self.heartbeat(user).await;
        }
    }

    /// The finished sessions of `user`, newest first.
    pub async fn sessions(&self, user: &TestUser) -> Vec<StudySession> {
        let response: ListStudySessionsResponse =
            self.ok(STUDY, "ListStudySessions", user, &ListStudySessionsRequest::default()).await;
        response.sessions
    }
}

/// A protobuf timestamp field for requests.
pub fn stamp<P: buffa::ProtoBox<tokuly_proto::Timestamp>>(
    at: DateTime<Utc>,
) -> buffa::MessageField<tokuly_proto::Timestamp, P> {
    buffa::MessageField::some(tokuly_proto::Timestamp::from(at))
}

/// The instant of a protobuf timestamp field.
pub fn instant<P: buffa::ProtoBox<tokuly_proto::Timestamp>>(
    field: &buffa::MessageField<tokuly_proto::Timestamp, P>,
) -> DateTime<Utc> {
    let ts = field.as_option().expect("a timestamp");
    DateTime::from_timestamp(ts.seconds, ts.nanos as u32).expect("a valid timestamp")
}
