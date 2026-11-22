//! Shared setup for the quest integration tests.
#![allow(dead_code)] // each module uses its own subset

use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use serde_json::json;
use tokuly_core::{analytics::AnalyticsEvent, events::Event, prelude::*};
use tokuly_proto::{
    quest::{quest_event::Event as Push, *},
    realtime::server_frame::Body,
    social::{
        FriendSource, RemoveFriendRequest, RemoveFriendResponse, RespondFriendRequestRequest,
        RespondFriendRequestResponse, SendFriendRequestRequest, SendFriendRequestResponse, send_friend_request_request,
    },
};
use tokuly_quest::{Quest, QuestConfig};
use tokuly_social::{Social, SocialConfig};
use tokuly_testkit::{FrameListener, RpcError, TestApp, TestUser};
use tokuly_wallet::ledger;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0200", "0400", "1400", "1900"];

pub const QUEST: &str = "tokuly.quest.v1.QuestService";
pub const MISSIONS: &str = "tokuly.quest.v1.MissionService";
pub const BADGES: &str = "tokuly.quest.v1.BadgeService";
pub const STREAKS: &str = "tokuly.quest.v1.StreakService";
pub const BOARDS: &str = "tokuly.quest.v1.LeaderboardService";
pub const ADMIN: &str = "tokuly.quest.v1.QuestAdminService";
const FRIENDS: &str = "tokuly.social.v1.FriendService";

/// How long to listen for realtime frames (generous: the machine may be busy).
pub const SOON: Duration = Duration::from_millis(1500);

/// A running test app plus the domain under test (for its worker entry points).
pub struct Ctx {
    pub app: TestApp,
    pub quest: Quest,
}

pub async fn start() -> Ctx {
    start_with(QuestConfig::default()).await
}

pub async fn start_with(cfg: QuestConfig) -> Ctx {
    let mut quest = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        let social = Social::new(core.clone(), identity.clone(), SocialConfig::default()).expect("social");
        let domain = Quest::new(core.clone(), identity.clone(), cfg).expect("quest config");
        let router = domain.register(social.register(identity.register(router)));
        quest = Some(domain);
        router
    })
    .await;
    Ctx { app, quest: quest.expect("register ran") }
}

/// A domain event as the consumer would hand it over, happening now.
pub fn event(topic: &str, actor: Option<Uuid>, data: serde_json::Value) -> Event {
    event_at(topic, actor, data, Utc::now())
}

pub fn event_at(topic: &str, actor: Option<Uuid>, data: serde_json::Value, at: DateTime<Utc>) -> Event {
    Event { id: Uuid::now_v7(), topic: topic.to_owned(), actor_id: actor, occurred_at: at, data }
}

/// The date it is for a user living in `zone`.
pub fn today_in(zone: &str) -> NaiveDate {
    Utc::now().with_timezone(&zone.parse::<chrono_tz::Tz>().unwrap()).date_naive()
}

/// The date it is in the zone test users get (Asia/Tokyo).
pub fn today() -> NaiveDate {
    today_in("Asia/Tokyo")
}

/// A fixed-offset zone in which it is ten in the evening right now: two
/// hours (give or take the minutes) before midnight.
pub fn evening_zone() -> String {
    use chrono::Timelike as _;
    let utc_hour = Utc::now().hour() as i32;
    // Hours to add to UTC to read 22:xx, folded into the range zones exist for.
    let offset = (22 - utc_hour + 12).rem_euclid(24) - 12;
    // POSIX-style names have the sign the wrong way round: Etc/GMT-9 is UTC+9.
    if offset >= 0 { format!("Etc/GMT-{offset}") } else { format!("Etc/GMT+{}", -offset) }
}

impl Ctx {
    /// Stores a running streak for two friends living in `zone`: `days`
    /// long, complete up to yesterday, with today's part done by `done_by`.
    pub async fn running_today(&self, a: &TestUser, b: &TestUser, zone: &str, days: i32, done_by: Option<&TestUser>) {
        for user in [a, b] {
            self.set_zone(user, zone).await;
        }
        let (lo, hi) = if a.id < b.id { (a.id, b.id) } else { (b.id, a.id) };
        let today = today_in(zone);
        let tz: chrono_tz::Tz = zone.parse().unwrap();
        let midnight = chrono::TimeZone::from_local_datetime(
            &tz,
            &(today + chrono::Duration::days(1)).and_hms_opt(0, 0, 0).unwrap(),
        )
        .unwrap()
        .with_timezone(&Utc);
        let done: Vec<NaiveDate> = vec![today];
        let none: Vec<NaiveDate> = Vec::new();
        sqlx::query("INSERT INTO quest_streaks (user_lo, user_hi, count, best, settled_day, started_on, lo_days, hi_days, expires_at) VALUES ($1, $2, $3, $3, $4, $4 - ($3 - 1), $5, $6, $7)")
            .bind(lo)
            .bind(hi)
            .bind(days)
            .bind(today - chrono::Duration::days(1))
            .bind(if done_by.is_some_and(|user| user.id == lo) { &done } else { &none })
            .bind(if done_by.is_some_and(|user| user.id == hi) { &done } else { &none })
            .bind(midnight)
            .execute(&self.app.core.db)
            .await
            .unwrap();
    }

    /// Replaces the player's missions with a set the tests know: today send
    /// 5 stamps, post a note, give 3 likes; this week send 30 stamps.
    pub async fn known_set(&self, user: &TestUser) {
        // Drawn first, so the day's rows exist the way the domain makes them.
        self.missions(user).await;
        let day = today();
        let week = day - chrono::Duration::days(chrono::Datelike::weekday(&day).num_days_from_monday() as i64);
        sqlx::query("DELETE FROM quest_missions WHERE user_id = $1")
            .bind(user.id)
            .execute(&self.app.core.db)
            .await
            .unwrap();
        for (period, start, slot, key) in [
            (1_i16, day, 0_i16, "stamps_5"),
            (1, day, 1, "note_1"),
            (1, day, 2, "likes_3"),
            (2, week, 0, "w_stamps_30"),
        ] {
            let mission = tokuly_quest::missions::pool::find(key).unwrap();
            let payout = mission.payout();
            sqlx::query("INSERT INTO quest_missions (user_id, period, period_start, slot, mission_key, metric, target, reward_coins, reward_xp) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)")
                .bind(user.id)
                .bind(period)
                .bind(start)
                .bind(slot)
                .bind(key)
                .bind(mission.metric.key())
                .bind(mission.target)
                .bind(payout.coins)
                .bind(payout.xp)
                .execute(&self.app.core.db)
                .await
                .unwrap();
        }
    }

    /// `user` posts a public note, as the notes domain announces it.
    pub async fn post_note(&self, user: &TestUser) {
        self.feed(event("notes.note.created", Some(user.id), json!({ "note_id": Uuid::now_v7(), "author_id": user.id, "visibility": 1, "reply_to_id": null, "quote_of_id": null }))).await;
    }

    /// `user` likes `count` notes of other people, as the notes domain tracks it.
    pub async fn like_notes(&self, user: &TestUser, count: usize) {
        for _ in 0..count {
            self.track(
                AnalyticsEvent::new("note.like")
                    .user(Some(user.id))
                    .subject("note", Uuid::now_v7())
                    .owner(Uuid::now_v7()),
            )
            .await;
        }
    }

    pub async fn claim(&self, user: &TestUser, mission_id: &str) -> Result<ClaimMissionResponse, RpcError> {
        self.call(
            MISSIONS,
            "ClaimMission",
            Some(user),
            &ClaimMissionRequest { mission_id: mission_id.to_owned(), ..Default::default() },
        )
        .await
    }

    pub async fn reroll(&self, user: &TestUser, mission_id: &str) -> Result<RerollMissionResponse, RpcError> {
        self.call(
            MISSIONS,
            "RerollMission",
            Some(user),
            &RerollMissionRequest { mission_id: mission_id.to_owned(), ..Default::default() },
        )
        .await
    }

    pub async fn restore(&self, user: &TestUser, friend: &TestUser) -> Result<RestoreStreakResponse, RpcError> {
        self.call(
            STREAKS,
            "RestoreStreak",
            Some(user),
            &RestoreStreakRequest { user_id: friend.id.to_string(), ..Default::default() },
        )
        .await
    }

    pub async fn call<Req: serde::Serialize, Resp: serde::de::DeserializeOwned>(
        &self,
        service: &str,
        method: &str,
        user: Option<&TestUser>,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.app.call(&format!("{service}/{method}"), user, request).await
    }

    /// Feeds a domain event to the domain.
    pub async fn feed(&self, event: Event) {
        self.quest.handle_event(event).await.expect("event handled");
    }

    /// Feeds an analytics event to the domain.
    pub async fn track(&self, tracked: AnalyticsEvent) {
        self.quest.handle_analytics(tracked).await.expect("analytics event handled");
    }

    /// Writes what was collected for these players, as the flush worker would.
    pub async fn flush(&self, users: &[&TestUser]) {
        for user in users {
            self.quest.flush_user(user.id).await.expect("flush");
        }
    }

    /// Makes two users friends through the social API.
    pub async fn befriend(&self, a: &TestUser, b: &TestUser) {
        let request = SendFriendRequestRequest {
            target: Some(send_friend_request_request::Target::UserId(b.id.to_string())),
            source: FriendSource::FRIEND_SOURCE_PROFILE.into(),
            ..Default::default()
        };
        let sent: SendFriendRequestResponse = self
            .app
            .call(&format!("{FRIENDS}/SendFriendRequest"), Some(a), &request)
            .await
            .expect("send friend request");
        let Some(request) = sent.request.as_option() else { return };
        let accept = RespondFriendRequestRequest { request_id: request.id.clone(), accept: true, ..Default::default() };
        let _: RespondFriendRequestResponse = self
            .app
            .call(&format!("{FRIENDS}/RespondFriendRequest"), Some(b), &accept)
            .await
            .expect("accept friend request");
    }

    /// `a` ends the friendship with `b` through the social API and the
    /// resulting `social.friend.removed` event reaches the quest domain.
    pub async fn unfriend(&self, a: &TestUser, b: &TestUser) {
        let _: RemoveFriendResponse = self
            .app
            .call(
                &format!("{FRIENDS}/RemoveFriend"),
                Some(a),
                &RemoveFriendRequest { user_id: b.id.to_string(), ..Default::default() },
            )
            .await
            .expect("remove friend");
        let removed = self.app.events_by("social.friend.removed", a.id).await;
        self.feed(removed.last().expect("social.friend.removed was emitted").clone()).await;
    }

    pub async fn set_zone(&self, user: &TestUser, zone: &str) {
        sqlx::query("UPDATE users SET time_zone = $2 WHERE id = $1")
            .bind(user.id)
            .bind(zone)
            .execute(&self.app.core.db)
            .await
            .unwrap();
    }

    pub async fn progress(&self, user: &TestUser) -> Progress {
        let response: GetProgressResponse =
            self.call(QUEST, "GetProgress", Some(user), &GetProgressRequest::default()).await.unwrap();
        response.progress.into_option().unwrap()
    }

    /// XP a source gave the player today, and whether it is capped.
    pub async fn source_today(&self, user: &TestUser, source: &str) -> (i32, bool) {
        let progress = self.progress(user).await;
        let source = progress
            .sources
            .iter()
            .find(|row| row.source.as_option().unwrap().key == source)
            .unwrap_or_else(|| panic!("no source {source}"));
        (source.xp_today, source.capped)
    }

    /// A lifetime counter of the player.
    pub async fn counter(&self, user: &TestUser, metric: &str) -> i64 {
        sqlx::query_scalar("SELECT value FROM quest_counters WHERE user_id = $1 AND metric = $2")
            .bind(user.id)
            .bind(metric)
            .fetch_optional(&self.app.core.db)
            .await
            .unwrap()
            .unwrap_or(0)
    }

    pub async fn badge_keys(&self, user: &TestUser) -> Vec<String> {
        let response: ListUserBadgesResponse =
            self.call(BADGES, "ListUserBadges", Some(user), &ListUserBadgesRequest::default()).await.unwrap();
        response.badges.into_iter().map(|badge| badge.key).collect()
    }

    pub async fn missions(&self, user: &TestUser) -> ListMissionsResponse {
        self.call(MISSIONS, "ListMissions", Some(user), &ListMissionsRequest::default()).await.unwrap()
    }

    pub async fn streak(&self, viewer: &TestUser, friend: &TestUser) -> Streak {
        let response: GetStreakResponse = self
            .call(
                STREAKS,
                "GetStreak",
                Some(viewer),
                &GetStreakRequest { user_id: friend.id.to_string(), ..Default::default() },
            )
            .await
            .unwrap();
        response.streak.into_option().unwrap()
    }

    pub async fn coins(&self, user: &TestUser) -> i64 {
        ledger::balance(&self.app.core, user.id).await.unwrap().coins
    }

    /// Gives `user` spendable coins.
    pub async fn fund(&self, user: &TestUser, coins: i64) {
        let key = format!("test.fund:{}", tokuly_core::ids::random_base62(10));
        ledger::grant(&self.app.core, user.id, coins, "test.fund", &key, json!({})).await.unwrap();
    }

    /// Notification requests of `kind` addressed to `user`, oldest first.
    pub async fn notifications(&self, user: &TestUser, kind: &str) -> Vec<serde_json::Value> {
        self.app
            .events("notify.requested")
            .await
            .into_iter()
            .map(|event| event.data)
            .filter(|data| data["user_id"] == user.id.to_string() && data["kind"] == kind)
            .collect()
    }

    /// Stores a streak between two friends as if it had been kept day by
    /// day: `days` long, with every date up to `settled_days_ago` days before
    /// today (Asia/Tokyo) settled, and nobody having done the part after it.
    pub async fn seed_streak(&self, a: &TestUser, b: &TestUser, days: i32, settled_days_ago: i64) {
        let (lo, hi) = if a.id < b.id { (a.id, b.id) } else { (b.id, a.id) };
        let settled = today() - chrono::Duration::days(settled_days_ago);
        // The deadline of the date after `settled`: midnight in Tokyo, UTC+9.
        let expires_at =
            (settled + chrono::Duration::days(2)).and_hms_opt(0, 0, 0).unwrap().and_utc() - chrono::Duration::hours(9);
        sqlx::query(
            "INSERT INTO quest_streaks (user_lo, user_hi, count, best, settled_day, started_on, expires_at)
             VALUES ($1, $2, $3, $3, $4, $4 - ($3 - 1), $5)
             ON CONFLICT (user_lo, user_hi) DO UPDATE SET count = EXCLUDED.count, best = EXCLUDED.best, settled_day = EXCLUDED.settled_day,
                 started_on = EXCLUDED.started_on, expires_at = EXCLUDED.expires_at, lo_days = '{}', hi_days = '{}', lost_count = 0, lost_at = NULL",
        )
        .bind(lo)
        .bind(hi)
        .bind(days)
        .bind(settled)
        .bind(expires_at)
        .execute(&self.app.core.db)
        .await
        .unwrap();
    }

    /// `from` reacts to `to` on the map, as the social domain announces it.
    pub async fn map_reaction(&self, from: &TestUser, to: &TestUser) {
        self.map_reaction_at(from, to, Utc::now()).await;
    }

    pub async fn map_reaction_at(&self, from: &TestUser, to: &TestUser, at: DateTime<Utc>) {
        let data = json!({
            "user_id": to.id, "kind": "social.map_reaction", "actor_id": from.id,
            "title": { "ja": "リアクション", "en": "Reaction" }, "body": { "ja": "", "en": "" },
            "path": "/map", "inbox": true, "push": true, "collapse_key": format!("social.map_reaction:{}", from.id),
        });
        self.feed(event_at("notify.requested", Some(from.id), data, at)).await;
    }

    /// `user` sends `count` stamps, as the stamp domain tracks them.
    pub async fn send_stamps(&self, user: &TestUser, count: usize) {
        for _ in 0..count {
            self.track(
                AnalyticsEvent::new("stamp.send")
                    .user(Some(user.id))
                    .subject("stamp", Uuid::now_v7())
                    .props(json!({ "context": "chat" })),
            )
            .await;
        }
    }

    /// Two users become friends as the social domain announces it (the
    /// event only; use [`Self::befriend`] for the friendship itself).
    pub async fn friend_accepted(&self, a: &TestUser, b: &TestUser) {
        self.feed(event(
            "social.friend.accepted",
            Some(a.id),
            json!({ "user_id": a.id, "friend_id": b.id, "source": 1 }),
        ))
        .await;
    }
}

/// The quest events among the frames a listener received within [`SOON`].
pub async fn pushes(listener: &mut FrameListener) -> Vec<Push> {
    listener
        .drain(SOON)
        .await
        .into_iter()
        .filter_map(|frame| match frame.body {
            Some(Body::Quest(event)) => event.event,
            _ => None,
        })
        .collect()
}

/// Asserts an RPC failed with this Connect code and reason.
pub fn assert_error<T: std::fmt::Debug>(result: Result<T, RpcError>, code: &str, reason: &str) {
    let error = result.expect_err("the call should fail");
    assert_eq!((error.code.as_str(), error.reason.as_str()), (code, reason), "{error}");
}
