//! "勉強中": who is studying right now, as far as they show it, and cheers.
use std::{collections::HashMap, time::Duration as StdDuration};

use chrono::{DateTime, Duration, Utc};
use tokuly_core::{
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::{
    common::PageInfo,
    school::{FriendStudying, StudyCheer, StudyingFriend, school_event::Event as Push},
};
use tokuly_social::graph;

use super::sessions::{self, ENDED, PAUSED, SessionRow};
use crate::{
    School,
    rooms::{self, RoomRow},
    settings, subjects,
};

pub(crate) const KIND_CHEER: &str = "school.cheer";
/// One cheer per friend in this many seconds.
const CHEER_COOLDOWN_SECONDS: u64 = 300;

impl School {
    /// The live sessions of `user_ids` whose owners show their studying:
    /// believed recently enough, or paused not too long ago. A suspended account
    /// shows nothing, exactly like one that hides its studying.
    async fn visible_sessions(&self, user_ids: &[Uuid], at: Time) -> AppResult<Vec<SessionRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM school_study_sessions s
             WHERE s.user_id = ANY($1) AND s.state <> {ENDED}
               AND ((s.state = {PAUSED} AND s.paused_at >= $3) OR (s.state <> {PAUSED} AND {believed} >= $2))
               AND NOT EXISTS (SELECT 1 FROM school_settings st WHERE st.user_id = s.user_id AND st.study_visibility <> 1)
               AND {}",
            sessions::COLUMNS,
            crate::account_is_active("s.user_id"),
            believed = sessions::BELIEVED_UNTIL
        )))
        .bind(user_ids)
        .bind(at - self.idle_limit())
        .bind(at - Duration::seconds(self.cfg.session_max_pause_seconds))
        .fetch_all(&self.core.db)
        .await?)
    }

    /// Turns live sessions into what friends see. The room a session is in
    /// is named only when `viewer` may enter it (without a viewer: when it
    /// is public).
    async fn studying_protos(
        &self,
        sessions: Vec<SessionRow>,
        viewer: Option<Uuid>,
        at: Time,
    ) -> AppResult<Vec<StudyingFriend>> {
        let core = &self.core;
        let user_ids: Vec<Uuid> = sessions.iter().map(|session| session.user_id).collect();
        let users = tokuly_identity::summaries(core, &user_ids).await?;
        let subjects =
            subjects::by_ids(core, &sessions.iter().filter_map(|session| session.subject_id).collect::<Vec<_>>())
                .await?;
        let rooms: HashMap<Uuid, RoomRow> =
            rooms::by_ids(core, &sessions.iter().filter_map(|session| session.room_id).collect::<Vec<_>>()).await?;
        let mut enterable: HashMap<Uuid, bool> = HashMap::new();
        for room in rooms.values() {
            let open_to_viewer = match viewer {
                Some(viewer) => self.can_enter(room, viewer).await?,
                None => room.is_open() && room.is_public(),
            };
            enterable.insert(room.id, open_to_viewer);
        }
        let mut friends: Vec<StudyingFriend> = sessions
            .iter()
            .filter_map(|session| {
                let user = users.get(&session.user_id).filter(|user| !user.deleted)?;
                let room = session.room_id.and_then(|id| rooms.get(&id));
                let timer = session.timer(room);
                let shown_room = room.filter(|room| enterable.get(&room.id).copied().unwrap_or(false));
                Some(StudyingFriend {
                    user: user.clone().into(),
                    session_id: session.id.to_string(),
                    state: (session.state as i32).into(),
                    subject: session.subject_id.and_then(|id| subjects.get(&id)).map(|subject| subject.label()).into(),
                    started_at: ts(session.started_at),
                    studied_seconds: session.studied_seconds(&timer, at, self.idle_limit()),
                    phase: timer.phase_at(at).phase.into(),
                    room_id: shown_room.map(|room| room.id.to_string()).unwrap_or_default(),
                    room_title: shown_room.map(|room| room.title.clone()).unwrap_or_default(),
                    server_time: ts(at),
                    ..Default::default()
                })
            })
            .collect();
        friends.sort_by_key(|friend| std::cmp::Reverse(friend.studied_seconds));
        Ok(friends)
    }

    pub(crate) async fn studying_friends(&self, viewer: Uuid, at: Time) -> AppResult<Vec<StudyingFriend>> {
        let friends = graph::friend_ids(&self.core, viewer).await?;
        let sessions = self.visible_sessions(&friends, at).await?;
        self.studying_protos(sessions, Some(viewer), at).await
    }

    /// What `owner`'s profile and friends may say about them studying right
    /// now: `None` when they are not, or do not show it.
    pub(crate) async fn studying_now(
        &self,
        owner: Uuid,
        viewer: Option<Uuid>,
        at: Time,
    ) -> AppResult<Option<StudyingFriend>> {
        let sessions = self.visible_sessions(&[owner], at).await?;
        Ok(self.studying_protos(sessions, viewer, at).await?.pop())
    }

    /// Tells the user's friends whether they are studying now. With `hidden`,
    /// the user just stopped showing it: friends are told they stopped.
    pub(crate) async fn announce_studying(&self, user_id: Uuid, at: Time, hidden: bool) -> AppResult<()> {
        let core = &self.core;
        let friend = if hidden {
            None
        } else {
            // Someone who never shows their studying has nothing to take back.
            if !settings::of(core, user_id).await?.shows_studying() {
                return Ok(());
            }
            self.studying_now(user_id, None, at).await?
        };
        let friends = graph::friend_ids(core, user_id).await?;
        let event = FriendStudying {
            user_id: user_id.to_string(),
            studying: friend.is_some(),
            friend: friend.into(),
            ..Default::default()
        };
        self.push_many(friends, Push::FriendStudying(Box::new(event))).await;
        Ok(())
    }

    /// [`Self::announce_studying`] for after a commit: presence is an extra,
    /// so a failure is logged rather than returned.
    pub(crate) async fn announce_studying_quietly(&self, user_id: Uuid, at: Time) {
        if let Err(e) = self.announce_studying(user_id, at, false).await {
            tracing::warn!(%user_id, error = %e, "announcing study presence failed");
        }
    }

    /// "がんばれ!" from `from` to their friend `to`, who must be studying
    /// where `from` can see it.
    pub(crate) async fn cheer(&self, from: Uuid, to: Uuid) -> AppResult<StudyCheer> {
        let core = &self.core;
        let at = self.now();
        ensure!(from != to, invalid, "CANNOT_CHEER_SELF", "cheering yourself on does not count");
        let session = if graph::are_friends(core, from, to).await? {
            self.visible_sessions(&[to], at).await?.pop()
        } else {
            None
        };
        // Not a friend, not studying, not showing it: all the same to the sender.
        let session =
            session.ok_or_else(|| AppError::precondition("NOT_STUDYING", "this person is not studying right now"))?;
        core.cache.rate_limit(&format!("school:cheer:{from}"), 60, StdDuration::from_secs(60)).await?;
        // The cool-down has a reason of its own: the client shows the last
        // cheer as still counting, which it must not do when the sender's
        // overall limit refused this one.
        core.cache
            .rate_limit(&format!("school:cheer:{from}:{to}"), 1, StdDuration::from_secs(CHEER_COOLDOWN_SECONDS))
            .await
            .map_err(|e| match e.reason.as_ref() {
                "RATE_LIMITED" => AppError::rate_limited("CHEER_TOO_SOON", "one cheer per friend every few minutes"),
                _ => e,
            })?;

        let sender = tokuly_identity::summary(core, from).await?;
        let notify = settings::of(core, to).await?.cheer_notifications;
        let mut tx = core.db.begin().await?;
        let (cheer_id, created_at): (Uuid, DateTime<Utc>) = sqlx::query_as(
            "INSERT INTO school_study_cheers (session_id, from_user_id, to_user_id, created_at) VALUES ($1, $2, $3, $4) RETURNING id, created_at",
        )
        .bind(session.id)
        .bind(from)
        .bind(to)
        .bind(at)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("UPDATE school_study_sessions SET cheer_count = cheer_count + 1 WHERE id = $1")
            .bind(session.id)
            .execute(&mut *tx)
            .await?;
        if notify {
            let request = NotificationRequest::new(
                to,
                KIND_CHEER,
                Localized::new("がんばれ!", "You've got this!"),
                Localized::new(
                    format!("{}さんが応援しています📣", sender.display_name),
                    format!("{} is cheering you on 📣", sender.display_name),
                ),
                "/school/study",
            )
            .actor(from)
            .collapse(format!("{KIND_CHEER}:{}:{from}", session.id))
            .data(serde_json::json!({ "session_id": session.id }));
            outbound::notify(&mut *tx, &request).await?;
        }
        tx.commit().await?;

        let cheer = StudyCheer {
            id: cheer_id.to_string(),
            from: sender.into(),
            session_id: session.id.to_string(),
            created_at: ts(created_at),
            ..Default::default()
        };
        self.push(to, Push::Cheer(Box::new(cheer.clone()))).await;
        Ok(cheer)
    }

    /// The cheers of one of the caller's sessions, newest first.
    pub(crate) async fn list_cheers(
        &self,
        user_id: Uuid,
        session_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<StudyCheer>, PageInfo)> {
        let core = &self.core;
        let owned: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM school_study_sessions WHERE id = $1 AND user_id = $2)")
                .bind(session_id)
                .bind(user_id)
                .fetch_one(&core.db)
                .await?;
        ensure!(owned, not_found, "SESSION_NOT_FOUND", "study session not found");
        let mut rows: Vec<(Uuid, Uuid, DateTime<Utc>)> = sqlx::query_as(
            "SELECT id, from_user_id, created_at FROM school_study_cheers
             WHERE session_id = $1 AND ($2::uuid IS NULL OR id < $2) ORDER BY id DESC LIMIT $3",
        )
        .bind(session_id)
        .bind(page.decode::<Uuid>()?)
        .bind(page.fetch())
        .fetch_all(&core.db)
        .await?;
        let info = page.finish(&mut rows, |(id, _, _)| *id);
        let users =
            tokuly_identity::summaries(core, &rows.iter().map(|(_, from, _)| *from).collect::<Vec<_>>()).await?;
        let cheers = rows
            .into_iter()
            .map(|(id, from, created_at)| StudyCheer {
                id: id.to_string(),
                from: tokuly_identity::users::pick(&users, from).into(),
                session_id: session_id.to_string(),
                created_at: ts(created_at),
                ..Default::default()
            })
            .collect();
        Ok((cheers, info))
    }
}
